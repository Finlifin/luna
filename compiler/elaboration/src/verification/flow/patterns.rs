//! Scalar pattern candidates, ordered failure continuation and guard effects.
use super::*;
use nir::program::{MatchArm, Pattern};
use std::collections::BTreeSet;

enum MatchCase<'a> {
    Expression(&'a MatchArm),
    Contextual(&'a nir::program::NormalMatchArm),
}

impl State {
    pub(super) fn merge(mut self, other: Self, limits: &mut Limits) -> Result<Self, Unknown> {
        self.join_aggregates(&other, limits)?;
        self.join_paths_and_facts(&other, limits)?;
        self.domains.extend(other.domains);
        Ok(self)
    }
    fn keep(&mut self, existing: &BTreeSet<PlaceId>, limits: &mut Limits) -> Result<(), Unknown> {
        limits.step(
            self.domains
                .len()
                .saturating_add(self.reads.len())
                .saturating_add(existing.len()),
        )?;
        self.reads
            .retain(|address, _| existing.contains(&address.root));
        // A read's ghost value outlives the pattern candidate when the address
        // binding remains in scope. It is not a newly exposed source binding.
        let mut retained = existing.clone();
        retained.extend(self.reads.values().map(|(_, root)| *root));
        self.domains.retain(|place, _| retained.contains(place));
        self.aggregates.retain(|place, _| retained.contains(place));
        limits.step(self.paths.len())?;
        self.paths.retain(|path, _| retained.contains(&path.root));
        retained.extend(self.paths.values().map(|id| id.place));
        self.facts.retain_places(&retained, limits)
    }
}
impl Walker<'_> {
    fn split_condition(
        &mut self,
        mut state: State,
        condition: Option<&Condition>,
    ) -> Result<(State, State), Reason> {
        let mut failure = state.fork(self.limits)?;
        if let Some(condition) = condition {
            state.facts.assume(condition, true, self.limits)?;
            failure.facts.assume(condition, false, self.limits)?;
        }
        Ok((state, failure))
    }
    fn input_range(&mut self, input: &Value, state: &State) -> Result<Option<Interval>, Reason> {
        if let Some(Scalar::Local(id)) = &input.scalar {
            if state.facts.value(id.place) == Some(*id) && state.facts.is_reachable() {
                match state.facts.bound(*id, self.limits) {
                    Ok(bound) => return Ok(Some(bound.copy(self.limits)?)),
                    Err(Unknown::MissingBound(_)) => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
        Ok(input
            .range
            .as_ref()
            .map(|b| b.copy(self.limits))
            .transpose()?)
    }
    fn literal_condition(
        &mut self,
        input: &Value,
        literal: &Literal,
        state: &State,
    ) -> Result<Option<Condition>, Reason> {
        if let (Some(Condition::Bool(a)), Literal::Bool(b)) = (&input.condition, literal) {
            return Ok(Some(Condition::Bool(a == b)));
        }
        let constant = match literal {
            Literal::SignedInteger(n) => Integer::from_i128(*n),
            Literal::UnsignedInteger(n) => Integer::from_u128(*n),
            Literal::Integer(text) => self.integer_spelling(text)?,
            _ => return Ok(None),
        };
        Ok(match &input.scalar {
            Some(Scalar::Local(value)) if state.facts.value(value.place) == Some(*value) => {
                Some(Condition::Compare {
                    value: *value,
                    relation: C::Equal,
                    constant,
                })
            }
            Some(Scalar::Constant(value)) => Some(Condition::Bool(
                self.limits.compare(value, &constant)?.is_eq(),
            )),
            _ => None,
        })
    }
    fn fields_pattern(
        &mut self,
        patterns: &[Pattern],
        types: &[Term],
        input: &Value,
        kind: AggregateKind,
        mut state: State,
        depth: usize,
    ) -> Result<(State, State), Reason> {
        if patterns.len() != types.len()
            || input
                .fields
                .as_ref()
                .is_some_and(|fields| fields.len() != patterns.len())
        {
            return Err(Reason::InvalidSignature);
        }
        let mut failure = state.fork(self.limits)?;
        failure
            .facts
            .assume(&Condition::Bool(false), true, self.limits)?;
        for (index, (pattern, ty)) in patterns.iter().zip(types).enumerate() {
            let mut field = match &input.fields {
                Some(fields) => fields[index].snapshot(self.limits, 0)?,
                None => self.unknown_value(ty)?,
            };
            if field.range.is_none() {
                field.range = domain(ty, self.limits)?;
            }
            // Unknown variant alternatives have no materialized payload shape
            // yet. Known constructors and tuples/records can expose identity.
            if !matches!(kind, AggregateKind::Variant(..)) || input.aggregate_kind == Some(kind) {
                self.project_component(
                    &mut state,
                    &mut field,
                    input.projection.clone(),
                    (kind, index),
                )?;
            }
            let (yes, no) = self.pattern(pattern, &field, ty, state, depth)?;
            failure = failure.merge(no, self.limits)?;
            state = yes;
        }
        Ok((state, failure))
    }
    fn pattern(
        &mut self,
        pattern: &Pattern,
        input: &Value,
        ty: &Term,
        mut state: State,
        depth: usize,
    ) -> Result<(State, State), Reason> {
        self.limits.step(1)?;
        if depth >= 128 {
            return Err(Reason::DepthLimit);
        }
        let depth = depth + 1;
        if !state.facts.is_reachable() {
            let other = state.fork(self.limits)?;
            return Ok((state, other));
        }
        match pattern {
            Pattern::Tuple(patterns) => {
                let Some(types) = self.tuple_field_types(ty)? else {
                    return Err(Reason::Unsupported);
                };
                self.fields_pattern(patterns, &types, input, AggregateKind::Tuple, state, depth)
            }
            Pattern::AnonymousRecord { names, fields } => {
                let Term::Record(types) = ty else {
                    return Err(Reason::InvalidSignature);
                };
                if names.len() != fields.len()
                    || names.len() != types.len()
                    || names
                        .iter()
                        .zip(types)
                        .any(|(name, (expected, _))| name != expected)
                {
                    return Err(Reason::InvalidSignature);
                }
                let types = types.iter().map(|(_, ty)| ty.clone()).collect::<Vec<_>>();
                self.fields_pattern(
                    fields,
                    &types,
                    input,
                    AggregateKind::AnonymousRecord,
                    state,
                    depth,
                )
            }
            Pattern::Record {
                declaration,
                fields,
            } => {
                let types = self.nominal_fields(ty, *declaration, None)?;
                if input
                    .aggregate_kind
                    .is_some_and(|kind| kind != AggregateKind::Record(*declaration))
                {
                    return Err(Reason::InvalidSignature);
                }
                self.fields_pattern(
                    fields,
                    &types,
                    input,
                    AggregateKind::Record(*declaration),
                    state,
                    depth,
                )
            }
            Pattern::Variant {
                declaration,
                index,
                fields,
            } => {
                let types = self.nominal_fields(ty, *declaration, Some(*index))?;
                let condition = match input.aggregate_kind {
                    Some(AggregateKind::Variant(actual, variant)) if actual == *declaration => {
                        Some(Condition::Bool(variant == *index))
                    }
                    None => None,
                    _ => return Err(Reason::InvalidSignature),
                };
                let (yes, no) = self.split_condition(state, condition.as_ref())?;
                if !yes.facts.is_reachable() {
                    return Ok((yes, no));
                }
                let (yes, failed_fields) = self.fields_pattern(
                    fields,
                    &types,
                    input,
                    AggregateKind::Variant(*declaration, *index),
                    yes,
                    depth,
                )?;
                Ok((yes, no.merge(failed_fields, self.limits)?))
            }
            Pattern::Wild | Pattern::Unit => {
                self.split_condition(state, Some(&Condition::Bool(true)))
            }
            Pattern::Literal(literal) => {
                let condition = self.literal_condition(input, literal, &state)?;
                self.split_condition(state, condition.as_ref())
            }

            Pattern::Observe { binding, inner } => {
                let snapshot = input.snapshot(self.limits, 0)?;
                bind_value(&mut state, *binding, ty, snapshot, self.limits)?;
                let (mut yes, mut no) = self.pattern(inner, input, ty, state, depth)?;
                for branch in [&mut yes, &mut no] {
                    let retained = branch
                        .domains
                        .keys()
                        .copied()
                        .filter(|id| *id != place(*binding))
                        .collect();
                    branch.keep(&retained, self.limits)?;
                }
                Ok((yes, no))
            }
            Pattern::Binding { binding, inner } => {
                let (mut yes, no) = self.pattern(inner, input, ty, state, depth)?;
                if yes.facts.is_reachable() {
                    let range = self.input_range(input, &yes)?;
                    let mut snapshot = input.snapshot(self.limits, 0)?;
                    snapshot.range = range;
                    let transfer = self.prepare_copy(input, &yes)?;
                    bind_value(&mut yes, *binding, ty, snapshot, self.limits)?;
                    if let Some(transfer) = transfer {
                        let installed = self.install_copy(&mut yes, *binding, transfer)?;
                        self.finish_copies(&mut yes, installed)?;
                    }
                }
                Ok((yes, no))
            }
            Pattern::Guard { pattern, condition } => {
                let (mut yes, no) = self.pattern(pattern, input, ty, state, depth)?;
                let condition = self.child(condition, &mut yes, depth)?.condition;
                let (yes, failed_guard) = self.split_condition(yes, condition.as_ref())?;
                Ok((yes, no.merge(failed_guard, self.limits)?))
            }
            Pattern::Then { first, value, next } => {
                let (mut yes, no) = self.pattern(first, input, ty, state, depth)?;
                let next_input = self.child(value, &mut yes, depth)?;
                let (yes, failed_next) = self.pattern(next, &next_input, &value.ty, yes, depth)?;
                Ok((yes, no.merge(failed_next, self.limits)?))
            }
            Pattern::Or(alternatives) => {
                let existing: BTreeSet<_> = state.domains.keys().copied().collect();
                let mut accepted = state.fork(self.limits)?;
                accepted
                    .facts
                    .assume(&Condition::Bool(false), true, self.limits)?;
                for alternative in alternatives {
                    let (yes, mut no) = self.pattern(alternative, input, ty, state, depth)?;
                    accepted = accepted.merge(yes, self.limits)?;
                    no.keep(&existing, self.limits)?;
                    state = no;
                }
                Ok((accepted, state))
            }
            Pattern::Remap { pattern, bindings } => {
                let (mut yes, no) = self.pattern(pattern, input, ty, state, depth)?;
                if yes.facts.is_reachable() {
                    let mut snapshots = Vec::new();
                    for (from, to) in bindings {
                        self.limits.step(1)?;
                        let source_value = self.local_value(&yes, *from)?;
                        let transfer = self.prepare_copy(&source_value, &yes)?;
                        let from = place(*from);
                        let domain = yes
                            .domains
                            .get(&from)
                            .ok_or(Reason::InvalidSignature)?
                            .as_ref()
                            .map(|b| b.copy(self.limits))
                            .transpose()?;
                        let bound = if let Some(id) = yes.facts.value(from) {
                            match yes.facts.bound(id, self.limits) {
                                Ok(b) => Some(b.copy(self.limits)?),
                                Err(Unknown::MissingBound(_)) => None,
                                Err(e) => return Err(e.into()),
                            }
                        } else {
                            None
                        };
                        let tuple = yes
                            .aggregates
                            .get(&from)
                            .map(|_| source_value.snapshot(self.limits, 0))
                            .transpose()?;
                        snapshots.push((*to, domain, bound, tuple, transfer));
                    }
                    let mut copied_relations = Vec::new();
                    for (binding, domain, bound, tuple, transfer) in snapshots {
                        let to = place(binding);
                        yes.invalidate_paths(Some(to), &[], self.limits)?;
                        if let Some(tuple) = tuple {
                            yes.aggregates.insert(to, tuple);
                        } else {
                            yes.aggregates.remove(&to);
                        }
                        yes.domains.insert(to, domain);
                        if let Some(bound) = bound {
                            yes.facts.assign(to, bound)?;
                        } else {
                            yes.facts.invalidate(to)?;
                        }
                        if let Some(transfer) = transfer {
                            copied_relations
                                .extend(self.install_copy(&mut yes, binding, transfer)?);
                        }
                    }
                    self.finish_copies(&mut yes, copied_relations)?;
                    self.limits
                        .step(yes.domains.len().saturating_add(bindings.len()))?;
                    let destinations: BTreeSet<_> =
                        bindings.iter().map(|(_, to)| place(*to)).collect();
                    let mut retained: BTreeSet<_> = yes.domains.keys().copied().collect();
                    for (from, _) in bindings {
                        let from = place(*from);
                        if !destinations.contains(&from) {
                            retained.remove(&from);
                        }
                    }
                    yes.keep(&retained, self.limits)?;
                }
                Ok((yes, no))
            }
            _ => Err(Reason::Unsupported),
        }
    }
    pub(super) fn let_pattern(
        &mut self,
        pattern: &Pattern,
        value: &Expr,
        guard: Option<&Expr>,
        otherwise: Option<&Expr>,
        state: &mut State,
        depth: usize,
    ) -> Result<(), Reason> {
        let input = self.child(value, state, depth)?;
        let input = self.materialize_aggregate(input, &value.ty, state)?;
        let existing: BTreeSet<_> = state.domains.keys().copied().collect();
        let initial = state.fork(self.limits)?;
        let (mut success, mut failure) =
            self.pattern(pattern, &input, &value.ty, initial, depth)?;
        if let Some(guard) = guard {
            let condition = self.child(guard, &mut success, depth)?.condition;
            let (yes, no) = self.split_condition(success, condition.as_ref())?;
            success = yes;
            failure = failure.merge(no, self.limits)?;
        }
        failure.keep(&existing, self.limits)?;
        if let Some(otherwise) = otherwise {
            self.child(otherwise, &mut failure, depth)?;
            if failure.facts.is_reachable() {
                return Err(Reason::InvalidSignature);
            }
        }
        *state = success;
        Ok(())
    }
    pub(super) fn match_expression(
        &mut self,
        value: &Expr,
        arms: &[MatchArm],
        result_type: &Term,
        state: &mut State,
        depth: usize,
        tail: bool,
    ) -> Result<Value, Reason> {
        self.match_cases(
            value,
            arms.iter().map(MatchCase::Expression),
            result_type,
            state,
            depth,
            tail,
        )
    }

    pub(super) fn normal_match_input(
        &mut self,
        value: &Expr,
        arms: &[nir::program::NormalMatchArm],
        result_type: &Term,
        state: &mut State,
        depth: usize,
    ) -> Result<Value, Reason> {
        self.match_cases(
            value,
            arms.iter().map(MatchCase::Contextual),
            result_type,
            state,
            depth,
            false,
        )
    }

    fn match_cases<'b>(
        &mut self,
        value: &Expr,
        arms: impl Iterator<Item = MatchCase<'b>>,
        result_type: &Term,
        state: &mut State,
        depth: usize,
        tail: bool,
    ) -> Result<Value, Reason> {
        let input = self.child(value, state, depth)?;
        let input = self.materialize_aggregate(input, &value.ty, state)?;
        let existing: BTreeSet<_> = state.domains.keys().copied().collect();
        let result_root = if tail {
            None
        } else {
            self.result_root(result_type)?
        };
        let mut result_scope = existing.clone();
        if tail {
            if let Some(binding) = self.exit_contract.as_ref().and_then(|c| c.binding) {
                result_scope.insert(place(binding));
            }
        }
        if let Some(root) = result_root {
            result_scope.insert(root);
        }
        let mut pending = state.fork(self.limits)?;
        let mut merged = state.fork(self.limits)?;
        merged
            .facts
            .assume(&Condition::Bool(false), true, self.limits)?;
        let mut joined_value = Value::default();
        let mut has_result = false;
        for arm in arms {
            let (pattern, guard) = match &arm {
                MatchCase::Expression(arm) => (&arm.pattern, arm.guard.as_ref()),
                MatchCase::Contextual(arm) => (&arm.pattern, arm.guard.as_ref()),
            };
            let (mut candidate, mut failure) =
                self.pattern(pattern, &input, &value.ty, pending, depth)?;
            if let Some(guard) = guard {
                let condition = self.child(guard, &mut candidate, depth)?.condition;
                let (yes, no) = self.split_condition(candidate, condition.as_ref())?;
                candidate = yes;
                failure = failure.merge(no, self.limits)?;
            }
            let result = match arm {
                MatchCase::Expression(arm) if tail => {
                    self.tail(&arm.body, &mut candidate, depth)?;
                    Value::default()
                }
                MatchCase::Expression(arm) => self.child(&arm.body, &mut candidate, depth)?,
                MatchCase::Contextual(arm) => {
                    self.normal_input_value(
                        &arm.body,
                        Some(result_type),
                        &mut candidate,
                        depth + 1,
                    )?
                    .0
                }
            };
            if candidate.facts.is_reachable() {
                if let Some(root) = result_root {
                    self.bind_result(&mut candidate, root, result_type, result)?;
                } else {
                    joined_value = if !has_result {
                        result
                    } else {
                        joined_value.join(&result, self.limits, 0)?
                    };
                }
                has_result = true;
            }
            candidate.keep(&result_scope, self.limits)?;
            failure.keep(&existing, self.limits)?;
            merged = merged.merge(candidate, self.limits)?;
            pending = failure;
        }
        // Exhaustiveness is checked by elaboration. A residual failure path
        // would terminate with NonExhaustiveMatch, never yield a normal result.
        *state = merged;
        if let Some(root) = result_root {
            if has_result {
                return self.local_value_at(state, root);
            }
        }
        Ok(joined_value)
    }
}

impl Walker<'_> {
    /// Validate branches at an arbitrary request point. The handled body may
    /// mutate captures before any request, and may request zero or many times.
    /// Branch facts therefore start from declared domains and never become
    /// unconditional facts after the installation.
    pub(super) fn effect_handler_arms(
        &mut self,
        handler: &nir::program::EffectHandler,
        state: &State,
        depth: usize,
    ) -> Result<(), Reason> {
        let Term::Function { parameters, .. } = &handler.signature else {
            return Err(Reason::InvalidSignature);
        };
        let ty = Term::Tuple(parameters.iter().map(|p| p.ty.clone()).collect());
        let mut pending = state.fork(self.limits)?;
        pending.havoc(self.limits)?;
        let input = self.unknown_value(&ty)?;
        let input = self.materialize_aggregate(input, &ty, &mut pending)?;
        let existing: BTreeSet<_> = pending.domains.keys().copied().collect();
        for arm in &handler.arms {
            let (mut candidate, mut failure) =
                self.pattern(&arm.pattern, &input, &ty, pending, depth)?;
            if let Some(guard) = &arm.guard {
                let condition = self.child(guard, &mut candidate, depth)?.condition;
                let (yes, no) = self.split_condition(candidate, condition.as_ref())?;
                candidate = yes;
                failure = failure.merge(no, self.limits)?;
            }
            self.child(&arm.body, &mut candidate, depth)?;
            failure.keep(&existing, self.limits)?;
            pending = failure;
        }
        // Any residual path uses the separately checked outer fallback.
        Ok(())
    }
}
