//! Trait truth justified by one borrowed, immutable implementation world.
//! Decisions are deliberately not global PTC assumptions or runtime dispatch.
use super::*;
use crate::implementation::TypeAssessment;
use crate::ptc::TraitQuery;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraitTruth {
    Established,
    Excluded,
    Pending,
}

pub struct TraitDecision<'a, 'w> {
    world: &'a World<'w>,
    query: TraitQuery,
    truth: TraitTruth,
    given: Option<TraitRequirement>,
    resolution: Option<Resolution>,
    input_equalities: Vec<crate::ptc::Proof>,
}

/// Owns the PTC result and its additional evidence while borrowing the exact
/// world. Callers cannot detach it into an unconditional global theorem.
pub struct WorldProposition<'a, 'w> {
    world: &'a World<'w>,
    decisions: Vec<TraitDecision<'a, 'w>>,
    context: Context,
    outcome: Outcome,
}

impl WorldProposition<'_, '_> {
    pub fn outcome(&self) -> &Outcome {
        &self.outcome
    }
    pub fn decisions(&self) -> &[TraitDecision<'_, '_>] {
        &self.decisions
    }
    pub fn verify(&self, world: &World<'_>, budget: Budget) -> Result<(), crate::ptc::ProofError> {
        if !std::ptr::eq(self.world, world) || self.decisions.iter().any(|d| !d.belongs_to(world)) {
            return Err(crate::ptc::ProofError::Invalid);
        }
        let Outcome::Entailed(proof) = &self.outcome else {
            return Err(crate::ptc::ProofError::Invalid);
        };
        self.context.verify_proof(proof, budget)
    }
}

impl TraitDecision<'_, '_> {
    pub fn truth(&self) -> TraitTruth {
        self.truth
    }
    pub fn query(&self) -> &TraitQuery {
        &self.query
    }
    pub fn resolution(&self) -> Option<&Resolution> {
        self.resolution.as_ref()
    }
    pub fn belongs_to(&self, world: &World<'_>) -> bool {
        std::ptr::eq(self.world, world)
            && self
                .resolution
                .as_ref()
                .is_none_or(|resolution| world.index.type_resolution_is_current(&resolution.types))
    }
}

