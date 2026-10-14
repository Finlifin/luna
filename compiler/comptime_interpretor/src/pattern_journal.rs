//! Candidate-local values and their runtime subject positions are one transaction.
use super::*;

#[derive(Default)]
pub(super) struct PatternBindings(pub BTreeMap<LocalId, Option<Vec<usize>>>, pub BTreeMap<LocalId, usize>);

impl fmt::Display for PatternBindings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(pattern-bindings")?;
        for (binding, origin) in &self.0 {
            write!(f, " (binding {binding} (rank {}) ", self.1[binding])?;
            if let Some(path) = origin {
                f.write_str("(subject")?;
                for field in path { write!(f, " {field}")?; }
                f.write_str(")")?;
            } else { f.write_str("(derived)")?; }
            f.write_str(")")?;
        }
        f.write_str(")")
    }
}

pub(super) struct PatternJournal {
    pub changes: Vec<(LocalId, Option<Value>, Option<Option<Vec<usize>>>, Option<usize>)>,
    pub bindings: PatternBindings,
    pub path: Option<Vec<usize>>,
}
impl PatternJournal {
    pub fn new() -> Self {
        Self { changes: vec![], bindings: PatternBindings::default(), path: Some(vec![]) }
    }
    pub fn len(&self) -> usize { self.changes.len() }
    pub fn insert(&mut self, id: LocalId, value: Value, path: Option<Vec<usize>>, env: &mut Environment) {
        self.insert_ranked(id, value, path, self.changes.len(), env);
    }
    pub fn insert_ranked(&mut self, id: LocalId, value: Value, path: Option<Vec<usize>>, rank: usize, env: &mut Environment) {
        self.changes.push((id, env.insert(id, value), self.bindings.0.insert(id, path), self.bindings.1.insert(id, rank)));
    }
    pub fn remove(&mut self, id: LocalId, env: &mut Environment) {
        self.changes.push((id, env.remove(&id), self.bindings.0.remove(&id), self.bindings.1.remove(&id)));
    }
    pub fn rollback(&mut self, mark: usize, env: &mut Environment) {
        while self.changes.len() > mark {
            let (id, value, origin, rank) = self.changes.pop().unwrap();
            if let Some(rank) = rank {self.bindings.1.insert(id, rank);} else {self.bindings.1.remove(&id);}
            if let Some(value) = value { env.insert(id, value); } else { env.remove(&id); }
            if let Some(origin) = origin { self.bindings.0.insert(id, origin); } else { self.bindings.0.remove(&id); }
        }
    }
}

impl Interpreter<'_> {
    pub(super) fn match_pattern_field(
        &mut self, pattern: &nir::program::Pattern, value: &Value,
        environment: &mut Environment, journal: &mut PatternJournal, field: Option<usize>,
    ) -> Result<Flow, Error> {
        self.pattern_path_work(journal.path.as_deref().unwrap_or_default())?;
        let previous = journal.path.take();
        journal.path = match (&previous, field) {
            (Some(path), Some(field)) => { let mut path = path.clone(); path.push(field); Some(path) }
            _ => None,
        };
        let result = self.match_pattern_node(pattern, value, environment, journal);
        journal.path = previous;
        result
    }
    fn pattern_path_work(&mut self, path: &[usize]) -> Result<(), Error> {
        for _ in path { self.tick(None)?; }
        Ok(())
    }
}
