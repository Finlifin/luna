//! Select ordinary callees and form their invocation under the caller's
//! delivery context. An explicit comptime callee can return its unfinished
//! clause check alongside the ordinary source inputs that depend on it.
use super::*;

impl<'pkg, 'arena> Checker<'pkg, 'arena> {
    pub(super) fn explicit_function_instance(
        &mut self,
        expr: &Expr<'_>,
        callee: &Expr<'_>,
        args: &[Arg<'_>],
        expected: Option<&Term>,
        allow_optional: bool,
    ) -> Result<n::Expr, Error> {
        match self.select_explicit_instance(expr, callee, args, expected, allow_optional)? {
            block_regions::ExplicitInstance::Checking(frame) => {
                let value = self.check_declaration_inputs_region(*frame)?;
                if allow_optional {
                    Ok(value)
                } else {
                    self.callable_value_view(value, expected, expr.span)
                }
            }
            block_regions::ExplicitInstance::Ready(value) => Ok(value),
        }
    }

    pub(super) fn select_explicit_instance<'h>(
        &mut self,
        expr: &Expr<'h>,
        callee: &Expr<'_>,
        args: &[Arg<'h>],
        expected: Option<&Term>,
        allow_optional: bool,
    ) -> Result<block_regions::ExplicitInstance<'h>, Error>
    where
        'pkg: 'h,
        'arena: 'h,
    {
        if let H::Projection(base, name) = &callee.kind {
            if let Some(method) =
                self.static_method(base, name.name.as_str(), callee.span, callee.hir_id)?
            {
                return self
                    .explicit_method_instance(expr, method, args, expected, allow_optional)
                    .map(block_regions::ExplicitInstance::Ready);
            }
            if let Some(assoc) =
                self.associated_projection(base, name.name.as_str(), callee.span, callee.hir_id)?
                && assoc.ty.is_comptime_callable()
            {
                let call = self.finish_call(expr, assoc, args, Vec::new(), expected, None)?;
                return Ok(block_regions::ExplicitInstance::Ready(call));
            }
        }
        let H::ResolvedDef { def_id, .. } = callee.kind else {
            return Err(error(
                expr.span,
                "explicit instantiation requires a resolved generic function",
            ));
        };
        let owner = self
            .package
            .definition_owner(def_id)
            .ok_or_else(|| error(expr.span, "generic function has no HIR owner"))?;
        if matches!(
            self.package.item(owner).map(|item| &item.kind),
            Some(ItemKind::Associated(_))
        ) {
            let assoc = self.lexical_associated_value(owner, callee.span)?;
            if assoc.ty.is_comptime_callable() {
                let call = self.finish_call(expr, assoc, args, Vec::new(), expected, None)?;
                return Ok(block_regions::ExplicitInstance::Ready(call));
            }
        }
        self.ensure_signature(owner, expr.span)?;
        let lexical = self.lexical_function_arguments(owner, expr.span)?;
        if let Some(frame) = self.begin_comptime_function_instance(
            expr,
            owner,
            args,
            &lexical,
            None,
            allow_optional,
            None,
        )? {
            return Ok(block_regions::ExplicitInstance::Checking(Box::new(frame)));
        }
        self.select_static_instance(expr, owner, args, lexical, expected, allow_optional)
            .map(block_regions::ExplicitInstance::Ready)
    }

    fn select_static_instance(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        args: &[Arg<'_>],
        lexical: Vec<Term>,
        expected: Option<&Term>,
        allow_optional: bool,
    ) -> Result<n::Expr, Error> {
        if allow_optional {
            if let Some(arguments) =
                self.partial_positional_clause_prefix(owner, args, &lexical, expr.span)?
            {
                return self.partial_callable_instance(expr, owner, arguments);
            }
        }
        if let Some(value) = self.required_collected_callable_instance(
            expr,
            owner,
            args,
            &lexical,
            None,
            allow_optional,
        )? {
            return if allow_optional {
                Ok(value)
            } else {
                self.callable_value_view(value, expected, expr.span)
            };
        }
        if !lexical.is_empty() {
            let arguments =
                self.type_clause_arguments_with_prefix(owner, args, None, expr.span, lexical)?;
            let value = self.function_instance(expr, owner, arguments, expected, allow_optional)?;
            return if allow_optional {
                Ok(value)
            } else {
                self.callable_value_view(value, expected, expr.span)
            };
        }
        if self.has_optional_type_clauses(owner)
            || self
                .trait_domains(owner)?
                .iter()
                .any(|domain| *domain != p(P::Type))
        {
            let arguments = self.type_clause_arguments(owner, args, None, expr.span)?;
            let value = self.function_instance(expr, owner, arguments, expected, allow_optional)?;
            return if allow_optional {
                Ok(value)
            } else {
                self.callable_value_view(value, expected, expr.span)
            };
        }
        let arguments = self
            .expanded_type_inputs(args, self.type_parameters[&owner].len(), expr.span)?
            .into_iter()
            .map(|(name, ty)| {
                if name.is_some() {
                    Err(error(
                        expr.span,
                        "explicit type arguments currently require positional inputs",
                    ))
                } else {
                    Ok(ty)
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        let value = self.function_instance(expr, owner, arguments, expected, allow_optional)?;
        if allow_optional {
            Ok(value)
        } else {
            self.callable_value_view(value, expected, expr.span)
        }
    }

    pub(super) fn call(
        &mut self,
        expr: &Expr<'_>,
        callee: &Expr<'_>,
        args: &[Arg<'_>],
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        let result = self.with_call_operand_scope(expr, |checker| {
            checker.select_call(expr, callee, args, expected, partial)
        })?;
        self.maybe_unfold_spec_call(result, expr.span)
    }

    #[inline(never)]
    fn select_explicit_call_callee(
        &mut self,
        expr: &Expr<'_>,
        callee: &Expr<'_>,
        head: &Expr<'_>,
        types: &[Arg<'_>],
        args: &[Arg<'_>],
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<Result<n::Expr, n::Expr>, Error> {
        if let H::Projection(base, name) = &head.kind
            && let Some(method) =
                self.static_method(base, name.name.as_str(), head.span, head.hir_id)?
        {
            let mut prefix = Some(Vec::new());
            if let Some(call) = self.partial_named_method_call(
                expr,
                callee,
                &method,
                types,
                args,
                &mut prefix,
                expected,
                partial,
            )? {
                return Ok(Ok(call));
            }
        }
        let instantiated = match self.select_explicit_instance(callee, head, types, None, true)? {
            block_regions::ExplicitInstance::Checking(frame) => {
                return self
                    .check_invocation_region(expr, args, expected, partial, *frame)
                    .map(Ok);
            }
            block_regions::ExplicitInstance::Ready(callee) => callee,
        };
        if args.is_empty()
            && !matches!(
                instantiated.ty,
                Term::Function { .. }
                    | Term::Closure { .. }
                    | Term::ComptimeFunction(_)
                    | Term::NormalFormFunction(..)
                    | Term::UnsafeFunction(_)
                    | Term::PureFunction(_)
            )
        {
            return Ok(Ok(instantiated));
        }
        Ok(Err(instantiated))
    }

    #[inline(never)]
    fn select_call(
        &mut self,
        expr: &Expr<'_>,
        callee: &Expr<'_>,
        args: &[Arg<'_>],
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        // Direct comptime calls must be checked in their source phase before
        // runtime expected types or spread preparation can affect their inputs.
        let head = if let H::NFApplication(head, _) = callee.kind {
            head
        } else {
            callee
        };
        if let H::ResolvedDef { def_id, .. } = head.kind {
            if self.package.definition_owner(def_id).is_some_and(|owner| {
                self.is_code_generator(owner) || self.is_pattern_generator(owner)
            }) {
                return Err(error(
                    expr.span,
                    "code generator must be invoked in a comptime code position",
                ));
            }
        }
        let comptime = if let H::ResolvedDef { def_id, .. } = head.kind {
            self.package.definition_owner(def_id).and_then(|owner| self.package.item(owner))
                .is_some_and(|item| matches!(&item.kind, ItemKind::Fn(sig, _) if sig.modifiers.is_comptime && !sig.modifiers.is_normal_form_head))
        } else {
            false
        };
        if comptime && self.protocols.splice.is_some() && self.runtime_value_context() {
            let value =
                self.with_staged_input(true, |this| this.call(expr, callee, args, None, None))?;
            return self.deliver_comptime_result(expr, value, expected);
        }
        if args.iter().any(|arg| matches!(arg, Arg::Expand(_))) {
            return self.spread_call(expr, callee, args, expected, partial);
        }
        if let Some(call) = self.receiver_call(expr, callee, args, expected, partial)? {
            return Ok(call);
        }
        let explicit = if let H::NFApplication(head, types) = &callee.kind {
            Some((*head, *types))
        } else {
            None
        };
        let head = explicit.map_or(callee, |(head, _)| head);
        let direct = if let H::ResolvedDef { def_id, .. } = head.kind {
            self.package.definition_owner(def_id).filter(|owner| {
                !matches!(
                    self.package.item(*owner).map(|item| &item.kind),
                    Some(ItemKind::Const(..) | ItemKind::Associated(_))
                )
            })
        } else {
            None
        };
        if let Some(owner) = direct {
            if matches!(&self.package.item(owner).unwrap().kind, ItemKind::Fn(sig, _) if sig.modifiers.is_normal_form_head)
            {
                return Err(error(
                    callee.span,
                    "normal-form heads cannot be invoked as ordinary functions",
                ));
            }
            self.require_non_trait_body_reference(owner, callee.span)?;
            self.ensure_signature(owner, expr.span)?;
            if self.open_functions.contains_key(&owner) {
                return self.explicit_open_function_call(
                    expr,
                    owner,
                    explicit.map(|(_, args)| args),
                    args,
                    expected,
                );
            }
            if let Some((_, clauses)) = explicit
                && let Some(call) = self.with_partial_named_call_plan(
                    owner,
                    clauses,
                    expr,
                    args,
                    expected,
                    partial,
                    |this, plan| {
                        this.generic_call_with_plan(
                            expr,
                            owner,
                            args,
                            expected,
                            partial,
                            vec![],
                            plan,
                            None,
                            CallUse::Invocation,
                        )
                    },
                )?
            {
                return Ok(call);
            }
            if explicit.is_none()
                && self
                    .type_parameters
                    .get(&owner)
                    .is_some_and(|types| !types.is_empty())
            {
                return self.generic_call(expr, owner, args, expected, partial);
            }
        }
        let callee = if let Some((head, types)) = explicit {
            match self
                .select_explicit_call_callee(expr, callee, head, types, args, expected, partial)?
            {
                Ok(done) => return Ok(done),
                Err(instantiated) => instantiated,
            }
        } else if let Some(owner) = direct {
            self.ensure_signature(owner, expr.span)?;
            let ty = self
                .signatures
                .get(&owner)
                .ok_or_else(|| error(callee.span, "declaration is not a supported callable"))?
                .clone();
            let ty = self.callable_type(owner, ty);
            self.check_type_application(owner, &[], expr.span)?;
            n::Expr {
                ty,
                source: source(callee.span),
                kind: if self.requirements.get(&owner).is_some_and(Vec::is_empty) {
                    K::Function(declaration(self.package, &self.package_identities, owner))
                } else {
                    K::FunctionInstance {
                        declaration: declaration(self.package, &self.package_identities, owner),
                        arguments: vec![],
                    }
                },
            }
        } else {
            self.invocation_callee(callee)?
        };
        self.finish_call(expr, callee, args, Vec::new(), expected, partial)
    }

    #[inline(never)]
    pub(super) fn finish_planned_prefix_call(
        &mut self,
        expr: &Expr<'_>,
        callee: n::Expr,
        args: &[Arg<'_>],
        prefix: Vec<n::Expr>,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        // The receiver is the leftmost source computation. Bind it outside
        // clause preparation, also when a spread later wraps the inner call.
        let saved_locals = self.locals.clone();
        let result = (|| {
            let mut statements = Vec::new();
            let mut prepared = Vec::new();
            for value in prefix {
                prepared.push(self.bind_type_input(value, &mut statements, expr.span)?);
            }
            let call = self.finish_prepared_call(
                expr,
                callee,
                args,
                prepared,
                expected,
                partial,
                CallUse::Invocation,
            )?;
            Ok(n::Expr {
                source: source(expr.span),
                ty: call.ty.clone(),
                kind: K::Block {
                    statements,
                    result: Some(Box::new(call)),
                },
            })
        })();
        self.locals = saved_locals;
        result
    }

    /// One source-ordered plan for ordinary, static and receiver calls.
    #[inline(never)]
    pub(super) fn finish_call(
        &mut self,
        expr: &Expr<'_>,
        callee: n::Expr,
        args: &[Arg<'_>],
        prefix: Vec<n::Expr>,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        if prefix.is_empty()
            || !matches!(
                callee.kind,
                K::PlannedFunctionInstance { .. } | K::ComptimeFunctionInstance { .. }
            )
        {
            return self.finish_prepared_call(
                expr,
                callee,
                args,
                prefix,
                expected,
                partial,
                CallUse::Invocation,
            );
        }
        self.finish_planned_prefix_call(expr, callee, args, prefix, expected, partial)
    }

    #[inline(never)]
    pub(super) fn finish_prepared_call(
        &mut self,
        expr: &Expr<'_>,
        callee: n::Expr,
        args: &[Arg<'_>],
        prefix: Vec<n::Expr>,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
        call_use: CallUse,
    ) -> Result<n::Expr, Error> {
        self.plan_prepared_call(expr, callee, args, prefix, expected, partial, call_use)?
            .finish(self)
    }

    #[inline(never)]
    pub(super) fn plan_prepared_call<'h>(
        &mut self,
        expr: &Expr<'h>,
        callee: n::Expr,
        args: &[Arg<'h>],
        prefix: Vec<n::Expr>,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
        call_use: CallUse,
    ) -> Result<block_regions::CallPlan<'h>, Error> {
        if callee.ty.is_normal_form_callable() && !matches!(expr.kind, H::NFApplication(..)) {
            return Err(error(
                expr.span,
                "normal-form heads cannot be invoked as ordinary functions",
            ));
        }
        // Selected trait methods and stored comptime callables use this same
        // boundary. The synthesized splice method itself remains runtime.
        if callee.ty.is_comptime_callable()
            && self.protocols.splice.is_some()
            && self.runtime_value_context()
        {
            let value = self.with_staged_input(true, |this| {
                this.finish_prepared_call(expr, callee, args, prefix, None, None, call_use)
            })?;
            return self
                .deliver_comptime_result(expr, value, expected)
                .map(block_regions::CallPlan::Ready);
        }
        let direct_id = match &callee.kind {
            K::PlannedFunctionInstance {
                declaration: id, ..
            }
            | K::ComptimeFunctionInstance {
                declaration: id, ..
            }
            | K::Function(id)
            | K::FunctionInstance {
                declaration: id, ..
            } => Some(*id),
            // Interface entry metadata remains independent of the selected body.
            K::TraitMethod { member, .. } => Some(*member),
            _ => None,
        };
        let direct = direct_id
            .filter(|_| !callee.ty.is_normal_form_callable())
            .and_then(|id| self.source_declarations.owner(id));
        if let Some(owner) = direct {
            self.check_quote_callable(owner, expr.span)?;
            self.check_spec_callable(owner, expr.span)?;
        }
        if direct.is_some_and(|owner| self.is_quote_function(owner)) {
            self.record_quote_application(expr.hir_id, expr.span)?;
        }
        if let (Some(owner), K::FunctionInstance { arguments, .. }) = (direct, &callee.kind) {
            if arguments.len() < self.type_parameters[&owner].len() {
                return self
                    .generic_call_with_receiver(
                        expr,
                        owner,
                        args,
                        expected,
                        partial,
                        prefix,
                        arguments.clone(),
                        None,
                    )
                    .map(block_regions::CallPlan::Ready);
            }
        }
        // Preserve selected clause arguments when a constant/function value
        // denotes an open declaration. Definition-side edges keep the template.
        if let Some(owner) = direct
            && self.open_functions.contains_key(&owner)
            && call_use == CallUse::Invocation
            && prefix.is_empty()
        {
            let clauses = match &callee.kind {
                K::Function(_) => Some(Vec::new()),
                K::FunctionInstance { arguments, .. } => Some(arguments.clone()),
                _ => None,
            };
            if let Some(clauses) = clauses {
                self.check_type_application(owner, &clauses, expr.span)?;
                let saved = self.alias_substitution.clone();
                for (binder, argument) in self.declaration_binders(owner).into_iter().zip(clauses) {
                    self.alias_substitution
                        .insert(binder.declaration, std::sync::Arc::new(argument));
                }
                let result = self.instantiated_open_call(expr, owner, args, expected);
                self.alias_substitution = saved;
                return result.map(block_regions::CallPlan::Ready);
            }
        }
        if let (
            Some(owner),
            K::TraitMethod {
                interface_arguments,
                method_arguments,
                ..
            },
        ) = (direct, &callee.kind)
        {
            if 1 + interface_arguments.len() + method_arguments.len()
                < self.type_parameters[&owner].len()
            {
                return self
                    .generic_trait_call(expr, owner, callee, args, prefix, expected, partial)
                    .map(block_regions::CallPlan::Ready);
            }
        }
        let retained_callable = self.structural_type(&callee.ty, expr.span)?;
        let callable = self
            .observe_call_type(&retained_callable, expr)?
            .into_callable_signature();
        let Some(signature) = dependent_inputs::CallSignature::of(&callable) else {
            return Err(error(
                expr.span,
                if is_never(&callee.ty) {
                    "diverging callee has no callable signature for checking its arguments"
                } else {
                    "called expression is not a function"
                },
            ));
        };
        let declarations = direct
            .and_then(|owner| self.package.item(owner))
            .and_then(|item| match &item.kind {
                ItemKind::Fn(signature, _)
                | ItemKind::Case(signature, _)
                | ItemKind::FnDeclaration(signature) => Some(signature.params),
                _ => None,
            });
        let offset = prefix.len();
        if offset > signature.parameters.len() {
            return Err(error(expr.span, "method receiver count mismatch"));
        }
        let (routes, mut assigned) = route_arguments(
            args,
            declarations.map(|p| &p[offset..]),
            signature.parameters.len() - offset,
            expr.span,
        )
        .map_err(|mut failure| {
            if offset != 0 && failure.message.starts_with("argument count mismatch") {
                failure.message = format!("method {}", failure.message);
            }
            failure
        })?;
        let contextual =
            self.resolve_contextual_quote_inputs(expr, declarations, offset, &assigned)?;
        for &(slot, _) in &contextual {
            assigned[slot - offset] = true;
        }
        let site = dependent_inputs::CallSite {
            callee,
            owner: direct,
            call_use,
            routes,
            source: expr.clone(),
        };
        let parameter_bindings = declarations
            .map(|params| params.iter().map(|p| local(p.hir_id)).collect())
            .unwrap_or_default();
        self.plan_ordinary_call(
            site,
            callable,
            retained_callable,
            assigned,
            prefix,
            contextual,
            parameter_bindings,
            expected,
        )
    }
}
