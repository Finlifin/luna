//! Candidate-local Boolean requirements. Nested queries reuse Solver::active;
//! their facts and index-scoped resolutions accompany the resulting proof.
use super::*;
use crate::implementation::{EvidenceError, TypeAssessment};
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

#[cfg(test)]
mod tests {
    use super::*;
    use nir::program::TypePredicate;
    use nir::{DeclarationId, Primitive};

    #[test]
    fn rejected_projection_support_returns_query_work_before_propagating_failure() {
        let id = |local| DeclarationId { package: 96, local };
        let index = Index::default();
        let context = Context::default();
        let world = World {
            index: &index,
            context: &context,
            traits: &[],
            implementations: &[],
            assumptions: &[],
        };
        let subject = Term::Primitive(Primitive::Bool);
        let mut resolution = world.resolve(id(1), &subject, Budget::default()).unwrap();
        let Outcome::Entailed(proof) = context.prove(Formula::True, Budget::default()) else {
            panic!("True has a proof");
        };
        let source = Term::Pointer(Box::new(Term::AssociatedType {
            origin: Box::new(nir::AssociatedTypeOrigin {
                interface: id(1),
                member: id(2),
            }),
            arguments: vec![],
            subject: Box::new(subject.clone()),
        }));
        // Forge an extra support entry referencing an unregistered member.
        // It must fail while rebuilding the view, before trusting its proof.
        resolution.condition_support.insert(
            id(3),
            vec![ConditionSupport {
                context: context.clone(),
                facts: vec![],
                proof,
                projections: vec![ConditionProjection {
                    source: source.clone(),
                    value: subject,
                }],
            }],
        );
        for max_depth in [1, Budget::default().max_depth] {
            let mut expected = Budget {
                steps: Budget::default().steps - 1,
                max_depth,
            };
            let error = world
                .associated_type_view_cached_metered(
                    &source,
                    &mut expected,
                    &mut std::collections::HashMap::new(),
                )
                .unwrap_err();
            if max_depth == 1 {
                assert!(matches!(error, QueryError::Budget));
            } else {
                assert!(matches!(error, QueryError::Invalid(_)));
            }
            let mut actual = Budget {
                max_depth,
                ..Budget::default()
            };
            assert_eq!(
                world.verify_support_in_domain(&resolution, &mut actual, None),
                Err(EvidenceError::Query(error))
            );
            assert_eq!(
                actual.steps, expected.steps,
                "the failed view's work must survive the evidence boundary"
            );
            assert!(actual.steps > 0 && actual.steps < Budget::default().steps - 1);
        }
        resolution.condition_support.clear();
        let mut retry = Budget::default();
        world
            .verify_support_in_domain(&resolution, &mut retry, None)
            .unwrap();
        assert_eq!(retry.steps, Budget::default().steps - 1);
    }

    #[test]
    fn nested_condition_resolution_retains_recursion_and_budget_failures() {
        let contract = Trait {
            declaration: DeclarationId {
                package: 95,
                local: 1,
            },
            self_parameter: DeclarationId {
                package: 95,
                local: 2,
            },
            name: "Nested".into(),
            comptime_only: false,
            parameter_types: vec![],
            type_parameters: vec![],
            parents: vec![],
            associated: vec![],
            methods: vec![],
            capabilities: vec![],
            requirements: vec![],
        };
        let query = TraitQuery {
            interface: contract.declaration,
            subject: Term::Primitive(Primitive::Bool),
            arguments: vec![],
        };
        let traits = [contract];
        let index = Index::default();
        let context = Context::default();
        let world = World {
            index: &index,
            context: &context,
            traits: &traits,
            implementations: &[],
            assumptions: &[],
        };
        let mut solver = Solver {
            world: &world,
            scope: ResolutionScope::capture(&world),
            active: HashSet::new(),
            remaining: 100_000,
        };
        let key = (
            InterfaceInstance {
                declaration: query.interface,
                arguments: vec![],
            },
            query.subject.clone(),
        );
        solver.active.insert(key.clone());
        assert!(
            matches!(solver.condition_fact(&query, Budget::default(), 0),
            Err(QueryError::Unresolved(reason)) if reason.contains("recursive"))
        );
        assert!(solver.active.contains(&key));
        solver.active.remove(&key);
        assert!(matches!(
            solver.condition_fact(
                &query,
                Budget {
                    steps: 0,
                    ..Budget::default()
                },
                0
            ),
            Err(QueryError::Budget)
        ));
        assert!(
            !solver
                .condition_fact(&query, Budget::default(), 0)
                .unwrap()
                .truth
        );
        assert!(solver.active.is_empty());
    }

