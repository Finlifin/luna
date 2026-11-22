//! Bind computed Type identities at the point their checked lexical consumer needs them.
use super::*;
use std::sync::Arc;

mod forward;

type Inputs = BTreeMap<usize, nir::DeclarationParameter>;

#[derive(Clone)]
pub(super) struct Scope {
    site: hir::HirId,
    inputs: Option<Arc<Inputs>>,
}

/// The checking region owns the temporary Type identity until it has either
/// abstracted every use or rejected the escaping consumer.
#[derive(Default)]
pub(super) enum Boundary {
    #[default]
    Idle,
    Selector {
        enclosing: Option<Scope>,
    },
    Consumer {
        inputs: Arc<Inputs>,
    },
}

impl Boundary {
    pub(super) fn enter(&mut self, checker: &mut Checker<'_, '_>, site: hir::HirId) {
        let enclosing = checker.operand_scope.replace(Scope { site, inputs: None });
        *self = Self::Selector { enclosing };
    }

    pub(super) fn leave(&mut self, checker: &mut Checker<'_, '_>) {
        if let Self::Selector { enclosing } = self {
            let selected = std::mem::replace(&mut checker.operand_scope, enclosing.take());
            *self = match selected.and_then(|scope| scope.inputs) {
                Some(inputs) => Self::Consumer { inputs },
                None => Self::Idle,
            };
        }
    }

    pub(super) fn close(
        &mut self,
        checker: &mut Checker<'_, '_>,
        value: n::Expr,
        span: Span,
    ) -> Result<n::Expr, Error> {
        match std::mem::take(self) {
            Self::Consumer { inputs } => checker.close_operand_consumer(value, &inputs, span),
            Self::Idle => Ok(value),
            Self::Selector { .. } => unreachable!("selector must complete before its consumer"),
        }
    }
}

impl std::fmt::Display for Boundary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Idle => f.write_str("(operand-scope idle)"),
            Self::Selector { .. } => f.write_str("(operand-scope selector)"),
            Self::Consumer { inputs } => {
                f.write_str("(operand-scope")?;
                for (slot, binder) in inputs.iter() {
                    write!(f, " (input (slot {slot}) {binder})")?;
                }
                f.write_str(")")
            }
        }
    }
}

#[derive(Default)]
struct Segment {
    preparations: Vec<n::NormalFormPreparation>,
    parameters: Vec<DeclarationId>,
    arguments: Vec<n::NormalFormArgument>,
}

struct OperandSchedule {
    segments: Vec<Segment>,
    current: Segment,
    pending: BTreeSet<DeclarationId>,
    unavailable: BTreeSet<DeclarationId>,
}

impl OperandSchedule {
    fn enter(
        &mut self,
        checker: &mut Checker<'_, '_>,
        expression: &n::Expr,
        span: Span,
    ) -> Result<(), Error> {
        self.enter_uses(&checker.checked_declaration_inputs(expression, span)?, span)
    }

    fn enter_uses(&mut self, uses: &BTreeSet<DeclarationId>, span: Span) -> Result<(), Error> {
        if !uses.is_disjoint(&self.unavailable) {
            return Err(needs_frame(span, "operand needs a later type input"));
        }
        if !uses.is_disjoint(&self.pending) {
            self.segments.push(std::mem::take(&mut self.current));
            self.pending.clear();
        }
        Ok(())
    }

    fn supply(&mut self, binder: DeclarationId, binding: n::LocalId, source: n::SourceRange) {
        if self.unavailable.remove(&binder) {
            self.pending.insert(binder);
            self.current.parameters.push(binder);
            self.current
                .arguments
                .push(n::NormalFormArgument::Value(n::Expr {
                    source,
                    ty: p(P::Type),
                    kind: K::Local(binding),
                }));
        }
    }
}

fn needs_frame(span: Span, reason: &str) -> Error {
    context_error(
        span,
        format!("dependent call input needs its original comptime invocation: {reason}"),
    )
}

impl Checker<'_, '_> {
    pub(super) fn bind_scoped_operand(
        &mut self,
        site: hir::HirId,
        slot: usize,
        span: Span,
    ) -> Result<Option<Term>, Error> {
        let Some(scope) = self
            .operand_scope
            .as_ref()
            .filter(|scope| scope.site == site)
        else {
            return Ok(None);
        };
        let binder = match scope.inputs.as_ref().and_then(|inputs| inputs.get(&slot)) {
            Some(binder) => *binder,
            None => {
                let binder = self.fresh_type_binder(site.owner, span)?;
                let scope = self.operand_scope.as_mut().unwrap();
                Arc::make_mut(scope.inputs.get_or_insert_with(Arc::default)).insert(slot, binder);
                binder
            }
        };
        Ok(Some(Term::Rigid {
            declaration: binder.declaration,
            sort: binder.sort,
        }))
    }

