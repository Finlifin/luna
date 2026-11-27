//! Associated signatures are admitted before their values are demanded.
//! Rows carry completed values or checked programs awaiting concrete inputs.
use super::*;
use nir::associated_initializers::MemberSource;

pub(in crate::program) fn associated_source<'a>(
    row: &'a n::Implementation,
    member: DeclarationId,
    steps: &mut u64,
    span: Span,
) -> Result<Option<MemberSource<'a>>, Error> {
    let before = usize::try_from(*steps).unwrap_or(usize::MAX);
    let mut fuel = before;
    let result = row.associated_source(member, &mut fuel);
    *steps -= (before - fuel) as u64;
    result.map_err(|failure| error(span, failure))
}

fn associated_program_is_open(
    value: &n::Expr,
    parameters: &[DeclarationId],
    steps: &mut u64,
    span: Span,
) -> Result<bool, Error> {
    let used =
        nir::copy_budget::expression_declaration_inputs(value, steps).map_err(|failure| {
            error(
                span,
                format!("associated default input observation failed: {failure:?}"),
            )
        })?;
    if parameters.iter().any(|id| used.contains(id)) {
        return Ok(true);
    }
    let before = usize::try_from(*steps).unwrap_or(usize::MAX);
    let mut fuel = before;
    let mut member_read = false;
    let observed = iir::visit::expression(value, &mut fuel, |expr, _| {
        member_read |= matches!(expr.kind, K::AssociatedValue { .. });
        Ok(())
    });
    *steps -= (before - fuel) as u64;
    observed.map_err(|failure| error(span, failure))?;
    Ok(member_read)
}

enum Supply {
    Value(n::Expr),
    Initializer(nir::associated_initializers::AssociatedInitializer),
}

impl Checker<'_, '_> {
    pub(in crate::program) fn associated_contract(
        &mut self,
        owner: OwnerId,
        evaluate_default: bool,
    ) -> Result<n::AssociatedContract, Error> {
        let item = self.package.item(owner).unwrap();
        let ItemKind::Associated(associated) = &item.kind else {
            unreachable!()
        };
        if !associated.clause_params.is_empty() {
            return Err(error(
                item.span,
                "associated value declarations do not support where type/clause parameters",
            ));
        }
        if !associated.clause_constraints.is_empty() && associated.parameters.is_empty() {
            self.ensure_contract(owner, item.span)?;
        }
        self.in_formal_declaration(owner, |this| {
            this.context = this.definition_context(owner);
            if associated.parameters.is_empty() {
                let ty = this.type_expr(associated.ty)?;
                let default =
                    if let Some(initializer) = associated.default.filter(|_| evaluate_default) {
                        // A definition-side Type value may contain its stable Self or
                        // interface binders. Form the template without interpreting it
                        // as a closed runtime type; selection substitutes those binders.
                        if ty == p(P::Type) {
                            Some(n::Expr {
                                ty: ty.clone(),
                                source: source(initializer.span),
                                kind: K::TypeValue(this.type_expr(initializer)?),
                            })
                        } else {
                            Some(this.constant_value(owner, item.span)?)
                        }
                    } else {
                        None
                    };
                return Ok(n::AssociatedContract {
                    phase: this.member_phases.get(&owner).copied().unwrap_or_default(),
                    declaration: declaration(this.package, &this.package_identities, owner),
                    name: item.ident.name.to_string(),
                    ty,
                    default,
                });
            }
            this.in_required_comptime = true;
            let (param_ids, param_types, result_ty, req_predicates) =
                this.bind_associated_parameters(associated, item.span)?;
            let ty = Term::ComptimeFunction(Box::new(Term::Function {
                parameters: param_types,
                result: Box::new(
                    result_ty
                        .clone()
                        .abstract_parameters(&[])
                        .map_err(|e| error(item.span, e.to_string()))?,
                ),
            }));
            let default = if let Some(initializer) = associated.default.filter(|_| evaluate_default)
            {
                Some(this.associated_closure_default(
                    &param_ids,
                    &result_ty,
                    &ty,
                    req_predicates,
                    initializer,
                )?)
            } else {
                None
            };
            Ok(n::AssociatedContract {
                phase: this.member_phases.get(&owner).copied().unwrap_or_default(),
                declaration: declaration(this.package, &this.package_identities, owner),
                name: item.ident.name.to_string(),
                ty,
                default,
            })
        })
    }

    fn bind_associated_parameters(
        &mut self,
        associated: &hir::item::AssociatedDecl<'_>,
        span: Span,
    ) -> Result<(Vec<n::LocalId>, Vec<Parameter>, Term, Vec<n::Expr>), Error> {
        let mut param_ids = Vec::with_capacity(associated.parameters.len());
        let mut param_types = Vec::with_capacity(associated.parameters.len());
        for parameter in associated.parameters {
            let annotation = match parameter.ty.kind {
                hir::common::TyParamKind::Positional(expr)
                | hir::common::TyParamKind::PositionalDependencyCatched(_, expr) => expr,
                _ => {
                    return Err(error(
                        parameter.span,
                        "associated parameter requires an explicit positional type annotation",
                    ));
                }
            };
            if matches!(annotation.kind, H::TyPlaceholder) {
                return Err(error(
                    parameter.span,
                    "associated parameter requires an explicit type annotation",
                ));
            }
            let param_ty = self.type_expr(annotation)?;
            let id = local(parameter.hir_id);
            self.locals.insert(id, param_ty.clone());
            param_ids.push(id);
            param_types.push(Parameter {
                sort: Sort::Runtime,
                ty: param_ty
                    .abstract_parameters(&[])
                    .map_err(|e| error(parameter.span, e.to_string()))?,
            });
        }
        let mut req_predicates = Vec::new();
        for clause in associated.clause_constraints {
            match &clause.kind {
                hir::clause::ClauseConstraintKind::Requires(value) => {
                    if self.is_type_assumption(value) && !self.program_requirement(value)? {
                        self.check_refinement_expression(value)?;
                        let predicate = self.type_predicate(value)?;
                        let fact = crate::ptc::Formula::from_predicate(&predicate);
                        self.context
                            .validate_formula(&fact, &mut crate::ptc::Budget::default())
                            .map_err(|failure| error(clause.span, failure.to_string()))?;
                        self.context.assume(fact);
                    } else {
                        let checked = self.expr(value, Some(&p(P::Bool)))?;
                        let src = checked.source;
                        req_predicates.push(n::Expr {
                            ty: p(P::Void),
                            source: src,
                            kind: K::RequirePredicate(Box::new(checked)),
                        });
                    }
                }
                hir::clause::ClauseConstraintKind::Ensures(_)
                | hir::clause::ClauseConstraintKind::Decreases(_)
                | hir::clause::ClauseConstraintKind::Outcome => {
                    return Err(error(
                        clause.span,
                        "associated declarations only support requires precondition clauses",
                    ));
                }
            }
        }
        let result_ty = self.type_expr(associated.ty)?;
        let _ = span;
        Ok((param_ids, param_types, result_ty, req_predicates))
    }

