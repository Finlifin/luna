//! Scalar pattern candidates, ordered failure continuation and guard effects.
use super::*;
use nir::{
    DeclarationId, Primitive,
    program::{MatchArm, Pattern},
};
use std::collections::BTreeSet;

#[derive(Clone, Copy)]
enum MatchCase<'a> {
    Expression(&'a MatchArm),
    Contextual(&'a nir::program::NormalMatchArm),
}

impl<'a> MatchCase<'a> {
    fn parts(&self) -> (&'a Pattern, [Option<&'a Expr>; 2]) {
        let (pattern, guard) = match self {
            Self::Expression(arm) => (&arm.pattern, arm.guard.as_ref()),
            Self::Contextual(arm) => (&arm.pattern, arm.guard.as_ref()),
        };
        match pattern {
            Pattern::Guard { pattern, condition } => (pattern, [Some(condition), guard]),
            _ => (pattern, [None, guard]),
        }
    }
}

fn conjunction(a: Condition, b: Condition) -> Condition {
    match (a, b) {
        (Condition::Bool(false), _) | (_, Condition::Bool(false)) => Condition::Bool(false),
        (Condition::Bool(true), b) => b,
        (a, Condition::Bool(true)) => a,
        (a, b) => Condition::And(Box::new(a), Box::new(b)),
    }
}

fn disjunction(a: Condition, b: Condition) -> Condition {
    match (a, b) {
        (Condition::Bool(true), _) | (_, Condition::Bool(true)) => Condition::Bool(true),
        (Condition::Bool(false), b) => b,
        (a, Condition::Bool(false)) => a,
        (a, b) => Condition::Or(Box::new(a), Box::new(b)),
    }
}

fn negation(condition: Condition) -> Condition {
    match condition {
        Condition::Bool(value) => Condition::Bool(!value),
        Condition::Not(inner) => *inner,
        condition => Condition::Not(Box::new(condition)),
    }
}

impl State {
    pub(super) fn merge(mut self, other: Self, limits: &mut Limits) -> Result<Self, Unknown> {
        if !self.facts.shares_lineage(&other.facts) {
            return Err(Unknown::UnrelatedFacts);
        }
        if !self.facts.is_reachable() {
            limits.step(1)?;
            return Ok(other);
        }
        if !other.facts.is_reachable() {
            limits.step(1)?;
            return Ok(self);
        }
        self.join_aggregates(&other, limits)?;
        self.join_paths_and_facts(&other, limits)?;
        self.domains.extend(other.domains);
        Ok(self)
    }

