//! Code-method selection uses the ordinary implementation proofs. Value
//! receivers enter the comptime input pipeline before the remaining arguments.
use super::*;
mod traits;

impl Checker<'_, '_> {
    pub(super) fn install_generator_captures(
        &mut self,
        definition: &Generator,
        span: Span,
    ) -> Result<(), Error> {
        self.install_generator_capture_values(&definition.captures, span)
    }

    pub(in crate::program::cmtt) fn install_generator_capture_values(
        &mut self,
        captures: &[(DeclarationId, Term)],
        span: Span,
    ) -> Result<(), Error> {
        self.alias_substitution.extend(
            captures
                .iter()
                .map(|(id, value)| (*id, Arc::new(value.clone()))),
        );
        for (id, value) in captures {
            let domain = self.trait_input_domain(*id).unwrap_or_else(|| p(P::Type));
            let completed_domain = self.substituted_trait_domain(&domain, span)?;
            let sort = if completed_domain == p(P::Type) {
                Sort::Type
            } else {
                Sort::ComptimeValue
            };
            if value.sort() != sort {
                return Err(error(
                    span,
                    "captured generator argument has the wrong sort",
                ));
            }
            if domain == p(P::Type) {
                self.context.assume(crate::ptc::Formula::equal(
                    Term::Rigid {
                        declaration: *id,
                        sort: Sort::Type,
                    },
                    value.clone(),
                ));
                continue;
            }
            self.required_steps = self
                .required_steps
                .checked_sub(self.trait_parameter_types.len() as u64)
                .ok_or_else(|| error(span, "generator capture lookup budget exhausted"))?;
            let clause = self
                .trait_parameter_types
                .keys()
                .find_map(|owner| {
                    self.type_clause_params(*owner).into_iter().find(|clause| {
                        clause.definition.is_some_and(|definition| {
                            DeclarationId {
                                package: semantic_package(&self.package_identities, definition.pkg),
                                local: definition.index,
                            } == *id
                        })
                    })
                })
                .ok_or_else(|| error(span, "captured value clause declaration is unavailable"))?;
            let domain = completed_domain;
            let binding = local(clause.hir_id);
            self.locals.insert(binding, domain.clone());
            self.immutable.insert(binding);
            self.normal_default_bindings
                .insert(*id, (binding, domain.clone()));
            let data = if self.checking_code_generator {
                None
            } else {
                let site = Expr {
                    hir_id: clause.hir_id,
                    span: clause.span,
                    kind: H::Unit,
                };
                let input = n::Expr {
                    source: source(span),
                    ty: domain.clone(),
                    kind: K::Local(binding),
                };
                let (data, ty) = self.required_normal_input(
                    &site,
                    input,
                    vec![(binding, domain, value.clone())],
                )?;
                Some(self.materialize_generator_data(data, ty, span)?)
            };
            self.generator_data.insert(binding, data);
        }
        Ok(())
    }

    /// The public declaration and its selected implementation are two views
    /// of one call. Export exit parameter state for public postconditions,
    /// without writing ordinary by-value inputs back into the caller's frame.
    pub(in crate::program::cmtt) fn selected_generator_frame<R>(
        &mut self,
        target: OwnerId,
        bindings: &[(n::LocalId, n::LocalId)],
        span: Span,
        run: impl FnOnce(&mut Self) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let (result, outputs) = self.generator_frame(target, |this| {
            let result = run(this)?;
            let mut outputs = Vec::new();
            for &(public, selected) in bindings {
                this.required_steps = this
                    .required_steps
                    .checked_sub(1)
                    .ok_or_else(|| error(span, "selected method exit frame budget exhausted"))?;
                let Some(value) = this.generator_data.get(&selected) else {
                    continue;
                };
                if let Some(value) = value {
                    constants::charge_copy(value, &mut this.required_steps, span)?;
                }
                let mut value = value.clone();
                if let Some(value) = &mut value {
                    if matches!(value.kind, K::Local(id) if id == selected) {
                        value.kind = K::Local(public);
                    }
                }
                let mut cell = this.generator_cells.get(&selected).cloned();
                if let Some((snapshot, _)) = &mut cell {
                    constants::charge_copy(snapshot, &mut this.required_steps, span)?;
                    if matches!(snapshot.kind, K::Local(id) if id == selected) {
                        snapshot.kind = K::Local(public);
                    }
                }
                outputs.push((public, value, cell));
            }
            Ok((result, outputs))
        })?;
        for (binding, value, cell) in outputs {
            self.generator_data.insert(binding, value);
            if let Some(cell) = cell {
                self.generator_cells.insert(binding, cell);
            } else {
                self.generator_cells.remove(&binding);
            }
        }
        Ok(result)
    }

