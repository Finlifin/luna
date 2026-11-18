//! Transparent alias expansion, declaration-scoped completion and formation contracts.
use super::required::requirement_terms;
use super::*;

/// Only the declaration's own arguments and its lexical suppliers distinguish
/// completed applications. Caller locals and unrelated substitutions have no
/// part in the identity of a published declaration result.
#[derive(PartialEq, Eq, Hash)]
pub(super) struct Application {
    owner: OwnerId,
    arguments: Vec<Term>,
    captures: Vec<Term>,
}

impl Checker<'_, '_> {
    pub(super) fn copy_alias_type(&mut self, ty: &Term, span: Span) -> Result<Term, Error> {
        let start = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let mut fuel = start;
        let result = nir::copy_budget::clone_term(ty, &mut fuel);
        self.required_steps -= (start - fuel) as u64;
        result.map_err(|limit| {
            let message = match limit {
                nir::copy_budget::CopyBudgetExhausted::Steps => {
                    "type alias expansion budget exhausted"
                }
                nir::copy_budget::CopyBudgetExhausted::Depth => {
                    "type alias expansion depth exceeded"
                }
            };
            error(span, message)
        })
    }

    pub(super) fn check_alias_definition(
        &mut self,
        owner: OwnerId,
        value: &Expr<'_>,
        span: Span,
    ) -> Result<(), Error> {
        if self.checked_alias_definitions.contains(&owner) {
            return Ok(());
        }
        if self.aliases.len() >= 64 || !self.aliases.insert(owner) {
            return Err(error(
                span,
                "cyclic type alias or excessively deep expansion",
            ));
        }
        let saved_owner = self.signature_owner.replace(owner);
        let saved_context = self.context.clone();
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_mode = std::mem::replace(&mut self.in_required_comptime, true);
        let saved_substitution = std::mem::take(&mut self.alias_substitution);
        let saved_value_bindings = std::mem::take(&mut self.alias_value_bindings);
        let saved_normal_bindings = self.normal_default_bindings.clone();
        let saved_scopes = std::mem::take(&mut self.alias_scopes);
        self.context = self.binder_context(owner);
        // Normalize/check the target as a Type-valued expression without
        // demanding concrete results of calls whose inputs are still symbolic.
        let result = (|| {
            let mut parameters = self.nominal_captures(owner, span)?;
            let mut domains = parameters
                .iter()
                .map(|id| self.trait_input_domain(*id).unwrap_or_else(|| p(P::Type)))
                .collect::<Vec<_>>();
            parameters.extend(self.type_parameters[&owner].iter().copied());
            domains.extend(self.trait_domains(owner)?);
            for (parameter, domain) in parameters.into_iter().zip(domains) {
                if domain != p(P::Type) {
                    let binding = self.spread_binding(span)?;
                    self.locals.insert(binding, domain.clone());
                    self.normal_default_bindings
                        .insert(parameter, (binding, domain));
                }
            }
            self.expr(value, Some(&p(P::Type))).map(|_| ())
        })();
        self.signature_owner = saved_owner;
        self.context = saved_context;
        self.locals = saved_locals;
        self.in_required_comptime = saved_mode;
        self.alias_substitution = saved_substitution;
        self.alias_value_bindings = saved_value_bindings;
        self.alias_scopes = saved_scopes;
        self.normal_default_bindings = saved_normal_bindings;
        self.aliases.remove(&owner);
        if result.is_ok() {
            self.checked_alias_definitions.insert(owner);
        }
        result
    }

    pub(super) fn completed_alias(
        &mut self,
        owner: OwnerId,
        value: &Expr<'_>,
        arguments: &[Term],
        span: Span,
    ) -> Result<CompletedType, Error> {
        let parameters = self
            .type_parameters
            .get(&owner)
            .cloned()
            .unwrap_or_default();
        if parameters.len() != arguments.len() {
            return Err(error(span, "type alias argument count mismatch"));
        }
        // Bound work before copying cache keys or complete expansion results.
        let arguments = arguments
            .iter()
            .map(|ty| self.copy_alias_type(ty, span))
            .collect::<Result<Vec<_>, _>>()?;
        let mut captures = self.nominal_capture_arguments(owner, span)?;
        captures.extend(self.lexical_arguments(owner, span)?);
        let key = Application {
            owner,
            arguments,
            captures,
        };
        if let Some(data) = self.completed_aliases.get(&key).cloned() {
            return self.copy_completed_type(&data, span);
        }
        let data =
            self.with_alias_arguments(owner, &parameters, &key.arguments, span, |checker| {
                checker.completed_type(value)
            })?;
        let cached = self.copy_completed_type(&data, span)?;
        self.completed_aliases
            .insert(key, std::sync::Arc::new(cached));
        Ok(data)
    }