    fn close_operand_consumer(
        &mut self,
        mut body: n::Expr,
        inputs: &Inputs,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let identities: BTreeSet<_> = inputs.values().map(|p| p.declaration).collect();
        if !self
            .term_declaration_inputs(&body.ty, span)?
            .is_disjoint(&identities)
        {
            return Err(context_error(
                span,
                "dependent match result would escape its operand scope",
            ));
        }
        let K::Match { value, .. } = &mut body.kind else {
            return Err(needs_frame(span, "expected the original match"));
        };
        // Routing may already have evaluated the callee, a spread packet, or
        // collected inputs into private locals. Transfer these statements in
        // their original order before staging the normalized call's operands.
        let mut prefix = Vec::new();
        while let K::CallPreparation {
            statements,
            result: Some(result),
        } = &mut value.kind
        {
            dependent_inputs::tasks::spend(&mut self.required_steps, statements.len() + 1, span)?;
            prefix.append(statements);
            let inner = std::mem::replace(
                result.as_mut(),
                n::Expr {
                    ty: p(P::Void),
                    source: body.source,
                    kind: K::Unit,
                },
            );
            **value = inner;
        }
        let K::Call {
            callee, arguments, ..
        } = &mut value.kind
        else {
            return Err(needs_frame(span, "expected the original selector call"));
        };
        let mut schedule = OperandSchedule {
            segments: Vec::new(),
            current: Segment::default(),
            pending: BTreeSet::new(),
            unavailable: identities,
        };
        let mut prepared = BTreeMap::<_, Vec<_>>::new();
        if !prefix.is_empty() {
            dependent_inputs::tasks::spend(&mut self.required_steps, arguments.len(), span)?;
            for argument in arguments.iter() {
                if let K::Local(binding) = argument.value.kind
                    && let Some(binder) = inputs.get(&argument.parameter)
                {
                    prepared
                        .entry(binding)
                        .or_default()
                        .push((binder.declaration, argument.value.source));
                }
            }
        }
        for statement in prefix {
            schedule.enter(self, &statement, span)?;
            let preparation = n::NormalFormPreparation {
                before: schedule.current.arguments.len(),
                expression: statement,
            };
            let valid = preparation.visit_bindings(
                |n| dependent_inputs::tasks::spend(&mut self.required_steps, n, span),
                |binding| {
                    if let Some(witnesses) = prepared.remove(&binding) {
                        for (binder, source) in witnesses {
                            schedule.supply(binder, binding, source);
                        }
                    }
                },
            )?;
            if !valid {
                return Err(needs_frame(span, "invalid selector preparation"));
            }
            schedule.current.preparations.push(preparation);
        }
        schedule.enter(self, callee, span)?;
        if !schedule.current.parameters.is_empty() || !schedule.unavailable.is_empty() {
            self.retain_lexical_operand(callee, &mut schedule.current, span)?;
        }
        for argument in arguments {
            if argument.is_default {
                break;
            }
            dependent_inputs::tasks::spend(&mut self.required_steps, 1, span)?;
            let uses = self.checked_declaration_inputs(&argument.value, span)?;
            if argument.passing == n::ArgumentPassing::Value
                && !uses.is_disjoint(&schedule.unavailable)
            {
                self.prepare_forward_operand(&mut argument.value, &mut schedule, span)?;
                continue;
            }
            schedule.enter_uses(&uses, span)?;
            if schedule.current.parameters.is_empty() && schedule.unavailable.is_empty() {
                break;
            }
            if let Some(operand) =
                arguments::operand_to_prepare(&mut argument.value, argument.passing)
            {
                let binding = self.retain_lexical_operand(operand, &mut schedule.current, span)?;
                if let Some(binder) = inputs.get(&argument.parameter) {
                    schedule.supply(binder.declaration, binding, operand.source);
                }
            }
        }
        if !schedule.unavailable.is_empty() {
            return Err(needs_frame(span, "missing lexical type operand"));
        }
        if schedule.current.parameters.is_empty() {
            // All witnesses are in enclosing segments. These final producers
            // already use their types and remain ordinary call preparation.
            if !schedule.current.preparations.is_empty() {
                let statements = schedule
                    .current
                    .preparations
                    .into_iter()
                    .map(|p| p.expression)
                    .collect();
                let source = value.source;
                let ty = declaration_inputs::copy_input_domain(
                    &value.ty,
                    &mut self.required_steps,
                    span,
                )?;
                let call = std::mem::replace(
                    value.as_mut(),
                    n::Expr {
                        source,
                        ty,
                        kind: K::Unit,
                    },
                );
                value.kind = K::CallPreparation {
                    statements,
                    result: Some(Box::new(call)),
                };
            }
        } else {
            schedule.segments.push(schedule.current);
        }
        for segment in schedule.segments.into_iter().rev() {
            let ty =
                declaration_inputs::copy_input_domain(&body.ty, &mut self.required_steps, span)?;
            let source = body.source;
            body = n::Expr {
                source,
                ty,
                kind: K::WithTypeInputs {
                    preparations: segment.preparations,
                    parameters: segment.parameters,
                    arguments: segment.arguments,
                    body: Box::new(body),
                },
            };
        }
        Ok(body)
    }

    fn retain_lexical_operand(
        &mut self,
        operand: &mut n::Expr,
        segment: &mut Segment,
        span: Span,
    ) -> Result<n::LocalId, Error> {
        let binding = self.spread_binding(span)?;
        let source = operand.source;
        let read = n::Expr {
            source,
            ty: declaration_inputs::copy_input_domain(&operand.ty, &mut self.required_steps, span)?,
            kind: K::Local(binding),
        };
        segment.preparations.push(n::NormalFormPreparation {
            before: segment.arguments.len(),
            expression: n::Expr {
                source,
                ty: p(P::Void),
                kind: K::Let {
                    binding,
                    value: Box::new(std::mem::replace(operand, read)),
                },
            },
        });
        Ok(binding)
    }
}