    pub(super) fn keep(
        &mut self,
        existing: &BTreeSet<PlaceId>,
        limits: &mut Limits,
    ) -> Result<(), Unknown> {
        limits.step(
            self.domains
                .len()
                .saturating_add(self.reads.len())
                .saturating_add(self.read_aliases.len())
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
        self.read_aliases.retain(|alias, (path, _, _)| {
            retained.contains(alias) && retained.contains(&path.root)
        });
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
        match condition {
            Some(Condition::Bool(true)) => {
                let failure = state.unreachable(self.limits)?;
                return Ok((state, failure));
            }
            Some(Condition::Bool(false)) => {
                let unreachable = state.unreachable(self.limits)?;
                return Ok((unreachable, state));
            }
            _ => {}
        }
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
            Literal::Bool(value) => Integer::from_u128(u128::from(*value)),
            Literal::Char(value) => Integer::from_u128(u128::from(u32::from(*value))),
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

    fn pattern_selection_condition(
        &mut self,
        pattern: &Pattern,
        value: &Value,
        ty: &Term,
        state: &mut State,
        depth: usize,
    ) -> Result<Option<Condition>, Reason> {
        self.limits.step(1)?;
        if depth >= 128 {
            return Err(Reason::DepthLimit);
        }
        let (patterns, types, kind, mut selected): (
            &[Pattern],
            Vec<Term>,
            AggregateKind,
            Condition,
        ) = match pattern {
            Pattern::Wild | Pattern::Unit => return Ok(Some(Condition::Bool(true))),
            Pattern::Binding { inner, .. } => {
                return self.pattern_selection_condition(inner, value, ty, state, depth + 1);
            }
            Pattern::Remap { pattern, .. } => {
                // Binder renaming does not change which input values match.
                return self.pattern_selection_condition(pattern, value, ty, state, depth + 1);
            }
            Pattern::Or(alternatives) => {
                let mut selected = Condition::Bool(false);
                for alternative in alternatives {
                    let Some(condition) =
                        self.pattern_selection_condition(alternative, value, ty, state, depth + 1)?
                    else {
                        return Ok(None);
                    };
                    selected = disjunction(selected, condition);
                }
                return Ok(Some(selected));
            }
            Pattern::Guard { pattern, condition } => {
                if !nir::program::checkable_predicate(condition, &mut self.limits.arithmetic.steps)
                    .map_err(|_| Unknown::Arithmetic(comptime_runtime::Error::BudgetExhausted))?
                {
                    return Ok(None);
                }
                let Some(selected) =
                    self.pattern_selection_condition(pattern, value, ty, state, depth + 1)?
                else {
                    return Ok(None);
                };
                // Bind guard inputs in an isolated successful candidate. Export
                // only relations to the original observations, never its locals
                // or facts assumed solely by taking this candidate.
                let existing = state.domains.keys().copied().collect();
                let candidate = state.fork(self.limits)?;
                let (mut candidate, _) = self.pattern(pattern, value, ty, candidate, depth + 1)?;
                if !candidate.facts.is_reachable() {
                    return Ok(Some(Condition::Bool(false)));
                }
                let Some(guard) = self.child(condition, &mut candidate, depth + 1)?.condition
                else {
                    return Ok(None);
                };
                let guard = self.scoped_condition(&guard, &candidate, &existing)?;
                return Ok(Some(conjunction(selected, guard)));
            }
            Pattern::Literal(literal) => return self.literal_condition(value, literal, state),
            Pattern::Tuple(patterns) => {
                let Some(types) = self.tuple_field_types(ty)? else {
                    return Ok(None);
                };
                (patterns, types, AggregateKind::Tuple, Condition::Bool(true))
            }
            Pattern::Record {
                declaration,
                fields,
            } => (
                fields,
                self.nominal_fields(ty, *declaration, None)?,
                AggregateKind::Record(*declaration),
                Condition::Bool(true),
            ),
            Pattern::AnonymousRecord { names, fields } => {
                let Term::Record(types) = ty else {
                    return Err(Reason::InvalidSignature);
                };
                if names.len() != types.len()
                    || names
                        .iter()
                        .zip(types)
                        .any(|(name, (expected, _))| name != expected)
                {
                    return Err(Reason::InvalidSignature);
                }
                (
                    fields,
                    types.iter().map(|(_, ty)| ty.clone()).collect(),
                    AggregateKind::AnonymousRecord,
                    Condition::Bool(true),
                )
            }
            Pattern::ErrorResult { label, inner } => {
                let tag = self.error_pattern_tag(ty, label.as_ref())?;
                let selected = self.error_condition(value, ty, tag, state)?;
                let Some(selected) = selected else {
                    return Ok(None);
                };
                if selected == Condition::Bool(false) {
                    return Ok(Some(selected));
                }
                (
                    std::slice::from_ref(inner.as_ref()),
                    vec![self.error_payload_type(ty, tag)?.clone()],
                    AggregateKind::ErrorResult(tag),
                    selected,
                )
            }
            Pattern::OptionalAbsent | Pattern::OptionalPresent(_) => {
                let Term::Optional(payload) = ty else {
                    return Err(Reason::InvalidSignature);
                };
                let present = matches!(pattern, Pattern::OptionalPresent(_));
                let Some(selected) = self.optional_condition(value, present, state)? else {
                    return Ok(None);
                };
                if selected == Condition::Bool(false) || !present {
                    return Ok(Some(selected));
                }
                let Pattern::OptionalPresent(inner) = pattern else {
                    unreachable!()
                };
                (
                    std::slice::from_ref(inner.as_ref()),
                    vec![(**payload).clone()],
                    AggregateKind::Optional(true),
                    selected,
                )
            }
            Pattern::Variant {
                declaration,
                index,
                fields,
            } => {
                let Some(selected) = self.variant_condition(value, *declaration, *index, state)?
                else {
                    return Ok(None);
                };
                // Inactive variants have no payload of this layout to inspect.
                if selected == Condition::Bool(false) {
                    return Ok(Some(selected));
                }
                (
                    fields,
                    self.nominal_fields(ty, *declaration, Some(*index))?,
                    AggregateKind::Variant(*declaration, *index),
                    selected,
                )
            }
            _ => return Ok(None),
        };
        if patterns.len() != types.len()
            || value
                .fields
                .as_ref()
                .is_some_and(|fields| fields.len() != types.len())
        {
            return Err(Reason::InvalidSignature);
        }
        let existing = state.domains.keys().copied().collect();
        let mut candidate = state.fork(self.limits)?;
        candidate.facts.assume(&selected, true, self.limits)?;
        for (slot, (pattern, ty)) in patterns.iter().zip(&types).enumerate() {
            if !candidate.facts.is_reachable() {
                return Ok(Some(Condition::Bool(false)));
            }
            let mut field = match value.fields.as_ref().and_then(|fields| fields.get(slot)) {
                Some(field) => field.snapshot(self.limits, 0)?,
                None => self.unknown_value(ty)?,
            };
            self.project_component(
                &mut candidate,
                &mut field,
                value.projection.clone(),
                (kind, slot),
            )?;
            let Some(condition) =
                self.pattern_selection_condition(pattern, &field, ty, &mut candidate, depth + 1)?
            else {
                return Ok(None);
            };
            // Export before assuming this field's guard: its own equalities
            // must not justify rewriting the predicate we are constructing.
            let condition = self.scoped_condition(&condition, &candidate, &existing)?;
            selected = conjunction(selected, condition);
            // Later fields see earlier bindings only along this successful
            // candidate. Keep the selection formula separate from those facts.
            (candidate, _) = self.pattern(pattern, &field, ty, candidate, depth + 1)?;
        }
        Ok(Some(selected))
    }

    fn optional_condition(
        &mut self,
        input: &Value,
        present: bool,
        state: &mut State,
    ) -> Result<Option<Condition>, Reason> {
        match input.aggregate_kind {
            Some(AggregateKind::Optional(actual)) => Ok(Some(Condition::Bool(actual == present))),
            None => {
                let mut tag = Value {
                    range: Some(Interval::new(
                        Integer::from_u128(0),
                        Integer::from_u128(1),
                        self.limits,
                    )?),
                    ..Value::default()
                };
                self.project_component(
                    state,
                    &mut tag,
                    input.projection.clone(),
                    (AggregateKind::OptionalTag, 0),
                )?;
                Ok(match tag.scalar {
                    Some(Scalar::Local(value)) => Some(Condition::Compare {
                        value,
                        relation: crate::Comparison::Equal,
                        constant: Integer::from_u128(u128::from(present)),
                    }),
                    _ => None,
                })
            }
            _ => Err(Reason::InvalidSignature),
        }
    }

    fn variant_condition(
        &mut self,
        input: &Value,
        declaration: nir::DeclarationId,
        index: usize,
        state: &mut State,
    ) -> Result<Option<Condition>, Reason> {
        Ok(match input.aggregate_kind {
            Some(AggregateKind::Variant(actual, variant)) if actual == declaration => {
                Some(Condition::Bool(variant == index))
            }
            None => {
                let package = self.package.ok_or(Reason::Unsupported)?;
                self.limits.step(package.enums.len())?;
                let variants = package
                    .enums
                    .iter()
                    .find(|definition| definition.declaration == declaration)
                    .ok_or(Reason::InvalidSignature)?
                    .variants
                    .len();
                let upper = variants.checked_sub(1).ok_or(Reason::InvalidSignature)?;
                let mut tag = Value {
                    range: Some(Interval::new(
                        Integer::from_u128(0),
                        Integer::from_u128(upper as u128),
                        self.limits,
                    )?),
                    ..Value::default()
                };
                self.project_component(
                    state,
                    &mut tag,
                    input.projection.clone(),
                    (AggregateKind::EnumTag(declaration), 0),
                )?;
                match tag.scalar {
                    Some(Scalar::Local(value)) => Some(Condition::Compare {
                        value,
                        relation: crate::Comparison::Equal,
                        constant: Integer::from_u128(index as u128),
                    }),
                    _ => None,
                }
            }
            _ => return Err(Reason::InvalidSignature),
        })
    }

    fn prime_payload_observations(
        &mut self,
        value: &mut Value,
        state: &mut State,
        ancestors: &mut Vec<nir::DeclarationId>,
        depth: usize,
    ) -> Result<(), Reason> {
        self.limits.step(ancestors.len().saturating_add(1))?;
        if depth >= 128 {
            return Err(Reason::DepthLimit);
        }
        if let Some(ty) = value.nominal_type.clone() {
            if let Term::ErrorResult { errors, .. } = ty.as_ref() {
                self.error_condition(value, &ty, 0, state)?;
                for tag in 0..=errors.len() {
                    self.limits.step(1)?;
                    if value
                        .aggregate_kind
                        .is_some_and(|kind| kind != AggregateKind::ErrorResult(tag))
                    {
                        continue;
                    }
                    let payload = self.error_payload_type(&ty, tag)?.clone();
                    let mut field = match value.fields.as_ref().and_then(|fields| fields.first()) {
                        Some(field) => field.snapshot(self.limits, 0)?,
                        None => self.unknown_value(&payload)?,
                    };
                    self.project_component(
                        state,
                        &mut field,
                        value.projection.clone(),
                        (AggregateKind::ErrorResult(tag), 0),
                    )?;
                    self.prime_payload_observations(&mut field, state, ancestors, depth + 1)?;
                }
                return Ok(());
            }
            if let Term::Optional(payload) = ty.as_ref() {
                self.optional_condition(value, true, state)?;
                if value.aggregate_kind != Some(AggregateKind::Optional(false)) {
                    let mut field = match value.fields.as_ref().and_then(|fields| fields.first()) {
                        Some(field) => field.snapshot(self.limits, 0)?,
                        None => self.unknown_value(payload)?,
                    };
                    self.project_component(
                        state,
                        &mut field,
                        value.projection.clone(),
                        (AggregateKind::Optional(true), 0),
                    )?;
                    self.prime_payload_observations(&mut field, state, ancestors, depth + 1)?;
                }
                return Ok(());
            }
        }
        self.limits
            .step(self.package.map_or(0, |package| package.enums.len()))?;
        let enumeration = value.nominal_type.as_deref().and_then(|ty| {
            let Term::Nominal { declaration, .. } = ty else {
                return None;
            };
            self.package?
                .enums
                .iter()
                .find(|definition| definition.declaration == *declaration)
                .map(|definition| (*declaration, definition.variants.len()))
        });
        if let Some((declaration, count)) = enumeration {
            if count == 0 {
                return Ok(());
            }
            self.variant_condition(value, declaration, 0, state)?;
            // Unknown recursive layouts do not authorize infinite unfolding.
            // Publish this tag, then leave deeper payloads to later matches.
            if ancestors.contains(&declaration) {
                return Ok(());
            }
            ancestors.push(declaration);
            let ty = value.nominal_type.clone().ok_or(Reason::InvalidSignature)?;
            for index in 0..count {
                self.limits.step(1)?;
                if value
                    .aggregate_kind
                    .is_some_and(|kind| kind != AggregateKind::Variant(declaration, index))
                {
                    continue;
                }
                let fields = self.nominal_fields(&ty, declaration, Some(index))?;
                for (slot, ty) in fields.iter().enumerate() {
                    let mut field = match value.fields.as_ref().and_then(|fields| fields.get(slot))
                    {
                        Some(field) => field.snapshot(self.limits, 0)?,
                        None => self.unknown_value(ty)?,
                    };
                    self.project_component(
                        state,
                        &mut field,
                        value.projection.clone(),
                        (AggregateKind::Variant(declaration, index), slot),
                    )?;
                    self.prime_payload_observations(&mut field, state, ancestors, depth + 1)?;
                }
            }
            ancestors.pop();
            return Ok(());
        }
        self.expand_owned_shape(value, depth)?;
        if let (Some(kind), Some(fields)) = (value.aggregate_kind, &mut value.fields) {
            for (slot, field) in fields.iter_mut().enumerate() {
                self.project_component(state, field, value.projection.clone(), (kind, slot))?;
                self.prime_payload_observations(field, state, ancestors, depth + 1)?;
            }
        }
        Ok(())
    }

    #[inline(never)]
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
        let mut failure = state.unreachable(self.limits)?;
        for (index, (pattern, ty)) in patterns.iter().zip(types).enumerate() {
            let mut field = match &input.fields {
                Some(fields) => fields[index].snapshot(self.limits, 0)?,
                None => self.unknown_value(ty)?,
            };
            if field.range.is_none() {
                field.range = domain_in(ty, self.target(), self.limits)?;
            }
            // Variant-qualified paths keep different payloads distinct even
            // when the active tag was not known before entering this arm.
            self.project_component(
                &mut state,
                &mut field,
                input.projection.clone(),
                (kind, index),
            )?;
            let (yes, no) = self.pattern(pattern, &field, ty, state, depth)?;
            failure = failure.merge(no, self.limits)?;
            state = yes;
        }
        Ok((state, failure))
    }

    #[inline(never)]
    fn pattern(
        &mut self,
        pattern: &Pattern,
        input: &Value,
        ty: &Term,
        state: State,
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
            Pattern::NominalType {
                declaration,
                fields,
            } => self.pattern_nominal_type(*declaration, fields, ty, state, depth),
            Pattern::NormalForm { head, arguments } => {
                self.pattern_normal_form(*head, arguments, ty, state, depth)
            }
            Pattern::TypeTuple(fields) => self.pattern_type_tuple(fields, ty, state, depth),
            Pattern::Tuple(patterns) => self.pattern_tuple(patterns, input, ty, state, depth),
            Pattern::AnonymousRecord { names, fields } => {
                self.pattern_anonymous_record(names, fields, input, ty, state, depth)
            }
            Pattern::Record {
                declaration,
                fields,
            } => self.pattern_record(*declaration, fields, input, ty, state, depth),
            Pattern::ErrorResult { label, inner } => {
                self.pattern_error_result(label.as_ref(), inner, input, ty, state, depth)
            }
            Pattern::OptionalAbsent | Pattern::OptionalPresent(_) => {
                self.pattern_optional(pattern, input, ty, state, depth)
            }
            Pattern::Variant {
                declaration,
                index,
                fields,
            } => self.pattern_variant(*declaration, *index, fields, input, ty, state, depth),
            Pattern::Wild | Pattern::Unit => {
                let failure = state.unreachable(self.limits)?;
                Ok((state, failure))
            }
            Pattern::Literal(literal) => self.pattern_literal(literal, input, state),
            Pattern::Observe { binding, inner } => {
                self.pattern_observe(*binding, inner, input, ty, state, depth)
            }
            Pattern::Binding { binding, inner } => {
                self.pattern_binding(*binding, inner, input, ty, state, depth)
            }
            Pattern::Guard { pattern, condition } => {
                self.pattern_guard(pattern, condition, input, ty, state, depth)
            }
            Pattern::Then { first, value, next } => {
                self.pattern_then(first, value, next, input, ty, state, depth)
            }
            Pattern::Or(alternatives) => self.pattern_or(alternatives, input, ty, state, depth),
            Pattern::Remap { pattern, bindings } => {
                self.pattern_remap(pattern, bindings, input, ty, state, depth)
            }
            _ => Err(Reason::Unsupported),
        }
    }

