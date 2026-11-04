//! Observe contextual-code categories without invoking producers or arguments.
//! Failure to finish an observation must not select ordinary value elaboration.
use super::*;
use crate::ptc::Budget;

#[derive(Clone, Copy)]
enum QuotedKind {
    Expression,
    Pattern,
}
impl QuotedKind {
    fn annotation(self, value: &Expr<'_>) -> bool {
        matches!(
            (self, &value.kind),
            (Self::Expression, H::ContextualExprType(..))
                | (Self::Pattern, H::ContextualPatternType(..))
        )
    }
    fn owner(self, checker: &Checker<'_, '_>, owner: OwnerId) -> bool {
        match self {
            Self::Expression => checker.is_code_generator(owner),
            Self::Pattern => checker.is_pattern_generator(owner),
        }
    }
}

impl Checker<'_, '_> {
    pub(in crate::program) fn code_producer(&mut self, value: &Expr<'_>) -> Result<bool, Error> {
        self.quoted_producer_in(value, &mut BTreeSet::new(), 0, QuotedKind::Expression)
    }

    pub(in crate::program) fn pattern_code_producer(
        &mut self,
        value: &Expr<'_>,
        annotation: Option<&Expr<'_>>,
    ) -> Result<bool, Error> {
        if annotation.is_some_and(|ty| QuotedKind::Pattern.annotation(ty)) {
            return Ok(true);
        }
        self.quoted_producer_in(value, &mut BTreeSet::new(), 0, QuotedKind::Pattern)
    }

