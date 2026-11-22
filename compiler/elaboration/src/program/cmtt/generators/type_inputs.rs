//! Required Type computations share generator storage and its failure recovery.
use super::*;
use crate::program::normal::observation::Failure;

impl Checker<'_, '_> {
    /// Only a successfully checked definition supplies phase routing to its
    /// invocations. Layouts, local values and caller storage remain per call.
    pub(in crate::program::cmtt) fn check_generator_body<T>(
        &mut self,
        owner: OwnerId,
        check: impl FnOnce(&mut Self) -> Result<T, Error>,
    ) -> Result<SourceBindings<()>, Error> {
        let checking = std::mem::replace(&mut self.checking_code_generator, true);
        let parent = self
            .checking_constructor_operands
            .replace((owner, Default::default()));
        let result = self.generator_frame(owner, check);
        let (_, operands) =
            std::mem::replace(&mut self.checking_constructor_operands, parent).unwrap();
        self.checking_code_generator = checking;
        result?;
        Ok(operands)
    }

    pub(in crate::program) fn generator_constructed_type(
        &mut self,
        callee: &Expr<'_>,
    ) -> Result<Option<Term>, Error> {
        let owner = callee.hir_id.owner;
        if self.checking_code_generator
            || !self.in_code_body
            || !self.runtime_value_context()
            || self.signature_owner != Some(owner)
        {
            return Ok(None);
        }
        let operands = if let Some(definition) = self.code_generators.get(&owner) {
            if definition.checked_operands.is_none() {
                self.check_code_generator(owner)?;
            }
            &self.code_generators[&owner].checked_operands
        } else if let Some(definition) = self.data_consumers.get(&owner) {
            if definition.checked_operands.is_none() {
                self.check_data_consumer(owner)?;
            }
            &self.data_consumers[&owner].checked_operands
        } else if let Some(definition) = self.pattern_generators.get(&owner) {
            if definition.checked_operands.is_none() {
                self.check_pattern_generator(owner)?;
            }
            &self.pattern_generators[&owner].checked_operands
        } else {
            return Ok(None);
        };
        if operands
            .as_ref()
            .is_some_and(|sites| sites.contains_key(&callee.hir_id))
        {
            return self
                .generator_type_data(callee)?
                .identity(callee.span)
                .map(Some);
        }
        Ok(None)
    }

    pub(in crate::program) fn generator_type_data(
        &mut self,
        value: &Expr<'_>,
    ) -> Result<CompletedType, Error> {
        if !self.checking_code_generator
            && (self.generator_data.is_empty() || self.data_consumer_call(value)?.is_some())
        {
            let (completed, _) = self.generator_data_expression(value, Some(&p(P::Type)))?;
            return CompletedType::from_value(completed, value.span);
        }
        self.generator_type_data_with(value, |this| this.expr(value, Some(&p(P::Type))))
    }

    // Callee routing may already have checked this producer to distinguish
    // a Type from a callable. Keep the same producer and generator storage.
    pub(in crate::program) fn generator_type_data_with(
        &mut self,
        value: &Expr<'_>,
        check: impl FnOnce(&mut Self) -> Result<n::Expr, Error>,
    ) -> Result<CompletedType, Error> {
        let mut evidence = None;
        let completed = self.generator_data_frame_with(value, |this| {
            let checked = check(this)?;
            if this.checking_code_generator {
                evidence = this
                    .generator_type_result_evidence(&checked, value.span)?
                    .map(CompletedType::Identity);
                if evidence.is_none() {
                    let mut fuel = usize::try_from(this.required_steps).unwrap_or(usize::MAX);
                    let free = nir::free_bindings::collect(&checked, &mut fuel);
                    this.required_steps = fuel as u64;
                    let free = free.map_err(|message| error(value.span, message))?;
                    // Closed computations retain ordinary required invocation.
                    // Never supply fabricated values for a caller's live locals.
                    if free.is_empty() {
                        this.validate_generator_data(&checked, value.span)?;
                        constants::charge_copy(&checked, &mut this.required_steps, value.span)?;
                        let (result, _) =
                            this.required_normal_input(value, checked.clone(), Vec::new())?;
                        evidence = Some(CompletedType::from_value(result, value.span)?);
                    }
                }
            }
            Ok(checked)
        })?;
        if let Some((result, _)) = completed {
            return CompletedType::from_value(result, value.span);
        }
        evidence.ok_or_else(|| error(value.span,
            "contextual Type computation requires a result identity known during definition checking"))
    }

