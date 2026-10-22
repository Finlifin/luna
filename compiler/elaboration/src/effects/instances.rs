//! Formation checks for declared operation instances, before handler routing.
use super::{Error, Route, RowLayer, charge, route, select_layer};
use crate::capabilities::{World, WorldProposition};
use crate::ptc::{Budget, Formula, Outcome};
use nir::effects::{Abi, Declaration, Frame, Operation};
use nir::program::TypePredicate;
use nir::{Sort, Term};
use std::collections::BTreeSet;
use std::fmt;

/// Formation evidence remains tied to its declaration and implementation world.
/// This does not establish handler coverage, cleanup or a control-backend ABI.
pub struct Instance<'a, 'w> {
    declaration: &'a Declaration,
    pub(super) world: &'a World<'w>,
    operation: Operation,
    signature: Term,
    proofs: Vec<WorldProposition<'a, 'w>>,
}
impl Instance<'_, '_> {
    pub fn operation(&self) -> &Operation {
        &self.operation
    }
    pub fn signature(&self) -> &Term {
        &self.signature
    }
    pub fn abi(&self) -> Abi {
        self.declaration.abi
    }
    pub fn select_layer<'s>(
        &'s self,
        row: &'s [Operation],
        layer: usize,
        budget: Budget,
    ) -> Result<RowLayer<'s>, Error> {
        select_layer(row, &self.operation, layer, self.world.context, budget)
    }
    pub fn route<'s>(&'s self, frames: &'s [Frame], budget: Budget) -> Result<Route<'s>, Error> {
        route(
            &self.operation,
            self.abi(),
            frames,
            self.world.context,
            budget,
        )
    }
}
impl fmt::Display for Instance<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(effect-instance {} (signature {}) (abi {}) (formation-proofs",
            self.operation,
            self.signature,
            self.abi()
        )?;
        for proof in &self.proofs {
            write!(f, " {}", proof.outcome())?;
        }
        f.write_str("))")
    }
}

fn predicate_cost(value: &TypePredicate, fuel: &mut usize, depth: usize) -> Result<(), Error> {
    if *fuel == 0 || depth == 0 {
        return Err(Error::BudgetExhausted);
    }
    *fuel -= 1;
    match value {
        TypePredicate::Bool(_) => Ok(()),
        TypePredicate::Equal(left, right) => {
            charge(left, fuel, depth - 1)?;
            charge(right, fuel, depth - 1)
        }
        TypePredicate::Trait(requirement) => {
            charge(&requirement.subject, fuel, depth - 1)?;
            if requirement.interface_arguments.len() > *fuel {
                return Err(Error::BudgetExhausted);
            }
            for argument in &requirement.interface_arguments {
                charge(argument, fuel, depth - 1)?;
            }
            Ok(())
        }
        TypePredicate::Not(inner) => predicate_cost(inner, fuel, depth - 1),
        TypePredicate::And(values) | TypePredicate::Or(values) => {
            if values.len() > *fuel {
                return Err(Error::BudgetExhausted);
            }
            for value in values {
                predicate_cost(value, fuel, depth - 1)?;
            }
            Ok(())
        }
    }
}

pub fn instantiate<'a, 'w>(
    declaration: &'a Declaration,
    arguments: &[Term],
    world: &'a World<'w>,
    budget: Budget,
) -> Result<Instance<'a, 'w>, Error> {
    instantiate_with_domains(declaration, arguments, world, budget, |_, _, _| false)
}

pub fn instantiate_with_domains<'a, 'w>(
    declaration: &'a Declaration,
    arguments: &[Term],
    world: &'a World<'w>,
    budget: Budget,
    declared: impl FnMut(&Term, &Term, &mut usize) -> bool,
) -> Result<Instance<'a, 'w>, Error> {
    instantiate_with_evidence(
        declaration,
        arguments,
        world,
        budget,
        |_, _, _| {
            Err(nir::NominalAdmissionError::from(
                "effect value requires nominal layout evidence",
            ))
        },
        declared,
    )
}

