//! Formation contracts for transparent aliases. These declarations create no
//! nominal type identity; a check merely guards evaluation of an alias body.
use crate::program::{TraitRequirement, TypePredicate};
use crate::{DeclarationId, DeclarationParameter, Term, TypeError};
use std::{collections::BTreeSet, fmt};

#[derive(Clone, Debug, PartialEq)]
pub struct Contract {
    pub declaration: DeclarationId,
    pub parameters: Vec<DeclarationId>,
    /// Empty preserves the all-Type telescope; otherwise one domain per binder.
    pub parameter_types: Vec<Term>,
    pub requirements: Vec<TypePredicate>,
    pub capabilities: Vec<TraitRequirement>,
}
impl Contract {
    pub fn parameter_binders(&self) -> Result<Vec<DeclarationParameter>, TypeError> {
        crate::nominal_parameters::binders(&self.parameters, &self.parameter_types)
    }
}
impl fmt::Display for Contract {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(type-alias-contract (declaration {}) (parameters",
            self.declaration
        )?;
        for parameter in &self.parameters {
            write!(f, " {parameter}")?;
        }
        f.write_str(") (domains")?;
        for domain in &self.parameter_types {
            write!(f, " {domain}")?;
        }
        f.write_str(") (requires")?;
        for requirement in &self.requirements {
            write!(f, " {requirement}")?;
        }
        f.write_str(") (capabilities")?;
        for capability in &self.capabilities {
            write!(f, " {capability}")?;
        }
        f.write_str("))")
    }
}

/// Resource failure does not establish that a formation contract is invalid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidationError {
    Budget,
    Invalid(&'static str),
}
impl ValidationError {
    pub fn message(self) -> &'static str {
        match self {
            Self::Budget => "alias contract validation budget exhausted",
            Self::Invalid(message) => message,
        }
    }
}
impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}
impl std::error::Error for ValidationError {}

/// Compatibility entry for callers that do not yet own a shared allowance.
pub fn validate(contracts: &[Contract]) -> Result<(), &'static str> {
    validate_in(contracts, &mut 100_000).map_err(ValidationError::message)
}

/// Validate formation evidence using the caller's allowance, including failure.
pub fn validate_in(contracts: &[Contract], fuel: &mut usize) -> Result<(), ValidationError> {
    use ValidationError::Invalid;
    charge(fuel, contracts.len())?;
    let mut identities = BTreeSet::new();
    for contract in contracts {
        charge(fuel, contract.parameters.len())?;
        charge(fuel, contract.parameter_types.len())?;
        if !identities.insert(contract.declaration) {
            return Err(Invalid("duplicate alias contract identity"));
        }
        let parameters: BTreeSet<_> = contract.parameters.iter().copied().collect();
        if parameters.len() != contract.parameters.len() {
            return Err(Invalid("duplicate alias contract parameter"));
        }
        for domain in &contract.parameter_types {
            charge_term(domain, fuel)?;
        }
        let binders = contract
            .parameter_binders()
            .map_err(|_| Invalid("invalid alias contract parameter domains"))?;
        let positions: std::collections::BTreeMap<_, _> = binders
            .iter()
            .enumerate()
            .map(|(index, binder)| (binder.declaration, (binder.sort, index)))
            .collect();
        // Domains form a telescope: only earlier binders are in scope.
        for (index, domain) in contract.parameter_types.iter().enumerate() {
            if domain
                .rigid_parameters()
                .map_err(|_| Invalid("invalid alias contract parameter sorts"))?
                .into_iter()
                .any(|(id, sort)| {
                    !positions
                        .get(&id)
                        .is_some_and(|(expected, position)| *expected == sort && *position < index)
                })
            {
                return Err(Invalid(
                    "alias contract domain has an unavailable parameter",
                ));
            }
        }
        enum Work<'a> {
            Term(&'a Term),
            Predicate(&'a TypePredicate),
            Capability(&'a TraitRequirement),
        }
        charge(fuel, contract.capabilities.len())?;
        charge(fuel, contract.requirements.len())?;
        let mut pending: Vec<_> = contract
            .capabilities
            .iter()
            .map(Work::Capability)
            .chain(contract.requirements.iter().map(Work::Predicate))
            .collect();
        while let Some(node) = pending.pop() {
            match node {
                Work::Term(term) => {
                    charge_term(term, fuel)?;
                    term.check_scope(&[])
                        .map_err(|_| Invalid("invalid alias contract scope"))?;
                    if term
                        .rigid_parameters()
                        .map_err(|_| Invalid("invalid alias contract parameter sorts"))?
                        .into_iter()
                        .any(|(id, sort)| {
                            !positions
                                .get(&id)
                                .is_some_and(|(expected, _)| *expected == sort)
                        })
                    {
                        return Err(Invalid("alias contract has an undeclared parameter"));
                    }
                }
                Work::Capability(capability) => {
                    charge(fuel, capability.interface_arguments.len().saturating_add(1))?;
                    pending.push(Work::Term(&capability.subject));
                    pending.extend(capability.interface_arguments.iter().map(Work::Term));
                }
                Work::Predicate(predicate) => match predicate {
                    TypePredicate::Bool(_) => {}
                    TypePredicate::Equal(a, b) => {
                        charge(fuel, 2)?;
                        pending.extend([Work::Term(a), Work::Term(b)]);
                    }
                    TypePredicate::Trait(capability) => {
                        charge(fuel, 1)?;
                        pending.push(Work::Capability(capability));
                    }
                    TypePredicate::Not(inner) => {
                        charge(fuel, 1)?;
                        pending.push(Work::Predicate(inner));
                    }
                    TypePredicate::And(parts) | TypePredicate::Or(parts) => {
                        charge(fuel, parts.len())?;
                        pending.extend(parts.iter().map(Work::Predicate));
                    }
                },
            }
        }
    }
    Ok(())
}

fn charge(fuel: &mut usize, amount: usize) -> Result<(), ValidationError> {
    *fuel = fuel.checked_sub(amount).ok_or(ValidationError::Budget)?;
    Ok(())
}
fn charge_term(term: &Term, fuel: &mut usize) -> Result<(), ValidationError> {
    crate::type_computations::charge_term(term, fuel).map_err(|_| ValidationError::Budget)
}
