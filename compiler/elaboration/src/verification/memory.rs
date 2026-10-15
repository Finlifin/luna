//! Body-derived pointer footprints. This is separate from type propositions.
//! A summary preserves initialization and transfers no pointee ownership. It is
//! not a certificate for termination, functional specifications, or escape safety.
use super::pattern_facts::{constant_bool, known_pattern, outer_guards};
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
    pub parameter: usize,
    /// Each segment selects a pointer-valued slot, then loads that pointer.
    /// `fields` selects the final place after all these loads.
    pub through: Vec<Vec<(nir::program::FieldOwner, usize)>>,
    pub fields: Vec<(nir::program::FieldOwner, usize)>,
}
impl ParameterPlace {
    fn path_cost(&self) -> usize {
        self.through
            .iter()
            .fold(self.fields.len().saturating_add(1), |cost, fields| {
                cost.saturating_add(fields.len()).saturating_add(1)
            })
    }
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
    through: Vec<Vec<(nir::program::FieldOwner, usize)>>,
    fields: Vec<(nir::program::FieldOwner, usize)>,
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
        self.through
            .iter()
            .fold(self.fields.len().saturating_add(1), |cost, path| {
                cost.saturating_add(path.len()).saturating_add(1)
            })
    }
    fn append_path(&mut self, suffix: &ParameterPlace) -> Result<(), String> {
        if self.path_cost().saturating_add(suffix.path_cost()) > 128 {
            return Err("memory pointer path depth exhausted".into());
        }
        for fields in &suffix.through {
            self.fields.extend_from_slice(fields);
            self.through.push(std::mem::take(&mut self.fields));
        }
        self.fields.extend_from_slice(&suffix.fields);
        Ok(())
    }
    fn external(self) -> Result<ParameterPlace, String> {
        let PointerRoot::Parameter(parameter) = self.root else {
            return Err("local memory target cannot escape in a result".into());
        };
        Ok(ParameterPlace {
            parameter,
            through: self.through,
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
        f.write_str(") (through")?;
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
        f.write_str("))")
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
mod effects;
pub(crate) use effects::effect_data_layout;
pub use effects::{
    ConditionalClosureSummary, EffectInput, infer_closure_with_effect_inputs,
    infer_with_effect_inputs,
};

#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub declaration: DeclarationId,
    pub arguments: Vec<Term>,
    pub signature: Term,
    /// Concrete code premises on which this footprint is conditional.
    pub callable_inputs: BTreeMap<CallableInput, CallableResult>,
    pub prepared_inputs: PreparedCodeInputs,
    /// Handler safety premises; accesses alone are not an unconditional footprint.
    pub effect_inputs: Vec<EffectInput>,
    pub accesses: Vec<Access>,
    /// May reject a checked program predicate; not an error-result ABI effect.
    pub may_reject: bool,
    /// Possible parameter-relative targets of a pointer result. None is opaque;
    /// an empty set means no established normal return in the fixed point.
    pub result_targets: Option<BTreeSet<ParameterPlace>>,
    pub callable_result: CallableResult,
    /// Named code at each owned function-valued result leaf. NoReturn is the
    /// worklist bottom for that leaf; Unknown is not a recursive seed.
    pub callable_fields: BTreeMap<Vec<usize>, CallableResult>,
    pub code_fields: CodeFields,
}
impl fmt::Display for ParameterPlace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(parameter-place (parameter {})", self.parameter)?;
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
        f.write_str("))")
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
        f.write_str(") (effect-inputs")?;
        for input in &self.effect_inputs {
            write!(f, " {input}")?;
        }
        f.write_str(") (accesses")?;
        for access in &self.accesses {
            write!(f, " {access}")?;
        }
        write!(f, ") (may-reject {}) (result-targets", self.may_reject)?;
        if let Some(targets) = &self.result_targets {
            for target in targets {
                write!(f, " {target}")?;
            }
        } else {
            f.write_str(" unknown")?;
        }
        write!(f, ") {} (callable-fields", self.callable_result)?;
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
    match ty {
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

fn owned_variants(
    package: Option<&Package>,
    ty: &Term,
    fuel: &mut usize,
) -> Result<Vec<nir::program::Variant>, String> {
    bound_type(ty, fuel)?;
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
            ty = owned_fields(package, &ty, fuel)?
                .get(path[position])
                .cloned()
                .ok_or("memory callable field index out of bounds")?;
            position += 1;
        }
    }
    Ok(ty)
}
fn result_callable_bottom(
    package: Option<&Package>,
    ty: &Term,
    fuel: &mut usize,
) -> Result<BTreeMap<Vec<usize>, CallableResult>, String> {
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
        if matches!(ty, Term::Function { .. })
            || include_closures && matches!(ty, Term::Closure { .. })
        {
            leaves.insert(path, CallableResult::NoReturn);
            continue;
        }
        if matches!(&ty,Term::Nominal {declaration,..} if package.is_some_and(|p|p.enums.iter().any(|e|e.declaration==*declaration)))
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
            if package.is_some_and(|p|p.structs.iter().any(|s|s.declaration==*declaration)));
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
    local_candidates: BTreeMap<CallablePlace, BTreeSet<usize>>,
    callees: BTreeMap<CallablePlace, CallableResult>,
}

// Invocation-local evidence; unlike a named Summary it has no declaration identity.
struct CallFootprint {
    accesses: Vec<Access>,
    may_reject: bool,
    result_targets: Option<BTreeSet<ParameterPlace>>,
    callable_result: CallableResult,
    callable_fields: BTreeMap<Vec<usize>, CallableResult>,
    code_fields: CodeFields,
}
impl From<Summary> for CallFootprint {
    fn from(value: Summary) -> Self {
        Self {
            accesses: value.accesses,
            may_reject: value.may_reject,
            result_targets: value.result_targets,
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

struct Infer<'a> {
    local_code: Vec<LocalCode>,
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
    arguments: &'a [Term],
    round: &'a mut Round,
    effect_row: Vec<nir::effects::Operation>,
    environment: Environment,
    locals: BTreeMap<LocalId, Term>,
    // Stable layout evidence survives temporary default-parameter environments.
    local_storage: Vec<(LocalId, Term)>,
    loop_states: Vec<Environment>,
    accesses: Vec<Access>,
    may_reject: bool,
    // Ephemeral node identity within this immutable body traversal only.
    expression_results: HashMap<usize, Targets>,
    callable_results: HashMap<usize, CallableResult>,
    aggregate_callees: HashMap<usize, BTreeMap<Vec<usize>, CallableResult>>,
    returns: Option<BTreeSet<ParameterPlace>>,
    callable_returns: CallableResult,
    callable_field_returns: BTreeMap<Vec<usize>, CallableResult>,
    code_field_returns: CodeFields,
}
/// Layout-only prerequisite; ownership and destructor contracts remain separate.
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
    if matches!(ty, Term::Function { .. })
        || intrinsic_pointer_value(ty) && !matches!(ty, Term::Pointer(_))
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

impl<'a> Infer<'a> {
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
            let expected = owned_path_type(self.package, &parameter.ty, &input.fields, self.fuel)?;
            if !matches!(expected, Term::Function { .. } | Term::Closure { .. }) {
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
                        if self.ty(&definition.ty)? != expected {
                            return Err(
                                "prepared closure signature disagrees with parameter".into()
                            );
                        }
                        self.pointer_free_owned(&expected, 0)?;
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
                        for (capture, ty) in closure_captures.iter().zip(environment) {
                            self.step(0)?;
                            if self.ty(&capture.ty)? != *ty || !bindings.insert(capture.binding) {
                                return Err("prepared closure capture layout mismatch".into());
                            }
                        }
                        self.remember_closure(
                            definition,
                            captures.as_ref().clone(),
                            prepared_captures.clone(),
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
    // An owned local without pointer-bearing storage cannot carry an alias of
    // a parameter into its destructor. Ownership and Drop legality still need
    // their separate checks; conservatively retain possible cleanup rejection.
    fn pointer_free_owned(&mut self, ty: &Term, depth: usize) -> Result<(), String> {
        let ty = self.type_view(ty)?;
        require_pointer_free_owned(self.package, &ty, self.fuel, depth)
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
    fn ty(&mut self, ty: &Term) -> Result<Term, String> {
        self.step(0)?;
        bound_type(ty, self.fuel)?;
        let ty = ty
            .instantiate_types(self.parameters, self.arguments)
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
        right: Environment,
    ) -> Result<Environment, String> {
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
    ) -> Result<(), String> {
        if let Some(value) = &capture.value {
            let fields = self.local_fields_for(value)?;
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
        Ok(())
    }
    fn remember_closure(
        &mut self,
        definition: &Expr,
        captures: BTreeMap<CallablePlace, CallableResult>,
        prepared_captures: std::sync::Arc<PreparedCaptures>,
    ) -> Result<(), String> {
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
            if previous.as_ref() == &captures && old_prepared == &prepared_captures {
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
            .instantiate_types(self.parameters, self.arguments)
            .map_err(|e| e.to_string())?;
        self.local_code.push(LocalCode::Closure {
            definition: std::sync::Arc::new(owned),
            captures: std::sync::Arc::new(captures),
            prepared_captures,
        });
        self.closure_nodes.entry(node).or_default().push(id);
        self.local_code_results
            .insert(node, Some(BTreeSet::from([id])));
        Ok(())
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
            K::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let known = constant_bool(condition, 0);
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
            K::Variant { index, fields, .. } => {
                let ty = self.ty(&value.ty)?;
                owned_path_type(self.package, &ty, path, self.fuel)?;
                if path.len() < 3 || path[0] != VARIANT_FIELD {
                    return Ok(false);
                }
                if path[1] != *index {
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
            Term::Function { .. } | Term::Closure { .. }
        ) {
            return Ok(self
                .local_candidates_for(expr)?
                .map(|keys| BTreeMap::from([(vec![], keys)]))
                .unwrap_or_default());
        }
        let mut values = BTreeMap::new();
        if let Some(place) = self.callable_place(expr)? {
            for (leaf, keys) in &self.environment.local_candidates {
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
            summary.may_reject |= next.may_reject;
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
            | K::Call { .. } => {
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
        if matches!(self.ty(&expr.ty)?, Term::Function { .. }) {
            return Ok(BTreeMap::from([(vec![], self.callee_key(expr)?)]));
        }
        if let Some(place) = self.callable_place(expr)? {
            let mut values = BTreeMap::new();
            for (leaf, key) in &self.environment.callees {
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
                            || !matches!(leaf, Term::Function { .. } | Term::Closure { .. })
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
        self.step(depth)?;
        match pattern {
            Pattern::Wild | Pattern::Unit | Pattern::Literal(_) => Ok(()),
            Pattern::Binding { binding, inner } => {
                bound_type(ty, self.fuel)?;
                self.locals.insert(*binding, ty.clone());
                charge_callee_fields(values, self.fuel)?;
                self.replace_callees(CallablePlace::root(*binding), values.clone())?;
                self.charge_local_fields(local_fields)?;
                self.replace_local_fields(CallablePlace::root(*binding), local_fields.clone())?;
                self.bind_owned_callees(inner, ty, values, local_fields, depth + 1)
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
                for (index, (pattern, ty)) in patterns.iter().zip(fields).enumerate() {
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
                    self.bind_owned_callees(pattern, &ty, &nested, &local_nested, depth + 1)?;
                }
                Ok(())
            }
            Pattern::Variant {
                declaration,
                index,
                fields: patterns,
            } => {
                if !matches!(ty,Term::Nominal {declaration:actual,..} if actual==declaration) {
                    return Err("memory callable variant declaration mismatch".into());
                }
                let variants = owned_variants(self.package, ty, self.fuel)?;
                let fields = &variants
                    .get(*index)
                    .ok_or("memory callable variant index out of bounds")?
                    .fields;
                if fields.len() != patterns.len() {
                    return Err("memory callable variant arity mismatch".into());
                }
                for (field, (pattern, ty)) in patterns.iter().zip(fields).enumerate() {
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
                    self.bind_owned_callees(pattern, ty, &nested, &code, depth + 1)?;
                }
                Ok(())
            }
            Pattern::Guard { pattern, condition } => {
                self.bind_owned_callees(pattern, ty, values, local_fields, depth + 1)?;
                self.walk_guard(condition, depth + 1)
            }
            Pattern::Then { first, value, next } => {
                self.bind_owned_callees(first, ty, values, local_fields, depth + 1)?;
                let computed = self.ty(&value.ty)?;
                if !Self::guard_value_type(&computed) {
                    return Err(
                        "computed memory pattern needs independent result provenance".into(),
                    );
                }
                self.walk(value, depth + 1)?;
                let named = self.value_callees(value)?;
                let code = self.local_fields_for(value)?;
                self.bind_owned_callees(next, &computed, &named, &code, depth + 1)
            }
            Pattern::Remap { pattern, bindings } => {
                self.bind_owned_callees(pattern, ty, values, local_fields, depth + 1)?;
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
                    self.bind_owned_callees(pattern, ty, values, local_fields, depth + 1)?;
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
            has_pointer |= matches!(self.ty(&capture.ty)?, Term::Pointer(_));
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
        if !matches!(result.as_ref(), Term::Pointer(_)) {
            self.pointer_free_owned(result, depth + 1)?;
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
            if matches!(ty, Term::Pointer(_)) {
                let target = if let Some(value) = &capture.value {
                    self.target(value, depth + 1)?
                } else {
                    let known = self
                        .environment
                        .pointers
                        .get(&capture.binding)
                        .and_then(Option::as_ref)
                        .ok_or("borrowed capture target is unknown")?;
                    for target in known {
                        *self.fuel = self
                            .fuel
                            .checked_sub(target.path_cost())
                            .ok_or("borrowed capture budget exhausted")?;
                    }
                    known.clone()
                };
                targets.insert(capture_slot, target);
                capture_slot += 1;
            } else {
                self.pointer_free_owned(&ty, depth + 1)?;
                self.snapshot_capture_code(capture, &mut captured_code, &mut prepared_captures)?;
            }
        }
        for argument in arguments {
            let ty = self.ty(&argument.value.ty)?;
            if !matches!(ty, Term::Pointer(_)) {
                self.pointer_free_owned(&ty, depth + 1)?;
            }
            self.walk(&argument.value, depth + 1)?;
            if matches!(ty, Term::Pointer(_)) {
                targets.insert(argument.parameter, self.target(&argument.value, depth + 1)?);
            }
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
            .instantiate_types(self.parameters, self.arguments)
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
        let result = self.rebase_result(summary.result_targets.as_ref(), &targets)?;
        self.expression_results
            .insert(expr as *const Expr as usize, result);
        for access in summary.accesses {
            let actual = targets
                .get(&access.target.parameter)
                .ok_or("borrowed closure target is missing")?;
            self.target_work(actual)?;
            for base in actual {
                self.step(depth)?;
                let mut target = base.clone();
                target.append_path(&access.target)?;
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
                self.pointer_free_owned(&ty, depth + 1)?;
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
                    through: vec![],
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
            K::ReadPointer(pointer) if matches!(self.ty(&expr.ty)?, Term::Pointer(_)) => {
                let Some(targets) = self.optional_target(pointer, depth + 1)? else {
                    return Ok(None);
                };
                self.target_work(&targets)?;
                let mut loaded = BTreeSet::new();
                for mut target in targets {
                    if target.path_cost() >= 128 {
                        return Err("memory pointer path depth exhausted".into());
                    }
                    target.through.push(std::mem::take(&mut target.fields));
                    loaded.insert(target);
                }
                Ok(Some(loaded))
            }
            K::ReadPointer(_) => Ok(None),
            K::Return(_) => Ok(Some(BTreeSet::new())),
            K::Call { .. } | K::Match { .. } | K::If { .. } => {
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
    fn record_access(
        &mut self,
        targets: BTreeSet<PointerPlace>,
        write: bool,
        read_value: bool,
        ty: Term,
    ) -> Result<(), String> {
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
                let path = target
                    .fields
                    .iter()
                    .map(|(_, index)| *index)
                    .collect::<Vec<_>>();
                let actual = owned_path_type(self.package, root, &path, self.fuel)?;
                if actual != ty {
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
        arguments: &BTreeMap<usize, BTreeSet<PointerPlace>>,
    ) -> Result<Targets, String> {
        let Some(result) = result else {
            return Ok(None);
        };
        let mut targets = BTreeSet::new();
        for target in result {
            let Some(actual) = arguments.get(&target.parameter) else {
                return Ok(None);
            };
            self.target_work(actual)?;
            for base in actual {
                *self.fuel = self
                    .fuel
                    .checked_sub(target.path_cost())
                    .ok_or("memory result budget exhausted")?;
                let mut place = base.clone();
                place.append_path(target)?;
                targets.insert(place);
            }
        }
        Ok(Some(targets))
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
        depth: usize,
    ) -> Result<(), String> {
        self.step(depth)?;
        match pattern {
            Pattern::Wild | Pattern::Unit | Pattern::Literal(_) => Ok(()),
            Pattern::Or(alternatives) => {
                for alternative in alternatives {
                    self.bind_match_subject(
                        alternative,
                        pointer,
                        is_pointer,
                        subject_ty,
                        depth + 1,
                    )?;
                }
                Ok(())
            }
            Pattern::Remap { pattern, bindings } => {
                self.bind_match_subject(pattern, pointer, is_pointer, subject_ty, depth + 1)?;
                for (_, to) in bindings {
                    bound_type(subject_ty, self.fuel)?;
                    self.locals.insert(*to, subject_ty.clone());
                }
                if is_pointer {
                    for (from, to) in bindings {
                        self.step(depth)?;
                        let targets = self
                            .environment
                            .pointers
                            .get(from)
                            .cloned()
                            .ok_or("memory pattern remap source is missing")?;
                        self.set_pointer(*to, targets)?;
                    }
                }
                Ok(())
            }
            Pattern::Binding { binding, inner } => {
                bound_type(subject_ty, self.fuel)?;
                self.locals.insert(*binding, subject_ty.clone());
                if is_pointer {
                    if let Some(targets) = pointer {
                        self.target_work(targets)?;
                    }
                    self.set_pointer(*binding, pointer.clone())?;
                }
                self.bind_match_subject(inner, pointer, is_pointer, subject_ty, depth + 1)
            }
            _ => Err("memory match pattern requires a richer subject contract".into()),
        }
    }
    fn guard_value_type(ty: &Term) -> bool {
        matches!(ty, Term::Primitive(_)) && intrinsic_pointer_value(ty)
            || matches!(ty, Term::Primitive(Primitive::Void))
            || matches!(ty, Term::Function { .. })
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
            Pattern::Binding { binding, inner } => {
                bindings.insert(*binding);
                self.check_guard_pattern(inner, bindings, depth + 1)
            }
            Pattern::Then { first, value, next } => {
                self.check_guard_pattern(first, bindings, depth + 1)?;
                if !Self::guard_value_type(&self.ty(&value.ty)?) {
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
            Pattern::Wild | Pattern::Unit | Pattern::Literal(_) => Ok(()),
        }
    }
    fn check_guard(&mut self, expr: &'a Expr, depth: usize) -> Result<(), String> {
        self.step(depth)?;
        if matches!(expr.kind, K::Block { .. }) && self.package.is_some() {
            super::observation_region::check(expr, self.package.unwrap(), self.fuel).map_err(
                |reason| format!("memory guard requires read-only observation: {reason}"),
            )?;
            return Ok(());
        }
        match &expr.kind {
            K::Literal(_) | K::Unit | K::Local(_) | K::Function(_) | K::FunctionInstance { .. } => {
                Ok(())
            }
            K::RequirePredicate(condition)
            | K::EnsurePredicate(condition)
            | K::CheckPredicate(condition)
                if constant_bool(condition, 0) == Some(true) =>
            {
                self.check_guard(condition, depth + 1)
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
            | K::Snapshot(value)
            | K::TakeBits(value)
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
                let skip = constant_bool(left, 0).map(|value| {
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
            K::ScalarEqual(left, right)
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
                let scalar = Self::guard_value_type;
                if !scalar(&self.ty(&expr.ty)?) {
                    return Err("memory guard call requires a scalar or code result".into());
                }
                for argument in arguments {
                    self.step(depth)?;
                    if !scalar(&self.ty(&argument.value.ty)?) {
                        return Err("memory guard call requires scalar or code arguments".into());
                    }
                    self.check_guard(&argument.value, depth + 1)?;
                }
                Ok(())
            }
            K::Block { statements, result } => {
                let mut bindings = BTreeSet::new();
                let scalar = Self::guard_value_type;
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
                            if constant_bool(condition,0)==Some(true)=>self.check_guard(condition,depth+1)?,
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
                if !Self::guard_value_type(&self.ty(&value.ty)?)
                    || !Self::guard_value_type(&self.ty(&expr.ty)?)
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
                        match constant_bool(guard, 0) {
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
                let known = constant_bool(condition, 0);
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
        if !matches!(subject_ty, Term::Pointer(_)) && !data_subject {
            self.pointer_free_owned(&subject_ty, depth + 1)?;
        }
        self.walk(value, depth + 1)?;
        let pointer_subject = matches!(subject_ty, Term::Pointer(_));
        let subject = if pointer_subject {
            self.optional_target(value, depth + 1)?
        } else {
            None
        };
        let subject_callees = self.value_callees(value)?;
        let subject_code = self.local_fields_for(value)?;
        let base = self.save_environment()?;
        let pointer_result = matches!(self.ty(&expr.ty)?, Term::Pointer(_));
        let callable_result = matches!(
            self.ty(&expr.ty)?,
            Term::Function { .. } | Term::Closure { .. }
        );
        let mut callable = CallableResult::NoReturn;
        let mut closures = Some(BTreeSet::new());
        let mut results = Some(BTreeSet::new());
        let mut continuation = None;
        let mut aggregate = None;
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
                self.bind_match_subject(pattern, &subject, true, &subject_ty, depth + 1)?;
            } else if data_subject {
                self.bind_data_pattern(pattern, &subject_ty, depth + 1)?;
            } else {
                self.bind_owned_callees(
                    pattern,
                    &subject_ty,
                    &subject_callees,
                    &subject_code,
                    depth + 1,
                )?;
            }
            let mut guard_known = Some(true);
            for guard in guards {
                self.walk_guard(guard, depth + 1)?;
                match constant_bool(guard, 0) {
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
    fn walk(&mut self, expr: &'a Expr, depth: usize) -> Result<(), String> {
        self.step(depth)?;
        if let Some(error) = nir::program::pointer_operation_error(expr) {
            return Err(error.into());
        }
        match &expr.kind {
            K::AddressOf(value) => {
                self.local_address(value, depth + 1)?;
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
                self.pointer_free_owned(&ty, depth + 1)?;
                let environment = match &ty {
                    Term::Closure { environment, .. } => environment.as_slice(),
                    Term::Function { .. } => &[][..],
                    _ => return Err("closure construction requires a callable type".into()),
                };
                if environment.len() != captures.len() {
                    return Err("closure memory layout disagrees with captures".into());
                }
                let mut captured_code = BTreeMap::new();
                let mut prepared_captures = BTreeMap::new();
                for (capture, expected) in captures.iter().zip(environment) {
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
                    self.snapshot_capture_code(
                        capture,
                        &mut captured_code,
                        &mut prepared_captures,
                    )?;
                }
                self.charge_capture_code(&captured_code)?;
                self.remember_closure(expr, captured_code, std::sync::Arc::new(prepared_captures))?;
                // Building code does not invoke its body. A nonempty environment
                // can still retire captured owners; without a cleanup contract
                // that remains a possible rejection at scope exit.
                self.may_reject |= !captures.is_empty();
            }
            K::Unit | K::Literal(_) | K::Local(_) | K::Break(_) | K::Continue(_) => (),
            K::AnonymousRecord { fields }
            | K::Record { fields, .. }
            | K::Variant { fields, .. }
            | K::Tuple(fields) => {
                let ty = self.ty(&expr.ty)?;
                self.pointer_free_owned(&ty, depth + 1)?;
                let mut values = result_callable_bottom(self.package, &ty, self.fuel)?;
                let mut local_fields: LocalCodeFields =
                    owned_callable_bottom(self.package, &ty, self.fuel, true)?
                        .into_keys()
                        .map(|path| (path, BTreeSet::new()))
                        .collect();
                for (index, field) in fields.iter().enumerate() {
                    let prefix = if let K::Variant { index: variant, .. } = &expr.kind {
                        vec![VARIANT_FIELD, *variant, index]
                    } else {
                        vec![index]
                    };
                    self.walk(field, depth + 1)?;
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
                self.may_reject |= !scalar_packet;
            }
            K::Field { value, index, .. } | K::AnonymousField { value, index, .. } => {
                self.walk(value, depth + 1)?;
                let parent = self.ty(&value.ty)?;
                if let K::Field { declaration, .. } = &expr.kind {
                    if !matches!(&parent,Term::Nominal {declaration:actual,..} if actual==declaration)
                    {
                        return Err("memory field projection declaration mismatch".into());
                    }
                }
                let field = owned_path_type(self.package, &parent, &[*index], self.fuel)?;
                if field != self.ty(&expr.ty)? {
                    return Err("memory field projection type mismatch".into());
                }
                self.pointer_free_owned(&field, depth + 1)?;
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
            K::ReadPointer(pointer) => {
                let ty = self.ty(&expr.ty)?;
                if !intrinsic_pointer_value(&ty) {
                    return Err("owned read requires a richer contract".into());
                }
                self.walk(pointer, depth + 1)?;
                let target = self.target(pointer, depth + 1)?;
                self.record_access(target, false, true, ty)?;
            }
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
                if let Some(reason) = nir::program::bit_update_error(
                    &storage,
                    selections,
                    &self.ty(&value.ty)?,
                    &self.ty(&expr.ty)?,
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
                if !intrinsic_pointer_value(&ty) || matches!(ty, Term::Pointer(_)) {
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
                let mut closures = Vec::new();
                let mut named = Vec::new();
                let candidates = self.local_candidates_for(callee)?;
                let code_bottom = candidates.as_ref().is_some_and(BTreeSet::is_empty);
                for id in candidates.unwrap_or_default() {
                    self.step(depth)?;
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
                            definition.clone(),
                            captures.clone(),
                            prepared_captures.clone(),
                        )),
                        LocalCode::Named(key) => {
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
                            self.expression_results
                                .insert(expr as *const Expr as usize, Some(BTreeSet::new()));
                            self.callable_results
                                .insert(expr as *const Expr as usize, CallableResult::NoReturn);
                            let ty = self.ty(&expr.ty)?;
                            let fields = result_callable_bottom(self.package, &ty, self.fuel)?;
                            self.aggregate_callees
                                .insert(expr as *const Expr as usize, fields);
                            let code_fields = code_result_bottom(self.package, &ty, self.fuel)?;
                            self.install_code_results(expr, &code_fields)?;
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
                    recovered = if let Some((closure, _, _)) = closures.first() {
                        let K::Closure { parameters, .. } = &closure.kind else {
                            unreachable!()
                        };
                        parameters.clone()
                    } else {
                        package
                            .functions
                            .iter()
                            .chain(&package.quote_templates)
                            .find(|f| f.declaration == keys[0].declaration)
                            .ok_or("memory callee body is missing")?
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
                let mut argument_callees = BTreeMap::new();
                let mut argument_local_fields = BTreeMap::new();
                let mut prepared_callees: BTreeMap<CallablePlace, CallableResult> = BTreeMap::new();
                for argument in arguments {
                    let saved = if argument.is_default {
                        for (place, key) in &prepared_callees {
                            self.step(place.fields.len() + 1)?;
                            key.charge(self.fuel)?;
                        }
                        let environment = Environment {
                            pointers: self.clone_pointer_environment(&prepared)?,
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
                        let target = if matches!(self.ty(&argument.value.ty)?, Term::Pointer(_)) {
                            self.optional_target(&argument.value, depth + 1)?
                        } else {
                            None
                        };
                        let values = self.value_callees(&argument.value)?;
                        let local_fields = self.local_fields_for(&argument.value)?;
                        Ok::<_, String>((target, values, local_fields))
                    })();
                    if let Some((locals, environment, loops)) = saved {
                        self.locals = locals;
                        self.environment = environment;
                        self.loop_states = loops;
                    }
                    let (target, values, local_fields) = result?;
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
                    if matches!(self.ty(&argument.value.ty)?, Term::Pointer(_)) {
                        if let Some(binding) = parameter_bindings.get(argument.parameter) {
                            prepared.insert(*binding, target.clone());
                        }
                    }
                    if let Some(target) = target {
                        targets.insert(argument.parameter, target);
                    }
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
                for (closure, captures, prepared_captures) in closures {
                    // Capture identities belong to construction time; later
                    // assignments in the enclosing frame cannot change them.
                    let next = infer_closure_in_round(
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
                    )?;
                    if self.ty(closure.ty.callable_signature())? != signature {
                        return Err("closure memory callee signature mismatch".into());
                    }
                    self.join_call_footprint(
                        &mut combined,
                        CallFootprint {
                            accesses: next.accesses,
                            may_reject: next.may_reject,
                            result_targets: next.result_targets,
                            callable_result: CallableResult::Unknown,
                            callable_fields: BTreeMap::new(),
                            code_fields: next.code_fields,
                        },
                    )?;
                }
                for key in keys {
                    self.step(depth)?;
                    let next = derive(
                        package,
                        key.declaration,
                        &key.arguments,
                        &inputs,
                        &prepared,
                        self.fuel,
                        self.round,
                    )?;
                    if next.signature != signature {
                        return Err("memory callee signature disagrees with its body".into());
                    }
                    self.join_call_footprint(&mut combined, next.into())?;
                }
                let summary = combined.ok_or("empty callable candidate set")?;
                self.install_code_results(expr, &summary.code_fields)?;
                self.may_reject |= summary.may_reject;
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
                let result_targets =
                    self.rebase_result(summary.result_targets.as_ref(), &targets)?;
                self.expression_results
                    .insert(expr as *const Expr as usize, result_targets);
                for access in summary.accesses {
                    self.step(depth)?;
                    let actual = targets
                        .get(&access.target.parameter)
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
                        target.append_path(&access.target)?;
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
                let ty = self.ty(&value.ty)?;
                let values = self.value_callees(value)?;
                let local_fields = self.local_fields_for(value)?;
                self.replace_callees(CallablePlace::root(*binding), values)?;
                self.locals.insert(*binding, ty);
                self.replace_local_fields(CallablePlace::root(*binding), local_fields)?;
                if matches!(self.ty(&value.ty)?, Term::Pointer(_)) {
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
                self.pointer_free_owned(&ty, depth + 1)?;
                let base = self.save_environment()?;
                if effect_data_layout(self.package, &ty, self.fuel, 0)? {
                    self.bind_data_pattern(pattern, &ty, depth + 1)?;
                } else {
                    let values = self.value_callees(value)?;
                    let local_fields = self.local_fields_for(value)?;
                    self.bind_owned_callees(pattern, &ty, &values, &local_fields, depth + 1)?;
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
                    K::BitAssign { start, end, .. } => nir::program::bit_writeback_error(
                        &storage, *start, *end, &value_ty, &result_ty,
                    ),
                    K::BitUpdate { selections, .. } => {
                        *self.fuel = self
                            .fuel
                            .checked_sub(selections.len())
                            .ok_or("bit selection memory budget exhausted")?;
                        nir::program::bit_update_error(&storage, selections, &value_ty, &result_ty)
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
                if !place.fields.is_empty() {
                    return Err("memory bit writeback field requires an ownership contract".into());
                }
                if self.locals.get(&place.root) != Some(&storage) {
                    return Err("memory bit writeback local storage type mismatch".into());
                }
                // Only a by-value scalar local is modified. Its initialization
                // follows the binding; the RHS may still access caller memory.
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
                let mut expected = root_ty;
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
                if !path.is_empty() {
                    self.pointer_free_owned(&expected, depth + 1)?;
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
                if path.is_empty() && matches!(self.ty(&value.ty)?, Term::Pointer(_)) {
                    let targets = self.optional_target(value, depth + 1)?;
                    self.set_pointer(place.root, targets)?;
                }
            }
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
                    if matches!(self.ty(&expr.ty)?, Term::Function { .. }) {
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
                let known = constant_bool(condition, 0);
                let pointer_result = matches!(self.ty(&expr.ty)?, Term::Pointer(_));
                let callable_result = matches!(
                    self.ty(&expr.ty)?,
                    Term::Function { .. } | Term::Closure { .. }
                );
                let mut callable = CallableResult::NoReturn;
                let mut closures = Some(BTreeSet::new());
                let mut results = Some(BTreeSet::new());
                let mut continuation = None;
                let mut aggregate = None;
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
            K::Return(value) => {
                if let Some(value) = value {
                    self.walk(value, depth + 1)?;
                    self.merge_callable_field_return(value)?;
                    if matches!(self.ty(&value.ty)?, Term::Pointer(_)) {
                        let targets = self.optional_target(value, depth + 1)?;
                        self.merge_return(targets)?;
                    } else if matches!(self.ty(&value.ty)?, Term::Function { .. }) {
                        let key = self.callee_key(value)?;
                        self.callable_returns.join(&key, self.fuel)?;
                    }
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
                self.may_reject |= constant_bool(condition, 0) != Some(true);
                // The footprint is conservative over the successful path. No
                // ownership or memory-state facts are inferred from the check.
            }
            K::Discard(value)
            | K::Not(value)
            | K::IntegerNegate(value)
            | K::FloatNegate(value)
            | K::BitConvert(value)
            | K::Snapshot(value)
            | K::TakeBits(value)
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
                let known = constant_bool(a, 0);
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
}
#[derive(Default)]
struct Round {
    effects: Vec<EffectInput>,
    complete_calls: bool,
    code: Vec<PreparedCode>,
    previous: HashMap<MemoryInstance, Summary>,
    current: HashMap<MemoryInstance, Summary>,
    queue: Vec<MemoryInstance>,
    discovered: HashSet<MemoryInstance>,
}

pub fn infer(function: &Function, fuel: &mut usize) -> Result<Summary, String> {
    infer_body(
        function,
        &[],
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

/// Recover concrete code for body-derived footprints without erasing interface
/// predicates. Predicate-bearing interfaces need their own footprint composition
/// and remain rejected until that path is available.
pub(crate) fn method_callee(
    package: &Package,
    callee: &Expr,
    fuel: &mut usize,
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
    if !contract.program.preconditions.is_empty() || !contract.program.postconditions.is_empty() {
        return Err("interface predicates require a composed memory contract".into());
    }
    let method = if contract.is_quote {
        crate::materialization::resolve_quote_method_instance(package, callee, fuel)?
    } else {
        crate::materialization::resolve_method_instance(package, callee, fuel)?
    };
    Ok(Expr {
        source: callee.source,
        ty: callee.ty.clone(),
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
fn infer_context(
    package: &Package,
    declaration: DeclarationId,
    arguments: &[Term],
    inputs: &BTreeMap<CallableInput, CallableResult>,
    prepared: &PreparedCodeInputs,
    effects: &[EffectInput],
    fuel: &mut usize,
) -> Result<Summary, String> {
    let mut round = Round {
        effects: effects::copy_inputs(effects, fuel)?,
        ..Round::default()
    };
    derive(
        package,
        declaration,
        arguments,
        inputs,
        prepared,
        fuel,
        &mut round,
    )?;
    loop {
        let mut index = 0;
        while index < round.queue.len() {
            effects::charge_inputs(&round.queue[index].effects, fuel)?;
            let key = round.queue[index].clone();
            let function = package
                .functions
                .iter()
                .chain(&package.quote_templates)
                .find(|function| function.declaration == key.instance.declaration)
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
                &key.instance.arguments,
                &key.inputs,
                &prepared,
                Some(package),
                fuel,
                &mut round,
            );
            round.effects = previous_effects;
            let mut summary = result?;
            if summary.accesses.len() > *fuel {
                return Err("memory summary budget exhausted".into());
            }
            *fuel -= summary.accesses.len();
            summary.accesses.sort_by_cached_key(ToString::to_string);
            summary.accesses.dedup();
            round.current.insert(key, summary);
            index += 1;
        }
        if round.current == round.previous {
            return round
                .current
                .remove(&round.queue[0])
                .ok_or("missing root memory summary".into());
        }
        round.previous = std::mem::take(&mut round.current);
    }
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
            Term::Pointer(inner) | Term::ComptimeFunction(inner) => {
                pending.push((inner, depth + 1))
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
fn derive(
    package: &Package,
    declaration: DeclarationId,
    arguments: &[Term],
    inputs: &BTreeMap<CallableInput, CallableResult>,
    prepared: &PreparedCodeInputs,
    fuel: &mut usize,
    round: &mut Round,
) -> Result<Summary, String> {
    if round.complete_calls {
        return infer_context(
            package,
            declaration,
            arguments,
            inputs,
            prepared,
            &round.effects,
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
    let key = MemoryInstance {
        effects: effects::copy_inputs(&round.effects, fuel)?,
        instance: iir::InstanceKey {
            declaration,
            arguments: arguments.to_vec(),
        },
        inputs: inputs.clone(),
        prepared: prepared_ids,
    };
    if round.discovered.insert(key.clone()) {
        round.queue.push(key.clone());
    }
    if let Some(summary) = round.previous.get(&key) {
        effects::charge_inputs(&summary.effect_inputs, fuel)?;
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
    let function = package
        .functions
        .iter()
        .chain(&package.quote_templates)
        .find(|function| function.declaration == declaration)
        .ok_or("memory summary function is missing")?;
    if function.phase != FunctionPhase::Runtime || function.type_parameters.len() != arguments.len()
    {
        return Err("memory summary needs a concrete runtime body".into());
    }
    bound_type(&function.signature, fuel)?;
    bound_type(&function.signature, fuel)?;
    let signature = function
        .signature
        .instantiate_types(&function.type_parameters, arguments)
        .map_err(|error| error.to_string())?;
    bound_type(&signature, fuel)?;
    let Term::Function { result, .. } = signature.callable_signature() else {
        return Err("memory summary requires a function signature".into());
    };
    let callable_fields = result_callable_bottom(Some(package), result, fuel)?;
    let code_fields = code_result_bottom(Some(package), result, fuel)?;
    // A provisional bottom footprint is consumed only inside this worklist.
    // Every queued body must validate and all summaries must converge first.
    Ok(Summary {
        declaration,
        arguments: arguments.to_vec(),
        signature,
        callable_inputs: inputs.clone(),
        prepared_inputs: prepared.clone(),
        effect_inputs: effects::copy_inputs(&round.effects, fuel)?,
        accesses: vec![],
        may_reject: false,
        result_targets: Some(BTreeSet::new()),
        callable_result: CallableResult::NoReturn,
        callable_fields,
        code_fields,
    })
}
fn validate_code_premise(
    package: &Package,
    expected: &Term,
    value: &CallableResult,
    fuel: &mut usize,
) -> Result<(), String> {
    if !matches!(expected, Term::Function { .. }) {
        return Err("memory callable premise path is not a function".into());
    }
    // NoReturn is private worklist bottom, admitted only between rounds.
    if matches!(value, CallableResult::NoReturn) {
        return Ok(());
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
        let function = package
            .functions
            .iter()
            .find(|function| function.declaration == key.declaration)
            .ok_or("memory callable definition is missing")?;
        if function.phase != FunctionPhase::Runtime
            || function.type_parameters.len() != key.arguments.len()
        {
            return Err("memory callable requires a concrete runtime instance".into());
        }
        bound_type(&function.signature, fuel)?;
        let signature = function
            .signature
            .instantiate_types(&function.type_parameters, &key.arguments)
            .map_err(|error| error.to_string())?;
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
    let signature = function
        .signature
        .instantiate_types(&function.type_parameters, arguments)
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
            &expected,
            value,
            fuel,
        )?;
    }
    let frames = vec![parameters.iter().map(|p| p.sort).collect()];
    let mut type_context = crate::ptc::Context::with_scope(frames.clone());
    let declarations = function
        .type_parameters
        .iter()
        .map(|id| nir::DeclarationParameter {
            declaration: *id,
            sort: nir::Sort::Type,
        })
        .collect::<Vec<_>>();
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
        local_code: vec![],
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
        arguments,
        round,
        effect_row: vec![],
        environment: Environment::default(),
        locals: BTreeMap::new(),
        local_storage: vec![],
        loop_states: vec![],
        accesses: vec![],
        may_reject: false,
        expression_results: HashMap::new(),
        callable_results: HashMap::new(),
        aggregate_callees: HashMap::new(),
        returns: Some(BTreeSet::new()),
        callable_returns: CallableResult::NoReturn,
        callable_field_returns: BTreeMap::new(),
        code_field_returns: BTreeMap::new(),
    };
    let result_view = infer.effect_completion(result)?;
    if let Term::Computation { effects, .. } = result.as_ref() {
        infer.effect_row = effects.clone();
    }
    if !intrinsic_pointer_value(&result_view) && result_view != Term::Primitive(Primitive::Void) {
        infer
            .pointer_free_owned(&result_view, 0)
            .map_err(|reason| {
                format!("memory result requires an escape or ownership contract: {reason}")
            })?;
    }
    infer.callable_field_returns = result_callable_bottom(package, result, infer.fuel)?;
    infer.code_field_returns = code_result_bottom(package, result, infer.fuel)?;
    for (index, (binding, parameter)) in function.parameters.iter().zip(parameters).enumerate() {
        infer.step(0)?;
        infer.locals.insert(*binding, parameter.ty.clone());
        // Type inputs select types, not runtime owned storage or pointer roots.
        // Keep their positional slot and local annotation for body inspection.
        if parameter.sort == nir::Sort::Type && parameter.ty == Term::Primitive(Primitive::Type) {
            continue;
        }
        for (fields, _) in result_callable_bottom(package, &parameter.ty, infer.fuel)? {
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
        if !intrinsic_pointer_value(&parameter.ty) {
            infer
                .pointer_free_owned(&parameter.ty, 0)
                .map_err(|reason| {
                    format!("memory argument requires an ownership contract: {reason}")
                })?;
        }
        if matches!(parameter.ty, Term::Pointer(_)) {
            infer.environment.pointers.insert(
                *binding,
                Some(BTreeSet::from([PointerPlace {
                    root: PointerRoot::Parameter(index),
                    through: vec![],
                    fields: vec![],
                }])),
            );
        }
    }
    infer.install_code_inputs(&function.parameters, parameters, inputs, &code_inputs)?;
    for predicate in &function.preconditions {
        if !nir::program::checkable_predicate(predicate) {
            return Err("invalid runtime predicate check".into());
        }
        infer.walk_guard(predicate, 0)?;
        infer.may_reject |= constant_bool(predicate, 0) != Some(true);
    }
    infer.walk(&function.body, 0)?;
    let completes = !super::pattern_facts::cannot_complete(&function.body, infer.fuel, 0)?;
    if completes && infer.ty(&function.body.ty)? == **result {
        infer.merge_callable_field_return(&function.body)?;
    }
    let pointer_result = matches!(result.as_ref(), Term::Pointer(_));
    if completes && pointer_result && matches!(infer.ty(&function.body.ty)?, Term::Pointer(_)) {
        let targets = infer.optional_target(&function.body, 0)?;
        infer.merge_return(targets)?;
    }
    let callable_result = matches!(result.as_ref(), Term::Function { .. });
    if completes && callable_result && matches!(infer.ty(&function.body.ty)?, Term::Function { .. })
    {
        let key = infer.callee_key(&function.body)?;
        infer.callable_returns.join(&key, infer.fuel)?;
    }
    for predicate in &function.postconditions {
        if !nir::program::checkable_predicate(predicate) {
            return Err("invalid runtime predicate check".into());
        }
        infer.walk_guard(predicate, 0)?;
        infer.may_reject |= constant_bool(predicate, 0) != Some(true);
    }
    Ok(Summary {
        declaration: function.declaration,
        arguments: arguments.to_vec(),
        signature,
        callable_inputs: inputs.clone(),
        prepared_inputs: prepared.clone(),
        effect_inputs: effects::copy_inputs(&infer.round.effects, infer.fuel)?,
        accesses: infer.accesses,
        code_fields: infer.code_field_returns,
        may_reject: infer.may_reject,
        result_targets: if pointer_result { infer.returns } else { None },
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
    pub result_targets: Option<BTreeSet<ParameterPlace>>,
}
impl fmt::Display for ArgumentSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(argument-memory (accesses")?;
        for access in &self.accesses {
            write!(f, " {access}")?;
        }
        write!(f, ") (may-reject {}) (result-targets", self.may_reject)?;
        if let Some(targets) = &self.result_targets {
            for target in targets {
                write!(f, " {target}")?;
            }
        } else {
            f.write_str(" unknown")?;
        }
        f.write_str(")")?;
        display_code_fields(&self.code_fields, f)?;
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
        && !matches!(expression.ty, Term::Function { .. })
        && expression.ty != Term::Primitive(Primitive::Void)
    {
        // The footprint only needs to rule out untracked escaping pointers.
        // Ownership, moves and cleanup are checked separately by the caller's
        // isolated default-evaluation frame, including aggregate results.
        require_pointer_free_owned(Some(package), &expression.ty, fuel, 0).map_err(|reason| {
            format!("default argument requires an ownership contract: {reason}")
        })?;
    }
    let mut round = Round {
        complete_calls: true,
        ..Default::default()
    };
    let mut infer = Infer {
        local_code: vec![],
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
        arguments: &[],
        round: &mut round,
        effect_row: vec![],
        environment: Environment::default(),
        locals: BTreeMap::new(),
        local_storage: vec![],
        loop_states: vec![],
        accesses: vec![],
        may_reject: false,
        expression_results: HashMap::new(),
        callable_results: HashMap::new(),
        aggregate_callees: HashMap::new(),
        returns: Some(BTreeSet::new()),
        callable_returns: CallableResult::NoReturn,
        callable_field_returns: BTreeMap::new(),
        code_field_returns: BTreeMap::new(),
    };
    for (binding, ty, slot) in prepared {
        infer.step(0)?;
        bound_type(ty, infer.fuel)?;
        infer.locals.insert(*binding, ty.clone());
        if matches!(ty, Term::Pointer(_)) {
            infer.environment.pointers.insert(
                *binding,
                Some(BTreeSet::from([PointerPlace {
                    root: PointerRoot::Parameter(*slot),
                    through: vec![],
                    fields: vec![],
                }])),
            );
        }
    }
    infer.walk(expression, 0)?;
    let result_targets = if matches!(expression.ty, Term::Pointer(_)) {
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
        accesses: infer.accesses,
        code_fields: infer.code_field_returns,
        may_reject: infer.may_reject,
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

/// Direct pointer captures are modeled as additional pointer inputs, in capture
/// order after ordinary inputs. The caller must map these virtual slots to the
/// actual environment storage's checked pointee origins. This view does not
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
    )
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
) -> Result<ArgumentSummary, String> {
    let K::Closure { captures, body, .. } = &definition.kind else {
        return Err("closure memory requires a closure definition".into());
    };
    *fuel = fuel
        .checked_sub(captures.len())
        .ok_or("borrowed capture budget exhausted")?;
    if !captures
        .iter()
        .any(|capture| matches!(capture.ty, Term::Pointer(_)))
    {
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
            false,
        );
    }
    let pointer_bindings: BTreeSet<_> = captures
        .iter()
        .filter(|capture| matches!(capture.ty, Term::Pointer(_)))
        .map(|capture| capture.binding)
        .collect();
    // Bound the complete retained tree before cloning, but validate storage
    // effects only where the borrowed environment is actually in scope.
    nir::visit::expression(definition, fuel, |expr, fuel| bound_type(&expr.ty, fuel))?;
    validate_borrowed_capture_storage(body, &pointer_bindings, fuel)?;
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
    for capture in captures {
        if matches!(capture.ty, Term::Pointer(_)) {
            bindings.push(capture.binding);
            inputs_ty.push(nir::Parameter {
                sort: nir::Sort::Runtime,
                ty: capture.ty.clone(),
            });
        }
    }
    owned.retain(|capture| !matches!(capture.ty, Term::Pointer(_)));
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
    infer_closure_in_round(
        package,
        &checked,
        callees,
        prepared_captures,
        inputs,
        code_inputs,
        fuel,
        round,
        (&[], &[]),
        false,
    )
}

fn validate_borrowed_capture_storage<'a>(
    body: &'a Expr,
    bindings: &BTreeSet<LocalId>,
    fuel: &mut usize,
) -> Result<(), String> {
    let mut pending = vec![body];
    while let Some(root) = pending.pop() {
        nir::visit::expression_pruned(root, fuel, |expr, fuel| {
            match &expr.kind {
                K::Assign {place,..} if bindings.contains(&place.root) =>
                    return Err("captured pointer rebinding requires an environment writeback contract".into()),
                K::AddressOf(value) if matches!(value.kind,K::Local(id) if bindings.contains(&id)) =>
                    return Err("addressing captured pointer storage requires an environment memory contract".into()),
                K::Closure {captures,..} => {
                    // A nested body uses private capture cells; initializers
                    // still execute against the current environment.
                    *fuel=fuel.checked_sub(captures.len()).ok_or("capture storage traversal budget exhausted")?;
                    pending.extend(captures.iter().filter_map(|capture|capture.value.as_deref()));
                    return Ok(false);
                }
                K::Call {callee,arguments,..} => {
                    *fuel=fuel.checked_sub(arguments.len().saturating_add(1)).ok_or("capture storage traversal budget exhausted")?;
                    pending.push(callee);
                    pending.extend(arguments.iter().filter(|argument|!argument.is_default).map(|argument|&argument.value));
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
        let (summary, writes) = infer_closure_pass(
            package,
            definition,
            &BTreeMap::new(),
            &invariant,
            inputs,
            code_inputs,
            fuel,
            round,
            specialization,
            true,
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
    let closure_ty = definition
        .ty
        .instantiate_types(specialization.0, specialization.1)
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
            .instantiate_types(specialization.0, specialization.1)
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
                .instantiate_types(specialization.0, specialization.1)
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
        local_code: vec![],
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
        arguments: specialization.1,
        round,
        effect_row: vec![],
        environment: Environment::default(),
        locals: BTreeMap::new(),
        local_storage: vec![],
        loop_states: vec![],
        accesses: vec![],
        may_reject: false,
        expression_results: HashMap::new(),
        callable_results: HashMap::new(),
        aggregate_callees: HashMap::new(),
        returns: Some(BTreeSet::new()),
        callable_returns: CallableResult::NoReturn,
        callable_field_returns: BTreeMap::new(),
        code_field_returns: BTreeMap::new(),
    };
    let completion = infer.effect_completion(result)?;
    if let Term::Computation { effects, .. } = result.as_ref() {
        infer.effect_row = effects.clone();
    }
    if !intrinsic_pointer_value(&completion) && completion != Term::Primitive(Primitive::Void) {
        require_pointer_free_owned(Some(package), &completion, infer.fuel, 0)?;
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
        if !matches!(expected, Term::Function { .. })
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
            let function = package
                .functions
                .iter()
                .find(|function| function.declaration == key.declaration)
                .ok_or("closure memory callable definition is missing")?;
            if function.phase != FunctionPhase::Runtime
                || function.type_parameters.len() != key.arguments.len()
            {
                return Err("closure memory callable requires a concrete runtime instance".into());
            }
            bound_type(&function.signature, infer.fuel)?;
            let signature = function
                .signature
                .instantiate_types(&function.type_parameters, &key.arguments)
                .map_err(|error| error.to_string())?;
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
                || !matches!(leaf, Term::Function { .. } | Term::Closure { .. })
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
        if !intrinsic_pointer_value(&parameter.ty) {
            infer.pointer_free_owned(&parameter.ty, 0)?;
        }
        infer.locals.insert(*binding, parameter.ty.clone());
        if matches!(parameter.ty, Term::Pointer(_)) {
            infer.environment.pointers.insert(
                *binding,
                Some(BTreeSet::from([PointerPlace {
                    root: PointerRoot::Parameter(index),
                    through: vec![],
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
        validate_code_premise(package, &expected, value, infer.fuel)?;
        infer.environment.callees.insert(
            CallablePlace {
                binding: bindings[input.parameter],
                fields: input.fields.clone(),
            },
            value.clone(),
        );
    }
    infer.install_code_inputs(bindings, parameters, inputs, code_inputs)?;
    infer.code_field_returns = code_result_bottom(Some(package), result, infer.fuel)?;
    infer.walk(body, 0)?;
    if infer.ty(&body.ty)? == **result {
        infer.merge_code_field_return(body)?;
    }
    let pointer_result = matches!(result.as_ref(), Term::Pointer(_));
    if pointer_result && matches!(body.ty, Term::Pointer(_)) {
        let targets = infer.optional_target(body, 0)?;
        infer.merge_return(targets)?;
    }
    Ok((
        ArgumentSummary {
            accesses: infer.accesses,
            code_fields: infer.code_field_returns,
            may_reject: infer.may_reject,
            result_targets: if pointer_result { infer.returns } else { None },
        },
        infer.capture_code_writes,
    ))
}

#[cfg(test)]
mod record_layout_tests {
    use super::*;
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
        assert!(validate_borrowed_capture_storage(&address(), &bindings, &mut 100).is_err());
        let write = node(K::Assign {
            place: Place {
                root: id(),
                fields: vec![],
            },
            value: Box::new(node(K::Local(id()))),
        });
        assert!(validate_borrowed_capture_storage(&write, &bindings, &mut 100).is_err());
        for private in [address(), write] {
            validate_borrowed_capture_storage(&nested(private, None), &bindings, &mut 100).unwrap();
        }
        assert!(
            validate_borrowed_capture_storage(
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
                validate_borrowed_capture_storage(&call, &BTreeSet::from([id()]), &mut 100).is_ok(),
                is_default
            );
        }
    }
    #[test]
    fn scope_scan_is_bounded_and_budget_failure_does_not_poison_later_checks() {
        let body = nested(node(K::Unit), Some(node(K::Unit)));
        assert!(
            validate_borrowed_capture_storage(&body, &BTreeSet::from([id()]), &mut 1)
                .unwrap_err()
                .contains("budget")
        );
        validate_borrowed_capture_storage(&body, &BTreeSet::from([id()]), &mut 100).unwrap();
    }
}

#[cfg(test)]
mod pointer_root_tests {
    use super::*;
    #[test]
    fn local_roots_are_distinct_from_parameter_slots_and_cannot_publish() {
        let local = PointerPlace {
            root: PointerRoot::Local {
                binding: LocalId { owner: 7, index: 1 },
                storage: 0,
            },
            through: vec![],
            fields: vec![],
        };
        let parameter = PointerPlace {
            root: PointerRoot::Parameter(1),
            through: vec![],
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
                through: vec![],
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
