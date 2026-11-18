//! Source trait and member identities are fixed before implementation checking.
//! Associated defaults complete later within the checker’s own contract view.
use super::*;

#[derive(Clone, Default)]
pub(in crate::program) struct CheckedTraits {
    // The input map orders contracts by semantic declaration identity.
    contracts: Vec<n::Trait>,
    methods: BTreeMap<DeclarationId, (usize, usize)>,
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
        let mut methods = BTreeMap::new();
        for (index, contract) in contracts.values().enumerate() {
            spend(fuel, contract.methods.len())?;
            for (slot, method) in contract.methods.iter().enumerate() {
                if methods.insert(method.declaration, (index, slot)).is_some() {
                    return Err(error(
                        Span::default(),
                        "duplicate source trait method identity",
                    ));
                }
            }
        }
        Ok(Self {
            contracts: contracts.into_values().collect(),
            methods,
        })
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
        let index = self
            .contracts
            .binary_search_by_key(&interface, |t| t.declaration)
            .expect("checked associated interface");
        let slot = self.contracts[index]
            .associated
            .iter_mut()
            .find(|member| member.declaration == contract.declaration)
            .expect("checked associated member");
        *slot = contract;
    }

    pub(in crate::program) fn method(
        &self,
        id: DeclarationId,
    ) -> Option<(&n::Trait, &n::MethodContract)> {
        let &(index, slot) = self.methods.get(&id)?;
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