    fn charge_code_classification(&mut self, span: Span, depth: usize) -> Result<(), Error> {
        self.required_steps = self
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(span, "contextual code classification budget exhausted"))?;
        if depth >= 64 {
            return Err(error(
                span,
                "contextual code classification depth budget exhausted",
            ));
        }
        Ok(())
    }

    fn quoted_producer_in(
        &mut self,
        value: &Expr<'_>,
        bindings: &mut BTreeSet<n::LocalId>,
        depth: usize,
        kind: QuotedKind,
    ) -> Result<bool, Error> {
        self.charge_code_classification(value.span, depth)?;
        Ok(match &value.kind {
            H::CodeQuote(..) => matches!(kind, QuotedKind::Expression),
            H::CodePatternQuote(..) => matches!(kind, QuotedKind::Pattern),
            H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) => {
                bindings.contains(&local(*id))
                    || match kind {
                        QuotedKind::Expression => self.code_values.contains_key(&local(*id)),
                        QuotedKind::Pattern => self.pattern_codes.contains_key(&local(*id)),
                    }
            }
            H::Application(callee, _) | H::ExtendedApplication(callee, _) => {
                let head = match callee.kind {
                    H::NFApplication(head, _) => head,
                    _ => callee,
                };
                let direct = match kind {
                    QuotedKind::Expression => self.generator_owner(callee)?.is_some(),
                    QuotedKind::Pattern => matches!(head.kind, H::ResolvedDef { def_id, .. }
                        if self.package.definition_owner(def_id)
                            .is_some_and(|owner| self.is_pattern_generator(owner))),
                };
                direct
                    || self
                        .generic_code_method_hint(callee)?
                        .is_some_and(|owner| kind.owner(self, owner))
            }
            H::Let(decl) => {
                let initializer = match decl.init {
                    Some(value) => self.quoted_producer_in(value, bindings, depth + 1, kind)?,
                    None => false,
                };
                if initializer || decl.ty.is_some_and(|ty| kind.annotation(ty)) {
                    bindings.extend(pattern_binding_ids(&decl.pattern));
                }
                false
            }
            H::Return(Some(value)) | H::Semi(value) | H::InlineControl(value) => {
                self.quoted_producer_in(value, bindings, depth + 1, kind)?
            }
            H::If(_, yes, no) => {
                let yes = self.quoted_block_producer(yes, bindings, depth + 1, kind)?;
                let no = match no {
                    Some(value) => self.quoted_producer_in(value, bindings, depth + 1, kind)?,
                    None => false,
                };
                yes || no
            }
            H::Block(body) | H::Loop(body) | H::For { body, .. } => {
                self.quoted_block_producer(body, bindings, depth + 1, kind)?
            }
            H::When(arms) => {
                let mut found = false;
                for arm in *arms {
                    if self.quoted_producer_in(arm.body, bindings, depth + 1, kind)? {
                        found = true;
                        break;
                    }
                }
                found
            }
            H::Match(_, arms) => {
                let mut found = false;
                for arm in *arms {
                    if self.quoted_producer_in(arm.body, bindings, depth + 1, kind)? {
                        found = true;
                        break;
                    }
                }
                found
            }
            _ => false,
        })
    }

    fn quoted_block_producer(
        &mut self,
        body: &hir::Block<'_>,
        bindings: &mut BTreeSet<n::LocalId>,
        depth: usize,
        kind: QuotedKind,
    ) -> Result<bool, Error> {
        self.charge_code_classification(body.span, depth)?;
        let mut returned = false;
        for statement in body.stmts {
            returned |= self.quoted_producer_in(statement, bindings, depth + 1, kind)?;
        }
        Ok(returned
            || match body.expr {
                Some(value) => self.quoted_producer_in(value, bindings, depth + 1, kind)?,
                None => false,
            })
    }

    // All structurally possible containers must agree. This observation does
    // not select an implementation or authorize invocation of the hinted body.
    fn computed_code_method_hint(
        &mut self,
        base: &Expr<'_>,
        projection: hir::HirId,
    ) -> Result<Option<OwnerId>, Error> {
        let mut pending = vec![base];
        let mut containers = BTreeSet::new();
        while let Some(value) = pending.pop() {
            self.charge_code_classification(value.span, 0)?;
            match value.kind {
                H::ResolvedDef { def_id, .. } => {
                    containers.insert(def_id);
                }
                H::NFApplication(head, _) => pending.push(head),
                H::Block(block) => match block.expr {
                    Some(value) => pending.push(value),
                    None => return Ok(None),
                },
                H::If(_, yes, Some(no)) => {
                    let Some(yes) = yes.expr else { return Ok(None) };
                    pending.push(yes);
                    pending.push(no);
                }
                H::When(arms) if arms.iter().any(|arm| arm.cond.is_none()) => {
                    pending.extend(arms.iter().map(|arm| arm.body));
                }
                H::Match(_, arms) if !arms.is_empty() => {
                    pending.extend(arms.iter().map(|arm| arm.body));
                }
                _ => return Ok(None),
            }
        }
        let Some(candidates) = self.package.member_access.get(&projection) else {
            return Ok(None);
        };
        let mut found = None;
        let mut category = None;
        for container in containers {
            let mut selected = None;
            for (parent, member) in candidates.iter() {
                self.required_steps = self.required_steps.checked_sub(1).ok_or_else(|| {
                    error(base.span, "contextual code classification budget exhausted")
                })?;
                if *parent == container && selected.replace(*member).is_some() {
                    return Ok(None);
                }
            }
            let Some(owner) = selected.and_then(|id| self.package.definition_owner(id)) else {
                return Ok(None);
            };
            let current_category = if self.is_pattern_generator(owner) {
                0
            } else if self.is_code_generator(owner) {
                1
            } else if self.is_data_consumer(owner) {
                2
            } else {
                return Ok(None);
            };
            if category.is_some_and(|previous| previous != current_category) {
                return Ok(None);
            }
            category = Some(current_category);
            found.get_or_insert(owner);
        }
        Ok(found)
    }

    pub(in crate::program::cmtt) fn generic_code_method_hint(
        &mut self,
        callee: &Expr<'_>,
    ) -> Result<Option<OwnerId>, Error> {
        if !self.interfaces_ready {
            return Ok(None);
        }
        let head = match callee.kind {
            H::NFApplication(head, _) => head,
            _ => callee,
        };
        let H::Projection(base, name) = head.kind else {
            return Ok(None);
        };
        if let H::Cast(_, interface) = base.kind
            && self.is_trait_view_target(interface)
        {
            let interface = match interface.kind {
                H::NFApplication(head, _) => head,
                _ => interface,
            };
            let H::ResolvedDef { def_id, .. } = interface.kind else {
                return Ok(None);
            };
            let Some(owner) = self.package.definition_owner(def_id) else {
                return Ok(None);
            };
            let id = declaration(self.package, &self.package_identities, owner);
            let mut pending = vec![id];
            let mut seen = BTreeSet::new();
            let mut members = BTreeSet::new();
            while let Some(id) = pending.pop() {
                self.charge_code_classification(head.span, 0)?;
                if !seen.insert(id) {
                    continue;
                }
                let interface = self
                    .checked_traits
                    .iter()
                    .find(|item| item.declaration == id)
                    .ok_or_else(|| {
                        error(
                            head.span,
                            "contextual method source lacks its trait contract",
                        )
                    })?;
                if let Some(member) = interface
                    .methods
                    .iter()
                    .find(|member| member.name == name.name.as_str())
                {
                    members.insert(member.declaration);
                } else if let Some(member) = interface
                    .associated
                    .iter()
                    .find(|member| member.name == name.name.as_str())
                {
                    // A local associated value shadows the inherited method
                    // just as it does during ordinary member resolution.
                    members.insert(member.declaration);
                } else {
                    pending.extend(interface.parents.iter().map(|parent| parent.declaration));
                }
            }
            if members.len() > 1 {
                return Err(error(
                    head.span,
                    "ambiguous inherited member; specify its interface source",
                ));
            }
            return members
                .into_iter()
                .next()
                .map(|member| self.contextual_declaration_owner(member, head.span))
                .transpose()
                .map(Option::flatten);
        }
        if let Some(owner) = self.computed_code_method_hint(base, head.hir_id)? {
            return Ok(Some(owner));
        }
        let subject = if self.is_static_method_subject(base) {
            self.generator_static_type_hint(base, 0, None)?
        } else {
            self.generator_intrinsic_input_hint(base, 0)?
        };
        let Some(subject) = subject else {
            return Ok(None);
        };
        self.contextual_method_owner(&subject, name.name.as_str(), head.span)
    }

    // Classification only: never execute a type producer, defaults or arguments.
    pub(in crate::program::cmtt) fn static_code_method_hint(
        &self,
        callee: &Expr<'_>,
    ) -> Result<Option<OwnerId>, Error> {
        if !self.interfaces_ready {
            return Ok(None);
        }
        let H::Projection(base, name) = callee.kind else {
            return Ok(None);
        };
        let H::ResolvedDef { def_id, .. } = base.kind else {
            return Ok(None);
        };
        let Some(container) = self.package.definition_owner(def_id) else {
            return Ok(None);
        };
        let Some(item) = self.package.item(container) else {
            return Ok(None);
        };
        let Some(parameters) = self.type_parameters.get(&container) else {
            return Ok(None);
        };
        if !matches!(item.kind, ItemKind::Struct(_) | ItemKind::Enum(_)) || !parameters.is_empty() {
            return Ok(None);
        }
        self.contextual_method_owner(
            &self.nominal_self_type(container),
            name.name.as_str(),
            callee.span,
        )
    }

    fn contextual_method_owner(
        &self,
        subject: &Term,
        name: &str,
        span: Span,
    ) -> Result<Option<OwnerId>, Error> {
        use crate::implementation::Selection;
        // Capability sources precede inherent lookup in ordinary method calls.
        // Their public declaration determines the category even when the
        // generic receiver has no uniquely selected implementation yet.
        if let Some(interface) = self.context_member_interface(subject, name, span)? {
            let contract = self
                .checked_traits
                .iter()
                .find(|item| item.declaration == interface.declaration)
                .ok_or_else(|| error(span, "contextual method source lacks its trait contract"))?;
            let Some(method) = contract.methods.iter().find(|method| method.name == name) else {
                // Associated values use ordinary value/call elaboration.
                return Ok(None);
            };
            return self.contextual_declaration_owner(method.declaration, span);
        }
        let resolution = self
            .resolve_inherent_capability(name, subject, Budget::default())
            .map_err(|failure| {
                error(
                    span,
                    format!("contextual method classification failed: {failure}"),
                )
            })?;
        let selected = match resolution.types.selection {
            Selection::Selected(id) => id,
            Selection::NoApplicableCandidate => return Ok(None),
            other => {
                return Err(error(
                    span,
                    format!("contextual method classification is unsettled: {other}"),
                ));
            }
        };
        let implementation = self
            .checked_implementations
            .iter()
            .find(|item| item.declaration == selected)
            .ok_or_else(|| {
                error(
                    span,
                    "selected contextual method lacks an implementation contract",
                )
            })?;
        let (_, body) = implementation
            .methods
            .iter()
            .find(|(member, _)| member == name)
            .ok_or_else(|| error(span, "selected contextual method lacks its member body"))?;
        self.contextual_declaration_owner(*body, span)
    }

    // A known declaration with no contextual signature is ordinary data;
    // a missing declaration is an incomplete observation, not that answer.
    fn contextual_declaration_owner(
        &self,
        member: DeclarationId,
        span: Span,
    ) -> Result<Option<OwnerId>, Error> {
        let owner = self
            .package
            .owners()
            .find_map(|(owner, _)| {
                matches_declaration(self.package, &self.package_identities, owner, member)
                    .then_some(owner)
            })
            .ok_or_else(|| error(span, "contextual method declaration is unavailable"))?;
        Ok((self.is_code_generator(owner)
            || self.is_pattern_generator(owner)
            || self.is_data_consumer(owner))
        .then_some(owner))
    }
}
