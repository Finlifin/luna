//! Required Type computations share generator storage and its failure recovery.
use super::*;

impl Checker<'_, '_> {
    pub(in crate::program) fn generator_required_type(
        &mut self,
        value: &Expr<'_>,
    ) -> Result<Term, Error> {
        if !self.checking_code_generator
            && (self.generator_data.is_empty() || self.data_consumer_call(value).is_some())
        {
            let (completed, _) = self.generator_data_expression(value, Some(&p(P::Type)))?;
            let comptime_interpretor::Value::Type(ty) = completed else {
                return Err(error(
                    value.span,
                    "contextual input did not produce a Type value",
                ));
            };
            return Ok(ty);
        }
        let mut evidence = None;
        let completed = self.generator_data_frame_with(value, |this| {
            let checked = this.expr(value, Some(&p(P::Type)))?;
            if this.checking_code_generator {
                evidence = this.generator_type_result_evidence(&checked, value.span)?;
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
                        if let (comptime_interpretor::Value::Type(ty), _) =
                            this.required_normal_input(value, checked.clone(), Vec::new())?
                        {
                            evidence = Some(ty);
                        }
                    }
                }
            }
            Ok(checked)
        })?;
        if let Some((comptime_interpretor::Value::Type(ty), _)) = completed {
            return Ok(ty);
        }
        evidence.ok_or_else(|| error(value.span,
            "contextual Type computation requires a result identity known during definition checking"))
    }

    fn generator_type_result_evidence(
        &mut self,
        value: &n::Expr,
        span: Span,
    ) -> Result<Option<Term>, Error> {
        if let Some(ty) = self.observed_expression_type(value, span)? {
            return Ok(Some(ty));
        }
        let K::Block {
            statements,
            result: Some(result),
        } = &value.kind
        else {
            return Ok(None);
        };
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let checked = (|| {
            // A closed tail identity is unaffected by earlier data writes, but
            // a transfer could bypass it and deliver a different result.
            for statement in statements {
                let mut transfer = false;
                nir::visit::expression(statement, &mut fuel, |node, _| {
                    transfer |= matches!(node.kind, K::Return(_) | K::Break(_) | K::Continue(_));
                    Ok::<_, String>(())
                })?;
                if transfer {
                    return Ok(false);
                }
            }
            Ok(nir::free_bindings::collect(result, &mut fuel)?.is_empty())
        })();
        self.required_steps = fuel as u64;
        if !checked.map_err(|message: String| error(span, message))? {
            return Ok(None);
        }
        self.observed_expression_type(result, span)
    }
}

impl Checker<'_, '_> {
    pub(in crate::program::cmtt) fn fresh_generator_type_binder(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<nir::DeclarationParameter, Error> {
        self.required_steps = self
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(span, "contextual input binder budget exhausted"))?;
        let index = self.next_definition_identity;
        self.next_definition_identity = index
            .checked_add(1)
            .ok_or_else(|| error(span, "contextual input identity space exhausted"))?;
        Ok(nir::DeclarationParameter {
            declaration: DeclarationId {
                package: declaration(self.package, &self.package_identities, owner).package,
                local: index,
            },
            sort: Sort::Type,
        })
    }

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
            let fresh = self.fresh_generator_type_binder(owner, span)?;
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
            self.type_input_bindings.insert(binding, value.clone());
            self.specialization_inputs
                .retain(|(id, _, _)| *id != binding);
            self.specialization_inputs
                .push((binding, p(P::Type), value));
        }
    }
}
