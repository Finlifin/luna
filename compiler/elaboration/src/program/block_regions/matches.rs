//! Contextual match continuations retain the selector, pattern and arm scope.
//! Pattern checking still uses the common synchronous pattern rules; selector
//! and body expressions return through the shared source checking driver.
use super::*;
use driver::{Request, Step};

enum State {
    Ready,
    Selector,
    Arms(n::Expr),
    Body(n::Expr, n::Pattern),
    Complete,
}

pub(super) struct Frame<'b, 'h> {
    source: &'b Expr<'h>,
    depth: usize,
    state: State,
    arms: Vec<n::NormalMatchArm>,
    scope: Option<LexicalScope>,
    premise: Option<refinements::TypeConditionScope>,
}

impl<'b, 'h> Frame<'b, 'h> {
    pub(super) fn new(
        checker: &mut Checker<'_, '_>,
        source: &'b Expr<'h>,
        depth: usize,
    ) -> Result<Self, Error> {
        if !matches!(source.kind, H::Match(..)) {
            return Err(error(source.span, "contextual match requires pattern arms"));
        }
        if depth >= 128 {
            return Err(error(
                source.span,
                "contextual input nesting limit exceeded",
            ));
        }
        checker.required_steps = checker
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(source.span, "match checking budget exhausted"))?;
        Ok(Self {
            source,
            depth,
            state: State::Ready,
            arms: Vec::new(),
            scope: None,
            premise: None,
        })
    }

    pub(super) fn step(
        &mut self,
        checker: &mut Checker<'_, '_>,
        returned: Option<CheckedBlock>,
    ) -> Result<Step<'b, 'h>, Error> {
        let H::Match(selector, arms) = self.source.kind else {
            unreachable!("validated match frame")
        };
        let state = std::mem::replace(&mut self.state, State::Complete);
        match (state, returned) {
            (State::Ready, None) => {
                self.state = State::Selector;
                Ok(Step::Child(Request::Expression(
                    selector,
                    Destination::statement(),
                )))
            }
            (State::Selector, Some(value)) => {
                self.state = State::Arms(value.ordinary(selector.span)?);
                Ok(Step::Continue)
            }
            (State::Arms(value), None) => {
                let Some(arm) = arms.get(self.arms.len()) else {
                    let patterns: Vec<_> = self.arms.iter().map(|arm| &arm.pattern).collect();
                    checker.require_match_coverage(&patterns, &value.ty, self.source.span)?;
                    return Ok(Step::Completed(CheckedBlock::Contextual(
                        n::NormalFormArgument::Match {
                            source: source(self.source.span),
                            value,
                            arms: std::mem::take(&mut self.arms),
                        },
                    )));
                };
                checker.required_steps = checker
                    .required_steps
                    .checked_sub(1)
                    .ok_or_else(|| error(arm.span, "match checking budget exhausted"))?;
                // Pattern checking can add bindings before failing. Own the
                // exit first so the driver can restore exactly this arm.
                self.scope = Some(LexicalScope::enter(checker, arm.span)?);
                let pattern = checker.match_arm_pattern(arm, &value)?;
                self.premise = Some(checker.enter_matched_pattern_type_facts(
                    &pattern,
                    &value,
                    arm.body.span,
                )?);
                self.state = State::Body(value, pattern);
                Ok(Step::Child(Request::Expression(
                    arm.body,
                    Destination::Contextual(ContextualTail { depth: self.depth }),
                )))
            }
            (State::Body(value, pattern), Some(body)) => {
                let body = body.contextual(arms[self.arms.len()].body.span)?;
                self.leave_arm(checker);
                // The common pattern rule embeds the final guard in Pattern.
                // Do not duplicate it in NormalMatchArm or discard it for coverage.
                self.arms.push(n::NormalMatchArm {
                    pattern,
                    guard: None,
                    body,
                });
                self.state = State::Arms(value);
                Ok(Step::Continue)
            }
            _ => Err(error(
                self.source.span,
                "match child result does not match its continuation",
            )),
        }
    }

    fn leave_arm(&mut self, checker: &mut Checker<'_, '_>) {
        if let Some(premise) = self.premise.take() {
            premise.leave(checker);
        }
        if let Some(scope) = self.scope.take() {
            scope.leave(checker);
        }
    }

    pub(super) fn unwind(mut self, checker: &mut Checker<'_, '_>) {
        self.leave_arm(checker);
    }
}

impl std::fmt::Display for Frame<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(match-check (source {}) (depth {}) (next {}) ",
            source(self.source.span),
            self.depth,
            self.arms.len()
        )?;
        match &self.state {
            State::Ready => f.write_str("(phase ready)"),
            State::Selector => f.write_str("(phase selector-child)"),
            State::Arms(value) => write!(f, "(phase arms) (selector {value})"),
            State::Body(value, pattern) => write!(
                f,
                "(phase body-child) (selector {value}) (pattern {pattern})"
            ),
            State::Complete => f.write_str("(phase complete)"),
        }?;
        f.write_str(" (arms")?;
        for arm in &self.arms {
            write!(f, " {arm}")?;
        }
        f.write_str(") (scope")?;
        if let Some(scope) = &self.scope {
            write!(f, " {scope}")?;
        }
        f.write_str(") (premise")?;
        if let Some(premise) = &self.premise {
            write!(f, " {premise}")?;
        }
        f.write_str("))")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dump_separates_selector_pattern_and_completed_arms() {
        let selector = Expr {
            hir_id: hir::HirId::INVALID,
            span: Span::default(),
            kind: H::Unit,
        };
        let expr = Expr {
            kind: H::Match(&selector, &[]),
            ..selector
        };
        let value = n::Expr {
            source: source(expr.span),
            ty: p(P::Str),
            kind: K::Literal(n::Literal::String("selector\"\n".into())),
        };
        let frame = Frame {
            source: &expr,
            depth: 5,
            state: State::Body(value, n::Pattern::Wild),
            arms: Vec::new(),
            scope: None,
            premise: None,
        };
        let dump = frame.to_string();
        assert!(dump.starts_with("(match-check (source "), "{dump}");
        assert!(
            dump.contains("(depth 5) (next 0) (phase body-child)"),
            "{dump}"
        );
        assert!(
            dump.contains("(selector ") && dump.contains("(pattern "),
            "{dump}"
        );
        assert!(dump.contains("selector\\\"\\n"), "{dump}");
        assert!(!dump.contains('\n'), "{dump}");
        assert_eq!(dump, frame.to_string());
    }
}