    #[inline(never)]
    fn pattern_nominal_type(
        &mut self,
        declaration: DeclarationId,
        fields: &[Pattern],
        ty: &Term,
        mut state: State,
        depth: usize,
    ) -> Result<(State, State), Reason> {
        if *ty != Term::Primitive(Primitive::Type) {
            return Err(Reason::InvalidSignature);
        }
        let package = self.package.ok_or(Reason::Unsupported)?;
        self.limits.step(
            package
                .structs
                .len()
                .saturating_add(package.enums.len())
                .saturating_add(package.newtypes.len()),
        )?;
        let (parameters, domains) = if let Some(definition) = package
            .structs
            .iter()
            .find(|definition| definition.declaration == declaration)
        {
            (&definition.type_parameters, &definition.parameter_types)
        } else if let Some(definition) = package
            .enums
            .iter()
            .find(|definition| definition.declaration == declaration)
        {
            (&definition.type_parameters, &definition.parameter_types)
        } else if let Some(definition) = package
            .newtypes
            .iter()
            .find(|definition| definition.declaration == declaration)
        {
            (&definition.type_parameters, &definition.parameter_types)
        } else {
            return Err(Reason::InvalidSignature);
        };
        if fields.len() != parameters.len()
            || (!domains.is_empty() && domains.len() != parameters.len())
        {
            return Err(Reason::InvalidSignature);
        }
        // These are constructor inputs, not runtime record fields.
        // Empty domains are the compact all-Type signature. Captured
        // parameters are included in the declaration telescope.
        let mut failure = state.fork(self.limits)?;
        for (index, field) in fields.iter().enumerate() {
            let domain = domains
                .get(index)
                .unwrap_or(&Term::Primitive(Primitive::Type));
            let value = self.unknown_value(domain)?;
            let (yes, no) = self.pattern(field, &value, domain, state, depth)?;
            failure = failure.merge(no, self.limits)?;
            state = yes;
        }
        Ok((state, failure))
    }

