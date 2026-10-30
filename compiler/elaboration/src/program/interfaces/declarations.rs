//! Associated Type formation before method contracts are available. These
//! queries read declarations only; they do not create implementation evidence.
use super::*;
use std::collections::HashSet;

impl Checker<'_, '_> {
    /// Read a projected member's declared domain without selecting an
    /// implementation or evaluating its initializer. Contract classification
    /// must distinguish Type equality from equality of program values first.
    pub(in crate::program) fn contract_projection_domain(
        &mut self,
        value: &Expr<'_>,
    ) -> Result<Option<Term>, Error> {
        let H::Projection(view, field) = &value.kind else {
            return Ok(None);
        };
        let H::Cast(base, interface) = &view.kind else {
            return Ok(None);
        };
        if !self.is_type_head(base) {
            return Ok(None);
        }
        let subject = self.type_expr(base)?;
        let requested = self.interface_expr_for(interface, &subject)?;
        let mut sources = self.declaration_interface_sources(
            &subject,
            requested,
            Some(field.name.as_str()),
            value.span,
        )?;
        if sources.len() > 1 {
            return Err(error(
                value.span,
                "ambiguous inherited member; specify its interface source",
            ));
        }
        let Some((origin, member)) = sources.pop() else {
            return Ok(None);
        };
        if !matches!(
            self.package.item(member).unwrap().kind,
            ItemKind::Associated(_)
        ) {
            return Ok(None);
        }
        if !self.active_associated_signatures.insert(member) {
            return Err(error(value.span, "cyclic associated signature dependency"));
        }
        let contract = self.associated_contract(member, false);
        self.active_associated_signatures.remove(&member);
        let contract = contract?;
        let owner = self.associated_owner(origin.declaration).unwrap();
        let parameters: Vec<_> = std::iter::once(origin.declaration)
            .chain(self.type_parameters[&owner].iter().copied())
            .collect();
        let inputs: Vec<_> = std::iter::once(subject).chain(origin.arguments).collect();
        self.instantiate_input_types(&contract.ty, &parameters, &inputs, value.span)
            .map(Some)
    }

