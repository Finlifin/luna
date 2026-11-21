//! Bind computed Type identities at the point their checked lexical consumer needs them.
use super::*;
use std::sync::Arc;

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
        let K::Call {
            callee, arguments, ..
        } = &mut value.kind
        else {
            return Err(needs_frame(span, "expected the original selector call"));
        };

        let mut segments = Vec::new();
        let mut segment = Segment::default();
        let mut pending = BTreeSet::new();
        let mut unavailable = identities;
        self.retain_lexical_operand(callee, &mut segment, span)?;
        for argument in arguments {
            if unavailable.is_empty() {
                break;
            }
            dependent_inputs::tasks::spend(&mut self.required_steps, 1, span)?;
            if argument.is_default {
                return Err(needs_frame(span, "operand follows an omitted default"));
            }
            let uses = self.checked_declaration_inputs(&argument.value, span)?;
            if !uses.is_disjoint(&unavailable) {
                return Err(needs_frame(span, "operand needs a later type input"));
            }
            if !uses.is_disjoint(&pending) {
                segments.push(std::mem::take(&mut segment));
                pending.clear();
            }
            if let Some(value) =
                arguments::operand_to_prepare(&mut argument.value, argument.passing)
            {
                self.retain_lexical_operand(value, &mut segment, span)?;
            }
            if let Some(binder) = inputs.get(&argument.parameter) {
                unavailable.remove(&binder.declaration);
                pending.insert(binder.declaration);
                segment.parameters.push(binder.declaration);
                // This read and the original selector use the same fresh local.
                segment
                    .arguments
                    .push(n::NormalFormArgument::Value(argument.value.clone()));
            }
        }
        if !unavailable.is_empty() {
            return Err(needs_frame(span, "missing lexical type operand"));
        }
        segments.push(segment);
        for segment in segments.into_iter().rev() {
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
    ) -> Result<(), Error> {
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
        Ok(())
    }
}
