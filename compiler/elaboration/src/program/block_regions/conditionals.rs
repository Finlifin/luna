//! Contextual branches retain checked children while their sibling is pending.
//! Their result remains an input program, never a fabricated ordinary type.
use super::*;
use driver::{Request, Step};
use refinements::TypeConditionScope;

enum State {
    Ready,
    Condition,
    Then(n::Expr),
    Else(n::Expr, n::NormalFormArgument),
    Complete,
}

pub(super) struct Frame<'b, 'h> {
    source: &'b Expr<'h>,
    depth: usize,
    state: State,
    branch: Option<TypeConditionScope>,
}

impl<'b, 'h> Frame<'b, 'h> {
    pub(super) fn new(
        checker: &mut Checker<'_, '_>,
        source: &'b Expr<'h>,
        depth: usize,
    ) -> Result<Self, Error> {
        if depth >= 128 {
            return Err(error(
                source.span,
                "contextual input nesting limit exceeded",
            ));
        }
        if !matches!(source.kind, H::If(_, _, Some(_))) {
            return Err(error(
                source.span,
                "contextual conditional requires two branches",
            ));
        }
        checker.required_steps = checker
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(source.span, "conditional checking budget exhausted"))?;
        Ok(Self {
            source,
            depth,
            state: State::Ready,
            branch: None,
        })
    }

    pub(super) fn step(
        &mut self,
        checker: &mut Checker<'_, '_>,
        returned: Option<CheckedBlock>,
    ) -> Result<Step<'b, 'h>, Error> {
        let H::If(condition, yes, Some(no)) = self.source.kind else {
            unreachable!("validated contextual conditional")
        };
        // Even when a child is rejected, the branch boundary stays owned by
        // this frame so the driver can restore it during failure unwinding.
        let state = std::mem::replace(&mut self.state, State::Complete);
        match (state, returned) {
            (State::Ready, None) => {
                let destination = Destination::ordinary(
                    checker,
                    OrdinaryTail {
                        expected: Some(&p(P::Bool)),
                        partial: None,
                        input: None,
                    },
                    condition.span,
                )?;
                self.state = State::Condition;
                Ok(Step::Child(Request::Expression(condition, destination)))
            }
            (State::Condition, Some(value)) => {
                let condition = value.ordinary(self.source.span)?;
                self.branch =
                    Some(checker.enter_type_condition(&condition, true, self.source.span)?);
                // Match the original block-rule depth and its diagnostic span.
                if self.depth + 1 >= 128 {
                    return Err(error(yes.span, "contextual input nesting limit exceeded"));
                }
                self.state = State::Then(condition);
                Ok(Step::Child(Request::Block(
                    yes,
                    Destination::Contextual(ContextualTail {
                        depth: self.depth + 1,
                    }),
                )))
            }
            (State::Then(condition), Some(value)) => {
                let then_input = value.contextual(self.source.span)?;
                self.leave_branch(checker);
                self.branch =
                    Some(checker.enter_type_condition(&condition, false, self.source.span)?);
                self.state = State::Else(condition, then_input);
                // Destination's depth is the parent rule depth. Checking this
                // expression adds one; a source block then adds its own level.
                Ok(Step::Child(Request::Expression(
                    no,
                    Destination::Contextual(ContextualTail { depth: self.depth }),
                )))
            }
            (State::Else(condition, then_input), Some(value)) => {
                let else_input = value.contextual(self.source.span)?;
                self.leave_branch(checker);
                Ok(Step::Conditional(n::NormalFormArgument::If {
                    source: source(self.source.span),
                    condition,
                    then_input: Box::new(then_input),
                    else_input: Box::new(else_input),
                }))
            }
            _ => Err(error(
                self.source.span,
                "conditional child result does not match its continuation",
            )),
        }
    }

    fn leave_branch(&mut self, checker: &mut Checker<'_, '_>) {
        if let Some(scope) = self.branch.take() {
            scope.leave(checker);
        }
    }

    pub(super) fn unwind(mut self, checker: &mut Checker<'_, '_>) {
        self.leave_branch(checker);
    }
}

impl std::fmt::Display for Frame<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(contextual-if-check (source {}) (depth {}) ",
            source(self.source.span),
            self.depth
        )?;
        match &self.state {
            State::Ready => f.write_str("(phase ready)"),
            State::Condition => f.write_str("(phase condition-child)"),
            State::Then(condition) => write!(f, "(phase then-child) (condition {condition})"),
            State::Else(condition, then_input) => write!(
                f,
                "(phase else-child) (condition {condition}) (then {then_input})"
            ),
            State::Complete => f.write_str("(phase complete)"),
        }?;
        f.write_str(" (branch")?;
        if let Some(branch) = &self.branch {
            write!(f, " {branch}")?;
        }
        f.write_str("))")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_else_dump_keeps_checked_condition_then_input_and_destination() {
        let block = hir::Block {
            hir_id: hir::HirId::INVALID,
            stmts: &[],
            expr: None,
            span: Span::default(),
        };
        let leaf = Expr {
            hir_id: hir::HirId::INVALID,
            span: block.span,
            kind: H::Unit,
        };
        let expr = Expr {
            kind: H::If(&leaf, &block, Some(&leaf)),
            ..leaf
        };
        let condition = n::Expr {
            source: source(expr.span),
            ty: p(P::Bool),
            kind: K::Literal(n::Literal::Bool(true)),
        };
        let then_input = n::NormalFormArgument::Value(n::Expr {
            source: source(expr.span),
            ty: p(P::Str),
            kind: K::Literal(n::Literal::String("branch\"\n".into())),
        });
        let frame = Frame {
            source: &expr,
            depth: 7,
            state: State::Else(condition, then_input),
            branch: None,
        };
        let dump = frame.to_string();
        assert!(dump.starts_with("(contextual-if-check "), "{dump}");
        assert!(dump.contains("(depth 7) (phase else-child)"), "{dump}");
        assert!(
            dump.contains("(condition ") && dump.contains("(then "),
            "{dump}"
        );
        assert!(dump.contains("branch\\\"\\n"), "{dump}");
        assert!(!dump.contains('\n'), "{dump}");
        let request = Step::Child(Request::Block(
            &block,
            Destination::Contextual(ContextualTail { depth: 8 }),
        ))
        .to_string();
        assert!(
            request.starts_with("(check-step (check-block "),
            "{request}"
        );
        assert!(
            request.ends_with("(contextual-tail (depth 8))))"),
            "{request}"
        );
    }
}
