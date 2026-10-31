//! Explicit model-search proposals remain isolated until original obligations
//! are proved. Candidate requirements never become model-search assumptions.
use super::*;

pub enum TypeInputSearch {
    Proposed {
        candidate: Engine,
        obligation: ObligationId,
        evidence: ptc::TypeInputSolution,
    },
    Unresolved(ptc::TypeInputOutcome),
}
impl Engine {
    /// Fallback after direct matching, for explicitly requested unassigned Type
    /// variables. This never searches the engine's requires to infer inputs.
    /// A proposal must still advance and commit through the normal transaction
    /// API; the original problem and every pre-existing obligation remain live.
    pub fn search_type_inputs(
        &self,
        inputs: &[MetaId],
        problem: &Requirement,
        budget: &mut Budget,
    ) -> Result<TypeInputSearch, Error> {
        let mut seen = BTreeSet::new();
        for &input in inputs {
            charge(budget)?;
            if !seen.insert(input) || self.inference.is_assigned(input)? {
                return Err(InferenceError::AlreadyAssigned(input).into());
            }
            let actual = self.inference.sort(&InferenceTerm::Meta(input))?;
            if actual != Sort::Type {
                return Err(InferenceError::WrongSort {
                    expected: Sort::Type,
                    actual,
                }
                .into());
            }
        }
        // Internal model variables must not capture any existing declaration
        // input, including those nested in binders or retained assignments.
        let mut used = existing_inputs(self, problem, budget)?;
        let mut encoded = self.inference.trial();
        let mut identities = Vec::new();
        for &input in inputs {
            charge(budget)?;
            let mut identity = DeclarationId {
                package: u64::MAX,
                local: u32::try_from(identities.len()).map_err(|_| InferenceError::Budget)?,
            };
            // Allocation checks existing inputs instead of reserving a magic
            // package namespace that source declarations might also occupy.
            while used.contains(&identity) {
                charge(budget)?;
                identity.package = identity
                    .package
                    .checked_sub(1)
                    .ok_or(InferenceError::Budget)?;
            }
            used.insert(identity);
            identities.push(identity);
            encoded.assign_with_limits(
                input,
                InferenceTerm::Stable(Term::Rigid {
                    declaration: identity,
                    sort: Sort::Type,
                }),
                &mut budget.steps,
                budget.max_depth,
            )?;
        }
        let formula = problem.freeze_metered(
            &encoded,
            &self.scope,
            self.context.scope() == scope_frames(&self.scope).as_slice(),
            budget,
            0,
        )?;
        let outcome = self
            .context
            .search_type_inputs(&identities, formula, budget);
        let ptc::TypeInputOutcome::Unique(evidence) = outcome else {
            return Ok(TypeInputSearch::Unresolved(outcome));
        };
        let mut candidate = self.trial();
        for (&input, identity) in inputs.iter().zip(&identities) {
            let value = &evidence.bindings[identity].0;
            nir::type_computations::charge_term_with_depth(
                value,
                &mut budget.steps,
                budget.max_depth,
            )
            .map_err(|_| InferenceError::Budget)?;
            candidate.inference.assign_with_limits(
                input,
                InferenceTerm::Stable(value.clone()),
                &mut budget.steps,
                budget.max_depth,
            )?;
        }
        for obligation in candidate.obligations.values() {
            charge(budget)?;
            if let State::Waiting(dependencies) = &obligation.state {
                budget.steps = budget
                    .steps
                    .checked_sub(dependencies.len())
                    .ok_or(InferenceError::Budget)?;
            }
        }
        candidate.invalidate();
        // Search used a temporary equation problem. Re-prove the substituted
        // original in the actual region before allowing this candidate to commit.
        let obligation = candidate.require(problem.clone());
        Ok(TypeInputSearch::Proposed {
            candidate,
            obligation,
            evidence,
        })
    }
}
fn charge(budget: &mut Budget) -> Result<(), InferenceError> {
    budget.steps = budget.steps.checked_sub(1).ok_or(InferenceError::Budget)?;
    Ok(())
}