impl<'w> World<'w> {
    /// Check a retained type contract without publishing world facts into the
    /// caller context. The scoped support is consumed by this check only.
    pub fn check_predicate(
        &self,
        predicate: &nir::program::TypePredicate,
        budget: Budget,
    ) -> Result<Outcome, String> {
        let goal = Formula::from_predicate(predicate);
        let direct = self.context.prove(goal.clone(), budget);
        if matches!(direct, Outcome::Entailed(_) | Outcome::IllFormed(_)) {
            return Ok(direct);
        }
        Ok(self.prove_proposition(goal, budget)?.outcome().clone())
    }
    /// Share a single allowance across the direct proof and world-dependent
    /// fallback. Trait decisions reserve their bounded allowance; PTC returns
    /// the work it actually consumed.
    pub fn check_predicate_metered(
        &self, predicate: &nir::program::TypePredicate, budget: &mut Budget,
    ) -> Result<Outcome, String> {
        let goal=Formula::from_predicate(predicate);
        let direct=self.context.prove_metered(goal.clone(),budget);
        if matches!(direct,Outcome::Entailed(_) | Outcome::IllFormed(_)) {return Ok(direct);}
        Ok(self.prove_proposition_metered(goal,budget)?.outcome().clone())
    }
    pub fn prove_proposition<'a>(
        &'a self,
        goal: Formula,
        mut budget: Budget,
    ) -> Result<WorldProposition<'a, 'w>, String> {
        self.prove_proposition_metered(goal, &mut budget)
    }
    pub fn prove_proposition_metered<'a>(
        &'a self, goal: Formula, budget: &mut Budget,
    ) -> Result<WorldProposition<'a, 'w>, String> {
        let mut fuel = budget.steps;
        for formula in self
            .context
            .assumptions()
            .iter()
            .chain(std::iter::once(&goal))
        {
            if !crate::ptc::within_limits(formula, budget.max_depth.min(128), &mut fuel) {
                budget.steps = fuel;
                return Err("trait proposition input budget exhausted".into());
            }
        }
        budget.steps = fuel;
        let mut pending: Vec<_> = self
            .context
            .assumptions()
            .iter()
            .chain(std::iter::once(&goal))
            .collect();
        let mut queries = Vec::new();
        let mut seen = HashSet::new();
        while let Some(formula) = pending.pop() {
            match formula {
                Formula::Trait(query) if seen.insert(query) => queries.push(query),
                Formula::Not(inner) => pending.push(inner),
                Formula::And(items) | Formula::Or(items) => pending.extend(items),
                _ => {}
            }
        }
        // Each query gets a disjoint allowance; the final Boolean/theory proof
        // cannot multiply the budget by the number of atoms.
        let allowance = Budget {
            steps: fuel / queries.len().saturating_add(1),
            ..*budget
        };
        if allowance.steps == 0 {
            return Err("trait proposition budget exhausted".into());
        }
        let mut context = self.context.fork();
        let mut decisions = Vec::new();
        for query in queries {
            budget.steps -= allowance.steps;
            let decision = self.decide_trait(query, allowance)?;
            let fact = Formula::Trait(query.clone());
            match decision.truth {
                TraitTruth::Established => context.assume(fact),
                TraitTruth::Excluded => context.assume(fact.negate()),
                TraitTruth::Pending => {}
            }
            decisions.push(decision);
        }
        let mut proof_budget = allowance;
        let outcome = context.prove_metered(goal, &mut proof_budget);
        budget.steps -= allowance.steps - proof_budget.steps;
        Ok(WorldProposition {
            world: self,
            decisions,
            context,
            outcome,
        })
    }

    fn proposition_input_views(
        &self,
        query: &TraitQuery,
        budget: Budget,
    ) -> Result<(TraitQuery, Vec<crate::ptc::Proof>), String> {
        let allowance = Budget {
            steps: budget.steps / query.arguments.len().saturating_add(1),
            ..budget
        };
        if allowance.steps == 0 {
            return Err("trait proposition input view budget exhausted".into());
        }
        let mut proofs = Vec::new();
        let mut view = |ty: &Term| -> Result<Term, String> {
            if ty.check_scope(&[]).is_ok() {
                return Ok(ty.clone());
            }
            match self.context.structural_view(ty, allowance) {
                Ok(Some((value, proof))) => {
                    proofs.push(proof);
                    Ok(value)
                }
                Err(crate::ptc::UnknownReason::BudgetExhausted) => {
                    Err("trait proposition input view budget exhausted".into())
                }
                _ => Ok(ty.clone()),
            }
        };
        let subject = view(&query.subject)?;
        let arguments = query
            .arguments
            .iter()
            .map(&mut view)
            .collect::<Result<_, _>>()?;
        Ok((
            TraitQuery {
                interface: query.interface,
                subject,
                arguments,
            },
            proofs,
        ))
    }

    /// Query a formed interface in this admitted world. Parameter formation is
    /// checked here; subject-dependent promises are justified by the selected
    /// implementation or given evidence, as in ordinary capability checking.
    pub fn decide_trait<'a>(
        &'a self,
        query: &TraitQuery,
        budget: Budget,
    ) -> Result<TraitDecision<'a, 'w>, String> {
        let mut fuel = budget.steps;
        for input in query.inputs() {
            crate::associated::charge_projection_copy(input, &mut fuel)?;
            if input.sort() != nir::Sort::Type {
                return Err("trait proposition requires Type inputs".into());
            }
            input
                .check_scope(self.context.scope())
                .map_err(|e| format!("invalid trait proposition input: {e:?}"))?;
        }
        if !crate::ptc::within_limits(
            &Formula::Trait(query.clone()),
            budget.max_depth.min(128),
            &mut fuel,
        ) {
            return Err("trait proposition input budget exhausted".into());
        }
        let contract = self
            .traits
            .iter()
            .find(|t| t.declaration == query.interface)
            .ok_or("trait proposition references an unknown interface")?;
        if contract.type_parameters.len() != query.arguments.len() {
            return Err("trait proposition interface argument count mismatch".into());
        }
        // Reserve separate bounded allowances for every proof/selection query.
        let count = contract
            .requirements
            .len()
            .saturating_add(contract.capabilities.len().saturating_mul(2))
            .saturating_add(query.arguments.len())
            .saturating_add(4);
        let allowance = Budget {
            steps: fuel / count,
            ..budget
        };
        if allowance.steps == 0 {
            return Err("trait proposition budget exhausted".into());
        }
        for requirement in &contract.requirements {
            if requirement.mentions_rigid(contract.self_parameter) {
                continue;
            }
            let requirement = requirement
                .instantiate_parameters_in_scope(
                    &contract
                        .type_parameters
                        .iter()
                        .map(|id| nir::DeclarationParameter {
                            declaration: *id,
                            sort: nir::Sort::Type,
                        })
                        .collect::<Vec<_>>(),
                    &query.arguments,
                    self.context.scope(),
                )
                .map_err(|e| e.to_string())?;
            if !matches!(
                self.context
                    .prove(Formula::from_predicate(&requirement), allowance),
                Outcome::Entailed(_)
            ) {
                return Err("trait proposition interface requirement is not established".into());
            }
        }
        for requirement in &contract.capabilities {
            if requirement.mentions_rigid(contract.self_parameter) {
                continue;
            }
            let requirement = requirement
                .instantiate_parameters_in_scope(
                    &contract
                        .type_parameters
                        .iter()
                        .map(|id| nir::DeclarationParameter {
                            declaration: *id,
                            sort: nir::Sort::Type,
                        })
                        .collect::<Vec<_>>(),
                    &query.arguments,
                    self.context.scope(),
                )
                .map_err(|e| e.to_string())?;
            if self.given_in_context(&requirement, allowance).is_none() {
                let (formed, _) =
                    self.proposition_input_views(&TraitQuery::from(&requirement), allowance)?;
                if formed.inputs().any(|ty| ty.check_scope(&[]).is_err()) {
                    return Err("trait proposition interface capability is not established".into());
                }
                let resolution = self.resolve_instance(
                    &InterfaceInstance {
                        declaration: formed.interface,
                        arguments: formed.arguments,
                    },
                    &formed.subject,
                    allowance,
                )?;
                if !matches!(resolution.types.selection, Selection::Selected(_)) {
                    return Err("trait proposition interface capability is not established".into());
                }
            }
        }
        let requirement = TraitRequirement {
            interface: query.interface,
            interface_arguments: query.arguments.clone(),
            subject: query.subject.clone(),
        };
        if let Some(given) = self.given_in_context(&requirement, allowance) {
            return Ok(TraitDecision {
                world: self,
                query: query.clone(),
                truth: TraitTruth::Established,
                given: Some(given),
                resolution: None,
                input_equalities: Vec::new(),
            });
        }
        let (selected_query, input_equalities) = self.proposition_input_views(query, allowance)?;
        if selected_query
            .inputs()
            .any(|ty| ty.check_scope(&[]).is_err())
        {
            return Ok(TraitDecision {
                world: self,
                query: query.clone(),
                truth: TraitTruth::Pending,
                given: None,
                resolution: None,
                input_equalities,
            });
        }
        let resolution = self.resolve_instance(
            &InterfaceInstance {
                declaration: selected_query.interface,
                arguments: selected_query.arguments.clone(),
            },
            &selected_query.subject,
            allowance,
        )?;
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
                TraitTruth::Established
            }
            Selection::NoApplicableCandidate
                if self.closed_trait_inputs(&selected_query, allowance)
                    && resolution.types.assessments.iter().all(
                        |(id, assessment)| match assessment {
                            TypeAssessment::Excluded(_) | TypeAssessment::ForbiddenSubject(_) => {
                                true
                            }
                            TypeAssessment::Applicable(_) => matches!(
                                resolution.qualifications.get(id),
                                Some(Qualification::Excluded(_))
                            ),
                            _ => false,
                        },
                    ) =>
            {
                TraitTruth::Excluded
            }
            _ => TraitTruth::Pending,
        };
        Ok(TraitDecision {
            world: self,
            query: query.clone(),
            truth,
            given: None,
            resolution: Some(resolution),
            input_equalities,
        })
    }
}

impl fmt::Display for TraitDecision<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let truth = match self.truth {
            TraitTruth::Established => "established",
            TraitTruth::Excluded => "excluded",
            TraitTruth::Pending => "pending",
        };
        write!(f, "(trait-decision (query {}) (truth {truth})", self.query)?;
        if !self.input_equalities.is_empty() {
            f.write_str(" (input-equalities")?;
            for proof in &self.input_equalities {
                write!(f, " {proof}")?;
            }
            f.write_str(")")?;
        }
        if let Some(given) = &self.given {
            write!(f, " (given {given})")?;
        }
        if let Some(resolution) = &self.resolution {
            write!(f, " (resolution {resolution})")?;
        }
        f.write_str(")")
    }
}

impl fmt::Display for WorldProposition<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(world-proposition (outcome {}) (decisions",
            self.outcome
        )?;
        for decision in &self.decisions {
            write!(f, " {decision}")?;
        }
        f.write_str("))")
    }
}
