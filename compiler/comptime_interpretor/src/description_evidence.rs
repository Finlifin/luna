//! A gradual Type carries universal facts about one subject in its publishing
//! world. Observing that Type and checking a returned Type use the same proof.
use super::*;
use nir::program::{DescriptionLiteral, TypePredicate as P};

pub(super) struct DescriptionEvidence {
    pub(super) subject: Term,
    // Keep the antecedent once across successive queries on this subject.
    implication: P,
}

impl Interpreter<'_> {
    pub(super) fn description_evidence(
        &mut self,
        description: &DescriptionLiteral,
        binder: Option<DeclarationId>,
        source: SourceRange,
    ) -> Result<DescriptionEvidence, Error> {
        let world = self
            .declaration_world
            .filter(|w| self.comptime_only && w.refinements.contains(description))
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
        // Membership above establishes the base and its ownership. Traversing
        // the immutable parent chain can now fail only on traversal fuel.
        let subject = world
            .refinements
            .subject(description, &mut self.budget.steps)
            .map_err(|_| self.failure(ErrorKind::StepsExhausted, Some(source)))?;
        let declaration = match binder {
            Some(id) => id,
            None => {
                self.query_admission(Some(source), |_, fuel| Ok(subject.fresh_subject_id(fuel)?))?
            }
        };
        let term = Term::Rigid {
            declaration,
            sort: nir::Sort::Type,
        };
        let mut premises = Vec::new();
        if subject.application.is_some_and(|app| {
            self.normal_forms
                .get(&app.head())
                .is_some_and(|head| head.role == nir::normal_forms::Role::Tuple)
        }) {
            premises.push(P::Kind(term.clone(), nir::TypeKind::Tuple));
        }
        for requirement in subject.requirements() {
            premises.push(self.query_admission(Some(source), |_, fuel| {
                Ok(requirement.instantiate(&term, &[], fuel)?)
            })?);
        }
        Ok(DescriptionEvidence {
            subject: term,
            implication: P::Or(vec![P::Not(Box::new(P::And(premises))), P::Bool(false)]),
        })
    }
}

impl DescriptionEvidence {
    pub(super) fn entails(
        &mut self,
        vm: &mut Interpreter<'_>,
        goal: P,
        source: SourceRange,
    ) -> Result<bool, Error> {
        let P::Or(parts) = &mut self.implication else {
            unreachable!()
        };
        let [P::Not(antecedent), consequent] = parts.as_mut_slice() else {
            unreachable!()
        };
        let P::And(premises) = antecedent.as_ref() else {
            unreachable!()
        };
        *consequent = goal;
        let mut pending = vec![premises.iter()];
        while let Some(iter) = pending.last_mut() {
            let Some(fact) = iter.next() else {
                pending.pop();
                continue;
            };
            vm.tick(Some(source))?;
            // Equality only needs to inspect as much structure as this fact.
            if std::mem::discriminant(fact) == std::mem::discriminant(consequent) {
                vm.charge_copy_work(Some(source), |fuel| {
                    nir::copy_budget::charge_predicate_substitution(fact, &[], &[], fuel)
                })?;
                if fact == consequent {
                    return Ok(true);
                }
            }
            if let (Some((a, fact_kind, truth)), Some((b, goal_kind, desired))) =
                (kind_fact(fact), kind_fact(consequent))
                && a == &self.subject
                && b == &self.subject
                && let Some(implied) = fact_kind.consequence(truth, goal_kind)
            {
                return Ok(implied == desired);
            }
            if let P::And(children) = fact {
                pending.push(children.iter());
            }
        }
        let query = if premises.is_empty() {
            &*consequent
        } else {
            &self.implication
        };
        vm.tick(Some(source))?;
        match query.trivial_truth() {
            Some(truth) => Ok(truth),
            None => Ok(vm
                .solve_type_predicate(query)
                .map_err(|kind| vm.failure(kind, Some(source)))?
                == ContractOutcome::Satisfied),
        }
    }
}

fn kind_fact(predicate: &P) -> Option<(&Term, nir::TypeKind, bool)> {
    match predicate {
        P::Kind(term, kind) => Some((term, *kind, true)),
        P::Not(inner) => match inner.as_ref() {
            P::Kind(term, kind) => Some((term, *kind, false)),
            _ => None,
        },
        _ => None,
    }
}
