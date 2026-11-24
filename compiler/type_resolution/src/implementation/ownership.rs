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
            Term::TraitObject { interface, .. } if interface.package == package => {
                return Ok(Some(*interface));
            }
            Term::Nominal { arguments, .. }
            | Term::TraitObject { arguments, .. }
            | Term::Tuple(arguments)
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
            Term::ListConcat { ty, parts: fields } | Term::NominalValue { ty, fields, .. } => {
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
            Term::RuntimeField { base: inner, .. }
            | Term::Optional(inner)
            | Term::Pointer(inner)
            | Term::Lift(inner)
            | Term::ComptimeFunction(inner)
            | Term::NormalFormFunction(inner, _)
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
            | Term::EffectRowType
            | Term::ErrorRowType
            | Term::ReachabilityType
            | Term::AnyType
            | Term::ClosureContextValue(_)
            | Term::ClosureSnapshot { .. }
            | Term::ClosureValue { .. }
            | Term::Primitive(_)
            | Term::MethodValue(_)
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
            | Term::Lift(_)
            | Term::ErrorResult { .. }
            | Term::Function { .. }
            | Term::ComptimeFunction(_)
            | Term::NormalFormFunction(..)
            | Term::UnsafeFunction(_)
            | Term::Tuple(_)
            | Term::Record(_)
            | Term::Closure { .. }
            | Term::ContextualClosure { .. }
            | Term::Computation { .. }
            | Term::ClosureContextType
            | Term::EffectRowType
            | Term::ErrorRowType
            | Term::ReachabilityType
            | Term::AnyType
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::implementation::{Index, InherentHead, InherentIndex};
    use nir::normal_forms::{Head as NormalFormHead, Role};
    use nir::{Primitive, Sort};
    use std::collections::BTreeSet;

    fn decl(package: u64, local: u32) -> DeclarationId {
        DeclarationId { package, local }
    }

    fn builtin_subjects(tuple_head: DeclarationId, list_head: DeclarationId) -> Vec<Term> {
        let bool_ty = Term::Primitive(Primitive::Bool);
        let sig = Term::Function {
            parameters: vec![],
            result: Box::new(bool_ty.clone()),
        };
        vec![
            bool_ty.clone(),
            Term::Tuple(vec![bool_ty.clone()]),
            Term::Record(vec![("x".into(), bool_ty.clone())]),
            Term::Closure {
                environment: vec![bool_ty.clone()],
                signature: Box::new(sig.clone()),
            },
            Term::ContextualClosure {
                context: Box::new(Term::Rigid {
                    declaration: decl(0, 99),
                    sort: Sort::ComptimeValue,
                }),
                signature: Box::new(sig),
            },
            Term::Computation {
                effects: vec![],
                result: Box::new(bool_ty.clone()),
            },
            Term::ClosureContextType,
            Term::NormalForm {
                head: tuple_head,
                sort: Sort::Type,
                arguments: vec![Term::TupleValue(vec![bool_ty.clone()])],
            },
            Term::NormalForm {
                head: list_head,
                sort: Sort::Type,
                arguments: vec![bool_ty],
            },
        ]
    }

    #[test]
    fn builtin_packages_exempt_all_builtin_constructors_without_granting_user_packages() {
        let builtin_pkg = 0;
        let user_pkg = 7;
        let foreign_trait = decl(9, 1);
        let tuple_head = decl(builtin_pkg, 10);
        let list_head = decl(1, 11);
        let opaque_head = decl(user_pkg, 12);
        let nf_metadata = vec![
            NormalFormHead {
                collector: None,
                role: Role::Tuple,
                declaration: tuple_head,
                parameters: vec![],
                requirements: vec![],
            },
            NormalFormHead {
                collector: None,
                role: Role::TypeList,
                declaration: list_head,
                parameters: vec![],
                requirements: vec![],
            },
            NormalFormHead {
                collector: None,
                role: Role::Opaque,
                declaration: opaque_head,
                parameters: vec![],
                requirements: vec![],
            },
        ];

        let mut trait_index = Index::for_builtin_packages(None, [builtin_pkg]);
        trait_index.set_normal_form_metadata(nf_metadata.clone());
        let mut inherent_index = InherentIndex::for_builtin_packages(None, [builtin_pkg]);
        inherent_index.set_normal_form_metadata(nf_metadata);

        for (i, subject) in builtin_subjects(tuple_head, list_head)
            .into_iter()
            .enumerate()
        {
            let impl_id = decl(builtin_pkg, 100 + i as u32);
            trait_index
                .insert(
                    ImplementationHead {
                        parameter_types: vec![],
                        declaration: impl_id,
                        interface: Some(foreign_trait),
                        interface_arguments: vec![],
                        subject: subject.clone(),
                        type_parameters: vec![],
                        requirements: vec![],
                    },
                    Budget::default(),
                )
                .expect("builtin package should implement foreign trait for builtin subject");
            inherent_index
                .insert(
                    InherentHead {
                        parameter_types: vec![],
                        declaration: impl_id,
                        subject: subject.clone(),
                        type_parameters: vec![],
                        requirements: vec![],
                        members: BTreeSet::from(["m".into()]),
                    },
                    Budget::default(),
                )
                .expect("builtin package should add inherent impl for builtin subject");

            let user_impl_id = decl(user_pkg, 200 + i as u32);
            assert!(
                trait_index
                    .insert(
                        ImplementationHead {
                            parameter_types: vec![],
                            declaration: user_impl_id,
                            interface: Some(foreign_trait),
                            interface_arguments: vec![],
                            subject: subject.clone(),
                            type_parameters: vec![],
                            requirements: vec![],
                        },
                        Budget::default(),
                    )
                    .is_err()
            );
            assert!(
                inherent_index
                    .insert(
                        InherentHead {
                            parameter_types: vec![],
                            declaration: user_impl_id,
                            subject,
                            type_parameters: vec![],
                            requirements: vec![],
                            members: BTreeSet::from(["m".into()]),
                        },
                        Budget::default(),
                    )
                    .is_err()
            );
        }

        // Opaque user normal-form heads are not exempted even for builtin packages.
        let opaque_nf = Term::NormalForm {
            head: opaque_head,
            sort: Sort::Type,
            arguments: vec![],
        };
        assert!(
            trait_index
                .insert(
                    ImplementationHead {
                        parameter_types: vec![],
                        declaration: decl(builtin_pkg, 500),
                        interface: Some(foreign_trait),
                        interface_arguments: vec![],
                        subject: opaque_nf.clone(),
                        type_parameters: vec![],
                        requirements: vec![],
                    },
                    Budget::default(),
                )
                .is_err()
        );
        assert!(
            inherent_index
                .check_ownership(&opaque_nf, builtin_pkg)
                .is_err()
        );
    }
}
