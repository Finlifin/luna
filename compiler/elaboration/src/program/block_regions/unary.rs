//! Unary operands use the same retained child checks as binary operators.
//! Protocol probing observes a domain; it never executes the operand.
use super::*;
use driver::{Request, Step};

pub(super) fn operator(expr: &Expr<'_>) -> Option<crate::arithmetic::Operator> {
    use crate::arithmetic::Operator as O;
    match expr.kind {
        H::Unary(UnOp::Neg, _) => Some(O::Negate),
        H::Unary(UnOp::Not, _) => Some(O::Invert),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug)]
enum Phase {
    Start,
    Select,
    Native,
    Complete,
    Finished,
}

pub(super) struct Frame<'h> {
    source: Expr<'h>,
    operand: &'h Expr<'h>,
    operator: crate::arithmetic::Operator,
    destination: Destination,
    protocol: Option<DeclarationId>,
    phase: Phase,
    pending: bool,
    attempt: Option<checking_attempts::Attempt>,
    checked: Option<n::Expr>,
    preserve_effect: Option<bool>,
}

impl<'h> Frame<'h> {
    pub(super) fn new(
        checker: &mut Checker<'_, '_>,
        source: &Expr<'h>,
        destination: Destination,
        own_effect_delivery: bool,
    ) -> Result<Self, Error> {
        let operator =
            operator(source).ok_or_else(|| error(source.span, "expected a unary operator"))?;
        let H::Unary(_, operand) = source.kind else {
            unreachable!()
        };
        if destination.ordinary_view().is_none() {
            return Err(error(
                source.span,
                "unary operator requires an ordinary destination",
            ));
        }
        Ok(Self {
            source: source.clone(),
            operand,
            operator,
            destination,
            protocol: checker.protocols.arithmetic[operator as usize],
            phase: Phase::Start,
            pending: false,
            attempt: None,
            checked: None,
            preserve_effect: own_effect_delivery
                .then(|| std::mem::take(&mut checker.preserve_effect_value)),
        })
    }

    fn request(&mut self, checker: &mut Checker<'_, '_>, probe: bool) -> Result<Step<'h>, Error> {
        let destination = Destination::ordinary(
            checker,
            OrdinaryTail {
                expected: if probe {
                    None
                } else {
                    self.destination.expected()
                },
                partial: None,
                input: None,
            },
            self.operand.span,
        )?;
        if probe {
            self.attempt = Some(checking_attempts::Attempt::begin(
                checker,
                self.operand.span,
                false,
                CheckingDisposition::Commit,
            )?);
        }
        self.pending = true;
        Ok(Step::Child(Request::expression(self.operand, destination)))
    }

    fn accept(&mut self, checked: Option<n::Expr>) -> Result<(), Error> {
        if !self.pending {
            return Err(error(self.source.span, "unsolicited unary operand"));
        }
        self.pending = false;
        self.checked = checked;
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
            self.accept(value)?;
        }
        if self.pending {
            return Err(error(self.source.span, "unary operand is still pending"));
        }
        match self.phase {
            Phase::Start => {
                // Signed literal formation must not first fit the positive
                // magnitude into the destination's signed range.
                let signed_literal = self.operator == crate::arithmetic::Operator::Negate
                    && is_numeric_literal(self.operand);
                if !signed_literal && self.protocol.is_some() {
                    self.phase = Phase::Select;
                    return self.request(checker, true);
                }
                self.phase = Phase::Native;
            }
            Phase::Select => {
                // A probe disabled numeric defaults. Do not repeat it unless
                // the destination or restored defaults provide new evidence.
                if self.checked.is_none()
                    && self.destination.expected().is_none()
                    && !checker.numeric_defaults
                {
                    return Err(context_error(
                        self.source.span,
                        "unary operand requires a type context",
                    ));
                }
                if self
                    .checked
                    .as_ref()
                    .is_some_and(|value| !is_never(&value.ty))
                {
                    if let Some(value) = checker.unary_protocol_result(
                        &self.source,
                        self.destination.expected(),
                        self.operator,
                        self.protocol.expect("protocol probe"),
                        &mut self.checked,
                    )? {
                        return self.finish(checker, value);
                    }
                }
                self.phase = Phase::Native;
            }
            Phase::Native => {
                // Nested negation still checks the inner operator normally.
                // Only an immediate literal is formed together with its sign.
                let immediate_literal = self.operator == crate::arithmetic::Operator::Negate
                    && matches!(
                        self.operand.kind,
                        H::Lit(hir::Lit {
                            kind: hir::LitKind::Integer(_) | hir::LitKind::Float(_),
                            ..
                        })
                    );
                self.phase = Phase::Complete;
                if self.checked.is_none() && !immediate_literal {
                    return self.request(checker, false);
                }
            }
            Phase::Complete => {
                let value = checker.native_unary_expression(
                    &self.source,
                    self.destination.expected(),
                    self.checked.take(),
                )?;
                return self.finish(checker, value);
            }
            Phase::Finished => {
                return Err(error(self.source.span, "unary operator already completed"));
            }
        }
        Ok(Step::Continue)
    }

    fn finish(
        &mut self,
        checker: &mut Checker<'_, '_>,
        mut value: n::Expr,
    ) -> Result<Step<'h>, Error> {
        self.phase = Phase::Finished;
        if self.preserve_effect == Some(false) {
            value =
                checker.run_computation(value, self.destination.expected(), self.source.span)?;
        }
        Ok(Step::Completed(CheckedBlock::Ordinary(value)))
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
        self.accept(value)
    }
}

