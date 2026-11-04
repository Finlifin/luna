//! Snapshot-local declaration positions, shared without self-referential storage.
use super::*;

pub(super) struct Declarations<'a, T> {
    declarations: &'a [T],
    pub(super) positions: Arc<BTreeMap<DeclarationId, usize>>,
}
impl<'a, T> Declarations<'a, T> {
    pub(super) fn new(
        declarations: &'a [T],
        positions: Option<Arc<BTreeMap<DeclarationId, usize>>>,
        identity: impl Fn(&T) -> DeclarationId,
    ) -> Result<Self, Error> {
        let positions = match positions {
            Some(positions) => positions,
            None => {
                let mut positions = BTreeMap::new();
                for (position, declaration) in declarations.iter().enumerate() {
                    if positions.insert(identity(declaration), position).is_some() {
                        return Err(Error {
                            kind: ErrorKind::InvalidCallPlan,
                            source: None,
                        });
                    }
                }
                Arc::new(positions)
            }
        };
        Ok(Self {
            declarations,
            positions,
        })
    }
    pub(super) fn get(&self, id: &DeclarationId) -> Option<&'a T> {
        self.positions
            .get(id)
            .map(|position| &self.declarations[*position])
    }
    pub(super) fn contains_key(&self, id: &DeclarationId) -> bool {
        self.positions.contains_key(id)
    }
}

impl<T> Default for Declarations<'_, T> {
    fn default() -> Self {
        Self {
            declarations: &[],
            positions: Arc::new(BTreeMap::new()),
        }
    }
}
pub(super) type Functions<'a> = Declarations<'a, nir::program::Function>;
pub(super) type RuntimeIdentities<'a> = Declarations<'a, nir::program::RuntimeFunctionIdentity>;

pub(super) type Effects<'a> = Declarations<'a, nir::effects::Declaration>;

/// Source indexes stay shared across invocations; published additions are local.
pub(super) struct Newtypes<'a> {
    pub(super) source: Declarations<'a, nir::newtypes::Newtype>,
    local: BTreeMap<DeclarationId, generated_newtypes::NewtypeDefinition<'a>>,
}
impl<'a> Newtypes<'a> {
    pub(super) fn new(
        declarations: &'a [nir::newtypes::Newtype],
        positions: Option<Arc<BTreeMap<DeclarationId, usize>>>,
        identity: impl Fn(&nir::newtypes::Newtype) -> DeclarationId,
    ) -> Result<Self, Error> {
        Ok(Self {
            source: Declarations::new(declarations, positions, identity)?,
            local: BTreeMap::new(),
        })
    }
    pub(super) fn get(&self, id: &DeclarationId) -> Option<&nir::newtypes::Newtype> {
        self.local
            .get(id)
            .map(|definition| &**definition)
            .or_else(|| self.source.get(id))
    }
    pub(super) fn handle(
        &self,
        id: &DeclarationId,
    ) -> Option<generated_newtypes::NewtypeDefinition<'a>> {
        self.local.get(id).cloned().or_else(|| {
            self.source
                .get(id)
                .map(generated_newtypes::NewtypeDefinition::Borrowed)
        })
    }
    pub(super) fn contains_key(&self, id: &DeclarationId) -> bool {
        self.source.contains_key(id) || self.local.contains_key(id)
    }
    pub(super) fn insert(
        &mut self,
        id: DeclarationId,
        definition: generated_newtypes::NewtypeDefinition<'a>,
    ) {
        debug_assert!(!self.contains_key(&id));
        self.local.insert(id, definition);
    }
    pub(super) fn remove(&mut self, id: &DeclarationId) {
        self.local.remove(id);
    }
}
impl Default for Newtypes<'_> {
    fn default() -> Self {
        Self {
            source: Declarations::default(),
            local: BTreeMap::new(),
        }
    }
}
pub(super) type Aliases<'a> = Declarations<'a, nir::type_aliases::Contract>;

pub(super) type NormalForms<'a> = Declarations<'a, nir::normal_forms::Head>;

/// Non-executable metadata still reserves declaration identities. This passive
/// index belongs to one immutable source world and grants no execution authority.
#[derive(Default)]
pub(super) struct ReservedDeclarations {
    identities: std::sync::OnceLock<BTreeSet<DeclarationId>>,
}