    pub(super) fn run_selected_code_method(
        &mut self,
        owner: OwnerId,
        public: &Generator,
        span: Span,
    ) -> Result<Option<Arc<ExpressionCode>>, Error> {
        let Some(target) = self.code_method_targets.get(&owner).copied() else {
            return Ok(None);
        };
        self.ensure_code_generator(target, span)?;
        let template = self.copy_code_generator(target, span)?;
        let (definition, arguments) =
            self.prepare_selected_contextual_method(target, public, &template, span)?;
        let bindings: Vec<_> = public
            .inputs
            .iter()
            .zip(&definition.inputs)
            .map(|((public, _), (selected, _))| (*public, *selected))
            .collect();
        self.selected_generator_frame(target, &bindings, span, |this| {
            this.run_code_generator(&definition, arguments)
        })
        .map(Some)
    }

    pub(super) fn prepare_selected_contextual_method<
        O: nir::code_interfaces::SignatureOutput + Clone,
    >(
        &mut self,
        target: OwnerId,
        public: &Generator<O>,
        template: &Generator<O>,
        span: Span,
    ) -> Result<(Generator<O>, Vec<Option<Argument>>), Error> {
        let types = public
            .types
            .iter()
            .map(|(_, value)| value.clone())
            .collect::<Vec<_>>();
        if template.types.len() != types.len() || template.inputs.len() != public.inputs.len() {
            return Err(error(span, "selected code method signature arity mismatch"));
        }
        let mut definition = self.instantiate_code_generator(template, &types, span)?;
        for (&slot, value) in &public.input_values {
            self.specialize_generator_input(&mut definition, slot, value, span)?;
        }
        let saved = definition
            .captures
            .iter()
            .map(|(id, value)| {
                (
                    *id,
                    self.alias_substitution.insert(*id, Arc::new(value.clone())),
                )
            })
            .collect::<Vec<_>>();
        let checked = self.check_type_application(target, &types, span);
        for (id, value) in saved {
            match value {
                Some(value) => {
                    self.alias_substitution.insert(id, value);
                }
                None => {
                    self.alias_substitution.remove(&id);
                }
            }
        }
        checked?;
        let mut arguments = Vec::new();
        for ((binding, input), (_, expected)) in public.inputs.iter().zip(&definition.inputs) {
            let argument = match (input, expected) {
                (Input::Data(actual), Input::Data(expected)) => {
                    self.require(actual, expected, span)?;
                    let value = self
                        .generator_data
                        .get(binding)
                        .ok_or_else(|| error(span, "missing trait code data input"))?;
                    if let Some(value) = value {
                        constants::charge_copy(value, &mut self.required_steps, span)?;
                    }
                    if value
                        .as_ref()
                        .is_some_and(|value| matches!(value.kind, K::Local(id) if id == *binding))
                    {
                        let cell = self
                            .generator_cells
                            .get(binding)
                            .ok_or_else(|| error(span, "missing live trait code input"))?
                            .1
                            .clone();
                        Argument::LiveData(match cell {
                            GeneratorCell::Reference(reference) => {
                                comptime_interpretor::Value::ComptimeReference(reference)
                            }
                            cell => self.read_generator_cell(&cell, span)?,
                        })
                    } else {
                        Argument::Data(value.clone())
                    }
                }
                (Input::Pattern(_), Input::Pattern(expected)) => {
                    let code = self
                        .pattern_codes
                        .get(binding)
                        .cloned()
                        .ok_or_else(|| error(span, "missing trait pattern code input"))?;
                    self.ascribe_pattern_input(&code, expected, span)?;
                    Argument::Code(CodeValue::Pattern(code))
                }
                (Input::Code(_), Input::Code(expected)) => {
                    let code = self
                        .code_values
                        .get(binding)
                        .cloned()
                        .ok_or_else(|| error(span, "missing trait code input"))?;
                    Argument::Code(CodeValue::Expression(self.ascribe_code(
                        code,
                        Some(expected),
                        span,
                    )?))
                }
                _ => return Err(error(span, "selected code method input category mismatch")),
            };
            arguments.push(Some(argument));
        }
        Ok((definition, arguments))
    }

