//! Definition-local callable aliases. Mutation or exposure invalidates the
//! whole binding, so branch/loop ordering cannot reuse an obsolete initializer.
use super::*;
use nir::program::{Function, LocalId};
use std::collections::{BTreeMap, BTreeSet};

// Only selects possible resulting values. Caller-side pattern/guard evaluation
// and exits have already been accounted for by control; nothing executes here.
pub(super) fn selected_bodies<'a>(
    value: &Expr,
    arms: &'a [MatchArm],
    fuel: &mut usize,
) -> Result<Vec<&'a Expr>, String> {
    use super::super::pattern_facts::{irrefutable_shape, known_pattern};
    let mut bodies = Vec::new();
    for arm in arms {
        step(fuel, 1)?;
        let mut pattern = &arm.pattern;
        let mut guards = Vec::new();
        while let Pattern::Guard {
            pattern: inner,
            condition,
        } = pattern
        {
            step(fuel, 1)?;
            guards.push(condition.as_ref());
            pattern = inner;
        }
        let matched = known_pattern(pattern, value, 0)
            .or_else(|| irrefutable_shape(pattern, 0).then_some(true));
        if matched == Some(false) {
            continue;
        }
        let mut guard = Some(true);
        for condition in guards.into_iter().rev().chain(arm.guard.as_ref()) {
            match completed_bool(condition, fuel, 0)? {
                Some(false) => {
                    guard = Some(false);
                    break;
                }
                None => guard = None,
                Some(true) => {}
            }
        }
        if guard == Some(false) {
            continue;
        }
        if arm.body.ty != Term::Primitive(nir::Primitive::Never) {
            bodies.push(&arm.body);
        }
        if matched == Some(true) && guard == Some(true) {
            break;
        }
    }
    Ok(bodies)
}

fn pattern_binds(
    pattern: &Pattern,
    exposed: &mut BTreeSet<LocalId>,
    fuel: &mut usize,
) -> Result<(), String> {
    let mut pending = vec![pattern];
    while let Some(pattern) = pending.pop() {
        step(fuel, 1)?;
        match pattern {
            Pattern::Binding {
                binding: local,
                inner,
            } => {
                exposed.insert(*local);
                pending.push(inner);
            }
            Pattern::Remap { pattern, bindings } => {
                step(fuel, bindings.len())?;
                for (from, to) in bindings {
                    exposed.insert(*from);
                    exposed.insert(*to);
                }
                pending.push(pattern);
            }
            Pattern::Observe { inner: pattern, .. } | Pattern::Guard { pattern, .. } => {
                pending.push(pattern)
            }
            Pattern::Then { first, next, .. } => {
                pending.push(first);
                pending.push(next);
            }
            Pattern::Tuple(fields)
            | Pattern::TypeTuple(fields)
            | Pattern::Or(fields)
            | Pattern::Record { fields, .. }
            | Pattern::AnonymousRecord { fields, .. }
            | Pattern::Variant { fields, .. }
            | Pattern::NormalForm {
                arguments: fields, ..
            } => {
                step(fuel, fields.len())?;
                pending.extend(fields);
            }
            _ => {}
        }
    }
    Ok(())
}

