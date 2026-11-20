//! Source trait and member identities are fixed before implementation checking.
//! Associated defaults complete later within the checker’s own contract view.
use super::*;

#[derive(Clone, Copy)]
enum Member {
    Method(usize),
    Associated(usize),
}

#[derive(Clone, Default)]
pub(in crate::program) struct CheckedTraits {
    // The input map orders contracts by semantic declaration identity.
    contracts: Vec<n::Trait>,
    members: BTreeMap<DeclarationId, (usize, Member)>,
}

impl CheckedTraits {
    pub(super) fn new(
        contracts: BTreeMap<DeclarationId, n::Trait>,
        fuel: &mut u64,
    ) -> Result<Self, Error> {
        let spend = |fuel: &mut u64, count: usize| -> Result<(), Error> {
            *fuel = fuel.checked_sub(count as u64).ok_or_else(|| {
                error(
                    Span::default(),
                    "source interface directory budget exhausted",
                )
            })?;
            Ok(())
        };
        spend(fuel, contracts.len())?;
        let mut members = BTreeMap::new();
        for (index, contract) in contracts.values().enumerate() {
            spend(fuel, contract.methods.len())?;
            spend(fuel, contract.associated.len())?;
            let methods = contract
                .methods
                .iter()
                .enumerate()
                .map(|(slot, m)| (m.declaration, Member::Method(slot)));
            let associated = contract
                .associated
                .iter()
                .enumerate()
                .map(|(slot, m)| (m.declaration, Member::Associated(slot)));
            for (id, member) in methods.chain(associated) {
                if members.insert(id, (index, member)).is_some() {
                    return Err(error(
                        Span::default(),
                        "duplicate source trait member identity",
                    ));
                }
            }
        }
        Ok(Self {
            contracts: contracts.into_values().collect(),
            members,
        })
    }

    pub(in crate::program) fn member_interface(&self, id: DeclarationId) -> Option<&n::Trait> {
        let &(index, _) = self.members.get(&id)?;
        Some(&self.contracts[index])
    }

    pub(in crate::program) fn by_id(&self, id: DeclarationId) -> Option<&n::Trait> {
        let index = self
            .contracts
            .binary_search_by_key(&id, |contract| contract.declaration)
            .ok()?;
        Some(&self.contracts[index])
    }

    pub(super) fn complete_associated(
        &mut self,
        interface: DeclarationId,
        contract: n::AssociatedContract,
    ) {
        let &(index, Member::Associated(slot)) = self
            .members
            .get(&contract.declaration)
            .expect("checked associated member")
        else {
            unreachable!("checked associated member kind")
        };
        assert_eq!(self.contracts[index].declaration, interface);
        self.contracts[index].associated[slot] = contract;
    }

    pub(in crate::program) fn method(
        &self,
        id: DeclarationId,
    ) -> Option<(&n::Trait, &n::MethodContract)> {
        let &(index, Member::Method(slot)) = self.members.get(&id)? else {
            return None;
        };
        let contract = &self.contracts[index];
        Some((contract, &contract.methods[slot]))
    }
}

impl std::ops::Deref for CheckedTraits {
    type Target = [n::Trait];
    fn deref(&self) -> &Self::Target {
        &self.contracts
    }
}

impl std::ops::Index<&DeclarationId> for CheckedTraits {
    type Output = n::Trait;
    fn index(&self, id: &DeclarationId) -> &Self::Output {
        self.by_id(*id).expect("source interface has been checked")
    }
}
