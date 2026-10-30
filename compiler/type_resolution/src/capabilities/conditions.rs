//! Candidate-local Boolean requirements. Nested queries reuse Solver::active;
//! their facts and index-scoped resolutions accompany the resulting proof.
use super::*;
use crate::implementation::TypeAssessment;
use crate::ptc::{Proof, ProofError, TraitQuery, UnknownReason};

#[derive(Debug)]
pub struct ConditionSupport {
    context: Context,
    facts: Vec<ConditionFact>,
    projections: Vec<ConditionProjection>,
    proof: Proof,
}

/// Candidate-local associated reductions, never equations for opaque heads.
/// Replay recomputes the view in the same immutable implementation world.
#[derive(Debug)]
struct ConditionProjection {
    source: Term,
    value: Term,
}

#[derive(Debug)]
struct ConditionFact {
    query: TraitQuery,
    truth: bool,
    given: Option<TraitRequirement>,
    resolution: Option<Box<Resolution>>,
}

impl Solver<'_, '_> {
    pub(super) fn prove_condition_metered(
        &mut self,
        goal: Formula,
        budget: &mut Budget,
        depth: usize,
        support: &mut Vec<ConditionSupport>,
    ) -> Outcome {
        let outer = self.remaining;
        let allowance = budget.steps.min(outer);
        self.remaining = allowance;
        let outcome = self.prove_condition(
            goal,
            Budget {
                steps: allowance,
                ..*budget
            },
            depth,
            support,
        );
        let consumed = allowance - self.remaining;
        self.remaining = outer - consumed;
        budget.steps -= consumed;
        outcome
    }

    pub(super) fn prove_condition(
        &mut self,
        goal: Formula,
        budget: Budget,
        depth: usize,
        support: &mut Vec<ConditionSupport>,
    ) -> Outcome {
        let unknown = |goal| Outcome::Unknown {
            reason: UnknownReason::BudgetExhausted,
            residual: goal,
        };
        if depth >= budget.max_depth.min(128) || budget.steps < 4 {
            return unknown(goal);
        }
        if !crate::ptc::within_limits(
            &goal,
            budget.max_depth.min(128) - depth,
            &mut self.remaining,
        ) {
            return unknown(goal);
        }
        let mut proof_budget = Budget {
            steps: (budget.steps / 4).min(self.remaining),
            ..budget
        };
        let before = proof_budget.steps;
        let direct = self
            .world
            .context
            .prove_metered(goal.clone(), &mut proof_budget);
        self.remaining -= before - proof_budget.steps;
        if matches!(direct, Outcome::Entailed(_) | Outcome::IllFormed(_)) {
            return direct;
        }
        let mut pending = vec![&goal];
        pending.extend(self.world.context.assumptions());
        let mut queries = Vec::new();
        let mut seen = HashSet::new();
        let mut projection_types = Vec::new();
        let mut seen_types = HashSet::new();
        while let Some(formula) = pending.pop() {
            if self.remaining == 0 {
                return unknown(goal);
            }
            self.remaining -= 1;
            match formula {
                Formula::Trait(query) if seen.insert(query) => queries.push(query),
                Formula::Equal(crate::ptc::Equality(left, right)) => {
                    for ty in [left, right] {
                        if ty.sort() == nir::Sort::Type
                            && !matches!(ty, Term::Primitive(_))
                            && seen_types.insert(ty)
                        {
                            projection_types.push(ty);
                        }
                    }
                }
                Formula::Not(inner) => pending.push(inner),
                Formula::And(items) | Formula::Or(items) => pending.extend(items),
                _ => {}
            }
        }
        if queries.is_empty() && projection_types.is_empty() {
            return direct;
        }
        // Keep a share for the final proof, while returning unused search
        // work to the following obligations instead of stranding fixed slices.
        let mut obligations = queries.len() + projection_types.len();
        let mut context = self.world.context.fork();
        let mut facts = Vec::new();
        for query in queries {
            let allowance = Budget {
                steps: self.remaining / (obligations + 1),
                ..budget
            };
            obligations -= 1;
            match self.condition_fact(query, allowance, depth + 1) {
                Ok(Some(fact)) => {
                    let atom = Formula::Trait(query.clone());
                    context.assume(if fact.truth { atom } else { atom.negate() });
                    facts.push(fact);
                }
                Ok(None) => {}
                Err(reason) => return Outcome::IllFormed(reason),
            }
        }
        let mut projections = Vec::new();
        for ty in projection_types {
            let allowance = Budget {
                steps: self.remaining / (obligations + 1),
                ..budget
            };
            obligations -= 1;
            // Normalization uses the original world and the active selection
            // stack. A candidate cannot justify itself with its own equation.
            if let Ok(value) = self.prerequisite_type(ty, allowance, depth + 1) {
                if value != *ty {
                    context.assume(Formula::equal(ty.clone(), value.clone()));
                    projections.push(ConditionProjection {
                        source: ty.clone(),
                        value,
                    });
                }
            }
        }
        let mut proof_budget = Budget {
            steps: self.remaining,
            ..proof_budget
        };
        let before = proof_budget.steps;
        let outcome = context.prove_metered(goal, &mut proof_budget);
        self.remaining -= before - proof_budget.steps;
        if let Outcome::Entailed(proof) = &outcome {
            support.push(ConditionSupport {
                context,
                facts,
                projections,
                proof: proof.clone(),
            });
        }
        outcome
    }

