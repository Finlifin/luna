//! Source declaration contracts. Runtime method bodies use the ordinary checker;
//! required declarations never enter its callable-body table.
use super::*;
use crate::ptc::{Budget, Formula, Outcome};
mod associated;
mod declarations;
mod parents;
mod signatures;

impl Checker<'_, '_> {
    fn method_contract(
        &mut self,
        owner: OwnerId,
        container: OwnerId,
        premise_traits: Option<&[n::Trait]>,
    ) -> Result<n::MethodContract, Error> {
        let item = self.package.item(owner).unwrap();
        let (signature, default) = match &item.kind {
            ItemKind::Fn(sig, _) => (
                sig,
                Some(declaration(self.package, &self.package_identities, owner)),
            ),
            ItemKind::FnDeclaration(sig) => (sig, None),
            _ => {
                return Err(error(
                    item.span,
                    "associated values require trait substitution not yet implemented",
                ));
            }
        };
        let saved_owner = self.signature_owner.replace(owner);
        let mut context = self.definition_context(owner);
        // The enclosing declaration is already admitted, but its checked trait
        // descriptor is not published while member contracts are constructed.
        // Make its type premises available now, without importing program facts
        // or turning them into requirements local to the method.
        for requirement in self.requirements.get(&container).into_iter().flatten() {
            context.assume(Formula::from_predicate(requirement));
        }
        if let Some(traits) = premise_traits {
            let identity = declaration(self.package, &self.package_identities, container);
            let interface = traits
                .iter()
                .find(|interface| interface.declaration == identity)
                .ok_or_else(|| error(item.span, "enclosing trait premises are unavailable"))?;
            let assumption = n::TraitRequirement {
                subject: Term::Rigid {
                    declaration: interface.self_parameter,
                    sort: Sort::Type,
                },
                interface: identity,
                interface_arguments: interface
                    .type_parameters
                    .iter()
                    .map(|parameter| Term::Rigid {
                        declaration: *parameter,
                        sort: if self
                            .trait_input_domain(*parameter)
                            .is_some_and(|ty| ty != p(P::Type))
                        {
                            Sort::ComptimeValue
                        } else {
                            Sort::Type
                        },
                    })
                    .collect(),
            };
            // Forward only subject guarantees from already-declared parent
            // capabilities. Parameter-only requirements still need independent
            // formation evidence; they cannot bootstrap their own admission.
            for requirement in
                crate::capabilities::subject_requirements(traits, &[assumption], Budget::default())
            {
                context.assume(Formula::from_predicate(&requirement));
            }
        }
        let saved_context = std::mem::replace(&mut self.context, context);
        let saved_locals = std::mem::take(&mut self.locals);
        let program = (|| {
            let ty = self
                .signatures
                .get(&owner)
                .or_else(|| self.interface_signatures.get(&owner))
                .unwrap()
                .clone();
            let Term::Function { parameters, result } = ty else {
                unreachable!()
            };
            for (parameter, ty) in signature.params.iter().zip(&parameters) {
                self.locals.insert(local(parameter.hir_id), ty.ty.clone());
            }
            let preconditions = self.program_preconditions(owner)?;
            let result_binding = signature.return_binding.map(local);
            let (postconditions, entry_snapshots) =
                self.program_postconditions(owner, result_binding, &result)?;
            Ok::<_, Error>(nir::contracts::ProgramContract {
                preconditions,
                postconditions,
                result_binding,
                entry_snapshots,
            })
        })();
        self.signature_owner = saved_owner;
        self.context = saved_context;
        self.locals = saved_locals;
        let program = program?;
        Ok(n::MethodContract {
            is_quote: signature.modifiers.is_quote,
            pure: signature.modifiers.is_pure,
            type_arguments: vec![],
            type_parameters: signature
                .clause_params
                .iter()
                .filter_map(|parameter| {
                    parameter.definition.map(|id| DeclarationId {
                        package: semantic_package(&self.package_identities, id.pkg),
                        local: id.index,
                    })
                })
                .collect(),
            capabilities: self.capabilities[&owner].clone(),
            program,
            parameter_bindings: signature.params.iter().map(|p| local(p.hir_id)).collect(),
            declaration: declaration(self.package, &self.package_identities, owner),
            by_reference_receiver: signature
                .params
                .first()
                .is_some_and(|p| matches!(p.ty.kind, TyParamKind::Itself { is_ref: true })),
            by_value_receiver: signature
                .params
                .first()
                .is_some_and(|p| matches!(p.ty.kind, TyParamKind::Itself { is_ref: false })),
            name: item.ident.name.to_string(),
            signature: self.callable_type(
                owner,
                self.signatures
                    .get(&owner)
                    .or_else(|| self.interface_signatures.get(&owner))
                    .unwrap()
                    .clone(),
            ),
            requirements: self.requirements[&owner].clone(),
            default,
        })
    }

