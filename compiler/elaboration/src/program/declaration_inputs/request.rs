//! Source child requests shared by slots, dependency checks and collectors.
use super::*;

pub(super) struct Request<'b, 'h> {
    pub(super) input: &'b Expr<'h>,
    pub(super) expected: Option<Term>,
    pub(super) contextual: bool,
}

impl Request<'_, '_> {
    pub(super) fn check(
        self,
        checker: &mut Checker<'_, '_>,
    ) -> Result<n::NormalFormArgument, Error> {
        if self.contextual {
            checker.dependent_normal_argument(self.input, 0)
        } else {
            checker
                .expr(self.input, self.expected.as_ref())
                .map(n::NormalFormArgument::Value)
        }
    }
}

impl std::fmt::Display for Request<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(declaration-input-request (source {} {}) (contextual {}) (expected",
            self.input.hir_id,
            source(self.input.span),
            self.contextual
        )?;
        if let Some(expected) = &self.expected {
            write!(f, " {expected}")?;
        }
        f.write_str("))")
    }
}
