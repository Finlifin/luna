//! Bind already evaluated immutable inputs before runtime invocation.
//! Declaration/type substitution precedes this pass. This pass does not invoke
//! user code or prove contracts; all program predicates remain executable.
//!
//! Layout-aware entrypoints receive normal-form descriptors from the caller's
//! validated package. Tuple and list roles expose their respective layouts;
//! neither spelling nor the shape of application arguments grants that role.
//! Folding preserves the original expression types. Frozen tuple values enter
//! instance keys as TupleValue, independently of their retained Type identity.
use nir::program::{Expr, ExprKind as K, Function, Literal, LocalId};
use nir::{Primitive, Sort, Term};
use std::collections::{BTreeMap, BTreeSet};
mod error_values;
mod frozen;
mod general;
use general::fold_general_input;
mod lists;
mod patterns;
mod preparation;
pub use preparation::{preparation, preparation_with_layouts};

/// Borrowed, checked layout metadata. Type identity remains nominal; this view
/// authorizes reconstruction only and never participates in type equality.
#[derive(Clone, Copy)]
pub struct Layouts<'a> {
    target: Option<nir::target::TargetData>,
    heads: &'a [nir::normal_forms::Head],
    newtypes: &'a [nir::newtypes::Newtype],
    newtype_index:
        Option<&'a std::collections::BTreeMap<nir::DeclarationId, nir::newtypes::Newtype>>,
}
impl<'a> Layouts<'a> {
    fn list_element(self, ty: &Term, fuel: &mut usize) -> Result<Option<Term>, String> {
        for head in self.heads {
            charge(fuel, 1)?;
            if let Some(element) = head.list_element(ty) {
                nir::type_computations::charge_term(element, fuel).map_err(|e| e.to_string())?;
                return Ok(Some(element.clone()));
            }
        }
        Ok(None)
    }
    pub fn new(
        heads: &'a [nir::normal_forms::Head],
        newtypes: &'a [nir::newtypes::Newtype],
    ) -> Self {
        Self {
            target: None,
            heads,
            newtypes,
            newtype_index: None,
        }
    }
    pub fn from_index(
        heads: &'a [nir::normal_forms::Head],
        newtypes: &'a std::collections::BTreeMap<nir::DeclarationId, nir::newtypes::Newtype>,
    ) -> Self {
        Self {
            target: None,
            heads,
            newtypes: &[],
            newtype_index: Some(newtypes),
        }
    }
    pub fn with_target(mut self, target: Option<nir::target::TargetData>) -> Self {
        self.target = target;
        self
    }

    fn integer_layout(self, primitive: Primitive) -> Option<(bool, u32)> {
        match self.target {
            Some(target) => target.integer_layout(primitive),
            None => primitive.fixed_integer_layout(),
        }
    }

    fn representation(self, ty: &Term, fuel: &mut usize) -> Result<Option<Term>, String> {
        let Term::Nominal {
            declaration,
            arguments,
        } = ty
        else {
            return Ok(None);
        };
        if let Some(index) = self.newtype_index {
            charge(fuel, 1)?;
            return index
                .get(declaration)
                .map(|definition| {
                    definition
                        .representation_budgeted(arguments, &[], fuel)
                        .map_err(|error| error.to_string())
                })
                .transpose();
        }
        for definition in self.newtypes {
            charge(fuel, 1)?;
            if definition.declaration == *declaration {
                return definition
                    .representation_budgeted(arguments, &[], fuel)
                    .map(Some)
                    .map_err(|error| error.to_string());
            }
        }
        Ok(None)
    }
}
impl<'a> From<&'a [nir::normal_forms::Head]> for Layouts<'a> {
    fn from(heads: &'a [nir::normal_forms::Head]) -> Self {
        Self::new(heads, &[])
    }
}
impl<'a, const N: usize> From<&'a [nir::normal_forms::Head; N]> for Layouts<'a> {
    fn from(heads: &'a [nir::normal_forms::Head; N]) -> Self {
        Self::new(heads, &[])
    }
}
impl<'a> From<&'a Vec<nir::normal_forms::Head>> for Layouts<'a> {
    fn from(heads: &'a Vec<nir::normal_forms::Head>) -> Self {
        Self::new(heads, &[])
    }
}
impl<'a> From<&'a nir::program::Package> for Layouts<'a> {
    fn from(package: &'a nir::program::Package) -> Self {
        Self::new(&package.normal_forms, &package.newtypes).with_target(package.target)
    }
}
impl<'a> IntoIterator for Layouts<'a> {
    type Item = &'a nir::normal_forms::Head;
    type IntoIter = std::slice::Iter<'a, nir::normal_forms::Head>;
    fn into_iter(self) -> Self::IntoIter {
        self.heads.iter()
    }
}
impl std::ops::Deref for Layouts<'_> {
    type Target = [nir::normal_forms::Head];
    fn deref(&self) -> &Self::Target {
        self.heads
    }
}

/// Remove supplied staged inputs from the callable signature and local input
/// list. Closed constants are restored as local bindings in each independently
/// evaluated body/contract/default scope. The caller retains these values in
/// the instance identity and checks type formation and stage availability.
/// This preserves addressability,
/// closure capture and parameter mutation semantics without re-evaluating the
/// original comptime expression. Remaining default indices are renumbered.
pub fn bind_inputs_with_layouts<'layout>(
    heads: impl Into<Layouts<'layout>>,
    template: &Function,
    inputs: &[Option<Expr>],
    fuel: &mut usize,
) -> Result<Function, String> {
    let heads = heads.into();
    bind_inputs_with_type_view(heads, template, inputs, fuel, |ty, _| Ok(ty.clone()))
}

fn bind_inputs_with_type_view<'layout>(
    heads: impl Into<Layouts<'layout>>,
    template: &Function,
    inputs: &[Option<Expr>],
    fuel: &mut usize,
    mut type_view: impl FnMut(&Term, &mut usize) -> Result<Term, String>,
) -> Result<Function, String> {
    let heads = heads.into();
    let Term::Function { parameters, .. } = template.signature.callable_signature() else {
        return Err("staged input template is not a function".into());
    };
    if parameters.len() != inputs.len() || template.parameters.len() != inputs.len() {
        return Err("staged input arity mismatch".into());
    }
    let unique: BTreeSet<_> = template.parameters.iter().copied().collect();
    if unique.len() != inputs.len() {
        return Err("duplicate staged parameter binding".into());
    }
    let mut symbols = Vec::with_capacity(inputs.len());
    for (parameter, input) in parameters.iter().zip(inputs) {
        let symbol = if let Some(value) = input {
            if parameter.sort == Sort::Runtime {
                return Err("runtime input cannot be erased as a staged input".into());
            }
            Some(constant(heads, value, fuel, 0)?)
        } else {
            None
        };
        symbols.push(symbol);
    }
    let scope =
        prepare_binding_with_type_view(&template.signature, &symbols, fuel, &mut type_view)?;
    let mut bindings = Vec::new();
    for ((parameter, binding), input) in parameters.iter().zip(&template.parameters).zip(inputs) {
        if let Some(value) = input {
            let expected = scope.rebind(&parameter.ty).map_err(|e| e.to_string())?;
            let expected = binding_type_view(&expected, scope.signature(), fuel, &mut type_view)?;
            if value.ty != expected {
                return Err("staged value type differs from parameter type".into());
            }
            bindings.push((*binding, value.clone()));
        }
    }
    crate::visit::expressions(template, fuel, |_, _| Ok(()))?;
    let mut result = template.clone();
    result.signature = scope.signature().clone();
    for root in std::iter::once(&mut result.body)
        .chain(&mut result.preconditions)
        .chain(&mut result.postconditions)
        .chain(result.defaults.iter_mut().map(|(_, value)| value))
    {
        root.rewrite_types(&mut |ty| {
            charge(fuel, 1)?;
            let ty = scope.rebind(ty).map_err(|error| error.to_string())?;
            binding_type_view(&ty, scope.signature(), fuel, &mut type_view)
        })?;
    }
    for requirement in &mut result.capabilities {
        rebind_requirement(requirement, &scope, fuel)?;
    }
    let mut predicates = result.requirements.iter_mut().collect::<Vec<_>>();
    while let Some(predicate) = predicates.pop() {
        charge(fuel, 1)?;
        use nir::program::TypePredicate as P;
        match predicate {
            P::Bool(_) => {}
            P::Trait(requirement) => rebind_requirement(requirement, &scope, fuel)?,
            P::Equal(left, right) => {
                *left = scope.rebind(left).map_err(|e| e.to_string())?;
                *right = scope.rebind(right).map_err(|e| e.to_string())?;
            }
            P::Not(value) => predicates.push(value),
            P::And(values) | P::Or(values) => predicates.extend(values),
        }
    }
    let mut slots = Vec::new();
    result.parameters.clear();
    for (binding, input) in template.parameters.iter().zip(inputs) {
        slots.push(if input.is_none() {
            let index = result.parameters.len();
            result.parameters.push(*binding);
            Some(index)
        } else {
            None
        });
    }
    // Entry snapshots of erased inputs no longer read the invocation's argument
    // frame. Restore their original constant in predicate/body scopes instead.
    result.entry_snapshots.clear();
    for snapshot in &template.entry_snapshots {
        if let Some((_, value)) = bindings
            .iter()
            .find(|(binding, _)| *binding == snapshot.parameter)
        {
            let value = value.clone();
            if unique.contains(&snapshot.binding)
                || bindings.iter().any(|(id, _)| *id == snapshot.binding)
            {
                return Err("duplicate staged snapshot binding".into());
            }
            bindings.push((snapshot.binding, value));
        } else {
            result.entry_snapshots.push(snapshot.clone());
        }
    }
    substitute_forwarded_inputs(heads, &mut result, &bindings, fuel)?;
    wrap(heads, &mut result.body, &bindings, fuel)?;
    for expression in result
        .preconditions
        .iter_mut()
        .chain(&mut result.postconditions)
    {
        wrap(heads, expression, &bindings, fuel)?;
    }
    let defaults = std::mem::take(&mut result.defaults);
    for (index, expression) in &defaults {
        let slot = slots
            .get(*index)
            .ok_or("default parameter index is out of range")?;
        if let Some(slot) = slot {
            let mut expression = expression.clone();
            wrap(heads, &mut expression, &bindings, fuel)?;
            result.defaults.push((*slot, expression));
        }
    }
    Ok(result)
}

fn binding_type_view(
    ty: &Term,
    signature: &Term,
    fuel: &mut usize,
    view: &mut impl FnMut(&Term, &mut usize) -> Result<Term, String>,
) -> Result<Term, String> {
    let Term::Function { parameters, .. } = signature.callable_signature() else {
        return Err("staged annotation view requires a callable frame".into());
    };
    view_in_input_scope(ty, parameters, fuel, view)
}

fn view_in_input_scope(
    ty: &Term,
    parameters: &[nir::Parameter],
    fuel: &mut usize,
    view: &mut impl FnMut(&Term, &mut usize) -> Result<Term, String>,
) -> Result<Term, String> {
    if ty
        .substitution_parameters()
        .map_err(|e| e.to_string())?
        .is_empty()
    {
        return view(ty, fuel);
    }
    charge(fuel, parameters.len() + 1)?;
    let scoped = Term::Function {
        parameters: parameters.to_vec(),
        result: Box::new(ty.clone()),
    };
    let Term::Function { result, .. } = view(&scoped, fuel)? else {
        return Err("staged annotation view lost its callable frame".into());
    };
    Ok(*result)
}

struct InputViewError(String);
impl From<nir::TypeError> for InputViewError {
    fn from(error: nir::TypeError) -> Self {
        Self(error.to_string())
    }
}

fn prepare_binding_with_type_view(
    signature: &Term,
    inputs: &[Option<Term>],
    fuel: &mut usize,
    view: &mut impl FnMut(&Term, &mut usize) -> Result<Term, String>,
) -> Result<nir::FunctionInputBinding, String> {
    signature
        .prepare_function_inputs_with_type_view(inputs, |ty, parameters| {
            view_in_input_scope(ty, parameters, fuel, view).map_err(InputViewError)
        })
        .map_err(|error: InputViewError| error.0)
}

fn rebind_requirement(
    requirement: &mut nir::program::TraitRequirement,
    scope: &nir::FunctionInputBinding,
    fuel: &mut usize,
) -> Result<(), String> {
    for ty in std::iter::once(&mut requirement.subject).chain(&mut requirement.interface_arguments)
    {
        charge(fuel, 1)?;
        *ty = scope.rebind(ty).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// Forwarding reads the compile-time input, so it must not silently observe a
// different runtime local after a write or address escape. Until stage-aware
// flow analysis can establish a finer fact, conservatively reject those cases.
/// Resolve closed immutable local producers before specializing calls in an
/// ordinary function that has no erased input frame of its own.
pub fn forward_closed_locals_with_layouts<'layout>(
    heads: impl Into<Layouts<'layout>>,
    function: &mut Function,
    fuel: &mut usize,
) -> Result<(), String> {
    let heads = heads.into();
    substitute_forwarded_inputs(heads, function, &[], fuel)
}

fn substitute_forwarded_inputs<'layout>(
    heads: impl Into<Layouts<'layout>>,
    function: &mut Function,
    bindings: &[(LocalId, Expr)],
    fuel: &mut usize,
) -> Result<(), String> {
    let heads = heads.into();
    let mut unavailable = BTreeSet::new();
    for root in std::iter::once(&function.body)
        .chain(&function.preconditions)
        .chain(&function.postconditions)
        .chain(function.defaults.iter().map(|(_, expr)| expr))
    {
        forwarding_writes(root, &BTreeSet::new(), &mut unavailable, fuel, 0)?;
    }
    let mut completed = std::collections::BTreeMap::new();
    for (binding, value) in bindings {
        constant(heads, value, fuel, 0)?;
        completed.insert(*binding, value.clone());
    }
    for root in std::iter::once(&function.body)
        .chain(&function.preconditions)
        .chain(&function.postconditions)
        .chain(function.defaults.iter().map(|(_, expr)| expr))
    {
        collect_forwarded_locals(heads, root, &mut completed, &unavailable, fuel)?;
    }
    let values = completed
        .iter()
        .map(|(binding, value)| (*binding, value))
        .collect();
    for root in std::iter::once(&mut function.body)
        .chain(&mut function.preconditions)
        .chain(&mut function.postconditions)
        .chain(function.defaults.iter_mut().map(|(_, expr)| expr))
    {
        substitute_forwarded_expression(
            heads,
            root,
            &values,
            &unavailable,
            &BTreeSet::new(),
            false,
            fuel,
            0,
        )?;
    }
    Ok(())
}

// Recover immutable aliases and structural tuple/record projections of completed
// inputs, including total Boolean/structural compositions. Never invoke a
// producer or cross a separate callable/default environment. Writes/escapes
// still invalidate every subsequent substitution.
fn collect_forwarded_locals<'layout>(
    heads: impl Into<Layouts<'layout>>,
    root: &Expr,
    values: &mut std::collections::BTreeMap<LocalId, Expr>,
    unavailable: &BTreeSet<LocalId>,
    fuel: &mut usize,
) -> Result<(), String> {
    let heads = heads.into();
    let mut pending = Vec::new();
    crate::visit::expression_pruned(root, fuel, |expr, _| {
        if matches!(expr.kind, K::Closure { .. } | K::Call { .. }) {
            return Ok(false);
        }
        if matches!(
            expr.kind,
            K::Let { .. }
                | K::LetPattern {
                    guard: None,
                    otherwise: None,
                    ..
                }
        ) {
            pending.push(expr);
        }
        Ok(true)
    })?;
    // Traversal is not source ordered; resolve dependencies to a fixed point.
    while !pending.is_empty() {
        let mut waiting = Vec::new();
        let mut progress = false;
        for declaration in pending {
            charge(fuel, 1)?;
            let initializer = match &declaration.kind {
                K::Let { value, .. } | K::LetPattern { value, .. } => value,
                _ => unreachable!(),
            };
            let mut origin = initializer.as_ref();
            let mut projections = Vec::new();
            while let K::AnonymousField { value, .. } | K::Field { value, .. } = &origin.kind {
                charge(fuel, 1)?;
                projections.push(origin);
                origin = value;
            }
            let value = if let K::Local(input) = origin.kind {
                if unavailable.contains(&input) {
                    continue;
                }
                let Some(mut value) = values.get(&input) else {
                    waiting.push(declaration);
                    continue;
                };
                constant(heads, value, fuel, 0)?;
                for projection in projections.into_iter().rev() {
                    value = staged_record_field(value, projection, fuel)?;
                }
                value.clone()
            } else {
                let Some(dependencies) =
                    structural_boolean_dependencies_in(initializer, heads.target, fuel)?
                else {
                    continue;
                };
                if dependencies.iter().any(|id| unavailable.contains(id)) {
                    continue;
                }
                if dependencies.iter().any(|id| !values.contains_key(id)) {
                    waiting.push(declaration);
                    continue;
                }
                fold_local_input(heads, initializer, values, fuel, 0)?
            };
            match &declaration.kind {
                K::Let { binding, .. } => {
                    values.insert(*binding, value);
                }
                K::LetPattern { pattern, .. } => {
                    let mut patterns = vec![pattern];
                    let mut structural = true;
                    while let Some(pattern) = patterns.pop() {
                        charge(fuel, 1)?;
                        match pattern {
                            nir::program::Pattern::Wild => {}
                            nir::program::Pattern::OptionalPresent(inner)
                            | nir::program::Pattern::ErrorResult { inner, .. }
                            | nir::program::Pattern::Observe { inner, .. }
                            | nir::program::Pattern::Binding { inner, .. } => patterns.push(inner),
                            nir::program::Pattern::Tuple(fields)
                            | nir::program::Pattern::AnonymousRecord { fields, .. }
                            | nir::program::Pattern::Record { fields, .. } => {
                                patterns.extend(fields)
                            }
                            _ => {
                                structural = false;
                                break;
                            }
                        }
                    }
                    if structural {
                        bind_staged_pattern(heads, pattern, &value, values, fuel, 0)?;
                    }
                }
                _ => unreachable!(),
            }
            progress = true;
        }
        if !progress {
            break;
        }
        pending = waiting;
    }
    Ok(())
}

/// Dependencies of the total, non-invoking scalar/BitVec/structural initializer
/// fragment. None excludes calls, writes, pointer reads and checked arithmetic.
/// Availability of every returned binding must be established by the caller.
pub fn structural_boolean_dependencies(
    expression: &Expr,
    fuel: &mut usize,
) -> Result<Option<BTreeSet<LocalId>>, String> {
    structural_boolean_dependencies_in(expression, None, fuel)
}

pub fn structural_boolean_dependencies_in(
    expression: &Expr,
    target: Option<nir::target::TargetData>,
    fuel: &mut usize,
) -> Result<Option<BTreeSet<LocalId>>, String> {
    let layouts = Layouts::new(&[], &[]).with_target(target);
    let mut inputs = BTreeSet::new();
    let mut local_bindings = BTreeSet::new();
    let mut supported = true;
    crate::visit::expression_pruned(expression, fuel, |expr, fuel| {
        match &expr.kind {
            K::Local(binding) => {
                inputs.insert(*binding);
            }
            _ if total_bitvec_operation(expr) => {}
            _ if total_bit_representation_in(expr, target) => {}
            _ if total_float_operation(expr) => {}
            K::Literal(_)
            | K::TypeValue(_)
            | K::Unit
            | K::Tuple(_)
            | K::AnonymousRecord { .. }
            | K::Record { .. }
            | K::Variant { .. }
            | K::AnonymousField { .. }
            | K::Field { .. } => {}
            K::Not(_) | K::And(..) | K::Or(..) if expr.ty == Term::Primitive(Primitive::Bool) => {}
            K::IntegerBinary { op, left, right }
                if op.is_comparison()
                    && expr.ty == Term::Primitive(Primitive::Bool)
                    && left.ty == Term::Primitive(Primitive::Integer)
                    && left.ty == right.ty => {}
            K::ScalarEqual(left, right)
            | K::ScalarLess(left, right)
            | K::ScalarLessEqual(left, right)
            | K::ScalarGreater(left, right)
            | K::ScalarGreaterEqual(left, right)
                if expr.ty == Term::Primitive(Primitive::Bool)
                    && left.ty == right.ty
                    && matches!(left.ty,Term::Primitive(primitive)
                        if layouts.integer_layout(primitive).is_some()
                            || matches!(primitive, Primitive::Char | Primitive::Float | Primitive::F32 | Primitive::F64)
                            || (matches!(expr.kind,K::ScalarEqual(..))
                                && matches!(primitive,Primitive::Bool | Primitive::Str | Primitive::Symbol))) =>
                {}
            K::If { .. } => {}
            K::Match { value, arms } if value.ty == Term::Primitive(Primitive::Bool) => {
                let mut covered = 0;
                for arm in arms {
                    let Some(outcomes) = boolean_pattern(&arm.pattern, fuel)? else {
                        supported &= patterns::bindings(&arm.pattern, &mut local_bindings, fuel)?;
                        continue;
                    };
                    for (index, bindings) in outcomes.into_iter().enumerate() {
                        if let Some(bindings) = bindings {
                            charge(fuel, bindings.len())?;
                            local_bindings.extend(bindings);
                            if arm.guard.is_none() {
                                covered |= 1 << index;
                            }
                        }
                    }
                }
                supported &= covered == 3;
            }
            K::Match { arms, .. } => {
                for arm in arms {
                    supported &= patterns::bindings(&arm.pattern, &mut local_bindings, fuel)?;
                }
            }
            K::Block { .. } | K::Discard(_) => {}
            K::Let { binding, .. } => {
                local_bindings.insert(*binding);
            }
            K::LetPattern {
                pattern,
                guard: None,
                otherwise: None,
                ..
            } => {
                let mut patterns = vec![pattern];
                while let Some(pattern) = patterns.pop() {
                    charge(fuel, 1)?;
                    match pattern {
                        nir::program::Pattern::Wild | nir::program::Pattern::Unit => {}
                        nir::program::Pattern::Binding { binding, inner } => {
                            local_bindings.insert(*binding);
                            patterns.push(inner);
                        }
                        nir::program::Pattern::Tuple(fields)
                        | nir::program::Pattern::AnonymousRecord { fields, .. }
                        | nir::program::Pattern::Record { fields, .. } => patterns.extend(fields),
                        _ => {
                            supported = false;
                            break;
                        }
                    }
                }
            }
            _ => supported = false,
        }
        Ok(supported)
    })?;
    charge(fuel, inputs.len())?;
    inputs.retain(|binding| !local_bindings.contains(binding));
    Ok(supported.then_some(inputs))
}

// Each Boolean input has its own first successful alternative and exported
// bindings. Unioning bindings across alternatives would leak unselected locals.
fn boolean_pattern(
    pattern: &nir::program::Pattern,
    fuel: &mut usize,
) -> Result<Option<[Option<Vec<LocalId>>; 2]>, String> {
    boolean_pattern_at(pattern, fuel, 0)
}