    fn condition_fact(
        &mut self,
        query: &TraitQuery,
        budget: Budget,
        depth: usize,
    ) -> Result<Option<ConditionFact>, String> {
        if depth >= budget.max_depth.min(128) || self.remaining == 0 {
            return Ok(None);
        }
        let contract = self
            .world
            .traits
            .iter()
            .find(|t| t.declaration == query.interface)
            .ok_or("conditional requirement references an unknown interface")?;
        if contract.type_parameters.len() != query.arguments.len() {
            return Err("conditional requirement interface argument count mismatch".into());
        }
        contract.validate_arguments_with_layouts(
            &query.subject,
            &query.arguments,
            &[],
            self.world.traits,
            &self.world.index.parameter_domains,
            &mut self.remaining,
            |left, right, fuel| {
                super::equivalent_domains(self.world.context, left, right, budget.max_depth, fuel)
            },
            Some(&self.world.index.nominal_layouts),
        )?;
        let inputs: Vec<_> = std::iter::once(query.subject.clone())
            .chain(query.arguments.iter().cloned())
            .collect();
        let substitution = contract
            .input_substitution(&inputs, &[], &mut self.remaining)
            .map_err(|error| error.to_string())?;
        for requirement in &contract.requirements {
            if requirement.mentions_rigid(contract.self_parameter) {
                continue;
            }
            let requirement = substitution
                .instantiate_predicate_bounded(requirement, &mut self.remaining)
                .map_err(|error| error.to_string())?;
            if !matches!(
                self.world
                    .context
                    .prove(Formula::from_predicate(&requirement), budget),
                Outcome::Entailed(_)
            ) {
                return Ok(None);
            }
        }
        // Formation must not bootstrap itself from the queried implementation.
        for capability in &contract.capabilities {
            if capability.mentions_rigid(contract.self_parameter) {
                continue;
            }
            let capability = substitution
                .instantiate_requirement_bounded(capability, &mut self.remaining)
                .map_err(|error| error.to_string())?;
            if self.given(&capability, budget).is_none() {
                let Ok(resolution) = self.resolve(
                    &capability.interface_instance(),
                    &capability.subject,
                    budget,
                    depth + 1,
                ) else {
                    return Ok(None);
                };
                if !matches!(resolution.types.selection, Selection::Selected(_)) {
                    return Ok(None);
                }
            }
        }
        let requirement = TraitRequirement {
            interface: query.interface,
            interface_arguments: query.arguments.clone(),
            subject: query.subject.clone(),
        };
        if let Some(given) = self.given(&requirement, budget) {
            return Ok(Some(ConditionFact {
                query: query.clone(),
                truth: true,
                given: Some(given),
                resolution: None,
            }));
        }
        let Ok(resolution) = self.resolve(
            &requirement.interface_instance(),
            &query.subject,
            budget,
            depth,
        ) else {
            return Ok(None);
        };
        let truth = match &resolution.types.selection {
            Selection::Selected(id)
                if matches!(
                    resolution.types.assessments.get(id),
                    Some(TypeAssessment::Applicable(_))
                ) && matches!(
                    resolution.qualifications.get(id),
                    Some(Qualification::Proved(_))
                ) =>
            {
                true
            }
            Selection::NoApplicableCandidate if self.world.closed_trait_inputs(query, budget) => {
                false
            }
            _ => return Ok(None),
        };
        Ok(Some(ConditionFact {
            query: query.clone(),
            truth,
            given: None,
            resolution: Some(Box::new(resolution)),
        }))
    }
}