    pub(super) fn check_interfaces(
        &mut self,
    ) -> Result<(Vec<n::Trait>, Vec<n::Implementation>), Error> {
        let mut traits = BTreeMap::new();
        let mut required_members = BTreeSet::new();
        for (owner, info) in self.package.owners() {
            let item = info.node.expect_item();
            let ItemKind::Trait(def) = &item.kind else {
                continue;
            };
            self.signature_owner = Some(owner);
            let mut names = BTreeSet::new();
            let mut associated = Vec::new();
            for member in &def.items {
                let member_item = self.package.item(*member).unwrap();
                if !names.insert(member_item.ident.name.to_string()) {
                    return Err(error(item.span, "duplicate trait member"));
                }
                required_members.insert(*member);
                if matches!(member_item.kind, ItemKind::Associated(_)) {
                    associated.push(self.associated_contract(*member, false)?);
                }
            }
            traits.insert(
                declaration(self.package, &self.package_identities, owner),
                n::Trait {
                    parameter_types: self
                        .trait_parameter_types
                        .get(&owner)
                        .cloned()
                        .unwrap_or_default(),
                    associated,
                    capabilities: self.capabilities[&owner].clone(),
                    requirements: self.requirements[&owner].clone(),
                    type_parameters: self.type_parameters[&owner].clone(),
                    declaration: declaration(self.package, &self.package_identities, owner),
                    parents: def
                        .super_traits
                        .map(|expr| self.parent_interfaces(expr))
                        .transpose()?
                        .unwrap_or_default()
                        .into_iter()
                        .map(Into::into)
                        .collect(),
                    self_parameter: declaration(self.package, &self.package_identities, owner),
                    name: item.ident.name.to_string(),
                    methods: Vec::new(),
                },
            );
        }
        self.validate_parent_graph(&traits)?;
        // Build the complete declaration premise graph before checking method
        // program contracts. These method-free headers are only an input to
        // bounded implication; never publish callable contracts with missing
        // preconditions, nor make ordering in the source determine guarantees.
        let premise_traits = traits.values().cloned().collect::<Vec<_>>();
        for (owner, info) in self.package.owners() {
            let ItemKind::Trait(def) = &info.node.expect_item().kind else {
                continue;
            };
            let mut methods = Vec::new();
            for member in &def.items {
                if !matches!(
                    self.package.item(*member).unwrap().kind,
                    ItemKind::Associated(_)
                ) {
                    methods.push(self.method_contract(*member, owner, Some(&premise_traits))?);
                }
            }
            traits
                .get_mut(&declaration(self.package, &self.package_identities, owner))
                .unwrap()
                .methods = methods;
        }
        for (owner, info) in self.package.owners() {
            let item = info.node.expect_item();
            if matches!(item.kind, ItemKind::Associated(_)) && !required_members.contains(&owner)
                && !self.self_containers.get(&owner).is_some_and(|parent| matches!(self.package.item(*parent).map(|i| &i.kind), Some(ItemKind::Impl(def)) if def.trait_ref.is_some()))
            {
                return Err(error(item.span,"associated values require an enclosing trait or trait implementation"));
            }
            if matches!(&item.kind, ItemKind::FnDeclaration(sig) if !sig.modifiers.is_quote)
                && !required_members.contains(&owner)
            {
                return Err(error(
                    item.span,
                    "bodyless function requires a trait contract",
                ));
            }
        }
        let mut implementations: Vec<n::Implementation> = Vec::new();
        let mut method_capability_checks = Vec::new();
        // Declaration guarantees are available when checking implementation
        // binders, but the implementation being checked is never an assumption.
        self.checked_traits = traits.values().cloned().collect();
        let mut index = crate::implementation::Index::default();
        let mut inherent_index = crate::implementation::InherentIndex::default();
        for (owner, info) in self.package.owners() {
            let item = info.node.expect_item();
            self.signature_owner = Some(owner);
            let (subject, instance, members) = match &item.kind {
                ItemKind::Impl(def) => {
                    let subject = self.type_expr(def.self_ty)?;
                    let instance = def
                        .trait_ref
                        .map(|expr| self.interface_expr_for(expr, &subject))
                        .transpose()?;
                    (subject, instance, def.items.clone())
                }
                ItemKind::Struct(_) | ItemKind::Enum(_) => {
                    let nested = match &item.kind {
                        ItemKind::Struct(def) => &def.nested_items,
                        ItemKind::Enum(def) => &def.nested_items,
                        _ => unreachable!(),
                    };
                    let members: Vec<_> = nested
                        .iter()
                        .copied()
                        .filter(|member| {
                            matches!(
                                self.package.item(*member).map(|item| &item.kind),
                                Some(ItemKind::Fn(..) | ItemKind::FnDeclaration(_))
                            )
                        })
                        .collect();
                    if members.is_empty() {
                        continue;
                    }
                    (self.nominal_self_type(owner), None, members)
                }
                _ => continue,
            };
            let parameters = self.type_parameters[&owner].clone();
            let requirements = self.requirements[&owner].clone();
            let interface_arguments = instance
                .as_ref()
                .map(|i| i.arguments.clone())
                .unwrap_or_default();
            let interface = instance.as_ref().map(|i| i.declaration);
            if let Some(interface) = interface {
                let contract = &traits[&interface];
                let arguments: Vec<_> = std::iter::once(subject.clone())
                    .chain(interface_arguments.iter().cloned())
                    .collect();
                let context = self.definition_context(owner);
                for requirement in &contract.requirements {
                    if !requirement.mentions_rigid(contract.self_parameter) {
                        continue;
                    }
                    let requirement = requirement
                        .instantiate_parameters(
                            &contract
                                .binders_with_self(&mut 100_000)
                                .map_err(|e| error(item.span, e))?,
                            &arguments,
                        )
                        .map_err(|e| error(item.span, e.to_string()))?;
                    if !matches!(
                        context.prove(Formula::from_predicate(&requirement), Budget::default()),
                        Outcome::Entailed(_)
                    ) {
                        return Err(error(
                            item.span,
                            "implementation does not establish trait subject requirement",
                        ));
                    }
                }
                if !matches!(
                    subject,
                    Term::Nominal { .. }
                        | Term::Primitive(_)
                        | Term::Tuple(_)
                        | Term::Record(_)
                        | Term::Pointer(_)
                        | Term::Function { .. }
                        | Term::ComptimeFunction(_)
                        | Term::Rigid {
                            sort: Sort::Type,
                            ..
                        }
                ) && !context.is_tuple_type(&subject)
                {
                    return Err(error(
                        item.span,
                        "implementation subject eligibility is not established",
                    ));
                }
                index
                    .insert(
                        crate::implementation::ImplementationHead {
                            declaration: declaration(self.package, &self.package_identities, owner),
                            interface: Some(interface),
                            subject: subject.clone(),
                            type_parameters: parameters.clone(),
                            interface_arguments: interface_arguments.clone(),
                            requirements: requirements.clone(),
                        },
                        Budget::default(),
                    )
                    .map_err(|e| {
                        error(item.span, format!("implementation admission failed: {e:?}"))
                    })?;
            } else {
                crate::implementation::inherent_ownership(
                    &subject,
                    declaration(self.package, &self.package_identities, owner).package,
                )
                .map_err(|e| {
                    error(
                        item.span,
                        format!("inherent implementation admission failed: {e:?}"),
                    )
                })?;
            }
            let mut supplied = BTreeMap::new();
            let mut supplied_associated = BTreeMap::new();
            let mut supplied_names = BTreeSet::new();
            for member in &members {
                let member_item = self.package.item(*member).unwrap();
                if !supplied_names.insert(member_item.ident.name.to_string()) {
                    return Err(error(item.span, "duplicate implementation member"));
                }
                if matches!(member_item.kind, ItemKind::Associated(_)) {
                    let associated = self.associated_contract(*member, false)?;
                    if !self.associated_has_initializer(associated.declaration) {
                        return Err(error(
                            item.span,
                            "implementation associated value requires an initializer",
                        ));
                    }
                    supplied_associated.insert(associated.name.clone(), associated);
                    continue;
                }
                let method = self.method_contract(*member, owner, None)?;
                if method.default.is_none() {
                    return Err(error(item.span, "implementation method requires a body"));
                }
                let name = method.name.clone();
                if supplied.insert(name, method).is_some() {
                    return Err(error(item.span, "duplicate implementation member"));
                }
            }
            let mut methods = Vec::new();
            if let Some(interface) = interface {
                for contract in &traits[&interface].methods {
                    if let Some(actual) = supplied.get(&contract.name) {
                        if actual.is_quote && !contract.is_quote {
                            return Err(error(
                                item.span,
                                "quote implementation of an ordinary trait contract requires callable adaptation, which is not yet implemented",
                            ));
                        }
                        if contract.is_quote && !actual.is_quote {
                            return Err(error(
                                item.span,
                                format!(
                                    "method `{}` requires a quote fn implementation; ordinary calls cannot satisfy deterministic expansion",
                                    contract.name
                                ),
                            ));
                        }
                        if actual.by_value_receiver != contract.by_value_receiver
                            || actual.by_reference_receiver != contract.by_reference_receiver
                        {
                            return Err(error(
                                item.span,
                                "method receiver does not match its trait contract",
                            ));
                        }
                        if contract.type_parameters.len() != actual.type_parameters.len() {
                            return Err(error(
                                item.span,
                                "method generic parameter count does not match its trait contract",
                            ));
                        }
                        let substitution_parameters: Vec<_> = traits[&interface]
                            .binders_with_self(&mut 100_000)
                            .map_err(|e| error(item.span, e))?
                            .into_iter()
                            .chain(contract.type_parameters.iter().map(|id| {
                                nir::DeclarationParameter {
                                    declaration: *id,
                                    sort: Sort::Type,
                                }
                            }))
                            .collect();
                        let substitution_arguments: Vec<_> = std::iter::once(subject.clone())
                            .chain(interface_arguments.iter().cloned())
                            .chain(actual.type_parameters.iter().map(|id| Term::Rigid {
                                declaration: *id,
                                sort: Sort::Type,
                            }))
                            .collect();
                        let expected_signature = contract
                            .signature
                            .instantiate_parameters(
                                &substitution_parameters,
                                &substitution_arguments,
                            )
                            .map_err(|e| error(item.span, e.to_string()))?;
                        // Compatibility assumes the interface contract, never the
                        // implementation method's own (possibly stronger) premises.
                        let base = self.definition_context(owner);
                        let scope = match actual.signature.callable_signature() {
                            Term::Function { parameters, .. } if !parameters.is_empty() => {
                                vec![parameters.iter().map(|parameter| parameter.sort).collect()]
                            }
                            _ => Vec::new(),
                        };
                        let mut context = base.theory_in_scope(scope);
                        for assumption in base.assumptions() {
                            context.assume(assumption.clone());
                        }
                        let scoped_parameters = substitution_parameters.clone();
                        let equations = self.candidate_associated_equations(
                            &Term::Tuple(vec![
                                expected_signature.clone(),
                                actual.signature.clone(),
                            ]),
                            &traits[&interface],
                            &subject,
                            &interface_arguments,
                            &supplied_associated,
                            item.span,
                        )?;
                        for (projection, value) in &equations {
                            context.assume(Formula::equal(projection.clone(), value.clone()));
                        }
                        for requirement in &contract.requirements {
                            let requirement = requirement
                                .instantiate_parameters_in_scope(
                                    &scoped_parameters,
                                    &substitution_arguments,
                                    context.scope(),
                                )
                                .map_err(|e| error(item.span, e.to_string()))?;
                            context.assume(Formula::from_predicate(&requirement));
                        }
                        let mut assumptions = self.capabilities[&owner].clone();
                        for capability in &contract.capabilities {
                            let capability = capability
                                .instantiate_parameters_in_scope(
                                    &scoped_parameters,
                                    &substitution_arguments,
                                    context.scope(),
                                )
                                .map_err(|e| error(item.span, e.to_string()))?;
                            context
                                .assume(Formula::Trait(crate::ptc::TraitQuery::from(&capability)));
                            assumptions.push(capability);
                        }
                        method_capability_checks.push((
                            actual.capabilities.clone(),
                            assumptions,
                            context.clone(),
                            item.span,
                            contract.name.clone(),
                        ));
                        // Open matching function binders in the contract's input
                        // frame. Comparing only the closed function terms would
                        // hide A == B premises from their parameter/result types.
                        let goal = match (&expected_signature, &actual.signature) {
                            (
                                Term::Function {
                                    parameters: expected,
                                    result: left,
                                },
                                Term::Function {
                                    parameters: supplied_parameters,
                                    result: right,
                                },
                            ) if expected.len() == supplied_parameters.len()
                                && expected
                                    .iter()
                                    .zip(supplied_parameters)
                                    .all(|(a, b)| a.sort == b.sort)
                                && expected_signature.check_scope(&[]).is_ok()
                                && actual.signature.check_scope(&[]).is_ok() =>
                            {
                                if signatures::equivalent(
                                    &context,
                                    expected
                                        .iter()
                                        .zip(supplied_parameters)
                                        .map(|(a, b)| (&a.ty, &b.ty))
                                        .chain(std::iter::once((left.as_ref(), right.as_ref()))),
                                    Budget::default(),
                                ) {
                                    Formula::True
                                } else {
                                    Formula::False
                                }
                            }
                            _ => Formula::equal(expected_signature, actual.signature.clone()),
                        };
                        if !matches!(context.prove(goal, Budget::default()), Outcome::Entailed(_)) {
                            return Err(error(
                                item.span,
                                format!(
                                    "method `{}` does not match its trait signature",
                                    contract.name
                                ),
                            ));
                        }
                        for requirement in &actual.requirements {
                            if !matches!(
                                context
                                    .prove(Formula::from_predicate(requirement), Budget::default()),
                                Outcome::Entailed(_)
                            ) {
                                return Err(error(
                                    item.span,
                                    format!(
                                        "method `{}` strengthens trait preconditions",
                                        contract.name
                                    ),
                                ));
                            }
                        }
                        // Retain only definition equations for this method's
                        // body, never the compatibility proof's assumptions.
                        self.associated_definition_equations
                            .insert(actual.declaration, equations);
                        methods.push((contract.name.clone(), actual.declaration));
                    } else if let Some(default) = contract.default {
                        methods.push((contract.name.clone(), default));
                    } else {
                        return Err(error(
                            item.span,
                            format!("missing required trait method `{}`", contract.name),
                        ));
                    }
                }
            } else {
                methods.extend(supplied.values().map(|m| (m.name.clone(), m.declaration)));
            }
            if interface.is_none() {
                inherent_index
                    .insert(
                        crate::implementation::InherentHead {
                            declaration: declaration(self.package, &self.package_identities, owner),
                            subject: subject.clone(),
                            type_parameters: parameters.clone(),
                            requirements: requirements.clone(),
                            members: methods.iter().map(|(name, _)| name.clone()).collect(),
                        },
                        Budget::default(),
                    )
                    .map_err(|e| {
                        error(
                            item.span,
                            format!("inherent implementation admission failed: {e:?}"),
                        )
                    })?;
            }
            let associated = Vec::new();
            if let Some(interface) = interface {
                let trait_definition = &traits[&interface];
                let arguments: Vec<_> = std::iter::once(subject.clone())
                    .chain(interface_arguments.iter().cloned())
                    .collect();
                let declared_associated = supplied_associated.clone();
                for contract in &trait_definition.associated {
                    if supplied.contains_key(&contract.name) {
                        return Err(error(
                            item.span,
                            "associated value cannot be supplied by a method",
                        ));
                    }
                    let expected = contract
                        .ty
                        .instantiate_parameters(
                            &trait_definition
                                .binders_with_self(&mut 100_000)
                                .map_err(|e| error(item.span, e))?,
                            &arguments,
                        )
                        .map_err(|e| error(item.span, e.to_string()))?;
                    let (initializer, parameters, substitution) = if let Some(actual) =
                        supplied_associated.remove(&contract.name)
                    {
                        let mut context = self.definition_context(owner);
                        let equations = self.candidate_associated_equations(
                            &Term::Tuple(vec![expected.clone(), actual.ty.clone()]),
                            trait_definition,
                            &subject,
                            &interface_arguments,
                            &declared_associated,
                            item.span,
                        )?;
                        for (projection, value) in equations {
                            context.assume(Formula::equal(projection, value));
                        }
                        if !matches!(
                            context.prove(
                                Formula::equal(expected.clone(), actual.ty),
                                Budget::default()
                            ),
                            Outcome::Entailed(_)
                        ) {
                            return Err(error(
                                item.span,
                                format!(
                                    "associated value `{}` does not match its declared type",
                                    contract.name
                                ),
                            ));
                        }
                        (actual.declaration, vec![], vec![])
                    } else {
                        if !self.associated_has_initializer(contract.declaration) {
                            return Err(error(
                                item.span,
                                format!("missing required associated value `{}`", contract.name),
                            ));
                        }
                        (
                            contract.declaration,
                            trait_definition.parameters_with_self(),
                            arguments.clone(),
                        )
                    };
                    let initializer_owner = self.associated_owner(initializer).unwrap();
                    self.associated_initializers.insert(
                        (
                            declaration(self.package, &self.package_identities, owner),
                            contract.declaration,
                        ),
                        (initializer_owner, parameters, substitution),
                    );
                }
            }
            if !supplied_associated.is_empty() {
                return Err(error(
                    item.span,
                    "unknown associated value in implementation",
                ));
            }
            implementations.push(n::Implementation {
                associated,
                interface_arguments: interface_arguments.clone(),
                capabilities: self.capabilities[&owner].clone(),
                type_parameters: parameters,
                requirements,
                declaration: declaration(self.package, &self.package_identities, owner),
                subject,
                interface,
                methods,
            });
        }
        let coherence_context = self.nominal_context();
        inherent_index
            .check_coherence(&coherence_context, Budget::default())
            .map_err(|message| error(Span::default(), message))?;
        let coherence = index.check_type_coherence(&coherence_context, Budget::default());
        if !index.verify_type_coherence(&coherence, &coherence_context, Budget::default()) {
            let pair = coherence
                .pairs
                .iter()
                .find(|(_, _, result)| !result.is_proved());
            let span = self
                .package
                .owners()
                .find_map(|(owner, info)| {
                    let item = info.node.expect_item();
                    (matches!(item.kind, ItemKind::Impl(_))
                        && pair.is_none_or(|(_, right, _)| {
                            declaration(self.package, &self.package_identities, owner) == *right
                        }))
                    .then_some(item.span)
                })
                .unwrap_or_default();
            return Err(error(
                span,
                format!(
                    "implementation overlap is not provably disjoint{}",
                    pair.map_or_else(
                        || " (coherence budget or proof replay incomplete)".into(),
                        |(left, right, _)| format!(" between {left} and {right}")
                    )
                ),
            ));
        }
        let trait_values: Vec<_> = traits.values().cloned().collect();
        for (requirements, assumptions, context, span, name) in method_capability_checks {
            let world = crate::capabilities::World {
                index: &index,
                implementations: &implementations,
                traits: &trait_values,
                assumptions: &assumptions,
                context: &context,
            };
            for requirement in requirements {
                let decision = world
                    .decide_trait(
                        &crate::ptc::TraitQuery::from(&requirement),
                        Budget::default(),
                    )
                    .map_err(|message| error(span, message))?;
                if decision.truth() != crate::capabilities::TraitTruth::Established {
                    return Err(error(
                        span,
                        format!("method `{name}` strengthens trait capability requirements"),
                    ));
                }
            }
        }

        for implementation in &implementations {
            if let Some(interface) = implementation.interface {
                let contract = &traits[&interface];
                let owner = self
                    .package
                    .owners()
                    .find_map(|(owner, _)| {
                        (declaration(self.package, &self.package_identities, owner)
                            == implementation.declaration)
                            .then_some(owner)
                    })
                    .unwrap();
                let span = self.package.item(owner).unwrap().span;
                let arguments: Vec<_> = std::iter::once(implementation.subject.clone())
                    .chain(implementation.interface_arguments.iter().cloned())
                    .collect();
                let requirements = contract
                    .parents
                    .iter()
                    .map(|parent| {
                        (
                            n::TraitRequirement {
                                subject: Term::Rigid {
                                    declaration: contract.self_parameter,
                                    sort: Sort::Type,
                                },
                                interface: parent.declaration,
                                interface_arguments: parent.arguments.clone(),
                            },
                            "parent interface",
                        )
                    })
                    .chain(
                        contract
                            .capabilities
                            .iter()
                            .filter(|requirement| {
                                requirement.mentions_rigid(contract.self_parameter)
                            })
                            .cloned()
                            .map(|requirement| (requirement, "trait contract capability")),
                    );
                let context = self.definition_context(owner);
                let world = crate::capabilities::World {
                    index: &index,
                    implementations: &implementations,
                    traits: &trait_values,
                    assumptions: &implementation.capabilities,
                    context: &context,
                };
                for (requirement, kind) in requirements {
                    let requirement = requirement
                        .instantiate_parameters(
                            &contract
                                .binders_with_self(&mut 100_000)
                                .map_err(|e| error(span, e))?,
                            &arguments,
                        )
                        .map_err(|e| error(span, e.to_string()))?;
                    if world.given(&requirement, Budget::default()).is_some() {
                        continue;
                    }
                    let resolution = world
                        .resolve_instance(
                            &requirement.interface_instance(),
                            &requirement.subject,
                            Budget::default(),
                        )
                        .map_err(|e| {
                            error(span, format!("{kind} implementation query failed: {e}"))
                        })?
                        .types;
                    if !matches!(
                        resolution.selection,
                        crate::implementation::Selection::Selected(_)
                    ) {
                        return Err(error(
                            span,
                            format!(
                                "required {kind} implementation is not established: {}",
                                resolution.selection
                            ),
                        ));
                    }
                }
            }
        }
        self.implementation_index = index;
        self.inherent_index = inherent_index;
        Ok((traits.into_values().collect(), implementations))
    }
}

