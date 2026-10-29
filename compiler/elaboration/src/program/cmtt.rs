//! Definition-bound expression code. Construction checks a runtime body but
//! publishes no runtime reads, moves or effects. Connection produces ordinary
//! NIR, which goes through the existing resource/effect verification pipeline.
use super::*;
mod functions;
mod generators;
pub(super) mod patterns;
mod test_templates;
use comptime_interpretor::CodeValue;
pub(super) use functions::expand_quote_functions;
pub(crate) use functions::{expand_bound_quote, expand_materialized_quotes, inline_template};
pub(super) use generators::Generator;
use std::{fmt, sync::Arc};

use nir::code_interfaces::ExpressionCodeInterface as Interface;
pub(super) use nir::code_values::ExpressionCode;

fn binding(value: &Expr<'_>) -> Result<n::LocalId, Error> {
    match value.kind {
        H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) => Ok(local(id)),
        _ => Err(error(
            value.span,
            "code substitution requires a resolved runtime binding",
        )),
    }
}

impl Checker<'_, '_> {
    // Phase checks inspect representation storage without equating nominal
    // identities or unfolding normal-form heads. Keep every nominal family in
    // this snapshot so a wrapper cannot hide comptime-only data in code ports.
    fn code_storage_metadata(&self) -> n::Package {
        n::Package {
            target: self.target,
            structs: self.structs.values().cloned().collect(),
            enums: self.enums.values().cloned().collect(),
            newtypes: self.newtypes.values().cloned().collect(),
            normal_forms: self.normal_form_metadata(),
            ..Default::default()
        }
    }

    pub(super) fn freshen_constant_graph(
        &mut self,
        value: &mut n::Expr,
        span: Span,
    ) -> Result<(), Error> {
        // Completed snapshots are closed. Definition-side formation witnesses
        // may still refer to the enclosing staged inputs: rename only their
        // internal bindings, preserving those lexical connections.
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let free = nir::free_bindings::collect(value, &mut fuel);
        self.required_steps -= (before - fuel) as u64;
        let connections = free
            .map_err(|message| error(span, message))?
            .into_iter()
            .map(|binding| (binding, binding))
            .collect();
        freshen(
            value,
            connections,
            &mut self.next_temporary_binding,
            &mut self.expansion_sources,
            span,
        )
    }

    fn pattern_generator_callee(&mut self, callee: &Expr<'_>) -> bool {
        let head = match callee.kind {
            H::NFApplication(head, _) => head,
            _ => callee,
        };
        matches!(head.kind,H::ResolvedDef {def_id,..} if self.package.definition_owner(def_id).is_some_and(|owner|self.is_pattern_generator(owner)))
            || self
                .generic_code_method_hint(head)
                .is_some_and(|owner| self.is_pattern_generator(owner))
    }

    fn pattern_code_producer(&mut self, value: &Expr<'_>, annotation: Option<&Expr<'_>>) -> bool {
        annotation.is_some_and(|ty| matches!(ty.kind, H::ContextualPatternType(..)))
            || self.pattern_producer_in(value, &mut BTreeSet::new(), 0)
    }

