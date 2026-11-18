//! Shared arm pattern/guard checking and match coverage requirements.
use super::*;

impl Checker<'_, '_> {
    pub(in crate::program) fn require_match_coverage(
        &mut self,
        patterns: &[&n::Pattern],
        ty: &Term,
        span: Span,
    ) -> Result<(), Error> {
        match self.patterns_exhaustive(patterns, ty) {
            crate::coverage::Coverage::Exhaustive => Ok(()),
            crate::coverage::Coverage::Unknown => Err(error(
                span,
                "match exhaustiveness could not be proved within supported analysis and budget; add an unguarded fallback pattern",
            )),
            _ => Err(error(
                span,
                "non-exhaustive match; add an unguarded fallback pattern",
            )),
        }
    }

    pub(in crate::program) fn match_arm_pattern(
        &mut self,
        arm: &hir::pattern::PatternArm<'_>,
        value: &n::Expr,
    ) -> Result<n::Pattern, Error> {
        let observed = self.known_pattern_type_value(value, arm.pat.span)?;
        let domain = arguments::inference_type(value).unwrap_or(&value.ty);
        let pattern = self.pattern_observed(&arm.pat, domain, observed.as_ref())?;
        let guard = arm
            .guard
            .map(|guard| {
                self.under_matched_pattern_type_facts(&pattern, value, guard.span, |checker| {
                    checker.expr(guard, Some(&p(P::Bool)))
                })
            })
            .transpose()?;
        // Final guard runs after candidate selection, including or-patterns.
        Ok(match guard {
            Some(condition) => n::Pattern::Guard {
                pattern: Box::new(pattern),
                condition: Box::new(condition),
            },
            None => pattern,
        })
    }
}