impl Checker<'_, '_> {
    // Method routing recognizes Type inputs without changing their ordinary
    // expression representation, which carries value/provenance bindings.
    fn is_static_method_subject(&self, value: &Expr<'_>) -> bool {
        self.is_type_head(value)
            || matches!(&value.kind,H::ResolvedFnParam(_,id)
            if self.type_input_bindings.contains_key(&local(*id)))
    }

    pub(super) fn static_method(
        &mut self,
        value: &Expr<'_>,
        name: &str,
        span: Span,
    ) -> Result<Option<n::Expr>, Error> {
        if let H::ResolvedDef { def_id, .. } = value.kind {
            if self.package.definition_owner(def_id).is_some_and(|owner| {
                matches!(
                    self.package.item(owner).map(|item| &item.kind),
                    Some(ItemKind::Trait(_))
                )
            }) {
                return Err(error(
                    span,
                    "trait method requires a selected implementation",
                ));
            }
        }
        let (base, interface_expr) = match value.kind {
            H::Cast(base, interface) => (base, Some(interface)),
            _ => (value, None),
        };
        let subject = if self.is_static_method_subject(base) {
            self.type_expr(base)?
        } else if let H::Projection(view, field) = &base.kind {
            match self.associated_projection(view, field.name.as_str(), base.span)? {
                Some(n::Expr {
                    kind: K::TypeValue(ty),
                    ..
                }) => ty,
                _ => return Ok(None),
            }
        } else {
            return Ok(None);
        };
        let interface = interface_expr
            .map(|expr| self.interface_expr_for(expr, &subject))
            .transpose()?;
        let interface = match interface {
            Some(interface) => Some(interface),
            None => self.context_member_interface(&subject, name, span)?,
        };
        let subject = if interface
            .as_ref()
            .is_some_and(|interface| self.symbolic_interface_evidence(&subject, interface))
        {
            subject
        } else {
            self.type_identity_view(&subject, span)?
        };
        self.selected_method(subject, interface, name, span)
    }

