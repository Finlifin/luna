//! Typed operation requests over already-evaluated symbolic inputs. Evaluation
//! order, program contracts, resource transfer and handler execution are later
//! obligations; no ordinary function identity is manufactured here.
use super::{Error, Instance, RowLayer, charge};
use crate::calls::Argument;
use crate::capabilities::WorldProposition;
use crate::ptc::{Budget, Formula, Outcome, UnknownReason};
use nir::effects::{Abi, Operation};
use nir::{Sort, Term};
use std::fmt;

pub struct Request<'a, 'w> {
    operation: &'a Operation,
    abi: Abi,
    arguments: &'a [Argument],
    layer: RowLayer<'a>,
    result: Term,
    proofs: Vec<WorldProposition<'a, 'w>>,
}
impl Request<'_, '_> {
    pub fn operation(&self) -> &Operation {
        self.operation
    }
    pub fn abi(&self) -> Abi {
        self.abi
    }
    pub fn arguments(&self) -> &[Argument] {
        self.arguments
    }
    pub fn allowed_index(&self) -> usize {
        self.layer.index()
    }
    pub fn result(&self) -> &Term {
        &self.result
    }
}
impl fmt::Display for Request<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(effect-request-plan {} (abi {}) (allowed-index {}) (arguments",
            self.operation,
            self.abi,
            self.allowed_index()
        )?;
        for argument in self.arguments {
            write!(
                f,
                " (argument (value {}) (type {}))",
                argument.value, argument.ty
            )?;
        }
        write!(f, ") (result {}) (argument-proofs", self.result)?;
        for proof in &self.proofs {
            write!(f, " {}", proof.outcome())?;
        }
        f.write_str("))")
    }
}
impl<'a, 'w> Instance<'a, 'w> {
    pub fn request<'s>(
        &'s self,
        arguments: &'s [Argument],
        allowed: &'s [Operation],
        layer: usize,
        budget: Budget,
    ) -> Result<Request<'s, 'w>, Error> {
        let Term::Function { parameters, .. } = self.signature() else {
            return Err(Error::InvalidInput);
        };
        if parameters.len() != arguments.len() {
            return Err(Error::ArgumentCount {
                expected: parameters.len(),
                actual: arguments.len(),
            });
        }
        let depth = budget.max_depth.min(64);
        let row_budget = Budget {
            steps: budget.steps / 3,
            max_depth: depth,
        };
        let selected = self.select_layer(allowed, layer, row_budget)?;
        let mut fuel = budget.steps - row_budget.steps;
        fuel = fuel
            .checked_sub(arguments.len())
            .ok_or(Error::BudgetExhausted)?;
        let before = fuel;
        charge(self.signature(), &mut fuel, depth)?;
        let signature_cost = before - fuel;
        let before = fuel;
        for argument in arguments {
            charge(&argument.value, &mut fuel, depth)?;
            charge(&argument.ty, &mut fuel, depth)?;
            if argument.value.sort() != Sort::Runtime || argument.ty.sort() != Sort::Type {
                return Err(Error::InvalidInput);
            }
            argument
                .value
                .check_scope(&[])
                .map_err(|_| Error::InvalidInput)?;
            argument
                .ty
                .check_scope(&[])
                .map_err(|_| Error::InvalidInput)?;
        }
        let expansion = signature_cost
            .checked_mul((before - fuel).max(1))
            .ok_or(Error::BudgetExhausted)?;
        fuel = fuel.checked_sub(expansion).ok_or(Error::BudgetExhausted)?;
        let mut expanded_fuel = expansion;
        let per_argument = Budget {
            steps: fuel / arguments.len().max(1),
            max_depth: depth,
        };
        let values: Vec<_> = arguments
            .iter()
            .map(|argument| argument.value.clone())
            .collect();
        let mut proofs = Vec::new();
        let result = self
            .signature()
            .instantiate_function(&values, |index, _, expected| {
                charge(expected, &mut expanded_fuel, depth)?;
                let proof = self
                    .world
                    .prove_proposition(
                        Formula::equal(arguments[index].ty.clone(), expected.clone()),
                        per_argument,
                    )
                    .map_err(|_| Error::InvalidInput)?;
                match proof.outcome() {
                    Outcome::Entailed(_) => proofs.push(proof),
                    Outcome::Unknown {
                        reason: UnknownReason::BudgetExhausted,
                        ..
                    } => return Err(Error::BudgetExhausted),
                    Outcome::IllFormed(_) => return Err(Error::InvalidInput),
                    _ => return Err(Error::ArgumentType { index }),
                }
                Ok(())
            })
            .map_err(|error| match error {
                nir::CallError::Contract { error, .. } => error,
                _ => Error::InvalidInput,
            })?;
        charge(&result, &mut expanded_fuel, depth)?;
        Ok(Request {
            operation: self.operation(),
            abi: self.abi(),
            arguments,
            layer: selected,
            result,
            proofs,
        })
    }
}