    fn copy_completed_type(
        &mut self,
        data: &CompletedType,
        span: Span,
    ) -> Result<CompletedType, Error> {
        match data {
            CompletedType::Identity(ty) => {
                self.copy_alias_type(ty, span).map(CompletedType::Identity)
            }
            CompletedType::Description(leaf) => {
                self.required_steps = self
                    .required_steps
                    .checked_sub(1)
                    .ok_or_else(|| error(span, "type alias expansion budget exhausted"))?;
                Ok(CompletedType::Description(leaf.clone()))
            }
        }
    }

    pub(super) fn bare_alias(
        &mut self,
        owner: OwnerId,
        target: &Expr<'_>,
        span: Span,
    ) -> Result<CompletedType, Error> {
        let arguments =
            self.type_clause_arguments(owner, &[], None, span)
                .map_err(|mut failure| {
                    if failure.message == "type argument count mismatch" {
                        failure.message =
                            "generic type alias requires explicit type arguments".into();
                    }
                    failure
                })?;
        self.check_type_application(owner, &arguments, span)?;
        self.completed_alias(owner, target, &arguments, span)
    }

    /// A type position completes suppliers in source order. When they all have
    /// stable terms the application retains its ordinary declaration identity.
    /// A description can instead be bound as Type data by the residual plan.
    pub(super) fn completed_alias_application(
        &mut self,
        owner: OwnerId,
        target: &Expr<'_>,
        args: &[Arg<'_>],
        site: &Expr<'_>,
    ) -> Result<CompletedType, Error> {
        let span = site.span;
        let parameters = self.type_parameters[&owner].len();
        if parameters == 0 {
            return Err(error(span, "generic type alias argument count mismatch"));
        }
        let alias_name = self.package.item(owner).unwrap().ident.name;
        let argument_error = |mut failure: Error| {
            failure.message = format!(
                "in arguments to type alias {alias_name}: {}",
                failure.message
            );
            failure
        };
        if self.has_optional_type_clauses(owner)
            || self
                .trait_domains(owner)?
                .iter()
                .any(|domain| *domain != p(P::Type))
        {
            let arguments = self
                .type_clause_arguments(owner, args, None, span)
                .map_err(argument_error)?;
            self.check_type_application(owner, &arguments, span)?;
            return self.completed_alias(owner, target, &arguments, span);
        }
        let inputs = self.expand_type_inputs_with(args, parameters, span, |data, _| Ok(data))?;
        let slots = Self::route_type_slots(&self.type_clause_params(owner), inputs, span)
            .map_err(argument_error)?;
        let supplied = slots
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| error(span, "generic type alias argument count mismatch"))?;
        if supplied
            .iter()
            .any(|value| matches!(value, CompletedType::Description(_)))
        {
            let values = supplied
                .into_iter()
                .map(|data| data.into_expression(span))
                .collect();
            let checked = self.alias_value_expression(owner, target, values, span)?;
            return self.complete_checked_type(site, checked);
        }
        let arguments = supplied
            .into_iter()
            .map(|data| data.identity(span))
            .collect::<Result<Vec<_>, _>>()?;
        self.check_type_application(owner, &arguments, span)?;
        self.completed_alias(owner, target, &arguments, span)
    }

