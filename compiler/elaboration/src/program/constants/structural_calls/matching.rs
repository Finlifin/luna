//! Ordered match observation. A rejected guard leaves its outer writes visible
//! to later arms, while pattern bindings belong only to the selected candidate.
use super::flow::Returns;
use super::*;

mod structure;

impl Checker<'_, '_> {
    pub(super) fn observe_factory_match(
        &mut self,
        subject: &Arc<PartialStagedValue>,
        arms: &[n::MatchArm],
        scope: &mut ObservationScope,
        returns: &mut Option<Returns>,
        span: Span,
        depth: usize,
    ) -> Result<Completion, Error> {
        // Keep a structural miss separate from a successful test followed by
        // a rejected guard. The same test on the saved subject has the same
        // verdict, even if the guard changed the original scrutinee's place.
        let mut missed = Some(self.schema_fuel(span, |_, fuel| scope.fork(fuel))?);
        let mut failed_guard = None;
        let mut previous = None;
        let mut result = Completion::Returned;
        for arm in arms {
            if missed.is_none() && failed_guard.is_none() {
                break;
            }
            let mut guards = Vec::new();
            let mut pattern = &arm.pattern;
            while let n::Pattern::Guard {
                pattern: inner,
                condition,
            } = pattern
            {
                self.required_steps = self
                    .required_steps
                    .checked_sub(1)
                    .ok_or_else(|| error(span, "match guard observation budget exhausted"))?;
                guards.push(condition.as_ref());
                pattern = inner;
            }
            let repeated = if let Some(prior) = previous {
                self.same_structural_test(prior, pattern, span)?
            } else {
                false
            };
            previous = Some(pattern);
            let input = if repeated {
                // Only failed guards can retry a body with this same pattern.
                failed_guard.take()
            } else {
                self.merge_observed_scope(&mut missed, failed_guard.take(), span)?;
                missed.take()
            };
            let Some(input) = input else { continue };
            let Some(mut paths) =
                self.observe_candidate(pattern, subject, input, returns, span, depth)?
            else {
                return Ok(Completion::Unavailable);
            };
            if !repeated {
                missed = paths.rejected.take();
            }
            paths.rejected = None;
            for guard in guards.into_iter().rev().chain(arm.guard.as_ref()) {
                if !self.observe_match_guard(&mut paths, guard, returns, span, depth)? {
                    return Ok(Completion::Unavailable);
                }
            }
            failed_guard = paths.rejected;
            for rejected in missed.iter_mut().chain(failed_guard.iter_mut()) {
                self.schema_fuel(span, |_, fuel| {
                    rejected.retain(|id| scope.values.contains_key(id), fuel)
                })?;
            }
            if let Some(mut frame) = paths.matched {
                let body =
                    self.observe_factory_expr(&arm.body, &mut frame, returns, span, depth)?;
                self.schema_fuel(span, |_, fuel| {
                    frame.retain(|id| scope.values.contains_key(id), fuel)
                })?;
                result = self.merge_factory_paths(scope, result, frame, body, span)?;
                if matches!(result, Completion::Unavailable) {
                    return Ok(result);
                }
            }
        }
        Ok(result)
    }

    fn same_structural_test(
        &mut self,
        first: &n::Pattern,
        next: &n::Pattern,
        span: Span,
    ) -> Result<bool, Error> {
        if std::mem::discriminant(first) != std::mem::discriminant(next) {
            return Ok(false);
        }
        if !self.schema_fuel(span, |_, fuel| {
            Ok(nir::visit::pattern_observations(first, fuel)?.is_empty())
        })? {
            return Ok(false);
        }
        for pattern in [first, next] {
            nir::copy_budget::charge_pattern_rewrite(pattern, &mut self.required_steps)
                .map_err(|_| error(span, "match pattern comparison budget exhausted"))?;
        }
        Ok(first == next)
    }

    fn observe_match_guard(
        &mut self,
        paths: &mut PatternPaths,
        guard: &n::Expr,
        returns: &mut Option<Returns>,
        span: Span,
        depth: usize,
    ) -> Result<bool, Error> {
        let Some(matched) = paths.matched.take() else {
            return Ok(true);
        };
        let Some(outcome) = self.observe_condition(guard, matched, returns, span, depth)? else {
            return Ok(false);
        };
        paths.matched = outcome.when_true;
        self.merge_observed_scope(&mut paths.rejected, outcome.when_false, span)?;
        Ok(true)
    }

