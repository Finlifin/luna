//! Close each computed initializer type over the portion of its lexical block
//! that can read the initialized locals.
use super::*;

#[derive(Default)]
pub(in crate::program) struct BindingScopes(BTreeMap<n::LocalId, BindingInputs>);

enum BindingInputs {
    Call(Arc<Inputs>),
    Annotation(Vec<(Arc<n::Expr>, Input)>),
}

impl BindingInputs {
    fn visit(&self, mut f: impl FnMut(&Input)) {
        match self {
            Self::Call(inputs) => inputs.values().for_each(f),
            Self::Annotation(inputs) => inputs.iter().for_each(|(_, input)| f(input)),
        }
    }

    fn close(
        self,
        checker: &mut Checker<'_, '_>,
        body: n::Expr,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let inputs = match self {
            Self::Call(inputs) => {
                return checker.close_operand_consumer(body, &inputs, Consumer::Binding, span);
            }
            Self::Annotation(inputs) => inputs,
        };
        let escaping = checker.term_declaration_inputs(&body.ty, span)?;
        if inputs
            .iter()
            .any(|(_, input)| escaping.contains(&input.binder.declaration))
        {
            return Err(context_error(
                span,
                "dependent result would escape its local type annotation",
            ));
        }
        let mut schedule = OperandSchedule::new(
            inputs
                .iter()
                .map(|(_, input)| input.binder.declaration)
                .collect(),
        );
        for (value, input) in inputs {
            let local = if let K::Local(local) = value.kind {
                local
            } else {
                schedule.enter(checker, &value, span)?;
                let mut value = match Arc::try_unwrap(value) {
                    Ok(value) => value,
                    Err(shared) => {
                        constants::charge_copy(&shared, &mut checker.required_steps, span)?;
                        (*shared).clone()
                    }
                };
                checker.retain_lexical_operand(&mut value, &mut schedule.current, span)?
            };
            schedule.supply(input.binder.declaration, local, input.producer);
        }
        schedule.segments.push(schedule.current);
        checker.enclose_operand_segments(body, schedule.segments, span)
    }
}

impl BindingScopes {
    pub(in crate::program) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(in crate::program) fn retain(
        &mut self,
        checker: &mut Checker<'_, '_>,
        boundary: Boundary,
        annotation: Option<Annotation>,
        statement: &mut n::Expr,
        prefix: &mut Vec<n::Expr>,
        span: Span,
    ) -> Result<(), Error> {
        if matches!(boundary, Boundary::Idle)
            && annotation.as_ref().is_none_or(|a| a.inputs.is_empty())
        {
            return Ok(());
        }
        let (local, value) = match &mut statement.kind {
            K::Let { binding, value } => (Some(*binding), value),
            K::LetPattern { value, .. } => (None, value),
            _ => return Err(needs_frame(span, "expected a checked local binding")),
        };
        let inputs = match (boundary, annotation) {
            (_, Some(annotation)) if !annotation.inputs.is_empty() => {
                BindingInputs::Annotation(annotation.inputs)
            }
            (Boundary::Consumer { inputs }, _) => {
                let used = checker.term_declaration_inputs(&value.ty, span)?;
                if !inputs
                    .values()
                    .any(|input| used.contains(&input.binder.declaration))
                {
                    let source = value.source;
                    let call = std::mem::replace(
                        &mut **value,
                        n::Expr {
                            source,
                            ty: p(P::Void),
                            kind: K::Unit,
                        },
                    );
                    **value =
                        checker.close_operand_consumer(call, &inputs, Consumer::Call, span)?;
                    return Ok(());
                }
                // A surviving result may now be referred to through the original
                // immutable Type name in a later annotation or another call.
                for input in inputs.values() {
                    if let Some(binding) = input.stable_local
                        && used.contains(&input.binder.declaration)
                    {
                        checker.input_facts.named_types.bind(binding, input.binder);
                    }
                }
                BindingInputs::Call(inputs)
            }
            _ => return Ok(()),
        };
        let binding = match local {
            Some(binding) => binding,
            None => {
                // Pattern stabilization can split a declaration into several
                // statements. Keep its original subject at a unique local.
                let binding = checker.spread_binding(span)?;
                let read = n::Expr {
                    source: value.source,
                    ty: declaration_inputs::copy_input_domain(
                        &value.ty,
                        &mut checker.required_steps,
                        span,
                    )?,
                    kind: K::Local(binding),
                };
                prefix.push(n::Expr {
                    source: value.source,
                    ty: p(if is_never(&value.ty) {
                        P::Never
                    } else {
                        P::Void
                    }),
                    kind: K::Let {
                        binding,
                        value: std::mem::replace(value, Box::new(read)),
                    },
                });
                binding
            }
        };
        self.0.insert(binding, inputs);
        Ok(())
    }