fn boolean_pattern_at(
    pattern: &nir::program::Pattern,
    fuel: &mut usize,
    depth: usize,
) -> Result<Option<[Option<Vec<LocalId>>; 2]>, String> {
    use nir::program::Pattern;
    if depth >= 128 {
        return Err("staged pattern depth exceeded".into());
    }
    charge(fuel, 1)?;
    let mut result = [None, None];
    match pattern {
        Pattern::Wild => result = [Some(Vec::new()), Some(Vec::new())],
        Pattern::Literal(Literal::Bool(value)) => result[usize::from(*value)] = Some(Vec::new()),
        Pattern::Binding { binding, inner } => {
            let Some(mut inner) = boolean_pattern_at(inner, fuel, depth + 1)? else {
                return Ok(None);
            };
            for bindings in inner.iter_mut().flatten() {
                charge(fuel, bindings.len() + 1)?;
                if bindings.contains(binding) {
                    return Err("duplicate staged match binding".into());
                }
                bindings.push(*binding);
            }
            result = inner;
        }
        Pattern::Or(patterns) => {
            for pattern in patterns {
                let Some(alternative) = boolean_pattern_at(pattern, fuel, depth + 1)? else {
                    return Ok(None);
                };
                for (selected, bindings) in result.iter_mut().zip(alternative) {
                    if selected.is_none() {
                        *selected = bindings;
                    }
                }
            }
        }
        Pattern::Remap {
            pattern,
            bindings: remaps,
        } => {
            let mut removed = BTreeSet::new();
            let mut targets = BTreeSet::new();
            for (from, to) in remaps {
                charge(fuel, 1)?;
                if !removed.insert(*from) || !targets.insert(*to) {
                    return Err("duplicate staged match remap source or destination".into());
                }
            }
            let Some(mut inner) = boolean_pattern_at(pattern, fuel, depth + 1)? else {
                return Ok(None);
            };
            for bindings in inner.iter_mut().flatten() {
                // Resolve all sources before removing any: swaps and chains
                // refer to the original environment, just like the interpreter.
                for (from, _) in remaps {
                    charge(fuel, bindings.len() + 1)?;
                    if !bindings.contains(from) {
                        return Err("missing staged match remap binding".into());
                    }
                }
                for target in &targets {
                    charge(fuel, bindings.len() + 1)?;
                    if bindings.contains(target) && !removed.contains(target) {
                        return Err("staged match remap merges bindings".into());
                    }
                }
                charge(fuel, bindings.len())?;
                bindings.retain(|id| !removed.contains(id) || targets.contains(id));
                for target in &targets {
                    charge(fuel, bindings.len() + 1)?;
                    if !bindings.contains(target) {
                        bindings.push(*target);
                    }
                }
            }
            result = inner;
        }
        _ => return Ok(None),
    }
    Ok(Some(result))
}

#[inline(never)]
fn fold_boolean_match<'layout>(
    heads: impl Into<Layouts<'layout>>,
    expr: &Expr,
    fuel: &mut usize,
    depth: usize,
) -> Result<Expr, String> {
    let heads = heads.into();
    if depth >= 128 {
        return Err("staged expression depth exceeded".into());
    }
    charge(fuel, 1)?;
    let K::Match { value, arms } = &expr.kind else {
        unreachable!()
    };
    let value = fold_input(heads, value, fuel, depth + 1)?;
    let Term::Bool(selected) = constant(heads, &value, fuel, depth + 1)? else {
        return Err("staged Boolean match requires a Boolean input".into());
    };
    let selected = usize::from(selected);
    for arm in arms {
        let mut outcomes = boolean_pattern(&arm.pattern, fuel)?
            .ok_or("staged Boolean match pattern is not supported")?;
        let Some(bindings) = outcomes[selected].take() else {
            continue;
        };
        let mut locals = std::collections::BTreeMap::new();
        for binding in bindings {
            charge(fuel, 1)?;
            if locals.insert(binding, value.clone()).is_some() {
                return Err("duplicate staged match binding".into());
            }
        }
        if let Some(guard) = &arm.guard {
            let guard = fold_local_input(heads, guard, &locals, fuel, depth + 1)?;
            match constant(heads, &guard, fuel, depth + 1)? {
                Term::Bool(false) => continue,
                Term::Bool(true) => {}
                _ => return Err("staged match guard is not Boolean".into()),
            }
        }
        let mut result = fold_local_input(heads, &arm.body, &locals, fuel, depth + 1)?;
        if result.ty != expr.ty {
            return Err("staged match result type mismatch".into());
        }
        constant(heads, &result, fuel, depth)?;
        result.source = expr.source;
        return Ok(result);
    }
    Err("staged Boolean match is not exhaustive for its input".into())
}

fn total_float_operation(expr: &Expr) -> bool {
    // Exact binary arithmetic can exceed representable magnitude even when
    // both inputs are valid. Do not promote it as a total initializer.
    supported_float_operation(expr)
        && !(expr.ty == Term::Primitive(Primitive::Float)
            && matches!(expr.kind, K::FloatBinary { .. }))
}

fn supported_float_operation(expr: &Expr) -> bool {
    if expr.ty == Term::Primitive(Primitive::Float) {
        return match &expr.kind {
            K::FloatNegate(value) => value.ty == expr.ty,
            K::FloatBinary { op, left, right } => {
                !matches!(op, nir::program::FloatOp::Divide)
                    && left.ty == expr.ty
                    && right.ty == expr.ty
            }
            _ => false,
        };
    }
    if !matches!(
        expr.ty,
        Term::Primitive(Primitive::F16 | Primitive::Bf16 | Primitive::F32 | Primitive::F64)
    ) {
        return false;
    }
    match &expr.kind {
        K::FloatNegate(value) => value.ty == expr.ty,
        K::FloatBinary { left, right, .. } => left.ty == expr.ty && right.ty == expr.ty,
        _ => false,
    }
}

#[inline(never)]
fn fold_float_operation<'layout>(
    heads: impl Into<Layouts<'layout>>,
    expr: &Expr,
    fuel: &mut usize,
    depth: usize,
) -> Result<Expr, String> {
    let heads = heads.into();
    if depth >= 128 {
        return Err("staged expression depth exceeded".into());
    }
    charge(fuel, 1)?;
    if !supported_float_operation(expr) {
        return Err("staged floating operation domain mismatch".into());
    }
    let (left, right, op) = match &expr.kind {
        K::FloatNegate(value) => (value, None, None),
        K::FloatBinary { op, left, right } => (left, Some(right), Some(op)),
        _ => unreachable!(),
    };
    let left = fold_input(heads, left, fuel, depth + 1)?;
    let left = constant(heads, &left, fuel, depth + 1)?;
    let right = right
        .map(|value| {
            let value = fold_input(heads, value, fuel, depth + 1)?;
            constant(heads, &value, fuel, depth + 1)
        })
        .transpose()?;
    charge(fuel, 1)?;
    let literal = match (left, right, op) {
        (Term::Float(value), None, None) => {
            let mut budget = comptime_runtime::Budget {
                steps: *fuel,
                max_limbs: 4096,
            };
            let result = value
                .negate(&mut budget)
                .and_then(|v| v.to_literal(&mut budget));
            *fuel = budget.steps;
            Literal::Float(result.map_err(|e| e.to_string())?)
        }
        (Term::Float16(value), None, None) => Literal::Float16(value ^ 0x8000),
        (Term::BFloat16(value), None, None) => Literal::BFloat16(value ^ 0x8000),
        (Term::Float32(value), None, None) => Literal::Float32(value ^ (1 << 31)),
        (Term::Float64(value), None, None) => Literal::Float64(value ^ (1 << 63)),
        (Term::Float(left), Some(Term::Float(right)), Some(op)) => {
            let mut budget = comptime_runtime::Budget {
                steps: *fuel,
                max_limbs: 4096,
            };
            let result = match op {
                nir::program::FloatOp::Add => left.add(&right, &mut budget),
                nir::program::FloatOp::Subtract => left.subtract(&right, &mut budget),
                nir::program::FloatOp::Multiply => left.multiply(&right, &mut budget),
                nir::program::FloatOp::Divide => unreachable!("unsupported exact division"),
            }
            .and_then(|v| v.to_literal(&mut budget));
            *fuel = budget.steps;
            Literal::Float(result.map_err(|e| e.to_string())?)
        }
        (Term::Float16(left), Some(Term::Float16(right)), Some(op)) => {
            let mut budget = comptime_runtime::Budget {
                steps: *fuel,
                max_limbs: 4096,
            };
            let result = op.evaluate_half(
                comptime_runtime::float16::Float16Format::Binary16,
                left,
                right,
                &mut budget,
            );
            *fuel = budget.steps;
            Literal::Float16(result.map_err(|e| e.to_string())?)
        }
        (Term::BFloat16(left), Some(Term::BFloat16(right)), Some(op)) => {
            let mut budget = comptime_runtime::Budget {
                steps: *fuel,
                max_limbs: 4096,
            };
            let result = op.evaluate_half(
                comptime_runtime::float16::Float16Format::BFloat16,
                left,
                right,
                &mut budget,
            );
            *fuel = budget.steps;
            Literal::BFloat16(result.map_err(|e| e.to_string())?)
        }
        (Term::Float32(left), Some(Term::Float32(right)), Some(op)) => {
            Literal::Float32(op.evaluate_f32(left, right))
        }
        (Term::Float64(left), Some(Term::Float64(right)), Some(op)) => {
            Literal::Float64(op.evaluate_f64(left, right))
        }
        _ => return Err("staged floating operation requires matching constants".into()),
    };
    let result = Expr {
        source: expr.source,
        ty: expr.ty.clone(),
        kind: K::Literal(literal),
    };
    constant(heads, &result, fuel, depth)?;
    Ok(result)
}

fn total_bitvec_operation(expr: &Expr) -> bool {
    let width = |value: &Expr| match value.ty {
        Term::Primitive(Primitive::BitVec(width)) => Some(width),
        _ => None,
    };
    match &expr.kind {
        K::Not(value) => width(value).is_some() && value.ty == expr.ty,
        K::And(left, right) | K::Or(left, right) => {
            width(left).is_some() && left.ty == right.ty && left.ty == expr.ty
        }
        K::BitEqual(left, right) => {
            width(left).is_some()
                && left.ty == right.ty
                && expr.ty == Term::Primitive(Primitive::Bool)
        }
        K::BitConcat(left, right) => match (width(left), width(right), width(expr)) {
            (Some(left), Some(right), Some(result)) => left.checked_add(right) == Some(result),
            _ => false,
        },
        K::BitSlice { value, start, end } => {
            width(value).is_some_and(|width| start <= end && *end <= width)
                && width(expr) == end.checked_sub(*start)
        }
        _ => false,
    }
}

#[cfg(test)]
fn total_bit_representation(expr: &Expr) -> bool {
    total_bit_representation_in(expr, None)
}

fn total_bit_representation_in(expr: &Expr, target: Option<nir::target::TargetData>) -> bool {
    match &expr.kind {
        K::TakeBits(value) => match (&value.ty, &expr.ty) {
            (Term::Primitive(source), Term::Primitive(Primitive::BitVec(width))) => {
                *source == Primitive::BitVec(*width)
                    || source
                        .readable_scalar_width_in(target)
                        .map(|value| value as usize)
                        == Some(*width)
            }
            _ => false,
        },
        K::BitConvert(value) => match (&value.ty, &expr.ty) {
            (Term::Primitive(Primitive::BitVec(from)), Term::Primitive(to)) => {
                if let Primitive::BitVec(to) = to {
                    return from <= to;
                }
                to.unrestricted_scalar_width_in(target)
                    .is_some_and(|width| {
                        if target
                            .map_or_else(
                                || to.fixed_integer_layout(),
                                |target| target.integer_layout(*to),
                            )
                            .is_some_and(|(signed, _)| !signed)
                        {
                            *from <= width as usize
                        } else {
                            *from == width as usize
                        }
                    })
            }
            _ => false,
        },
        _ => false,
    }
}

#[inline(never)]
fn fold_bit_representation<'layout>(
    heads: impl Into<Layouts<'layout>>,
    expr: &Expr,
    fuel: &mut usize,
    depth: usize,
) -> Result<Expr, String> {
    let heads = heads.into();
    if depth >= 128 {
        return Err("staged expression depth exceeded".into());
    }
    charge(fuel, 1)?;
    if !total_bit_representation_in(expr, heads.target) {
        return Err("staged bit representation domain mismatch".into());
    }
    let (K::TakeBits(operand) | K::BitConvert(operand)) = &expr.kind else {
        unreachable!()
    };
    let value = fold_input(heads, operand, fuel, depth + 1)?;
    let term = constant(heads, &value, fuel, depth + 1)?;
    let mut budget = comptime_runtime::Budget {
        steps: *fuel,
        max_limbs: 4096,
    };
    let result = (|| -> Result<Literal, String> {
        let bits = if matches!(expr.kind, K::TakeBits(_)) {
            let Term::Primitive(Primitive::BitVec(width)) = expr.ty else {
                unreachable!()
            };
            match term {
                Term::BitVec(bits) => bits,
                _ => {
                    let raw = match value.kind {
                        K::Literal(Literal::SignedInteger(value)) => value as u128,
                        K::Literal(Literal::UnsignedInteger(value)) => value,
                        K::Literal(Literal::Float16(value) | Literal::BFloat16(value)) => {
                            u128::from(value)
                        }
                        K::Literal(Literal::Float32(value)) => u128::from(value),
                        K::Literal(Literal::Float64(value)) => u128::from(value),
                        K::Literal(Literal::Char(value)) => u128::from(u32::from(value)),
                        _ => {
                            return Err(
                                "staged bit representation requires a scalar literal".into()
                            );
                        }
                    };
                    let raw = if width == 128 {
                        raw
                    } else {
                        raw & ((1u128 << width) - 1)
                    };
                    comptime_runtime::Integer::from_u128(raw)
                        .to_bitvec(width, &mut budget)
                        .map_err(|error| error.to_string())?
                }
            }
        } else {
            let Term::BitVec(bits) = term else {
                unreachable!()
            };
            match expr.ty {
                Term::Primitive(Primitive::BitVec(width)) => bits
                    .widen(width, &mut budget)
                    .map_err(|error| error.to_string())?,
                Term::Primitive(primitive) => {
                    budget
                        .consume(bits.words().len().saturating_add(1))
                        .map_err(|error| error.to_string())?;
                    let raw = bits
                        .words()
                        .iter()
                        .rev()
                        .fold(0u128, |raw, word| (raw << 32) | u128::from(*word));
                    return Ok(match primitive {
                        Primitive::F16 => Literal::Float16(raw as u16),
                        Primitive::Bf16 => Literal::BFloat16(raw as u16),
                        Primitive::F32 => Literal::Float32(raw as u32),
                        Primitive::F64 => Literal::Float64(raw as u64),
                        _ => {
                            let (signed, width) = heads.integer_layout(primitive).unwrap();
                            if signed {
                                Literal::SignedInteger(
                                    ((raw << (128 - width)) as i128) >> (128 - width),
                                )
                            } else {
                                Literal::UnsignedInteger(raw)
                            }
                        }
                    });
                }
                _ => unreachable!(),
            }
        };
        budget
            .consume(bits.words().len().saturating_add(1))
            .map_err(|error| error.to_string())?;
        Ok(Literal::BitVec {
            width: bits.width(),
            words: bits.words().to_vec(),
        })
    })();
    *fuel = budget.steps;
    let result = Expr {
        source: expr.source,
        ty: expr.ty.clone(),
        kind: K::Literal(result?),
    };
    constant(heads, &result, fuel, depth)?;
    Ok(result)
}

#[inline(never)]
fn folded_bits<'layout>(
    heads: impl Into<Layouts<'layout>>,
    expr: &Expr,
    fuel: &mut usize,
    depth: usize,
) -> Result<comptime_runtime::bitvec::BitVec, String> {
    let heads = heads.into();
    let value = fold_input(heads, expr, fuel, depth)?;
    match constant(heads, &value, fuel, depth)? {
        Term::BitVec(bits) => Ok(bits),
        _ => Err("staged bit operation requires a BitVec value".into()),
    }
}

#[inline(never)]
fn fold_bitvec_operation<'layout>(
    heads: impl Into<Layouts<'layout>>,
    expr: &Expr,
    fuel: &mut usize,
    depth: usize,
) -> Result<Expr, String> {
    let heads = heads.into();
    if depth >= 128 {
        return Err("staged expression depth exceeded".into());
    }
    charge(fuel, 1)?;
    if !total_bitvec_operation(expr) {
        return Err("staged bit operation type or range mismatch".into());
    }
    let (left, right) = match &expr.kind {
        K::Not(value) | K::BitSlice { value, .. } => (value, None),
        K::And(left, right)
        | K::Or(left, right)
        | K::BitEqual(left, right)
        | K::BitConcat(left, right) => (left, Some(right)),
        _ => unreachable!(),
    };
    let left = folded_bits(heads, left, fuel, depth + 1)?;
    // Bitwise and/or evaluate both operands, including when one is all-zero/all-one.
    let right = right
        .map(|right| folded_bits(heads, right, fuel, depth + 1))
        .transpose()?;
    let mut budget = comptime_runtime::Budget {
        steps: *fuel,
        max_limbs: 4096,
    };
    let result = (|| -> Result<Literal, comptime_runtime::bitvec::Error> {
        if matches!(expr.kind, K::BitEqual(..)) {
            budget.consume(left.words().len().saturating_add(1))?;
            return Ok(Literal::Bool(left == right.unwrap()));
        }
        let bits = match &expr.kind {
            K::Not(_) => left.not(&mut budget)?,
            K::And(..) => left.and(&right.unwrap(), &mut budget)?,
            K::Or(..) => left.or(&right.unwrap(), &mut budget)?,
            K::BitConcat(..) => left.concat(&right.unwrap(), &mut budget)?,
            K::BitSlice { start, end, .. } => left.slice(*start, *end, &mut budget)?,
            _ => unreachable!(),
        };
        budget.consume(bits.words().len().saturating_add(1))?;
        Ok(Literal::BitVec {
            width: bits.width(),
            words: bits.words().to_vec(),
        })
    })();
    *fuel = budget.steps;
    let result = Expr {
        source: expr.source,
        ty: expr.ty.clone(),
        kind: K::Literal(result.map_err(|error| error.to_string())?),
    };
    constant(heads, &result, fuel, depth)?;
    Ok(result)
}

// A comptime closure owns captured copies. Writes inside that namespace do
// not invalidate the source binding, but capture initializers run outside it.
fn forwarding_writes(
    expression: &Expr,
    private: &BTreeSet<LocalId>,
    unavailable: &mut BTreeSet<LocalId>,
    fuel: &mut usize,
    depth: usize,
) -> Result<(), String> {
    if depth >= 128 {
        return Err("staged capture scope depth exceeded".into());
    }
    crate::visit::expression_pruned(expression, fuel, |expr, fuel| {
        match &expr.kind {
            K::Closure { captures, body, .. } if expr.ty.is_comptime_callable() => {
                charge(fuel, private.len() + captures.len())?;
                let mut inner = private.clone();
                for capture in captures {
                    if let Some(value) = &capture.value {
                        forwarding_writes(value, private, unavailable, fuel, depth + 1)?;
                    }
                    inner.insert(capture.binding);
                }
                forwarding_writes(body, &inner, unavailable, fuel, depth + 1)?;
                return Ok(false);
            }
            K::Assign { place, .. } | K::BitAssign { place, .. } | K::BitUpdate { place, .. } => {
                if !private.contains(&place.root) {
                    unavailable.insert(place.root);
                }
            }
            K::AddressOf(place) => {
                let mut root = place.as_ref();
                while let K::Field { value, .. } | K::AnonymousField { value, .. } = &root.kind {
                    root = value;
                }
                if let K::Local(binding) = root.kind {
                    if !private.contains(&binding) {
                        unavailable.insert(binding);
                    }
                }
            }
            _ => {}
        }
        Ok(true)
    })
}

// Callee-frame references in defaults must survive substitution of caller
// inputs, including recursive calls whose LocalIds share the same owner.
fn substitute_forwarded_expression<'layout>(
    heads: impl Into<Layouts<'layout>>,
    expression: &mut Expr,
    values: &std::collections::BTreeMap<LocalId, &Expr>,
    unavailable: &BTreeSet<LocalId>,
    masked: &BTreeSet<LocalId>,
    staged: bool,
    fuel: &mut usize,
    depth: usize,
) -> Result<(), String> {
    let heads = heads.into();
    if depth >= 128 {
        return Err("staged forwarding depth exceeded".into());
    }
    crate::visit::expression_mut_pruned(expression, fuel, |expr, fuel| {
        if let Some(plan) = preparation_with_layouts(heads, expr, fuel)? {
            let K::CallPreparation { statements, result } = &mut expr.kind else {
                unreachable!()
            };
            for (index, statement) in statements.iter_mut().enumerate() {
                substitute_forwarded_expression(
                    heads,
                    statement,
                    values,
                    unavailable,
                    masked,
                    staged || plan.indices.contains(&index),
                    fuel,
                    depth + 1,
                )?;
            }
            if let Some(result) = result {
                substitute_forwarded_expression(
                    heads,
                    result,
                    values,
                    unavailable,
                    masked,
                    staged,
                    fuel,
                    depth + 1,
                )?;
            }
            return Ok(false);
        }
        if let K::Closure {
            captures,
            body,
            parameters,
        } = &mut expr.kind
        {
            charge(fuel, masked.len() + captures.len() + parameters.len())?;
            let mut inner = masked.clone();
            inner.extend(parameters.iter().copied());
            for capture in captures {
                if staged {
                    inner.insert(capture.binding);
                }
                if staged && capture.value.is_none() && !masked.contains(&capture.binding) {
                    if let Some(value) = values.get(&capture.binding) {
                        if unavailable.contains(&capture.binding) {
                            return Err("forwarding a staged capture after writes or address escape is not yet supported".into());
                        }
                        if value.ty != capture.ty {
                            return Err("staged capture type mismatch".into());
                        }
                        constant(heads, value, fuel, 0)?;
                        capture.value = Some(Box::new((*value).clone()));
                    }
                }
                if let Some(value) = &mut capture.value {
                    substitute_forwarded_expression(
                        heads,
                        value,
                        values,
                        unavailable,
                        masked,
                        staged,
                        fuel,
                        depth + 1,
                    )?;
                }
            }
            substitute_closure_locals(
                heads,
                std::sync::Arc::make_mut(body),
                values,
                unavailable,
                &inner,
                staged,
                fuel,
                depth + 1,
            )?;
            return Ok(false);
        }
        if let K::Local(binding) = expr.kind {
            if staged && !masked.contains(&binding) {
                if let Some(value) = values.get(&binding) {
                    if unavailable.contains(&binding) {
                        return Err("forwarding a staged input after runtime writes or address escape is not yet supported".into());
                    }
                    if value.ty != expr.ty {
                        return Err("forwarded staged input type mismatch".into());
                    }
                    constant(heads, value, fuel, 0)?;
                    let source = expr.source;
                    *expr = (*value).clone();
                    expr.source = source;
                    return Ok(false);
                }
            }
        }
        if let K::Call {
            callee,
            arguments,
            parameter_bindings,
        } = &mut expr.kind
        {
            let Term::Function { parameters, .. } = callee.ty.callable_signature() else {
                return Ok(true);
            };
            let sorts: Vec<_> = parameters.iter().map(|p| p.sort).collect();
            charge(fuel, sorts.len())?;
            substitute_forwarded_expression(
                heads,
                callee,
                values,
                unavailable,
                masked,
                staged,
                fuel,
                depth + 1,
            )?;
            for argument in arguments {
                let sort = sorts
                    .get(argument.parameter)
                    .ok_or("forwarded staged argument index out of range")?;
                if argument.is_default {
                    charge(fuel, masked.len() + parameter_bindings.len())?;
                    let mut inner = masked.clone();
                    inner.extend(parameter_bindings.iter().copied());
                    substitute_forwarded_expression(
                        heads,
                        &mut argument.value,
                        values,
                        unavailable,
                        &inner,
                        staged || *sort != Sort::Runtime,
                        fuel,
                        depth + 1,
                    )?;
                } else {
                    substitute_forwarded_expression(
                        heads,
                        &mut argument.value,
                        values,
                        unavailable,
                        masked,
                        staged || *sort != Sort::Runtime,
                        fuel,
                        depth + 1,
                    )?;
                }
            }
            return Ok(false);
        }
        Ok(true)
    })
}

