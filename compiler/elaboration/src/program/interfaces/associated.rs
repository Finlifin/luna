//! Associated signatures are admitted before their values are demanded.
//! Only completed values enter published implementation tables.
use super::*;

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
        let saved = self.signature_owner.replace(owner);
        let result = (|| {
            let ty = self.type_expr(associated.ty)?;
            let default = if let Some(initializer) = associated.default.filter(|_| evaluate_default)
            {
                // A definition-side Type value may contain its stable Self or
                // interface binders. Form the template without interpreting it
                // as a closed runtime type; selection substitutes those binders.
                if ty == p(P::Type) {
                    Some(n::Expr {
                        ty: ty.clone(),
                        source: source(initializer.span),
                        kind: K::TypeValue(self.type_expr(initializer)?),
                    })
                } else {
                    Some(self.constant_value(owner, item.span)?)
                }
            } else {
                None
            };
            Ok(n::AssociatedContract {
                phase: self.member_phases.get(&owner).copied().unwrap_or_default(),
                declaration: declaration(self.package, &self.package_identities, owner),
                name: item.ident.name.to_string(),
                ty,
                default,
            })
        })();
        self.signature_owner = saved;
        result
    }
}

impl Checker<'_, '_> {
    pub(in crate::program) fn associated_projection(
        &mut self,
        view: &Expr<'_>,
        name: &str,
        span: Span,
    ) -> Result<Option<n::Expr>, Error> {
        let H::Cast(base, interface) = view.kind else {
            return Ok(None);
        };
        let mut subject = if self.is_type_head(base)
            || self.local_field_domain(base)? == Some(p(P::Type))
            || matches!(&base.kind, H::ResolvedFnParam(_, id) if self.type_input_bindings.contains_key(&local(*id)))
        {
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
        let mut requested = self.interface_expr_for(interface, &subject)?;
        // The declaration is checked separately under its abstract binders.
        // While evaluating a selected initializer, queries use its concrete
        // (or implementation-generic) Self and interface arguments.
        if let Some((parameters, arguments)) = self.initializer_substitution() {
            requested = requested
                .instantiate_parameters(&self.input_binders(parameters), arguments)
                .map_err(|e| error(span, e.to_string()))?;
            subject = subject
                .instantiate_parameters(&self.input_binders(parameters), arguments)
                .map_err(|e| error(span, e.to_string()))?;
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
            .iter()
            .find(|t| t.declaration == interface.declaration)
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
        if let Some(implementation) =
            self.initializer_projection_selection(&interface, &subject, member)
        {
            let mut value = self.selected_associated_value(implementation, member, span)?;
            value.source = source(span);
            return Ok(Some(value));
        }

        if interface != requested && !self.symbolic_interface_evidence(&subject, &requested) {
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
        if self.symbolic_interface_evidence(&subject, &interface) {
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
                needs_capability: true,
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
                needs_capability: true,
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
        let value = self.selected_associated_value(selected, member, span)?;
        constants::charge_copy(&value, &mut self.required_steps, span)?;
        let implementation = self
            .checked_implementations
            .iter()
            .find(|i| i.declaration == selected)
            .unwrap();
        let mut value = value
            .instantiate_parameters(
                &implementation
                    .parameter_binders()
                    .map_err(|e| error(span, e.to_string()))?,
                &evidence.type_arguments,
            )
            .map_err(|e| error(span, e.to_string()))?;
        if matches!(value.ty, Term::Primitive(_))
            && value.ty != p(P::Type)
            && self.initializer_implementation().is_none()
        {
            // Preserve observation identity even for a literal initializer: a
            // caller's checked assumes must describe the same constant as a
            // deferred method contract. Materialization evaluates the reference.
            let (parameters, inputs) = self.interface_substitution(&interface, &subject);
            let ty = self.instantiate_input_types(&member_type, &parameters, &inputs, span)?;
            let reference = n::Expr {
                ty,
                source: source(span),
                kind: K::AssociatedValue {
                    interface,
                    member,
                    subject,
                },
            };
            // As for method values, expose the proved concrete type through
            // an outer view while retaining the declaration type on the
            // observation node itself for phase validation.
            return Ok(Some(self.callable_value_view(reference, None, span)?));
        }
        value.source = source(span);
        Ok(Some(value))
    }
}

impl Checker<'_, '_> {
    pub(in crate::program) fn associated_owner(&self, member: DeclarationId) -> Option<OwnerId> {
        self.package.owners().find_map(|(owner, _)| {
            (matches_declaration(self.package, &self.package_identities, owner, member))
                .then_some(owner)
        })
    }

    pub(in crate::program) fn associated_has_initializer(&self, member: DeclarationId) -> bool {
        self.associated_owner(member)
            .and_then(|owner| self.package.item(owner))
            .is_some_and(
                |item| matches!(&item.kind, ItemKind::Associated(value) if value.default.is_some()),
            )
    }

    pub(in crate::program) fn selected_associated_value(
        &mut self,
        implementation: DeclarationId,
        member: DeclarationId,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let selected = self
            .checked_implementations
            .iter()
            .find(|item| item.declaration == implementation)
            .ok_or_else(|| error(span, "unknown associated implementation"))?;
        if !selected.phases.comptime
            && self
                .checked_traits
                .iter()
                .find(|interface| Some(interface.declaration) == selected.interface)
                .and_then(|interface| {
                    interface
                        .associated
                        .iter()
                        .find(|item| item.declaration == member)
                })
                .is_some_and(|member| member.phase == n::TraitPhase::Comptime)
        {
            return Err(error(
                span,
                "selected implementation lacks comptime trait phase",
            ));
        }
        if let Some(value) = self
            .checked_implementations
            .iter()
            .find(|i| i.declaration == implementation)
            .and_then(|i| i.associated.iter().find(|(id, _)| *id == member))
        {
            constants::charge_copy(&value.1, &mut self.required_steps, span)?;
            return Ok(value.1.clone());
        }
        if let Some(value) = self.associated_initializer_reference(implementation, member, span) {
            return Ok(value);
        }
        let key = (implementation, member);
        let (owner, parameters, arguments) = self
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
        let saved_selection = self
            .selected_associated_implementation
            .replace(implementation);
        let mut deferred = None;
        let result = (|| {
            let generic = self
                .checked_implementations
                .iter()
                .find(|item| item.declaration == implementation)
                .is_some_and(|item| !item.type_parameters.is_empty());
            if generic {
                let contract = self.associated_contract(owner, false)?;
                if contract.ty != p(P::Type) {
                    let template = self.associated_default_template(owner, &contract.ty)?;
                    let mut fuel = self.required_steps.min(100_000) as usize;
                    let before = fuel;
                    let specialized = template.instantiate(
                        &self.input_binders(&parameters),
                        &arguments,
                        &mut fuel,
                    );
                    self.required_steps -= (before - fuel) as u64;
                    let template = specialized.map_err(|e| error(span, e))?.into_expression();
                    let before = fuel;
                    let completed = nir::completed_values::validate_with_declarations(
                        &template,
                        self.newtypes.values(),
                        &|head| self.protocols.tuple == Some(head),
                        &|id, fuel| {
                            *fuel = fuel.checked_sub(1)?;
                            match (self.structs.get(&id), self.enums.get(&id)) {
                                (Some(definition), None) => Some(
                                    nir::normal_forms::NominalValueDeclaration::Struct(definition),
                                ),
                                (None, Some(definition)) => Some(
                                    nir::normal_forms::NominalValueDeclaration::Enum(definition),
                                ),
                                _ => None,
                            }
                        },
                        &mut fuel,
                    );
                    self.required_steps -= (before - fuel) as u64;
                    if completed.is_ok() {
                        return Ok(template);
                    }
                    constants::charge_copy(&template, &mut self.required_steps, span)?;
                    fuel = self.required_steps.min(100_000) as usize;
                    let before = fuel;
                    let initializer = nir::associated_initializers::AssociatedInitializer::new(
                        template.clone(),
                        &mut fuel,
                    );
                    self.required_steps -= (before - fuel) as u64;
                    let initializer = initializer.map_err(|e| error(span, e))?;
                    deferred = Some(initializer);
                    return Ok(template);
                }
            }
            let value = self
                .associated_contract(owner, true)?
                .default
                .ok_or_else(|| error(span, "associated value initializer is missing"))?;
            constants::charge_copy(&value, &mut self.required_steps, span)?;
            value
                .instantiate_parameters(&self.input_binders(&parameters), &arguments)
                .map_err(|e| error(span, e.to_string()))
        })();
        self.selected_associated_implementation = saved_selection;
        self.active_associated_values.remove(&key);
        if let Some(initializer) = deferred {
            self.checked_implementations
                .iter_mut()
                .find(|item| item.declaration == implementation)
                .unwrap()
                .associated_initializers
                .push((member, initializer));
            return Ok(self
                .associated_initializer_reference(implementation, member, span)
                .unwrap());
        }
        if let Ok(value) = &result {
            constants::charge_copy(value, &mut self.required_steps, span)?;
            self.checked_implementations
                .iter_mut()
                .find(|i| i.declaration == implementation)
                .unwrap()
                .associated
                .push((member, value.clone()));
        }
        result
    }

    fn associated_initializer_reference(
        &self,
        implementation: DeclarationId,
        member: DeclarationId,
        span: Span,
    ) -> Option<n::Expr> {
        let selected = self
            .checked_implementations
            .iter()
            .find(|item| item.declaration == implementation)?;
        let initializer = selected
            .associated_initializers
            .iter()
            .find(|(id, _)| *id == member)?;
        Some(n::Expr {
            ty: initializer.1.expression().ty.clone(),
            source: source(span),
            kind: K::AssociatedValue {
                interface: selected.interface_instance()?,
                subject: selected.subject.clone(),
                member,
            },
        })
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
            let owner = self.associated_initializers[&(implementation, member)].0;
            self.selected_associated_value(
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
                let mut template = self
                    .associated_default_template(owner, &contract.ty)?
                    .into_expression();
                let mut dependent = false;
                let binders = self
                    .checked_traits
                    .iter()
                    .find(|t| t.declaration == interface)
                    .unwrap()
                    .parameters_with_self();
                template
                    .rewrite_types(&mut |ty| {
                        dependent |= binders.iter().any(|binder| ty.mentions_rigid(*binder));
                        Ok::<_, String>(ty.clone())
                    })
                    .map_err(|message| error(self.package.item(owner).unwrap().span, message))?;
                iir::visit::expression(&template, &mut 100_000, |expr, _| {
                    if matches!(expr.kind, K::AssociatedValue { .. }) {
                        dependent = true;
                    }
                    Ok(())
                })
                .map_err(|message| error(self.package.item(owner).unwrap().span, message))?;
                contract.default = if dependent {
                    Some(template)
                } else {
                    self.associated_contract(owner, true)?.default
                };
            }
            *self
                .checked_traits
                .iter_mut()
                .find(|t| t.declaration == interface)
                .unwrap()
                .associated
                .iter_mut()
                .find(|a| a.declaration == member)
                .unwrap() = contract;
        }
        // Demand order is not declaration order: normalize the published table.
        for implementation in &mut self.checked_implementations {
            if let Some(interface) = implementation.interface {
                let members = &self
                    .checked_traits
                    .iter()
                    .find(|t| t.declaration == interface)
                    .unwrap()
                    .associated;
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
        self.checked_traits
            .iter()
            .find(|t| t.declaration == interface)
            .is_some_and(|t| {
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
            .iter()
            .find(|t| t.declaration == interface)
            .and_then(|t| t.associated.iter().find(|a| a.name == name))
            .map(|a| a.declaration)
            .ok_or_else(|| error(span, "associated value has no originating interface member"))?;
        let mut value = self.selected_associated_value(implementation, member, span)?;
        if original != implementation
            && self.selected_associated_implementation != Some(implementation)
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
                        value = value
                            .instantiate_parameters(
                                &interface
                                    .binders_with_self(&mut fuel)
                                    .map_err(|message| error(span, message))?,
                                &inputs,
                            )
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
                | Term::TypeListValue(arguments)
                | Term::TypeListConcat(arguments)
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
                Term::NominalValue { ty, fields, .. } => {
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
        let selected = self.selected_associated_implementation?;
        let owner = self.signature_owner?;
        if !matches!(
            self.package.item(owner).map(|item| &item.kind),
            Some(ItemKind::Associated(_))
        ) {
            return None;
        }
        let implementation = self
            .checked_implementations
            .iter()
            .find(|i| i.declaration == selected)?;
        let interface = self
            .checked_traits
            .iter()
            .find(|t| Some(t.declaration) == implementation.interface)?;
        interface
            .associated
            .iter()
            .any(|a| a.declaration == member)
            .then_some(selected)
    }

    pub(in crate::program) fn initializer_implementation(&self) -> Option<&n::Implementation> {
        self.initializer_substitution()?;
        let selected = self.selected_associated_implementation?;
        self.checked_implementations
            .iter()
            .find(|i| i.declaration == selected)
    }

    pub(in crate::program) fn initializer_substitution(
        &self,
    ) -> Option<(&[DeclarationId], &[Term])> {
        let selected = self.selected_associated_implementation?;
        let owner = self.signature_owner?;
        let (_, parameters, arguments) = self
            .associated_initializers
            .iter()
            .find(|((implementation, _), (initializer, _, _))| {
                *implementation == selected && *initializer == owner
            })?
            .1;
        Some((parameters, arguments))
    }

    fn initializer_projection_selection(
        &self,
        interface: &n::InterfaceInstance,
        subject: &Term,
        member: DeclarationId,
    ) -> Option<DeclarationId> {
        let selected = self.initializer_selection(member)?;
        let owner = self.signature_owner?;
        let (_, parameters, arguments) = self
            .associated_initializers
            .iter()
            .find(|((implementation, _), (initializer, _, _))| {
                *implementation == selected && *initializer == owner
            })?
            .1;
        let implementation = self
            .checked_implementations
            .iter()
            .find(|i| i.declaration == selected)?;
        let subject = subject
            .instantiate_parameters(&self.input_binders(parameters), arguments)
            .ok()?;
        let interface = interface
            .instantiate_parameters(&self.input_binders(parameters), arguments)
            .ok()?;
        (subject == implementation.subject
            && Some(interface) == implementation.interface_instance())
        .then_some(selected)
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
        let saved_owner = self.signature_owner.replace(owner);
        let saved_context = self.context.clone();
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_checking = std::mem::replace(&mut self.checking_contract, false);
        let saved_mode = std::mem::replace(&mut self.in_required_comptime, true);
        let saved_numeric = std::mem::replace(&mut self.numeric_defaults, true);
        let saved_result = self.result.take();
        let saved_partial_return = self.partial_return_context.take();
        let saved_optional_returns = std::mem::take(&mut self.pending_optional_returns);
        let saved_returns = self.return_observations.take();
        let saved_loops = std::mem::take(&mut self.loops);
        let saved_captures = std::mem::take(&mut self.captures);
        self.context = self.definition_context(owner);
        let result = self.expr(initializer, Some(expected)).and_then(|expr| {
            let mut fuel = self.required_steps.min(100_000) as usize;
            let before = fuel;
            let template =
                nir::associated_initializers::AssociatedInitializer::new(expr, &mut fuel);
            self.required_steps -= (before - fuel) as u64;
            template.map_err(|message| error(initializer.span, message))
        });
        self.signature_owner = saved_owner;
        self.context = saved_context;
        self.locals = saved_locals;
        self.checking_contract = saved_checking;
        self.in_required_comptime = saved_mode;
        self.numeric_defaults = saved_numeric;
        self.result = saved_result;
        self.partial_return_context = saved_partial_return;
        self.pending_optional_returns = saved_optional_returns;
        self.return_observations = saved_returns;
        self.loops = saved_loops;
        self.captures = saved_captures;
        result
    }
}
