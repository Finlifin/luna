//! Logical operands retain inference and short-circuit premises across child
//! checks. BitVec uses the same domain search, without Boolean branch facts.
use super::*;
use driver::{Request, Step};
use operators::peers;
use refinements::TypeConditionScope;

pub(super) fn is_logical(expr: &Expr<'_>) -> bool {
    matches!(expr.kind, H::Binary(BinOp::And | BinOp::Or, _, _))
}

enum State<'h> {
    Start,
    Left,
    HaveLeft(n::Expr),
    DivergingRight(n::Expr),
    Peers(peers::Progress<'h, 'h>),
    Pair(n::Expr, n::Expr),
    Right { left: n::Expr, domain: Term },
    Finished(n::Expr),
    Complete,
}

pub(super) struct Frame<'h> {
    source: Expr<'h>,
    destination: Destination,
    state: State<'h>,
    attempt: Option<checking_attempts::Attempt>,
    premise: Option<TypeConditionScope>,
    preserve_effect: Option<bool>,
}

impl<'h> Frame<'h> {
    pub(super) fn new(
        checker: &mut Checker<'_, '_>,
        source: &Expr<'h>,
        destination: Destination,
        own_effect_delivery: bool,
    ) -> Result<Self, Error> {
        if !is_logical(source) || destination.ordinary_view().is_none() {
            return Err(error(
                source.span,
                "logical operator requires an ordinary destination",
            ));
        }
        checker.required_steps = checker
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(source.span, "logical checking budget exhausted"))?;
        Ok(Self {
            source: source.clone(),
            destination,
            state: State::Start,
            attempt: None,
            premise: None,
            preserve_effect: own_effect_delivery
                .then(|| std::mem::take(&mut checker.preserve_effect_value)),
        })
    }

    fn operands(&self) -> [&'h Expr<'h>; 2] {
        let H::Binary(_, left, right) = self.source.kind else {
            unreachable!("validated logical expression")
        };
        [left, right]
    }

    fn begin_peers(
        &mut self,
        checker: &mut Checker<'_, '_>,
        ready: Option<[Option<n::Expr>; 2]>,
    ) -> Result<(), Error> {
        self.state = State::Peers(peers::Progress::new(
            checker,
            self.operands(),
            self.destination.expected(),
            None,
            ready,
        )?);
        Ok(())
    }

    fn accept(
        &mut self,
        checker: &mut Checker<'_, '_>,
        value: Option<n::Expr>,
    ) -> Result<(), Error> {
        if let State::Peers(progress) = &mut self.state {
            return progress.accept(value);
        }
        match std::mem::replace(&mut self.state, State::Complete) {
            State::Left => match value {
                Some(left) => self.state = State::HaveLeft(left),
                None => self.begin_peers(checker, None)?,
            },
            State::DivergingRight(left) => {
                self.begin_peers(checker, Some([Some(left), value]))?;
            }
            State::Right { left, domain } => {
                // Finish operand checking under the premise, but validate the
                // whole result only after leaving that premise.
                self.leave_premise(checker);
                let right =
                    value.ok_or_else(|| error(self.source.span, "logical RHS is missing"))?;
                self.state = State::Finished(checker.finish_logical_expression(
                    &self.source,
                    self.destination.expected(),
                    left,
                    right,
                    domain,
                )?);
            }
            _ => return Err(error(self.source.span, "unsolicited logical operand")),
        }
        Ok(())
    }

    pub(super) fn step(
        &mut self,
        checker: &mut Checker<'_, '_>,
        returned: Option<CheckedBlock>,
    ) -> Result<Step<'h>, Error> {
        if let Some(value) = returned {
            let value = value.ordinary(self.source.span)?;
            let value = match self.attempt.take() {
                Some(attempt) => attempt.finish(checker, Ok(value))?,
                None => Some(value),
            };
            self.accept(checker, value)?;
        }
        if let State::Peers(progress) = &mut self.state {
            if let Some(request) = progress.next()? {
                let destination = Destination::ordinary(
                    checker,
                    OrdinaryTail {
                        expected: request.expected,
                        partial: None,
                        input: None,
                    },
                    request.input.span,
                )?;
                if request.attempt {
                    self.attempt = Some(checking_attempts::Attempt::begin(
                        checker,
                        request.input.span,
                        false,
                        CheckingDisposition::Commit,
                    )?);
                }
                return Ok(Step::Child(Request::expression(request.input, destination)));
            }
            let State::Peers(progress) = std::mem::replace(&mut self.state, State::Complete) else {
                unreachable!()
            };
            let (left, right) = progress.finish()?;
            self.state = State::Pair(left, right);
        }
        match std::mem::replace(&mut self.state, State::Complete) {
            State::Start => {
                let left = self.operands()[0];
                let destination = Destination::ordinary(
                    checker,
                    OrdinaryTail {
                        expected: self.destination.expected(),
                        partial: None,
                        input: None,
                    },
                    left.span,
                )?;
                self.attempt = Some(checking_attempts::Attempt::begin(
                    checker,
                    left.span,
                    false,
                    CheckingDisposition::Commit,
                )?);
                self.state = State::Left;
                Ok(Step::Child(Request::expression(left, destination)))
            }
            State::HaveLeft(left) if is_never(&left.ty) => {
                // A nonreturning prefix cannot constrain an already typed RHS.
                // Probe it without the whole expression's result expectation.
                let right = self.operands()[1];
                let destination = Destination::statement();
                self.attempt = Some(checking_attempts::Attempt::begin(
                    checker,
                    right.span,
                    false,
                    CheckingDisposition::Commit,
                )?);
                self.state = State::DivergingRight(left);
                Ok(Step::Child(Request::expression(right, destination)))
            }
            State::HaveLeft(left) => self.check_right(checker, left),
            State::Pair(left, right) => {
                let domain = checker.structural_type(
                    if is_never(&left.ty) {
                        &right.ty
                    } else {
                        &left.ty
                    },
                    self.source.span,
                )?;
                self.validate_domain(&domain)?;
                self.state = State::Finished(checker.finish_logical_expression(
                    &self.source,
                    self.destination.expected(),
                    left,
                    right,
                    domain,
                )?);
                Ok(Step::Continue)
            }
            State::Finished(mut value) => {
                if self.preserve_effect == Some(false) {
                    value = checker.run_computation(
                        value,
                        self.destination.expected(),
                        self.source.span,
                    )?;
                }
                Ok(Step::Completed(CheckedBlock::Ordinary(value)))
            }
            _ => Err(error(self.source.span, "logical operand result is missing")),
        }
    }

    fn validate_domain(&self, domain: &Term) -> Result<(), Error> {
        if matches!(domain, Term::Primitive(P::Bool | P::BitVec(_) | P::Never)) {
            Ok(())
        } else {
            Err(error(
                self.source.span,
                "and/or require bool or equal-width BitVec operands",
            ))
        }
    }

    fn check_right(
        &mut self,
        checker: &mut Checker<'_, '_>,
        left: n::Expr,
    ) -> Result<Step<'h>, Error> {
        let domain = checker.structural_type(&left.ty, self.source.span)?;
        self.validate_domain(&domain)?;
        let right = self.operands()[1];
        let destination = Destination::ordinary(
            checker,
            OrdinaryTail {
                expected: Some(&domain),
                partial: None,
                input: None,
            },
            right.span,
        )?;
        if domain == p(P::Bool) {
            let H::Binary(op, hir_left, _) = self.source.kind else {
                unreachable!()
            };
            self.premise = Some(checker.enter_source_type_condition(
                Some(hir_left),
                &left,
                op == BinOp::And,
                self.source.span,
            )?);
        }
        self.state = State::Right { left, domain };
        Ok(Step::Child(Request::expression(right, destination)))
    }

    pub(super) fn recover(
        &mut self,
        checker: &mut Checker<'_, '_>,
        failure: Error,
    ) -> Result<(), Error> {
        let Some(attempt) = self.attempt.take() else {
            return Err(failure);
        };
        let value = attempt.finish::<n::Expr>(checker, Err(failure))?;
        self.accept(checker, value)
    }

    fn leave_premise(&mut self, checker: &mut Checker<'_, '_>) {
        if let Some(premise) = self.premise.take() {
            premise.leave(checker);
        }
    }

    pub(super) fn unwind(mut self, checker: &mut Checker<'_, '_>) {
        self.leave_premise(checker);
    }
}