enum Input<'a> {
    Requirement(&'a Requirement),
    Formula(&'a Formula),
    Inference(&'a InferenceTerm),
    Term(&'a Term),
}
fn existing_inputs(
    engine: &Engine,
    problem: &Requirement,
    budget: &mut Budget,
) -> Result<BTreeSet<DeclarationId>, InferenceError> {
    let mut used = BTreeSet::new();
    let mut pending = vec![Input::Requirement(problem)];
    for fact in engine.context.assumptions() {
        charge(budget)?;
        pending.push(Input::Formula(fact));
    }
    for variable in engine.inference.variables.values() {
        charge(budget)?;
        if let Some(value) = &variable.value {
            pending.push(Input::Inference(value));
        }
    }
    for (_, domain) in engine.scope.iter() {
        charge(budget)?;
        if let Some(domain) = domain {
            pending.push(Input::Term(domain));
        }
    }
    while let Some(input) = pending.pop() {
        charge(budget)?;
        match input {
            Input::Requirement(requirement) => match requirement {
                Requirement::Stable(formula) => pending.push(Input::Formula(formula)),
                Requirement::Equal(a, b) => {
                    pending.extend([Input::Inference(a), Input::Inference(b)])
                }
                Requirement::Kind(ty, _) => pending.push(Input::Inference(ty)),
                Requirement::Not(inner) => pending.push(Input::Requirement(inner)),
                Requirement::And(parts) | Requirement::Or(parts) => {
                    for part in parts {
                        charge(budget)?;
                        pending.push(Input::Requirement(part));
                    }
                }
            },
            Input::Formula(formula) => match formula {
                Formula::Equal(ptc::Equality(a, b)) => {
                    pending.extend([Input::Term(a), Input::Term(b)])
                }
                Formula::Kind(query) => pending.push(Input::Term(&query.ty)),
                Formula::Trait(query) => {
                    for term in query.inputs() {
                        charge(budget)?;
                        pending.push(Input::Term(term));
                    }
                }
                Formula::IntLe(query) => {
                    used.extend(query.left.variable);
                    used.extend(query.right.variable);
                }
                Formula::Not(inner) => pending.push(Input::Formula(inner)),
                Formula::And(parts) | Formula::Or(parts) => {
                    for part in parts {
                        charge(budget)?;
                        pending.push(Input::Formula(part));
                    }
                }
                Formula::True | Formula::False => {}
            },
            Input::Inference(term) => match term {
                InferenceTerm::Meta(_) => {}
                InferenceTerm::Stable(term) => pending.push(Input::Term(term)),
                InferenceTerm::Instantiate {
                    template,
                    arguments,
                } => {
                    pending.push(Input::Term(template));
                    for argument in arguments {
                        charge(budget)?;
                        pending.push(Input::Inference(argument));
                    }
                }
            },
            Input::Term(term) => {
                nir::type_computations::charge_term_with_depth(
                    term,
                    &mut budget.steps,
                    budget.max_depth,
                )
                .map_err(|_| InferenceError::Budget)?;
                used.extend(
                    term.rigid_parameters()
                        .map_err(InferenceError::InvalidSubstitution)?
                        .into_keys(),
                );
            }
        }
    }
    Ok(used)
}

impl fmt::Display for TypeInputSearch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Proposed {
                candidate,
                obligation,
                ..
            } => write!(
                f,
                "(type-input-proposal (obligation {}) {candidate})",
                obligation.0
            ),
            Self::Unresolved(outcome) => write!(f, "(type-input-search {outcome})"),
        }
    }
}

impl Engine {
    /// Use only original matching equations, never the candidate's requires.
    /// Model search is optional work after structural matching reaches a fixed
    /// point. A bounded share leaves fuel for the caller's remaining checks.
    pub(super) fn try_model_matching(
        &mut self,
        relations: &[(InferenceTerm, InferenceTerm)],
        budget: &mut Budget,
    ) -> Result<(), crate::matching::MatchError> {
        let mut local = Budget {
            steps: budget.steps / 4,
            ..*budget
        };
        let before = local.steps;
        let result = (|| -> Result<(), Error> {
            let mut inputs = BTreeSet::new();
            for (left, right) in relations {
                for term in [left, right] {
                    inputs.extend(self.inference.unresolved_with_limits(
                        term,
                        &mut local.steps,
                        local.max_depth,
                    )?);
                }
            }
            if inputs.is_empty() {
                return Ok(());
            }
            let mut equations = Vec::new();
            for (left, right) in relations {
                let left =
                    crate::inference_budget::copy_input(left, &mut local.steps, local.max_depth)
                        .map_err(|_| InferenceError::Budget)?;
                let right =
                    crate::inference_budget::copy_input(right, &mut local.steps, local.max_depth)
                        .map_err(|_| InferenceError::Budget)?;
                equations.push(Requirement::Equal(left, right));
            }
            let problem = Requirement::And(equations);
            for &input in &inputs {
                charge(&mut local)?;
                if self.inference.sort(&InferenceTerm::Meta(input))? != Sort::Type {
                    return Ok(());
                }
            }
            let inputs: Vec<_> = inputs.into_iter().collect();
            let TypeInputSearch::Proposed { candidate, .. } =
                self.search_type_inputs(&inputs, &problem, &mut local)?
            else {
                return Ok(());
            };
            // Preserve this trial's parent/revision. The search's nested trial
            // cannot be returned directly to our caller as its own child.
            let mut proposed = self.inference.trial();
            for input in inputs {
                let value =
                    candidate.inspect_type_metered(&InferenceTerm::Meta(input), &mut local)?;
                proposed.assign_with_limits(
                    input,
                    InferenceTerm::Stable(value),
                    &mut local.steps,
                    local.max_depth,
                )?;
            }
            self.inference.commit(proposed)?;
            Ok(())
        })();
        budget.steps -= before - local.steps;
        match result {
            Ok(()) | Err(Error::Inference(InferenceError::Budget)) => Ok(()),
            Err(Error::Inference(error)) => Err(crate::matching::MatchError::Inference(error)),
            // Other transaction errors cannot authorize any assignment. Keep the
            // original equations pending just as for an unavailable model.
            Err(_) => Ok(()),
        }
    }
}
