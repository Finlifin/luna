//! Immutable dispatch positions with invocation-local test/host overrides.
use super::*;

type Key = (InterfaceInstance, Term, DeclarationId);
pub(super) type Positions = HashMap<Key, Position>;
#[derive(Clone, Copy)]
pub(super) struct Position {
    pub(super) dispatch: usize,
    pub(super) member: usize,
    pub(super) initializer: bool,
}
#[derive(Default)]
pub(super) struct Catalog<'a> {
    source: Option<&'a Package>,
    pub(super) positions: Arc<Positions>,
    local: HashMap<Key, &'a Expr>,
    owned: HashMap<Key, Arc<Expr>>,
}
#[cfg(test)]
impl<'a, const N: usize> From<[(Key, &'a Expr); N]> for Catalog<'a> {
    fn from(values: [(Key, &'a Expr); N]) -> Self {
        Self {
            local: HashMap::from(values),
            ..Default::default()
        }
    }
}
impl<'a> Catalog<'a> {
    pub(super) fn source(source: &'a Package) -> Self {
        Self {
            source: Some(source),
            ..Default::default()
        }
    }
    pub(super) fn indexed(source: &'a Package, positions: Arc<Positions>) -> Self {
        Self {
            source: Some(source),
            positions,
            local: HashMap::new(),
            owned: HashMap::new(),
        }
    }
    pub(super) fn get(&self, key: &Key) -> Option<&Expr> {
        self.owned
            .get(key)
            .map(Arc::as_ref)
            .or_else(|| self.get_static(key))
    }
    pub(super) fn get_body(&self, key: &Key) -> Option<invocation::Body<'a>> {
        self.owned
            .get(key)
            .map(|body| invocation::Body::Owned(body.clone()))
            .or_else(|| self.get_static(key).map(invocation::Body::Package))
    }
    pub(super) fn insert_owned(&mut self, key: Key, expression: Arc<Expr>) {
        self.owned.insert(key, expression);
    }
    pub(super) fn remove_owned(&mut self, key: &Key) {
        self.owned.remove(key);
    }
    fn get_static(&self, key: &Key) -> Option<&'a Expr> {
        if let Some(value) = self.local.get(key) {
            return Some(*value);
        }
        let position = self.positions.get(key)?;
        let instance = &self.source?.dispatch_instances[position.dispatch];
        Some(if position.initializer {
            instance.associated_initializers[position.member]
                .1
                .expression()
        } else {
            &instance.associated[position.member].1
        })
    }
    pub(super) fn insert_at(&mut self, key: Key, position: Position) -> Option<&'a Expr> {
        let previous = self.get_static(&key);
        Arc::make_mut(&mut self.positions).insert(key, position);
        previous
    }
    #[cfg(test)]
    pub(super) fn insert(&mut self, key: Key, expression: &'a Expr) -> Option<&'a Expr> {
        let previous = self.get_static(&key);
        self.local.insert(key, expression);
        previous
    }
    pub(super) fn contains_key(&self, key: &Key) -> bool {
        self.owned.contains_key(key)
            || self.local.contains_key(key)
            || self.positions.contains_key(key)
    }
    pub(super) fn is_empty(&self) -> bool {
        self.owned.is_empty() && self.local.is_empty() && self.positions.is_empty()
    }
    #[cfg(test)]
    pub(super) fn clear(&mut self) {
        self.local.clear();
        self.owned.clear();
        self.positions = Arc::new(HashMap::new());
    }
}