    fn declaration_interface_sources(
        &mut self,
        subject: &Term,
        root: n::InterfaceInstance,
        name: Option<&str>,
        span: Span,
    ) -> Result<Vec<(n::InterfaceInstance, OwnerId)>, Error> {
        let mut pending = vec![(root, 0usize)];
        let mut seen = HashSet::new();
        let mut found = Vec::new();
        let mut fuel = 10_000usize;
        while let Some((instance, depth)) = pending.pop() {
            if depth >= 64 || fuel == 0 {
                return Err(error(span, "declaration interface lookup budget exhausted"));
            }
            fuel -= 1;
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(span, "declaration interface lookup budget exhausted"))?;
            for argument in &instance.arguments {
                crate::associated::charge_projection_copy(argument, &mut fuel)
                    .map_err(|message| error(span, message))?;
            }
            if !seen.insert(instance.clone()) {
                continue;
            }
            let owner = self
                .associated_owner(instance.declaration)
                .ok_or_else(|| error(span, "unknown interface declaration"))?;
            let ItemKind::Trait(def) = &self.package.item(owner).unwrap().kind else {
                return Err(error(span, "interface declaration must be a trait"));
            };
            if let Some(name) = name {
                if let Some(member) = def
                    .items
                    .iter()
                    .find(|member| self.package.item(**member).unwrap().ident.name.as_str() == name)
                {
                    if !found
                        .iter()
                        .any(|(old, _)| self.same_interface(old, &instance))
                    {
                        found.push((instance, *member));
                    }
                    continue;
                }
            } else {
                found.push((instance.clone(), owner));
            }
            let parent_expr = def.super_traits;
            let saved_owner = self.signature_owner.replace(owner);
            let context = self.definition_context(owner);
            let saved_context = std::mem::replace(&mut self.context, context);
            let parents = parent_expr
                .map(|expr| self.parent_interfaces(expr))
                .transpose();
            self.signature_owner = saved_owner;
            self.context = saved_context;
            let parameters: Vec<_> = std::iter::once(instance.declaration)
                .chain(self.type_parameters[&owner].iter().copied())
                .collect();
            let inputs: Vec<_> = std::iter::once(subject.clone())
                .chain(instance.arguments)
                .collect();
            let frames = self.call_input_frames();
            let substitution = self
                .input_substitution(&parameters, &inputs, &frames)
                .map_err(|e| error(span, e.to_string()))?;
            for parent in parents?.unwrap_or_default() {
                let parent = substitution
                    .instantiate_interface_bounded(&parent, &mut fuel)
                    .map_err(|e| error(span, e.to_string()))?;
                for argument in &parent.arguments {
                    crate::associated::charge_projection_copy(argument, &mut fuel)
                        .map_err(|message| error(span, message))?;
                }
                pending.push((parent, depth + 1));
            }
        }
        Ok(found)
    }

    pub(super) fn declaration_associated_projection(
        &mut self,
        subject: Term,
        requested: n::InterfaceInstance,
        name: &str,
        span: Span,
    ) -> Result<Option<n::Expr>, Error> {
        if self.declaration_projection_depth >= 32 {
            return Err(error(
                span,
                "cyclic or excessively deep declaration projection",
            ));
        }
        self.declaration_projection_depth += 1;
        let result = self.declaration_associated_projection_inner(subject, requested, name, span);
        self.declaration_projection_depth -= 1;
        result
    }

    fn declaration_associated_projection_inner(
        &mut self,
        subject: Term,
        requested: n::InterfaceInstance,
        name: &str,
        span: Span,
    ) -> Result<Option<n::Expr>, Error> {
        let mut sources =
            self.declaration_interface_sources(&subject, requested.clone(), Some(name), span)?;
        if sources.len() > 1 {
            return Err(error(
                span,
                "ambiguous inherited member; specify its interface source",
            ));
        }
        let Some((origin, member)) = sources.pop() else {
            return Ok(None);
        };
        if !matches!(
            self.package.item(member).unwrap().kind,
            ItemKind::Associated(_)
        ) {
            return Ok(None);
        }
        let mut givens = self
            .signature_owner
            .and_then(|owner| self.capabilities.get(&owner))
            .cloned()
            .unwrap_or_default();
        if let Some(scope) = self.signature_owner {
            // Trait Self is a declaration assumption, including required methods.
            for (owner, info) in self.package.owners() {
                if let ItemKind::Trait(def) = &info.node.expect_item().kind {
                    if owner == scope || def.items.contains(&scope) {
                        let interface = declaration(self.package, &self.package_identities, owner);
                        givens.push(n::TraitRequirement {
                            subject: Term::Rigid {
                                declaration: interface,
                                sort: Sort::Type,
                            },
                            interface,
                            interface_arguments: self
                                .declaration_binders(owner)
                                .iter()
                                .map(|binder| Term::Rigid {
                                    declaration: binder.declaration,
                                    sort: binder.sort,
                                })
                                .collect(),
                        });
                    }
                }
            }
            // Within an implementation signature this permits naming its own
            // result only. Admission still checks definitions and all contracts;
            // no such assumption is published to selection or body checking.
            if let Some(parent) = self.self_containers.get(&scope).copied() {
                // Associated declarations inherit the implementation's clause
                // assumptions, just as its method signatures do.
                for given in self.capabilities.get(&parent).into_iter().flatten() {
                    if !givens.contains(given) {
                        givens.push(given.clone());
                    }
                }
                if let ItemKind::Impl(def) = &self.package.item(parent).unwrap().kind {
                    if let Some(interface) = def.trait_ref {
                        let own_subject = self.type_expr(def.self_ty)?;
                        let interface = self.interface_expr_for(interface, &own_subject)?;
                        givens.push(n::TraitRequirement {
                            subject: own_subject,
                            interface: interface.declaration,
                            interface_arguments: interface.arguments,
                        });
                    }
                }
            }
        }
        let mut established = false;
        for given in givens {
            if given.subject != subject
                && !matches!(
                    self.context.prove(
                        Formula::equal(given.subject.clone(), subject.clone()),
                        Budget::default()
                    ),
                    Outcome::Entailed(_)
                )
            {
                continue;
            }
            let root = n::InterfaceInstance {
                declaration: given.interface,
                arguments: given.interface_arguments,
            };
            if self
                .declaration_interface_sources(&subject, root, None, span)?
                .iter()
                .any(|(interface, _)| self.same_interface(interface, &requested))
            {
                established = true;
                break;
            }
        }
        if !self.active_associated_signatures.insert(member) {
            return Err(error(span, "cyclic associated signature dependency"));
        }
        let contract = self.associated_contract(member, false);
        self.active_associated_signatures.remove(&member);
        let contract = contract?;
        let origin_owner = self.associated_owner(origin.declaration).unwrap();
        let parameters: Vec<_> = std::iter::once(origin.declaration)
            .chain(self.type_parameters[&origin_owner].iter().copied())
            .collect();
        let inputs: Vec<_> = std::iter::once(subject.clone())
            .chain(origin.arguments.iter().cloned())
            .collect();
        let ty = self.instantiate_input_types(&contract.ty, &parameters, &inputs, span)?;
        let deferred = !established
            && ty != p(P::Type)
            && self.defer_contract_projection(
                n::TraitRequirement {
                    subject: subject.clone(),
                    interface: requested.declaration,
                    interface_arguments: requested.arguments,
                },
                span,
            )?;
        if !established && !deferred {
            return Err(Error {
                needs_capability: true,
                ..error(
                    span,
                    "declaration associated projection requires a given interface constraint",
                )
            });
        }
        if ty != p(P::Type) {
            return Ok(Some(n::Expr {
                ty,
                source: source(span),
                kind: K::AssociatedValue {
                    interface: origin,
                    member: contract.declaration,
                    subject,
                },
            }));
        }
        Ok(Some(n::Expr {
            ty,
            source: source(span),
            kind: K::TypeValue(Term::AssociatedType {
                origin: Box::new(nir::AssociatedTypeOrigin {
                    interface: origin.declaration,
                    member: contract.declaration,
                }),
                arguments: origin.arguments,
                subject: Box::new(subject),
            }),
        }))
    }
}
