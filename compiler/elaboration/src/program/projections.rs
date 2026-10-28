//! Field and associated-member checking lives outside the recursive expression
//! dispatcher so its temporaries do not inflate every signature dependency frame.
use super::*;

impl Checker<'_, '_> {
    /// Local Type values acquire their container identity during elaboration.
    /// Reuse the resolver's use-site candidate set, including access checks;
    /// ordinary value receivers must keep the field/method projection path.
    /// Hand the completed intermediate type back to this selection operation:
    /// discarding it makes type checking recursively re-elaborate every prefix.
    /// This evidence is local to the operation, not a cache across environments.
    fn local_type_receiver(
        &mut self,
        projection: &Expr<'_>,
        completed: &mut Option<Term>,
    ) -> Result<bool, Error> {
        if self
            .package
            .member_projections
            .contains_key(&projection.hir_id)
            || self
                .package
                .deferred_member_projections
                .contains_key(&projection.hir_id)
        {
            return Ok(false);
        }
        let H::Projection(base, _) = projection.kind else {
            return Ok(false);
        };
        self.required_steps = self
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(projection.span, "type member receiver budget exhausted"))?;
        match base.kind {
            H::ResolvedLocal(_, binding) | H::ResolvedFnParam(_, binding) => {
                let binding = local(binding);
                Ok(self.locals.get(&binding) == Some(&p(P::Type))
                    || self
                        .generator_data
                        .get(&binding)
                        .and_then(Option::as_ref)
                        .is_some_and(|value| value.ty == p(P::Type)))
            }
            H::Projection(..) | H::NFApplication(..) => {
                // Resolve each intermediate type member with its own use-site
                // permissions. A value member cannot become a Type receiver.
                *completed = self.projected_member_type(base)?;
                if completed.is_none() && matches!(base.kind, H::Projection(..)) {
                    *completed = self.observed_field_type(base)?;
                    if completed.is_none() && self.local_field_domain(base)? == Some(p(P::Type)) {
                        *completed = Some(self.type_expr(base)?);
                    }
                }
                Ok(completed.is_some())
            }
            H::Block(_) | H::If(..) | H::When(_) | H::Match(..) => {
                // Like a local Type receiver, this establishes only its domain.
                // Type positions complete its identity; expression positions
                // must retain the receiver's evaluation prefix instead.
                Ok(self.local_field_domain(base)? == Some(p(P::Type)))
            }
            _ => Ok(false),
        }
    }

    /// Spelling alone does not distinguish a data field from Type.vars.
    /// For computed receivers, prepare first and inspect the result domain.
    /// Only a Type-valued receiver is invoked here; data receivers are invoked
    /// later as part of the complete ordinary projection computation.
    pub(super) fn metatype_namespace_receiver(&mut self, base: &Expr<'_>) -> Result<bool, Error> {
        if let Some(domain) = self.local_field_domain(base)? {
            return if domain == p(P::Type) {
                self.type_expr(base).map(|ty| ty == p(P::Type))
            } else {
                Ok(false)
            };
        }
        if self.is_type_head(base) {
            return self.type_expr(base).map(|ty| ty == p(P::Type));
        }
        let prepared = self.prepare_required_expression(base, None, None, Vec::new(), false)?;
        if prepared.result_type != p(P::Type) {
            return Ok(false);
        }
        let (value, _) =
            self.invoke_required_publishing(prepared, Vec::new(), base.hir_id.owner, base.span)?;
        Ok(matches!(value, comptime_interpretor::Value::Type(ty) if ty == p(P::Type)))
    }

    /// Classify a data-field path by local or comptime call result domains,
    /// without interpreting its values. This only disambiguates builtin namespace
    /// syntax; ordinary projection still performs the actual field checks.
    pub(super) fn local_field_domain(&mut self, expr: &Expr<'_>) -> Result<Option<Term>, Error> {
        let mut root = expr;
        let mut fields = Vec::new();
        while let H::Projection(base, field) = root.kind {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(expr.span, "field domain observation budget exhausted"))?;
            if fields.len() >= 128 {
                return Ok(None);
            }
            fields.push(field.name);
            root = base;
        }
        let call_domain;
        let domain = match root.kind {
            H::ResolvedLocal(_, binding) | H::ResolvedFnParam(_, binding) => {
                self.locals.get(&local(binding))
            }
            H::Application(..) | H::ExtendedApplication(..) => {
                call_domain = self.comptime_call_result(root)?;
                call_domain.as_ref()
            }
            H::Block(_) | H::If(..) | H::When(_) | H::Match(..) => {
                // Discover only the checked domain. The speculative branch
                // never publishes local storage, captures or control evidence,
                // and no invocation is performed to classify this receiver.
                // Keep the caller's phase: classifying a runtime value must
                // not demand a comptime invocation. A Type result still goes
                // through required evaluation when its identity is needed.
                call_domain = self
                    .checking_attempt(true, false, |checker| checker.expr(root, None))?
                    .map(|checked| checked.ty);
                call_domain.as_ref()
            }
            _ => None,
        };
        let Some(domain) = domain else {
            return Ok(None);
        };
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let charged = nir::type_computations::charge_term(domain, &mut fuel);
        self.required_steps -= (before - fuel) as u64;
        charged.map_err(|e| error(expr.span, e.to_string()))?;
        let mut domain = domain.clone();
        for name in fields.into_iter().rev() {
            let shape = self.structural_type(&domain, expr.span)?;
            domain = match &shape {
                Term::Record(fields) => {
                    let Some((_, field)) = fields.iter().find(|(field, _)| field == name.as_str())
                    else {
                        return Ok(None);
                    };
                    self.copy_alias_type(field, expr.span)?
                }
                Term::Nominal {
                    declaration,
                    arguments,
                } => {
                    self.ensure_nominal_type(&shape, expr.span)?;
                    let Some(definition) = self.structs.get(declaration) else {
                        return Ok(None);
                    };
                    let Some((_, field)) = definition
                        .fields
                        .iter()
                        .find(|(field, _)| field == name.as_str())
                    else {
                        return Ok(None);
                    };
                    self.instantiate_input_types(
                        field,
                        &definition.type_parameters,
                        arguments,
                        expr.span,
                    )?
                }
                // Type-valued intermediate expressions may select members,
                // constants or metatype namespaces. Leave those to type_expr.
                _ => return Ok(None),
            };
        }
        Ok(Some(domain))
    }

    /// Only inspect a field path rooted in an available observed local. Source
    /// calls, dereferences and computed receivers keep their ordinary required
    /// invocation path; discovering a container must not run a producer twice.
    pub(super) fn observed_field_type(&mut self, expr: &Expr<'_>) -> Result<Option<Term>, Error> {
        if !matches!(expr.kind, H::Projection(..)) {
            return Ok(None);
        }
        let mut root = expr;
        while let H::Projection(base, _) = root.kind {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(expr.span, "field type observation budget exhausted"))?;
            root = base;
        }
        let H::ResolvedLocal(_, binding) = root.kind else {
            return Ok(None);
        };
        if !self.locals.contains_key(&local(binding))
            || self.locals.get(&local(binding)) == Some(&p(P::Type))
            || !self.partial_staged_bindings.contains_key(&local(binding))
        {
            return Ok(None);
        }
        let value = self.expr(expr, None)?;
        if !matches!(value.kind, K::Field { .. } | K::AnonymousField { .. }) {
            return Ok(None);
        }
        self.observed_expression_type(&value, expr.span)
    }

    pub(super) fn projected_constant_owner(
        &mut self,
        expr: &Expr<'_>,
    ) -> Result<Option<OwnerId>, Error> {
        let H::Projection(base, _) = expr.kind else {
            return Ok(None);
        };
        let mut completed_subject = None;
        let local_receiver = self.local_type_receiver(expr, &mut completed_subject)?;
        if completed_subject.is_none() && !self.type_member_base(base)? {
            return Ok(None);
        }
        let selected =
            if let Some(definition) = self.package.member_projections.get(&expr.hir_id).copied() {
                Some(definition)
            } else if let Some(candidates) = self
                .package
                .deferred_member_projections
                .get(&expr.hir_id)
                .or_else(|| {
                    local_receiver
                        .then(|| self.package.member_access.get(&expr.hir_id))
                        .flatten()
                })
                .cloned()
            {
                let Term::Nominal {
                    declaration: parent,
                    ..
                } = (match completed_subject.take() {
                    Some(subject) => subject,
                    None => self.type_expr(base)?,
                })
                else {
                    return Ok(None);
                };
                candidates.iter().find_map(|(container, member)| {
                    let owner = self.package.definition_owner(*container)?;
                    (declaration(self.package, &self.package_identities, owner) == parent)
                        .then_some(*member)
                })
            } else {
                return Ok(None);
            };
        let Some(definition) = selected else {
            return Ok(None);
        };
        let Some(owner) = self.package.definition_owner(definition) else {
            return Ok(None);
        };
        if !self
            .package
            .item(owner)
            .is_some_and(|item| matches!(item.kind, ItemKind::Const(..)))
        {
            return Ok(None);
        }
        // Even a constant independent of the container's parameters cannot
        // bypass the formation check on an explicitly supplied container.
        if self.package.member_projections.contains_key(&expr.hir_id) {
            self.type_expr(base)?;
        }
        Ok(Some(owner))
    }

    fn type_member_base(&mut self, mut base: &Expr<'_>) -> Result<bool, Error> {
        loop {
            if matches!(base.kind, H::Projection(..))
                && self.local_field_domain(base)? == Some(p(P::Type))
            {
                return Ok(true);
            }
            match base.kind {
                H::Projection(inner, _) | H::NFApplication(inner, _) => base = inner,
                H::Application(..) | H::ExtendedApplication(..) => {
                    return self.completed_type_call(base);
                }
                _ => return Ok(true),
            }
        }
    }

    fn completed_type_call(&mut self, base: &Expr<'_>) -> Result<bool, Error> {
        Ok(self.comptime_call_result(base)? == Some(p(P::Type)))
    }

    fn comptime_call_result(&mut self, base: &Expr<'_>) -> Result<Option<Term>, Error> {
        let Some(domain) = self.declared_comptime_call_result(base)? else {
            return Ok(None);
        };
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let dependent = nir::type_computations::has_rigid_parameters_bounded(&domain, &mut fuel);
        self.required_steps -= (before - fuel) as u64;
        if !dependent.map_err(|failure| error(base.span, failure.to_string()))? {
            return Ok(Some(domain));
        }
        // The declared result is still in the callee's telescope. Complete
        // ordinary call argument/default inference before inspecting fields.
        // Preparation supplies the instantiated domain without invoking the
        // producer; the eventual required computation keeps its effects.
        let prepared = self.prepare_required_expression(base, None, None, Vec::new(), false)?;
        Ok(Some(prepared.result_type))
    }

    fn declared_comptime_call_result(&mut self, base: &Expr<'_>) -> Result<Option<Term>, Error> {
        fn copy_result(ty: &Term, remaining: &mut u64, span: Span) -> Result<Option<Term>, Error> {
            let Term::Function { result, .. } = ty.callable_signature() else {
                return Ok(None);
            };
            let mut fuel = usize::try_from(*remaining).unwrap_or(usize::MAX);
            let before = fuel;
            let charged = nir::type_computations::charge_term(result, &mut fuel);
            *remaining -= (before - fuel) as u64;
            charged.map_err(|failure| error(span, failure.to_string()))?;
            Ok(Some((**result).clone()))
        }
        let (H::Application(head, _) | H::ExtendedApplication(head, _)) = base.kind else {
            return Ok(None);
        };
        let head = if let H::NFApplication(head, _) = head.kind {
            head
        } else {
            head
        };
        if let H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) = head.kind {
            let id = local(id);
            let ty = self
                .generator_data
                .get(&id)
                .and_then(|value| value.as_ref())
                .map(|value| value.ty.clone())
                .or_else(|| self.locals.get(&id).cloned())
                .or_else(|| self.signature_comptime_input(id).map(|(_, ty)| ty));
            return match ty.filter(|ty| ty.is_comptime_callable()) {
                Some(ty) => copy_result(&ty, &mut self.required_steps, base.span),
                None => Ok(None),
            };
        }
        let def_id = match head.kind {
            H::ResolvedDef { def_id, .. } => def_id,
            H::Projection(container, _) => {
                if let Some(def_id) = self.package.member_projections.get(&head.hir_id).copied() {
                    def_id
                } else if let Some(candidates) = self
                    .package
                    .deferred_member_projections
                    .get(&head.hir_id)
                    .cloned()
                {
                    if !self.type_member_base(container)? {
                        return Ok(None);
                    }
                    let Term::Nominal {
                        declaration: parent,
                        ..
                    } = self.type_expr(container)?
                    else {
                        return Ok(None);
                    };
                    let Some(member) = candidates.iter().find_map(|(container, member)| {
                        let owner = self.package.definition_owner(*container)?;
                        (declaration(self.package, &self.package_identities, owner) == parent)
                            .then_some(*member)
                    }) else {
                        return Ok(None);
                    };
                    member
                } else {
                    return Ok(None);
                }
            }
            _ => return Ok(None),
        };
        let Some(owner) = self.package.definition_owner(def_id) else {
            return Ok(None);
        };
        if self
            .package
            .item(owner)
            .is_some_and(|item| matches!(item.kind, ItemKind::Const(..)))
        {
            let value = if self.is_build_context(owner) {
                self.build_context_value(base, owner)?
            } else {
                self.constant_value(owner, base.span)?
            };
            return if value.ty.is_comptime_callable() {
                copy_result(&value.ty, &mut self.required_steps, base.span)
            } else {
                Ok(None)
            };
        }
        if !self.package.item(owner).is_some_and(|item| matches!(&item.kind, ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig) if sig.modifiers.is_comptime)) {
            return Ok(None);
        }
        self.ensure_signature(owner, base.span)?;
        match self
            .signatures
            .get(&owner)
            .or_else(|| self.interface_signatures.get(&owner))
        {
            Some(ty) => copy_result(ty, &mut self.required_steps, base.span),
            None => Ok(None),
        }
    }

    pub(super) fn instantiate_formation_environment(
        &self,
        predicate: &n::TypePredicate,
        environment: &BTreeMap<DeclarationId, std::sync::Arc<Term>>,
        span: Span,
    ) -> Result<n::TypePredicate, Error> {
        if environment.is_empty() {
            return Ok(predicate.clone());
        }
        let parameters = self.input_binders(&environment.keys().copied().collect::<Vec<_>>());
        let arguments: Vec<_> = environment.values().map(|ty| ty.as_ref().clone()).collect();
        predicate
            .instantiate_parameters_in_scope(&parameters, &arguments, self.context.scope())
            .map_err(|e| error(span, e.to_string()))
    }

    pub(super) fn projected_member_type(&mut self, expr: &Expr<'_>) -> Result<Option<Term>, Error> {
        self.with_projected_type_member(expr, |checker, owner, mut arguments| {
            checker.check_type_application(owner, &arguments, expr.span)?;
            match &checker.package.item(owner).unwrap().kind {
                ItemKind::TypeAlias(value, ..) => {
                    checker.alias_type(owner, value, &arguments, expr.span)
                }
                ItemKind::Newtype(..) | ItemKind::Struct(_) | ItemKind::Enum(_) => {
                    arguments.extend(checker.nominal_capture_arguments(owner, expr.span)?);
                    Ok(Term::Nominal {
                        declaration: declaration(
                            checker.package,
                            &checker.package_identities,
                            owner,
                        ),
                        arguments,
                    })
                }
                _ => unreachable!(),
            }
        })
    }

    pub(super) fn projected_member_expression(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<Option<n::Expr>, Error> {
        let (projection, supplied) = match &expr.kind {
            H::NFApplication(head, arguments) => (*head, *arguments),
            H::Projection(..) => (expr, &[][..]),
            _ => return Ok(None),
        };
        let H::Projection(base, _) = projection.kind else {
            return Ok(None);
        };
        if !self.type_member_base(base)? {
            return Ok(None);
        }
        // Expression elaboration still needs the container's preparation code;
        // a completed logical Type cannot replace that evaluation prefix.
        let mut receiver_type = None;
        // A local Type root permits a staged constructor chain. Do not probe
        // arbitrary value-field chains: speculative expression checking could
        // repeat comptime work before the ordinary projection path runs.
        let mut root = projection;
        loop {
            match root.kind {
                H::Projection(inner, _)
                    if matches!(inner.kind, H::Projection(..) | H::NFApplication(..)) =>
                {
                    root = inner;
                }
                H::NFApplication(head, _) => root = head,
                _ => break,
            }
        }
        let local_receiver = self.local_type_receiver(root, &mut receiver_type)?;
        let (member, prepared) = if let Some(candidates) = self
            .package
            .deferred_member_projections
            .get(&projection.hir_id)
            .or_else(|| {
                local_receiver
                    .then(|| self.package.member_access.get(&projection.hir_id))
                    .flatten()
            })
            .cloned()
        {
            let Some((parent_id, instance, arguments)) = self.prepared_alias_container(base)?
            else {
                return Ok(None);
            };
            let selected = candidates.iter().find_map(|(container, member)| {
                let parent = self.package.definition_owner(*container)?;
                (declaration(self.package, &self.package_identities, parent) == parent_id)
                    .then_some((*member, parent))
            });
            let Some((member, parent)) = selected else {
                return Ok(None);
            };
            (member, Some((parent, instance, arguments)))
        } else {
            let Some(member) = self
                .package
                .member_projections
                .get(&projection.hir_id)
                .copied()
            else {
                return Ok(None);
            };
            (member, None)
        };
        let Some(owner) = self.package.definition_owner(member) else {
            return Ok(None);
        };
        if !matches!(
            self.package.item(owner).unwrap().kind,
            ItemKind::TypeAlias(..)
                | ItemKind::Newtype(..)
                | ItemKind::Struct(_)
                | ItemKind::Enum(_)
                | ItemKind::Const(..)
        ) {
            return Ok(None);
        }
        let constant = matches!(self.package.item(owner).unwrap().kind, ItemKind::Const(..));
        if constant && !supplied.is_empty() {
            return Err(error(
                expr.span,
                "constant member does not accept type arguments",
            ));
        }
        let (parent, instance, arguments) = if let Some(prepared) = prepared {
            prepared
        } else if !matches!(base.kind, H::NFApplication(head, _) if matches!(head.kind, H::ResolvedDef { .. }))
        {
            let Some((parent_id, instance, arguments)) = self.prepared_alias_container(base)?
            else {
                return Ok(None);
            };
            let Some(parent) = self.self_containers.get(&owner).copied() else {
                return Ok(None);
            };
            if declaration(self.package, &self.package_identities, parent) != parent_id {
                return Err(error(base.span, "type member container identity mismatch"));
            }
            (parent, instance, arguments)
        } else {
            let H::NFApplication(head, inputs) = base.kind else {
                unreachable!()
            };
            let H::ResolvedDef { def_id, .. } = head.kind else {
                unreachable!()
            };
            let parent = self
                .package
                .definition_owner(def_id)
                .ok_or_else(|| error(base.span, "type member container declaration is missing"))?;
            let (instance, arguments) = if let Some(value) =
                self.contextual_nominal_expression(parent, inputs, base.span, Some(&p(P::Type)))?
            {
                self.expose_nominal_inputs(parent, value, base.span)?
            } else {
                self.prepared_type_instance(parent, inputs, base.span, Some(&p(P::Type)))?
            };
            (parent, instance, arguments)
        };
        let mut parameters = self.type_parameters[&parent].clone();
        parameters.extend(self.nominal_captures(parent, expr.span)?);
        if constant && self.runtime_value_context() {
            // Formation belongs to comptime; only the Splice result may enter
            // runtime storage. Reuse the already checked container expression.
            let prepared = self.prepare_required_expression(
                base,
                Some(&p(P::Type)),
                Some(instance),
                Vec::new(),
                false,
            )?;
            let (value, _) = self.invoke_required_publishing(
                prepared,
                Vec::new(),
                base.hir_id.owner,
                base.span,
            )?;
            let comptime_interpretor::Value::Type(Term::Nominal { arguments, .. }) = value else {
                return Err(error(
                    base.span,
                    "constant member container is not a completed nominal type",
                ));
            };
            if parameters.len() != arguments.len() {
                return Err(error(
                    base.span,
                    "type member container argument count mismatch",
                ));
            }
            let saved_substitution = self.alias_substitution.clone();
            self.alias_substitution.extend(
                parameters
                    .iter()
                    .copied()
                    .zip(arguments.into_iter().map(std::sync::Arc::new)),
            );
            let H::Projection(_, field) = projection.kind else {
                unreachable!()
            };
            let definition = Expr {
                kind: H::ResolvedDef {
                    name: field.name,
                    has_clause_param: false,
                    def_id: member,
                },
                ..expr.clone()
            };
            let result = self.expr(&definition, expected).map(Some);
            self.alias_substitution = saved_substitution;
            return result;
        }
        if parameters.len() != arguments.len() {
            return Err(error(
                base.span,
                "type member container argument count mismatch",
            ));
        }
        let saved_substitution = self.alias_substitution.clone();
        let saved_bindings = self.alias_value_bindings.clone();
        let saved_normal_bindings = self.normal_default_bindings.clone();
        let saved_locals = self.locals.clone();
        let saved_context = self.context.clone();
        let result = (|| {
            // Explicit member arguments belong to the caller's environment;
            // only defaults and the member body see the selected container.
            let count = self
                .type_clause_params(owner)
                .iter()
                .filter(|clause| !matches!(clause.kind, hir::clause::ClauseParamKind::Optional(..)))
                .count();
            let binders = self.input_binders(&parameters);
            // Prepared parent values live in locals. Recover only structural
            // evidence from their initializers, without invoking input code.
            let mut known_parent = BTreeMap::new();
            let mut budget = comptime_runtime::Budget {
                steps: usize::try_from(self.required_steps).unwrap_or(usize::MAX),
                max_limbs: 4096,
            };
            let before = budget.steps;
            if let K::Block { statements, .. } = &instance.kind {
                for statement in statements {
                    if let K::Let { binding, value } = &statement.kind {
                        if let Some(value) =
                            self.known_normal_input_in(value, 0, &mut budget, &known_parent)
                        {
                            known_parent.insert(*binding, std::sync::Arc::new(value));
                        }
                    }
                }
            }
            let parent_terms = arguments
                .iter()
                .zip(&binders)
                .map(|(argument, binder)| {
                    self.known_normal_input_in(argument, 0, &mut budget, &known_parent)
                        .unwrap_or(Term::Rigid {
                            declaration: binder.declaration,
                            sort: binder.sort,
                        })
                })
                .collect::<Vec<_>>();
            self.required_steps -= (before - budget.steps) as u64;
            let domains = self
                .trait_domains(owner)?
                .iter()
                .map(|domain| {
                    self.instantiate_trait_domain(domain, &binders, &parent_terms, expr.span)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let member_captures = self.nominal_captures(owner, expr.span)?;
            let contextual_alias = matches!(
                self.package.item(owner).map(|item| &item.kind),
                Some(ItemKind::TypeAlias(..))
            ) && self
                .trait_domains(owner)?
                .iter()
                .any(|domain| *domain != p(P::Type))
                || self
                    .trait_domains(owner)?
                    .iter()
                    .any(|domain| member_captures.iter().any(|id| domain.mentions_rigid(*id)));
            let mut member_statements = Vec::new();
            let member_inputs = if contextual_alias {
                Vec::new()
            } else {
                self.prepared_nominal_inputs_in_domains(
                    owner,
                    supplied,
                    count,
                    &mut member_statements,
                    expr.span,
                    &domains,
                )?
            };

            let (mut statements, instance_body) = match instance.kind {
                K::Block {
                    statements,
                    result: Some(body),
                } => (statements, *body),
                kind => (Vec::new(), n::Expr { kind, ..instance }),
            };
            for (parameter, argument) in parameters.into_iter().zip(arguments) {
                match argument.kind {
                    K::TypeValue(ty) => {
                        self.alias_substitution
                            .insert(parameter, std::sync::Arc::new(ty));
                        self.alias_value_bindings.remove(&parameter);
                        self.normal_default_bindings.remove(&parameter);
                    }
                    K::Local(binding) => {
                        if argument.ty == p(P::Type) {
                            self.alias_value_bindings.insert(parameter, binding);
                            self.normal_default_bindings.remove(&parameter);
                        } else {
                            self.normal_default_bindings
                                .insert(parameter, (binding, argument.ty.clone()));
                            self.alias_value_bindings.remove(&parameter);
                        }
                        if let Some(value) = known_parent.get(&binding) {
                            let value = self.copy_alias_type(value, base.span)?;
                            self.alias_substitution
                                .insert(parameter, std::sync::Arc::new(value));
                        } else {
                            self.alias_substitution.remove(&parameter);
                        }
                        self.locals.insert(binding, argument.ty);
                    }
                    _ => return Err(error(base.span, "unprepared type member input")),
                }
            }
            // This check executes before member inputs, even if the member does
            // not use every outer parameter. Reuse its value for a dynamic Self.
            let binding = self.spread_binding(base.span)?;
            self.locals.insert(binding, p(P::Type));
            self.alias_value_bindings.insert(
                declaration(self.package, &self.package_identities, parent),
                binding,
            );
            statements.push(n::Expr {
                source: source(base.span),
                ty: p(P::Void),
                kind: K::Let {
                    binding,
                    value: Box::new(instance_body),
                },
            });
            // The base constructor dominates the transparent member computation.
            // Keep its checked type premises local to this projection.
            let mut premises = self.requirements.get(&parent).cloned().unwrap_or_default();
            premises.extend(
                self.capabilities
                    .get(&parent)
                    .into_iter()
                    .flatten()
                    .cloned()
                    .map(n::TypePredicate::Trait),
            );
            for premise in premises {
                let premise = self.instantiate_formation_environment(
                    &premise,
                    &self.alias_substitution,
                    base.span,
                )?;
                self.context
                    .assume(crate::ptc::Formula::from_predicate(&premise));
            }
            let body = if constant {
                let H::Projection(_, field) = projection.kind else {
                    unreachable!()
                };
                let definition = Expr {
                    kind: H::ResolvedDef {
                        name: field.name,
                        has_clause_param: false,
                        def_id: member,
                    },
                    ..expr.clone()
                };
                self.expr(&definition, expected)?
            } else if contextual_alias {
                let caller = nominal::InputEnvironment {
                    substitution: &saved_substitution,
                    bindings: &saved_bindings,
                    normal_bindings: &saved_normal_bindings,
                    context: &saved_context,
                };
                self.contextual_nominal_expression_in(
                    owner,
                    supplied,
                    expr.span,
                    Some(&p(P::Type)),
                    Some(&caller),
                )?
                .ok_or_else(|| error(expr.span, "missing contextual alias input domain"))?
            } else {
                self.prepared_type_member_expression(
                    owner,
                    member_inputs,
                    member_statements,
                    expr.span,
                    Some(&p(P::Type)),
                )?
            };
            let ty = if statements.iter().any(|value| is_never(&value.ty)) {
                p(P::Never)
            } else {
                body.ty.clone()
            };
            if let Some(expected) = expected {
                self.require_expression(&ty, expected, expr.span)?;
            }
            Ok(n::Expr {
                source: source(expr.span),
                ty,
                kind: K::Block {
                    statements,
                    result: Some(Box::new(body)),
                },
            })
        })();
        self.alias_substitution = saved_substitution;
        self.alias_value_bindings = saved_bindings;
        self.normal_default_bindings = saved_normal_bindings;
        self.locals = saved_locals;
        self.context = saved_context;
        result.map(Some)
    }

    fn expose_nominal_inputs(
        &mut self,
        owner: OwnerId,
        value: n::Expr,
        span: Span,
    ) -> Result<(n::Expr, Vec<n::Expr>), Error> {
        let parent = declaration(self.package, &self.package_identities, owner);
        let domains = self.nominal_parameter_types(owner, span)?;
        // Preserve already completed Type inputs while still evaluating and
        // checking the entire parent constructor exactly once.
        let mut known = BTreeMap::new();
        let mut root = &value;
        loop {
            root = match &root.kind {
                K::Block {
                    result: Some(result),
                    ..
                } => result,
                K::CheckTypeAlias {
                    body: Some(body), ..
                } => body,
                _ => break,
            };
        }
        if let K::NominalType {
            arguments: inputs, ..
        } = &root.kind
        {
            for (index, input) in inputs.iter().enumerate() {
                if let n::NormalFormArgument::Value(n::Expr {
                    kind: K::TypeValue(ty),
                    ..
                }) = input
                {
                    known.insert(index, self.copy_alias_type(ty, span)?);
                }
            }
        }
        let mut parameter_ids = self.type_parameters[&owner].clone();
        parameter_ids.extend(self.nominal_captures(owner, span)?);
        let binders = self.input_binders(&parameter_ids);
        let known_binders = known
            .keys()
            .map(|index| binders[*index])
            .collect::<Vec<_>>();
        let known_types = known
            .values()
            .map(|ty| self.copy_alias_type(ty, span))
            .collect::<Result<Vec<_>, _>>()?;
        let mut arguments = Vec::new();
        let mut fields = Vec::new();
        for (index, domain) in domains.into_iter().enumerate() {
            let domain =
                self.instantiate_trait_domain(&domain, &known_binders, &known_types, span)?;
            let binding = self.spread_binding(span)?;
            self.locals.insert(binding, domain.clone());
            arguments.push(n::Expr {
                source: source(span),
                ty: domain,
                kind: known
                    .remove(&index)
                    .map(K::TypeValue)
                    .unwrap_or(K::Local(binding)),
            });
            fields.push(n::Pattern::Binding {
                binding,
                inner: Box::new(n::Pattern::Wild),
            });
        }
        let binding = self.spread_binding(span)?;
        self.locals.insert(binding, p(P::Type));
        let ty = value.ty.clone();
        let local = n::Expr {
            source: source(span),
            ty: p(P::Type),
            kind: K::Local(binding),
        };
        let instance = n::Expr {
            source: source(span),
            ty,
            kind: K::Block {
                statements: vec![
                    n::Expr {
                        source: source(span),
                        ty: if is_never(&value.ty) {
                            p(P::Never)
                        } else {
                            p(P::Void)
                        },
                        kind: K::Let {
                            binding,
                            value: Box::new(value),
                        },
                    },
                    n::Expr {
                        source: source(span),
                        ty: p(P::Void),
                        kind: K::LetPattern {
                            pattern: n::Pattern::NominalType {
                                declaration: parent,
                                fields,
                            },
                            value: Box::new(local.clone()),
                            guard: None,
                            otherwise: None,
                        },
                    },
                ],
                result: Some(Box::new(local)),
            },
        };
        Ok((instance, arguments))
    }

    /// Preserve an alias's evaluation prefix and expose the arguments of its
    /// final nominal constructor. This inspects generated code, never unfolds a
    /// normal-form head or re-invokes a Type computation to recover its inputs.
    fn prepared_alias_container(
        &mut self,
        base: &Expr<'_>,
    ) -> Result<Option<(DeclarationId, n::Expr, Vec<n::Expr>)>, Error> {
        let mut value = if self.completed_type_call(base)?
            || (matches!(
                base.kind,
                H::Block(_) | H::If(..) | H::When(_) | H::Match(..)
            ) && self.local_field_domain(base)? == Some(p(P::Type)))
        {
            // These computations produce a comptime Type value. Complete the
            // whole computation before exposing its nominal arguments; do not
            // assume its last expression is already a literal TypeValue.
            n::Expr {
                source: source(base.span),
                ty: p(P::Type),
                kind: K::TypeValue(self.type_expr(base)?),
            }
        } else {
            self.expr(base, None)?
        };
        if value.ty != p(P::Type) && !is_never(&value.ty) {
            return Ok(None);
        }
        let ty = value.ty.clone();
        let mut statements = Vec::new();
        loop {
            match value.kind {
                K::Block {
                    statements: prefix,
                    result: Some(result),
                } => {
                    statements.extend(prefix);
                    value = *result;
                }
                _ => break,
            }
        }
        let mut terminal = &value;
        loop {
            terminal = match &terminal.kind {
                K::Block {
                    result: Some(result),
                    ..
                } => result,
                K::CheckTypeAlias {
                    body: Some(body), ..
                } => body,
                _ => break,
            };
        }
        if let K::NominalType {
            declaration: parent,
            ..
        } = &terminal.kind
        {
            let parent = *parent;
            let owner = self
                .package
                .owners()
                .find_map(|(owner, _)| {
                    (declaration(self.package, &self.package_identities, owner) == parent)
                        .then_some(owner)
                })
                .ok_or_else(|| error(base.span, "nominal container declaration is missing"))?;
            let value = n::Expr {
                source: source(base.span),
                ty,
                kind: K::Block {
                    statements,
                    result: Some(Box::new(value)),
                },
            };
            let (instance, arguments) = self.expose_nominal_inputs(owner, value, base.span)?;
            return Ok(Some((parent, instance, arguments)));
        }
        let K::TypeValue(Term::Nominal {
            declaration: parent,
            arguments,
        }) = &value.kind
        else {
            return Ok(None);
        };
        let parent = *parent;
        let arguments = arguments
            .iter()
            .map(|argument| n::Expr {
                source: value.source,
                ty: p(P::Type),
                kind: K::TypeValue(argument.clone()),
            })
            .collect();
        let instance = n::Expr {
            source: source(base.span),
            ty,
            kind: K::Block {
                statements,
                result: Some(Box::new(value)),
            },
        };
        Ok(Some((parent, instance, arguments)))
    }

    fn with_projected_type_member<R>(
        &mut self,
        expr: &Expr<'_>,
        expand: impl FnOnce(&mut Self, OwnerId, Vec<Term>) -> Result<R, Error>,
    ) -> Result<Option<R>, Error> {
        let (projection, supplied) = match &expr.kind {
            H::NFApplication(head, arguments) => (*head, *arguments),
            H::Projection(..) => (expr, &[][..]),
            _ => return Ok(None),
        };
        let mut completed_subject = None;
        let local_receiver = self.local_type_receiver(projection, &mut completed_subject)?;
        let member = if let Some(candidates) = self
            .package
            .deferred_member_projections
            .get(&projection.hir_id)
            .or_else(|| {
                local_receiver
                    .then(|| self.package.member_access.get(&projection.hir_id))
                    .flatten()
            })
            .cloned()
        {
            let H::Projection(base, _) = projection.kind else {
                unreachable!()
            };
            if completed_subject.is_none() && !self.type_member_base(base)? {
                return Ok(None);
            }
            let subject = match completed_subject.take() {
                Some(subject) => subject,
                None => self.type_expr(base)?,
            };
            let Term::Nominal {
                declaration: parent,
                ..
            } = &subject
            else {
                return Ok(None);
            };
            let selected = candidates.iter().find_map(|(container, member)| {
                let owner = self.package.definition_owner(*container)?;
                (declaration(self.package, &self.package_identities, owner) == *parent)
                    .then_some(*member)
            });
            completed_subject = Some(subject);
            selected
        } else {
            self.package
                .member_projections
                .get(&projection.hir_id)
                .copied()
        };
        let Some(member) = member else {
            return Ok(None);
        };
        let Some(owner) = self.package.definition_owner(member) else {
            return Ok(None);
        };
        let item = self.package.item(owner).unwrap();
        if !matches!(
            item.kind,
            ItemKind::TypeAlias(..)
                | ItemKind::Newtype(..)
                | ItemKind::Struct(_)
                | ItemKind::Enum(_)
        ) {
            return Ok(None);
        }
        let H::Projection(base, _) = &projection.kind else {
            unreachable!()
        };
        let subject = match completed_subject {
            Some(subject) => subject,
            None => self.type_expr(base)?,
        };
        let Term::Nominal {
            declaration: parent_id,
            arguments: outer,
        } = subject
        else {
            return Err(error(
                expr.span,
                "type member requires a nominal container instance",
            ));
        };
        let parent = self
            .package
            .owners()
            .find_map(|(owner, _)| {
                matches_declaration(self.package, &self.package_identities, owner, parent_id)
                    .then_some(owner)
            })
            .ok_or_else(|| error(expr.span, "type member container declaration is missing"))?;
        let mut parameters = self.type_parameters[&parent].clone();
        parameters.extend(self.nominal_captures(parent, expr.span)?);
        if parameters.len() != outer.len() {
            return Err(error(
                expr.span,
                "type member container argument count mismatch",
            ));
        }
        let count = self
            .type_clause_params(owner)
            .iter()
            .filter(|clause| !matches!(clause.kind, hir::clause::ClauseParamKind::Optional(..)))
            .count();
        let domains = self.trait_domains(owner)?;
        let mixed = domains.iter().any(|domain| *domain != p(P::Type));
        let domains = if mixed {
            let binders = self.input_binders(&parameters);
            domains
                .iter()
                .map(|domain| self.instantiate_trait_domain(domain, &binders, &outer, expr.span))
                .collect::<Result<Vec<_>, _>>()?
        } else {
            domains
        };
        let environment = parameters
            .iter()
            .copied()
            .zip(outer.into_iter().map(std::sync::Arc::new))
            .collect::<BTreeMap<_, _>>();
        let (arguments, supplied) = if mixed {
            (
                Some(self.trait_clause_arguments_with_environment(
                    owner,
                    supplied,
                    None,
                    expr.span,
                    &domains,
                    Some(&environment),
                )?),
                None,
            )
        } else {
            (
                None,
                Some(self.expanded_type_inputs(supplied, count, expr.span)?),
            )
        };
        let saved = self.alias_substitution.clone();
        let saved_bindings = self.alias_value_bindings.clone();
        let saved_normal_bindings = self.normal_default_bindings.clone();
        for parameter in environment.keys() {
            self.alias_value_bindings.remove(parameter);
            self.normal_default_bindings.remove(parameter);
        }
        self.alias_substitution.extend(environment);
        let result = (|| {
            let arguments = match arguments {
                Some(arguments) => arguments,
                None => {
                    self.type_clause_values(owner, supplied.unwrap(), None, expr.span, Vec::new())?
                }
            };
            expand(self, owner, arguments)
        })();
        self.alias_substitution = saved;
        self.alias_value_bindings = saved_bindings;
        self.normal_default_bindings = saved_normal_bindings;
        result.map(Some)
    }

    #[inline(never)]
    pub(super) fn projection_expression(
        &mut self,
        expr: &Expr<'_>,
        value: &Expr<'_>,
        field: &hir::common::Ident,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        // Group prefixes are namespaces in an enum, not values. Resolve their
        // complete path before member and place probes can evaluate the prefix.
        if matches!(value.kind, H::Projection(..))
            && let Some((ty, group)) = self.enum_member_reference(value, None)?
            && matches!(&ty, Term::Nominal {declaration, ..}
                if self.enums.get(declaration).is_some_and(|definition| definition.groups.contains(&group)))
        {
            let Some((ty, index)) = self.variant_target(expr, expected)? else {
                return Err(error(expr.span, "unknown enum variant"));
            };
            let Term::Nominal { declaration, .. } = &ty else {
                unreachable!()
            };
            let name = self.enums[declaration].variants[index].name.clone();
            let variant = self.enum_variant(&ty, &name, expr.span)?;
            if let Some(expected) = expected {
                self.require(&variant.ty, expected, expr.span)?;
            }
            return Ok(variant);
        }
        if let Some(value) = self.projected_member_expression(expr, expected)? {
            return Ok(value);
        }
        if let Some(value) = self.associated_projection(value, field.name.as_str(), expr.span)? {
            if let Some(expected) = expected {
                self.require(&value.ty, expected, expr.span)?;
            }
            return Ok(value);
        }
        let (ty, kind) = {
            if let Some(pointer) = self.dereferenced_place(expr)? {
                let Term::Pointer(ty) = &pointer.ty else {
                    unreachable!()
                };
                if !self.intrinsic_pointer_input(ty) && !self.copy_read_input(ty) {
                    return Err(error(
                        expr.span,
                        "pointer reads of non-intrinsic values require Copy evidence or a memory transfer contract; this read is not yet supported",
                    ));
                }
                if let Some(expected) = expected {
                    self.require(ty, expected, expr.span)?;
                }
                return Ok(n::Expr {
                    ty: (**ty).clone(),
                    source: source(expr.span),
                    kind: K::ReadPointer(Box::new(pointer)),
                });
            }

            if matches!(field.name.as_str(), "NaN" | "Infinity") && self.is_type_head(value) {
                let ty = self.type_expr(value)?;
                if let Some(constant) =
                    self.float_associated_constant(&ty, field.name.as_str(), expr.span)?
                {
                    if let Some(expected) = expected {
                        self.require(&constant.ty, expected, expr.span)?;
                    }
                    return Ok(constant);
                }
            }
            if let Some(method) =
                self.static_method(value, field.name.as_str(), expr.span, expr.hir_id)?
            {
                if self.incomplete_method_instance(&method) {
                    return self.contextual_method_instance(expr, method, expected, partial);
                }
                return self.callable_value_view(method, expected, expr.span);
            }
            if let Some((ty, index)) = self.variant_target(expr, expected)? {
                let Term::Nominal { declaration, .. } = &ty else {
                    unreachable!()
                };
                let name = self.enums[declaration].variants[index].name.clone();
                let variant = self.enum_variant(&ty, &name, expr.span)?;
                if let Some(expected) = expected {
                    self.require(&variant.ty, expected, expr.span)?;
                }
                return Ok(variant);
            }
            if self.is_type_head(value) {
                let ty = self.type_expr(value)?;
                if let Term::Nominal { declaration, .. } = &ty {
                    if self.enums.contains_key(declaration) {
                        let variant = self.enum_variant(&ty, field.name.as_str(), expr.span)?;
                        if let Some(expected) = expected {
                            self.require(&variant.ty, expected, expr.span)?;
                        }
                        return Ok(variant);
                    }
                }
            }
            let value = self.expr(value, None)?;
            // A resolved builtin view or Type constant is already a pure
            // TypeValue. Do not erase a dynamic Type-valued computation.
            if let K::TypeValue(ty) = &value.kind {
                if let Some(constant) =
                    self.float_associated_constant(ty, field.name.as_str(), expr.span)?
                {
                    if let Some(expected) = expected {
                        self.require(&constant.ty, expected, expr.span)?;
                    }
                    return Ok(constant);
                }
            }

            let pointer_projection = matches!(value.ty, Term::Pointer(_));
            let subject = match &value.ty {
                Term::Pointer(inner) => inner.as_ref(),
                ty => ty,
            };
            let shape = self.structural_type(subject, field.span)?;
            self.ensure_nominal_type(&shape, field.span)?;
            if let Term::Record(fields) = &shape {
                let index = fields
                    .iter()
                    .position(|(name, _)| name == field.name.as_str())
                    .ok_or_else(|| {
                        error(field.span, format!("unknown record field {}", field.name))
                    })?;
                let ty = fields[index].1.clone();
                let mut projected = if pointer_projection {
                    n::Expr {
                        ty: Term::Pointer(Box::new(ty.clone())),
                        source: source(expr.span),
                        kind: K::PointerAnonymousField {
                            pointer: Box::new(value),
                            name: field.name.to_string(),
                            index,
                        },
                    }
                } else {
                    n::Expr {
                        ty: ty.clone(),
                        source: source(expr.span),
                        kind: K::AnonymousField {
                            value: Box::new(value),
                            name: field.name.to_string(),
                            index,
                        },
                    }
                };
                if pointer_projection {
                    projected = self.contextual_field_read(projected, expected, expr.span)?;
                }
                if let Some(expected) = expected {
                    self.require(&projected.ty, expected, expr.span)?;
                }
                return Ok(projected);
            }
            let Term::Nominal {
                declaration,
                arguments,
            } = &shape
            else {
                return Err(error(
                    expr.span,
                    "field projection requires a supported struct",
                ));
            };
            let def = self
                .structs
                .get(declaration)
                .ok_or_else(|| error(expr.span, "missing struct descriptor"))?;
            let index = def
                .fields
                .iter()
                .position(|(name, _)| name == field.name.as_str())
                .ok_or_else(|| error(field.span, format!("unknown struct field {}", field.name)))?;
            let ty = self.instantiate_input_types(
                &def.fields[index].1,
                &def.type_parameters,
                arguments,
                expr.span,
            )?;
            let ty = self.callable_input_view(&ty, expr.span)?;
            let declaration = *declaration;
            if pointer_projection {
                let projection = K::PointerField {
                    pointer: Box::new(value),
                    declaration,
                    index,
                };
                let pointer = n::Expr {
                    source: source(expr.span),
                    ty: Term::Pointer(Box::new(ty)),
                    kind: projection,
                };
                let projected = self.contextual_field_read(pointer, expected, expr.span)?;
                (projected.ty, projected.kind)
            } else {
                (
                    ty,
                    K::Field {
                        value: Box::new(value),
                        declaration,
                        index,
                    },
                )
            }
        };
        if let Some(expected) = expected {
            self.require(&ty, expected, expr.span)?;
        }
        Ok(n::Expr {
            ty,
            source: source(expr.span),
            kind,
        })
    }

    /// A field pointer keeps its identity without a target. With a concrete
    /// target, only built-in Copy reads may remove pointer layers.
    fn contextual_field_read(
        &self,
        mut value: n::Expr,
        expected: Option<&Term>,
        span: rustc_span::Span,
    ) -> Result<n::Expr, Error> {
        let Some(expected) = expected else {
            return Ok(value);
        };
        for _ in 0..128 {
            if self.require(&value.ty, expected, span).is_ok() {
                return Ok(value);
            }
            let Term::Pointer(pointee) = &value.ty else {
                break;
            };
            if !self.intrinsic_pointer_input(pointee) {
                break;
            }
            value = n::Expr {
                source: value.source,
                ty: (**pointee).clone(),
                kind: K::ReadPointer(Box::new(value)),
            };
        }
        self.require(&value.ty, expected, span)?;
        Ok(value)
    }
}
