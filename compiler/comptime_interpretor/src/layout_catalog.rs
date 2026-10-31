//! Immutable source layouts with invocation-local generated definitions.
use super::*;

pub(super) struct Catalog<T> {
    pub(super) base: Arc<BTreeMap<DeclarationId, T>>,
    local: BTreeMap<DeclarationId, T>,
}
impl<T> Default for Catalog<T> {
    fn default() -> Self {
        Self {
            base: Arc::new(BTreeMap::new()),
            local: BTreeMap::new(),
        }
    }
}
impl<T> Catalog<T> {
    pub(super) fn from_base(base: Arc<BTreeMap<DeclarationId, T>>) -> Self {
        Self {
            base,
            local: BTreeMap::new(),
        }
    }
    pub(super) fn get(&self, id: &DeclarationId) -> Option<&T> {
        self.local.get(id).or_else(|| self.base.get(id))
    }
    pub(super) fn contains_key(&self, id: &DeclarationId) -> bool {
        self.local.contains_key(id) || self.base.contains_key(id)
    }
    pub(super) fn remove(&mut self, id: &DeclarationId) -> Option<T> {
        self.local.remove(id)
    }
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.base.len() + self.local.len()
    }
}
impl<T: Clone> Catalog<T> {
    pub(super) fn insert(&mut self, id: DeclarationId, definition: T) -> Option<T> {
        if let Some(existing) = self.base.get(&id) {
            return Some(existing.clone());
        }
        self.local.insert(id, definition)
    }
}

impl<T> std::ops::Index<&DeclarationId> for Catalog<T> {
    type Output = T;
    fn index(&self, id: &DeclarationId) -> &T {
        self.get(id).expect("registered layout declaration")
    }
}
