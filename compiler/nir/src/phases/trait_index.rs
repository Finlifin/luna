//! Borrowed member evidence for one immutable package. This is a phase index,
//! not implementation selection or a proof of member type compatibility.
use crate::{
    DeclarationId,
    program::{AssociatedContract, MethodContract, Package, Trait, TraitPhase},
};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Error {
    Budget,
    Invalid(&'static str),
}
impl Error {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Budget => "trait phase validation budget exhausted",
            Self::Invalid(message) => message,
        }
    }
}
fn charge(fuel: &mut usize, amount: usize) -> Result<(), Error> {
    *fuel = fuel.checked_sub(amount).ok_or(Error::Budget)?;
    Ok(())
}
struct Entry<'a> {
    contract: &'a Trait,
    methods: BTreeMap<DeclarationId, &'a MethodContract>,
    associated: BTreeMap<DeclarationId, &'a AssociatedContract>,
    phases: BTreeMap<&'a str, TraitPhase>,
}
pub(super) struct Index<'a> {
    interfaces: BTreeMap<DeclarationId, Entry<'a>>,
    methods: BTreeMap<DeclarationId, &'a MethodContract>,
}
impl<'a> Index<'a> {
    pub(super) fn new(package: &'a Package, fuel: &mut usize) -> Result<Self, Error> {
        use Error::Invalid;
        charge(fuel, package.traits.len())?;
        let mut interfaces = BTreeMap::new();
        let mut all_methods = BTreeMap::new();
        for contract in &package.traits {
            charge(fuel, contract.methods.len())?;
            charge(fuel, contract.associated.len())?;
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
                return Err(Invalid(
                    "comptime-only trait contains runtime phase members",
                ));
            }
            let mut methods = BTreeMap::new();
            let mut phases = BTreeMap::new();
            for member in &contract.methods {
                charge(fuel, member.name.len())?;
                if methods.insert(member.declaration, member).is_some() {
                    return Err(Invalid("duplicate trait method identity"));
                }
                phases.insert(member.name.as_str(), member.phase);
                // Preserve the former package-order lookup for retained method
                // references; scoped uses still require their own interface.
                all_methods.entry(member.declaration).or_insert(member);
            }
            let mut associated = BTreeMap::new();
            for member in &contract.associated {
                if associated.insert(member.declaration, member).is_some() {
                    return Err(Invalid("duplicate associated member identity"));
                }
            }
            if interfaces
                .insert(
                    contract.declaration,
                    Entry {
                        contract,
                        methods,
                        associated,
                        phases,
                    },
                )
                .is_some()
            {
                return Err(Invalid("duplicate trait phase identity"));
            }
        }
        charge(fuel, package.implementations.len())?;
        for definition in &package.implementations {
            let interface = definition.interface.and_then(|id| interfaces.get(&id));
            if definition.phases.runtime
                && interface.is_some_and(|entry| entry.contract.comptime_only)
            {
                return Err(Invalid("comptime-only trait has a runtime implementation"));
            }
            if (!definition.phases.runtime && !definition.phases.comptime)
                || (definition.interface.is_none() && definition.phases.comptime)
            {
                return Err(Invalid("invalid implementation phase availability"));
            }
            if let Some(entry) = interface {
                charge(fuel, definition.methods.len())?;
                for (name, _) in &definition.methods {
                    charge(fuel, name.len())?;
                    if entry
                        .phases
                        .get(name.as_str())
                        .is_some_and(|phase| !definition.phases.contains(*phase))
                    {
                        return Err(Invalid(
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
    pub(super) fn associated(
        &self,
        interface: DeclarationId,
        member: DeclarationId,
        fuel: &mut usize,
    ) -> Result<(&'a Trait, &'a AssociatedContract), Error> {
        charge(fuel, 1)?;
        let entry = self
            .interfaces
            .get(&interface)
            .ok_or(Error::Invalid("associated value has no interface contract"))?;
        let member = entry
            .associated
            .get(&member)
            .ok_or(Error::Invalid("associated value has no member contract"))?;
        Ok((entry.contract, member))
    }
    pub(super) fn method(
        &self,
        interface: DeclarationId,
        member: DeclarationId,
        fuel: &mut usize,
    ) -> Result<(&'a Trait, &'a MethodContract), Error> {
        charge(fuel, 1)?;
        let entry = self
            .interfaces
            .get(&interface)
            .ok_or(Error::Invalid("trait method has no interface contract"))?;
        let member = entry
            .methods
            .get(&member)
            .ok_or(Error::Invalid("trait method has no member contract"))?;
        Ok((entry.contract, member))
    }
    pub(super) fn retained_method(
        &self,
        member: DeclarationId,
        fuel: &mut usize,
    ) -> Result<Option<&'a MethodContract>, Error> {
        charge(fuel, 1)?;
        Ok(self.methods.get(&member).copied())
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
            index.method(id(0), id(3000), &mut fuel).unwrap().1,
            &package.traits[0].methods[0]
        ));
        assert_eq!(fuel, 0);
        assert!(matches!(
            index.method(id(0), id(3000), &mut fuel),
            Err(Error::Budget)
        ));
        assert!(matches!(
            index.method(id(1), id(3000), &mut 1),
            Err(Error::Invalid("trait method has no member contract"))
        ));
        assert!(index.retained_method(id(3000), &mut 1).unwrap().is_some());
        assert!(index.retained_method(id(9999), &mut 1).unwrap().is_none());
        let mut changed = package.clone();
        changed.traits[0].associated[0].ty = Term::Primitive(Primitive::Char);
        let other = Index::new(&changed, &mut 10_000).unwrap();
        assert_eq!(
            index.associated(id(0), id(3001), &mut 1).unwrap().1.ty,
            Term::Primitive(Primitive::Bool)
        );
        assert_eq!(
            other.associated(id(0), id(3001), &mut 1).unwrap().1.ty,
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
            Err(Error::Budget)
        ));
        assert!(fuel > 0 && fuel < 8);
        Index::new(&package, &mut 10_000).unwrap();
        let duplicate = package.traits[0].methods[0].clone();
        package.traits[0].methods.push(duplicate);
        assert!(matches!(
            Index::new(&package, &mut 10_000),
            Err(Error::Invalid("duplicate trait method identity"))
        ));
        package.traits[0].methods.pop();
        package.traits.push(interface(0));
        assert!(matches!(
            Index::new(&package, &mut 10_000),
            Err(Error::Invalid("duplicate trait phase identity"))
        ));
        let empty = Package {
            traits: (0..100).map(interface).collect(),
            ..Default::default()
        };
        assert!(matches!(Index::new(&empty, &mut 99), Err(Error::Budget)));
        Index::new(&empty, &mut 100).unwrap();
    }
}
