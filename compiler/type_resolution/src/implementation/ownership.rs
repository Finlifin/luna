//! Declaration admission uses normalized syntax, never caller assumptions or
//! implementation requires. Package IDs belong to declarations, not aliases.
use super::{Error, ImplementationHead, spend};
use crate::ptc::Budget;
use nir::{DeclarationId, Term};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ownership {
    LocalInterface(DeclarationId),
    LocalNominal(DeclarationId),
    LocalNormalForm(DeclarationId),
    UnresolvedInterface,
    BuiltinPackage(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnershipError {
    NoLocalIdentity,
    OpenBeforeLocalIdentity,
    InherentRequiresLocalNominal,
}

impl ImplementationHead {
    pub fn ownership(&self, budget: Budget) -> Result<Ownership, Error> {
        let mut fuel = budget.steps;
        self.ownership_with_fuel(&mut fuel, budget.max_depth)
    }

    pub(super) fn ownership_with_fuel(
        &self,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<Ownership, Error> {
        spend(fuel)?;
        let Some(interface) = self.interface else {
            return Ok(Ownership::UnresolvedInterface);
        };
        if interface.package == self.declaration.package {
            return Ok(Ownership::LocalInterface(interface));
        }
        // Separate roots preserve subject-before-arguments without allocating
        // an unbounded work list for the whole declaration.
        for root in std::iter::once(&self.subject).chain(&self.interface_arguments) {
            if let Some(id) = local_identity(root, self.declaration.package, fuel, depth)? {
                return Ok(Ownership::LocalNominal(id));
            }
        }
        Err(Error::Ownership(OwnershipError::NoLocalIdentity))
    }
}

/// Default strategy registration observes the trait head and its arguments;
/// the eventual derived subject cannot grant registration ownership.
pub fn default_strategy(
    declaration: DeclarationId,
    interface: DeclarationId,
    arguments: &[Term],
    budget: Budget,
) -> Result<Ownership, Error> {
    let mut fuel = budget.steps;
    spend(&mut fuel)?;
    if interface.package == declaration.package {
        return Ok(Ownership::LocalInterface(interface));
    }
    for argument in arguments {
        if let Some(identity) =
            local_identity(argument, declaration.package, &mut fuel, budget.max_depth)?
        {
            return Ok(Ownership::LocalNominal(identity));
        }
    }
    Err(Error::Ownership(OwnershipError::NoLocalIdentity))
}

/// Inherent implementations require the subject's own declared head; a
/// local argument inside a foreign wrapper does not own that wrapper.
pub fn inherent(subject: &Term, package: u64) -> Result<Ownership, Error> {
    match subject {
        Term::Nominal { declaration, .. } if declaration.package == package => {
            Ok(Ownership::LocalNominal(*declaration))
        }
        Term::NormalForm {
            head,
            sort: nir::Sort::Type,
            ..
        } if head.package == package => Ok(Ownership::LocalNormalForm(*head)),
        _ => Err(Error::Ownership(
            OwnershipError::InherentRequiresLocalNominal,
        )),
    }
}

fn local_identity(
    root: &Term,
    package: u64,
    fuel: &mut usize,
    limit: usize,
) -> Result<Option<DeclarationId>, Error> {
    let mut work = vec![(root, 0)];
    while let Some((term, depth)) = work.pop() {
        spend(fuel)?;
        if depth >= limit.min(128) {
            return Err(Error::BudgetExhausted);
        }
        let children: &[Term] = match term {
            Term::Nominal { declaration, .. } if declaration.package == package => {
                return Ok(Some(*declaration));
            }
            Term::Nominal { arguments, .. }
            | Term::Tuple(arguments)
            | Term::TypeListValue(arguments)
            | Term::TypeListConcat(arguments)
            | Term::TupleValue(arguments) => arguments,
            Term::OptionalValue { ty, value } => {
                work.push((ty, depth + 1));
                if let Some(value) = value {
                    work.push((value, depth + 1));
                }
                continue;
            }
            Term::ErrorValue { ty, label, value } => {
                work.push((ty, depth + 1));
                if let Some(label) = label {
                    work.push((label, depth + 1));
                }
                work.push((value, depth + 1));
                continue;
            }
            Term::NominalValue { ty, fields, .. } => {
                if fields.len().saturating_add(1) > *fuel {
                    return Err(Error::BudgetExhausted);
                }
                work.push((ty, depth + 1));
                work.extend(fields.iter().map(|field| (field, depth + 1)));
                continue;
            }
            Term::Record(fields) | Term::RecordValue(fields) => {
                if fields.len() > *fuel {
                    return Err(Error::BudgetExhausted);
                }
                work.extend(fields.iter().rev().map(|(_, ty)| (ty, depth + 1)));
                continue;
            }
            Term::Optional(inner)
            | Term::Pointer(inner)
            | Term::ComptimeFunction(inner)
            | Term::UnsafeFunction(inner) => {
                work.push((inner, depth + 1));
                continue;
            }
            Term::Function { parameters, result } => {
                if parameters.len().saturating_add(1) > *fuel {
                    return Err(Error::BudgetExhausted);
                }
                work.push((result, depth + 1));
                work.extend(parameters.iter().rev().map(|p| (&p.ty, depth + 1)));
                continue;
            }
            // Opaque applications expose neither the resulting nominal head
            // nor ownership through their inputs. Never unfold a local factory.
            Term::ErrorResult { .. }
            | Term::Computation { .. }
            | Term::NormalForm { .. }
            | Term::AssociatedType { .. }
            | Term::Rigid { .. }
            | Term::Bound { .. }
            | Term::Hole { .. }
            | Term::Closure { .. }
            | Term::ContextualClosure { .. } => {
                return Err(Error::Ownership(OwnershipError::OpenBeforeLocalIdentity));
            }
            Term::ClosureContextType
            | Term::ClosureContextValue(_)
            | Term::ClosureSnapshot { .. }
            | Term::Primitive(_)
            | Term::FunctionValue { .. }
            | Term::Unit
            | Term::Bool(_)
            | Term::Float(_)
            | Term::Integer(_)
            | Term::BitVec(_)
            | Term::Float16(_)
            | Term::BFloat16(_)
            | Term::Float32(_)
            | Term::Float64(_)
            | Term::Char(_)
            | Term::String(_)
            | Term::Symbol(_)
            | Term::Runtime(_)
            | Term::Value { .. } => continue,
        };
        if children.len() > *fuel {
            return Err(Error::BudgetExhausted);
        }
        work.extend(children.iter().rev().map(|child| (child, depth + 1)));
    }
    Ok(None)
}

impl fmt::Display for Ownership {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LocalInterface(id) => write!(f, "(local-interface {id})"),
            Self::LocalNominal(id) => write!(f, "(local-nominal {id})"),
            Self::LocalNormalForm(id) => write!(f, "(local-normal-form {id})"),
            Self::BuiltinPackage(id) => write!(f, "(builtin-package {id})"),
            Self::UnresolvedInterface => write!(f, "(unresolved-interface)"),
        }
    }
}

/// The driver grants this capability by semantic package identity. Looking at
/// an alias, a namespace spelling, or a local type argument cannot grant it.
pub(super) fn builtin_subject(subject: &Term) -> bool {
    matches!(
        subject,
        Term::Primitive(_)
            | Term::Pointer(_)
            | Term::Optional(_)
            | Term::ErrorResult { .. }
            | Term::Function { .. }
            | Term::ComptimeFunction(_)
            | Term::UnsafeFunction(_)
    )
}
