//! Test templates compose checked expression code with a runtime setup body.
use super::*;

impl Checker<'_, '_> {
    pub(in crate::program) fn expand_test_templates(
        &mut self,
        output: &mut n::Package,
    ) -> Result<(), Error> {
        let package = self.package;
        // Check the shared body even if no instance selects it.
        for (owner, info) in package.owners() {
            if matches!(&info.node.expect_item().kind, ItemKind::Fn(sig, _) if sig.modifiers.is_test_template)
            {
                self.check_test_template(owner, None)?;
            }
        }
        for (owner, info) in package.owners() {
            let item = info.node.expect_item();
            let ItemKind::TestTemplateInstance {
                template,
                interface,
                ports,
                body,
            } = &item.kind
            else {
                continue;
            };
            let (template, arguments) = match &template.kind {
                H::NFApplication(head, arguments) => (*head, *arguments),
                _ => (*template, &[][..]),
            };
            let H::ResolvedDef { def_id, .. } = template.kind else {
                return Err(error(item.span, "test template requires a declaration"));
            };
            let target = package
                .definition_owner(def_id)
                .ok_or_else(|| error(item.span, "missing test template declaration"))?;
            let interface = interface
                .as_ref()
                .ok_or_else(|| error(item.span, "test template interface was not linked"))?;
            let (value, contract) = self
                .check_test_template(target, Some((interface, ports, *body, owner, arguments)))?;
            let id = declaration(package, &self.package_identities, owner);
            output.tests.push(id);
            output.functions.push(n::Function {
                phase: n::FunctionPhase::Runtime,
                inline: false,
                pure: false,
                is_unsafe: false,
                declaration: id,
                name: item.ident.name.to_string(),
                signature: Term::Function {
                    parameters: vec![],
                    result: Box::new(p(P::Void)),
                },
                parameters: vec![],
                type_parameters: vec![],
                requirements: vec![],
                capabilities: vec![],
                defaults: vec![],
                preconditions: contract.preconditions,
                postconditions: contract.postconditions,
                entry_snapshots: contract.entry_snapshots,
                result_binding: contract.result_binding,
                body: value,
            });
        }
        output.tests.clear();
        for (owner, info) in package.owners() {
            if matches!(&info.node.expect_item().kind, ItemKind::Fn(sig, _) if sig.modifiers.is_test)
                || matches!(
                    info.node.expect_item().kind,
                    ItemKind::TestTemplateInstance { .. }
                )
            {
                output
                    .tests
                    .push(declaration(package, &self.package_identities, owner));
            }
        }
        Ok(())
    }

