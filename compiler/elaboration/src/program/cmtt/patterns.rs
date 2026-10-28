//! Checked pattern code stays at comptime; each splice creates a fresh
//! ordinary NIR pattern and publishes only its success outputs.
use super::*;
mod generators;
pub(in crate::program) use generators::{PatternGenerator, PatternInterface};

#[derive(Debug)]
pub(in crate::program) struct PatternCode {
    pub(super) subject: Term,
    pub(super) outputs: BTreeMap<String, (n::LocalId, Term)>,
    /// Definition-side data inputs may be unavailable. Such evidence carries
    /// the checked interface, never an executable pattern with live captures.
    pub(super) body: Option<n::Pattern>,
}

impl fmt::Display for PatternCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(pattern-code (subject {}) (outputs", self.subject)?;
        for (name, (binding, ty)) in &self.outputs {
            write!(f, " (output {name:?} {binding} {ty})")?;
        }
        match &self.body {
            Some(body) => write!(f, ") (body {body}))"),
            None => f.write_str(") (body pending-pattern-code))"),
        }
    }
}

impl Checker<'_, '_> {
    pub(in crate::program::cmtt) fn pattern_input_interface(
        &mut self,
        value: &Expr<'_>,
    ) -> Result<PatternInterface, Error> {
        let H::ContextualPatternType(subject, entries) = &value.kind else {
            return Err(error(value.span, "expected a pattern interface"));
        };
        let subject = self.type_expr(subject)?;
        let mut outputs = BTreeMap::new();
        for entry in entries {
            let ty = self.type_expr(entry.ty)?;
            if outputs.insert(entry.name.name.to_string(), ty).is_some() {
                return Err(error(entry.name.span, "duplicate pattern output name"));
            }
        }
        Ok(PatternInterface { subject, outputs })
    }

    pub(in crate::program::cmtt) fn abstract_pattern_input(
        &mut self,
        interface: &PatternInterface,
        span: Span,
    ) -> Result<Arc<PatternCode>, Error> {
        let mut outputs = BTreeMap::new();
        for (name, ty) in &interface.outputs {
            outputs.insert(name.clone(), (self.spread_binding(span)?, ty.clone()));
        }
        let code = Arc::new(PatternCode {
            subject: interface.subject.clone(),
            outputs,
            body: None,
        });
        self.validate_pattern_code(&code, span)?;
        Ok(code)
    }

    pub(in crate::program::cmtt) fn ascribe_pattern_input(
        &mut self,
        code: &PatternCode,
        interface: &PatternInterface,
        span: Span,
    ) -> Result<(), Error> {
        self.require(&code.subject, &interface.subject, span)?;
        if !code.outputs.keys().eq(interface.outputs.keys()) {
            return Err(error(span, "pattern argument output interface mismatch"));
        }
        for (name, ty) in &interface.outputs {
            self.require(&code.outputs[name].1, ty, span)?;
        }
        self.validate_pattern_code(code, span)
    }

    pub(in crate::program::cmtt) fn pattern_interface_context(
        interface: &PatternInterface,
    ) -> crate::calls::InputContext {
        use crate::calls::InputContext as C;
        C::Tuple(
            std::iter::once(C::Known(interface.subject.clone()))
                .chain(interface.outputs.values().cloned().map(C::Known))
                .collect(),
        )
    }

    pub(in crate::program::cmtt) fn pattern_input_value(
        &mut self,
        value: &Expr<'_>,
        interface: &PatternInterface,
        subject_hint: Option<&Term>,
        interface_context: Option<&crate::calls::InputContext>,
    ) -> Result<Arc<PatternCode>, Error> {
        let full_context;
        let interface_context = match interface_context {
            Some(context) => Some(context),
            None => {
                full_context = Self::pattern_interface_context(interface);
                Some(&full_context)
            }
        };
        match &value.kind {
            H::ResolvedLocal(_, id) | H::ResolvedFnParam(_, id) => self
                .pattern_codes
                .get(&local(*id))
                .cloned()
                .ok_or_else(|| error(value.span, "expected a pattern code argument")),
            H::Application(callee, args) => self.invoke_pattern_generator_in_context(
                callee,
                args,
                subject_hint,
                interface_context,
                value.span,
            ),
            H::ExtendedApplication(callee, args) => self.invoke_pattern_properties_in_context(
                value,
                callee,
                args,
                subject_hint,
                interface_context,
            ),
            H::CodePatternQuote(..) => self.pattern_generator_value(value, interface, 0),
            _ => self.pattern_generator_value_inferred(
                value,
                None,
                subject_hint,
                interface_context,
                0,
            ),
        }
    }

    pub(in crate::program) fn pattern_code_binding(
        &mut self,
        decl: &hir::decl::LetDecl<'_>,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        self.pattern_code_binding_mode(decl, expected, false)
    }

