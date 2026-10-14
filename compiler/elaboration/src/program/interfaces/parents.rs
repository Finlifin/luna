use super::*;
use std::collections::HashSet;

impl Checker<'_, '_> {
    pub(super) fn interface_expr_for(
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
        let arguments =
            self.type_clause_arguments(owner, args.unwrap_or_default(), Some(subject), expr.span)
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
    pub(super) fn inherited_member_source(
        &self,
        subject: &Term,
        root: &n::InterfaceInstance,
        name: &str,
        span: Span,
    ) -> Result<Option<n::InterfaceInstance>, Error> {
        let mut pending = vec![root.clone()];
        let scope = self.call_inference_scope();
        let frames = if scope.is_empty() { vec![] } else {
            vec![scope.iter().map(|parameter| parameter.sort).collect()]
        };
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
                let (parameters, arguments) = self.interface_substitution(&instance, subject);
                let parameters = parameters.into_iter().map(|declaration|
                    nir::DeclarationParameter { declaration, sort: Sort::Type }).collect::<Vec<_>>();
                for parent in &interface.parents {
                    pending.push(
                        parent
                            .instantiate_parameters_in_scope(&parameters, &arguments, &frames)
                            .map_err(|e| error(span, e.to_string()))?,
                    );
                }
            }
        }
        if sources.len() > 1 {
            return Err(error(
                span,
                "ambiguous inherited member; specify its interface source",
            ));
        }
        Ok(sources.pop())
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
                        sort: Sort::Type,
                    })
                    .collect(),
            })
    }
    pub(super) fn symbolic_interface_evidence(
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
            subject: subject.clone(), interface: target.declaration,
            interface_arguments: target.arguments.clone(),
        };
        // An open definition has a complete sort frame but no complete Type
        // telescope. Context facts must be queried in that original frame.
        if self.context.scope() == self.call_input_frames().as_slice() {
            world.given_in_context(&requirement, Budget::default()).is_some()
        } else {
            world.given_in_scope(&requirement, Budget::default(), &self.call_inference_scope()).is_some()
        }
    }
    pub(super) fn context_member_interface(
        &self,
        subject: &Term,
        name: &str,
        span: Span,
    ) -> Result<Option<n::InterfaceInstance>, Error> {
        let mut sources = Vec::new();
        let scope = self.call_inference_scope();
        let frames = if scope.is_empty() { vec![] } else {
            vec![scope.iter().map(|parameter| parameter.sort).collect()]
        };
        for capability in crate::capabilities::capability_sources_in_scope(
            &self.checked_traits,
            &self.capability_assumptions(),
            Budget::default(),
            &frames,
        )
        .map_err(|message| error(span, message))?
        {
            if capability.subject != *subject
                && !matches!(
                    self.context.prove(
                        Formula::equal(capability.subject.clone(), subject.clone()),
                        Budget::default()
                    ),
                    Outcome::Entailed(_)
                )
            {
                continue;
            }
            if let Some(source) =
                self.inherited_member_source(subject, &capability.interface_instance(), name, span)?
            {
                if !sources.iter().any(|old| self.same_interface(old, &source)) {
                    sources.push(source);
                }
            }
        }
        if sources.len() > 1 {
            return Err(error(
                span,
                "ambiguous capability member; specify its interface source",
            ));
        }
        Ok(sources.pop())
    }
}