    #[inline(never)]
    fn pattern_normal_form(
        &mut self,
        head: DeclarationId,
        arguments: &[Pattern],
        ty: &Term,
        mut state: State,
        depth: usize,
    ) -> Result<(State, State), Reason> {
        if *ty != Term::Primitive(Primitive::Type) {
            return Err(Reason::InvalidSignature);
        }
        let package = self.package.ok_or(Reason::Unsupported)?;
        self.limits.step(package.normal_forms.len())?;
        let signature = package
            .normal_forms
            .iter()
            .find(|signature| signature.declaration == head)
            .ok_or(Reason::InvalidSignature)?;
        if signature.parameters.len() != arguments.len() {
            return Err(Reason::InvalidSignature);
        }
        // Matching a Type observes retained inputs, never the head's
        // body. Without a known subject, both head outcomes remain
        // possible. Keep failure effects in declaration order, just
        // as for runtime aggregate patterns.
        let mut failure = state.fork(self.limits)?;
        for (argument, parameter) in arguments.iter().zip(&signature.parameters) {
            let value = self.unknown_value(&parameter.ty)?;
            let (yes, no) = self.pattern(argument, &value, &parameter.ty, state, depth)?;
            failure = failure.merge(no, self.limits)?;
            state = yes;
        }
        Ok((state, failure))
    }

    #[inline(never)]
    fn pattern_type_tuple(
        &mut self,
        fields: &[Pattern],
        ty: &Term,
        mut state: State,
        depth: usize,
    ) -> Result<(State, State), Reason> {
        if *ty != Term::Primitive(Primitive::Type) {
            return Err(Reason::InvalidSignature);
        }
        let mut failure = state.fork(self.limits)?;
        for field in fields {
            let value = self.unknown_value(ty)?;
            let (yes, no) = self.pattern(field, &value, ty, state, depth)?;
            failure = failure.merge(no, self.limits)?;
            state = yes;
        }
        Ok((state, failure))
    }

    #[inline(never)]
    fn pattern_tuple(
        &mut self,
        patterns: &[Pattern],
        input: &Value,
        ty: &Term,
        state: State,
        depth: usize,
    ) -> Result<(State, State), Reason> {
        let Some(types) = self.tuple_field_types(ty)? else {
            return Err(Reason::Unsupported);
        };
        self.fields_pattern(patterns, &types, input, AggregateKind::Tuple, state, depth)
    }

