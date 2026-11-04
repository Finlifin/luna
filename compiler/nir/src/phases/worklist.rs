//! Phase work retains lexical stage and direct-callee context. Charge pending
//! nodes before allocating their slots; popping an admitted node is free.
use super::{Error, Expr, Pattern, SourceRange};

pub(super) enum Work<'a> {
    Expr(&'a Expr, bool, bool), // direct callee, comptime execution context
    Pattern(&'a Pattern, bool),
}

pub(super) struct Worklist<'a> {
    pending: Vec<Work<'a>>,
    owner: SourceRange,
}

impl<'a> Worklist<'a> {
    pub(super) fn new(owner: SourceRange) -> Self {
        Self {
            pending: Vec::new(),
            owner,
        }
    }

    fn charge(fuel: &mut usize, count: usize, source: SourceRange) -> Result<(), Error> {
        *fuel = fuel.checked_sub(count).ok_or(Error {
            source,
            message: "phase traversal budget exhausted",
        })?;
        Ok(())
    }

    pub(super) fn push(&mut self, node: Work<'a>, fuel: &mut usize) -> Result<(), Error> {
        let source = match &node {
            Work::Expr(expr, ..) => expr.source,
            Work::Pattern(..) => self.owner,
        };
        Self::charge(fuel, 1, source)?;
        self.pending.push(node);
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
            self.pending.extend(nodes);
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
        self.pending.pop()
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
    fn exact_width_is_reserved_before_iteration_and_preserves_existing_work() {
        let expr = expression();
        let owner = SourceRange { start: 1, end: 2 };
        let mut work = Worklist::new(owner);
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
        let mut work = Worklist::new(owner);
        let mut fuel = 2;
        let batch = [
            Work::Expr(&expr, true, false),
            Work::Pattern(&pattern, true),
            Work::Expr(&expr, false, true),
        ]
        .into_iter()
        .filter(|_| true);
        let error = work.extend(batch, &mut fuel).unwrap_err();
        assert_eq!(error.source, expr.source);
        assert_eq!(fuel, 0);
        assert!(
            matches!(work.pop(), Some(Work::Pattern(value, true)) if std::ptr::eq(value, &pattern))
        );
        assert!(matches!(work.pop(), Some(Work::Expr(_, true, false))));
        assert!(work.pop().is_none());
        assert_eq!(
            work.push(Work::Pattern(&pattern, false), &mut 0)
                .unwrap_err()
                .source,
            owner
        );
        assert!(work.pop().is_none());
    }
}
