//! Conjunctive capability qualification retains query failures until selection.
//! Independent proofs or exclusions may settle a candidate despite a sibling
//! being unresolved; exhaustion is never converted into absence.
use super::*;

// Preserve a malformed prerequisite, then exhaustion, ahead of ordinary
// semantic uncertainty. A later unrelated pending conjunct cannot erase either.
fn retain_failure(pending: &mut Option<QueryError>, error: QueryError) {
    let priority = |error: &QueryError| match error {
        QueryError::Invalid(_) => 2,
        QueryError::Budget => 1,
        QueryError::Unresolved(_) => 0,
    };
    if pending
        .as_ref()
        .is_none_or(|previous| priority(&error) > priority(previous))
    {
        *pending = Some(error);
    }
}

pub(super) fn check_selection(
    types: &TypeResolution,
    qualifications: &BTreeMap<Id, Qualification>,
) -> Result<(), QueryError> {
    if !matches!(types.selection, Selection::Deferred { .. }) {
        return Ok(());
    }
    let mut failure = None;
    for assessment in types.assessments.values() {
        if matches!(
            assessment,
            crate::implementation::TypeAssessment::BudgetExhausted
        ) {
            retain_failure(&mut failure, QueryError::Budget);
        }
    }
    for qualification in qualifications.values() {
        if let Qualification::Pending(error) = qualification {
            retain_failure(&mut failure, error.clone());
        }
    }
    match failure {
        Some(error @ (QueryError::Budget | QueryError::Invalid(_))) => Err(error),
        _ => Ok(()),
    }
}

impl Solver<'_, '_> {
    pub(super) fn qualify(
        &mut self,
        evidence: &TypeEvidence,
        budget: Budget,
        depth: usize,
    ) -> Qualification {
        let Some(implementation) = self
            .world
            .implementations
            .iter()
            .find(|i| i.declaration == evidence.implementation)
        else {
            return Qualification::Pending("missing implementation contract".into());
        };
        let mut witnesses = Vec::new();
        let mut pending = None;
        let substitution = match nir::DeclarationSubstitution::bounded(
            &implementation.type_parameters,
            &implementation.parameter_types,
            &evidence.type_arguments,
            self.world.context.scope(),
            &mut self.remaining,
        ) {
            Ok(substitution) => substitution,
            Err(error) => return Qualification::Pending(error.into()),
        };
        for capability in &implementation.capabilities {
            if self.remaining == 0 {
                retain_failure(&mut pending, QueryError::Budget);
                break;
            }
            self.remaining -= 1;
            let requirement = match substitution
                .instantiate_requirement_bounded(capability, &mut self.remaining)
            {
                Ok(requirement) => requirement,
                Err(error) => {
                    retain_failure(&mut pending, error.into());
                    continue;
                }
            };
            let mut incomplete_given = None;
            match self.given(&requirement, budget) {
                Ok(Some(given)) => {
                    witnesses.push(Witness {
                        requirement,
                        origin: Origin::Given(given),
                    });
                    continue;
                }
                Ok(None) => {}
                Err(error @ nir::NominalAdmissionError::Invalid(_)) => {
                    retain_failure(&mut pending, error);
                    continue;
                }
                Err(error) => retain_failure(&mut incomplete_given, error),
            }
            let requirement = match (|| {
                Ok::<_, QueryError>(TraitRequirement {
                    interface: requirement.interface,
                    subject: self.prerequisite_type(&requirement.subject, budget, depth)?,
                    interface_arguments: requirement
                        .interface_arguments
                        .iter()
                        .map(|ty| self.prerequisite_type(ty, budget, depth))
                        .collect::<Result<_, _>>()?,
                })
            })() {
                Ok(requirement) => requirement,
                Err(error) => {
                    retain_failure(&mut pending, error);
                    if let Some(error) = incomplete_given {
                        retain_failure(&mut pending, error);
                    }
                    continue;
                }
            };
            match self.given(&requirement, budget) {
                Ok(Some(given)) => {
                    witnesses.push(Witness {
                        requirement,
                        origin: Origin::Given(given),
                    });
                    continue;
                }
                Ok(None) => {}
                Err(error @ nir::NominalAdmissionError::Invalid(_)) => {
                    retain_failure(&mut pending, error);
                    continue;
                }
                Err(error) => retain_failure(&mut incomplete_given, error),
            }
            let resolution = match self.resolve(
                &requirement.interface_instance(),
                &requirement.subject,
                budget,
                depth,
            ) {
                Ok(r) => r,
                Err(e) => {
                    retain_failure(&mut pending, e);
                    if let Some(error) = incomplete_given {
                        retain_failure(&mut pending, error);
                    }
                    continue;
                }
            };
            if !matches!(resolution.types.selection, Selection::Selected(_)) {
                if let Some(reason) = incomplete_given {
                    retain_failure(&mut pending, reason);
                    continue;
                }
            }
            match resolution.types.selection {
                Selection::Selected(id) => {
                    let Some(crate::implementation::TypeAssessment::Applicable(evidence)) =
                        resolution.types.assessments.get(&id)
                    else {
                        return Qualification::Pending("missing nested type evidence".into());
                    };
                    let Some(Qualification::Proved(premises)) = resolution.qualifications.get(&id)
                    else {
                        return Qualification::Pending("missing nested capability evidence".into());
                    };
                    witnesses.push(Witness {
                        requirement,
                        origin: Origin::Implementation {
                            declaration: id,
                            arguments: evidence.type_arguments.clone(),
                            premises: premises.clone(),
                        },
                    });
                }
                Selection::NoApplicableCandidate => {
                    let mut closure_budget = Budget {
                        steps: self.remaining,
                        ..budget
                    };
                    let closed = self.world.closed_inputs_metered(
                        std::iter::once(&requirement.subject)
                            .chain(&requirement.interface_arguments),
                        &mut closure_budget,
                    );
                    self.remaining = closure_budget.steps;
                    match closed {
                        Ok(true) => {}
                        Ok(false) => {
                            retain_failure(
                                &mut pending,
                                QueryError::Unresolved(
                                    "absence for an abstract capability subject is not established"
                                        .into(),
                                ),
                            );
                            continue;
                        }
                        Err(error) => {
                            retain_failure(&mut pending, error);
                            continue;
                        }
                    }
                    witnesses.push(Witness {
                        requirement,
                        origin: Origin::Absent,
                    });
                    return Qualification::Excluded(witnesses);
                }
                other => {
                    for qualification in resolution.qualifications.values() {
                        if let Qualification::Pending(reason) = qualification {
                            retain_failure(&mut pending, reason.clone());
                        }
                    }
                    retain_failure(
                        &mut pending,
                        QueryError::Unresolved(format!("nested capability is unsettled: {other}")),
                    );
                }
            }
        }
        match pending {
            Some(reason) => Qualification::Pending(reason),
            None => Qualification::Proved(witnesses),
        }
    }
}
