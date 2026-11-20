//! Immutable declaration evidence shared inside one package analysis. This
//! contains no call-site assumptions, cleanup choices or behavior certificates.
use nir::{DeclarationId, Primitive, Sort, Term, program::Package, type_computations::Error};
use std::{borrow::Cow, cell::OnceCell, collections::BTreeMap, sync::Arc};

pub(crate) struct Metadata<'a> {
    package: Option<&'a Package>,
    domains: OnceCell<Arc<BTreeMap<DeclarationId, Term>>>,
    sorts: OnceCell<Arc<BTreeMap<DeclarationId, Sort>>>,
    function_index: OnceCell<Arc<BTreeMap<DeclarationId, usize>>>,
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
        assert!(metadata.parameter_domains(&[id(1000)], &mut fuel).is_err());
        assert!(fuel < 32);
        assert!(!metadata.domains_prepared());
        let domains = metadata
            .parameter_domains(&[id(1000)], &mut 10_000)
            .unwrap();
        assert_eq!(domains[0], Term::Primitive(Primitive::Type));
        let consumer = metadata.clone();
        let mut fuel = 3;
        assert_eq!(
            consumer.parameter_domains(&[id(1000)], &mut fuel).unwrap(),
            domains
        );
        assert_eq!(fuel, 0);
        assert!(consumer.parameter_domains(&[id(1000)], &mut 0).is_err());
        assert!(
            Metadata::new(Some(&package))
                .parameter_domains(&[id(1000)], &mut 32)
                .is_err()
        );

        let mut other = package.clone();
        other.alias_contracts[0].parameter_types[0] = Term::Primitive(Primitive::Bool);
        assert_eq!(
            Metadata::new(Some(&other))
                .parameter_domains(&[id(1000)], &mut 10_000)
                .unwrap()[0],
            Term::Primitive(Primitive::Bool)
        );
        assert_eq!(
            metadata.parameter_domains(&[id(1000)], &mut 3).unwrap()[0],
            Term::Primitive(Primitive::Type)
        );

        other.alias_contracts[0].parameters.push(id(1000));
        other.alias_contracts[0]
            .parameter_types
            .push(Term::Primitive(Primitive::Bool));
        let invalid = Metadata::new(Some(&other));
        for _ in 0..2 {
            assert!(invalid.parameter_domains(&[id(1000)], &mut 10_000).is_err());
            assert!(!invalid.domains_prepared());
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
            metadata.sorts(&mut 10_000).unwrap()[&id(2)],
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
            function_index: OnceCell::new(),
        }
    }

    pub(crate) fn package(&self) -> Option<&'a Package> {
        self.package
    }

    pub(crate) fn function_index(
        &self,
        fuel: &mut usize,
    ) -> Result<Option<Arc<BTreeMap<DeclarationId, usize>>>, &'static str> {
        let Some(package) = self.package else {
            return Ok(None);
        };
        if let Some(index) = self.function_index.get() {
            return Ok(Some(Arc::clone(index)));
        }
        let cost = package.functions.len().max(1);
        *fuel = fuel
            .checked_sub(cost)
            .ok_or("rejection summary budget exhausted")?;
        let mut definitions = BTreeMap::new();
        for (slot, function) in package.functions.iter().enumerate() {
            if definitions.insert(function.declaration, slot).is_some() {
                return Err("duplicate function identity in rejection summary");
            }
        }
        let index = Arc::new(definitions);
        let _ = self.function_index.set(Arc::clone(&index));
        Ok(Some(index))
    }

    /// A residual body without declaration inputs needs no copy or rewrite.
    /// Borrow it from its owner unless an uncanonical error row still needs
    /// instantiation normalization; consumers still pay for their analysis.
    /// This is preparation of checked code, not a replacement for NIR
    /// validation.
    pub(crate) fn specialize_function<'f>(
        &self,
        function: &'f nir::program::Function,
        arguments: &[Term],
        fuel: &mut usize,
    ) -> Result<Cow<'f, nir::program::Function>, Error> {
        *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
        if function.type_parameters.is_empty()
            && arguments.is_empty()
            && !term_has_noncanonical_error_row(&function.signature, fuel)?
            && !term_has_noncanonical_error_row(&function.body.ty, fuel)?
        {
            return Ok(Cow::Borrowed(function));
        }
        let domains = self.parameter_domains(&function.type_parameters, fuel)?;
        function
            .instantiate_declaration_parameters(&domains, arguments, fuel)
            .map(Cow::Owned)
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

    pub(crate) fn domains_prepared(&self) -> bool {
        self.domains.get().is_some()
    }
}

fn term_has_noncanonical_error_row(ty: &Term, fuel: &mut usize) -> Result<bool, Error> {
    let mut stack = vec![(ty, 0usize)];
    while let Some((term, depth)) = stack.pop() {
        *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
        if depth >= 128 {
            return Ok(true);
        }
        if let Term::ErrorResult { errors, result } = term {
            if errors.is_empty() || matches!(result.as_ref(), Term::ErrorResult { .. }) {
                return Ok(true);
            }
            if errors.len() > 1 {
                let mut keys = errors.iter().map(ToString::to_string);
                let mut prev = keys.next().unwrap();
                for key in keys {
                    *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
                    if prev >= key {
                        return Ok(true);
                    }
                    prev = key;
                }
            }
        }
        term.visit_children::<Error>(|child| {
            stack.push((child, depth + 1));
            Ok(())
        })?;
    }
    Ok(false)
}
