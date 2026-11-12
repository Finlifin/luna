//! Immutable declaration evidence shared inside one package analysis. This
//! contains no call-site assumptions, cleanup choices or behavior certificates.
use nir::{
    DeclarationId, DeclarationParameter, Primitive, Sort, Term, program::Package,
    type_computations::Error,
};
use std::{cell::OnceCell, collections::BTreeMap, sync::Arc};

pub(crate) struct Metadata<'a> {
    package: Option<&'a Package>,
    domains: OnceCell<Arc<BTreeMap<DeclarationId, Term>>>,
    sorts: OnceCell<Arc<BTreeMap<DeclarationId, Sort>>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use nir::{Primitive, Term, type_aliases::Contract};

    fn id(local: u32) -> DeclarationId {
        DeclarationId { package: 7, local }
    }

    #[test]
    fn preparation_is_bounded_retryable_and_package_local() {
        let mut package = Package::default();
        package.alias_contracts = (0..256)
            .map(|local| Contract {
                declaration: id(local),
                parameters: vec![id(local + 1000)],
                parameter_types: vec![Term::Primitive(Primitive::Type)],
                requirements: vec![],
                capabilities: vec![],
            })
            .collect();
        let metadata = std::rc::Rc::new(Metadata::new(Some(&package)));
        let mut fuel = 32;
        assert!(metadata.binders(&[id(1000)], &mut fuel).is_err());
        assert!(fuel < 32);
        assert!(!metadata.is_prepared());
        let binders = metadata.binders(&[id(1000)], &mut 10_000).unwrap();
        assert_eq!(binders[0].sort, Sort::Type);
        let consumer = metadata.clone();
        let mut fuel = 2;
        assert_eq!(consumer.binders(&[id(1000)], &mut fuel).unwrap(), binders);
        assert_eq!(fuel, 0);
        assert!(consumer.binders(&[id(1000)], &mut 0).is_err());
        assert!(
            Metadata::new(Some(&package))
                .binders(&[id(1000)], &mut 32)
                .is_err()
        );

        let mut other = package.clone();
        other.alias_contracts[0].parameter_types[0] = Term::Primitive(Primitive::Bool);
        assert_eq!(
            Metadata::new(Some(&other))
                .binders(&[id(1000)], &mut 10_000)
                .unwrap()[0]
                .sort,
            Sort::ComptimeValue
        );
        assert_eq!(
            metadata.binders(&[id(1000)], &mut 2).unwrap()[0].sort,
            Sort::Type
        );

        other.alias_contracts[0].parameters.push(id(1000));
        other.alias_contracts[0]
            .parameter_types
            .push(Term::Primitive(Primitive::Bool));
        let invalid = Metadata::new(Some(&other));
        for _ in 0..2 {
            assert!(invalid.binders(&[id(1000)], &mut 10_000).is_err());
            assert!(!invalid.is_prepared());
        }
    }

    #[test]
    fn warm_directory_still_charges_selected_domain_payloads() {
        let domain = Term::Tuple(vec![Term::Primitive(Primitive::Bool); 256]);
        let package = Package {
            alias_contracts: vec![Contract {
                declaration: id(1),
                parameters: vec![id(2)],
                parameter_types: vec![domain.clone()],
                requirements: vec![],
                capabilities: vec![],
            }],
            ..Default::default()
        };
        let metadata = Metadata::new(Some(&package));
        metadata.domains(&mut 10_000).unwrap();
        assert_eq!(
            metadata.parameter_domains(&[id(2)], &mut 8),
            Err(Error::Budget)
        );
        let mut fuel = 10_000;
        assert_eq!(
            metadata.parameter_domains(&[id(2)], &mut fuel).unwrap(),
            vec![domain]
        );
        assert!(10_000 - fuel >= 256);
        assert_eq!(
            metadata.binders(&[id(2)], &mut 10_000).unwrap()[0].sort,
            Sort::ComptimeValue
        );
    }
}

impl<'a> Metadata<'a> {
    pub(crate) fn new(package: Option<&'a Package>) -> Self {
        Self {
            package,
            domains: OnceCell::new(),
            sorts: OnceCell::new(),
        }
    }

    pub(crate) fn package(&self) -> Option<&'a Package> {
        self.package
    }

    pub(crate) fn domains(
        &self,
        fuel: &mut usize,
    ) -> Result<&Arc<BTreeMap<DeclarationId, Term>>, Error> {
        *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
        if self.domains.get().is_none() {
            let domains = match self.package {
                Some(package) => package.declaration_parameter_domains_checked(fuel)?,
                None => BTreeMap::new(),
            };
            let _ = self.domains.set(Arc::new(domains));
        }
        Ok(self.domains.get().expect("complete declaration domains"))
    }

    pub(crate) fn parameter_domains(
        &self,
        parameters: &[DeclarationId],
        fuel: &mut usize,
    ) -> Result<Vec<Term>, Error> {
        *fuel = fuel.checked_sub(parameters.len()).ok_or(Error::Budget)?;
        if parameters.is_empty() {
            return Ok(vec![]);
        }
        let domains = self.domains(fuel)?;
        parameters
            .iter()
            .map(|id| {
                let domain = domains.get(id).unwrap_or(&Term::Primitive(Primitive::Type));
                nir::type_computations::charge_term(domain, fuel)?;
                Ok(domain.clone())
            })
            .collect()
    }

    pub(crate) fn sorts(
        &self,
        fuel: &mut usize,
    ) -> Result<&Arc<BTreeMap<DeclarationId, Sort>>, Error> {
        *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
        if self.sorts.get().is_none() {
            let domains = self.domains(fuel)?;
            *fuel = fuel.checked_sub(domains.len()).ok_or(Error::Budget)?;
            let sorts = domains
                .iter()
                .map(|(id, domain)| {
                    (
                        *id,
                        if *domain == Term::Primitive(Primitive::Type) {
                            Sort::Type
                        } else {
                            Sort::ComptimeValue
                        },
                    )
                })
                .collect();
            // Publish only a complete directory. A failed attempt remains
            // retryable and never refunds the work already performed.
            let _ = self.sorts.set(Arc::new(sorts));
        }
        Ok(self.sorts.get().expect("complete declaration directory"))
    }

    pub(crate) fn binders(
        &self,
        parameters: &[DeclarationId],
        fuel: &mut usize,
    ) -> Result<Vec<DeclarationParameter>, Error> {
        *fuel = fuel.checked_sub(parameters.len()).ok_or(Error::Budget)?;
        if parameters.is_empty() {
            return Ok(vec![]);
        }
        let sorts = self.sorts(fuel)?;
        Ok(parameters
            .iter()
            .map(|declaration| DeclarationParameter {
                declaration: *declaration,
                sort: sorts.get(declaration).copied().unwrap_or(Sort::Type),
            })
            .collect())
    }

    #[cfg(test)]
    pub(crate) fn is_prepared(&self) -> bool {
        self.sorts.get().is_some()
    }
}