    #[inline(never)]
    fn pattern_anonymous_record(
        &mut self,
        names: &[String],
        fields: &[Pattern],
        input: &Value,
        ty: &Term,
        state: State,
        depth: usize,
    ) -> Result<(State, State), Reason> {
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

    #[inline(never)]
    fn pattern_record(
        &mut self,
        declaration: DeclarationId,
        fields: &[Pattern],
        input: &Value,
        ty: &Term,
        state: State,
        depth: usize,
    ) -> Result<(State, State), Reason> {
        let types = self.nominal_fields(ty, declaration, None)?;
        if input
            .aggregate_kind
            .is_some_and(|kind| kind != AggregateKind::Record(declaration))
        {
            return Err(Reason::InvalidSignature);
        }
        self.fields_pattern(
            fields,
            &types,
            input,
            AggregateKind::Record(declaration),
            state,
            depth,
        )
    }

    #[inline(never)]
    fn pattern_error_result(
        &mut self,
        label: Option<&Term>,
        inner: &Pattern,
        input: &Value,
        ty: &Term,
        mut state: State,
        depth: usize,
    ) -> Result<(State, State), Reason> {
        let tag = self.error_pattern_tag(ty, label)?;
        let condition = self.error_condition(input, ty, tag, &mut state)?;
        let (yes, no) = self.split_condition(state, condition.as_ref())?;
        if !yes.facts.is_reachable() {
            return Ok((yes, no));
        }
        let payload = self.error_payload_type(ty, tag)?.clone();
        let (yes, failed) = self.fields_pattern(
            std::slice::from_ref(inner),
            std::slice::from_ref(&payload),
            input,
            AggregateKind::ErrorResult(tag),
            yes,
            depth,
        )?;
        Ok((yes, no.merge(failed, self.limits)?))
    }

    #[inline(never)]
    fn pattern_optional(
        &mut self,
        pattern: &Pattern,
        input: &Value,
        ty: &Term,
        mut state: State,
        depth: usize,
    ) -> Result<(State, State), Reason> {
        let Term::Optional(payload) = ty else {
            return Err(Reason::InvalidSignature);
        };
        let present = matches!(pattern, Pattern::OptionalPresent(_));
        let condition = self.optional_condition(input, present, &mut state)?;
        let (yes, no) = self.split_condition(state, condition.as_ref())?;
        if !yes.facts.is_reachable() || !present {
            return Ok((yes, no));
        }
        let Pattern::OptionalPresent(inner) = pattern else {
            unreachable!()
        };
        let (yes, failed) = self.fields_pattern(
            std::slice::from_ref(inner.as_ref()),
            std::slice::from_ref(payload.as_ref()),
            input,
            AggregateKind::Optional(true),
            yes,
            depth,
        )?;
        Ok((yes, no.merge(failed, self.limits)?))
    }

    #[inline(never)]
    #[allow(clippy::too_many_arguments)]
    fn pattern_variant(
        &mut self,
        declaration: DeclarationId,
        index: usize,
        fields: &[Pattern],
        input: &Value,
        ty: &Term,
        mut state: State,
        depth: usize,
    ) -> Result<(State, State), Reason> {
        let types = self.nominal_fields(ty, declaration, Some(index))?;
        let condition = self.variant_condition(input, declaration, index, &mut state)?;
        let (yes, no) = self.split_condition(state, condition.as_ref())?;
        if !yes.facts.is_reachable() {
            return Ok((yes, no));
        }
        let (yes, failed_fields) = self.fields_pattern(
            fields,
            &types,
            input,
            AggregateKind::Variant(declaration, index),
            yes,
            depth,
        )?;
        Ok((yes, no.merge(failed_fields, self.limits)?))
    }

    #[inline(never)]
    fn pattern_literal(
        &mut self,
        literal: &nir::program::Literal,
        input: &Value,
        state: State,
    ) -> Result<(State, State), Reason> {
        let condition = self.literal_condition(input, literal, &state)?;
        self.split_condition(state, condition.as_ref())
    }

    #[inline(never)]
    fn pattern_observe(
        &mut self,
        binding: nir::program::LocalId,
        inner: &Pattern,
        input: &Value,
        ty: &Term,
        mut state: State,
        depth: usize,
    ) -> Result<(State, State), Reason> {
        let snapshot = input.snapshot(self.limits, 0)?;
        bind_value(&mut state, binding, ty, snapshot, self.limits)?;
        let (mut yes, mut no) = self.pattern(inner, input, ty, state, depth)?;
        for branch in [&mut yes, &mut no] {
            let retained = branch
                .domains
                .keys()
                .copied()
                .filter(|id| *id != place(binding))
                .collect();
            branch.keep(&retained, self.limits)?;
        }
        Ok((yes, no))
    }

    #[inline(never)]
    fn pattern_binding(
        &mut self,
        binding: nir::program::LocalId,
        inner: &Pattern,
        input: &Value,
        ty: &Term,
        state: State,
        depth: usize,
    ) -> Result<(State, State), Reason> {
        let (mut yes, no) = self.pattern(inner, input, ty, state, depth)?;
        if yes.facts.is_reachable() {
            let range = self.input_range(input, &yes)?;
            let mut snapshot = input.snapshot(self.limits, 0)?;
            snapshot.range = range;
            let immutable = input
                .projection
                .as_ref()
                .filter(|projection| {
                    input.nominal_type.is_some()
                        && !self.written_locals.contains(&binding)
                        && !self.addressed_locals.contains(&binding)
                        && yes.frozen.contains_key(&projection.path.root)
                        && yes.facts.value(projection.path.root) == Some(projection.root_version)
                })
                .cloned();
            let transfer = if immutable.is_none() {
                self.prepare_copy(input, &yes)?
            } else {
                None
            };
            bind_value(&mut yes, binding, ty, snapshot, self.limits)?;
            if let Some(projection) = immutable {
                self.limits
                    .step(projection.path.fields.len().saturating_add(1))?;
                yes.aggregates
                    .get_mut(&place(binding))
                    .ok_or(Reason::InvalidSignature)?
                    .immutable_projection = Some((yes.frame, projection));
            }
            if let Some(transfer) = transfer {
                let installed = self.install_copy(&mut yes, binding, transfer)?;
                self.finish_copies(&mut yes, installed)?;
            }
        }
        Ok((yes, no))
    }

    #[inline(never)]
    fn pattern_guard(
        &mut self,
        pattern: &Pattern,
        condition: &Expr,
        input: &Value,
        ty: &Term,
        state: State,
        depth: usize,
    ) -> Result<(State, State), Reason> {
        let (mut yes, no) = self.pattern(pattern, input, ty, state, depth)?;
        let condition = self.child(condition, &mut yes, depth)?.condition;
        let (yes, failed_guard) = self.split_condition(yes, condition.as_ref())?;
        Ok((yes, no.merge(failed_guard, self.limits)?))
    }

    #[inline(never)]
    #[allow(clippy::too_many_arguments)]
    fn pattern_then(
        &mut self,
        first: &Pattern,
        value: &Expr,
        next: &Pattern,
        input: &Value,
        ty: &Term,
        state: State,
        depth: usize,
    ) -> Result<(State, State), Reason> {
        let (mut yes, no) = self.pattern(first, input, ty, state, depth)?;
        let next_input = self.child(value, &mut yes, depth)?;
        let (yes, failed_next) = self.pattern(next, &next_input, &value.ty, yes, depth)?;
        Ok((yes, no.merge(failed_next, self.limits)?))
    }

    #[inline(never)]
    fn pattern_or(
        &mut self,
        alternatives: &[Pattern],
        input: &Value,
        ty: &Term,
        mut state: State,
        depth: usize,
    ) -> Result<(State, State), Reason> {
        let existing: BTreeSet<_> = state.domains.keys().copied().collect();
        let mut accepted = state.unreachable(self.limits)?;
        for alternative in alternatives {
            let (yes, mut no) = self.pattern(alternative, input, ty, state, depth)?;
            accepted = accepted.merge(yes, self.limits)?;
            no.keep(&existing, self.limits)?;
            state = no;
        }
        Ok((accepted, state))
    }

    #[inline(never)]
    fn pattern_remap(
        &mut self,
        pattern: &Pattern,
        bindings: &[(nir::program::LocalId, nir::program::LocalId)],
        input: &Value,
        ty: &Term,
        state: State,
        depth: usize,
    ) -> Result<(State, State), Reason> {
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
                    copied_relations.extend(self.install_copy(&mut yes, binding, transfer)?);
                }
            }
            self.finish_copies(&mut yes, copied_relations)?;
            self.limits
                .step(yes.domains.len().saturating_add(bindings.len()))?;
            let destinations: BTreeSet<_> = bindings.iter().map(|(_, to)| place(*to)).collect();
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