    fn associated_closure_default(
        &mut self,
        param_ids: &[n::LocalId],
        result_ty: &Term,
        callable_ty: &Term,
        mut req_predicates: Vec<n::Expr>,
        initializer: &Expr<'_>,
    ) -> Result<n::Expr, Error> {
        let value = self.expr(initializer, Some(result_ty))?;
        let body = if req_predicates.is_empty() {
            value
        } else {
            let src = source(initializer.span);
            let ty = value.ty.clone();
            req_predicates.reserve(0);
            n::Expr {
                ty,
                source: src,
                kind: K::Block {
                    statements: req_predicates,
                    result: Some(Box::new(value)),
                },
            }
        };
        Ok(n::Expr {
            ty: callable_ty.clone(),
            source: source(initializer.span),
            kind: K::Closure {
                parameters: param_ids.to_vec(),
                captures: Vec::new(),
                body: std::sync::Arc::new(body),
            },
        })
    }
}

impl Checker<'_, '_> {
    pub(in crate::program) fn associated_projection(
        &mut self,
        view: &Expr<'_>,
        name: &str,
        span: Span,
        site: hir::HirId,
    ) -> Result<Option<n::Expr>, Error> {
        let (base, explicit) = match view.kind {
            H::Cast(base, interface) => (base, Some(interface)),
            _ => (view, None),
        };
        let mut subject = if self.is_type_receiver(base)? {
            self.type_expr(base)?
        } else if let H::Projection(view, field) = &base.kind {
            match self.associated_projection(view, field.name.as_str(), base.span, base.hir_id)? {
                Some(n::Expr {
                    kind: K::TypeValue(ty),
                    ..
                }) => ty,
                _ => return Ok(None),
            }
        } else {
            return Ok(None);
        };
        let mut requested = if let Some(interface) = explicit {
            self.interface_expr_for(interface, &subject)?
        } else if let Some(interface) = self.context_member_interface(&subject, name, span)? {
            interface
        } else {
            // Inherent methods precede implicit trait discovery. This probe
            // checks membership only; it does not evaluate a receiver, body, or
            // contextual dispatch contract.
            if self.has_inherent_method(subject.clone(), name, span)? {
                return Ok(None);
            }
            let Some(interface) = self.discovered_member_interface(&subject, name, site, span)?
            else {
                return Ok(None);
            };
            interface
        };
        // The declaration is checked separately under its abstract binders.
        // While evaluating a selected initializer, queries use its concrete
        // (or implementation-generic) Self and interface arguments.
        let mut steps = self.required_steps;
        if let Some((parameters, arguments)) = self.initializer_substitution() {
            let view = nominal_fields::metered(&mut steps, |fuel| {
                let substitution =
                    self.bounded_input_substitution(parameters, arguments, &[], fuel)?;
                Ok((
                    substitution.instantiate_interface_bounded(&requested, fuel)?,
                    substitution.instantiate_bounded(&subject, fuel)?,
                ))
            });
            self.required_steps = steps;
            (requested, subject) = view.map_err(|e| error(span, e.to_string()))?;
        }
        if self.checked_traits.is_empty() && !self.interfaces_ready {
            return self.declaration_associated_projection(subject, requested, name, span);
        }
        let Some(interface) = self.inherited_member_source(&subject, &requested, name, span)?
        else {
            return Ok(None);
        };
        let Some(contract) = self
            .checked_traits
            .by_id(interface.declaration)
            .and_then(|t| t.associated.iter().find(|a| a.name == name))
        else {
            return Ok(None);
        };
        let member = contract.declaration;
        let member_type = contract.ty.clone();
        if self.checking_method_contract && !self.interfaces_ready && member_type != p(P::Type) {
            // Keep the observation symbolic. Selecting or invoking its initializer
            // before all implementations are checked would make declaration order
            // significant and could publish an invalid implementation's value.
            let (parameters, inputs) = self.interface_substitution(&interface, &subject);
            let ty = self.instantiate_input_types(&member_type, &parameters, &inputs, span)?;
            self.defer_contract_projection(
                n::TraitRequirement {
                    subject: subject.clone(),
                    interface: requested.declaration,
                    interface_arguments: requested.arguments,
                },
                span,
            )?;
            return Ok(Some(n::Expr {
                ty,
                source: source(span),
                kind: K::AssociatedValue {
                    interface,
                    member,
                    subject,
                },
            }));
        }
        if let Some(implementation) = self.initializer_implementation()
            && implementation.subject == subject
            && implementation.interface == Some(interface.declaration)
            && implementation.interface_arguments == interface.arguments
        {
            let mut value =
                self.selected_associated_value(implementation.declaration, member, span)?;
            value.source = source(span);
            return Ok(Some(value));
        }

        if interface != requested {
            let incomplete_requested = match self.symbolic_interface_evidence(&subject, &requested)
            {
                Ok(true) => None,
                Ok(false) => Some(None),
                Err(e @ nir::NominalAdmissionError::Unresolved(_)) => Some(Some(e)),
                Err(e) => return Err(capability_query_error(span, e)),
            };
            if let Some(incomplete) = incomplete_requested {
                let resolution = self
                    .resolve_trait_capability(&requested, &subject, Budget::default())
                    .map_err(|message| error(span, message))?;
                if !matches!(
                    resolution.selection,
                    crate::implementation::Selection::Selected(_)
                ) {
                    if let Some(given_error) = incomplete {
                        return Err(capability_query_error(span, given_error));
                    }
                    return Err(error(span, "requested interface view is not established"));
                }
            }
        }
        let incomplete_given = match self.symbolic_interface_evidence(&subject, &interface) {
            Ok(true) => {
                // Given evidence fixes the member's origin, not its selected value.
                // In particular, a default must not become an abstract equality.
                let (parameters, inputs) = self.interface_substitution(&interface, &subject);
                let ty = self.instantiate_input_types(&member_type, &parameters, &inputs, span)?;
                if ty != p(P::Type) {
                    return Ok(Some(n::Expr {
                        ty,
                        source: source(span),
                        kind: K::AssociatedValue {
                            interface,
                            member,
                            subject,
                        },
                    }));
                }
                return Ok(Some(n::Expr {
                    ty,
                    source: source(span),
                    kind: K::TypeValue(Term::AssociatedType {
                        origin: Box::new(nir::AssociatedTypeOrigin {
                            interface: interface.declaration,
                            member,
                        }),
                        arguments: interface.arguments,
                        subject: Box::new(subject),
                    }),
                }));
            }
            Ok(false) => None,
            Err(e @ nir::NominalAdmissionError::Unresolved(_)) => Some(e),
            Err(e) => return Err(capability_query_error(span, e)),
        };
        if std::iter::once(&subject)
            .chain(&interface.arguments)
            .any(|ty| {
                ty.substitution_parameters()
                    .is_ok_and(|slots| !slots.is_empty())
            })
        {
            if let Some(given_error) = incomplete_given {
                return Err(capability_query_error(span, given_error));
            }
            return Err(Error {
                retry_after_capability_progress: true,
                ..error(
                    span,
                    "associated projection with ordinary Type inputs requires established trait evidence",
                )
            });
        }
        let resolution = self
            .resolve_trait_capability(&interface, &subject, Budget::default())
            .map_err(|message| error(span, message))?;
        let crate::implementation::Selection::Selected(selected) = resolution.selection else {
            if let Some(given_error) = incomplete_given {
                return Err(capability_query_error(span, given_error));
            }
            return Err(Error {
                retry_after_capability_progress: true,
                ..error(
                    span,
                    format!(
                        "associated value requires an established concrete implementation or a given interface constraint: {resolution}"
                    ),
                )
            });
        };
        let Some(crate::implementation::TypeAssessment::Applicable(evidence)) =
            resolution.assessments.get(&selected)
        else {
            return Err(error(
                span,
                "associated value selection lacks implementation evidence",
            ));
        };
        let (slot, contract) = self.associated_supply_contract(selected, member, span)?;
        if let Some(value) = self.associated_data_read(interface, subject, contract, span)? {
            return Ok(Some(value));
        }

        // A Type used as a receiver or annotation needs its selected identity.
        // Every other member read keeps its declaration origin until observation.
        let value = self.prepared_associated_value(selected, member, span)?;
        let implementation = &self.checked_implementations[slot];
        let mut value = nominal_fields::metered(&mut self.required_steps, |fuel| {
            value.instantiate_declaration_parameters_bounded(
                &implementation.type_parameters,
                &implementation.parameter_types,
                &evidence.type_arguments,
                &[],
                fuel,
            )
        })
        .map_err(|e| error(span, e.to_string()))?;
        value.source = source(span);
        Ok(Some(value))
    }
}

