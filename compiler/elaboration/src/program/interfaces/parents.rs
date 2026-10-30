use super::*;
use std::collections::HashSet;

impl Checker<'_, '_> {
    /// Classify an explicit method view by declaration identity, not by the
    /// spelling of a Type producer or the shape of its application syntax.
    pub(in crate::program) fn is_trait_view_target(&self, expr: &Expr<'_>) -> bool {
        let head = match expr.kind {
            H::NFApplication(head, _) => head,
            _ => expr,
        };
        let H::ResolvedDef { def_id, .. } = head.kind else {
            return false;
        };
        self.package
            .definition_owner(def_id)
            .and_then(|owner| self.package.item(owner))
            .is_some_and(|item| matches!(item.kind, ItemKind::Trait(_)))
    }

    pub(in crate::program) fn interface_expr_for(
        &mut self,
        expr: &Expr<'_>,
        subject: &Term,
    ) -> Result<n::InterfaceInstance, Error> {
        let (head, args) = match &expr.kind {
            H::NFApplication(head, args) => (*head, Some(*args)),
            _ => (expr, None),
        };
        let H::ResolvedDef { def_id, .. } = head.kind else {
            return Err(error(
                expr.span,
                "interface expression requires a fixed trait declaration",
            ));
        };
        let owner = self
            .package
            .definition_owner(def_id)
            .ok_or_else(|| error(expr.span, "interface has no declaration"))?;
        if !matches!(
            self.package.item(owner).map(|i| &i.kind),
            Some(ItemKind::Trait(_))
        ) {
            return Err(error(expr.span, "interface expression must be a trait"));
        }
        let arguments = self
            .type_clause_arguments(owner, args.unwrap_or_default(), Some(subject), expr.span)
            .map_err(|mut failure| {
                if failure.message == "type argument count mismatch" {
                    failure.message = "interface type argument count mismatch".into();
                }
                failure
            })?;
        self.check_type_application(owner, &arguments, expr.span)?;
        Ok(n::InterfaceInstance {
            declaration: declaration(self.package, &self.package_identities, owner),
            arguments,
        })
    }
    pub(crate) fn parent_interfaces(
        &mut self,
        expr: &Expr<'_>,
    ) -> Result<Vec<n::InterfaceInstance>, Error> {
        let owner = self
            .signature_owner
            .ok_or_else(|| error(expr.span, "parent interface requires declaration context"))?;
        let subject = self.self_type(owner, expr.span)?;
        self.parent_interfaces_for(expr, &subject)
    }
    pub(crate) fn parent_interfaces_for(
        &mut self,
        expr: &Expr<'_>,
        subject: &Term,
    ) -> Result<Vec<n::InterfaceInstance>, Error> {
        let mut pending = vec![expr];
        let mut seen = HashSet::new();
        let mut parents = Vec::new();
        let mut fuel = Budget::default().steps;
        while let Some(expr) = pending.pop() {
            fuel = fuel
                .checked_sub(1)
                .ok_or_else(|| error(expr.span, "parent interface budget exhausted"))?;
            if let H::Binary(BinOp::Add, left, right) = expr.kind {
                pending.push(right);
                pending.push(left);
            } else {
                let interface = self
                    .interface_expr_for(expr, subject)
                    .map_err(|mut failure| {
                        if failure.message == "interface expression must be a trait" {
                            failure.message = "parent interface must be a trait".into();
                        }
                        failure
                    })?;
                if seen.insert(interface.clone()) {
                    parents.push(interface);
                }
            }
        }
        Ok(parents)
    }
    pub(super) fn validate_parent_graph(
        &self,
        traits: &BTreeMap<DeclarationId, n::Trait>,
    ) -> Result<(), Error> {
        let mut fuel = Budget::default().steps;
        for (owner, info) in self.package.owners() {
            let id = declaration(self.package, &self.package_identities, owner);
            let Some(interface) = traits.get(&id) else {
                continue;
            };
            let span = info.node.expect_item().span;
            let mut pending: Vec<_> = interface.parents.iter().map(|p| p.declaration).collect();
            let mut seen = BTreeSet::new();
            while let Some(parent) = pending.pop() {
                fuel = fuel
                    .checked_sub(1)
                    .ok_or_else(|| error(span, "parent interface graph budget exhausted"))?;
                if parent == id {
                    return Err(error(
                        span,
                        "cyclic parent interface contracts are not yet implemented",
                    ));
                }
                if !seen.insert(parent) {
                    continue;
                }
                let parent = traits
                    .get(&parent)
                    .ok_or_else(|| error(span, "unknown parent interface"))?;
                pending.extend(parent.parents.iter().map(|p| p.declaration));
            }
        }
        Ok(())
    }