    pub(in crate::program::cmtt) fn pattern_code_binding_mode(
        &mut self,
        decl: &hir::decl::LetDecl<'_>,
        expected: Option<&Term>,
        generator: bool,
    ) -> Result<n::Expr, Error> {
        if (decl.mutable && !generator) || decl.guard.is_some() || decl.otherwise.is_some() {
            return Err(error(
                decl.span,
                "pattern code requires an immutable unconditional const",
            ));
        }
        if let Some(expected) = expected {
            self.require(&p(P::Void), expected, decl.span)?;
        }
        let mut bindings = Vec::new();
        let mut target = &decl.pattern;
        loop {
            match &target.kind {
                hir::PatternKind::Wild => break,
                hir::PatternKind::Binding(hir::common::BindingMode::ByValue, _, inner) => {
                    bindings.push(local(target.hir_id));
                    if let Some(inner) = inner {
                        target = inner;
                    } else {
                        break;
                    }
                }
                _ => {
                    return Err(error(
                        target.span,
                        "pattern code const binds a whole code value",
                    ));
                }
            }
        }
        let value = decl
            .init
            .ok_or_else(|| error(decl.span, "pattern code requires an initializer"))?;
        let saved = self.locals.clone();
        let context = self.context.clone();
        let in_code = std::mem::replace(&mut self.in_code_body, true);
        let holes = std::mem::take(&mut self.quote_holes);
        let result = (|| {
            let interface = if let Some(annotation) = decl.ty {
                let H::ContextualPatternType(subject, entries) = &annotation.kind else {
                    return Err(error(
                        annotation.span,
                        "pattern code requires a pattern interface",
                    ));
                };
                let subject = self.type_expr(subject)?;
                let mut outputs = BTreeMap::new();
                for entry in entries {
                    let ty = self.type_expr(entry.ty)?;
                    if outputs
                        .insert(entry.name.name.to_string(), ty.clone())
                        .is_some()
                    {
                        return Err(error(entry.name.span, "duplicate pattern output name"));
                    }
                    self.locals.insert(local(entry.hir_id), ty);
                }
                Some((subject, outputs))
            } else {
                None
            };
            let code = if let Some((subject, outputs)) = &interface {
                self.pattern_generator_value(
                    value,
                    &PatternInterface {
                        subject: subject.clone(),
                        outputs: outputs.clone(),
                    },
                    0,
                )?
            } else {
                self.pattern_generator_value_inferred(value, None, None, None, 0)?
            };
            if let Some((subject, outputs)) = interface {
                self.require(&code.subject, &subject, value.span)?;
                if !outputs.keys().eq(code.outputs.keys()) {
                    return Err(error(value.span, "pattern code output interface mismatch"));
                }
                for (name, ty) in outputs {
                    self.require(&code.outputs[&name].1, &ty, value.span)?;
                }
            }
            self.validate_pattern_code(&code, value.span)?;
            Ok(code)
        })();
        self.locals = saved;
        self.context = context;
        self.in_code_body = in_code;
        self.quote_holes = holes;
        let code = result?;
        for id in bindings {
            if decl.mutable {
                self.immutable.remove(&id);
            } else {
                self.immutable.insert(id);
            }
            self.pattern_codes.insert(id, code.clone());
        }
        Ok(n::Expr {
            source: source(decl.span),
            ty: p(P::Void),
            kind: K::Unit,
        })
    }

    pub(super) fn validate_pattern_code(
        &self,
        code: &PatternCode,
        span: Span,
    ) -> Result<(), Error> {
        let metadata = n::Package {
            target: self.target,
            structs: self.structs.values().cloned().collect(),
            enums: self.enums.values().cloned().collect(),
            normal_forms: self.normal_form_metadata(),
            ..Default::default()
        };
        let frames = vec![
            self.call_inference_scope()
                .iter()
                .map(|binder| binder.sort)
                .collect(),
        ];
        for ty in std::iter::once(&code.subject).chain(code.outputs.values().map(|(_, ty)| ty)) {
            nir::phases::validate_runtime_storage(ty, &metadata, &frames)
                .map_err(|message| error(span, format!("pattern runtime interface: {message}")))?;
        }
        Ok(())
    }