impl Checker<'_, '_> {
    pub(in crate::program) fn associated_owner(&self, member: DeclarationId) -> Option<OwnerId> {
        self.source_declarations.owner(member)
    }

    pub(in crate::program) fn associated_has_initializer(&self, member: DeclarationId) -> bool {
        self.associated_owner(member)
            .and_then(|owner| self.package.item(owner))
            .is_some_and(
                |item| matches!(&item.kind, ItemKind::Associated(value) if value.default.is_some()),
            )
    }

    // The member contract supplies the inner node's type. Selection may also
    // expose a concrete outer view (for example an associated callable type).
    // Only a Type projection needs the identity stored in the implementation.
    fn associated_data_read(
        &self,
        interface: n::InterfaceInstance,
        subject: Term,
        contract: &n::AssociatedContract,
        span: Span,
    ) -> Result<Option<n::Expr>, Error> {
        let (parameters, inputs) = self.interface_substitution(&interface, &subject);
        let ty = self.instantiate_input_types(&contract.ty, &parameters, &inputs, span)?;
        if ty == p(P::Type) {
            return Ok(None);
        }
        let reference = self.callable_value_view(
            n::Expr {
                ty,
                source: source(span),
                kind: K::AssociatedValue {
                    interface,
                    member: contract.declaration,
                    subject,
                },
            },
            None,
            span,
        )?;
        Ok((reference.ty != p(P::Type)).then_some(reference))
    }

    fn selected_associated_value(
        &mut self,
        implementation: DeclarationId,
        member: DeclarationId,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let (slot, contract) = self.associated_supply_contract(implementation, member, span)?;
        let row = &self.checked_implementations[slot];
        let interface = row
            .interface_instance()
            .expect("associated trait implementation");
        if let Some(value) =
            self.associated_data_read(interface, row.subject.clone(), contract, span)?
        {
            return Ok(value);
        }
        self.prepared_associated_value(implementation, member, span)
    }

    fn prepared_associated_value(
        &mut self,
        implementation: DeclarationId,
        member: DeclarationId,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let slot = self.ensure_associated_value(implementation, member, span)?;
        let selected = &self.checked_implementations[slot];
        let initializer = match associated_source(selected, member, &mut self.required_steps, span)?
            .expect("prepared associated supply")
        {
            MemberSource::Value(value) => {
                constants::charge_copy(value, &mut self.required_steps, span)?;
                return Ok(value.clone());
            }
            MemberSource::Initializer(initializer) => initializer,
        };
        Ok(n::Expr {
            ty: initializer.expression().ty.clone(),
            source: source(span),
            kind: K::AssociatedValue {
                interface: selected
                    .interface_instance()
                    .expect("associated trait implementation"),
                subject: selected.subject.clone(),
                member,
            },
        })
    }

    fn associated_supply_contract(
        &self,
        implementation: DeclarationId,
        member: DeclarationId,
        span: Span,
    ) -> Result<(usize, &n::AssociatedContract), Error> {
        let slot = *self
            .implementation_positions
            .get(&implementation)
            .ok_or_else(|| error(span, "unknown associated implementation"))?;
        let selected = &self.checked_implementations[slot];
        let contract = selected
            .interface
            .and_then(|id| self.checked_traits.by_id(id))
            .and_then(|interface| {
                interface
                    .associated
                    .iter()
                    .find(|item| item.declaration == member)
            })
            .ok_or_else(|| {
                error(
                    span,
                    "associated member does not belong to selected implementation",
                )
            })?;
        if contract.phase == n::TraitPhase::Comptime && !selected.phases.comptime {
            return Err(error(
                span,
                "selected implementation lacks comptime trait phase",
            ));
        }
        Ok((slot, contract))
    }

    /// Publish the selected supply once. Definition checking and dependency
    /// discovery only need its row; building a use-site expression is separate.
    pub(in crate::program) fn ensure_associated_value(
        &mut self,
        implementation: DeclarationId,
        member: DeclarationId,
        span: Span,
    ) -> Result<usize, Error> {
        let (slot, _) = self.associated_supply_contract(implementation, member, span)?;
        let selected = &self.checked_implementations[slot];
        if associated_source(selected, member, &mut self.required_steps, span)?.is_some() {
            return Ok(slot);
        }
        let generic = !selected.type_parameters.is_empty();
        let key = (implementation, member);
        let AssociatedRecipe {
            owner,
            parameters,
            arguments,
            ..
        } = self
            .associated_initializers
            .get(&key)
            .cloned()
            .ok_or_else(|| {
                error(
                    span,
                    "selected implementation has no associated value initializer",
                )
            })?;
        if self.active_associated_values.len() >= 64 || !self.active_associated_values.insert(key) {
            return Err(error(
                span,
                "cyclic or excessively deep associated value dependency",
            ));
        }
        let saved_selection = self.selected_associated_initializer.replace(key);
        let result = (|| {
            let mut steps = self.required_steps;
            let inputs = nominal_fields::metered(&mut steps, |fuel| {
                self.bounded_input_substitution(&parameters, &arguments, &[], fuel)
            });
            self.required_steps = steps;
            let inputs = inputs.map_err(|failure| error(span, failure.to_string()))?;
            if generic {
                let contract = self.associated_contract(owner, false)?;
                let template = self.associated_default_template(owner, &contract.ty)?;
                let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let mut fuel = before;
                let specialized = template.instantiate(&inputs, &mut fuel);
                self.required_steps -= (before - fuel) as u64;
                let template = specialized.map_err(|failure| error(span, failure))?;
                let before = fuel;
                let completed = nir::completed_values::validate_with_collection_evidence_in(
                    template.expression(),
                    self.newtypes.values(),
                    &|ty| self.bound_tuple_elements(ty),
                    &|ty, fuel| {
                        let element = self.list_element_type(ty);
                        if let Some(element) = element {
                            nir::type_computations::charge_term(element, fuel)?;
                        }
                        Ok(element.cloned())
                    },
                    &|id, fuel| {
                        *fuel = fuel
                            .checked_sub(1)
                            .ok_or(nir::NominalAdmissionError::Budget)?;
                        Ok(match (self.structs.get(&id), self.enums.get(&id)) {
                            (Some(definition), None) => Some(
                                nir::normal_forms::NominalValueDeclaration::Struct(definition),
                            ),
                            (None, Some(definition)) => {
                                Some(nir::normal_forms::NominalValueDeclaration::Enum(definition))
                            }
                            (None, None) => None,
                            _ => return Err("duplicate completed nominal declaration".into()),
                        })
                    },
                    self.target,
                    &mut fuel,
                );
                self.required_steps -= (before - fuel) as u64;
                match completed {
                    Ok(()) => {
                        return template
                            .into_expression(&mut self.required_steps)
                            .map(Supply::Value)
                            .map_err(|failure| error(span, failure));
                    }
                    Err(
                        failure @ (nir::NominalAdmissionError::Budget
                        | nir::NominalAdmissionError::Unresolved(_)),
                    ) => return Err(error(span, failure)),
                    Err(nir::NominalAdmissionError::Invalid(_)) => {
                        // Keep closed Type identities available to passive signature queries.
                        if contract.ty != p(P::Type)
                            || associated_program_is_open(
                                template.expression(),
                                &self.checked_implementations[slot].type_parameters,
                                &mut self.required_steps,
                                span,
                            )?
                        {
                            return Ok(Supply::Initializer(template));
                        }
                    }
                }
            }
            let value = self
                .associated_contract(owner, true)?
                .default
                .ok_or_else(|| error(span, "associated value initializer is missing"))?;
            nominal_fields::metered(&mut self.required_steps, |fuel| {
                inputs
                    .instantiate_expression_bounded(&value, fuel)
                    .map_err(Into::into)
            })
            .map(Supply::Value)
            .map_err(|e| error(span, e.to_string()))
        })();
        self.selected_associated_initializer = saved_selection;
        self.active_associated_values.remove(&key);
        let selected = &mut self.checked_implementations[slot];
        match result? {
            Supply::Value(value) => selected.associated.push((member, value)),
            Supply::Initializer(initializer) => {
                selected.associated_initializers.push((member, initializer))
            }
        }
        Ok(slot)
    }

