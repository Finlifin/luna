//! Parameter domains belong to one immutable storage inspection world.
use super::{Package, StorageError, Term};
use crate::DeclarationId;
use std::{cell::OnceCell, collections::BTreeMap};

pub(super) struct Domains<'a> {
    package: &'a Package,
    borrowed: Option<&'a BTreeMap<DeclarationId, Term>>,
    prepared: OnceCell<BTreeMap<DeclarationId, Term>>,
}

impl<'a> Domains<'a> {
    pub(super) fn new(package: &'a Package) -> Self {
        Self {
            package,
            borrowed: None,
            prepared: OnceCell::new(),
        }
    }

    pub(super) fn admitted(
        package: &'a Package,
        declarations: &'a super::declarations::Declarations,
    ) -> Self {
        Self {
            borrowed: Some(&declarations.parameter_domains),
            ..Self::new(package)
        }
    }

    pub(super) fn get(
        &self,
        declaration: &DeclarationId,
        fuel: &mut usize,
    ) -> Result<Option<&Term>, StorageError> {
        *fuel = fuel.checked_sub(1).ok_or(StorageError::Budget)?;
        if let Some(domains) = self.borrowed {
            return Ok(domains.get(declaration));
        }
        if self.prepared.get().is_none() {
            let domains = self.package.declaration_parameter_domains_checked(fuel)?;
            // Publish only the complete table. A failed query can retry with
            // new fuel, but never observes a partial declaration environment.
            let _ = self.prepared.set(domains);
        }
        Ok(self
            .prepared
            .get()
            .expect("completed domain table")
            .get(declaration))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Primitive, Sort, function_parameters::Declaration};

    fn id(local: u32) -> DeclarationId {
        DeclarationId { package: 42, local }
    }

    fn package() -> Package {
        Package {
            function_parameters: (0..256)
                .map(|index| Declaration {
                    declaration: id(index * 2),
                    parameters: vec![id(index * 2 + 1)],
                    domains: vec![Term::ClosureContextType],
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn incomplete_domain_preparation_is_retryable_and_complete_queries_stay_metered() {
        let package = package();
        let domains = Domains::new(&package);
        let mut fuel = 8;
        assert_eq!(domains.get(&id(1), &mut fuel), Err(StorageError::Budget));
        assert!(fuel < 8);
        assert!(domains.prepared.get().is_none());
        assert_eq!(
            domains.get(&id(1), &mut 100_000),
            Ok(Some(&Term::ClosureContextType))
        );
        for key in [id(1), id(511)] {
            let mut fuel = 1;
            assert_eq!(
                domains.get(&key, &mut fuel),
                Ok(Some(&Term::ClosureContextType))
            );
            assert_eq!(fuel, 0);
            assert_eq!(domains.get(&key, &mut fuel), Err(StorageError::Budget));
        }
        assert_eq!(domains.get(&id(9999), &mut 1), Ok(None));
    }

    #[test]
    fn conflicting_domains_never_publish_a_partial_table() {
        let mut package = package();
        package.function_parameters.push(Declaration {
            declaration: id(999),
            parameters: vec![id(1)],
            domains: vec![Term::Primitive(Primitive::Bool)],
        });
        let domains = Domains::new(&package);
        for _ in 0..2 {
            assert!(matches!(
                domains.get(&id(1), &mut 100_000),
                Err(StorageError::Invalid(_))
            ));
            assert!(domains.prepared.get().is_none());
        }
    }

    #[test]
    fn storage_context_evidence_cannot_cross_worlds_with_the_same_ids() {
        let a = package();
        let mut b = a.clone();
        b.function_parameters[0].domains[0] = Term::Primitive(Primitive::Bool);
        let closure = Term::ContextualClosure {
            context: Box::new(Term::Rigid {
                declaration: id(1),
                sort: Sort::ComptimeValue,
            }),
            signature: Box::new(Term::Function {
                parameters: vec![],
                result: Box::new(Term::Primitive(Primitive::Bool)),
            }),
        };
        let mut first = super::super::storage_cache::StorageQueries::new(&a);
        first.check_runtime(&closure, &[], &mut 100_000).unwrap();
        let mut second = super::super::storage_cache::StorageQueries::new(&b);
        assert!(matches!(
            second.check_runtime(&closure, &[], &mut 100_000),
            Err(StorageError::Invalid(_))
        ));
        first.check_runtime(&closure, &[], &mut 100).unwrap();
        assert!(matches!(
            first.check_comptime(&closure, &[], &mut 100),
            Ok(())
        ));
    }
}
