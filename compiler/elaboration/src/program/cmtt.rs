//! Definition-bound expression code. Construction checks a runtime body but
//! publishes no runtime reads, moves or effects. Connection produces ordinary
//! NIR, which goes through the existing resource/effect verification pipeline.
use super::*;
mod generators;
mod functions;
pub(super) use functions::expand_quote_functions;
pub(crate) use functions::{expand_materialized_quotes,expand_bound_quote,inline_template};
pub(super) use generators::Generator;
use std::{fmt, sync::Arc};

#[derive(Debug)]
pub(super) struct ExpressionCode {
    body: Option<Arc<n::Expr>>,
    result: Term,
    /// All free runtime identities, including fixed dependencies.
    context: BTreeMap<n::LocalId, Term>,
    /// Only explicitly exposed ports may be redirected.
    ports: BTreeMap<String, n::LocalId>,
    defaults: BTreeMap<n::LocalId, n::LocalId>,
}

#[derive(Clone)]
struct Interface {
    /// Call-site expected result without rebinding a quote's lexical inputs.
    infer_context: bool,
    context: BTreeMap<n::LocalId, Term>,
    ports: BTreeMap<String, n::LocalId>,
    defaults: BTreeMap<n::LocalId, n::LocalId>,
    result: Term,
}

impl fmt::Display for Interface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(code-interface (infer-context {}) (context", self.infer_context)?;
        for (binding, ty) in &self.context { write!(f, " (binding {binding} {ty})")?; }
        f.write_str(") (ports")?;
        for (name, binding) in &self.ports { write!(f, " (port {name:?} {binding})")?; }
        f.write_str(") (defaults")?;
        for (port, actual) in &self.defaults { write!(f, " (connect {port} {actual})")?; }
        write!(f, ") (result {}))", self.result)
    }
}

impl fmt::Display for ExpressionCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(expression-code (context")?;
        for (binding, ty) in &self.context { write!(f, " (binding {binding} {ty})")?; }
        f.write_str(") (ports")?;
        for (name, binding) in &self.ports { write!(f, " (port {name:?} {binding})")?; }
        f.write_str(") (defaults")?;
        for (port, actual) in &self.defaults { write!(f, " (connect {port} {actual})")?; }
        match &self.body {
            Some(body) => write!(f, ") (result {}) (body {}))", self.result, body),
            None => write!(f, ") (result {}) (body pending-code-parameter))", self.result),
        }
    }
}

fn binding(value: &Expr<'_>) -> Result<n::LocalId, Error> {
    match value.kind {
        H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) => Ok(local(id)),
        _ => Err(error(value.span, "code substitution requires a resolved runtime binding")),
    }
}

impl Checker<'_, '_> {
    pub(super) fn freshen_constant_graph(&mut self,value:&mut n::Expr,span:Span)->Result<(),Error> {
        freshen(value,BTreeMap::new(),&mut self.next_temporary_binding,&mut self.expansion_sources,span)
    }

    pub(super) fn code_declaration(&mut self, decl: &hir::decl::LetDecl<'_>, expected: Option<&Term>) -> Result<n::Expr, Error> {
        let initializer=decl.init.ok_or_else(||error(decl.span,"local const requires an initializer"))?;
        if self.code_producer(initializer) || decl.ty.is_some_and(|ty|matches!(ty.kind,H::ContextualExprType(..))) {
            return self.code_binding(decl,expected,false);
        }
        if decl.mutable || decl.guard.is_some() || decl.otherwise.is_some() {
            return Err(error(decl.span,"local const currently requires an unconditional immutable binding"));
        }
        if let Some(expected)=expected {self.require(&p(P::Void),expected,decl.span)?;}
        let in_code=std::mem::replace(&mut self.in_code_body,false);
        let result=self.local_data_binding(decl);
        self.in_code_body=in_code;
        result?;
        Ok(n::Expr {ty:p(P::Void),source:source(decl.span),kind:K::Unit})
    }