    pub(in crate::program) fn complete_associated_values(&mut self) -> Result<(), Error> {
        // Admission already checked every member's existence and declared type.
        // Value dependencies can now query the coherent implementation world.
        // Selected Type templates supply the immutable world used while
        // checking scalar initializer calls and their formation predicates.
        // Publish them first, independent of declaration order. Dependencies
        // inside a template still use selected_associated_value's demand and
        // cycle checks; this does not turn abstract defaults into equations.
        let type_members: BTreeSet<_> = self
            .checked_traits
            .iter()
            .flat_map(|interface| interface.associated.iter())
            .filter(|member| member.ty == p(P::Type))
            .map(|member| member.declaration)
            .collect();
        let mut pending: Vec<_> = self.associated_initializers.keys().copied().collect();
        pending.sort_by_key(|(_, member)| !type_members.contains(member));
        for (implementation, member) in pending {
            let owner = self.associated_initializers[&(implementation, member)].owner;
            self.ensure_associated_value(
                implementation,
                member,
                self.package.item(owner).unwrap().span,
            )?;
        }
        self.check_complementary_associated()?;
        let members: Vec<_> = self
            .checked_traits
            .iter()
            .flat_map(|t| {
                t.associated
                    .iter()
                    .map(move |a| (t.declaration, a.declaration))
            })
            .collect();
        for (interface, member) in members {
            let owner = self.associated_owner(member).unwrap();
            let mut contract = self.associated_contract(owner, false)?;
            if self.associated_has_initializer(member) {
                let template = self.associated_default_template(owner, &contract.ty)?;
                let span = self.package.item(owner).unwrap().span;
                let parameters = self
                    .checked_traits
                    .by_id(interface)
                    .unwrap()
                    .parameters_with_self();
                contract.default = if associated_program_is_open(
                    template.expression(),
                    &parameters,
                    &mut self.required_steps,
                    span,
                )? {
                    Some(
                        template
                            .into_expression(&mut self.required_steps)
                            .map_err(|failure| error(span, failure))?,
                    )
                } else {
                    self.associated_contract(owner, true)?.default
                };
            }
            std::sync::Arc::make_mut(&mut self.checked_traits)
                .complete_associated(interface, contract);
        }
        // Demand order is not declaration order: normalize the published table.
        for implementation in &mut self.checked_implementations {
            if let Some(interface) = implementation.interface {
                let members = &self.checked_traits.by_id(interface).unwrap().associated;
                implementation.associated.sort_by_key(|(member, _)| {
                    members
                        .iter()
                        .position(|a| a.declaration == *member)
                        .unwrap()
                });
            }
        }
        Ok(())
    }
}

impl Checker<'_, '_> {
    pub(in crate::program) fn associated_reference_is_type(&self, expr: &Expr<'_>) -> bool {
        let Some(owner) = self.associated_reference_owner(expr) else {
            return false;
        };
        let member = declaration(self.package, &self.package_identities, owner);
        if self
            .checked_traits
            .iter()
            .flat_map(|t| &t.associated)
            .any(|a| a.declaration == member && a.ty == p(P::Type))
        {
            return true;
        }
        let Some(parent) = self.self_containers.get(&owner) else {
            return false;
        };
        let original = declaration(self.package, &self.package_identities, *parent);
        let implementation = self
            .implementation_relationships
            .get(&original)
            .copied()
            .unwrap_or(original);
        let Some(interface) = self
            .checked_implementations
            .iter()
            .find(|i| i.declaration == implementation)
            .and_then(|i| i.interface)
        else {
            return false;
        };
        let name = self.package.item(owner).unwrap().ident.name.as_str();
        self.checked_traits.by_id(interface).is_some_and(|t| {
            t.associated
                .iter()
                .any(|a| a.name == name && a.ty == p(P::Type))
        })
    }

    pub(in crate::program) fn associated_reference_owner(
        &self,
        expr: &Expr<'_>,
    ) -> Option<OwnerId> {
        let H::ResolvedDef { def_id, .. } = expr.kind else {
            return None;
        };
        self.package.definition_owner(def_id).filter(|owner| {
            self.package
                .item(*owner)
                .is_some_and(|item| matches!(item.kind, ItemKind::Associated(_)))
        })
    }