    fn observe_candidate(
        &mut self,
        pattern: &n::Pattern,
        subject: &Arc<PartialStagedValue>,
        mut candidate: ObservationScope,
        returns: &mut Option<Returns>,
        span: Span,
        depth: usize,
    ) -> Result<Option<PatternPaths>, Error> {
        if depth >= 64 {
            return Ok(None);
        }
        self.schema_fuel(span, |_, fuel| {
            *fuel = fuel
                .checked_sub(1)
                .ok_or("match observation budget exhausted")?;
            Ok(())
        })?;
        use n::Pattern as Pat;
        match pattern {
            Pat::Guard { pattern, condition } => {
                let Some(mut paths) =
                    self.observe_candidate(pattern, subject, candidate, returns, span, depth + 1)?
                else {
                    return Ok(None);
                };
                return Ok(self
                    .observe_match_guard(&mut paths, condition, returns, span, depth + 1)?
                    .then_some(paths));
            }
            Pat::Then { first, value, next } => {
                let Some(mut paths) =
                    self.observe_candidate(first, subject, candidate, returns, span, depth + 1)?
                else {
                    return Ok(None);
                };
                if let Some(mut matched) = paths.matched.take() {
                    match self.observe_factory_expr(
                        value,
                        &mut matched,
                        returns,
                        span,
                        depth + 1,
                    )? {
                        Completion::Unavailable => return Ok(None),
                        Completion::Returned => {}
                        Completion::Next(produced) => {
                            let Some(next) = self.observe_candidate(
                                next,
                                &produced,
                                matched,
                                returns,
                                span,
                                depth + 1,
                            )?
                            else {
                                return Ok(None);
                            };
                            paths.matched = next.matched;
                            self.merge_observed_scope(&mut paths.rejected, next.rejected, span)?;
                        }
                    }
                }
                return Ok(Some(paths));
            }
            Pat::Observe { binding, inner } | Pat::Binding { binding, inner } => {
                let alias = matches!(pattern, Pat::Observe { .. });
                if alias {
                    self.schema_fuel(span, |_, fuel| {
                        candidate.insert(*binding, subject.clone(), fuel)
                    })?;
                }
                let Some(mut paths) =
                    self.observe_candidate(inner, subject, candidate, returns, span, depth + 1)?
                else {
                    return Ok(None);
                };
                if alias {
                    for frame in paths.matched.iter_mut().chain(paths.rejected.iter_mut()) {
                        frame.values.remove(binding);
                        frame.known.remove(binding);
                    }
                } else if let Some(frame) = &mut paths.matched {
                    self.schema_fuel(span, |_, fuel| {
                        frame.insert(*binding, subject.clone(), fuel)
                    })?;
                }
                return Ok(Some(paths));
            }
            Pat::Or(alternatives) => {
                let mut paths = PatternPaths {
                    matched: None,
                    rejected: Some(self.schema_fuel(span, |_, fuel| candidate.fork(fuel))?),
                };
                for alternative in alternatives {
                    let Some(frame) = paths.rejected.take() else {
                        break;
                    };
                    let Some(next) = self.observe_candidate(
                        alternative,
                        subject,
                        frame,
                        returns,
                        span,
                        depth + 1,
                    )?
                    else {
                        return Ok(None);
                    };
                    self.merge_observed_scope(&mut paths.matched, next.matched, span)?;
                    if let Some(mut rejected) = next.rejected {
                        self.schema_fuel(span, |_, fuel| {
                            rejected.retain(|id| candidate.values.contains_key(id), fuel)
                        })?;
                        paths.rejected = Some(rejected);
                    }
                }
                return Ok(Some(paths));
            }
            Pat::Remap { pattern, bindings } => {
                let Some(mut paths) =
                    self.observe_candidate(pattern, subject, candidate, returns, span, depth + 1)?
                else {
                    return Ok(None);
                };
                if let Some(frame) = &mut paths.matched {
                    self.schema_fuel(span, |_, fuel| {
                        *fuel = fuel
                            .checked_sub(bindings.len())
                            .ok_or("pattern remap budget exhausted")?;
                        let values: Vec<_> = bindings
                            .iter()
                            .map(|(from, to)| {
                                (
                                    *to,
                                    frame
                                        .values
                                        .get(from)
                                        .cloned()
                                        .unwrap_or_else(PartialStagedValue::unknown),
                                )
                            })
                            .collect();
                        for (from, _) in bindings {
                            frame.values.remove(from);
                            frame.known.remove(from);
                        }
                        for (to, value) in values {
                            frame.insert(to, value, fuel)?;
                        }
                        Ok(())
                    })?;
                }
                return Ok(Some(paths));
            }
            _ => {}
        }
        self.observe_structural_candidate(pattern, subject, candidate, returns, span, depth)
    }
}

struct PatternPaths {
    matched: Option<ObservationScope>,
    rejected: Option<ObservationScope>,
}
