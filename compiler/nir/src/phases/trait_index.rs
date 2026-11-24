//! Member positions for one immutable package. This is a phase index,
//! not implementation selection or a proof of member type compatibility.
use crate::{
    DeclarationId,
    program::{AssociatedContract, MethodContract, Package, Trait, TraitPhase},
};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Error {
    Budget(Option<crate::program::SourceRange>),
    Invalid {
        source: Option<crate::program::SourceRange>,
        message: &'static str,
    },
}

impl Error {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Budget(_) => "trait phase validation budget exhausted",
            Self::Invalid { message, .. } => message,
        }
    }

    pub(super) fn source(self) -> Option<crate::program::SourceRange> {
        match self {
            Self::Budget(source) | Self::Invalid { source, .. } => source,
        }
    }
}

fn charge(
    fuel: &mut usize,
    amount: usize,
    source: Option<crate::program::SourceRange>,
) -> Result<(), Error> {
    *fuel = fuel.checked_sub(amount).ok_or(Error::Budget(source))?;
    Ok(())
}

struct Entry {
    contract: usize,
    methods: BTreeMap<DeclarationId, usize>,
    associated: BTreeMap<DeclarationId, usize>,
}

pub(super) struct Index {
    interfaces: BTreeMap<DeclarationId, Entry>,
    methods: BTreeMap<DeclarationId, (usize, usize)>,
}

impl Index {
    pub(super) fn new(package: &Package, fuel: &mut usize) -> Result<Self, Error> {
        let invalid = |source, message| Error::Invalid { source, message };
        charge(fuel, package.traits.len(), None)?;
        let mut interfaces = BTreeMap::new();
        let mut all_methods = BTreeMap::new();
        let mut phase_maps = Vec::with_capacity(package.traits.len());
        for (contract_index, contract) in package.traits.iter().enumerate() {
            let trait_source = contract.methods.first().map(|m| m.source);
            charge(fuel, contract.methods.len(), trait_source)?;
            charge(fuel, contract.associated.len(), trait_source)?;
            if contract.comptime_only
                && (contract
                    .methods
                    .iter()
                    .any(|m| m.phase != TraitPhase::Comptime)
                    || contract
                        .associated
                        .iter()
                        .any(|m| m.phase != TraitPhase::Comptime))
            {
                let offending = contract
                    .methods
                    .iter()
                    .find(|m| m.phase != TraitPhase::Comptime)
                    .map(|m| m.source)
                    .or(trait_source);
                return Err(invalid(
                    offending,
                    "comptime-only trait contains runtime phase members",
                ));
            }
            let mut methods = BTreeMap::new();
            let mut phases = BTreeMap::new();
            for (member_index, member) in contract.methods.iter().enumerate() {
                charge(fuel, member.name.len(), Some(member.source))?;
                if methods.insert(member.declaration, member_index).is_some() {
                    return Err(invalid(
                        Some(member.source),
                        "duplicate trait method identity",
                    ));
                }
                phases.insert(member.name.as_str(), member.phase);
                // Preserve the former package-order lookup for retained method
                // references; scoped uses still require their own interface.
                all_methods
                    .entry(member.declaration)
                    .or_insert((contract_index, member_index));
            }
            let mut associated = BTreeMap::new();
            for (member_index, member) in contract.associated.iter().enumerate() {
                if associated
                    .insert(member.declaration, member_index)
                    .is_some()
                {
                    return Err(invalid(
                        trait_source,
                        "duplicate associated member identity",
                    ));
                }
            }
            if interfaces
                .insert(
                    contract.declaration,
                    Entry {
                        contract: contract_index,
                        methods,
                        associated,
                    },
                )
                .is_some()
            {
                return Err(invalid(trait_source, "duplicate trait phase identity"));
            }
            phase_maps.push(phases);
        }
        charge(fuel, package.implementations.len(), None)?;
        for definition in &package.implementations {
            let impl_source = super::implementation_entry_source(package, definition);
            let interface = definition.interface.and_then(|id| interfaces.get(&id));
            if definition.phases.runtime
                && interface.is_some_and(|entry| package.traits[entry.contract].comptime_only)
            {
                return Err(invalid(
                    impl_source,
                    "comptime-only trait has a runtime implementation",
                ));
            }
            if (!definition.phases.runtime && !definition.phases.comptime)
                || (definition.interface.is_none()
                    && definition.phases.runtime
                    && definition.phases.comptime)
            {
                return Err(invalid(
                    impl_source,
                    "invalid implementation phase availability",
                ));
            }
            if let Some(entry) = interface {
                charge(fuel, definition.methods.len(), impl_source)?;
                for (name, method_id) in &definition.methods {
                    let method_source = package
                        .functions
                        .iter()
                        .find(|f| f.declaration == *method_id)
                        .map(|f| f.source)
                        .or_else(|| {
                            package
                                .quote_templates
                                .iter()
                                .find(|f| f.declaration == *method_id)
                                .map(|f| f.source)
                        })
                        .or_else(|| {
                            package
                                .method_links
                                .iter()
                                .find(|link| link.declaration == *method_id)
                                .map(|link| link.source)
                        })
                        .or(impl_source);
                    charge(fuel, name.len(), method_source)?;
                    if phase_maps[entry.contract]
                        .get(name.as_str())
                        .is_some_and(|phase| !definition.phases.contains(*phase))
                    {
                        return Err(invalid(
                            method_source,
                            "implementation method belongs to an unavailable trait phase",
                        ));
                    }
                }
            }
        }
        Ok(Self {
            interfaces,
            methods: all_methods,
        })
    }

