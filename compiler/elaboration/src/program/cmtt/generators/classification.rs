//! Observe contextual-code categories without invoking producers or arguments.
//! Failure to finish an observation must not select ordinary value elaboration.
use super::*;
use crate::ptc::Budget;

#[derive(Clone, Copy)]
enum QuotedKind {
    Expression,
    Pattern,
}

// A selected ordinary declaration is a completed category observation. It
// must not be confused with absence, which permits another lookup route.
enum MethodCategory {
    Ordinary,
    Contextual(OwnerId),
}

impl MethodCategory {
    fn owner(self) -> Option<OwnerId> {
        match self {
            Self::Ordinary => None,
            Self::Contextual(owner) => Some(owner),
        }
    }
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
    pub(in crate::program) fn is_any_type_expr(&mut self, value: &Expr<'_>) -> Result<bool, Error> {
        self.charge_code_classification(value.span, 0)?;
        match &value.kind {
            H::TyAny => Ok(true),
            H::Builtin(name) if name.as_str() == "Any" => Ok(true),
            H::ContextualExprType(..) | H::ContextualPatternType(..) => Ok(false),
            H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) => {
                Ok(match self.generator_data.get(&local(*id)) {
                    Some(Some(n::Expr {
                        kind: K::Literal(n::Literal::Description(n::DescriptionLiteral::Any)),
                        ..
                    })) => true,
                    Some(Some(n::Expr {
                        kind: K::TypeValue(Term::AnyType),
                        ..
                    })) => true,
                    _ => false,
                })
            }
            H::ResolvedDef { def_id, .. } => {
                let Some(owner) = self.package.definition_owner(*def_id) else {
                    return Ok(false);
                };
                match self.package.item(owner).map(|item| &item.kind) {
                    Some(ItemKind::TypeAlias(..) | ItemKind::Const(..)) => {
                        Ok(self.completed_type_is_any(value)?)
                    }
                    Some(ItemKind::Associated(..)) => {
                        self.completed_type_is_any_or_out_of_scope(value)
                    }
                    _ => Ok(false),
                }
            }
            H::Projection(..)
            | H::NFApplication(..)
            | H::Application(..)
            | H::ExtendedApplication(..)
            | H::Block(..) => self.completed_type_is_any_or_out_of_scope(value),
            _ => Ok(false),
        }
    }

    fn completed_type_is_any(&mut self, value: &Expr<'_>) -> Result<bool, Error> {
        Ok(matches!(
            self.completed_type(value)?,
            CompletedType::Description(n::DescriptionLiteral::Any)
                | CompletedType::Identity(Term::AnyType)
        ))
    }

    fn completed_type_is_any_or_out_of_scope(&mut self, value: &Expr<'_>) -> Result<bool, Error> {
        match self.completed_type(value) {
            Ok(
                CompletedType::Description(n::DescriptionLiteral::Any)
                | CompletedType::Identity(Term::AnyType),
            ) => Ok(true),
            Ok(_) => Ok(false),
            Err(err)
                if self.required_steps > 0
                    && !err.message.contains("budget")
                    && !err.message.contains("exhausted")
                    && !err.message.contains("cyclic") =>
            {
                Ok(false)
            }
            Err(err) => Err(err),
        }
    }

    pub(in crate::program::cmtt) fn code_domain_or_any_type_expr(
        &mut self,
        value: &Expr<'_>,
    ) -> Result<Term, Error> {
        match self.completed_type(value)? {
            CompletedType::Description(n::DescriptionLiteral::Any) => Ok(Term::AnyType),
            other => other.identity(value.span),
        }
    }

    pub(in crate::program::cmtt) fn is_abstract_any_value(value: &Expr<'_>) -> bool {
        matches!(value.kind, H::TyAny)
            || matches!(&value.kind, H::Builtin(name) if name.as_str() == "any")
    }

    pub(in crate::program) fn is_any_code_function(
        &mut self,
        owner: OwnerId,
    ) -> Result<bool, Error> {
        let Some(item) = self.package.item(owner) else {
            return Ok(false);
        };
        let (ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)) = &item.kind else {
            return Ok(false);
        };
        if !sig.modifiers.is_comptime || sig.modifiers.is_normal_form_head || sig.modifiers.is_quote
        {
            return Ok(false);
        }
        let Some(ret) = sig.return_ty else {
            return Ok(false);
        };
        self.is_any_type_expr(ret)
    }

    pub(in crate::program::cmtt) fn any_returning_comptime_owner(
        &mut self,
        callee: &Expr<'_>,
    ) -> Result<Option<OwnerId>, Error> {
        let head = match callee.kind {
            H::NFApplication(head, _) => head,
            _ => callee,
        };
        if let H::ResolvedDef { def_id, .. } = head.kind
            && let Some(owner) = self.package.definition_owner(def_id)
            && self.is_any_code_function(owner)?
        {
            return Ok(Some(owner));
        }
        Ok(None)
    }

    pub(in crate::program) fn any_code_producer(
        &mut self,
        value: &Expr<'_>,
    ) -> Result<bool, Error> {
        let mut seen = BTreeSet::new();
        self.any_code_producer_in(value, &mut BTreeSet::new(), &mut seen, 0)
    }

    fn any_code_producer_in(
        &mut self,
        value: &Expr<'_>,
        bindings: &mut BTreeSet<n::LocalId>,
        seen: &mut BTreeSet<OwnerId>,
        depth: usize,
    ) -> Result<bool, Error> {
        self.charge_code_classification(value.span, depth)?;
        if Self::is_abstract_any_value(value) {
            return Ok(true);
        }
        Ok(match &value.kind {
            H::CodeQuote(..) | H::CodePatternQuote(..) => true,
            H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) => {
                let id = local(*id);
                bindings.contains(&id)
                    || self.code_values.contains_key(&id)
                    || self.pattern_codes.contains_key(&id)
                    || self.locals.get(&id) == Some(&Term::AnyType)
            }
            H::Application(callee, args) | H::ExtendedApplication(callee, args) => {
                let head = match callee.kind {
                    H::NFApplication(head, _) => head,
                    _ => callee,
                };
                let direct_generator = self.generator_owner(callee)?.is_some()
                    || matches!(head.kind, H::ResolvedDef { def_id, .. }
                        if self.package.definition_owner(def_id)
                            .is_some_and(|owner| self.is_pattern_generator(owner)))
                    || match self.generic_code_method_hint(callee)? {
                        Some(owner) => {
                            self.is_code_generator(owner)
                                || self.is_pattern_generator(owner)
                                || self.is_any_code_function(owner)?
                        }
                        None => false,
                    };
                if direct_generator {
                    return Ok(true);
                }
                if let Some(owner) = self.any_returning_comptime_owner(callee)? {
                    for arg in *args {
                        let expr = match arg {
                            Arg::Positional(e)
                            | Arg::Named(_, e)
                            | Arg::Expand(e)
                            | Arg::Implicit(e)
                            | Arg::PostLambda(e) => e,
                        };
                        if self.any_code_producer_in(expr, bindings, seen, depth + 1)? {
                            return Ok(true);
                        }
                    }
                    if seen.insert(owner)
                        && let Some(item) = self.package.item(owner)
                        && let ItemKind::Fn(_, body_id) = item.kind
                        && let Some(body) = self.package.body(body_id)
                    {
                        return self.any_code_producer_in(body.value, bindings, seen, depth + 1);
                    }
                }
                false
            }
            H::Let(decl) => {
                let init = match decl.init {
                    Some(expr) => self.any_code_producer_in(expr, bindings, seen, depth + 1)?,
                    None => false,
                };
                let is_any = decl
                    .ty
                    .map(|ty| self.is_any_type_expr(ty))
                    .transpose()?
                    .unwrap_or(false);
                if init
                    || is_any
                    || decl.ty.is_some_and(|ty| {
                        matches!(
                            ty.kind,
                            H::ContextualExprType(..) | H::ContextualPatternType(..)
                        )
                    })
                {
                    bindings.extend(pattern_binding_ids(&decl.pattern));
                }
                false
            }
            H::Return(Some(v)) | H::Semi(v) | H::InlineControl(v) => {
                self.any_code_producer_in(v, bindings, seen, depth + 1)?
            }
            H::If(_, yes, no) => {
                let mut found = self.any_code_block_producer(yes, bindings, seen, depth + 1)?;
                if !found && let Some(no) = no {
                    found = self.any_code_producer_in(no, bindings, seen, depth + 1)?;
                }
                found
            }
            H::Block(body) | H::Loop(body) | H::For { body, .. } => {
                self.any_code_block_producer(body, bindings, seen, depth + 1)?
            }
            H::When(arms) => {
                let mut found = false;
                for arm in *arms {
                    if self.any_code_producer_in(arm.body, bindings, seen, depth + 1)? {
                        found = true;
                        break;
                    }
                }
                found
            }
            H::Match(_, arms) => {
                let mut found = false;
                for arm in *arms {
                    if self.any_code_producer_in(arm.body, bindings, seen, depth + 1)? {
                        found = true;
                        break;
                    }
                }
                found
            }
            _ => false,
        })
    }

    fn any_code_block_producer(
        &mut self,
        body: &hir::Block<'_>,
        bindings: &mut BTreeSet<n::LocalId>,
        seen: &mut BTreeSet<OwnerId>,
        depth: usize,
    ) -> Result<bool, Error> {
        self.charge_code_classification(body.span, depth)?;
        let mut returned = false;
        for statement in body.stmts {
            returned |= self.any_code_producer_in(statement, bindings, seen, depth + 1)?;
        }
        Ok(returned
            || match body.expr {
                Some(value) => self.any_code_producer_in(value, bindings, seen, depth + 1)?,
                None => false,
            })
    }

    pub(in crate::program) fn code_producer(&mut self, value: &Expr<'_>) -> Result<bool, Error> {
        self.quoted_producer_in(
            value,
            &mut BTreeSet::new(),
            &mut BTreeSet::new(),
            0,
            QuotedKind::Expression,
        )
    }

    pub(in crate::program) fn pattern_code_producer(
        &mut self,
        value: &Expr<'_>,
        annotation: Option<&Expr<'_>>,
    ) -> Result<bool, Error> {
        if annotation.is_some_and(|ty| QuotedKind::Pattern.annotation(ty)) {
            return Ok(true);
        }
        self.quoted_producer_in(
            value,
            &mut BTreeSet::new(),
            &mut BTreeSet::new(),
            0,
            QuotedKind::Pattern,
        )
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
        seen: &mut BTreeSet<OwnerId>,
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
            H::Application(callee, args) | H::ExtendedApplication(callee, args) => {
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
                if direct
                    || self
                        .generic_code_method_hint(callee)?
                        .is_some_and(|owner| kind.owner(self, owner))
                {
                    return Ok(true);
                }
                if let Some(owner) = self.any_returning_comptime_owner(callee)? {
                    for arg in *args {
                        let expr = match arg {
                            Arg::Positional(e)
                            | Arg::Named(_, e)
                            | Arg::Expand(e)
                            | Arg::Implicit(e)
                            | Arg::PostLambda(e) => e,
                        };
                        if self.quoted_producer_in(expr, bindings, seen, depth + 1, kind)? {
                            return Ok(true);
                        }
                    }
                    if seen.insert(owner)
                        && let Some(item) = self.package.item(owner)
                        && let ItemKind::Fn(_, body_id) = item.kind
                        && let Some(body) = self.package.body(body_id)
                    {
                        return self.quoted_producer_in(
                            body.value,
                            bindings,
                            seen,
                            depth + 1,
                            kind,
                        );
                    }
                }
                false
            }
            H::Let(decl) => {
                let initializer = match decl.init {
                    Some(value) => {
                        self.quoted_producer_in(value, bindings, seen, depth + 1, kind)?
                    }
                    None => false,
                };
                if initializer || decl.ty.is_some_and(|ty| kind.annotation(ty)) {
                    bindings.extend(pattern_binding_ids(&decl.pattern));
                }
                false
            }
            H::Return(Some(value)) | H::Semi(value) | H::InlineControl(value) => {
                self.quoted_producer_in(value, bindings, seen, depth + 1, kind)?
            }
            H::If(_, yes, no) => {
                let yes = self.quoted_block_producer(yes, bindings, seen, depth + 1, kind)?;
                let no = match no {
                    Some(value) => {
                        self.quoted_producer_in(value, bindings, seen, depth + 1, kind)?
                    }
                    None => false,
                };
                yes || no
            }
            H::Block(body) | H::Loop(body) | H::For { body, .. } => {
                self.quoted_block_producer(body, bindings, seen, depth + 1, kind)?
            }
            H::When(arms) => {
                let mut found = false;
                for arm in *arms {
                    if self.quoted_producer_in(arm.body, bindings, seen, depth + 1, kind)? {
                        found = true;
                        break;
                    }
                }
                found
            }
            H::Match(_, arms) => {
                let mut found = false;
                for arm in *arms {
                    if self.quoted_producer_in(arm.body, bindings, seen, depth + 1, kind)? {
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
        seen: &mut BTreeSet<OwnerId>,
        depth: usize,
        kind: QuotedKind,
    ) -> Result<bool, Error> {
        self.charge_code_classification(body.span, depth)?;
        let mut returned = false;
        for statement in body.stmts {
            returned |= self.quoted_producer_in(statement, bindings, seen, depth + 1, kind)?;
        }
        Ok(returned
            || match body.expr {
                Some(value) => self.quoted_producer_in(value, bindings, seen, depth + 1, kind)?,
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
                let interface = self.checked_traits.by_id(id).ok_or_else(|| {
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
        if let Some(domain) = self.computed_type_receiver_domain(base, head.hir_id)?
            && let Some(category) =
                self.contextual_method_category(&domain, name.name.as_str(), head.span)?
        {
            return Ok(category.owner());
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
        Ok(self
            .contextual_method_category(&subject, name.name.as_str(), head.span)?
            .and_then(MethodCategory::owner))
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
        Ok(self
            .contextual_method_category(
                &self.nominal_self_type(container),
                name.name.as_str(),
                callee.span,
            )?
            .and_then(MethodCategory::owner))
    }

    fn contextual_method_category(
        &self,
        subject: &Term,
        name: &str,
        span: Span,
    ) -> Result<Option<MethodCategory>, Error> {
        use crate::implementation::Selection;
        // Capability sources precede inherent lookup in ordinary method calls.
        // Their public declaration determines the category even when the
        // generic receiver has no uniquely selected implementation yet.
        if let Some(interface) = self.context_member_interface(subject, name, span)? {
            let contract = self
                .checked_traits
                .by_id(interface.declaration)
                .ok_or_else(|| error(span, "contextual method source lacks its trait contract"))?;
            let Some(method) = contract.methods.iter().find(|method| method.name == name) else {
                // Associated values use ordinary value/call elaboration.
                return Ok(Some(MethodCategory::Ordinary));
            };
            return self
                .contextual_declaration_category(method.declaration, span)
                .map(Some);
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
                    format!(
                        "contextual method classification is unsettled for {subject}.{name}: {other}{}",
                        resolution
                            .types
                            .assessments
                            .iter()
                            .filter_map(|(id, assessment)| {
                                match assessment {
                                    crate::implementation::TypeAssessment::Pending(reason)
                                    | crate::implementation::TypeAssessment::Invalid(reason) => {
                                        Some(format!("; {id}: {reason}"))
                                    }
                                    _ => None,
                                }
                            })
                            .collect::<String>()
                    ),
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
        self.contextual_declaration_category(*body, span).map(Some)
    }

    // A known declaration with no contextual signature is ordinary data;
    // a missing declaration is an incomplete observation, not that answer.
    fn contextual_declaration_owner(
        &self,
        member: DeclarationId,
        span: Span,
    ) -> Result<Option<OwnerId>, Error> {
        Ok(self.contextual_declaration_category(member, span)?.owner())
    }

    fn contextual_declaration_category(
        &self,
        member: DeclarationId,
        span: Span,
    ) -> Result<MethodCategory, Error> {
        let owner = self
            .source_declarations
            .owner(member)
            .ok_or_else(|| error(span, "contextual method declaration is unavailable"))?;
        Ok(
            if self.is_code_generator(owner)
                || self.is_pattern_generator(owner)
                || self.is_data_consumer(owner)
            {
                MethodCategory::Contextual(owner)
            } else {
                MethodCategory::Ordinary
            },
        )
    }
}
