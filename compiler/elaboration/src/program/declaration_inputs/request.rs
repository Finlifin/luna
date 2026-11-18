//! Source child requests shared by routing, slots, dependencies and collectors.
use super::*;

pub(in crate::program) enum Demand {
    Ordinary(Option<Term>),
    Input(crate::calls::InputContext),
    Contextual,
}

pub(in crate::program) struct Request<'b, 'h> {
    input: &'b Expr<'h>,
    demand: Demand,
}

impl<'b, 'h> Request<'b, 'h> {
    pub(super) fn ordinary(input: &'b Expr<'h>, expected: Option<Term>) -> Self {
        Self {
            input,
            demand: Demand::Ordinary(expected),
        }
    }

    pub(super) fn input(input: &'b Expr<'h>, context: crate::calls::InputContext) -> Self {
        Self {
            input,
            demand: Demand::Input(context),
        }
    }

    pub(super) fn contextual(input: &'b Expr<'h>) -> Self {
        Self {
            input,
            demand: Demand::Contextual,
        }
    }

    pub(in crate::program) fn into_parts(self) -> (&'b Expr<'h>, Demand) {
        (self.input, self.demand)
    }
}

impl std::fmt::Display for Request<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(declaration-input-request (source {} {}) ",
            self.input.hir_id,
            source(self.input.span)
        )?;
        match &self.demand {
            Demand::Ordinary(expected) => {
                f.write_str("(ordinary (expected")?;
                if let Some(expected) = expected {
                    write!(f, " {expected}")?;
                }
                f.write_str("))")?;
            }
            Demand::Input(context) => write!(f, "(input {context})")?,
            Demand::Contextual => f.write_str("(contextual)")?,
        }
        f.write_str(")")
    }
}