    #[test]
    fn conditional_formation_proofs_consume_their_reserved_allowance() {
        let id = |local| DeclarationId { package: 94, local };
        let trait_with = |requirements| Trait {
            declaration: id(1),
            self_parameter: id(2),
            name: "Conditional".into(),
            comptime_only: false,
            parameter_types: vec![],
            type_parameters: vec![],
            parents: vec![],
            associated: vec![],
            methods: vec![],
            capabilities: vec![],
            requirements,
        };
        let index = Index::default();
        let context = Context::default();
        let query = TraitQuery {
            interface: id(1),
            subject: Term::Primitive(Primitive::Bool),
            arguments: vec![],
        };
        let check = |traits: &[Trait], steps| {
            let world = World {
                index: &index,
                context: &context,
                traits,
                implementations: &[],
                assumptions: &[],
            };
            let mut solver = Solver {
                world: &world,
                scope: ResolutionScope::capture(&world),
                active: HashSet::new(),
                remaining: 100_000,
            };
            let result = solver.condition_fact(
                &query,
                Budget {
                    steps,
                    max_depth: 128,
                },
                0,
            );
            (result, 100_000 - solver.remaining)
        };
        let (empty, baseline) = check(&[trait_with(vec![])], 10_000);
        assert!(!empty.unwrap().truth);
        let count = 32;
        let traits = [trait_with(vec![TypePredicate::Bool(true); count])];
        let (complete, consumed) = check(&traits, 10_000);
        assert!(!complete.unwrap().truth);
        let mut proof_budget = Budget {
            steps: 10_000,
            max_depth: 128,
        };
        assert!(matches!(
            context.prove_metered(Formula::True, &mut proof_budget),
            Outcome::Entailed(_)
        ));
        let proof_cost = 10_000 - proof_budget.steps;
        assert!(proof_cost > 0);
        assert!(consumed >= baseline + count * (proof_cost + 1));
        let (short, consumed) = check(&traits, 8);
        assert!(matches!(short, Err(nir::NominalAdmissionError::Budget)));
        assert!(consumed > 0 && consumed <= 8);
        assert!(!check(&traits, 10_000).0.unwrap().truth);
        let malformed = [trait_with(vec![TypePredicate::Equal(
            Term::Bool(true),
            Term::Primitive(Primitive::Bool),
        )])];
        assert!(matches!(
            check(&malformed, 10_000).0,
            Err(nir::NominalAdmissionError::Invalid(_))
        ));
    }
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
        let mut pending_reason = None;
        for query in queries {
            let allowance = Budget {
                steps: self.remaining / (obligations + 1),
                ..budget
            };
            obligations -= 1;
            match self.condition_fact(query, allowance, depth + 1) {
                Ok(fact) => {
                    let atom = Formula::Trait(query.clone());
                    context.assume(if fact.truth { atom } else { atom.negate() });
                    facts.push(fact);
                }
                Err(nir::NominalAdmissionError::Budget) => {
                    pending_reason = Some(UnknownReason::BudgetExhausted);
                }
                Err(nir::NominalAdmissionError::Unresolved(reason)) => {
                    pending_reason.get_or_insert(UnknownReason::UnresolvedPrerequisite(reason));
                }
                Err(nir::NominalAdmissionError::Invalid(reason)) => {
                    return Outcome::IllFormed(reason);
                }
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
            match self.prerequisite_type(ty, allowance, depth + 1) {
                Ok(value) if value != *ty => {
                    context.assume(Formula::equal(ty.clone(), value.clone()));
                    projections.push(ConditionProjection {
                        source: ty.clone(),
                        value,
                    });
                }
                Ok(_) => {}
                Err(QueryError::Budget) => pending_reason = Some(UnknownReason::BudgetExhausted),
                Err(QueryError::Unresolved(reason)) => {
                    pending_reason.get_or_insert(UnknownReason::UnresolvedPrerequisite(reason));
                }
                Err(QueryError::Invalid(reason)) => return Outcome::IllFormed(reason),
            }
        }
        let mut proof_budget = Budget {
            steps: self.remaining,
            ..proof_budget
        };
        let before = proof_budget.steps;
        let outcome = context.prove_metered(goal, &mut proof_budget);
        self.remaining -= before - proof_budget.steps;
        // A missing fact does not prevent an independent proof. It does
        // prevent a partial-world countermodel from excluding the candidate.
        let outcome = retain_incomplete_queries(outcome, pending_reason);
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
    ) -> Result<ConditionFact, nir::NominalAdmissionError> {
        let outer = self.remaining;
        let allowance = outer.min(budget.steps);
        self.remaining = allowance;
        let result = self.condition_fact_in(query, budget, depth);
        self.remaining = outer - (allowance - self.remaining);
        result
    }