    #[inline(never)]
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
        let collected = self.collect_match_cases(arms)?;
        let (compose_tags, tag_conditions) =
            self.prepare_match_tags(&collected, &input, &value.ty, result_type, state, tail)?;
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
        let pending = state.fork(self.limits)?;
        let merged = state.unreachable(self.limits)?;
        let mut join_state = MatchCasesState {
            pending: Some(pending),
            merged: Some(merged),
            joined_value: Value::default(),
            has_result: false,
            sole_condition: None,
            tagged_condition: Condition::Bool(false),
            remaining_condition: Condition::Bool(true),
            alternative_remaining: Condition::Bool(true),
            arm_success: Condition::Bool(false),
            guard_failures: None,
            compose_tags,
        };
        for ((arm, pattern, guards, last), tag) in collected.into_iter().zip(tag_conditions) {
            self.eval_match_arm(
                arm,
                pattern,
                guards,
                last,
                tag,
                &input,
                &value.ty,
                result_type,
                &existing,
                &result_scope,
                result_root,
                depth,
                tail,
                &mut join_state,
            )?;
        }
        // Exhaustiveness is checked by elaboration. A residual failure path
        // would terminate with NonExhaustiveMatch, never yield a normal result.
        *state = join_state.merged.take().expect("match cases merged state");
        self.finish_match_cases(
            state,
            result_root,
            join_state.has_result,
            join_state.compose_tags,
            join_state.tagged_condition,
            join_state.sole_condition,
            join_state.joined_value,
        )
    }

    #[inline(never)]
    fn collect_match_cases<'b>(
        &mut self,
        arms: impl Iterator<Item = MatchCase<'b>>,
    ) -> Result<Vec<CollectedMatchCase<'b>>, Reason> {
        let mut collected = Vec::new();
        for arm in arms {
            self.limits.step(1)?;
            let (pattern, guards) = arm.parts();
            if let Pattern::Or(alternatives) = pattern {
                // Keep the original arm boundary: a failed outer guard skips
                // all remaining alternatives, while pattern failure retries.
                for (index, alternative) in alternatives.iter().enumerate() {
                    self.limits.step(1)?;
                    collected.push((arm, alternative, guards, index + 1 == alternatives.len()));
                }
            } else {
                collected.push((arm, pattern, guards, true));
            }
        }
        Ok(collected)
    }

    #[inline(never)]
    fn prepare_match_tags<'b>(
        &mut self,
        collected: &[CollectedMatchCase<'b>],
        input: &Value,
        value_ty: &Term,
        result_type: &Term,
        state: &mut State,
        tail: bool,
    ) -> Result<(bool, Vec<Option<Condition>>), Reason> {
        let mut tag_conditions = Vec::new();
        let mut prepared_structural_input = false;
        let mut compose_tags = !tail && *result_type == Term::Primitive(nir::Primitive::Bool);
        for (_, pattern, guards, _) in collected {
            let pattern = *pattern;
            for guard in guards.iter().flatten() {
                if !nir::program::checkable_predicate(guard, &mut self.limits.arithmetic.steps)
                    .map_err(|_| Unknown::Arithmetic(comptime_runtime::Error::BudgetExhausted))?
                {
                    compose_tags = false;
                    break;
                }
            }
            if let Pattern::Variant {
                declaration, index, ..
            } = pattern
            {
                let mut tag = self.variant_condition(input, *declaration, *index, state)?;
                if input.aggregate_kind.is_none()
                    || input.aggregate_kind == Some(AggregateKind::Variant(*declaration, *index))
                {
                    let types = self.nominal_fields(value_ty, *declaration, Some(*index))?;
                    for (slot, ty) in types.iter().enumerate() {
                        let mut field =
                            match input.fields.as_ref().and_then(|fields| fields.get(slot)) {
                                Some(field) => field.snapshot(self.limits, 0)?,
                                None => self.unknown_value(ty)?,
                            };
                        self.project_component(
                            state,
                            &mut field,
                            input.projection.clone(),
                            (AggregateKind::Variant(*declaration, *index), slot),
                        )?;
                        // Publish owned nested payload fields before branching,
                        // so later pattern bindings can copy their relations.
                        self.prime_payload_observations(&mut field, state, &mut Vec::new(), 0)?;
                    }
                }
                if compose_tags {
                    tag = self.pattern_selection_condition(pattern, input, value_ty, state, 0)?;
                    compose_tags &= tag.is_some();
                }
                tag_conditions.push(tag);
            } else if matches!(pattern, Pattern::Wild) {
                tag_conditions.push(Some(Condition::Bool(true)));
            } else if compose_tags {
                if !prepared_structural_input {
                    let mut prepared = input.snapshot(self.limits, 0)?;
                    prepared.projection = input.projection.clone();
                    self.prime_payload_observations(&mut prepared, state, &mut Vec::new(), 0)?;
                    prepared_structural_input = true;
                }
                let condition =
                    self.pattern_selection_condition(pattern, input, value_ty, state, 0)?;
                compose_tags &= condition.is_some();
                tag_conditions.push(condition);
            } else {
                tag_conditions.push(None);
            }
        }
        Ok((compose_tags, tag_conditions))
    }

    #[inline(never)]
    #[allow(clippy::too_many_arguments)]
    fn eval_match_arm<'b>(
        &mut self,
        arm: MatchCase<'b>,
        pattern: &Pattern,
        guards: [Option<&Expr>; 2],
        last: bool,
        tag: Option<Condition>,
        input: &Value,
        value_ty: &Term,
        result_type: &Term,
        existing: &BTreeSet<PlaceId>,
        result_scope: &BTreeSet<PlaceId>,
        result_root: Option<PlaceId>,
        depth: usize,
        tail: bool,
        join_state: &mut MatchCasesState,
    ) -> Result<(), Reason> {
        let pending = join_state
            .pending
            .take()
            .expect("match cases pending state");
        let (mut candidate, mut failure) =
            self.pattern(pattern, input, value_ty, pending, depth)?;
        let pattern_selection = if join_state.compose_tags {
            tag.as_ref()
                .map(|tag| tag.copy_bounded(self.limits, 0))
                .transpose()?
        } else {
            None
        };
        let mut selection = tag;
        for guard in guards.into_iter().flatten() {
            let condition = self.child(guard, &mut candidate, depth)?.condition;
            if join_state.compose_tags && candidate.facts.is_reachable() {
                if let (Some(selected), Some(condition)) = (selection.take(), condition.as_ref()) {
                    selection = Some(conjunction(
                        selected,
                        self.scoped_condition(condition, &candidate, existing)?,
                    ));
                } else {
                    join_state.compose_tags = false;
                }
            }
            let (yes, mut no) = self.split_condition(candidate, condition.as_ref())?;
            candidate = yes;
            no.keep(existing, self.limits)?;
            join_state.guard_failures = Some(match join_state.guard_failures.take() {
                Some(previous) => previous.merge(no, self.limits)?,
                None => no,
            });
        }
        let selected = if join_state.compose_tags {
            if let (Some(selection), Some(pattern_selection)) = (selection, pattern_selection) {
                let remaining_copy = join_state
                    .remaining_condition
                    .copy_bounded(self.limits, 0)?;
                let alt_remaining_copy = join_state
                    .alternative_remaining
                    .copy_bounded(self.limits, 0)?;
                let selected_alternative = conjunction(alt_remaining_copy, selection);
                let selected_alt_copy = selected_alternative.copy_bounded(self.limits, 0)?;
                let selected = conjunction(remaining_copy, selected_alt_copy);
                let arm_success =
                    std::mem::replace(&mut join_state.arm_success, Condition::Bool(false));
                join_state.arm_success = disjunction(arm_success, selected_alternative);
                let alt_remaining =
                    std::mem::replace(&mut join_state.alternative_remaining, Condition::Bool(true));
                join_state.alternative_remaining =
                    conjunction(alt_remaining, negation(pattern_selection));
                if last {
                    let remaining = std::mem::replace(
                        &mut join_state.remaining_condition,
                        Condition::Bool(true),
                    );
                    let arm_success_done =
                        std::mem::replace(&mut join_state.arm_success, Condition::Bool(false));
                    join_state.remaining_condition =
                        conjunction(remaining, negation(arm_success_done));
                    join_state.alternative_remaining = Condition::Bool(true);
                }
                Some(selected)
            } else {
                join_state.compose_tags = false;
                None
            }
        } else {
            None
        };
        let result = match arm {
            MatchCase::Expression(arm) if tail => {
                self.tail(&arm.body, &mut candidate, depth)?;
                Value::default()
            }
            MatchCase::Expression(arm) => self.child(&arm.body, &mut candidate, depth)?,
            MatchCase::Contextual(arm) => {
                self.normal_input_value(&arm.body, Some(result_type), &mut candidate, depth + 1)?
                    .0
            }
        };
        if candidate.facts.is_reachable() {
            if join_state.compose_tags {
                if let (Some(selected), Some(condition)) = (selected, result.condition.as_ref()) {
                    let condition = self.scoped_condition(condition, &candidate, existing)?;
                    let tagged =
                        std::mem::replace(&mut join_state.tagged_condition, Condition::Bool(false));
                    join_state.tagged_condition =
                        disjunction(tagged, conjunction(selected, condition));
                } else {
                    join_state.compose_tags = false;
                }
            }
            join_state.sole_condition = if !join_state.has_result {
                result
                    .condition
                    .as_ref()
                    .map(|condition| self.scoped_condition(condition, &candidate, existing))
                    .transpose()?
            } else {
                None
            };
            if let Some(root) = result_root {
                self.bind_result(&mut candidate, root, result_type, result)?;
            } else if !join_state.has_result {
                join_state.joined_value = result;
            } else {
                join_state.joined_value = join_state.joined_value.join(&result, self.limits, 0)?;
            }
            join_state.has_result = true;
        }
        candidate.keep(result_scope, self.limits)?;
        failure.keep(existing, self.limits)?;
        if last {
            if let Some(guard_failed) = join_state.guard_failures.take() {
                failure = failure.merge(guard_failed, self.limits)?;
            }
        }
        let merged = join_state.merged.take().expect("match cases merged state");
        join_state.merged = Some(merged.merge(candidate, self.limits)?);
        join_state.pending = Some(failure);
        Ok(())
    }

    #[inline(never)]
    #[allow(clippy::too_many_arguments)]
    fn finish_match_cases(
        &mut self,
        state: &mut State,
        result_root: Option<PlaceId>,
        has_result: bool,
        compose_tags: bool,
        tagged_condition: Condition,
        sole_condition: Option<Condition>,
        joined_value: Value,
    ) -> Result<Value, Reason> {
        let sole_condition = if compose_tags {
            Some(tagged_condition)
        } else {
            sole_condition
        };
        if let Some(root) = result_root {
            if has_result {
                let mut value = self.local_value_at(state, root)?;
                if let Some(condition) = sole_condition {
                    if condition.is_current(&state.facts, self.limits, 0)? {
                        value.condition = Some(condition);
                    }
                }
                return Ok(value);
            }
        }
        Ok(joined_value)
    }
}

