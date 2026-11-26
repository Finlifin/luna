//! Definition-bound expression code. Construction checks a runtime body but
//! publishes no runtime reads, moves or effects. Connection produces ordinary
//! NIR, which goes through the existing resource/effect verification pipeline.
use super::*;

mod function_templates;
mod functions;
pub(super) mod generators;
pub(super) mod patterns;
mod test_templates;

use comptime_interpretor::CodeValue;
pub(super) use functions::expand_quote_functions;
pub(crate) use functions::{expand_bound_quote, expand_materialized_quotes, inline_template};
pub(super) use generators::Generator;
use std::{fmt, sync::Arc};

use nir::code_interfaces::ExpressionCodeInterface as Interface;
pub(super) use nir::code_values::ExpressionCode;

pub(in crate::program::cmtt) enum AnyValue {
    Expression(Arc<ExpressionCode>),
    Pattern(Arc<patterns::PatternCode>),
    AbstractAny,
    Data(generators::Argument, Term),
}

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

    pub(super) fn code_declaration(
        &mut self,
        decl: &hir::decl::LetDecl<'_>,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let initializer = decl
            .init
            .ok_or_else(|| error(decl.span, "local const requires an initializer"))?;
        if self.pattern_code_producer(initializer, decl.ty)? {
            return self.pattern_code_binding(decl, expected);
        }
        if self.code_producer(initializer)?
            || decl
                .ty
                .is_some_and(|ty| matches!(ty.kind, H::ContextualExprType(..)))
        {
            return self.code_binding(decl, expected, false);
        }
        let is_any_ty = decl
            .ty
            .map(|ty| self.is_any_type_expr(ty))
            .transpose()?
            .unwrap_or(false);
        if (is_any_ty || self.any_code_producer(initializer)?)
            && let Some(expr) = self.try_any_code_declaration(decl, initializer, expected, false)?
        {
            return Ok(expr);
        }
        if decl.mutable {
            return Err(error(
                decl.span,
                "local const requires an immutable binding",
            ));
        }
        if self.checking_open_behavior
            || (self.in_unselected_inline && !self.checking_code_generator)
        {
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
        let is_any_ty = decl
            .ty
            .map(|annotation| self.is_any_type_expr(annotation))
            .transpose()?
            .unwrap_or(false);
        let (value, any_result_ty) = (|| {
            let interface = match decl.ty {
                Some(_) if is_any_ty => None,
                Some(annotation) => Some(self.code_interface(annotation)?),
                None => None,
            };
            let any_result_ty = is_any_ty
                || interface
                    .as_ref()
                    .is_some_and(|iface| iface.result == Term::AnyType);
            let value = self.code_value(initializer, interface.as_ref(), 0)?;
            Ok::<_, Error>((value, any_result_ty))
        })()
        .inspect_err(|_| {
            self.in_code_body = in_code;
            self.locals = saved_locals.clone();
        })?;
        self.in_code_body = in_code;
        self.locals = saved_locals;
        if let Some(expected) = expected {
            self.require(&p(P::Void), expected, decl.span)?;
        }
        for id in bindings {
            if !decl.mutable {
                self.immutable.insert(id);
            } else {
                self.immutable.remove(&id);
            }
            if is_any_ty && mutable {
                self.locals.insert(id, Term::AnyType);
            }
            if any_result_ty {
                self.any_result_code_locals.insert(id);
            } else {
                self.any_result_code_locals.remove(&id);
            }
            self.pattern_codes.remove(&id);
            self.generator_data.remove(&id);
            self.generator_cells.remove(&id);
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
                "code annotation requires an expression-code interface or Any",
            ));
        };
        let mut context = BTreeMap::new();
        let mut ports = BTreeMap::new();
        let mut defaults = BTreeMap::new();
        for entry in bindings {
            let ty = self.type_expr(entry.ty)?;
            if ty == p(P::Type)
                || ty.is_comptime_callable()
                || matches!(ty, Term::NormalFormFunction(..))
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
        let result = self.code_domain_or_any_type_expr(result)?;
        if result == p(P::Type)
            || result.is_comptime_callable()
            || matches!(result, Term::NormalFormFunction(..))
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
                self.visible_clause_input(*id, value.span)?;
                self.ascribe_code(
                    self.template_code_values[&(id.pkg, id.index)].clone(),
                    interface,
                    value.span,
                )
            }
            H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) => {
                let local_id = local(*id);
                if let Some(code) = self.code_values.shared(&local_id).cloned() {
                    self.ascribe_code(code, interface, value.span)
                } else if self.pattern_codes.contains_key(&local_id) {
                    Err(error(
                        value.span,
                        "expression splice cannot use pattern code; pattern code requires a pattern splice",
                    ))
                } else if self.locals.get(&local_id) == Some(&Term::AnyType) {
                    if self.checking_code_generator
                        && matches!(value.kind, H::ResolvedFnParam(..))
                        && self.generator_data.get(&local_id) == Some(&None)
                        && let Some(interface) = interface
                    {
                        Ok(Arc::new(ExpressionCode {
                            body: None,
                            result: interface.result.clone(),
                            context: interface.context.clone(),
                            ports: interface.ports.clone(),
                            defaults: BTreeMap::new(),
                        }))
                    } else if self.generator_data.contains_key(&local_id) {
                        Err(error(
                            value.span,
                            "splice requires an actual comptime code value, not ordinary data",
                        ))
                    } else {
                        Err(error(
                            value.span,
                            "splice requires an actual comptime code value, not an abstract any wildcard",
                        ))
                    }
                } else {
                    Err(error(
                        value.span,
                        "splice requires an actual comptime code value, not a runtime value",
                    ))
                }
            }
            H::TyAny => Err(error(
                value.span,
                "splice requires an actual comptime code value, not an abstract any wildcard",
            )),
            H::Builtin(name) if name.as_str() == "any" => Err(error(
                value.span,
                "splice requires an actual comptime code value, not an abstract any wildcard",
            )),
            H::Application(callee, arguments) => {
                let head = match callee.kind {
                    H::NFApplication(head, _) => head,
                    _ => callee,
                };
                if matches!(head.kind, H::ResolvedDef { def_id, .. }
                    if self.package.definition_owner(def_id)
                        .is_some_and(|owner| self.is_pattern_generator(owner)))
                {
                    return Err(error(
                        value.span,
                        "expression splice cannot use pattern code; pattern code requires a pattern splice",
                    ));
                }
                if self.generator_owner(callee)?.is_none()
                    && self
                        .generic_code_method_hint(callee)?
                        .is_none_or(|owner| !self.is_code_generator(owner))
                    && self.any_returning_comptime_owner(callee)?.is_some()
                {
                    let result =
                        self.invoke_any_code_generator(value, callee, arguments, depth + 1)?;
                    let code = match result {
                        AnyValue::Expression(code) => code,
                        AnyValue::Pattern(_) => {
                            return Err(error(
                                value.span,
                                "expression splice cannot use pattern code; pattern code requires a pattern splice",
                            ));
                        }
                        AnyValue::AbstractAny => {
                            return Err(error(
                                value.span,
                                "splice requires an actual comptime code value, not an abstract any wildcard",
                            ));
                        }
                        AnyValue::Data(..) => {
                            return Err(error(
                                value.span,
                                "splice requires an actual comptime code value, not ordinary data",
                            ));
                        }
                    };
                    return self.ascribe_code(code, interface, value.span);
                }
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
            checker.control.result = return_type;
            // Definition checking admits pattern interface evidence;
            // the quoted body is retained only after actual invocation.
            checker.pattern_quote_pending = Some(false);
            checker.checking_attempt(
                value.span,
                true,
                CheckingDisposition::RetainCode,
                |checker| {
                    if interface.is_none()
                        && let Some(context) = result_context
                    {
                        checker.input_context_expression(body, context)
                    } else {
                        let expected = interface
                            .map(|interface| &interface.result)
                            .filter(|ty| **ty != Term::AnyType);
                        checker.expr(body, expected)
                    }
                },
            )
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
                || ty.is_comptime_callable()
                || matches!(ty, Term::NormalFormFunction(..))
                || self.input_facts.has_identity(&id)
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
        let saved_control = std::mem::take(&mut self.control);
        let captures = std::mem::take(&mut self.captures);
        let resume = self.effect_resume_result.take();
        let required = std::mem::replace(&mut self.in_required_comptime, false);
        let staged = std::mem::replace(&mut self.in_staged_input, false);
        let in_code = std::mem::replace(&mut self.in_code_body, true);
        let holes = std::mem::take(&mut self.quote_holes);
        let pattern_pending = self.pattern_quote_pending.take();
        let checked = check(self);
        self.pattern_quote_pending = pattern_pending;
        self.control = saved_control;
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
            if self.input_facts.has_identity(&to) {
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
        if self.control.result.is_none() || !self.captures.is_empty() {
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
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let runtime_checked = nir::visit::expression(&body, &mut fuel, |node, fuel| {
            nir::type_computations::charge_term(&node.ty, fuel).map_err(|e| e.to_string())?;
            for rt_id in node.ty.runtime_dependencies() {
                if let Some(delivery) = self.local_runtime_deliveries.get(&rt_id) {
                    for binding in nir::free_bindings::collect(&delivery.expr, fuel)? {
                        if connections
                            .get(&binding)
                            .is_some_and(|&redirected| redirected != binding)
                        {
                            return Err(
                                "splice cannot redirect a binding captured by a local runtime-dependent type"
                                    .into(),
                            );
                        }
                    }
                }
            }
            Ok(())
        });
        self.required_steps = fuel as u64;
        runtime_checked.map_err(|message| error(site.span, message))?;
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

    pub(in crate::program::cmtt) fn try_any_code_declaration(
        &mut self,
        decl: &hir::decl::LetDecl<'_>,
        initializer: &Expr<'_>,
        expected: Option<&Term>,
        mutable: bool,
    ) -> Result<Option<n::Expr>, Error> {
        if !self.any_code_producer(initializer)? {
            return Ok(None);
        }
        if (decl.mutable && !mutable) || decl.guard.is_some() || decl.otherwise.is_some() {
            return Err(error(
                decl.span,
                "quoted code requires an unconditional immutable comptime binding",
            ));
        }
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
        let saved_locals = self.locals.clone();
        let in_code = std::mem::replace(&mut self.in_code_body, false);
        let value = self.any_code_value(initializer, 0);
        self.in_code_body = in_code;
        self.locals = saved_locals;
        let value = value?;
        if let AnyValue::Data(..) = &value {
            if !mutable {
                return Ok(None);
            }
        }
        if let Some(expected) = expected {
            self.require(&p(P::Void), expected, decl.span)?;
        }
        match value {
            AnyValue::Expression(code) => {
                for id in bindings {
                    if !decl.mutable {
                        self.immutable.insert(id);
                    } else {
                        self.immutable.remove(&id);
                    }
                    if mutable {
                        self.locals.insert(id, Term::AnyType);
                    }
                    self.pattern_codes.remove(&id);
                    self.generator_data.remove(&id);
                    self.generator_cells.remove(&id);
                    self.code_values.insert(id, code.clone());
                }
            }
            AnyValue::Pattern(code) => {
                self.validate_pattern_code(&code, initializer.span)?;
                for id in bindings {
                    if !decl.mutable {
                        self.immutable.insert(id);
                    } else {
                        self.immutable.remove(&id);
                    }
                    if mutable {
                        self.locals.insert(id, Term::AnyType);
                    }
                    self.code_values.remove(&id);
                    self.generator_data.remove(&id);
                    self.generator_cells.remove(&id);
                    self.pattern_codes.insert(id, code.clone());
                }
            }
            AnyValue::AbstractAny => {
                for id in bindings {
                    if !decl.mutable {
                        self.immutable.insert(id);
                    } else {
                        self.immutable.remove(&id);
                    }
                    self.code_values.remove(&id);
                    self.pattern_codes.remove(&id);
                    self.generator_data.remove(&id);
                    self.generator_cells.remove(&id);
                    self.locals.insert(id, Term::AnyType);
                }
            }
            AnyValue::Data(arg, ty) => {
                for id in bindings {
                    if !decl.mutable {
                        self.immutable.insert(id);
                    } else {
                        self.immutable.remove(&id);
                    }
                    self.code_values.remove(&id);
                    self.pattern_codes.remove(&id);
                    match &arg {
                        generators::Argument::Data(expr) => {
                            self.generator_cells.remove(&id);
                            self.generator_data.insert(id, expr.clone());
                        }
                        generators::Argument::LiveData(live) => {
                            self.install_live_generator_input(id, &ty, live.clone(), decl.span);
                        }
                        generators::Argument::Code(_) | generators::Argument::CodeCollection(_) => {
                        }
                    }
                    self.locals.insert(id, Term::AnyType);
                }
            }
        }
        Ok(Some(n::Expr {
            ty: p(P::Void),
            source: source(decl.span),
            kind: K::Unit,
        }))
    }

    pub(in crate::program::cmtt) fn any_code_value(
        &mut self,
        value: &Expr<'_>,
        depth: usize,
    ) -> Result<AnyValue, Error> {
        if depth >= 64 || self.required_steps == 0 {
            return Err(error(value.span, "code construction budget exhausted"));
        }
        self.required_steps -= 1;
        if Self::is_abstract_any_value(value) {
            return Ok(AnyValue::AbstractAny);
        }
        match &value.kind {
            H::CodeQuote(body, substitutions) => self
                .quote_expression(value, body, substitutions, None, None, None)
                .map(AnyValue::Expression),
            H::CodePatternQuote(..) => self
                .pattern_generator_value_inferred(value, None, None, None, depth + 1)
                .map(AnyValue::Pattern),
            H::ResolvedClauseParam(_, id)
                if self.template_code_values.contains_key(&(id.pkg, id.index)) =>
            {
                self.visible_clause_input(*id, value.span)?;
                Ok(AnyValue::Expression(
                    self.template_code_values[&(id.pkg, id.index)].clone(),
                ))
            }
            H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) => {
                let local_id = local(*id);
                if let Some(code) = self.code_values.shared(&local_id).cloned() {
                    Ok(AnyValue::Expression(code))
                } else if let Some(code) = self.pattern_codes.shared(&local_id).cloned() {
                    Ok(AnyValue::Pattern(code))
                } else if self.generator_data.contains_key(&local_id) {
                    if self.checking_code_generator {
                        let ty = self.locals.get(&local_id).cloned().unwrap_or(Term::AnyType);
                        Ok(AnyValue::Data(generators::Argument::Data(None), ty))
                    } else {
                        let (data, ty) = self.generator_data_expression(value, None)?;
                        let arg = if self.generator_value_needs_live_storage(&data, value.span)? {
                            generators::Argument::LiveData(data)
                        } else {
                            generators::Argument::Data(Some(self.materialize_generator_data(
                                data,
                                ty.clone(),
                                value.span,
                            )?))
                        };
                        Ok(AnyValue::Data(arg, ty))
                    }
                } else if self.locals.get(&local_id) == Some(&Term::AnyType) {
                    Ok(AnyValue::AbstractAny)
                } else {
                    Err(error(
                        value.span,
                        "splice requires an actual comptime code value, not a runtime value",
                    ))
                }
            }
            H::Application(callee, arguments) => {
                let head = match callee.kind {
                    H::NFApplication(head, _) => head,
                    _ => callee,
                };
                if matches!(head.kind, H::ResolvedDef { def_id, .. }
                    if self.package.definition_owner(def_id)
                        .is_some_and(|owner| self.is_pattern_generator(owner)))
                {
                    return self
                        .invoke_pattern_generator(callee, arguments, None, None, value.span)
                        .map(AnyValue::Pattern);
                }
                if self.generator_owner(callee)?.is_some()
                    || self
                        .generic_code_method_hint(callee)?
                        .is_some_and(|owner| self.is_code_generator(owner))
                {
                    return self
                        .invoke_code_generator(value, callee, arguments, None, None)
                        .map(AnyValue::Expression);
                }
                if self.any_returning_comptime_owner(callee)?.is_some() {
                    return self.invoke_any_code_generator(value, callee, arguments, depth + 1);
                }
                self.any_data_fallback(value)
            }
            H::ExtendedApplication(callee, arguments) => {
                let head = match callee.kind {
                    H::NFApplication(head, _) => head,
                    _ => callee,
                };
                if matches!(head.kind, H::ResolvedDef { def_id, .. }
                    if self.package.definition_owner(def_id)
                        .is_some_and(|owner| self.is_pattern_generator(owner)))
                {
                    return self
                        .invoke_pattern_properties(value, callee, arguments, None, None)
                        .map(AnyValue::Pattern);
                }
                if self.generator_owner(callee)?.is_some()
                    || self
                        .generic_code_method_hint(callee)?
                        .is_some_and(|owner| self.is_code_generator(owner))
                {
                    return self
                        .invoke_property_generator(value, callee, arguments, None, None)
                        .map(AnyValue::Expression);
                }
                self.any_data_fallback(value)
            }
            H::Block(block) => self.any_code_block(block, depth + 1),
            H::Return(Some(inner)) => self.any_code_value(inner, depth + 1),
            H::InlineControl(inner)
                if matches!(inner.kind, H::If(..) | H::When(..) | H::Match(..)) =>
            {
                self.any_code_value(inner, depth + 1)
            }
            H::If(condition, yes, Some(no)) => {
                if self.checking_code_generator {
                    let checked = self.expr(condition, Some(&p(P::Bool)))?;
                    self.validate_generator_data(&checked, condition.span)?;
                    let yes_val = self.any_code_block(yes, depth + 1)?;
                    let no_val = self.any_code_value(no, depth + 1)?;
                    Ok(Self::join_any_values(yes_val, no_val))
                } else {
                    let (condition_val, _) =
                        self.generator_data_expression(condition, Some(&p(P::Bool)))?;
                    match condition_val {
                        comptime_interpretor::Value::Bool(true) => {
                            self.any_code_block(yes, depth + 1)
                        }
                        comptime_interpretor::Value::Bool(false) => {
                            self.any_code_value(no, depth + 1)
                        }
                        _ => Err(error(
                            value.span,
                            "code selection requires a comptime boolean",
                        )),
                    }
                }
            }
            H::When(arms) => {
                if !arms.iter().any(|arm| arm.cond.is_none()) {
                    return Err(error(value.span, "code-valued when requires an else arm"));
                }
                if self.checking_code_generator {
                    let mut joined: Option<AnyValue> = None;
                    for arm in *arms {
                        let _ = self.generator_when_condition(arm.cond)?;
                        let arm_val = self.generator_scope(arm.span, |this| {
                            this.any_code_value(arm.body, depth + 1)
                        })?;
                        joined = Some(match joined {
                            None => arm_val,
                            Some(previous) => Self::join_any_values(previous, arm_val),
                        });
                    }
                    return joined
                        .ok_or_else(|| error(value.span, "code-valued when selected no arm"));
                }
                for arm in *arms {
                    let selected = self.generator_when_condition(arm.cond)?;
                    if selected == Some(true) {
                        return self.generator_scope(arm.span, |this| {
                            this.any_code_value(arm.body, depth + 1)
                        });
                    }
                }
                Err(error(value.span, "code-valued when selected no arm"))
            }
            H::Match(subject, arms) => {
                let mut results =
                    self.generator_match(value, subject, arms, None, |this, _, body| {
                        this.any_code_value(body, depth + 1)
                    })?;
                results
                    .drain(..)
                    .next()
                    .ok_or_else(|| error(value.span, "code-valued match selected no arm"))
            }
            _ => self.any_data_fallback(value),
        }
    }

    fn any_data_fallback(&mut self, value: &Expr<'_>) -> Result<AnyValue, Error> {
        if self.checking_code_generator {
            let checked = self.expr(value, None)?;
            self.validate_generator_data(&checked, value.span)?;
            Ok(AnyValue::Data(generators::Argument::Data(None), checked.ty))
        } else {
            let (data, ty) = self.generator_data_expression(value, None)?;
            let arg = if self.generator_value_needs_live_storage(&data, value.span)? {
                generators::Argument::LiveData(data)
            } else {
                generators::Argument::Data(Some(self.materialize_generator_data(
                    data,
                    ty.clone(),
                    value.span,
                )?))
            };
            Ok(AnyValue::Data(arg, ty))
        }
    }

    fn any_code_block(&mut self, block: &hir::Block<'_>, depth: usize) -> Result<AnyValue, Error> {
        self.generator_scope(block.span, |this| {
            let mut loops = Vec::new();
            let mut sequence = constants::SchemaSequence::default();
            for statement in block.stmts {
                let next = this.generator_statement_in_sequence(
                    statement,
                    generators::statements::ResultInterface::Any,
                    depth + 1,
                    &mut loops,
                    &mut sequence,
                )?;
                match next {
                    generators::statements::Control::AnyReturn(val) => return Ok(val),
                    generators::statements::Control::Next => {}
                    _ => {
                        return Err(error(
                            statement.span,
                            "generator loop transfer escaped its loop",
                        ));
                    }
                }
            }
            if let Some(value) = block.expr {
                return this.any_code_value(value, depth + 1);
            }
            Ok(AnyValue::Data(
                generators::Argument::Data(Some(n::Expr {
                    source: source(block.span),
                    ty: p(P::Void),
                    kind: K::Unit,
                })),
                p(P::Void),
            ))
        })
    }

    pub(in crate::program::cmtt) fn invoke_any_code_generator(
        &mut self,
        site: &Expr<'_>,
        callee: &Expr<'_>,
        arguments: &[Arg<'_>],
        depth: usize,
    ) -> Result<AnyValue, Error> {
        let owner = self
            .any_returning_comptime_owner(callee)?
            .ok_or_else(|| error(callee.span, "expected an Any-returning comptime function"))?;
        let template = self.form_contextual_generator(owner, site.span, |this, output| {
            output
                .map(|output| this.code_domain_or_any_type_expr(output))
                .unwrap_or(Ok(Term::AnyType))
        })?;
        let mut packets = generators::spreads::SpreadPlan::default();
        let (definition, values) = self.prepare_contextual_inputs(
            owner,
            template,
            site,
            callee,
            arguments,
            None,
            |ty| ty.clone(),
            None,
            &mut packets,
            None,
        )?;
        self.generator_frame(owner, |this| {
            this.run_any_code_generator(&definition, values, site.span, depth + 1)
        })
    }

    fn run_any_code_generator(
        &mut self,
        definition: &Generator<Term>,
        arguments: Vec<Option<generators::Argument>>,
        span: Span,
        depth: usize,
    ) -> Result<AnyValue, Error> {
        let owner = self
            .signature_owner
            .ok_or_else(|| error(span, "missing Any generator owner"))?;
        let item = self
            .package
            .item(owner)
            .ok_or_else(|| error(span, "missing Any generator declaration"))?;
        let (signature, body_id) = match &item.kind {
            ItemKind::Fn(sig, body) => (sig, Some(*body)),
            ItemKind::FnDeclaration(sig) => (sig, None),
            _ => return Err(error(span, "Any generator requires a function declaration")),
        };
        self.install_generator_capture_values(&definition.captures, span)?;
        self.alias_substitution.extend(
            definition
                .types
                .iter()
                .map(|(id, value)| (*id, Arc::new(value.clone()))),
        );
        self.install_generator_clause_values(owner, &definition.types, &definition.clause_domains)?;
        for (slot, ((id, input), argument)) in definition.inputs.iter().zip(arguments).enumerate() {
            let value = argument
                .ok_or_else(|| error(signature.params[slot].span, "missing generator argument"))?;
            self.mark_generator_input_binding(owner, *id);
            self.install_generator_type_input(*id, definition.input_value(slot));
            match (input, value) {
                (nir::code_interfaces::CodeInput::Data(ty), generators::Argument::Data(val)) => {
                    self.locals.insert(*id, ty.clone());
                    self.generator_data.insert(*id, val);
                }
                (
                    nir::code_interfaces::CodeInput::Data(ty),
                    generators::Argument::LiveData(val),
                ) => {
                    self.install_live_generator_input(*id, ty, val, signature.params[slot].span);
                }
                (
                    nir::code_interfaces::CodeInput::Data(ty),
                    generators::Argument::Code(CodeValue::Expression(code)),
                ) if *ty == Term::AnyType => {
                    self.locals.insert(*id, Term::AnyType);
                    self.code_values.insert(*id, code);
                }
                (
                    nir::code_interfaces::CodeInput::Data(ty),
                    generators::Argument::Code(CodeValue::Pattern(code)),
                ) if *ty == Term::AnyType => {
                    self.locals.insert(*id, Term::AnyType);
                    self.pattern_codes.insert(*id, code);
                }
                (
                    nir::code_interfaces::CodeInput::Code(interface),
                    generators::Argument::Code(CodeValue::Expression(code)),
                ) => {
                    let code =
                        self.ascribe_code(code, Some(interface), signature.params[slot].span)?;
                    self.code_values.insert(*id, code);
                }
                (
                    nir::code_interfaces::CodeInput::Pattern(interface),
                    generators::Argument::Code(CodeValue::Pattern(code)),
                ) => {
                    self.ascribe_pattern_input(&code, interface, signature.params[slot].span)?;
                    self.pattern_codes.insert(*id, code);
                }
                _ => {
                    return Err(error(
                        signature.params[slot].span,
                        "Any comptime generator argument category mismatch",
                    ));
                }
            }
        }
        let body_id = body_id.ok_or_else(|| error(span, "Any generator requires a body"))?;
        let body = self
            .package
            .body(body_id)
            .ok_or_else(|| error(span, "missing Any generator body"))?;
        self.any_code_value(body.value, depth + 1)
    }

    fn join_any_values(left: AnyValue, right: AnyValue) -> AnyValue {
        match (left, right) {
            (AnyValue::Expression(y), AnyValue::Expression(n))
                if y.result == n.result && y.context == n.context && y.ports == n.ports =>
            {
                AnyValue::Expression(y)
            }
            (AnyValue::Pattern(y), AnyValue::Pattern(n))
                if y.subject == n.subject && y.outputs == n.outputs =>
            {
                AnyValue::Pattern(y)
            }
            (AnyValue::Data(a, ty_a), AnyValue::Data(_, ty_b)) if ty_a == ty_b => {
                AnyValue::Data(a, ty_a)
            }
            _ => AnyValue::AbstractAny,
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(in crate::program::cmtt) fn assemble_code_sequence(
        &mut self,
        elements: &[Arc<ExpressionCode>],
        interface: Option<&Interface>,
        span: Span,
    ) -> Result<Arc<ExpressionCode>, Error> {
        let metadata = self.code_storage_metadata();
        let scope = self.call_inference_scope();
        let frames: Vec<Vec<Sort>> = vec![scope.iter().map(|parameter| parameter.sort).collect()];
        let check_return_escape = self.control.result.is_none() || !self.captures.is_empty();
        let checking = self.checking_code_generator;
        let immutable = &self.immutable;
        let mut next_binding = self.next_temporary_binding;
        let mut steps = self.required_steps;
        let mut sources = std::mem::take(&mut self.expansion_sources);
        let res = assemble_code_sequence_with(
            elements,
            interface,
            span,
            checking,
            check_return_escape,
            immutable,
            &metadata,
            &frames,
            &mut next_binding,
            &mut sources,
            &mut steps,
            |actual, expected| self.require(actual, expected, span),
        );
        self.next_temporary_binding = next_binding;
        self.expansion_sources = sources;
        self.required_steps = steps;
        let assembled = res?;
        self.ascribe_code(assembled, interface, span)
    }
}

#[allow(clippy::too_many_arguments)]
#[cfg_attr(not(test), allow(dead_code))]
fn assemble_code_sequence_with(
    elements: &[Arc<ExpressionCode>],
    interface: Option<&Interface>,
    span: Span,
    checking_code_generator: bool,
    check_return_escape: bool,
    immutable: &BTreeSet<n::LocalId>,
    metadata: &n::Package,
    frames: &[Vec<Sort>],
    next_temporary_binding: &mut u32,
    expansion_sources: &mut Vec<n::ExpansionSource>,
    required_steps: &mut u64,
    mut require_ty: impl FnMut(&Term, &Term) -> Result<(), Error>,
) -> Result<Arc<ExpressionCode>, Error> {
    *required_steps = required_steps
        .checked_sub(elements.len().saturating_add(1) as u64)
        .ok_or_else(|| error(span, "code sequence assembly budget exhausted"))?;
    if elements.is_empty() {
        return Ok(Arc::new(ExpressionCode {
            body: (!checking_code_generator).then(|| {
                Arc::new(n::Expr {
                    ty: p(P::Void),
                    source: source(span),
                    kind: K::Unit,
                })
            }),
            result: p(P::Void),
            context: BTreeMap::new(),
            ports: BTreeMap::new(),
            defaults: BTreeMap::new(),
        }));
    }
    let explicit_target = interface.filter(|target| !target.infer_context);
    let mut merged_context = BTreeMap::new();
    let mut merged_ports = BTreeMap::new();
    let mut merged_defaults = BTreeMap::new();
    let mut conflicted_defaults = BTreeSet::new();
    let mut port_ids: BTreeMap<String, (n::LocalId, Term)> = BTreeMap::new();
    let mut lowered = Vec::with_capacity(elements.len());

    for element in elements {
        let mut connections = BTreeMap::new();
        for (from, ty) in &element.context {
            let port_names = element
                .ports
                .iter()
                .filter(|(_, id)| *id == from)
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>();
            let target_id = if let Some(target) = explicit_target {
                if port_names.is_empty() {
                    let expected_ty = target.context.get(from).ok_or_else(|| {
                        error(
                            span,
                            "code sequence element requires a fixed dependency absent from target context",
                        )
                    })?;
                    require_ty(ty, expected_ty)?;
                    *from
                } else {
                    let mut resolved = None;
                    for name in &port_names {
                        let port_id = *target.ports.get(name).ok_or_else(|| {
                            error(
                                span,
                                format!("code sequence missing target context port {name}"),
                            )
                        })?;
                        require_ty(ty, &target.context[&port_id])?;
                        if let Some(prev) = resolved
                            && prev != port_id
                        {
                            return Err(error(
                                span,
                                "code sequence port aliases map to distinct target bindings",
                            ));
                        }
                        resolved = Some(port_id);
                    }
                    resolved
                        .ok_or_else(|| error(span, "code sequence target port resolution failed"))?
                }
            } else if port_names.is_empty() {
                if let Some(existing) = merged_context.get(from) {
                    require_ty(ty, existing)?;
                } else {
                    merged_context.insert(*from, ty.clone());
                }
                if let Some(&default_id) = element.defaults.get(from) {
                    match merged_defaults.entry(*from) {
                        std::collections::btree_map::Entry::Vacant(slot)
                            if !conflicted_defaults.contains(from) =>
                        {
                            slot.insert(default_id);
                        }
                        std::collections::btree_map::Entry::Occupied(slot)
                            if *slot.get() != default_id =>
                        {
                            slot.remove();
                            conflicted_defaults.insert(*from);
                        }
                        _ => {}
                    }
                }
                *from
            } else {
                let canonical =
                    if let Some((existing_id, existing_ty)) = port_ids.get(&port_names[0]) {
                        require_ty(ty, existing_ty)?;
                        *existing_id
                    } else {
                        let candidate = if merged_context.contains_key(from) {
                            let fresh = n::LocalId {
                                owner: u32::MAX,
                                index: *next_temporary_binding,
                            };
                            *next_temporary_binding =
                                next_temporary_binding.checked_add(1).ok_or_else(|| {
                                    error(span, "code sequence context identity space exhausted")
                                })?;
                            fresh
                        } else {
                            *from
                        };
                        merged_context.insert(candidate, ty.clone());
                        candidate
                    };
                for name in &port_names {
                    if let Some((prev_id, prev_ty)) = port_ids.get(name) {
                        if *prev_id != canonical {
                            return Err(error(
                                span,
                                "code sequence element mixes incompatible context ports",
                            ));
                        }
                        require_ty(ty, prev_ty)?;
                    } else {
                        port_ids.insert(name.clone(), (canonical, ty.clone()));
                        merged_ports.insert(name.clone(), canonical);
                    }
                }
                if let Some(&default_id) = element.defaults.get(from) {
                    match merged_defaults.entry(canonical) {
                        std::collections::btree_map::Entry::Vacant(slot)
                            if !conflicted_defaults.contains(&canonical) =>
                        {
                            slot.insert(default_id);
                        }
                        std::collections::btree_map::Entry::Occupied(slot)
                            if *slot.get() != default_id =>
                        {
                            slot.remove();
                            conflicted_defaults.insert(canonical);
                        }
                        _ => {}
                    }
                }
                canonical
            };
            connections.insert(*from, target_id);
        }

        if !checking_code_generator {
            let template = element
                .body
                .as_ref()
                .ok_or_else(|| error(span, "code sequence requires concrete code bodies"))?;
            if check_return_escape {
                let mut fuel = *required_steps as usize;
                let checked = nir::visit::expression_in_scope(template, &mut fuel, |node, _| {
                    if matches!(node.kind, K::Return(_)) {
                        return Err(
                            "code return cannot cross a closure or quote boundary at sequence splice"
                                .into(),
                        );
                    }
                    Ok(())
                });
                *required_steps = fuel as u64;
                checked.map_err(|message| error(span, message))?;
            }
            constants::charge_copy(template, required_steps, span)?;
            let mut body = (**template).clone();
            freshen(
                &mut body,
                connections,
                next_temporary_binding,
                expansion_sources,
                span,
            )?;
            let mut fuel = *required_steps as usize;
            let checked = nir::visit::expression(&body, &mut fuel, |expression, _| {
                if let K::Assign { place, .. }
                | K::BitAssign { place, .. }
                | K::BitUpdate { place, .. } = &expression.kind
                    && immutable.contains(&place.root)
                {
                    return Err("code sequence cannot write to an immutable runtime binding".into());
                }
                Ok(())
            });
            *required_steps = fuel as u64;
            checked.map_err(|message| error(span, message))?;
            lowered.push(body);
        }
    }

    let Some(last_element) = elements.last() else {
        return Err(error(span, "code sequence is missing final element"));
    };
    let result_ty = last_element.result.clone();
    let body = if checking_code_generator {
        None
    } else {
        let last_expr = lowered
            .pop()
            .ok_or_else(|| error(span, "code sequence is missing final expression"))?;
        Some(Arc::new(n::Expr {
            ty: result_ty.clone(),
            source: source(span),
            kind: K::Block {
                statements: lowered,
                result: Some(Box::new(last_expr)),
            },
        }))
    };
    let (context, ports, defaults) = if let Some(target) = explicit_target {
        (
            target.context.clone(),
            target.ports.clone(),
            target.defaults.clone(),
        )
    } else {
        (merged_context, merged_ports, merged_defaults)
    };
    let assembled = ExpressionCode {
        body,
        result: result_ty,
        context,
        ports,
        defaults,
    };
    if assembled.result != Term::AnyType {
        assembled
            .validate_runtime_interface(metadata, frames)
            .map_err(|message| {
                error(span, format!("code sequence runtime interface: {message}"))
            })?;
    }
    Ok(Arc::new(assembled))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustc_span::DUMMY_SP;

    #[test]
    fn assemble_code_sequence_merges_ports_freshens_binders_and_preserves_last_result() {
        let port_x = n::LocalId { owner: 1, index: 0 };
        let internal_a = n::LocalId { owner: 1, index: 1 };
        let internal_b = n::LocalId { owner: 2, index: 1 };
        let other_port_x = n::LocalId { owner: 2, index: 0 };

        let first = Arc::new(ExpressionCode {
            body: Some(Arc::new(n::Expr {
                ty: p(P::Void),
                source: source(DUMMY_SP),
                kind: K::Let {
                    binding: internal_a,
                    value: Box::new(n::Expr {
                        ty: p(P::I32),
                        source: source(DUMMY_SP),
                        kind: K::Local(port_x),
                    }),
                },
            })),
            result: p(P::Void),
            context: BTreeMap::from([(port_x, p(P::I32))]),
            ports: BTreeMap::from([("x".into(), port_x)]),
            defaults: BTreeMap::new(),
        });

        let second = Arc::new(ExpressionCode {
            body: Some(Arc::new(n::Expr {
                ty: p(P::I32),
                source: source(DUMMY_SP),
                kind: K::Block {
                    statements: vec![n::Expr {
                        ty: p(P::Void),
                        source: source(DUMMY_SP),
                        kind: K::Let {
                            binding: internal_b,
                            value: Box::new(n::Expr {
                                ty: p(P::I32),
                                source: source(DUMMY_SP),
                                kind: K::Local(other_port_x),
                            }),
                        },
                    }],
                    result: Some(Box::new(n::Expr {
                        ty: p(P::I32),
                        source: source(DUMMY_SP),
                        kind: K::Local(internal_b),
                    })),
                },
            })),
            result: p(P::I32),
            context: BTreeMap::from([(other_port_x, p(P::I32))]),
            ports: BTreeMap::from([("x".into(), other_port_x)]),
            defaults: BTreeMap::new(),
        });

        let metadata = n::Package::default();
        let immutable = BTreeSet::new();
        let mut next = 100;
        let mut sources = Vec::new();
        let mut steps = 10_000;
        let assembled = assemble_code_sequence_with(
            &[first, second],
            None,
            DUMMY_SP,
            false,
            true,
            &immutable,
            &metadata,
            &[vec![]],
            &mut next,
            &mut sources,
            &mut steps,
            |a, b| {
                if a == b {
                    Ok(())
                } else {
                    Err(error(DUMMY_SP, "mismatch"))
                }
            },
        )
        .expect("hygienic code sequence");
        assert_eq!(assembled.result, p(P::I32));
        assert_eq!(assembled.context.len(), 1);
        assert_eq!(assembled.ports.get("x"), Some(&port_x));
        let Some(body) = &assembled.body else {
            panic!("expected concrete assembled body");
        };
        let K::Block { statements, result } = &body.kind else {
            panic!("expected block body");
        };
        assert_eq!(statements.len(), 1);
        assert!(result.is_some());
        assert!(next > 100);
    }

    #[test]
    fn assemble_code_sequence_rejects_mismatched_port_types_and_return_escape() {
        let port_a = n::LocalId { owner: 1, index: 0 };
        let port_b = n::LocalId { owner: 2, index: 0 };
        let first = Arc::new(ExpressionCode {
            body: Some(Arc::new(n::Expr {
                ty: p(P::I32),
                source: source(DUMMY_SP),
                kind: K::Local(port_a),
            })),
            result: p(P::I32),
            context: BTreeMap::from([(port_a, p(P::I32))]),
            ports: BTreeMap::from([("x".into(), port_a)]),
            defaults: BTreeMap::new(),
        });
        let mismatched = Arc::new(ExpressionCode {
            body: Some(Arc::new(n::Expr {
                ty: p(P::Bool),
                source: source(DUMMY_SP),
                kind: K::Local(port_b),
            })),
            result: p(P::Bool),
            context: BTreeMap::from([(port_b, p(P::Bool))]),
            ports: BTreeMap::from([("x".into(), port_b)]),
            defaults: BTreeMap::new(),
        });

        let metadata = n::Package::default();
        let immutable = BTreeSet::new();
        let mut next = 100;
        let mut sources = Vec::new();
        let mut steps = 10_000;
        assert!(
            assemble_code_sequence_with(
                &[first.clone(), mismatched],
                None,
                DUMMY_SP,
                false,
                true,
                &immutable,
                &metadata,
                &[vec![]],
                &mut next,
                &mut sources,
                &mut steps,
                |a, b| if a == b {
                    Ok(())
                } else {
                    Err(error(DUMMY_SP, "mismatch"))
                },
            )
            .is_err()
        );

        let escaping = Arc::new(ExpressionCode {
            body: Some(Arc::new(n::Expr {
                ty: p(P::Never),
                source: source(DUMMY_SP),
                kind: K::Return(Some(Box::new(n::Expr {
                    ty: p(P::I32),
                    source: source(DUMMY_SP),
                    kind: K::Literal(n::Literal::SignedInteger(1)),
                }))),
            })),
            result: p(P::Never),
            context: BTreeMap::new(),
            ports: BTreeMap::new(),
            defaults: BTreeMap::new(),
        });
        assert!(
            assemble_code_sequence_with(
                &[first, escaping],
                None,
                DUMMY_SP,
                false,
                true,
                &immutable,
                &metadata,
                &[vec![]],
                &mut next,
                &mut sources,
                &mut steps,
                |a, b| if a == b {
                    Ok(())
                } else {
                    Err(error(DUMMY_SP, "mismatch"))
                },
            )
            .is_err()
        );
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
    freshen_with_renaming(body, connections, next, sources, span).map(|_| ())
}

pub(super) fn freshen_with_renaming(
    body: &mut n::Expr,
    connections: BTreeMap<n::LocalId, n::LocalId>,
    next: &mut u32,
    sources: &mut Vec<n::ExpansionSource>,
    span: Span,
) -> Result<BTreeMap<n::LocalId, n::LocalId>, Error> {
    nir::freshening::expression_with_renaming(body, connections, next, sources, source(span))
        .map_err(|message| error(span, message))
}