    pub(in crate::program) fn finish(
        mut self,
        checker: &mut Checker<'_, '_>,
        mut block: n::Expr,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let K::Block { statements, result } = &mut block.kind else {
            unreachable!()
        };
        // Descending source order makes the first observed use the last use.
        // A generated block must also enclose every later read of any ordinary
        // local it encloses, even if that local's type is independent.
        let mut uses = LastUses::default();
        let mut parts = Vec::new();
        if let Some(tail) = result.take() {
            let index = statements.len();
            uses.observe(checker, index, &tail, span)?;
            parts.push(Part {
                index,
                through: index,
                depth: 0,
                is_result: true,
                value: *tail,
            });
        }
        while !self.0.is_empty() {
            let Some(statement) = statements.pop() else {
                return Err(needs_frame(
                    span,
                    "local type initializer was lost during block completion",
                ));
            };
            let index = statements.len();
            let through = uses.local_extent(checker, index, &statement, span)?;
            let inputs = match statement.kind {
                K::Let { binding, .. } => self.0.remove(&binding),
                _ => None,
            };
            uses.observe(checker, index, &statement, span)?;
            let mut part = Part {
                index,
                through,
                depth: 0,
                is_result: false,
                value: statement,
            };
            if let Some(inputs) = inputs {
                if let BindingInputs::Annotation(annotation) = &inputs {
                    for (producer, _) in annotation {
                        uses.observe(checker, index, producer, span)?;
                    }
                }
                inputs.visit(|input| {
                    if let Some(&last) = uses.types.get(&input.binder.declaration) {
                        part.through = part.through.max(last);
                    }
                });
                part = part.enclose(checker, inputs, &mut parts, span)?;
            }
            parts.push(part);
        }
        for part in parts.into_iter().rev() {
            if part.is_result {
                *result = Some(Box::new(part.value));
            } else {
                statements.push(part.value);
            }
        }
        Ok(block)
    }
}

/// One checked statement or completed lexical region. The extent may lie
/// beyond this part if it introduces a local read by a later sibling.
struct Part {
    index: usize,
    through: usize,
    depth: usize,
    is_result: bool,
    value: n::Expr,
}

impl Part {
    fn enclose(
        mut self,
        checker: &mut Checker<'_, '_>,
        inputs: BindingInputs,
        parts: &mut Vec<Self>,
        span: Span,
    ) -> Result<Self, Error> {
        let source = self.value.source;
        let mut diverges = is_never(&self.value.ty);
        let mut region = vec![self.value];
        let mut tail = None;
        while parts.last().is_some_and(|part| part.index <= self.through) {
            let part = parts.pop().unwrap();
            self.through = self.through.max(part.through);
            self.depth = self.depth.max(part.depth);
            if part.is_result {
                tail = Some(Box::new(part.value));
            } else {
                diverges |= is_never(&part.value.ty);
                region.push(part.value);
            }
        }
        self.is_result = tail.is_some();
        let ty = match &tail {
            _ if diverges => p(P::Never),
            Some(value) => {
                declaration_inputs::copy_input_domain(&value.ty, &mut checker.required_steps, span)?
            }
            None => p(P::Void),
        };
        let body = n::Expr {
            source,
            ty,
            kind: K::Block {
                statements: region,
                result: tail,
            },
        };
        self.value = inputs.close(checker, body, span)?;
        self.depth += 1;
        let mut current = &self.value;
        while let K::WithTypeInputs { body, .. } = &current.kind {
            self.depth += 1;
            current = body;
        }
        if self.depth >= nir::copy_budget::MAX_COPY_DEPTH {
            return Err(error(span, "lexical operand nesting budget exhausted"));
        }
        Ok(self)
    }
}

#[derive(Default)]
struct LastUses {
    locals: BTreeMap<n::LocalId, usize>,
    types: BTreeMap<DeclarationId, usize>,
}

impl LastUses {
    fn observe(
        &mut self,
        checker: &mut Checker<'_, '_>,
        index: usize,
        value: &n::Expr,
        span: Span,
    ) -> Result<(), Error> {
        for id in checker.checked_declaration_inputs(value, span)? {
            self.types.entry(id).or_insert(index);
        }
        checker.with_observation_budget(span, |_, budget| {
            let locals = nir::free_bindings::collect(value, &mut budget.steps)?;
            budget.consume(locals.len())?;
            for local in locals {
                self.locals.entry(local).or_insert(index);
            }
            Ok(())
        })
    }

    fn local_extent(
        &self,
        checker: &mut Checker<'_, '_>,
        index: usize,
        statement: &n::Expr,
        span: Span,
    ) -> Result<usize, Error> {
        let mut end = index;
        let mut include = |id| {
            if let Some(&last) = self.locals.get(&id) {
                end = end.max(last);
            }
        };
        match &statement.kind {
            K::Let { binding, .. } => include(*binding),
            K::LetPattern { pattern, .. } => {
                checker.with_observation_budget(span, |_, budget| {
                    nir::free_bindings::visit_possible_pattern_bindings(
                        pattern,
                        &mut budget.steps,
                        include,
                    )?;
                    Ok(())
                })?
            }
            _ => {}
        }
        Ok(end)
    }
}

impl std::fmt::Display for BindingScopes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(local-operand-scopes")?;
        for (binding, inputs) in &self.0 {
            write!(f, " (local {binding}")?;
            match inputs {
                BindingInputs::Call(inputs) => {
                    for input in inputs.values() {
                        write!(f, " {}", input.binder)?;
                    }
                }
                BindingInputs::Annotation(inputs) => {
                    for (local, input) in inputs {
                        write!(f, " (annotation {local} {})", input.binder)?;
                    }
                }
            }
            f.write_str(")")?;
        }
        f.write_str(")")
    }
}