    pub(in crate::program) fn lexical_associated_value(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let referenced = declaration(self.package, &self.package_identities, owner);
        if let Some(implementation) = self.initializer_selection(referenced) {
            let mut value = self.selected_associated_value(implementation, referenced, span)?;
            value.source = source(span);
            return Ok(value);
        }
        let container = self.self_containers.get(&owner).copied();
        if container.is_none_or(|parent| {
            matches!(
                self.package.item(parent).map(|item| &item.kind),
                Some(ItemKind::Trait(_))
            )
        }) {
            if let Some(value) = self.abstract_associated_type(owner, span)? {
                return Ok(value);
            }
        }
        let parent = container.ok_or_else(|| {
            error(
                span,
                if self.self_types.contains_key(&owner) {
                    "abstract associated value requires its enclosing trait context"
                } else {
                    "associated value has no interface container"
                },
            )
        })?;
        if !matches!(
            self.package.item(parent).map(|item| &item.kind),
            Some(ItemKind::Impl(_))
        ) {
            return Err(error(
                span,
                "associated value requires a trait or implementation container",
            ));
        }
        let current_parent = self
            .signature_owner
            .and_then(|owner| self.self_containers.get(&owner).copied());
        if current_parent != Some(parent) {
            return Err(error(
                span,
                "associated implementation value requires its enclosing implementation context",
            ));
        }
        if !self.interfaces_ready {
            // A lexical implementation Type definition can form its own
            // signatures before interface admission. This does not select an
            // implementation or assume that its eventual contract is valid.
            // Cache only completed templates in their definition's rigid
            // context. Admission later uses its own selected-value cache.
            if let Some(value) = self.associated_signature_values.get(&owner) {
                constants::charge_copy(value, &mut self.required_steps, span)?;
                let mut value = value.clone();
                value.source = source(span);
                return Ok(value);
            }
            if self.active_associated_signatures.len() >= 32
                || !self.active_associated_signatures.insert(owner)
            {
                return Err(error(
                    span,
                    "cyclic or excessively deep associated signature dependency",
                ));
            }
            let result = (|| {
                let contract = self.associated_contract(owner, false)?;
                if contract.ty != p(P::Type) {
                    let returns_type = contract.ty.is_comptime_callable()
                        && matches!(
                            contract.ty.callable_signature(),
                            Term::Function { result, .. } if **result == p(P::Type)
                        );
                    if !returns_type {
                        return Err(error(
                            span,
                            "associated value signature requires the interface world to be established",
                        ));
                    }
                    let ItemKind::Impl(def) = &self.package.item(parent).unwrap().kind else {
                        unreachable!()
                    };
                    let trait_ref = def.trait_ref.ok_or_else(|| {
                        error(
                            span,
                            "associated trait member requires a trait implementation",
                        )
                    })?;
                    let own_subject = self.type_expr(def.self_ty)?;
                    let interface = self.interface_expr_for(trait_ref, &own_subject)?;
                    let trait_owner =
                        self.associated_owner(interface.declaration)
                            .ok_or_else(|| {
                                error(span, "associated value has no originating interface owner")
                            })?;
                    let name = self.package.item(owner).unwrap().ident.name.as_str();
                    let trait_def = match &self.package.item(trait_owner).unwrap().kind {
                        ItemKind::Trait(t) => t,
                        _ => return Err(error(span, "originating interface must be a trait")),
                    };
                    let member_owner = trait_def
                        .items
                        .iter()
                        .copied()
                        .find(|m| self.package.item(*m).unwrap().ident.name.as_str() == name)
                        .ok_or_else(|| {
                            error(span, "associated value has no originating interface member")
                        })?;
                    let member = declaration(self.package, &self.package_identities, member_owner);
                    return Ok(n::Expr {
                        ty: contract.ty,
                        source: source(span),
                        kind: K::AssociatedValue {
                            interface,
                            member,
                            subject: own_subject,
                        },
                    });
                }
                let mut value =
                    self.associated_contract(owner, true)?
                        .default
                        .ok_or_else(|| {
                            error(
                                span,
                                "implementation associated type requires an initializer",
                            )
                        })?;
                constants::charge_copy(&value, &mut self.required_steps, span)?;
                value.source = source(span);
                Ok(value)
            })();
            self.active_associated_signatures.remove(&owner);
            if let Ok(value) = &result {
                constants::charge_copy(value, &mut self.required_steps, span)?;
                self.associated_signature_values
                    .insert(owner, value.clone());
            }
            return result;
        }
        let original = declaration(self.package, &self.package_identities, parent);
        let implementation = self
            .implementation_relationships
            .get(&original)
            .copied()
            .unwrap_or(original);
        let interface = self
            .checked_implementations
            .iter()
            .find(|i| i.declaration == implementation)
            .and_then(|i| i.interface)
            .ok_or_else(|| {
                error(
                    span,
                    "associated value has no admitted interface implementation",
                )
            })?;
        let name = self.package.item(owner).unwrap().ident.name.as_str();
        let member = self
            .checked_traits
            .by_id(interface)
            .and_then(|t| t.associated.iter().find(|a| a.name == name))
            .map(|a| a.declaration)
            .ok_or_else(|| error(span, "associated value has no originating interface member"))?;
        let mut value = self.selected_associated_value(implementation, member, span)?;
        if original != implementation
            && self
                .signature_owner
                .and_then(|owner| self.initializer_bindings(owner))
                .map(|(id, _, _)| id)
                != Some(implementation)
        {
            let canonical = self
                .checked_implementations
                .iter()
                .find(|item| item.declaration == implementation)
                .unwrap();
            let binders = canonical
                .parameter_binders()
                .map_err(|e| error(span, e.to_string()))?;
            let parameters = self
                .type_parameters
                .get(&parent)
                .cloned()
                .unwrap_or_default();
            let arguments = self
                .input_binders(&parameters)
                .iter()
                .map(|parameter| Term::Rigid {
                    declaration: parameter.declaration,
                    sort: parameter.sort,
                })
                .collect::<Vec<_>>();
            constants::charge_copy(&value, &mut self.required_steps, span)?;
            value = value
                .instantiate_parameters(&binders, &arguments)
                .map_err(|e| error(span, e.to_string()))?;
        }
        value.source = source(span);
        Ok(value)
    }