    pub(super) fn generator_type_result_evidence(
        &mut self,
        value: &n::Expr,
        span: Span,
    ) -> Result<Option<Term>, Error> {
        if let Some(ty) = self.observed_expression_type(value, span)? {
            return Ok(Some(ty));
        }
        if matches!(value.kind, K::Call { .. }) {
            return self.projected_type_return(value, span);
        }
        if matches!(
            value.kind,
            K::Block { .. } | K::CallPreparation { .. } | K::If { .. } | K::Match { .. }
        ) {
            // A required data frame can return early. A fact about its normal
            // tail alone is insufficient to describe such an invocation.
            let transfers = self.with_observation_budget(span, |_, budget| {
                let mut exits = false;
                nir::visit::expression_in_closure_environment(
                    value,
                    &mut budget.steps,
                    |node, _| {
                        exits |= matches!(node.kind, K::Return(_) | K::Break(_) | K::Continue(_));
                        Ok(())
                    },
                )
                .map_err(Failure::Invalid)?;
                Ok(exits)
            })?;
            if !transfers {
                return self.preserved_input_identity(value, |_, _| Ok(None), span);
            }
        }
        Ok(None)
    }
}

impl Checker<'_, '_> {
    pub(in crate::program::cmtt) fn freshen_generator_signature<
        O: nir::code_interfaces::SignatureOutput + Clone,
    >(
        &mut self,
        owner: OwnerId,
        signature: &mut CodeFunctionSignature<O>,
        span: Span,
    ) -> Result<(), Error> {
        if signature.input_binders.is_empty() {
            return Ok(());
        }
        let mut binders = BTreeMap::new();
        let mut parameters = Vec::new();
        let mut arguments = Vec::new();
        for (slot, old) in &signature.input_binders {
            let fresh = self.fresh_type_binder(owner, span)?;
            parameters.push(*old);
            arguments.push(Term::Rigid {
                declaration: fresh.declaration,
                sort: fresh.sort,
            });
            binders.insert(*slot, fresh);
        }
        let frames = self.call_input_frames();
        let mut fresh = signature.try_map_types(|ty| {
            self.instantiate_generator_type_in_scope(ty, &parameters, &arguments, &frames, span)
        })?;
        fresh.input_binders = binders;
        *signature = fresh;
        Ok(())
    }

    pub(in crate::program::cmtt) fn specialize_generator_input<
        O: nir::code_interfaces::SignatureOutput + Clone,
    >(
        &mut self,
        definition: &mut Generator<O>,
        slot: usize,
        value: &Term,
        span: Span,
    ) -> Result<(), Error> {
        let frames = self.call_input_frames();
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let result = definition
            .signature
            .specialize_input(slot, value, &frames, &mut fuel);
        self.required_steps = fuel as u64;
        definition.signature = result.map_err(|failure| error(span, failure.to_string()))?;
        definition
            .input_values
            .insert(slot, self.copy_alias_type(value, span)?);
        Ok(())
    }

    pub(in crate::program::cmtt) fn install_generator_type_input(
        &mut self,
        binding: n::LocalId,
        value: Option<Term>,
    ) {
        if let Some(value) = value {
            self.input_facts.types.insert(binding, value.clone());
            self.input_facts
                .specialized
                .retain(|(id, _, _)| *id != binding);
            self.input_facts
                .specialized
                .push((binding, p(P::Type), value));
        }
    }
}
