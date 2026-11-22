//! Match continuations retain the selector, pattern, delivery and arm scope.
//! Pattern checking still uses the common synchronous pattern rules; selector
//! and body expressions return through the shared source checking driver.
use super::*;
use driver::{Request, Step};
mod ordinary;
use ordinary::Ordinary;

enum State {
    Ready,
    Selector,
    Inferring(n::Expr, branch_probes::Search),
    Arms(n::Expr),
    Body(n::Expr, n::Pattern),
    Complete,
}

pub(super) struct Frame<'h> {
    source: Expr<'h>,
    destination: Destination,
    ordinary: Option<Ordinary>,
    probe: Option<branch_probes::Probe>,
    input_delivery: Option<Destination>,
    operand_scope: scoped_operands::Boundary,
    state: State,
    arms: Vec<(n::Pattern, CheckedBlock)>,
    scope: Option<LexicalScope>,
    premise: Option<refinements::TypeConditionScope>,
}

impl<'h> Frame<'h> {
    pub(super) fn new(
        checker: &mut Checker<'_, '_>,
        source: &Expr<'h>,
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
            source: source.clone(),
            destination: Destination::Contextual(ContextualTail { depth }),
            ordinary: None,
            probe: None,
            input_delivery: None,
            operand_scope: Default::default(),
            state: State::Ready,
            arms: Vec::new(),
            scope: None,
            premise: None,
        })
    }

    pub(super) fn ordinary(
        checker: &mut Checker<'_, '_>,
        source: &Expr<'h>,
        destination: Destination,
        own_effect_delivery: bool,
    ) -> Result<Self, Error> {
        if !matches!(source.kind, H::Match(..)) || destination.ordinary_view().is_none() {
            return Err(error(
                source.span,
                "ordinary match requires pattern arms and an ordinary destination",
            ));
        }
        checker.required_steps = checker
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(source.span, "match checking budget exhausted"))?;
        let input_delivery = (own_effect_delivery && destination.structural_input().is_some())
            .then(|| destination.clone());
        let ordinary = Ordinary::new(
            checker,
            source,
            own_effect_delivery && input_delivery.is_none(),
        )?;
        Ok(Self {
            source: source.clone(),
            destination,
            ordinary: Some(ordinary),
            probe: None,
            input_delivery,
            operand_scope: Default::default(),
            state: State::Ready,
            arms: Vec::new(),
            scope: None,
            premise: None,
        })
    }

    fn accept_selector(
        &mut self,
        checker: &mut Checker<'_, '_>,
        value: n::Expr,
    ) -> Result<(), Error> {
        let H::Match(_, arms) = self.source.kind else {
            unreachable!()
        };
        if let Some(context) = self.destination.structural_input() {
            self.state = State::Inferring(
                value,
                branch_probes::Search::new(checker, context, arms.len(), self.source.span),
            );
            return Ok(());
        }
        self.state = State::Arms(value);
        Ok(())
    }

    pub(super) fn step(
        &mut self,
        checker: &mut Checker<'_, '_>,
        returned: Option<CheckedBlock>,
    ) -> Result<Step<'h>, Error> {
        let H::Match(selector, arms) = self.source.kind else {
            unreachable!("validated match frame")
        };
        if matches!(self.state, State::Inferring(..)) {
            return self.advance_search(checker, returned);
        }
        let state = std::mem::replace(&mut self.state, State::Complete);
        match (state, returned) {
            (State::Ready, None) => {
                if let Some(value) = self
                    .ordinary
                    .as_mut()
                    .and_then(|ordinary| ordinary.selector.take())
                {
                    self.accept_selector(checker, value)?;
                    return Ok(Step::Continue);
                }
                if self.ordinary.is_some()
                    && matches!(selector.kind, H::Application(..))
                    && checker.uses_lexical_operands()
                {
                    self.operand_scope.enter(checker, selector.hir_id);
                }
                self.state = State::Selector;
                Ok(Step::Child(Request::expression(
                    selector,
                    Destination::statement(),
                )))
            }
            (State::Selector, Some(value)) => {
                self.operand_scope.leave(checker);
                if let Some(ordinary) = &mut self.ordinary {
                    ordinary.selector_exit = value.selected_exit();
                }
                self.accept_selector(checker, value.ordinary(selector.span)?)?;
                Ok(Step::Continue)
            }
            (State::Arms(value), None) => {
                let Some(arm) = arms.get(self.arms.len()) else {
                    return self.finish(checker, value);
                };
                checker.required_steps = checker
                    .required_steps
                    .checked_sub(1)
                    .ok_or_else(|| error(arm.span, "match checking budget exhausted"))?;
                // Own the exit before pattern checking can introduce bindings.
                self.scope = Some(LexicalScope::enter(checker, arm.span)?);
                if let Some(ordinary) = &mut self.ordinary {
                    ordinary.enter_arm(checker, arm.span)?;
                }
                let pattern = if self
                    .ordinary
                    .as_ref()
                    .is_some_and(|ordinary| ordinary.behavior_pattern)
                {
                    checker
                        .with_staged_input(true, |checker| checker.match_arm_pattern(arm, &value))?
                } else {
                    checker.match_arm_pattern(arm, &value)?
                };
                let destination = if let Some(ordinary) = &mut self.ordinary {
                    ordinary.body_destination(
                        checker,
                        &self.source,
                        &self.destination,
                        arm,
                        self.arms.len(),
                        &pattern,
                        self.scope.as_ref().unwrap(),
                    )?
                } else {
                    self.destination.clone()
                };
                self.premise = Some(checker.enter_matched_pattern_type_facts(
                    &pattern,
                    &value,
                    arm.body.span,
                )?);
                self.state = State::Body(value, pattern);
                Ok(Step::Child(Request::expression(arm.body, destination)))
            }
            (State::Body(value, pattern), Some(body)) => {
                // Join outside matched premises, with bindings still available
                // for the ordinary success-payload stabilization rule.
                self.leave_body(checker);
                let arm = &arms[self.arms.len()];
                let checked = if let Some(ordinary) = &mut self.ordinary {
                    ordinary.accept_body(checker, pattern, body, &value.ty, arm.span)?
                } else {
                    (
                        pattern,
                        CheckedBlock::Contextual(body.contextual(arm.body.span)?),
                    )
                };
                self.leave_arm(checker);
                // The common pattern rule embeds the final guard in Pattern.
                // Do not duplicate it in an arm or discard it for coverage.
                self.arms.push(checked);
                self.state = State::Arms(value);
                Ok(Step::Continue)
            }
            _ => Err(error(
                self.source.span,
                "match child result does not match its continuation",
            )),
        }
    }

    fn advance_search(
        &mut self,
        checker: &mut Checker<'_, '_>,
        returned: Option<CheckedBlock>,
    ) -> Result<Step<'h>, Error> {
        if let Some(probe) = self.probe.take() {
            // Type facts leave before the checkpoint restores the candidate.
            if let Some(premise) = self.premise.take() {
                premise.leave(checker);
            }
            let result = returned
                .ok_or_else(|| error(self.source.span, "match probe child is missing"))
                .and_then(|value| value.ordinary(self.source.span));
            let result = probe.finish(checker, result)?;
            let State::Inferring(_, search) = &mut self.state else {
                unreachable!()
            };
            search.observe(result);
        } else if returned.is_some() {
            return Err(error(
                self.source.span,
                "match search received an unsolicited child",
            ));
        }
        let State::Inferring(value, search) = &mut self.state else {
            unreachable!()
        };
        if let Some(request) = search.next(checker)? {
            self.probe = Some(branch_probes::Probe::begin(
                checker,
                self.source.span,
                request,
            )?);
            let H::Match(_, arms) = self.source.kind else {
                unreachable!()
            };
            let arm = &arms[request.index];
            let pattern = checker.match_arm_pattern(arm, value)?;
            self.premise =
                Some(checker.enter_matched_pattern_type_facts(&pattern, value, arm.body.span)?);
            Ok(Step::Child(Request::expression(
                arm.body,
                self.destination.clone(),
            )))
        } else {
            let State::Inferring(value, search) =
                std::mem::replace(&mut self.state, State::Complete)
            else {
                unreachable!()
            };
            if let Some(inferred) = search.result() {
                self.destination =
                    self.destination
                        .with_expected(checker, Some(&inferred), self.source.span)?;
            }
            self.state = State::Arms(value);
            Ok(Step::Continue)
        }
    }

    pub(super) fn recover_probe(
        &mut self,
        checker: &mut Checker<'_, '_>,
        failure: Error,
    ) -> Result<(), Error> {
        let Some(probe) = self.probe.take() else {
            return Err(failure);
        };
        if let Some(premise) = self.premise.take() {
            premise.leave(checker);
        }
        probe.finish(checker, Err(failure))?;
        Ok(())
    }

    fn finish(&mut self, checker: &mut Checker<'_, '_>, value: n::Expr) -> Result<Step<'h>, Error> {
        let mut selected_exit = false;
        let result = if let Some(ordinary) = &mut self.ordinary {
            selected_exit = ordinary.join_exits(checker, &value, &self.arms, self.source.span)?;
            let arms = std::mem::take(&mut self.arms)
                .into_iter()
                .map(|(pattern, body)| {
                    Ok(n::MatchArm {
                        pattern,
                        guard: None,
                        body: body.ordinary(self.source.span)?,
                    })
                })
                .collect::<Result<Vec<_>, Error>>()?;
            let value = ordinary.finish(checker, &self.source, &self.destination, value, arms)?;
            CheckedBlock::Ordinary(self.operand_scope.close(
                checker,
                value,
                scoped_operands::Consumer::Match,
                self.source.span,
            )?)
        } else {
            let patterns: Vec<_> = self.arms.iter().map(|(pattern, _)| pattern).collect();
            checker.require_match_coverage(&patterns, &value.ty, self.source.span)?;
            let arms = std::mem::take(&mut self.arms)
                .into_iter()
                .map(|(pattern, body)| {
                    Ok(n::NormalMatchArm {
                        pattern,
                        guard: None,
                        body: body.contextual(self.source.span)?,
                    })
                })
                .collect::<Result<Vec<_>, Error>>()?;
            CheckedBlock::Contextual(n::NormalFormArgument::Match {
                source: source(self.source.span),
                value,
                arms,
            })
        };
        let result = if let Some(destination) = &self.input_delivery {
            CheckedBlock::Ordinary(destination.deliver_input(
                checker,
                result.ordinary(self.source.span)?,
                self.source.span,
            )?)
        } else {
            result
        };
        Ok(Step::Completed(result.with_selected_exit(selected_exit)))
    }

    fn leave_body(&mut self, checker: &mut Checker<'_, '_>) {
        if let Some(ordinary) = &mut self.ordinary {
            ordinary.leave_body(checker);
        }
        if let Some(premise) = self.premise.take() {
            premise.leave(checker);
        }
    }

    fn leave_arm(&mut self, checker: &mut Checker<'_, '_>) {
        self.leave_body(checker);
        if let Some(scope) = self.scope.take() {
            scope.leave(checker);
        }
        if let Some(ordinary) = &mut self.ordinary {
            ordinary.leave_arm(checker);
        }
    }

    pub(super) fn unwind(mut self, checker: &mut Checker<'_, '_>) {
        debug_assert!(
            self.probe.is_none(),
            "finish the attempt before leaving its frame"
        );
        self.operand_scope.leave(checker);
        self.leave_arm(checker);
        if let Some(ordinary) = self.ordinary {
            ordinary.unwind(checker);
        }
    }
}