    pub(super) fn associated<'a>(
        &self,
        package: &'a Package,
        interface: DeclarationId,
        member: DeclarationId,
        fuel: &mut usize,
    ) -> Result<(&'a Trait, &'a AssociatedContract), Error> {
        charge(fuel, 1, None)?;
        let entry = self.interfaces.get(&interface).ok_or(Error::Invalid {
            source: None,
            message: "associated value has no interface contract",
        })?;
        let member = entry.associated.get(&member).ok_or(Error::Invalid {
            source: None,
            message: "associated value has no member contract",
        })?;
        Ok((
            &package.traits[entry.contract],
            &package.traits[entry.contract].associated[*member],
        ))
    }

    pub(super) fn method<'a>(
        &self,
        package: &'a Package,
        interface: DeclarationId,
        member: DeclarationId,
        fuel: &mut usize,
    ) -> Result<(&'a Trait, &'a MethodContract), Error> {
        charge(fuel, 1, None)?;
        let entry = self.interfaces.get(&interface).ok_or(Error::Invalid {
            source: None,
            message: "trait method has no interface contract",
        })?;
        let member = entry.methods.get(&member).ok_or(Error::Invalid {
            source: None,
            message: "trait method has no member contract",
        })?;
        Ok((
            &package.traits[entry.contract],
            &package.traits[entry.contract].methods[*member],
        ))
    }

    pub(super) fn retained_method<'a>(
        &self,
        package: &'a Package,
        member: DeclarationId,
        fuel: &mut usize,
    ) -> Result<Option<&'a MethodContract>, Error> {
        charge(fuel, 1, None)?;
        Ok(self
            .methods
            .get(&member)
            .map(|(contract, member)| &package.traits[*contract].methods[*member]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Primitive, Term};

    fn id(local: u32) -> DeclarationId {
        DeclarationId { package: 88, local }
    }

    fn interface(local: u32) -> Trait {
        Trait {
            declaration: id(local),
            self_parameter: id(local + 1000),
            name: "Interface".into(),
            comptime_only: false,
            type_parameters: vec![],
            parameter_types: vec![],
            requirements: vec![],
            capabilities: vec![],
            parents: vec![],
            methods: vec![],
            associated: vec![],
        }
    }

    fn method() -> MethodContract {
        MethodContract {
            source: crate::program::SourceRange { start: 0, end: 0 },
            phase: TraitPhase::Runtime,
            is_quote: false,
            pure: false,
            is_unsafe: false,
            declaration: id(3000),
            name: "run".into(),
            signature: Term::Function {
                parameters: vec![],
                result: Box::new(Term::Primitive(Primitive::Bool)),
            }
            .into(),
            type_arguments: vec![],
            type_parameters: vec![],
            capabilities: vec![],
            program: Default::default(),
            parameter_bindings: vec![],
            by_value_receiver: false,
            by_reference_receiver: false,
            requirements: vec![],
            default: None,
        }
    }

    #[test]
    fn lookup_cost_is_local_and_member_ownership_survives_world_changes() {
        let mut package = Package {
            traits: (0..100).map(interface).collect(),
            ..Default::default()
        };
        package.traits[0].methods.push(method());
        package.traits[0].associated.push(AssociatedContract {
            phase: TraitPhase::Comptime,
            declaration: id(3001),
            name: "value".into(),
            ty: Term::Primitive(Primitive::Bool),
            default: None,
        });
        let index = Index::new(&package, &mut 10_000).unwrap();
        let mut fuel = 1;
        assert!(std::ptr::eq(
            index
                .method(&package, id(0), id(3000), &mut fuel)
                .unwrap()
                .1,
            &package.traits[0].methods[0]
        ));
        assert_eq!(fuel, 0);
        assert!(matches!(
            index.method(&package, id(0), id(3000), &mut fuel),
            Err(Error::Budget(_))
        ));
        assert!(matches!(
            index.method(&package, id(1), id(3000), &mut 1),
            Err(Error::Invalid {
                message: "trait method has no member contract",
                ..
            })
        ));
        assert!(
            index
                .retained_method(&package, id(3000), &mut 1)
                .unwrap()
                .is_some()
        );
        assert!(
            index
                .retained_method(&package, id(9999), &mut 1)
                .unwrap()
                .is_none()
        );
        let mut changed = package.clone();
        changed.traits[0].associated[0].ty = Term::Primitive(Primitive::Char);
        let other = Index::new(&changed, &mut 10_000).unwrap();
        assert_eq!(
            index
                .associated(&package, id(0), id(3001), &mut 1)
                .unwrap()
                .1
                .ty,
            Term::Primitive(Primitive::Bool)
        );
        assert_eq!(
            other
                .associated(&changed, id(0), id(3001), &mut 1)
                .unwrap()
                .1
                .ty,
            Term::Primitive(Primitive::Char)
        );
    }

    #[test]
    fn construction_limits_names_empty_interfaces_and_duplicate_identities() {
        let mut package = Package {
            traits: vec![interface(0)],
            ..Default::default()
        };
        let mut member = method();
        member.name = "m".repeat(1024);
        package.traits[0].methods.push(member);
        let mut fuel = 8;
        assert!(matches!(
            Index::new(&package, &mut fuel),
            Err(Error::Budget(_))
        ));
        assert!(fuel > 0 && fuel < 8);
        Index::new(&package, &mut 10_000).unwrap();
        let duplicate = package.traits[0].methods[0].clone();
        package.traits[0].methods.push(duplicate);
        assert!(matches!(
            Index::new(&package, &mut 10_000),
            Err(Error::Invalid {
                message: "duplicate trait method identity",
                ..
            })
        ));
        package.traits[0].methods.pop();
        package.traits.push(interface(0));
        assert!(matches!(
            Index::new(&package, &mut 10_000),
            Err(Error::Invalid {
                message: "duplicate trait phase identity",
                ..
            })
        ));
        let empty = Package {
            traits: (0..100).map(interface).collect(),
            ..Default::default()
        };
        assert!(matches!(Index::new(&empty, &mut 99), Err(Error::Budget(_))));
        Index::new(&empty, &mut 100).unwrap();
    }
}
