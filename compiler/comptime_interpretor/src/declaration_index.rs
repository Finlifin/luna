//! Snapshot-local declaration positions, shared without self-referential storage.
use super::*;

#[derive(Default)]
pub(super) struct Functions<'a> {
    declarations: &'a [nir::program::Function],
    pub(super) positions: Arc<BTreeMap<DeclarationId, usize>>,
}
impl<'a> Functions<'a> {
    pub(super) fn new(
        declarations: &'a [nir::program::Function],
        positions: Option<Arc<BTreeMap<DeclarationId, usize>>>,
    ) -> Result<Self, Error> {
        let positions = match positions {
            Some(positions) => positions,
            None => {
                let mut positions = BTreeMap::new();
                for (position, declaration) in declarations.iter().enumerate() {
                    if positions
                        .insert(declaration.declaration, position)
                        .is_some()
                    {
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
    pub(super) fn get(&self, id: &DeclarationId) -> Option<&'a nir::program::Function> {
        self.positions
            .get(id)
            .map(|position| &self.declarations[*position])
    }
    pub(super) fn contains_key(&self, id: &DeclarationId) -> bool {
        self.positions.contains_key(id)
    }
}