    fn abstract_associated_type(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<Option<n::Expr>, Error> {
        let Some(subject) = self.self_types.get(&owner).cloned() else {
            return Ok(None);
        };
        let Some(parent) = self.package.owners().find_map(|(candidate, info)| {
            match &info.node.expect_item().kind {
                ItemKind::Trait(def) if def.items.contains(&owner) => Some(candidate),
                _ => None,
            }
        }) else {
            return Ok(None);
        };
        let arguments: Vec<_> = self.type_parameters[&parent]
            .iter()
            .map(|parameter| Term::Rigid {
                declaration: *parameter,
                sort: self
                    .trait_input_domain(*parameter)
                    .as_ref()
                    .map_or(Sort::Type, Sort::of_comptime_domain),
            })
            .collect();
        let parent_decl = declaration(self.package, &self.package_identities, parent);
        let interface_instance = n::InterfaceInstance {
            declaration: parent_decl,
            arguments: arguments.clone(),
        };
        let target_subject = match self
            .signature_owner
            .and_then(|current| self.self_types.get(&current))
            .cloned()
        {
            Some(current) => {
                if current == subject
                    || self
                        .symbolic_interface_evidence(&current, &interface_instance)
                        .map_err(|failure| error(span, failure.to_string()))?
                {
                    current
                } else {
                    return Err(error(
                        span,
                        "abstract associated type requires its enclosing trait context",
                    ));
                }
            }
            None => {
                return Err(error(
                    span,
                    "abstract associated type requires its enclosing trait context",
                ));
            }
        };
        if self.active_associated_signatures.len() >= 32
            || !self.active_associated_signatures.insert(owner)
        {
            return Err(error(
                span,
                "cyclic or excessively deep associated signature dependency",
            ));
        }
        let contract = self.associated_contract(owner, false);
        self.active_associated_signatures.remove(&owner);
        let ty = contract?.ty;
        if ty != p(P::Type) {
            return Ok(Some(n::Expr {
                ty,
                source: source(span),
                kind: K::AssociatedValue {
                    interface: interface_instance,
                    member: declaration(self.package, &self.package_identities, owner),
                    subject: target_subject,
                },
            }));
        }
        Ok(Some(n::Expr {
            ty: p(P::Type),
            source: source(span),
            kind: K::TypeValue(Term::AssociatedType {
                origin: Box::new(nir::AssociatedTypeOrigin {
                    interface: parent_decl,
                    member: declaration(self.package, &self.package_identities, owner),
                }),
                arguments,
                subject: Box::new(target_subject),
            }),
        }))
    }
}

impl Checker<'_, '_> {
    /// Definition equations for this candidate only. They do not select or
    /// publish an implementation, and may not escape its checking context.
    pub(super) fn candidate_associated_equations(
        &mut self,
        signature: &Term,
        interface: &n::Trait,
        subject: &Term,
        arguments: &[Term],
        supplied: &BTreeMap<String, n::AssociatedContract>,
        span: Span,
    ) -> Result<Vec<(Term, Term)>, Error> {
        enum Work {
            Visit(Term, usize),
            Finish(DeclarationId, Term, Term),
        }
        let mut fuel = 10_000usize;
        crate::associated::charge_projection_copy(signature, &mut fuel)
            .map_err(|message| error(span, message))?;
        let mut pending = vec![Work::Visit(signature.clone(), 0)];
        let mut active = BTreeSet::new();
        let mut completed = BTreeSet::new();
        let mut equations = Vec::new();
        while let Some(work) = pending.pop() {
            fuel = fuel
                .checked_sub(1)
                .ok_or_else(|| error(span, "associated signature equation budget exhausted"))?;
            let Work::Visit(term, depth) = work else {
                let Work::Finish(member, projection, value) = work else {
                    unreachable!()
                };
                active.remove(&member);
                completed.insert(member);
                equations.push((projection, value));
                continue;
            };
            if depth >= 64 {
                return Err(error(span, "associated signature equation depth exhausted"));
            }
            match term {
                Term::AssociatedType {
                    ref origin,
                    subject: ref projected_subject,
                    arguments: ref projected_arguments,
                } if origin.interface == interface.declaration
                    && projected_subject.as_ref() == subject
                    && projected_arguments == arguments =>
                {
                    let member = origin.member;
                    if completed.contains(&member) {
                        continue;
                    }
                    if !active.insert(member) {
                        return Err(error(
                            span,
                            "cyclic implementation associated type dependency",
                        ));
                    }
                    let contract = interface
                        .associated
                        .iter()
                        .find(|a| a.declaration == member)
                        .ok_or_else(|| error(span, "associated signature has an unknown member"))?;
                    let (declaration, substitution) = if let Some(actual) =
                        supplied.get(&contract.name)
                    {
                        if actual.ty != p(P::Type) {
                            return Err(error(
                                span,
                                "associated signature requires a Type-valued implementation member",
                            ));
                        }
                        (actual.declaration, false)
                    } else {
                        (contract.declaration, true)
                    };
                    let owner = self.associated_owner(declaration).unwrap();
                    let Some(value) = self.associated_contract(owner, true)?.default else {
                        /* Another phase may provide this required signature item.
                         * Keep its equation unresolved until the merged Type world
                         * checks the deferred signature; completeness is separate.
                         */
                        active.remove(&member);
                        completed.insert(member);
                        continue;
                    };
                    constants::charge_copy(&value, &mut self.required_steps, span)?;
                    let K::TypeValue(mut value) = value.kind else {
                        return Err(error(
                            span,
                            "associated signature requires a completed Type definition",
                        ));
                    };
                    if substitution {
                        let inputs: Vec<_> = std::iter::once(subject.clone())
                            .chain(arguments.iter().cloned())
                            .collect();
                        value = interface
                            .input_substitution(&inputs, &[], &mut fuel)
                            .and_then(|substitution| {
                                substitution.instantiate_bounded(&value, &mut fuel)
                            })
                            .map_err(|e| error(span, e.to_string()))?;
                    }
                    crate::associated::charge_projection_copy(&value, &mut fuel)
                        .map_err(|message| error(span, message))?;
                    pending.push(Work::Finish(member, term, value.clone()));
                    pending.push(Work::Visit(value, depth + 1));
                }
                Term::AssociatedTypeCall(ref call)
                    if call.origin.interface == interface.declaration
                        && call.subject == *subject
                        && call.arguments == arguments =>
                {
                    let member = call.origin.member;
                    let contract = interface
                        .associated
                        .iter()
                        .find(|a| a.declaration == member)
                        .ok_or_else(|| error(span, "associated signature has an unknown member"))?;
                    let (declaration, substitution) =
                        if let Some(actual) = supplied.get(&contract.name) {
                            (actual.declaration, false)
                        } else {
                            (contract.declaration, true)
                        };
                    let owner = self.associated_owner(declaration).unwrap();
                    let default_callee = if !substitution
                        && !self
                            .declaration_or_enclosing_impl_type_parameters(owner)
                            .is_empty()
                    {
                        Some(
                            self.associated_default_template(owner, &contract.ty)?
                                .into_expression(&mut self.required_steps)
                                .map_err(|failure| error(span, failure))?,
                        )
                    } else {
                        self.associated_contract(owner, true)?.default
                    };
                    if let Some(mut callee) = default_callee {
                        constants::charge_copy(&callee, &mut self.required_steps, span)?;
                        if substitution {
                            let inputs: Vec<_> = std::iter::once(subject.clone())
                                .chain(arguments.iter().cloned())
                                .collect();
                            let binders = interface
                                .binders_with_self_checked(&mut fuel)
                                .map_err(|e| error(span, e.to_string()))?;
                            let binder_ids: Vec<_> =
                                binders.iter().map(|b| b.declaration).collect();
                            callee = callee
                                .instantiate_declaration_parameters_bounded(
                                    &binder_ids,
                                    &std::iter::once(p(P::Type))
                                        .chain(interface.type_parameters.iter().map(|id| {
                                            self.trait_input_domain(*id)
                                                .unwrap_or_else(|| p(P::Type))
                                        }))
                                        .collect::<Vec<_>>(),
                                    &inputs,
                                    &[],
                                    &mut fuel,
                                )
                                .map_err(|e| error(span, e.to_string()))?;
                        }
                        if let Some(resolved) = self.try_constant_associated_type_call(
                            callee,
                            &call.call_arguments,
                            span,
                        )? {
                            crate::associated::charge_projection_copy(&resolved, &mut fuel)
                                .map_err(|message| error(span, message))?;
                            equations.push((term, resolved));
                            continue;
                        }
                    }
                    pending.push(Work::Visit(call.subject.clone(), depth + 1));
                    pending.extend(
                        call.arguments
                            .iter()
                            .cloned()
                            .map(|t| Work::Visit(t, depth + 1)),
                    );
                    pending.extend(
                        call.call_arguments
                            .iter()
                            .cloned()
                            .map(|t| Work::Visit(t, depth + 1)),
                    );
                }
                Term::AssociatedTypeCall(call) => {
                    pending.push(Work::Visit(call.subject, depth + 1));
                    pending.extend(
                        call.arguments
                            .into_iter()
                            .map(|t| Work::Visit(t, depth + 1)),
                    );
                    pending.extend(
                        call.call_arguments
                            .into_iter()
                            .map(|t| Work::Visit(t, depth + 1)),
                    );
                }
                Term::AssociatedType {
                    arguments, subject, ..
                } => {
                    pending.push(Work::Visit(*subject, depth + 1));
                    pending.extend(arguments.into_iter().map(|t| Work::Visit(t, depth + 1)));
                }
                Term::Nominal { arguments, .. }
                | Term::NormalForm { arguments, .. }
                | Term::Tuple(arguments)
                | Term::TupleValue(arguments) => {
                    pending.extend(arguments.into_iter().map(|t| Work::Visit(t, depth + 1)))
                }
                Term::OptionalValue { ty, value } => {
                    pending.push(Work::Visit(*ty, depth + 1));
                    if let Some(value) = value {
                        pending.push(Work::Visit(*value, depth + 1));
                    }
                }
                Term::ErrorValue { ty, label, value } => {
                    pending.push(Work::Visit(*ty, depth + 1));
                    if let Some(label) = label {
                        pending.push(Work::Visit(*label, depth + 1));
                    }
                    pending.push(Work::Visit(*value, depth + 1));
                }
                Term::ListConcat { ty, parts: fields } | Term::NominalValue { ty, fields, .. } => {
                    pending.push(Work::Visit(*ty, depth + 1));
                    pending.extend(
                        fields
                            .into_iter()
                            .map(|field| Work::Visit(field, depth + 1)),
                    );
                }
                Term::Record(fields) | Term::RecordValue(fields) => {
                    pending.extend(fields.into_iter().map(|(_, t)| Work::Visit(t, depth + 1)))
                }
                Term::Pointer(inner)
                | Term::Lift(inner)
                | Term::ComptimeFunction(inner)
                | Term::NormalFormFunction(inner, _)
                | Term::UnsafeFunction(inner)
                | Term::PureFunction(inner) => pending.push(Work::Visit(*inner, depth + 1)),
                Term::Function { parameters, result } => {
                    pending.push(Work::Visit(*result, depth + 1));
                    pending.extend(parameters.into_iter().map(|p| Work::Visit(p.ty, depth + 1)));
                }
                Term::ErrorResult { errors, result } => {
                    pending.push(Work::Visit(*result, depth + 1));
                    pending.extend(
                        errors
                            .into_iter()
                            .map(|label| Work::Visit(label, depth + 1)),
                    );
                }
                Term::Computation { effects, result } => {
                    pending.push(Work::Visit(*result, depth + 1));
                    for operation in effects {
                        pending.extend(
                            operation
                                .arguments
                                .into_iter()
                                .map(|t| Work::Visit(t, depth + 1)),
                        );
                    }
                }
                Term::Closure {
                    environment,
                    signature,
                } => {
                    pending.push(Work::Visit(*signature, depth + 1));
                    pending.extend(environment.into_iter().map(|t| Work::Visit(t, depth + 1)));
                }
                _ => (),
            }
        }
        Ok(equations)
    }

