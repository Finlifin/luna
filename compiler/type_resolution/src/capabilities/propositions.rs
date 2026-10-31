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

// Each operation stays within its assigned slot; only reported work is
// withdrawn from the shared query allowance, including errors.
fn metered<T>(fuel: &mut usize, allowance: Budget, run: impl FnOnce(&mut Budget) -> T) -> T {
    let mut local = Budget {
        steps: allowance.steps.min(*fuel),
        ..allowance
    };
    let before = local.steps;
    let result = run(&mut local);
    *fuel -= before - local.steps.min(before);
    result
}

// Unmetered operations retain conservative charging, bounded by the actual
// remaining counter rather than receiving fresh fuel after exhaustion.
fn reserved<T>(fuel: &mut usize, allowance: Budget, run: impl FnOnce(Budget) -> T) -> T {
    let local = Budget {
        steps: allowance.steps.min(*fuel),
        ..allowance
    };
    *fuel -= local.steps;
    run(local)
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
    /// fallback. Trait selection and PTC return unused work; operations without
    /// accounting still reserve their bounded allowance.
    pub fn check_predicate_metered(
        &self,
        predicate: &nir::program::TypePredicate,
        budget: &mut Budget,
    ) -> Result<Outcome, String> {
        let goal = Formula::from_predicate(predicate);
        let direct = self.context.prove_metered(goal.clone(), budget);
        if matches!(direct, Outcome::Entailed(_) | Outcome::IllFormed(_)) {
            return Ok(direct);
        }
        Ok(self
            .prove_proposition_metered(goal, budget)?
            .outcome()
            .clone())
    }
    pub fn prove_proposition<'a>(
        &'a self,
        goal: Formula,
        mut budget: Budget,
    ) -> Result<WorldProposition<'a, 'w>, String> {
        self.prove_proposition_metered(goal, &mut budget)
    }
    pub fn prove_proposition_metered<'a>(
        &'a self,
        goal: Formula,
        budget: &mut Budget,
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
        // Metered queries share the remaining allowance. Fixed per-atom
        // slices stranded the unused work of cheap given assumptions while
        // the one implementation search returned Pending prematurely.
        let mut context = self.context.fork();
        let mut decisions = Vec::new();
        for query in queries {
            let decision = self.decide_trait_metered(query, budget)?;
            let fact = Formula::Trait(query.clone());
            match decision.truth {
                TraitTruth::Established => context.assume(fact),
                TraitTruth::Excluded => context.assume(fact.negate()),
                TraitTruth::Pending => {}
            }
            decisions.push(decision);
        }
        let outcome = context.prove_metered(goal, budget);
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
        budget: &mut Budget,
    ) -> Result<(TraitQuery, Vec<crate::ptc::Proof>), String> {
        let mut proofs = Vec::new();
        let mut view = |ty: &Term| -> Result<Term, String> {
            crate::associated::charge_projection_copy(ty, &mut budget.steps)?;
            if ty.check_scope(&[]).is_ok() {
                return Ok(ty.clone());
            }
            let (result, consumed) = self.context.structural_view_metered(ty, *budget);
            budget.steps -= consumed;
            match result {
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
        self.decide_trait_metered(query, &mut { budget })
    }

    /// Return unused selection/proof allowance even if formation fails.
    pub fn decide_trait_metered<'a>(
        &'a self,
        query: &TraitQuery,
        remaining: &mut Budget,
    ) -> Result<TraitDecision<'a, 'w>, String> {
        let budget = *remaining;
        let mut fuel = budget.steps;
        let result = (|| {
            if query.subject.sort() != nir::Sort::Type {
                return Err("trait proposition requires a Type subject".into());
            }
            for input in query.inputs() {
                crate::associated::charge_projection_copy(input, &mut fuel)?;
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
            contract.validate_arguments_with_layouts(
                &query.subject,
                &query.arguments,
                self.context.scope(),
                self.traits,
                &self.index.parameter_domains,
                &mut fuel,
                |left, right, fuel| {
                    super::equivalent_domains(self.context, left, right, budget.max_depth, fuel)
                },
                Some(&self.index.nominal_layouts),
            )?;
            let inputs: Vec<_> = std::iter::once(query.subject.clone())
                .chain(query.arguments.iter().cloned())
                .collect();
            let substitution = contract
                .input_substitution(&inputs, self.context.scope(), &mut fuel)
                .map_err(|error| error.to_string())?;
            // Bound formation checks separately; selection below reuses their
            // unused work through the shared meter.
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
                let requirement = substitution
                    .instantiate_predicate_bounded(requirement, &mut fuel)
                    .map_err(|e| e.to_string())?;
                if !matches!(
                    metered(&mut fuel, allowance, |budget| self
                        .context
                        .prove_metered(Formula::from_predicate(&requirement), budget)),
                    Outcome::Entailed(_)
                ) {
                    return Err("trait proposition interface requirement is not established".into());
                }
            }
            for requirement in &contract.capabilities {
                if requirement.mentions_rigid(contract.self_parameter) {
                    continue;
                }
                let requirement = substitution
                    .instantiate_requirement_bounded(requirement, &mut fuel)
                    .map_err(|e| e.to_string())?;
                if metered(&mut fuel, allowance, |budget| {
                    self.given_in_context_metered(&requirement, budget)
                })
                .is_none()
                {
                    let (formed, _) = metered(&mut fuel, allowance, |budget| {
                        self.proposition_input_views(&TraitQuery::from(&requirement), budget)
                    })?;
                    if formed.inputs().any(|ty| ty.check_scope(&[]).is_err()) {
                        return Err(
                            "trait proposition interface capability is not established".into()
                        );
                    }
                    let resolution = metered(&mut fuel, allowance, |budget| {
                        self.resolve_instance_metered(
                            &InterfaceInstance {
                                declaration: formed.interface,
                                arguments: formed.arguments,
                            },
                            &formed.subject,
                            budget,
                        )
                    })?;
                    if !matches!(resolution.types.selection, Selection::Selected(_)) {
                        return Err(
                            "trait proposition interface capability is not established".into()
                        );
                    }
                }
            }
            let requirement = TraitRequirement {
                interface: query.interface,
                interface_arguments: query.arguments.clone(),
                subject: query.subject.clone(),
            };
            if let Some(given) = metered(&mut fuel, allowance, |budget| {
                self.given_in_context_metered(&requirement, budget)
            }) {
                return Ok(TraitDecision {
                    world: self,
                    query: query.clone(),
                    truth: TraitTruth::Established,
                    given: Some(given),
                    resolution: None,
                    input_equalities: Vec::new(),
                });
            }
            let (selected_query, input_equalities) = metered(&mut fuel, allowance, |budget| {
                self.proposition_input_views(query, budget)
            })?;
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
            // Formation and given checks have finished. Let selection use
            // their returned work; its meter preserves the caller's hard cap.
            let selection_budget = Budget {
                steps: fuel,
                ..budget
            };
            let resolution = metered(&mut fuel, selection_budget, |budget| {
                self.resolve_instance_metered(
                    &InterfaceInstance {
                        declaration: selected_query.interface,
                        arguments: selected_query.arguments.clone(),
                    },
                    &selected_query.subject,
                    budget,
                )
            })?;
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
                    if reserved(&mut fuel, allowance, |budget| {
                        self.closed_trait_inputs(&selected_query, budget)
                    }) && resolution.types.assessments.iter().all(|(id, assessment)| {
                        match assessment {
                            TypeAssessment::Excluded(_)
                            | TypeAssessment::ValueIndexMismatch { .. }
                            | TypeAssessment::ForbiddenSubject(_) => true,
                            TypeAssessment::Applicable(_) => matches!(
                                resolution.qualifications.get(id),
                                Some(Qualification::Excluded(_))
                            ),
                            _ => false,
                        }
                    }) =>
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
        })();
        remaining.steps = fuel;
        result
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
