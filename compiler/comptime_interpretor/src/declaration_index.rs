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

pub(super) struct SourceIndexes {
    pub(super) functions: Arc<BTreeMap<DeclarationId, usize>>,
    pub(super) runtime_identities: Arc<BTreeMap<DeclarationId, usize>>,
    pub(super) effects: Arc<BTreeMap<DeclarationId, usize>>,
}