    pub(super) fn selected_method(
        &mut self,
        subject: Term,
        interface: Option<n::InterfaceInstance>,
        name: &str,
        span: Span,
    ) -> Result<Option<n::Expr>, Error> {
        if self.in_required_comptime && !self.interfaces_ready {
            return Err(error(
                span,
                "required comptime evaluation is waiting for the trait implementation world",
            ));
        }
        if let Some(interface) = interface.as_ref() {
            if let Some(origin) = self.inherited_member_source(&subject, interface, name, span)? {
                if &origin != interface {
                    // The requested child view must itself be justified. A
                    // parent implementation alone cannot grant that view.
                    if !self.symbolic_interface_evidence(&subject, interface) {
                        let resolution = self
                            .resolve_trait_capability(interface, &subject, Budget::default())
                            .map_err(|e| {
                                error(span, format!("interface view query failed: {e:?}"))
                            })?;
                        if !matches!(
                            resolution.selection,
                            crate::implementation::Selection::Selected(_)
                        ) {
                            return Err(error(span, "requested interface view is not established"));
                        }
                    }
                    return self.selected_method(subject, Some(origin), name, span);
                }
            }
            if self.symbolic_interface_evidence(&subject, interface) {
                let contract = self
                    .checked_traits
                    .iter()
                    .find(|t| t.declaration == interface.declaration)
                    .unwrap()
                    .methods
                    .iter()
                    .find(|m| m.name == name)
                    .ok_or_else(|| error(span, "unknown trait method"))?;
                self.check_method_contract(contract, interface, &subject, span)?;
                return Ok(Some(n::Expr {
                    ty: contract
                        .signature
                        .instantiate_parameters_in_scope(
                            &self.interface_parameter_binders(interface, span)?,
                            &self.interface_substitution(interface, &subject).1,
                            &self.call_input_frames(),
                        )
                        .map_err(|e| error(span, e.to_string()))?,
                    source: source(span),
                    kind: K::TraitMethod {
                        method_arguments: vec![],
                        interface_arguments: interface.arguments.clone(),
                        interface: interface.declaration,
                        member: contract.declaration,
                        subject,
                    },
                }));
            }
        }
        let mut implementation_arguments = Vec::new();
        let selected = if let Some(interface) = interface.as_ref() {
            let resolution = self
                .resolve_trait_capability(interface, &subject, Budget::default())
                .map_err(|e| error(span, format!("method implementation query failed: {e:?}")))?;
            match resolution.selection {
                crate::implementation::Selection::Selected(id) => {
                    let Some(crate::implementation::TypeAssessment::Applicable(evidence)) =
                        resolution.assessments.get(&id)
                    else {
                        return Err(error(span, "selected implementation has no type evidence"));
                    };
                    implementation_arguments = evidence.type_arguments.clone();
                    Some(id)
                }
                other => {
                    return Err(error(
                        span,
                        format!("method implementation is not uniquely established: {other}"),
                    ));
                }
            }
        } else {
            let resolution = self
                .resolve_inherent_capability(name, &subject, Budget::default())
                .map_err(|e| error(span, format!("inherent method query failed: {e:?}")))?;
            match resolution.types.selection {
                crate::implementation::Selection::Selected(id) => {
                    let Some(crate::implementation::TypeAssessment::Applicable(evidence)) =
                        resolution.types.assessments.get(&id)
                    else {
                        return Err(error(
                            span,
                            "selected inherent implementation has no type evidence",
                        ));
                    };
                    let assumptions = self.capability_assumptions();
                    crate::capabilities::World {
                        index: &self.implementation_index,
                        implementations: &self.checked_implementations,
                        traits: &self.checked_traits,
                        assumptions: &assumptions,
                        context: &self.context,
                    }
                    .verify_inherent_evidence(
                        &self.inherent_index,
                        name,
                        &subject,
                        &resolution,
                        Budget::default(),
                    )
                    .map_err(|e| {
                        error(
                            span,
                            format!("inherent method evidence replay failed: {e:?}"),
                        )
                    })?;
                    implementation_arguments = evidence.type_arguments.clone();
                    Some(id)
                }
                crate::implementation::Selection::NoApplicableCandidate => None,
                other => {
                    return Err(error(
                        span,
                        format!("inherent method applicability is unresolved: {other}"),
                    ));
                }
            }
        };
        let Some(selected) = selected else {
            return Ok(None);
        };
        let implementation = self
            .checked_implementations
            .iter()
            .find(|i| i.declaration == selected)
            .unwrap();
        let body = implementation
            .methods
            .iter()
            .find(|(method, _)| method == name)
            .map(|(_, body)| *body)
            .ok_or_else(|| error(span, "unknown trait method"))?;
        if let Some(interface) = interface.as_ref() {
            let contract = self
                .checked_traits
                .iter()
                .find(|t| t.declaration == interface.declaration)
                .unwrap()
                .methods
                .iter()
                .find(|m| m.name == name)
                .unwrap();
            self.check_method_contract(contract, interface, &subject, span)?;
        }
        if let Some(interface) = interface.as_ref() {
            let contract = self
                .checked_traits
                .iter()
                .find(|t| t.declaration == interface.declaration)
                .unwrap()
                .methods
                .iter()
                .find(|m| m.name == name)
                .unwrap();
            let (_, arguments) = self.interface_substitution(interface, &subject);
            let parameters = self.interface_parameter_binders(interface, span)?;
            return Ok(Some(n::Expr {
                ty: contract
                    .signature
                    .instantiate_parameters_in_scope(
                        &parameters,
                        &arguments,
                        &self.call_input_frames(),
                    )
                    .map_err(|e| error(span, e.to_string()))?,
                source: source(span),
                kind: K::TraitMethod {
                    method_arguments: vec![],
                    interface: interface.declaration,
                    interface_arguments: interface.arguments.clone(),
                    member: contract.declaration,
                    subject,
                },
            }));
        }
        let owner = self
            .package
            .owners()
            .find_map(|(owner, _)| {
                (declaration(self.package, &self.package_identities, owner) == body)
                    .then_some(owner)
            })
            .unwrap();
        let arguments = if self.type_parameters[&owner].is_empty() {
            Vec::new()
        } else if self.self_containers.contains_key(&owner) {
            implementation_arguments
        } else {
            std::iter::once(subject)
                .chain(interface.as_ref().unwrap().arguments.iter().cloned())
                .collect()
        };
        let parameters = &self.type_parameters[&owner];
        if arguments.len() > parameters.len() {
            return Err(error(
                span,
                "method implementation type argument count mismatch",
            ));
        }
        let incomplete = arguments.len() < parameters.len();
        if !incomplete {
            self.check_requirements(owner, &arguments, span)?;
        }
        // Resolution fixes implementation arguments. Method-local parameters
        // remain rigid until invocation inference completes them; this value
        // must not escape as a published generic function instance.
        let ty = self.signatures[&owner]
            .instantiate_types_in_scope(
                &parameters[..arguments.len()],
                &arguments,
                &self.call_input_frames(),
            )
            .map_err(|e| error(span, e.to_string()))?;
        Ok(Some(n::Expr {
            ty: self.callable_type(owner, ty),
            source: source(span),
            kind: if !incomplete && arguments.is_empty() && self.requirements[&owner].is_empty() {
                K::Function(body)
            } else {
                K::FunctionInstance {
                    declaration: body,
                    arguments,
                }
            },
        }))
    }
}