    fn pattern_producer_in(
        &mut self,
        value: &Expr<'_>,
        bindings: &mut BTreeSet<n::LocalId>,
        depth: usize,
    ) -> bool {
        if depth >= 64 {
            return false;
        }
        match &value.kind {
            H::CodePatternQuote(..) => true,
            H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) => {
                bindings.contains(&local(*id)) || self.pattern_codes.contains_key(&local(*id))
            }
            H::Application(callee, _) | H::ExtendedApplication(callee, _) => {
                self.pattern_generator_callee(callee)
            }
            H::Let(decl) => {
                if decl
                    .init
                    .is_some_and(|value| self.pattern_producer_in(value, bindings, depth + 1))
                    || decl
                        .ty
                        .is_some_and(|ty| matches!(ty.kind, H::ContextualPatternType(..)))
                {
                    bindings.extend(pattern_binding_ids(&decl.pattern));
                }
                false
            }
            H::Return(Some(value)) | H::Semi(value) | H::InlineControl(value) => {
                self.pattern_producer_in(value, bindings, depth + 1)
            }
            H::If(_, yes, no) => {
                let yes = self.pattern_block_producer(yes, bindings, depth + 1);
                let no =
                    no.is_some_and(|value| self.pattern_producer_in(value, bindings, depth + 1));
                yes || no
            }
            H::Block(body) | H::Loop(body) | H::For { body, .. } => {
                self.pattern_block_producer(body, bindings, depth + 1)
            }
            H::When(arms) => arms
                .iter()
                .any(|arm| self.pattern_producer_in(arm.body, bindings, depth + 1)),
            H::Match(_, arms) => arms
                .iter()
                .any(|arm| self.pattern_producer_in(arm.body, bindings, depth + 1)),
            _ => false,
        }
    }

    fn pattern_block_producer(
        &mut self,
        body: &hir::Block<'_>,
        bindings: &mut BTreeSet<n::LocalId>,
        depth: usize,
    ) -> bool {
        if depth >= 64 {
            return false;
        }
        let mut returned = false;
        for statement in body.stmts {
            returned |= self.pattern_producer_in(statement, bindings, depth + 1);
        }
        returned
            || body
                .expr
                .is_some_and(|value| self.pattern_producer_in(value, bindings, depth + 1))
    }

    pub(super) fn code_declaration(
        &mut self,
        decl: &hir::decl::LetDecl<'_>,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let initializer = decl
            .init
            .ok_or_else(|| error(decl.span, "local const requires an initializer"))?;
        if self.pattern_code_producer(initializer, decl.ty) {
            return self.pattern_code_binding(decl, expected);
        }
        if self.code_producer(initializer)
            || decl
                .ty
                .is_some_and(|ty| matches!(ty.kind, H::ContextualExprType(..)))
        {
            return self.code_binding(decl, expected, false);
        }
        if decl.mutable {
            return Err(error(
                decl.span,
                "local const requires an immutable binding",
            ));
        }
        if self.checking_open_behavior {
            return if decl.guard.is_some() || decl.otherwise.is_some() {
                self.ordinary_conditional_constant_binding(decl, expected)
            } else {
                self.ordinary_constant_binding(decl, expected)
            };
        }
        if decl.guard.is_some() || decl.otherwise.is_some() {
            let value = self.conditional_local_data_binding(decl)?;
            if let Some(expected) = expected {
                self.require_expression(&value.ty, expected, decl.span)?;
            }
            return Ok(value);
        }
        if let Some(expected) = expected {
            self.require(&p(P::Void), expected, decl.span)?;
        }
        let in_code = std::mem::replace(&mut self.in_code_body, false);
        let result = self.local_data_binding(decl);
        self.in_code_body = in_code;
        result?;
        Ok(n::Expr {
            ty: p(P::Void),
            source: source(decl.span),
            kind: K::Unit,
        })
    }

    fn code_binding(
        &mut self,
        decl: &hir::decl::LetDecl<'_>,
        expected: Option<&Term>,
        mutable: bool,
    ) -> Result<n::Expr, Error> {
        if (decl.mutable && !mutable) || decl.guard.is_some() || decl.otherwise.is_some() {
            return Err(error(
                decl.span,
                "quoted code requires an unconditional immutable comptime binding",
            ));
        }
        // Code witnesses remain elaboration values. Wildcards discard a
        // completed witness; `as` chains bind aliases of that same witness,
        // without inventing a runtime representation or inspecting its body.
        let mut pattern = &decl.pattern;
        let mut bindings = Vec::new();
        loop {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(pattern.span, "code binding pattern budget exhausted"))?;
            match &pattern.kind {
                hir::PatternKind::Wild => break,
                hir::PatternKind::Binding(hir::common::BindingMode::ByValue, _, child) => {
                    bindings.push(local(pattern.hir_id));
                    if let Some(child) = child {
                        pattern = child;
                    } else {
                        break;
                    }
                }
                _ => {
                    return Err(error(
                        pattern.span,
                        "code binding supports names, wildcards and whole-value aliases; code destructuring is not yet implemented",
                    ));
                }
            }
        }
        let initializer = decl
            .init
            .ok_or_else(|| error(decl.span, "comptime code requires an initializer"))?;
        let saved_locals = self.locals.clone();
        let in_code = std::mem::replace(&mut self.in_code_body, false);
        let value = (|| {
            let interface = decl
                .ty
                .map(|annotation| self.code_interface(annotation))
                .transpose()?;
            self.code_value(initializer, interface.as_ref(), 0)
        })();
        self.in_code_body = in_code;
        self.locals = saved_locals;
        let value = value?;
        if let Some(expected) = expected {
            self.require(&p(P::Void), expected, decl.span)?;
        }
        for id in bindings {
            if !decl.mutable {
                self.immutable.insert(id);
            }
            self.code_values.insert(id, value.clone());
        }
        Ok(n::Expr {
            ty: p(P::Void),
            source: source(decl.span),
            kind: K::Unit,
        })
    }

    fn code_interface(&mut self, annotation: &Expr<'_>) -> Result<Interface, Error> {
        let H::ContextualExprType(bindings, result) = &annotation.kind else {
            return Err(error(
                annotation.span,
                "code annotation requires an expression-code interface; Any evidence is not yet implemented",
            ));
        };
        let mut context = BTreeMap::new();
        let mut ports = BTreeMap::new();
        let mut defaults = BTreeMap::new();
        for entry in bindings {
            let ty = self.type_expr(entry.ty)?;
            if ty == p(P::Type)
                || matches!(ty, Term::ComptimeFunction(_) | Term::NormalFormFunction(..))
            {
                return Err(error(
                    entry.name.span,
                    "quote contexts contain only runtime bindings",
                ));
            }
            let id = local(entry.hir_id);
            if ports.insert(entry.name.name.to_string(), id).is_some() {
                return Err(error(entry.name.span, "duplicate code context name"));
            }
            if let Some(default) = entry.default {
                defaults.insert(id, binding(default)?);
            }
            context.insert(id, ty.clone());
            self.locals.insert(id, ty);
        }
        let result = self.type_expr(result)?;
        if result == p(P::Type)
            || matches!(
                result,
                Term::ComptimeFunction(_) | Term::NormalFormFunction(..)
            )
        {
            return Err(error(
                annotation.span,
                "expression code must deliver a runtime result",
            ));
        }
        Ok(Interface {
            infer_context: false,
            context,
            ports,
            defaults,
            result,
        })
    }

    fn code_value(
        &mut self,
        value: &Expr<'_>,
        interface: Option<&Interface>,
        depth: usize,
    ) -> Result<Arc<ExpressionCode>, Error> {
        self.code_value_in_context(value, interface, None, depth)
    }

    fn code_value_in_context(
        &mut self,
        value: &Expr<'_>,
        interface: Option<&Interface>,
        result_context: Option<&crate::calls::InputContext>,
        depth: usize,
    ) -> Result<Arc<ExpressionCode>, Error> {
        if self.code_construction_depth >= 32 {
            return Err(error(value.span, "code construction depth exhausted"));
        }
        self.code_construction_depth += 1;
        let result = self.code_value_inner(value, interface, result_context, depth);
        self.code_construction_depth -= 1;
        result
    }

    fn code_value_inner(
        &mut self,
        value: &Expr<'_>,
        interface: Option<&Interface>,
        result_context: Option<&crate::calls::InputContext>,
        depth: usize,
    ) -> Result<Arc<ExpressionCode>, Error> {
        if depth >= 64 || self.required_steps == 0 {
            return Err(error(value.span, "code construction budget exhausted"));
        }
        self.required_steps -= 1;
        match &value.kind {
            H::ResolvedClauseParam(_, id)
                if self.template_code_values.contains_key(&(id.pkg, id.index)) =>
            {
                self.ascribe_code(
                    self.template_code_values[&(id.pkg, id.index)].clone(),
                    interface,
                    value.span,
                )
            }
            H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) => {
                let code = self.code_values.get(&local(*id)).cloned().ok_or_else(|| {
                    error(
                        value.span,
                        "splice requires an actual comptime code value, not a runtime value",
                    )
                })?;
                self.ascribe_code(code, interface, value.span)
            }
            H::Application(callee, arguments) => {
                let code = self.invoke_code_generator(
                    value,
                    callee,
                    arguments,
                    interface,
                    result_context,
                )?;
                self.ascribe_code(code, interface, value.span)
            }
            H::ExtendedApplication(callee, arguments) => {
                let code = self.invoke_property_generator(
                    value,
                    callee,
                    arguments,
                    interface,
                    result_context,
                )?;
                self.ascribe_code(code, interface, value.span)
            }
            H::Block(block) => self.code_block(block, interface, result_context, depth + 1),
            H::Match(subject, arms) => self.generator_code_match(
                value,
                subject,
                arms,
                interface,
                result_context,
                depth + 1,
            ),
            H::InlineControl(inner)
                if matches!(inner.kind, H::If(..) | H::When(..) | H::Match(..)) =>
            {
                // Construction decisions already run at comptime; definition
                // checking must still validate the unselected branches.
                self.code_value_in_context(inner, interface, result_context, depth + 1)
            }
            H::When(arms) => {
                if !arms.iter().any(|arm| arm.cond.is_none()) {
                    return Err(error(value.span, "code-valued when requires an else arm"));
                }
                let mut first: Option<Arc<ExpressionCode>> = None;
                for arm in *arms {
                    let selected = self.generator_when_condition(arm.cond)?;
                    if self.checking_code_generator {
                        let code = self.generator_scope(arm.span, |this| {
                            this.code_value_in_context(
                                arm.body,
                                interface,
                                result_context,
                                depth + 1,
                            )
                        })?;
                        if let Some(previous) = &first {
                            self.require_code_branch(previous, code, arm.span)?;
                        } else {
                            first = Some(code);
                        }
                    } else if selected == Some(true) {
                        return self.generator_scope(arm.span, |this| {
                            this.code_value_in_context(
                                arm.body,
                                interface,
                                result_context,
                                depth + 1,
                            )
                        });
                    }
                }
                first.ok_or_else(|| error(value.span, "code-valued when selected no arm"))
            }
            H::Return(Some(value)) => {
                self.code_value_in_context(value, interface, result_context, depth + 1)
            }
            H::If(condition, yes, Some(no)) => {
                if self.checking_code_generator {
                    let checked = self.expr(condition, Some(&p(P::Bool)))?;
                    self.validate_generator_data(&checked, condition.span)?;
                    let yes = self.code_block(yes, interface, result_context, depth + 1)?;
                    let no =
                        self.code_value_in_context(no, interface, result_context, depth + 1)?;
                    self.require_code_branch(&yes, no, value.span)?;
                    Ok(yes)
                } else {
                    let (condition, _) =
                        self.generator_data_expression(condition, Some(&p(P::Bool)))?;
                    match condition {
                        comptime_interpretor::Value::Bool(true) => {
                            self.code_block(yes, interface, result_context, depth + 1)
                        }
                        comptime_interpretor::Value::Bool(false) => {
                            self.code_value_in_context(no, interface, result_context, depth + 1)
                        }
                        _ => Err(error(
                            value.span,
                            "code selection requires a comptime boolean",
                        )),
                    }
                }
            }
            H::CodeQuote(body, substitutions) => {
                self.quote_expression(value, body, substitutions, interface, result_context, None)
            }
            _ => Err(error(
                value.span,
                "local const declaration stage checking: this initializer is not yet a supported comptime code producer",
            )),
        }
    }

    fn quote_expression(
        &mut self,
        value: &Expr<'_>,
        body: &Expr<'_>,
        substitutions: &[hir::expr::CodeSubstitution<'_>],
        interface: Option<&Interface>,
        result_context: Option<&crate::calls::InputContext>,
        return_type: Option<Term>,
    ) -> Result<Arc<ExpressionCode>, Error> {
        // Ordinary quotes have no outer control targets. Template bodies may
        // explicitly supply the return type of their generated function.
        let checked = self.quoted_runtime_boundary(|checker| {
            checker.result = return_type;
            // Definition checking admits pattern interface evidence;
            // the quoted body is retained only after actual invocation.
            checker.pattern_quote_pending = Some(false);
            checker.checking_attempt(true, false, |checker| {
                if interface.is_none()
                    && let Some(context) = result_context
                {
                    checker.input_context_expression(body, context)
                } else {
                    checker.expr(body, interface.map(|interface| &interface.result))
                }
            })
        });
        let body = checked?.ok_or_else(|| {
            error(
                value.span,
                "quoted expression requires a result type context",
            )
        })?;
        constants::charge_copy(&body, &mut self.required_steps, value.span)?;
        let mut fuel = self.required_steps as usize;
        let free = nir::free_bindings::collect_with_dependencies(
            &body,
            &self.abstract_code_dependencies,
            &mut fuel,
        );
        self.required_steps = fuel as u64;
        let free = free.map_err(|message| error(value.span, message))?;
        let mut actual_free = BTreeSet::new();
        for id in free {
            if !(self.checking_code_generator && self.generator_data.contains_key(&id)) {
                actual_free.insert(id);
            }
        }
        let mut context = BTreeMap::new();
        for id in actual_free {
            if interface.is_some_and(|interface| {
                !interface.infer_context && !interface.context.contains_key(&id)
            }) {
                return Err(error(
                    value.span,
                    "quoted body requires a runtime binding absent from its declared context",
                ));
            }
            let ty = self.locals.get(&id).ok_or_else(|| {
                error(
                    value.span,
                    "quoted code has an unavailable runtime dependency",
                )
            })?;
            if *ty == p(P::Type)
                || matches!(ty, Term::ComptimeFunction(_) | Term::NormalFormFunction(..))
                || self.type_input_bindings.contains_key(&id)
                || self.value_input_bindings.contains_key(&id)
            {
                return Err(error(
                    value.span,
                    "quote contexts contain only runtime bindings; comptime data lifting is not yet implemented",
                ));
            }
            context.insert(id, ty.clone());
        }
        let mut defaults = context.keys().map(|&id| (id, id)).collect();
        let mut ports = BTreeMap::new();
        if let Some(interface) = interface.filter(|interface| !interface.infer_context) {
            context = interface.context.clone();
            ports = interface.ports.clone();
            defaults = interface.defaults.clone();
        }
        for entry in substitutions {
            let id = binding(entry.binding)?;
            if !context.contains_key(&id) {
                return Err(error(
                    entry.port.span,
                    "quote port must expose a runtime dependency of its body",
                ));
            }
            if ports.insert(entry.port.name.to_string(), id).is_some() {
                return Err(error(entry.port.span, "duplicate quote interface name"));
            }
        }
        let metadata = self.code_storage_metadata();
        let scope = self.call_inference_scope();
        let frames = vec![scope.iter().map(|parameter| parameter.sort).collect()];
        let result = body.ty.clone();
        let body = (!self.checking_code_generator).then(|| Arc::new(body));
        let code = ExpressionCode {
            body,
            result,
            context,
            ports,
            defaults,
        };
        code.validate_runtime_interface(&metadata, &frames)
            .map_err(|message| error(value.span, format!("quoted runtime interface: {message}")))?;
        Ok(Arc::new(code))
    }

    fn quoted_runtime_boundary<R>(
        &mut self,
        check: impl FnOnce(&mut Self) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let result = self.result.take();
        let returns = self.return_observations.take();
        let partial = self.partial_return_context.take();
        let optional = std::mem::take(&mut self.pending_optional_returns);
        let loops = std::mem::take(&mut self.loops);
        let captures = std::mem::take(&mut self.captures);
        let resume = self.effect_resume_result.take();
        let required = std::mem::replace(&mut self.in_required_comptime, false);
        let staged = std::mem::replace(&mut self.in_staged_input, false);
        let in_code = std::mem::replace(&mut self.in_code_body, true);
        let holes = std::mem::take(&mut self.quote_holes);
        let pattern_pending = self.pattern_quote_pending.take();
        let checked = check(self);
        self.pattern_quote_pending = pattern_pending;
        self.result = result;
        self.return_observations = returns;
        self.partial_return_context = partial;
        self.pending_optional_returns = optional;
        self.loops = loops;
        self.captures = captures;
        self.effect_resume_result = resume;
        self.in_required_comptime = required;
        self.in_staged_input = staged;
        self.in_code_body = in_code;
        self.quote_holes = holes;
        checked
    }

    fn require_runtime_code_position(&self, span: Span) -> Result<(), Error> {
        let comptime = self.in_required_comptime
            || self.in_staged_input
            || self
                .captures
                .last()
                .map(|frame| frame.comptime)
                .unwrap_or_else(|| {
                    self.signature_owner
                        .and_then(|owner| self.package.item(owner))
                        .is_some_and(|item| {
                            matches!(&item.kind, ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)
                        if sig.modifiers.is_comptime || sig.modifiers.is_normal_form_head)
                        })
                });
        if comptime && !self.in_code_body {
            return Err(error(
                span,
                "code splice requires a runtime position; quoted runtime code cannot execute in comptime",
            ));
        }
        Ok(())
    }

    pub(super) fn connect_code(
        &mut self,
        site: &Expr<'_>,
        operand: &Expr<'_>,
        substitutions: &[hir::expr::CodeSubstitution<'_>],
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        self.require_runtime_code_position(site.span)?;
        let in_code = std::mem::replace(&mut self.in_code_body, false);
        let code = self.code_value(operand, None, 0);
        self.in_code_body = in_code;
        let code = code?;
        let mut connections = BTreeMap::new();
        let mut names = BTreeSet::new();
        for entry in substitutions {
            if !names.insert(entry.port.name.to_string()) {
                return Err(error(entry.port.span, "duplicate splice interface name"));
            }
            let from = *code.ports.get(entry.port.name.as_str()).ok_or_else(|| {
                error(
                    entry.port.span,
                    "splice cannot redirect a fixed dependency or unknown code port",
                )
            })?;
            let to = binding(entry.binding)?;
            if connections
                .insert(from, to)
                .is_some_and(|previous| previous != to)
            {
                return Err(error(
                    entry.port.span,
                    "two names for the same code identity have conflicting substitutions",
                ));
            }
        }
        for (&from, ty) in &code.context {
            let to = *connections
                .entry(from)
                .or_insert_with(|| *code.defaults.get(&from).unwrap_or(&from));
            let actual = self.locals.get(&to).cloned().ok_or_else(|| {
                error(
                    site.span,
                    "splice is missing a required runtime binding; names do not supply identity",
                )
            })?;
            if self.type_input_bindings.contains_key(&to)
                || self.value_input_bindings.contains_key(&to)
            {
                return Err(error(
                    site.span,
                    "a code context port requires a runtime binding",
                ));
            }
            self.require(&actual, ty, site.span)?;
            // Splicing inside a closure imports actual dependencies, not the
            // compile-time code object. Creation of that object captures none.
            for frame in &mut self.captures {
                if frame.outer.contains(&to)
                    && !frame.values.iter().any(|capture| capture.binding == to)
                {
                    frame.values.push(n::Capture {
                        binding: to,
                        ty: actual.clone(),
                        value: None,
                    });
                }
            }
        }
        if let Some(expected) = expected {
            self.require_expression(&code.result, expected, site.span)?;
        }
        let Some(template) = &code.body else {
            if !self.checking_code_generator {
                return Err(error(
                    site.span,
                    "symbolic code evidence cannot be emitted as runtime code",
                ));
            }
            let id = n::LocalId {
                owner: u32::MAX,
                index: self.next_temporary_binding,
            };
            self.next_temporary_binding = self
                .next_temporary_binding
                .checked_add(1)
                .ok_or_else(|| error(site.span, "symbolic code identity space exhausted"))?;
            self.locals.insert(id, code.result.clone());
            self.abstract_code_dependencies
                .insert(id, connections.values().copied().collect());
            if let Some(pending) = &mut self.pattern_quote_pending {
                *pending = true;
            }
            return Ok(n::Expr {
                ty: code.result.clone(),
                source: source(site.span),
                kind: K::Local(id),
            });
        };
        // A template body can carry a return to the generated test. Inserting
        // that control transfer into a closure or an independent quote must
        // not silently retarget it to the innermost function.
        if self.result.is_none() || !self.captures.is_empty() {
            let mut fuel = self.required_steps as usize;
            let checked = nir::visit::expression_in_scope(template, &mut fuel, |node, _| {
                if matches!(node.kind, K::Return(_)) {
                    return Err(
                        "code return cannot cross a closure or quote boundary at its splice site"
                            .into(),
                    );
                }
                Ok(())
            });
            self.required_steps = fuel as u64;
            checked.map_err(|message| error(site.span, message))?;
        }
        constants::charge_copy(template, &mut self.required_steps, site.span)?;
        let mut body = (**template).clone();
        let mut next = self.next_temporary_binding;
        let result = freshen(
            &mut body,
            connections,
            &mut next,
            &mut self.expansion_sources,
            site.span,
        );
        self.next_temporary_binding = next;
        result?;
        let mut fuel = self.required_steps as usize;
        let checked = nir::visit::expression(&body, &mut fuel, |expression, _| {
            if let K::Assign { place, .. }
            | K::BitAssign { place, .. }
            | K::BitUpdate { place, .. } = &expression.kind
            {
                if self.immutable.contains(&place.root) {
                    return Err(
                        "code substitution cannot write to an immutable runtime binding".into(),
                    );
                }
            }
            Ok(())
        });
        self.required_steps = fuel as u64;
        checked.map_err(|message| error(site.span, message))?;
        Ok(body)
    }
}

/// Simultaneous alpha-renaming. Free identities are supplied by the connection;
/// every other local/control binder gets one new identity per expansion. This
/// includes patterns, default frames, closure captures and normal-form plans.
pub(super) fn freshen(
    body: &mut n::Expr,
    connections: BTreeMap<n::LocalId, n::LocalId>,
    next: &mut u32,
    sources: &mut Vec<n::ExpansionSource>,
    span: Span,
) -> Result<(), Error> {
    nir::freshening::expression(body, connections, next, sources, source(span))
        .map_err(|message| error(span, message))
}
