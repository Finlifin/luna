//! A shared block driver retains the destination without inventing a type for
//! contextual results. Ordinary statement children always use the ordinary rule.
use super::*;
use tails::{OrdinaryResult, TailCheck};

#[derive(Clone)]
pub(super) enum Destination {
    Ordinary(std::sync::Arc<OrdinaryDestination>),
    Contextual(ContextualTail),
    /// A whole declaration input starts at contextual depth zero.
    ContextualInput,
}

/// Owned only by this checking region. Sharing nested destinations does not
/// turn partial candidate information into a stable type or executable input.
pub(super) struct OrdinaryDestination {
    expected: Option<Term>,
    partial: Option<crate::calls::CallableContext>,
    input: Option<crate::calls::InputContext>,
}

impl OrdinaryDestination {
    fn copy(
        tail: OrdinaryTail<'_>,
        fuel: &mut usize,
    ) -> Result<Self, nir::copy_budget::CopyBudgetExhausted> {
        *fuel = fuel
            .checked_sub(1)
            .ok_or(nir::copy_budget::CopyBudgetExhausted::Steps)?;
        Ok(Self {
            expected: tail
                .expected
                .map(|value| nir::copy_budget::clone_term(value, fuel))
                .transpose()?,
            partial: tail
                .partial
                .map(|value| value.clone_bounded(fuel))
                .transpose()?,
            input: tail
                .input
                .map(|value| value.clone_bounded(fuel))
                .transpose()?,
        })
    }

    fn view(&self) -> OrdinaryTail<'_> {
        OrdinaryTail {
            expected: self.expected.as_ref(),
            partial: self.partial.as_ref(),
            input: self.input.as_ref(),
        }
    }
}

impl std::fmt::Display for OrdinaryDestination {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.view().fmt(f)
    }
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

impl Destination {
    pub(super) fn ordinary(
        checker: &mut Checker<'_, '_>,
        tail: OrdinaryTail<'_>,
        span: Span,
    ) -> Result<Self, Error> {
        let before = usize::try_from(checker.required_steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let retained = OrdinaryDestination::copy(tail, &mut fuel);
        checker.required_steps -= (before - fuel) as u64;
        let retained = retained.map_err(|failure| {
            error(
                span,
                match failure {
                    nir::copy_budget::CopyBudgetExhausted::Steps => {
                        "block destination copy budget exhausted"
                    }
                    nir::copy_budget::CopyBudgetExhausted::Depth => {
                        "block destination copy depth exhausted"
                    }
                },
            )
        })?;
        Ok(Self::Ordinary(std::sync::Arc::new(retained)))
    }

    pub(super) fn statement() -> Self {
        Self::Ordinary(std::sync::Arc::new(OrdinaryDestination {
            expected: None,
            partial: None,
            input: None,
        }))
    }

    pub(super) fn ordinary_view(&self) -> Option<OrdinaryTail<'_>> {
        match self {
            Self::Ordinary(tail) => Some(tail.view()),
            Self::Contextual(_) | Self::ContextualInput => None,
        }
    }

    pub(super) fn contextual_depth(&self) -> Option<usize> {
        match self {
            Self::Ordinary(_) => None,
            Self::Contextual(tail) => Some(tail.depth + 1),
            Self::ContextualInput => Some(0),
        }
    }

    pub(super) fn structural_input(&self) -> Option<&crate::calls::InputContext> {
        use crate::calls::InputContext as C;
        let view = self.ordinary_view()?;
        if view.expected.is_some() {
            return None;
        }
        view.input.filter(|input| {
            matches!(
                input,
                C::Optional(_)
                    | C::Tuple(_)
                    | C::NormalTuple { .. }
                    | C::Record(_)
                    | C::Nominal { .. }
                    | C::ErrorResult { .. }
            )
        })
    }

    pub(super) fn with_expected(
        &self,
        checker: &mut Checker<'_, '_>,
        expected: Option<&Term>,
        span: Span,
    ) -> Result<Self, Error> {
        let view = self
            .ordinary_view()
            .ok_or_else(|| error(span, "expected an ordinary destination"))?;
        Self::ordinary(checker, OrdinaryTail { expected, ..view }, span)
    }

    pub(super) fn deliver_input(
        &self,
        checker: &mut Checker<'_, '_>,
        value: n::Expr,
        span: Span,
    ) -> Result<n::Expr, Error> {
        if let Some(crate::calls::InputContext::ErrorResult { errors, .. }) =
            self.structural_input()
        {
            checker.deliver_error_context(value, errors, span)
        } else {
            Ok(value)
        }
    }

    pub(super) fn following_branch(
        &self,
        checker: &mut Checker<'_, '_>,
        previous: &Term,
        span: Span,
    ) -> Result<Self, Error> {
        let Some(view) = self.ordinary_view() else {
            return Ok(self.clone());
        };
        if view.expected.is_some() || is_never(previous) {
            return Ok(self.clone());
        }
        Self::ordinary(
            checker,
            OrdinaryTail {
                expected: Some(previous),
                ..view
            },
            span,
        )
    }