    fn condition_fact_in(
        &mut self,
        query: &TraitQuery,
        budget: Budget,
        depth: usize,
    ) -> Result<ConditionFact, nir::NominalAdmissionError> {
        if depth >= budget.max_depth.min(128) || self.remaining == 0 {
            return Err(nir::NominalAdmissionError::Budget);
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
        let substitution = contract.input_substitution(&inputs, &[], &mut self.remaining)?;
        for requirement in &contract.requirements {
            if requirement.mentions_rigid(contract.self_parameter) {
                continue;
            }
            let requirement =
                substitution.instantiate_predicate_bounded(requirement, &mut self.remaining)?;
            let mut proof_budget = Budget {
                steps: self.remaining.min(budget.steps),
                ..budget
            };
            let before = proof_budget.steps;
            let outcome = self
                .world
                .context
                .prove_metered(Formula::from_predicate(&requirement), &mut proof_budget);
            self.remaining -= before - proof_budget.steps;
            match outcome {
                Outcome::Entailed(_) => {}
                Outcome::NotEntailed(_) => {
                    return Err(QueryError::Unresolved(
                        "conditional interface requirement is not established".into(),
                    ));
                }
                Outcome::Unknown {
                    reason: UnknownReason::BudgetExhausted,
                    ..
                } => return Err(nir::NominalAdmissionError::Budget),
                Outcome::Unknown { reason, .. } => {
                    return Err(nir::NominalAdmissionError::Unresolved(format!(
                        "conditional interface requirement: {reason:?}"
                    )));
                }
                Outcome::IllFormed(reason) => {
                    return Err(nir::NominalAdmissionError::Invalid(reason));
                }
            }
        }
        // Formation must not bootstrap itself from the queried implementation.
        for capability in &contract.capabilities {
            if capability.mentions_rigid(contract.self_parameter) {
                continue;
            }
            let capability =
                substitution.instantiate_requirement_bounded(capability, &mut self.remaining)?;
            if self.given(&capability, budget)?.is_none() {
                let resolution = self.resolve(
                    &capability.interface_instance(),
                    &capability.subject,
                    budget,
                    depth + 1,
                )?;
                match resolution.types.selection {
                    Selection::Selected(_) => {}
                    _ => {
                        return Err(QueryError::Unresolved(format!(
                            "conditional interface capability: {}",
                            resolution.types.selection
                        )));
                    }
                }
            }
        }
        let requirement = TraitRequirement {
            interface: query.interface,
            interface_arguments: query.arguments.clone(),
            subject: query.subject.clone(),
        };
        let incomplete_given = match self.given(&requirement, budget) {
            Ok(Some(given)) => {
                return Ok(ConditionFact {
                    query: query.clone(),
                    truth: true,
                    given: Some(given),
                    resolution: None,
                });
            }
            Ok(None) => None,
            Err(error @ nir::NominalAdmissionError::Invalid(_)) => return Err(error),
            Err(error) => Some(error),
        };
        let resolution = self.resolve(
            &requirement.interface_instance(),
            &query.subject,
            budget,
            depth,
        )?;
        if !matches!(resolution.types.selection, Selection::Selected(_)) {
            if let Some(error) = incomplete_given {
                return Err(error);
            }
        }
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
            Selection::NoApplicableCandidate => {
                let mut allowance = Budget {
                    steps: self.remaining.min(budget.steps),
                    ..budget
                };
                let before = allowance.steps;
                let closed = self
                    .world
                    .closed_trait_inputs_metered(query, &mut allowance);
                self.remaining -= before - allowance.steps;
                if !closed? {
                    return Err(QueryError::Unresolved(
                        "conditional trait inputs remain open".into(),
                    ));
                }
                false
            }
            _ => {
                return Err(QueryError::Unresolved(format!(
                    "conditional trait query: {}",
                    resolution.types.selection
                )));
            }
        };
        Ok(ConditionFact {
            query: query.clone(),
            truth,
            given: None,
            resolution: Some(Box::new(resolution)),
        })
    }
}