/// Route a checked direct call after required invocation and substitution of
/// enclosing staged bindings. Supported effect-free residual expressions fold
/// to canonical data here. Return a keyed instance and a call containing only
/// runtime arguments, still in their original evaluation order.
/// `template` has already received its declaration arguments; these arguments
/// remain the prefix of the resulting instance identity.
pub fn bind_call_with_layouts<'layout>(
    heads: impl Into<Layouts<'layout>>,
    template: &Function,
    declaration_arguments: &[Term],
    call: &Expr,
    fuel: &mut usize,
) -> Result<(crate::FunctionInstance, Expr), String> {
    let heads = heads.into();
    bind_call_with_layouts_and_evaluator(
        heads,
        template,
        declaration_arguments,
        call,
        fuel,
        |expr, fuel| fold_input(heads, expr, fuel, 0),
    )
}

/// Prepare staged inputs once in source/default order. The host evaluator runs
/// only after preceding staged results have been substituted into a default's
/// callee frame. It must return completed immutable data of the checked type.
pub fn bind_call_with_layouts_and_evaluator<'layout>(
    heads: impl Into<Layouts<'layout>>,
    template: &Function,
    declaration_arguments: &[Term],
    call: &Expr,
    fuel: &mut usize,
    mut evaluate: impl FnMut(&Expr, &mut usize) -> Result<Expr, String>,
) -> Result<(crate::FunctionInstance, Expr), String> {
    let heads = heads.into();
    bind_call_with_layouts_and_type_views(
        heads,
        template,
        declaration_arguments,
        call,
        fuel,
        &mut evaluate,
        |evaluate, input, fuel| evaluate(input, fuel),
        |_, ty, _| Ok(ty.clone()),
    )
}

/// Resolve substituted input domains using the same world as staged evaluation.
/// The view normalizes types; it cannot bypass value/type consistency checks.
pub fn bind_call_with_layouts_and_type_views<'layout, C>(
    heads: impl Into<Layouts<'layout>>,
    template: &Function,
    declaration_arguments: &[Term],
    call: &Expr,
    fuel: &mut usize,
    context: &mut C,
    mut evaluate: impl FnMut(&mut C, &Expr, &mut usize) -> Result<Expr, String>,
    mut type_view: impl FnMut(&mut C, &Term, &mut usize) -> Result<Term, String>,
) -> Result<(crate::FunctionInstance, Expr), String> {
    let heads = heads.into();
    charge(fuel, 1)?;
    let K::Call {
        callee,
        arguments,
        parameter_bindings,
    } = &call.kind
    else {
        return Err("staged routing requires a direct call".into());
    };
    let actual = match &callee.kind {
        K::Function(id) if declaration_arguments.is_empty() => (*id, &[][..]),
        K::FunctionInstance {
            declaration,
            arguments,
        } => (*declaration, arguments.as_slice()),
        _ => return Err("staged call target is not a declaration instance".into()),
    };
    if actual.0 != template.declaration || actual.1 != declaration_arguments {
        return Err("staged call target differs from template identity".into());
    }
    let signature_differs = callee.ty != template.callable_type();
    let Term::Function { parameters, .. } = template.signature.callable_signature() else {
        return Err("staged template is not callable".into());
    };
    let Term::Function {
        parameters: entry_parameters,
        ..
    } = callee.ty.callable_signature()
    else {
        return Err("staged call signature differs from template".into());
    };
    if callee.ty.is_comptime_callable() != (template.phase == nir::program::FunctionPhase::Comptime)
        || parameters.len() != entry_parameters.len()
        || parameters
            .iter()
            .zip(entry_parameters)
            .any(|(a, b)| a.sort != b.sort)
    {
        return Err("staged call signature differs from template".into());
    }
    if !parameter_bindings.is_empty() && parameter_bindings != &template.parameters {
        return Err("staged call preparation bindings differ from template".into());
    }
    let mut seen = vec![false; parameters.len()];
    let mut inputs = vec![None; parameters.len()];
    // Canonicalize each completed input once here. Later defaults need the
    // accumulated telescope, not another traversal of every preceding value.
    let mut symbols = vec![None; parameters.len()];
    let mut defaults_started = false;
    let mut last_default = None;
    let mut prepared = std::collections::BTreeMap::new();
    for argument in arguments {
        charge(fuel, 1)?;
        let slot = seen
            .get_mut(argument.parameter)
            .ok_or("staged call argument index out of range")?;
        if *slot {
            return Err("duplicate staged call argument".into());
        }
        *slot = true;
        if argument.is_default {
            if parameter_bindings.is_empty()
                || last_default.is_some_and(|last| last >= argument.parameter)
            {
                return Err("invalid staged call default ordering".into());
            }
            defaults_started = true;
            last_default = Some(argument.parameter);
        } else if defaults_started {
            return Err("explicit staged argument follows a default".into());
        }
    }
    if seen.contains(&false) {
        return Err("missing staged call argument".into());
    }
    if template.parameters.len() != parameters.len()
        || template
            .parameters
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .len()
            != parameters.len()
    {
        return Err("invalid staged template parameter bindings".into());
    }
    // Validate the complete routing plan before executing any user input.
    for argument in arguments {
        charge(fuel, 1)?;
        if parameters[argument.parameter].sort != Sort::Runtime {
            let input = if argument.is_default {
                crate::visit::expression(&argument.value, fuel, |_, _| Ok(()))?;
                let mut input = argument.value.clone();
                let scope = prepare_binding_with_type_view(
                    &template.signature,
                    &symbols,
                    fuel,
                    &mut |ty, fuel| type_view(context, ty, fuel),
                )?;
                input.rewrite_types(&mut |ty| {
                    charge(fuel, 1)?;
                    let ty = scope.rebind(ty).map_err(|e| e.to_string())?;
                    binding_type_view(&ty, scope.signature(), fuel, &mut |ty, fuel| {
                        type_view(context, ty, fuel)
                    })
                })?;
                prepare_local_input(heads, &input, &prepared, fuel, 0)?
            } else {
                crate::visit::expression(&argument.value, fuel, |_, _| Ok(()))?;
                argument.value.clone()
            };
            let value = evaluate(context, &input, fuel)?;
            let symbol = constant(heads, &value, fuel, 0)?;
            if value.ty != input.ty {
                return Err("staged evaluator result type mismatch".into());
            }
            let binding = template
                .parameters
                .get(argument.parameter)
                .ok_or("staged template parameter binding is missing")?;
            prepared.insert(*binding, value.clone());
            symbols[argument.parameter] = Some(symbol);
            inputs[argument.parameter] = Some(value);
        }
    }
    let function = bind_inputs_with_type_view(heads, template, &inputs, fuel, |ty, fuel| {
        type_view(context, ty, fuel)
    })?;
    let mut key_arguments = declaration_arguments.to_vec();
    let mut bindings = Vec::new();
    for (index, input) in inputs.iter().enumerate() {
        if let Some(value) = input {
            key_arguments.push(symbols[index].as_ref().unwrap().clone());
            bindings.push((template.parameters[index], value.clone()));
        }
    }
    let key = crate::InstanceKey {
        declaration: template.declaration,
        arguments: (key_arguments).into(),
    };
    let mut slots = Vec::new();
    let mut next = 0;
    for input in &inputs {
        slots.push(if input.is_none() {
            let index = next;
            next += 1;
            Some(index)
        } else {
            None
        });
    }
    let scope =
        prepare_binding_with_type_view(&template.signature, &symbols, fuel, &mut |ty, fuel| {
            type_view(context, ty, fuel)
        })?;
    if signature_differs {
        // Interface and implementation annotations can differ under their
        // ordinary Type contract. Validate equivalence after binding actual
        // staged inputs, including the domains of erased parameters.
        let entry_scope = prepare_binding_with_type_view(
            callee.ty.callable_signature(),
            &symbols,
            fuel,
            &mut |ty, fuel| type_view(context, ty, fuel),
        )?;
        if entry_scope.signature() != scope.signature() {
            return Err("staged call signature differs from template".into());
        }
        for (expected, actual) in entry_parameters.iter().zip(parameters) {
            let expected = entry_scope
                .rebind(&expected.ty)
                .map_err(|e| e.to_string())?;
            let actual = scope.rebind(&actual.ty).map_err(|e| e.to_string())?;
            let expected =
                binding_type_view(&expected, entry_scope.signature(), fuel, &mut |ty, fuel| {
                    type_view(context, ty, fuel)
                })?;
            let actual = binding_type_view(&actual, scope.signature(), fuel, &mut |ty, fuel| {
                type_view(context, ty, fuel)
            })?;
            if expected != actual {
                return Err("staged call parameter domain differs from template".into());
            }
        }
    }
    let mut runtime_arguments = Vec::new();
    charge(fuel, bindings.len())?;
    let staged_values = bindings
        .iter()
        .map(|(binding, value)| (*binding, value))
        .collect();
    for argument in arguments {
        if let Some(slot) = slots[argument.parameter] {
            crate::visit::expression(&argument.value, fuel, |_, _| Ok(()))?;
            let mut argument = argument.clone();
            argument.parameter = slot;
            if argument.is_default {
                argument.value.rewrite_types(&mut |ty| {
                    charge(fuel, 1)?;
                    scope.rebind(ty).map_err(|e| e.to_string())
                })?;
                // This default is evaluated in the callee's preparation frame.
                // Its nested staged calls need the completed inputs now, before
                // runtime reconstruction hides them behind ordinary let nodes.
                substitute_closure_locals(
                    heads,
                    &mut argument.value,
                    &staged_values,
                    &BTreeSet::new(),
                    &BTreeSet::new(),
                    false,
                    fuel,
                    0,
                )?;
                wrap(heads, &mut argument.value, &bindings, fuel)?;
            }
            runtime_arguments.push(argument);
        }
    }
    let result = Expr {
        ty: call.ty.clone(),
        source: call.source,
        kind: K::Call {
            callee: Box::new(Expr {
                ty: function.callable_type(),
                source: callee.source,
                kind: K::FunctionInstance {
                    declaration: key.declaration,
                    arguments: key.arguments.to_vec(),
                },
            }),
            arguments: runtime_arguments,
            // Specialization has selected a concrete body even when the source
            // was an indirect call. Publish its remaining parameter frame so
            // resource/default analysis can bind each runtime input uniquely.
            parameter_bindings: function.parameters.clone(),
        },
    };
    Ok((crate::FunctionInstance { key, function }, result))
}

fn charge(fuel: &mut usize, work: usize) -> Result<(), String> {
    *fuel = fuel
        .checked_sub(work)
        .ok_or("staged input budget exhausted")?;
    Ok(())
}

fn wrap<'layout>(
    heads: impl Into<Layouts<'layout>>,
    expression: &mut Expr,
    bindings: &[(LocalId, Expr)],
    fuel: &mut usize,
) -> Result<(), String> {
    let heads = heads.into();
    if bindings.is_empty() {
        return Ok(());
    }
    // A computation is an explicit environment boundary, not a first-class
    // block result. Reconstruct staged constants inside that boundary so the
    // residual body retains its ordinary delivered type and effect context.
    let mut expression = expression;
    while matches!(expression.kind, K::EffectComputation { .. }) {
        charge(fuel, 1)?;
        let K::EffectComputation { body, .. } = &mut expression.kind else {
            unreachable!()
        };
        expression = body;
    }
    // Only surviving uses need runtime reconstruction. In particular, a Type
    // or Integer used exclusively to prepare another instance has no runtime
    // storage. A closure's body uses its own capture frame; initialization is
    // the point that may still need an outer binding.
    let mut needed = BTreeSet::new();
    let mut pending = vec![&*expression];
    while let Some(root) = pending.pop() {
        crate::visit::expression_pruned(root, fuel, |expr, fuel| {
            match &expr.kind {
                K::Local(binding) => {
                    needed.insert(*binding);
                }
                K::Assign { place, .. }
                | K::BitAssign { place, .. }
                | K::BitUpdate { place, .. } => {
                    needed.insert(place.root);
                }
                K::Closure { captures, .. } => {
                    charge(fuel, captures.len())?;
                    for capture in captures {
                        if let Some(value) = &capture.value {
                            pending.push(value);
                        } else {
                            needed.insert(capture.binding);
                        }
                    }
                    return Ok(false);
                }
                _ => {}
            }
            Ok(true)
        })?;
    }
    let bindings = bindings
        .iter()
        .filter(|(binding, _)| needed.contains(binding))
        .collect::<Vec<_>>();
    if bindings.is_empty() {
        return Ok(());
    }
    charge(fuel, bindings.len() + 1)?;
    for (_, value) in &bindings {
        constant(heads, value, fuel, 0)?;
    }
    let source = expression.source;
    let mut statements: Vec<_> = bindings
        .iter()
        .map(|(binding, value)| Expr {
            ty: Term::Primitive(Primitive::Void),
            source,
            kind: K::Let {
                binding: *binding,
                value: Box::new(value.clone()),
            },
        })
        .collect();
    // A block already provides the required lexical scope. Nesting another
    // block at the same source boundary creates two indistinguishable cleanup
    // exits. Bind staged locals in that existing scope instead.
    if let K::Block {
        statements: existing,
        ..
    }
    | K::CallPreparation {
        statements: existing,
        ..
    } = &mut expression.kind
    {
        statements.append(existing);
        *existing = statements;
        return Ok(());
    }
    let original = std::mem::replace(&mut expression.kind, K::Unit);
    expression.kind = K::Block {
        statements,
        result: Some(Box::new(Expr {
            ty: expression.ty.clone(),
            source,
            kind: original,
        })),
    };
    Ok(())
}

// Only effect-free residual structure is folded here. General comptime calls
// still belong to required invocation, not to runtime graph materialization.
/// Complete the supported effect-free staged input fragment without invoking user code.
pub fn fold_value_with_layouts<'layout>(
    heads: impl Into<Layouts<'layout>>,
    expr: &Expr,
    fuel: &mut usize,
) -> Result<Expr, String> {
    let heads = heads.into();
    fold_input(heads, expr, fuel, 0)
}

// Both alias discovery and folding use the same identity checks. Nominal
// layouts originate in checked aggregate expressions supplied by the caller.
fn staged_record_field<'a>(
    value: &'a Expr,
    projection: &Expr,
    fuel: &mut usize,
) -> Result<&'a Expr, String> {
    charge(fuel, 1)?;
    let (base, index, fields) = match (&projection.kind, &value.kind, &value.ty) {
        (
            K::AnonymousField {
                value: base,
                name,
                index,
            },
            K::AnonymousRecord { fields },
            Term::Record(types),
        ) => {
            charge(fuel, name.len())?;
            let (expected, ty) = types
                .get(*index)
                .ok_or("staged field index is out of range")?;
            if expected != name || ty != &projection.ty {
                return Err("staged field identity or type mismatch".into());
            }
            (base, index, fields)
        }
        (
            K::Field {
                value: base,
                declaration,
                index,
            },
            K::Record {
                declaration: actual,
                fields,
            },
            Term::Nominal {
                declaration: owner, ..
            },
        ) if declaration == actual && declaration == owner => (base, index, fields),
        _ => return Err("staged field requires the matching record representation".into()),
    };
    if base.ty != value.ty {
        return Err("staged field base type mismatch".into());
    }
    let field = fields.get(*index).ok_or("staged field value is missing")?;
    if field.ty != projection.ty {
        return Err("staged field result type mismatch".into());
    }
    Ok(field)
}

#[inline(never)]
fn fold_record_field<'layout>(
    heads: impl Into<Layouts<'layout>>,
    expr: &Expr,
    fuel: &mut usize,
    depth: usize,
) -> Result<Expr, String> {
    let heads = heads.into();
    // Flatten projection chains before folding their producer. Recursing once
    // per field would exhaust the host stack before the semantic depth limit.
    let mut root = expr;
    let mut path = Vec::new();
    while let K::AnonymousField { value, .. } | K::Field { value, .. } = &root.kind {
        let current_depth = depth.saturating_add(path.len());
        if current_depth >= 128 {
            return Err("staged expression depth exceeded".into());
        }
        charge(fuel, 1)?;
        path.push((root, current_depth));
        root = value;
    }
    let completed = fold_input(heads, root, fuel, depth.saturating_add(path.len()))?;
    let mut selected = &completed;
    for (projection, current_depth) in path.into_iter().rev() {
        let field = staged_record_field(selected, projection, fuel)?;
        constant(heads, field, fuel, current_depth + 1)?;
        selected = field;
    }
    Ok(selected.clone())
}

#[inline(never)]
fn fold_integer_comparison<'layout>(
    heads: impl Into<Layouts<'layout>>,
    expr: &Expr,
    fuel: &mut usize,
    depth: usize,
) -> Result<Expr, String> {
    let heads = heads.into();
    let K::IntegerBinary { op, left, right } = &expr.kind else {
        unreachable!()
    };
    if !op.is_comparison()
        || expr.ty != Term::Primitive(Primitive::Bool)
        || left.ty != Term::Primitive(Primitive::Integer)
        || left.ty != right.ty
    {
        return Err("staged Integer comparison domain mismatch".into());
    }
    let left = fold_input(heads, left, fuel, depth + 1)?;
    let right = fold_input(heads, right, fuel, depth + 1)?;
    let (Term::Integer(left), Term::Integer(right)) = (
        constant(heads, &left, fuel, depth + 1)?,
        constant(heads, &right, fuel, depth + 1)?,
    ) else {
        return Err("staged Integer comparison requires canonical operands".into());
    };
    let mut budget = comptime_runtime::Budget {
        steps: *fuel,
        max_limbs: 4096,
    };
    let result = op.evaluate(&left, &right, &mut budget);
    *fuel = budget.steps;
    let Term::Bool(value) = result.map_err(|e| e.to_string())? else {
        return Err("staged Integer comparison result is not Boolean".into());
    };
    Ok(Expr {
        source: expr.source,
        ty: expr.ty.clone(),
        kind: K::Literal(Literal::Bool(value)),
    })
}

#[inline(never)]
fn fold_aggregate_input<'layout>(
    heads: impl Into<Layouts<'layout>>,
    expr: &Expr,
    fuel: &mut usize,
    depth: usize,
) -> Result<Expr, String> {
    let heads = heads.into();
    if depth >= 128 {
        return Err("staged expression depth exceeded".into());
    }
    charge(fuel, 1)?;
    let fields = match &expr.kind {
        K::Tuple(fields)
        | K::AnonymousRecord { fields }
        | K::Record { fields, .. }
        | K::Variant { fields, .. } => fields,
        _ => unreachable!(),
    };
    let fields = fields
        .iter()
        .map(|field| fold_input(heads, field, fuel, depth + 1))
        .collect::<Result<Vec<_>, _>>()?;
    let kind = match &expr.kind {
        K::Tuple(_) => K::Tuple(fields),
        K::Record { declaration, .. } => K::Record {
            declaration: *declaration,
            fields,
        },
        K::Variant {
            declaration, index, ..
        } => K::Variant {
            declaration: *declaration,
            index: *index,
            fields,
        },
        _ => K::AnonymousRecord { fields },
    };
    let result = Expr {
        source: expr.source,
        ty: expr.ty.clone(),
        kind,
    };
    constant(heads, &result, fuel, depth)?;
    Ok(result)
}

#[inline(never)]
fn fold_scalar_comparison<'layout>(
    heads: impl Into<Layouts<'layout>>,
    expr: &Expr,
    left: &Expr,
    right: &Expr,
    fuel: &mut usize,
    depth: usize,
) -> Result<Expr, String> {
    let heads = heads.into();
    if left.ty != right.ty {
        return Err("staged comparison operand types differ".into());
    }
    let left = fold_input(heads, left, fuel, depth + 1)?;
    let right = fold_input(heads, right, fuel, depth + 1)?;
    let a = constant(heads, &left, fuel, depth + 1)?;
    let b = constant(heads, &right, fuel, depth + 1)?;
    let equality = matches!(expr.kind, K::ScalarEqual(..));
    let ordering = match (&a, &b) {
        (Term::Bool(a), Term::Bool(b)) if equality => Some(a.cmp(b)),
        (Term::String(a), Term::String(b)) | (Term::Symbol(a), Term::Symbol(b)) if equality => {
            Some(a.cmp(b))
        }
        (Term::Char(a), Term::Char(b)) => Some(a.cmp(b)),
        (Term::Float(a), Term::Float(b)) if left.ty == Term::Primitive(Primitive::Float) => {
            let mut budget = comptime_runtime::Budget {
                steps: *fuel,
                max_limbs: 4096,
            };
            let ordering = a.compare(b, &mut budget);
            *fuel = budget.steps;
            Some(ordering.map_err(|e| e.to_string())?)
        }
        (Term::Float16(a), Term::Float16(b)) => {
            comptime_runtime::float16::Float16Format::Binary16.compare(*a, *b)
        }
        (Term::BFloat16(a), Term::BFloat16(b)) => {
            comptime_runtime::float16::Float16Format::BFloat16.compare(*a, *b)
        }
        (Term::Float32(a), Term::Float32(b)) => f32::from_bits(*a).partial_cmp(&f32::from_bits(*b)),
        (Term::Float64(a), Term::Float64(b)) => f64::from_bits(*a).partial_cmp(&f64::from_bits(*b)),
        (Term::Integer(a), Term::Integer(b)) if matches!(left.ty,Term::Primitive(p) if heads.integer_layout(p).is_some()) => {
            Some(a.cmp(b))
        }
        _ => {
            return Err("staged scalar comparison is not supported for these operands".into());
        }
    };
    // NaN is unordered: <= and >= must not turn a missing ordering
    // into true by simply negating the opposite strict comparison.
    let value = match expr.kind {
        K::ScalarEqual(..) => ordering.is_some_and(|value| value.is_eq()),
        K::ScalarLess(..) => ordering.is_some_and(|value| value.is_lt()),
        K::ScalarLessEqual(..) => ordering.is_some_and(|value| !value.is_gt()),
        K::ScalarGreater(..) => ordering.is_some_and(|value| value.is_gt()),
        K::ScalarGreaterEqual(..) => ordering.is_some_and(|value| !value.is_lt()),
        _ => unreachable!(),
    };
    Ok(Expr {
        kind: K::Literal(Literal::Bool(value)),
        ty: expr.ty.clone(),
        source: expr.source,
    })
}

