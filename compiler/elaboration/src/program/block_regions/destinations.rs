//! A shared block driver retains the destination without inventing a type for
//! contextual results. Ordinary statement children always use the ordinary rule.
use super::*;
use tails::{OrdinaryResult, TailCheck};

#[derive(Clone, Copy)]
pub(super) enum Destination<'a> {
    Ordinary(OrdinaryTail<'a>),
    Contextual(ContextualTail),
}

pub(super) enum CheckedBlock {
    Ordinary(n::Expr),
    Contextual(n::NormalFormArgument),
}

pub(super) enum CheckedTail {
    Ordinary(OrdinaryResult),
    Contextual(Option<Box<n::NormalFormArgument>>),
}

impl CheckedBlock {
    pub(super) fn ordinary(self, span: Span) -> Result<n::Expr, Error> {
        match self {
            Self::Ordinary(value) => Ok(value),
            Self::Contextual(_) => Err(error(span, "block requires an ordinary child result")),
        }
    }

    pub(super) fn contextual(self, span: Span) -> Result<n::NormalFormArgument, Error> {
        match self {
            Self::Contextual(value) => Ok(value),
            Self::Ordinary(_) => Err(error(span, "block requires a contextual child result")),
        }
    }
}

impl<'a> Destination<'a> {
    pub(super) fn expected(self) -> Option<&'a Term> {
        match self {
            Self::Ordinary(tail) => tail.expected,
            Self::Contextual(_) => None,
        }
    }

    pub(super) fn prepare(
        self,
        checker: &mut Checker<'_, '_>,
        block: &hir::Block<'_>,
        diverges: bool,
    ) -> Result<TailCheck<CheckedBlock>, Error> {
        match self {
            Self::Ordinary(tail) => Ok(match tail.prepare(checker, block, diverges)? {
                TailCheck::Child => TailCheck::Child,
                TailCheck::Checked(value) => TailCheck::Checked(value.map(CheckedBlock::Ordinary)),
            }),
            Self::Contextual(_) => Ok(if block.expr.is_some() {
                TailCheck::Child
            } else {
                TailCheck::Checked(None)
            }),
        }
    }

    pub(super) fn complete(
        self,
        checker: &mut Checker<'_, '_>,
        block: &hir::Block<'_>,
        diverges: bool,
        value: Option<CheckedBlock>,
    ) -> Result<CheckedTail, Error> {
        match self {
            Self::Ordinary(tail) => tail
                .complete(
                    checker,
                    block,
                    diverges,
                    value.map(|value| value.ordinary(block.span)).transpose()?,
                )
                .map(CheckedTail::Ordinary),
            Self::Contextual(_) => value
                .map(|value| value.contextual(block.span).map(Box::new))
                .transpose()
                .map(CheckedTail::Contextual),
        }
    }

    pub(super) fn check_expression(
        self,
        checker: &mut Checker<'_, '_>,
        value: &Expr<'_>,
    ) -> Result<CheckedBlock, Error> {
        match self {
            Self::Ordinary(tail) => tail
                .check_expression(checker, value)
                .map(CheckedBlock::Ordinary),
            Self::Contextual(tail) => checker
                .dependent_normal_argument(value, tail.depth + 1)
                .map(CheckedBlock::Contextual),
        }
    }

    /// A nested plain block can stay on the same driver. Partial ordinary
    /// inputs retain their own rule. Contextual depth still counts both the
    /// expression and block rule, exactly as the source dispatcher does.
    pub(super) fn nested(self, expr: &Expr<'_>) -> Result<Option<Self>, Error> {
        let H::Block(block) = expr.kind else {
            return Ok(None);
        };
        match self {
            Self::Ordinary(tail) => Ok(tail.input.is_none().then_some(self)),
            Self::Contextual(tail) => {
                if tail.depth >= 127 {
                    return Err(error(expr.span, "contextual input nesting limit exceeded"));
                }
                if tail.depth >= 126 {
                    return Err(error(block.span, "contextual input nesting limit exceeded"));
                }
                Ok(Some(Self::Contextual(ContextualTail {
                    depth: tail.depth + 2,
                })))
            }
        }
    }
}

impl Tail for Destination<'_> {
    type Checked = CheckedTail;
    type Output = CheckedBlock;

    fn references(
        checked: &CheckedTail,
        fuel: &mut usize,
    ) -> Result<BTreeSet<n::LocalId>, nir::preparation::Error> {
        match checked {
            CheckedTail::Ordinary(value) => OrdinaryTail::references(value, fuel),
            CheckedTail::Contextual(value) => ContextualTail::references(value, fuel),
        }
    }

    fn finish(checked: CheckedTail, statements: Vec<n::Expr>, span: Span) -> CheckedBlock {
        match checked {
            CheckedTail::Ordinary(value) => {
                CheckedBlock::Ordinary(OrdinaryTail::finish(value, statements, span))
            }
            CheckedTail::Contextual(value) => {
                CheckedBlock::Contextual(ContextualTail::finish(value, statements, span))
            }
        }
    }
}

impl std::fmt::Display for Destination<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ordinary(tail) => tail.fmt(f),
            Self::Contextual(tail) => tail.fmt(f),
        }
    }
}

impl std::fmt::Display for CheckedBlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ordinary(value) => write!(f, "(ordinary-block-result {value})"),
            Self::Contextual(value) => write!(f, "(contextual-block-result {value})"),
        }
    }
}

impl std::fmt::Display for CheckedTail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ordinary(value) => value.fmt(f),
            Self::Contextual(value) => {
                f.write_str("(checked-contextual-tail")?;
                if let Some(value) = value {
                    write!(f, " {value}")?;
                }
                f.write_str(")")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_contextual_blocks_keep_logical_depth_and_result_kind() {
        let id = hir::HirId::new(
            hir::OwnerId::new(hir::LocalDefId::new(1)),
            hir::ItemLocalId::new(0),
        );
        let block = hir::Block {
            hir_id: id,
            stmts: &[],
            expr: None,
            span: Span::default(),
        };
        let expr = Expr {
            hir_id: id,
            span: block.span,
            kind: H::Block(&block),
        };
        let tail = Destination::Contextual(ContextualTail { depth: 124 });
        let nested = tail.nested(&expr).unwrap().unwrap();
        assert_eq!(nested.to_string(), "(contextual-tail (depth 126))");
        assert!(nested.nested(&expr).is_err());
        let result = CheckedBlock::Contextual(n::NormalFormArgument::Integer {
            source: source(block.span),
            spelling: "7".into(),
            negative: false,
        });
        assert!(result.to_string().starts_with("(contextual-block-result "));
        assert!(result.ordinary(block.span).is_err());
        let checked = CheckedTail::Contextual(None);
        assert_eq!(checked.to_string(), "(checked-contextual-tail)");
        assert!(
            Destination::finish(checked, vec![], block.span)
                .contextual(block.span)
                .is_ok()
        );
    }
}