type CollectedMatchCase<'b> = (MatchCase<'b>, &'b Pattern, [Option<&'b Expr>; 2], bool);

struct MatchCasesState {
    pending: Option<State>,
    merged: Option<State>,
    joined_value: Value,
    has_result: bool,
    sole_condition: Option<Condition>,
    tagged_condition: Condition,
    remaining_condition: Condition,
    alternative_remaining: Condition,
    arm_success: Condition,
    guard_failures: Option<State>,
    compose_tags: bool,
}

impl Walker<'_> {
    /// Validate branches at an arbitrary request point. The handled body may
    /// mutate captures before any request, and may request zero or many times.
    /// Written or escaped captures therefore start from declared domains.
    /// A mutable declaration alone does not invalidate an unwritten local.
    /// Stable lexical scalars retain installation facts; branch results never
    /// become unconditional facts after the installation.
    pub(super) fn effect_handler_arms(
        &mut self,
        handler: &nir::program::EffectHandler,
        state: &State,
        depth: usize,
    ) -> Result<(), Reason> {
        let Term::Function { parameters, .. } = &handler.signature else {
            return Err(Reason::InvalidSignature);
        };
        let mut parameter_types: Vec<Term> = parameters.iter().map(|p| p.ty.clone()).collect();
        if handler.abi == nir::effects::Abi::Capturing {
            parameter_types.push(Term::Primitive(nir::Primitive::Continuation));
        }
        let ty = Term::Tuple(parameter_types);
        let mut pending = state.fork(self.limits)?;
        pending.havoc_with_private_scalars(self.handler_unstable_locals.as_ref(), self.limits)?;
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

/// Writes in pattern computations run in the handler just like its guard and
/// body. Include all three roots when deciding which caller facts may survive
/// a request; nested computations are deliberately conservative.
pub(super) fn handler_arm_writes(
    arm: &nir::program::MatchArm,
    fuel: &mut usize,
) -> Result<BTreeSet<LocalId>, String> {
    let roots = nir::visit::pattern_observations(&arm.pattern, fuel)?;
    let mut writes = BTreeSet::new();
    for value in roots
        .into_iter()
        .chain(arm.guard.iter())
        .chain(std::iter::once(&arm.body))
    {
        nir::visit::expression(value, fuel, |expr, _| {
            if let K::Assign { place, .. }
            | K::BitAssign { place, .. }
            | K::BitUpdate { place, .. } = &expr.kind
            {
                writes.insert(place.root);
            }
            Ok(())
        })?;
    }
    Ok(writes)
}

#[cfg(test)]
mod handler_write_tests {
    use super::*;

    #[test]
    fn handler_writes_include_pattern_guard_and_body_and_budget_retry() {
        use nir::program::{MatchArm, Pattern, Place};
        let source = nir::program::SourceRange { start: 0, end: 1 };
        let unit = Expr {
            source,
            ty: Term::Primitive(nir::Primitive::Void),
            kind: K::Unit,
        };
        let binding = |index| LocalId { owner: 1, index };
        // This fixture tests the structural footprint, independently of typing
        // the computed pattern or proving its observation contract.
        let assignment = |index| Expr {
            source,
            ty: unit.ty.clone(),
            kind: K::Assign {
                place: Place {
                    root: binding(index),
                    fields: vec![],
                },
                value: Box::new(unit.clone()),
            },
        };
        let arm = MatchArm {
            pattern: Pattern::Then {
                first: Box::new(Pattern::Wild),
                value: Box::new(assignment(0)),
                next: Box::new(Pattern::Wild),
            },
            guard: Some(assignment(1)),
            body: assignment(2),
        };
        assert!(handler_arm_writes(&arm, &mut 0).is_err());
        assert_eq!(
            handler_arm_writes(&arm, &mut 100).unwrap(),
            [binding(0), binding(1), binding(2)].into_iter().collect()
        );
        let empty = MatchArm {
            pattern: Pattern::Wild,
            guard: None,
            body: unit,
        };
        assert!(handler_arm_writes(&empty, &mut 100).unwrap().is_empty());
    }
}