#[inline(never)]
fn fold_type_list<'layout>(
    heads: impl Into<Layouts<'layout>>,
    expr: &Expr,
    fuel: &mut usize,
    depth: usize,
) -> Result<Expr, String> {
    let heads = heads.into();
    let K::TypeList(fields) = &expr.kind else {
        unreachable!()
    };
    if depth >= 128 {
        return Err("staged list depth exceeded".into());
    }
    charge(fuel, fields.len().saturating_add(1))?;
    let element = heads
        .list_element(&expr.ty, fuel)?
        .ok_or("staged list requires a bound list head")?;
    let mut folded = Vec::with_capacity(fields.len());
    for field in fields {
        if field.ty != element {
            return Err("staged list element type mismatch".into());
        }
        folded.push(fold_input(heads, field, fuel, depth + 1)?);
    }
    let result = Expr {
        source: expr.source,
        ty: expr.ty.clone(),
        kind: K::TypeList(folded),
    };
    constant(heads, &result, fuel, depth + 1)?;
    Ok(result)
}

#[inline(never)]
fn fold_type_list_index<'layout>(
    heads: impl Into<Layouts<'layout>>,
    expr: &Expr,
    fuel: &mut usize,
    depth: usize,
) -> Result<Expr, String> {
    let heads = heads.into();
    if depth >= 128 {
        return Err("staged type-list index depth exceeded".into());
    }
    charge(fuel, 1)?;
    let K::TypeListIndex(values, index) = &expr.kind else {
        unreachable!()
    };
    let element = heads
        .list_element(&values.ty, fuel)?
        .ok_or("staged list requires a bound list head")?;
    if expr.ty != element || index.ty != Term::Primitive(Primitive::Integer) {
        return Err("invalid staged list index types".into());
    }
    let values = fold_input(heads, values, fuel, depth + 1)?;
    let mut fields = lists::elements(heads, values, fuel, depth + 1)?;
    let index = fold_input(heads, index, fuel, depth + 1)?;
    let Term::Integer(index) = constant(heads, &index, fuel, depth + 1)? else {
        return Err("staged list index requires an Integer".into());
    };
    let position = index
        .to_u128()
        .ok()
        .and_then(|n| usize::try_from(n).ok())
        .filter(|n| *n < fields.len())
        .ok_or("staged list index is out of bounds")?;
    let mut selected = fields.swap_remove(position);
    selected.source = expr.source;
    Ok(selected)
}

#[inline(never)]
fn fold_newtype_conversion(
    heads: Layouts<'_>,
    expr: &Expr,
    fuel: &mut usize,
    depth: usize,
) -> Result<Expr, String> {
    let K::NewtypeConvert(value) = &expr.kind else {
        unreachable!()
    };
    if depth >= 128 {
        return Err("staged newtype depth exceeded".into());
    }
    charge(fuel, 1)?;
    let value = fold_input(heads, value, fuel, depth + 1)?;
    if heads.representation(&expr.ty, fuel)?.as_ref() == Some(&value.ty) {
        let result = Expr {
            kind: K::NewtypeConvert(Box::new(value)),
            ty: expr.ty.clone(),
            source: expr.source,
        };
        constant(heads, &result, fuel, depth)?;
        return Ok(result);
    }
    if heads.representation(&value.ty, fuel)?.as_ref() == Some(&expr.ty) {
        if let K::NewtypeConvert(mut inner) = value.kind {
            if inner.ty != expr.ty {
                return Err("staged newtype representation type mismatch".into());
            }
            inner.source = expr.source;
            return Ok(*inner);
        }
    }
    Err("staged newtype conversion requires a checked representation".into())
}

// Literal formation owns large temporary values. Keep it out of the recursive
// dispatcher so nested expressions do not reserve that storage at every level.
#[inline(never)]
fn fold_pending_literal(
    heads: Layouts<'_>,
    expr: &Expr,
    fuel: &mut usize,
    depth: usize,
) -> Result<Expr, String> {
    if let K::Literal(Literal::PendingInteger { spelling, negative }) = &expr.kind {
        if depth >= 128 {
            return Err("staged expression depth exceeded".into());
        }
        charge(fuel, 1)?;
        let Term::Primitive(domain) = expr.ty else {
            return Err("staged integer formation requires a concrete integer domain".into());
        };
        let mut budget = comptime_runtime::Budget {
            steps: *fuel,
            max_limbs: *fuel,
        };
        let result =
            nir::integer_formation::form(spelling, *negative, domain, heads.target, &mut budget);
        *fuel = budget.steps;
        return Ok(Expr {
            source: expr.source,
            ty: expr.ty.clone(),
            kind: K::Literal(result.map_err(|cause| {
                nir::integer_formation::FormationError {
                    source: expr.source,
                    target: expr.ty.clone(),
                    layout: heads.target,
                    cause,
                }
                .to_string()
            })?),
        });
    }
    if let K::Literal(Literal::PendingFloat { spelling, negative }) = &expr.kind {
        if depth >= 128 {
            return Err("staged expression depth exceeded".into());
        }
        charge(fuel, 1)?;
        let Term::Primitive(
            domain @ (Primitive::Float
            | Primitive::F16
            | Primitive::Bf16
            | Primitive::F32
            | Primitive::F64),
        ) = expr.ty
        else {
            return Err("staged float formation requires a concrete float domain".into());
        };
        // Folding may precede annotation normalization, but canonical-input
        // validation must continue to reject an unformed literal.
        let mut budget = comptime_runtime::Budget {
            steps: *fuel,
            max_limbs: *fuel,
        };
        let result = nir::float_formation::FloatFormation::parse(spelling, *negative, &mut budget)
            .and_then(|formation| formation.form(domain, &mut budget));
        *fuel = budget.steps;
        return Ok(Expr {
            source: expr.source,
            ty: expr.ty.clone(),
            kind: K::Literal(result.map_err(|cause| {
                nir::float_formation::FormationError {
                    source: expr.source,
                    target: expr.ty.clone(),
                    cause,
                }
                .to_string()
            })?),
        });
    }

    unreachable!("pending literal dispatcher")
}

fn fold_input<'layout>(
    heads: impl Into<Layouts<'layout>>,
    expr: &Expr,
    fuel: &mut usize,
    depth: usize,
) -> Result<Expr, String> {
    let heads = heads.into();
    if let K::ComptimeValue(value) = &expr.kind {
        return frozen::fold(heads, expr, value, fuel, depth);
    }

    if matches!(
        expr.kind,
        K::Literal(Literal::PendingInteger { .. } | Literal::PendingFloat { .. })
    ) {
        return fold_pending_literal(heads, expr, fuel, depth);
    }

    match &expr.kind {
        K::ScalarEqual(left, right)
        | K::ScalarLess(left, right)
        | K::ScalarLessEqual(left, right)
        | K::ScalarGreater(left, right)
        | K::ScalarGreaterEqual(left, right)
            if expr.ty == Term::Primitive(Primitive::Bool) =>
        {
            if depth >= 128 {
                return Err("staged expression depth exceeded".into());
            }
            charge(fuel, 1)?;
            return fold_scalar_comparison(heads, expr, left, right, fuel, depth);
        }
        _ => {}
    }
    if let K::OptionalPresent(value) = &expr.kind {
        if depth >= 128 {
            return Err("staged expression depth exceeded".into());
        }
        charge(fuel, 1)?;
        let folded = Expr {
            source: expr.source,
            ty: expr.ty.clone(),
            kind: K::OptionalPresent(Box::new(fold_input(heads, value, fuel, depth + 1)?)),
        };
        constant(heads, &folded, fuel, depth)?;
        return Ok(folded);
    }
    if matches!(expr.kind, K::ErrorConvert { .. }) {
        return error_values::fold(heads, expr, fuel, depth);
    }
    if matches!(expr.kind, K::NewtypeConvert(_)) {
        return fold_newtype_conversion(heads, expr, fuel, depth);
    }
    if matches!(expr.kind, K::TypeList(_)) {
        return fold_type_list(heads, expr, fuel, depth);
    }
    if matches!(expr.kind, K::ListLength(_) | K::ListConcat(..)) {
        return lists::fold(heads, expr, fuel, depth);
    }
    if matches!(expr.kind, K::TypeListIndex(..)) {
        return fold_type_list_index(heads, expr, fuel, depth);
    }
    if let K::Match { value, arms } = &expr.kind {
        let mut boolean = value.ty == Term::Primitive(Primitive::Bool);
        if boolean {
            for arm in arms {
                if boolean_pattern(&arm.pattern, fuel)?.is_none() {
                    boolean = false;
                    break;
                }
            }
        }
        return if boolean {
            fold_boolean_match(heads, expr, fuel, depth)
        } else {
            patterns::fold(heads, expr, fuel, depth)
        };
    }
    if matches!(expr.kind, K::If { .. })
        || (expr.ty == Term::Primitive(Primitive::Bool)
            && matches!(expr.kind, K::Not(_) | K::And(..) | K::Or(..)))
    {
        fold_boolean_control(heads, expr, fuel, depth)
    } else if matches!(
        expr.kind,
        K::Tuple(_) | K::AnonymousRecord { .. } | K::Record { .. } | K::Variant { .. }
    ) {
        fold_aggregate_input(heads, expr, fuel, depth)
    } else if matches!(expr.kind, K::FloatNegate(_) | K::FloatBinary { .. }) {
        fold_float_operation(heads, expr, fuel, depth)
    } else if matches!(expr.kind, K::TakeBits(_) | K::BitConvert(_)) {
        fold_bit_representation(heads, expr, fuel, depth)
    } else if matches!(
        expr.kind,
        K::Not(_)
            | K::And(..)
            | K::Or(..)
            | K::BitEqual(..)
            | K::BitConcat(..)
            | K::BitSlice { .. }
    ) {
        fold_bitvec_operation(heads, expr, fuel, depth)
    } else {
        fold_general_input(heads, expr, fuel, depth)
    }
}

#[inline(never)]
fn folded_boolean<'layout>(
    heads: impl Into<Layouts<'layout>>,
    expr: &Expr,
    fuel: &mut usize,
    depth: usize,
) -> Result<bool, String> {
    let heads = heads.into();
    let value = fold_input(heads, expr, fuel, depth)?;
    if value.ty != Term::Primitive(Primitive::Bool) {
        return Err("staged condition is not boolean".into());
    }
    match value.kind {
        K::Literal(Literal::Bool(value)) => Ok(value),
        _ => Err("staged condition is not a boolean constant".into()),
    }
}

#[inline(never)]
fn fold_boolean_control<'layout>(
    heads: impl Into<Layouts<'layout>>,
    expr: &Expr,
    fuel: &mut usize,
    depth: usize,
) -> Result<Expr, String> {
    let heads = heads.into();
    if depth >= 128 {
        return Err("staged expression depth exceeded".into());
    }
    charge(fuel, 1)?;
    let mut result = match &expr.kind {
        K::If {
            condition,
            then_branch,
            else_branch,
        } => {
            if folded_boolean(heads, condition, fuel, depth + 1)? {
                fold_input(heads, then_branch, fuel, depth + 1)?
            } else if let Some(branch) = else_branch {
                fold_input(heads, branch, fuel, depth + 1)?
            } else {
                Expr {
                    source: expr.source,
                    ty: Term::Primitive(Primitive::Void),
                    kind: K::Unit,
                }
            }
        }
        _ => {
            let value = match &expr.kind {
                K::Not(value) => !folded_boolean(heads, value, fuel, depth + 1)?,
                K::And(left, right) => {
                    folded_boolean(heads, left, fuel, depth + 1)?
                        && folded_boolean(heads, right, fuel, depth + 1)?
                }
                K::Or(left, right) => {
                    folded_boolean(heads, left, fuel, depth + 1)?
                        || folded_boolean(heads, right, fuel, depth + 1)?
                }
                _ => unreachable!(),
            };
            Expr {
                source: expr.source,
                ty: expr.ty.clone(),
                kind: K::Literal(Literal::Bool(value)),
            }
        }
    };
    if result.ty != expr.ty {
        return Err("staged selected value type differs from expression type".into());
    }
    constant(heads, &result, fuel, depth)?;
    result.source = expr.source;
    Ok(result)
}

// Structural, irrefutable patterns require no protocol invocation. Binding uses
// the already evaluated producer; destructuring never repeats that computation.
fn bind_staged_pattern<'layout>(
    heads: impl Into<Layouts<'layout>>,
    pattern: &nir::program::Pattern,
    value: &Expr,
    locals: &mut std::collections::BTreeMap<LocalId, Expr>,
    fuel: &mut usize,
    depth: usize,
) -> Result<(), String> {
    let heads = heads.into();
    use nir::program::Pattern;
    if depth >= 128 {
        return Err("staged pattern depth exceeded".into());
    }
    charge(fuel, 1)?;
    match pattern {
        Pattern::Wild => Ok(()),
        Pattern::ErrorResult { label, inner } => {
            let payload = error_values::payload(value, label.as_ref(), fuel)?
                .ok_or("staged error binding selected a different branch")?;
            bind_staged_pattern(heads, inner, payload, locals, fuel, depth + 1)
        }
        Pattern::Unit if matches!(value.kind, K::Unit) => Ok(()),
        Pattern::Binding { binding, inner } => {
            bind_staged_pattern(heads, inner, value, locals, fuel, depth + 1)?;
            constant(heads, value, fuel, depth)?;
            if locals.insert(*binding, value.clone()).is_some() {
                return Err("duplicate staged pattern binding".into());
            }
            Ok(())
        }
        Pattern::Tuple(fields) => {
            let K::Tuple(values) = &value.kind else {
                return Err("staged tuple pattern requires a tuple value".into());
            };
            if fields.len() != values.len() {
                return Err("staged tuple pattern arity mismatch".into());
            }
            for (field, value) in fields.iter().zip(values) {
                bind_staged_pattern(heads, field, value, locals, fuel, depth + 1)?;
            }
            Ok(())
        }
        Pattern::Record {
            declaration,
            fields,
        } => {
            let (
                K::Record {
                    declaration: actual,
                    fields: values,
                },
                Term::Nominal {
                    declaration: owner, ..
                },
            ) = (&value.kind, &value.ty)
            else {
                return Err("staged nominal record pattern requires a nominal record".into());
            };
            if declaration != actual || declaration != owner || fields.len() != values.len() {
                return Err("staged nominal record pattern identity or arity mismatch".into());
            }
            for (field, value) in fields.iter().zip(values) {
                bind_staged_pattern(heads, field, value, locals, fuel, depth + 1)?;
            }
            Ok(())
        }
        Pattern::AnonymousRecord { names, fields } => {
            let (K::AnonymousRecord { fields: values }, Term::Record(types)) =
                (&value.kind, &value.ty)
            else {
                return Err("staged record pattern requires a record value".into());
            };
            if names.len() != fields.len()
                || fields.len() != values.len()
                || values.len() != types.len()
            {
                return Err("staged record pattern arity mismatch".into());
            }
            // Checked patterns use declaration order, regardless of the order
            // of labels in source. Require the labels as well as the arity.
            for ((name, field), (item, (expected, ty))) in
                names.iter().zip(fields).zip(values.iter().zip(types))
            {
                charge(fuel, name.len().saturating_add(expected.len()))?;
                if name != expected || item.ty != *ty {
                    return Err("staged record pattern field identity or type mismatch".into());
                }
                bind_staged_pattern(heads, field, item, locals, fuel, depth + 1)?;
            }
            Ok(())
        }
        _ => Err("staged pattern requires unsupported matching or invocation".into()),
    }
}

// Only immutable canonical data enters this local environment. Substitution is
// simultaneous and scoped to a single block; nested blocks receive outer values
// through their checked references before creating their own environment.
fn fold_local_input<'layout>(
    heads: impl Into<Layouts<'layout>>,
    expr: &Expr,
    locals: &std::collections::BTreeMap<LocalId, Expr>,
    fuel: &mut usize,
    depth: usize,
) -> Result<Expr, String> {
    let heads = heads.into();
    let prepared = prepare_local_input(heads, expr, locals, fuel, depth)?;
    fold_input(heads, &prepared, fuel, depth)
}

fn prepare_local_input<'layout>(
    heads: impl Into<Layouts<'layout>>,
    expr: &Expr,
    locals: &std::collections::BTreeMap<LocalId, Expr>,
    fuel: &mut usize,
    depth: usize,
) -> Result<Expr, String> {
    let heads = heads.into();
    crate::visit::expression(expr, fuel, |_, _| Ok(()))?;
    let mut prepared = expr.clone();
    prepare_local_input_in_place(heads, &mut prepared, locals, fuel, depth)?;
    Ok(prepared)
}

/// Substitute completed inputs while preserving closure-owned capture bindings.
pub fn prepare_input_with_layouts<'layout>(
    heads: impl Into<Layouts<'layout>>,
    expr: &Expr,
    locals: &std::collections::BTreeMap<LocalId, Expr>,
    fuel: &mut usize,
) -> Result<Expr, String> {
    let heads = heads.into();
    prepare_local_input(heads, expr, locals, fuel, 0)
}

fn prepare_local_input_in_place<'layout>(
    heads: impl Into<Layouts<'layout>>,
    prepared: &mut Expr,
    locals: &std::collections::BTreeMap<LocalId, Expr>,
    fuel: &mut usize,
    depth: usize,
) -> Result<(), String> {
    let heads = heads.into();
    if depth >= 128 {
        return Err("staged capture scope depth exceeded".into());
    }
    crate::visit::expression_mut_pruned(prepared, fuel, |expr, fuel| {
        if let K::Closure {
            captures,
            body,
            parameters,
        } = &mut expr.kind
        {
            // Values are canonical, so copying this map does not share storage.
            charge(fuel, captures.len() + parameters.len())?;
            for value in locals.values() {
                constant(heads, value, fuel, depth)?;
            }
            let mut inner = locals.clone();
            for parameter in parameters {
                inner.remove(parameter);
            }
            for capture in captures {
                inner.remove(&capture.binding);
                if capture.value.is_none() {
                    if let Some(value) = locals.get(&capture.binding) {
                        if value.ty != capture.ty {
                            return Err("staged capture type mismatch".into());
                        }
                        constant(heads, value, fuel, depth)?;
                        capture.value = Some(Box::new(value.clone()));
                    }
                }
                if let Some(value) = &mut capture.value {
                    prepare_local_input_in_place(heads, value, locals, fuel, depth + 1)?;
                }
            }
            prepare_local_input_in_place(
                heads,
                std::sync::Arc::make_mut(body),
                &inner,
                fuel,
                depth + 1,
            )?;
            return Ok(false);
        }
        if let K::Local(binding) = expr.kind {
            if let Some(value) = locals.get(&binding) {
                if value.ty != expr.ty {
                    return Err("staged block local type mismatch".into());
                }
                constant(heads, value, fuel, depth)?;
                let source = expr.source;
                *expr = value.clone();
                expr.source = source;
                return Ok(false);
            }
        }
        Ok(true)
    })
}

// Closure bodies and call defaults belong to a new invocation frame. Only closed producers
// are discovered here; enclosing values are not seeds for local evaluation.
#[inline(never)]
fn substitute_closure_locals<'layout>(
    heads: impl Into<Layouts<'layout>>,
    body: &mut Expr,
    enclosing: &std::collections::BTreeMap<LocalId, &Expr>,
    unavailable: &BTreeSet<LocalId>,
    masked: &BTreeSet<LocalId>,
    staged: bool,
    fuel: &mut usize,
    depth: usize,
) -> Result<(), String> {
    let heads = heads.into();
    if depth >= 128 {
        return Err("staged forwarding depth exceeded".into());
    }
    let mut local_unavailable = BTreeSet::new();
    forwarding_writes(body, &BTreeSet::new(), &mut local_unavailable, fuel, depth)?;
    let mut completed = std::collections::BTreeMap::new();
    collect_forwarded_locals(heads, body, &mut completed, &local_unavailable, fuel)?;
    charge(
        fuel,
        enclosing
            .len()
            .saturating_add(unavailable.len())
            .saturating_add(completed.len()),
    )?;
    let mut values = enclosing.clone();
    values.extend(completed.iter().map(|(binding, value)| (*binding, value)));
    local_unavailable.extend(unavailable.iter().copied());
    substitute_forwarded_expression(
        heads,
        body,
        &values,
        &local_unavailable,
        masked,
        staged,
        fuel,
        depth,
    )
}

fn fold_machine_input<'layout>(
    heads: impl Into<Layouts<'layout>>,
    expr: &Expr,
    fuel: &mut usize,
    depth: usize,
) -> Result<Expr, String> {
    let heads = heads.into();
    use comptime_runtime::machine::MachineInteger;
    use nir::program::IntegerOp as Op;
    let Term::Primitive(primitive) = expr.ty else {
        return Err("staged arithmetic requires a machine integer result".into());
    };
    let (signed, width) = heads
        .integer_layout(primitive)
        .ok_or("staged arithmetic requires a machine integer result")?;
    let operand = |value: &Expr, fuel: &mut usize| -> Result<MachineInteger, String> {
        if value.ty != expr.ty {
            return Err("staged arithmetic operand type mismatch".into());
        }
        let value = fold_input(heads, value, fuel, depth + 1)?;
        match value.kind {
            K::Literal(Literal::SignedInteger(value)) if signed => {
                MachineInteger::from_signed(value, width as u16)
            }
            K::Literal(Literal::UnsignedInteger(value)) if !signed => {
                MachineInteger::from_unsigned(value, width as u16)
            }
            _ => return Err("staged arithmetic requires an integer constant".into()),
        }
        .map_err(|e| e.to_string())
    };
    let (left, right, op) = match &expr.kind {
        K::IntegerNegate(value) => (operand(value, fuel)?, None, None),
        K::IntegerBinary { op, left, right }
            if matches!(
                op,
                Op::Add | Op::Subtract | Op::Multiply | Op::DivideTrunc | Op::RemainderTrunc
            ) =>
        {
            (operand(left, fuel)?, Some(operand(right, fuel)?), Some(*op))
        }
        _ => return Err("staged machine arithmetic operator is not implemented".into()),
    };
    let mut budget = comptime_runtime::Budget {
        steps: *fuel,
        max_limbs: 4096,
    };
    let value = match op {
        Some(Op::Add) => left.add(right.unwrap(), &mut budget),
        Some(Op::Subtract) => left.subtract(right.unwrap(), &mut budget),
        Some(Op::Multiply) => left.multiply(right.unwrap(), &mut budget),
        Some(Op::DivideTrunc) => left
            .div_rem_trunc(right.unwrap(), &mut budget)
            .map(|(q, _)| q),
        Some(Op::RemainderTrunc) => left
            .div_rem_trunc(right.unwrap(), &mut budget)
            .map(|(_, r)| r),
        None => left.negate(&mut budget),
        _ => unreachable!(),
    };
    *fuel = budget.steps;
    let value = value.map_err(|e| e.to_string())?;
    let literal = if signed {
        Literal::SignedInteger(value.signed_value().unwrap())
    } else {
        Literal::UnsignedInteger(value.unsigned_value().unwrap())
    };
    Ok(Expr {
        ty: expr.ty.clone(),
        source: expr.source,
        kind: K::Literal(literal),
    })
}