    pub(super) fn expected(&self) -> Option<&Term> {
        match self {
            Self::Ordinary(tail) => tail.expected.as_ref(),
            Self::Contextual(_) | Self::ContextualInput => None,
        }
    }

    pub(super) fn prepare(
        &self,
        checker: &mut Checker<'_, '_>,
        block: &hir::Block<'_>,
        diverges: bool,
    ) -> Result<TailCheck<CheckedBlock>, Error> {
        match self {
            Self::Ordinary(tail) => Ok(match tail.view().prepare(checker, block, diverges)? {
                TailCheck::Child => TailCheck::Child,
                TailCheck::Checked(value) => TailCheck::Checked(value.map(CheckedBlock::Ordinary)),
            }),
            Self::Contextual(_) | Self::ContextualInput => Ok(if block.expr.is_some() {
                TailCheck::Child
            } else {
                TailCheck::Checked(None)
            }),
        }
    }

    pub(super) fn complete(
        &self,
        checker: &mut Checker<'_, '_>,
        block: &hir::Block<'_>,
        diverges: bool,
        value: Option<CheckedBlock>,
    ) -> Result<CheckedTail, Error> {
        match self {
            Self::Ordinary(tail) => tail
                .view()
                .complete(
                    checker,
                    block,
                    diverges,
                    value.map(|value| value.ordinary(block.span)).transpose()?,
                )
                .map(CheckedTail::Ordinary),
            Self::Contextual(_) | Self::ContextualInput => value
                .map(|value| value.contextual(block.span).map(Box::new))
                .transpose()
                .map(CheckedTail::Contextual),
        }
    }

    pub(super) fn check_expression(
        &self,
        checker: &mut Checker<'_, '_>,
        value: &Expr<'_>,
    ) -> Result<CheckedBlock, Error> {
        match self {
            Self::Ordinary(tail) => tail
                .view()
                .check_expression(checker, value)
                .map(CheckedBlock::Ordinary),
            Self::Contextual(_) | Self::ContextualInput => checker
                .dependent_normal_argument(value, self.contextual_depth().unwrap())
                .map(CheckedBlock::Contextual),
        }
    }

    /// A nested plain block can stay on the same driver. Partial ordinary
    /// inputs keep their delivery boundary. Contextual depth still counts both the
    /// expression and block rule, exactly as the source dispatcher does.
    pub(super) fn nested(&self, expr: &Expr<'_>) -> Result<Option<Self>, Error> {
        let H::Block(block) = expr.kind else {
            return Ok(None);
        };
        match self {
            Self::Ordinary(tail) => Ok((tail.input.is_none()
                || tail.expected.is_some()
                || self.structural_input().is_some())
            .then(|| self.clone())),
            Self::ContextualInput => Ok(Some(Self::Contextual(ContextualTail { depth: 1 }))),
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

impl Tail for Destination {
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

impl std::fmt::Display for Destination {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ordinary(tail) => tail.fmt(f),
            Self::Contextual(tail) => tail.fmt(f),
            Self::ContextualInput => f.write_str("(contextual-input (depth 0))"),
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
    fn retained_targets_outlive_borrowed_inputs_and_share_nested_payloads() {
        let retained = {
            let ty = Term::Primitive(P::Bool);
            let input = crate::calls::InputContext::Optional(Box::new(
                crate::calls::InputContext::UnknownDependentValue,
            ));
            let partial = crate::calls::CallableContext {
                comptime: true,
                open_environment: true,
                environment: None,
                parameters: vec![None],
                result: None,
                result_context: Some(Box::new(input.clone())),
            };
            let tail = OrdinaryTail {
                expected: Some(&ty),
                input: Some(&input),
                partial: Some(&partial),
            };
            assert!(OrdinaryDestination::copy(tail, &mut 0).is_err());
            Destination::Ordinary(std::sync::Arc::new(
                OrdinaryDestination::copy(tail, &mut 1000).unwrap(),
            ))
        };
        let nested = retained.clone();
        let (Destination::Ordinary(parent), Destination::Ordinary(child)) = (&retained, &nested)
        else {
            unreachable!()
        };
        assert!(std::sync::Arc::ptr_eq(parent, child));
        drop(retained);
        let dump = nested.to_string();
        assert!(dump.contains("(expected (primitive bool))"), "{dump}");
        assert!(dump.contains("(open-environment true)"), "{dump}");
        assert!(dump.contains("(unknown-dependent-value)"), "{dump}");
    }

    #[test]
    fn whole_contextual_inputs_keep_the_original_expression_depth() {
        let destination = Destination::ContextualInput;
        assert_eq!(destination.contextual_depth(), Some(0));
        assert_eq!(
            Destination::Contextual(ContextualTail { depth: 4 }).contextual_depth(),
            Some(5)
        );
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
        let nested = destination.nested(&expr).unwrap().unwrap();
        assert_eq!(nested.to_string(), "(contextual-tail (depth 1))");
        assert_eq!(nested.contextual_depth(), Some(2));
    }

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