fn argument_binds(
    arguments: &[nir::program::NormalFormArgument],
    exposed: &mut BTreeSet<LocalId>,
    fuel: &mut usize,
) -> Result<(), String> {
    use nir::program::NormalFormArgument as A;
    step(fuel, arguments.len())?;
    let mut pending: Vec<_> = arguments.iter().collect();
    while let Some(argument) = pending.pop() {
        step(fuel, 1)?;
        match argument {
            A::Match { arms, .. } => {
                for arm in arms {
                    pattern_binds(&arm.pattern, exposed, fuel)?;
                    pending.push(&arm.body);
                }
            }
            A::If {
                then_input,
                else_input,
                ..
            } => {
                pending.push(then_input);
                pending.push(else_input);
            }
            A::Block { result, .. } => {
                if let Some(result) = result {
                    pending.push(result);
                }
            }
            A::TypeList { items, .. } | A::Tuple { items, .. } => {
                step(fuel, items.len())?;
                pending.extend(items);
            }
            A::TypeListSpread(_) | A::Value(_) | A::Unit { .. } | A::Integer { .. } => {}
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub(super) enum Projection {
    Record(Option<nir::DeclarationId>, usize),
    Tuple(usize),
    Variant(nir::DeclarationId, usize, usize),
    // Continue into the callable value's return value. This is a value query,
    // not an invocation: control() accounts for execution separately.
    Returned,
}
#[derive(Clone)]
struct Alias<'a> {
    value: &'a Expr,
    path: Vec<Projection>,
    remapped_from: Option<LocalId>,
}
fn structural_bindings<'a>(
    pattern: &'a Pattern,
    value: &'a Expr,
    fuel: &mut usize,
) -> Result<Option<Vec<(LocalId, Alias<'a>)>>, String> {
    enum Work<'a> {
        Pattern(&'a Pattern, &'a Expr, Vec<Projection>),
        Remap(usize, &'a [(LocalId, LocalId)]),
    }
    let mut result: Vec<(LocalId, Alias<'a>)> = Vec::new();
    let mut pending = vec![Work::Pattern(pattern, value, Vec::new())];
    while let Some(work) = pending.pop() {
        step(fuel, 1)?;
        let (pattern, value, path) = match work {
            Work::Pattern(pattern, value, path) => (pattern, value, path),
            Work::Remap(start, bindings) => {
                let end = result.len();
                let mut mapped = Vec::new();
                for (from, to) in bindings {
                    step(fuel, 1)?;
                    let mut found = false;
                    for (binding, alias) in &result[start..end] {
                        step(fuel, 1)?;
                        if binding == from {
                            step(fuel, alias.path.len() + 1)?;
                            let mut mapped_alias = alias.clone();
                            mapped_alias.remapped_from = Some(*from);
                            mapped.push((*to, mapped_alias));
                            found = true;
                        }
                    }
                    if !found {
                        return Ok(None);
                    }
                }
                result.extend(mapped);
                continue;
            }
        };
        match pattern {
            Pattern::Wild | Pattern::Unit | Pattern::Literal(_) => {}
            Pattern::Binding { binding, inner } => {
                step(fuel, path.len())?;
                pending.push(Work::Pattern(inner, value, path.clone()));
                result.push((
                    *binding,
                    Alias {
                        value,
                        path,
                        remapped_from: None,
                    },
                ));
            }
            Pattern::Observe { inner: pattern, .. } | Pattern::Guard { pattern, .. } => {
                pending.push(Work::Pattern(pattern, value, path))
            }
            Pattern::Then {
                first,
                value: transformed,
                next,
            } => {
                pending.push(Work::Pattern(first, value, path));
                pending.push(Work::Pattern(next, transformed, Vec::new()));
            }
            Pattern::AnonymousRecord { fields, .. } => {
                for (index, field) in fields.iter().enumerate() {
                    step(fuel, path.len() + 1)?;
                    let mut child = path.clone();
                    child.push(Projection::Record(None, index));
                    pending.push(Work::Pattern(field, value, child));
                }
            }
            Pattern::Record {
                declaration,
                fields,
            } => {
                for (index, field) in fields.iter().enumerate() {
                    step(fuel, path.len() + 1)?;
                    let mut child = path.clone();
                    child.push(Projection::Record(Some(*declaration), index));
                    pending.push(Work::Pattern(field, value, child));
                }
            }
            Pattern::Tuple(fields) => {
                for (index, field) in fields.iter().enumerate() {
                    step(fuel, path.len() + 1)?;
                    let mut child = path.clone();
                    child.push(Projection::Tuple(index));
                    pending.push(Work::Pattern(field, value, child));
                }
            }
            Pattern::Variant {
                declaration,
                index: tag,
                fields,
            } => {
                for (index, field) in fields.iter().enumerate() {
                    step(fuel, path.len() + 1)?;
                    let mut child = path.clone();
                    child.push(Projection::Variant(*declaration, *tag, index));
                    pending.push(Work::Pattern(field, value, child));
                }
            }
            Pattern::Or(alternatives) => {
                for alternative in alternatives {
                    step(fuel, path.len() + 1)?;
                    pending.push(Work::Pattern(alternative, value, path.clone()));
                }
            }
            Pattern::Remap { pattern, bindings } => {
                pending.push(Work::Remap(result.len(), bindings));
                pending.push(Work::Pattern(pattern, value, path));
            }
            _ => return Ok(None),
        }
    }
    Ok(Some(result))
}
fn register_pattern<'a>(
    pattern: &'a Pattern,
    value: &'a Expr,
    definitions: &mut BTreeMap<LocalId, Vec<Alias<'a>>>,
    exposed: &mut BTreeSet<LocalId>,
    fuel: &mut usize,
) -> Result<(), String> {
    if let Some(bindings) = structural_bindings(pattern, value, fuel)? {
        let mut alternatives: BTreeMap<LocalId, Vec<Alias<'a>>> = BTreeMap::new();
        for (binding, alias) in bindings {
            step(fuel, 1)?;
            alternatives.entry(binding).or_default().push(alias);
        }
        for (binding, aliases) in alternatives {
            if definitions.insert(binding, aliases).is_some() {
                exposed.insert(binding);
            }
        }
    } else {
        pattern_binds(pattern, exposed, fuel)?;
    }
    Ok(())
}
/// Borrowed definitions only: no summary survives a fixed-point iteration or
/// crosses an instance graph. Mutation exposure is stable for this graph.
pub(super) struct Aliases<'a> {
    definitions: BTreeMap<LocalId, Vec<Alias<'a>>>,
}
impl<'a> Aliases<'a> {
    pub(super) fn prepare(function: &'a Function, fuel: &mut usize) -> Result<Self, String> {
        step(fuel, function.parameters.len())?;
        let mut exposed: BTreeSet<_> = function.parameters.iter().copied().collect();
        let mut definitions = BTreeMap::new();
        iir::visit::expressions(function, fuel, |expr, fuel| {
            match &expr.kind {
                K::Let { binding, value } => {
                    if definitions
                        .insert(
                            *binding,
                            vec![Alias {
                                value,
                                path: vec![],
                                remapped_from: None,
                            }],
                        )
                        .is_some()
                    {
                        exposed.insert(*binding);
                    }
                }
                K::LetPattern { pattern, value, .. } => {
                    register_pattern(pattern, value, &mut definitions, &mut exposed, fuel)?;
                }
                K::NormalFormType { arguments, .. } => {
                    argument_binds(arguments, &mut exposed, fuel)?
                }
                K::Match { value, arms } => {
                    for arm in arms {
                        register_pattern(
                            &arm.pattern,
                            value,
                            &mut definitions,
                            &mut exposed,
                            fuel,
                        )?;
                    }
                }
                K::EffectHandle { handlers, .. } => {
                    for handler in handlers {
                        for arm in &handler.arms {
                            pattern_binds(&arm.pattern, &mut exposed, fuel)?;
                        }
                    }
                }
                K::Call {
                    parameter_bindings, ..
                } => {
                    step(fuel, parameter_bindings.len())?;
                    exposed.extend(parameter_bindings);
                }
                K::Assign { place, .. }
                | K::BitAssign { place, .. }
                | K::BitUpdate { place, .. } => {
                    exposed.insert(place.root);
                }
                K::AddressOf(value) => {
                    iir::visit::expression(value, fuel, |value, _| {
                        if let K::Local(local) = value.kind {
                            exposed.insert(local);
                        }
                        Ok(())
                    })?;
                }
                K::Closure {
                    parameters,
                    captures,
                    body,
                } => {
                    step(fuel, parameters.len() + captures.len())?;
                    exposed.extend(parameters);
                    exposed.extend(captures.iter().map(|c| c.binding));
                    iir::visit::expression(body, fuel, |value, _| {
                        if let K::Local(local) = value.kind {
                            exposed.insert(local);
                        }
                        Ok(())
                    })?;
                }
                _ => {}
            }
            Ok(())
        })?;
        // Guard writes to a branch-local source must invalidate the canonical
        // binding produced by a later Remap, not only the temporary identity.
        loop {
            let mut changed = false;
            for (binding, aliases) in &definitions {
                step(fuel, aliases.len() + 1)?;
                if aliases.iter().any(|alias| {
                    alias
                        .remapped_from
                        .is_some_and(|from| exposed.contains(&from))
                }) {
                    changed |= exposed.insert(*binding);
                }
            }
            if !changed {
                break;
            }
        }
        step(fuel, definitions.len())?;
        definitions.retain(|binding, _| !exposed.contains(binding));
        Ok(Self { definitions })
    }
}
impl Analyzer<'_, '_, '_, '_> {
    pub(super) fn field_callable(
        &mut self,
        value: &Expr,
        owner: Option<nir::DeclarationId>,
        index: usize,
        n: usize,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<Uses, String> {
        self.projected_callable(value, &[Projection::Record(owner, index)], n, fuel, depth)
    }

    pub(super) fn projected_callable(
        &mut self,
        value: &Expr,
        path: &[Projection],
        n: usize,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<Uses, String> {
        step(fuel, n + 1)?;
        if depth >= 128 {
            return Err("effect usage callable projection depth exhausted".into());
        }
        if value.ty == Term::Primitive(nir::Primitive::Never) {
            return Ok(vec![0; n]);
        }
        let Some((&projection, tail)) = path.split_first() else {
            return self.invoked(value, fuel, depth + 1);
        };
        let fields = match &value.kind {
            K::Closure { body, .. } if matches!(projection, Projection::Returned) => {
                return self.closure_return_projection(body, tail, n, fuel, depth + 1);
            }
            K::Function(_) | K::FunctionInstance { .. }
                if matches!(projection, Projection::Returned) =>
            {
                return self.returned_projection(value, tail, n, fuel, depth + 1);
            }
            K::Call { callee, .. } => {
                return self.returned_projection(callee, path, n, fuel, depth + 1);
            }
            K::EffectRun { computation, .. } => {
                if let K::Call { callee, .. } = &computation.kind {
                    return self.returned_projection(callee, path, n, fuel, depth + 1);
                }
                None
            }
            K::Match { value, arms } => {
                let mut uses = vec![0; n];
                for body in selected_bodies(value, arms, fuel)? {
                    join(
                        &mut uses,
                        &self.projected_callable(body, path, n, fuel, depth + 1)?,
                        fuel,
                    )?;
                }
                return Ok(uses);
            }
            K::Field {
                value,
                declaration,
                index,
            } => {
                step(fuel, path.len() + 1)?;
                let mut nested = vec![Projection::Record(Some(*declaration), *index)];
                nested.extend_from_slice(path);
                return self.projected_callable(value, &nested, n, fuel, depth + 1);
            }
            K::AnonymousField { value, index, .. } => {
                step(fuel, path.len() + 1)?;
                let mut nested = vec![Projection::Record(None, *index)];
                nested.extend_from_slice(path);
                return self.projected_callable(value, &nested, n, fuel, depth + 1);
            }
            K::Local(binding) => {
                let definitions = self.aliases;
                if let Some(aliases) = definitions.definitions.get(binding) {
                    let mut uses = vec![0; n];
                    for alias in aliases {
                        step(fuel, alias.path.len() + path.len() + 1)?;
                        let mut nested = alias.path.clone();
                        nested.extend_from_slice(path);
                        join(
                            &mut uses,
                            &self.projected_callable(alias.value, &nested, n, fuel, depth + 1)?,
                            fuel,
                        )?;
                    }
                    return Ok(uses);
                }
                None
            }
            K::Block {
                result: Some(result),
                ..
            }
            | K::CallPreparation {
                result: Some(result),
                ..
            } => {
                return self.projected_callable(result, path, n, fuel, depth + 1);
            }
            K::If {
                condition,
                then_branch,
                else_branch: Some(else_branch),
            } => {
                let known = completed_bool(condition, fuel, 0)?;
                let mut uses = vec![0; n];
                for (branch, reachable) in [
                    (then_branch, known != Some(false)),
                    (else_branch, known != Some(true)),
                ] {
                    if reachable {
                        join(
                            &mut uses,
                            &self.projected_callable(branch, path, n, fuel, depth + 1)?,
                            fuel,
                        )?;
                    }
                }
                return Ok(uses);
            }
            K::Record {
                declaration,
                fields,
            } if matches!(projection, Projection::Record(Some(owner), _) if owner == *declaration) => {
                Some(fields)
            }
            K::AnonymousRecord { fields } if matches!(projection, Projection::Record(None, _)) => {
                Some(fields)
            }
            K::Tuple(fields) if matches!(projection, Projection::Tuple(_)) => Some(fields),
            K::Variant {
                declaration,
                index: tag,
                fields,
            } => {
                match projection {
                    Projection::Variant(owner, expected, _) if owner == *declaration => {
                        // The binding exists only on a successful tag match.
                        if expected != *tag {
                            return Ok(vec![0; n]);
                        }
                        Some(fields)
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some(fields) = fields {
            let index = match projection {
                Projection::Record(_, index)
                | Projection::Tuple(index)
                | Projection::Variant(_, _, index) => index,
                Projection::Returned => return Err("effect usage return is not a field".into()),
            };
            self.projected_callable(
                fields
                    .get(index)
                    .ok_or("effect usage callable field missing")?,
                tail,
                n,
                fuel,
                depth + 1,
            )
        } else {
            Ok(vec![2; n])
        }
    }

    pub(super) fn local_callable(
        &mut self,
        binding: LocalId,
        n: usize,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<Option<Uses>, String> {
        step(fuel, 1)?;
        if depth >= 128 {
            return Err("effect usage callable alias depth exhausted".into());
        }
        let definitions = self.aliases;
        let Some(aliases) = definitions.definitions.get(&binding) else {
            return Ok(None);
        };
        let mut uses = vec![0; n];
        for alias in aliases {
            step(fuel, 1)?;
            join(
                &mut uses,
                &self.projected_callable(alias.value, &alias.path, n, fuel, depth + 1)?,
                fuel,
            )?;
        }
        Ok(Some(uses))
    }
}