impl ReservedDeclarations {
    pub(super) fn contains(
        &self,
        package: &Package,
        id: DeclarationId,
        budget: &mut Budget,
    ) -> Result<bool, Error> {
        if let Some(identities) = self.identities.get() {
            return Ok(identities.contains(&id));
        }
        let charge = |budget: &mut Budget, amount: usize| {
            budget.steps = budget
                .steps
                .checked_sub(u64::try_from(amount).unwrap_or(u64::MAX))
                .ok_or(Error {
                    kind: ErrorKind::StepsExhausted,
                    source: None,
                })?;
            Ok::<_, Error>(())
        };
        let mut identities = BTreeSet::new();
        for interface in &package.traits {
            charge(
                budget,
                interface
                    .methods
                    .len()
                    .saturating_add(interface.associated.len())
                    .saturating_add(2),
            )?;
            identities.insert(interface.declaration);
            identities.insert(interface.self_parameter);
            identities.extend(interface.methods.iter().map(|method| method.declaration));
            identities.extend(interface.associated.iter().map(|member| member.declaration));
        }
        for implementation in &package.implementations {
            charge(budget, 1)?;
            identities.insert(implementation.declaration);
        }
        for parameters in &package.function_parameters {
            charge(budget, 1)?;
            identities.insert(parameters.declaration);
        }
        for template in &package.quote_templates {
            charge(budget, 1)?;
            identities.insert(template.declaration);
        }
        // Failed preparation never publishes a partial set or a cached absence.
        Ok(self.identities.get_or_init(|| identities).contains(&id))
    }
}

pub(super) struct SourceIndexes {
    // Published only after successful comptime preparation. Owning the exact
    // snapshot makes the phase-validation evidence immune to address reuse.
    pub(super) source: Arc<Package>,
    pub(super) reserved_declarations: Arc<ReservedDeclarations>,
    pub(super) implementations: Arc<HashMap<(InterfaceInstance, Term), DeclarationId>>,
    pub(super) methods:
        Arc<HashMap<(InterfaceInstance, Term, DeclarationId), (DeclarationId, Vec<Term>)>>,
    pub(super) associated: Arc<associated_catalog::Positions>,
    pub(super) initializers: Arc<associated_catalog::Positions>,
    pub(super) method_contracts: Arc<method_catalog::Entries>,
    pub(super) normal_forms: Arc<BTreeMap<DeclarationId, usize>>,
    pub(super) newtypes: Arc<BTreeMap<DeclarationId, usize>>,
    pub(super) aliases: Arc<BTreeMap<DeclarationId, usize>>,
    pub(super) structs: Arc<BTreeMap<DeclarationId, generated_structs::StructDefinition<'static>>>,
    pub(super) enums: Arc<BTreeMap<DeclarationId, generated_enums::EnumDefinition<'static>>>,
    pub(super) method_parameters: Arc<HashMap<(DeclarationId, DeclarationId), Vec<LocalId>>>,
    pub(super) method_origins: Arc<HashMap<DeclarationId, Option<DeclarationId>>>,
    pub(super) parameter_domains: Arc<BTreeMap<DeclarationId, Term>>,
    pub(super) parameter_sorts: Arc<BTreeMap<DeclarationId, nir::Sort>>,
    pub(super) functions: Arc<BTreeMap<DeclarationId, usize>>,
    pub(super) runtime_identities: Arc<BTreeMap<DeclarationId, usize>>,
    pub(super) effects: Arc<BTreeMap<DeclarationId, usize>>,
}

/// Shape admission needs declaration lookup, not ownership of a particular map.
pub(super) trait HeadLookup {
    fn head(&self, id: &DeclarationId) -> Option<&nir::normal_forms::Head>;
}
impl HeadLookup for NormalForms<'_> {
    fn head(&self, id: &DeclarationId) -> Option<&nir::normal_forms::Head> {
        self.get(id)
    }
}
impl HeadLookup for BTreeMap<DeclarationId, &nir::normal_forms::Head> {
    fn head(&self, id: &DeclarationId) -> Option<&nir::normal_forms::Head> {
        self.get(id).copied()
    }
}

#[cfg(test)]
mod reserved_tests;