    pub(super) fn same_interface(
        &self,
        a: &n::InterfaceInstance,
        b: &n::InterfaceInstance,
    ) -> bool {
        a.declaration == b.declaration
            && a.arguments.len() == b.arguments.len()
            && a.arguments.iter().zip(&b.arguments).all(|(a, b)| {
                a == b
                    || matches!(
                        self.context
                            .prove(Formula::equal(a.clone(), b.clone()), Budget::default()),
                        Outcome::Entailed(_)
                    )
            })
    }
    pub(crate) fn interface_parameter_binders(
        &self,
        interface: &n::InterfaceInstance,
        span: Span,
    ) -> Result<Vec<nir::DeclarationParameter>, Error> {
        self.checked_traits
            .iter()
            .find(|t| t.declaration == interface.declaration)
            .ok_or_else(|| error(span, "unknown interface"))?
            .binders_with_self(&mut 100_000)
            .map_err(|e| error(span, e))
    }
    pub(crate) fn interface_substitution(
        &self,
        interface: &n::InterfaceInstance,
        subject: &Term,
    ) -> (Vec<DeclarationId>, Vec<Term>) {
        let def = self
            .checked_traits
            .iter()
            .find(|t| t.declaration == interface.declaration)
            .unwrap();
        (
            def.parameters_with_self(),
            std::iter::once(subject.clone())
                .chain(interface.arguments.iter().cloned())
                .collect(),
        )
    }
    pub(in crate::program) fn inherited_member_source(
        &self,
        subject: &Term,
        root: &n::InterfaceInstance,
        name: &str,
        span: Span,
    ) -> Result<Option<n::InterfaceInstance>, Error> {
        let mut sources = self.inherited_member_sources(subject, root, name, span)?;
        if sources.len() > 1 {
            return Err(error(
                span,
                "ambiguous inherited member; specify its interface source",
            ));
        }
        Ok(sources.pop())
    }

