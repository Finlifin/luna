use super::*;
use crate::NominalKind;
use std::collections::{BTreeMap, BTreeSet};

/// Share immutable formation evidence across forks and invocation snapshots.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FormedNullaryNominals(Arc<BTreeMap<NominalKind, BTreeSet<DeclarationId>>>);

impl FormedNullaryNominals {
    pub fn admit_checked(&mut self, definition: crate::normal_forms::NominalValueDeclaration<'_>) {
        let (kind, id) = definition.identity();
        if !definition.is_unconditional_nullary() {
            self.remove(kind, id);
        } else if !self.contains(kind, id) {
            Arc::make_mut(&mut self.0)
                .entry(kind)
                .or_default()
                .insert(id);
        }
    }

    pub fn remove(&mut self, kind: NominalKind, id: DeclarationId) {
        if self.contains(kind, id) {
            let groups = Arc::make_mut(&mut self.0);
            let ids = groups.get_mut(&kind).unwrap();
            ids.remove(&id);
            if ids.is_empty() {
                groups.remove(&kind);
            }
        }
    }

    pub fn of_kind(&self, kind: NominalKind) -> impl Iterator<Item = DeclarationId> + '_ {
        self.0.get(&kind).into_iter().flatten().copied()
    }

    pub fn kinds(&self) -> impl Iterator<Item = NominalKind> + '_ {
        self.0.keys().copied()
    }

    pub fn contains(&self, kind: NominalKind, id: DeclarationId) -> bool {
        self.0.get(&kind).is_some_and(|ids| ids.contains(&id))
    }

    pub fn contains_declaration(&self, id: DeclarationId) -> bool {
        self.0.values().any(|ids| ids.contains(&id))
    }

    pub fn len(&self) -> usize {
        self.0.values().map(|ids| ids.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Display for FormedNullaryNominals {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(formed-nullary-nominals")?;
        for (kind, ids) in self.0.iter() {
            for id in ids {
                write!(f, " ({kind} {id})")?;
            }
        }
        f.write_str(")")
    }
}
