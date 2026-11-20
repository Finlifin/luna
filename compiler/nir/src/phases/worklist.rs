//! Phase work retains lexical stage and direct-callee context. Charge pending
//! nodes before allocating their slots; popping an admitted node is free.
use super::{Error, Expr, Pattern, SourceRange};

pub(super) enum Work<'a> {
    Argument(&'a crate::program::NormalFormArgument, bool),
    Expr(&'a Expr, bool, bool), // direct callee, comptime execution context
    Pattern(&'a Pattern, bool, SourceRange),
}

impl Work<'_> {
    fn source(&self) -> SourceRange {
        match self {
            Self::Argument(input, _) => input.source(),
            Self::Expr(expr, ..) => expr.source,
            Self::Pattern(_, _, source) => *source,
        }
    }
}

pub(super) struct Worklist<'a> {
    pending: Vec<(Work<'a>, bool)>,
    pub(super) permits_unsafe: bool,
    owner: SourceRange,
}

impl<'a> Worklist<'a> {
    pub(super) fn new(owner: SourceRange, permits_unsafe: bool) -> Self {
        Self {
            pending: Vec::new(),
            permits_unsafe,
            owner,
        }
    }

    pub(super) fn charge(fuel: &mut usize, count: usize, source: SourceRange) -> Result<(), Error> {
        *fuel = fuel.checked_sub(count).ok_or(Error {
            kind: super::ErrorKind::Budget,
            source,
            message: "phase traversal budget exhausted",
        })?;
        Ok(())
    }

    pub(super) fn push(&mut self, node: Work<'a>, fuel: &mut usize) -> Result<(), Error> {
        Self::charge(fuel, 1, node.source())?;
        self.pending.push((node, self.permits_unsafe));
        Ok(())
    }

    pub(super) fn extend(
        &mut self,
        nodes: impl IntoIterator<Item = Work<'a>>,
        fuel: &mut usize,
    ) -> Result<(), Error> {
        let nodes = nodes.into_iter();
        let (lower, upper) = nodes.size_hint();
        if upper == Some(lower) {
            // Slices and mapped parameter lists reserve the whole width before
            // iteration. A failed reservation cannot grow the pending stack.
            Self::charge(fuel, lower, self.owner)?;
            self.pending
                .extend(nodes.map(|node| (node, self.permits_unsafe)));
        } else {
            // Filtered captures and contextual inputs can have unknown width.
            // Their individual nodes still cannot be queued without payment.
            for node in nodes {
                self.push(node, fuel)?;
            }
        }
        Ok(())
    }

    pub(super) fn pop(&mut self) -> Option<Work<'a>> {
        let (node, permits_unsafe) = self.pending.pop()?;
        self.permits_unsafe = permits_unsafe;
        self.owner = node.source();
        Some(node)
    }

    pub(super) fn argument_children(
        &mut self,
        input: &'a crate::program::NormalFormArgument,
        ct: bool,
        fuel: &mut usize,
    ) -> Result<(), Error> {
        use crate::program::NormalFormArgument as A;
        let expr = |e| Work::Expr(e, false, ct);
        let arg = |a| Work::Argument(a, ct);
        match input {
            A::Value(value) | A::ListSpread(value) => self.push(expr(value), fuel)?,
            A::Null { .. } | A::Unit { .. } | A::Integer { .. } => {}
            A::List { items, .. } | A::Tuple { items, .. } => {
                self.extend(items.iter().map(arg), fuel)?;
            }
            A::If {
                condition,
                then_input,
                else_input,
                ..
            } => {
                self.push(expr(condition), fuel)?;
                self.push(arg(then_input), fuel)?;
                self.push(arg(else_input), fuel)?;
            }
            A::Block {
                statements, result, ..
            } => {
                self.extend(statements.iter().map(expr), fuel)?;
                self.extend(result.as_deref().map(arg), fuel)?;
            }
            A::Match {
                value,
                arms,
                source,
            } => {
                self.push(expr(value), fuel)?;
                for arm in arms {
                    self.push(Work::Pattern(&arm.pattern, ct, *source), fuel)?;
                    self.extend(arm.guard.iter().map(expr), fuel)?;
                    self.push(arg(&arm.body), fuel)?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Primitive, Term, program::ExprKind};

    fn expression() -> Expr {
        Expr {
            source: SourceRange { start: 30, end: 40 },
            ty: Term::Primitive(Primitive::Void),
            kind: ExprKind::Unit,
        }
    }

    #[test]
    fn contextual_literals_reserve_their_width_even_without_expression_children() {
        use crate::program::NormalFormArgument as A;
        let source = SourceRange { start: 10, end: 20 };
        let input = A::List {
            source,
            items: vec![A::Unit { source }; 80],
        };
        let mut work = Worklist::new(source, false);
        let mut limited = 20;
        assert!(work.argument_children(&input, true, &mut limited).is_err());
        assert_eq!(limited, 20);
        assert!(work.pop().is_none());
        let mut exact = 80;
        work.argument_children(&input, true, &mut exact).unwrap();
        assert_eq!(exact, 0);
        assert_eq!(std::iter::from_fn(|| work.pop()).count(), 80);
    }

    #[test]
    fn exact_width_is_reserved_before_iteration_and_preserves_existing_work() {
        let expr = expression();
        let owner = SourceRange { start: 1, end: 2 };
        let mut work = Worklist::new(owner, false);
        let mut fuel = 4;
        work.push(Work::Expr(&expr, true, false), &mut fuel)
            .unwrap();
        let visited = std::cell::Cell::new(0);
        let batch = (0..10).map(|_| {
            visited.set(visited.get() + 1);
            Work::Expr(&expr, false, true)
        });
        let error = work.extend(batch, &mut fuel).unwrap_err();
        assert_eq!(error.source, owner);
        assert_eq!(error.message, "phase traversal budget exhausted");
        assert_eq!(visited.get(), 0);
        assert_eq!(fuel, 3);
        assert!(
            matches!(work.pop(), Some(Work::Expr(value, true, false)) if std::ptr::eq(value, &expr))
        );
        assert!(work.pop().is_none());
        work.extend((0..3).map(|_| Work::Expr(&expr, false, true)), &mut fuel)
            .unwrap();
        assert_eq!(fuel, 0);
        for _ in 0..3 {
            assert!(matches!(work.pop(), Some(Work::Expr(_, false, true))));
        }
        assert!(work.pop().is_none());
    }

    #[test]
    fn mixed_work_keeps_stack_order_and_charges_unknown_width_incrementally() {
        let expr = expression();
        let pattern = Pattern::Wild;
        let owner = SourceRange { start: 1, end: 2 };
        let mut work = Worklist::new(owner, false);
        let mut fuel = 2;
        let batch = [
            Work::Expr(&expr, true, false),
            Work::Pattern(&pattern, true, owner),
            Work::Expr(&expr, false, true),
        ]
        .into_iter()
        .filter(|_| true);
        let error = work.extend(batch, &mut fuel).unwrap_err();
        assert_eq!(error.source, expr.source);
        assert_eq!(fuel, 0);
        assert!(
            matches!(work.pop(), Some(Work::Pattern(value, true, _)) if std::ptr::eq(value, &pattern))
        );
        assert!(matches!(work.pop(), Some(Work::Expr(_, true, false))));
        assert!(work.pop().is_none());
        assert_eq!(
            work.push(Work::Pattern(&pattern, false, owner), &mut 0)
                .unwrap_err()
                .source,
            owner
        );
        assert!(work.pop().is_none());
    }
}