impl Checker<'_, '_> {
    pub(super) fn require_non_trait_body_reference(
        &self,
        owner: OwnerId,
        span: Span,
    ) -> Result<(), Error> {
        if matches!(
            self.package.item(owner).map(|item| &item.kind),
            Some(ItemKind::Effect(..))
        ) {
            return Err(error(
                span,
                "effect operation requires effect request elaboration; it is not an ordinary function",
            ));
        }
        if self.checked_traits.iter().any(|interface| {
            interface.methods.iter().any(|method| {
                method.declaration == declaration(self.package, &self.package_identities, owner)
            })
        }) {
            return Err(error(
                span,
                "trait method requires a selected implementation",
            ));
        }
        Ok(())
    }

    pub(super) fn check_ordinary_capabilities(
        &self,
        owner: OwnerId,
        type_arguments: &[Term],
        inputs: &[Option<Term>],
        span: Span,
    ) -> Result<(), Error> {
        let Some(requirements) = self.capabilities.get(&owner) else {
            return Ok(());
        };
        let requirements = requirements
            .iter()
            .filter(|requirement| {
                std::iter::once(&requirement.subject)
                    .chain(&requirement.interface_arguments)
                    .any(|ty| {
                        ty.substitution_parameters()
                            .is_ok_and(|slots| !slots.is_empty())
                    })
            })
            .collect::<Vec<_>>();
        if requirements.is_empty() {
            return Ok(());
        }
        let signature = self
            .signatures
            .get(&owner)
            .or_else(|| self.interface_signatures.get(&owner))
            .ok_or_else(|| error(span, "ordinary capability requires function signature"))?;
        let Term::Function {
            parameters: ordinary,
            ..
        } = signature.callable_signature()
        else {
            return Ok(());
        };
        let caller = self.call_inference_scope();
        let mut frames = if caller.is_empty() {
            vec![]
        } else {
            vec![caller.iter().map(|p| p.sort).collect()]
        };
        let specialized = signature
            .instantiate_parameters_in_scope(
                &self.declaration_binders(owner),
                type_arguments,
                &frames,
            )
            .map_err(|e| error(span, e.to_string()))?;
        let Term::Function {
            parameters: specialized_parameters,
            ..
        } = specialized.callable_signature()
        else {
            unreachable!()
        };
        let binding = self.prepare_call_binding(specialized_parameters, inputs, span)?;
        frames.push(ordinary.iter().map(|p| p.sort).collect());
        let parameters = self.declaration_binders(owner);
        let arguments = type_arguments
            .iter()
            .map(|ty| ty.abstract_parameters(&[]))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| error(span, e.to_string()))?;
        for requirement in requirements {
            let requirement = requirement
                .instantiate_parameters_in_scope(&parameters, &arguments, &frames)
                .map_err(|e| error(span, e.to_string()))?;
            let requirement = n::TraitRequirement {
                interface: requirement.interface,
                subject: self.callable_input_view(
                    &binding
                        .substitute(&requirement.subject)
                        .map_err(|e| error(span, e.to_string()))?,
                    span,
                )?,
                interface_arguments: requirement
                    .interface_arguments
                    .iter()
                    .map(|ty| {
                        let ty = binding
                            .substitute(ty)
                            .map_err(|e| error(span, e.to_string()))?;
                        self.callable_input_view(&ty, span)
                    })
                    .collect::<Result<_, _>>()?,
            };
            let interface = requirement.interface_instance();
            if self.symbolic_interface_evidence(&requirement.subject, &interface) {
                continue;
            }
            // This is an obligation, not method selection. The proposition
            // solver retains scoped equality evidence while obtaining concrete
            // implementation truth; raw head matching loses ordinary binders.
            let assumptions = self.capability_assumptions();
            let outcome = crate::capabilities::World {
                index: &self.implementation_index,
                implementations: &self.checked_implementations,
                traits: &self.checked_traits,
                assumptions: &assumptions,
                context: &self.context,
            }
            .check_predicate(&n::TypePredicate::Trait(requirement), Budget::default())
            .map_err(|e| error(span, format!("ordinary input trait query failed: {e}")))?;
            if !matches!(outcome, crate::ptc::Outcome::Entailed(_)) {
                return Err(error(
                    span,
                    format!("ordinary input trait bound is not established: {outcome}"),
                ));
            }
        }
        Ok(())
    }
}

impl Checker<'_, '_> {
    fn nominal_self_type(&self, owner: OwnerId) -> Term {
        Term::Nominal {
            declaration: declaration(self.package, &self.package_identities, owner),
            arguments: self.type_parameters[&owner]
                .iter()
                .map(|parameter| Term::Rigid {
                    declaration: *parameter,
                    sort: Sort::Type,
                })
                .collect(),
        }
    }

    pub(super) fn prepare_self_types(&mut self) -> Result<(), Error> {
        for (owner, info) in self.package.owners() {
            let item = info.node.expect_item();
            match &item.kind {
                ItemKind::Trait(def) => {
                    let ty = Term::Rigid {
                        declaration: declaration(self.package, &self.package_identities, owner),
                        sort: Sort::Type,
                    };
                    self.self_types.insert(owner, ty.clone());
                    for member in &def.items {
                        self.self_types.insert(*member, ty.clone());
                    }
                }
                ItemKind::Struct(_) | ItemKind::Enum(_) => {
                    let members = match &item.kind {
                        ItemKind::Struct(def) => &def.nested_items,
                        ItemKind::Enum(def) => &def.nested_items,
                        _ => unreachable!(),
                    };
                    let ty = self.nominal_self_type(owner);
                    self.self_types.insert(owner, ty.clone());
                    for member in members {
                        if matches!(
                            self.package.item(*member).map(|item| &item.kind),
                            Some(ItemKind::Fn(..) | ItemKind::FnDeclaration(_))
                        ) {
                            self.self_containers.insert(*member, owner);
                            self.self_types.insert(*member, ty.clone());
                        }
                    }
                }
                ItemKind::Impl(def) => {
                    // Delay head interpretation until a member actually uses
                    // Self; unrelated signatures need not already be collected.
                    for member in &def.items {
                        self.self_containers.insert(*member, owner);
                    }
                }
                _ => {}
            }
        }
        self.signature_owner = None;
        Ok(())
    }
}