    fn build_pattern_quote(
        &mut self,
        value: &Expr<'_>,
        subject: &Term,
        declared: Option<&BTreeMap<String, Term>>,
    ) -> Result<Arc<PatternCode>, Error> {
        let H::CodePatternQuote(pattern, substitutions) = &value.kind else {
            return Err(error(value.span, "expected a pattern quote"));
        };
        let saved = self.locals.clone();
        let context = self.context.clone();
        let result = self.quoted_runtime_boundary(|this| {
            this.pattern_quote_pending = Some(false);
            let body = this.pattern(pattern, subject)?;
            let names = pattern_bindings(pattern)?;
            let mut ports = BTreeMap::new();
            for entry in substitutions {
                let binding = binding(entry.binding)?;
                if !names.values().any(|id| *id == binding) {
                    return Err(error(
                        entry.port.span,
                        "pattern quote port must name a success binding",
                    ));
                }
                if ports.insert(entry.port.name.to_string(), binding).is_some() {
                    return Err(error(entry.port.span, "duplicate pattern quote port"));
                }
            }
            let inferred;
            let declared = match declared {
                Some(declared) => declared,
                None => {
                    let selected = if ports.is_empty() { &names } else { &ports };
                    inferred = selected
                        .iter()
                        .map(|(name, id)| {
                            this.locals
                                .get(id)
                                .cloned()
                                .map(|ty| (name.clone(), ty))
                                .ok_or_else(|| error(value.span, "pattern output is unavailable"))
                        })
                        .collect::<Result<BTreeMap<_, _>, _>>()?;
                    &inferred
                }
            };
            if ports.keys().any(|name| !declared.contains_key(name)) {
                return Err(error(
                    value.span,
                    "pattern quote exposes an undeclared output",
                ));
            }
            let mut outputs = BTreeMap::new();
            for (name, expected) in declared {
                let id = ports
                    .get(name)
                    .or_else(|| names.get(name))
                    .copied()
                    .ok_or_else(|| {
                        error(
                            value.span,
                            format!("pattern does not produce output {name}"),
                        )
                    })?;
                let actual = this.locals.get(&id).ok_or_else(|| {
                    error(value.span, "pattern output is not available on success")
                })?;
                this.require(actual, expected, value.span)?;
                if outputs.values().any(|(previous, _)| *previous == id) {
                    return Err(error(
                        value.span,
                        "pattern output binding cannot be exported twice",
                    ));
                }
                outputs.insert(name.clone(), (id, actual.clone()));
            }
            let arm = n::MatchArm {
                pattern: body,
                guard: None,
                body: n::Expr {
                    source: source(value.span),
                    ty: p(P::Void),
                    kind: K::Unit,
                },
            };
            let mut fuel = usize::try_from(this.required_steps).unwrap_or(usize::MAX);
            let before = fuel;
            let free = nir::free_bindings::collect_arm_with_dependencies(
                &arm,
                &this.abstract_code_dependencies,
                &mut fuel,
            );
            this.required_steps -= (before - fuel) as u64;
            let free = free.map_err(|message| error(value.span, message))?;
            let pending = this.pattern_quote_pending == Some(true)
                || (this.checking_code_generator
                    && free.iter().any(|id| this.generator_data.contains_key(id)));
            if free
                .iter()
                .any(|id| !(this.checking_code_generator && this.generator_data.contains_key(id)))
            {
                return Err(error(
                    value.span,
                    "standalone pattern code cannot capture runtime inputs",
                ));
            }
            Ok(Arc::new(PatternCode {
                subject: subject.clone(),
                outputs,
                body: (!pending).then_some(arm.pattern),
            }))
        });
        self.locals = saved;
        self.context = context;
        result
    }

    pub(in crate::program) fn connect_pattern_code(
        &mut self,
        operand: &Expr<'_>,
        outputs: &[hir::pattern::CodePatternOutput],
        subject: &Term,
        span: Span,
    ) -> Result<n::Pattern, Error> {
        self.require_runtime_code_position(span)?;
        let code = self.pattern_generator_value_inferred(operand, None, Some(subject), None, 0)?;
        self.validate_pattern_code(&code, span)?;
        self.require(subject, &code.subject, span)?;
        let mut connections = BTreeMap::new();
        let mut locals = BTreeMap::new();
        for output in outputs {
            let (binding, ty) = code
                .outputs
                .get(output.port.name.as_str())
                .ok_or_else(|| error(output.port.span, "unknown pattern output port"))?;
            if connections.insert(*binding, local(output.hir_id)).is_some() {
                return Err(error(
                    output.port.span,
                    "duplicate pattern output substitution",
                ));
            }
            locals.insert(local(output.hir_id), ty.clone());
        }
        if connections.len() != code.outputs.len() {
            return Err(error(
                span,
                "pattern splice requires explicit connections for all outputs",
            ));
        }
        if code.body.is_none()
            && self.checking_code_generator
            && self.pattern_quote_pending.is_some()
        {
            self.pattern_quote_pending = Some(true);
            let outputs = locals.iter().map(|(id, ty)| (*id, ty.clone())).collect();
            self.locals.extend(locals);
            return Ok(n::Pattern::CodeEvidence { outputs });
        }
        let template = code.body.as_ref().ok_or_else(|| error(span,
            "pattern expansion awaits its generator data; symbolic pattern evidence cannot execute"))?;
        nir::copy_budget::charge_pattern_rewrite(template, &mut self.required_steps).map_err(
            |failure| {
                error(
                    span,
                    format!("pattern expansion budget exhausted: {failure:?}"),
                )
            },
        )?;
        let mut body = template.clone();
        nir::freshening::pattern(
            &mut body,
            connections,
            &mut self.next_temporary_binding,
            &mut self.expansion_sources,
            source(span),
        )
        .map_err(|message| error(span, message))?;
        self.locals.extend(locals);
        Ok(body)
    }
}