impl std::fmt::Display for Frame<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(logical-frame (source {}) {} (state ",
            source(self.source.span),
            self.destination
        )?;
        match &self.state {
            State::Start => f.write_str("start")?,
            State::Left => f.write_str("left")?,
            State::HaveLeft(left) => write!(f, "(have-left {left})")?,
            State::DivergingRight(left) => write!(f, "(diverging-right {left})")?,
            State::Peers(progress) => progress.fmt(f)?,
            State::Pair(left, right) => write!(f, "(pair {left} {right})")?,
            State::Right { left, domain } => write!(f, "(right (domain {domain}) {left})")?,
            State::Finished(value) => write!(f, "(finished {value})")?,
            State::Complete => f.write_str("complete")?,
        }
        f.write_str(") (effect-delivery")?;
        if let Some(preserve) = self.preserve_effect {
            write!(f, " (preserve {preserve})")?;
        }
        f.write_str(") (attempt")?;
        if let Some(attempt) = &self.attempt {
            write!(f, " {attempt}")?;
        }
        f.write_str(") (premise")?;
        if let Some(premise) = &self.premise {
            write!(f, " {premise}")?;
        }
        f.write_str("))")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_frame_dump_keeps_pending_right_separate_from_checked_left() {
        let span = Span::default();
        let source_expr = Expr {
            hir_id: hir::HirId::new(
                hir::OwnerId::new(hir::LocalDefId::new(1)),
                hir::ItemLocalId::new(2),
            ),
            span,
            kind: H::Unit,
        };
        let frame = Frame {
            source: source_expr,
            destination: Destination::statement(),
            state: State::Right {
                left: n::Expr {
                    source: source(span),
                    ty: p(P::Bool),
                    kind: K::Block {
                        statements: vec![n::Expr {
                            source: source(span),
                            ty: p(P::Str),
                            kind: K::Literal(n::Literal::String("kept\"\n".into())),
                        }],
                        result: Some(Box::new(n::Expr {
                            source: source(span),
                            ty: p(P::Bool),
                            kind: K::Literal(n::Literal::Bool(true)),
                        })),
                    },
                },
                domain: p(P::Bool),
            },
            attempt: None,
            premise: None,
            preserve_effect: Some(false),
        };
        let dump = frame.to_string();
        for fragment in [
            "(logical-frame ",
            "(state (right (domain (primitive bool))",
            "(effect-delivery (preserve false))",
            "(attempt)",
            "(premise)",
            "kept\\\"\\n",
        ] {
            assert!(dump.contains(fragment), "missing {fragment}: {dump}");
        }
        assert!(!dump.contains('\n'), "{dump}");
    }
}