#[inline(never)]
fn nominal_constant<'layout>(
    heads: impl Into<Layouts<'layout>>,
    expr: &Expr,
    fields: &[Expr],
    fuel: &mut usize,
    depth: usize,
) -> Result<Term, String> {
    let heads = heads.into();
    charge(fuel, fields.len())?;
    nir::type_computations::charge_term(&expr.ty, fuel).map_err(|error| error.to_string())?;
    Ok(Term::NominalValue {
        ty: Box::new(expr.ty.clone()),
        variant: match &expr.kind {
            K::Variant { index, .. } => Some(
                u32::try_from(*index)
                    .map_err(|_| "enum variant index exceeds frozen representation")?,
            ),
            _ => None,
        },
        fields: fields
            .iter()
            .map(|field| constant(heads, field, fuel, depth + 1))
            .collect::<Result<_, _>>()?,
    })
}

/// Check that a staged input is already canonical, without invoking code or
/// resolving locals. Uses the same representation contract as staged binding.
pub fn canonical_input_with_layouts(
    layouts: Layouts<'_>,
    expr: &Expr,
    fuel: &mut usize,
) -> Result<(), String> {
    constant(layouts, expr, fuel, 0).map(|_| ())
}

// This accepts canonical immutable data and named callable identities, never a call,
// local reference, pointer or expression whose evaluation could repeat effects.
fn constant<'layout>(
    heads: impl Into<Layouts<'layout>>,
    expr: &Expr,
    fuel: &mut usize,
    depth: usize,
) -> Result<Term, String> {
    let heads = heads.into();
    if depth >= 128 {
        return Err("staged constant depth exceeded".into());
    }
    charge(fuel, 1)?;
    Ok(match (&expr.kind, &expr.ty) {
        (K::ComptimeValue(value), _) => {
            let decoded = frozen::fold(heads, expr, value, fuel, depth + 1)?;
            constant(heads, &decoded, fuel, depth + 1)?
        }

        (K::TypeList(fields), ty) => {
            let element = heads
                .list_element(ty, fuel)?
                .ok_or("staged list requires a bound list head")?;
            charge(fuel, fields.len())?;
            nir::type_computations::charge_term(ty, fuel).map_err(|e| e.to_string())?;
            let mut values = Vec::with_capacity(fields.len());
            for field in fields {
                if field.ty != element {
                    return Err("staged list element type mismatch".into());
                }
                values.push(constant(heads, field, fuel, depth + 1)?);
            }
            nir::comptime_values::freeze_admitted_list(ty, values, fuel)
                .map_err(|error| error.to_string())?
        }
        (K::OptionalAbsent, Term::Optional(_)) => {
            nir::type_computations::charge_term(&expr.ty, fuel).map_err(|e| e.to_string())?;
            Term::OptionalValue {
                ty: Box::new(expr.ty.clone()),
                value: None,
            }
        }
        (K::OptionalPresent(value), Term::Optional(payload)) if value.ty == **payload => {
            nir::type_computations::charge_term(&expr.ty, fuel).map_err(|e| e.to_string())?;
            Term::OptionalValue {
                ty: Box::new(expr.ty.clone()),
                value: Some(Box::new(constant(heads, value, fuel, depth + 1)?)),
            }
        }
        (K::Function(declaration), ty)
            if matches!(
                ty,
                Term::Function { .. } | Term::ComptimeFunction(_) | Term::UnsafeFunction(_)
            ) =>
        {
            crate::completed_type(ty, &BTreeSet::new(), fuel)?;
            Term::FunctionValue {
                declaration: *declaration,
                arguments: vec![],
            }
        }
        (
            K::FunctionInstance {
                declaration,
                arguments,
            },
            ty,
        ) if matches!(
            ty,
            Term::Function { .. } | Term::ComptimeFunction(_) | Term::UnsafeFunction(_)
        ) =>
        {
            crate::completed_type(ty, &BTreeSet::new(), fuel)?;
            for argument in arguments {
                nir::type_computations::charge_term(argument, fuel)
                    .map_err(|error| error.to_string())?;
                crate::completed_type(argument, &BTreeSet::new(), fuel)?;
            }
            Term::FunctionValue {
                declaration: *declaration,
                arguments: arguments.clone(),
            }
        }
        (K::ErrorConvert { conversion, value }, ty) => {
            use nir::error_results::Conversion as C;
            charge(fuel, heads.len())?;
            let tuple_heads: BTreeMap<_, _> = heads
                .iter()
                .filter(|head| head.role == nir::normal_forms::Role::Tuple)
                .map(|head| (head.declaration, head))
                .collect();
            nir::error_results::validate_conversion_with_layouts(
                *conversion,
                &value.ty,
                ty,
                fuel,
                &mut |a, b| a == b,
                &|ty| {
                    let Term::NormalForm { head, .. } = ty else {
                        return None;
                    };
                    tuple_heads.get(head)?.tuple_elements(ty)
                },
            )
            .map_err(|error| error.to_string())?;
            let payload = constant(heads, value, fuel, depth + 1)?;
            match conversion {
                C::Success | C::Failure => Term::ErrorValue {
                    ty: Box::new(ty.clone()),
                    label: (*conversion == C::Failure).then(|| Box::new(value.ty.clone())),
                    value: Box::new(payload),
                },
                C::Embed | C::Product => {
                    let plan = nir::error_results::transport_plan_with_layouts(
                        &value.ty,
                        ty,
                        *conversion == C::Product,
                        fuel,
                        &mut |a, b| a == b,
                        &|ty| {
                            let Term::NormalForm { head, .. } = ty else {
                                return None;
                            };
                            tuple_heads.get(head)?.tuple_elements(ty)
                        },
                    )
                    .map_err(|error| error.to_string())?;
                    plan.frozen(&payload, fuel)
                        .map_err(|error| error.to_string())?
                }
            }
        }
        (K::NewtypeConvert(value), ty) => {
            let representation = heads
                .representation(ty, fuel)?
                .ok_or("staged newtype constant requires its declaration layout")?;
            if value.ty != representation {
                return Err("staged newtype constant representation mismatch".into());
            }
            nir::type_computations::charge_term(ty, fuel).map_err(|error| error.to_string())?;
            Term::NominalValue {
                ty: Box::new(ty.clone()),
                variant: None,
                fields: vec![constant(heads, value, fuel, depth + 1)?],
            }
        }
        (K::TypeValue(ty), Term::Primitive(Primitive::Type)) => {
            crate::completed_type(ty, &BTreeSet::new(), fuel)?;
            if ty.sort() != Sort::Type {
                return Err("staged Type input is not a type".into());
            }
            ty.clone()
        }
        (K::Unit, Term::Primitive(Primitive::Void)) => Term::Unit,
        (K::Literal(Literal::Bool(v)), Term::Primitive(Primitive::Bool)) => Term::Bool(*v),
        (K::Literal(Literal::Char(v)), Term::Primitive(Primitive::Char)) => Term::Char(*v),
        (K::Literal(Literal::Float(spelling)), Term::Primitive(Primitive::Float)) => {
            let mut budget = comptime_runtime::Budget {
                steps: *fuel,
                max_limbs: 4096,
            };
            let value =
                comptime_runtime::decimal::DecimalValue::parse_literal(spelling, &mut budget);
            *fuel = budget.steps;
            Term::Float(value.map_err(|e| e.to_string())?)
        }
        (K::Literal(Literal::Float16(bits)), Term::Primitive(Primitive::F16)) => {
            Term::Float16(*bits)
        }
        (K::Literal(Literal::BFloat16(bits)), Term::Primitive(Primitive::Bf16)) => {
            Term::BFloat16(*bits)
        }
        (K::Literal(Literal::Float32(bits)), Term::Primitive(Primitive::F32)) => {
            Term::Float32(*bits)
        }
        (K::Literal(Literal::Float64(bits)), Term::Primitive(Primitive::F64)) => {
            Term::Float64(*bits)
        }
        (K::Literal(Literal::Symbol(v)), Term::Primitive(Primitive::Symbol)) => {
            charge(fuel, v.len())?;
            Term::Symbol(v.clone().into())
        }
        (K::Literal(Literal::String(v)), Term::Primitive(Primitive::Str)) => {
            charge(fuel, v.len())?;
            Term::String(v.clone().into())
        }
        (K::Literal(Literal::UnsignedInteger(v)), Term::Primitive(p)) => {
            let value = comptime_runtime::Integer::from_u128(*v);
            let Some((false, width)) = heads.integer_layout(*p) else {
                return Err("invalid staged unsigned integer type".into());
            };
            value.to_unsigned(width).map_err(|e| e.to_string())?;
            Term::Integer(value)
        }
        (K::Literal(Literal::SignedInteger(v)), Term::Primitive(p)) => {
            let value = comptime_runtime::Integer::from_i128(*v);
            let Some((true, width)) = heads.integer_layout(*p) else {
                return Err("invalid staged signed integer type".into());
            };
            value.to_signed(width).map_err(|e| e.to_string())?;
            Term::Integer(value)
        }
        (K::Literal(Literal::Integer(spelling)), Term::Primitive(Primitive::Integer)) => {
            let mut budget = comptime_runtime::Budget {
                steps: *fuel,
                max_limbs: 4096,
            };
            // Materialized Integer constants use signed hexadecimal spellings;
            // the lexical parser intentionally accepts only unsigned tokens.
            let (negative, digits) = spelling
                .strip_prefix('-')
                .map_or((false, spelling.as_str()), |digits| (true, digits));
            let value =
                comptime_runtime::Integer::parse_literal(digits, &mut budget).and_then(|value| {
                    if negative {
                        value.negate(&mut budget)
                    } else {
                        Ok(value)
                    }
                });
            *fuel = budget.steps;
            Term::Integer(value.map_err(|e| e.to_string())?)
        }
        (
            K::Literal(Literal::BitVec { width, words }),
            Term::Primitive(Primitive::BitVec(expected)),
        ) if width == expected => {
            let mut budget = comptime_runtime::Budget {
                steps: *fuel,
                max_limbs: 4096,
            };
            let value = comptime_runtime::bitvec::BitVec::from_words(*width, words, &mut budget);
            *fuel = budget.steps;
            let value = value.map_err(|e| e.to_string())?;
            if value.words() != words {
                return Err("noncanonical staged bit vector".into());
            }
            Term::BitVec(value)
        }
        (
            K::Record {
                declaration,
                fields,
            },
            Term::Nominal {
                declaration: expected,
                ..
            },
        )
        | (
            K::Variant {
                declaration,
                fields,
                ..
            },
            Term::Nominal {
                declaration: expected,
                ..
            },
        ) if declaration == expected => nominal_constant(heads, expr, fields, fuel, depth)?,
        (K::AnonymousRecord { fields }, Term::Record(types)) if fields.len() == types.len() => {
            let mut names = BTreeSet::new();
            let mut values = Vec::new();
            for (field, (name, ty)) in fields.iter().zip(types) {
                charge(fuel, name.len())?;
                if !names.insert(name) || field.ty != *ty {
                    return Err("staged record field type or label mismatch".into());
                }
                values.push((name.clone(), constant(heads, field, fuel, depth + 1)?));
            }
            Term::RecordValue(values)
        }
        (K::Tuple(values), ty) => {
            let types = match ty {
                Term::Tuple(types) => types.as_slice(),
                Term::NormalForm { head, .. } => {
                    let mut selected = None;
                    for metadata in heads {
                        charge(fuel, 1)?;
                        if metadata.declaration == *head {
                            selected = metadata.tuple_elements(ty);
                            break;
                        }
                    }
                    selected.ok_or("staged tuple requires trusted Tuple metadata")?
                }
                _ => return Err("staged tuple requires a tuple type".into()),
            };
            if values.len() != types.len() {
                return Err("staged tuple arity mismatch".into());
            }
            let mut fields = Vec::new();
            for (value, ty) in values.iter().zip(types) {
                if value.ty != *ty {
                    return Err("staged tuple field type mismatch".into());
                }
                fields.push(constant(heads, value, fuel, depth + 1)?);
            }
            Term::TupleValue(fields)
        }
        _ => return Err("staged input requires a supported canonical constant".into()),
    })
}

// Context-free entrypoints retain support for structural constants only.
pub fn bind_inputs(
    template: &Function,
    inputs: &[Option<Expr>],
    fuel: &mut usize,
) -> Result<Function, String> {
    bind_inputs_with_layouts(&[], template, inputs, fuel)
}

pub fn forward_closed_locals(function: &mut Function, fuel: &mut usize) -> Result<(), String> {
    forward_closed_locals_with_layouts(&[], function, fuel)
}

pub fn bind_call(
    template: &Function,
    declaration_arguments: &[Term],
    call: &Expr,
    fuel: &mut usize,
) -> Result<(crate::FunctionInstance, Expr), String> {
    bind_call_with_layouts(&[], template, declaration_arguments, call, fuel)
}

pub fn bind_call_with(
    template: &Function,
    declaration_arguments: &[Term],
    call: &Expr,
    fuel: &mut usize,
    evaluate: impl FnMut(&Expr, &mut usize) -> Result<Expr, String>,
) -> Result<(crate::FunctionInstance, Expr), String> {
    bind_call_with_layouts_and_evaluator(&[], template, declaration_arguments, call, fuel, evaluate)
}

pub fn bind_call_with_type_views<C>(
    template: &Function,
    declaration_arguments: &[Term],
    call: &Expr,
    fuel: &mut usize,
    context: &mut C,
    evaluate: impl FnMut(&mut C, &Expr, &mut usize) -> Result<Expr, String>,
    type_view: impl FnMut(&mut C, &Term, &mut usize) -> Result<Term, String>,
) -> Result<(crate::FunctionInstance, Expr), String> {
    bind_call_with_layouts_and_type_views(
        &[],
        template,
        declaration_arguments,
        call,
        fuel,
        context,
        evaluate,
        type_view,
    )
}

pub fn fold_value(expr: &Expr, fuel: &mut usize) -> Result<Expr, String> {
    fold_value_with_layouts(&[], expr, fuel)
}

pub fn prepare_input(
    expr: &Expr,
    locals: &std::collections::BTreeMap<LocalId, Expr>,
    fuel: &mut usize,
) -> Result<Expr, String> {
    prepare_input_with_layouts(&[], expr, locals, fuel)
}
#[cfg(test)]
mod tests {
    #[test]
    fn platform_comparison_alias_retains_target_and_availability() {
        use super::*;
        let input = LocalId { owner: 1, index: 0 };
        let alias = LocalId { owner: 1, index: 1 };
        let make = |kind, primitive| Expr {
            kind,
            ty: Term::Primitive(primitive),
            source: nir::program::SourceRange { start: 0, end: 1 },
        };
        let comparison = make(
            K::ScalarLess(
                Box::new(make(K::Local(input), Primitive::Usize)),
                Box::new(make(
                    K::Literal(Literal::UnsignedInteger(9)),
                    Primitive::Usize,
                )),
            ),
            Primitive::Bool,
        );
        let declaration = make(
            K::Let {
                binding: alias,
                value: Box::new(comparison.clone()),
            },
            Primitive::Void,
        );
        for width in [32, 64] {
            let target = Some(nir::target::TargetData::new(width).unwrap());
            let layouts = Layouts::new(&[], &[]).with_target(target);
            assert_eq!(
                structural_boolean_dependencies_in(&comparison, target, &mut 1000).unwrap(),
                Some(BTreeSet::from([input]))
            );
            assert_eq!(
                structural_boolean_dependencies(&comparison, &mut 1000).unwrap(),
                None
            );
            for unavailable in [BTreeSet::new(), BTreeSet::from([input])] {
                let mut values = std::collections::BTreeMap::from([(
                    input,
                    make(K::Literal(Literal::UnsignedInteger(4)), Primitive::Usize),
                )]);
                collect_forwarded_locals(
                    layouts,
                    &declaration,
                    &mut values,
                    &unavailable,
                    &mut 10000,
                )
                .unwrap();
                if unavailable.is_empty() {
                    assert_eq!(values[&alias].kind, K::Literal(Literal::Bool(true)));
                } else {
                    assert!(!values.contains_key(&alias));
                }
            }
        }
    }
    use super::*;

    #[test]
    fn symbol_name_folding_checks_types_and_budget_before_publishing() {
        let symbol = expr(
            K::Literal(Literal::Symbol("ready".into())),
            Primitive::Symbol,
        );
        let name = expr(K::SymbolName(Box::new(symbol.clone())), Primitive::Str);
        assert_eq!(
            fold_input(&[], &name, &mut 100, 0).unwrap().kind,
            K::Literal(Literal::String("ready".into()))
        );
        let forged = expr(K::SymbolName(Box::new(symbol)), Primitive::Bool);
        assert!(fold_input(&[], &forged, &mut 100, 0).is_err());
        let wrong = expr(
            K::SymbolName(Box::new(expr(
                K::Literal(Literal::String("ready".into())),
                Primitive::Str,
            ))),
            Primitive::Str,
        );
        assert!(fold_input(&[], &wrong, &mut 100, 0).is_err());
        assert!(fold_input(&[], &name, &mut 1, 0).is_err());
        assert_eq!(
            fold_input(&[], &name, &mut 100, 0).unwrap().kind,
            K::Literal(Literal::String("ready".into()))
        );
    }
    use nir::program::SourceRange;

    #[test]
    fn bound_tuple_constants_require_metadata_and_preserve_annotations() {
        use nir::normal_forms::{Head, Kind, Parameter, Role};
        let id = nir::DeclarationId {
            package: 3,
            local: 0,
        };
        let mut heads = vec![
            Head {
                collector: None,
                declaration: id,
                role: Role::Tuple,
                requirements: vec![],
                parameters: vec![Parameter {
                    declaration: nir::DeclarationId {
                        package: 3,
                        local: 1,
                    },
                    binding: LocalId { owner: 0, index: 0 },
                    kind: Kind::Value,
                    ty: Term::NormalForm {
                        sort: Sort::Type,
                        head: nir::DeclarationId {
                            package: 3,
                            local: 2,
                        },
                        arguments: vec![Term::Primitive(Primitive::Type)],
                    },
                }],
            },
            Head {
                collector: None,
                declaration: nir::DeclarationId {
                    package: 3,
                    local: 2,
                },
                role: Role::TypeList,
                requirements: vec![],
                parameters: vec![Parameter {
                    declaration: nir::DeclarationId {
                        package: 3,
                        local: 3,
                    },
                    binding: LocalId { owner: 2, index: 0 },
                    kind: Kind::Type,
                    ty: Term::Primitive(Primitive::Type),
                }],
            },
        ];
        let domain = heads[0].parameters[0].ty.clone();
        let tuple = |values: Vec<Expr>| Expr {
            ty: Term::NormalForm {
                head: id,
                sort: Sort::Type,
                arguments: vec![Term::NominalValue {
                    ty: Box::new(domain.clone()),
                    variant: None,
                    fields: values.iter().map(|v| v.ty.clone()).collect(),
                }],
            },
            kind: K::Tuple(values),
            source: SourceRange { start: 0, end: 0 },
        };
        let value = tuple(vec![
            tuple(vec![expr(K::Literal(Literal::Bool(true)), Primitive::Bool)]),
            tuple(vec![]),
        ]);
        let expected = Term::TupleValue(vec![
            Term::TupleValue(vec![Term::Bool(true)]),
            Term::TupleValue(vec![]),
        ]);
        assert_eq!(constant(&heads, &value, &mut 100, 0).unwrap(), expected);
        assert_eq!(
            fold_value_with_layouts(&heads, &value, &mut 100).unwrap(),
            value
        );
        assert!(fold_value(&value, &mut 100).is_err());
        assert!(constant(&heads, &value, &mut 1, 0).is_err());
        assert_eq!(constant(&heads, &value, &mut 100, 0).unwrap(), expected);
        let mut invalid = value.clone();
        let K::Tuple(fields) = &mut invalid.kind else {
            unreachable!()
        };
        fields.pop();
        assert!(
            constant(&heads, &invalid, &mut 100, 0)
                .unwrap_err()
                .contains("arity")
        );
        let K::Tuple(fields) = &mut invalid.kind else {
            unreachable!()
        };
        fields.push(expr(K::Literal(Literal::Bool(false)), Primitive::Bool));
        assert!(
            constant(&heads, &invalid, &mut 100, 0)
                .unwrap_err()
                .contains("field type")
        );
        heads[0].role = Role::Opaque;
        assert!(
            constant(&heads, &value, &mut 100, 0)
                .unwrap_err()
                .contains("trusted Tuple")
        );
        let list = Expr {
            source: value.source,
            ty: heads[0].parameters[0].ty.clone(),
            kind: K::ComptimeValue(Term::NominalValue {
                ty: Box::new(heads[0].parameters[0].ty.clone()),
                variant: None,
                fields: vec![Term::Primitive(Primitive::Bool)],
            }),
        };
        assert_eq!(
            constant(&heads, &list, &mut 100, 0).unwrap(),
            Term::NominalValue {
                ty: Box::new(list.ty.clone()),
                variant: None,
                fields: vec![Term::Primitive(Primitive::Bool)]
            }
        );
        let ordinary = Expr {
            kind: K::TypeList(vec![expr(
                K::TypeValue(Term::Primitive(Primitive::Bool)),
                Primitive::Type,
            )]),
            ..list.clone()
        };
        assert_eq!(
            constant(&heads, &ordinary, &mut 100, 0).unwrap(),
            constant(&heads, &list, &mut 100, 0).unwrap()
        );
        assert!(constant(&[], &ordinary, &mut 100, 0).is_err());
        assert!(constant(&heads, &ordinary, &mut 1, 0).is_err());
        let wrong_element = Expr {
            kind: K::TypeList(vec![expr(K::Literal(Literal::Bool(true)), Primitive::Bool)]),
            ..ordinary.clone()
        };
        assert!(constant(&heads, &wrong_element, &mut 100, 0).is_err());
        assert!(constant(&[], &list, &mut 100, 0).is_err());
        assert!(constant(&heads, &list, &mut 1, 0).is_err());
        let mut wrong = list.clone();
        wrong.kind = K::ComptimeValue(Term::NominalValue {
            ty: Box::new(heads[0].parameters[0].ty.clone()),
            variant: None,
            fields: vec![Term::Bool(true)],
        });
        assert!(constant(&heads, &wrong, &mut 100, 0).is_err());
        wrong = list.clone();
        wrong.ty = Term::Primitive(Primitive::Type);
        assert!(constant(&heads, &wrong, &mut 100, 0).is_err());
        assert_eq!(
            fold_value_with_layouts(&heads, &list, &mut 100).unwrap(),
            ordinary
        );
    }

