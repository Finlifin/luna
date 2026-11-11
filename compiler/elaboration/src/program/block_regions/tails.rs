//! Destination-specific tail checking; binding completion belongs to the region.
use super::*;

#[derive(Clone, Copy)]
pub(in crate::program) struct OrdinaryTail<'a> {
    pub expected: Option<&'a Term>,
    pub partial: Option<&'a crate::calls::CallableContext>,
    pub input: Option<&'a crate::calls::InputContext>,
}

pub(in crate::program) struct OrdinaryResult {
    ty: Term,
    value: Option<Box<n::Expr>>,
}

impl Tail for OrdinaryTail<'_> {
    type Checked = OrdinaryResult;
    type Output = n::Expr;

    fn check(
        &self,
        checker: &mut Checker<'_, '_>,
        block: &hir::Block<'_>,
        diverges: bool,
    ) -> Result<Self::Checked, Error> {
        let value = match self.prepare(checker, block, diverges)? {
            TailCheck::Checked(value) => value,
            TailCheck::Child => Some(self.check_expression(checker, block.expr.unwrap())?),
        };
        self.complete(checker, block, diverges, value)
    }

    fn references(
        checked: &Self::Checked,
        fuel: &mut usize,
    ) -> Result<BTreeSet<n::LocalId>, nir::preparation::Error> {
        checked
            .value
            .as_deref()
            .map(|value| nir::preparation::references(value, fuel))
            .transpose()
            .map(Option::unwrap_or_default)
    }

    fn finish(self, checked: Self::Checked, statements: Vec<n::Expr>, span: Span) -> n::Expr {
        n::Expr {
            ty: checked.ty,
            source: source(span),
            kind: K::Block {
                statements,
                result: checked.value,
            },
        }
    }
}

pub(super) enum TailCheck {
    Checked(Option<n::Expr>),
    Child,
}
impl std::fmt::Display for TailCheck {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Child => f.write_str("(tail-check child)"),
            Self::Checked(Some(value)) => write!(f, "(tail-check (checked {value}))"),
            Self::Checked(None) => f.write_str("(tail-check (checked void))"),
        }
    }
}
impl OrdinaryTail<'_> {
    pub(super) fn prepare(
        &self,
        checker: &mut Checker<'_, '_>,
        block: &hir::Block<'_>,
        diverges: bool,
    ) -> Result<TailCheck, Error> {
        let Some(value) = block.expr else {
            return Ok(TailCheck::Checked(None));
        };
        // Unreachable tails still receive ordinary checking; an independently
        // typed tail need not be forced into the block's result destination.
        if diverges
            && checker.runtime_value_context()
            && checker.preparing_spread.is_none()
            && self.input.is_none()
            && let Some(checked) = checker.argument_attempt(value, None, None)?
        {
            return Ok(TailCheck::Checked(Some(checked)));
        }
        Ok(TailCheck::Child)
    }

    pub(super) fn check_expression(
        &self,
        checker: &mut Checker<'_, '_>,
        value: &Expr<'_>,
    ) -> Result<n::Expr, Error> {
        match self.input {
            Some(input) if self.expected.is_none() => {
                checker.input_context_expression(value, input)
            }
            _ => checker.expr_context(value, self.expected, self.partial),
        }
    }

    pub(super) fn complete(
        &self,
        checker: &mut Checker<'_, '_>,
        block: &hir::Block<'_>,
        diverges: bool,
        value: Option<n::Expr>,
    ) -> Result<OrdinaryResult, Error> {
        let mut value = value.map(Box::new);
        if value.is_none() && !diverges {
            value = checker
                .implicit_error_success(self.expected, block.span)?
                .map(Box::new);
        }
        let ty = if diverges {
            p(P::Never)
        } else {
            value
                .as_ref()
                .map(|value| value.ty.clone())
                .unwrap_or(p(P::Void))
        };
        if let Some(expected) = self.expected {
            checker.require(&ty, expected, block.span)?;
        }
        Ok(OrdinaryResult { ty, value })
    }
}

pub(in crate::program) struct ContextualTail {
    pub depth: usize,
}

impl Tail for ContextualTail {
    type Checked = Option<Box<n::NormalFormArgument>>;
    type Output = n::NormalFormArgument;

    fn check(
        &self,
        checker: &mut Checker<'_, '_>,
        block: &hir::Block<'_>,
        _diverges: bool,
    ) -> Result<Self::Checked, Error> {
        block
            .expr
            .map(|value| checker.dependent_normal_argument(value, self.depth + 1))
            .transpose()
            .map(|value| value.map(Box::new))
    }

    fn references(
        checked: &Self::Checked,
        fuel: &mut usize,
    ) -> Result<BTreeSet<n::LocalId>, nir::preparation::Error> {
        checked
            .as_deref()
            .map(|value| nir::preparation::argument_references(value, fuel))
            .transpose()
            .map(Option::unwrap_or_default)
    }

    fn finish(self, result: Self::Checked, statements: Vec<n::Expr>, span: Span) -> Self::Output {
        n::NormalFormArgument::Block {
            source: source(span),
            statements,
            result,
        }
    }
}

impl std::fmt::Display for OrdinaryTail<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(ordinary-tail (expected")?;
        if let Some(expected) = self.expected {
            write!(f, " {expected}")?;
        }
        f.write_str(") (partial")?;
        if let Some(partial) = self.partial {
            write!(f, " {partial}")?;
        }
        f.write_str(") (input")?;
        if let Some(input) = self.input {
            write!(f, " {input}")?;
        }
        f.write_str("))")
    }
}

impl std::fmt::Display for OrdinaryResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(checked-block-tail (type {}) (value", self.ty)?;
        if let Some(value) = &self.value {
            write!(f, " {value}")?;
        }
        f.write_str("))")
    }
}

impl std::fmt::Display for ContextualTail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(contextual-tail (depth {}))", self.depth)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_dump_retains_nested_context_and_escaped_values() {
        let target = Term::NormalForm {
            head: DeclarationId {
                package: 3,
                local: 7,
            },
            sort: Sort::Type,
            arguments: vec![Term::String("tail\"\n".into())],
        };
        let input = crate::calls::InputContext::Optional(Box::new(
            crate::calls::InputContext::Known(target.clone()),
        ));
        let tail = OrdinaryTail {
            expected: Some(&target),
            partial: None,
            input: Some(&input),
        };
        let dump = tail.to_string();
        assert!(dump.contains(&format!("(expected {target})")), "{dump}");
        assert!(dump.contains(&format!("(input {input})")), "{dump}");
        assert!(dump.contains("tail\\\"\\n"), "{dump}");
        assert!(!dump.contains('\n'));
    }
}
