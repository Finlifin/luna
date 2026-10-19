//! Formation contracts for transparent aliases. These declarations create no
//! nominal type identity; a check merely guards evaluation of an alias body.
use crate::program::{TraitRequirement, TypePredicate};
use crate::{DeclarationId, Sort, Term};
use std::{collections::BTreeSet, fmt};

#[derive(Clone, Debug, PartialEq)]
pub struct Contract {
    pub declaration: DeclarationId,
    pub parameters: Vec<DeclarationId>,
    pub requirements: Vec<TypePredicate>,
    pub capabilities: Vec<TraitRequirement>,
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

pub fn validate(contracts: &[Contract]) -> Result<(), &'static str> {
    let mut fuel = 100_000usize;
    let mut identities = BTreeSet::new();
    for contract in contracts {
        fuel = fuel
            .checked_sub(contract.parameters.len().saturating_add(1))
            .ok_or("alias contract validation budget exhausted")?;
        if !identities.insert(contract.declaration) {
            return Err("duplicate alias contract identity");
        }
        let parameters: BTreeSet<_> = contract.parameters.iter().copied().collect();
        if parameters.len() != contract.parameters.len() {
            return Err("duplicate alias contract parameter");
        }
        let mut check = |term: &Term| -> Result<(), &'static str> {
            crate::type_computations::charge_term(term, &mut fuel)
                .map_err(|_| "alias contract validation budget exhausted")?;
            term.check_scope(&[])
                .map_err(|_| "invalid alias contract scope")?;
            if term
                .rigid_parameters()
                .map_err(|_| "invalid alias contract parameter sorts")?
                .into_iter()
                .any(|(id, sort)| sort != Sort::Type || !parameters.contains(&id))
            {
                return Err("alias contract has an undeclared parameter");
            }
            Ok(())
        };
        for capability in &contract.capabilities {
            check(&capability.subject)?;
            for argument in &capability.interface_arguments {
                check(argument)?;
            }
        }
        drop(check);
        fuel = fuel
            .checked_sub(contract.requirements.len())
            .ok_or("alias contract validation budget exhausted")?;
        let mut pending: Vec<_> = contract.requirements.iter().collect();
        while let Some(predicate) = pending.pop() {
            let mut terms = Vec::new();
            match predicate {
                TypePredicate::Bool(_) => {}
                TypePredicate::Equal(a, b) => terms.extend([a, b]),
                TypePredicate::Trait(capability) => {
                    terms.push(&capability.subject);
                    terms.extend(&capability.interface_arguments);
                }
                TypePredicate::Not(inner) => {
                    fuel = fuel
                        .checked_sub(1)
                        .ok_or("alias contract validation budget exhausted")?;
                    pending.push(inner);
                }
                TypePredicate::And(parts) | TypePredicate::Or(parts) => {
                    fuel = fuel
                        .checked_sub(parts.len())
                        .ok_or("alias contract validation budget exhausted")?;
                    pending.extend(parts);
                }
            }
            for term in terms {
                crate::type_computations::charge_term(term, &mut fuel)
                    .map_err(|_| "alias contract validation budget exhausted")?;
                term.check_scope(&[])
                    .map_err(|_| "invalid alias contract scope")?;
                if term
                    .rigid_parameters()
                    .map_err(|_| "invalid alias contract parameter sorts")?
                    .into_iter()
                    .any(|(id, sort)| sort != Sort::Type || !parameters.contains(&id))
                {
                    return Err("alias contract has an undeclared parameter");
                }
            }
        }
    }
    Ok(())
}