    fn extract_constant_closure<'a>(expr: &'a n::Expr) -> Option<(&'a [n::LocalId], &'a n::Expr)> {
        match &expr.kind {
            K::Closure {
                parameters, body, ..
            } => Some((parameters.as_slice(), body.as_ref())),
            K::Block {
                statements,
                result: Some(inner),
            } if statements.is_empty() => Self::extract_constant_closure(inner),
            K::Call {
                callee, arguments, ..
            } if arguments.is_empty() => {
                let K::Closure {
                    parameters, body, ..
                } = &callee.kind
                else {
                    return None;
                };
                if !parameters.is_empty() {
                    return None;
                }
                let K::Block {
                    statements,
                    result: Some(ret),
                } = &body.kind
                else {
                    return None;
                };
                let K::Local(target) = ret.kind else {
                    return None;
                };
                statements.iter().rev().find_map(|stmt| match &stmt.kind {
                    K::Let { binding, value } if *binding == target => {
                        Self::extract_constant_closure(value)
                    }
                    _ => None,
                })
            }
            _ => None,
        }
    }

    fn constant_closure_ignores_param(&self, callee: &n::Expr, slot: usize) -> bool {
        let Some((closure_params, body)) = Self::extract_constant_closure(callee) else {
            return false;
        };
        let Some(param_id) = closure_params.get(slot) else {
            return false;
        };
        let param_decl = self
            .callable_type_inputs
            .get(param_id)
            .and_then(|entry| entry.as_ref().map(|staged| staged.declaration));
        let mut fuel = 10_000usize;
        let mut used = false;
        let visited = nir::visit::expression(body, &mut fuel, |expr, _| {
            if matches!(expr.kind, K::Local(id) if id == *param_id) {
                used = true;
            } else if let Some(decl) = param_decl {
                if expr.ty.structural_terms().any(|term| {
                    matches!(term, Term::Rigid { declaration, .. } if *declaration == decl)
                }) {
                    used = true;
                } else if let K::TypeValue(ty) | K::ComptimeValue(ty) = &expr.kind
                    && ty.structural_terms().any(|term| {
                        matches!(term, Term::Rigid { declaration, .. } if *declaration == decl)
                    })
                {
                    used = true;
                }
            }
            Ok(())
        });
        visited.is_ok() && !used
    }

    fn try_constant_associated_type_call(
        &mut self,
        callee: n::Expr,
        call_arguments: &[Term],
        span: Span,
    ) -> Result<Option<Term>, Error> {
        let Term::Function { parameters, result } = callee.ty.callable_signature().clone() else {
            return Ok(None);
        };
        if !callee.ty.is_comptime_callable()
            || parameters.len() != call_arguments.len()
            || *result != p(P::Type)
        {
            return Ok(None);
        }
        const ABSTRACT_CALL_ARG_PACKAGE: u64 = u64::MAX - 10;
        let mut expr_args = Vec::with_capacity(call_arguments.len());
        for (slot, (arg, param)) in call_arguments.iter().zip(&parameters).enumerate() {
            let value = if arg.check_scope(&[]).is_ok()
                && !nir::type_computations::has_rigid_parameters_bounded(arg, &mut 10_000)
                    .unwrap_or(true)
            {
                let kind = if param.ty == p(P::Type) {
                    K::TypeValue(arg.clone())
                } else {
                    K::ComptimeValue(arg.clone())
                };
                n::Expr {
                    source: source(span),
                    ty: param.ty.clone(),
                    kind,
                }
            } else if param.ty == p(P::Type) && self.constant_closure_ignores_param(&callee, slot) {
                n::Expr {
                    source: source(span),
                    ty: param.ty.clone(),
                    kind: K::TypeValue(p(P::Void)),
                }
            } else {
                return self.try_symbolic_associated_type_call(&callee, call_arguments, span);
            };
            expr_args.push(n::CallArgument {
                passing: n::ArgumentPassing::Value,
                parameter: slot,
                is_default: false,
                value,
            });
        }
        let dummy = Expr {
            hir_id: hir::HirId::new(
                self.signature_owner.unwrap_or(OwnerId::INVALID),
                hir::ItemLocalId::new(0),
            ),
            span,
            kind: H::Unit,
        };
        let call_expr = if callee.ty.is_normal_form_callable() {
            let input_ids = (0..expr_args.len())
                .map(|slot| DeclarationId {
                    package: u64::MAX - 11,
                    local: slot as u32,
                })
                .collect();
            let arguments = expr_args
                .into_iter()
                .map(|arg| n::NormalFormArgument::Value(arg.value))
                .collect();
            n::Expr {
                source: source(span),
                ty: p(P::Type),
                kind: K::ApplyNormalForm {
                    callee: std::sync::Arc::new(callee),
                    input_ids,
                    arguments,
                    plan: None,
                },
            }
        } else {
            n::Expr {
                source: source(span),
                ty: p(P::Type),
                kind: K::Call {
                    callee: Box::new(callee),
                    arguments: expr_args,
                    env_arguments: Vec::new(),
                    parameter_bindings: Vec::new(),
                },
            }
        };
        let evaluated = self.with_staged_input(true, |this| {
            this.required_normal_input(&dummy, call_expr, Vec::new())
        });
        let Ok((comptime_interpretor::Value::Type(resolved), _)) = evaluated else {
            return Ok(None);
        };
        let leaked_abstract = resolved.structural_terms().any(|term| {
            matches!(
                term,
                Term::Rigid { declaration, .. } if declaration.package == ABSTRACT_CALL_ARG_PACKAGE
            )
        });
        if leaked_abstract {
            return Ok(None);
        }
        Ok(Some(resolved))
    }