    pub(in crate::program::cmtt) fn with_code_method_callee<R>(
        &mut self,
        callee: &Expr<'_>,
        invoke: impl FnOnce(&mut Self, &Expr<'_>, Option<&Expr<'_>>) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let (head, arguments) = match callee.kind {
            H::NFApplication(head, arguments) => (head, Some(arguments)),
            _ => (callee, None),
        };
        if let H::Projection(view, name) = head.kind
            && let H::Cast(base, interface) = view.kind
            && self.is_trait_view_target(interface)
        {
            return self.with_trait_code_method_callee(
                callee, head, arguments, base, interface, name, invoke,
            );
        }
        // Resolution can already bind a nested nominal member directly. Its
        // identity is still checked against the admitted implementation world.
        let candidate = match head.kind {
            H::Projection(base, name)
                if self.is_static_method_subject(base)
                    || self.local_field_domain(base)? == Some(p(P::Type)) =>
            {
                Some((self.type_expr(base)?, name.name, None, None))
            }
            H::Projection(base, name) => {
                let subject = self
                    .generator_intrinsic_input_hint(base, 0)?
                    .ok_or_else(|| {
                        error(base.span, "code method receiver type is not established")
                    })?;
                Some((subject, name.name, None, Some(base)))
            }
            H::ResolvedDef { def_id, .. } => {
                let owner = self.package.definition_owner(def_id);
                let parent = owner.and_then(|owner| self.self_containers.get(&owner).copied());
                match (owner, parent) {
                    (Some(owner), Some(parent))
                        if self.package.item(parent).is_some_and(|item| {
                            matches!(item.kind, ItemKind::Struct(_) | ItemKind::Enum(_))
                        }) =>
                    {
                        if !self.type_parameters[&parent].is_empty() {
                            return Err(error(
                                head.span,
                                "generic code method requires an explicitly instantiated container",
                            ));
                        }
                        self.check_type_application(parent, &[], head.span)?;
                        Some((
                            self.nominal_self_type(parent),
                            self.package.item(owner).unwrap().ident.name,
                            Some(owner),
                            None,
                        ))
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some((subject, name, requested_owner, receiver)) = candidate {
            if !self.interfaces_ready {
                return Err(error(
                    head.span,
                    "code method selection is waiting for the implementation world",
                ));
            }
            let subject = self.method_subject_view(&subject, head.span)?;
            if let Some((selected, inputs)) =
                self.select_inherent_method_implementation(&subject, name.as_str(), head.span)?
            {
                let implementation = self
                    .checked_implementations
                    .iter()
                    .find(|implementation| implementation.declaration == selected)
                    .ok_or_else(|| {
                        error(head.span, "selected code implementation is unavailable")
                    })?;
                let body = implementation
                    .methods
                    .iter()
                    .find(|(member, _)| member == name.as_str())
                    .map(|(_, body)| *body)
                    .ok_or_else(|| error(head.span, "selected code method is unavailable"))?;
                let owner = self
                    .package
                    .owners()
                    .find_map(|(owner, _)| {
                        matches_declaration(self.package, &self.package_identities, owner, body)
                            .then_some(owner)
                    })
                    .ok_or_else(|| {
                        error(head.span, "selected code method declaration is unavailable")
                    })?;
                if requested_owner.is_some_and(|requested| requested != owner) {
                    return Err(error(
                        head.span,
                        "resolved code member disagrees with implementation selection",
                    ));
                }
                if !self.is_code_generator(owner)
                    && !self.is_pattern_generator(owner)
                    && !self.is_data_consumer(owner)
                {
                    return Err(error(
                        head.span,
                        "code position requires a code-producing method",
                    ));
                }
                if implementation.type_parameters.len() != inputs.len() {
                    return Err(error(
                        head.span,
                        "code method captured argument count mismatch",
                    ));
                }
                let captures = implementation
                    .type_parameters
                    .iter()
                    .copied()
                    .zip(inputs)
                    .collect::<Vec<_>>();
                self.check_method_declaration_access(Some(body), head.hir_id, head.span)?;
                let ItemKind::Fn(signature, _) = &self.package.item(owner).unwrap().kind else {
                    return Err(error(head.span, "selected code method requires a body"));
                };
                if receiver.is_some()
                    && !signature
                        .params
                        .first()
                        .is_some_and(|param| matches!(param.ty.kind, TyParamKind::Itself { .. }))
                {
                    return Err(error(head.span, "instance code call requires a receiver"));
                }
                let definition = self.package.owner_definition(owner).ok_or_else(|| {
                    error(head.span, "selected code method has no definition identity")
                })?;
                let selected_head = Expr {
                    kind: H::ResolvedDef {
                        name,
                        has_clause_param: !signature.clause_params.is_empty(),
                        def_id: definition,
                    },
                    ..head.clone()
                };
                let selected_callee = match arguments {
                    Some(arguments) => Expr {
                        kind: H::NFApplication(&selected_head, arguments),
                        ..callee.clone()
                    },
                    None => selected_head.clone(),
                };
                let saved = self.code_method_captures.insert(owner, captures);
                let by_reference = signature.params.first().is_some_and(|param| {
                    matches!(param.ty.kind, TyParamKind::Itself { is_ref: true })
                });
                let addressed = receiver.map(|base| Expr {
                    kind: H::Ref(base),
                    ..base.clone()
                });
                let result = invoke(
                    self,
                    &selected_callee,
                    if by_reference {
                        addressed.as_ref()
                    } else {
                        receiver
                    },
                );
                match saved {
                    Some(saved) => {
                        self.code_method_captures.insert(owner, saved);
                    }
                    None => {
                        self.code_method_captures.remove(&owner);
                    }
                }
                return result;
            }
        }
        if self.generator_owner(callee)?.is_some_and(|owner| {
            self.self_types.contains_key(&owner) || self.self_containers.contains_key(&owner)
        }) {
            return Err(error(
                callee.span,
                "code method requires an established implementation selection",
            ));
        }
        invoke(self, callee, None)
    }
}
