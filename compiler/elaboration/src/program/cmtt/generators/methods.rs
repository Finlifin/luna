//! Code-method selection uses the ordinary implementation proofs. Value
//! receivers enter the comptime input pipeline before the remaining arguments.
use super::*;
use crate::ptc::Budget;
mod traits;

impl Checker<'_, '_> {
    pub(super) fn install_generator_captures(
        &mut self,
        definition: &Generator,
        span: Span,
    ) -> Result<(), Error> {
        self.alias_substitution.extend(
            definition
                .captures
                .iter()
                .map(|(id, value)| (*id, Arc::new(value.clone()))),
        );
        for (id, value) in &definition.captures {
            let domain = self.trait_input_domain(*id).unwrap_or_else(|| p(P::Type));
            let sort = if domain == p(P::Type) {
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
            let domain = self.substituted_trait_domain(&domain, span)?;
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
        let types = public
            .types
            .iter()
            .map(|(_, value)| value.clone())
            .collect::<Vec<_>>();
        if template.types.len() != types.len() || template.inputs.len() != public.inputs.len() {
            return Err(error(span, "selected code method signature arity mismatch"));
        }
        let definition = self.instantiate_code_generator(&template, &types, span)?;
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
                        Argument::LiveData(self.read_generator_cell(&cell, span)?)
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
                    Argument::Pattern(code)
                }
                (Input::Code(_), Input::Code(expected)) => {
                    let code = self
                        .code_values
                        .get(binding)
                        .cloned()
                        .ok_or_else(|| error(span, "missing trait code input"))?;
                    Argument::Code(self.ascribe_code(code, Some(expected), span)?)
                }
                _ => return Err(error(span, "selected code method input category mismatch")),
            };
            arguments.push(Some(argument));
        }
        self.generator_frame(target, |this| {
            this.run_code_generator(&definition, arguments)
        })
        .map(Some)
    }

    pub(super) fn generic_code_method_hint(&mut self, callee: &Expr<'_>) -> Option<OwnerId> {
        if !self.interfaces_ready {
            return None;
        }
        let head = match callee.kind {
            H::NFApplication(head, _) => head,
            _ => callee,
        };
        let H::Projection(base, name) = head.kind else {
            return None;
        };
        if let H::Cast(_, interface) = base.kind {
            let interface = match interface.kind {
                H::NFApplication(head, _) => head,
                _ => interface,
            };
            let H::ResolvedDef { def_id, .. } = interface.kind else {
                return None;
            };
            let owner = self.package.definition_owner(def_id)?;
            let id = declaration(self.package, &self.package_identities, owner);
            let mut pending = vec![id];
            let mut seen = BTreeSet::new();
            let mut members = BTreeSet::new();
            while let Some(id) = pending.pop() {
                self.required_steps = self.required_steps.checked_sub(1)?;
                if !seen.insert(id) {
                    continue;
                }
                let interface = self
                    .checked_traits
                    .iter()
                    .find(|item| item.declaration == id)?;
                if let Some(member) = interface
                    .methods
                    .iter()
                    .find(|member| member.name == name.name.as_str())
                {
                    members.insert(member.declaration);
                } else {
                    pending.extend(interface.parents.iter().map(|parent| parent.declaration));
                }
            }
            if members.len() != 1 {
                return None;
            }
            let member = *members.iter().next()?;
            return self.package.owners().find_map(|(owner, _)| {
                (matches_declaration(self.package, &self.package_identities, owner, member)
                    && self.is_code_generator(owner))
                .then_some(owner)
            });
        }
        let subject = if self.is_static_method_subject(base) {
            self.generator_static_type_hint(base, 0, None).ok()??
        } else {
            self.generator_intrinsic_input_hint(base, 0).ok()??
        };
        let resolution = self
            .resolve_inherent_capability(name.name.as_str(), &subject, Budget::default())
            .ok()?;
        let crate::implementation::Selection::Selected(selected) = resolution.types.selection
        else {
            return None;
        };
        let implementation = self
            .checked_implementations
            .iter()
            .find(|item| item.declaration == selected)?;
        let (_, body) = implementation
            .methods
            .iter()
            .find(|(member, _)| member == name.name.as_str())?;
        self.package.owners().find_map(|(owner, _)| {
            (matches_declaration(self.package, &self.package_identities, owner, *body)
                && self.is_code_generator(owner))
            .then_some(owner)
        })
    }

    // Classification only: do not invoke a type producer, defaults, or arguments.
    // Invocation below replays the selected implementation evidence and checks
    // access. In particular, a matching name in an unrelated impl is no hint.
    pub(super) fn static_code_method_hint(&self, callee: &Expr<'_>) -> Option<OwnerId> {
        if !self.interfaces_ready {
            return None;
        }
        let H::Projection(base, name) = callee.kind else {
            return None;
        };
        let H::ResolvedDef { def_id, .. } = base.kind else {
            return None;
        };
        let container = self.package.definition_owner(def_id)?;
        if !matches!(
            self.package.item(container)?.kind,
            ItemKind::Struct(_) | ItemKind::Enum(_)
        ) || !self.type_parameters.get(&container)?.is_empty()
        {
            return None;
        }
        let subject = self.nominal_self_type(container);
        let resolution = self
            .resolve_inherent_capability(name.name.as_str(), &subject, Budget::default())
            .ok()?;
        let crate::implementation::Selection::Selected(selected) = resolution.types.selection
        else {
            return None;
        };
        let implementation = self
            .checked_implementations
            .iter()
            .find(|item| item.declaration == selected)?;
        let (_, body) = implementation
            .methods
            .iter()
            .find(|(member, _)| member == name.name.as_str())?;
        self.package.owners().find_map(|(owner, _)| {
            (matches_declaration(self.package, &self.package_identities, owner, *body)
                && self.is_code_generator(owner))
            .then_some(owner)
        })
    }

    pub(super) fn with_code_method_callee<R>(
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
        {
            return self.with_trait_code_method_callee(
                callee, head, arguments, base, interface, name, invoke,
            );
        }
        // Resolution can already bind a nested nominal member directly. Its
        // identity is still checked against the admitted implementation world.
        let candidate = match head.kind {
            H::Projection(base, name) if self.is_static_method_subject(base) => {
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
            let subject = self.type_identity_view(&subject, head.span)?;
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
                if !self.is_code_generator(owner) {
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
        if self.generator_owner(callee).is_some_and(|owner| {
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
