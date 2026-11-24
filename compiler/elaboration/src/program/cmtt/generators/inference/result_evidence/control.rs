//! A function can deliver through a lexical return or through the tail of its
//! body. Keep the values of these paths separate until the function boundary.
use super::*;

// Closed has no normal completion. Unknown may complete with a value whose
// identity is not fixed. Only the latter can invalidate a known alternative.
pub(super) enum Paths {
    Closed,
    Known(Term),
    Unknown,
}

impl Paths {
    pub(super) fn known(self) -> Option<Term> {
        match self {
            Self::Known(value) => Some(value),
            _ => None,
        }
    }

    fn join(self, other: Self) -> Self {
        match (self, other) {
            (Self::Closed, other) | (other, Self::Closed) => other,
            (Self::Known(a), Self::Known(b)) if a == b => Self::Known(a),
            _ => Self::Unknown,
        }
    }

    fn void(expected: Option<&Term>) -> Self {
        if expected.is_none_or(|domain| *domain == p(P::Void)) {
            Self::Known(Term::Unit)
        } else {
            Self::Unknown
        }
    }

    fn discarded(self, expected: Option<&Term>) -> Self {
        match self {
            Self::Closed => Self::Closed,
            _ => Self::void(expected),
        }
    }
}

pub(super) fn result(
    checker: &mut Checker<'_, '_>,
    body: &Expr<'_>,
    domain: &Term,
    depth: usize,
) -> Result<Paths, Error> {
    let mut flow = Observation {
        result: domain,
        returns: Paths::Closed,
    };
    let tail = flow.expr(checker, body, Some(domain), depth)?;
    Ok(flow.returns.join(tail))
}

pub(super) struct Observation<'a> {
    result: &'a Term,
    returns: Paths,
}

impl Observation<'_> {
    fn opaque(&mut self) -> Paths {
        // Unknown control may leave the observed function before its tail.
        self.returns = Paths::Unknown;
        Paths::Unknown
    }

    fn block(
        &mut self,
        checker: &mut Checker<'_, '_>,
        block: &hir::expr::Block<'_>,
        expected: Option<&Term>,
        depth: usize,
    ) -> Result<Paths, Error> {
        for statement in block.stmts {
            let next = self.expr(checker, statement, None, depth)?;
            if matches!(next, Paths::Closed) || matches!(self.returns, Paths::Unknown) {
                return Ok(next);
            }
        }
        match block.expr {
            Some(tail) => self.expr(checker, tail, expected, depth),
            None => Ok(Paths::void(expected)),
        }
    }

    pub(super) fn expr(
        &mut self,
        checker: &mut Checker<'_, '_>,
        value: &Expr<'_>,
        expected: Option<&Term>,
        depth: usize,
    ) -> Result<Paths, Error> {
        checker.required_steps = checker
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(value.span, "call result control evidence budget exhausted"))?;
        if depth >= 64 {
            return Ok(self.opaque());
        }
        let depth = depth + 1;
        match value.kind {
            H::Block(block) => self.block(checker, block, expected, depth),
            H::Semi(inner) => Ok(self.expr(checker, inner, None, depth)?.discarded(expected)),
            H::Return(output) => {
                let delivered = match output {
                    Some(output) => self.expr(checker, output, Some(self.result), depth)?,
                    None => Paths::void(Some(self.result)),
                };
                let earlier = std::mem::replace(&mut self.returns, Paths::Closed);
                self.returns = earlier.join(delivered);
                Ok(Paths::Closed)
            }
            H::If(guard, yes, no) => {
                let selected = self.expr(checker, guard, Some(&p(P::Bool)), depth)?;
                if matches!(selected, Paths::Closed) {
                    return Ok(Paths::Closed);
                }
                let taken = match selected {
                    Paths::Known(Term::Bool(false)) => Paths::Closed,
                    _ if no.is_none() => self.block(checker, yes, None, depth)?.discarded(expected),
                    _ => self.block(checker, yes, expected, depth)?,
                };
                if matches!(selected, Paths::Known(Term::Bool(true))) {
                    return Ok(taken);
                }
                let other = match no {
                    Some(no) => self.expr(checker, no, expected, depth)?,
                    None => Paths::void(expected),
                };
                Ok(taken.join(other))
            }
            H::When(arms) => {
                let has_fallback = arms.last().is_some_and(|arm| arm.cond.is_none());
                let mut tails = Paths::Closed;
                for arm in arms {
                    let selected = match arm.cond {
                        Some(guard) => self.expr(checker, guard, Some(&p(P::Bool)), depth)?,
                        None => Paths::Known(Term::Bool(true)),
                    };
                    match selected {
                        Paths::Closed => return Ok(tails),
                        Paths::Known(Term::Bool(false)) => continue,
                        _ => {}
                    }
                    let taken = if has_fallback {
                        self.expr(checker, arm.body, expected, depth)?
                    } else {
                        self.expr(checker, arm.body, None, depth)?
                            .discarded(expected)
                    };
                    tails = tails.join(taken);
                    if matches!(selected, Paths::Known(Term::Bool(true))) {
                        return Ok(tails);
                    }
                }
                Ok(tails.join(Paths::void(expected)))
            }
            H::Unary(UnOp::Not, inner) if expected == Some(&p(P::Bool)) => {
                let operand = self.expr(checker, inner, Some(&p(P::Bool)), depth)?;
                Ok(match operand {
                    Paths::Known(Term::Bool(value)) => Paths::Known(Term::Bool(!value)),
                    Paths::Closed => Paths::Closed,
                    _ => Paths::Unknown,
                })
            }
            // BitVec shares these tokens and evaluates both operands. Only
            // a Boolean destination establishes short-circuit control here.
            H::Binary(op @ (BinOp::And | BinOp::Or), left, right)
                if expected == Some(&p(P::Bool)) =>
            {
                let short = op == BinOp::Or;
                let left = self.expr(checker, left, Some(&p(P::Bool)), depth)?;
                if matches!(left, Paths::Closed) {
                    return Ok(Paths::Closed);
                }
                if matches!(left, Paths::Known(Term::Bool(value)) if value == short) {
                    return Ok(left);
                }
                let right = self.expr(checker, right, Some(&p(P::Bool)), depth)?;
                let value = if matches!(left, Paths::Known(Term::Bool(_))) {
                    right
                } else {
                    right.join(Paths::Known(Term::Bool(short)))
                };
                Ok(value)
            }
            H::Assumes(guard) | H::Asserts(guard) | H::Invariant(guard) => {
                let guard = self.expr(checker, guard, Some(&p(P::Bool)), depth)?;
                Ok(match guard {
                    Paths::Known(Term::Bool(false)) => Paths::Closed,
                    _ => guard.discarded(expected),
                })
            }
            H::Decreases(metric) => {
                let _ = self.expr(checker, metric, None, depth)?;
                Ok(Paths::void(expected))
            }
            H::Application(callee, args) => {
                let observed = checker.call_result_paths(
                    callee,
                    args,
                    expected,
                    depth,
                    Caller::Function(self),
                )?;
                Ok(observed.unwrap_or_else(|| self.opaque()))
            }
            H::Unit => Ok(Paths::void(expected)),
            _ => {
                if let Some(domain) = expected
                    && let Some(result) =
                        checker.generator_static_value_hint(value, domain, depth, FORMALS)?
                {
                    return Ok(Paths::Known(result));
                }
                Ok(if direct_read(value) {
                    Paths::Unknown
                } else {
                    self.opaque()
                })
            }
        }
    }
}
