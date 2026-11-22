//! Inspect structural patterns on whole values and partially observed fields.
use super::*;

enum Projection {
    Unknown,
    Mismatch,
    Fields(Vec<Arc<PartialStagedValue>>),
}

impl Checker<'_, '_> {
    pub(super) fn observe_structural_candidate(
        &mut self,
        pattern: &n::Pattern,
        subject: &Arc<PartialStagedValue>,
        mut candidate: ObservationScope,
        returns: &mut Option<Returns>,
        span: Span,
        depth: usize,
    ) -> Result<Option<PatternPaths>, Error> {
        if let Some((_, children)) = nir::pattern_structure::View::from_pattern(pattern)
            && let projected = self.schema_fuel(span, |this, fuel| {
                Self::project_match_fields(pattern, subject, this.protocols.meta_list, fuel)
            })?
            && !matches!(projected, Projection::Unknown)
        {
            let Projection::Fields(fields) = projected else {
                return Ok(Some(PatternPaths {
                    matched: None,
                    rejected: Some(candidate),
                }));
            };
            let mut paths = PatternPaths {
                matched: Some(candidate),
                rejected: None,
            };
            for (child, field) in children.iter().zip(&fields) {
                let Some(frame) = paths.matched.take() else {
                    break;
                };
                let Some(next) =
                    self.observe_candidate(child, field, frame, returns, span, depth + 1)?
                else {
                    return Ok(None);
                };
                paths.matched = next.matched;
                self.merge_observed_scope(&mut paths.rejected, next.rejected, span)?;
            }
            return Ok(Some(paths));
        }
        let verdict = self.schema_fuel(span, |this, fuel| {
            // Nested pattern programs can write and return. An immutable
            // pattern query cannot stand in for observing those programs.
            if !nir::visit::pattern_observations(pattern, fuel)?.is_empty() {
                return Ok(None);
            }
            if let Some(term) = subject.known_term() {
                let mut budget = comptime_runtime::Budget {
                    steps: *fuel,
                    max_limbs: 4096,
                };
                let mut bindings = BTreeMap::new();
                let decision =
                    this.observe_normal_pattern(pattern, term, 0, &mut budget, &mut bindings);
                *fuel = budget.steps;
                if let Some(matched) =
                    normal::observation::hint(decision).map_err(|e| e.to_string())?
                {
                    return Ok(Some((
                        Some(matched),
                        bindings
                            .into_iter()
                            .map(|(id, term)| (id, PartialStagedValue::known(term)))
                            .collect(),
                    )));
                }
            }
            let (possible, bindings) =
                this.known_staged_pattern_bindings(pattern, subject, 0, fuel)?;
            let decision = if !possible {
                Some(false)
            } else if matches!(pattern, n::Pattern::Wild) {
                Some(true)
            } else {
                None
            };
            Ok(Some((decision, bindings)))
        })?;
        let Some((decision, bindings)) = verdict else {
            return Ok(None);
        };
        if decision == Some(false) {
            return Ok(Some(PatternPaths {
                matched: None,
                rejected: Some(candidate),
            }));
        }
        let rejected = if decision.is_none() {
            Some(self.schema_fuel(span, |_, fuel| candidate.fork(fuel))?)
        } else {
            None
        };
        self.schema_fuel(span, |_, fuel| {
            for (id, value) in bindings {
                candidate.insert(id, value, fuel)?;
            }
            Ok(())
        })?;
        Ok(Some(PatternPaths {
            matched: Some(candidate),
            rejected,
        }))
    }

    fn project_match_fields(
        pattern: &n::Pattern,
        subject: &PartialStagedValue,
        meta_list: Option<DeclarationId>,
        fuel: &mut usize,
    ) -> Result<Projection, String> {
        use PartialStagedShape as Shape;
        use n::Pattern as Pat;
        subject.charge(fuel)?;
        if let Some(term) = subject.known_term() {
            match (pattern, term) {
                (Pat::OptionalPresent(_), Term::OptionalValue { value, .. }) => {
                    return Ok(match value {
                        Some(field) => Projection::Fields(vec![PartialStagedValue::known(
                            field.as_ref().clone(),
                        )]),
                        None => Projection::Mismatch,
                    });
                }
                (
                    Pat::ErrorResult { label, .. },
                    Term::ErrorValue {
                        label: actual,
                        value,
                        ..
                    },
                ) if !term.has_pending_error_branch() && actual.as_deref() == label.as_ref() => {
                    return Ok(Projection::Fields(vec![PartialStagedValue::known(
                        value.as_ref().clone(),
                    )]));
                }
                _ => {}
            }
        }
        let Some((shape, fields)) = subject.aggregate(meta_list) else {
            return Ok(Projection::Unknown);
        };
        let count = match (pattern, shape) {
            (Pat::Tuple(children), Shape::Tuple) => children.len(),
            (
                Pat::Record {
                    declaration,
                    fields: children,
                },
                Shape::Record(owner),
            ) if *declaration == owner => children.len(),
            (
                Pat::AnonymousRecord {
                    names,
                    fields: children,
                },
                Shape::AnonymousRecord(actual),
            ) if *names == actual => children.len(),
            (
                Pat::Variant {
                    declaration,
                    index,
                    fields: children,
                },
                Shape::Variant(owner, active),
            ) if *declaration == owner => {
                if *index != active {
                    return Ok(Projection::Mismatch);
                }
                children.len()
            }
            _ => return Ok(Projection::Unknown),
        };
        Ok(if count == fields.len() {
            Projection::Fields(fields)
        } else {
            Projection::Unknown
        })
    }
}
