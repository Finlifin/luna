//! Associated signatures are admitted before their values are demanded.
//! Only completed values enter published implementation tables.
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
        if !associated.parameters.is_empty()
            || !associated.clause_params.is_empty()
            || !associated.clause_constraints.is_empty()
        {
            return Err(error(
                item.span,
                "parameterized associated values require dependent member elaboration not yet implemented",
            ));
        }
        self.in_formal_declaration(owner, |this| {
            this.context = this.definition_context(owner);
            let ty = this.type_expr(associated.ty)?;
            let default = if let Some(initializer) = associated.default.filter(|_| evaluate_default)
            {
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
            Ok(n::AssociatedContract {
                phase: this.member_phases.get(&owner).copied().unwrap_or_default(),
                declaration: declaration(this.package, &this.package_identities, owner),
                name: item.ident.name.to_string(),
                ty,
                default,
            })
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
            // selects metadata only; it does not evaluate a receiver or body.
            if self
                .selected_method(subject.clone(), None, name, span)?
                .is_some()
            {
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

        if interface != requested
            && !self
                .symbolic_interface_evidence(&subject, &requested)
                .map_err(|e| capability_query_error(span, e))?
        {
            let resolution = self
                .resolve_trait_capability(&requested, &subject, Budget::default())
                .map_err(|message| error(span, message))?;
            if !matches!(
                resolution.selection,
                crate::implementation::Selection::Selected(_)
            ) {
                return Err(error(span, "requested interface view is not established"));
            }
        }
        if self
            .symbolic_interface_evidence(&subject, &interface)
            .map_err(|e| capability_query_error(span, e))?
        {
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
        if std::iter::once(&subject)
            .chain(&interface.arguments)
            .any(|ty| {
                ty.substitution_parameters()
                    .is_ok_and(|slots| !slots.is_empty())
            })
        {
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
                if contract.ty != p(P::Type) {
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
                                (None, Some(definition)) => Some(
                                    nir::normal_forms::NominalValueDeclaration::Enum(definition),
                                ),
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
                        // Reconstruction cannot provide this value. Preserve the
                        // checked, specialized program for selection and execution.
                        Err(nir::NominalAdmissionError::Invalid(_)) => {
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
            if contract.ty == p(P::Type) {
                contract = self.associated_contract(owner, true)?;
            } else if self.associated_has_initializer(member) {
                let template = self.associated_default_template(owner, &contract.ty)?;
                let span = self.package.item(owner).unwrap().span;
                let used = nir::copy_budget::expression_declaration_inputs(
                    template.expression(),
                    &mut self.required_steps,
                )
                .map_err(|failure| {
                    error(
                        span,
                        format!("associated default input observation failed: {failure:?}"),
                    )
                })?;
                let dependent = self
                    .checked_traits
                    .by_id(interface)
                    .unwrap()
                    .parameters_with_self()
                    .iter()
                    .any(|id| used.contains(id));
                let mut member_read = false;
                if !dependent {
                    let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                    let mut fuel = before;
                    let observed =
                        iir::visit::expression(template.expression(), &mut fuel, |expr, _| {
                            member_read |= matches!(expr.kind, K::AssociatedValue { .. });
                            Ok(())
                        });
                    self.required_steps -= (before - fuel) as u64;
                    observed.map_err(|failure| error(span, failure))?;
                }
                contract.default = if dependent || member_read {
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
        if !self.self_containers.contains_key(&owner) {
            if let Some(value) = self.abstract_associated_type(owner, span)? {
                return Ok(value);
            }
        }
        let parent = self.self_containers.get(&owner).copied()
            .ok_or_else(|| error(span, if self.self_types.contains_key(&owner) {
                "abstract associated values require symbolic member elaboration not yet implemented"
            } else { "associated value has no interface container" }))?;
        if !matches!(
            self.package.item(parent).map(|item| &item.kind),
            Some(ItemKind::Impl(_))
        ) {
            return Err(error(
                span,
                "abstract associated values require symbolic member elaboration not yet implemented",
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
                    return Err(error(
                        span,
                        "associated value signature requires the interface world to be established",
                    ));
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
        if self
            .signature_owner
            .and_then(|current| self.self_types.get(&current))
            != Some(&subject)
        {
            return Err(error(
                span,
                "abstract associated type requires its enclosing trait context",
            ));
        }
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
        let arguments = self.type_parameters[&parent]
            .iter()
            .map(|parameter| Term::Rigid {
                declaration: *parameter,
                sort: Sort::Type,
            })
            .collect();
        if ty != p(P::Type) {
            return Ok(Some(n::Expr {
                ty,
                source: source(span),
                kind: K::AssociatedValue {
                    interface: n::InterfaceInstance {
                        declaration: declaration(self.package, &self.package_identities, parent),
                        arguments,
                    },
                    member: declaration(self.package, &self.package_identities, owner),
                    subject,
                },
            }));
        }
        Ok(Some(n::Expr {
            ty: p(P::Type),
            source: source(span),
            kind: K::TypeValue(Term::AssociatedType {
                origin: Box::new(nir::AssociatedTypeOrigin {
                    interface: declaration(self.package, &self.package_identities, parent),
                    member: declaration(self.package, &self.package_identities, owner),
                }),
                arguments,
                subject: Box::new(subject),
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
                | Term::ComptimeFunction(inner)
                | Term::NormalFormFunction(inner, _)
                | Term::UnsafeFunction(inner) => pending.push(Work::Visit(*inner, depth + 1)),
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
            let expr = this.expr(initializer, Some(expected))?;
            let before = usize::try_from(this.required_steps).unwrap_or(usize::MAX);
            let mut fuel = before;
            let template =
                nir::associated_initializers::AssociatedInitializer::new(expr, &mut fuel);
            this.required_steps -= (before - fuel) as u64;
            template.map_err(|message| error(initializer.span, message))
        })
    }
}
