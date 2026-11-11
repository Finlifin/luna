//! Region-local input evidence, separate from invocation storage and code caches.
//! A checking retry restores the entire owner. Lexical exit removes new bindings
//! while retaining invalidation of outer evidence; those are different operations.
use super::*;

#[derive(Clone, Default)]
pub(super) struct State {
    pub specialized: Vec<(n::LocalId, Term, Term)>,
    // Ordinary Type inputs belong to the function telescope, independently of
    // declaration parameters and opaque normal-form identities.
    pub types: BTreeMap<n::LocalId, Term>,
    pub partial: BTreeMap<n::LocalId, std::sync::Arc<constants::PartialStagedValue>>,
    // Completed value identities retain their domains. Runtime inputs must not
    // become static values merely because an observation refers to their binder.
    pub values: BTreeMap<n::LocalId, (Term, Term)>,
    pub derived: BTreeSet<n::LocalId>,
    pub supplied: required::inputs::Scope,
}

pub(super) struct LexicalBoundary {
    specialized: BTreeSet<n::LocalId>,
    types: BTreeSet<n::LocalId>,
    partial: BTreeSet<n::LocalId>,
    values: BTreeSet<n::LocalId>,
    derived: BTreeSet<n::LocalId>,
}

impl std::fmt::Display for LexicalBoundary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(lexical-input-boundary")?;
        for (name, bindings) in [
            ("specialized", &self.specialized),
            ("types", &self.types),
            ("partial", &self.partial),
            ("values", &self.values),
            ("derived", &self.derived),
        ] {
            write!(f, " ({name}")?;
            for binding in bindings {
                write!(f, " {binding}")?;
            }
            f.write_str(")")?;
        }
        f.write_str(")")
    }
}

impl State {
    pub fn is_empty(&self) -> bool {
        self.specialized.is_empty()
            && self.types.is_empty()
            && self.partial.is_empty()
            && self.values.is_empty()
            && self.derived.is_empty()
            && self.supplied.is_empty()
    }

    /// Open bodies inherit lexical Type/value evidence and supplied input
    /// scopes, but never inherit the caller's active construction observations.
    pub fn for_open_body(&self) -> Self {
        Self {
            types: self.types.clone(),
            values: self.values.clone(),
            supplied: self.supplied.clone(),
            ..Default::default()
        }
    }

    pub fn lexical_boundary(&self) -> LexicalBoundary {
        LexicalBoundary {
            specialized: self.specialized.iter().map(|(id, _, _)| *id).collect(),
            types: self.types.keys().copied().collect(),
            partial: self.partial.keys().copied().collect(),
            values: self.values.keys().copied().collect(),
            derived: self.derived.clone(),
        }
    }

    pub fn leave_lexical(&mut self, boundary: &LexicalBoundary) {
        self.specialized
            .retain(|(id, _, _)| boundary.specialized.contains(id));
        self.types.retain(|id, _| boundary.types.contains(id));
        self.partial.retain(|id, _| boundary.partial.contains(id));
        self.values.retain(|id, _| boundary.values.contains(id));
        self.derived.retain(|id| boundary.derived.contains(id));
        // Supplied frames belong to required preparation, not block binders.
        // Keep any outer snapshot invalidations made in this lexical region.
    }

    pub fn invalidate(
        &mut self,
        mut roots: BTreeSet<n::LocalId>,
        staged: bool,
        fuel: &mut u64,
        span: Span,
    ) -> Result<(), Error> {
        self.supplied.invalidate(&roots, fuel, span)?;
        roots.retain(|id| {
            self.specialized.iter().any(|(input, _, _)| input == id)
                || self.partial.contains_key(id)
        });
        self.partial.retain(|id, _| !roots.contains(id));
        if !staged {
            self.specialized.retain(|(id, _, _)| !roots.contains(id));
            self.values.retain(|id, _| !roots.contains(id));
            self.types.retain(|id, _| !roots.contains(id));
        }
        // Provenance survives writes; it does not authorize reading a stale
        // completed value. The owning invocation checks actual mutable storage.
        Ok(())
    }
}

impl std::fmt::Display for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(input-facts (specialized")?;
        for (binding, domain, value) in &self.specialized {
            write!(f, " (input {binding} {domain} {value})")?;
        }
        f.write_str(") (types")?;
        for (binding, value) in &self.types {
            write!(f, " (input {binding} {value})")?;
        }
        f.write_str(") (values")?;
        for (binding, (value, domain)) in &self.values {
            write!(f, " (input {binding} {domain} {value})")?;
        }
        f.write_str(") (partial")?;
        for (binding, value) in &self.partial {
            write!(f, " (input {binding} {value})")?;
        }
        f.write_str(") (derived")?;
        for binding in &self.derived {
            write!(f, " {binding}")?;
        }
        write!(f, ") {})", self.supplied)
    }
}