    pub(super) fn applied_alias_expression(
        &mut self,
        owner: OwnerId,
        target: &Expr<'_>,
        args: &[Arg<'_>],
        span: Span,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        if self.type_parameters[&owner].is_empty() {
            return Err(error(span, "generic type alias argument count mismatch"));
        }
        if self.has_optional_type_clauses(owner)
            || self
                .trait_domains(owner)?
                .iter()
                .any(|domain| *domain != p(P::Type))
            || args.iter().any(|arg| !matches!(arg, Arg::Positional(_)))
        {
            return self.defaulted_type_expression(owner, args, span, expected);
        }
        if args.len() != self.type_parameters[&owner].len() {
            return Err(error(span, "generic type alias argument count mismatch"));
        }
        let mut inputs = Vec::with_capacity(args.len());
        for arg in args {
            let Arg::Positional(input) = arg else {
                unreachable!()
            };
            inputs.push(self.expr(input, Some(&p(P::Type)))?);
        }
        let result = self.alias_value_expression(owner, target, inputs, span)?;
        if let Some(expected) = expected {
            self.require_expression(&result.ty, expected, span)?;
        }
        Ok(result)
    }

    // Keep a Type-valued computation residual while checking another alias.
    // This is transparent substitution, not a new nominal or normal-form head.
    pub(super) fn alias_expression(
        &mut self,
        owner: OwnerId,
        value: &Expr<'_>,
        arguments: &[Term],
        span: Span,
    ) -> Result<n::Expr, Error> {
        let parameters = self.type_parameters[&owner].clone();
        if parameters.len() != arguments.len() {
            return Err(error(span, "type alias argument count mismatch"));
        }
        let arguments = arguments
            .iter()
            .map(|ty| self.copy_alias_type(ty, span))
            .collect::<Result<Vec<_>, _>>()?;
        self.with_alias_arguments(owner, &parameters, &arguments, span, |checker| {
            checker.expr(value, Some(&p(P::Type)))
        })
    }

    /// Publish the declaration contract and return its lexical input prefix.
    /// The alias's own clause parameters always follow that prefix.
    pub(super) fn alias_formation_captures(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<Vec<DeclarationId>, Error> {
        let declaration = declaration(self.package, &self.package_identities, owner);
        if let Some(contract) = self.alias_contracts.get(&declaration) {
            let own_count = self.type_parameters[&owner].len();
            return Ok(contract.parameters[..contract.parameters.len() - own_count].to_vec());
        }
        self.ensure_contract(owner, span)?;
        let mut requirements = self.requirements[&owner].clone();
        let mut capabilities = self.capabilities.get(&owner).cloned().unwrap_or_default();
        let mut container = self.self_containers.get(&owner).copied();
        let mut depth = 0;
        while let Some(parent) = container {
            depth += 1;
            if depth > 64 {
                return Err(error(
                    span,
                    "alias formation environment is excessively deep",
                ));
            }
            self.ensure_contract(parent, span)?;
            for premise in &self.requirements[&parent] {
                if !requirements.contains(premise) {
                    requirements.push(premise.clone());
                }
            }
            for capability in self.capabilities.get(&parent).into_iter().flatten() {
                if !capabilities.contains(capability) {
                    capabilities.push(capability.clone());
                }
            }
            container = self.self_containers.get(&parent).copied();
        }
        let mut parameters = self.type_parameters[&owner].clone();
        let mut domains = self.trait_domains(owner)?;
        if domains.is_empty() {
            domains.resize(parameters.len(), p(P::Type));
        }
        if requirements.is_empty()
            && capabilities.is_empty()
            && domains.iter().all(|ty| *ty == p(P::Type))
        {
            return Ok(Vec::new());
        }
        let captures =
            self.ordered_alias_captures(&parameters, &domains, &requirements, &capabilities, span)?;
        let (mut ids, mut types): (Vec<_>, Vec<_>) = captures.into_iter().unzip();
        let capture_ids = ids.clone();
        ids.append(&mut parameters);
        types.append(&mut domains);
        self.alias_contracts.insert(
            declaration,
            nir::type_aliases::Contract {
                declaration,
                parameters: ids,
                parameter_types: types,
                requirements,
                capabilities,
            },
        );
        Ok(capture_ids)
    }

    /// Close the contract over free declaration inputs, then order its prefix
    /// by domain dependencies. Ready inputs retain declaration-ID tie order.
    fn ordered_alias_captures(
        &mut self,
        parameters: &[DeclarationId],
        domains: &[Term],
        requirements: &[n::TypePredicate],
        capabilities: &[n::TraitRequirement],
        span: Span,
    ) -> Result<Vec<(DeclarationId, Term)>, Error> {
        let own: BTreeSet<_> = parameters.iter().copied().collect();
        let mut references = Vec::new();
        let mut observe = |term| -> Result<(), Error> {
            references.extend(
                input_observation::term_inputs(term, &mut self.required_steps, span)?.declarations,
            );
            Ok(())
        };
        for domain in domains {
            observe(domain)?;
        }
        for predicate in requirements {
            for term in requirement_terms(predicate, span)? {
                observe(term)?;
            }
        }
        for capability in capabilities {
            observe(&capability.subject)?;
            for argument in &capability.interface_arguments {
                observe(argument)?;
            }
        }

        let mut captures: BTreeMap<DeclarationId, (Term, usize)> = BTreeMap::new();
        let mut consumers = BTreeMap::<DeclarationId, Vec<DeclarationId>>::new();
        let mut ready = BTreeSet::new();
        while let Some((id, sort)) = references.pop() {
            if own.contains(&id) {
                continue;
            }
            let domain = match captures.get(&id) {
                Some((domain, _)) if Sort::of_comptime_domain(domain) == sort => continue,
                Some(_) => return Err(error(span, "alias capture domain has incompatible sort")),
                None => self.trait_input_domain(id).unwrap_or_else(|| p(P::Type)),
            };
            if Sort::of_comptime_domain(&domain) != sort {
                return Err(error(span, "alias capture domain has incompatible sort"));
            }
            let dependencies =
                input_observation::term_inputs(&domain, &mut self.required_steps, span)?
                    .declarations;
            if dependencies.is_empty() {
                ready.insert(id);
            }
            captures.insert(id, (domain, dependencies.len()));
            for (dependency, sort) in dependencies {
                consumers.entry(dependency).or_default().push(id);
                references.push((dependency, sort));
            }
        }

        let mut ordered = Vec::with_capacity(captures.len());
        while let Some(id) = ready.pop_first() {
            let (domain, _) = captures.remove(&id).unwrap();
            ordered.push((id, domain));
            for dependent in consumers.remove(&id).into_iter().flatten() {
                let (_, waiting) = captures.get_mut(&dependent).unwrap();
                *waiting -= 1;
                if *waiting == 0 {
                    ready.insert(dependent);
                }
            }
        }
        if !captures.is_empty() {
            return Err(error(span, "cyclic or unavailable alias capture domain"));
        }
        Ok(ordered)
    }

    pub(super) fn alias_value_expression(
        &mut self,
        owner: OwnerId,
        value: &Expr<'_>,
        arguments: Vec<n::Expr>,
        span: Span,
    ) -> Result<n::Expr, Error> {
        self.charge_alias_expression(span)?;
        let parameters = self.type_parameters[&owner].clone();
        let mut terms = Vec::new();
        for (argument, parameter) in arguments.iter().zip(&parameters) {
            terms.push(if let K::TypeValue(ty) = &argument.kind {
                self.copy_alias_type(ty, span)?
            } else {
                Term::Rigid {
                    declaration: *parameter,
                    sort: self.input_binders(&[*parameter])[0].sort,
                }
            });
        }
        if arguments
            .iter()
            .all(|argument| matches!(argument.kind, K::TypeValue(_)))
        {
            self.check_type_application(owner, &terms, span)?;
            return self.alias_expression(owner, value, &terms, span);
        }
        let captures = self.alias_formation_captures(owner, span)?;
        let mut deferred: BTreeSet<_> = parameters
            .iter()
            .zip(&arguments)
            .filter(|(_, argument)| !matches!(argument.kind, K::TypeValue(_)))
            .map(|(id, _)| *id)
            .collect();
        deferred.extend(
            captures
                .iter()
                .chain(&parameters)
                .filter(|id| {
                    self.alias_value_bindings.contains_key(id)
                        || self.normal_default_bindings.contains_key(id)
                })
                .copied(),
        );
        self.check_formation_application(
            owner,
            FormationInputs {
                terms: terms.clone(),
                deferred,
                environment: DeclarationBindings::default(),
            },
            span,
        )?;
        let diverges = arguments.iter().any(|argument| is_never(&argument.ty));
        self.with_alias_arguments(owner, &parameters, &terms, span, |checker| {
            let mut statements = Vec::new();
            let guard = checker.alias_contracts.contains_key(&declaration(
                checker.package,
                &checker.package_identities,
                owner,
            ));
            let mut guard_inputs = Vec::new();
            for (argument, parameter) in arguments.into_iter().zip(&parameters) {
                if matches!(argument.kind, K::TypeValue(_)) {
                    if guard {
                        guard_inputs.push(argument);
                    }
                    continue;
                }
                let binding = checker.spread_binding(span)?;
                // A diverging input never initializes this local. Preserve the
                // declared domain for the unreachable alias body and guard.
                let binding_ty = if is_never(&argument.ty) {
                    checker
                        .trait_input_domain(*parameter)
                        .unwrap_or_else(|| p(P::Type))
                } else {
                    argument.ty.clone()
                };
                checker.install_alias_local(*parameter, binding, binding_ty.clone());
                if guard {
                    guard_inputs.push(n::Expr {
                        source: argument.source,
                        ty: binding_ty,
                        kind: K::Local(binding),
                    });
                }
                statements.push(n::Expr {
                    source: argument.source,
                    ty: if is_never(&argument.ty) {
                        p(P::Never)
                    } else {
                        p(P::Void)
                    },
                    kind: K::Let {
                        binding,
                        value: Box::new(argument),
                    },
                });
            }
            let body = checker.check_alias_value_body(value)?;
            let body = if guard {
                let mut guard_arguments = Vec::with_capacity(captures.len() + guard_inputs.len());
                for id in captures {
                    guard_arguments.push(checker.captured_input_expression(id, span)?);
                }
                guard_arguments.extend(guard_inputs);
                n::Expr {
                    source: source(span),
                    ty: body.ty.clone(),
                    kind: K::CheckTypeAlias {
                        declaration: declaration(
                            checker.package,
                            &checker.package_identities,
                            owner,
                        ),
                        arguments: guard_arguments
                            .into_iter()
                            .map(n::NormalFormArgument::Value)
                            .collect(),
                        preparations: vec![],
                        bindings: None,
                        order: None,
                        body: Some(Box::new(body)),
                    },
                }
            } else {
                body
            };
            Ok(n::Expr {
                source: source(span),
                ty: if diverges {
                    p(P::Never)
                } else {
                    body.ty.clone()
                },
                kind: K::Block {
                    statements,
                    result: Some(Box::new(body)),
                },
            })
        })
    }

    /// The declaration-input frame evaluates and admits the whole telescope
    /// before it enters this body. Reads refer to that frame's original slots.
    pub(super) fn planned_alias_body(
        &mut self,
        owner: OwnerId,
        value: &Expr<'_>,
        bindings: &[n::LocalId],
        domains: &[Term],
        span: Span,
    ) -> Result<n::Expr, Error> {
        self.charge_alias_expression(span)?;
        let parameters = self.type_parameters[&owner].clone();
        let terms = self
            .input_binders(&parameters)
            .into_iter()
            .map(|binder| Term::Rigid {
                declaration: binder.declaration,
                sort: binder.sort,
            })
            .collect::<Vec<_>>();
        self.with_alias_arguments(owner, &parameters, &terms, span, |checker| {
            for ((&id, &binding), domain) in parameters.iter().zip(bindings).zip(domains) {
                let ty = checker.copy_alias_type(domain, span)?;
                checker.install_alias_local(id, binding, ty);
            }
            checker.check_alias_value_body(value)
        })
    }

    fn charge_alias_expression(&mut self, span: Span) -> Result<(), Error> {
        self.alias_expression_budget = self
            .alias_expression_budget
            .checked_sub(1)
            .ok_or_else(|| error(span, "type alias expression expansion budget exhausted"))?;
        Ok(())
    }

    fn install_alias_local(&mut self, parameter: DeclarationId, binding: n::LocalId, ty: Term) {
        if ty == p(P::Type) {
            self.alias_value_bindings.insert(parameter, binding);
            self.normal_default_bindings.remove(&parameter);
        } else {
            self.normal_default_bindings
                .insert(parameter, (binding, ty.clone()));
            self.alias_value_bindings.remove(&parameter);
        }
        self.locals.insert(binding, ty);
    }

    fn check_alias_value_body(&mut self, value: &Expr<'_>) -> Result<n::Expr, Error> {
        let mode = std::mem::replace(&mut self.in_required_comptime, true);
        let body = self.expr(value, Some(&p(P::Type)));
        self.in_required_comptime = mode;
        body
    }

    fn with_alias_arguments<R>(
        &mut self,
        owner: OwnerId,
        parameters: &[DeclarationId],
        arguments: &[Term],
        span: Span,
        expand: impl FnOnce(&mut Self) -> Result<R, Error>,
    ) -> Result<R, Error> {
        if self.aliases.len() >= 64 || !self.aliases.insert(owner) {
            return Err(error(
                span,
                "cyclic type alias or excessively deep expansion",
            ));
        }
        let generic = !parameters.is_empty();
        let saved_owner = self.signature_owner.replace(owner);
        let caller = saved_owner.and_then(|scope| {
            self.alias_scopes
                .get(&scope)
                .copied()
                .unwrap_or(Some(scope))
        });
        let previous_scope = if generic {
            self.alias_scopes.insert(owner, caller)
        } else {
            None
        };
        let saved_context = self.context.clone();
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_bindings = self.alias_value_bindings.clone();
        let saved_normal_bindings = self.normal_default_bindings.clone();
        let saved_substitution = self.alias_substitution.clone();
        self.alias_substitution.extend(
            parameters
                .iter()
                .copied()
                .zip(arguments.iter().cloned().map(std::sync::Arc::new)),
        );
        if !generic {
            self.context = self.binder_context(owner);
        }
        let result = expand(self);
        self.signature_owner = saved_owner;
        self.context = saved_context;
        self.locals = saved_locals;
        self.alias_value_bindings = saved_bindings;
        self.normal_default_bindings = saved_normal_bindings;
        self.alias_substitution = saved_substitution;
        if generic {
            if let Some(scope) = previous_scope {
                self.alias_scopes.insert(owner, scope);
            } else {
                self.alias_scopes.remove(&owner);
            }
        }
        self.aliases.remove(&owner);
        result
    }
}