impl Checker<'_, '_> {
    pub(super) fn self_type(&mut self, owner: OwnerId, span: Span) -> Result<Term, Error> {
        if let Some(ty) = self.self_types.get(&owner) {
            return Ok(ty.clone());
        }
        let container = self.self_containers.get(&owner).copied().ok_or_else(|| {
            error(
                span,
                "Self requires an enclosing nominal type, trait or implementation",
            )
        })?;
        let ItemKind::Impl(def) = &self.package.item(container).unwrap().kind else {
            unreachable!()
        };
        let ty = self.type_expr(def.self_ty)?;
        self.self_types.insert(owner, ty.clone());
        Ok(ty)
    }
}

impl Checker<'_, '_> {
    pub(super) fn receiver_call(
        &mut self,
        expr: &Expr<'_>,
        callee: &Expr<'_>,
        args: &[Arg<'_>],
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<Option<n::Expr>, Error> {
        let Some((method, prefix)) = self.resolve_receiver(expr, callee)? else {
            return Ok(None);
        };
        self.finish_call(expr, method, args, prefix, expected, partial)
            .map(Some)
    }

    /// Resolve and elaborate a receiver independently of argument preparation.
    /// Callers reuse this value rather than elaborating/evaluating it again
    /// after a spread obtains its element contexts from the method signature.
    pub(super) fn resolve_receiver(
        &mut self,
        expr: &Expr<'_>,
        callee: &Expr<'_>,
    ) -> Result<Option<(n::Expr, Vec<n::Expr>)>, Error> {
        if let H::NFApplication(head, arguments) = callee.kind {
            if let Some((method, prefix)) = self.resolve_receiver(expr, head)? {
                let method =
                    self.explicit_method_instance(callee, method, arguments, None, true)?;
                return Ok(Some((method, prefix)));
            }
            return Ok(None);
        }
        let H::Projection(view, name) = callee.kind else {
            return Ok(None);
        };
        // A projected associated Type is a static subject, even though the
        // syntax-only type-head predicate cannot classify its member type.
        // Resolve it before treating the Type value as a runtime receiver.
        let projected_subject = match view.kind {
            H::Cast(base, _) => matches!(base.kind, H::Projection(..)),
            H::Projection(..) => true,
            _ => false,
        };
        if projected_subject {
            if let Some(method) = self.static_method(view, name.name.as_str(), callee.span)? {
                return Ok(Some((method, vec![])));
            }
        }
        if matches!(view.kind, H::Cast(base, _) if self.is_static_method_subject(base)) {
            return Ok(None);
        }
        let (base, interface) = match view.kind {
            H::Cast(base, interface) if !self.is_static_method_subject(base) => {
                (base, Some(interface))
            }
            _ if !self.is_static_method_subject(view)
                && (self.default_interface().is_some()
                    || self
                        .signature_owner
                        .and_then(|owner| self.capabilities.get(&owner))
                        .is_some_and(|requirements| !requirements.is_empty())
                    || self.checked_implementations.iter().any(|i| {
                        i.interface.is_none()
                            && i.methods
                                .iter()
                                .any(|(method, _)| method == name.name.as_str())
                    })) =>
            {
                (view, None)
            }
            _ => return Ok(None),
        };
        // Keep a dereferenced receiver as a provisional place. Selection may
        // cancel this read when the chosen method needs its original address.
        let mut receiver = if let Some(pointer) = self.dereferenced_place(base)? {
            let Term::Pointer(ty) = &pointer.ty else {
                unreachable!()
            };
            n::Expr {
                ty: (**ty).clone(),
                source: source(base.span),
                kind: K::ReadPointer(Box::new(pointer)),
            }
        } else {
            self.expr(base, None)?
        };
        let interface_source = interface;
        let interface = interface
            .map(|expr| self.interface_expr_for(expr, &receiver.ty))
            .transpose()?;
        let mut subject = if interface
            .as_ref()
            .is_some_and(|interface| self.symbolic_interface_evidence(&receiver.ty, interface))
        {
            receiver.ty.clone()
        } else {
            self.type_identity_view(&receiver.ty, base.span)?
        };
        let mut depth = 0;
        let method = loop {
            let candidate_interface = match &interface {
                Some(interface) if subject == receiver.ty => Some(interface.clone()),
                Some(_) => Some(self.interface_expr_for(interface_source.unwrap(), &subject)?),
                None => self.context_member_interface(&subject, name.name.as_str(), expr.span)?,
            };
            // Structural callable fields keep their source priority.
            if candidate_interface.is_none() {
                self.ensure_nominal_type(&subject, base.span)?;
                if let Term::Nominal { declaration, .. } = &subject {
                    if self.structs.get(declaration).is_some_and(|s| {
                        s.fields
                            .iter()
                            .any(|(field, _)| field == name.name.as_str())
                    }) {
                        return Ok(None);
                    }
                }
            }
            let absent_at_pointer = if let Some(interface) = &candidate_interface {
                matches!(subject, Term::Pointer(_))
                    && !self.symbolic_interface_evidence(&subject, interface)
                    && matches!(
                        self.resolve_trait_capability(interface, &subject, Budget::default())
                            .map_err(|message| error(expr.span, message))?
                            .selection,
                        crate::implementation::Selection::NoApplicableCandidate
                    )
            } else {
                false
            };
            if !absent_at_pointer {
                if let Some(method) = self.selected_method(
                    subject.clone(),
                    candidate_interface,
                    name.name.as_str(),
                    expr.span,
                )? {
                    break method;
                }
            }
            let Term::Pointer(inner) = subject else {
                return Ok(None);
            };
            if depth == 128 {
                return Err(error(expr.span, "receiver dereference depth exhausted"));
            }
            depth += 1;
            subject = self.type_identity_view(&inner, base.span)?;
        };
        let has_receiver = match &method.kind {
            K::TraitMethod {
                interface, member, ..
            } => {
                let contract = self
                    .checked_traits
                    .iter()
                    .find(|t| t.declaration == *interface)
                    .unwrap()
                    .methods
                    .iter()
                    .find(|m| m.declaration == *member)
                    .unwrap();
                contract.by_value_receiver || contract.by_reference_receiver
            }
            K::Function(body)
            | K::FunctionInstance {
                declaration: body, ..
            } => {
                let owner = self
                    .package
                    .owners()
                    .find_map(|(owner, _)| {
                        (declaration(self.package, &self.package_identities, owner) == *body)
                            .then_some(owner)
                    })
                    .unwrap();
                let ItemKind::Fn(sig, _) = &self.package.item(owner).unwrap().kind else {
                    unreachable!()
                };
                sig.params
                    .first()
                    .is_some_and(|p| matches!(p.ty.kind, TyParamKind::Itself { .. }))
            }
            _ => unreachable!(),
        };
        if !has_receiver {
            return Err(error(
                expr.span,
                "value method call requires a receiver parameter",
            ));
        }
        let Term::Function { parameters, .. } = method.ty.callable_signature() else {
            unreachable!()
        };
        // The first receiver has no preceding callee inputs. Remove that
        // telescope frame before comparing with the caller's receiver type.
        let target = parameters[0]
            .ty
            .substitute_partial(&[])
            .map_err(|e| error(base.span, e.to_string()))?;
        let target = &target;
        for _ in 0..=128 {
            if &receiver.ty == target {
                break;
            }
            if matches!(target, Term::Pointer(inner) if **inner == receiver.ty) {
                if let K::ReadPointer(pointer) = receiver.kind {
                    receiver = *pointer;
                    break;
                }
                let address = n::Expr {
                    ty: target.clone(),
                    source: source(base.span),
                    kind: K::AddressOf(Box::new(receiver)),
                };
                if let Some(message) = n::pointer_operation_error(&address) {
                    return Err(error(base.span, message));
                }
                receiver = address;
                break;
            }
            let Term::Pointer(inner) = &receiver.ty else {
                break;
            };
            if !n::intrinsic_pointer_value(inner) {
                return Err(error(
                    base.span,
                    "by-value pointer receiver requires memory transfer contracts",
                ));
            }
            receiver = n::Expr {
                ty: (**inner).clone(),
                source: source(base.span),
                kind: K::ReadPointer(Box::new(receiver)),
            };
        }
        if matches!(receiver.kind, K::ReadPointer(_)) && !n::intrinsic_pointer_value(&receiver.ty) {
            return Err(error(
                base.span,
                "by-value pointer receiver requires memory transfer contracts",
            ));
        }
        self.require(&receiver.ty, target, base.span)?;
        Ok(Some((method, vec![receiver])))
    }
}

impl Checker<'_, '_> {
    fn check_method_contract(
        &self,
        contract: &n::MethodContract,
        interface: &n::InterfaceInstance,
        subject: &Term,
        span: Span,
    ) -> Result<(), Error> {
        if !contract.type_parameters.is_empty() {
            return Ok(());
        }
        let owner = self
            .package
            .owners()
            .find_map(|(owner, _)| {
                (declaration(self.package, &self.package_identities, owner) == contract.declaration)
                    .then_some(owner)
            })
            .ok_or_else(|| error(span, "missing trait method declaration"))?;
        let arguments = self.interface_substitution(interface, subject).1;
        self.check_capabilities(owner, &arguments, span)?;
        self.check_formation_requirements(owner, &FormationInputs::ready(&arguments), span)
            .map_err(|_| error(span, "trait method precondition is not established"))
    }

    pub(super) fn check_capabilities(
        &self,
        owner: OwnerId,
        arguments: &[Term],
        span: Span,
    ) -> Result<(), Error> {
        if let Some(requirements) = self.capabilities.get(&owner) {
            for requirement in requirements {
                if std::iter::once(&requirement.subject)
                    .chain(&requirement.interface_arguments)
                    .any(|ty| {
                        ty.substitution_parameters()
                            .is_ok_and(|slots| !slots.is_empty())
                    })
                {
                    // Ordinary inputs are supplied by calls, not by clause
                    // instantiation. Their obligations are checked below after
                    // input preparation and retained for staged specialization.
                    let signature = self
                        .signatures
                        .get(&owner)
                        .or_else(|| self.interface_signatures.get(&owner))
                        .ok_or_else(|| {
                            error(span, "ordinary capability requires function signature")
                        })?;
                    let Term::Function { parameters, .. } = signature.callable_signature() else {
                        return Err(error(
                            span,
                            "ordinary capability requires function signature",
                        ));
                    };
                    requirement
                        .instantiate_parameters_in_scope(
                            &[],
                            &[],
                            &[parameters.iter().map(|p| p.sort).collect()],
                        )
                        .map_err(|e| error(span, e.to_string()))?;
                    continue;
                }
                if matches!(
                    self.package.item(owner).map(|item| &item.kind),
                    Some(ItemKind::Trait(_))
                ) && requirement.mentions_rigid(declaration(
                    self.package,
                    &self.package_identities,
                    owner,
                )) {
                    continue;
                }
                let scope = self.call_inference_scope();
                let frames = if scope.is_empty() {
                    vec![]
                } else {
                    vec![scope.iter().map(|parameter| parameter.sort).collect()]
                };
                let parameters = self.declaration_binders(owner);
                let requirement = requirement
                    .instantiate_parameters_in_scope(&parameters, arguments, &frames)
                    .map_err(|e| error(span, e.to_string()))?;
                let instance = requirement.interface_instance();
                if self.symbolic_interface_evidence(&requirement.subject, &instance) {
                    continue;
                }
                let mut requirement = requirement;
                requirement.subject = self.callable_input_view(&requirement.subject, span)?;
                for argument in &mut requirement.interface_arguments {
                    *argument = self.callable_input_view(argument, span)?;
                }
                let instance = requirement.interface_instance();
                if self.symbolic_interface_evidence(&requirement.subject, &instance) {
                    continue;
                }
                let resolution = self
                    .resolve_trait_capability(&instance, &requirement.subject, Budget::default())
                    .map_err(|e| error(span, format!("trait bound query failed: {e}")))?;
                if !matches!(
                    resolution.selection,
                    crate::implementation::Selection::Selected(_)
                ) {
                    return Err(error(
                        span,
                        format!("trait bound is not established: {}", resolution.selection),
                    ));
                }
            }
        }
        Ok(())
    }
}

impl Checker<'_, '_> {
    pub(super) fn capability_assumptions(&self) -> Vec<n::TraitRequirement> {
        let mut assumptions = self
            .signature_owner
            .and_then(|owner| self.capabilities.get(&owner))
            .cloned()
            .unwrap_or_default();
        // Positive conjunctive type facts are local trait evidence. Do not
        // extract a branch of a disjunction or turn negative facts positive.
        let mut pending = self.context.assumptions().iter().collect::<Vec<_>>();
        let mut fuel = Budget::default().steps;
        while let Some(fact) = pending.pop() {
            if fuel == 0 {
                break;
            }
            fuel -= 1;
            match fact {
                crate::ptc::Formula::Trait(query) => {
                    let requirement = n::TraitRequirement {
                        subject: query.subject.clone(),
                        interface: query.interface,
                        interface_arguments: query.arguments.clone(),
                    };
                    if !assumptions.contains(&requirement) {
                        assumptions.push(requirement);
                    }
                }
                crate::ptc::Formula::And(parts) => pending.extend(parts),
                _ => {}
            }
        }
        if let Some(implementation) = self.initializer_implementation() {
            assumptions.extend(implementation.capabilities.iter().cloned());
        }
        if let Some(interface) = self.default_interface() {
            assumptions.push(n::TraitRequirement {
                interface_arguments: interface.arguments,
                interface: interface.declaration,
                subject: Term::Rigid {
                    declaration: interface.declaration,
                    sort: Sort::Type,
                },
            });
        }
        assumptions
    }
    pub(super) fn resolve_trait_capability(
        &self,
        interface: &n::InterfaceInstance,
        subject: &Term,
        budget: Budget,
    ) -> Result<crate::implementation::TypeResolution, String> {
        let assumptions = self.capability_assumptions();
        crate::capabilities::World {
            index: &self.implementation_index,
            implementations: &self.checked_implementations,
            traits: &self.checked_traits,
            assumptions: &assumptions,
            context: &self.context,
        }
        .resolve_instance(interface, subject, budget)
        .map(|r| r.types)
    }
    fn resolve_inherent_capability(
        &self,
        name: &str,
        subject: &Term,
        budget: Budget,
    ) -> Result<crate::capabilities::Resolution, String> {
        let assumptions = self.capability_assumptions();
        crate::capabilities::World {
            index: &self.implementation_index,
            implementations: &self.checked_implementations,
            traits: &self.checked_traits,
            assumptions: &assumptions,
            context: &self.context,
        }
        .resolve_inherent(&self.inherent_index, name, subject, budget)
    }
}

impl Checker<'_, '_> {
    pub(super) fn method_instantiation_inputs(
        &self,
        value: &n::Expr,
    ) -> Option<(OwnerId, Vec<Term>)> {
        let (id, arguments) = match &value.kind {
            K::FunctionInstance {
                declaration,
                arguments,
            } => (*declaration, arguments.clone()),
            K::TraitMethod {
                member,
                subject,
                interface_arguments,
                method_arguments,
                ..
            } => (
                *member,
                std::iter::once(subject.clone())
                    .chain(interface_arguments.iter().cloned())
                    .chain(method_arguments.iter().cloned())
                    .collect(),
            ),
            _ => return None,
        };
        self.package
            .owners()
            .find_map(|(owner, _)| {
                (declaration(self.package, &self.package_identities, owner) == id).then_some(owner)
            })
            .map(|owner| (owner, arguments))
    }
    pub(super) fn incomplete_method_instance(&self, value: &n::Expr) -> bool {
        self.method_instantiation_inputs(value)
            .is_some_and(|(owner, arguments)| {
                self.type_parameters
                    .get(&owner)
                    .is_some_and(|parameters| arguments.len() < parameters.len())
            })
    }
    fn retain_method_origin(&self, method: n::Expr, mut value: n::Expr) -> n::Expr {
        if let K::TraitMethod {
            interface,
            interface_arguments,
            member,
            subject,
            ..
        } = method.kind
        {
            let K::FunctionInstance { arguments, .. } = &value.kind else {
                unreachable!()
            };
            let method_arguments = arguments[1 + interface_arguments.len()..].to_vec();
            value.kind = K::TraitMethod {
                interface,
                interface_arguments,
                member,
                subject,
                method_arguments,
            };
        }
        value
    }
    pub(super) fn contextual_method_instance(
        &mut self,
        expr: &Expr<'_>,
        method: n::Expr,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        let (owner, fixed) = self
            .method_instantiation_inputs(&method)
            .ok_or_else(|| error(expr.span, "method has no generic declaration"))?;
        let value = self.contextual_function_instance_with_fixed_view(
            expr,
            owner,
            fixed,
            expected,
            partial,
            Some(&method),
        )?;
        self.callable_value_view(
            self.retain_method_origin(method, value),
            expected,
            expr.span,
        )
    }
    pub(super) fn invocation_callee(&mut self, callee: &Expr<'_>) -> Result<n::Expr, Error> {
        if self.protocols.splice.is_some()
            && self.runtime_value_context()
            && matches!(
                callee.kind,
                H::Application(..) | H::ExtendedApplication(..) | H::Block(_)
            )
        {
            let saved = self.comptime_callee_result.replace(callee.hir_id);
            let value = self.expr(callee, None);
            self.comptime_callee_result = saved;
            return value;
        }
        // Calling a stored comptime function does not transfer that function
        // value to runtime. Only the invocation's eventual result is spliced.
        if self.protocols.splice.is_some() && self.runtime_value_context() {
            if let H::ResolvedDef { def_id, .. } = callee.kind {
                if let Some(owner) = self.package.definition_owner(def_id) {
                    if matches!(
                        self.package.item(owner).map(|item| &item.kind),
                        Some(ItemKind::Const(..))
                    ) {
                        let value = self.constant_value(owner, callee.span)?;
                        if value.ty.is_comptime_callable() {
                            return Ok(value);
                        }
                    }
                }
            }
        }
        if let Some(owner) = self.associated_reference_owner(callee) {
            let value = self.lexical_associated_value(owner, callee.span)?;
            return Ok(n::Expr {
                ty: value.ty.clone(),
                source: value.source,
                kind: K::Block {
                    statements: vec![],
                    result: Some(Box::new(value)),
                },
            });
        }
        if let H::Projection(base, name) = &callee.kind {
            // Associated values may themselves be callable. Resolve the shared
            // member namespace before asking for a method declaration.
            if let Some(value) =
                self.associated_projection(base, name.name.as_str(), callee.span)?
            {
                // This is an ordinary stored callable, not a declaration-side
                // parameter entry. Preserve its identity inside a value scope
                // without recovering names/defaults/collectors from that ID.
                return Ok(n::Expr {
                    ty: value.ty.clone(),
                    source: value.source,
                    kind: K::Block {
                        statements: vec![],
                        result: Some(Box::new(value)),
                    },
                });
            }
            if let Some(method) = self.static_method(base, name.name.as_str(), callee.span)? {
                return Ok(method);
            }
        }
        self.expr(callee, None)
    }
}

impl Checker<'_, '_> {
    pub(super) fn explicit_method_instance(
        &mut self,
        expr: &Expr<'_>,
        method: n::Expr,
        args: &[Arg<'_>],
        expected: Option<&Term>,
        allow_optional: bool,
    ) -> Result<n::Expr, Error> {
        let (owner, mut fixed) = self.method_instantiation_inputs(&method).ok_or_else(|| {
            error(
                expr.span,
                "explicit instantiation requires a generic method",
            )
        })?;
        if self.has_optional_type_clauses(owner) {
            let full =
                self.type_clause_arguments_with_prefix(owner, args, None, expr.span, fixed)?;
            let value = self.function_instance(expr, owner, full, None, allow_optional)?;
            let value = self.retain_method_origin(method, value);
            return if allow_optional {
                Ok(value)
            } else {
                self.callable_value_view(value, expected, expr.span)
            };
        }
        let remaining = self.type_parameters[&owner]
            .len()
            .saturating_sub(fixed.len());
        let inputs = self.expanded_type_inputs(args, remaining, expr.span)?;
        if remaining == 0 || inputs.len() != remaining {
            return Err(error(expr.span, "method type argument count mismatch"));
        }
        for (name, ty) in inputs {
            if name.is_some() {
                return Err(error(
                    expr.span,
                    "explicit type arguments currently require positional inputs",
                ));
            }
            fixed.push(ty);
        }
        let value = self.function_instance(expr, owner, fixed, None, allow_optional)?;
        let value = self.retain_method_origin(method, value);
        if allow_optional {
            Ok(value)
        } else {
            self.callable_value_view(value, expected, expr.span)
        }
    }
}

impl Checker<'_, '_> {
    #[inline(never)]
    pub(super) fn generic_trait_call(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        callee: n::Expr,
        args: &[Arg<'_>],
        prefix: Vec<n::Expr>,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        let K::TraitMethod {
            interface,
            interface_arguments,
            subject,
            member,
            ..
        } = callee.kind
        else {
            unreachable!()
        };
        let fixed: Vec<_> = std::iter::once(subject.clone())
            .chain(interface_arguments.iter().cloned())
            .collect();
        let inherited = fixed.len();
        let mut call =
            self.generic_call_with_receiver(expr, owner, args, expected, partial, prefix, fixed)?;
        fn prepared_callee(call: &mut n::Expr) -> Option<&mut n::Expr> {
            match &mut call.kind {
                K::Call { callee, .. } => Some(callee),
                K::CallPreparation {
                    result: Some(result),
                    ..
                } => prepared_callee(result),
                _ => None,
            }
        }
        let callee = prepared_callee(&mut call)
            .ok_or_else(|| error(expr.span, "generic method preparation lacks a call"))?;
        let K::FunctionInstance { arguments, .. } = &callee.kind else {
            unreachable!()
        };
        let method_arguments = arguments[inherited..].to_vec();
        let scope = self.call_inference_scope();
        let frames = if scope.is_empty() {
            vec![]
        } else {
            vec![scope.iter().map(|p| p.sort).collect()]
        };
        let signature = self
            .signatures
            .get(&owner)
            .or_else(|| self.interface_signatures.get(&owner))
            .unwrap()
            .instantiate_parameters_in_scope(&self.declaration_binders(owner), arguments, &frames)
            .map_err(|e| error(expr.span, e.to_string()))?;
        callee.ty = self.callable_type(owner, signature);
        callee.kind = K::TraitMethod {
            interface,
            interface_arguments,
            member,
            subject,
            method_arguments,
        };
        Ok(call)
    }
}

impl Checker<'_, '_> {
    /// Expose the selected callable type to value contexts while retaining the
    /// member's declaration signature on the inner node for phase validation.
    pub(super) fn callable_value_view(
        &self,
        value: n::Expr,
        expected: Option<&Term>,
        span: Span,
    ) -> Result<n::Expr, Error> {
        if let Some((owner, _)) = self.method_instantiation_inputs(&value) {
            if let Some(ItemKind::Fn(signature, _) | ItemKind::FnDeclaration(signature)) =
                self.package.item(owner).map(|item| &item.kind)
            {
                if signature
                    .params
                    .iter()
                    .any(|parameter| parameter.ty.is_implicit() || parameter.ty.is_lambda())
                {
                    arguments::require_function_value_parameters(signature.params, span)?;
                }
            }
        }
        let ty = self.callable_input_view(&value.ty, span)?;
        if let Some(expected) = expected {
            self.require(&ty, expected, span)?;
        }
        if ty == value.ty {
            return Ok(value);
        }
        Ok(n::Expr {
            ty,
            source: value.source,
            kind: K::Block {
                statements: vec![],
                result: Some(Box::new(value)),
            },
        })
    }

    pub(super) fn callable_input_view(&self, signature: &Term, span: Span) -> Result<Term, Error> {
        self.callable_input_view_budgeted(signature, Budget::default(), span)
    }

    pub(super) fn callable_input_view_budgeted(
        &self,
        signature: &Term,
        budget: Budget,
        span: Span,
    ) -> Result<Term, Error> {
        // This view is also used for complete interface argument vectors.
        // A finished symbol index has no associated Type projection to solve.
        if matches!(signature, Term::Symbol(_)) {
            let mut fuel = budget.steps;
            crate::associated::charge_projection_copy(signature, &mut fuel)
                .map_err(|e| error(span, e))?;
            return Ok(signature.clone());
        }
        if !self.interfaces_ready {
            return Ok(signature.clone());
        }
        if !signature
            .substitution_parameters()
            .map_err(|e| error(span, e.to_string()))?
            .is_empty()
        {
            let parameters = self.call_inference_scope();
            if parameters.is_empty() {
                return Err(error(
                    span,
                    "dependent callable has no enclosing function frame",
                ));
            }
            let scoped = Term::Function {
                parameters,
                result: Box::new(signature.clone()),
            };
            scoped
                .check_scope(&[])
                .map_err(|e| error(span, e.to_string()))?;
            let checked = self.callable_input_view_budgeted(&scoped, budget, span)?;
            let Term::Function { result, .. } = checked else {
                unreachable!()
            };
            return Ok(*result);
        }
        let assumptions = self.capability_assumptions();
        crate::capabilities::World {
            index: &self.implementation_index,
            implementations: &self.checked_implementations,
            traits: &self.checked_traits,
            assumptions: &assumptions,
            context: &self.context,
        }
        .associated_type_view(signature, budget)
        .map(|value| value.value)
        .map_err(|message| error(span, message))
    }
}