    fn inherited_member_sources(
        &self,
        subject: &Term,
        root: &n::InterfaceInstance,
        name: &str,
        span: Span,
    ) -> Result<Vec<n::InterfaceInstance>, Error> {
        let mut pending = vec![root.clone()];
        let frames = self.call_input_frames();
        let mut seen = HashSet::new();
        let mut sources = Vec::new();
        let mut fuel = Budget::default().steps;
        while let Some(instance) = pending.pop() {
            fuel = fuel
                .checked_sub(1)
                .ok_or_else(|| error(span, "member source search budget exhausted"))?;
            if !seen.insert(instance.clone()) {
                continue;
            }
            let interface = self
                .checked_traits
                .iter()
                .find(|t| t.declaration == instance.declaration)
                .ok_or_else(|| error(span, "unknown interface in member lookup"))?;
            if interface.methods.iter().any(|m| m.name == name)
                || interface.associated.iter().any(|a| a.name == name)
            {
                if !sources
                    .iter()
                    .any(|old| self.same_interface(old, &instance))
                {
                    sources.push(instance);
                }
            } else {
                let (_, arguments) = self.interface_substitution(&instance, subject);
                let substitution = interface
                    .input_substitution(&arguments, &frames, &mut fuel)
                    .map_err(|e| error(span, e.to_string()))?;
                for parent in &interface.parents {
                    pending.push(
                        substitution
                            .instantiate_interface_bounded(parent, &mut fuel)
                            .map_err(|e| error(span, e.to_string()))?,
                    );
                }
            }
        }
        Ok(sources)
    }
    pub(super) fn default_interface(&self) -> Option<n::InterfaceInstance> {
        self.signature_owner
            .and_then(|owner| {
                self.checked_traits.iter().find(|t| {
                    t.methods.iter().any(|m| {
                        m.default
                            == Some(declaration(self.package, &self.package_identities, owner))
                    }) || t.associated.iter().any(|a| {
                        a.declaration == declaration(self.package, &self.package_identities, owner)
                    })
                })
            })
            .map(|t| n::InterfaceInstance {
                declaration: t.declaration,
                arguments: t
                    .type_parameters
                    .iter()
                    .map(|id| Term::Rigid {
                        declaration: *id,
                        sort: if self
                            .trait_input_domain(*id)
                            .is_some_and(|ty| ty != p(P::Type))
                        {
                            Sort::ComptimeValue
                        } else {
                            Sort::Type
                        },
                    })
                    .collect(),
            })
    }
    pub(in crate::program) fn symbolic_interface_evidence(
        &self,
        subject: &Term,
        target: &n::InterfaceInstance,
    ) -> bool {
        let assumptions = self.capability_assumptions();
        let world = crate::capabilities::World {
            index: &self.implementation_index,
            implementations: &self.checked_implementations,
            traits: &self.checked_traits,
            assumptions: &assumptions,
            context: &self.context,
        };
        let requirement = n::TraitRequirement {
            subject: subject.clone(),
            interface: target.declaration,
            interface_arguments: target.arguments.clone(),
        };
        // An open definition has a complete sort frame but no complete Type
        // telescope. Context facts must be queried in that original frame.
        if self.context.scope() == self.call_input_frames().as_slice() {
            world
                .given_in_context(&requirement, Budget::default())
                .is_some()
        } else if self
            .call_inference_domains()
            .iter()
            .any(|(_, domain)| domain.is_none())
        {
            world
                .given_in_input_scope(&requirement, Budget::default(), &self.call_input_frames())
                .is_some()
        } else {
            world
                .given_in_scope(
                    &requirement,
                    Budget::default(),
                    &self.call_inference_scope(),
                )
                .is_some()
        }
    }
    pub(super) fn comptime_member_context(&self) -> bool {
        self.in_required_comptime
            || self.in_staged_input
            || (!self.in_code_body
                && self
                    .captures
                    .last()
                    .map(|frame| frame.comptime)
                    .unwrap_or_else(|| {
                        self.signature_owner
                            .and_then(|owner| self.package.item(owner))
                            .is_some_and(|item| {
                                matches!(&item.kind,
                        ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)
                        if sig.modifiers.is_comptime || sig.modifiers.is_normal_form_head)
                            })
                    }))
    }

    pub(super) fn context_member_interface(
        &self,
        subject: &Term,
        name: &str,
        span: Span,
    ) -> Result<Option<n::InterfaceInstance>, Error> {
        let mut sources = Vec::new();
        let mut pending_sources = Vec::new();
        let mut inherited_ambiguities = Vec::new();
        let frames = self.call_input_frames();
        for capability in crate::capabilities::capability_sources_with_layouts(
            &self.checked_traits,
            &self.capability_assumptions(),
            Budget::default(),
            &frames,
            self.implementation_index.parameter_domains(),
            Some(self.implementation_index.nominal_layouts()),
        )
        .map_err(|message| error(span, message))?
        {
            let proven = if capability.subject == *subject {
                true
            } else {
                match self.context.prove(
                    Formula::equal(capability.subject.clone(), subject.clone()),
                    Budget::default(),
                ) {
                    Outcome::Entailed(_) => true,
                    Outcome::NotEntailed(_) => continue,
                    Outcome::Unknown { .. } => false,
                    Outcome::IllFormed(message) => return Err(error(span, message)),
                }
            };
            let inherited = self.inherited_member_sources(
                subject,
                &capability.interface_instance(),
                name,
                span,
            )?;
            if proven && inherited.len() > 1 {
                inherited_ambiguities.push(inherited.clone());
            }
            for source in inherited {
                let target = if proven {
                    &mut sources
                } else {
                    &mut pending_sources
                };
                if !target.iter().any(|old| self.same_interface(old, &source)) {
                    target.push(source);
                }
            }
        }
        let comptime_only = |source: &n::InterfaceInstance| {
            self.checked_traits.iter().any(|contract| {
                contract.declaration == source.declaration && contract.comptime_only
            })
        };
        if self.comptime_member_context() {
            if sources.iter().chain(&pending_sources).any(comptime_only) {
                sources.retain(comptime_only);
                pending_sources.retain(comptime_only);
            }
        } else {
            sources.retain(|source| !comptime_only(source));
            pending_sources.retain(|source| !comptime_only(source));
        }
        if !pending_sources.is_empty() {
            return Err(error(
                span,
                "capability member source remains unresolved; cannot select a fallback",
            ));
        }
        if sources.len() > 1 {
            return Err(error(
                span,
                if inherited_ambiguities.iter().any(|group| {
                    sources
                        .iter()
                        .filter(|source| group.iter().any(|old| self.same_interface(old, source)))
                        .count()
                        > 1
                }) {
                    "ambiguous inherited member; specify its interface source"
                } else {
                    "ambiguous capability member; specify its interface source"
                },
            ));
        }
        Ok(sources.pop())
    }
}