    fn code_binding(&mut self, decl: &hir::decl::LetDecl<'_>, expected: Option<&Term>, mutable:bool) -> Result<n::Expr, Error> {
        if (decl.mutable && !mutable) || decl.guard.is_some() || decl.otherwise.is_some() {
            return Err(error(decl.span, "quoted code requires an unconditional immutable comptime binding"));
        }
        let hir::PatternKind::Binding(hir::common::BindingMode::ByValue, _, None) = decl.pattern.kind else {
            return Err(error(decl.span, "comptime code destructuring is not yet implemented"));
        };
        let initializer = decl.init.ok_or_else(|| error(decl.span, "comptime code requires an initializer"))?;
        let saved_locals = self.locals.clone();
        let in_code = std::mem::replace(&mut self.in_code_body, false);
        let value = (|| {
            let interface = decl.ty.map(|annotation| self.code_interface(annotation)).transpose()?;
            self.code_value(initializer, interface.as_ref(), 0)
        })();
        self.in_code_body = in_code;
        self.locals = saved_locals;
        let value = value?;
        if let Some(expected) = expected { self.require(&p(P::Void), expected, decl.span)?; }
        let id=local(decl.pattern.hir_id);
        if !decl.mutable {self.immutable.insert(id);}
        self.code_values.insert(id, value);
        Ok(n::Expr { ty: p(P::Void), source: source(decl.span), kind: K::Unit })
    }

    fn code_interface(&mut self, annotation: &Expr<'_>) -> Result<Interface, Error> {
        let H::ContextualExprType(bindings, result) = &annotation.kind else {
            return Err(error(annotation.span, "code annotation requires an expression-code interface; Any evidence is not yet implemented"));
        };
        let mut context = BTreeMap::new();
        let mut ports = BTreeMap::new();
        let mut defaults = BTreeMap::new();
        for entry in bindings {
            let ty = self.type_expr(entry.ty)?;
            if ty == p(P::Type) || matches!(ty, Term::ComptimeFunction(_)) {
                return Err(error(entry.name.span, "quote contexts contain only runtime bindings"));
            }
            let id = local(entry.hir_id);
            if ports.insert(entry.name.name.to_string(), id).is_some() {
                return Err(error(entry.name.span, "duplicate code context name"));
            }
            if let Some(default) = entry.default { defaults.insert(id, binding(default)?); }
            context.insert(id, ty.clone());
            self.locals.insert(id, ty);
        }
        let result = self.type_expr(result)?;
        if result == p(P::Type) || matches!(result, Term::ComptimeFunction(_)) {
            return Err(error(annotation.span, "expression code must deliver a runtime result"));
        }
        Ok(Interface {infer_context:false, context, ports, defaults, result})
    }

    fn code_value(&mut self, value: &Expr<'_>, interface: Option<&Interface>, depth: usize) -> Result<Arc<ExpressionCode>, Error> {
        if self.code_construction_depth >= 32 {
            return Err(error(value.span, "code construction depth exhausted"));
        }
        self.code_construction_depth += 1;
        let result = self.code_value_inner(value, interface, depth);
        self.code_construction_depth -= 1;
        result
    }