    fn expr(kind: K, ty: Primitive) -> Expr {
        Expr {
            kind,
            ty: Term::Primitive(ty),
            source: SourceRange { start: 0, end: 0 },
        }
    }

    #[test]
    fn staged_integer_comparisons_keep_full_magnitude_and_sign() {
        use nir::program::IntegerOp as Op;
        for (left, right, expected) in [
            (
                "0x100000000000000000000000000000000",
                "0xffffffffffffffffffffffffffffffff",
                [false, false, true, true, false, true],
            ),
            (
                "0x100000000000000000000000000000000",
                "0",
                [false, false, true, true, false, true],
            ),
            (
                "-0x100000000000000000000000000000000",
                "-1",
                [true, true, false, false, false, true],
            ),
            (
                "0x100000000000000000000000000000000",
                "0x100000000000000000000000000000000",
                [false, true, false, true, true, false],
            ),
        ] {
            for (op, expected) in [
                Op::Less,
                Op::LessEqual,
                Op::Greater,
                Op::GreaterEqual,
                Op::Equal,
                Op::NotEqual,
            ]
            .into_iter()
            .zip(expected)
            {
                let expression = expr(
                    K::IntegerBinary {
                        op,
                        left: Box::new(expr(
                            K::Literal(Literal::Integer(left.into())),
                            Primitive::Integer,
                        )),
                        right: Box::new(expr(
                            K::Literal(Literal::Integer(right.into())),
                            Primitive::Integer,
                        )),
                    },
                    Primitive::Bool,
                );
                assert!(
                    structural_boolean_dependencies(&expression, &mut 1000)
                        .unwrap()
                        .is_some()
                );
                assert!(fold_value(&expression, &mut 1).is_err());
                let mut fuel = 100_000;
                let result = fold_value(&expression, &mut fuel).unwrap();
                assert!(fuel < 100_000);
                assert!(
                    matches!(result.kind,K::Literal(Literal::Bool(actual)) if actual==expected),
                    "{op:?}: {left} {right}"
                );
                let mut wrong = expression;
                let K::IntegerBinary { right, .. } = &mut wrong.kind else {
                    unreachable!()
                };
                **right = expr(K::Literal(Literal::UnsignedInteger(0)), Primitive::U8);
                assert!(
                    structural_boolean_dependencies(&wrong, &mut 1000)
                        .unwrap()
                        .is_none()
                );
                assert!(fold_value(&wrong, &mut 100_000).is_err());
            }
        }
    }

    #[test]
    fn integer_comparison_aliases_require_completed_unmodified_inputs() {
        let left = LocalId { owner: 1, index: 0 };
        let right = LocalId { owner: 1, index: 1 };
        let alias = LocalId { owner: 1, index: 2 };
        let comparison = expr(
            K::IntegerBinary {
                op: nir::program::IntegerOp::Greater,
                left: Box::new(expr(K::Local(left), Primitive::Integer)),
                right: Box::new(expr(K::Local(right), Primitive::Integer)),
            },
            Primitive::Bool,
        );
        assert_eq!(
            structural_boolean_dependencies(&comparison, &mut 1000).unwrap(),
            Some(BTreeSet::from([left, right]))
        );
        let declaration = expr(
            K::Let {
                binding: alias,
                value: Box::new(comparison),
            },
            Primitive::Void,
        );
        let seed = std::collections::BTreeMap::from([
            (
                left,
                expr(
                    K::Literal(Literal::Integer(
                        "0x100000000000000000000000000000000".into(),
                    )),
                    Primitive::Integer,
                ),
            ),
            (
                right,
                expr(K::Literal(Literal::Integer("0".into())), Primitive::Integer),
            ),
        ]);
        for unavailable in [BTreeSet::from([left]), BTreeSet::from([right])] {
            let mut values = seed.clone();
            collect_forwarded_locals(&[], &declaration, &mut values, &unavailable, &mut 10000)
                .unwrap();
            assert!(!values.contains_key(&alias));
        }
        let mut values = seed.clone();
        values.remove(&right);
        collect_forwarded_locals(&[], &declaration, &mut values, &BTreeSet::new(), &mut 10000)
            .unwrap();
        assert!(!values.contains_key(&alias));
        values.insert(right, seed[&right].clone());
        collect_forwarded_locals(&[], &declaration, &mut values, &BTreeSet::new(), &mut 10000)
            .unwrap();
        assert_eq!(values[&alias].kind, K::Literal(Literal::Bool(true)));
    }

    #[test]
    fn boolean_alternatives_keep_selected_bindings_and_simultaneous_remaps() {
        use nir::program::Pattern;
        let a = LocalId { owner: 1, index: 0 };
        let b = LocalId { owner: 1, index: 1 };
        let c = LocalId { owner: 1, index: 2 };
        let bind = |binding, value| Pattern::Binding {
            binding,
            inner: Box::new(Pattern::Literal(Literal::Bool(value))),
        };
        let pattern = Pattern::Or(vec![
            bind(a, true),
            Pattern::Remap {
                pattern: Box::new(bind(b, false)),
                bindings: vec![(b, a)],
            },
            Pattern::Binding {
                binding: c,
                inner: Box::new(Pattern::Wild),
            },
        ]);
        assert_eq!(
            boolean_pattern(&pattern, &mut 1000).unwrap(),
            Some([Some(vec![a]), Some(vec![a])])
        );
        assert!(boolean_pattern(&pattern, &mut 1).is_err());
        let swap = Pattern::Remap {
            pattern: Box::new(Pattern::Binding {
                binding: a,
                inner: Box::new(bind(b, true)),
            }),
            bindings: vec![(a, b), (b, a)],
        };
        let outcomes = boolean_pattern(&swap, &mut 1000).unwrap().unwrap();
        assert!(outcomes[0].is_none());
        assert_eq!(
            outcomes[1]
                .as_ref()
                .unwrap()
                .iter()
                .copied()
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([a, b])
        );
        let missing = Pattern::Remap {
            pattern: Box::new(bind(a, true)),
            bindings: vec![(a, b), (b, c)],
        };
        assert!(
            boolean_pattern(&missing, &mut 1000)
                .unwrap_err()
                .contains("missing")
        );
        for bindings in [vec![(a, c), (a, b)], vec![(a, c), (b, c)], vec![(a, b)]] {
            let invalid = Pattern::Remap {
                pattern: Box::new(Pattern::Binding {
                    binding: a,
                    inner: Box::new(bind(b, true)),
                }),
                bindings,
            };
            assert!(
                boolean_pattern(&invalid, &mut 1000)
                    .unwrap_err()
                    .contains("remap")
            );
        }
        let mut deep = Pattern::Wild;
        for _ in 0..160 {
            deep = Pattern::Or(vec![deep]);
        }
        assert!(
            boolean_pattern(&deep, &mut 10000)
                .unwrap_err()
                .contains("depth")
        );
    }

    #[test]
    fn staged_boolean_matches_select_guards_and_keep_arm_bindings_local() {
        use nir::program::{MatchArm, Pattern};
        let binding = LocalId { owner: 1, index: 0 };
        let yes = expr(K::Literal(Literal::Bool(true)), Primitive::Bool);
        let no = expr(K::Literal(Literal::Bool(false)), Primitive::Bool);
        let local = expr(K::Local(binding), Primitive::Bool);
        let matched = expr(
            K::Match {
                value: Box::new(no.clone()),
                arms: vec![
                    MatchArm {
                        pattern: Pattern::Binding {
                            binding,
                            inner: Box::new(Pattern::Wild),
                        },
                        guard: Some(local.clone()),
                        body: expr(
                            K::Local(LocalId {
                                owner: 1,
                                index: 99,
                            }),
                            Primitive::Bool,
                        ),
                    },
                    MatchArm {
                        pattern: Pattern::Binding {
                            binding,
                            inner: Box::new(Pattern::Wild),
                        },
                        guard: None,
                        body: expr(K::Not(Box::new(local)), Primitive::Bool),
                    },
                ],
            },
            Primitive::Bool,
        );
        assert!(fold_value(&matched, &mut 1).is_err());
        assert_eq!(fold_value(&matched, &mut 10000).unwrap(), yes);
        let partial = expr(
            K::Match {
                value: Box::new(no),
                arms: vec![MatchArm {
                    pattern: Pattern::Literal(Literal::Bool(true)),
                    guard: None,
                    body: yes.clone(),
                }],
            },
            Primitive::Bool,
        );
        assert_eq!(
            structural_boolean_dependencies(&partial, &mut 1000).unwrap(),
            None
        );
        assert!(
            fold_value(&partial, &mut 1000)
                .unwrap_err()
                .contains("not exhaustive")
        );
        let mut exhaustive = partial;
        let K::Match { arms, .. } = &mut exhaustive.kind else {
            unreachable!()
        };
        arms.push(MatchArm {
            pattern: Pattern::Literal(Literal::Bool(false)),
            guard: None,
            body: yes.clone(),
        });
        assert_eq!(
            structural_boolean_dependencies(&exhaustive, &mut 1000).unwrap(),
            Some(BTreeSet::new())
        );
        assert_eq!(fold_value(&exhaustive, &mut 1000).unwrap(), yes);
        let mut deep = yes.clone();
        for _ in 0..160 {
            deep = expr(
                K::Match {
                    value: Box::new(yes.clone()),
                    arms: vec![MatchArm {
                        pattern: Pattern::Wild,
                        guard: None,
                        body: deep,
                    }],
                },
                Primitive::Bool,
            );
        }
        assert!(
            fold_value(&deep, &mut 1_000_000)
                .unwrap_err()
                .contains("depth")
        );
    }

    #[test]
    fn structural_block_dependencies_exclude_only_owned_bindings() {
        let outer = LocalId { owner: 1, index: 0 };
        let inner = LocalId { owner: 1, index: 1 };
        let local = |binding| expr(K::Local(binding), Primitive::Bool);
        let block = expr(
            K::Block {
                statements: vec![expr(
                    K::Let {
                        binding: inner,
                        value: Box::new(expr(K::Not(Box::new(local(outer))), Primitive::Bool)),
                    },
                    Primitive::Void,
                )],
                result: Some(Box::new(local(inner))),
            },
            Primitive::Bool,
        );
        assert_eq!(
            structural_boolean_dependencies(&block, &mut 1000).unwrap(),
            Some(BTreeSet::from([outer]))
        );
        assert!(structural_boolean_dependencies(&block, &mut 0).is_err());
        let values = std::collections::BTreeMap::from([(
            outer,
            expr(K::Literal(Literal::Bool(true)), Primitive::Bool),
        )]);
        assert_eq!(
            fold_local_input(&[], &block, &values, &mut 1000, 0)
                .unwrap()
                .kind,
            K::Literal(Literal::Bool(false))
        );
        let mut invalid = block;
        let K::Block { statements, .. } = &mut invalid.kind else {
            unreachable!()
        };
        // A return changes enclosing control flow and is not a total initializer.
        statements.push(expr(
            K::Return(Some(Box::new(local(outer)))),
            Primitive::Never,
        ));
        assert_eq!(
            structural_boolean_dependencies(&invalid, &mut 1000).unwrap(),
            None
        );
    }

    #[test]
    fn nested_scalar_comparisons_and_blocks_reach_logical_depth_limits() {
        let yes = expr(K::Literal(Literal::Bool(true)), Primitive::Bool);
        for mode in 0..3 {
            for depth in [16, 64, 127, 128, 160] {
                let mut value = yes.clone();
                for _ in 0..depth {
                    value = match mode {
                        0 => expr(
                            K::ScalarEqual(Box::new(value), Box::new(yes.clone())),
                            Primitive::Bool,
                        ),
                        1 => expr(
                            K::Block {
                                statements: vec![],
                                result: Some(Box::new(value)),
                            },
                            Primitive::Bool,
                        ),
                        _ => expr(K::Discard(Box::new(value)), Primitive::Void),
                    };
                }
                let result = fold_value(&value, &mut 1_000_000);
                if depth >= 128 {
                    assert!(result.unwrap_err().contains("depth"), "mode={mode}");
                } else {
                    assert_eq!(
                        result.unwrap().kind,
                        if mode == 2 { K::Unit } else { yes.kind.clone() }
                    );
                }
            }
        }
        assert_eq!(fold_value(&yes, &mut 100).unwrap(), yes);
    }

    #[test]
    fn staged_record_fields_fold_before_canonical_validation() {
        let yes = expr(K::Literal(Literal::Bool(true)), Primitive::Bool);
        let no = expr(K::Not(Box::new(yes.clone())), Primitive::Bool);
        let record = Expr {
            source: yes.source,
            ty: Term::Record(vec![
                ("flag".into(), yes.ty.clone()),
                (
                    "pair".into(),
                    Term::Tuple(vec![yes.ty.clone(), yes.ty.clone()]),
                ),
            ]),
            kind: K::AnonymousRecord {
                fields: vec![
                    no.clone(),
                    Expr {
                        source: yes.source,
                        ty: Term::Tuple(vec![yes.ty.clone(), yes.ty.clone()]),
                        kind: K::Tuple(vec![yes.clone(), no]),
                    },
                ],
            },
        };
        assert!(fold_value(&record, &mut 1).is_err());
        let folded = fold_value(&record, &mut 10000).unwrap();
        assert_eq!(
            constant(&[], &folded, &mut 1000, 0).unwrap(),
            Term::RecordValue(vec![
                ("flag".into(), Term::Bool(false)),
                (
                    "pair".into(),
                    Term::TupleValue(vec![Term::Bool(true), Term::Bool(false)])
                ),
            ])
        );
        for change in 0..3 {
            let mut bad = record.clone();
            let Term::Record(fields) = &mut bad.ty else {
                unreachable!()
            };
            match change {
                0 => fields[1].0 = "flag".into(),
                1 => fields[0].1 = Term::Primitive(Primitive::U8),
                _ => {
                    fields.pop();
                }
            }
            assert!(fold_value(&bad, &mut 10000).is_err());
        }
        let mut missing = record;
        let K::AnonymousRecord { fields } = &mut missing.kind else {
            unreachable!()
        };
        fields[0] = expr(K::Local(LocalId { owner: 1, index: 0 }), Primitive::Bool);
        assert_eq!(
            structural_boolean_dependencies(&missing, &mut 1000).unwrap(),
            Some(BTreeSet::from([LocalId { owner: 1, index: 0 }]))
        );
        assert!(fold_value(&missing, &mut 10000).is_err());
    }

    #[test]
    fn staged_float_arithmetic_preserves_rounding_nan_and_negation_bits() {
        use nir::program::FloatOp as Op;
        for primitive in [Primitive::F32, Primitive::F64] {
            let literal = |number: f64| {
                if primitive == Primitive::F32 {
                    Literal::Float32((number as f32).to_bits())
                } else {
                    Literal::Float64(number.to_bits())
                }
            };
            let value = |number: f64| expr(K::Literal(literal(number)), primitive);
            for (op, expected) in [
                (Op::Add, 8.0),
                (Op::Subtract, 4.0),
                (Op::Multiply, 12.0),
                (Op::Divide, 3.0),
            ] {
                let operation = expr(
                    K::FloatBinary {
                        op,
                        left: Box::new(value(6.0)),
                        right: Box::new(value(2.0)),
                    },
                    primitive,
                );
                assert!(fold_value(&operation, &mut 1).is_err());
                assert_eq!(fold_value(&operation, &mut 1000).unwrap(), value(expected));
            }
            let nan = if primitive == Primitive::F32 {
                Literal::Float32(0x7f800042)
            } else {
                Literal::Float64(0x7ff0000000000042)
            };
            let negative_nan = if primitive == Primitive::F32 {
                Literal::Float32(0xff800042)
            } else {
                Literal::Float64(0xfff0000000000042)
            };
            let nan_value = expr(K::Literal(nan), primitive);
            let negative = expr(K::FloatNegate(Box::new(nan_value.clone())), primitive);
            assert_eq!(
                fold_value(&negative, &mut 1000).unwrap().kind,
                K::Literal(negative_nan)
            );
            let canonical = if primitive == Primitive::F32 {
                Literal::Float32(0x7fc00000)
            } else {
                Literal::Float64(0x7ff8000000000000)
            };
            for op in [Op::Add, Op::Subtract, Op::Multiply, Op::Divide] {
                let operation = expr(
                    K::FloatBinary {
                        op,
                        left: Box::new(nan_value.clone()),
                        right: Box::new(value(1.0)),
                    },
                    primitive,
                );
                assert_eq!(
                    fold_value(&operation, &mut 1000).unwrap().kind,
                    K::Literal(canonical.clone())
                );
            }
            let addition = expr(
                K::FloatBinary {
                    op: Op::Add,
                    left: Box::new(value(16777216.0)),
                    right: Box::new(value(1.0)),
                },
                primitive,
            );
            let difference = expr(
                K::FloatBinary {
                    op: Op::Subtract,
                    left: Box::new(addition),
                    right: Box::new(value(16777216.0)),
                },
                primitive,
            );
            assert_eq!(
                fold_value(&difference, &mut 1000).unwrap(),
                value(if primitive == Primitive::F32 {
                    0.0
                } else {
                    1.0
                })
            );
            let mut deep = value(0.0);
            for _ in 0..160 {
                deep = expr(K::FloatNegate(Box::new(deep)), primitive);
            }
            assert!(fold_value(&deep, &mut 10000).unwrap_err().contains("depth"));
            let wrong = expr(K::FloatNegate(Box::new(value(0.0))), Primitive::Bool);
            assert!(!total_float_operation(&wrong));
            assert!(fold_value(&wrong, &mut 1000).is_err());
            let unavailable = expr(K::Local(LocalId { owner: 1, index: 0 }), primitive);
            let product = expr(
                K::FloatBinary {
                    op: Op::Multiply,
                    left: Box::new(value(0.0)),
                    right: Box::new(unavailable),
                },
                primitive,
            );
            assert!(fold_value(&product, &mut 1000).is_err());
            let other = if primitive == Primitive::F32 {
                Primitive::F64
            } else {
                Primitive::F32
            };
            let mismatched = expr(K::FloatNegate(Box::new(value(0.0))), other);
            assert!(!total_float_operation(&mismatched));
            assert!(fold_value(&mismatched, &mut 1000).is_err());
        }
    }

    #[test]
    fn staged_platform_bit_representations_require_matching_target() {
        for width in [32, 64] {
            let layouts = Layouts::new(&[], &[])
                .with_target(Some(nir::target::TargetData::new(width).unwrap()));
            let value = expr(K::Literal(Literal::SignedInteger(-1)), Primitive::Isize);
            let bits = expr(
                K::TakeBits(Box::new(value.clone())),
                Primitive::BitVec(width as usize),
            );
            let converted = expr(K::BitConvert(Box::new(bits.clone())), Primitive::Isize);
            assert!(fold_value(&converted, &mut 10000).is_err());
            assert!(fold_value_with_layouts(layouts, &converted, &mut 1).is_err());
            assert_eq!(
                fold_value_with_layouts(layouts, &converted, &mut 10000).unwrap(),
                value
            );
            assert!(
                structural_boolean_dependencies_in(&bits, layouts.target, &mut 1000)
                    .unwrap()
                    .is_some()
            );
            let wrong = Layouts::new(&[], &[]).with_target(Some(
                nir::target::TargetData::new(if width == 32 { 64 } else { 32 }).unwrap(),
            ));
            assert!(fold_value_with_layouts(wrong, &converted, &mut 10000).is_err());
        }
    }

    #[test]
    fn staged_bit_representations_preserve_payloads_and_reject_invalid_domains() {
        for (literal, primitive, width, words) in [
            (Literal::Float16(0x7c01), Primitive::F16, 16, vec![0x7c01]),
            (Literal::Float16(0x8000), Primitive::F16, 16, vec![0x8000]),
            (Literal::BFloat16(0xffc2), Primitive::Bf16, 16, vec![0xffc2]),
            (Literal::BFloat16(0x8000), Primitive::Bf16, 16, vec![0x8000]),
            (Literal::SignedInteger(-1), Primitive::I8, 8, vec![255]),
            (
                Literal::SignedInteger(i128::MIN),
                Primitive::I128,
                128,
                vec![0, 0, 0, 0x80000000],
            ),
            (
                Literal::UnsignedInteger(u128::MAX),
                Primitive::U128,
                128,
                vec![u32::MAX; 4],
            ),
            (
                Literal::Float32(0x7f800001),
                Primitive::F32,
                32,
                vec![0x7f800001],
            ),
            (
                Literal::Float32(0x80000000),
                Primitive::F32,
                32,
                vec![0x80000000],
            ),
            (
                Literal::Float64(0xfff8000000000042),
                Primitive::F64,
                64,
                vec![0x42, 0xfff80000],
            ),
        ] {
            let value = expr(K::Literal(literal), primitive);
            let bits = expr(
                K::TakeBits(Box::new(value.clone())),
                Primitive::BitVec(width),
            );
            let folded = fold_value(&bits, &mut 10000).unwrap();
            assert_eq!(folded.kind, K::Literal(Literal::BitVec { width, words }));
            let converted = expr(K::BitConvert(Box::new(bits)), primitive);
            assert!(fold_value(&converted, &mut 1).is_err());
            assert_eq!(fold_value(&converted, &mut 10000).unwrap(), value);
            let widened = expr(
                K::BitConvert(Box::new(folded.clone())),
                Primitive::BitVec(129),
            );
            let result = fold_value(&widened, &mut 10000).unwrap();
            let K::Literal(Literal::BitVec {
                width: actual,
                words,
            }) = result.kind
            else {
                panic!("not bits")
            };
            assert_eq!(actual, 129);
            assert_eq!(words.len(), 5);
            assert_eq!(words[4], 0);
            let K::Literal(Literal::BitVec {
                words: original, ..
            }) = &folded.kind
            else {
                unreachable!()
            };
            assert_eq!(&words[..original.len()], original);
            let bad = expr(
                K::BitConvert(Box::new(folded)),
                Primitive::BitVec(width - 1),
            );
            assert!(!total_bit_representation(&bad));
            assert!(fold_value(&bad, &mut 10000).is_err());
        }
        let empty = expr(
            K::Literal(Literal::BitVec {
                width: 0,
                words: vec![],
            }),
            Primitive::BitVec(0),
        );
        let mut deep = empty.clone();
        for _ in 0..160 {
            deep = expr(K::TakeBits(Box::new(deep)), Primitive::BitVec(0));
        }
        assert!(
            fold_value(&deep, &mut 100000)
                .unwrap_err()
                .contains("depth")
        );
        let zero = expr(K::BitConvert(Box::new(empty.clone())), Primitive::U8);
        assert_eq!(
            fold_value(&zero, &mut 1000).unwrap().kind,
            K::Literal(Literal::UnsignedInteger(0))
        );
        for target in [
            Primitive::I8,
            Primitive::F32,
            Primitive::Bool,
            Primitive::Char,
        ] {
            let bad = expr(K::BitConvert(Box::new(empty.clone())), target);
            assert!(!total_bit_representation(&bad));
            assert!(fold_value(&bad, &mut 1000).is_err());
        }
        let boolean = expr(
            K::TakeBits(Box::new(expr(
                K::Literal(Literal::Bool(true)),
                Primitive::Bool,
            ))),
            Primitive::BitVec(1),
        );
        assert!(!total_bit_representation(&boolean));
        assert!(fold_value(&boolean, &mut 1000).is_err());
    }

