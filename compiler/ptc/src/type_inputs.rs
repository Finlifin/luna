//! Explicit Type-input search. A satisfying model is only a candidate; each
//! exported binding additionally needs entailment under the supplied problem.
//! This does not authorize treating an unproved function requirement as a fact.
use super::*;
mod symbolic;

#[derive(Clone, Debug)]
pub struct TypeInputSolution {
    pub model: Witness,
    pub bindings: BTreeMap<DeclarationId, (Term, Proof)>,
}
#[derive(Clone, Debug)]
pub enum TypeInputOutcome {
    Unique(TypeInputSolution),
    Ambiguous {
        variable: DeclarationId,
        candidate: Witness,
        alternative: Witness,
    },
    Inconsistent(Proof),
    Unknown(UnknownReason),
    IllFormed(String),
}
impl Context {
    /// Solve only the explicitly requested rigid Type inputs. The constraint is
    /// the equation problem being solved, temporarily assumed only in the
    /// problem's private fork and never published as a regional fact.
    /// Returned evidence belongs to `type_input_context`, leaving `self` intact.
    ///
    /// Currently closed model reification bounds this fallback's completeness.
    /// Symbolic solutions may retain ambient inputs when their equality is
    /// proved for the whole problem. Direct unification should run first.
    pub fn search_type_inputs(
        &self,
        inputs: &[DeclarationId],
        constraint: Formula,
        budget: &mut Budget,
    ) -> TypeInputOutcome {
        let problem = match self.type_input_context(inputs, constraint, budget) {
            Ok(problem) => problem,
            Err(ValidationError::BudgetExhausted) => {
                return TypeInputOutcome::Unknown(UnknownReason::BudgetExhausted);
            }
            Err(ValidationError::IllFormed(message)) => {
                return TypeInputOutcome::IllFormed(message);
            }
        };
        let model = match problem.prove_metered(Formula::False, budget) {
            Outcome::NotEntailed(model) => model,
            Outcome::Entailed(proof) => return TypeInputOutcome::Inconsistent(proof),
            Outcome::Unknown { reason, .. } => return TypeInputOutcome::Unknown(reason),
            Outcome::IllFormed(message) => return TypeInputOutcome::IllFormed(message),
        };
        let mut bindings = BTreeMap::new();
        let mut symbolic_candidates = None;
        for &variable in inputs {
            let Some(value) = model.types.get(&variable) else {
                return TypeInputOutcome::Unknown(UnknownReason::UnvalidatedModel);
            };
            if !within_input_limits(
                Input::Term(value),
                budget.max_depth.min(256),
                &mut budget.steps,
            ) {
                return TypeInputOutcome::Unknown(UnknownReason::BudgetExhausted);
            }
            let goal = Formula::equal(type_variable(variable), value.clone());
            match problem.prove_metered(goal, budget) {
                Outcome::Entailed(proof) => {
                    bindings.insert(variable, (value.clone(), proof));
                }
                Outcome::NotEntailed(alternative) => {
                    let candidates = match &symbolic_candidates {
                        Some(candidates) => candidates,
                        None => {
                            let candidates = match symbolic::candidates(&problem, inputs, budget) {
                                Ok(candidates) => candidates,
                                Err(reason) => return TypeInputOutcome::Unknown(reason),
                            };
                            symbolic_candidates.insert(candidates)
                        }
                    };
                    match symbolic::binding(&problem, variable, candidates, budget) {
                        Ok(Some(binding)) => {
                            bindings.insert(variable, binding);
                            continue;
                        }
                        Ok(None) => {}
                        Err(reason) => return TypeInputOutcome::Unknown(reason),
                    }
                    // Two different ambient worlds do not establish ambiguity
                    // for an input relative to one fixed surrounding interface.
                    match symbolic::same_ambient(&model, &alternative, inputs, budget) {
                        Ok(true) => {}
                        Ok(false) => {
                            return TypeInputOutcome::Unknown(UnknownReason::UnvalidatedModel);
                        }
                        Err(reason) => return TypeInputOutcome::Unknown(reason),
                    }
                    return TypeInputOutcome::Ambiguous {
                        variable,
                        candidate: model,
                        alternative,
                    };
                }
                Outcome::Unknown { reason, .. } => return TypeInputOutcome::Unknown(reason),
                Outcome::IllFormed(message) => return TypeInputOutcome::IllFormed(message),
            }
        }
        TypeInputOutcome::Unique(TypeInputSolution { model, bindings })
    }

    /// Reconstruct the precise context for independently replaying input-search
    /// evidence. Reflexive markers register even unconstrained requested inputs
    /// in the model without restricting their values. Duplicate requests are an
    /// error; model values for ambient inputs are not exported as solutions.
    pub fn type_input_context(
        &self,
        inputs: &[DeclarationId],
        constraint: Formula,
        budget: &mut Budget,
    ) -> Result<Self, ValidationError> {
        self.validate_formula(&constraint, budget)?;
        budget.steps = budget.steps.checked_sub(inputs.len()).ok_or_else(|| {
            budget.steps = 0;
            ValidationError::BudgetExhausted
        })?;
        let mut seen = BTreeSet::new();
        let mut parts = vec![constraint];
        for &variable in inputs {
            if !seen.insert(variable) {
                return Err(ValidationError::IllFormed(
                    "duplicate requested Type input".into(),
                ));
            }
            parts.push(Formula::equal(
                type_variable(variable),
                type_variable(variable),
            ));
        }
        let mut context = self.fork();
        context.assume(Formula::And(parts));
        Ok(context)
    }
}
fn type_variable(declaration: DeclarationId) -> Term {
    Term::Rigid {
        declaration,
        sort: Sort::Type,
    }
}
impl fmt::Display for TypeInputOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unique(solution) => {
                f.write_str("(type-input-solution")?;
                for (variable, (value, _)) in &solution.bindings {
                    write!(f, " (binding {} {value})", type_variable(*variable))?;
                }
                f.write_str(")")
            }
            Self::Ambiguous { variable, .. } => {
                write!(f, "(ambiguous-type-input {})", type_variable(*variable))
            }
            Self::Inconsistent(_) => f.write_str("(inconsistent-type-inputs)"),
            Self::Unknown(reason) => write!(f, "(unknown-type-inputs {reason:?})"),
            Self::IllFormed(message) => write!(f, "(ill-formed-type-inputs {message:?})"),
        }
    }
}