    fn code_value_inner(&mut self, value: &Expr<'_>, interface: Option<&Interface>, depth: usize) -> Result<Arc<ExpressionCode>, Error> {
        if depth >= 64 || self.required_steps == 0 {
            return Err(error(value.span, "code construction budget exhausted"));
        }
        self.required_steps -= 1;
        match &value.kind {
            H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) => {
                let code = self.code_values.get(&local(*id)).cloned()
                    .ok_or_else(|| error(value.span, "splice requires an actual comptime code value, not a runtime value"))?;
                self.ascribe_code(code, interface, value.span)
            }
            H::Application(callee, arguments) => {
                let code = self.invoke_code_generator(value, callee, arguments, interface)?;
                self.ascribe_code(code, interface, value.span)
            }
            H::ExtendedApplication(callee, arguments) => {
                let code = self.invoke_property_generator(value, callee, arguments, interface)?;
                self.ascribe_code(code, interface, value.span)
            }
            H::Block(block) => self.code_block(block, interface, depth + 1),
            H::Match(subject,arms) => self.generator_code_match(value,subject,arms,interface,depth+1),
            H::Return(Some(value)) => self.code_value(value, interface, depth + 1),
            H::If(condition, yes, Some(no)) => {
                if self.checking_code_generator {
                    let checked=self.expr(condition, Some(&p(P::Bool)))?;
                    self.validate_generator_data(&checked,condition.span)?;
                    let yes = self.code_block(yes, interface, depth + 1)?;
                    let no = self.code_value(no, interface, depth + 1)?;
                    self.require_expression(&yes.result, &no.result, value.span)?;
                    Ok(yes)
                } else {
                    let (condition, _) = self.required_typed_expression(condition, Some(&p(P::Bool)))?;
                    match condition {
                        comptime_interpretor::Value::Bool(true) => self.code_block(yes, interface, depth + 1),
                        comptime_interpretor::Value::Bool(false) => self.code_value(no, interface, depth + 1),
                        _ => Err(error(value.span, "code selection requires a comptime boolean")),
                    }
                }
            }
            H::CodeQuote(body, substitutions) => {
                // A quote owns a runtime boundary. Outer return/loop/resumption
                // targets are not captured or reinterpreted at a splice site.
                let result = self.result.take();
                let loops = std::mem::take(&mut self.loops);
                let captures = std::mem::take(&mut self.captures);
                let resume = self.effect_resume_result.take();
                let required = std::mem::replace(&mut self.in_required_comptime, false);
                let staged = std::mem::replace(&mut self.in_staged_input, false);
                let in_code = std::mem::replace(&mut self.in_code_body, true);
                let holes=std::mem::take(&mut self.quote_holes);
                let checked = self.checking_attempt(true, false, |checker| checker.expr(body, interface.map(|interface| &interface.result)));
                self.result = result;
                self.loops = loops;
                self.captures = captures;
                self.effect_resume_result = resume;
                self.in_required_comptime = required;
                self.in_staged_input = staged;
                self.in_code_body = in_code;
                self.quote_holes=holes;
                let body = checked?.ok_or_else(|| error(value.span, "quoted expression requires a result type context"))?;
                constants::charge_copy(&body, &mut self.required_steps, value.span)?;
                let mut fuel = self.required_steps as usize;
                let free = nir::free_bindings::collect(&body, &mut fuel);
                self.required_steps = fuel as u64;
                let free = free.map_err(|message| error(value.span, message))?;
                let mut actual_free = BTreeSet::new();
                for id in free {
                    if let Some(dependencies) = self.abstract_code_dependencies.get(&id) {
                        actual_free.extend(dependencies);
                    } else if !(self.checking_code_generator && self.generator_data.contains_key(&id)) {
                        actual_free.insert(id);
                    }
                }
                let mut context = BTreeMap::new();
                for id in actual_free {
                    if interface.is_some_and(|interface| !interface.infer_context && !interface.context.contains_key(&id)) {
                        return Err(error(value.span, "quoted body requires a runtime binding absent from its declared context"));
                    }
                    let ty = self.locals.get(&id).ok_or_else(|| error(value.span, "quoted code has an unavailable runtime dependency"))?;
                    if *ty == p(P::Type) || matches!(ty, Term::ComptimeFunction(_))
                        || self.type_input_bindings.contains_key(&id) || self.value_input_bindings.contains_key(&id) {
                        return Err(error(value.span, "quote contexts contain only runtime bindings; comptime data lifting is not yet implemented"));
                    }
                    context.insert(id, ty.clone());
                }
                let mut defaults = context.keys().map(|&id| (id,id)).collect();
                let mut ports = BTreeMap::new();
                if let Some(interface) = interface.filter(|interface| !interface.infer_context) {
                    context = interface.context.clone();
                    ports = interface.ports.clone();
                    defaults = interface.defaults.clone();
                }
                for entry in substitutions {
                    let id = binding(entry.binding)?;
                    if !context.contains_key(&id) { return Err(error(entry.port.span, "quote port must expose a runtime dependency of its body")); }
                    if ports.insert(entry.port.name.to_string(), id).is_some() {
                        return Err(error(entry.port.span, "duplicate quote interface name"));
                    }
                }
                let metadata = n::Package { structs: self.structs.values().cloned().collect(),
                    enums: self.enums.values().cloned().collect(), normal_forms: self.normal_form_metadata(), ..Default::default() };
                let scope = self.call_inference_scope();
                let frames = vec![scope.iter().map(|parameter| parameter.sort).collect()];
                for ty in context.values().chain(std::iter::once(&body.ty)) {
                    nir::phases::validate_runtime_storage(ty, &metadata, &frames)
                        .map_err(|message| error(value.span, format!("quoted runtime interface: {message}")))?;
                }
                let result = body.ty.clone();
                let body = (!self.checking_code_generator).then(|| Arc::new(body));
                Ok(Arc::new(ExpressionCode { body, result, context, ports, defaults }))
            }
            _ => Err(error(value.span, "local const declaration stage checking: this initializer is not yet a supported comptime code producer")),
        }
    }

    pub(super) fn connect_code(&mut self, site: &Expr<'_>, operand: &Expr<'_>, substitutions: &[hir::expr::CodeSubstitution<'_>], expected: Option<&Term>) -> Result<n::Expr, Error> {
        let comptime = self.in_required_comptime || self.in_staged_input
            || self.captures.last().map(|frame| frame.comptime).unwrap_or_else(||
                self.signature_owner.and_then(|owner| self.package.item(owner)).is_some_and(|item|
                    matches!(&item.kind, ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)
                        if sig.modifiers.is_comptime || sig.modifiers.is_normal_form_head)));
        if comptime && !self.in_code_body {
            return Err(error(site.span, "code splice requires a runtime position; quoted runtime code cannot execute in comptime"));
        }
        let in_code = std::mem::replace(&mut self.in_code_body, false);
        let code = self.code_value(operand, None, 0);
        self.in_code_body = in_code;
        let code = code?;
        let mut connections = BTreeMap::new();
        let mut names = BTreeSet::new();
        for entry in substitutions {
            if !names.insert(entry.port.name.to_string()) { return Err(error(entry.port.span, "duplicate splice interface name")); }
            let from = *code.ports.get(entry.port.name.as_str())
                .ok_or_else(|| error(entry.port.span, "splice cannot redirect a fixed dependency or unknown code port"))?;
            let to = binding(entry.binding)?;
            if connections.insert(from, to).is_some_and(|previous| previous != to) {
                return Err(error(entry.port.span, "two names for the same code identity have conflicting substitutions"));
            }
        }
        for (&from, ty) in &code.context {
            let to = *connections.entry(from).or_insert_with(|| *code.defaults.get(&from).unwrap_or(&from));
            let actual = self.locals.get(&to).cloned()
                .ok_or_else(|| error(site.span, "splice is missing a required runtime binding; names do not supply identity"))?;
            if self.type_input_bindings.contains_key(&to) || self.value_input_bindings.contains_key(&to) {
                return Err(error(site.span, "a code context port requires a runtime binding"));
            }
            self.require(&actual, ty, site.span)?;
            // Splicing inside a closure imports actual dependencies, not the
            // compile-time code object. Creation of that object captures none.
            for frame in &mut self.captures {
                if frame.outer.contains(&to) && !frame.values.iter().any(|capture| capture.binding == to) {
                    frame.values.push(n::Capture { binding: to, ty: actual.clone(), value: None });
                }
            }
        }
        if let Some(expected) = expected { self.require_expression(&code.result, expected, site.span)?; }
        let Some(template) = &code.body else {
            if !self.checking_code_generator {
                return Err(error(site.span, "symbolic code evidence cannot be emitted as runtime code"));
            }
            let id = n::LocalId { owner: u32::MAX, index: self.next_temporary_binding };
            self.next_temporary_binding = self.next_temporary_binding.checked_add(1)
                .ok_or_else(|| error(site.span, "symbolic code identity space exhausted"))?;
            self.locals.insert(id, code.result.clone());
            self.abstract_code_dependencies.insert(id, connections.values().copied().collect());
            return Ok(n::Expr {ty: code.result.clone(), source: source(site.span), kind: K::Local(id)});
        };
        constants::charge_copy(template, &mut self.required_steps, site.span)?;
        let mut body = (**template).clone();
        let mut next = self.next_temporary_binding;
        let result = freshen(&mut body, connections, &mut next, &mut self.expansion_sources, site.span);
        self.next_temporary_binding = next;
        result?;
        let mut fuel = self.required_steps as usize;
        let checked = nir::visit::expression(&body, &mut fuel, |expression, _| {
            if let K::Assign {place, ..} | K::BitAssign {place, ..} | K::BitUpdate {place, ..} = &expression.kind {
                if self.immutable.contains(&place.root) {
                    return Err("code substitution cannot write to an immutable runtime binding".into());
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
fn freshen(body: &mut n::Expr, connections: BTreeMap<n::LocalId, n::LocalId>, next: &mut u32, sources: &mut Vec<n::ExpansionSource>, span: Span) -> Result<(), Error> {
    body.rewrite_with(&mut Renamer { bindings: connections, next, sources, locations: BTreeMap::new(), span })
}

struct Renamer<'a> {
    bindings: BTreeMap<n::LocalId, n::LocalId>,
    next: &'a mut u32,
    sources: &'a mut Vec<n::ExpansionSource>,
    locations: BTreeMap<(u32,u32), n::SourceRange>,
    span: Span,
}
impl Renamer<'_> {
    fn fresh(&mut self) -> Result<n::LocalId, Error> {
        let fresh = n::LocalId { owner: u32::MAX, index: *self.next };
        *self.next = self.next.checked_add(1).ok_or_else(|| error(self.span, "code expansion identity space exhausted"))?;
        Ok(fresh)
    }
    fn rename(&mut self, id: &mut n::LocalId) -> Result<(), Error> {
        let mapped = if let Some(mapped) = self.bindings.get(id) { *mapped } else {
            let fresh = self.fresh()?;
            self.bindings.insert(*id, fresh);
            fresh
        };
        *id = mapped;
        Ok(())
    }
}
impl n::Rewriter for Renamer<'_> {
    type Error = Error;
    fn ty(&mut self, ty: &Term) -> Result<Term, Error> {
        let runtime_slots = ty.substitution_parameters().map_err(|failure| error(self.span, failure.to_string()))?
            .values().any(|sort| *sort == Sort::Runtime);
        if !ty.runtime_dependencies().is_empty() || runtime_slots {
            return Err(error(self.span, "runtime-dependent quoted type substitution is not yet implemented"));
        }
        Ok(ty.clone())
    }
    fn expression(&mut self, expression: &mut n::Expr) -> Result<bool, Error> {
        let original = expression.source;
        let generated = if let Some(generated) = self.locations.get(&(original.start, original.end)) { *generated } else {
            let index = u32::try_from(self.sources.len()).map_err(|_| error(self.span, "code expansion source space exhausted"))?;
            self.sources.push(n::ExpansionSource { original, site: source(self.span) });
            let generated = n::SourceRange::generated(index);
            self.locations.insert((original.start, original.end), generated);
            generated
        };
        expression.source = generated;
        match &mut expression.kind {
            K::Local(id) | K::Let { binding: id, .. } | K::Loop { identity: id, .. }
            | K::Break(id) | K::Continue(id) => self.rename(id)?,
            K::Assign { place, .. } | K::BitAssign { place, .. } | K::BitUpdate { place, .. } => self.rename(&mut place.root)?,
            K::Closure { parameters, captures, body } => {
                expression.ty = self.ty(&expression.ty)?;
                let mut inner = BTreeMap::new();
                for capture in captures {
                    capture.ty = self.ty(&capture.ty)?;
                    let original = capture.binding;
                    if let Some(value) = &mut capture.value { value.rewrite_with(self)?; }
                    else {
                        let mut actual = original;
                        self.rename(&mut actual)?;
                        capture.value = Some(Box::new(n::Expr {ty: capture.ty.clone(), source: generated, kind: K::Local(actual)}));
                    }
                    capture.binding = self.fresh()?;
                    inner.insert(original, capture.binding);
                }
                for parameter in parameters {
                    let fresh = self.fresh()?;
                    inner.insert(*parameter, fresh);
                    *parameter = fresh;
                }
                let saved = std::mem::replace(&mut self.bindings, inner);
                let result = Arc::make_mut(body).rewrite_with(self);
                self.bindings = saved;
                result?;
                return Ok(false);
            }
            K::Call { callee, arguments, parameter_bindings } => {
                expression.ty = self.ty(&expression.ty)?;
                callee.rewrite_with(self)?;
                for argument in arguments {
                    if argument.is_default {
                        // A selected callable owns this frame and its ABI. The
                        // caller's context substitution must never rename it.
                        let defaults = parameter_bindings.iter().map(|&id| (id,id)).collect();
                        let saved = std::mem::replace(&mut self.bindings, defaults);
                        let result = argument.value.rewrite_with(self);
                        self.bindings = saved;
                        result?;
                    } else { argument.value.rewrite_with(self)?; }
                }
                return Ok(false);
            }
            K::NormalFormType { plan: Some(plan), .. } => { for id in &mut plan.bindings { self.rename(id)?; } }
            _ => {}
        }
        Ok(true)
    }
    fn pattern(&mut self, pattern: &mut n::Pattern) -> Result<(), Error> {
        match pattern {
            n::Pattern::Binding { binding, .. } => self.rename(binding)?,
            n::Pattern::Remap { bindings, .. } => { for (from, to) in bindings { self.rename(from)?; self.rename(to)?; } }
            _ => {}
        }
        Ok(())
    }
}