impl std::fmt::Display for Frame<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(match-check (source {}) {} (next {}) ",
            source(self.source.span),
            self.destination,
            self.arms.len()
        )?;
        match &self.state {
            State::Ready => f.write_str("(phase ready)"),
            State::Selector => f.write_str("(phase selector-child)"),
            State::Inferring(value, search) => {
                write!(f, "(phase inferring) (selector {value}) {search}")
            }
            State::Arms(value) => write!(f, "(phase arms) (selector {value})"),
            State::Body(value, pattern) => write!(
                f,
                "(phase body-child) (selector {value}) (pattern {pattern})"
            ),
            State::Complete => f.write_str("(phase complete)"),
        }?;
        f.write_str(" (probe")?;
        if let Some(probe) = &self.probe {
            write!(f, " {probe}")?;
        }
        f.write_str(") (input-delivery")?;
        if let Some(destination) = &self.input_delivery {
            write!(f, " {destination}")?;
        }
        write!(f, ") {} (arms", self.operand_scope)?;
        for (pattern, body) in &self.arms {
            write!(f, " (arm {pattern} {body})")?;
        }
        f.write_str(") (ordinary")?;
        if let Some(ordinary) = &self.ordinary {
            write!(f, " {ordinary}")?;
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
            source: expr,
            destination: Destination::Contextual(ContextualTail { depth: 5 }),
            ordinary: None,
            probe: None,
            input_delivery: None,
            operand_scope: Default::default(),
            state: State::Body(value, n::Pattern::Wild),
            arms: Vec::new(),
            scope: None,
            premise: None,
        };
        let dump = frame.to_string();
        assert!(dump.starts_with("(match-check (source "), "{dump}");
        assert!(
            dump.contains("(depth 5)) (next 0) (phase body-child)"),
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