impl World<'_> {
    /// Replay candidate-local requirement support. This validates support, not
    /// the entire head/admission/selection protocol.
    pub fn verify_condition_support(
        &self,
        resolution: &Resolution,
        budget: Budget,
    ) -> Result<(), EvidenceError> {
        self.verify_support_in_domain(resolution, &mut { budget }, None)
    }

    /// The root belongs to the inherent member index; nested facts still belong
    /// to this world's trait index. Neither index may be substituted on replay.
    pub fn verify_inherent_condition_support(
        &self,
        index: &InherentIndex,
        resolution: &Resolution,
        budget: Budget,
    ) -> Result<(), EvidenceError> {
        self.verify_support_in_domain(resolution, &mut { budget }, Some(index))
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
    ) -> Result<(), EvidenceError> {
        let mut budget = budget;
        self.verify_support_in_domain(resolution, &mut budget, Some(index))?;
        let Selection::Selected(id) = resolution.types.selection else {
            return Err(ProofError::Invalid.into());
        };
        let Some(TypeAssessment::Applicable(evidence)) = resolution.types.assessments.get(&id)
        else {
            return Err(ProofError::Invalid.into());
        };
        index.verify_evidence_with(
            name,
            evidence,
            subject,
            self.context,
            budget,
            |proof, allowance| {
                let supports = resolution.condition_support.get(&id);
                let mut allowance = allowance;
                let contexts = std::iter::once(self.context).chain(
                    supports
                        .into_iter()
                        .flatten()
                        .map(|support| &support.context),
                );
                let mut failure = ProofError::Invalid;
                for context in contexts {
                    match context.verify_proof_metered(proof, &mut allowance) {
                        Ok(()) => return Ok(()),
                        Err(ProofError::BudgetExhausted) => failure = ProofError::BudgetExhausted,
                        Err(ProofError::ArithmeticRange)
                            if failure != ProofError::BudgetExhausted =>
                        {
                            failure = ProofError::ArithmeticRange
                        }
                        Err(_) => {}
                    }
                }
                Err(failure)
            },
        )
    }

    fn verify_support_in_domain(
        &self,
        resolution: &Resolution,
        budget: &mut Budget,
        inherent: Option<&InherentIndex>,
    ) -> Result<(), EvidenceError> {
        let mut fuel = budget.steps;
        let result = (|| {
            let mut pending = vec![(resolution, 0usize)];
            while let Some((resolution, depth)) = pending.pop() {
                if depth >= budget.max_depth.min(128) || fuel == 0 {
                    return Err(ProofError::BudgetExhausted.into());
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
                    return Err(ProofError::Invalid.into());
                }
                for support in resolution.condition_support.values().flatten() {
                    let mut expected = self.context.fork();
                    for fact in &support.facts {
                        let atom = Formula::Trait(fact.query.clone());
                        expected.assume(if fact.truth { atom } else { atom.negate() });
                        if let Some(given) = &fact.given {
                            if !fact.truth {
                                return Err(ProofError::Invalid.into());
                            }
                            let mut allowance = Budget {
                                steps: fuel,
                                ..*budget
                            };
                            let found = self.given_metered(given, &mut allowance);
                            fuel = allowance.steps;
                            let found = found?;
                            if found.is_none() {
                                return Err(ProofError::Invalid.into());
                            }
                        }
                        if let Some(nested) = &fact.resolution {
                            pending.push((nested, depth + 1));
                        }
                    }
                    let mut completed = std::collections::HashMap::new();
                    for projection in &support.projections {
                        let mut allowance = Budget {
                            steps: fuel,
                            ..*budget
                        };
                        let normalized = self.associated_type_view_cached_metered(
                            &projection.source,
                            &mut allowance,
                            &mut completed,
                        );
                        fuel = allowance.steps;
                        let normalized = normalized?;
                        if normalized.value != projection.value {
                            return Err(ProofError::Invalid.into());
                        }
                        expected.assume(Formula::equal(
                            projection.source.clone(),
                            projection.value.clone(),
                        ));
                    }
                    if expected.assumptions() != support.context.assumptions() {
                        return Err(ProofError::Invalid.into());
                    }
                    let mut allowance = Budget {
                        steps: fuel,
                        ..*budget
                    };
                    let checked = expected.verify_proof_metered(&support.proof, &mut allowance);
                    fuel = allowance.steps;
                    checked?;
                }
            }
            Ok(())
        })();
        budget.steps = fuel;
        result
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