    #[test]
    fn staged_character_bits_are_readable_but_not_constructible() {
        for value in ['\0', '\u{d7ff}', '\u{e000}', '🦀', '\u{10ffff}'] {
            let read = expr(
                K::TakeBits(Box::new(expr(
                    K::Literal(Literal::Char(value)),
                    Primitive::Char,
                ))),
                Primitive::BitVec(32),
            );
            let folded = fold_value(&read, &mut 1000).unwrap();
            assert_eq!(
                folded.kind,
                K::Literal(Literal::BitVec {
                    width: 32,
                    words: vec![u32::from(value)]
                })
            );
            let reconstruct = expr(K::BitConvert(Box::new(folded)), Primitive::Char);
            assert!(!total_bit_representation(&reconstruct));
            assert!(fold_value(&reconstruct, &mut 1000).is_err());
        }
    }

    #[test]
    fn staged_bit_operations_validate_width_range_budget_and_eager_operands() {
        for width in [0usize, 1, 33, 129] {
            let ty = Primitive::BitVec(width);
            let zero = expr(
                K::Literal(Literal::BitVec {
                    width,
                    words: vec![0; width.div_ceil(32)],
                }),
                ty,
            );
            let inverse = expr(K::Not(Box::new(zero.clone())), ty);
            let mut deep = zero.clone();
            for _ in 0..160 {
                deep = expr(K::Not(Box::new(deep)), ty);
            }
            assert!(
                fold_value(&deep, &mut 100000)
                    .unwrap_err()
                    .contains("depth")
            );
            let folded = fold_value(&inverse, &mut 10000).unwrap();
            let K::Literal(Literal::BitVec { words, .. }) = &folded.kind else {
                panic!("not a bit constant")
            };
            let expected = (0..width.div_ceil(32))
                .map(|index| {
                    let remaining = width - index * 32;
                    if remaining >= 32 {
                        u32::MAX
                    } else {
                        (1u32 << remaining) - 1
                    }
                })
                .collect::<Vec<_>>();
            assert_eq!(words, &expected);
            let joined = expr(
                K::BitConcat(Box::new(inverse.clone()), Box::new(zero.clone())),
                Primitive::BitVec(width * 2),
            );
            for (start, end, expected) in [(0, width, zero.clone()), (width, width * 2, folded)] {
                let slice = expr(
                    K::BitSlice {
                        value: Box::new(joined.clone()),
                        start,
                        end,
                    },
                    ty,
                );
                assert!(total_bitvec_operation(&slice));
                assert!(fold_value(&slice, &mut 1).is_err());
                assert_eq!(fold_value(&slice, &mut 10000).unwrap(), expected);
            }
            let bad = expr(
                K::BitSlice {
                    value: Box::new(zero.clone()),
                    start: width + 1,
                    end: width + 1,
                },
                Primitive::BitVec(0),
            );
            assert!(!total_bitvec_operation(&bad));
            assert!(fold_value(&bad, &mut 10000).is_err());
            let wrong = expr(
                K::BitEqual(
                    Box::new(zero.clone()),
                    Box::new(expr(K::Unit, Primitive::Void)),
                ),
                Primitive::Bool,
            );
            assert!(!total_bitvec_operation(&wrong));
            assert!(fold_value(&wrong, &mut 10000).is_err());
            let unknown = expr(K::Local(LocalId { owner: 1, index: 0 }), ty);
            for kind in [
                K::And(Box::new(zero.clone()), Box::new(unknown.clone())),
                K::Or(Box::new(inverse), Box::new(unknown)),
            ] {
                assert!(fold_value(&expr(kind, ty), &mut 10000).is_err());
            }
        }
    }

    #[test]
    fn staged_exact_float_arithmetic_preserves_precision_and_rejects_unbounded_alignment() {
        use nir::program::FloatOp;
        let literal = |text: &str| expr(K::Literal(Literal::Float(text.into())), Primitive::Float);
        for (op, left, right, expected) in [
            (FloatOp::Add, "0.1", "0.2", "0.3"),
            (
                FloatOp::Subtract,
                "1.0000000000000000000000000000001",
                "1.0",
                "0.0000000000000000000000000000001",
            ),
            (
                FloatOp::Multiply,
                "1.0000000000000000000000000000001",
                "10.0",
                "10.000000000000000000000000000001",
            ),
        ] {
            let expression = expr(
                K::FloatBinary {
                    op,
                    left: Box::new(literal(left)),
                    right: Box::new(literal(right)),
                },
                Primitive::Float,
            );
            assert!(
                structural_boolean_dependencies(&expression, &mut 1000)
                    .unwrap()
                    .is_none()
            );
            assert!(fold_value(&expression, &mut 0).is_err());
            let folded = fold_value(&expression, &mut 100_000).unwrap();
            assert_eq!(
                constant(&[], &folded, &mut 100_000, 0).unwrap(),
                Term::Float(
                    comptime_runtime::decimal::DecimalValue::parse_literal(
                        expected,
                        &mut comptime_runtime::Budget::default()
                    )
                    .unwrap()
                )
            );
        }
        for op in [FloatOp::Add, FloatOp::Divide] {
            let expression = expr(
                K::FloatBinary {
                    op,
                    left: Box::new(literal("1.0e9999999999999999999999999999999999999999")),
                    right: Box::new(literal("1.0")),
                },
                Primitive::Float,
            );
            assert!(fold_value(&expression, &mut 100_000).is_err());
        }
        let failing = expr(
            K::FloatBinary {
                op: FloatOp::Add,
                left: Box::new(literal("1.0e9999999999999999999999999999999999999999")),
                right: Box::new(literal("1.0")),
            },
            Primitive::Float,
        );
        let zero_product = expr(
            K::FloatBinary {
                op: FloatOp::Multiply,
                left: Box::new(literal("0.0")),
                right: Box::new(failing),
            },
            Primitive::Float,
        );
        assert!(fold_value(&zero_product, &mut 100_000).is_err());
        let malformed = expr(
            K::FloatBinary {
                op: FloatOp::Add,
                left: Box::new(literal("1.0")),
                right: Box::new(expr(
                    K::Literal(Literal::Float64(1.0f64.to_bits())),
                    Primitive::F64,
                )),
            },
            Primitive::Float,
        );
        assert!(fold_value(&malformed, &mut 100_000).is_err());
    }

    #[test]
    fn staged_exact_float_negation_checks_domains_precision_and_budget() {
        for (positive, negative) in [
            ("0.0", "-0.0"),
            (
                "1.0000000000000000000000000000001",
                "-1.0000000000000000000000000000001",
            ),
            (
                "1.0e9999999999999999999999999999999999999999",
                "-1.0e9999999999999999999999999999999999999999",
            ),
        ] {
            for (input, expected) in [(positive, negative), (negative, positive)] {
                let value = expr(K::Literal(Literal::Float(input.into())), Primitive::Float);
                let negative = expr(K::FloatNegate(Box::new(value.clone())), Primitive::Float);
                assert!(fold_value(&negative, &mut 0).is_err());
                let actual = fold_value(&negative, &mut 100_000).unwrap();
                let term = constant(&[], &actual, &mut 100_000, 0).unwrap();
                let expected = comptime_runtime::decimal::DecimalValue::parse_literal(
                    expected,
                    &mut comptime_runtime::Budget::default(),
                )
                .unwrap();
                assert_eq!(term, Term::Float(expected));
                let wrong = expr(K::FloatNegate(Box::new(value)), Primitive::F64);
                assert!(fold_value(&wrong, &mut 100_000).is_err());
            }
        }
    }

    #[test]
    fn staged_exact_float_comparisons_preserve_decimal_order_and_zero_equality() {
        let cases = [
            ("-1.0e9999999999999999999999999999999999999999", -3),
            ("-0.000000000000000000000001", -2),
            ("-0.0", 0),
            ("0.0", 0),
            ("1.0000000000000000000000000000001", 1),
            ("10.000000000000000000000000000001e-1", 1),
            ("1.0000000000000000000000000000002", 2),
            ("1.0e9999999999999999999999999999999999999999", 3),
        ];
        for (a, rank_a) in cases {
            for (b, rank_b) in cases {
                for (index, expected) in [
                    rank_a == rank_b,
                    rank_a < rank_b,
                    rank_a <= rank_b,
                    rank_a > rank_b,
                    rank_a >= rank_b,
                ]
                .into_iter()
                .enumerate()
                {
                    let left =
                        Box::new(expr(K::Literal(Literal::Float(a.into())), Primitive::Float));
                    let right =
                        Box::new(expr(K::Literal(Literal::Float(b.into())), Primitive::Float));
                    let kind = match index {
                        0 => K::ScalarEqual(left, right),
                        1 => K::ScalarLess(left, right),
                        2 => K::ScalarLessEqual(left, right),
                        3 => K::ScalarGreater(left, right),
                        _ => K::ScalarGreaterEqual(left, right),
                    };
                    let comparison = expr(kind, Primitive::Bool);
                    assert!(fold_value(&comparison, &mut 0).is_err());
                    let folded = fold_value(&comparison, &mut 100_000).unwrap();
                    assert_eq!(
                        folded.kind,
                        K::Literal(Literal::Bool(expected)),
                        "{a} {b} op={index}"
                    );
                }
            }
        }
        let malformed = expr(
            K::ScalarEqual(
                Box::new(expr(
                    K::Literal(Literal::Float("1.0".into())),
                    Primitive::F64,
                )),
                Box::new(expr(
                    K::Literal(Literal::Float("1.0".into())),
                    Primitive::F64,
                )),
            ),
            Primitive::Bool,
        );
        assert!(fold_value(&malformed, &mut 1000).is_err());
    }