    fn check_test_template(
        &mut self,
        owner: OwnerId,
        instance: Option<(
            &hir::item::TestTemplateInterface<'_>,
            &[hir::item::TestTemplatePort],
            hir::BodyId,
            OwnerId,
            &[hir::common::Arg<'_>],
        )>,
    ) -> Result<(n::Expr, nir::contracts::ProgramContract), Error> {
        let saved_parameters = self.normal_parameters.remove(&owner);
        let saved_capabilities = self.capabilities.remove(&owner);
        let result = self.generator_frame(owner, |checker| {
            checker.check_test_template_in_frame(owner, instance)
        });
        self.capabilities.remove(&owner);
        if let Some(capabilities) = saved_capabilities {
            self.capabilities.insert(owner, capabilities);
        }
        self.normal_parameters.remove(&owner);
        if let Some(parameters) = saved_parameters {
            self.normal_parameters.insert(owner, parameters);
        }
        result
    }

    fn check_test_template_in_frame(
        &mut self,
        owner: OwnerId,
        instance: Option<(
            &hir::item::TestTemplateInterface<'_>,
            &[hir::item::TestTemplatePort],
            hir::BodyId,
            OwnerId,
            &[hir::common::Arg<'_>],
        )>,
    ) -> Result<(n::Expr, nir::contracts::ProgramContract), Error> {
        let package = self.package;
        let item = package.item(owner).unwrap();
        let ItemKind::Fn(signature, body) = &item.kind else {
            unreachable!()
        };
        let parameter = signature
            .test_template_body()
            .map_err(|message| error(item.span, message))?
            .ok_or_else(|| error(item.span, "not a test template"))?;
        let mut kind = &parameter.kind;
        while let hir::ClauseParamKind::Quote(inner) = kind {
            kind = inner;
        }
        let hir::ClauseParamKind::Positional(_, annotation) = kind else {
            return Err(error(
                item.span,
                "test template body requires an expression-code interface",
            ));
        };
        let definition = parameter
            .definition
            .ok_or_else(|| error(item.span, "missing template body identity"))?;
        let saved_owner = self.signature_owner.replace(owner);
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_immutable = std::mem::take(&mut self.immutable);
        let context = self.nominal_context();
        let saved_context = std::mem::replace(&mut self.context, context);
        let saved_code = std::mem::take(&mut self.template_code_values);
        let saved_checking = self.checking_code_generator;
        let result = (|| {
            self.checking_code_generator = instance.is_none();
            self.prepare_test_template_defaults(
                signature,
                parameter.hir_id,
                instance.map(|(_, _, _, owner, args)| (owner, args)),
            )?;
            let mut contract = self.test_template_contract(signature, instance.is_none())?;
            let formal = self.code_interface(annotation)?;
            let code = if let Some((linked, ports, body, instance_owner, _)) = instance {
                if linked.parameter != parameter.hir_id {
                    return Err(error(item.span, "test template body identity mismatch"));
                }
                let mut actual = formal.clone();
                actual.context.clear();
                actual.ports.clear();
                actual.defaults.clear();
                for port in ports {
                    let from = local(
                        port.formal
                            .ok_or_else(|| error(port.name.span, "unlinked template port"))?,
                    );
                    let to = local(port.binding);
                    let ty = formal
                        .context
                        .get(&from)
                        .ok_or_else(|| error(port.name.span, "unknown formal template port"))?
                        .clone();
                    actual.context.insert(to, ty.clone());
                    actual.ports.insert(port.name.name.to_string(), to);
                    self.locals.insert(to, ty);
                }
                self.signature_owner = Some(instance_owner);
                self.checking_code_generator = false;
                let source_body = package
                    .body(body)
                    .ok_or_else(|| error(item.span, "missing test instance body"))?;
                self.required_steps = self.required_steps.checked_sub(1).ok_or_else(|| {
                    error(
                        item.span,
                        "test template code construction budget exhausted",
                    )
                })?;
                let code = self.quote_expression(
                    source_body.value,
                    source_body.value,
                    &[],
                    Some(&actual),
                    None,
                    Some(p(P::Void)),
                )?;
                self.signature_owner = Some(owner);
                self.ascribe_code(code, Some(&formal), item.span)?
            } else {
                Arc::new(ExpressionCode {
                    body: None,
                    result: formal.result.clone(),
                    context: formal.context.clone(),
                    ports: formal.ports.clone(),
                    defaults: formal.defaults.clone(),
                })
            };
            self.locals.clear();
            self.template_code_values
                .insert((definition.pkg, definition.index), code);
            self.checking_code_generator = instance.is_none();
            let body = package
                .body(*body)
                .ok_or_else(|| error(item.span, "missing template setup body"))?;
            let mut value = self.quoted_runtime_boundary(|checker| {
                checker.result = Some(p(P::Void));
                checker.computation_body(body.value, Some(&p(P::Void)))
            })?;
            if instance.is_some() {
                freshen(
                    &mut value,
                    BTreeMap::new(),
                    &mut self.next_temporary_binding,
                    &mut self.expansion_sources,
                    item.span,
                )?;
            }
            if instance.is_some() {
                for predicate in contract
                    .preconditions
                    .iter_mut()
                    .chain(&mut contract.postconditions)
                {
                    freshen(
                        predicate,
                        BTreeMap::new(),
                        &mut self.next_temporary_binding,
                        &mut self.expansion_sources,
                        item.span,
                    )?;
                }
            }
            Ok((value, contract))
        })();
        self.checking_code_generator = saved_checking;
        self.template_code_values = saved_code;
        self.context = saved_context;
        self.immutable = saved_immutable;
        self.locals = saved_locals;
        self.signature_owner = saved_owner;
        result
    }

    // Optional clauses do not consume the anchored body input. Check defaults
    // against the declaration prefix, then install either abstract evidence or
    // the invocation's computed immutable value.
    fn prepare_test_template_defaults(
        &mut self,
        signature: &hir::item::FnSig<'_>,
        body: hir::HirId,
        instance: Option<(OwnerId, &[hir::common::Arg<'_>])>,
    ) -> Result<(), Error> {
        let abstract_inputs = instance.is_none();
        let mut supplied = BTreeMap::new();
        let mut positional = Vec::new();
        let variadics = signature
            .clause_params
            .iter()
            .filter(|clause| matches!(clause.kind, hir::ClauseParamKind::Varadic(..)))
            .count();
        let named_inputs = instance
            .map(|(_, arguments)| {
                arguments
                    .iter()
                    .filter_map(|arg| {
                        if let hir::common::Arg::Named(name, _) = arg {
                            Some(name.name)
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let remaining_variadics = signature
            .clause_params
            .iter()
            .filter(|clause| {
                matches!(clause.kind, hir::ClauseParamKind::Varadic(..))
                    && !named_inputs.contains(&clause.name.name)
            })
            .collect::<Vec<_>>();
        if let Some((_, arguments)) = instance {
            for argument in arguments {
                if let hir::common::Arg::Positional(value) = argument {
                    if remaining_variadics.len() == 1 {
                        positional.push(((*value).clone(), false));
                        continue;
                    }
                }
                if let hir::common::Arg::Expand(value) = argument {
                    if remaining_variadics.len() == 1 {
                        positional.push(((*value).clone(), true));
                        continue;
                    }
                }
                if matches!(
                    argument,
                    hir::common::Arg::Positional(_) | hir::common::Arg::Expand(_)
                ) && variadics != 0
                {
                    return Err(error(
                        signature.clause_params[0].span,
                        if remaining_variadics.is_empty() {
                            "test template positional inputs cannot append to directly supplied collections"
                        } else {
                            "ambiguous test template variadic input; supply complete named packages"
                        },
                    ));
                }
                let hir::common::Arg::Named(name, value) = argument else {
                    return Err(error(
                        self.package
                            .item(self.signature_owner.unwrap())
                            .unwrap()
                            .span,
                        "test template inputs require named optional arguments; the anchored input is supplied by the test body",
                    ));
                };
                if !signature.clause_params.iter().any(|clause| {
                    clause.name.name == name.name
                        && matches!(
                            clause.kind,
                            hir::ClauseParamKind::Optional(..) | hir::ClauseParamKind::Varadic(..)
                        )
                }) {
                    return Err(error(name.span, "unknown optional test template argument"));
                }
                if supplied.insert(name.name.to_string(), *value).is_some() {
                    return Err(error(name.span, "duplicate test template argument"));
                }
            }
        }
        let mut pending_capabilities = Vec::new();
        if abstract_inputs {
            for clause in signature.clause_constraints {
                if let hir::clause::ClauseConstraintKind::Requires(value) = clause.kind {
                    super::super::required::signature_capability_atoms(
                        value,
                        &mut pending_capabilities,
                        0,
                    )?;
                }
            }
        }
        let saved_forbidden = self.forbidden_default_inputs.clone();
        let saved_required = std::mem::replace(&mut self.in_required_comptime, true);
        let result = (|| {
            for clause in signature.clause_params {
                if clause.hir_id == body {
                    continue;
                }
                let definition = clause
                    .definition
                    .ok_or_else(|| error(clause.span, "missing template clause identity"))?;
                self.forbidden_default_inputs.insert(DeclarationId {
                    package: semantic_package(&self.package_identities, definition.pkg),
                    local: definition.index,
                });
            }
            for clause in signature.clause_params {
                if clause.hir_id == body {
                    continue;
                }
                if let hir::ClauseParamKind::Varadic(_, annotation) = clause.kind {
                    let definition = clause.definition.unwrap();
                    let id = DeclarationId {
                        package: semantic_package(&self.package_identities, definition.pkg),
                        local: definition.index,
                    };
                    let domain = self.type_expr(annotation)?;
                    if !self.is_type_list_type(&domain)
                        && !matches!(self.structural_type(&domain, clause.span)?, Term::Tuple(_))
                    {
                        return Err(error(
                            clause.span,
                            "test template variadic collection requires a tuple layout or the bound meta.List<Type>",
                        ));
                    }
                    let binding = local(clause.hir_id);
                    self.normal_parameters
                        .entry(self.signature_owner.unwrap())
                        .or_default()
                        .push((id, binding, domain.clone()));
                    let value = if abstract_inputs {
                        None
                    } else {
                        let named = supplied.get(&clause.name.name.to_string()).copied();
                        let previous_owner = self.signature_owner;
                        self.signature_owner = instance.map(|(owner, _)| owner);
                        let result = if let Some(input) = named {
                            self.required_typed_expression(input, Some(&domain))
                        } else {
                            self.collect_test_template_inputs(&positional, &domain, clause.span)
                        };
                        self.signature_owner = previous_owner;
                        let (value, ty) = result?;
                        Some(self.materialize_generator_data(value, ty, clause.span)?)
                    };
                    self.locals.insert(binding, domain.clone());
                    self.normal_default_bindings.insert(id, (binding, domain));
                    self.generator_data.insert(binding, value);
                    self.forbidden_default_inputs.remove(&id);
                    if abstract_inputs {
                        self.ready_signature_capabilities_with_clauses(
                            self.signature_owner.unwrap(),
                            &mut pending_capabilities,
                            false,
                            true,
                        )?;
                    }
                    continue;
                }
                let hir::ClauseParamKind::Optional(_, annotation, default) = clause.kind else {
                    return Err(error(
                        clause.span,
                        "test template variadic clause inputs are not yet implemented",
                    ));
                };
                let definition = clause.definition.unwrap();
                let id = DeclarationId {
                    package: semantic_package(&self.package_identities, definition.pkg),
                    local: definition.index,
                };
                let domain = self.type_expr(annotation)?;
                let type_inputs = self.alias_substitution.keys().copied().collect::<Vec<_>>();
                if !self.generator_data_domain(&domain, &type_inputs) {
                    return Err(error(
                        clause.span,
                        "test template default requires an immutable value domain",
                    ));
                }
                self.normal_parameters
                    .entry(self.signature_owner.unwrap())
                    .or_default()
                    .push((id, local(clause.hir_id), domain.clone()));
                let explicit = supplied.get(&clause.name.name.to_string()).copied();
                let default = explicit.unwrap_or(default);
                let saved_owner = self.signature_owner;
                if explicit.is_some() {
                    self.signature_owner = instance.map(|(owner, _)| owner);
                }
                let binding = local(clause.hir_id);
                if domain == p(P::Type) {
                    let value = self.type_expr(default)?;
                    self.alias_substitution.insert(
                        id,
                        Arc::new(if abstract_inputs {
                            Term::Rigid {
                                declaration: id,
                                sort: Sort::Type,
                            }
                        } else {
                            value
                        }),
                    );
                } else {
                    let value = if abstract_inputs {
                        let checked = self.expr(default, Some(&domain))?;
                        self.validate_generator_data(&checked, default.span)?;
                        None
                    } else {
                        let (value, ty) = self.required_typed_expression(default, Some(&domain))?;
                        Some(self.materialize_generator_data(value, ty, default.span)?)
                    };
                    self.locals.insert(binding, domain.clone());
                    self.normal_default_bindings.insert(id, (binding, domain));
                    self.generator_data.insert(binding, value);
                }
                self.signature_owner = saved_owner;
                self.forbidden_default_inputs.remove(&id);
                if abstract_inputs {
                    // Only the completed prefix may authorize the next
                    // default. A later clause must remain unavailable.
                    self.ready_signature_capabilities_with_clauses(
                        self.signature_owner.unwrap(),
                        &mut pending_capabilities,
                        false,
                        true,
                    )?;
                }
            }
            Ok(())
        })();
        self.in_required_comptime = saved_required;
        self.forbidden_default_inputs = saved_forbidden;
        result
    }

    fn collect_test_template_inputs(
        &mut self,
        inputs: &[(Expr<'_>, bool)],
        domain: &Term,
        span: Span,
    ) -> Result<(comptime_interpretor::Value, Term), Error> {
        use comptime_interpretor::Value;
        if self.is_type_list_type(domain) {
            let mut elements = Vec::new();
            for (input, expand) in inputs {
                self.required_steps = self.required_steps.checked_sub(1).ok_or_else(|| {
                    error(
                        input.span,
                        "test template type-list collection budget exhausted",
                    )
                })?;
                if *expand {
                    let (value, ty) = self.required_typed_expression(input, Some(domain))?;
                    if !self.is_type_list_type(&ty) {
                        return Err(error(
                            input.span,
                            "type-list spread requires the bound meta.List<Type>",
                        ));
                    }
                    let items = match value {
                        Value::List { ty, values } if ty == *domain => values
                            .into_iter()
                            .map(|value| match value {
                                Value::Type(ty) => Ok(ty),
                                _ => Err(error(
                                    input.span,
                                    "type-list collection requires Type values",
                                )),
                            })
                            .collect::<Result<Vec<_>, _>>()?,
                        _ => {
                            return Err(error(
                                input.span,
                                "type-list spread did not produce Type list data",
                            ));
                        }
                    };
                    self.required_steps = self
                        .required_steps
                        .checked_sub(items.len() as u64)
                        .ok_or_else(|| {
                            error(
                                input.span,
                                "test template type-list spread budget exhausted",
                            )
                        })?;
                    elements.extend(items);
                } else {
                    let (value, _) = self.required_typed_expression(input, Some(&p(P::Type)))?;
                    let Value::Type(ty) = value else {
                        return Err(error(
                            input.span,
                            "type-list collection requires Type values",
                        ));
                    };
                    elements.push(ty);
                }
            }
            return Ok((
                Value::List {
                    ty: domain.clone(),
                    values: elements.into_iter().map(Value::Type).collect(),
                },
                domain.clone(),
            ));
        }
        let Term::Tuple(expected) = self.structural_type(domain, span)? else {
            return Err(error(
                span,
                "test template collection requires tuple layout evidence",
            ));
        };
        let arguments = inputs
            .iter()
            .map(|(input, expand)| {
                if *expand {
                    hir::common::Arg::Expand(input)
                } else {
                    hir::common::Arg::Positional(input)
                }
            })
            .collect::<Vec<_>>();
        let expanded = super::super::normal::literal_spreads(&arguments);
        let inputs = expanded
            .as_deref()
            .unwrap_or(&arguments)
            .iter()
            .map(|argument| match argument {
                hir::common::Arg::Positional(input) => (*input, false),
                hir::common::Arg::Expand(input) => (*input, true),
                _ => unreachable!(),
            })
            .collect::<Vec<_>>();
        let mut values = Vec::new();
        let mut types = Vec::new();
        for (input, expand) in &inputs {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(input.span, "test template collection budget exhausted"))?;
            if *expand {
                // Evaluate a completed packet once, then distribute its values.
                // Literal spreads were expanded earlier to retain numeric context.
                let (packet, ty) = self.required_typed_expression(input, None)?;
                let Term::Tuple(packet_types) = self.structural_type(&ty, input.span)? else {
                    return Err(error(
                        input.span,
                        "test template spread requires a tuple packet",
                    ));
                };
                let Value::Tuple(fields) = packet else {
                    return Err(error(
                        input.span,
                        "test template spread did not produce tuple data",
                    ));
                };
                if fields.len() != packet_types.len() {
                    return Err(error(
                        input.span,
                        "test template spread value/type arity mismatch",
                    ));
                }
                for (value, ty) in fields.into_iter().zip(packet_types) {
                    self.required_steps = self.required_steps.checked_sub(1).ok_or_else(|| {
                        error(input.span, "test template spread budget exhausted")
                    })?;
                    let target = expected.get(values.len()).ok_or_else(|| {
                        error(
                            input.span,
                            "too many inputs for test template tuple collection",
                        )
                    })?;
                    self.require_expression(&ty, target, input.span)?;
                    types.push(ty);
                    values.push(value);
                }
            } else {
                let target = expected.get(values.len()).ok_or_else(|| {
                    error(
                        input.span,
                        "too many inputs for test template tuple collection",
                    )
                })?;
                let (value, ty) = self.required_typed_expression(input, Some(target))?;
                types.push(ty);
                values.push(value);
            }
        }
        if values.len() != expected.len() {
            return Err(error(
                span,
                "too few inputs for test template tuple collection",
            ));
        }
        // Construct the ordinary Tuple type and check it against the declared
        // domain. Layout evidence alone cannot assign an opaque type identity.
        let actual = self.tuple_value_type(types, span)?;
        self.require_expression(&actual, domain, span)?;
        Ok((Value::Tuple(values), actual))
    }

    fn test_template_contract(
        &mut self,
        signature: &hir::item::FnSig<'_>,
        abstract_inputs: bool,
    ) -> Result<nir::contracts::ProgramContract, Error> {
        // Publish only positive conjunctive trait atoms, using the same
        // readiness/retry protocol as ordinary signatures. Concrete instances
        // must select actual implementations instead of acquiring assumptions.
        if abstract_inputs {
            let mut pending = Vec::new();
            for clause in signature.clause_constraints {
                if let hir::clause::ClauseConstraintKind::Requires(value) = clause.kind {
                    super::super::required::signature_capability_atoms(value, &mut pending, 0)?;
                }
            }
            self.ready_signature_capabilities(self.signature_owner.unwrap(), &mut pending, true)?;
        }
        let mut contract = nir::contracts::ProgramContract::default();
        let mut postconditions = Vec::new();
        for clause in signature.clause_constraints {
            let (value, post) = match clause.kind {
                hir::clause::ClauseConstraintKind::Requires(value) => (value, false),
                hir::clause::ClauseConstraintKind::Ensures(value) => (value, true),
                _ => {
                    return Err(error(
                        clause.span,
                        "test template specification kind is not yet implemented",
                    ));
                }
            };
            if post {
                postconditions.push(value);
                continue;
            }
            if !self.program_requirement(value)? {
                let predicate = self.type_predicate(value)?;
                let formula = crate::ptc::Formula::from_predicate(&predicate);
                if abstract_inputs {
                    self.context.assume(formula);
                } else {
                    self.establish_type_requirement(&predicate, value.span)?;
                }
                continue;
            }
            let predicate = self.boolean_predicate(value)?;
            if !n::checkable_predicate(&predicate) {
                return Err(error(
                    value.span,
                    "test template contract requires a supported effect-free program predicate",
                ));
            }
            contract.preconditions.push(predicate);
        }
        let (predicates, snapshots) =
            self.program_postcondition_values(self.signature_owner.unwrap(), None, postconditions)?;
        contract.postconditions = predicates;
        contract.entry_snapshots = snapshots;
        Ok(contract)
    }
}
