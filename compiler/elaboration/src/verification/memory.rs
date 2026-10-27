//! Body-derived pointer footprints. This is separate from type propositions.
//! A summary preserves initialization and transfers no pointee ownership. It is
//! not a certificate for termination, functional specifications, or escape safety.
mod cleanup;
mod method_contracts;
use super::pattern_facts::{constant_bool, known_pattern, outer_guards};
pub use method_contracts::{
    MethodInvocation, MethodSummary, infer_method_in_package, infer_method_with_code_inputs,
    prepare_method_invocation,
};
use nir::normal_forms::tuple_fields;
use nir::program::{
    Expr, ExprKind as K, Function, FunctionPhase, LocalId, Package, Pattern,
    intrinsic_pointer_value,
};
use nir::{DeclarationId, Primitive, Term};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fmt,
};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ParameterPlace {
    /// Signature slot supplying the input. After selecting `input_fields` and
    /// unwrapping its pointer representation, `through` and `fields` begin at
    /// pointee storage. They may traverse wrappers stored in that pointee.
    pub parameter: usize,
    /// Owned input components selected before obtaining the first pointer.
    /// Empty means the argument itself supplies the pointer. These indices are
    /// not projections from pointee storage and must be resolved at the caller.
    pub input_fields: Vec<usize>,
    /// Each segment selects a pointer-valued slot, then loads that pointer.
    /// `fields` selects the final place after all these loads.
    pub through: Vec<Vec<(nir::program::FieldOwner, usize)>>,
    pub fields: Vec<(nir::program::FieldOwner, usize)>,
    /// Payload guards by load stage; the final stage is `through.len()`.
    pub conditions: BTreeMap<usize, Vec<(Vec<usize>, usize)>>,
}
impl ParameterPlace {
    pub(super) fn path_cost(&self) -> usize {
        self.through.iter().fold(
            condition_cost(&self.conditions).saturating_add(
                self.fields
                    .len()
                    .saturating_add(self.input_fields.len())
                    .saturating_add(1),
            ),
            |cost, fields| cost.saturating_add(fields.len()).saturating_add(1),
        )
    }
}
fn condition_cost(conditions: &BTreeMap<usize, Vec<(Vec<usize>, usize)>>) -> usize {
    conditions
        .values()
        .flatten()
        .fold(conditions.len(), |cost, (path, _)| {
            cost.saturating_add(path.len()).saturating_add(2)
        })
}
fn display_conditions(
    conditions: &BTreeMap<usize, Vec<(Vec<usize>, usize)>>,
    f: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    for (stage, guards) in conditions {
        write!(f, " (conditions (stage {stage})")?;
        for (path, tag) in guards {
            f.write_str(" (guard (fields")?;
            for field in path {
                write!(f, " {field}")?;
            }
            write!(f, ") (variant {tag}))")?;
        }
        f.write_str(")")?;
    }
    Ok(())
}
// Internal roots never enter a published parameter footprint.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum PointerRoot {
    Parameter(usize),
    Local { binding: LocalId, storage: usize },
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct PointerPlace {
    root: PointerRoot,
    input_fields: Vec<usize>,
    through: Vec<Vec<(nir::program::FieldOwner, usize)>>,
    fields: Vec<(nir::program::FieldOwner, usize)>,
    conditions: BTreeMap<usize, Vec<(Vec<usize>, usize)>>,
}
trait TargetCost {
    fn target_cost(&self) -> usize;
}
impl TargetCost for ParameterPlace {
    fn target_cost(&self) -> usize {
        self.path_cost()
    }
}
impl TargetCost for PointerPlace {
    fn target_cost(&self) -> usize {
        self.path_cost()
    }
}
impl PointerPlace {
    fn path_cost(&self) -> usize {
        self.through.iter().fold(
            condition_cost(&self.conditions).saturating_add(
                self.fields
                    .len()
                    .saturating_add(self.input_fields.len())
                    .saturating_add(1),
            ),
            |cost, path| cost.saturating_add(path.len()).saturating_add(1),
        )
    }
    fn append_path(&mut self, suffix: &ParameterPlace) -> Result<(), String> {
        if !suffix.input_fields.is_empty() {
            return Err(
                "input-field pointer origin must be resolved before appending a pointee path"
                    .into(),
            );
        }
        if suffix
            .conditions
            .keys()
            .any(|stage| *stage > suffix.through.len())
        {
            return Err("memory guard stage out of bounds".into());
        }
        let rebasing = self
            .fields
            .len()
            .saturating_mul(suffix.conditions.get(&0).map_or(0, Vec::len));
        if self
            .path_cost()
            .saturating_add(suffix.path_cost())
            .saturating_add(rebasing)
            > 128
        {
            return Err("memory pointer path depth exhausted".into());
        }
        let stage = self.through.len();
        for (offset, guards) in &suffix.conditions {
            let selected = self.conditions.entry(stage + offset).or_default();
            for (path, tag) in guards {
                let mut prefix = if *offset == 0 {
                    self.fields
                        .iter()
                        .map(|(_, index)| *index)
                        .collect::<Vec<_>>()
                } else {
                    vec![]
                };
                prefix.extend(path);
                selected.push((prefix, *tag));
            }
        }
        for fields in &suffix.through {
            self.fields.extend_from_slice(fields);
            self.through.push(std::mem::take(&mut self.fields));
        }
        self.fields.extend_from_slice(&suffix.fields);
        Ok(())
    }
    // Callers charge the path before copying, and select the actual pointer at
    // input_fields before discarding that prefix. It is not a pointee projection.
    fn append_resolved_path(&mut self, suffix: &ParameterPlace) -> Result<(), String> {
        let mut pointee = suffix.clone();
        pointee.input_fields.clear();
        self.append_path(&pointee)
    }
    fn external(self) -> Result<ParameterPlace, String> {
        let PointerRoot::Parameter(parameter) = self.root else {
            return Err("local memory target cannot escape in a result".into());
        };
        Ok(ParameterPlace {
            parameter,
            input_fields: self.input_fields,
            through: self.through,
            conditions: self.conditions,
            fields: self.fields,
        })
    }
}
impl fmt::Display for PointerPlace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(pointer-place (root ")?;
        match self.root {
            PointerRoot::Parameter(index) => write!(f, "(parameter {index})")?,
            PointerRoot::Local { binding, storage } => {
                write!(f, "(local (binding {binding}) (storage {storage}))")?
            }
        }
        f.write_str(")")?;
        if !self.input_fields.is_empty() {
            f.write_str(" (input-fields")?;
            for index in &self.input_fields {
                write!(f, " {index}")?;
            }
            f.write_str(")")?;
        }
        f.write_str(" (through")?;
        for path in &self.through {
            f.write_str(" (load")?;
            for (owner, index) in path {
                write!(f, " (field {owner} {index})")?;
            }
            f.write_str(")")?;
        }
        f.write_str(") (fields")?;
        for (owner, index) in &self.fields {
            write!(f, " (field {owner} {index})")?;
        }
        f.write_str(")")?;
        display_conditions(&self.conditions, f)?;
        f.write_str(")")
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Access {
    pub target: ParameterPlace,
    pub write: bool,
    pub read_value: bool,
    pub ty: Term,
}
/// A symbolic code source in the current function's delivered arguments.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CallableInput {
    pub parameter: usize,
    pub fields: Vec<usize>,
}
impl fmt::Display for CallableInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(callable-input (parameter {}) (fields", self.parameter)?;
        display_owned_path(&self.fields, f)?;
        f.write_str("))")
    }
}
/// Callable result lattice used by the interprocedural worklist.
/// NoReturn is bottom, not a guessed callable or an opaque result.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CallableResult {
    NoReturn,
    Known(iir::InstanceKey),
    Candidates(Vec<iir::InstanceKey>),
    Forwarded {
        named: Vec<iir::InstanceKey>,
        inputs: Vec<CallableInput>,
    },
    Unknown,
}
impl CallableResult {
    fn keys(&self) -> &[iir::InstanceKey] {
        match self {
            Self::Known(key) => std::slice::from_ref(key),
            Self::Candidates(keys) | Self::Forwarded { named: keys, .. } => keys,
            _ => &[],
        }
    }
    fn inputs(&self) -> &[CallableInput] {
        match self {
            Self::Forwarded { inputs, .. } => inputs,
            _ => &[],
        }
    }
    fn charge(&self, fuel: &mut usize) -> Result<(), String> {
        for key in self.keys() {
            *fuel = fuel
                .checked_sub(1)
                .ok_or("callable candidate budget exhausted")?;
            for ty in &key.arguments {
                bound_type(ty, fuel)?;
            }
        }
        for input in self.inputs() {
            *fuel = fuel
                .checked_sub(input.fields.len() + 1)
                .ok_or("callable input budget exhausted")?;
        }
        Ok(())
    }
    pub(crate) fn substitute_inputs(
        &self,
        arguments: &BTreeMap<usize, BTreeMap<Vec<usize>, CallableResult>>,
        fuel: &mut usize,
    ) -> Result<Self, String> {
        self.charge(fuel)?;
        let Self::Forwarded { named, inputs } = self else {
            return Ok(self.clone());
        };
        let mut result = match named.as_slice() {
            [] => Self::NoReturn,
            [key] => Self::Known(key.clone()),
            _ => Self::Candidates(named.clone()),
        };
        // Simultaneous substitution: values already refer to the caller's
        // parameters and must not be substituted again in the callee's frame.
        for input in inputs {
            let value = arguments
                .get(&input.parameter)
                .and_then(|fields| fields.get(&input.fields))
                .unwrap_or(&Self::Unknown);
            result.join(value, fuel)?;
        }
        Ok(result)
    }
    pub(crate) fn join(&mut self, other: &Self, fuel: &mut usize) -> Result<(), String> {
        self.charge(fuel)?;
        other.charge(fuel)?;
        match (&*self, other) {
            (Self::Unknown, _) | (_, Self::Unknown) => *self = Self::Unknown,
            (Self::NoReturn, _) => *self = other.clone(),
            (_, Self::NoReturn) => {}
            _ => {
                let mut keys = self.keys().to_vec();
                for key in other.keys() {
                    *fuel = fuel
                        .checked_sub(keys.len())
                        .ok_or("callable candidate merge budget exhausted")?;
                    if !keys.contains(key) {
                        keys.push(key.clone());
                    }
                }
                keys.sort_by_cached_key(ToString::to_string);
                let mut inputs = self.inputs().to_vec();
                for input in other.inputs() {
                    *fuel = fuel
                        .checked_sub(inputs.len() + 1)
                        .ok_or("callable input join budget exhausted")?;
                    if !inputs.contains(input) {
                        inputs.push(input.clone());
                    }
                }
                inputs.sort();
                *self = if !inputs.is_empty() {
                    Self::Forwarded {
                        named: keys,
                        inputs,
                    }
                } else if keys.len() == 1 {
                    Self::Known(keys.pop().unwrap())
                } else {
                    Self::Candidates(keys)
                };
            }
        }
        Ok(())
    }
}
impl fmt::Display for CallableResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoReturn => f.write_str("(callable-result no-return)"),
            Self::Known(key) => write!(f, "(callable-result {key})"),
            Self::Candidates(keys) => {
                f.write_str("(callable-result (candidates")?;
                for key in keys {
                    write!(f, " {key}")?;
                }
                f.write_str("))")
            }
            Self::Forwarded { named, inputs } => {
                f.write_str("(callable-result (forwarded (named")?;
                for key in named {
                    write!(f, " {key}")?;
                }
                f.write_str(") (inputs")?;
                for input in inputs {
                    write!(f, " {input}")?;
                }
                f.write_str(")))")
            }
            Self::Unknown => f.write_str("(callable-result unknown)"),
        }
    }
}
mod borrowed_code;
pub use borrowed_code::{
    BorrowedCallableInputs, BorrowedCallableValue, infer_with_borrowed_callable_inputs,
    infer_with_borrowed_callable_inputs_and_cleanup,
};
mod effects;
pub(crate) use effects::effect_transfer_layout;
pub use effects::{
    ConditionalClosureSummary, EffectInput, EffectParameterAccess, HandlerParameterSummary,
    infer_closure_with_effect_inputs, infer_handler_arm_memory, infer_with_effect_inputs,
};
pub(crate) use effects::{
    data_passing_signature, direct_handler_accesses, direct_handler_accesses_with_code,
    effect_borrowed_code_layout, effect_data_layout, stationary_handler_write_sites,
};

#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub declaration: DeclarationId,
    pub arguments: Vec<Term>,
    pub signature: Term,
    /// Concrete code premises on which this footprint is conditional.
    pub callable_inputs: BTreeMap<CallableInput, CallableResult>,
    pub prepared_inputs: PreparedCodeInputs,
    /// Stable code identities reached through delivered pointers; conditional evidence.
    pub borrowed_inputs: BorrowedCallableInputs,
    /// Handler safety premises; accesses alone are not an unconditional footprint.
    pub effect_inputs: Vec<EffectInput>,
    pub accesses: Vec<Access>,
    /// May reject a checked program predicate; not an error-result ABI effect.
    pub may_reject: bool,
    /// Owned cleanup may access memory beyond the explicit body footprint.
    pub may_run_cleanup: bool,
    /// Possible parameter-relative targets of a pointer result. None is opaque;
    /// an empty set means no established normal return in the fixed point.
    pub result_targets: Option<BTreeSet<ParameterPlace>>,
    /// Parameter-relative origins for each pointer-bearing owned result field.
    pub pointer_fields: BTreeMap<Vec<usize>, Option<BTreeSet<ParameterPlace>>>,
    pub callable_result: CallableResult,
    /// Named code at each owned function-valued result leaf. NoReturn is the
    /// worklist bottom for that leaf; Unknown is not a recursive seed.
    pub callable_fields: BTreeMap<Vec<usize>, CallableResult>,
    pub code_fields: CodeFields,
}
impl fmt::Display for ParameterPlace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(parameter-place (parameter {})", self.parameter)?;
        if !self.input_fields.is_empty() {
            f.write_str(" (input-fields")?;
            for index in &self.input_fields {
                write!(f, " {index}")?;
            }
            f.write_str(")")?;
        }
        if !self.through.is_empty() {
            f.write_str(" (through")?;
            for fields in &self.through {
                f.write_str(" (load-pointer (fields")?;
                for (owner, index) in fields {
                    write!(f, " (field {owner} {index})")?;
                }
                f.write_str("))")?;
            }
            f.write_str(")")?;
        }
        f.write_str(" (fields")?;
        for (declaration, index) in &self.fields {
            write!(f, " (field {declaration} {index})")?;
        }
        f.write_str(")")?;
        display_conditions(&self.conditions, f)?;
        f.write_str(")")
    }
}
impl fmt::Display for Access {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let operation = if self.write {
            "write"
        } else if self.read_value {
            "read"
        } else {
            "address"
        };
        write!(f, "({operation} {} (type {}))", self.target, self.ty)
    }
}
impl fmt::Display for Summary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(memory-summary (declaration {}) (arguments",
            self.declaration
        )?;
        for argument in &self.arguments {
            write!(f, " {argument}")?;
        }
        write!(f, ") (signature {}) (callable-inputs", self.signature)?;
        for (input, value) in &self.callable_inputs {
            write!(f, " (premise {input} {value})")?;
        }
        f.write_str(") (prepared-inputs")?;
        for (input, values) in &self.prepared_inputs {
            write!(f, " (premise {input} (candidates")?;
            for value in values {
                write!(f, " {value}")?;
            }
            f.write_str("))")?;
        }
        f.write_str(") (borrowed-inputs")?;
        for (input, fields) in &self.borrowed_inputs {
            write!(f, " (premise {input} (fields")?;
            for (path, value) in fields {
                f.write_str(" (field (path")?;
                display_owned_path(path, f)?;
                write!(f, ") {value})")?;
            }
            f.write_str("))")?;
        }
        f.write_str(") (effect-inputs")?;
        for input in &self.effect_inputs {
            write!(f, " {input}")?;
        }
        f.write_str(") (accesses")?;
        for access in &self.accesses {
            write!(f, " {access}")?;
        }
        write!(
            f,
            ") (may-reject {}) (may-run-cleanup {}) (result-targets",
            self.may_reject, self.may_run_cleanup
        )?;
        if let Some(targets) = &self.result_targets {
            for target in targets {
                write!(f, " {target}")?;
            }
        } else {
            f.write_str(" unknown")?;
        }
        f.write_str(")")?;
        display_pointer_fields(&self.pointer_fields, f)?;
        write!(f, " {} (callable-fields", self.callable_result)?;
        for (path, key) in &self.callable_fields {
            f.write_str(" (leaf (path")?;
            display_owned_path(path, f)?;
            write!(f, ") {key})")?;
        }
        f.write_str(")")?;
        display_code_fields(&self.code_fields, f)?;
        f.write_str(")")
    }
}
fn display_pointer_fields(
    fields: &BTreeMap<Vec<usize>, Option<BTreeSet<ParameterPlace>>>,
    f: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    if !fields.is_empty() {
        f.write_str(" (pointer-fields")?;
        for (path, targets) in fields {
            f.write_str(" (leaf (path")?;
            display_owned_path(path, f)?;
            f.write_str(") (targets")?;
            if let Some(targets) = targets {
                for target in targets {
                    write!(f, " {target}")?;
                }
            } else {
                f.write_str(" unknown")?;
            }
            f.write_str("))")?;
        }
        f.write_str(")")?;
    }
    Ok(())
}
/// An owned callable leaf inside a captured or local value. Field indices are
/// checked against the owned layout; enum payloads carry a variant path marker.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CallablePlace {
    pub binding: LocalId,
    pub fields: Vec<usize>,
}
impl CallablePlace {
    fn root(binding: LocalId) -> Self {
        Self {
            binding,
            fields: vec![],
        }
    }
}
// Lexicographic paths keep a subtree contiguous. Stop at its first sibling;
// unrelated roots and later fields must not consume projection fuel.
fn callable_subtree<'a, T>(
    values: &'a BTreeMap<CallablePlace, T>,
    place: &CallablePlace,
) -> impl Iterator<Item = (&'a CallablePlace, &'a T)> {
    let place = place.clone();
    values.range(place.clone()..).take_while(move |(leaf, _)| {
        leaf.binding == place.binding && leaf.fields.starts_with(&place.fields)
    })
}
impl fmt::Display for CallablePlace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(callable-place (binding {}) (fields", self.binding)?;
        display_owned_path(&self.fields, f)?;
        f.write_str("))")
    }
}
fn owned_fields(
    package: Option<&Package>,
    ty: &Term,
    fuel: &mut usize,
) -> Result<Vec<Term>, String> {
    bound_type(ty, fuel)?;
    *fuel = fuel
        .checked_sub(1)
        .ok_or("owned memory layout budget exhausted")?;
    if let Some(fields) = tuple_fields(package, ty) {
        return Ok(fields.to_vec());
    }
    if let Term::Nominal {
        declaration,
        arguments,
    } = ty
    {
        if let Some(definition) = package.and_then(|package| {
            package
                .newtypes
                .iter()
                .find(|definition| definition.declaration == *declaration)
        }) {
            return definition
                .representation_budgeted(arguments, &[], fuel)
                .map(|representation| vec![representation])
                .map_err(|error| error.to_string());
        }
    }
    match ty {
        // `str` is a borrowed character view, not pointer-free scalar data.
        // Until its backing-region provenance is represented, neither a Copy
        // implementation nor a trivial destructor authorizes forgetting it.
        Term::Primitive(Primitive::Str) => Err(
            "string-view memory provenance is not implemented; str requires its backing character storage to remain valid".into(),
        ),
        Term::Record(fields) => Ok(fields.iter().map(|(_, ty)| ty.clone()).collect()),
        Term::Nominal {
            declaration,
            arguments,
        } => package
            .and_then(|p| p.structs.iter().find(|s| s.declaration == *declaration))
            .ok_or("memory callable fields require a known struct layout")?
            .instantiate_fields(arguments)
            .map(|fields| fields.into_iter().map(|(_, ty)| ty).collect())
            .map_err(|e| e.to_string()),
        _ => Err("memory callable fields require an owned record or tuple layout".into()),
    }
}

/// One pointer input with distinct symbolic and physical paths. Guards are
/// ordered outermost first, so an inactive payload is never projected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PointerInputLeaf {
    pub input_fields: Vec<usize>,
    pub storage_fields: Vec<usize>,
    pub guards: Vec<(Vec<usize>, usize)>,
    pub pointee: Term,
}
impl fmt::Display for PointerInputLeaf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(pointer-input (input-fields")?;
        display_owned_path(&self.input_fields, f)?;
        f.write_str(") (storage-fields")?;
        for field in &self.storage_fields {
            write!(f, " {field}")?;
        }
        f.write_str(") (guards")?;
        for (path, tag) in &self.guards {
            f.write_str(" (guard (path")?;
            for field in path {
                write!(f, " {field}")?;
            }
            write!(f, ") (tag {tag}))")?;
        }
        write!(f, ") (pointee {}))", self.pointee)
    }
}

/// Enumerate conditional pointer inputs without discarding variant guards.
/// This is layout evidence, not a proof that any pointee is live or accessible.
pub fn pointer_input_layout(
    package: Option<&Package>,
    ty: &Term,
    fuel: &mut usize,
) -> Result<Vec<PointerInputLeaf>, String> {
    fn walk(
        package: Option<&Package>,
        ty: &Term,
        input: &mut Vec<usize>,
        storage: &mut Vec<usize>,
        guards: &mut Vec<(Vec<usize>, usize)>,
        output: &mut Vec<PointerInputLeaf>,
        fuel: &mut usize,
    ) -> Result<(), String> {
        if input.len() >= 128 {
            return Err("input pointer layout depth exhausted".into());
        }
        *fuel = fuel
            .checked_sub(1)
            .ok_or("input pointer layout budget exhausted")?;
        if let Some((wrappers, pointee)) = pointer_representation(package, ty, fuel)? {
            let guard_cost = guards
                .iter()
                .try_fold(0usize, |cost, (path, _)| cost.checked_add(path.len() + 1))
                .ok_or("input pointer path budget exhausted")?;
            let cost = input
                .len()
                .saturating_add(storage.len())
                .saturating_add(wrappers)
                .saturating_add(guard_cost)
                .saturating_add(1);
            *fuel = fuel
                .checked_sub(cost)
                .ok_or("input pointer path budget exhausted")?;
            let mut physical = storage.clone();
            physical.extend(std::iter::repeat_n(0, wrappers));
            output.push(PointerInputLeaf {
                input_fields: input.clone(),
                storage_fields: physical,
                guards: guards.clone(),
                pointee,
            });
            return Ok(());
        }
        if intrinsic_pointer_value(ty)
            || matches!(
                ty,
                Term::Function { .. }
                    | Term::UnsafeFunction(_)
                    | Term::Primitive(Primitive::Void | Primitive::Never)
            )
        {
            return Ok(());
        }
        if let Term::Closure { environment, .. } = ty {
            for (index, field) in environment.iter().enumerate() {
                input.push(index);
                storage.push(index);
                walk(package, field, input, storage, guards, output, fuel)?;
                storage.pop();
                input.pop();
            }
            return Ok(());
        }
        if matches!(ty, Term::ErrorResult { .. } | Term::Optional(_))
            || matches!(ty,Term::Nominal {declaration,..} if package.is_some_and(|p|p.enums.iter().any(|e|e.declaration==*declaration)))
        {
            for (tag, variant) in owned_variants(package, ty, fuel)?.into_iter().enumerate() {
                *fuel = fuel
                    .checked_sub(storage.len() + 1)
                    .ok_or("input pointer guard budget exhausted")?;
                guards.push((storage.clone(), tag));
                for (index, field) in variant.fields.iter().enumerate() {
                    input.extend([VARIANT_FIELD, tag, index]);
                    storage.push(index);
                    walk(package, field, input, storage, guards, output, fuel)?;
                    storage.pop();
                    input.truncate(input.len() - 3);
                }
                guards.pop();
            }
            return Ok(());
        }
        for (index, field) in owned_fields(package, ty, fuel)?.iter().enumerate() {
            input.push(index);
            storage.push(index);
            walk(package, field, input, storage, guards, output, fuel)?;
            storage.pop();
            input.pop();
        }
        Ok(())
    }
    let mut output = vec![];
    walk(
        package,
        ty,
        &mut vec![],
        &mut vec![],
        &mut vec![],
        &mut output,
        fuel,
    )?;
    Ok(output)
}
fn unconditional_pointer_inputs(
    package: Option<&Package>,
    ty: &Term,
    fuel: &mut usize,
) -> Result<Vec<PointerInputLeaf>, String> {
    let layout = pointer_input_layout(package, ty, fuel)?;
    if layout.iter().any(|leaf| !leaf.guards.is_empty()) {
        return Err("aggregate pointer dependencies require a richer memory contract".into());
    }
    Ok(layout)
}

/// Legacy unconditional input contract. Conditional layouts must retain their
/// guards through invocation before this boundary can accept them.
pub fn input_pointer_fields(
    package: Option<&Package>,
    ty: &Term,
    fuel: &mut usize,
) -> Result<BTreeMap<Vec<usize>, Term>, String> {
    Ok(unconditional_pointer_inputs(package, ty, fuel)?
        .into_iter()
        .map(|leaf| (leaf.input_fields, leaf.pointee))
        .collect())
}
/// Conditional result delivery preserves logical keys until origins are
/// merged at physical slots. Callers must union collisions, never overwrite.
pub(super) fn conditional_pointer_storage_paths(
    package: Option<&Package>,
    ty: &Term,
    fuel: &mut usize,
) -> Result<BTreeMap<Vec<usize>, Vec<usize>>, String> {
    Ok(pointer_input_layout(package, ty, fuel)?
        .into_iter()
        .map(|leaf| (leaf.input_fields, leaf.storage_fields))
        .collect())
}
// Symbolic summaries keep variant-qualified identities. Their consumers must
// establish or exclude each leaf at the invocation boundary.
fn symbolic_pointer_fields(
    package: Option<&Package>,
    ty: &Term,
    fuel: &mut usize,
) -> Result<BTreeMap<Vec<usize>, Term>, String> {
    Ok(pointer_input_layout(package, ty, fuel)?
        .into_iter()
        .map(|leaf| (leaf.input_fields, leaf.pointee))
        .collect())
}
const VARIANT_FIELD: usize = usize::MAX;
fn display_owned_path(path: &[usize], f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let mut indices = path.iter();
    while let Some(index) = indices.next() {
        if *index == VARIANT_FIELD {
            if let Some(variant) = indices.next() {
                write!(f, " (variant {variant})")?;
            } else {
                f.write_str(" (invalid-variant)")?;
            }
        } else {
            write!(f, " {index}")?;
        }
    }
    Ok(())
}

pub(crate) fn error_tag_mapping(
    expr: &Expr,
    package: Option<&Package>,
    fuel: &mut usize,
) -> Result<nir::error_results::RepresentationTransport, String> {
    let K::ErrorConvert { conversion, value } = &expr.kind else {
        return Err("expected error transport".into());
    };
    if !matches!(
        conversion,
        nir::error_results::Conversion::Embed | nir::error_results::Conversion::Product
    ) {
        return Err("expected error transport".into());
    }
    nir::error_results::transport_plan(
        &value.ty,
        &expr.ty,
        *conversion == nir::error_results::Conversion::Product,
        fuel,
        &mut |a, b| a == b,
        &|head| {
            package.is_some_and(|p| {
                p.normal_forms
                    .iter()
                    .any(|m| m.declaration == head && m.role == nir::normal_forms::Role::Tuple)
            })
        },
    )
    .map_err(|failure| failure.to_string())
}
pub(crate) fn remap_error_path(
    path: &mut [usize],
    mapping: &nir::error_results::RepresentationTransport,
    fuel: &mut usize,
) -> Result<(), String> {
    mapping
        .remap_path(path, fuel)
        .map_err(|failure| failure.to_string())
}

// Cleanup uses success tag zero and error row index plus one. This is a
// private storage layout, not an enum type or an effect continuation.
pub(crate) fn owned_constructor<'a>(
    expr: &'a Expr,
    fuel: &mut usize,
) -> Result<(&'a [Expr], Option<usize>), String> {
    match &expr.kind {
        K::Tuple(fields) | K::AnonymousRecord { fields } | K::Record { fields, .. } => {
            Ok((fields, None))
        }
        K::OptionalAbsent => Ok((&[], Some(0))),
        K::OptionalPresent(value) => Ok((std::slice::from_ref(value.as_ref()), Some(1))),
        K::Variant { fields, index, .. } => Ok((fields, Some(*index))),
        K::ErrorConvert { conversion, value } => {
            use nir::error_results::Conversion;
            if matches!(conversion, Conversion::Embed | Conversion::Product) {
                return Err("error transport is not an owned constructor".into());
            }
            nir::error_results::validate_conversion(
                *conversion,
                &value.ty,
                &expr.ty,
                fuel,
                &mut |a, b| a == b,
            )
            .map_err(|e| e.to_string())?;
            let Term::ErrorResult { errors, .. } = &expr.ty else {
                unreachable!()
            };
            let tag = match conversion {
                Conversion::Success => 0,
                Conversion::Failure => {
                    errors
                        .iter()
                        .position(|ty| ty == &value.ty)
                        .ok_or("error payload label has no concrete layout")?
                        + 1
                }
                Conversion::Embed | Conversion::Product => {
                    return Err("error row transport requires resource provenance remapping".into());
                }
            };
            Ok((std::slice::from_ref(value.as_ref()), Some(tag)))
        }
        _ => Err("expected owned constructor".into()),
    }
}

pub(crate) fn owned_variant_pattern<'p>(
    package: Option<&Package>,
    pattern: &'p Pattern,
    ty: &Term,
    fuel: &mut usize,
) -> Result<(usize, &'p [Pattern], Vec<Term>), String> {
    bound_type(ty, fuel)?;
    let (tag, patterns) = match pattern {
        Pattern::Variant {
            declaration,
            index,
            fields,
        } => {
            if !matches!(ty,Term::Nominal {declaration:actual,..} if actual==declaration) {
                return Err("variant pattern declaration mismatch".into());
            }
            (*index, fields.as_slice())
        }
        Pattern::OptionalAbsent if matches!(ty, Term::Optional(_)) => (0, &[][..]),
        Pattern::OptionalPresent(inner) if matches!(ty, Term::Optional(_)) => {
            (1, std::slice::from_ref(inner.as_ref()))
        }
        Pattern::ErrorResult { label, inner } => {
            let Term::ErrorResult { errors, .. } = ty else {
                return Err("error pattern requires an error result".into());
            };
            let tag = match label {
                None => 0,
                Some(label) => {
                    bound_type(label, fuel)?;
                    let mut selected = None;
                    for (index, candidate) in errors.iter().enumerate() {
                        bound_type(candidate, fuel)?;
                        if candidate == label {
                            selected = Some(index + 1);
                            break;
                        }
                    }
                    selected.ok_or("error pattern label is absent from input row")?
                }
            };
            (tag, std::slice::from_ref(inner.as_ref()))
        }
        _ => return Err("expected variant or error pattern".into()),
    };
    let mut variants = owned_variants(package, ty, fuel)?;
    let fields = std::mem::take(
        &mut variants
            .get_mut(tag)
            .ok_or("pattern branch out of bounds")?
            .fields,
    );
    if fields.len() != patterns.len() {
        return Err("pattern payload arity mismatch".into());
    }
    Ok((tag, patterns, fields))
}

pub(crate) fn owned_variants(
    package: Option<&Package>,
    ty: &Term,
    fuel: &mut usize,
) -> Result<Vec<nir::program::Variant>, String> {
    bound_type(ty, fuel)?;
    if let Term::Optional(payload) = ty {
        *fuel = fuel
            .checked_sub(2)
            .ok_or("optional variant layout budget exhausted")?;
        return Ok(vec![
            nir::program::Variant {
                name: "absent".into(),
                shape: nir::program::VariantShape::Unit,
                fields: vec![],
            },
            nir::program::Variant {
                name: "present".into(),
                shape: nir::program::VariantShape::Tuple,
                fields: vec![(**payload).clone()],
            },
        ]);
    }
    if let Term::ErrorResult { errors, result } = ty {
        return Ok(std::iter::once(result.as_ref())
            .chain(errors.iter())
            .enumerate()
            .map(|(tag, ty)| nir::program::Variant {
                name: tag.to_string(),
                shape: nir::program::VariantShape::Tuple,
                fields: vec![ty.clone()],
            })
            .collect());
    }
    let Term::Nominal {
        declaration,
        arguments,
    } = ty
    else {
        return Err("memory variant requires a nominal layout".into());
    };
    let package = package.ok_or("memory variant requires a package")?;
    for definition in &package.enums {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("memory variant lookup budget exhausted")?;
        if definition.declaration == *declaration {
            *fuel = fuel
                .checked_sub(definition.variants.len())
                .ok_or("memory variant layout budget exhausted")?;
            for variant in &definition.variants {
                for field in &variant.fields {
                    bound_type(field, fuel)?;
                    for argument in arguments {
                        bound_type(argument, fuel)?;
                    }
                }
            }
            return definition
                .instantiate_variants(arguments)
                .map_err(|e| e.to_string());
        }
    }
    Err("memory variant metadata missing".into())
}
pub(crate) fn owned_path_type(
    package: Option<&Package>,
    ty: &Term,
    path: &[usize],
    fuel: &mut usize,
) -> Result<Term, String> {
    bound_type(ty, fuel)?;
    let mut ty = ty.clone();
    let mut position = 0;
    while position < path.len() {
        if path[position] == VARIANT_FIELD {
            let tail = path
                .get(position + 1..position + 3)
                .ok_or("incomplete memory variant field path")?;
            ty = owned_variants(package, &ty, fuel)?
                .get(tail[0])
                .and_then(|variant| variant.fields.get(tail[1]))
                .cloned()
                .ok_or("memory variant field index out of bounds")?;
            position += 3;
        } else {
            // Cleanup footprints can traverse a closure's owned environment.
            // This is a storage path, not projection into its call signature.
            let fields = if let Term::Closure { environment, .. } = &ty {
                bound_type(&ty, fuel)?;
                *fuel = fuel
                    .checked_sub(environment.len() + 1)
                    .ok_or("memory closure path budget exhausted")?;
                environment.clone()
            } else {
                owned_fields(package, &ty, fuel)?
            };
            ty = fields
                .get(path[position])
                .cloned()
                .ok_or("memory callable field index out of bounds")?;
            position += 1;
        }
    }
    Ok(ty)
}
/// A checked storage projection. Guards name enum containers in physical
/// coordinates relative to the supplied root, before the corresponding payload.
#[derive(Clone, Debug, PartialEq)]
pub struct StorageProjection {
    pub fields: Vec<(nir::program::FieldOwner, usize)>,
    pub guards: Vec<(Vec<usize>, usize)>,
    pub ty: Term,
}
/// Truth of a storage guard in the current program state, not a type proof.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuardActivity {
    Active,
    Inactive,
    Conditional,
}
impl fmt::Display for GuardActivity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Active => "(guard-activity active)",
            Self::Inactive => "(guard-activity inactive)",
            Self::Conditional => "(guard-activity conditional)",
        })
    }
}
/// Query each enum domain with its enclosing variant selections in the path.
/// Raw physical paths are insufficient: distinct variants can share a slot.
/// An inactive outer guard short-circuits before touching an absent payload.
pub fn storage_guard_activity(
    guards: &[(Vec<usize>, usize)],
    fuel: &mut usize,
    mut query: impl FnMut(&[usize], usize) -> GuardActivity,
) -> Result<GuardActivity, String> {
    let mut activity = GuardActivity::Active;
    for (index, (fields, tag)) in guards.iter().enumerate() {
        let length = fields.len().saturating_add(index.saturating_mul(2));
        if length > 128 {
            return Err("storage guard depth exhausted".into());
        }
        *fuel = fuel
            .checked_sub(length.saturating_add(1))
            .ok_or("storage guard budget exhausted")?;
        if index > 0 {
            let previous = &guards[index - 1].0;
            *fuel = fuel
                .checked_sub(previous.len().saturating_add(1))
                .ok_or("storage guard budget exhausted")?;
            if previous.len() >= fields.len() || !fields.starts_with(previous) {
                return Err("storage guards are not an ordered ancestor chain".into());
            }
        }
        let mut symbolic = Vec::with_capacity(length);
        let mut ancestors = guards[..index].iter().peekable();
        for (depth, field) in fields.iter().enumerate() {
            if ancestors
                .peek()
                .is_some_and(|(path, _)| path.len() == depth)
            {
                let (_, selected) = ancestors.next().unwrap();
                symbolic.extend([VARIANT_FIELD, *selected]);
            }
            symbolic.push(*field);
        }
        match query(&symbolic, *tag) {
            GuardActivity::Inactive => return Ok(GuardActivity::Inactive),
            GuardActivity::Conditional => activity = GuardActivity::Conditional,
            GuardActivity::Active => (),
        }
    }
    Ok(activity)
}
impl StorageProjection {
    pub fn activity(
        &self,
        fuel: &mut usize,
        query: impl FnMut(&[usize], usize) -> GuardActivity,
    ) -> Result<GuardActivity, String> {
        storage_guard_activity(&self.guards, fuel, query)
    }
}
impl fmt::Display for StorageProjection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(storage-projection (fields")?;
        for (owner, index) in &self.fields {
            write!(f, " (field {owner} {index})")?;
        }
        f.write_str(") (guards")?;
        for (path, tag) in &self.guards {
            f.write_str(" (guard (fields")?;
            for index in path {
                write!(f, " {index}")?;
            }
            write!(f, ") (variant {tag}))")?;
        }
        write!(f, ") (type {}))", self.ty)
    }
}
/// Resolve the same symbolic variant paths used by pointer input/result leaves.
/// This describes conditional storage; it does not prove the guards hold.
pub fn project_owned_storage(
    package: Option<&Package>,
    root: &Term,
    path: &[usize],
    fuel: &mut usize,
) -> Result<StorageProjection, String> {
    bound_type(root, fuel)?;
    let mut result = StorageProjection {
        fields: vec![],
        guards: vec![],
        ty: root.clone(),
    };
    extend_storage_projection(package, &mut result, path, fuel)?;
    Ok(result)
}
impl StorageProjection {
    /// Continue a checked projection within the same storage root. No pointer
    /// load occurs here: guards in the suffix retain the outer field prefix.
    /// Failure leaves the original projection unchanged; fuel remains consumed.
    pub fn then(
        &self,
        package: Option<&Package>,
        path: &[usize],
        fuel: &mut usize,
    ) -> Result<Self, String> {
        bound_type(&self.ty, fuel)?;
        let cost = self
            .guards
            .iter()
            .fold(self.fields.len().saturating_add(1), |cost, (fields, _)| {
                cost.saturating_add(fields.len()).saturating_add(1)
            });
        *fuel = fuel
            .checked_sub(cost)
            .ok_or("storage projection copy budget exhausted")?;
        let mut result = self.clone();
        extend_storage_projection(package, &mut result, path, fuel)?;
        Ok(result)
    }
}
/// A typed traversal across distinct pointee storage roots. Each load keeps
/// its own root-relative guards; target guards belong only to the last root.
#[derive(Clone, Debug, PartialEq)]
pub struct StorageTraversal {
    pub loads: Vec<StorageProjection>,
    pub target: StorageProjection,
}
impl StorageTraversal {
    pub fn new(
        package: Option<&Package>,
        root: &Term,
        path: &[usize],
        fuel: &mut usize,
    ) -> Result<Self, String> {
        Ok(Self {
            loads: vec![],
            target: project_owned_storage(package, root, path, fuel)?,
        })
    }
    fn check_extension(&self, extra: usize, fuel: &mut usize) -> Result<(), String> {
        let mut depth = self.loads.len().saturating_add(extra);
        for projection in self.loads.iter().chain(std::iter::once(&self.target)) {
            *fuel = fuel
                .checked_sub(1)
                .ok_or("storage traversal budget exhausted")?;
            depth = depth
                .saturating_add(projection.fields.len())
                .saturating_add(projection.guards.len().saturating_mul(2));
        }
        if depth > 128 {
            return Err("storage traversal depth exhausted".into());
        }
        Ok(())
    }
    pub fn then(
        &mut self,
        package: Option<&Package>,
        path: &[usize],
        fuel: &mut usize,
    ) -> Result<(), String> {
        self.check_extension(path.len(), fuel)?;
        let next = self.target.then(package, path, fuel)?;
        self.target = next;
        Ok(())
    }
    /// Load exactly one pointer. Conditions already accumulated remain attached
    /// to that slot, and the new projection starts at the loaded pointee root.
    pub fn load(
        &mut self,
        package: Option<&Package>,
        path: &[usize],
        fuel: &mut usize,
    ) -> Result<(), String> {
        self.check_extension(path.len().saturating_add(1), fuel)?;
        let Term::Pointer(subject) = &self.target.ty else {
            return Err("storage traversal load is not a pointer".into());
        };
        let next = project_owned_storage(package, subject, path, fuel)?;
        let slot = std::mem::replace(&mut self.target, next);
        self.loads.push(slot);
        Ok(())
    }
    /// Validate a transported physical path and its payload-existence guards.
    /// Both are relative to the current target; enclosing conditions are kept.
    /// This reconstructs the typed path, without asserting that any tag holds.
    pub fn then_guarded(
        &mut self,
        package: Option<&Package>,
        fields: &[(nir::program::FieldOwner, usize)],
        guards: &[(Vec<usize>, usize)],
        fuel: &mut usize,
    ) -> Result<(), String> {
        let length = fields.len().saturating_add(guards.len().saturating_mul(2));
        self.check_extension(length, fuel)?;
        *fuel = fuel
            .checked_sub(length.saturating_add(1))
            .ok_or("transported storage path budget exhausted")?;
        let mut previous_depth = None;
        for (container, _) in guards {
            *fuel = fuel
                .checked_sub(container.len().saturating_add(1))
                .ok_or("transported storage guard budget exhausted")?;
            if container.len() >= fields.len()
                || previous_depth.is_some_and(|depth| depth >= container.len())
                || !container
                    .iter()
                    .copied()
                    .eq(fields.iter().take(container.len()).map(|(_, index)| *index))
            {
                return Err(
                    "transported storage guards do not select an ordered payload path".into(),
                );
            }
            previous_depth = Some(container.len());
        }
        let mut path = Vec::with_capacity(length);
        let mut conditions = guards.iter().peekable();
        for (depth, (_, index)) in fields.iter().enumerate() {
            if conditions
                .peek()
                .is_some_and(|(container, _)| container.len() == depth)
            {
                let (_, tag) = conditions.next().unwrap();
                path.extend([VARIANT_FIELD, *tag]);
            }
            path.push(*index);
        }
        let next = self.target.then(package, &path, fuel)?;
        if next.fields[self.target.fields.len()..] != *fields {
            return Err("storage projection field owner mismatch".into());
        }
        self.target = next;
        Ok(())
    }
}
impl fmt::Display for StorageTraversal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(storage-traversal (loads")?;
        for slot in &self.loads {
            write!(f, " (load {slot})")?;
        }
        write!(f, ") (target {}))", self.target)
    }
}

fn extend_storage_projection(
    package: Option<&Package>,
    result: &mut StorageProjection,
    path: &[usize],
    fuel: &mut usize,
) -> Result<(), String> {
    if result
        .fields
        .len()
        .saturating_add(result.guards.len().saturating_mul(2))
        .saturating_add(path.len())
        > 128
    {
        return Err("storage projection depth exhausted".into());
    }
    let mut cursor = 0;
    while cursor < path.len() {
        *fuel = fuel
            .checked_sub(result.fields.len() + 1)
            .ok_or("storage projection budget exhausted")?;
        if path[cursor] == VARIANT_FIELD {
            let tail = path
                .get(cursor + 1..cursor + 3)
                .ok_or("incomplete storage variant path")?;
            let owner = match &result.ty {
                Term::Nominal { declaration, .. } => {
                    nir::program::FieldOwner::Nominal(*declaration)
                }
                Term::ErrorResult { .. } | Term::Optional(_) => nir::program::FieldOwner::Anonymous,
                _ => return Err("storage variant owner has no variant layout".into()),
            };
            let selected = owned_variants(package, &result.ty, fuel)?
                .get(tail[0])
                .and_then(|variant| variant.fields.get(tail[1]))
                .cloned()
                .ok_or("storage variant field index out of bounds")?;
            result.guards.push((
                result.fields.iter().map(|(_, index)| *index).collect(),
                tail[0],
            ));
            result.fields.push((owner, tail[1]));
            result.ty = selected;
            cursor += 3;
        } else {
            let owner = if tuple_fields(package, &result.ty).is_some()
                || matches!(result.ty, Term::Record(_) | Term::Closure { .. })
            {
                nir::program::FieldOwner::Anonymous
            } else if let Term::Nominal { declaration, .. } = &result.ty {
                nir::program::FieldOwner::Nominal(*declaration)
            } else {
                return Err("storage field owner is not established".into());
            };
            result.ty = owned_path_type(package, &result.ty, &[path[cursor]], fuel)?;
            result.fields.push((owner, path[cursor]));
            cursor += 1;
        }
    }
    Ok(())
}

fn result_callable_bottom(
    package: Option<&Package>,
    ty: &Term,
    fuel: &mut usize,
) -> Result<BTreeMap<Vec<usize>, CallableResult>, String> {
    let ty = match ty {
        Term::Computation { result, .. } => result,
        ty => ty,
    };
    owned_callable_bottom(package, ty, fuel, false)
}
// Mutation tracking must also visit environment-bearing callable leaves. They
// cannot disappear just because the published named-result lattice omits them.
fn owned_callable_bottom(
    package: Option<&Package>,
    ty: &Term,
    fuel: &mut usize,
    include_closures: bool,
) -> Result<BTreeMap<Vec<usize>, CallableResult>, String> {
    bound_type(ty, fuel)?;
    let mut leaves = BTreeMap::new();
    let mut pending = vec![(vec![], ty.clone())];
    while let Some((path, ty)) = pending.pop() {
        *fuel = fuel
            .checked_sub(path.len() + 1)
            .ok_or("memory callable result layout budget exhausted")?;
        if path.len() >= 64 {
            return Err("memory callable result layout depth exhausted".into());
        }
        if matches!(ty, Term::Function { .. } | Term::UnsafeFunction(_))
            || include_closures && matches!(ty, Term::Closure { .. })
        {
            leaves.insert(path, CallableResult::NoReturn);
            continue;
        }
        if matches!(&ty, Term::ErrorResult { .. } | Term::Optional(_))
            || matches!(&ty,Term::Nominal {declaration,..} if package.is_some_and(|p|p.enums.iter().any(|e|e.declaration==*declaration)))
        {
            for (variant, definition) in owned_variants(package, &ty, fuel)?.into_iter().enumerate()
            {
                for (index, field) in definition.fields.into_iter().enumerate() {
                    let mut child = path.clone();
                    child.extend([VARIANT_FIELD, variant, index]);
                    pending.push((child, field));
                }
            }
            continue;
        }
        let structured = tuple_fields(package, &ty).is_some()
            || matches!(ty, Term::Record(_))
            || matches!(&ty,Term::Nominal {declaration,..}
            if package.is_some_and(|p|p.structs.iter().any(|s|s.declaration==*declaration)
                || p.newtypes.iter().any(|n|n.declaration==*declaration)));
        if structured {
            for (index, field) in owned_fields(package, &ty, fuel)?.into_iter().enumerate() {
                let mut child = path.clone();
                child.push(index);
                pending.push((child, field));
            }
        }
    }
    Ok(leaves)
}
fn charge_callee_fields(
    fields: &BTreeMap<Vec<usize>, CallableResult>,
    fuel: &mut usize,
) -> Result<(), String> {
    for (path, key) in fields {
        *fuel = fuel
            .checked_sub(path.len() + 1)
            .ok_or("memory callable result evidence budget exhausted")?;
        key.charge(fuel)?;
    }
    Ok(())
}
type Targets = Option<BTreeSet<PointerPlace>>;
type PointerEnvironment = BTreeMap<LocalId, Targets>;
type LocalCodeFields = BTreeMap<Vec<usize>, BTreeSet<usize>>;
#[derive(Clone, Default, PartialEq, Eq)]
struct Environment {
    pointers: PointerEnvironment,
    field_pointers: BTreeMap<CallablePlace, Targets>,
    local_candidates: BTreeMap<CallablePlace, BTreeSet<usize>>,
    callees: BTreeMap<CallablePlace, CallableResult>,
}

// Invocation-local evidence; unlike a named Summary it has no declaration identity.
struct CallFootprint {
    accesses: Vec<Access>,
    may_reject: bool,
    may_run_cleanup: bool,
    result_targets: Option<BTreeSet<ParameterPlace>>,
    pointer_fields: BTreeMap<Vec<usize>, Option<BTreeSet<ParameterPlace>>>,
    callable_result: CallableResult,
    callable_fields: BTreeMap<Vec<usize>, CallableResult>,
    code_fields: CodeFields,
}
impl From<Summary> for CallFootprint {
    fn from(value: Summary) -> Self {
        Self {
            accesses: value.accesses,
            may_reject: value.may_reject,
            may_run_cleanup: value.may_run_cleanup,
            result_targets: value.result_targets,
            pointer_fields: value.pointer_fields,
            callable_result: value.callable_result,
            callable_fields: value.callable_fields,
            code_fields: value.code_fields,
        }
    }
}

/// Result evidence distinguishes recursive bottom from an opaque runtime value.
/// A Known empty set is absence at a validated variant-guarded callable leaf.
#[derive(Clone, Debug, PartialEq)]
pub enum CodeResult {
    NoReturn,
    Known(Vec<PreparedCode>),
    Unknown,
}
pub type CodeFields = BTreeMap<Vec<usize>, CodeResult>;
impl CodeResult {
    fn charge(&self, fuel: &mut usize) -> Result<(), String> {
        *fuel = fuel.checked_sub(1).ok_or("code result budget exhausted")?;
        if let Self::Known(values) = self {
            charge_prepared_codes(values, fuel, 0)?;
        }
        Ok(())
    }
    fn join(&mut self, other: &Self, fuel: &mut usize) -> Result<(), String> {
        self.charge(fuel)?;
        other.charge(fuel)?;
        match (&mut *self, other) {
            (_, Self::NoReturn) | (Self::Unknown, _) => (),
            (slot @ Self::NoReturn, value) => *slot = value.clone(),
            (Self::Known(left), Self::Known(right)) => {
                join_prepared_codes(left, right.clone(), fuel)?;
            }
            (slot, Self::Unknown) => *slot = Self::Unknown,
        }
        Ok(())
    }
}
impl fmt::Display for CodeResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoReturn => f.write_str("(code-result no-return)"),
            Self::Unknown => f.write_str("(code-result unknown)"),
            Self::Known(values) if values.is_empty() => f.write_str("(code-result absent-variant)"),
            Self::Known(values) => {
                f.write_str("(code-result (candidates")?;
                for value in values {
                    write!(f, " {value}")?;
                }
                f.write_str("))")
            }
        }
    }
}
pub(crate) fn code_result_bottom(
    package: Option<&Package>,
    ty: &Term,
    fuel: &mut usize,
) -> Result<CodeFields, String> {
    let ty = match ty {
        Term::Computation { result, .. } => result,
        ty => ty,
    };
    Ok(owned_callable_bottom(package, ty, fuel, true)?
        .into_keys()
        .map(|path| (path, CodeResult::NoReturn))
        .collect())
}
fn display_code_fields(fields: &CodeFields, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.write_str(" (code-fields")?;
    for (path, value) in fields {
        f.write_str(" (leaf (path")?;
        display_owned_path(path, f)?;
        write!(f, ") {value})")?;
    }
    f.write_str(")")
}
pub(crate) fn join_code_fields(
    left: &mut CodeFields,
    right: &CodeFields,
    fuel: &mut usize,
) -> Result<(), String> {
    for (path, value) in left.iter_mut() {
        *fuel = fuel
            .checked_sub(path.len() + 1)
            .ok_or("code result field budget exhausted")?;
        value.join(right.get(path).unwrap_or(&CodeResult::Unknown), fuel)?;
    }
    for path in right.keys() {
        *fuel = fuel
            .checked_sub(path.len() + 1)
            .ok_or("code result field budget exhausted")?;
        left.entry(path.clone()).or_insert(CodeResult::Unknown);
    }
    Ok(())
}

/// Complete code evidence for one prepared argument candidate. The caller must
/// establish that this definition and capture snapshot belong to the value.
#[derive(Clone)]
pub enum CodeDefinition<'a> {
    Closure {
        definition: &'a Expr,
        captures: std::sync::Arc<BTreeMap<CallablePlace, CallableResult>>,
        prepared_captures: std::sync::Arc<PreparedCaptures>,
    },
    Named(iir::InstanceKey),
}

type LocalCode = PreparedCode;
/// A supplied empty set asserts absence of a validated variant-guarded code
/// leaf. The caller must establish the variant fact; omit unknown evidence.
pub type CodeInputs<'a> = BTreeMap<CallableInput, Vec<CodeDefinition<'a>>>;

/// Owned evidence retained by an interprocedural summary. Captures describe
/// code identities, not ownership or validity of memory reached by invocation.
#[derive(Clone, Debug, PartialEq)]
pub enum PreparedCode {
    Closure {
        definition: std::sync::Arc<Expr>,
        captures: std::sync::Arc<BTreeMap<CallablePlace, CallableResult>>,
        prepared_captures: std::sync::Arc<PreparedCaptures>,
    },
    Named(iir::InstanceKey),
}
/// Conditional capture evidence. An empty candidate set is permitted only at
/// a validated variant-guarded callable leaf and asserts that leaf is absent.
/// Producers must establish this from construction or checked enum domains;
/// missing/unknown provenance is represented by a missing entry instead.
pub type PreparedCaptures = BTreeMap<CallablePlace, Vec<PreparedCode>>;
pub type PreparedCodeInputs = BTreeMap<CallableInput, Vec<PreparedCode>>;
impl PreparedCode {
    fn borrowed(&self) -> CodeDefinition<'_> {
        match self {
            Self::Closure {
                definition,
                captures,
                prepared_captures,
            } => CodeDefinition::Closure {
                definition,
                captures: captures.clone(),
                prepared_captures: prepared_captures.clone(),
            },
            Self::Named(key) => CodeDefinition::Named(key.clone()),
        }
    }
    fn charge(&self, fuel: &mut usize) -> Result<(), String> {
        charge_prepared_codes(std::slice::from_ref(self), fuel, 0)
    }
}
fn join_prepared_codes(
    previous: &mut Vec<PreparedCode>,
    values: Vec<PreparedCode>,
    fuel: &mut usize,
) -> Result<bool, String> {
    let mut changed = false;
    for value in values {
        value.charge(fuel)?;
        let mut present = false;
        for old in previous.iter() {
            old.charge(fuel)?;
            if old == &value {
                present = true;
                break;
            }
        }
        if !present {
            previous.push(value);
            changed = true;
        }
    }
    Ok(changed)
}
fn charge_prepared_captures(
    captures: &PreparedCaptures,
    fuel: &mut usize,
    depth: usize,
) -> Result<(), String> {
    for (place, codes) in captures {
        *fuel = fuel
            .checked_sub(place.fields.len() + 1)
            .ok_or("prepared capture budget exhausted")?;
        charge_prepared_codes(codes, fuel, depth + 1)?;
    }
    Ok(())
}
fn charge_prepared_codes(
    codes: &[PreparedCode],
    fuel: &mut usize,
    depth: usize,
) -> Result<(), String> {
    if depth >= 64 {
        return Err("prepared capture depth exhausted".into());
    }
    for code in codes {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("prepared code budget exhausted")?;
        match code {
            PreparedCode::Named(key) => {
                for ty in &key.arguments {
                    bound_type(ty, fuel)?;
                }
            }
            PreparedCode::Closure {
                definition,
                captures,
                prepared_captures,
            } => {
                nir::visit::expression(definition, fuel, |expr, fuel| bound_type(&expr.ty, fuel))?;
                for (place, key) in captures.as_ref() {
                    *fuel = fuel
                        .checked_sub(place.fields.len() + 1)
                        .ok_or("prepared capture budget exhausted")?;
                    key.charge(fuel)?;
                }
                charge_prepared_captures(prepared_captures, fuel, depth)?;
            }
        }
    }
    Ok(())
}
impl fmt::Display for PreparedCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.borrowed().fmt(f)
    }
}
pub(crate) fn own_code_inputs(
    inputs: &CodeInputs<'_>,
    parameters: &[DeclarationId],
    arguments: &[Term],
    fuel: &mut usize,
) -> Result<PreparedCodeInputs, String> {
    let mut result = BTreeMap::new();
    for (input, definitions) in inputs {
        *fuel = fuel
            .checked_sub(input.fields.len() + 1)
            .ok_or("prepared input budget exhausted")?;
        let mut values = vec![];
        for definition in definitions {
            definition.charge(fuel)?;
            values.push(match definition {
                CodeDefinition::Named(key) => PreparedCode::Named(key.clone()),
                CodeDefinition::Closure {
                    definition,
                    captures,
                    prepared_captures,
                } => {
                    nir::visit::expression(definition, fuel, |expr, fuel| {
                        bound_type(&expr.ty, fuel)
                    })?;
                    PreparedCode::Closure {
                        definition: std::sync::Arc::new(
                            definition
                                .instantiate_types(parameters, arguments)
                                .map_err(|e| e.to_string())?,
                        ),
                        captures: captures.clone(),
                        prepared_captures: prepared_captures.clone(),
                    }
                }
            });
        }
        result.insert(input.clone(), values);
    }
    Ok(result)
}
fn borrow_code_inputs(inputs: &PreparedCodeInputs) -> CodeInputs<'_> {
    inputs
        .iter()
        .map(|(input, values)| {
            (
                input.clone(),
                values.iter().map(PreparedCode::borrowed).collect(),
            )
        })
        .collect()
}

impl CodeDefinition<'_> {
    fn charge(&self, fuel: &mut usize) -> Result<(), String> {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("code argument evidence budget exhausted")?;
        match self {
            Self::Named(key) => {
                for argument in &key.arguments {
                    bound_type(argument, fuel)?;
                }
            }
            Self::Closure {
                definition,
                captures,
                prepared_captures,
            } => {
                bound_type(&definition.ty, fuel)?;
                charge_prepared_captures(prepared_captures, fuel, 0)?;
                for (place, key) in captures.as_ref() {
                    *fuel = fuel
                        .checked_sub(place.fields.len() + 1)
                        .ok_or("code argument capture budget exhausted")?;
                    key.charge(fuel)?;
                }
            }
        }
        Ok(())
    }
}
impl fmt::Display for CodeDefinition<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Named(key) => write!(f, "(named-code {key})"),
            Self::Closure {
                definition,
                captures,
                prepared_captures,
            } => {
                write!(f, "(closure-code (definition {definition}) (captures")?;
                for (place, key) in captures.as_ref() {
                    write!(f, " (capture {place} {key})")?;
                }
                f.write_str(") (prepared-captures")?;
                for (place, values) in prepared_captures.as_ref() {
                    if values.is_empty() {
                        write!(f, " (capture {place} (absent-variant))")?;
                    } else {
                        write!(f, " (capture {place} (candidates")?;
                        for value in values {
                            write!(f, " {value}")?;
                        }
                        f.write_str("))")?;
                    }
                }
                f.write_str("))")
            }
        }
    }
}

type BorrowedCaptures = BTreeMap<LocalId, BTreeMap<Vec<usize>, Targets>>;

fn charge_pointer_captures(values: &BorrowedCaptures, fuel: &mut usize) -> Result<(), String> {
    for fields in values.values() {
        for (path, targets) in fields {
            *fuel = fuel
                .checked_sub(path.len() + 1)
                .ok_or("closure pointer snapshot budget exhausted")?;
            if let Some(targets) = targets {
                for target in targets {
                    *fuel = fuel
                        .checked_sub(target.path_cost())
                        .ok_or("closure pointer snapshot budget exhausted")?;
                }
            }
        }
    }
    Ok(())
}
struct Infer<'a, 'cleanup> {
    // Complete current runtime scope for stable handler code-capture checks.
    scope: &'a Expr,
    borrowed_callable_slots: BTreeMap<PointerPlace, CallableResult>,
    local_code: Vec<LocalCode>,
    // Invocation-local origins never escape in reusable PreparedCode evidence.
    borrowed_code_pointers: BTreeMap<usize, std::sync::Arc<BorrowedCaptures>>,
    stable_capture_roots: BTreeSet<LocalId>,
    capture_code_writes: PreparedCaptures,
    named_code_slots: HashMap<iir::InstanceKey, usize>,
    // Source-node addresses are lookup keys only. Candidate ordering uses
    // deterministic first-visit ordinals, never allocator addresses.
    closure_nodes: HashMap<usize, Vec<usize>>,
    local_code_results: HashMap<usize, Option<BTreeSet<usize>>>,
    local_code_fields: HashMap<usize, LocalCodeFields>,
    type_context: crate::ptc::Context,
    type_views: HashMap<Term, Term>,
    fuel: &'a mut usize,
    package: Option<&'a Package>,
    parameters: &'a [DeclarationId],
    declarations: Vec<nir::DeclarationParameter>,
    arguments: &'a [Term],
    round: &'a mut Round<'cleanup>,
    effect_row: Vec<nir::effects::Operation>,
    environment: Environment,
    locals: BTreeMap<LocalId, Term>,
    // Local bindings whose storage is neither addressed nor captured.
    private_storage_roots: BTreeSet<LocalId>,
    // Stable layout evidence survives temporary default-parameter environments.
    local_storage: Vec<(LocalId, Term)>,
    loop_states: Vec<Environment>,
    accesses: Vec<Access>,
    may_reject: bool,
    may_run_cleanup: bool,
    // Ephemeral node identity within this immutable body traversal only.
    expression_results: HashMap<usize, Targets>,
    callable_results: HashMap<usize, CallableResult>,
    aggregate_callees: HashMap<usize, BTreeMap<Vec<usize>, CallableResult>>,
    aggregate_pointers: HashMap<usize, BTreeMap<Vec<usize>, Targets>>,
    returns: Option<BTreeSet<ParameterPlace>>,
    pointer_field_returns: Option<BTreeMap<Vec<usize>, Option<BTreeSet<ParameterPlace>>>>,
    // Formal input cells are local to an invocation. Only lifted capture slots
    // can later be interpreted as environment writeback by the caller.
    input_pointer_slots: BTreeMap<CallablePlace, CallableInput>,
    input_code_slots: BTreeMap<CallablePlace, CallableInput>,
    input_code_exits: BTreeMap<CallableInput, Option<Vec<PreparedCode>>>,
    input_pointer_exits: BTreeMap<CallableInput, Option<BTreeSet<ParameterPlace>>>,
    callable_returns: CallableResult,
    callable_field_returns: BTreeMap<Vec<usize>, CallableResult>,
    code_field_returns: CodeFields,
}
/// Layout-only prerequisite; ownership and destructor contracts remain separate.
/// Private representation depth and pointee for a pointer or a newtype chain.
/// This is storage metadata, never a source projection or a type equality.
pub fn pointer_representation(
    package: Option<&Package>,
    ty: &Term,
    fuel: &mut usize,
) -> Result<Option<(usize, Term)>, String> {
    bound_type(ty, fuel)?;
    let mut ty = ty.clone();
    for depth in 0..128 {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("pointer representation budget exhausted")?;
        match ty {
            Term::Pointer(subject) => return Ok(Some((depth, *subject))),
            Term::Nominal {
                declaration,
                arguments,
            } => {
                let Some(definition) = package.and_then(|package| {
                    package
                        .newtypes
                        .iter()
                        .find(|definition| definition.declaration == declaration)
                }) else {
                    return Ok(None);
                };
                ty = definition
                    .representation_budgeted(&arguments, &[], fuel)
                    .map_err(|error| error.to_string())?;
            }
            _ => return Ok(None),
        }
    }
    Err("pointer representation depth exhausted".into())
}

/// Copy-readable storage layout; no ownership permission is implied. Aggregates
/// may contain data and environment-free code leaves. Data-pointer origins are
/// supported for a single pointer under newtypes, not pointer-bearing aggregates.
pub(crate) fn readable_representation(
    package: Option<&Package>,
    ty: &Term,
    fuel: &mut usize,
) -> Result<bool, String> {
    storage_representation(package, ty, fuel, true, true, false)
}

/// Whether captured storage can be replaced using pointer-leaf exit evidence.
/// Callable leaves are checked by capture-code invariants separately; variant-
/// dependent pointer slots still need a representable input-pointer layout.
pub(crate) fn capture_pointer_replacement_layout(
    package: Option<&Package>,
    ty: &Term,
    fuel: &mut usize,
) -> Result<bool, String> {
    symbolic_pointer_fields(package, ty, fuel)?;
    Ok(true)
}

/// Data-only replacement leaves pointer provenance and callable identity intact.
/// Copy and cleanup legality are separate obligations of the transfer analysis.
pub(crate) fn data_representation(
    package: Option<&Package>,
    ty: &Term,
    fuel: &mut usize,
) -> Result<bool, String> {
    storage_representation(package, ty, fuel, false, false, false)
}

pub(crate) fn copy_readable_representation(
    package: Option<&Package>,
    ty: &Term,
    fuel: &mut usize,
) -> Result<bool, String> {
    storage_representation(package, ty, fuel, true, true, true)
}

fn storage_representation(
    package: Option<&Package>,
    ty: &Term,
    fuel: &mut usize,
    pointer: bool,
    code: bool,
    nested_pointers: bool,
) -> Result<bool, String> {
    fn walk(
        package: Option<&Package>,
        ty: &Term,
        fuel: &mut usize,
        depth: usize,
        pointer: bool,
        code: bool,
        nested_pointers: bool,
    ) -> Result<bool, String> {
        if depth >= 128 {
            return Err("read representation depth exhausted".into());
        }
        bound_type(ty, fuel)?;
        if intrinsic_pointer_value(ty) {
            return Ok(pointer || !matches!(ty, Term::Pointer(_)));
        }
        if let Some(fields) = tuple_fields(package, ty) {
            for field in fields {
                if !walk(
                    package,
                    field,
                    fuel,
                    depth + 1,
                    pointer && nested_pointers,
                    code,
                    nested_pointers,
                )? {
                    return Ok(false);
                }
            }
            return Ok(true);
        }
        if matches!(ty, Term::Function { .. } | Term::UnsafeFunction(_)) {
            return Ok(code);
        }
        let fields = match ty {
            Term::Optional(payload) => vec![(**payload).clone()],
            Term::ErrorResult { errors, result } => {
                let mut fields = errors.clone();
                if !matches!(result.as_ref(), Term::Primitive(nir::Primitive::Never)) {
                    fields.push((**result).clone());
                }
                fields
            }
            Term::Record(fields) => fields.iter().map(|(_, ty)| ty.clone()).collect::<Vec<_>>(),
            Term::Nominal {
                declaration,
                arguments,
            } => {
                let Some(package) = package else {
                    return Ok(false);
                };
                if let Some(definition) = package
                    .newtypes
                    .iter()
                    .find(|d| d.declaration == *declaration)
                {
                    let value = definition
                        .representation_budgeted(arguments, &[], fuel)
                        .map_err(|e| e.to_string())?;
                    return walk(
                        Some(package),
                        &value,
                        fuel,
                        depth + 1,
                        pointer,
                        code,
                        nested_pointers,
                    );
                }
                if let Some(definition) = package
                    .structs
                    .iter()
                    .find(|d| d.declaration == *declaration)
                {
                    definition
                        .instantiate_fields(arguments)
                        .map_err(|e| e.to_string())?
                        .into_iter()
                        .map(|(_, ty)| ty)
                        .collect()
                } else if let Some(definition) =
                    package.enums.iter().find(|d| d.declaration == *declaration)
                {
                    definition
                        .instantiate_variants(arguments)
                        .map_err(|e| e.to_string())?
                        .into_iter()
                        .flat_map(|variant| variant.fields)
                        .collect()
                } else {
                    return Ok(false);
                }
            }
            _ => return Ok(false),
        };
        for field in fields {
            if !walk(
                package,
                &field,
                fuel,
                depth + 1,
                pointer && nested_pointers,
                code,
                nested_pointers,
            )? {
                return Ok(false);
            }
        }
        Ok(true)
    }
    walk(package, ty, fuel, 0, pointer, code, nested_pointers)
}

pub(crate) fn require_pointer_free_owned(
    package: Option<&Package>,
    ty: &Term,
    fuel: &mut usize,
    depth: usize,
) -> Result<(), String> {
    if *fuel == 0 || depth >= 128 {
        return Err("pointer-free layout budget exhausted".into());
    }
    *fuel -= 1;
    bound_type(ty, fuel)?;
    // Environment-free code identities do not store data-pointer aliases.
    // Their parameter/result types describe invocation, not stored dependencies.
    if matches!(
        ty,
        Term::Function { .. }
            | Term::UnsafeFunction(_)
            | Term::Primitive(Primitive::Void | Primitive::Never)
    ) || intrinsic_pointer_value(ty) && !matches!(ty, Term::Pointer(_))
    {
        return Ok(());
    }
    let fields = if let Some(fields) = tuple_fields(package, ty) {
        fields.to_vec()
    } else {
        match ty {
            Term::Closure {
                environment: fields,
                ..
            } => fields.clone(),
            Term::Record(fields) => fields.iter().map(|(_, ty)| ty.clone()).collect(),
            Term::Optional(payload) => vec![(**payload).clone()],
            Term::ErrorResult { errors, result } => {
                let mut fields = errors.clone();
                fields.push((**result).clone());
                fields
            }
            Term::Nominal {
                declaration,
                arguments,
            } => {
                let package = package.ok_or("aggregate footprint requires a package")?;
                if let Some(definition) = package
                    .structs
                    .iter()
                    .find(|d| d.declaration == *declaration)
                {
                    definition
                        .instantiate_fields(arguments)
                        .map_err(|e| e.to_string())?
                        .into_iter()
                        .map(|(_, ty)| ty)
                        .collect()
                } else if let Some(definition) =
                    package.enums.iter().find(|d| d.declaration == *declaration)
                {
                    definition
                        .instantiate_variants(arguments)
                        .map_err(|e| e.to_string())?
                        .into_iter()
                        .flat_map(|variant| variant.fields)
                        .collect()
                } else if let Some(definition) = package
                    .newtypes
                    .iter()
                    .find(|definition| definition.declaration == *declaration)
                {
                    vec![
                        definition
                            .representation_budgeted(arguments, &[], fuel)
                            .map_err(|error| error.to_string())?,
                    ]
                } else {
                    return Err("aggregate footprint requires a known pointer-free layout".into());
                }
            }
            _ => {
                return Err(
                    "aggregate pointer dependencies require a richer memory contract".into(),
                );
            }
        }
    };
    for field in fields {
        require_pointer_free_owned(package, &field, fuel, depth + 1)?;
    }
    Ok(())
}

impl<'a> Infer<'a, '_> {
    fn install_input_pointer_fields(
        &mut self,
        binding: LocalId,
        parameter: usize,
        ty: &Term,
    ) -> Result<(), String> {
        let view = self.type_view(ty)?;
        for (fields, _) in symbolic_pointer_fields(self.package, &view, self.fuel)? {
            if fields.is_empty() {
                continue;
            }
            self.environment.field_pointers.insert(
                CallablePlace {
                    binding,
                    fields: fields.clone(),
                },
                Some(BTreeSet::from([PointerPlace {
                    root: PointerRoot::Parameter(parameter),
                    input_fields: fields,
                    through: vec![],
                    conditions: BTreeMap::new(),
                    fields: vec![],
                }])),
            );
        }
        Ok(())
    }

    fn pointer_fields_for(&mut self, expr: &Expr) -> Result<BTreeMap<Vec<usize>, Targets>, String> {
        let mut expr = expr;
        loop {
            self.step(0)?;
            if let Some(fields) = self.aggregate_pointers.get(&(expr as *const Expr as usize)) {
                for (path, targets) in fields {
                    *self.fuel = self
                        .fuel
                        .checked_sub(path.len() + 1)
                        .ok_or("aggregate pointer snapshot budget exhausted")?;
                    if let Some(targets) = targets {
                        for target in targets {
                            *self.fuel = self
                                .fuel
                                .checked_sub(target.path_cost())
                                .ok_or("aggregate pointer snapshot budget exhausted")?;
                        }
                    }
                }
                return Ok(fields.clone());
            }
            match &expr.kind {
                K::UnsafeScope(body) => expr = body,
                K::Block {
                    result: Some(result),
                    ..
                }
                | K::CallPreparation {
                    result: Some(result),
                    ..
                } => expr = result,
                _ => break,
            }
        }
        // A stored closure may have been invoked since its construction. Its
        // current candidates own the authoritative capture snapshots; the
        // ordinary field map still describes the originally delivered value.
        if matches!(expr.ty, Term::Closure { .. }) && self.callable_place(expr)?.is_some() {
            if let Some(ids) = self.local_candidates_for(expr)? {
                let mut current = None;
                let mut missing = false;
                for id in ids {
                    self.step(0)?;
                    let Some(pointers) = self.borrowed_code_pointers.get(&id).cloned() else {
                        missing = true;
                        continue;
                    };
                    charge_pointer_captures(&pointers, self.fuel)?;
                    let Some(LocalCode::Closure { definition, .. }) = self.local_code.get(id)
                    else {
                        return Err("capture snapshot lacks closure code".into());
                    };
                    let K::Closure { captures, .. } = &definition.kind else {
                        return Err("capture snapshot lacks closure definition".into());
                    };
                    let mut fields = BTreeMap::new();
                    for (index, capture) in captures.iter().enumerate() {
                        if let Some(paths) = pointers.get(&capture.binding) {
                            for (path, targets) in paths {
                                *self.fuel = self
                                    .fuel
                                    .checked_sub(path.len() + 1)
                                    .ok_or("closure result snapshot budget exhausted")?;
                                let mut field = vec![index];
                                field.extend(path);
                                fields.insert(field, targets.clone());
                            }
                        }
                    }
                    current = Some(match current {
                        Some(previous) => self.join_pointer_fields(previous, fields)?,
                        None => fields,
                    });
                }
                if let Some(mut fields) = current {
                    if missing {
                        // A candidate without a local snapshot cannot borrow
                        // another candidate's post-invocation environment.
                        for targets in fields.values_mut() {
                            *targets = None;
                        }
                    }
                    return Ok(fields);
                }
            }
        }
        let mut output = BTreeMap::new();
        let Some(base) = self.callable_place(expr)? else {
            return Ok(output);
        };
        for (leaf, targets) in &self.environment.field_pointers {
            *self.fuel = self
                .fuel
                .checked_sub(leaf.fields.len() + 1)
                .ok_or("pointer field projection budget exhausted")?;
            if leaf.binding == base.binding && leaf.fields.starts_with(&base.fields) {
                if let Some(targets) = targets {
                    for target in targets {
                        *self.fuel = self
                            .fuel
                            .checked_sub(target.path_cost())
                            .ok_or("pointer field projection budget exhausted")?;
                    }
                }
                output.insert(leaf.fields[base.fields.len()..].to_vec(), targets.clone());
            }
        }
        Ok(output)
    }

    fn capture_pointer_fields(
        &mut self,
        capture: &nir::program::Capture,
        depth: usize,
    ) -> Result<BTreeMap<Vec<usize>, Targets>, String> {
        let ty = self.ty(&capture.ty)?;
        let paths = symbolic_pointer_fields(self.package, &ty, self.fuel)?;
        let mut fields = if let Some(value) = &capture.value {
            self.pointer_fields_for(value)?
        } else {
            self.pointer_fields_for(&Expr {
                ty: ty.clone(),
                source: nir::program::SourceRange { start: 0, end: 0 },
                kind: K::Local(capture.binding),
            })?
        };
        if self.local_pointer_value(&ty)? {
            let target = if let Some(value) = &capture.value {
                self.optional_target(value, depth + 1)?
            } else {
                let target = self.environment.pointers.get(&capture.binding);
                if let Some(Some(targets)) = target {
                    for target in targets {
                        *self.fuel = self
                            .fuel
                            .checked_sub(target.path_cost())
                            .ok_or("closure pointer snapshot budget exhausted")?;
                    }
                }
                target.cloned().flatten()
            };
            fields.insert(vec![], target);
        }
        for path in paths.keys() {
            fields.entry(path.clone()).or_insert(None);
        }
        Ok(fields)
    }

    fn replace_pointer_fields(
        &mut self,
        place: CallablePlace,
        values: BTreeMap<Vec<usize>, Targets>,
    ) -> Result<(), String> {
        for field in self.environment.field_pointers.keys() {
            *self.fuel = self
                .fuel
                .checked_sub(field.fields.len() + 1)
                .ok_or("pointer field replacement budget exhausted")?;
        }
        self.environment.field_pointers.retain(|field, _| {
            field.binding != place.binding || !field.fields.starts_with(&place.fields)
        });
        for (path, targets) in values {
            self.step(path.len() + place.fields.len())?;
            let mut fields = place.fields.clone();
            fields.extend(path);
            self.environment.field_pointers.insert(
                CallablePlace {
                    binding: place.binding,
                    fields,
                },
                targets,
            );
        }
        Ok(())
    }

    fn install_code_inputs(
        &mut self,
        bindings: &[LocalId],
        parameters: &[nir::Parameter],
        inputs: &BTreeMap<CallableInput, CallableResult>,
        code_inputs: &CodeInputs<'a>,
    ) -> Result<(), String> {
        for (input, definitions) in code_inputs {
            self.step(input.fields.len() + 1)?;
            if inputs.contains_key(input) {
                return Err("duplicate prepared code parameter premise".into());
            }
            if definitions.is_empty() && !input.fields.contains(&VARIANT_FIELD) {
                return Err(
                    "empty prepared code candidate set requires an absent variant leaf".into(),
                );
            }
            let parameter = parameters
                .get(input.parameter)
                .ok_or("prepared code parameter is missing")?;
            let mut expected =
                owned_path_type(self.package, &parameter.ty, &input.fields, self.fuel)?;
            if !matches!(
                expected,
                Term::Function { .. } | Term::UnsafeFunction(_) | Term::Closure { .. }
            ) {
                return Err("prepared code field is not callable".into());
            }
            let mut ids = BTreeSet::new();
            let mut named = CallableResult::NoReturn;
            let mut anonymous = false;
            for definition in definitions {
                definition.charge(self.fuel)?;
                match definition {
                    CodeDefinition::Named(key) => {
                        let candidate = CallableResult::Known(key.clone());
                        validate_code_premise(
                            self.package.ok_or("prepared code requires a package")?,
                            self.round,
                            &expected,
                            &candidate,
                            self.fuel,
                        )?;
                        named.join(&candidate, self.fuel)?;
                        ids.extend(self.intern_named_candidates(candidate)?.unwrap());
                    }
                    CodeDefinition::Closure {
                        definition,
                        captures,
                        prepared_captures,
                    } => {
                        anonymous = true;
                        let K::Closure {
                            parameters: closure_parameters,
                            captures: closure_captures,
                            ..
                        } = &definition.kind
                        else {
                            return Err(
                                "prepared closure evidence requires a closure definition".into()
                            );
                        };
                        let actual = self.ty(&definition.ty)?;
                        if actual != expected {
                            expected = self.completed_type_view(&expected)?;
                        }
                        if actual != expected {
                            return Err(
                                "prepared closure signature disagrees with parameter".into()
                            );
                        }
                        let environment = match &expected {
                            Term::Closure { environment, .. } => environment.as_slice(),
                            _ => &[],
                        };
                        let Term::Function {
                            parameters: signature_parameters,
                            ..
                        } = expected.callable_signature()
                        else {
                            unreachable!()
                        };
                        if environment.len() != closure_captures.len()
                            || signature_parameters.len() != closure_parameters.len()
                        {
                            return Err(
                                "prepared closure layout disagrees with its signature".into()
                            );
                        }
                        let mut bindings = BTreeSet::new();
                        for binding in closure_parameters {
                            self.step(0)?;
                            if !bindings.insert(*binding) {
                                return Err("prepared closure binding collision".into());
                            }
                        }
                        let mut pointer_captures = BTreeMap::new();
                        for (index, (capture, ty)) in
                            closure_captures.iter().zip(environment).enumerate()
                        {
                            self.step(0)?;
                            if self.ty(&capture.ty)? != *ty || !bindings.insert(capture.binding) {
                                return Err("prepared closure capture layout mismatch".into());
                            }
                            let mut fields = BTreeMap::new();
                            for (path, _) in symbolic_pointer_fields(self.package, ty, self.fuel)? {
                                self.step(input.fields.len() + path.len() + 1)?;
                                let mut input_fields = input.fields.clone();
                                input_fields.push(index);
                                input_fields.extend(&path);
                                fields.insert(
                                    path,
                                    Some(BTreeSet::from([PointerPlace {
                                        root: PointerRoot::Parameter(input.parameter),
                                        input_fields,
                                        through: vec![],
                                        conditions: BTreeMap::new(),
                                        fields: vec![],
                                    }])),
                                );
                            }
                            if !fields.is_empty() {
                                pointer_captures.insert(capture.binding, fields);
                            }
                        }
                        self.remember_closure(
                            definition,
                            captures.as_ref().clone(),
                            prepared_captures.clone(),
                            pointer_captures,
                        )?;
                        ids.extend(
                            self.local_candidates_for(definition)?
                                .ok_or("prepared closure identity is missing")?,
                        );
                    }
                }
            }
            let place = CallablePlace {
                binding: bindings[input.parameter],
                fields: input.fields.clone(),
            };
            self.environment.callees.insert(
                place.clone(),
                if anonymous {
                    CallableResult::Unknown
                } else {
                    named
                },
            );
            self.environment.local_candidates.insert(place, ids);
        }
        Ok(())
    }
    // Pointer provenance follows a single representation leaf while source
    // operations and nominal signatures keep their declared types.
    fn local_pointer_value(&mut self, ty: &Term) -> Result<bool, String> {
        let ty = self.ty(ty)?;
        Ok(pointer_representation(self.package, &ty, self.fuel)?.is_some())
    }
    fn pointer_free_owned(&mut self, ty: &Term, depth: usize) -> Result<(), String> {
        let ty = self.type_view(ty)?;
        require_pointer_free_owned(self.package, &ty, self.fuel, depth)
    }
    fn completed_type_view(&mut self, ty: &Term) -> Result<Term, String> {
        if let Some(value) = self.type_views.get(ty) {
            bound_type(value, self.fuel)?;
            return Ok(value.clone());
        }
        let package = self
            .package
            .ok_or("completed type view requires a package")?;
        // Complete nested associated views within the invocation's budget,
        // while reserving work for the remaining provenance checks.
        let allowance = *self.fuel / 2;
        *self.fuel -= allowance;
        let mut remaining = allowance;
        let value = crate::materialization::normalize_concrete_type(package, ty, &mut remaining);
        *self.fuel += remaining;
        let value = value?;
        self.type_views.insert(ty.clone(), value.clone());
        Ok(value)
    }

    fn type_view(&mut self, ty: &Term) -> Result<Term, String> {
        if !matches!(
            ty,
            Term::Bound { .. }
                | Term::Rigid { .. }
                | Term::AssociatedType { .. }
                | Term::NormalForm { .. }
                | Term::Hole { .. }
        ) {
            return Ok(ty.clone());
        }
        if let Some(view) = self.type_views.get(ty) {
            return Ok(view.clone());
        }
        let steps = (*self.fuel / 4).min(4096);
        if steps == 0 {
            return Err("memory type view budget exhausted".into());
        }
        *self.fuel -= steps;
        let view = self
            .type_context
            .structural_view(
                ty,
                crate::ptc::Budget {
                    steps,
                    ..Default::default()
                },
            )
            .ok()
            .flatten()
            .map(|(ty, _)| ty)
            // Scalar views cannot introduce pointer roots or callable leaves.
            // Those need corresponding dependency evidence, not just equality.
            .filter(|ty| matches!(ty, Term::Primitive(_)) && intrinsic_pointer_value(ty))
            .unwrap_or_else(|| ty.clone());
        self.type_views.insert(ty.clone(), view.clone());
        Ok(view)
    }
    // Observe immutable declaration data without executing user code or
    // unfolding normal-form heads. Admission precedes projection, so a field
    // cannot hide malformed siblings or a mismatched record domain.
    fn known_data(&mut self, expr: &Expr, depth: usize) -> Result<Option<Term>, String> {
        self.step(depth)?;
        match &expr.kind {
            K::ComptimeValue(value) => {
                let domain = self.ty(&expr.ty)?;
                let value = nir::type_computations::instantiate_parameters_bounded(
                    value,
                    &self.declarations,
                    self.arguments,
                    &[],
                    self.fuel,
                )
                .map_err(|e| e.to_string())?;
                nir::comptime_values::validate(&value, &domain, self.fuel)
                    .map_err(|e| e.to_string())?;
                if !value.is_structural_index_value() {
                    return Ok(None);
                }
                // This singleton binder is local to admission; it is never
                // inserted into the package or used to identify a declaration.
                nir::validate_declaration_arguments(
                    &[DeclarationId {
                        package: 0,
                        local: 0,
                    }],
                    &[domain],
                    std::slice::from_ref(&value),
                    self.fuel,
                )
                .map_err(|e| e.to_string())?;
                Ok(Some(value))
            }
            K::AnonymousField { value, index, .. } => {
                let Some(Term::RecordValue(fields)) = self.known_data(value, depth + 1)? else {
                    return Ok(None);
                };
                let Term::Record(domains) = self.ty(&value.ty)? else {
                    return Err("captured record projection requires a record domain".into());
                };
                let Some((_, domain)) = domains.get(*index) else {
                    return Err("captured record projection is out of bounds".into());
                };
                if *domain != self.ty(&expr.ty)? {
                    return Err("captured record projection type mismatch".into());
                }
                Ok(fields.into_iter().nth(*index).map(|(_, value)| value))
            }
            _ => Ok(None),
        }
    }
    fn captured_data_domain(&mut self, ty: &Term, depth: usize) -> Result<bool, String> {
        self.step(depth)?;
        match ty {
            Term::Primitive(_) => Ok(true),
            Term::Record(fields) => {
                for (_, field) in fields {
                    if !self.captured_data_domain(field, depth + 1)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }
    fn captured_data(&mut self, expr: &Expr, depth: usize) -> Result<(), String> {
        let domain = self.ty(&expr.ty)?;
        if !self.captured_data_domain(&domain, depth)? {
            return Err("captured value requires a richer memory contract".into());
        }
        self.known_data(expr, depth)?;
        Ok(())
    }
    fn known_bool(&mut self, expr: &Expr, depth: usize) -> Result<Option<bool>, String> {
        self.step(depth)?;
        Ok(match &expr.kind {
            K::ComptimeValue(_) | K::AnonymousField { .. } => {
                match self.known_data(expr, depth + 1)? {
                    Some(Term::Bool(value)) => Some(value),
                    _ => None,
                }
            }
            K::Not(value) => self.known_bool(value, depth + 1)?.map(|v| !v),
            K::And(left, right) => match self.known_bool(left, depth + 1)? {
                Some(false) => Some(false),
                Some(true) => self.known_bool(right, depth + 1)?,
                None => None,
            },
            K::Or(left, right) => match self.known_bool(left, depth + 1)? {
                Some(true) => Some(true),
                Some(false) => self.known_bool(right, depth + 1)?,
                None => None,
            },
            _ => constant_bool(expr, depth),
        })
    }
    fn ty(&mut self, ty: &Term) -> Result<Term, String> {
        self.step(0)?;
        bound_type(ty, self.fuel)?;
        let ty = ty
            .instantiate_parameters(&self.declarations, self.arguments)
            .map_err(|error| error.to_string())?;
        bound_type(&ty, self.fuel)?;
        self.type_view(&ty)
    }
    fn step(&mut self, depth: usize) -> Result<(), String> {
        if *self.fuel == 0 || depth > 128 {
            return Err("memory summary budget exhausted".into());
        }
        *self.fuel -= 1;
        Ok(())
    }
    fn target_work<T: TargetCost>(&mut self, targets: &BTreeSet<T>) -> Result<(), String> {
        for target in targets {
            *self.fuel = self
                .fuel
                .checked_sub(target.target_cost())
                .ok_or("memory target set budget exhausted")?;
        }
        Ok(())
    }
    fn join_targets<T: TargetCost + Ord>(
        &mut self,
        left: Option<BTreeSet<T>>,
        right: Option<BTreeSet<T>>,
    ) -> Result<Option<BTreeSet<T>>, String> {
        match (left, right) {
            (Some(mut left), Some(right)) => {
                self.target_work(&right)?;
                left.extend(right);
                Ok(Some(left))
            }
            _ => Ok(None),
        }
    }
    fn join_pointer_fields<T: TargetCost + Ord>(
        &mut self,
        left: BTreeMap<Vec<usize>, Option<BTreeSet<T>>>,
        mut right: BTreeMap<Vec<usize>, Option<BTreeSet<T>>>,
    ) -> Result<BTreeMap<Vec<usize>, Option<BTreeSet<T>>>, String> {
        let mut joined = BTreeMap::new();
        for (path, targets) in left {
            self.step(path.len())?;
            let other = right.remove(&path).unwrap_or(None);
            joined.insert(path, self.join_targets(targets, other)?);
        }
        for (path, _) in right {
            self.step(path.len())?;
            // Absent evidence on either completing branch is unknown.
            joined.insert(path, None);
        }
        Ok(joined)
    }
    fn clone_pointer_environment(
        &mut self,
        environment: &PointerEnvironment,
    ) -> Result<PointerEnvironment, String> {
        for targets in environment.values() {
            self.step(0)?;
            if let Some(targets) = targets {
                self.target_work(targets)?;
            }
        }
        Ok(environment.clone())
    }
    fn clone_environment(&mut self, environment: &Environment) -> Result<Environment, String> {
        let pointers = self.clone_pointer_environment(&environment.pointers)?;
        for (place, targets) in &environment.field_pointers {
            self.step(place.fields.len())?;
            if let Some(targets) = targets {
                self.target_work(targets)?;
            }
        }
        for (place, keys) in &environment.local_candidates {
            *self.fuel = self
                .fuel
                .checked_sub(place.fields.len() + keys.len() + 1)
                .ok_or("closure memory identity budget exhausted")?;
        }
        for key in environment.callees.values() {
            self.step(0)?;
            key.charge(self.fuel)?;
        }
        Ok(Environment {
            pointers,
            field_pointers: environment.field_pointers.clone(),
            local_candidates: environment.local_candidates.clone(),
            callees: environment.callees.clone(),
        })
    }
    fn save_environment(&mut self) -> Result<Environment, String> {
        let environment = std::mem::take(&mut self.environment);
        let copy = self.clone_environment(&environment);
        self.environment = environment;
        copy
    }
    fn join_environment(
        &mut self,
        mut left: Environment,
        mut right: Environment,
    ) -> Result<Environment, String> {
        let mut fields = BTreeSet::new();
        for field in left
            .field_pointers
            .keys()
            .chain(right.field_pointers.keys())
        {
            self.step(field.fields.len())?;
            fields.insert(field.clone());
        }
        for field in fields {
            self.step(field.fields.len())?;
            let a = left.field_pointers.remove(&field).unwrap_or(None);
            let b = right.field_pointers.remove(&field).unwrap_or(None);
            left.field_pointers.insert(field, self.join_targets(a, b)?);
        }
        for place in left.local_candidates.keys() {
            *self.fuel = self
                .fuel
                .checked_sub(place.fields.len() + 1)
                .ok_or("closure memory identity budget exhausted")?;
        }
        for binding in left.local_candidates.keys().cloned().collect::<Vec<_>>() {
            if let Some(other) = right.local_candidates.get(&binding) {
                *self.fuel = self
                    .fuel
                    .checked_sub(other.len())
                    .ok_or("closure memory identity budget exhausted")?;
                left.local_candidates
                    .get_mut(&binding)
                    .unwrap()
                    .extend(other);
            } else {
                left.local_candidates.remove(&binding);
            }
        }
        for (binding, targets) in right.pointers {
            self.step(0)?;
            let targets = if let Some(previous) = left.pointers.remove(&binding) {
                self.join_targets(previous, targets)?
            } else {
                targets
            };
            left.pointers.insert(binding, targets);
        }
        // Keep every known continuing target; missing identity remains opaque.
        for (binding, key) in &mut left.callees {
            self.step(0)?;
            key.join(
                right
                    .callees
                    .get(binding)
                    .unwrap_or(&CallableResult::Unknown),
                self.fuel,
            )?;
        }
        for (binding, _) in right.callees {
            self.step(0)?;
            left.callees
                .entry(binding)
                .or_insert(CallableResult::Unknown);
        }
        Ok(left)
    }
    fn local_candidates_for(&mut self, expr: &Expr) -> Result<Option<BTreeSet<usize>>, String> {
        self.step(0)?;
        if let K::UnsafeScope(body) = &expr.kind {
            return self.local_candidates_for(body);
        }
        let place = self.callable_place(expr)?;
        let keys = place
            .as_ref()
            .and_then(|place| self.environment.local_candidates.get(place))
            .or_else(|| {
                self.local_code_results
                    .get(&(expr as *const Expr as usize))
                    .and_then(Option::as_ref)
            })
            .or_else(|| {
                self.local_code_fields
                    .get(&(expr as *const Expr as usize))
                    .and_then(|fields| fields.get(&vec![]))
            });
        if let Some(keys) = keys {
            *self.fuel = self
                .fuel
                .checked_sub(keys.len())
                .ok_or("closure memory identity budget exhausted")?;
        }
        if let Some(keys) = keys {
            return Ok(Some(keys.clone()));
        }
        let named = self.callee_key(expr)?;
        self.intern_named_candidates(named)
    }
    fn intern_named_candidates(
        &mut self,
        named: CallableResult,
    ) -> Result<Option<BTreeSet<usize>>, String> {
        match named {
            CallableResult::Known(_) | CallableResult::Candidates(_) => {
                named.charge(self.fuel)?;
                let mut candidates = BTreeSet::new();
                for key in named.keys() {
                    self.step(0)?;
                    let id = if let Some(id) = self.named_code_slots.get(key) {
                        *id
                    } else {
                        let id = self.local_code.len();
                        self.local_code.push(LocalCode::Named(key.clone()));
                        self.named_code_slots.insert(key.clone(), id);
                        id
                    };
                    candidates.insert(id);
                }
                Ok(Some(candidates))
            }
            CallableResult::NoReturn => Ok(Some(BTreeSet::new())),
            CallableResult::Unknown | CallableResult::Forwarded { .. } => Ok(None),
        }
    }
    fn charge_capture_code(
        &mut self,
        captures: &BTreeMap<CallablePlace, CallableResult>,
    ) -> Result<(), String> {
        for (place, key) in captures {
            *self.fuel = self
                .fuel
                .checked_sub(place.fields.len() + 1)
                .ok_or("closure capture snapshot budget exhausted")?;
            key.charge(self.fuel)?;
        }
        Ok(())
    }
    fn snapshot_prepared_fields(
        &mut self,
        binding: LocalId,
        fields: LocalCodeFields,
    ) -> Result<PreparedCaptures, String> {
        let mut result = BTreeMap::new();
        for (path, ids) in fields {
            self.step(path.len())?;
            let mut definitions = vec![];
            let mut anonymous = false;
            for id in ids {
                let code = self
                    .local_code
                    .get(id)
                    .ok_or("captured code identity is missing")?;
                code.charge(self.fuel)?;
                anonymous |= matches!(code, PreparedCode::Closure { .. });
                definitions.push(code.clone());
            }
            if anonymous || definitions.is_empty() && path.contains(&VARIANT_FIELD) {
                result.insert(
                    CallablePlace {
                        binding,
                        fields: path,
                    },
                    definitions,
                );
            }
        }
        Ok(result)
    }
    // The initializer has already executed. Snapshot both named identities and
    // environment-bearing code before later captures/arguments can change them.
    fn snapshot_capture_code(
        &mut self,
        capture: &nir::program::Capture,
        named: &mut BTreeMap<CallablePlace, CallableResult>,
        prepared: &mut PreparedCaptures,
    ) -> Result<bool, String> {
        let pending;
        if let Some(value) = &capture.value {
            let fields = self.local_fields_for(value)?;
            // Non-variant leaves are required to construct this value. Empty
            // candidates there are bottom, unlike an absent enum payload.
            pending = fields
                .iter()
                .any(|(path, ids)| ids.is_empty() && !path.contains(&VARIANT_FIELD));
            prepared.extend(self.snapshot_prepared_fields(capture.binding, fields)?);
            for (fields, key) in self.value_callees(value)? {
                if matches!(
                    key,
                    CallableResult::Known(_) | CallableResult::Candidates(_)
                ) {
                    named.insert(
                        CallablePlace {
                            binding: capture.binding,
                            fields,
                        },
                        key,
                    );
                }
            }
        } else {
            let mut fields = BTreeMap::new();
            for (place, ids) in &self.environment.local_candidates {
                *self.fuel = self
                    .fuel
                    .checked_sub(place.fields.len() + ids.len() + 1)
                    .ok_or("closure capture snapshot budget exhausted")?;
                if place.binding == capture.binding {
                    fields.insert(place.fields.clone(), ids.clone());
                }
            }
            // Non-variant leaves are required to construct this value. Empty
            // candidates there are bottom, unlike an absent enum payload.
            pending = fields
                .iter()
                .any(|(path, ids)| ids.is_empty() && !path.contains(&VARIANT_FIELD));
            prepared.extend(self.snapshot_prepared_fields(capture.binding, fields)?);
            for (place, key) in &self.environment.callees {
                *self.fuel = self
                    .fuel
                    .checked_sub(place.fields.len() + 1)
                    .ok_or("closure capture snapshot budget exhausted")?;
                if place.binding == capture.binding
                    && matches!(
                        key,
                        CallableResult::Known(_) | CallableResult::Candidates(_)
                    )
                {
                    key.charge(self.fuel)?;
                    named.insert(place.clone(), key.clone());
                }
            }
        }
        Ok(pending)
    }
    fn remember_closure(
        &mut self,
        definition: &Expr,
        captures: BTreeMap<CallablePlace, CallableResult>,
        prepared_captures: std::sync::Arc<PreparedCaptures>,
        pointer_captures: BorrowedCaptures,
    ) -> Result<(), String> {
        charge_pointer_captures(&pointer_captures, self.fuel)?;
        charge_prepared_captures(&prepared_captures, self.fuel, 0)?;
        let node = definition as *const Expr as usize;
        let variants = self.closure_nodes.get(&node);
        if let Some(variants) = variants {
            *self.fuel = self
                .fuel
                .checked_sub(variants.len())
                .ok_or("closure capture snapshot budget exhausted")?;
        }
        let variants = variants.cloned().unwrap_or_default();
        for id in variants {
            self.charge_capture_code(&captures)?;
            let LocalCode::Closure {
                captures: previous,
                prepared_captures: old_prepared,
                ..
            } = &self.local_code[id]
            else {
                unreachable!()
            };
            charge_prepared_captures(old_prepared, self.fuel, 0)?;
            if let Some(pointers) = self.borrowed_code_pointers.get(&id) {
                charge_pointer_captures(pointers, self.fuel)?;
            }
            if previous.as_ref() == &captures
                && old_prepared == &prepared_captures
                && self.borrowed_code_pointers.get(&id).map(|p| p.as_ref())
                    == Some(&pointer_captures)
            {
                self.local_code_results
                    .insert(node, Some(BTreeSet::from([id])));
                return Ok(());
            }
        }
        let id = self.local_code.len();
        nir::visit::expression(definition, self.fuel, |expr, fuel| {
            bound_type(&expr.ty, fuel)
        })?;
        let owned = definition
            .instantiate_parameters(&self.declarations, self.arguments)
            .map_err(|e| e.to_string())?;
        self.local_code.push(LocalCode::Closure {
            definition: std::sync::Arc::new(owned),
            captures: std::sync::Arc::new(captures),
            prepared_captures,
        });
        self.borrowed_code_pointers
            .insert(id, std::sync::Arc::new(pointer_captures));
        self.closure_nodes.entry(node).or_default().push(id);
        self.local_code_results
            .insert(node, Some(BTreeSet::from([id])));
        Ok(())
    }
    fn transition_closure_environment(
        &mut self,
        id: usize,
        pointers: BorrowedCaptures,
        code_updates: PreparedCaptures,
    ) -> Result<usize, String> {
        charge_pointer_captures(&pointers, self.fuel)?;
        let LocalCode::Closure {
            definition,
            captures,
            prepared_captures,
        } = self
            .local_code
            .get(id)
            .ok_or("closure transition definition is missing")?
        else {
            return Err("pointer transition requires a closure".into());
        };
        let definition = definition.clone();
        let mut captures = captures.clone();
        let mut prepared_captures = prepared_captures.clone();
        if !code_updates.is_empty() {
            self.charge_capture_code(&captures)?;
            charge_prepared_captures(&prepared_captures, self.fuel, 0)?;
            charge_prepared_captures(&code_updates, self.fuel, 0)?;
            let mut named = captures.as_ref().clone();
            let mut prepared = prepared_captures.as_ref().clone();
            for (place, codes) in code_updates {
                named.remove(&place);
                prepared.insert(place, codes);
            }
            captures = std::sync::Arc::new(named);
            prepared_captures = std::sync::Arc::new(prepared);
        }
        for (candidate, code) in self.local_code.iter().enumerate() {
            *self.fuel = self
                .fuel
                .checked_sub(1)
                .ok_or("closure transition budget exhausted")?;
            if let LocalCode::Closure {
                definition: old,
                captures: named,
                prepared_captures: prepared,
            } = code
            {
                if std::sync::Arc::ptr_eq(&definition, old) {
                    for (place, key) in named.as_ref() {
                        *self.fuel = self
                            .fuel
                            .checked_sub(place.fields.len() + 1)
                            .ok_or("closure transition code budget exhausted")?;
                        key.charge(self.fuel)?;
                    }
                    charge_prepared_captures(prepared, self.fuel, 0)?;
                    if captures.as_ref() != named.as_ref()
                        || prepared_captures.as_ref() != prepared.as_ref()
                    {
                        continue;
                    }
                    if let Some(previous) = self.borrowed_code_pointers.get(&candidate) {
                        charge_pointer_captures(previous, self.fuel)?;
                        if previous.as_ref() == &pointers {
                            return Ok(candidate);
                        }
                    }
                }
            }
        }
        let next = self.local_code.len();
        self.local_code.push(LocalCode::Closure {
            definition,
            captures,
            prepared_captures,
        });
        self.borrowed_code_pointers
            .insert(next, std::sync::Arc::new(pointers));
        Ok(next)
    }

    fn abstract_capture_data(&mut self, ty: &Term, depth: usize) -> Result<bool, String> {
        self.step(depth)?;
        let ty = self.type_view(ty)?;
        effect_data_layout(self.package, &ty, self.fuel, depth + 1)
    }
    // A reusable capture invariant may include code selected from fixed source
    // definitions. Code supplied by a particular invocation needs actual state
    // transport; it must not be treated as fixed across later invocations.
    fn stable_code_source(
        &mut self,
        value: &Expr,
        path: &[usize],
        depth: usize,
    ) -> Result<bool, String> {
        self.step(depth)?;
        match &value.kind {
            K::Local(_) | K::Field { .. } | K::AnonymousField { .. } => {
                let Some(place) = self.callable_place(value)? else {
                    return Ok(false);
                };
                if !self.stable_capture_roots.contains(&place.binding) {
                    return Ok(false);
                }
                let fields = self.local_fields_for(value)?;
                let Some(ids) = fields.get(path) else {
                    return Ok(false);
                };
                if ids.is_empty() {
                    return Ok(false);
                }
                for id in ids {
                    self.step(depth)?;
                    let code = self
                        .local_code
                        .get(*id)
                        .ok_or("captured code source definition is missing")?;
                    if let LocalCode::Closure { definition, .. } = code {
                        let definition = definition.clone();
                        let K::Closure { captures, .. } = &definition.kind else {
                            return Err("captured code source is not a closure".into());
                        };
                        // Reusing code must not silently reuse a private pointer
                        // environment from another invocation or capture slot.
                        for capture in captures {
                            let ty = self.ty(&capture.ty)?;
                            if !self.abstract_capture_data(&ty, depth + 1)? {
                                return Ok(false);
                            }
                        }
                    }
                }
                Ok(true)
            }
            K::Function(_) | K::FunctionInstance { .. } | K::TraitMethod { .. } => {
                Ok(path.is_empty())
            }
            K::Closure { captures, .. } => {
                if !path.is_empty() {
                    return Ok(false);
                }
                // Data values stay abstract in body inference. Their changing
                // values cannot select an unrecorded code or pointer identity.
                // Initialization effects were checked when walking the RHS.
                for capture in captures {
                    let ty = self.ty(&capture.ty)?;
                    if !self.abstract_capture_data(&ty, depth + 1)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            K::Block {
                result: Some(value),
                ..
            }
            | K::CallPreparation {
                result: Some(value),
                ..
            } => self.stable_code_source(value, path, depth + 1),
            K::UnsafeScope(value) => self.stable_code_source(value, path, depth + 1),
            K::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let known = self.known_bool(condition, 0)?;
                if known != Some(false) && !self.stable_code_source(then_branch, path, depth + 1)? {
                    return Ok(false);
                }
                if known != Some(true) {
                    return match else_branch {
                        Some(value) => self.stable_code_source(value, path, depth + 1),
                        None => Ok(false),
                    };
                }
                Ok(true)
            }
            K::Match { arms, .. } => {
                // The scrutinee and guards may vary between invocations, but
                // each selected code identity must come from a fixed definition.
                // Invocation-local pattern bindings are deliberately not treated
                // as reusable environment premises.
                if arms.is_empty() {
                    return Ok(false);
                }
                for arm in arms {
                    if !self.stable_code_source(&arm.body, path, depth + 1)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            K::Variant { .. } | K::OptionalPresent(_) | K::OptionalAbsent => {
                let (fields, tag) = owned_constructor(value, self.fuel)?;
                let index = tag.ok_or("sum constructor is missing its branch")?;
                let ty = self.ty(&value.ty)?;
                owned_path_type(self.package, &ty, path, self.fuel)?;
                if path.len() < 3 || path[0] != VARIANT_FIELD {
                    return Ok(false);
                }
                if path[1] != index {
                    return Ok(true);
                }
                match fields.get(path[2]) {
                    Some(field) => self.stable_code_source(field, &path[3..], depth + 1),
                    None => Ok(false),
                }
            }
            K::Tuple(fields) | K::Record { fields, .. } | K::AnonymousRecord { fields } => {
                let Some((index, rest)) = path.split_first() else {
                    return Ok(false);
                };
                match fields.get(*index) {
                    Some(value) => self.stable_code_source(value, rest, depth + 1),
                    None => Ok(false),
                }
            }
            _ => Ok(false),
        }
    }
    fn record_capture_code_write(
        &mut self,
        root: LocalId,
        prefix: &[usize],
        expected: &Term,
        value: &Expr,
        fields: &LocalCodeFields,
    ) -> Result<(), String> {
        if !self.stable_capture_roots.contains(&root) {
            return Ok(());
        }
        for (path, _) in owned_callable_bottom(self.package, expected, self.fuel, true)? {
            if !self.stable_code_source(value, &path, 0)? {
                return Err("captured code derived from invocation values requires an environment transition contract".into());
            }
            let ids = fields
                .get(&path)
                .ok_or("captured code transition has unknown identity")?;
            if ids.is_empty() {
                // This path is absent in the assigned value. Keep prior
                // reachable identities in the reusable capture invariant.
                if path.contains(&VARIANT_FIELD) {
                    continue;
                }
                return Err("captured code transition has unknown identity".into());
            }
            let mut definitions = vec![];
            for id in ids {
                let code = self
                    .local_code
                    .get(*id)
                    .ok_or("captured code transition definition is missing")?;
                code.charge(self.fuel)?;
                definitions.push(code.clone());
            }
            let mut captured_path = prefix.to_vec();
            captured_path.extend(path);
            let previous = self
                .capture_code_writes
                .entry(CallablePlace {
                    binding: root,
                    fields: captured_path,
                })
                .or_default();
            join_prepared_codes(previous, definitions, self.fuel)?;
        }
        Ok(())
    }
    fn join_local_candidates(
        &mut self,
        left: Option<BTreeSet<usize>>,
        right: Option<BTreeSet<usize>>,
    ) -> Result<Option<BTreeSet<usize>>, String> {
        match (left, right) {
            (Some(mut left), Some(right)) => {
                *self.fuel = self
                    .fuel
                    .checked_sub(right.len() + 1)
                    .ok_or("closure memory identity budget exhausted")?;
                left.extend(right);
                Ok(Some(left))
            }
            _ => Ok(None),
        }
    }
    fn set_local_candidates(
        &mut self,
        binding: CallablePlace,
        keys: Option<BTreeSet<usize>>,
    ) -> Result<(), String> {
        self.step(binding.fields.len())?;
        for state in &mut self.loop_states {
            *self.fuel = self
                .fuel
                .checked_sub(1)
                .ok_or("closure memory identity budget exhausted")?;
            if let (Some(previous), Some(keys)) =
                (state.local_candidates.get_mut(&binding), keys.as_ref())
            {
                *self.fuel = self
                    .fuel
                    .checked_sub(keys.len())
                    .ok_or("closure memory identity budget exhausted")?;
                previous.extend(keys);
            } else {
                state.local_candidates.remove(&binding);
            }
        }
        if let Some(keys) = keys {
            self.environment.local_candidates.insert(binding, keys);
        } else {
            self.environment.local_candidates.remove(&binding);
        }
        Ok(())
    }
    fn charge_local_fields(&mut self, fields: &LocalCodeFields) -> Result<(), String> {
        for (path, keys) in fields {
            *self.fuel = self
                .fuel
                .checked_sub(path.len() + keys.len() + 1)
                .ok_or("local code field budget exhausted")?;
        }
        Ok(())
    }
    fn local_fields_for(&mut self, expr: &Expr) -> Result<LocalCodeFields, String> {
        if matches!(
            self.ty(&expr.ty)?,
            Term::Function { .. } | Term::UnsafeFunction(_) | Term::Closure { .. }
        ) {
            return Ok(self
                .local_candidates_for(expr)?
                .map(|keys| BTreeMap::from([(vec![], keys)]))
                .unwrap_or_default());
        }
        let mut values = BTreeMap::new();
        if let Some(place) = self.callable_place(expr)? {
            for (leaf, keys) in callable_subtree(&self.environment.local_candidates, &place) {
                *self.fuel = self
                    .fuel
                    .checked_sub(leaf.fields.len() + keys.len() + 1)
                    .ok_or("local code projection budget exhausted")?;
                if leaf.binding == place.binding && leaf.fields.starts_with(&place.fields) {
                    values.insert(leaf.fields[place.fields.len()..].to_vec(), keys.clone());
                }
            }
        } else if let Some(fields) = self.local_code_fields.get(&(expr as *const Expr as usize)) {
            for (path, keys) in fields {
                *self.fuel = self
                    .fuel
                    .checked_sub(path.len() + keys.len() + 1)
                    .ok_or("local code field budget exhausted")?;
            }
            values = fields.clone();
        }
        for (path, named) in self.value_callees(expr)? {
            if !values.contains_key(&path) {
                if let Some(keys) = self.intern_named_candidates(named)? {
                    values.insert(path, keys);
                }
            }
        }
        Ok(values)
    }
    fn join_local_fields(
        &mut self,
        mut left: LocalCodeFields,
        right: LocalCodeFields,
    ) -> Result<LocalCodeFields, String> {
        self.charge_local_fields(&left)?;
        for path in left.keys().cloned().collect::<Vec<_>>() {
            if let Some(keys) = right.get(&path) {
                *self.fuel = self
                    .fuel
                    .checked_sub(keys.len() + 1)
                    .ok_or("local code field join budget exhausted")?;
                left.get_mut(&path).unwrap().extend(keys);
            } else {
                left.remove(&path);
            }
        }
        Ok(left)
    }
    fn replace_local_fields(
        &mut self,
        place: CallablePlace,
        values: LocalCodeFields,
    ) -> Result<(), String> {
        let mut stale = vec![];
        for leaf in self.environment.local_candidates.keys() {
            *self.fuel = self
                .fuel
                .checked_sub(leaf.fields.len() + 1)
                .ok_or("local code replacement budget exhausted")?;
            if leaf.binding == place.binding
                && leaf.fields.starts_with(&place.fields)
                && !values.contains_key(&leaf.fields[place.fields.len()..])
            {
                stale.push(leaf.clone());
            }
        }
        for leaf in stale {
            self.set_local_candidates(leaf, None)?;
        }
        for (path, keys) in values {
            self.step(path.len())?;
            let mut leaf = place.clone();
            leaf.fields.extend(path);
            self.set_local_candidates(leaf, Some(keys))?;
        }
        Ok(())
    }
    fn join_call_footprint(
        &mut self,
        combined: &mut Option<CallFootprint>,
        next: CallFootprint,
    ) -> Result<(), String> {
        if let Some(summary) = combined {
            join_code_fields(&mut summary.code_fields, &next.code_fields, self.fuel)?;
            summary.pointer_fields = self.join_pointer_fields(
                std::mem::take(&mut summary.pointer_fields),
                next.pointer_fields,
            )?;
            summary.may_reject |= next.may_reject;
            summary.may_run_cleanup |= next.may_run_cleanup;
            summary.result_targets =
                self.join_targets(summary.result_targets.take(), next.result_targets)?;
            summary
                .callable_result
                .join(&next.callable_result, self.fuel)?;
            summary.callable_fields = self.join_callee_fields(
                std::mem::take(&mut summary.callable_fields),
                next.callable_fields,
            )?;
            summary.accesses.extend(next.accesses);
        } else {
            *combined = Some(next);
        }
        Ok(())
    }
    fn callee_key(&mut self, expr: &Expr) -> Result<CallableResult, String> {
        self.step(0)?;
        if let K::UnsafeScope(body) = &expr.kind {
            return self.callee_key(body);
        }
        match &expr.kind {
            K::TraitMethod {
                interface,
                interface_arguments,
                subject,
                member,
                method_arguments,
            } => {
                let package = self
                    .package
                    .ok_or("method memory inference requires a package")?;
                let concrete = Expr {
                    source: expr.source,
                    ty: self.ty(&expr.ty)?,
                    kind: K::TraitMethod {
                        interface: *interface,
                        member: *member,
                        subject: self.ty(subject)?,
                        interface_arguments: interface_arguments
                            .iter()
                            .map(|ty| self.ty(ty))
                            .collect::<Result<_, _>>()?,
                        method_arguments: method_arguments
                            .iter()
                            .map(|ty| self.ty(ty))
                            .collect::<Result<_, _>>()?,
                    },
                };
                let selected = method_callee(package, &concrete, self.fuel)?;
                let K::FunctionInstance {
                    declaration,
                    arguments,
                } = selected.kind
                else {
                    unreachable!()
                };
                Ok(CallableResult::Known(iir::InstanceKey {
                    declaration,
                    arguments,
                }))
            }
            K::Function(declaration) => Ok(CallableResult::Known(iir::InstanceKey {
                declaration: *declaration,
                arguments: vec![],
            })),
            K::FunctionInstance {
                declaration,
                arguments,
            } => Ok(CallableResult::Known(iir::InstanceKey {
                declaration: *declaration,
                arguments: arguments
                    .iter()
                    .map(|ty| self.ty(ty))
                    .collect::<Result<_, _>>()?,
            })),
            K::Local(_) | K::Field { .. } | K::AnonymousField { .. } => {
                let Some(place) = self.callable_place(expr)? else {
                    if let Some(key) = self
                        .aggregate_callees
                        .get(&(expr as *const Expr as usize))
                        .and_then(|values| values.get(&vec![]))
                    {
                        key.charge(self.fuel)?;
                        return Ok(key.clone());
                    }
                    return Ok(CallableResult::Unknown);
                };
                if let Some(key) = self.environment.callees.get(&place) {
                    key.charge(self.fuel)?;
                }
                Ok(self
                    .environment
                    .callees
                    .get(&place)
                    .cloned()
                    .unwrap_or(CallableResult::Unknown))
            }
            K::Block { .. }
            | K::CallPreparation { .. }
            | K::If { .. }
            | K::Match { .. }
            | K::Call { .. }
            | K::NewtypeConvert(_)
            | K::ReadPointer(_)
            | K::EffectRequest { .. }
            | K::EffectComputation { .. }
            | K::EffectRun { .. }
            | K::EffectHandle { .. } => {
                let key = self.callable_results.get(&(expr as *const Expr as usize));
                if let Some(key) = key {
                    key.charge(self.fuel)?;
                }
                Ok(key.cloned().unwrap_or(CallableResult::Unknown))
            }
            _ => Ok(CallableResult::Unknown),
        }
    }
    fn join_callee_result(
        &mut self,
        joined: &mut CallableResult,
        value: CallableResult,
    ) -> Result<(), String> {
        self.step(0)?;
        joined.join(&value, self.fuel)?;
        Ok(())
    }
    fn set_callee_at(&mut self, place: CallablePlace, key: CallableResult) -> Result<(), String> {
        for index in 0..self.loop_states.len() {
            self.step(0)?;
            key.charge(self.fuel)?;
            let entry = self.loop_states[index]
                .callees
                .entry(place.clone())
                .or_insert(CallableResult::Unknown);
            entry.join(&key, self.fuel)?;
        }
        self.environment.callees.insert(place, key);
        Ok(())
    }
    fn callable_place(&mut self, expr: &Expr) -> Result<Option<CallablePlace>, String> {
        let mut cursor = expr;
        let mut fields = vec![];
        loop {
            self.step(fields.len())?;
            match &cursor.kind {
                K::Local(binding) => {
                    fields.reverse();
                    return Ok(Some(CallablePlace {
                        binding: *binding,
                        fields,
                    }));
                }
                K::Field { value, index, .. } | K::AnonymousField { value, index, .. } => {
                    fields.push(*index);
                    cursor = value;
                }
                _ => return Ok(None),
            }
        }
    }
    fn value_callees(
        &mut self,
        expr: &Expr,
    ) -> Result<BTreeMap<Vec<usize>, CallableResult>, String> {
        if let K::UnsafeScope(body) = &expr.kind {
            self.step(0)?;
            return self.value_callees(body);
        }
        if matches!(
            self.ty(&expr.ty)?,
            Term::Function { .. } | Term::UnsafeFunction(_)
        ) {
            return Ok(BTreeMap::from([(vec![], self.callee_key(expr)?)]));
        }
        if let Some(place) = self.callable_place(expr)? {
            let mut values = BTreeMap::new();
            for (leaf, key) in callable_subtree(&self.environment.callees, &place) {
                *self.fuel = self
                    .fuel
                    .checked_sub(leaf.fields.len() + 1)
                    .ok_or("memory callable projection budget exhausted")?;
                if leaf.binding == place.binding && leaf.fields.starts_with(&place.fields) {
                    key.charge(self.fuel)?;
                    values.insert(leaf.fields[place.fields.len()..].to_vec(), key.clone());
                }
            }
            return Ok(values);
        }
        if let Some(values) = self.aggregate_callees.get(&(expr as *const Expr as usize)) {
            for (path, key) in values {
                *self.fuel = self
                    .fuel
                    .checked_sub(path.len() + 1)
                    .ok_or("memory callable aggregate budget exhausted")?;
                key.charge(self.fuel)?;
            }
            return Ok(values.clone());
        }
        Ok(BTreeMap::new())
    }
    fn join_callee_fields(
        &mut self,
        mut left: BTreeMap<Vec<usize>, CallableResult>,
        right: BTreeMap<Vec<usize>, CallableResult>,
    ) -> Result<BTreeMap<Vec<usize>, CallableResult>, String> {
        for (path, key) in &mut left {
            self.step(path.len())?;
            key.join(
                right.get(path).unwrap_or(&CallableResult::Unknown),
                self.fuel,
            )?;
        }
        for (path, _) in right {
            self.step(path.len())?;
            left.entry(path).or_insert(CallableResult::Unknown);
        }
        Ok(left)
    }
    fn merge_code_field_return(&mut self, expr: &Expr) -> Result<(), String> {
        let fields = self.local_fields_for(expr)?;
        for (path, result) in &mut self.code_field_returns {
            *self.fuel = self
                .fuel
                .checked_sub(path.len() + 1)
                .ok_or("code return field budget exhausted")?;
            let value = match fields.get(path) {
                None => CodeResult::Unknown,
                Some(ids) if ids.is_empty() && path.contains(&VARIANT_FIELD) => {
                    CodeResult::Known(vec![])
                }
                Some(ids) if ids.is_empty() => CodeResult::NoReturn,
                Some(ids) => {
                    let mut values = vec![];
                    for id in ids {
                        let code = self
                            .local_code
                            .get(*id)
                            .ok_or("returned code definition is missing")?;
                        code.charge(self.fuel)?;
                        values.push(code.clone());
                    }
                    CodeResult::Known(values)
                }
            };
            result.join(&value, self.fuel)?;
        }
        Ok(())
    }
    fn remember_returned_code(&mut self, code: &PreparedCode) -> Result<usize, String> {
        code.charge(self.fuel)?;
        for (id, previous) in self.local_code.iter().enumerate() {
            previous.charge(self.fuel)?;
            if previous == code {
                return Ok(id);
            }
        }
        let id = self.local_code.len();
        self.local_code.push(code.clone());
        Ok(id)
    }
    fn install_code_results(&mut self, expr: &Expr, fields: &CodeFields) -> Result<(), String> {
        let mut local = BTreeMap::new();
        for (path, value) in fields {
            self.step(path.len())?;
            value.charge(self.fuel)?;
            let ids = match value {
                CodeResult::Unknown => continue,
                CodeResult::NoReturn => BTreeSet::new(),
                CodeResult::Known(values) => {
                    if values.is_empty() {
                        let leaf =
                            owned_path_type(self.package, &self.ty(&expr.ty)?, path, self.fuel)?;
                        if !path.contains(&VARIANT_FIELD)
                            || !matches!(
                                leaf,
                                Term::Function { .. }
                                    | Term::UnsafeFunction(_)
                                    | Term::Closure { .. }
                            )
                        {
                            return Err(
                                "empty returned code candidate set requires an absent variant leaf"
                                    .into(),
                            );
                        }
                    }
                    let mut ids = BTreeSet::new();
                    for value in values {
                        ids.insert(self.remember_returned_code(value)?);
                    }
                    ids
                }
            };
            local.insert(path.clone(), ids);
        }
        self.local_code_fields
            .insert(expr as *const Expr as usize, local);
        Ok(())
    }
    /// Reattach invocation-local origins after rebasing a returned value. Code
    /// identity alone cannot select the environment of a particular factory call.
    fn install_returned_capture_origins(&mut self, expr: &Expr) -> Result<(), String> {
        if self
            .local_code_fields
            .get(&(expr as *const Expr as usize))
            .is_none_or(BTreeMap::is_empty)
        {
            return Ok(());
        }
        let fields = self.pointer_fields_for(expr)?;
        let mut code_fields = self.local_fields_for(expr)?;
        for (prefix, ids) in &mut code_fields {
            let mut updated = BTreeSet::new();
            for id in std::mem::take(ids) {
                self.step(prefix.len())?;
                let Some(LocalCode::Closure { definition, .. }) = self.local_code.get(id) else {
                    updated.insert(id);
                    continue;
                };
                let definition = definition.clone();
                let K::Closure { captures, .. } = &definition.kind else {
                    return Err("returned capture origins require closure code".into());
                };
                let mut pointers = BTreeMap::new();
                for (index, capture) in captures.iter().enumerate() {
                    let mut paths = BTreeMap::new();
                    for (path, _) in symbolic_pointer_fields(self.package, &capture.ty, self.fuel)?
                    {
                        self.step(prefix.len() + path.len() + 1)?;
                        let mut full = prefix.clone();
                        full.push(index);
                        full.extend(&path);
                        paths.insert(path, fields.get(&full).cloned().unwrap_or(None));
                    }
                    if !paths.is_empty() {
                        pointers.insert(capture.binding, paths);
                    }
                }
                updated.insert(if pointers.is_empty() {
                    id
                } else {
                    self.transition_closure_environment(id, pointers, BTreeMap::new())?
                });
            }
            *ids = updated;
        }
        self.local_code_fields
            .insert(expr as *const Expr as usize, code_fields);
        Ok(())
    }

    fn merge_callable_field_return(&mut self, expr: &Expr) -> Result<(), String> {
        self.merge_code_field_return(expr)?;
        let values = self.value_callees(expr)?;
        for (path, key) in &mut self.callable_field_returns {
            *self.fuel = self
                .fuel
                .checked_sub(path.len() + 1)
                .ok_or("memory callable return budget exhausted")?;
            key.join(
                values.get(path).unwrap_or(&CallableResult::Unknown),
                self.fuel,
            )?;
        }
        Ok(())
    }
    fn replace_callees(
        &mut self,
        place: CallablePlace,
        values: BTreeMap<Vec<usize>, CallableResult>,
    ) -> Result<(), String> {
        let mut stale = vec![];
        for leaf in self.environment.callees.keys() {
            *self.fuel = self
                .fuel
                .checked_sub(leaf.fields.len() + 1)
                .ok_or("memory callable replacement budget exhausted")?;
            if leaf.binding == place.binding && leaf.fields.starts_with(&place.fields) {
                stale.push(leaf.clone());
            }
        }
        for leaf in stale {
            if !values.contains_key(&leaf.fields[place.fields.len()..]) {
                self.set_callee_at(leaf, CallableResult::Unknown)?;
            }
        }
        for (path, key) in values {
            self.step(path.len())?;
            let mut leaf = place.clone();
            leaf.fields.extend(path);
            self.set_callee_at(leaf, key)?;
        }
        Ok(())
    }
    fn bind_owned_callees(
        &mut self,
        pattern: &'a Pattern,
        ty: &Term,
        values: &BTreeMap<Vec<usize>, CallableResult>,
        local_fields: &LocalCodeFields,
        depth: usize,
    ) -> Result<(), String> {
        // Direct-pointer callers install their bindings in bind_match_subject.
        // This entry point adds only code evidence without overwriting them.
        self.bind_owned_values(
            pattern,
            ty,
            values,
            local_fields,
            &BTreeMap::new(),
            false,
            depth,
        )
    }

    fn project_pointer_fields(
        &mut self,
        values: &BTreeMap<Vec<usize>, Targets>,
        prefix: &[usize],
    ) -> Result<BTreeMap<Vec<usize>, Targets>, String> {
        let mut projected = BTreeMap::new();
        for (path, targets) in values {
            self.step(path.len())?;
            if path.starts_with(prefix) {
                if let Some(targets) = targets {
                    self.target_work(targets)?;
                }
                projected.insert(path[prefix.len()..].to_vec(), targets.clone());
            }
        }
        Ok(projected)
    }

    fn forget_pattern_alias(&mut self, binding: LocalId) -> Result<(), String> {
        self.locals.remove(&binding);
        self.environment.pointers.remove(&binding);
        self.replace_pointer_fields(CallablePlace::root(binding), BTreeMap::new())?;
        self.replace_callees(CallablePlace::root(binding), BTreeMap::new())?;
        self.replace_local_fields(CallablePlace::root(binding), BTreeMap::new())
    }

    fn bind_owned_values(
        &mut self,
        pattern: &'a Pattern,
        ty: &Term,
        values: &BTreeMap<Vec<usize>, CallableResult>,
        local_fields: &LocalCodeFields,
        pointers: &BTreeMap<Vec<usize>, Targets>,
        bind_pointers: bool,
        depth: usize,
    ) -> Result<(), String> {
        self.step(depth)?;
        match pattern {
            Pattern::Wild | Pattern::Unit | Pattern::Literal(_) => Ok(()),
            Pattern::Observe { binding, inner } | Pattern::Binding { binding, inner } => {
                bound_type(ty, self.fuel)?;
                self.locals.insert(*binding, ty.clone());
                charge_callee_fields(values, self.fuel)?;
                self.replace_callees(CallablePlace::root(*binding), values.clone())?;
                self.charge_local_fields(local_fields)?;
                self.replace_local_fields(CallablePlace::root(*binding), local_fields.clone())?;
                if bind_pointers {
                    let fields = self.project_pointer_fields(pointers, &[])?;
                    self.replace_pointer_fields(CallablePlace::root(*binding), fields)?;
                }
                if bind_pointers && self.local_pointer_value(ty)? {
                    // A pointer leaf is the empty relative path. Missing evidence
                    // stays unknown; it must never inherit a previous binding.
                    let targets = pointers.get(&Vec::new());
                    if let Some(Some(targets)) = targets {
                        self.target_work(targets)?;
                    }
                    self.set_pointer(*binding, targets.cloned().unwrap_or(None))?;
                }
                let result = self.bind_owned_values(
                    inner,
                    ty,
                    values,
                    local_fields,
                    pointers,
                    bind_pointers,
                    depth + 1,
                );
                if matches!(pattern, Pattern::Observe { .. }) {
                    self.forget_pattern_alias(*binding)?;
                }
                result
            }
            Pattern::Tuple(patterns)
            | Pattern::Record {
                fields: patterns, ..
            }
            | Pattern::AnonymousRecord {
                fields: patterns, ..
            } => {
                let fields = owned_fields(self.package, ty, self.fuel)?;
                if fields.len() != patterns.len() {
                    return Err("memory pattern field count mismatch".into());
                }
                if let Pattern::AnonymousRecord { names, .. } = pattern {
                    if !matches!(ty,Term::Record(types) if names.len() == types.len() && names.iter().zip(types).all(|(name,(expected,_))| name == expected))
                    {
                        return Err("memory anonymous record pattern labels mismatch".into());
                    }
                }
                if let Pattern::Record { declaration, .. } = pattern {
                    if !matches!(ty,Term::Nominal {declaration:actual,..} if actual==declaration) {
                        return Err("memory record pattern declaration mismatch".into());
                    }
                }
                for (index, (pattern, ty)) in patterns.iter().zip(&fields).enumerate() {
                    let mut nested = BTreeMap::new();
                    for (path, key) in values {
                        self.step(path.len())?;
                        key.charge(self.fuel)?;
                        if path.first() == Some(&index) {
                            nested.insert(path[1..].to_vec(), key.clone());
                        }
                    }
                    self.charge_local_fields(local_fields)?;
                    let local_nested = local_fields
                        .iter()
                        .filter(|(path, _)| path.first() == Some(&index))
                        .map(|(path, keys)| (path[1..].to_vec(), keys.clone()))
                        .collect();
                    let pointer_fields = self.project_pointer_fields(pointers, &[index])?;
                    self.bind_owned_values(
                        pattern,
                        &ty,
                        &nested,
                        &local_nested,
                        &pointer_fields,
                        bind_pointers,
                        depth + 1,
                    )?;
                }
                Ok(())
            }
            Pattern::OptionalAbsent
            | Pattern::OptionalPresent(_)
            | Pattern::Variant { .. }
            | Pattern::ErrorResult { .. } => {
                let (tag, patterns, fields) =
                    owned_variant_pattern(self.package, pattern, ty, self.fuel)?;
                let index = &tag;
                for (field, (pattern, ty)) in patterns.iter().zip(&fields).enumerate() {
                    let prefix = [VARIANT_FIELD, *index, field];
                    charge_callee_fields(values, self.fuel)?;
                    self.charge_local_fields(local_fields)?;
                    let nested = values
                        .iter()
                        .filter(|(path, _)| path.starts_with(&prefix))
                        .map(|(path, value)| (path[3..].to_vec(), value.clone()))
                        .collect();
                    let code = local_fields
                        .iter()
                        .filter(|(path, _)| path.starts_with(&prefix))
                        .map(|(path, value)| (path[3..].to_vec(), value.clone()))
                        .collect();
                    let pointer_fields = self.project_pointer_fields(pointers, &prefix)?;
                    self.bind_owned_values(
                        pattern,
                        ty,
                        &nested,
                        &code,
                        &pointer_fields,
                        bind_pointers,
                        depth + 1,
                    )?;
                }
                Ok(())
            }
            Pattern::Guard { pattern, condition } => {
                self.bind_owned_values(
                    pattern,
                    ty,
                    values,
                    local_fields,
                    pointers,
                    bind_pointers,
                    depth + 1,
                )?;
                self.walk_guard(condition, depth + 1)
            }
            Pattern::Then { first, value, next } => {
                self.bind_owned_values(
                    first,
                    ty,
                    values,
                    local_fields,
                    pointers,
                    bind_pointers,
                    depth + 1,
                )?;
                let computed = self.ty(&value.ty)?;
                if !matches!(computed, Term::Pointer(_))
                    && !Self::guard_value_type(self.package, &computed)
                {
                    return Err(
                        "computed memory pattern needs independent result provenance".into(),
                    );
                }
                self.walk(value, depth + 1)?;
                if matches!(computed, Term::Pointer(_)) {
                    let targets = self.optional_target(value, depth + 1)?;
                    self.bind_match_subject(next, &targets, true, &computed, true, depth + 1)?;
                }
                let named = self.value_callees(value)?;
                let code = self.local_fields_for(value)?;
                let mut pointers = self.pointer_fields_for(value)?;
                if self.local_pointer_value(&computed)? {
                    pointers.insert(vec![], self.optional_target(value, depth + 1)?);
                }
                self.bind_owned_values(next, &computed, &named, &code, &pointers, true, depth + 1)
            }
            Pattern::Remap { pattern, bindings } => {
                self.bind_owned_values(
                    pattern,
                    ty,
                    values,
                    local_fields,
                    pointers,
                    bind_pointers,
                    depth + 1,
                )?;
                let environment = self.save_environment()?;
                for (from, to) in bindings {
                    let ty = self
                        .locals
                        .get(from)
                        .ok_or("memory callable remap source missing")?;
                    bound_type(ty, self.fuel)?;
                    self.locals.insert(*to, ty.clone());
                    let mut named = BTreeMap::new();
                    let mut code = BTreeMap::new();
                    for (place, value) in &environment.callees {
                        self.step(place.fields.len())?;
                        value.charge(self.fuel)?;
                        if place.binding == *from {
                            named.insert(place.fields.clone(), value.clone());
                        }
                    }
                    for (place, value) in &environment.local_candidates {
                        self.step(place.fields.len())?;
                        *self.fuel = self
                            .fuel
                            .checked_sub(value.len())
                            .ok_or("memory callable remap budget exhausted")?;
                        if place.binding == *from {
                            code.insert(place.fields.clone(), value.clone());
                        }
                    }
                    if let Some(targets) = environment.pointers.get(from) {
                        if let Some(targets) = targets {
                            self.target_work(targets)?;
                        }
                        self.set_pointer(*to, targets.clone())?;
                    }
                    let mut pointers = BTreeMap::new();
                    for (place, targets) in &environment.field_pointers {
                        self.step(place.fields.len())?;
                        if place.binding == *from {
                            if let Some(targets) = targets {
                                self.target_work(targets)?;
                            }
                            pointers.insert(place.fields.clone(), targets.clone());
                        }
                    }
                    self.replace_pointer_fields(CallablePlace::root(*to), pointers)?;
                    self.replace_callees(CallablePlace::root(*to), named)?;
                    self.replace_local_fields(CallablePlace::root(*to), code)?;
                }
                Ok(())
            }
            Pattern::Or(patterns) => {
                let base = self.save_environment()?;
                let mut joined = None;
                for pattern in patterns {
                    self.environment = self.clone_environment(&base)?;
                    self.bind_owned_values(
                        pattern,
                        ty,
                        values,
                        local_fields,
                        pointers,
                        bind_pointers,
                        depth + 1,
                    )?;
                    let next = self.save_environment()?;
                    joined = Some(if let Some(previous) = joined {
                        self.join_environment(previous, next)?
                    } else {
                        next
                    });
                }
                self.environment = joined.unwrap_or(base);
                Ok(())
            }
            _ => Err("memory binding requires an owned pattern provenance contract".into()),
        }
    }
    fn set_pointer(&mut self, binding: LocalId, targets: Targets) -> Result<(), String> {
        // Every intermediate loop assignment contributes, including paths that
        // break/continue before later textual assignments. Unknown is absorbing.
        for index in 0..self.loop_states.len() {
            self.step(0)?;
            if let Some(targets) = &targets {
                self.target_work(targets)?;
            }
            let value = if let Some(previous) = self.loop_states[index].pointers.remove(&binding) {
                self.join_targets(previous, targets.clone())?
            } else {
                targets.clone()
            };
            self.loop_states[index].pointers.insert(binding, value);
        }
        self.environment.pointers.insert(binding, targets);
        Ok(())
    }
    fn install_no_return_call(&mut self, expr: &Expr) -> Result<(), String> {
        self.expression_results
            .insert(expr as *const Expr as usize, Some(BTreeSet::new()));
        self.callable_results
            .insert(expr as *const Expr as usize, CallableResult::NoReturn);
        let ty = self.ty(&expr.ty)?;
        let fields = result_callable_bottom(self.package, &ty, self.fuel)?;
        self.aggregate_callees
            .insert(expr as *const Expr as usize, fields);
        let code_fields = code_result_bottom(self.package, &ty, self.fuel)?;
        self.install_code_results(expr, &code_fields)
    }

    fn immediate_borrowed_call(
        &mut self,
        expr: &'a Expr,
        callee: &'a Expr,
        arguments: &'a [nir::program::CallArgument],
        bindings: &[LocalId],
        depth: usize,
    ) -> Result<bool, String> {
        let K::Closure {
            captures,
            parameters: closure_bindings,
            ..
        } = &callee.kind
        else {
            return Ok(false);
        };
        let mut has_pointer = false;
        for capture in captures {
            has_pointer |=
                !symbolic_pointer_fields(self.package, &self.ty(&capture.ty)?, self.fuel)?
                    .is_empty();
        }
        if !has_pointer {
            return Ok(false);
        }
        let package = self
            .package
            .ok_or("borrowed closure call requires a package")?;
        let signature = self.ty(callee.ty.callable_signature())?;
        let Term::Function { parameters, result } = &signature else {
            return Err("borrowed closure requires a signature".into());
        };
        validate_argument_plan(arguments, bindings, parameters.len())?;
        if arguments.iter().any(|a| a.is_default) {
            return Err("immediate closure cannot have default arguments".into());
        }
        if !self.local_pointer_value(result)? {
            symbolic_pointer_fields(self.package, result, self.fuel)?;
        }
        let mut targets = BTreeMap::new();
        let mut captured_code = BTreeMap::new();
        let mut prepared_captures = PreparedCaptures::new();
        let mut inputs = BTreeMap::new();
        let mut prepared_inputs = BTreeMap::new();
        if closure_bindings.len() != parameters.len() {
            return Err("borrowed closure parameter layout mismatch".into());
        }
        let mut capture_slot = parameters.len();
        let mut pending_capture = false;
        // Snapshot capture origins before ordinary argument evaluation. They are
        // virtual inputs to this invocation, never published environment fields.
        for capture in captures {
            let ty = self.ty(&capture.ty)?;
            if let Some(value) = &capture.value {
                if self.ty(&value.ty)? != ty {
                    return Err("borrowed capture initializer type mismatch".into());
                }
                self.walk(value, depth + 1)?;
            } else if self.locals.get(&capture.binding) != Some(&ty) {
                return Err("borrowed capture is not in scope".into());
            }
            let fields = self.capture_pointer_fields(capture, depth)?;
            if !fields.is_empty() {
                targets.insert(
                    capture_slot,
                    fields
                        .into_iter()
                        .filter_map(|(path, target)| target.map(|target| (path, target)))
                        .collect(),
                );
                capture_slot += 1;
            }
            pending_capture |=
                self.snapshot_capture_code(capture, &mut captured_code, &mut prepared_captures)?;
        }
        if pending_capture {
            self.install_no_return_call(expr)?;
            return Ok(true);
        }
        for argument in arguments {
            let ty = self.ty(&argument.value.ty)?;
            let view = self.type_view(&ty)?;
            symbolic_pointer_fields(self.package, &view, self.fuel)?;
            self.walk(&argument.value, depth + 1)?;
            let mut fields = self.pointer_fields_for(&argument.value)?;
            if self.local_pointer_value(&ty)? {
                fields.insert(vec![], self.optional_target(&argument.value, depth + 1)?);
            }
            targets.insert(
                argument.parameter,
                fields
                    .into_iter()
                    .filter_map(|(path, target)| target.map(|target| (path, target)))
                    .collect(),
            );
            for (fields, value) in self.value_callees(&argument.value)? {
                if matches!(
                    value,
                    CallableResult::Known(_)
                        | CallableResult::Candidates(_)
                        | CallableResult::NoReturn
                ) {
                    self.step(fields.len())?;
                    value.charge(self.fuel)?;
                    inputs.insert(
                        CallableInput {
                            parameter: argument.parameter,
                            fields,
                        },
                        value,
                    );
                }
            }
            let fields = self.local_fields_for(&argument.value)?;
            for (place, codes) in
                self.snapshot_prepared_fields(closure_bindings[argument.parameter], fields)?
            {
                prepared_inputs.insert(
                    CallableInput {
                        parameter: argument.parameter,
                        fields: place.fields,
                    },
                    codes,
                );
            }
        }
        nir::visit::expression(callee, self.fuel, |expr, fuel| bound_type(&expr.ty, fuel))?;
        let definition = callee
            .instantiate_parameters(&self.declarations, self.arguments)
            .map_err(|e| e.to_string())?;
        let code_inputs = borrow_code_inputs(&prepared_inputs);
        let summary = infer_closure_with_borrowed_in_round(
            package,
            &definition,
            &captured_code,
            &prepared_captures,
            &inputs,
            &code_inputs,
            self.round,
            self.fuel,
            false,
            false,
        )?;
        // Publish both forms of code evidence. Named-only results must remain
        // available to ordinary higher-order calls; mixed/anonymous results
        // retain their full environments in the local prepared-code table.
        self.install_code_results(expr, &summary.code_fields)?;
        let mut named_fields = BTreeMap::new();
        for (path, value) in &summary.code_fields {
            self.step(path.len())?;
            value.charge(self.fuel)?;
            let mut named = CallableResult::NoReturn;
            match value {
                CodeResult::NoReturn => (),
                CodeResult::Unknown => named = CallableResult::Unknown,
                CodeResult::Known(values) => {
                    for code in values {
                        let next = match code {
                            PreparedCode::Named(key) => CallableResult::Known(key.clone()),
                            PreparedCode::Closure { .. } => CallableResult::Unknown,
                        };
                        named.join(&next, self.fuel)?;
                    }
                }
            }
            named_fields.insert(path.clone(), named);
        }
        if let Some(root) = named_fields.get(&vec![]) {
            root.charge(self.fuel)?;
            self.callable_results
                .insert(expr as *const Expr as usize, root.clone());
        }
        self.aggregate_callees
            .insert(expr as *const Expr as usize, named_fields);
        self.may_reject |= summary.may_reject;
        self.may_run_cleanup |= summary.may_run_cleanup;
        let mut fields = BTreeMap::new();
        for (path, origins) in &summary.pointer_fields {
            self.step(path.len())?;
            fields.insert(
                path.clone(),
                self.rebase_result(origins.as_ref(), &targets)?,
            );
        }
        self.aggregate_pointers
            .insert(expr as *const Expr as usize, fields);
        self.install_returned_capture_origins(expr)?;
        let result = self.rebase_result(summary.result_targets.as_ref(), &targets)?;
        self.expression_results
            .insert(expr as *const Expr as usize, result);
        for access in summary.accesses {
            let actual = targets
                .get(&access.target.parameter)
                .and_then(|fields| fields.get(&access.target.input_fields))
                .ok_or("borrowed closure target is missing")?;
            self.target_work(actual)?;
            for base in actual {
                self.step(depth)?;
                let mut target = base.clone();
                self.step(access.target.path_cost())?;
                target.append_resolved_path(&access.target)?;
                bound_type(&access.ty, self.fuel)?;
                self.record_access(
                    BTreeSet::from([target]),
                    access.write,
                    access.read_value,
                    access.ty.clone(),
                )?;
            }
        }
        Ok(true)
    }
    fn local_address(&mut self, value: &Expr, depth: usize) -> Result<PointerPlace, String> {
        self.step(depth)?;
        let target = match &value.kind {
            K::Local(binding) => {
                let ty = self.ty(&value.ty)?;
                if self.locals.get(binding) != Some(&ty) {
                    return Err("local address type is not established".into());
                }
                // Known pointer-bearing local layouts are now valid storage
                // roots: loads resolve their current environment origins.
                // This does not authorize pointer stores or local escape.
                if !self.local_pointer_value(&ty)?
                    && symbolic_pointer_fields(self.package, &ty, self.fuel)?.is_empty()
                {
                    self.pointer_free_owned(&ty, depth + 1)?;
                }
                let mut storage = None;
                for (index, (known_binding, known_ty)) in self.local_storage.iter().enumerate() {
                    bound_type(known_ty, self.fuel)?;
                    if known_binding == binding && known_ty == &ty {
                        storage = Some(index);
                        break;
                    }
                }
                let storage = match storage {
                    Some(index) => index,
                    None => {
                        let index = self.local_storage.len();
                        self.local_storage.push((*binding, ty));
                        index
                    }
                };
                PointerPlace {
                    root: PointerRoot::Local {
                        binding: *binding,
                        storage,
                    },
                    input_fields: vec![],
                    through: vec![],
                    conditions: BTreeMap::new(),
                    fields: vec![],
                }
            }
            K::Field {
                value: parent,
                declaration,
                index,
            } => {
                let mut target = self.local_address(parent, depth + 1)?;
                if !matches!(self.ty(&parent.ty)?,Term::Nominal {declaration:actual,..} if actual==*declaration)
                {
                    return Err("local address field owner mismatch".into());
                }
                target
                    .fields
                    .push((nir::program::FieldOwner::Nominal(*declaration), *index));
                target
            }
            K::AnonymousField {
                value: parent,
                index,
                ..
            } => {
                let mut target = self.local_address(parent, depth + 1)?;
                let parent_ty = self.ty(&parent.ty)?;
                if !matches!(parent_ty, Term::Record(_))
                    && tuple_fields(self.package, &parent_ty).is_none()
                {
                    return Err("local address anonymous field owner mismatch".into());
                }
                target
                    .fields
                    .push((nir::program::FieldOwner::Anonymous, *index));
                target
            }
            _ => return Err("local address requires owned storage".into()),
        };
        if target.path_cost() > 128 {
            return Err("local address depth exhausted".into());
        }
        let PointerRoot::Local { binding, storage } = target.root else {
            unreachable!()
        };
        let (owner, root) = self
            .local_storage
            .get(storage)
            .ok_or("local address root is missing")?;
        if *owner != binding {
            return Err("local address storage identity mismatch".into());
        }
        let path = target
            .fields
            .iter()
            .map(|(_, index)| *index)
            .collect::<Vec<_>>();
        let expected = owned_path_type(self.package, root, &path, self.fuel)?;
        if expected != self.ty(&value.ty)? {
            return Err("local address field type mismatch".into());
        }
        Ok(target)
    }
    fn optional_target(&mut self, expr: &Expr, depth: usize) -> Result<Targets, String> {
        self.step(depth)?;
        if expr.ty == Term::Primitive(Primitive::Never) {
            return Ok(Some(BTreeSet::new()));
        }
        match &expr.kind {
            K::UnsafeScope(body) => self.optional_target(body, depth + 1),
            K::Field { .. } | K::AnonymousField { .. } => {
                if let Some(fields) = self.aggregate_pointers.get(&(expr as *const Expr as usize)) {
                    if let Some(Some(targets)) = fields.get(&vec![]) {
                        for target in targets {
                            *self.fuel = self
                                .fuel
                                .checked_sub(target.path_cost())
                                .ok_or("projected pointer snapshot budget exhausted")?;
                        }
                    }
                    return Ok(fields.get(&vec![]).cloned().flatten());
                }
                let place = self
                    .callable_place(expr)?
                    .ok_or("pointer field requires a known owned input source")?;
                // Bound alternatives before allocating the snapshot, just as
                // for a direct pointer local below.
                if let Some(Some(targets)) = self.environment.field_pointers.get(&place) {
                    for target in targets {
                        *self.fuel = self
                            .fuel
                            .checked_sub(target.path_cost())
                            .ok_or("pointer field target budget exhausted")?;
                    }
                }
                Ok(self
                    .environment
                    .field_pointers
                    .get(&place)
                    .cloned()
                    .flatten())
            }
            K::AddressOf(value) => Ok(Some(BTreeSet::from(
                [self.local_address(value, depth + 1)?],
            ))),
            K::Local(id) => {
                // Charge before copying potentially large alternatives.
                if let Some(Some(targets)) = self.environment.pointers.get(id) {
                    for target in targets {
                        *self.fuel = self
                            .fuel
                            .checked_sub(target.path_cost())
                            .ok_or("memory target set budget exhausted")?;
                    }
                }
                Ok(self.environment.pointers.get(id).cloned().flatten())
            }
            K::ReadPointer(pointer) if self.local_pointer_value(&expr.ty)? => {
                let Some(targets) = self.optional_target(pointer, depth + 1)? else {
                    return Ok(None);
                };
                self.target_work(&targets)?;
                let mut loaded = BTreeSet::new();
                for mut target in targets {
                    let mut ty = self.ty(&expr.ty)?;
                    while let Term::Nominal {
                        declaration,
                        arguments,
                    } = ty
                    {
                        let definition = self
                            .package
                            .and_then(|p| p.newtypes.iter().find(|d| d.declaration == declaration))
                            .ok_or("pointer representation metadata is missing")?;
                        target
                            .fields
                            .push((nir::program::FieldOwner::Nominal(declaration), 0));
                        ty = definition
                            .representation_budgeted(&arguments, &[], self.fuel)
                            .map_err(|e| e.to_string())?;
                    }
                    if target.path_cost() >= 128 {
                        return Err("memory pointer path depth exhausted".into());
                    }
                    target.through.push(std::mem::take(&mut target.fields));
                    loaded.insert(target);
                }
                Ok(Some(self.resolve_local_loads(loaded)?))
            }
            K::ReadPointer(_) => Ok(None),
            K::Return(_) => Ok(Some(BTreeSet::new())),
            K::EffectHandle { .. }
            | K::EffectRequest { .. }
            | K::EffectRun { .. }
            | K::EffectComputation { .. }
            | K::Call { .. }
            | K::Match { .. }
            | K::If { .. }
            | K::NewtypeConvert(_) => {
                let targets = self
                    .expression_results
                    .get(&(expr as *const Expr as usize))
                    .ok_or("memory expression result was not analyzed")?;
                if let Some(targets) = targets {
                    for target in targets {
                        *self.fuel = self
                            .fuel
                            .checked_sub(target.path_cost())
                            .ok_or("memory result budget exhausted")?;
                    }
                }
                Ok(targets.clone())
            }
            // Walk has already checked the entire block and established its
            // immutable local aliases; lexical LocalIds distinguish scopes.
            K::Block {
                result: Some(result),
                ..
            }
            | K::CallPreparation {
                result: Some(result),
                ..
            } => {
                if super::pattern_facts::cannot_complete(expr, self.fuel, 0)? {
                    Ok(Some(BTreeSet::new()))
                } else {
                    self.optional_target(result, depth + 1)
                }
            }
            K::PointerField {
                pointer,
                declaration,
                index,
            } => {
                let targets = self.target(pointer, depth + 1)?;
                self.target_work(&targets)?;
                Ok(Some(
                    targets
                        .into_iter()
                        .map(|mut target| {
                            target
                                .fields
                                .push((nir::program::FieldOwner::Nominal(*declaration), *index));
                            target
                        })
                        .collect(),
                ))
            }
            K::PointerAnonymousField { pointer, index, .. } => {
                let targets = self.target(pointer, depth + 1)?;
                self.target_work(&targets)?;
                Ok(Some(
                    targets
                        .into_iter()
                        .map(|mut target| {
                            target
                                .fields
                                .push((nir::program::FieldOwner::Anonymous, *index));
                            target
                        })
                        .collect(),
                ))
            }
            _ => Err("memory target requires a richer contract".into()),
        }
    }
    fn target(&mut self, expr: &Expr, depth: usize) -> Result<BTreeSet<PointerPlace>, String> {
        self.optional_target(expr, depth)?
            .ok_or("unknown memory target".into())
    }
    /// Resolve owned pointer slots at the current evaluation point. Pointer
    /// stores update known local slots, retaining old targets for alternatives;
    /// external or conditional stores need a richer transition contract.
    fn resolve_local_loads(
        &mut self,
        targets: BTreeSet<PointerPlace>,
    ) -> Result<BTreeSet<PointerPlace>, String> {
        self.target_work(&targets)?;
        let mut work: Vec<_> = targets.into_iter().collect();
        let mut resolved = BTreeSet::new();
        while let Some(target) = work.pop() {
            self.step(target.path_cost())?;
            let PointerRoot::Local { binding, storage } = target.root else {
                resolved.insert(target);
                continue;
            };
            let Some((first, remaining)) = target.through.split_first() else {
                resolved.insert(target);
                continue;
            };
            let (owner, root) = self
                .local_storage
                .get(storage)
                .ok_or("local pointer load storage is missing")?;
            if *owner != binding {
                return Err("local pointer load storage identity mismatch".into());
            }
            let guards = target.conditions.get(&0).map(Vec::as_slice).unwrap_or(&[]);
            let mut traversal = StorageTraversal::new(self.package, root, &[], self.fuel)?;
            traversal.then_guarded(self.package, first, guards, self.fuel)?;
            let ty = traversal.target.ty;
            let mut path = Vec::new();
            for (depth, (_, index)) in first.iter().enumerate() {
                for (_, tag) in guards
                    .iter()
                    .filter(|(container, _)| container.len() == depth)
                {
                    path.extend([VARIANT_FIELD, *tag]);
                }
                path.push(*index);
            }
            if !self.local_pointer_value(&ty)? {
                return Err("local pointer load requires a pointer slot".into());
            }
            let loaded = if path.is_empty() {
                self.environment.pointers.get(&binding)
            } else {
                self.environment.field_pointers.get(&CallablePlace {
                    binding,
                    fields: path,
                })
            }
            .and_then(Option::as_ref)
            .ok_or("local pointer slot origin is unknown")?;
            for origin in loaded {
                *self.fuel = self
                    .fuel
                    .checked_sub(origin.path_cost())
                    .ok_or("local pointer load budget exhausted")?;
            }
            let loaded = loaded.clone();
            let suffix = ParameterPlace {
                parameter: 0,
                input_fields: vec![],
                through: remaining.to_vec(),
                conditions: target
                    .conditions
                    .iter()
                    .filter(|(stage, _)| **stage > 0)
                    .map(|(stage, guards)| (stage - 1, guards.clone()))
                    .collect(),
                fields: target.fields.clone(),
            };
            for mut origin in loaded {
                self.step(suffix.path_cost())?;
                origin.append_resolved_path(&suffix)?;
                work.push(origin);
            }
        }
        Ok(resolved)
    }

    fn record_access(
        &mut self,
        targets: BTreeSet<PointerPlace>,
        write: bool,
        read_value: bool,
        ty: Term,
    ) -> Result<(), String> {
        // Imported call/effect footprints can write a private pointer slot even
        // when no assignment node occurs in this body. Such writes also need
        // cleanup-time transition evidence before value snapshots may suffice.
        if write && self.round.cleanup.is_some() && !self.round.mutates_pointer_storage {
            self.round.mutates_pointer_storage = pointer_input_layout(self.package, &ty, self.fuel)
                .map_or(true, |leaves| !leaves.is_empty());
        }
        let targets = self.resolve_local_loads(targets)?;
        self.target_work(&targets)?;
        for target in targets {
            bound_type(&ty, self.fuel)?;
            if let PointerRoot::Local { binding, storage } = target.root {
                if !target.through.is_empty() {
                    return Err("local pointer-slot loads require a storage contract".into());
                }
                let (owner, root) = self
                    .local_storage
                    .get(storage)
                    .ok_or("local memory root is missing")?;
                if *owner != binding {
                    return Err("local memory storage identity mismatch".into());
                }
                let mut traversal = StorageTraversal::new(self.package, root, &[], self.fuel)?;
                traversal.then_guarded(
                    self.package,
                    &target.fields,
                    target.conditions.get(&0).map(Vec::as_slice).unwrap_or(&[]),
                    self.fuel,
                )?;
                if traversal.target.ty != ty {
                    return Err("local memory access type mismatch".into());
                }
                continue;
            }
            self.accesses.push(Access {
                target: target.external()?,
                write,
                read_value,
                ty: ty.clone(),
            });
        }
        Ok(())
    }
    fn rebase_result(
        &mut self,
        result: Option<&BTreeSet<ParameterPlace>>,
        arguments: &BTreeMap<usize, BTreeMap<Vec<usize>, BTreeSet<PointerPlace>>>,
    ) -> Result<Targets, String> {
        let Some(result) = result else {
            return Ok(None);
        };
        let mut targets = BTreeSet::new();
        for target in result {
            let Some(actual) = arguments
                .get(&target.parameter)
                .and_then(|fields| fields.get(&target.input_fields))
            else {
                return Ok(None);
            };
            self.target_work(actual)?;
            for base in actual {
                *self.fuel = self
                    .fuel
                    .checked_sub(target.path_cost())
                    .ok_or("memory result budget exhausted")?;
                let mut place = base.clone();
                place.append_resolved_path(target)?;
                targets.insert(place);
            }
        }
        Ok(Some(self.resolve_local_loads(targets)?))
    }
    fn merge_pointer_field_return(&mut self, value: &Expr) -> Result<(), String> {
        let ty = self.ty(&value.ty)?;
        let ty = self.effect_completion(&ty)?;
        if matches!(ty, Term::Closure { .. })
            && self
                .local_candidates_for(value)?
                .is_some_and(|ids| ids.is_empty())
        {
            return Ok(());
        }
        if matches!(
            ty,
            Term::Primitive(_) | Term::Pointer(_) | Term::Function { .. } | Term::UnsafeFunction(_)
        ) {
            return Ok(());
        }
        if intrinsic_pointer_value(&ty) || self.local_pointer_value(&ty)? {
            return Ok(());
        }
        let layout = symbolic_pointer_fields(self.package, &ty, self.fuel)?;
        if layout.is_empty() {
            return Ok(());
        }
        let mut snapshot = self.pointer_fields_for(value)?;
        let mut fields = BTreeMap::new();
        for (path, _) in layout {
            self.step(path.len())?;
            let targets = snapshot.remove(&path).unwrap_or(None);
            let targets = targets
                .map(|targets| {
                    targets
                        .into_iter()
                        .map(PointerPlace::external)
                        .collect::<Result<BTreeSet<_>, _>>()
                })
                .transpose()?;
            fields.insert(path, targets);
        }
        self.pointer_field_returns =
            Some(if let Some(previous) = self.pointer_field_returns.take() {
                self.join_pointer_fields(previous, fields)?
            } else {
                fields
            });
        Ok(())
    }
    fn merge_input_code_exit(&mut self) -> Result<(), String> {
        for (place, input) in &self.input_code_slots {
            *self.fuel = self
                .fuel
                .checked_sub(place.fields.len() + input.fields.len() + 1)
                .ok_or("input code exit budget exhausted")?;
            let mut exported = None;
            if let Some(ids) = self.environment.local_candidates.get(place) {
                let mut codes = vec![];
                let mut known = true;
                for id in ids {
                    *self.fuel = self
                        .fuel
                        .checked_sub(1)
                        .ok_or("input code exit budget exhausted")?;
                    if let Some(pointers) = self.borrowed_code_pointers.get(id) {
                        if !pointers.is_empty() {
                            let Some(LocalCode::Closure { definition, .. }) =
                                self.local_code.get(*id)
                            else {
                                return Err("borrowed code exit lacks a closure definition".into());
                            };
                            for (path, _) in
                                symbolic_pointer_fields(self.package, &definition.ty, self.fuel)?
                            {
                                *self.fuel = self
                                    .fuel
                                    .checked_sub(place.fields.len() + path.len() + 1)
                                    .ok_or("input code exit layout budget exhausted")?;
                                let mut fields = place.fields.clone();
                                fields.extend(path);
                                if !self.input_pointer_slots.contains_key(&CallablePlace {
                                    binding: place.binding,
                                    fields,
                                }) {
                                    known = false;
                                    break;
                                }
                            }
                            if !known {
                                break;
                            }
                        }
                    }
                    let code = self
                        .local_code
                        .get(*id)
                        .ok_or("input code exit definition is missing")?;
                    code.charge(self.fuel)?;
                    codes.push(code.clone());
                }
                if known {
                    let mut distinct = vec![];
                    join_prepared_codes(&mut distinct, codes, self.fuel)?;
                    exported = Some(distinct);
                }
            } else if let Some(named) = self.environment.callees.get(place) {
                named.charge(self.fuel)?;
                if matches!(
                    named,
                    CallableResult::Known(_)
                        | CallableResult::Candidates(_)
                        | CallableResult::NoReturn
                ) {
                    exported = Some(
                        named
                            .keys()
                            .into_iter()
                            .cloned()
                            .map(PreparedCode::Named)
                            .collect(),
                    );
                }
            }
            match self.input_code_exits.entry(input.clone()) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(exported);
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    match (entry.get_mut(), exported) {
                        (Some(previous), Some(next)) => {
                            join_prepared_codes(previous, next, self.fuel)?;
                        }
                        (previous, _) => *previous = None,
                    }
                }
            }
        }
        Ok(())
    }

    fn merge_input_pointer_exit(&mut self) -> Result<(), String> {
        for (place, input) in &self.input_pointer_slots {
            *self.fuel = self
                .fuel
                .checked_sub(place.fields.len() + input.fields.len() + 1)
                .ok_or("input pointer exit budget exhausted")?;
            let origins = if place.fields.is_empty() {
                self.environment.pointers.get(&place.binding)
            } else {
                self.environment.field_pointers.get(place)
            }
            .and_then(Option::as_ref);
            let mut external = Some(BTreeSet::new());
            if let Some(origins) = origins {
                for origin in origins {
                    *self.fuel = self
                        .fuel
                        .checked_sub(origin.path_cost())
                        .ok_or("input pointer exit origin budget exhausted")?;
                    if let PointerRoot::Parameter(_) = &origin.root {
                        if let Some(targets) = &mut external {
                            targets.insert(origin.clone().external()?);
                        }
                    } else {
                        // A local target is not a reusable caller-relative
                        // origin. Do not publish its frame identity.
                        external = None;
                    }
                }
            } else {
                external = None;
            }
            match self.input_pointer_exits.entry(input.clone()) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(external);
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    match (entry.get_mut(), external) {
                        (Some(previous), Some(next)) => previous.extend(next),
                        (previous, _) => *previous = None,
                    }
                }
            }
        }
        Ok(())
    }

    fn merge_return(&mut self, targets: Targets) -> Result<(), String> {
        if let Some(targets) = &targets {
            self.target_work(targets)?;
        }
        let targets = targets
            .map(|targets| {
                targets
                    .into_iter()
                    .map(PointerPlace::external)
                    .collect::<Result<BTreeSet<_>, _>>()
            })
            .transpose()?;
        match (&mut self.returns, targets) {
            (Some(previous), Some(targets)) => previous.extend(targets),
            _ => self.returns = None,
        }
        Ok(())
    }
    fn bind_match_subject(
        &mut self,
        pattern: &Pattern,
        pointer: &Targets,
        is_pointer: bool,
        subject_ty: &Term,
        defer_observation: bool,
        depth: usize,
    ) -> Result<(), String> {
        self.step(depth)?;
        match pattern {
            Pattern::OptionalAbsent | Pattern::Wild | Pattern::Unit | Pattern::Literal(_) => Ok(()),
            // Computed results are subsequently traversed by bind_owned_callees,
            // which owns guard/computation effects. This pass installs only
            // subject pointer bindings before that traversal.
            Pattern::Guard { pattern, .. } | Pattern::Then { first: pattern, .. }
                if defer_observation =>
            {
                self.bind_match_subject(pattern, pointer, is_pointer, subject_ty, true, depth + 1)
            }
            Pattern::Or(alternatives) => {
                for alternative in alternatives {
                    self.bind_match_subject(
                        alternative,
                        pointer,
                        is_pointer,
                        subject_ty,
                        defer_observation,
                        depth + 1,
                    )?;
                }
                Ok(())
            }
            Pattern::Remap { pattern, bindings } => {
                self.bind_match_subject(
                    pattern,
                    pointer,
                    is_pointer,
                    subject_ty,
                    defer_observation,
                    depth + 1,
                )?;
                for (_, to) in bindings {
                    bound_type(subject_ty, self.fuel)?;
                    self.locals.insert(*to, subject_ty.clone());
                }
                if is_pointer {
                    for (from, to) in bindings {
                        self.step(depth)?;
                        let Some(targets) = self.environment.pointers.get(from).cloned() else {
                            if defer_observation {
                                continue;
                            }
                            return Err("memory pattern remap source is missing".into());
                        };
                        self.set_pointer(*to, targets)?;
                    }
                }
                Ok(())
            }
            Pattern::Observe { binding, inner } | Pattern::Binding { binding, inner } => {
                bound_type(subject_ty, self.fuel)?;
                self.locals.insert(*binding, subject_ty.clone());
                if is_pointer {
                    if let Some(targets) = pointer {
                        self.target_work(targets)?;
                    }
                    self.set_pointer(*binding, pointer.clone())?;
                }
                let result = self.bind_match_subject(
                    inner,
                    pointer,
                    is_pointer,
                    subject_ty,
                    defer_observation,
                    depth + 1,
                );
                if matches!(pattern, Pattern::Observe { .. }) {
                    self.forget_pattern_alias(*binding)?;
                }
                result
            }
            _ => Err("memory match pattern requires a richer subject contract".into()),
        }
    }
    fn guard_value_type(package: Option<&Package>, ty: &Term) -> bool {
        nir::program::scalar_observation_shape_in_package(package, ty)
            || matches!(ty, Term::Function { .. } | Term::UnsafeFunction(_))
    }
    fn walk_guard(&mut self, expr: &'a Expr, depth: usize) -> Result<(), String> {
        // Validation does not execute or install code evidence. The normal
        // walk owns evaluation order, callable results and memory footprints.
        self.check_guard(expr, depth + 1)?;
        self.walk(expr, depth + 1)
    }
    fn check_guard_pattern(
        &mut self,
        pattern: &'a Pattern,
        bindings: &mut BTreeSet<LocalId>,
        depth: usize,
    ) -> Result<(), String> {
        self.step(depth)?;
        match pattern {
            Pattern::OptionalPresent(inner) | Pattern::ErrorResult { inner, .. } => {
                self.check_guard_pattern(inner, bindings, depth + 1)
            }
            Pattern::CodeEvidence { .. } => Err("unresolved pattern-code evidence".into()),
            Pattern::Observe { binding, inner } => {
                let fresh = bindings.insert(*binding);
                let result = self.check_guard_pattern(inner, bindings, depth + 1);
                if fresh {
                    bindings.remove(binding);
                }
                result
            }

            Pattern::Binding { binding, inner } => {
                bindings.insert(*binding);
                self.check_guard_pattern(inner, bindings, depth + 1)
            }
            Pattern::Then { first, value, next } => {
                self.check_guard_pattern(first, bindings, depth + 1)?;
                if !Self::guard_value_type(self.package, &self.ty(&value.ty)?) {
                    return Err("memory computed observation needs scalar or code result".into());
                }
                self.check_guard(value, depth + 1)?;
                self.check_guard_pattern(next, bindings, depth + 1)
            }
            Pattern::Guard { pattern, condition } => {
                self.check_guard_pattern(pattern, bindings, depth + 1)?;
                self.check_guard(condition, depth + 1)
            }
            Pattern::Remap {
                pattern,
                bindings: mapping,
            } => {
                self.check_guard_pattern(pattern, bindings, depth + 1)?;
                for (_, to) in mapping {
                    self.step(depth)?;
                    bindings.insert(*to);
                }
                Ok(())
            }
            Pattern::Or(fields)
            | Pattern::Tuple(fields)
            | Pattern::TypeTuple(fields)
            | Pattern::NominalType { fields, .. }
            | Pattern::Record { fields, .. }
            | Pattern::AnonymousRecord { fields, .. }
            | Pattern::Variant { fields, .. }
            | Pattern::NormalForm {
                arguments: fields, ..
            } => {
                for field in fields {
                    self.check_guard_pattern(field, bindings, depth + 1)?;
                }
                Ok(())
            }
            Pattern::OptionalAbsent | Pattern::Wild | Pattern::Unit | Pattern::Literal(_) => Ok(()),
        }
    }
    fn check_guard(&mut self, expr: &'a Expr, depth: usize) -> Result<(), String> {
        self.step(depth)?;
        let mut constructed = false;
        if self.package.is_some() && !matches!(expr.kind, K::Block { .. }) {
            nir::visit::expression_pruned(expr, self.fuel, |node, _| {
                if constructed || matches!(node.kind, K::Closure { .. }) {
                    return Ok(false);
                }
                let owned = |ty: &Term| {
                    matches!(
                        ty,
                        Term::Nominal { .. }
                            | Term::Record(_)
                            | Term::Tuple(_)
                            | Term::NormalForm { .. }
                            | Term::ErrorResult { .. }
                            | Term::Optional(_)
                    )
                };
                constructed = matches!(
                    node.kind,
                    K::Tuple(_)
                        | K::AnonymousRecord { .. }
                        | K::Record { .. }
                        | K::Variant { .. }
                        | K::OptionalPresent(_)
                        | K::OptionalAbsent
                ) || matches!(&node.kind, K::Call { arguments, .. } if owned(&node.ty) || arguments.iter().any(|argument| owned(&argument.value.ty)));
                Ok(!constructed)
            })?;
        }
        let pointer_match =
            matches!(expr.kind, K::Match { .. }) && matches!(expr.ty, Term::Pointer(_));
        if (matches!(expr.kind, K::Block { .. }) || pointer_match || constructed)
            && self.package.is_some()
        {
            super::observation_region::check(expr, self.package.unwrap(), self.fuel).map_err(
                |reason| format!("memory guard requires read-only observation: {reason}"),
            )?;
            return Ok(());
        }
        match &expr.kind {
            K::ComptimeValue(_) => self.captured_data(expr, depth + 1),
            K::Literal(_) | K::Unit | K::Local(_) | K::Function(_) | K::FunctionInstance { .. } => {
                Ok(())
            }
            K::RequirePredicate(condition)
            | K::EnsurePredicate(condition)
            | K::CheckPredicate(condition)
                if self.known_bool(condition, 0)? == Some(true) =>
            {
                self.check_guard(condition, depth + 1)
            }
            K::AddressOf(place) => {
                if let Some(reason) = nir::program::pointer_operation_error_in(
                    expr,
                    self.package.and_then(|package| package.target),
                ) {
                    return Err(reason.into());
                }
                self.check_guard(place, depth + 1)
            }
            K::PointerAnonymousField { pointer, .. }
            | K::PointerField { pointer, .. }
            | K::ReadPointer(pointer) => {
                self.check_guard(pointer, depth + 1)?;
                Ok(())
            }
            K::Field { value, .. } | K::AnonymousField { value, .. } => {
                // Owned field reads in a contract are observations too. Check
                // the operand's purity before normal projection validation.
                self.check_guard(value, depth + 1)?;
                Ok(())
            }
            K::Not(value)
            | K::IntegerNegate(value)
            | K::FloatNegate(value)
            | K::BitConvert(value)
            | K::NewtypeConvert(value)
            | K::Snapshot(value)
            | K::TakeBits(value)
            | K::TextQuery { subject: value, .. }
            | K::IntegerRepresentation(value)
            | K::SymbolName(value)
            | K::EnumName(value)
            | K::BitSlice { value, .. } => self.check_guard(value, depth + 1),
            K::BitWindow { value, start, end } => {
                self.check_guard(value, depth + 1)?;
                self.check_guard(start, depth + 1)?;
                self.check_guard(end, depth + 1)
            }
            K::And(left, right) | K::Or(left, right) => {
                self.check_guard(left, depth + 1)?;
                let skip = self.known_bool(left, 0)?.map(|value| {
                    if matches!(expr.kind, K::And(..)) {
                        !value
                    } else {
                        value
                    }
                });
                if skip != Some(true) {
                    self.check_guard(right, depth + 1)?;
                }
                Ok(())
            }
            K::TemplateEqual(left, right)
            | K::ScalarEqual(left, right)
            | K::ScalarLess(left, right)
            | K::ScalarLessEqual(left, right)
            | K::ScalarGreater(left, right)
            | K::ScalarGreaterEqual(left, right)
            | K::BitEqual(left, right)
            | K::BitConcat(left, right)
            | K::BitIndex(left, right)
            | K::IntegerBinary { left, right, .. }
            | K::FloatBinary { left, right, .. } => {
                self.check_guard(left, depth + 1)?;
                self.check_guard(right, depth + 1)
            }
            K::Call {
                callee, arguments, ..
            } => {
                self.check_guard(callee, depth + 1)?;
                let package = self.package;
                let scalar = |ty: &Term| Self::guard_value_type(package, ty);
                let result = self.ty(&expr.ty)?;
                if !scalar(&result) && !matches!(result, Term::Pointer(_)) {
                    return Err(
                        "memory guard call requires a scalar, code or pointer result".into(),
                    );
                }
                for argument in arguments {
                    self.step(depth)?;
                    let ty = self.ty(&argument.value.ty)?;
                    if !scalar(&ty)
                        && !matches!(ty, Term::Pointer(_))
                        && !copy_readable_representation(self.package, &ty, self.fuel)?
                    {
                        return Err(
                            "memory guard argument requires a known readable representation".into(),
                        );
                    }
                    // This checks layout and effects only. The resource pass
                    // must establish Copy or private ownership for delivery.
                    self.check_guard(&argument.value, depth + 1)?;
                }
                Ok(())
            }
            K::Block { statements, result } => {
                let mut bindings = BTreeSet::new();
                let package = self.package;
                let scalar = |ty: &Term| Self::guard_value_type(package, ty);
                for statement in statements {
                    self.step(depth)?;
                    let statement = match &statement.kind {
                        K::Discard(value) => value.as_ref(),
                        _ => statement,
                    };
                    match &statement.kind {
                        K::Let {binding,value} if scalar(&self.ty(&value.ty)?)=> {
                            if bindings.contains(binding) {
                                return Err("memory guard block repeats a local binding".into());
                            }
                            self.check_guard(value,depth+1)?;
                            bindings.insert(*binding);
                        }
                        K::LetPattern {pattern,value,guard,otherwise:None} if scalar(&self.ty(&value.ty)?)=> {
                            self.check_guard(value,depth+1)?;
                            let mut declared=BTreeSet::new();
                            self.check_guard_pattern(pattern,&mut declared,depth+1)?;
                            if let Some(guard)=guard {self.check_guard(guard,depth+1)?;}
                            for binding in declared {
                                self.step(depth)?;
                                if !bindings.insert(binding) {return Err("memory guard repeats a local pattern binding".into());}
                            }
                        }
                        K::Assign {place,value} if place.fields.is_empty() && bindings.contains(&place.root)=> {
                            let ty=self.ty(&value.ty)?;
                            if !scalar(&ty) {
                                return Err("memory guard local assignment type mismatch".into());
                            }
                            self.check_guard(value,depth+1)?;
                        }
                        K::RequirePredicate(condition)|K::EnsurePredicate(condition)|K::CheckPredicate(condition)
                            if self.known_bool(condition, 0)?==Some(true)=>self.check_guard(condition,depth+1)?,
                        _ if scalar(&self.ty(&statement.ty)?) || matches!(statement.kind,K::Unit)=>
                            self.check_guard(statement,depth+1)?,
                        _=>return Err("memory guard requires read-only observation semantics: block changes external storage or creates non-scalar bindings".into()),
                    }
                }
                if let Some(value) = result {
                    self.check_guard(value, depth + 1)?;
                }
                Ok(())
            }
            K::Match { value, arms } => {
                if !Self::guard_value_type(self.package, &self.ty(&value.ty)?)
                    || !Self::guard_value_type(self.package, &self.ty(&expr.ty)?)
                {
                    return Err("memory observed match needs scalar or code values".into());
                }
                self.check_guard(value, depth + 1)?;
                for arm in arms {
                    self.step(depth)?;
                    let (pattern, mut guards) = outer_guards(&arm.pattern);
                    guards.extend(arm.guard.as_ref());
                    let known = known_pattern(pattern, value, 0);
                    if known == Some(false) {
                        continue;
                    }
                    self.check_guard_pattern(pattern, &mut BTreeSet::new(), depth + 1)?;
                    let mut guard_known = Some(true);
                    for guard in guards {
                        self.check_guard(guard, depth + 1)?;
                        match self.known_bool(guard, 0)? {
                            Some(false) => {
                                guard_known = Some(false);
                                break;
                            }
                            None => guard_known = None,
                            _ => {}
                        }
                    }
                    if guard_known != Some(false) {
                        self.check_guard(&arm.body, depth + 1)?;
                    }
                    if known == Some(true) && guard_known == Some(true) {
                        break;
                    }
                }
                Ok(())
            }
            K::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.check_guard(condition, depth + 1)?;
                let known = self.known_bool(condition, 0)?;
                if known != Some(false) {
                    self.check_guard(then_branch, depth + 1)?;
                }
                if known != Some(true) {
                    if let Some(other) = else_branch {
                        self.check_guard(other, depth + 1)?;
                    }
                }
                Ok(())
            }
            _ => Err("memory guard requires read-only observation semantics".into()),
        }
    }
    fn walk_match(
        &mut self,
        expr: &Expr,
        value: &'a Expr,
        arms: &'a [nir::program::MatchArm],
        depth: usize,
    ) -> Result<(), String> {
        self.step(depth)?;
        let subject_ty = self.ty(&value.ty)?;
        let data_subject = effect_data_layout(self.package, &subject_ty, self.fuel, 0)?;
        let pointer_subject = self.local_pointer_value(&subject_ty)?;
        if !pointer_subject && !data_subject {
            if symbolic_pointer_fields(self.package, &subject_ty, self.fuel)?.is_empty() {
                self.pointer_free_owned(&subject_ty, depth + 1)?;
            }
        }
        self.walk(value, depth + 1)?;
        let subject = if pointer_subject {
            self.optional_target(value, depth + 1)?
        } else {
            None
        };
        let subject_callees = self.value_callees(value)?;
        let subject_code = self.local_fields_for(value)?;
        let subject_pointers = self.pointer_fields_for(value)?;
        let base = self.save_environment()?;
        let pointer_result = self.local_pointer_value(&expr.ty)?;
        let callable_result = matches!(
            self.ty(&expr.ty)?,
            Term::Function { .. } | Term::UnsafeFunction(_) | Term::Closure { .. }
        );
        let mut callable = CallableResult::NoReturn;
        let mut closures = Some(BTreeSet::new());
        let mut results = Some(BTreeSet::new());
        let mut continuation = None;
        let mut aggregate = None;
        let mut pointers = None;
        let mut local_fields = None;
        for arm in arms {
            self.step(depth)?;
            let (pattern, mut guards) = outer_guards(&arm.pattern);
            guards.extend(arm.guard.as_ref());
            let known = known_pattern(pattern, value, 0);
            if known == Some(false) {
                continue;
            }
            self.environment = self.clone_environment(&base)?;
            if pointer_subject {
                self.bind_match_subject(pattern, &subject, true, &subject_ty, true, depth + 1)?;
                // Pointer provenance is installed first; the ordinary pattern
                // walk owns nested guards and computed observations, just as
                // it does for a pointer produced by a computed pattern.
                self.bind_owned_callees(
                    pattern,
                    &subject_ty,
                    &subject_callees,
                    &subject_code,
                    depth + 1,
                )?;
            } else if data_subject {
                self.bind_data_pattern(pattern, &subject_ty, depth + 1)?;
            } else {
                self.bind_owned_values(
                    pattern,
                    &subject_ty,
                    &subject_callees,
                    &subject_code,
                    &subject_pointers,
                    true,
                    depth + 1,
                )?;
            }
            let mut guard_known = Some(true);
            for guard in guards {
                self.walk_guard(guard, depth + 1)?;
                match self.known_bool(guard, 0)? {
                    Some(false) => {
                        guard_known = Some(false);
                        break;
                    }
                    None => guard_known = None,
                    _ => (),
                }
            }
            if guard_known == Some(false) {
                continue;
            }
            self.walk(&arm.body, depth + 1)?;
            if !super::pattern_facts::cannot_complete(&arm.body, self.fuel, 0)? {
                if pointer_result {
                    let targets = self.optional_target(&arm.body, depth + 1)?;
                    results = self.join_targets(results, targets)?;
                }
                if callable_result {
                    let key = self.callee_key(&arm.body)?;
                    self.join_callee_result(&mut callable, key)?;
                    let next = self.local_candidates_for(&arm.body)?;
                    closures = self.join_local_candidates(closures, next)?;
                }
                let fields = self.pointer_fields_for(&arm.body)?;
                pointers = Some(if let Some(previous) = pointers {
                    self.join_pointer_fields(previous, fields)?
                } else {
                    fields
                });
                let fields = self.local_fields_for(&arm.body)?;
                local_fields = Some(if let Some(previous) = local_fields {
                    self.join_local_fields(previous, fields)?
                } else {
                    fields
                });
                let fields = self.value_callees(&arm.body)?;
                aggregate = Some(if let Some(previous) = aggregate {
                    self.join_callee_fields(previous, fields)?
                } else {
                    fields
                });
                let branch = self.save_environment()?;
                continuation = Some(if let Some(previous) = continuation {
                    self.join_environment(previous, branch)?
                } else {
                    branch
                });
            }
            if known == Some(true) && guard_known == Some(true) {
                break;
            }
        }
        self.aggregate_pointers
            .insert(expr as *const Expr as usize, pointers.unwrap_or_default());
        self.local_code_fields.insert(
            expr as *const Expr as usize,
            local_fields.unwrap_or_default(),
        );
        self.local_code_results
            .insert(expr as *const Expr as usize, closures);
        self.aggregate_callees
            .insert(expr as *const Expr as usize, aggregate.unwrap_or_default());
        self.callable_results
            .insert(expr as *const Expr as usize, callable);
        self.environment = continuation.unwrap_or(base);
        self.expression_results.insert(
            expr as *const Expr as usize,
            if pointer_result { results } else { None },
        );
        Ok(())
    }
    fn note_owned_cleanup(&mut self, ty: &Term) -> Result<(), String> {
        if self.may_run_cleanup
            || matches!(
                ty,
                Term::Primitive(_)
                    | Term::Pointer(_)
                    | Term::Function { .. }
                    | Term::UnsafeFunction(_)
            )
        {
            return Ok(());
        }
        let possible = if let Some(possible) = self.round.cleanup_protocol_present {
            possible
        } else {
            let mut possible = self.package.is_none();
            if let Some(package) = self.package {
                // Lang-item identity is unavailable here. Every implemented
                // protocol with a drop member is a potential destructor.
                let mut cleanup_interfaces = BTreeSet::new();
                for interface in &package.traits {
                    self.step(0)?;
                    for method in &interface.methods {
                        self.step(0)?;
                        if method.name == "drop" {
                            cleanup_interfaces.insert(interface.declaration);
                            break;
                        }
                    }
                }
                for implementation in &package.implementations {
                    self.step(0)?;
                    if implementation
                        .interface
                        .is_some_and(|interface| cleanup_interfaces.contains(&interface))
                    {
                        possible = true;
                        break;
                    }
                }
            }
            self.round.cleanup_protocol_present = Some(possible);
            possible
        };
        if possible {
            let ty = self.ty(ty)?;
            let ty = self.type_view(&ty)?;
            if !matches!(
                ty,
                Term::Primitive(_)
                    | Term::Pointer(_)
                    | Term::Function { .. }
                    | Term::UnsafeFunction(_)
            ) {
                bound_type(&ty, self.fuel)?;
                if self.round.memory_inert_cleanup_types.contains(&ty)
                    || self.round.cleanup_reads.contains_key(&ty)
                {
                    return Ok(());
                }
                if let Some(cleanup) = self.round.cleanup.as_deref_mut() {
                    if cleanup.is_memory_inert(&ty, self.fuel)? {
                        self.round.memory_inert_cleanup_types.insert(ty);
                        return Ok(());
                    }
                    if let Some(reads) = cleanup.cleanup_reads(&ty, self.fuel)? {
                        for read in &reads {
                            cleanup::charge_read(read, self.fuel)?;
                            if read.write
                                || read.target.parameter != 0
                                || !read.target.input_fields.is_empty()
                            {
                                return Err("cleanup proof requires receiver-relative reads".into());
                            }
                        }
                        self.round.cleanup_reads.insert(ty, reads);
                        return Ok(());
                    }
                }
                self.may_run_cleanup = true;
            }
        }
        Ok(())
    }
    // Snapshot maps and guarded projection temporaries belong only to reads,
    // not to every recursive AST frame (including deeply inlined control flow).
    #[inline(never)]
    fn walk_read_pointer(
        &mut self,
        expr: &'a Expr,
        pointer: &'a Expr,
        depth: usize,
    ) -> Result<(), String> {
        let ty = self.ty(&expr.ty)?;
        if !copy_readable_representation(self.package, &ty, self.fuel)? {
            return Err("owned read requires a richer contract".into());
        }
        self.walk(pointer, depth + 1)?;
        let targets = self.target(pointer, depth + 1)?;
        let layout = symbolic_pointer_fields(self.package, &ty, self.fuel)?;
        if !self.local_pointer_value(&ty)? && !layout.is_empty() {
            let mut loaded = BTreeMap::new();
            for path in layout.keys() {
                let mut combined = Some(BTreeSet::new());
                for target in &targets {
                    self.step(target.path_cost().saturating_add(path.len()))?;
                    let values = match target.root {
                        PointerRoot::Local { binding, .. } if target.through.is_empty() => {
                            let mut fields: Vec<_> =
                                target.fields.iter().map(|(_, index)| *index).collect();
                            fields.extend(path);
                            self.environment
                                .field_pointers
                                .get(&CallablePlace { binding, fields })
                                .cloned()
                                .flatten()
                        }
                        _ => {
                            let projection =
                                project_owned_storage(self.package, &ty, path, self.fuel)?;
                            let mut slot = target.clone();
                            let prefix: Vec<_> =
                                slot.fields.iter().map(|(_, index)| *index).collect();
                            if !projection.guards.is_empty() {
                                let guards = slot.conditions.entry(slot.through.len()).or_default();
                                for (path, tag) in projection.guards {
                                    let mut rebased = prefix.clone();
                                    rebased.extend(path);
                                    guards.push((rebased, tag));
                                }
                            }
                            slot.fields.extend(projection.fields);
                            self.record_access(
                                BTreeSet::from([slot.clone()]),
                                false,
                                true,
                                projection.ty,
                            )?;
                            slot.through.push(std::mem::take(&mut slot.fields));
                            Some(BTreeSet::from([slot]))
                        }
                    };
                    if let (Some(accumulated), Some(values)) = (&mut combined, values) {
                        self.target_work(&values)?;
                        accumulated.extend(values);
                    } else {
                        combined = None;
                    }
                }
                loaded.insert(path.clone(), combined);
            }
            self.aggregate_pointers
                .insert(expr as *const Expr as usize, loaded);
        }
        if !owned_callable_bottom(self.package, &ty, self.fuel, true)?.is_empty() {
            let mut code = None;
            let mut named = None;
            for target in &targets {
                self.step(target.path_cost())?;
                let mut local_fields = BTreeMap::new();
                let mut named_fields = BTreeMap::new();
                if let PointerRoot::Local { binding, .. } = target.root {
                    if target.through.is_empty() {
                        let prefix: Vec<_> =
                            target.fields.iter().map(|(_, index)| *index).collect();
                        let subtree = CallablePlace {
                            binding,
                            fields: prefix.clone(),
                        };
                        for (place, candidates) in
                            callable_subtree(&self.environment.local_candidates, &subtree)
                        {
                            *self.fuel = self
                                .fuel
                                .checked_sub(place.fields.len() + candidates.len() + 1)
                                .ok_or("loaded code snapshot budget exhausted")?;
                            if place.binding == binding {
                                if let Some(path) = place.fields.strip_prefix(prefix.as_slice()) {
                                    local_fields.insert(path.to_vec(), candidates.clone());
                                }
                            }
                        }
                        for (place, candidates) in
                            callable_subtree(&self.environment.callees, &subtree)
                        {
                            *self.fuel = self
                                .fuel
                                .checked_sub(place.fields.len() + 1)
                                .ok_or("loaded callable snapshot budget exhausted")?;
                            if place.binding == binding {
                                if let Some(path) = place.fields.strip_prefix(prefix.as_slice()) {
                                    candidates.charge(self.fuel)?;
                                    named_fields.insert(path.to_vec(), candidates.clone());
                                }
                            }
                        }
                    }
                }
                if matches!(target.root, PointerRoot::Parameter(_)) {
                    for path in owned_callable_bottom(self.package, &ty, self.fuel, true)?.keys() {
                        let projection = project_owned_storage(self.package, &ty, path, self.fuel)?;
                        let slot = borrowed_code::project_slot(target, &projection);
                        if let Some(value) = self.borrowed_callable_slots.get(&slot) {
                            value.charge(self.fuel)?;
                            named_fields.insert(path.clone(), value.clone());
                        }
                    }
                }
                code = Some(match code {
                    Some(previous) => self.join_local_fields(previous, local_fields)?,
                    None => local_fields,
                });
                named = Some(match named {
                    Some(previous) => self.join_callee_fields(previous, named_fields)?,
                    None => named_fields,
                });
            }
            self.local_code_fields
                .insert(expr as *const Expr as usize, code.unwrap_or_default());
            let named = named.unwrap_or_default();
            if let Some(result) = named.get(&vec![]) {
                result.charge(self.fuel)?;
                self.callable_results
                    .insert(expr as *const Expr as usize, result.clone());
            }
            self.aggregate_callees
                .insert(expr as *const Expr as usize, named);
        }
        self.record_access(targets, false, true, ty)?;
        Ok(())
    }
    #[inline(never)]
    fn walk_error_transport(
        &mut self,
        expr: &'a Expr,
        value: &'a Expr,
        depth: usize,
    ) -> Result<(), String> {
        let mapping = error_tag_mapping(expr, self.package, self.fuel)?;
        self.walk(value, depth + 1)?;
        fn remap<T>(
            values: BTreeMap<Vec<usize>, T>,
            mapping: &nir::error_results::RepresentationTransport,
            fuel: &mut usize,
        ) -> Result<BTreeMap<Vec<usize>, T>, String> {
            let mut output = BTreeMap::new();
            for (mut path, value) in values {
                remap_error_path(&mut path, mapping, fuel)?;
                output.insert(path, value);
            }
            Ok(output)
        }
        let ty = self.ty(&expr.ty)?;
        let mut pointers = symbolic_pointer_fields(self.package, &ty, self.fuel)?
            .into_keys()
            .map(|path| (path, Some(BTreeSet::new())))
            .collect::<BTreeMap<_, _>>();
        let source_ty = self.ty(&value.ty)?;
        for mut path in symbolic_pointer_fields(self.package, &source_ty, self.fuel)?.into_keys() {
            remap_error_path(&mut path, &mapping, self.fuel)?;
            pointers.insert(path, None);
        }
        pointers.extend(remap(self.pointer_fields_for(value)?, &mapping, self.fuel)?);
        let mut callees = result_callable_bottom(self.package, &ty, self.fuel)?;
        for (mut path, _) in result_callable_bottom(self.package, &source_ty, self.fuel)? {
            remap_error_path(&mut path, &mapping, self.fuel)?;
            callees.insert(path, CallableResult::Unknown);
        }
        callees.extend(remap(self.value_callees(value)?, &mapping, self.fuel)?);
        let mut local: LocalCodeFields = owned_callable_bottom(self.package, &ty, self.fuel, true)?
            .into_keys()
            .map(|path| (path, BTreeSet::new()))
            .collect();
        // Unknown active source leaves must stay absent, not bottom.
        for (mut path, _) in owned_callable_bottom(self.package, &source_ty, self.fuel, true)? {
            remap_error_path(&mut path, &mapping, self.fuel)?;
            local.remove(&path);
        }
        local.extend(remap(self.local_fields_for(value)?, &mapping, self.fuel)?);
        self.aggregate_pointers
            .insert(expr as *const Expr as usize, pointers);
        self.aggregate_callees
            .insert(expr as *const Expr as usize, callees);
        self.local_code_fields
            .insert(expr as *const Expr as usize, local);
        Ok(())
    }

    fn walk(&mut self, expr: &'a Expr, depth: usize) -> Result<(), String> {
        self.note_cleanup_pointer_mutation(expr)?;
        self.walk_inner(expr, depth)?;
        self.record_value_cleanup_reads(expr)
    }
    fn walk_inner(&mut self, expr: &'a Expr, depth: usize) -> Result<(), String> {
        self.step(depth)?;
        self.note_owned_cleanup(&expr.ty)?;
        if let Some(error) = nir::program::pointer_operation_error_in(
            expr,
            self.package.and_then(|package| package.target),
        ) {
            return Err(error.into());
        }
        match &expr.kind {
            K::Block { .. } | K::CallPreparation { .. } | K::If { .. } => {
                self.walk_control(expr, depth)
            }
            K::Match { value, arms } => self.walk_match(expr, value, arms, depth + 1),
            K::ReadPointer(pointer) => self.walk_read_pointer(expr, pointer, depth),
            _ => self.walk_operation(expr, depth),
        }
    }
    #[inline(never)]
    fn walk_control(&mut self, expr: &'a Expr, depth: usize) -> Result<(), String> {
        match &expr.kind {
            K::Block { statements, result } | K::CallPreparation { statements, result } => {
                let mut live = true;
                for statement in statements {
                    self.walk(statement, depth + 1)?;
                    if super::pattern_facts::cannot_complete(statement, self.fuel, 0)? {
                        live = false;
                        break;
                    }
                }
                let mut key = if live {
                    CallableResult::Unknown
                } else {
                    CallableResult::NoReturn
                };
                if let Some(result) = result.as_ref().filter(|_| live) {
                    self.walk(result, depth + 1)?;
                    if matches!(
                        self.ty(&expr.ty)?,
                        Term::Function { .. } | Term::UnsafeFunction(_)
                    ) {
                        key = self.callee_key(result)?;
                    }
                    let values = self.value_callees(result)?;
                    self.aggregate_callees
                        .insert(expr as *const Expr as usize, values);
                    let fields = self.local_fields_for(result)?;
                    self.local_code_fields
                        .insert(expr as *const Expr as usize, fields);
                    let closures = self.local_candidates_for(result)?;
                    self.local_code_results
                        .insert(expr as *const Expr as usize, closures);
                }
                self.callable_results
                    .insert(expr as *const Expr as usize, key);
            }
            K::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.walk(condition, depth + 1)?;
                let before = self.save_environment()?;
                let known = self.known_bool(condition, 0)?;
                let pointer_result = self.local_pointer_value(&expr.ty)?;
                let callable_result = matches!(
                    self.ty(&expr.ty)?,
                    Term::Function { .. } | Term::UnsafeFunction(_) | Term::Closure { .. }
                );
                let mut callable = CallableResult::NoReturn;
                let mut closures = Some(BTreeSet::new());
                let mut results = Some(BTreeSet::new());
                let mut continuation = None;
                let mut aggregate = None;
                let mut pointers = None;
                let mut local_fields = None;
                // Capture each result while its branch environment is active.
                // A returning branch contributes accesses/explicit returns, but
                // cannot change the environment after this conditional.
                for (selected, branch) in [
                    (true, Some(then_branch.as_ref())),
                    (false, else_branch.as_deref()),
                ] {
                    if known.is_some_and(|value| value != selected) {
                        continue;
                    }
                    self.environment = self.clone_environment(&before)?;
                    if let Some(branch) = branch {
                        self.walk(branch, depth + 1)?;
                        if super::pattern_facts::cannot_complete(branch, self.fuel, 0)? {
                            continue;
                        }
                        if pointer_result {
                            let targets = self.optional_target(branch, depth + 1)?;
                            results = self.join_targets(results, targets)?;
                        }
                    }
                    if callable_result {
                        let key = match branch {
                            Some(branch) => self.callee_key(branch)?,
                            None => CallableResult::Unknown,
                        };
                        self.join_callee_result(&mut callable, key)?;
                        let next = if let Some(branch) = branch {
                            self.local_candidates_for(branch)?
                        } else {
                            None
                        };
                        closures = self.join_local_candidates(closures, next)?;
                    }
                    let fields = if let Some(branch) = branch {
                        self.pointer_fields_for(branch)?
                    } else {
                        BTreeMap::new()
                    };
                    pointers = Some(if let Some(previous) = pointers {
                        self.join_pointer_fields(previous, fields)?
                    } else {
                        fields
                    });
                    let fields = if let Some(branch) = branch {
                        self.local_fields_for(branch)?
                    } else {
                        BTreeMap::new()
                    };
                    local_fields = Some(if let Some(previous) = local_fields {
                        self.join_local_fields(previous, fields)?
                    } else {
                        fields
                    });
                    let fields = if let Some(branch) = branch {
                        self.value_callees(branch)?
                    } else {
                        BTreeMap::new()
                    };
                    aggregate = Some(if let Some(previous) = aggregate {
                        self.join_callee_fields(previous, fields)?
                    } else {
                        fields
                    });
                    let environment = self.save_environment()?;
                    continuation = Some(if let Some(previous) = continuation {
                        self.join_environment(previous, environment)?
                    } else {
                        environment
                    });
                }
                self.aggregate_pointers
                    .insert(expr as *const Expr as usize, pointers.unwrap_or_default());
                self.local_code_fields.insert(
                    expr as *const Expr as usize,
                    local_fields.unwrap_or_default(),
                );
                self.local_code_results
                    .insert(expr as *const Expr as usize, closures);
                self.aggregate_callees
                    .insert(expr as *const Expr as usize, aggregate.unwrap_or_default());
                self.callable_results
                    .insert(expr as *const Expr as usize, callable);
                self.environment = continuation.unwrap_or(before);
                self.expression_results.insert(
                    expr as *const Expr as usize,
                    if pointer_result { results } else { None },
                );
            }
            _ => unreachable!("control dispatcher"),
        }
        Ok(())
    }
    #[inline(never)]
    fn walk_operation(&mut self, expr: &'a Expr, depth: usize) -> Result<(), String> {
        match &expr.kind {
            K::AddressOf(value) => {
                self.local_address(value, depth + 1)?;
            }
            K::Intrinsic { inputs, .. } => {
                nir::intrinsics::checked(expr)
                    .map_err(|_| "intrinsic memory behavior requires a valid target protocol")?;
                for input in inputs {
                    self.walk(input, depth + 1)?;
                }
            }
            K::EffectRequest { .. }
            | K::EffectComputation { .. }
            | K::EffectRun { .. }
            | K::EffectHandle { .. }
            | K::EffectResume(..) => self.walk_effect(expr, depth + 1)?,
            K::Match { value, arms } => self.walk_match(expr, value, arms, depth + 1)?,
            // A named code value carries no environment and does not execute
            // its body. Invocation still takes the checked call-summary path.
            K::Function(_) | K::FunctionInstance { .. } | K::TraitMethod { .. } => (),
            K::Closure { captures, .. } => {
                let ty = self.ty(&expr.ty)?;
                let mut pointer_captures = BTreeMap::new();
                let environment = match &ty {
                    Term::Closure { environment, .. } => environment.as_slice(),
                    Term::Function { .. } | Term::UnsafeFunction(_) => &[][..],
                    _ => return Err("closure construction requires a callable type".into()),
                };
                if environment.len() != captures.len() {
                    return Err("closure memory layout disagrees with captures".into());
                }
                let mut captured_code = BTreeMap::new();
                let mut prepared_captures = BTreeMap::new();
                let mut environment_pointers = BTreeMap::new();
                let mut pending_capture = false;
                for (index, (capture, expected)) in captures.iter().zip(environment).enumerate() {
                    if &self.ty(&capture.ty)? != expected {
                        return Err("closure memory capture type mismatch".into());
                    }
                    if let Some(value) = &capture.value {
                        if &self.ty(&value.ty)? != expected {
                            return Err("closure memory initializer type mismatch".into());
                        }
                        self.walk(value, depth + 1)?;
                    } else if self.locals.get(&capture.binding) != Some(expected) {
                        return Err("closure memory capture is not in scope".into());
                    }
                    let fields = self.capture_pointer_fields(capture, depth)?;
                    for (path, targets) in &fields {
                        self.step(path.len())?;
                        if let Some(targets) = targets {
                            self.target_work(targets)?;
                        }
                        let mut environment_path = vec![index];
                        environment_path.extend(path);
                        environment_pointers.insert(environment_path, targets.clone());
                    }
                    if !fields.is_empty() {
                        pointer_captures.insert(capture.binding, fields);
                    }
                    pending_capture |= self.snapshot_capture_code(
                        capture,
                        &mut captured_code,
                        &mut prepared_captures,
                    )?;
                }
                self.charge_capture_code(&captured_code)?;
                if pending_capture {
                    // A provisional factory result has no normal code value
                    // yet. Preserve bottom until the worklist revisits it;
                    // turning the capture into unknown loses its dependency.
                    self.local_code_results
                        .insert(expr as *const Expr as usize, Some(BTreeSet::new()));
                    return Ok(());
                }
                self.remember_closure(
                    expr,
                    captured_code,
                    std::sync::Arc::new(prepared_captures),
                    pointer_captures,
                )?;
                self.aggregate_pointers
                    .insert(expr as *const Expr as usize, environment_pointers);
                // Building code does not invoke its body. A nonempty environment
                // can still retire captured owners; without a cleanup contract
                // that remains a possible rejection at scope exit.
                self.may_reject |= !captures.is_empty();
            }
            K::ComptimeValue(_) => self.captured_data(expr, depth + 1)?,
            K::Unit | K::Literal(_) | K::Local(_) | K::Break(_) | K::Continue(_) => (),
            K::NewtypeConvert(inner) => {
                let package = self
                    .package
                    .ok_or("newtype memory conversion requires metadata")?;
                let source = self.ty(&inner.ty)?;
                let target = self.ty(&expr.ty)?;
                let conversion =
                    nir::newtypes::conversion(&package.newtypes, &source, &target, &[], self.fuel)
                        .map_err(|error| error.to_string())?
                        .ok_or("invalid newtype memory conversion edge")?;
                let pointer = self.local_pointer_value(&target)?;
                if !pointer {
                    let view = self.type_view(&target)?;
                    symbolic_pointer_fields(self.package, &view, self.fuel)?;
                }
                self.walk(inner, depth + 1)?;
                if pointer {
                    let targets = self.optional_target(inner, depth + 1)?;
                    self.expression_results
                        .insert(expr as *const Expr as usize, targets);
                }
                fn remap<T>(
                    values: BTreeMap<Vec<usize>, T>,
                    direction: nir::newtypes::Direction,
                    fuel: &mut usize,
                ) -> Result<BTreeMap<Vec<usize>, T>, String> {
                    let mut result = BTreeMap::new();
                    for (mut path, value) in values {
                        *fuel = fuel
                            .checked_sub(path.len().saturating_add(1))
                            .ok_or("newtype memory origin budget exhausted")?;
                        match direction {
                            nir::newtypes::Direction::Wrap => path.insert(0, 0),
                            nir::newtypes::Direction::Unwrap => {
                                if path.first() != Some(&0) {
                                    return Err("invalid newtype memory origin".into());
                                }
                                path.remove(0);
                            }
                        }
                        result.insert(path, value);
                    }
                    Ok(result)
                }
                if !pointer {
                    let fields = self.pointer_fields_for(inner)?;
                    let fields = remap(fields, conversion.direction(), self.fuel)?;
                    self.aggregate_pointers
                        .insert(expr as *const Expr as usize, fields);
                }
                let values = self.value_callees(inner)?;
                let locals = self.local_fields_for(inner)?;
                let values = remap(values, conversion.direction(), self.fuel)?;
                let locals = remap(locals, conversion.direction(), self.fuel)?;
                if let Some(value) = values.get(&vec![]) {
                    value.charge(self.fuel)?;
                    self.callable_results
                        .insert(expr as *const Expr as usize, value.clone());
                }
                self.aggregate_callees
                    .insert(expr as *const Expr as usize, values);
                self.local_code_fields
                    .insert(expr as *const Expr as usize, locals);
            }
            K::ErrorConvert {
                conversion:
                    nir::error_results::Conversion::Embed | nir::error_results::Conversion::Product,
                value,
            } => {
                self.walk_error_transport(expr, value, depth)?;
            }
            K::AnonymousRecord { .. }
            | K::Record { .. }
            | K::Variant { .. }
            | K::OptionalPresent(_)
            | K::OptionalAbsent
            | K::Tuple(_)
            | K::ErrorConvert { .. } => {
                let (fields, variant) = owned_constructor(expr, self.fuel)?;
                let ty = self.ty(&expr.ty)?;
                let view = self.type_view(&ty)?;
                let mut pointers = symbolic_pointer_fields(self.package, &view, self.fuel)?
                    .into_keys()
                    .map(|path| {
                        (
                            path,
                            if variant.is_some() {
                                Some(BTreeSet::new())
                            } else {
                                None
                            },
                        )
                    })
                    .collect::<BTreeMap<_, _>>();
                let mut values = result_callable_bottom(self.package, &ty, self.fuel)?;
                let mut local_fields: LocalCodeFields =
                    owned_callable_bottom(self.package, &ty, self.fuel, true)?
                        .into_keys()
                        .map(|path| (path, BTreeSet::new()))
                        .collect();
                for (index, field) in fields.iter().enumerate() {
                    let prefix = if let Some(variant) = variant {
                        vec![VARIANT_FIELD, variant, index]
                    } else {
                        vec![index]
                    };
                    self.walk(field, depth + 1)?;
                    let fields = if self.local_pointer_value(&field.ty)? {
                        BTreeMap::from([(vec![], self.optional_target(field, depth + 1)?)])
                    } else {
                        self.pointer_fields_for(field)?
                    };
                    for (path, targets) in fields {
                        self.step(prefix.len() + path.len())?;
                        let mut destination = prefix.clone();
                        destination.extend(path);
                        pointers.insert(destination, targets);
                    }

                    // Only inactive variants keep bottom. An active field with
                    // missing provenance is unknown, never an impossible value.
                    self.charge_local_fields(&local_fields)?;
                    local_fields.retain(|path, _| !path.starts_with(&prefix));
                    let field_ty = self.ty(&field.ty)?;
                    for (path, _) in result_callable_bottom(self.package, &field_ty, self.fuel)? {
                        let mut path_prefix = prefix.clone();
                        path_prefix.extend(path);
                        values.insert(path_prefix, CallableResult::Unknown);
                    }
                    // Snapshot before later initializers can replace the source.
                    for (path, keys) in self.local_fields_for(field)? {
                        let mut path_prefix = prefix.clone();
                        path_prefix.extend(path);
                        local_fields.insert(path_prefix, keys);
                    }
                    for (path, key) in self.value_callees(field)? {
                        let mut path_prefix = prefix.clone();
                        path_prefix.extend(path);
                        values.insert(path_prefix, key);
                    }
                }
                self.aggregate_pointers
                    .insert(expr as *const Expr as usize, pointers);
                self.aggregate_callees
                    .insert(expr as *const Expr as usize, values);
                self.local_code_fields
                    .insert(expr as *const Expr as usize, local_fields);
                // Private scalar return storage has no user retirement code.
                // Its payload computations were walked above; constructing the
                // pending/ready tag must not invent a rejection path.
                let scalar_packet = matches!(&ty,Term::Nominal {declaration,..}
                    if self.package.is_some_and(|package|package.internal_return_packets.contains(declaration)))
                    && owned_variants(self.package, &ty, self.fuel)?
                        .iter()
                        .all(|variant| {
                            variant.fields.iter().all(|field| {
                                matches!(field, Term::Primitive(_))
                                    && intrinsic_pointer_value(field)
                                    || matches!(
                                        field,
                                        Term::Primitive(Primitive::Void | Primitive::Never)
                                    )
                            })
                        });
                if !scalar_packet {
                    // Payload evaluation was checked above. For ordinary owned
                    // aggregates, retirement is the remaining rejection source;
                    // use the captured cleanup selection when one is available.
                    self.may_reject |= match self.round.cleanup.as_deref_mut() {
                        Some(cleanup) => cleanup.may_reject(&ty, self.fuel)?,
                        None => true,
                    };
                }
            }
            K::Field { value, index, .. } | K::AnonymousField { value, index, .. } => {
                self.walk(value, depth + 1)?;
                let parent = self.ty(&value.ty)?;
                if matches!(&parent, Term::Nominal {declaration, ..}
                    if self.package.is_some_and(|package| package.newtypes.iter()
                        .any(|definition| definition.declaration == *declaration)))
                {
                    return Err("newtype representation is not a source field".into());
                }
                if let K::Field { declaration, .. } = &expr.kind {
                    if !matches!(&parent,Term::Nominal {declaration:actual,..} if actual==declaration)
                    {
                        return Err("memory field projection declaration mismatch".into());
                    }
                }
                let mut field = owned_path_type(self.package, &parent, &[*index], self.fuel)?;
                let actual = self.ty(&expr.ty)?;
                if field != actual {
                    field = self.completed_type_view(&field)?;
                }
                if field != actual {
                    return Err("memory field projection type mismatch".into());
                }
                let view = self.type_view(&field)?;
                symbolic_pointer_fields(self.package, &view, self.fuel)?;
                let mut pointers = BTreeMap::new();
                for (path, targets) in self.pointer_fields_for(value)? {
                    self.step(path.len())?;
                    if path.first() == Some(index) {
                        pointers.insert(path[1..].to_vec(), targets);
                    }
                }
                self.aggregate_pointers
                    .insert(expr as *const Expr as usize, pointers);
                let mut projected = BTreeMap::new();
                for (path, key) in self.value_callees(value)? {
                    if path.first() == Some(index) {
                        projected.insert(path[1..].to_vec(), key);
                    }
                }
                self.aggregate_callees
                    .insert(expr as *const Expr as usize, projected);
                let mut projected = BTreeMap::new();
                for (path, keys) in self.local_fields_for(value)? {
                    if path.first() == Some(index) {
                        projected.insert(path[1..].to_vec(), keys);
                    }
                }
                self.local_code_fields
                    .insert(expr as *const Expr as usize, projected);
            }
            K::ReadPointer(pointer) => self.walk_read_pointer(expr, pointer, depth)?,
            K::BitWritePointer {
                pointer,
                selections,
                value,
            } => {
                let Term::Pointer(storage) = self.ty(&pointer.ty)? else {
                    return Err("bit writeback requires a pointer".into());
                };
                *self.fuel = self
                    .fuel
                    .checked_sub(selections.len())
                    .ok_or("bit selection memory budget exhausted")?;
                if let Some(reason) = nir::program::bit_update_error_in(
                    &storage,
                    selections,
                    &self.ty(&value.ty)?,
                    &self.ty(&expr.ty)?,
                    self.package.and_then(|package| package.target),
                ) {
                    return Err(reason.into());
                }
                self.walk(pointer, depth + 1)?;
                let target = self.target(pointer, depth + 1)?;
                for input in selections
                    .iter()
                    .flat_map(nir::program::BitSelection::expressions)
                {
                    self.walk(input, depth + 1)?;
                }
                self.walk(value, depth + 1)?;
                self.record_access(target, true, true, *storage)?;
            }
            K::WritePointer { pointer, value } => {
                let ty = self.ty(&value.ty)?;
                let code_storage =
                    !owned_callable_bottom(self.package, &ty, self.fuel, true)?.is_empty();
                let pointer_layout = pointer_input_layout(self.package, &ty, self.fuel)?;
                let aggregate_pointers =
                    !pointer_layout.is_empty() && !self.local_pointer_value(&ty)?;
                if code_storage || aggregate_pointers {
                    if !copy_readable_representation(self.package, &ty, self.fuel)? {
                        return Err(
                            "code pointer replacement requires a callable transition contract"
                                .into(),
                        );
                    }
                    self.walk(pointer, depth + 1)?;
                    let targets = self.target(pointer, depth + 1)?;
                    let targets = self.resolve_local_loads(targets)?;
                    self.walk(value, depth + 1)?;
                    let code = self.value_callees(value)?;
                    let local_code = self.local_fields_for(value)?;
                    let pointers = self.pointer_fields_for(value)?;
                    self.write_local_value_slots(
                        targets,
                        code,
                        local_code,
                        pointers,
                        ty,
                        expr.source,
                    )?;
                    return Ok(());
                }
                if self.local_pointer_value(&ty)? {
                    self.walk(pointer, depth + 1)?;
                    let targets = self.target(pointer, depth + 1)?;
                    // The destination is evaluated before the RHS, which may
                    // itself change pointer slots used to compute this address.
                    let targets = self.resolve_local_loads(targets)?;
                    self.walk(value, depth + 1)?;
                    let values = self.optional_target(value, depth + 1)?;
                    self.write_local_pointer_slot(targets, values, ty, expr.source)?;
                    return Ok(());
                }
                if !readable_representation(self.package, &ty, self.fuel)?
                    || self.local_pointer_value(&ty)?
                {
                    return Err("pointer store or owned write requires a richer contract".into());
                }
                self.walk(pointer, depth + 1)?;
                let target = self.target(pointer, depth + 1)?;
                self.walk(value, depth + 1)?;
                self.record_access(target, true, true, ty)?;
            }
            K::PointerAnonymousField { pointer, .. } | K::PointerField { pointer, .. } => {
                self.walk(pointer, depth + 1)?;
                let target = self.target(expr, depth + 1)?;
                let Term::Pointer(ty) = &expr.ty else {
                    return Err("field pointer type mismatch".into());
                };
                let ty = self.ty(ty)?;
                self.record_access(target, false, false, ty)?;
            }
            K::Call {
                callee,
                arguments,
                parameter_bindings,
            } => {
                if self.immediate_borrowed_call(
                    expr,
                    callee,
                    arguments,
                    parameter_bindings,
                    depth + 1,
                )? {
                    return Ok(());
                }
                let package = self
                    .package
                    .ok_or("nested call requires a package memory context")?;
                self.walk(callee, depth + 1)?;
                if super::pattern_facts::cannot_complete(callee, self.fuel, 0)? {
                    return Ok(());
                }
                let mut closures = Vec::new();
                let mut named = Vec::new();
                let candidates = self.local_candidates_for(callee)?;
                let receiver_place = self.callable_place(callee)?;
                *self.fuel = self
                    .fuel
                    .checked_sub(candidates.as_ref().map_or(0, BTreeSet::len))
                    .ok_or("closure receiver snapshot budget exhausted")?;
                let receiver_candidates = candidates.clone();
                let mut post_candidates = BTreeSet::new();
                let code_bottom = candidates.as_ref().is_some_and(BTreeSet::is_empty);
                for id in candidates.unwrap_or_default() {
                    self.step(depth)?;
                    let pointer_captures = self
                        .borrowed_code_pointers
                        .get(&id)
                        .cloned()
                        .unwrap_or_default();
                    charge_pointer_captures(&pointer_captures, self.fuel)?;
                    match self
                        .local_code
                        .get(id)
                        .ok_or("local callable memory definition is missing")?
                    {
                        LocalCode::Closure {
                            definition,
                            captures,
                            prepared_captures,
                        } => closures.push((
                            id,
                            definition.clone(),
                            captures.clone(),
                            prepared_captures.clone(),
                            pointer_captures,
                        )),
                        LocalCode::Named(key) => {
                            post_candidates.insert(id);
                            for argument in &key.arguments {
                                bound_type(argument, self.fuel)?;
                            }
                            named.push(key.clone());
                        }
                    }
                }
                let keys = if !closures.is_empty() || !named.is_empty() {
                    named
                } else {
                    match if code_bottom {
                        CallableResult::NoReturn
                    } else {
                        self.callee_key(callee)?
                    } {
                        CallableResult::Known(key) => vec![key],
                        CallableResult::Candidates(keys) if !keys.is_empty() => keys,
                        CallableResult::Candidates(_) => {
                            return Err("empty callable candidate set".into());
                        }
                        CallableResult::Unknown | CallableResult::Forwarded { .. } => {
                            return Err("dynamic call requires a richer memory contract".into());
                        }
                        CallableResult::NoReturn => {
                            // This provisional path has no normal callee value yet.
                            // Dependencies discovered while evaluating the callee
                            // remain queued and will cause another worklist round.
                            self.install_no_return_call(expr)?;
                            return Ok(());
                        }
                    }
                };
                if keys.len() + closures.len() > 1 && arguments.iter().any(|arg| arg.is_default) {
                    return Err("multi-target defaults require a shared parameter contract".into());
                }
                let signature = self.ty(callee.ty.callable_signature())?;
                let recovered;
                let parameter_bindings = if parameter_bindings.is_empty()
                    && !matches!(callee.kind, K::Function(_) | K::FunctionInstance { .. })
                {
                    recovered = if let Some((_, closure, _, _, _)) = closures.first() {
                        let K::Closure { parameters, .. } = &closure.kind else {
                            unreachable!()
                        };
                        parameters.clone()
                    } else {
                        package
                            .runtime_body(keys[0].declaration, &keys[0].arguments)
                            .ok_or("memory callee body is missing")?
                            .0
                            .parameters
                            .clone()
                    };
                    recovered.as_slice()
                } else {
                    parameter_bindings.as_slice()
                };

                let Term::Function { ref parameters, .. } = signature else {
                    return Err("memory call lacks a function signature".into());
                };
                validate_argument_plan(arguments, parameter_bindings, parameters.len())?;
                if parameter_bindings.len() != parameters.len()
                    || parameter_bindings.iter().collect::<BTreeSet<_>>().len()
                        != parameter_bindings.len()
                {
                    return Err(
                        "memory call requires a complete unique parameter binding frame".into(),
                    );
                }
                let mut targets = BTreeMap::new();
                let mut prepared = BTreeMap::new();
                let mut prepared_types = BTreeMap::new();
                let mut prepared_fields: BTreeMap<CallablePlace, Targets> = BTreeMap::new();
                let mut argument_callees = BTreeMap::new();
                let mut argument_local_fields = BTreeMap::new();
                let mut prepared_callees: BTreeMap<CallablePlace, CallableResult> = BTreeMap::new();
                for argument in arguments {
                    let saved = if argument.is_default {
                        for (place, key) in &prepared_callees {
                            self.step(place.fields.len() + 1)?;
                            key.charge(self.fuel)?;
                        }
                        for (place, targets) in &prepared_fields {
                            self.step(place.fields.len())?;
                            if let Some(targets) = targets {
                                self.target_work(targets)?;
                            }
                        }
                        let environment = Environment {
                            pointers: self.clone_pointer_environment(&prepared)?,
                            field_pointers: prepared_fields.clone(),
                            local_candidates: BTreeMap::new(),
                            callees: prepared_callees.clone(),
                        };
                        for ty in prepared_types.values() {
                            bound_type(ty, self.fuel)?;
                        }
                        Some((
                            std::mem::replace(&mut self.locals, prepared_types.clone()),
                            std::mem::replace(&mut self.environment, environment),
                            std::mem::take(&mut self.loop_states),
                        ))
                    } else {
                        None
                    };
                    let result = (|| {
                        self.walk(&argument.value, depth + 1)?;
                        if super::pattern_facts::cannot_complete(&argument.value, self.fuel, 0)? {
                            return Ok(None);
                        }
                        let target = if self.local_pointer_value(&argument.value.ty)? {
                            self.optional_target(&argument.value, depth + 1)?
                        } else {
                            None
                        };
                        let values = self.value_callees(&argument.value)?;
                        let local_fields = self.local_fields_for(&argument.value)?;
                        let fields = self.pointer_fields_for(&argument.value)?;
                        Ok::<_, String>(Some((target, values, local_fields, fields)))
                    })();
                    if let Some((locals, environment, loops)) = saved {
                        self.locals = locals;
                        self.environment = environment;
                        self.loop_states = loops;
                    }
                    // Argument effects are retained, but an escaping argument never
                    // enters the callee or evaluates the remaining arguments.
                    let Some((target, values, local_fields, mut fields)) = result? else {
                        return Ok(());
                    };
                    for (path, targets) in &fields {
                        self.step(path.len())?;
                        if let Some(targets) = targets {
                            self.target_work(targets)?;
                        }
                        prepared_fields.insert(
                            CallablePlace {
                                binding: parameter_bindings[argument.parameter],
                                fields: path.clone(),
                            },
                            targets.clone(),
                        );
                    }
                    argument_local_fields.insert(argument.parameter, local_fields);
                    for (path, key) in &values {
                        self.step(path.len())?;
                        key.charge(self.fuel)?;
                        prepared_callees.insert(
                            CallablePlace {
                                binding: parameter_bindings[argument.parameter],
                                fields: path.clone(),
                            },
                            key.clone(),
                        );
                    }
                    argument_callees.insert(argument.parameter, values);
                    let argument_ty = self.ty(&argument.value.ty)?;
                    prepared_types.insert(parameter_bindings[argument.parameter], argument_ty);
                    if self.local_pointer_value(&argument.value.ty)? {
                        if let Some(binding) = parameter_bindings.get(argument.parameter) {
                            prepared.insert(*binding, target.clone());
                        }
                    }
                    if let Some(target) = target {
                        fields.insert(vec![], Some(target));
                    }
                    targets.insert(
                        argument.parameter,
                        fields
                            .into_iter()
                            .filter_map(|(path, target)| target.map(|target| (path, target)))
                            .collect::<BTreeMap<_, _>>(),
                    );
                }
                let mut inputs = BTreeMap::new();
                for (parameter, fields) in &argument_callees {
                    for (path, value) in fields {
                        if matches!(
                            value,
                            CallableResult::Known(_)
                                | CallableResult::Candidates(_)
                                | CallableResult::NoReturn
                        ) {
                            self.step(path.len() + 1)?;
                            value.charge(self.fuel)?;
                            inputs.insert(
                                CallableInput {
                                    parameter: *parameter,
                                    fields: path.clone(),
                                },
                                value.clone(),
                            );
                        }
                    }
                }
                let mut prepared = BTreeMap::new();
                for (parameter, fields) in argument_local_fields {
                    for (path, ids) in fields {
                        self.step(path.len())?;
                        // An empty local-code set is worklist bottom, not
                        // unknown code. Preserve it while factory summaries are
                        // still being discovered, including closure results.
                        if ids.is_empty() {
                            inputs.insert(
                                CallableInput {
                                    parameter,
                                    fields: path,
                                },
                                CallableResult::NoReturn,
                            );
                            continue;
                        }
                        let mut definitions = vec![];
                        let mut anonymous = false;
                        for id in ids {
                            let code = self
                                .local_code
                                .get(id)
                                .ok_or("prepared code definition is missing")?;
                            code.charge(self.fuel)?;
                            anonymous |= matches!(code, PreparedCode::Closure { .. });
                            definitions.push(code.clone());
                        }
                        if anonymous {
                            prepared.insert(
                                CallableInput {
                                    parameter,
                                    fields: path,
                                },
                                definitions,
                            );
                        }
                    }
                }
                let code_inputs = borrow_code_inputs(&prepared);
                let mut combined: Option<CallFootprint> = None;
                let mut next_capture_slot = parameters.len();
                for (id, closure, captures, prepared_captures, pointer_captures) in closures {
                    let K::Closure {
                        captures: definitions,
                        ..
                    } = &closure.kind
                    else {
                        unreachable!()
                    };
                    let transition_slots = closure_transition_slots(
                        package,
                        definitions,
                        parameters.len(),
                        true,
                        self.fuel,
                    )?;
                    // Capture identities belong to construction time; later
                    // assignments in the enclosing frame cannot change them.
                    let mut next = if transition_slots.is_empty() {
                        infer_closure_in_round(
                            package,
                            &closure,
                            &captures,
                            &prepared_captures,
                            &inputs,
                            &code_inputs,
                            self.fuel,
                            self.round,
                            (self.parameters, self.arguments),
                            true,
                        )?
                    } else {
                        infer_closure_with_borrowed_in_round(
                            package,
                            &closure,
                            &captures,
                            &prepared_captures,
                            &inputs,
                            &code_inputs,
                            self.round,
                            self.fuel,
                            false,
                            true,
                        )?
                    };
                    if !transition_slots.is_empty() {
                        let mut remap = BTreeMap::new();
                        let mut original = parameters.len();
                        for capture in definitions {
                            if let Some(origins) = pointer_captures.get(&capture.binding) {
                                self.step(depth)?;
                                remap.insert(original, next_capture_slot);
                                original += 1;
                                let mut fields = BTreeMap::new();
                                for (path, origins) in origins {
                                    self.step(path.len())?;
                                    if let Some(origins) = origins {
                                        self.target_work(origins)?;
                                        fields.insert(path.clone(), origins.clone());
                                    }
                                }
                                targets.insert(next_capture_slot, fields);
                                next_capture_slot = next_capture_slot
                                    .checked_add(1)
                                    .ok_or("closure capture input identity exhausted")?;
                            }
                        }
                        let mut updated = BTreeMap::new();
                        let mut code_updates = BTreeMap::new();
                        let mut original = parameters.len();
                        for capture in definitions {
                            if let Some(fields) = pointer_captures.get(&capture.binding) {
                                let mut output = BTreeMap::new();
                                for (path, previous) in fields {
                                    self.step(path.len())?;
                                    let next_origins = if next.input_pointer_exits.is_empty() {
                                        if let Some(previous) = previous {
                                            self.target_work(previous)?;
                                        }
                                        previous.clone()
                                    } else {
                                        let key = CallableInput {
                                            parameter: original,
                                            fields: path.clone(),
                                        };
                                        let origins = next
                                            .input_pointer_exits
                                            .get(&key)
                                            .and_then(Option::as_ref);
                                        if let Some(origins) = origins {
                                            self.target_work(origins)?;
                                        }
                                        let mut renamed = origins.cloned();
                                        if let Some(origins) = &mut renamed {
                                            let mut values = BTreeSet::new();
                                            for mut origin in std::mem::take(origins) {
                                                if let Some(slot) = remap.get(&origin.parameter) {
                                                    origin.parameter = *slot;
                                                }
                                                values.insert(origin);
                                            }
                                            *origins = values;
                                        }
                                        self.rebase_result(renamed.as_ref(), &targets)?
                                    };
                                    output.insert(path.clone(), next_origins);
                                }
                                updated.insert(capture.binding, output);
                                original += 1;
                            }
                        }
                        for (binding, slot) in &transition_slots {
                            for (input, codes) in &next.input_code_exits {
                                self.step(input.fields.len() + 1)?;
                                if input.parameter != *slot {
                                    continue;
                                }
                                let codes = codes.as_ref().ok_or(
                                    "closure capture code exit has unknown environment identity",
                                )?;
                                for code in codes {
                                    code.charge(self.fuel)?;
                                }
                                code_updates.insert(
                                    CallablePlace {
                                        binding: *binding,
                                        fields: input.fields.clone(),
                                    },
                                    codes.clone(),
                                );
                            }
                        }
                        let next_id =
                            self.transition_closure_environment(id, updated, code_updates)?;
                        post_candidates.insert(next_id);
                        for access in &mut next.accesses {
                            self.step(access.target.path_cost())?;
                            if let Some(slot) = remap.get(&access.target.parameter) {
                                access.target.parameter = *slot;
                            }
                        }
                        if let Some(results) = next.result_targets.take() {
                            let mut renamed = BTreeSet::new();
                            for mut result in results {
                                self.step(result.path_cost())?;
                                if let Some(slot) = remap.get(&result.parameter) {
                                    result.parameter = *slot;
                                }
                                renamed.insert(result);
                            }
                            next.result_targets = Some(renamed);
                        }
                        for (path, targets) in &mut next.pointer_fields {
                            self.step(path.len())?;
                            if let Some(targets) = targets {
                                let mut renamed = BTreeSet::new();
                                for mut target in std::mem::take(targets) {
                                    self.step(target.path_cost())?;
                                    if let Some(slot) = remap.get(&target.parameter) {
                                        target.parameter = *slot;
                                    }
                                    renamed.insert(target);
                                }
                                *targets = renamed;
                            }
                        }
                    }
                    if transition_slots.is_empty() {
                        post_candidates.insert(id);
                    }
                    if self.ty(closure.ty.callable_signature())? != signature {
                        return Err("closure memory callee signature mismatch".into());
                    }
                    self.join_call_footprint(
                        &mut combined,
                        CallFootprint {
                            accesses: next.accesses,
                            may_reject: next.may_reject,
                            may_run_cleanup: next.may_run_cleanup,
                            result_targets: next.result_targets,
                            pointer_fields: next.pointer_fields,
                            callable_result: CallableResult::Unknown,
                            callable_fields: BTreeMap::new(),
                            code_fields: next.code_fields,
                        },
                    )?;
                }
                let borrowed = self.forward_borrowed_callables(parameters, &targets)?;
                for key in keys {
                    self.step(depth)?;
                    let next = derive_with_borrowed(
                        package,
                        key.declaration,
                        &key.arguments,
                        &inputs,
                        &prepared,
                        &borrowed,
                        self.fuel,
                        self.round,
                    )?;
                    let matches = if next.signature == signature {
                        true
                    } else {
                        let normalized = self.completed_type_view(&signature)?;
                        next.signature == normalized
                    };
                    if !matches {
                        return Err(format!(
                            "memory callee signature disagrees with its body: expected {signature}, found {}",
                            next.signature
                        ));
                    }
                    self.join_call_footprint(&mut combined, next.into())?;
                }
                let summary = combined.ok_or("empty callable candidate set")?;
                if let Some(place) = receiver_place.filter(|_| {
                    receiver_candidates
                        .as_ref()
                        .is_some_and(|previous| previous != &post_candidates)
                }) {
                    if self.local_candidates_for(callee)? != receiver_candidates {
                        return Err("closure receiver changed while evaluating arguments".into());
                    }
                    self.set_local_candidates(place.clone(), Some(post_candidates))?;
                    let pointers = self.pointer_fields_for(callee)?;
                    self.replace_pointer_fields(place, pointers)?;
                }
                self.install_code_results(expr, &summary.code_fields)?;
                self.may_reject |= summary.may_reject;
                self.may_run_cleanup |= summary.may_run_cleanup;
                let callable_result = summary
                    .callable_result
                    .substitute_inputs(&argument_callees, self.fuel)?;
                self.callable_results
                    .insert(expr as *const Expr as usize, callable_result);
                let mut callable_fields = BTreeMap::new();
                for (path, key) in &summary.callable_fields {
                    self.step(path.len())?;
                    callable_fields.insert(
                        path.clone(),
                        key.substitute_inputs(&argument_callees, self.fuel)?,
                    );
                }
                self.aggregate_callees
                    .insert(expr as *const Expr as usize, callable_fields);
                let mut pointer_fields = BTreeMap::new();
                for (path, origins) in &summary.pointer_fields {
                    self.step(path.len())?;
                    pointer_fields.insert(
                        path.clone(),
                        self.rebase_result(origins.as_ref(), &targets)?,
                    );
                }
                self.aggregate_pointers
                    .insert(expr as *const Expr as usize, pointer_fields);
                self.install_returned_capture_origins(expr)?;
                let result_targets =
                    self.rebase_result(summary.result_targets.as_ref(), &targets)?;
                self.expression_results
                    .insert(expr as *const Expr as usize, result_targets);
                for access in summary.accesses {
                    self.step(depth)?;
                    let actual = targets
                        .get(&access.target.parameter)
                        .and_then(|fields| fields.get(&access.target.input_fields))
                        .ok_or("memory call target is not established")?;
                    self.target_work(actual)?;
                    for base in actual {
                        self.step(depth)?;
                        let mut target = base.clone();
                        let cost = access.target.path_cost();
                        *self.fuel = self
                            .fuel
                            .checked_sub(cost)
                            .ok_or("memory target set budget exhausted")?;
                        target.append_resolved_path(&access.target)?;
                        bound_type(&access.ty, self.fuel)?;
                        self.record_access(
                            BTreeSet::from([target]),
                            access.write,
                            access.read_value,
                            access.ty.clone(),
                        )?;
                    }
                }
            }
            K::Let { binding, value } => {
                self.walk(value, depth + 1)?;
                let pointers = self.pointer_fields_for(value)?;
                self.replace_pointer_fields(CallablePlace::root(*binding), pointers)?;
                let ty = self.ty(&value.ty)?;
                let values = self.value_callees(value)?;
                let local_fields = self.local_fields_for(value)?;
                self.replace_callees(CallablePlace::root(*binding), values)?;
                self.locals.insert(*binding, ty);
                self.replace_local_fields(CallablePlace::root(*binding), local_fields)?;
                if self.local_pointer_value(&value.ty)? {
                    let targets = self.optional_target(value, depth + 1)?;
                    self.set_pointer(*binding, targets)?;
                }
            }
            K::LetPattern {
                pattern,
                value,
                guard,
                otherwise,
            } => {
                self.walk(value, depth + 1)?;
                let ty = self.ty(&value.ty)?;
                let pointer_subject = self.local_pointer_value(&value.ty)?;
                if !pointer_subject
                    && symbolic_pointer_fields(self.package, &ty, self.fuel)?.is_empty()
                {
                    self.pointer_free_owned(&ty, depth + 1)?;
                }
                let pointers = self.pointer_fields_for(value)?;
                let base = self.save_environment()?;
                if pointer_subject {
                    let targets = self.optional_target(value, depth + 1)?;
                    self.bind_match_subject(pattern, &targets, true, &ty, true, depth + 1)?;
                    let values = self.value_callees(value)?;
                    let local_fields = self.local_fields_for(value)?;
                    self.bind_owned_callees(pattern, &ty, &values, &local_fields, depth + 1)?;
                } else if effect_data_layout(self.package, &ty, self.fuel, 0)? {
                    self.bind_data_pattern(pattern, &ty, depth + 1)?;
                } else {
                    let values = self.value_callees(value)?;
                    let local_fields = self.local_fields_for(value)?;
                    self.bind_owned_values(
                        pattern,
                        &ty,
                        &values,
                        &local_fields,
                        &pointers,
                        true,
                        depth + 1,
                    )?;
                }
                if let Some(guard) = guard {
                    self.walk_guard(guard, depth + 1)?;
                }
                if let Some(otherwise) = otherwise {
                    if otherwise.ty != Term::Primitive(Primitive::Never) {
                        return Err("memory let-else requires a diverging failure branch".into());
                    }
                    let success = self.save_environment()?;
                    self.environment = base;
                    self.walk(otherwise, depth + 1)?;
                    self.environment = success;
                }
            }
            K::BitAssign {
                place,
                storage_type,
                value,
                ..
            }
            | K::BitUpdate {
                place,
                storage_type,
                value,
                ..
            } => {
                let storage = self.ty(storage_type)?;
                let value_ty = self.ty(&value.ty)?;
                let result_ty = self.ty(&expr.ty)?;
                let reason = match &expr.kind {
                    K::BitAssign { start, end, .. } => nir::program::bit_writeback_error_in(
                        &storage,
                        *start,
                        *end,
                        &value_ty,
                        &result_ty,
                        self.package.and_then(|package| package.target),
                    ),
                    K::BitUpdate { selections, .. } => {
                        *self.fuel = self
                            .fuel
                            .checked_sub(selections.len())
                            .ok_or("bit selection memory budget exhausted")?;
                        nir::program::bit_update_error_in(
                            &storage,
                            selections,
                            &value_ty,
                            &result_ty,
                            self.package.and_then(|package| package.target),
                        )
                    }
                    _ => unreachable!(),
                };
                if let Some(reason) = reason {
                    return Err(reason.into());
                }
                if let K::BitUpdate { selections, .. } = &expr.kind {
                    for input in selections
                        .iter()
                        .flat_map(nir::program::BitSelection::expressions)
                    {
                        self.walk(input, depth + 1)?;
                    }
                }
                let root = self
                    .locals
                    .get(&place.root)
                    .ok_or("memory bit writeback root is missing")?;
                let mut traversal = StorageTraversal::new(self.package, root, &[], self.fuel)?;
                traversal.then_guarded(self.package, &place.fields, &[], self.fuel)?;
                if traversal.target.ty != storage {
                    return Err("memory bit writeback local storage type mismatch".into());
                }
                // Only scalar storage owned by a local is modified, including
                // direct nested fields. Positions and the RHS can still access
                // caller memory and retain their ordinary footprint checks.
                self.walk(value, depth + 1)?;
            }
            K::Assign { place, value } => {
                self.walk(value, depth + 1)?;
                let path: Vec<_> = place.fields.iter().map(|(_, index)| *index).collect();
                let root_ty = self
                    .locals
                    .get(&place.root)
                    .ok_or("memory assignment root is missing")?
                    .clone();
                bound_type(&root_ty, self.fuel)?;
                let mut expected = root_ty.clone();
                for (owner, index) in &place.fields {
                    let valid = match (&expected, owner) {
                        (
                            Term::Nominal { declaration, .. },
                            nir::program::FieldOwner::Nominal(actual),
                        ) => declaration == actual,
                        (Term::Record(_), nir::program::FieldOwner::Anonymous) => true,
                        (_, nir::program::FieldOwner::Anonymous)
                            if tuple_fields(self.package, &expected).is_some() =>
                        {
                            true
                        }
                        _ => false,
                    };
                    if !valid {
                        return Err("memory assignment field owner mismatch".into());
                    }
                    expected = owned_path_type(self.package, &expected, &[*index], self.fuel)?;
                }
                if expected != self.ty(&value.ty)? {
                    return Err("memory assignment type mismatch".into());
                }
                let mut pointers = self.pointer_fields_for(value)?;
                if !path.is_empty() && self.local_pointer_value(&expected)? {
                    pointers.insert(vec![], self.optional_target(value, depth + 1)?);
                }
                self.replace_pointer_fields(
                    CallablePlace {
                        binding: place.root,
                        fields: path.clone(),
                    },
                    pointers,
                )?;
                if !path.is_empty() {
                    symbolic_pointer_fields(self.package, &expected, self.fuel)?;
                }
                let values = self.value_callees(value)?;
                let local_fields = self.local_fields_for(value)?;
                self.record_capture_code_write(place.root, &path, &expected, value, &local_fields)?;
                self.replace_callees(
                    CallablePlace {
                        binding: place.root,
                        fields: path.clone(),
                    },
                    values,
                )?;
                self.replace_local_fields(
                    CallablePlace {
                        binding: place.root,
                        fields: path.clone(),
                    },
                    local_fields,
                )?;
                if path.is_empty() && self.local_pointer_value(&value.ty)? {
                    let targets = self.optional_target(value, depth + 1)?;
                    self.set_pointer(place.root, targets)?;
                }
                if self.private_storage_roots.contains(&place.root) {
                    // A field update can change any enclosing destructor's
                    // pointer dependencies. Keep previous accesses and add the
                    // complete root's reads using its new pointer environment.
                    self.record_value_cleanup_reads(&Expr {
                        source: expr.source,
                        ty: root_ty,
                        kind: K::Local(place.root),
                    })?;
                }
            }
            K::Block { .. } | K::CallPreparation { .. } | K::If { .. } => {
                unreachable!("control dispatcher")
            }
            K::Return(value) => {
                if let Some(value) = value {
                    self.walk(value, depth + 1)?;
                    self.merge_callable_field_return(value)?;
                    self.merge_pointer_field_return(value)?;
                    if self.local_pointer_value(&value.ty)? {
                        let targets = self.optional_target(value, depth + 1)?;
                        self.merge_return(targets)?;
                    } else if matches!(
                        self.ty(&value.ty)?,
                        Term::Function { .. } | Term::UnsafeFunction(_)
                    ) {
                        let key = self.callee_key(value)?;
                        self.callable_returns.join(&key, self.fuel)?;
                    }
                }
                if !value
                    .as_ref()
                    .map(|value| super::pattern_facts::cannot_complete(value, self.fuel, 0))
                    .transpose()?
                    .unwrap_or(false)
                {
                    self.merge_input_pointer_exit()?;
                    self.merge_input_code_exit()?;
                }
            }
            K::Loop { body, .. } => loop {
                self.step(depth)?;
                let head = self.save_environment()?;
                let accumulated = self.clone_environment(&head)?;
                self.loop_states.push(accumulated);
                self.walk(body, depth + 1)?;
                let next = self.loop_states.pop().ok_or("missing loop memory state")?;
                self.environment = next;
                if self.environment == head {
                    break;
                }
            },
            K::RequirePredicate(condition)
            | K::EnsurePredicate(condition)
            | K::CheckPredicate(condition) => {
                if self.ty(&expr.ty)? != Term::Primitive(Primitive::Void)
                    || !nir::program::checkable_predicate(condition)
                {
                    return Err("invalid runtime predicate check".into());
                }
                self.walk_guard(condition, depth + 1)?;
                self.may_reject |= self.known_bool(condition, 0)? != Some(true);
                // The footprint is conservative over the successful path. No
                // ownership or memory-state facts are inferred from the check.
            }
            K::UnsafeScope(value)
            | K::Discard(value)
            | K::Not(value)
            | K::IntegerNegate(value)
            | K::FloatNegate(value)
            | K::BitConvert(value)
            | K::Snapshot(value)
            | K::TakeBits(value)
            | K::TextQuery { subject: value, .. }
            | K::IntegerRepresentation(value)
            | K::SymbolName(value)
            | K::EnumName(value)
            | K::BitSlice { value, .. } => self.walk(value, depth + 1)?,
            K::BitWindow { value, start, end } => {
                self.walk(value, depth + 1)?;
                self.walk(start, depth + 1)?;
                self.walk(end, depth + 1)?;
            }
            K::And(a, b) | K::Or(a, b) => {
                self.walk(a, depth + 1)?;
                let known = self.known_bool(a, 0)?;
                let evaluate_on = matches!(&expr.kind, K::And(..));
                if self.ty(&expr.ty)? != Term::Primitive(Primitive::Bool)
                    || known == Some(evaluate_on)
                {
                    self.walk(b, depth + 1)?;
                } else if known.is_none() {
                    let skipped = self.save_environment()?;
                    self.walk(b, depth + 1)?;
                    let evaluated = std::mem::take(&mut self.environment);
                    self.environment = if b.ty == Term::Primitive(Primitive::Never) {
                        skipped
                    } else {
                        self.join_environment(skipped, evaluated)?
                    };
                }
            }
            K::BitEqual(a, b)
            | K::BitConcat(a, b)
            | K::BitIndex(a, b)
            | K::ScalarEqual(a, b)
            | K::TemplateEqual(a, b)
            | K::ScalarLess(a, b)
            | K::ScalarLessEqual(a, b)
            | K::ScalarGreater(a, b)
            | K::ScalarGreaterEqual(a, b)
            | K::IntegerBinary {
                left: a, right: b, ..
            }
            | K::FloatBinary {
                left: a, right: b, ..
            } => {
                self.walk(a, depth + 1)?;
                self.walk(b, depth + 1)?;
            }
            _ => return Err("operation requires a richer memory contract".into()),
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct MemoryInstance {
    effects: Vec<EffectInput>,
    instance: iir::InstanceKey,
    inputs: BTreeMap<CallableInput, CallableResult>,
    prepared: BTreeMap<CallableInput, Vec<usize>>,
    borrowed: BorrowedCallableInputs,
}
#[derive(Default)]
struct Round<'cleanup> {
    // Independent complete-layout evidence in this query's immutable Drop world.
    cleanup: Option<&'cleanup mut dyn super::moves::CleanupPolicy>,
    memory_inert_cleanup_types: HashSet<Term>,
    cleanup_reads: HashMap<Term, Vec<Access>>,
    used_external_cleanup: bool,
    mutates_pointer_storage: bool,
    // All instances in one fixed point belong to the same immutable package.
    // This is declaration metadata, independent of code inputs or call state.
    cleanup_protocol_present: Option<bool>,
    parameter_sorts: Option<BTreeMap<DeclarationId, nir::Sort>>,
    effects: Vec<EffectInput>,
    complete_calls: bool,
    code: Vec<PreparedCode>,
    previous: HashMap<MemoryInstance, Summary>,
    queue: Vec<MemoryInstance>,
    discovered: HashMap<MemoryInstance, usize>,
    pending: std::collections::VecDeque<usize>,
    scheduled: BTreeSet<usize>,
    dependents: BTreeMap<usize, BTreeSet<usize>>,
    active: Option<usize>,
}

impl Round<'_> {
    fn binders(
        &mut self,
        package: Option<&Package>,
        parameters: &[DeclarationId],
        fuel: &mut usize,
    ) -> Result<Vec<nir::DeclarationParameter>, String> {
        *fuel = fuel
            .checked_sub(parameters.len())
            .ok_or("memory binder budget exhausted")?;
        if parameters.is_empty() {
            return Ok(vec![]);
        }
        // A round belongs to one immutable package. Cache only completed
        // declaration evidence, never argument-derived sorts or partial scans.
        if self.parameter_sorts.is_none() {
            if let Some(package) = package {
                self.parameter_sorts = Some(package.declaration_parameter_sorts(fuel)?);
            }
        }
        Ok(parameters
            .iter()
            .map(|id| nir::DeclarationParameter {
                declaration: *id,
                sort: self
                    .parameter_sorts
                    .as_ref()
                    .and_then(|sorts| sorts.get(id))
                    .copied()
                    .unwrap_or(nir::Sort::Type),
            })
            .collect())
    }
}

// Binder sorts come from declarations, never from the supplied values. A
// package-free monomorphic summary has no extra value-parameter evidence.
pub(super) fn memory_binders(
    package: Option<&Package>,
    parameters: &[DeclarationId],
    fuel: &mut usize,
) -> Result<Vec<nir::DeclarationParameter>, String> {
    *fuel = fuel
        .checked_sub(parameters.len())
        .ok_or("memory binder budget exhausted")?;
    if parameters.is_empty() {
        return Ok(vec![]);
    }
    let sorts = match package {
        Some(package) => package.declaration_parameter_sorts(fuel)?,
        None => BTreeMap::new(),
    };
    Ok(parameters
        .iter()
        .map(|id| nir::DeclarationParameter {
            declaration: *id,
            sort: sorts.get(id).copied().unwrap_or(nir::Sort::Type),
        })
        .collect())
}

pub fn infer(function: &Function, fuel: &mut usize) -> Result<Summary, String> {
    infer_body(
        function,
        &[],
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        None,
        fuel,
        &mut Round::default(),
    )
}

/// Compute the least closed footprint for the reachable instance set. Empty
/// cycle seeds stay inside this transaction and are never returned as evidence.
pub fn infer_in_package(
    package: &Package,
    declaration: DeclarationId,
    arguments: &[Term],
    fuel: &mut usize,
) -> Result<Summary, String> {
    infer_with_inputs(package, declaration, arguments, &BTreeMap::new(), fuel)
}

type MethodKey = (
    bool,
    Term,
    DeclarationId,
    Vec<Term>,
    DeclarationId,
    Term,
    Vec<Term>,
);

/// Package-local successful dispatch evidence. Runtime arguments and source
/// locations do not select implementations; the full interface signature does.
#[derive(Default)]
pub(crate) struct MethodCallees(std::collections::HashMap<MethodKey, Expr>);

impl MethodCallees {
    pub(crate) fn select(
        &mut self,
        package: &Package,
        callee: &Expr,
        fuel: &mut usize,
    ) -> Result<Expr, String> {
        self.select_for(package, callee, fuel, false)
    }

    /// Select only code identity for behavior analysis. The original interface
    /// call remains in the program and retains all its predicate obligations.
    pub(crate) fn select_behavior(
        &mut self,
        package: &Package,
        callee: &Expr,
        fuel: &mut usize,
    ) -> Result<Expr, String> {
        self.select_for(package, callee, fuel, true)
    }

    fn select_for(
        &mut self,
        package: &Package,
        callee: &Expr,
        fuel: &mut usize,
        behavior_only: bool,
    ) -> Result<Expr, String> {
        let K::TraitMethod {
            interface,
            interface_arguments,
            member,
            subject,
            method_arguments,
        } = &callee.kind
        else {
            return Err("memory method cache requires a trait method".into());
        };
        for ty in std::iter::once(&callee.ty)
            .chain(std::iter::once(subject))
            .chain(interface_arguments)
            .chain(method_arguments)
        {
            bound_type(ty, fuel)?;
        }
        let key = (
            behavior_only,
            callee.ty.clone(),
            *interface,
            interface_arguments.clone(),
            *member,
            subject.clone(),
            method_arguments.clone(),
        );
        if let Some(selected) = self.0.get(&key) {
            bound_type(&selected.ty, fuel)?;
            if let K::FunctionInstance { arguments, .. } = &selected.kind {
                for argument in arguments {
                    bound_type(argument, fuel)?;
                }
            }
            let mut selected = selected.clone();
            selected.source = callee.source;
            return Ok(selected);
        }
        let selected = selected_method_callee(package, callee, fuel, behavior_only)?;
        self.0.insert(key, selected.clone());
        Ok(selected)
    }
}

/// Recover concrete code for body-derived footprints without erasing interface
/// predicates. Predicate-bearing interfaces need their own footprint composition
/// and remain rejected until that path is available.
pub(crate) fn method_callee(
    package: &Package,
    callee: &Expr,
    fuel: &mut usize,
) -> Result<Expr, String> {
    selected_method_callee(package, callee, fuel, false)
}

fn selected_method_callee(
    package: &Package,
    callee: &Expr,
    fuel: &mut usize,
    behavior_only: bool,
) -> Result<Expr, String> {
    let K::TraitMethod {
        interface, member, ..
    } = &callee.kind
    else {
        return Err("memory method selection requires a trait method".into());
    };
    let contract = package
        .traits
        .iter()
        .find(|definition| definition.declaration == *interface)
        .and_then(|definition| {
            definition
                .methods
                .iter()
                .find(|method| method.declaration == *member)
        })
        .ok_or("memory method selection lacks its interface contract")?;
    if !behavior_only
        && (!contract.program.preconditions.is_empty()
            || !contract.program.postconditions.is_empty())
    {
        return Err("interface predicates require a composed memory contract".into());
    }
    let method = if contract.is_quote {
        crate::materialization::resolve_quote_method_instance(package, callee, fuel)?
    } else {
        crate::materialization::resolve_method_instance(package, callee, fuel)?
    };
    // Use the same residual body as memory inference. Selecting the original
    // generic signature here reintroduces associated projections that the
    // concrete instance has already resolved.
    let (body, body_arguments) = package.runtime_body_checked(method.body, &method.arguments)?;
    bound_type(&body.signature, fuel)?;
    let binders = memory_binders(Some(package), &body.type_parameters, fuel)?;
    let signature = body
        .signature
        .instantiate_parameters(&binders, body_arguments)
        .map_err(|reason| reason.to_string())?;
    Ok(Expr {
        source: callee.source,
        // Dispatch has checked the declaration-side contract. This reference
        // names its concrete body, whose associated types are now resolved.
        ty: signature,
        kind: K::FunctionInstance {
            declaration: method.body,
            arguments: method.arguments,
        },
    })
}

/// Infer under concrete code premises. Missing fields remain unknown; this API
/// never treats an unestablished input as an empty set of possible callees.
pub fn infer_with_inputs(
    package: &Package,
    declaration: DeclarationId,
    arguments: &[Term],
    inputs: &BTreeMap<CallableInput, CallableResult>,
    fuel: &mut usize,
) -> Result<Summary, String> {
    for value in inputs.values() {
        if !matches!(
            value,
            CallableResult::Known(_) | CallableResult::Candidates(_)
        ) {
            return Err("memory callable premise requires named code identities".into());
        }
    }
    infer_context(
        package,
        declaration,
        arguments,
        inputs,
        &BTreeMap::new(),
        &[],
        fuel,
    )
}
/// Infer a named body with complete prepared anonymous/mixed code premises.
/// Returned summaries retain those premises; they are not unconditional effects.
pub fn infer_with_code_inputs(
    package: &Package,
    declaration: DeclarationId,
    arguments: &[Term],
    inputs: &BTreeMap<CallableInput, CallableResult>,
    code_inputs: &CodeInputs<'_>,
    fuel: &mut usize,
) -> Result<Summary, String> {
    for value in inputs.values() {
        if !matches!(
            value,
            CallableResult::Known(_) | CallableResult::Candidates(_)
        ) {
            return Err("memory callable premise requires named code identities".into());
        }
    }
    let code = own_code_inputs(code_inputs, &[], &[], fuel)?;
    infer_context(package, declaration, arguments, inputs, &code, &[], fuel)
}
fn validate_cleanup_access(
    package: &Package,
    signature: &Term,
    access: &nir::program::RuntimeCleanupAccess,
    fuel: &mut usize,
) -> Result<(), String> {
    let Term::Function { parameters, .. } = signature.callable_signature() else {
        return Err("cleanup footprint lacks a callable signature".into());
    };
    let parameter = parameters
        .get(access.parameter)
        .ok_or("cleanup footprint parameter is missing")?;
    let input = owned_path_type(Some(package), &parameter.ty, &access.input_fields, fuel)?;
    let (_, ty) = pointer_representation(Some(package), &input, fuel)?
        .ok_or("cleanup footprint input is not a pointer")?;
    let mut traversal = StorageTraversal::new(Some(package), &ty, &[], fuel)?;
    if access
        .conditions
        .keys()
        .any(|stage| *stage > access.through.len())
    {
        return Err("cleanup guard stage out of bounds".into());
    }
    for (stage, path) in access.through.iter().enumerate() {
        traversal.then_guarded(
            Some(package),
            path,
            access
                .conditions
                .get(&stage)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            fuel,
        )?;
        if !matches!(traversal.target.ty, Term::Pointer(_)) {
            return Err("cleanup footprint load is not a pointer".into());
        }
        traversal.load(Some(package), &[], fuel)?;
    }
    traversal.then_guarded(
        Some(package),
        &access.fields,
        access
            .conditions
            .get(&access.through.len())
            .map(Vec::as_slice)
            .unwrap_or(&[]),
        fuel,
    )?;
    if traversal.target.ty != access.ty {
        return Err("cleanup footprint target type mismatch".into());
    }
    Ok(())
}

fn infer_context(
    package: &Package,
    declaration: DeclarationId,
    arguments: &[Term],
    inputs: &BTreeMap<CallableInput, CallableResult>,
    prepared: &PreparedCodeInputs,
    effects: &[EffectInput],
    fuel: &mut usize,
) -> Result<Summary, String> {
    infer_borrowed_context(
        package,
        declaration,
        arguments,
        inputs,
        prepared,
        effects,
        &BTreeMap::new(),
        fuel,
    )
}
fn infer_borrowed_context(
    package: &Package,
    declaration: DeclarationId,
    arguments: &[Term],
    inputs: &BTreeMap<CallableInput, CallableResult>,
    prepared: &PreparedCodeInputs,
    effects: &[EffectInput],
    borrowed: &BorrowedCallableInputs,
    fuel: &mut usize,
) -> Result<Summary, String> {
    infer_borrowed_context_with_cleanup(
        package,
        declaration,
        arguments,
        inputs,
        prepared,
        effects,
        borrowed,
        None,
        fuel,
    )
}
fn infer_borrowed_context_with_cleanup(
    package: &Package,
    declaration: DeclarationId,
    arguments: &[Term],
    inputs: &BTreeMap<CallableInput, CallableResult>,
    prepared: &PreparedCodeInputs,
    effects: &[EffectInput],
    borrowed: &BorrowedCallableInputs,
    cleanup: Option<&mut dyn super::moves::CleanupPolicy>,
    fuel: &mut usize,
) -> Result<Summary, String> {
    let mut round = Round {
        cleanup,
        effects: effects::copy_inputs(effects, fuel)?,
        ..Round::default()
    };
    derive_with_borrowed(
        package,
        declaration,
        arguments,
        inputs,
        prepared,
        borrowed,
        fuel,
        &mut round,
    )?;
    while let Some(index) = round.pending.pop_front() {
        round.scheduled.remove(&index);
        round.active = Some(index);
        effects::charge_inputs(&round.queue[index].effects, fuel)?;
        borrowed_code::charge(&round.queue[index].borrowed, fuel)?;
        let key = round.queue[index].clone();
        let (function, body_arguments) = package
            .runtime_body(key.instance.declaration, &key.instance.arguments)
            .ok_or("memory summary function is missing")?;
        let prepared = key
            .prepared
            .iter()
            .map(|(input, ids)| {
                let values = ids.iter().map(|id| round.code[*id].clone()).collect();
                (input.clone(), values)
            })
            .collect();
        let context = effects::copy_inputs(&key.effects, fuel)?;
        let previous_effects = std::mem::replace(&mut round.effects, context);
        let result = infer_body(
            function,
            body_arguments,
            &key.inputs,
            &prepared,
            &key.borrowed,
            Some(package),
            fuel,
            &mut round,
        );
        round.effects = previous_effects;
        // Budget failures otherwise surface at an unrelated caller with only
        // an internal collection name. Report the active instance without
        // publishing its provisional footprint or claiming non-convergence.
        let failure = |reason: String| {
            if reason.contains("exhausted") {
                format!(
                    "{reason}; while inferring memory contract for {:?} ({}) [discovered instances: {}, pending instances: {}, previous accesses: {}]",
                    function.name,
                    key.instance,
                    round.queue.len(),
                    round.pending.len(),
                    round
                        .previous
                        .get(&key)
                        .map_or(0, |summary| summary.accesses.len()),
                )
            } else {
                reason
            }
        };
        let mut summary = result.map_err(failure)?;
        finish_instance_summary(
            package,
            key.instance.declaration,
            &key.instance.arguments,
            &mut summary,
            fuel,
        )
        .map_err(failure)?;
        round.active = None;
        if round.previous.get(&key) != Some(&summary) {
            round.previous.insert(key, summary);
            if let Some(dependents) = round.dependents.get(&index) {
                *fuel = fuel
                    .checked_sub(dependents.len())
                    .ok_or("memory dependency scheduling budget exhausted")?;
                for dependent in dependents {
                    if round.scheduled.insert(*dependent) {
                        round.pending.push_back(*dependent);
                    }
                }
            }
        }
    }
    let mut summary = round
        .previous
        .remove(&round.queue[0])
        .ok_or("missing root memory summary")?;
    // Value snapshots cover construction and delivery. Pointer-slot mutation
    // needs a cleanup-time storage transition proof before these reads suffice.
    summary.may_run_cleanup |= round.used_external_cleanup && round.mutates_pointer_storage;
    if !borrowed.is_empty()
        && (summary.may_run_cleanup || summary.accesses.iter().any(|access| access.write))
    {
        return Err("borrowed callable premises require a stable read-only memory contract".into());
    }
    Ok(summary)
}
// Import the same completed cleanup footprint for ordinary and composed calls.
fn finish_instance_summary(
    package: &Package,
    declaration: DeclarationId,
    arguments: &[Term],
    summary: &mut Summary,
    fuel: &mut usize,
) -> Result<(), String> {
    summary.arguments = arguments.to_vec();
    if let Some(instance) = package
        .runtime_instances
        .iter()
        .find(|instance| instance.declaration == declaration && instance.arguments == arguments)
    {
        for access in &instance.cleanup_accesses {
            let cost = access.through.iter().fold(
                access
                    .input_fields
                    .len()
                    .saturating_add(access.fields.len())
                    .saturating_add(1),
                |cost, path| cost.saturating_add(path.len()).saturating_add(1),
            );
            *fuel = fuel
                .checked_sub(cost)
                .ok_or("cleanup footprint import budget exhausted")?;
            bound_type(&access.ty, fuel)?;
            validate_cleanup_access(package, &summary.signature, access, fuel)?;
            summary.accesses.push(Access {
                target: ParameterPlace {
                    parameter: access.parameter,
                    input_fields: access.input_fields.clone(),
                    through: access.through.clone(),
                    conditions: access.conditions.clone(),
                    fields: access.fields.clone(),
                },
                ty: access.ty.clone(),
                write: access.write,
                read_value: access.read_value,
            });
        }
    }

    if summary.accesses.len() > *fuel {
        return Err("memory summary budget exhausted".into());
    }
    *fuel -= summary.accesses.len();
    summary.accesses.sort_by_cached_key(ToString::to_string);
    summary.accesses.dedup();
    Ok(())
}

// Type keys are walked iteratively before recursive cloning/substitution. This
// is a structural work limit, not a type proposition or normalization step.
pub(crate) fn bound_type(ty: &Term, fuel: &mut usize) -> Result<(), String> {
    let mut pending = vec![(ty, 0)];
    while let Some((ty, depth)) = pending.pop() {
        if *fuel == 0 || depth >= 64 {
            return Err("memory summary type budget exhausted".into());
        }
        *fuel -= 1;
        match ty {
            Term::ErrorResult { errors, result } => {
                if errors.len().saturating_add(1) > *fuel {
                    return Err("memory summary type budget exhausted".into());
                }
                pending.push((result, depth + 1));
                pending.extend(errors.iter().map(|ty| (ty, depth + 1)));
            }
            Term::Computation { effects, result } => {
                *fuel = fuel
                    .checked_sub(effects.len())
                    .ok_or("memory summary type budget exhausted")?;
                let width = effects.iter().try_fold(1usize, |n, operation| {
                    n.checked_add(operation.arguments.len())
                });
                if width.is_none_or(|n| n > *fuel) {
                    return Err("memory summary type budget exhausted".into());
                }
                pending.push((result, depth + 1));
                for operation in effects {
                    pending.extend(operation.arguments.iter().map(|ty| (ty, depth + 1)));
                }
            }
            Term::AssociatedType {
                arguments, subject, ..
            } => {
                pending.push((subject, depth + 1));
                pending.extend(arguments.iter().map(|ty| (ty, depth + 1)));
            }
            Term::Optional(inner)
            | Term::Pointer(inner)
            | Term::ComptimeFunction(inner)
            | Term::UnsafeFunction(inner) => pending.push((inner, depth + 1)),
            Term::OptionalValue { ty, value } => {
                pending.push((ty, depth + 1));
                if let Some(value) = value {
                    pending.push((value, depth + 1));
                }
            }
            Term::ErrorValue { ty, label, value } => {
                pending.push((ty, depth + 1));
                if let Some(label) = label {
                    pending.push((label, depth + 1));
                }
                pending.push((value, depth + 1));
            }
            Term::NominalValue { ty, fields, .. } => {
                pending.push((ty, depth + 1));
                pending.extend(fields.iter().map(|field| (field, depth + 1)));
            }
            Term::Record(fields) | Term::RecordValue(fields) => {
                pending.extend(fields.iter().map(|(_, ty)| (ty, depth + 1)))
            }
            Term::Tuple(fields)
            | Term::TypeListValue(fields)
            | Term::TypeListConcat(fields)
            | Term::TupleValue(fields)
            | Term::Nominal {
                arguments: fields, ..
            }
            | Term::NormalForm {
                arguments: fields, ..
            } => pending.extend(fields.iter().map(|ty| (ty, depth + 1))),
            Term::Closure {
                environment,
                signature,
            } => {
                pending.push((signature, depth + 1));
                pending.extend(environment.iter().map(|ty| (ty, depth + 1)));
            }
            Term::Function { parameters, result } => {
                pending.push((result, depth + 1));
                pending.extend(
                    parameters
                        .iter()
                        .map(|parameter| (&parameter.ty, depth + 1)),
                );
            }
            _ => (),
        }
    }
    Ok(())
}
fn derive_with_borrowed(
    package: &Package,
    declaration: DeclarationId,
    arguments: &[Term],
    inputs: &BTreeMap<CallableInput, CallableResult>,
    prepared: &PreparedCodeInputs,
    borrowed: &BorrowedCallableInputs,
    fuel: &mut usize,
    round: &mut Round,
) -> Result<Summary, String> {
    if round.complete_calls {
        return infer_borrowed_context(
            package,
            declaration,
            arguments,
            inputs,
            prepared,
            &round.effects,
            borrowed,
            fuel,
        );
    }
    if *fuel == 0 {
        return Err("memory summary budget exhausted".into());
    }
    *fuel -= 1;
    for argument in arguments {
        bound_type(argument, fuel)?;
    }
    for (input, value) in inputs {
        *fuel = fuel
            .checked_sub(input.fields.len() + 1)
            .ok_or("memory input budget exhausted")?;
        value.charge(fuel)?;
    }
    let mut prepared_ids = BTreeMap::new();
    for (input, values) in prepared {
        *fuel = fuel
            .checked_sub(input.fields.len() + 1)
            .ok_or("prepared code instance budget exhausted")?;
        let mut ids = vec![];
        for value in values {
            value.charge(fuel)?;
            let mut found = None;
            for (id, previous) in round.code.iter().enumerate() {
                previous.charge(fuel)?;
                if previous == value {
                    found = Some(id);
                    break;
                }
            }
            let id = found.unwrap_or_else(|| {
                let id = round.code.len();
                round.code.push(value.clone());
                id
            });
            ids.push(id);
        }
        ids.sort_unstable();
        ids.dedup();
        prepared_ids.insert(input.clone(), ids);
    }
    borrowed_code::charge(borrowed, fuel)?;
    let key = MemoryInstance {
        borrowed: borrowed.clone(),
        effects: effects::copy_inputs(&round.effects, fuel)?,
        instance: iir::InstanceKey {
            declaration,
            arguments: arguments.to_vec(),
        },
        inputs: inputs.clone(),
        prepared: prepared_ids,
    };
    let index = match round.discovered.get(&key) {
        Some(index) => *index,
        None => {
            let index = round.queue.len();
            round.discovered.insert(key.clone(), index);
            round.queue.push(key.clone());
            round.pending.push_back(index);
            round.scheduled.insert(index);
            index
        }
    };
    // Record dependencies even when the callee still has only a bottom summary.
    // Revisit callers only after a consumed summary changes. Old edges remain
    // conservative when later iterations discover different callable targets.
    if let Some(caller) = round.active {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("memory dependency budget exhausted")?;
        round.dependents.entry(index).or_default().insert(caller);
    }
    if let Some(summary) = round.previous.get(&key) {
        effects::charge_inputs(&summary.effect_inputs, fuel)?;
        for (path, targets) in &summary.pointer_fields {
            *fuel = fuel
                .checked_sub(path.len() + 1)
                .ok_or("pointer result field budget exhausted")?;
            if let Some(targets) = targets {
                for target in targets {
                    *fuel = fuel
                        .checked_sub(target.path_cost())
                        .ok_or("pointer result target budget exhausted")?;
                }
            }
        }
        summary.callable_result.charge(fuel)?;
        charge_callee_fields(&summary.callable_fields, fuel)?;
        for (path, value) in &summary.code_fields {
            *fuel = fuel
                .checked_sub(path.len() + 1)
                .ok_or("code result field budget exhausted")?;
            value.charge(fuel)?;
        }
        return Ok(summary.clone());
    }
    let (function, body_arguments) = package
        .runtime_body(declaration, arguments)
        .ok_or("memory summary function is missing")?;
    if function.phase != FunctionPhase::Runtime
        || function.type_parameters.len() != body_arguments.len()
    {
        return Err("memory summary needs a concrete runtime body".into());
    }
    bound_type(&function.signature, fuel)?;
    let signature = function
        .signature
        .instantiate_parameters(
            &round.binders(Some(package), &function.type_parameters, fuel)?,
            body_arguments,
        )
        .map_err(|error| error.to_string())?;
    bound_type(&signature, fuel)?;
    let Term::Function { result, .. } = signature.callable_signature() else {
        return Err("memory summary requires a function signature".into());
    };
    let callable_fields = result_callable_bottom(Some(package), result, fuel)?;
    let code_fields = code_result_bottom(Some(package), result, fuel)?;
    let result_value = if let Term::Computation { result, .. } = result.as_ref() {
        result
    } else {
        result
    };
    let pointer_fields = if matches!(
        result_value.as_ref(),
        Term::Tuple(_)
            | Term::Record(_)
            | Term::Nominal { .. }
            | Term::NormalForm { .. }
            | Term::Closure { .. }
            | Term::ErrorResult { .. }
            | Term::Optional(_)
    ) && pointer_representation(Some(package), result_value, fuel)?
        .is_none()
    {
        symbolic_pointer_fields(Some(package), result_value, fuel)?
            .into_keys()
            .map(|path| (path, Some(BTreeSet::new())))
            .collect()
    } else {
        BTreeMap::new()
    };
    // A provisional bottom footprint is consumed only inside this worklist.
    // Every queued body must validate and all summaries must converge first.
    Ok(Summary {
        declaration,
        arguments: arguments.to_vec(),
        signature,
        callable_inputs: inputs.clone(),
        prepared_inputs: prepared.clone(),
        borrowed_inputs: borrowed.clone(),
        effect_inputs: effects::copy_inputs(&round.effects, fuel)?,
        accesses: vec![],
        may_reject: false,
        may_run_cleanup: false,
        result_targets: Some(BTreeSet::new()),
        pointer_fields,
        callable_result: CallableResult::NoReturn,
        callable_fields,
        code_fields,
    })
}
fn validate_code_premise(
    package: &Package,
    round: &mut Round,
    expected: &Term,
    value: &CallableResult,
    fuel: &mut usize,
) -> Result<(), String> {
    // NoReturn is private worklist bottom for both named and environment-
    // bearing code. It grants no callable identity or successful invocation.
    if matches!(value, CallableResult::NoReturn)
        && matches!(
            expected,
            Term::Function { .. } | Term::UnsafeFunction(_) | Term::Closure { .. }
        )
    {
        return Ok(());
    }
    if !matches!(expected, Term::Function { .. } | Term::UnsafeFunction(_)) {
        return Err("memory callable premise path is not a function".into());
    }
    if !matches!(
        value,
        CallableResult::Known(_) | CallableResult::Candidates(_)
    ) || value.keys().is_empty()
    {
        return Err("memory callable premise requires named code identities".into());
    }
    value.charge(fuel)?;
    let mut seen = HashSet::new();
    for key in value.keys() {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("memory callable validation budget exhausted")?;
        if !seen.insert(key) {
            return Err("duplicate memory callable candidate".into());
        }
        let (function, body_arguments) = package
            .runtime_body(key.declaration, &key.arguments)
            .ok_or("memory callable definition is missing")?;
        if function.phase != FunctionPhase::Runtime
            || function.type_parameters.len() != body_arguments.len()
        {
            return Err("memory callable requires a concrete runtime instance".into());
        }
        bound_type(&function.signature, fuel)?;
        let signature = function
            .signature
            .instantiate_parameters(
                &round.binders(Some(package), &function.type_parameters, fuel)?,
                body_arguments,
            )
            .map_err(|error| error.to_string())?;
        let signature = signature.with_unsafe_call_requirement(function.is_unsafe);
        if &signature != expected {
            return Err("memory callable signature disagrees with parameter".into());
        }
    }
    Ok(())
}
fn infer_body(
    function: &Function,
    arguments: &[Term],
    inputs: &BTreeMap<CallableInput, CallableResult>,
    prepared: &PreparedCodeInputs,
    borrowed: &BorrowedCallableInputs,
    package: Option<&Package>,
    fuel: &mut usize,
    round: &mut Round,
) -> Result<Summary, String> {
    if *fuel == 0 {
        return Err("memory summary budget exhausted".into());
    }
    *fuel -= 1;
    if function.phase != FunctionPhase::Runtime || function.type_parameters.len() != arguments.len()
    {
        return Err("memory summary needs a concrete runtime body".into());
    }
    let declarations = round.binders(package, &function.type_parameters, fuel)?;
    let signature = function
        .signature
        .instantiate_parameters(&declarations, arguments)
        .map_err(|error| error.to_string())?;
    let Term::Function {
        parameters, result, ..
    } = signature.callable_signature()
    else {
        return Err("memory summary requires a function signature".into());
    };
    if parameters.len() != function.parameters.len() {
        return Err("memory summary parameter signature mismatch".into());
    }
    for (input, value) in inputs {
        let parameter = parameters
            .get(input.parameter)
            .ok_or("memory callable premise parameter is missing")?;
        let expected = owned_path_type(package, &parameter.ty, &input.fields, fuel)?;
        validate_code_premise(
            package.ok_or("memory callable premise requires a package")?,
            round,
            &expected,
            value,
            fuel,
        )?;
    }
    let frames = vec![parameters.iter().map(|p| p.sort).collect()];
    let mut type_context = crate::ptc::Context::with_scope(frames.clone());
    for requirement in &function.requirements {
        let formula = crate::ptc::Formula::from_predicate(requirement);
        if !crate::ptc::within_limits(&formula, 128, fuel) {
            return Err("memory type contract budget exhausted".into());
        }
        let requirement = requirement
            .instantiate_parameters_in_scope(&declarations, arguments, &frames)
            .map_err(|e| e.to_string())?;
        type_context.assume(crate::ptc::Formula::from_predicate(&requirement));
    }
    let code_inputs = borrow_code_inputs(prepared);
    let mut infer = Infer {
        scope: &function.body,
        borrowed_callable_slots: BTreeMap::new(),
        local_code: vec![],
        borrowed_code_pointers: BTreeMap::new(),
        stable_capture_roots: BTreeSet::new(),
        capture_code_writes: BTreeMap::new(),
        named_code_slots: HashMap::new(),
        closure_nodes: HashMap::new(),
        local_code_results: HashMap::new(),
        local_code_fields: HashMap::new(),
        type_context,
        type_views: HashMap::new(),
        fuel,
        package,
        parameters: &function.type_parameters,
        declarations,
        arguments,
        round,
        effect_row: vec![],
        environment: Environment::default(),
        locals: BTreeMap::new(),
        private_storage_roots: BTreeSet::new(),
        local_storage: vec![],
        loop_states: vec![],
        accesses: vec![],
        may_reject: false,
        may_run_cleanup: false,
        expression_results: HashMap::new(),
        callable_results: HashMap::new(),
        aggregate_callees: HashMap::new(),
        aggregate_pointers: HashMap::new(),
        returns: Some(BTreeSet::new()),
        pointer_field_returns: None,
        input_pointer_slots: BTreeMap::new(),
        input_code_slots: BTreeMap::new(),
        input_code_exits: BTreeMap::new(),
        input_pointer_exits: BTreeMap::new(),
        callable_returns: CallableResult::NoReturn,
        callable_field_returns: BTreeMap::new(),
        code_field_returns: BTreeMap::new(),
    };
    infer.borrowed_callable_slots =
        borrowed_code::validate(package, infer.round, parameters, borrowed, infer.fuel)?;
    let result_view = infer.effect_completion(result)?;
    if let Term::Computation { effects, .. } = result.as_ref() {
        infer.effect_row = effects.clone();
    }
    if !intrinsic_pointer_value(&result_view)
        && result_view != Term::Primitive(Primitive::Void)
        && !infer.local_pointer_value(&result_view)?
    {
        symbolic_pointer_fields(package, &result_view, infer.fuel).map_err(|reason| {
            format!("memory result requires an escape or ownership contract: {reason}")
        })?;
    }
    infer.callable_field_returns = result_callable_bottom(package, result, infer.fuel)?;
    infer.code_field_returns = code_result_bottom(package, result, infer.fuel)?;
    for (index, (binding, parameter)) in function.parameters.iter().zip(parameters).enumerate() {
        infer.step(0)?;
        infer.note_owned_cleanup(&parameter.ty)?;
        infer.locals.insert(*binding, parameter.ty.clone());
        // Type inputs select types, not runtime owned storage or pointer roots.
        // Keep their positional slot and local annotation for body inspection.
        if parameter.sort == nir::Sort::Type && parameter.ty == Term::Primitive(Primitive::Type) {
            continue;
        }
        // Parameter premises also carry closure-code bottom during the
        // worklist. The published named-result lattice omits closure leaves,
        // so it cannot be used to install these input facts.
        for (fields, _) in owned_callable_bottom(package, &parameter.ty, infer.fuel, true)? {
            infer.environment.callees.insert(
                CallablePlace {
                    binding: *binding,
                    fields: fields.clone(),
                },
                inputs
                    .get(&CallableInput {
                        parameter: index,
                        fields: fields.clone(),
                    })
                    .cloned()
                    .unwrap_or_else(|| CallableResult::Forwarded {
                        named: vec![],
                        inputs: vec![CallableInput {
                            parameter: index,
                            fields,
                        }],
                    }),
            );
        }
        if !intrinsic_pointer_value(&parameter.ty) && !infer.local_pointer_value(&parameter.ty)? {
            infer.install_input_pointer_fields(*binding, index, &parameter.ty)?;
        }
        if infer.local_pointer_value(&parameter.ty)? {
            infer.environment.pointers.insert(
                *binding,
                Some(BTreeSet::from([PointerPlace {
                    root: PointerRoot::Parameter(index),
                    input_fields: vec![],
                    through: vec![],
                    conditions: BTreeMap::new(),
                    fields: vec![],
                }])),
            );
        }
    }
    infer.install_code_inputs(&function.parameters, parameters, inputs, &code_inputs)?;
    infer.record_parameter_cleanup_reads(&function.parameters, parameters, function.body.source)?;
    for predicate in &function.preconditions {
        if !nir::program::checkable_predicate(predicate) {
            return Err("invalid runtime predicate check".into());
        }
        infer.walk_guard(predicate, 0)?;
        infer.may_reject |= infer.known_bool(predicate, 0)? != Some(true);
    }
    infer.prepare_private_storage_roots(&function.body)?;
    infer.walk(&function.body, 0)?;
    let completes = !super::pattern_facts::cannot_complete(&function.body, infer.fuel, 0)?;
    if completes && infer.ty(&function.body.ty)? == **result {
        infer.merge_callable_field_return(&function.body)?;
        infer.merge_pointer_field_return(&function.body)?;
    }
    let pointer_result = infer.local_pointer_value(&result_view)?;
    let body_ty = infer.ty(&function.body.ty)?;
    let body_completion = infer.effect_completion(&body_ty)?;
    if completes && pointer_result && infer.local_pointer_value(&body_completion)? {
        let targets = infer.optional_target(&function.body, 0)?;
        infer.merge_return(targets)?;
    }
    let callable_result = matches!(result_view, Term::Function { .. } | Term::UnsafeFunction(_));
    if completes
        && callable_result
        && matches!(
            body_completion,
            Term::Function { .. } | Term::UnsafeFunction(_)
        )
    {
        let key = infer.callee_key(&function.body)?;
        infer.callable_returns.join(&key, infer.fuel)?;
    }
    if pointer_result && let Some(binding) = function.result_binding {
        // Exit predicates observe the actual returned storage, including all
        // possible return paths. Leaving this binding absent makes an otherwise
        // read-only function opaque to callers merely because it has a contract.
        let targets = infer
            .returns
            .as_ref()
            .map(|targets| {
                let mut mapped = BTreeSet::new();
                for target in targets {
                    *infer.fuel = infer
                        .fuel
                        .checked_sub(target.path_cost())
                        .ok_or("memory result target budget exhausted")?;
                    mapped.insert(PointerPlace {
                        root: PointerRoot::Parameter(target.parameter),
                        input_fields: target.input_fields.clone(),
                        through: target.through.clone(),
                        conditions: target.conditions.clone(),
                        fields: target.fields.clone(),
                    });
                }
                Ok::<_, String>(mapped)
            })
            .transpose()?;
        infer.environment.pointers.insert(binding, targets);
    }
    for predicate in &function.postconditions {
        if !nir::program::checkable_predicate(predicate) {
            return Err("invalid runtime predicate check".into());
        }
        infer.walk_guard(predicate, 0)?;
        infer.may_reject |= infer.known_bool(predicate, 0)? != Some(true);
    }
    Ok(Summary {
        declaration: function.declaration,
        arguments: arguments.to_vec(),
        signature,
        callable_inputs: inputs.clone(),
        prepared_inputs: prepared.clone(),
        borrowed_inputs: borrowed.clone(),
        effect_inputs: effects::copy_inputs(&infer.round.effects, infer.fuel)?,
        accesses: infer.accesses,
        code_fields: infer.code_field_returns,
        may_reject: infer.may_reject,
        may_run_cleanup: infer.may_run_cleanup,
        result_targets: if pointer_result { infer.returns } else { None },
        pointer_fields: infer.pointer_field_returns.unwrap_or_default(),
        callable_fields: infer.callable_field_returns,
        callable_result: if callable_result {
            infer.callable_returns
        } else {
            CallableResult::Unknown
        },
    })
}

/// Validate the normalized evaluation plan, including declaration-ordered
/// defaults after all explicit source-ordered arguments.
pub(crate) fn validate_argument_plan(
    arguments: &[nir::program::CallArgument],
    bindings: &[LocalId],
    count: usize,
) -> Result<(), String> {
    if arguments.len() != count {
        return Err("invalid memory call argument count".into());
    }
    let has_defaults = arguments.iter().any(|argument| argument.is_default);
    if has_defaults
        && (bindings.len() != count
            || bindings
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != count)
    {
        return Err("invalid memory default parameter frame".into());
    }
    let mut slots = HashSet::new();
    let mut last_default = None;
    for argument in arguments {
        if argument.parameter >= count || !slots.insert(argument.parameter) {
            return Err("invalid memory call argument slot".into());
        }
        if argument.is_default {
            if last_default.is_some_and(|last| last >= argument.parameter) {
                return Err("memory defaults are not in declaration order".into());
            }
            last_default = Some(argument.parameter);
        } else if last_default.is_some() {
            return Err("explicit memory argument follows a default".into());
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
pub struct ArgumentSummary {
    pub accesses: Vec<Access>,
    pub code_fields: CodeFields,
    /// May reject a checked program predicate; not an error-result ABI effect.
    pub may_reject: bool,
    /// Owned cleanup may access memory beyond the explicit body footprint.
    pub may_run_cleanup: bool,
    pub pointer_fields: BTreeMap<Vec<usize>, Option<BTreeSet<ParameterPlace>>>,
    pub result_targets: Option<BTreeSet<ParameterPlace>>,
    /// Origins remaining in formal pointer inputs on normal completion. These
    /// are local input cells, not writes to ordinary caller arguments. Borrowed
    /// closure capture inputs are the only environment slots represented here.
    /// None includes unknown or invocation-local origins; absence proves nothing.
    pub input_pointer_exits: BTreeMap<CallableInput, Option<BTreeSet<ParameterPlace>>>,
    /// Code identities in formal input cells on normal completion. Unknown
    /// identities and borrowed environments without pointer-exit slots are not exported. Like
    /// input_pointer_exits, this does not imply writes to by-value arguments.
    pub input_code_exits: BTreeMap<CallableInput, Option<Vec<PreparedCode>>>,
}
impl fmt::Display for ArgumentSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(argument-memory (accesses")?;
        for access in &self.accesses {
            write!(f, " {access}")?;
        }
        write!(
            f,
            ") (may-reject {}) (may-run-cleanup {}) (result-targets",
            self.may_reject, self.may_run_cleanup
        )?;
        if let Some(targets) = &self.result_targets {
            for target in targets {
                write!(f, " {target}")?;
            }
        } else {
            f.write_str(" unknown")?;
        }
        f.write_str(")")?;
        display_pointer_fields(&self.pointer_fields, f)?;
        display_code_fields(&self.code_fields, f)?;
        if !self.input_code_exits.is_empty() {
            f.write_str(" (input-code-exits")?;
            for (input, codes) in &self.input_code_exits {
                write!(f, " (input (parameter {}) (fields", input.parameter)?;
                display_owned_path(&input.fields, f)?;
                f.write_str(") (codes")?;
                if let Some(codes) = codes {
                    for code in codes {
                        write!(f, " {code}")?;
                    }
                } else {
                    f.write_str(" unknown")?;
                }
                f.write_str("))")?;
            }
            f.write_str(")")?;
        }
        if !self.input_pointer_exits.is_empty() {
            f.write_str(" (input-pointer-exits")?;
            for (input, targets) in &self.input_pointer_exits {
                write!(f, " (input (parameter {}) (fields", input.parameter)?;
                for field in &input.fields {
                    write!(f, " {field}")?;
                }
                f.write_str(") (targets")?;
                if let Some(targets) = targets {
                    for target in targets {
                        write!(f, " {target}")?;
                    }
                } else {
                    f.write_str(" unknown")?;
                }
                f.write_str("))")?;
            }
            f.write_str(")")?;
        }
        f.write_str(")")
    }
}
/// Inspect a default in an isolated prepared-parameter environment. `slot` is
/// the original callee parameter index, not the order of explicit arguments.
pub fn infer_argument(
    package: &Package,
    expression: &Expr,
    prepared: &[(LocalId, Term, usize)],
    fuel: &mut usize,
) -> Result<ArgumentSummary, String> {
    if !intrinsic_pointer_value(&expression.ty)
        && !matches!(
            expression.ty,
            Term::Function { .. } | Term::UnsafeFunction(_)
        )
        && expression.ty != Term::Primitive(Primitive::Void)
        && pointer_representation(Some(package), &expression.ty, fuel)?.is_none()
    {
        // The footprint only needs to rule out untracked escaping pointers.
        // Ownership, moves and cleanup are checked separately by the caller's
        // isolated default-evaluation frame, including aggregate results.
        symbolic_pointer_fields(Some(package), &expression.ty, fuel).map_err(|reason| {
            format!("default argument requires an ownership contract: {reason}")
        })?;
    }
    let mut round = Round {
        complete_calls: true,
        ..Default::default()
    };
    let mut infer = Infer {
        scope: expression,
        borrowed_callable_slots: BTreeMap::new(),
        local_code: vec![],
        borrowed_code_pointers: BTreeMap::new(),
        stable_capture_roots: BTreeSet::new(),
        capture_code_writes: BTreeMap::new(),
        named_code_slots: HashMap::new(),
        closure_nodes: HashMap::new(),
        local_code_results: HashMap::new(),
        local_code_fields: HashMap::new(),
        type_context: crate::ptc::Context::default(),
        type_views: HashMap::new(),
        fuel,
        package: Some(package),
        parameters: &[],
        declarations: vec![],
        arguments: &[],
        round: &mut round,
        effect_row: vec![],
        environment: Environment::default(),
        locals: BTreeMap::new(),
        private_storage_roots: BTreeSet::new(),
        local_storage: vec![],
        loop_states: vec![],
        accesses: vec![],
        may_reject: false,
        may_run_cleanup: false,
        expression_results: HashMap::new(),
        callable_results: HashMap::new(),
        aggregate_callees: HashMap::new(),
        aggregate_pointers: HashMap::new(),
        returns: Some(BTreeSet::new()),
        pointer_field_returns: None,
        input_pointer_slots: BTreeMap::new(),
        input_code_slots: BTreeMap::new(),
        input_code_exits: BTreeMap::new(),
        input_pointer_exits: BTreeMap::new(),
        callable_returns: CallableResult::NoReturn,
        callable_field_returns: BTreeMap::new(),
        code_field_returns: BTreeMap::new(),
    };
    for (binding, ty, slot) in prepared {
        infer.step(0)?;
        bound_type(ty, infer.fuel)?;
        infer.locals.insert(*binding, ty.clone());
        if infer.local_pointer_value(ty)? {
            infer.environment.pointers.insert(
                *binding,
                Some(BTreeSet::from([PointerPlace {
                    root: PointerRoot::Parameter(*slot),
                    input_fields: vec![],
                    through: vec![],
                    conditions: BTreeMap::new(),
                    fields: vec![],
                }])),
            );
        } else {
            infer.install_input_pointer_fields(*binding, *slot, ty)?;
        }
    }
    infer.prepare_private_storage_roots(expression)?;
    infer.walk(expression, 0)?;
    if !super::pattern_facts::cannot_complete(expression, infer.fuel, 0)? {
        infer.merge_pointer_field_return(expression)?;
    }
    let result_targets = if infer.local_pointer_value(&expression.ty)? {
        infer
            .optional_target(expression, 0)?
            .map(|targets| {
                targets
                    .into_iter()
                    .map(PointerPlace::external)
                    .collect::<Result<BTreeSet<_>, _>>()
            })
            .transpose()?
    } else {
        None
    };
    Ok(ArgumentSummary {
        input_pointer_exits: infer.input_pointer_exits,
        input_code_exits: infer.input_code_exits,
        pointer_fields: infer.pointer_field_returns.unwrap_or_default(),
        accesses: infer.accesses,
        code_fields: infer.code_field_returns,
        may_reject: infer.may_reject,
        may_run_cleanup: infer.may_run_cleanup,
        result_targets,
    })
}

/// Parameter-relative footprint of a runtime closure with pointer-free captures.
/// Named callees use complete package fixed points; no synthetic declaration
/// is registered for the anonymous body or published as a callable identity.
pub fn infer_closure(
    package: &Package,
    definition: &Expr,
    fuel: &mut usize,
) -> Result<ArgumentSummary, String> {
    infer_closure_with_callees(package, definition, &BTreeMap::new(), fuel)
}

/// Conditional footprint: the caller establishes each captured code identity.
/// Matching signatures validate the premise's shape, not its truth at runtime.
/// Missing inputs remain unknown; none are guessed from capture types.
pub fn infer_closure_with_callees(
    package: &Package,
    definition: &Expr,
    callees: &BTreeMap<LocalId, CallableResult>,
    fuel: &mut usize,
) -> Result<ArgumentSummary, String> {
    let mut fields = BTreeMap::new();
    for (binding, key) in callees {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("closure memory input budget exhausted")?;
        key.charge(fuel)?;
        fields.insert(CallablePlace::root(*binding), key.clone());
    }
    infer_closure_with_fields(package, definition, &fields, fuel)
}

/// Conditional code premises at checked owned field paths inside captures.
pub fn infer_closure_with_fields(
    package: &Package,
    definition: &Expr,
    callees: &BTreeMap<CallablePlace, CallableResult>,
    fuel: &mut usize,
) -> Result<ArgumentSummary, String> {
    infer_closure_with_inputs(package, definition, callees, &BTreeMap::new(), fuel)
}

/// Capture premises and delivered parameter premises occupy distinct frames.
pub fn infer_closure_with_inputs(
    package: &Package,
    definition: &Expr,
    callees: &BTreeMap<CallablePlace, CallableResult>,
    inputs: &BTreeMap<CallableInput, CallableResult>,
    fuel: &mut usize,
) -> Result<ArgumentSummary, String> {
    infer_closure_with_code_inputs(package, definition, callees, inputs, &BTreeMap::new(), fuel)
}

/// Prepared anonymous or mixed code arguments supplement named input premises.
/// Capture initializers have already run; importing evidence never replays them.
pub fn infer_closure_with_code_inputs(
    package: &Package,
    definition: &Expr,
    callees: &BTreeMap<CallablePlace, CallableResult>,
    inputs: &BTreeMap<CallableInput, CallableResult>,
    code_inputs: &CodeInputs<'_>,
    fuel: &mut usize,
) -> Result<ArgumentSummary, String> {
    infer_closure_with_prepared_captures(
        package,
        definition,
        callees,
        &PreparedCaptures::new(),
        inputs,
        code_inputs,
        fuel,
    )
}
/// Captured anonymous code carries construction-time environments independently
/// from invocation argument evidence. Neither establishes pointer validity.
pub fn infer_closure_with_prepared_captures(
    package: &Package,
    definition: &Expr,
    callees: &BTreeMap<CallablePlace, CallableResult>,
    prepared_captures: &PreparedCaptures,
    inputs: &BTreeMap<CallableInput, CallableResult>,
    code_inputs: &CodeInputs<'_>,
    fuel: &mut usize,
) -> Result<ArgumentSummary, String> {
    infer_closure_with_prepared_effect_context(
        package,
        definition,
        callees,
        prepared_captures,
        inputs,
        code_inputs,
        &[],
        fuel,
    )
}
fn infer_closure_with_prepared_effect_context(
    package: &Package,
    definition: &Expr,
    callees: &BTreeMap<CallablePlace, CallableResult>,
    prepared_captures: &PreparedCaptures,
    inputs: &BTreeMap<CallableInput, CallableResult>,
    code_inputs: &CodeInputs<'_>,
    effects: &[EffectInput],
    fuel: &mut usize,
) -> Result<ArgumentSummary, String> {
    let mut round = Round {
        complete_calls: true,
        effects: effects::copy_inputs(effects, fuel)?,
        ..Default::default()
    };
    infer_closure_in_round(
        package,
        definition,
        callees,
        prepared_captures,
        inputs,
        code_inputs,
        fuel,
        &mut round,
        (&[], &[]),
        false,
    )
}

/// Captures containing pointer leaves are modeled as additional inputs, in
/// capture order after ordinary inputs. The caller must map each virtual slot's
/// field paths to the environment storage's checked pointee origins. This view does not
/// authorize a pointer-bearing closure passed through an unknown code interface.
pub fn infer_closure_with_borrowed_captures(
    package: &Package,
    definition: &Expr,
    callees: &BTreeMap<CallablePlace, CallableResult>,
    prepared_captures: &PreparedCaptures,
    inputs: &BTreeMap<CallableInput, CallableResult>,
    code_inputs: &CodeInputs<'_>,
    fuel: &mut usize,
) -> Result<ArgumentSummary, String> {
    infer_closure_with_borrowed_effect_context(
        package,
        definition,
        callees,
        prepared_captures,
        inputs,
        code_inputs,
        &[],
        fuel,
    )
}
fn infer_closure_with_borrowed_effect_context(
    package: &Package,
    definition: &Expr,
    callees: &BTreeMap<CallablePlace, CallableResult>,
    prepared_captures: &PreparedCaptures,
    inputs: &BTreeMap<CallableInput, CallableResult>,
    code_inputs: &CodeInputs<'_>,
    effects: &[EffectInput],
    fuel: &mut usize,
) -> Result<ArgumentSummary, String> {
    let mut round = Round {
        complete_calls: true,
        effects: effects::copy_inputs(effects, fuel)?,
        ..Default::default()
    };
    infer_closure_with_borrowed_in_round(
        package,
        definition,
        callees,
        prepared_captures,
        inputs,
        code_inputs,
        &mut round,
        fuel,
        false,
        false,
    )
}
fn closure_transition_slots(
    package: &Package,
    captures: &[nir::program::Capture],
    parameter_count: usize,
    include_code: bool,
    fuel: &mut usize,
) -> Result<BTreeMap<LocalId, usize>, String> {
    let mut slots = BTreeMap::new();
    for capture in captures {
        if !symbolic_pointer_fields(Some(package), &capture.ty, fuel)?.is_empty() {
            slots.insert(capture.binding, parameter_count + slots.len());
        }
    }
    if include_code {
        for capture in captures {
            if !slots.contains_key(&capture.binding) {
                let fields = owned_callable_bottom(Some(package), &capture.ty, fuel, true)?;
                if !fields.is_empty() {
                    slots.insert(capture.binding, parameter_count + slots.len());
                }
            }
        }
    }
    Ok(slots)
}

fn infer_closure_with_borrowed_in_round(
    package: &Package,
    definition: &Expr,
    callees: &BTreeMap<CallablePlace, CallableResult>,
    prepared_captures: &PreparedCaptures,
    inputs: &BTreeMap<CallableInput, CallableResult>,
    code_inputs: &CodeInputs<'_>,
    round: &mut Round,
    fuel: &mut usize,
    stable_captures: bool,
    transition_code_captures: bool,
) -> Result<ArgumentSummary, String> {
    let K::Closure { captures, body, .. } = &definition.kind else {
        return Err("closure memory requires a closure definition".into());
    };
    *fuel = fuel
        .checked_sub(captures.len())
        .ok_or("borrowed capture budget exhausted")?;
    let mut pointer_bindings = BTreeSet::new();
    for capture in captures {
        if !symbolic_pointer_fields(Some(package), &capture.ty, fuel)?.is_empty() {
            pointer_bindings.insert(capture.binding);
        }
    }
    let Term::Function {
        parameters: signature_parameters,
        ..
    } = definition.ty.callable_signature()
    else {
        return Err("closure memory requires a function signature".into());
    };
    let capture_slots = closure_transition_slots(
        package,
        captures,
        signature_parameters.len(),
        transition_code_captures,
        fuel,
    )?;
    if capture_slots.is_empty() {
        return infer_closure_in_round(
            package,
            definition,
            callees,
            prepared_captures,
            inputs,
            code_inputs,
            fuel,
            round,
            (&[], &[]),
            stable_captures,
        );
    }
    // Bound the complete retained tree before cloning, but validate storage
    // effects only where the borrowed environment is actually in scope.
    nir::visit::expression(definition, fuel, |expr, fuel| bound_type(&expr.ty, fuel))?;
    validate_borrowed_capture_storage(true, Some(package), body, &pointer_bindings, fuel)?;
    for capture in captures {
        bound_type(&capture.ty, fuel)?;
    }
    let Term::Closure {
        environment,
        signature,
    } = &definition.ty
    else {
        return Err("borrowed capture lacks its environment type".into());
    };
    if environment.len() != captures.len()
        || environment
            .iter()
            .zip(captures)
            .any(|(ty, capture)| ty != &capture.ty)
    {
        return Err("closure memory capture layout mismatch".into());
    }
    let Term::Function { parameters, result } = signature.as_ref() else {
        return Err("closure memory requires a function signature".into());
    };
    let mut checked = definition.clone();
    let K::Closure {
        parameters: bindings,
        captures: owned,
        ..
    } = &mut checked.kind
    else {
        unreachable!()
    };
    let mut inputs_ty = parameters.clone();
    let mut lifted = captures
        .iter()
        .filter_map(|capture| {
            capture_slots
                .get(&capture.binding)
                .map(|slot| (*slot, capture))
        })
        .collect::<Vec<_>>();
    lifted.sort_by_key(|(slot, _)| *slot);
    for (slot, capture) in lifted {
        if slot != inputs_ty.len() {
            return Err("closure transition input order mismatch".into());
        }
        bindings.push(capture.binding);
        inputs_ty.push(nir::Parameter {
            sort: nir::Sort::Runtime,
            ty: capture.ty.clone(),
        });
    }
    owned.retain(|capture| !capture_slots.contains_key(&capture.binding));
    let signature = Term::Function {
        parameters: inputs_ty,
        result: result.clone(),
    };
    checked.ty = if owned.is_empty() {
        signature
    } else {
        Term::Closure {
            environment: owned.iter().map(|capture| capture.ty.clone()).collect(),
            signature: Box::new(signature),
        }
    };
    if stable_captures {
        return infer_stable_closure_in_round(
            package,
            &checked,
            callees,
            prepared_captures,
            inputs,
            code_inputs,
            fuel,
            round,
            (&[], &[]),
            &capture_slots,
        )
        .map(|(summary, _)| summary);
    }
    // Callable leaves of a borrowed aggregate travel with the same virtual
    // input as its pointer leaves, while ordinary captures retain their slots.
    let mut retained_callees = BTreeMap::new();
    let mut retained_prepared = BTreeMap::new();
    for (input, value) in inputs {
        *fuel = fuel
            .checked_sub(input.fields.len() + 1)
            .ok_or("borrowed callable input budget exhausted")?;
        value.charge(fuel)?;
    }
    for (input, values) in code_inputs {
        *fuel = fuel
            .checked_sub(input.fields.len() + 1)
            .ok_or("borrowed code input budget exhausted")?;
        for value in values {
            value.charge(fuel)?;
        }
    }
    let mut lifted_inputs = inputs.clone();
    let mut lifted_code = code_inputs.clone();
    for (place, value) in callees {
        *fuel = fuel
            .checked_sub(place.fields.len() + 1)
            .ok_or("borrowed callable capture budget exhausted")?;
        value.charge(fuel)?;
        if let Some(slot) = capture_slots.get(&place.binding) {
            lifted_inputs.insert(
                CallableInput {
                    parameter: *slot,
                    fields: place.fields.clone(),
                },
                value.clone(),
            );
        } else {
            retained_callees.insert(place.clone(), value.clone());
        }
    }
    charge_prepared_captures(prepared_captures, fuel, 0)?;
    for (place, values) in prepared_captures {
        if let Some(slot) = capture_slots.get(&place.binding) {
            lifted_code.insert(
                CallableInput {
                    parameter: *slot,
                    fields: place.fields.clone(),
                },
                values.iter().map(PreparedCode::borrowed).collect(),
            );
        } else {
            retained_prepared.insert(place.clone(), values.clone());
        }
    }
    infer_closure_in_round(
        package,
        &checked,
        &retained_callees,
        &retained_prepared,
        &lifted_inputs,
        &lifted_code,
        fuel,
        round,
        (&[], &[]),
        // Local callers transport lifted slots exactly; remaining captures
        // retain their reusable invariant. Public resource queries describe
        // one actual invocation instead.
        transition_code_captures,
    )
}

fn validate_borrowed_capture_storage<'a>(
    allow_pointer_rebinding: bool,
    package: Option<&Package>,
    body: &'a Expr,
    bindings: &BTreeSet<LocalId>,
    fuel: &mut usize,
) -> Result<(), String> {
    validate_capture_storage(true, allow_pointer_rebinding, package, body, bindings, fuel)
}

// Alias publication is checked over the enclosing scope; writes can instead
// be checked over the exact lifetime of a handler's candidate snapshot.
fn validate_capture_storage(
    check_writes: bool,
    allow_pointer_rebinding: bool,
    package: Option<&Package>,
    body: &Expr,
    bindings: &BTreeSet<LocalId>,
    fuel: &mut usize,
) -> Result<(), String> {
    validate_capture_storage_except(
        check_writes,
        allow_pointer_rebinding,
        package,
        body,
        bindings,
        fuel,
        &BTreeSet::new(),
    )
}

// Ephemeral expression identities belong to this immutable traversal only.
// A proven arm exit can authorize its writes, never writes in the installer.
fn validate_capture_storage_except(
    check_writes: bool,
    allow_pointer_rebinding: bool,
    package: Option<&Package>,
    body: &Expr,
    bindings: &BTreeSet<LocalId>,
    fuel: &mut usize,
    stable_writes: &BTreeSet<usize>,
) -> Result<(), String> {
    let mut pending = vec![body];
    while let Some(root) = pending.pop() {
        nir::visit::expression_pruned(root, fuel, |expr, fuel| {
            match &expr.kind {
                K::Assign { place, value } if check_writes && bindings.contains(&place.root) => {
                    // A data-only field has no pointer/code provenance to write
                    // back. The ordinary assignment checker below validates its
                    // owner, path and type; repeated invocations retain runtime
                    // data state while the footprint covers either branch.
                    let data_field =
                        !place.fields.is_empty() && data_representation(package, &value.ty, fuel)?;
                    if !data_field
                        && !stable_writes.contains(&(expr as *const Expr as usize))
                        && !(allow_pointer_rebinding
                            && capture_pointer_replacement_layout(package, &value.ty, fuel)?)
                    {
                        return Err(
                            "captured pointer rebinding requires an environment writeback contract"
                                .into(),
                        );
                    }
                }
                K::ReadPointer(pointer) => {
                    // A syntactic address consumed immediately by a load never
                    // publishes an alias to the capture cell. Resource checking
                    // separately proves Copy permission and storage validity.
                    if let K::AddressOf(value) = &pointer.kind {
                        if value.ty == expr.ty
                            && copy_readable_representation(package, &expr.ty, fuel)?
                        {
                            pending.push(value);
                            return Ok(false);
                        }
                    }
                }
                K::AddressOf(value) => {
                    let mut base = value.as_ref();
                    while let K::Field { value, .. } | K::AnonymousField { value, .. } = &base.kind
                    {
                        *fuel = fuel
                            .checked_sub(1)
                            .ok_or("capture storage traversal budget exhausted")?;
                        base = value;
                    }
                    if matches!(base.kind,K::Local(id) if bindings.contains(&id)) {
                        return Err("addressing captured pointer storage requires an environment memory contract".into());
                    }
                }
                K::Closure { captures, .. } => {
                    // A nested body uses private capture cells; initializers
                    // still execute against the current environment.
                    *fuel = fuel
                        .checked_sub(captures.len())
                        .ok_or("capture storage traversal budget exhausted")?;
                    pending.extend(
                        captures
                            .iter()
                            .filter_map(|capture| capture.value.as_deref()),
                    );
                    return Ok(false);
                }
                K::Call {
                    callee, arguments, ..
                } => {
                    *fuel = fuel
                        .checked_sub(arguments.len().saturating_add(1))
                        .ok_or("capture storage traversal budget exhausted")?;
                    pending.push(callee);
                    pending.extend(
                        arguments
                            .iter()
                            .filter(|argument| !argument.is_default)
                            .map(|argument| &argument.value),
                    );
                    return Ok(false);
                }
                _ => {}
            }
            Ok(true)
        })?;
    }
    Ok(())
}

// Anonymous bodies participate in the caller's worklist so a named recursive
// edge does not restart summary inference on the Rust call stack.
fn infer_closure_in_round(
    package: &Package,
    definition: &Expr,
    callees: &BTreeMap<CallablePlace, CallableResult>,
    prepared_captures: &PreparedCaptures,
    inputs: &BTreeMap<CallableInput, CallableResult>,
    code_inputs: &CodeInputs<'_>,
    fuel: &mut usize,
    round: &mut Round,
    specialization: (&[DeclarationId], &[Term]),
    stable_captures: bool,
) -> Result<ArgumentSummary, String> {
    if !stable_captures {
        return infer_closure_pass(
            package,
            definition,
            callees,
            prepared_captures,
            inputs,
            code_inputs,
            fuel,
            round,
            specialization,
            false,
            &BTreeMap::new(),
        )
        .map(|(summary, _)| summary);
    }
    infer_stable_closure_in_round(
        package,
        definition,
        callees,
        prepared_captures,
        inputs,
        code_inputs,
        fuel,
        round,
        specialization,
        &BTreeMap::new(),
    )
    .map(|(summary, _)| summary)
}
/// A conservative envelope for every invocation of a returned mutable closure.
/// It contains identities, not current runtime values or pointer origins.
pub(crate) fn returned_capture_invariant(
    package: &Package,
    definition: &Expr,
    callees: &BTreeMap<CallablePlace, CallableResult>,
    prepared_captures: &PreparedCaptures,
    fuel: &mut usize,
) -> Result<PreparedCaptures, String> {
    let mut round = Round {
        complete_calls: true,
        ..Default::default()
    };
    infer_stable_closure_in_round(
        package,
        definition,
        callees,
        prepared_captures,
        &BTreeMap::new(),
        &BTreeMap::new(),
        fuel,
        &mut round,
        (&[], &[]),
        &BTreeMap::new(),
    )
    .map(|(_, captures)| captures)
}
fn infer_stable_closure_in_round(
    package: &Package,
    definition: &Expr,
    callees: &BTreeMap<CallablePlace, CallableResult>,
    prepared_captures: &PreparedCaptures,
    inputs: &BTreeMap<CallableInput, CallableResult>,
    code_inputs: &CodeInputs<'_>,
    fuel: &mut usize,
    round: &mut Round,
    specialization: (&[DeclarationId], &[Term]),
    lifted_captures: &BTreeMap<LocalId, usize>,
) -> Result<(ArgumentSummary, PreparedCaptures), String> {
    charge_prepared_captures(prepared_captures, fuel, 0)?;
    let mut invariant = prepared_captures.clone();
    for (place, key) in callees {
        *fuel = fuel
            .checked_sub(place.fields.len() + 1)
            .ok_or("closure capture invariant budget exhausted")?;
        if invariant.contains_key(place) {
            return Err("conflicting named and anonymous capture premises".into());
        }
        key.charge(fuel)?;
        if !matches!(
            key,
            CallableResult::Known(_) | CallableResult::Candidates(_)
        ) || key.keys().is_empty()
        {
            return Err("closure capture invariant requires complete code identities".into());
        }
        invariant.insert(
            place.clone(),
            key.keys()
                .into_iter()
                .cloned()
                .map(PreparedCode::Named)
                .collect(),
        );
    }
    loop {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("closure capture invariant budget exhausted")?;
        // Lifted pointer-bearing aggregates retain capture identity for code
        // mutation even though their storage is represented by formal inputs.
        let mut retained = BTreeMap::new();
        for (input, values) in code_inputs {
            *fuel = fuel
                .checked_sub(input.fields.len() + 1)
                .ok_or("lifted capture code budget exhausted")?;
            for value in values {
                value.charge(fuel)?;
            }
        }
        let mut pass_code = code_inputs.clone();
        charge_prepared_captures(&invariant, fuel, 0)?;
        for (place, values) in &invariant {
            if let Some(parameter) = lifted_captures.get(&place.binding) {
                pass_code.insert(
                    CallableInput {
                        parameter: *parameter,
                        fields: place.fields.clone(),
                    },
                    values.iter().map(PreparedCode::borrowed).collect(),
                );
            } else {
                retained.insert(place.clone(), values.clone());
            }
        }
        let (summary, writes) = infer_closure_pass(
            package,
            definition,
            &BTreeMap::new(),
            &retained,
            inputs,
            &pass_code,
            fuel,
            round,
            specialization,
            true,
            lifted_captures,
        )?;
        let mut changed = false;
        for (place, values) in writes {
            *fuel = fuel
                .checked_sub(place.fields.len() + 1)
                .ok_or("closure capture invariant budget exhausted")?;
            // The successful initial pass has checked every read using the
            // original premises. Subsequent passes widen with every established
            // fixed code identity, including environment-free anonymous code.
            changed |= join_prepared_codes(invariant.entry(place).or_default(), values, fuel)?;
        }
        if !changed {
            return Ok((summary, invariant));
        }
    }
}

fn infer_closure_pass(
    package: &Package,
    definition: &Expr,
    callees: &BTreeMap<CallablePlace, CallableResult>,
    prepared_captures: &PreparedCaptures,
    inputs: &BTreeMap<CallableInput, CallableResult>,
    code_inputs: &CodeInputs<'_>,
    fuel: &mut usize,
    round: &mut Round,
    specialization: (&[DeclarationId], &[Term]),
    stable_captures: bool,
    lifted_captures: &BTreeMap<LocalId, usize>,
) -> Result<(ArgumentSummary, PreparedCaptures), String> {
    let K::Closure {
        parameters: bindings,
        captures,
        body,
    } = &definition.kind
    else {
        return Err("closure memory summary requires a closure definition".into());
    };
    bound_type(&definition.ty, fuel)?;
    for argument in specialization.1 {
        bound_type(argument, fuel)?;
    }
    let declarations = round.binders(Some(package), specialization.0, fuel)?;
    let closure_ty = definition
        .ty
        .instantiate_parameters(&declarations, specialization.1)
        .map_err(|error| error.to_string())?;
    let environment = match &closure_ty {
        Term::Closure { environment, .. } => environment.as_slice(),
        _ => &[],
    };
    if environment.len() != captures.len() {
        return Err("closure memory capture layout mismatch".into());
    }
    for (capture, ty) in captures.iter().zip(environment) {
        bound_type(&capture.ty, fuel)?;
        if capture
            .ty
            .instantiate_parameters(&declarations, specialization.1)
            .map_err(|error| error.to_string())?
            != *ty
        {
            return Err("closure memory capture layout mismatch".into());
        }
    }
    let Term::Function {
        parameters, result, ..
    } = closure_ty.callable_signature()
    else {
        return Err("closure memory summary requires a function signature".into());
    };
    if bindings.len() != parameters.len()
        || bindings.iter().collect::<BTreeSet<_>>().len() != bindings.len()
    {
        return Err("closure memory parameter signature mismatch".into());
    }
    charge_prepared_captures(prepared_captures, fuel, 0)?;
    let mut capture_code_inputs = BTreeMap::new();
    let mut capture_parameters = vec![];
    let capture_bindings = captures
        .iter()
        .map(|capture| capture.binding)
        .collect::<Vec<_>>();
    for capture in captures {
        capture_parameters.push(nir::Parameter {
            sort: nir::Sort::Runtime,
            ty: capture
                .ty
                .instantiate_parameters(&declarations, specialization.1)
                .map_err(|e| e.to_string())?,
        });
    }
    for (place, codes) in prepared_captures {
        if callees.contains_key(place) {
            return Err("conflicting named and anonymous capture premises".into());
        }
        let parameter = capture_bindings
            .iter()
            .position(|binding| *binding == place.binding)
            .ok_or("prepared code names an unknown capture")?;
        if !codes.is_empty() {
            capture_code_inputs.insert(
                CallableInput {
                    parameter,
                    fields: place.fields.clone(),
                },
                codes.iter().map(PreparedCode::borrowed).collect(),
            );
        }
    }
    let mut infer = Infer {
        scope: body,
        borrowed_callable_slots: BTreeMap::new(),
        local_code: vec![],
        borrowed_code_pointers: BTreeMap::new(),
        stable_capture_roots: BTreeSet::new(),
        capture_code_writes: BTreeMap::new(),
        named_code_slots: HashMap::new(),
        closure_nodes: HashMap::new(),
        local_code_results: HashMap::new(),
        local_code_fields: HashMap::new(),
        type_context: crate::ptc::Context::default(),
        type_views: HashMap::new(),
        fuel,
        package: Some(package),
        parameters: specialization.0,
        declarations,
        arguments: specialization.1,
        round,
        effect_row: vec![],
        environment: Environment::default(),
        locals: BTreeMap::new(),
        private_storage_roots: BTreeSet::new(),
        local_storage: vec![],
        loop_states: vec![],
        accesses: vec![],
        may_reject: false,
        may_run_cleanup: false,
        expression_results: HashMap::new(),
        callable_results: HashMap::new(),
        aggregate_callees: HashMap::new(),
        aggregate_pointers: HashMap::new(),
        returns: Some(BTreeSet::new()),
        pointer_field_returns: None,
        input_pointer_slots: BTreeMap::new(),
        input_code_slots: BTreeMap::new(),
        input_code_exits: BTreeMap::new(),
        input_pointer_exits: BTreeMap::new(),
        callable_returns: CallableResult::NoReturn,
        callable_field_returns: BTreeMap::new(),
        code_field_returns: BTreeMap::new(),
    };
    for (binding, index) in lifted_captures {
        infer.step(0)?;
        if bindings.get(*index) != Some(binding) {
            return Err("lifted capture code slot mismatch".into());
        }
        infer.stable_capture_roots.insert(*binding);
    }
    let completion = infer.effect_completion(result)?;
    if let Term::Computation { effects, .. } = result.as_ref() {
        infer.effect_row = effects.clone();
    }
    if !intrinsic_pointer_value(&completion)
        && completion != Term::Primitive(Primitive::Void)
        && !infer.local_pointer_value(&completion)?
    {
        symbolic_pointer_fields(Some(package), &completion, infer.fuel)?;
    }
    for capture in captures {
        infer.step(0)?;
        if stable_captures {
            infer.stable_capture_roots.insert(capture.binding);
        }
        let capture_ty = infer.ty(&capture.ty)?;
        require_pointer_free_owned(Some(package), &capture_ty, infer.fuel, 0).map_err(
            |reason| format!("captured closure requires an environment memory contract: {reason}"),
        )?;
        if bindings.contains(&capture.binding)
            || infer.locals.insert(capture.binding, capture_ty).is_some()
        {
            return Err("closure memory capture binding collision".into());
        }
    }
    for (place, callable) in callees {
        infer.step(0)?;
        let capture = captures
            .iter()
            .find(|capture| capture.binding == place.binding)
            .ok_or("closure memory callable premise names an unknown capture")?;
        let capture_ty = infer.ty(&capture.ty)?;
        let expected = owned_path_type(Some(package), &capture_ty, &place.fields, infer.fuel)?;
        if !matches!(expected, Term::Function { .. } | Term::UnsafeFunction(_))
            || !matches!(
                callable,
                CallableResult::Known(_) | CallableResult::Candidates(_)
            )
            || callable.keys().is_empty()
        {
            return Err("closure memory callable premise requires named code identities".into());
        }
        callable.charge(infer.fuel)?;
        let mut seen = vec![];
        for key in callable.keys() {
            *infer.fuel = infer
                .fuel
                .checked_sub(seen.len().saturating_add(1))
                .ok_or("closure memory callable validation budget exhausted")?;
            if seen.contains(&key) {
                return Err("duplicate closure memory callable candidate".into());
            }
            seen.push(key);
            let (function, body_arguments) = package
                .runtime_body(key.declaration, &key.arguments)
                .ok_or("closure memory callable definition is missing")?;
            if function.phase != FunctionPhase::Runtime
                || function.type_parameters.len() != body_arguments.len()
            {
                return Err("closure memory callable requires a concrete runtime instance".into());
            }
            bound_type(&function.signature, infer.fuel)?;
            let signature = function
                .signature
                .instantiate_parameters(
                    &infer
                        .round
                        .binders(Some(package), &function.type_parameters, infer.fuel)?,
                    body_arguments,
                )
                .map_err(|error| error.to_string())?;
            let signature = signature.with_unsafe_call_requirement(function.is_unsafe);
            if signature != expected {
                return Err("closure memory callable signature disagrees with capture".into());
            }
        }
        infer
            .environment
            .callees
            .insert(place.clone(), callable.clone());
    }
    for (place, codes) in prepared_captures {
        if codes.is_empty() {
            let capture = captures
                .iter()
                .find(|capture| capture.binding == place.binding)
                .ok_or("absent code names an unknown capture")?;
            let ty = infer.ty(&capture.ty)?;
            let leaf = owned_path_type(Some(package), &ty, &place.fields, infer.fuel)?;
            if !place.fields.contains(&VARIANT_FIELD)
                || !matches!(
                    leaf,
                    Term::Function { .. } | Term::UnsafeFunction(_) | Term::Closure { .. }
                )
            {
                return Err(
                    "empty prepared capture requires a variant-guarded callable leaf".into(),
                );
            }
            infer
                .environment
                .callees
                .insert(place.clone(), CallableResult::NoReturn);
            infer
                .environment
                .local_candidates
                .insert(place.clone(), BTreeSet::new());
        }
    }
    infer.install_code_inputs(
        &capture_bindings,
        &capture_parameters,
        &BTreeMap::new(),
        &capture_code_inputs,
    )?;
    for (index, (binding, parameter)) in bindings.iter().zip(parameters).enumerate() {
        infer.step(0)?;
        for fields in
            owned_callable_bottom(Some(package), &parameter.ty, infer.fuel, true)?.into_keys()
        {
            infer.step(fields.len())?;
            infer.input_code_slots.insert(
                CallablePlace {
                    binding: *binding,
                    fields: fields.clone(),
                },
                CallableInput {
                    parameter: index,
                    fields,
                },
            );
        }
        for fields in symbolic_pointer_fields(Some(package), &parameter.ty, infer.fuel)?.into_keys()
        {
            infer.step(fields.len())?;
            infer.input_pointer_slots.insert(
                CallablePlace {
                    binding: *binding,
                    fields: fields.clone(),
                },
                CallableInput {
                    parameter: index,
                    fields,
                },
            );
        }
        infer.note_owned_cleanup(&parameter.ty)?;
        if !intrinsic_pointer_value(&parameter.ty) && !infer.local_pointer_value(&parameter.ty)? {
            infer.install_input_pointer_fields(*binding, index, &parameter.ty)?;
        }
        infer.locals.insert(*binding, parameter.ty.clone());
        if infer.local_pointer_value(&parameter.ty)? {
            infer.environment.pointers.insert(
                *binding,
                Some(BTreeSet::from([PointerPlace {
                    root: PointerRoot::Parameter(index),
                    input_fields: vec![],
                    through: vec![],
                    conditions: BTreeMap::new(),
                    fields: vec![],
                }])),
            );
        }
    }
    for (input, value) in inputs {
        infer.step(input.fields.len() + 1)?;
        if !matches!(
            value,
            CallableResult::Known(_) | CallableResult::Candidates(_)
        ) {
            return Err("closure parameter premise requires named code identities".into());
        }
        let parameter = parameters
            .get(input.parameter)
            .ok_or("closure callable parameter is missing")?;
        let expected = owned_path_type(Some(package), &parameter.ty, &input.fields, infer.fuel)?;
        validate_code_premise(package, infer.round, &expected, value, infer.fuel)?;
        infer.environment.callees.insert(
            CallablePlace {
                binding: bindings[input.parameter],
                fields: input.fields.clone(),
            },
            value.clone(),
        );
    }
    infer.install_code_inputs(bindings, parameters, inputs, code_inputs)?;
    infer.record_parameter_cleanup_reads(bindings, parameters, body.source)?;
    infer.code_field_returns = code_result_bottom(Some(package), result, infer.fuel)?;
    infer.prepare_private_storage_roots(body)?;
    infer.walk(body, 0)?;
    let completes = !super::pattern_facts::cannot_complete(body, infer.fuel, 0)?;
    if completes {
        infer.merge_input_pointer_exit()?;
        infer.merge_input_code_exit()?;
    }
    if completes && infer.ty(&body.ty)? == **result {
        infer.merge_code_field_return(body)?;
        infer.merge_pointer_field_return(body)?;
    }
    let pointer_result = infer.local_pointer_value(&completion)?;
    let body_ty = infer.ty(&body.ty)?;
    let body_completion = infer.effect_completion(&body_ty)?;
    if completes && pointer_result && infer.local_pointer_value(&body_completion)? {
        let targets = infer.optional_target(body, 0)?;
        infer.merge_return(targets)?;
    }
    Ok((
        ArgumentSummary {
            input_pointer_exits: infer.input_pointer_exits,
            input_code_exits: infer.input_code_exits,
            pointer_fields: infer.pointer_field_returns.unwrap_or_default(),
            accesses: infer.accesses,
            code_fields: infer.code_field_returns,
            may_reject: infer.may_reject,
            may_run_cleanup: infer.may_run_cleanup,
            result_targets: if pointer_result { infer.returns } else { None },
        },
        infer.capture_code_writes,
    ))
}

#[cfg(test)]
mod binder_cache_tests {
    use super::*;

    #[test]
    fn round_reuses_complete_sort_evidence_and_charges_each_lookup() {
        let id = |local| DeclarationId { package: 1, local };
        let mut package = Package::default();
        package
            .function_parameters
            .push(nir::function_parameters::Declaration {
                declaration: id(0),
                parameters: vec![id(1), id(2)],
                domains: vec![
                    Term::Primitive(Primitive::Type),
                    Term::Primitive(Primitive::Bool),
                ],
            });
        let parameters = [id(1), id(2)];
        let mut round = Round::default();
        assert!(round.binders(Some(&package), &parameters, &mut 2).is_err());
        assert!(round.parameter_sorts.is_none());
        let expected = memory_binders(Some(&package), &parameters, &mut 1000).unwrap();
        assert_eq!(expected[0].sort, nir::Sort::Type);
        assert_eq!(expected[1].sort, nir::Sort::ComptimeValue);
        let mut fuel = 1000;
        assert_eq!(
            round
                .binders(Some(&package), &parameters, &mut fuel)
                .unwrap(),
            expected
        );
        assert!(1000 - fuel > parameters.len());
        for _ in 0..10 {
            let mut fuel = parameters.len();
            assert_eq!(
                round
                    .binders(Some(&package), &parameters, &mut fuel)
                    .unwrap(),
                expected
            );
            assert_eq!(fuel, 0);
        }
        assert!(round.binders(Some(&package), &parameters, &mut 1).is_err());
        assert_eq!(
            round.binders(Some(&package), &parameters, &mut 2).unwrap(),
            expected
        );

        // A new query must see changed declaration evidence and reject invalid
        // metadata instead of borrowing a successful cache from another query.
        package.function_parameters[0].domains[1] = Term::Primitive(Primitive::Type);
        let fresh = Round::default()
            .binders(Some(&package), &parameters, &mut 1000)
            .unwrap();
        assert_eq!(fresh[1].sort, nir::Sort::Type);
        package
            .function_parameters
            .push(package.function_parameters[0].clone());
        let mut invalid = Round::default();
        assert!(
            invalid
                .binders(Some(&package), &parameters, &mut 1000)
                .is_err()
        );
        assert!(invalid.parameter_sorts.is_none());
    }
}

#[cfg(test)]
mod record_layout_tests {
    use super::*;
    #[test]
    fn data_replacement_layout_rejects_nested_code_and_pointer_leaves() {
        let scalar = Term::Primitive(nir::Primitive::Bool);
        let code = Term::Function {
            parameters: vec![],
            result: Box::new(scalar.clone()),
        };
        let nested = |ty| Term::Record(vec![("field".into(), Term::Tuple(vec![ty]))]);
        for (leaf, accepted) in [
            (scalar.clone(), true),
            (Term::Pointer(Box::new(scalar)), false),
            (code, false),
            (
                Term::NormalForm {
                    head: nir::DeclarationId {
                        package: 0,
                        local: 1,
                    },
                    sort: nir::Sort::Type,
                    arguments: vec![],
                },
                false,
            ),
        ] {
            let ty = nested(leaf);
            assert!(data_representation(None, &ty, &mut 1).is_err());
            assert_eq!(data_representation(None, &ty, &mut 1000).unwrap(), accepted);
        }
        let code_record = nested(Term::Function {
            parameters: vec![],
            result: Box::new(Term::Primitive(nir::Primitive::Bool)),
        });
        assert!(readable_representation(None, &code_record, &mut 1000).unwrap());
        assert!(!data_representation(None, &code_record, &mut 1000).unwrap());
    }
    #[test]
    fn error_result_pointer_footprints_check_success_and_every_complete_label() {
        let scalar = Term::Primitive(nir::Primitive::Bool);
        let pointer = Term::Pointer(Box::new(scalar.clone()));
        let closed = Term::error_result(vec![scalar.clone()], scalar.clone());
        require_pointer_free_owned(None, &closed, &mut 1000, 0).unwrap();
        assert!(require_pointer_free_owned(None, &closed, &mut 1, 0).is_err());
        for ty in [
            Term::error_result(vec![scalar.clone()], pointer.clone()),
            Term::error_result(vec![pointer.clone()], scalar.clone()),
            Term::error_result(
                vec![Term::error_result(vec![pointer], scalar.clone())],
                scalar.clone(),
            ),
            Term::error_result(
                vec![Term::NormalForm {
                    head: nir::DeclarationId {
                        package: 0,
                        local: 1,
                    },
                    sort: nir::Sort::Type,
                    arguments: vec![scalar.clone()],
                }],
                scalar,
            ),
        ] {
            assert!(
                require_pointer_free_owned(None, &ty, &mut 1000, 0).is_err(),
                "{ty}"
            );
        }
        require_pointer_free_owned(None, &closed, &mut 1000, 0).unwrap();
    }

    #[test]
    fn record_pointer_footprints_recurse_without_inventing_memory_evidence() {
        let scalar = Term::Primitive(nir::Primitive::Bool);
        let record = |ty| Term::Record(vec![("value".into(), Term::Tuple(vec![ty]))]);
        require_pointer_free_owned(None, &record(scalar.clone()), &mut 1000, 0).unwrap();
        let pointer = record(Term::Pointer(Box::new(scalar.clone())));
        assert!(
            require_pointer_free_owned(None, &pointer, &mut 1000, 0)
                .unwrap_err()
                .contains("memory contract")
        );
        let opaque = record(Term::NormalForm {
            head: nir::DeclarationId {
                package: 0,
                local: 1,
            },
            sort: nir::Sort::Type,
            arguments: vec![scalar.clone()],
        });
        assert!(require_pointer_free_owned(None, &opaque, &mut 1000, 0).is_err());
        require_pointer_free_owned(None, &record(scalar), &mut 1000, 0).unwrap();
    }
    #[test]
    fn code_signature_pointers_are_not_stored_data_aliases() {
        let pointer = Term::Pointer(Box::new(Term::Primitive(nir::Primitive::Bool)));
        let code = Term::Function {
            parameters: vec![],
            result: Box::new(pointer.clone()),
        };
        let record = Term::Record(vec![("code".into(), code.clone())]);
        require_pointer_free_owned(None, &record, &mut 1000, 0).unwrap();
        let data = Term::Record(vec![("data".into(), pointer.clone())]);
        assert!(require_pointer_free_owned(None, &data, &mut 1000, 0).is_err());
        let owned_environment = Term::Closure {
            environment: vec![Term::Primitive(nir::Primitive::Bool)],
            signature: Box::new(code.clone()),
        };
        require_pointer_free_owned(None, &owned_environment, &mut 1000, 0).unwrap();
        assert!(require_pointer_free_owned(None, &owned_environment, &mut 1, 0).is_err());
        require_pointer_free_owned(None, &owned_environment, &mut 1000, 0).unwrap();
        let closure = Term::Closure {
            environment: vec![pointer],
            signature: Box::new(code),
        };
        assert!(require_pointer_free_owned(None, &closure, &mut 1000, 0).is_err());
        assert!(require_pointer_free_owned(None, &record, &mut 0, 0).is_err());
        require_pointer_free_owned(None, &record, &mut 1000, 0).unwrap();
    }
    #[test]
    fn record_memory_type_budgets_cover_nested_fields_and_recover() {
        let mut deep = Term::Primitive(nir::Primitive::Bool);
        for _ in 0..70 {
            deep = Term::Record(vec![("next".into(), deep)]);
        }
        assert!(
            bound_type(&deep, &mut 10000)
                .unwrap_err()
                .contains("budget")
        );
        let shallow = Term::Record(vec![(
            "value".into(),
            Term::Primitive(nir::Primitive::Bool),
        )]);
        assert!(bound_type(&shallow, &mut 1).is_err());
        bound_type(&shallow, &mut 100).unwrap();
    }
}

#[cfg(test)]
mod borrowed_capture_scope_tests {
    use super::*;
    use nir::program::{CallArgument, Capture, Place, SourceRange};
    fn id() -> LocalId {
        LocalId { owner: 7, index: 1 }
    }
    fn node(kind: K) -> Expr {
        Expr {
            source: SourceRange { start: 1, end: 2 },
            ty: Term::Primitive(Primitive::Void),
            kind,
        }
    }
    fn address() -> Expr {
        node(K::AddressOf(Box::new(node(K::Local(id())))))
    }
    fn nested(body: Expr, value: Option<Expr>) -> Expr {
        node(K::Closure {
            parameters: vec![],
            captures: vec![Capture {
                binding: id(),
                ty: Term::Pointer(Box::new(Term::Primitive(Primitive::Bool))),
                value: value.map(Box::new),
            }],
            body: std::sync::Arc::new(body),
        })
    }
    #[test]
    fn private_capture_storage_does_not_alias_outer_binding_identity() {
        let bindings = BTreeSet::from([id()]);
        assert!(
            validate_borrowed_capture_storage(false, None, &address(), &bindings, &mut 100)
                .is_err()
        );
        let write = node(K::Assign {
            place: Place {
                root: id(),
                fields: vec![],
            },
            value: Box::new(node(K::Local(id()))),
        });
        assert!(
            validate_borrowed_capture_storage(false, None, &write, &bindings, &mut 100).is_err()
        );
        for private in [address(), write] {
            validate_borrowed_capture_storage(
                false,
                None,
                &nested(private, None),
                &bindings,
                &mut 100,
            )
            .unwrap();
        }
        assert!(
            validate_borrowed_capture_storage(
                false,
                None,
                &nested(node(K::Unit), Some(address())),
                &bindings,
                &mut 100
            )
            .is_err()
        );
    }
    #[test]
    fn call_defaults_use_callee_storage_but_explicit_arguments_use_caller_storage() {
        for is_default in [false, true] {
            let call = node(K::Call {
                callee: Box::new(node(K::Unit)),
                arguments: vec![CallArgument {
                    parameter: 0,
                    value: address(),
                    is_default,
                }],
                parameter_bindings: vec![],
            });
            assert_eq!(
                validate_borrowed_capture_storage(
                    false,
                    None,
                    &call,
                    &BTreeSet::from([id()]),
                    &mut 100
                )
                .is_ok(),
                is_default
            );
        }
    }
    #[test]
    fn scope_scan_is_bounded_and_budget_failure_does_not_poison_later_checks() {
        let body = nested(node(K::Unit), Some(node(K::Unit)));
        assert!(
            validate_borrowed_capture_storage(false, None, &body, &BTreeSet::from([id()]), &mut 1)
                .unwrap_err()
                .contains("budget")
        );
        validate_borrowed_capture_storage(false, None, &body, &BTreeSet::from([id()]), &mut 100)
            .unwrap();
    }
}

#[cfg(test)]
mod pointer_root_tests {
    use super::*;
    #[test]
    fn closure_storage_projection_uses_environment_and_preserves_bounds() {
        let boolean = Term::Primitive(nir::Primitive::Bool);
        let closure = Term::Closure {
            environment: vec![Term::Pointer(Box::new(boolean.clone()))],
            signature: Box::new(Term::Function {
                parameters: vec![],
                result: Box::new(Term::Primitive(nir::Primitive::Void)),
            }),
        };
        let mut traversal = StorageTraversal::new(None, &closure, &[0], &mut 1000).unwrap();
        assert_eq!(
            traversal.target.fields,
            vec![(nir::program::FieldOwner::Anonymous, 0)]
        );
        traversal.load(None, &[], &mut 1000).unwrap();
        assert_eq!(traversal.target.ty, boolean);
        assert_eq!(traversal.loads.len(), 1);
        assert!(StorageTraversal::new(None, &closure, &[1], &mut 1000).is_err());
        assert!(StorageTraversal::new(None, &closure, &[0], &mut 0).is_err());
    }
    #[test]
    fn input_field_origins_remain_distinct_from_pointee_projections() {
        let direct = PointerPlace {
            root: PointerRoot::Parameter(0),
            input_fields: vec![],
            through: vec![],
            conditions: BTreeMap::new(),
            fields: vec![(nir::program::FieldOwner::Anonymous, 1)],
        };
        let nested = PointerPlace {
            root: PointerRoot::Parameter(0),
            input_fields: vec![1],
            through: vec![],
            conditions: BTreeMap::new(),
            fields: vec![],
        };
        assert_eq!(BTreeSet::from([direct.clone(), nested.clone()]).len(), 2);
        let exported = nested.clone().external().unwrap();
        assert_ne!(direct.clone().external().unwrap(), exported);
        assert_eq!(
            exported.to_string(),
            "(parameter-place (parameter 0) (input-fields 1) (fields))"
        );
        let mut actual = direct.clone();
        // A caller cannot mistake an aggregate field origin for the direct
        // pointer argument and apply only its final pointee suffix.
        assert!(actual.append_path(&exported).is_err());
        assert_eq!(actual, direct);
        let mut actual = nested;
        actual
            .append_path(&ParameterPlace {
                parameter: 3,
                input_fields: vec![],
                through: vec![vec![]],
                conditions: BTreeMap::new(),
                fields: vec![(nir::program::FieldOwner::Anonymous, 2)],
            })
            .unwrap();
        let exported = actual.external().unwrap();
        assert_eq!(exported.parameter, 0);
        assert_eq!(exported.input_fields, vec![1]);
        assert_eq!(exported.through, vec![vec![]]);
        assert_eq!(
            exported.fields,
            vec![(nir::program::FieldOwner::Anonymous, 2)]
        );
    }

    #[test]
    fn conditional_paths_rebase_only_the_first_loaded_root() {
        let field = |index| (nir::program::FieldOwner::Anonymous, index);
        let base = PointerPlace {
            root: PointerRoot::Parameter(0),
            input_fields: vec![],
            through: vec![vec![field(2)]],
            fields: vec![field(3)],
            conditions: BTreeMap::from([(0, vec![(vec![], 1)])]),
        };
        let suffix = ParameterPlace {
            parameter: 0,
            input_fields: vec![],
            through: vec![vec![field(0)]],
            fields: vec![field(1)],
            conditions: BTreeMap::from([(0, vec![(vec![], 2)]), (1, vec![(vec![], 4)])]),
        };
        let mut actual = base.clone();
        actual.append_path(&suffix).unwrap();
        assert_eq!(
            actual.conditions,
            BTreeMap::from([
                (0, vec![(vec![], 1)]),
                (1, vec![(vec![3], 2)]),
                (2, vec![(vec![], 4)])
            ])
        );
        assert_eq!(
            actual.through,
            vec![vec![field(2)], vec![field(3), field(0)]]
        );
        assert_eq!(actual.fields, vec![field(1)]);
        let mut invalid = suffix;
        invalid.conditions.insert(2, vec![(vec![], 9)]);
        let mut unchanged = base.clone();
        assert!(unchanged.append_path(&invalid).is_err());
        assert_eq!(unchanged, base);
        let exported = actual.external().unwrap();
        assert!(
            exported
                .to_string()
                .contains("(conditions (stage 1) (guard (fields 3) (variant 2)))")
        );
        let mut different = exported.clone();
        different.conditions.get_mut(&1).unwrap()[0].1 = 7;
        assert_eq!(BTreeSet::from([exported, different]).len(), 2);
    }

    #[test]
    fn error_storage_projection_preserves_payload_guards() {
        let success = Term::Pointer(Box::new(Term::Primitive(Primitive::Bool)));
        let failure = Term::Pointer(Box::new(Term::Primitive(Primitive::Char)));
        let packet = Term::error_result(
            vec![Term::Record(vec![("pointer".into(), failure.clone())])],
            success.clone(),
        );
        for (path, expected, tag) in [
            (vec![VARIANT_FIELD, 0, 0], success, 0),
            (vec![VARIANT_FIELD, 1, 0, 0], failure, 1),
        ] {
            let projection = project_owned_storage(None, &packet, &path, &mut 1000).unwrap();
            assert_eq!(projection.ty, expected);
            assert_eq!(projection.guards, vec![(vec![], tag)]);
            assert!(
                projection
                    .fields
                    .iter()
                    .all(|(owner, _)| *owner == nir::program::FieldOwner::Anonymous)
            );
            let root = Term::Record(vec![("packet".into(), packet.clone())]);
            let rebased = project_owned_storage(None, &root, &[0], &mut 1000)
                .unwrap()
                .then(None, &path, &mut 1000)
                .unwrap();
            assert_eq!(rebased.guards, vec![(vec![0], tag)]);
        }
        for path in [
            vec![VARIANT_FIELD],
            vec![VARIANT_FIELD, 2, 0],
            vec![VARIANT_FIELD, 0, 1],
            vec![0],
        ] {
            assert!(project_owned_storage(None, &packet, &path, &mut 1000).is_err());
        }
        assert!(project_owned_storage(None, &packet, &[VARIANT_FIELD, 0, 0], &mut 0).is_err());
    }

    #[test]
    fn local_roots_are_distinct_from_parameter_slots_and_cannot_publish() {
        let local = PointerPlace {
            root: PointerRoot::Local {
                binding: LocalId { owner: 7, index: 1 },
                storage: 0,
            },
            input_fields: vec![],
            through: vec![],
            conditions: BTreeMap::new(),
            fields: vec![],
        };
        let parameter = PointerPlace {
            root: PointerRoot::Parameter(1),
            input_fields: vec![],
            through: vec![],
            conditions: BTreeMap::new(),
            fields: vec![],
        };
        assert_eq!(BTreeSet::from([local.clone(), parameter.clone()]).len(), 2);
        assert!(
            local
                .clone()
                .external()
                .unwrap_err()
                .contains("cannot escape")
        );
        assert_eq!(parameter.external().unwrap().parameter, 1);
        let mut field = local;
        field
            .append_path(&ParameterPlace {
                parameter: 0,
                input_fields: vec![],
                through: vec![],
                conditions: BTreeMap::new(),
                fields: vec![(nir::program::FieldOwner::Anonymous, 2)],
            })
            .unwrap();
        assert!(matches!(field.root, PointerRoot::Local { .. }));
        let dump = field.to_string();
        assert!(dump.starts_with("(pointer-place (root (local "), "{dump}");
        assert!(dump.contains("(fields (field "), "{dump}");
        assert_eq!(
            dump.chars().filter(|c| *c == '(').count(),
            dump.chars().filter(|c| *c == ')').count()
        );
    }
}

#[cfg(test)]
mod optional_layout_tests {
    use super::*;

    #[test]
    fn nested_optional_pointer_inputs_require_both_presence_guards() {
        let boolean = Term::Primitive(Primitive::Bool);
        let ty = Term::Optional(Box::new(Term::Optional(Box::new(Term::Pointer(Box::new(
            boolean.clone(),
        ))))));
        let leaves = pointer_input_layout(None, &ty, &mut 10000).unwrap();
        assert_eq!(leaves.len(), 1);
        assert_eq!(
            leaves[0].input_fields,
            vec![VARIANT_FIELD, 1, 0, VARIANT_FIELD, 1, 0]
        );
        assert_eq!(leaves[0].storage_fields, vec![0, 0]);
        assert_eq!(leaves[0].guards, vec![(vec![], 1), (vec![0], 1)]);
        assert_eq!(leaves[0].pointee, boolean);
        let projection =
            project_owned_storage(None, &ty, &leaves[0].input_fields, &mut 10000).unwrap();
        assert_eq!(projection.guards, leaves[0].guards);
        assert_eq!(projection.ty, Term::Pointer(Box::new(boolean)));
        assert!(project_owned_storage(None, &ty, &[VARIANT_FIELD, 0, 0], &mut 10000).is_err());
        assert!(require_pointer_free_owned(None, &ty, &mut 10000, 0).is_err());
        assert!(pointer_input_layout(None, &ty, &mut 0).is_err());
    }

    #[test]
    fn optional_owned_paths_keep_each_layer_and_absence_has_no_payload() {
        let boolean = Term::Primitive(nir::Primitive::Bool);
        let inner = Term::Optional(Box::new(boolean.clone()));
        let outer = Term::Optional(Box::new(inner.clone()));
        let absent = Pattern::OptionalAbsent;
        let present = Pattern::OptionalPresent(Box::new(Pattern::Wild));
        let (tag, patterns, fields) =
            owned_variant_pattern(None, &absent, &outer, &mut 1000).unwrap();
        assert_eq!(tag, 0);
        assert!(patterns.is_empty() && fields.is_empty());
        let (tag, patterns, fields) =
            owned_variant_pattern(None, &present, &outer, &mut 1000).unwrap();
        assert_eq!(tag, 1);
        assert_eq!(patterns.len(), 1);
        assert_eq!(fields, vec![inner.clone()]);
        assert_eq!(
            owned_path_type(None, &outer, &[VARIANT_FIELD, 1, 0], &mut 1000).unwrap(),
            inner
        );
        assert_eq!(
            owned_path_type(
                None,
                &outer,
                &[VARIANT_FIELD, 1, 0, VARIANT_FIELD, 1, 0],
                &mut 1000
            )
            .unwrap(),
            boolean
        );
        assert!(owned_path_type(None, &outer, &[VARIANT_FIELD, 0, 0], &mut 1000).is_err());
        assert!(owned_variant_pattern(None, &present, &boolean, &mut 1000).is_err());
        assert!(owned_variants(None, &outer, &mut 0).is_err());
        assert!(owned_variants(None, &outer, &mut 1000).is_ok());
    }
}
