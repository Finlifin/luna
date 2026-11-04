//! Subject eligibility requires a structural view or a proved disjunction.
//! Neither failed search nor exhausted traversal is evidence of ineligibility.
use super::*;
use nir::NominalAdmissionError as QueryError;

fn query_failure(reason: crate::ptc::UnknownReason) -> QueryError {
    match reason {
        crate::ptc::UnknownReason::BudgetExhausted => QueryError::Budget,
        reason => QueryError::Unresolved(format!("subject eligibility: {reason:?}")),
    }
}
fn assessment(error: QueryError) -> TypeAssessment {
    match error {
        QueryError::Budget => TypeAssessment::BudgetExhausted,
        QueryError::Unresolved(reason) => TypeAssessment::Pending(reason),
        QueryError::Invalid(reason) => TypeAssessment::Invalid(reason),
    }
}

pub(super) fn subject_view(
    context: &Context,
    subject: &Term,
    budget: Budget,
    head_budget: &mut Budget,
) -> Result<(Term, Option<Proof>), TypeAssessment> {
    if context.is_tuple_type(subject)
        || context.is_list_type(subject)
        || !matches!(
            subject,
            Term::Rigid { .. } | Term::NormalForm { .. } | Term::AssociatedType { .. }
        )
    {
        return Ok((subject.clone(), None));
    }
    let view_budget = Budget {
        steps: budget.steps.min(head_budget.steps),
        ..budget
    };
    let (view, spent) = context.structural_view_with_heads_metered(subject, view_budget, |head| {
        context.is_tuple_head(head) || context.is_list_inference_head(head)
    });
    head_budget.steps -= spent;
    let failed_view = match view {
        Ok(Some((shape, proof))) => return Ok((shape, Some(proof))),
        Ok(None) => None,
        Err(reason) => Some(query_failure(reason)),
    };
    // An independent disjunction proof can establish eligibility after a view
    // query failed. If both are incomplete, preserve resource failure rather
    // than turning it into ordinary semantic uncertainty.
    match prove_eligible_alternatives(context, subject, head_budget) {
        Ok(Some(proof)) => Ok((subject.clone(), Some(proof))),
        Err(error @ QueryError::Invalid(_)) => Err(assessment(error)),
        Err(QueryError::Budget) => Err(TypeAssessment::BudgetExhausted),
        outcome => {
            let error = match failed_view {
                Some(error) => error,
                None => match outcome {
                    Err(error) => error,
                    _ => QueryError::Unresolved(
                        "user implementation subject eligibility is unresolved".into(),
                    ),
                },
            };
            Err(assessment(error))
        }
    }
}

// Eligibility need not reveal one concrete shape. A finite disjunction of
// admissible shapes can authorize matching while the subject stays symbolic.
pub(super) fn eligible_shape(context: &Context, ty: &Term) -> bool {
    matches!(
        ty,
        Term::Primitive(_)
            | Term::ErrorResult { .. }
            | Term::Nominal { .. }
            | Term::Tuple(_)
            | Term::Record(_)
            | Term::Optional(_)
            | Term::Pointer(_)
            | Term::Function { .. }
            | Term::ComptimeFunction(_)
            | Term::NormalFormFunction(..)
            | Term::UnsafeFunction(_)
    ) || context.is_tuple_type(ty)
        || context.is_list_type(ty)
}

fn prove_eligible_alternatives(
    context: &Context,
    subject: &Term,
    budget: &mut Budget,
) -> Result<Option<Proof>, nir::NominalAdmissionError> {
    budget.steps = budget
        .steps
        .checked_sub(context.assumptions().len())
        .ok_or(nir::NominalAdmissionError::Budget)?;
    let mut pending: Vec<_> = context.assumptions().iter().collect();
    let mut seen = std::collections::HashSet::new();
    let mut alternatives = Vec::new();
    while let Some(formula) = pending.pop() {
        budget.steps = budget
            .steps
            .checked_sub(1)
            .ok_or(nir::NominalAdmissionError::Budget)?;
        match formula {
            Formula::Equal(crate::ptc::Equality(a, b)) => {
                for ty in [a, b] {
                    nir::type_computations::charge_term(ty, &mut budget.steps)?;
                    if eligible_shape(context, ty) && seen.insert(ty) {
                        nir::type_computations::charge_term(subject, &mut budget.steps)?;
                        alternatives.push(Formula::equal(subject.clone(), ty.clone()));
                    }
                }
            }
            Formula::And(parts) | Formula::Or(parts) => {
                budget.steps = budget
                    .steps
                    .checked_sub(parts.len())
                    .ok_or(nir::NominalAdmissionError::Budget)?;
                pending.extend(parts);
            }
            Formula::Not(inner) => pending.push(inner),
            _ => {}
        }
    }
    if alternatives.is_empty() {
        return Ok(None);
    }
    match context.prove_metered(Formula::Or(alternatives), budget) {
        Outcome::Entailed(proof) => Ok(Some(proof)),
        Outcome::NotEntailed(_) => Ok(None),
        Outcome::Unknown { reason, .. } => Err(query_failure(reason)),
        Outcome::IllFormed(reason) => Err(nir::NominalAdmissionError::Invalid(reason)),
    }
}

pub(super) fn eligible_alternatives(context: &Context, subject: &Term, formula: &Formula) -> bool {
    matches!(formula, Formula::Or(parts) if !parts.is_empty() && parts.iter().all(|part|
        matches!(part, Formula::Equal(crate::ptc::Equality(original, shape))
            if original == subject && eligible_shape(context, shape))))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn an_independent_disjunction_proof_can_recover_after_view_exhaustion() {
        let subject = Term::Rigid {
            declaration: DeclarationId {
                package: 1,
                local: 1,
            },
            sort: Sort::Type,
        };
        let mut context = Context::default();
        context.assume(Formula::Or(vec![
            Formula::equal(subject.clone(), Term::Primitive(nir::Primitive::Bool)),
            Formula::equal(subject.clone(), Term::Primitive(nir::Primitive::Char)),
        ]));
        let mut remaining = Budget::default();
        let (shape, proof) = subject_view(
            &context,
            &subject,
            Budget {
                steps: 1,
                ..Budget::default()
            },
            &mut remaining,
        )
        .unwrap();
        assert_eq!(shape, subject);
        let proof = proof.unwrap();
        assert!(eligible_alternatives(&context, &subject, &proof.goal));
        context.verify_proof(&proof, Budget::default()).unwrap();
        assert!(remaining.steps > 0 && remaining.steps < Budget::default().steps);
    }
}