impl std::fmt::Display for Frame<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(unary-frame (source {}) (operator {:?}) (phase {:?}) (pending {}) {} (operand {} (source {}) (checked",
            source(self.source.span),
            self.operator,
            self.phase,
            self.pending,
            self.destination,
            self.operand.hir_id,
            source(self.operand.span)
        )?;
        if let Some(value) = &self.checked {
            write!(f, " {value}")?;
        }
        f.write_str(")) (protocol")?;
        if let Some(protocol) = self.protocol {
            write!(f, " {protocol}")?;
        }
        f.write_str(") (effect-delivery")?;
        if let Some(preserve) = self.preserve_effect {
            write!(f, " (preserve {preserve})")?;
        }
        f.write_str(") (attempt")?;
        if let Some(attempt) = &self.attempt {
            write!(f, " {attempt}")?;
        }
        f.write_str("))")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unary_frame_dump_and_child_admission_preserve_retained_work() {
        let operand = Expr {
            hir_id: hir::HirId::new(
                hir::OwnerId::new(hir::LocalDefId::new(1)),
                hir::ItemLocalId::new(2),
            ),
            span: Span::default(),
            kind: H::Unit,
        };
        let mut frame = Frame {
            source: operand.clone(),
            operand: &operand,
            operator: crate::arithmetic::Operator::Invert,
            destination: Destination::statement(),
            protocol: Some(DeclarationId {
                package: 3,
                local: 4,
            }),
            phase: Phase::Select,
            pending: true,
            attempt: None,
            checked: None,
            preserve_effect: Some(false),
        };
        let waiting = frame.to_string();
        for text in [
            "(unary-frame",
            "(operator Invert)",
            "(phase Select)",
            "(pending true)",
            "(checked)",
            "(preserve false)",
        ] {
            assert!(waiting.contains(text), "{waiting}");
        }
        let value = n::Expr {
            source: source(operand.span),
            ty: p(P::Str),
            kind: K::Literal(n::Literal::String("child\"\n".into())),
        };
        frame.accept(Some(value)).unwrap();
        let retained = frame.to_string();
        assert!(retained.contains("child\\\"\\n"), "{retained}");
        assert!(!retained.contains('\n'), "{retained}");
        assert!(frame.accept(None).is_err());
        assert_eq!(
            frame.to_string(),
            retained,
            "an unsolicited child replaced checked code"
        );
    }
}