impl World<'_> {
    /// Replay candidate-local requirement support. This validates support, not
    /// the entire head/admission/selection protocol.
    pub fn verify_condition_support(
        &self,
        resolution: &Resolution,
        budget: Budget,
    ) -> Result<(), ProofError> {
        self.verify_support_in_domain(resolution, budget, None)
    }

    /// The root belongs to the inherent member index; nested facts still belong
    /// to this world's trait index. Neither index may be substituted on replay.
    pub fn verify_inherent_condition_support(
        &self,
        index: &InherentIndex,
        resolution: &Resolution,
        budget: Budget,
    ) -> Result<(), ProofError> {
        self.verify_support_in_domain(resolution, budget, Some(index))
    }

    /// Replay the selected inherent head and its requirements without importing
    /// candidate facts into unrelated head equalities.
    pub fn verify_inherent_evidence(
        &self,
        index: &InherentIndex,
        name: &str,
        subject: &Term,
        resolution: &Resolution,
        budget: Budget,
    ) -> Result<(), ProofError> {
        self.verify_inherent_condition_support(index, resolution, budget)?;
        let Selection::Selected(id) = resolution.types.selection else {
            return Err(ProofError::Invalid);
        };
        let Some(TypeAssessment::Applicable(evidence)) = resolution.types.assessments.get(&id)
        else {
            return Err(ProofError::Invalid);
        };
        index.verify_evidence_with(
            name,
            evidence,
            subject,
            self.context,
            budget,
            |proof, allowance| {
                if self.context.verify_proof(proof, allowance).is_ok() {
                    return Ok(());
                }
                for support in resolution.condition_support.get(&id).into_iter().flatten() {
                    if support.context.verify_proof(proof, allowance).is_ok() {
                        return Ok(());
                    }
                }
                Err(ProofError::Invalid)
            },
        )
    }

    fn verify_support_in_domain(
        &self,
        resolution: &Resolution,
        budget: Budget,
        inherent: Option<&InherentIndex>,
    ) -> Result<(), ProofError> {
        let mut fuel = budget.steps;
        let mut pending = vec![(resolution, 0usize)];
        while let Some((resolution, depth)) = pending.pop() {
            if depth >= budget.max_depth.min(128) || fuel == 0 {
                return Err(ProofError::BudgetExhausted);
            }
            fuel -= 1;
            let current = if depth == 0 {
                inherent.map_or_else(
                    || self.index.type_resolution_is_current(&resolution.types),
                    |index| index.resolution_is_current(&resolution.types),
                )
            } else {
                self.index.type_resolution_is_current(&resolution.types)
            };
            if !current || !resolution.scope.matches(self) {
                return Err(ProofError::Invalid);
            }
            for support in resolution.condition_support.values().flatten() {
                let mut expected = self.context.fork();
                for fact in &support.facts {
                    let atom = Formula::Trait(fact.query.clone());
                    expected.assume(if fact.truth { atom } else { atom.negate() });
                    if let Some(given) = &fact.given {
                        if !fact.truth
                            || self
                                .given(
                                    given,
                                    Budget {
                                        steps: fuel,
                                        ..budget
                                    },
                                )
                                .is_none()
                        {
                            return Err(ProofError::Invalid);
                        }
                    }
                    if let Some(nested) = &fact.resolution {
                        pending.push((nested, depth + 1));
                    }
                }
                let mut completed = std::collections::HashMap::new();
                for projection in &support.projections {
                    let allowance = Budget {
                        steps: fuel,
                        ..budget
                    };
                    let (normalized, remaining) = self
                        .associated_type_view_cached(&projection.source, allowance, &mut completed)
                        .map_err(|_| ProofError::Invalid)?;
                    fuel = remaining;
                    if normalized.value != projection.value {
                        return Err(ProofError::Invalid);
                    }
                    expected.assume(Formula::equal(
                        projection.source.clone(),
                        projection.value.clone(),
                    ));
                }
                if expected.assumptions() != support.context.assumptions() {
                    return Err(ProofError::Invalid);
                }
                expected.verify_proof(
                    &support.proof,
                    Budget {
                        steps: fuel,
                        ..budget
                    },
                )?;
            }
        }
        Ok(())
    }
}

impl fmt::Display for ConditionSupport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(condition-support (proof {}) (facts", self.proof)?;
        for fact in &self.facts {
            write!(f, "(fact (query {}) (truth {})", fact.query, fact.truth)?;
            if let Some(given) = &fact.given {
                write!(f, " (given {given})")?;
            }
            if let Some(resolution) = &fact.resolution {
                write!(f, " (resolution {resolution})")?;
            }
            f.write_str(")")?;
        }
        f.write_str(") (projections")?;
        for projection in &self.projections {
            write!(
                f,
                " (projection (source {}) (value {}))",
                projection.source, projection.value
            )?;
        }
        f.write_str("))")
    }
}