    fn try_symbolic_associated_type_call(
        &mut self,
        callee: &n::Expr,
        call_arguments: &[Term],
        span: Span,
    ) -> Result<Option<Term>, Error> {
        let Some((closure_params, body)) = Self::extract_constant_closure(callee) else {
            return Ok(None);
        };
        if closure_params.len() != call_arguments.len() {
            return Ok(None);
        }
        let mut facts = BTreeMap::new();
        let mut decl_ids = Vec::new();
        let mut decl_domains = Vec::new();
        let mut decl_values = Vec::new();
        for (&param_id, arg) in closure_params.iter().zip(call_arguments) {
            facts.insert(param_id, std::sync::Arc::new(arg.clone()));
            if let Some(Some(staged)) = self.callable_type_inputs.get(&param_id) {
                decl_ids.push(staged.declaration);
                decl_domains.push(staged.domain.clone());
                decl_values.push(arg.clone());
            }
        }
        let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let mut budget = comptime_runtime::Budget {
            steps: before,
            max_limbs: 4096,
        };
        let observed = super::super::normal::observation::optional(
            self.observe_preserved_input_in(body, facts, BTreeSet::new(), &mut budget),
        );
        self.required_steps -= (before - budget.steps) as u64;
        let Ok(Some(mut resolved)) = observed else {
            return Ok(None);
        };
        if !decl_ids.is_empty() {
            let frames = self.call_input_frames();
            let mut fuel = 10_000usize;
            let subst = nir::DeclarationSubstitution::bounded(
                &decl_ids,
                &decl_domains,
                &decl_values,
                &frames,
                &mut fuel,
            )
            .and_then(|s| s.instantiate_bounded(&resolved, &mut fuel));
            let Ok(instantiated) = subst else {
                return Ok(None);
            };
            resolved = instantiated;
        }
        if resolved.sort() != Sort::Type {
            return Ok(None);
        }
        Ok(Some(resolved))
    }

    /// Evaluate an associated Type call when the implementation target is closed
    /// and its body ignores any remaining rigid/bound call arguments.
    pub(in crate::program) fn try_constant_associated_call_term(
        &mut self,
        call: &nir::AssociatedTypeCall,
        span: Span,
    ) -> Result<Option<Term>, Error> {
        if !self.interfaces_ready {
            return Ok(None);
        }
        let closed_head = nominal_fields::metered(&mut self.required_steps, |fuel| {
            for input in std::iter::once(&call.subject).chain(&call.arguments) {
                if input.check_scope(&[]).is_err()
                    || nir::type_computations::has_rigid_parameters_bounded(input, fuel)?
                {
                    return Ok(false);
                }
            }
            Ok(true)
        })
        .map_err(|failure| error(span, failure.to_string()))?;
        if !closed_head {
            return Ok(None);
        }
        let interface = n::InterfaceInstance {
            declaration: call.origin.interface,
            arguments: call.arguments.clone(),
        };
        if matches!(
            self.symbolic_interface_evidence(&call.subject, &interface),
            Ok(true)
        ) {
            return Ok(None);
        }
        let resolution = self
            .resolve_trait_capability(&interface, &call.subject, Budget::default())
            .map_err(|message| error(span, message))?;
        let crate::implementation::Selection::Selected(selected) = resolution.selection else {
            return Ok(None);
        };
        let Some(crate::implementation::TypeAssessment::Applicable(evidence)) =
            resolution.assessments.get(&selected)
        else {
            return Ok(None);
        };
        let slot = *self
            .implementation_positions
            .get(&selected)
            .ok_or_else(|| error(span, "unknown associated implementation"))?;
        let implementation = &self.checked_implementations[slot];
        let type_params = implementation.type_parameters.clone();
        let param_types = implementation.parameter_types.clone();
        let evidence_args = evidence.type_arguments.clone();
        let callee = self.prepared_associated_value(selected, call.origin.member, span)?;
        let callee = nominal_fields::metered(&mut self.required_steps, |fuel| {
            callee.instantiate_declaration_parameters_bounded(
                &type_params,
                &param_types,
                &evidence_args,
                &[],
                fuel,
            )
        })
        .map_err(|e| error(span, e.to_string()))?;
        self.try_constant_associated_type_call(callee, &call.call_arguments, span)
    }
}

impl Checker<'_, '_> {
    /// Selection is local to checking an associated initializer, never a method
    /// body reached by comptime dependency discovery.
    fn initializer_selection(&self, member: DeclarationId) -> Option<DeclarationId> {
        let implementation = self.initializer_implementation()?;
        let interface = self.checked_traits.by_id(implementation.interface?)?;
        interface
            .associated
            .iter()
            .any(|a| a.declaration == member)
            .then_some(implementation.declaration)
    }

    pub(in crate::program) fn initializer_bindings(
        &self,
        owner: OwnerId,
    ) -> Option<(DeclarationId, &[DeclarationId], &[Term])> {
        let key = self.selected_associated_initializer?;
        let recipe = &self.associated_initializers[&key];
        (recipe.owner == owner).then_some((key.0, &recipe.parameters, &recipe.arguments))
    }

    pub(in crate::program) fn initializer_implementation(&self) -> Option<&n::Implementation> {
        let (selected, _, _) = self.initializer_bindings(self.signature_owner?)?;
        self.checked_implementations
            .iter()
            .find(|i| i.declaration == selected)
    }

    pub(in crate::program) fn initializer_substitution(
        &self,
    ) -> Option<(&[DeclarationId], &[Term])> {
        let (_, parameters, arguments) = self.initializer_bindings(self.signature_owner?)?;
        Some((parameters, arguments))
    }

    fn associated_default_template(
        &mut self,
        owner: OwnerId,
        expected: &Term,
    ) -> Result<nir::associated_initializers::AssociatedInitializer, Error> {
        let item = self.package.item(owner).unwrap();
        let ItemKind::Associated(associated) = &item.kind else {
            unreachable!()
        };
        let initializer = associated.default.unwrap();
        self.in_formal_declaration(owner, |this| {
            this.in_required_comptime = true;
            this.context = this.definition_context(owner);
            let expr = if !associated.parameters.is_empty() {
                let (param_ids, _, result_ty, req_predicates) =
                    this.bind_associated_parameters(associated, item.span)?;
                this.associated_closure_default(
                    &param_ids,
                    &result_ty,
                    expected,
                    req_predicates,
                    initializer,
                )?
            } else {
                let expr = this.expr(initializer, Some(expected))?;
                if *expected == p(P::Type)
                    && !matches!(expr.kind, K::TypeValue(_))
                    && let Some(term) =
                        this.preserved_input_identity(&expr, |_, _| Ok(None), initializer.span)?
                {
                    n::Expr {
                        ty: p(P::Type),
                        source: expr.source,
                        kind: K::TypeValue(term),
                    }
                } else {
                    expr
                }
            };
            let before = usize::try_from(this.required_steps).unwrap_or(usize::MAX);
            let mut fuel = before;
            let template =
                nir::associated_initializers::AssociatedInitializer::new(expr, &mut fuel);
            this.required_steps -= (before - fuel) as u64;
            template.map_err(|message| error(initializer.span, message))
        })
    }
}