/// Layout and symbolic-domain evidence must come from the caller's checked declarations.
pub fn instantiate_with_evidence<'a, 'w>(
    declaration: &'a Declaration,
    arguments: &[Term],
    world: &'a World<'w>,
    budget: Budget,
    layout: impl FnMut(&Term, Option<u32>, &mut usize) -> Result<Vec<Term>, nir::NominalAdmissionError>,
    declared: impl FnMut(&Term, &Term, &mut usize) -> bool,
) -> Result<Instance<'a, 'w>, Error> {
    if declaration.type_parameters.len() != arguments.len() {
        return Err(Error::InvalidInput);
    }
    let mut fuel = budget.steps;
    let depth = budget.max_depth.min(64);
    let count = arguments
        .len()
        .checked_add(declaration.requirements.len())
        .and_then(|n| n.checked_add(declaration.capabilities.len()))
        .ok_or(Error::BudgetExhausted)?;
    fuel = fuel.checked_sub(count).ok_or(Error::BudgetExhausted)?;
    for domain in &declaration.parameter_types {
        charge(domain, &mut fuel, depth)?;
    }
    let binders =
        nir::declaration_parameters(&declaration.type_parameters, &declaration.parameter_types)
            .map_err(|_| Error::InvalidInput)?;
    let before = fuel;
    for (argument, binder) in arguments.iter().zip(&binders) {
        charge(argument, &mut fuel, depth)?;
        if argument.sort() != binder.sort {
            return Err(Error::InvalidInput);
        }
        argument.check_scope(&[]).map_err(|_| Error::InvalidInput)?;
    }
    let argument_cost = before - fuel;
    nir::validate_declaration_arguments_with_evidence(
        &declaration.type_parameters,
        &declaration.parameter_types,
        arguments,
        &mut fuel,
        layout,
        declared,
    )
    .map_err(|e| {
        if matches!(e, nir::NominalAdmissionError::Budget) {
            Error::BudgetExhausted
        } else {
            Error::InvalidInput
        }
    })?;
    let before = fuel;
    charge(&declaration.signature, &mut fuel, depth)?;
    for requirement in &declaration.requirements {
        predicate_cost(requirement, &mut fuel, depth)?;
    }
    for requirement in &declaration.capabilities {
        fuel = fuel.checked_sub(1).ok_or(Error::BudgetExhausted)?;
        charge(&requirement.subject, &mut fuel, depth)?;
        for argument in &requirement.interface_arguments {
            charge(argument, &mut fuel, depth)?;
        }
    }
    // Substitution can duplicate a large argument at each rigid occurrence.
    // Reserve a conservative expanded-size allowance before recursive cloning.
    let expansion = (before - fuel)
        .checked_mul(argument_cost.max(1))
        .ok_or(Error::BudgetExhausted)?;
    fuel = fuel.checked_sub(expansion).ok_or(Error::BudgetExhausted)?;
    if declaration
        .type_parameters
        .iter()
        .collect::<BTreeSet<_>>()
        .len()
        != arguments.len()
    {
        return Err(Error::InvalidInput);
    }
    let Term::Function { parameters, .. } = &declaration.signature else {
        return Err(Error::InvalidInput);
    };
    if parameters.iter().any(|p| p.sort != Sort::Runtime)
        || (declaration.abi == Abi::DirectPassing && !parameters.is_empty())
    {
        return Err(Error::InvalidInput);
    }
    declaration
        .signature
        .check_scope(&[])
        .map_err(|_| Error::InvalidInput)?;
    let signature = declaration
        .signature
        .instantiate_parameters(&binders, arguments)
        .map_err(|_| Error::InvalidInput)?;
    let mut predicates = declaration
        .requirements
        .iter()
        .map(|predicate| {
            predicate
                .instantiate_parameters(&binders, arguments)
                .map_err(|_| Error::InvalidInput)
        })
        .collect::<Result<Vec<_>, _>>()?;
    for requirement in &declaration.capabilities {
        predicates.push(TypePredicate::Trait(
            requirement
                .instantiate_parameters(&binders, arguments)
                .map_err(|_| Error::InvalidInput)?,
        ));
    }
    // Composition may be deeper than either template or arguments alone.
    let mut expanded_fuel = expansion;
    charge(&signature, &mut expanded_fuel, depth)?;
    for predicate in &predicates {
        predicate_cost(predicate, &mut expanded_fuel, depth)?;
    }
    let allowance = Budget {
        steps: fuel / predicates.len().max(1),
        max_depth: depth,
    };
    let mut proofs = Vec::new();
    for predicate in predicates {
        let proof = world
            .prove_proposition(Formula::from_predicate(&predicate), allowance)
            .map_err(|_| Error::FormationNotEstablished)?;
        match proof.outcome() {
            Outcome::Entailed(_) => proofs.push(proof),
            Outcome::IllFormed(_) => return Err(Error::InvalidInput),
            _ => return Err(Error::FormationNotEstablished),
        }
    }
    Ok(Instance {
        declaration,
        world,
        operation: Operation {
            declaration: declaration.declaration,
            arguments: arguments.to_vec(),
        },
        signature,
        proofs,
    })
}