    #[test]
    fn staged_float_comparisons_preserve_unordered_and_signed_zero() {
        // Independent expected ranks: NaNs are unordered, both zeros rank alike.
        for primitive in [Primitive::F32, Primitive::F64] {
            let cases: Vec<(u64, Option<i8>)> = if primitive == Primitive::F32 {
                vec![
                    (0x7fc00001, None),
                    (0xffc00002, None),
                    (0x7f800001, None),
                    (0xff800000, Some(-2)),
                    (0x80000001, Some(-1)),
                    (0x80000000, Some(0)),
                    (0, Some(0)),
                    (1, Some(1)),
                    (0x7f800000, Some(2)),
                ]
            } else {
                vec![
                    (0x7ff8000000000001, None),
                    (0xfff8000000000002, None),
                    (0x7ff0000000000001, None),
                    (0xfff0000000000000, Some(-2)),
                    (0x8000000000000001, Some(-1)),
                    (0x8000000000000000, Some(0)),
                    (0, Some(0)),
                    (1, Some(1)),
                    (0x7ff0000000000000, Some(2)),
                ]
            };
            let literal = |bits| {
                expr(
                    K::Literal(if primitive == Primitive::F32 {
                        Literal::Float32(bits as u32)
                    } else {
                        Literal::Float64(bits)
                    }),
                    primitive,
                )
            };
            for &(a, rank_a) in &cases {
                for &(b, rank_b) in &cases {
                    let expected = match (rank_a, rank_b) {
                        (Some(a), Some(b)) => [a == b, a < b, a <= b, a > b, a >= b],
                        _ => [false; 5],
                    };
                    for (index, expected) in expected.into_iter().enumerate() {
                        let left = Box::new(literal(a));
                        let right = Box::new(literal(b));
                        let kind = match index {
                            0 => K::ScalarEqual(left, right),
                            1 => K::ScalarLess(left, right),
                            2 => K::ScalarLessEqual(left, right),
                            3 => K::ScalarGreater(left, right),
                            _ => K::ScalarGreaterEqual(left, right),
                        };
                        let comparison = expr(kind, Primitive::Bool);
                        assert!(fold_value(&comparison, &mut 0).is_err());
                        let folded = fold_value(&comparison, &mut 1000).unwrap();
                        assert_eq!(
                            folded.kind,
                            K::Literal(Literal::Bool(expected)),
                            "{primitive:?} {a:x} {b:x} op={index}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn derived_comparison_dependencies_preserve_scalar_domains() {
        for (left, right, equality, admitted) in [
            (Primitive::U8, Primitive::U8, false, true),
            (Primitive::I8, Primitive::I8, false, true),
            (Primitive::U8, Primitive::I8, true, false),
            (Primitive::Char, Primitive::Char, false, true),
            (Primitive::Str, Primitive::Str, true, true),
            (Primitive::Str, Primitive::Str, false, false),
            (Primitive::Bool, Primitive::Bool, true, true),
            (Primitive::Bool, Primitive::Bool, false, false),
            (Primitive::Type, Primitive::Type, true, false),
            (Primitive::F64, Primitive::F64, true, true),
            (Primitive::F32, Primitive::F32, false, true),
            (Primitive::F32, Primitive::F64, true, false),
        ] {
            let a = Box::new(expr(K::Local(LocalId { owner: 1, index: 0 }), left));
            let b = Box::new(expr(K::Local(LocalId { owner: 1, index: 1 }), right));
            let comparison = expr(
                if equality {
                    K::ScalarEqual(a, b)
                } else {
                    K::ScalarLess(a, b)
                },
                Primitive::Bool,
            );
            let inputs = structural_boolean_dependencies(&comparison, &mut 100).unwrap();
            assert_eq!(
                inputs.is_some(),
                admitted,
                "{left:?} {right:?} equality={equality}"
            );
            if let Some(inputs) = inputs {
                assert_eq!(inputs.len(), 2);
            }
        }
    }

    #[test]
    fn staged_field_chains_respect_depth_and_recover() {
        let projection = |length| {
            let mut value = expr(K::Literal(Literal::Bool(true)), Primitive::Bool);
            for _ in 0..length {
                value = Expr {
                    source: value.source,
                    ty: Term::Record(vec![("next".into(), value.ty.clone())]),
                    kind: K::AnonymousRecord {
                        fields: vec![value],
                    },
                };
            }
            for _ in 0..length {
                let Term::Record(fields) = &value.ty else {
                    unreachable!()
                };
                value = Expr {
                    source: value.source,
                    ty: fields[0].1.clone(),
                    kind: K::AnonymousField {
                        value: Box::new(value),
                        name: "next".into(),
                        index: 0,
                    },
                };
            }
            value
        };
        for length in [64, 140] {
            let deep = projection(length);
            let error = fold_value(&deep, &mut 100_000).unwrap_err();
            assert!(error.contains("depth"), "{error}");
        }
        let mut invalid = projection(3);
        let K::AnonymousField { name, .. } = &mut invalid.kind else {
            unreachable!()
        };
        *name = "wrong".into();
        assert!(
            fold_value(&invalid, &mut 100_000)
                .unwrap_err()
                .contains("identity")
        );
        for length in [8, 48, 63] {
            let small = projection(length);
            assert!(fold_value(&small, &mut 1).is_err());
            let completed = fold_value(&small, &mut 100_000).unwrap();
            assert!(matches!(completed.kind, K::Literal(Literal::Bool(true))));
        }
    }

    #[test]
    fn structural_boolean_dependencies_exclude_executable_producers() {
        let binding = LocalId { owner: 1, index: 0 };
        let local = expr(K::Local(binding), Primitive::Bool);
        let expression = expr(K::Not(Box::new(local.clone())), Primitive::Bool);
        assert_eq!(
            structural_boolean_dependencies(&expression, &mut 100).unwrap(),
            Some(BTreeSet::from([binding]))
        );
        assert!(structural_boolean_dependencies(&expression, &mut 0).is_err());
        let call = expr(
            K::Call {
                callee: Box::new(Expr {
                    source: local.source,
                    ty: Term::Function {
                        parameters: vec![],
                        result: Box::new(local.ty.clone()),
                    },
                    kind: K::Function(nir::DeclarationId {
                        package: 0,
                        local: 1,
                    }),
                }),
                arguments: vec![],
                parameter_bindings: vec![],
            },
            Primitive::Bool,
        );
        let hidden = expr(K::And(Box::new(local), Box::new(call)), Primitive::Bool);
        assert_eq!(
            structural_boolean_dependencies(&hidden, &mut 100).unwrap(),
            None
        );
    }

    #[test]
    fn staged_field_aliases_validate_paths_and_preserve_invalidations() {
        let input = LocalId { owner: 1, index: 1 };
        let alias = LocalId { owner: 1, index: 2 };
        let yes = expr(K::Literal(Literal::Bool(true)), Primitive::Bool);
        let record = Expr {
            source: yes.source,
            ty: Term::Record(vec![("flag".into(), yes.ty.clone())]),
            kind: K::AnonymousRecord {
                fields: vec![yes.clone()],
            },
        };
        let projection = Expr {
            source: yes.source,
            ty: yes.ty.clone(),
            kind: K::AnonymousField {
                value: Box::new(Expr {
                    source: yes.source,
                    ty: record.ty.clone(),
                    kind: K::Local(input),
                }),
                name: "flag".into(),
                index: 0,
            },
        };
        let declaration = expr(
            K::Let {
                binding: alias,
                value: Box::new(projection),
            },
            Primitive::Void,
        );
        let seed = std::collections::BTreeMap::from([(input, record)]);
        let mut completed = seed.clone();
        collect_forwarded_locals(
            &[],
            &declaration,
            &mut completed,
            &BTreeSet::new(),
            &mut 1000,
        )
        .unwrap();
        assert_eq!(completed.get(&alias), Some(&yes));
        for boundary in 0..4 {
            let mut invalid = declaration.clone();
            let K::Let {
                value: projection, ..
            } = &mut invalid.kind
            else {
                unreachable!()
            };
            let K::AnonymousField { value, name, index } = &mut projection.kind else {
                unreachable!()
            };
            match boundary {
                0 => *name = "other".into(),
                1 => *index = 1,
                2 => value.ty = Term::Primitive(Primitive::Bool),
                _ => projection.ty = Term::Primitive(Primitive::Char),
            }
            assert!(
                collect_forwarded_locals(
                    &[],
                    &invalid,
                    &mut seed.clone(),
                    &BTreeSet::new(),
                    &mut 1000
                )
                .is_err()
            );
        }
        let mut invalidated = seed.clone();
        collect_forwarded_locals(
            &[],
            &declaration,
            &mut invalidated,
            &BTreeSet::from([input]),
            &mut 1000,
        )
        .unwrap();
        assert!(!invalidated.contains_key(&alias));
        let mut runtime = std::collections::BTreeMap::new();
        collect_forwarded_locals(&[], &declaration, &mut runtime, &BTreeSet::new(), &mut 1000)
            .unwrap();
        assert!(runtime.is_empty());
        assert!(
            collect_forwarded_locals(
                &[],
                &declaration,
                &mut seed.clone(),
                &BTreeSet::new(),
                &mut 0
            )
            .is_err()
        );
    }

    #[test]
    fn nominal_staged_fields_check_identity_types_and_invalidations() {
        use nir::DeclarationId;
        let owner = DeclarationId {
            package: 1,
            local: 10,
        };
        let input = LocalId { owner: 1, index: 0 };
        let alias = LocalId { owner: 1, index: 1 };
        let yes = expr(K::Literal(Literal::Bool(true)), Primitive::Bool);
        let record = Expr {
            source: yes.source,
            ty: Term::Nominal {
                declaration: owner,
                arguments: vec![],
            },
            kind: K::Record {
                declaration: owner,
                fields: vec![yes.clone()],
            },
        };
        let projection = Expr {
            source: yes.source,
            ty: yes.ty.clone(),
            kind: K::Field {
                value: Box::new(record.clone()),
                declaration: owner,
                index: 0,
            },
        };
        assert_eq!(fold_value(&projection, &mut 1000).unwrap(), yes);
        assert!(fold_value(&projection, &mut 1).is_err());
        assert_eq!(fold_value(&projection, &mut 1000).unwrap(), yes);
        for boundary in 0..5 {
            let mut invalid = projection.clone();
            let K::Field {
                value,
                declaration,
                index,
            } = &mut invalid.kind
            else {
                unreachable!()
            };
            match boundary {
                0 => declaration.local += 1,
                1 => *index = 1,
                2 => invalid.ty = Term::Primitive(Primitive::Char),
                3 => {
                    value.ty = Term::Nominal {
                        declaration: DeclarationId {
                            package: 2,
                            local: 10,
                        },
                        arguments: vec![],
                    }
                }
                _ => {
                    value.kind = K::Variant {
                        declaration: owner,
                        index: 0,
                        fields: vec![yes.clone()],
                    }
                }
            }
            assert!(fold_value(&invalid, &mut 1000).is_err());
        }
        let mut selected = projection;
        let K::Field { value, .. } = &mut selected.kind else {
            unreachable!()
        };
        value.kind = K::Local(input);
        let declaration = expr(
            K::Let {
                binding: alias,
                value: Box::new(selected),
            },
            Primitive::Void,
        );
        let seed = std::collections::BTreeMap::from([(input, record)]);
        let mut values = seed.clone();
        collect_forwarded_locals(&[], &declaration, &mut values, &BTreeSet::new(), &mut 1000)
            .unwrap();
        assert_eq!(values.get(&alias), Some(&yes));
        let mut invalidated = seed;
        collect_forwarded_locals(
            &[],
            &declaration,
            &mut invalidated,
            &BTreeSet::from([input]),
            &mut 1000,
        )
        .unwrap();
        assert!(!invalidated.contains_key(&alias));
    }

    #[test]
    fn staged_record_patterns_check_field_identity_type_and_budget() {
        use nir::program::Pattern;
        let first = LocalId { owner: 1, index: 1 };
        let second = LocalId { owner: 1, index: 2 };
        let yes = expr(K::Literal(Literal::Bool(true)), Primitive::Bool);
        let no = expr(K::Literal(Literal::Bool(false)), Primitive::Bool);
        let record = Expr {
            source: yes.source,
            ty: Term::Record(vec![
                ("first".into(), yes.ty.clone()),
                ("second".into(), no.ty.clone()),
            ]),
            kind: K::AnonymousRecord {
                fields: vec![yes.clone(), no.clone()],
            },
        };
        let pattern = Pattern::AnonymousRecord {
            names: vec!["first".into(), "second".into()],
            fields: vec![first, second]
                .into_iter()
                .map(|binding| Pattern::Binding {
                    binding,
                    inner: Box::new(Pattern::Wild),
                })
                .collect(),
        };
        let mut locals = std::collections::BTreeMap::new();
        bind_staged_pattern(&[], &pattern, &record, &mut locals, &mut 1000, 0).unwrap();
        assert_eq!(locals.get(&first), Some(&yes));
        assert_eq!(locals.get(&second), Some(&no));
        for boundary in 0..3 {
            let mut invalid = pattern.clone();
            let Pattern::AnonymousRecord { names, fields } = &mut invalid else {
                unreachable!()
            };
            match boundary {
                0 => names.swap(0, 1),
                1 => {
                    fields.pop();
                }
                _ => names[1] = "first".into(),
            }
            assert!(
                bind_staged_pattern(
                    &[],
                    &invalid,
                    &record,
                    &mut Default::default(),
                    &mut 1000,
                    0
                )
                .is_err()
            );
        }
        let mut wrong = record.clone();
        let K::AnonymousRecord { fields } = &mut wrong.kind else {
            unreachable!()
        };
        fields[1] = expr(K::Literal(Literal::Char('x')), Primitive::Char);
        assert!(
            bind_staged_pattern(&[], &pattern, &wrong, &mut Default::default(), &mut 1000, 0)
                .is_err()
        );
        assert!(
            bind_staged_pattern(&[], &pattern, &record, &mut Default::default(), &mut 0, 0)
                .is_err()
        );
    }

    #[test]
    fn staged_records_validate_field_types_labels_and_budget() {
        let flag = expr(K::Literal(Literal::Bool(true)), Primitive::Bool);
        let mut record = Expr {
            kind: K::AnonymousRecord {
                fields: vec![flag.clone()],
            },
            ty: Term::Record(vec![("flag".into(), flag.ty.clone())]),
            source: flag.source,
        };
        assert_eq!(
            constant(&[], &record, &mut 100, 0).unwrap(),
            Term::RecordValue(vec![("flag".into(), Term::Bool(true))])
        );
        assert!(constant(&[], &record, &mut 2, 0).is_err());
        record.ty = Term::Record(vec![("flag".into(), Term::Primitive(Primitive::U8))]);
        assert!(constant(&[], &record, &mut 100, 0).is_err());
        record.ty = Term::Record(vec![("flag".into(), flag.ty.clone()); 2]);
        record.kind = K::AnonymousRecord {
            fields: vec![flag.clone(), flag],
        };
        assert!(constant(&[], &record, &mut 100, 0).is_err());
    }

    #[test]
    fn staged_discard_executes_its_operand_and_preserves_failures() {
        let known = expr(K::Literal(Literal::Bool(true)), Primitive::Bool);
        let discard = expr(K::Discard(Box::new(known)), Primitive::Void);
        assert_eq!(fold_value(&discard, &mut 100).unwrap().kind, K::Unit);
        let unknown = expr(K::Local(LocalId { owner: 1, index: 1 }), Primitive::Bool);
        assert!(
            fold_value(
                &expr(K::Discard(Box::new(unknown)), Primitive::Void),
                &mut 100
            )
            .is_err()
        );
        let overflow = expr(
            K::IntegerBinary {
                op: nir::program::IntegerOp::Add,
                left: Box::new(expr(
                    K::Literal(Literal::UnsignedInteger(255)),
                    Primitive::U8,
                )),
                right: Box::new(expr(K::Literal(Literal::UnsignedInteger(1)), Primitive::U8)),
            },
            Primitive::U8,
        );
        assert!(
            fold_value(
                &expr(K::Discard(Box::new(overflow)), Primitive::Void),
                &mut 1000
            )
            .unwrap_err()
            .contains("overflow")
        );
        assert!(fold_value(&discard, &mut 0).is_err());
        assert!(fold_value(&discard, &mut 100).is_ok());
    }

    #[test]
    fn staged_pattern_bindings_check_structure_and_scope() {
        use nir::program::Pattern;
        let binding = LocalId { owner: 1, index: 3 };
        let bind = Pattern::Binding {
            binding,
            inner: Box::new(Pattern::Wild),
        };
        let flag = expr(K::Literal(Literal::Bool(true)), Primitive::Bool);
        let pair = Expr {
            kind: K::Tuple(vec![flag.clone(), flag.clone()]),
            ty: Term::Tuple(vec![flag.ty.clone(), flag.ty.clone()]),
            source: flag.source,
        };
        let fold = |pattern, value, result| {
            let statement = expr(
                K::LetPattern {
                    pattern,
                    value: Box::new(value),
                    guard: None,
                    otherwise: None,
                },
                Primitive::Void,
            );
            let block = expr(
                K::Block {
                    statements: vec![statement],
                    result: Some(Box::new(result)),
                },
                Primitive::Bool,
            );
            fold_input(&[], &block, &mut 1000, 0)
        };
        assert_eq!(
            fold(
                Pattern::Tuple(vec![bind.clone(), Pattern::Wild]),
                pair.clone(),
                expr(K::Local(binding), Primitive::Bool)
            )
            .unwrap()
            .kind,
            flag.kind
        );
        assert!(
            fold(
                Pattern::Tuple(vec![bind.clone()]),
                pair.clone(),
                flag.clone()
            )
            .unwrap_err()
            .contains("arity")
        );
        assert!(
            fold(
                Pattern::Tuple(vec![bind.clone(), bind.clone()]),
                pair.clone(),
                flag.clone()
            )
            .unwrap_err()
            .contains("duplicate")
        );
        assert!(
            fold(Pattern::Tuple(vec![]), flag.clone(), flag.clone())
                .unwrap_err()
                .contains("tuple value")
        );
        assert!(fold(Pattern::Unit, flag.clone(), flag.clone()).is_err());
        // Protocol/computed patterns must not be silently treated as structural.
        assert!(
            fold(
                Pattern::Literal(Literal::Bool(true)),
                flag.clone(),
                flag.clone()
            )
            .is_err()
        );
        let mut locals = std::collections::BTreeMap::new();
        assert!(bind_staged_pattern(&[], &bind, &flag, &mut locals, &mut 0, 0).is_err());
        assert!(locals.is_empty());
        assert!(bind_staged_pattern(&[], &bind, &flag, &mut locals, &mut 1000, 128).is_err());
        assert!(locals.is_empty());
        bind_staged_pattern(&[], &bind, &flag, &mut locals, &mut 1000, 0).unwrap();
        assert_eq!(locals.get(&binding), Some(&flag));
        assert!(fold(Pattern::Tuple(vec![bind, Pattern::Wild]), pair, flag).is_ok());
    }

    #[test]
    fn staged_blocks_evaluate_in_order_and_reject_invalid_locals() {
        let flag = expr(K::Literal(Literal::Bool(true)), Primitive::Bool);
        let local = expr(K::Local(LocalId { owner: 1, index: 7 }), Primitive::Bool);
        let binding = expr(
            K::Let {
                binding: LocalId { owner: 1, index: 7 },
                value: Box::new(flag.clone()),
            },
            Primitive::Void,
        );
        let block = |statements, result| {
            expr(
                K::Block {
                    statements,
                    result: Some(Box::new(result)),
                },
                Primitive::Bool,
            )
        };
        let valid = block(
            vec![binding.clone()],
            expr(K::Not(Box::new(local.clone())), Primitive::Bool),
        );
        assert_eq!(
            fold_input(&[], &valid, &mut 1000, 0).unwrap().kind,
            K::Literal(Literal::Bool(false))
        );
        assert!(
            fold_input(
                &[],
                &block(vec![local.clone(), binding.clone()], flag.clone()),
                &mut 1000,
                0
            )
            .is_err(),
            "use before initialization must fail"
        );
        assert!(
            fold_input(
                &[],
                &block(vec![binding.clone(), binding.clone()], local.clone()),
                &mut 1000,
                0
            )
            .unwrap_err()
            .contains("duplicate")
        );
        let wrong = expr(K::Local(LocalId { owner: 1, index: 7 }), Primitive::U8);
        assert!(
            fold_input(
                &[],
                &block(vec![binding.clone(), wrong], flag.clone()),
                &mut 1000,
                0
            )
            .unwrap_err()
            .contains("type mismatch")
        );
        let overflow = expr(
            K::IntegerBinary {
                op: nir::program::IntegerOp::Add,
                left: Box::new(expr(
                    K::Literal(Literal::UnsignedInteger(255)),
                    Primitive::U8,
                )),
                right: Box::new(expr(K::Literal(Literal::UnsignedInteger(1)), Primitive::U8)),
            },
            Primitive::U8,
        );
        assert!(
            fold_input(&[], &block(vec![overflow], flag.clone()), &mut 1000, 0)
                .unwrap_err()
                .contains("overflow"),
            "discarded expressions must still evaluate"
        );
        let nested = block(vec![binding], local.clone());
        assert!(
            fold_input(&[], &block(vec![nested], local), &mut 1000, 0).is_err(),
            "nested locals must not escape"
        );
        assert!(fold_input(&[], &valid, &mut 1, 0).is_err());
        assert!(fold_input(&[], &valid, &mut 1000, 0).is_ok());
    }

    #[test]
    fn staged_machine_division_uses_truncation_and_rejects_zero_and_overflow() {
        use nir::program::IntegerOp as Op;
        for (ty, min) in [
            (Primitive::I8, i8::MIN as i128),
            (Primitive::I16, i16::MIN as i128),
            (Primitive::I32, i32::MIN as i128),
            (Primitive::I64, i64::MIN as i128),
            (Primitive::I128, i128::MIN),
        ] {
            let operation = |op, a, b| {
                expr(
                    K::IntegerBinary {
                        op,
                        left: Box::new(expr(K::Literal(Literal::SignedInteger(a)), ty)),
                        right: Box::new(expr(K::Literal(Literal::SignedInteger(b)), ty)),
                    },
                    ty,
                )
            };
            for op in [Op::DivideTrunc, Op::RemainderTrunc] {
                for (a, b) in [(-7, 3), (7, -3), (-7, -3), (7, 3)] {
                    assert_eq!(
                        fold_input(&[], &operation(op, a, b), &mut 1000, 0)
                            .unwrap()
                            .kind,
                        K::Literal(Literal::SignedInteger(if op == Op::DivideTrunc {
                            a / b
                        } else {
                            a % b
                        }))
                    );
                }
                assert!(
                    fold_input(&[], &operation(op, min, -1), &mut 1000, 0)
                        .unwrap_err()
                        .contains("overflow")
                );
                assert!(
                    fold_input(&[], &operation(op, 1, 0), &mut 1000, 0)
                        .unwrap_err()
                        .contains("division by zero")
                );
                assert!(fold_input(&[], &operation(op, 6, 3), &mut 0, 0).is_err());
                assert!(fold_input(&[], &operation(op, 6, 3), &mut 1000, 0).is_ok());
            }
        }
    }

    #[test]
    fn staged_machine_arithmetic_preserves_width_and_overflow_checks() {
        use nir::program::IntegerOp as Op;
        for (ty, max) in [
            (Primitive::U8, u8::MAX as u128),
            (Primitive::U16, u16::MAX as u128),
            (Primitive::U32, u32::MAX as u128),
            (Primitive::U64, u64::MAX as u128),
            (Primitive::U128, u128::MAX),
        ] {
            let operation = |op, a, b| {
                expr(
                    K::IntegerBinary {
                        op,
                        left: Box::new(expr(K::Literal(Literal::UnsignedInteger(a)), ty)),
                        right: Box::new(expr(K::Literal(Literal::UnsignedInteger(b)), ty)),
                    },
                    ty,
                )
            };
            assert_eq!(
                fold_input(&[], &operation(Op::Add, max, 0), &mut 1000, 0)
                    .unwrap()
                    .kind,
                K::Literal(Literal::UnsignedInteger(max))
            );
            for (op, a, b) in [
                (Op::Add, max, 1),
                (Op::Subtract, 0, 1),
                (Op::Multiply, max, 2),
            ] {
                assert!(
                    fold_input(&[], &operation(op, a, b), &mut 1000, 0)
                        .unwrap_err()
                        .contains("overflow")
                );
            }
            for op in [Op::DivideTrunc, Op::RemainderTrunc] {
                assert_eq!(
                    fold_input(&[], &operation(op, max, 3), &mut 1000, 0)
                        .unwrap()
                        .kind,
                    K::Literal(Literal::UnsignedInteger(if op == Op::DivideTrunc {
                        max / 3
                    } else {
                        max % 3
                    }))
                );
            }
            assert!(
                fold_input(&[], &operation(Op::DivideTrunc, 1, 0), &mut 1000, 0)
                    .unwrap_err()
                    .contains("division by zero")
            );
            let skipped = expr(
                K::If {
                    condition: Box::new(expr(K::Literal(Literal::Bool(false)), Primitive::Bool)),
                    then_branch: Box::new(operation(Op::Add, max, 1)),
                    else_branch: Some(Box::new(expr(K::Literal(Literal::UnsignedInteger(7)), ty))),
                },
                ty,
            );
            assert_eq!(
                fold_input(&[], &skipped, &mut 1000, 0).unwrap().kind,
                K::Literal(Literal::UnsignedInteger(7))
            );
        }
        for (ty, min) in [
            (Primitive::I8, i8::MIN as i128),
            (Primitive::I16, i16::MIN as i128),
            (Primitive::I32, i32::MIN as i128),
            (Primitive::I64, i64::MIN as i128),
            (Primitive::I128, i128::MIN),
        ] {
            let negative = expr(
                K::IntegerNegate(Box::new(expr(K::Literal(Literal::SignedInteger(min)), ty))),
                ty,
            );
            assert!(
                fold_input(&[], &negative, &mut 1000, 0)
                    .unwrap_err()
                    .contains("overflow")
            );
            let valid = expr(
                K::IntegerNegate(Box::new(expr(K::Literal(Literal::SignedInteger(-7)), ty))),
                ty,
            );
            assert_eq!(
                fold_input(&[], &valid, &mut 1000, 0).unwrap().kind,
                K::Literal(Literal::SignedInteger(7))
            );
        }
    }

    #[test]
    fn staged_folding_short_circuits_and_recovers_after_limits() {
        let yes = expr(K::Literal(Literal::Bool(true)), Primitive::Bool);
        let no = expr(K::Literal(Literal::Bool(false)), Primitive::Bool);
        let unknown = expr(K::Local(LocalId { owner: 1, index: 0 }), Primitive::Bool);
        for (kind, expected) in [
            (
                K::And(Box::new(no.clone()), Box::new(unknown.clone())),
                false,
            ),
            (
                K::Or(Box::new(yes.clone()), Box::new(unknown.clone())),
                true,
            ),
            (
                K::If {
                    condition: Box::new(yes.clone()),
                    then_branch: Box::new(no.clone()),
                    else_branch: Some(Box::new(unknown.clone())),
                },
                false,
            ),
        ] {
            let value = fold_input(&[], &expr(kind, Primitive::Bool), &mut 100, 0).unwrap();
            assert_eq!(value.kind, K::Literal(Literal::Bool(expected)));
        }
        assert!(
            fold_input(
                &[],
                &expr(
                    K::And(Box::new(yes.clone()), Box::new(unknown)),
                    Primitive::Bool
                ),
                &mut 100,
                0
            )
            .is_err()
        );
        let mut deep = yes.clone();
        for _ in 0..160 {
            deep = expr(K::Not(Box::new(deep)), Primitive::Bool);
        }
        assert!(
            fold_input(&[], &deep, &mut 1000, 0)
                .unwrap_err()
                .contains("depth")
        );
        assert!(fold_input(&[], &yes, &mut 0, 0).is_err());
        assert_eq!(fold_input(&[], &yes, &mut 100, 0).unwrap(), yes);
        let ty = expr(
            K::TypeValue(Term::Primitive(Primitive::Bool)),
            Primitive::Type,
        );
        assert!(
            fold_input(
                &[],
                &expr(
                    K::ScalarEqual(Box::new(ty.clone()), Box::new(ty)),
                    Primitive::Bool
                ),
                &mut 100,
                0
            )
            .is_err()
        );
    }

    #[test]
    fn floating_staged_constants_validate_width_and_preserve_nan_bits() {
        let value = expr(K::Literal(Literal::Float32(0x7fc00042)), Primitive::F32);
        assert_eq!(
            constant(&[], &value, &mut 100, 0).unwrap(),
            Term::Float32(0x7fc00042)
        );
        assert!(constant(&[], &value, &mut 0, 0).is_err());
        let wrong = expr(value.kind.clone(), Primitive::F64);
        assert!(constant(&[], &wrong, &mut 100, 0).is_err());
        let value = expr(
            K::Literal(Literal::Float64(0x8000000000000000)),
            Primitive::F64,
        );
        assert_eq!(
            constant(&[], &value, &mut 100, 0).unwrap(),
            Term::Float64(1 << 63)
        );
        assert!(constant(&[], &expr(value.kind, Primitive::F32), &mut 100, 0).is_err());
    }

    #[test]
    fn materialized_signed_integer_inputs_preserve_sign_and_budget() {
        for spelling in ["-0x4", "-4", "-0b100"] {
            let value = expr(
                K::Literal(Literal::Integer(spelling.into())),
                Primitive::Integer,
            );
            assert_eq!(
                constant(&[], &value, &mut 1000, 0).unwrap(),
                Term::Integer(comptime_runtime::Integer::from_i128(-4))
            );
            assert!(constant(&[], &value, &mut 0, 0).is_err());
            assert!(constant(&[], &value, &mut 1000, 0).is_ok());
        }
        for spelling in ["--0x4", "-", "-0x", "+4"] {
            let value = expr(
                K::Literal(Literal::Integer(spelling.into())),
                Primitive::Integer,
            );
            assert!(constant(&[], &value, &mut 1000, 0).is_err(), "{spelling}");
        }
    }

    #[test]
    fn canonical_inputs_validate_representations_and_charge_work() {
        let integer = expr(
            K::Literal(Literal::Integer(
                "0x100000000000000000000000000000000".into(),
            )),
            Primitive::Integer,
        );
        assert!(
            matches!(constant(&[], &integer,&mut 10_000,0).unwrap(),Term::Integer(value) if value.bit_len()==129)
        );
        let bits = expr(
            K::Literal(Literal::BitVec {
                width: 3,
                words: vec![5],
            }),
            Primitive::BitVec(3),
        );
        assert!(
            matches!(constant(&[], &bits,&mut 100,0).unwrap(),Term::BitVec(value) if value.words()==[5])
        );
        let invalid = expr(
            K::Literal(Literal::BitVec {
                width: 3,
                words: vec![13],
            }),
            Primitive::BitVec(3),
        );
        assert!(constant(&[], &invalid, &mut 100, 0).is_err());
        for (value, ok) in [(-128, true), (127, true), (-129, false), (128, false)] {
            let value = expr(K::Literal(Literal::SignedInteger(value)), Primitive::I8);
            assert_eq!(constant(&[], &value, &mut 100, 0).is_ok(), ok);
        }
        let string = expr(K::Literal(Literal::String("x".repeat(128))), Primitive::Str);
        assert!(constant(&[], &string, &mut 20, 0).is_err());
        let unknown = expr(
            K::TypeValue(Term::Rigid {
                declaration: nir::DeclarationId {
                    package: 1,
                    local: 1,
                },
                sort: Sort::Type,
            }),
            Primitive::Type,
        );
        assert!(constant(&[], &unknown, &mut 100, 0).is_err());
        assert!(constant(&[], &integer, &mut 100, 128).is_err());
        assert!(
            constant(
                &[],
                &expr(K::Literal(Literal::Bool(true)), Primitive::Bool),
                &mut 100,
                0
            )
            .is_ok()
        );
    }
}

#[cfg(test)]
mod newtype_staging_tests {
    use super::*;
    #[test]
    fn deeply_nested_newtype_values_fail_at_the_logical_depth_limit() {
        let source = nir::program::SourceRange { start: 0, end: 1 };
        let mut value = Expr {
            kind: K::Literal(Literal::Bool(true)),
            ty: Term::Primitive(Primitive::Bool),
            source,
        };
        let mut definitions = Vec::new();
        for local in 0..160 {
            let declaration = nir::DeclarationId { package: 7, local };
            definitions.push(nir::newtypes::Newtype {
                capture_count: 0,
                parameter_types: vec![],
                declaration,
                name: format!("Wrapper{local}"),
                type_parameters: vec![],
                requirements: vec![],
                capabilities: vec![],
                representation: value.ty.clone(),
            });
            value = Expr {
                kind: K::NewtypeConvert(Box::new(value)),
                ty: Term::Nominal {
                    declaration,
                    arguments: vec![],
                },
                source,
            };
        }
        assert!(
            fold_value_with_layouts(Layouts::new(&[], &definitions), &value, &mut 1_000_000)
                .unwrap_err()
                .contains("depth")
        );
    }

    #[test]
    fn newtype_staging_requires_layout_and_preserves_wrapping_direction() {
        let declaration = nir::DeclarationId {
            package: 3,
            local: 1,
        };
        let definition = nir::newtypes::Newtype {
            capture_count: 0,
            parameter_types: vec![],
            declaration,
            name: "Flag".into(),
            type_parameters: vec![],
            requirements: vec![],
            capabilities: vec![],
            representation: Term::Primitive(Primitive::Bool),
        };
        let definitions = [definition];
        let layouts = Layouts::new(&[], &definitions);
        let source = nir::program::SourceRange { start: 0, end: 1 };
        let ty = Term::Nominal {
            declaration,
            arguments: vec![],
        };
        let boolean = Expr {
            kind: K::Literal(Literal::Bool(true)),
            ty: Term::Primitive(Primitive::Bool),
            source,
        };
        let wrapped = Expr {
            kind: K::NewtypeConvert(Box::new(boolean.clone())),
            ty: ty.clone(),
            source,
        };
        assert!(fold_value(&wrapped, &mut 100).is_err());
        assert_eq!(
            constant(layouts, &wrapped, &mut 100, 0).unwrap(),
            Term::NominalValue {
                ty: Box::new(ty.clone()),
                variant: None,
                fields: vec![Term::Bool(true)]
            }
        );
        let peeled = Expr {
            kind: K::NewtypeConvert(Box::new(wrapped.clone())),
            ty: boolean.ty.clone(),
            source,
        };
        assert_eq!(
            fold_value_with_layouts(layouts, &peeled, &mut 100).unwrap(),
            boolean
        );
        assert!(constant(layouts, &peeled, &mut 100, 0).is_err());
        let wrong = Expr {
            kind: K::NewtypeConvert(Box::new(Expr {
                kind: K::Unit,
                ty: Term::Primitive(Primitive::Void),
                source,
            })),
            ty,
            source,
        };
        assert!(fold_value_with_layouts(layouts, &wrong, &mut 100).is_err());
        assert!(fold_value_with_layouts(layouts, &wrapped, &mut 0).is_err());
        assert_eq!(
            fold_value_with_layouts(layouts, &wrapped, &mut 100).unwrap(),
            wrapped
        );
    }
}

#[cfg(test)]
mod error_transport_tests;
