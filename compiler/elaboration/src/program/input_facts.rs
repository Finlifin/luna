//! Region-local input evidence, separate from invocation storage and code caches.
//! A checking retry restores the entire owner. Lexical exit removes new bindings
//! while retaining invalidation of outer evidence; those are different operations.
use super::*;

/// Symbolic ordinary inputs in the current declaration prefix. These bindings
/// advance at parameter boundaries and are shared across its default queries.
#[derive(Clone, Default)]
pub(super) struct Parameters {
    declared: LocalBindings<(Term, Term)>,
    declared_runtime: LocalBindings<(Term, Term)>,
    invalidated: BTreeSet<n::LocalId>,
}

impl Parameters {
    pub fn bind(&mut self, binding: n::LocalId, index: usize, sort: Sort, domain: &Term) {
        if sort != Sort::Runtime {
            self.declared.insert(
                binding,
                (
                    Term::Bound {
                        depth: 0,
                        index,
                        sort,
                    },
                    domain.clone(),
                ),
            );
        }
    }

    pub fn bind_runtime(&mut self, binding: n::LocalId, index: usize, domain: &Term) {
        self.declared_runtime.insert(
            binding,
            (
                Term::Bound {
                    depth: 0,
                    index,
                    sort: Sort::Runtime,
                },
                domain.clone(),
            ),
        );
    }

    pub fn bind_runtime_value(&mut self, binding: n::LocalId, identity: Term, domain: Term) {
        self.declared_runtime.insert(binding, (identity, domain));
    }

    pub fn runtime_identity(&self, binding: &n::LocalId) -> Option<&(Term, Term)> {
        if self.invalidated.contains(binding) {
            return None;
        }
        self.declared_runtime.get(binding)
    }

    pub fn runtime_identities(&self) -> impl Iterator<Item = (&n::LocalId, &(Term, Term))> {
        self.declared_runtime
            .iter()
            .filter(|(id, _)| !self.invalidated.contains(id))
    }

    pub fn has_runtime_identities(&self) -> bool {
        self.runtime_identities().next().is_some()
    }

    pub fn is_empty(&self) -> bool {
        self.declared.is_empty() && self.declared_runtime.is_empty()
    }

    fn entry_identity(&self, binding: &n::LocalId) -> Option<&(Term, Term)> {
        if self.invalidated.contains(binding) {
            return None;
        }
        self.declared.get(binding)
    }

    pub fn identities(&self) -> impl Iterator<Item = (&n::LocalId, &(Term, Term))> {
        self.declared
            .iter()
            .filter(|(id, _)| !self.invalidated.contains(id))
    }
}

#[derive(Clone, Default)]
pub(super) struct State {
    pub parameters: Parameters,
    pub specialized: Vec<(n::LocalId, Term, Term)>,
    // Completed Type observations can specialize a symbolic input.
    pub types: BTreeMap<n::LocalId, Term>,
    pub partial: BTreeMap<n::LocalId, std::sync::Arc<constants::PartialStagedValue>>,
    // Completed value identities retain their domains. Runtime inputs must not
    // become static values merely because an observation refers to their binder.
    pub values: BTreeMap<n::LocalId, (Term, Term)>,
    pub derived: BTreeSet<n::LocalId>,
    pub supplied: required::inputs::Scope,
    // These rigid identities exist inside executable lexical scopes only.
    pub named_types: scoped_operands::NamedTypes,
    pub annotation: Option<scoped_operands::Annotation>,
    pub observing_annotation_head: bool,
}

pub(super) struct LexicalBoundary {
    named_types: scoped_operands::NamedTypes,
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
    pub fn type_identity(&self, binding: &n::LocalId) -> Option<&Term> {
        self.types.get(binding).or_else(|| {
            let (symbol, _) = self.parameters.entry_identity(binding)?;
            (!self.values.contains_key(binding) && symbol.sort() == Sort::Type).then_some(symbol)
        })
    }

    pub fn value_identity(&self, binding: &n::LocalId) -> Option<&(Term, Term)> {
        self.values.get(binding).or_else(|| {
            let parameter = self.parameters.entry_identity(binding)?;
            (!self.types.contains_key(binding) && parameter.0.sort() == Sort::ComptimeValue)
                .then_some(parameter)
        })
    }

    pub fn has_identity(&self, binding: &n::LocalId) -> bool {
        self.parameters.entry_identity(binding).is_some()
            || self.types.contains_key(binding)
            || self.values.contains_key(binding)
    }

    pub fn is_staged(&self, binding: &n::LocalId) -> bool {
        self.parameters.declared.contains_key(binding)
            || self.types.contains_key(binding)
            || self.values.contains_key(binding)
    }

    pub fn is_empty(&self) -> bool {
        self.parameters.is_empty()
            && self.specialized.is_empty()
            && self.types.is_empty()
            && self.partial.is_empty()
            && self.values.is_empty()
            && self.derived.is_empty()
            && self.supplied.is_empty()
            && self.named_types.is_empty()
            && self.annotation.is_none()
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
            named_types: self.named_types.clone(),
            specialized: self.specialized.iter().map(|(id, _, _)| *id).collect(),
            types: self.types.keys().copied().collect(),
            partial: self.partial.keys().copied().collect(),
            values: self.values.keys().copied().collect(),
            derived: self.derived.clone(),
        }
    }

    pub fn leave_lexical(&mut self, boundary: &LexicalBoundary) {
        self.named_types = boundary.named_types.clone();
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
                || (!staged && self.parameters.declared.contains_key(id))
        });
        self.partial.retain(|id, _| !roots.contains(id));
        if !staged {
            self.specialized.retain(|(id, _, _)| !roots.contains(id));
            for binding in &roots {
                if self.parameters.declared.contains_key(binding) {
                    self.parameters.invalidated.insert(*binding);
                }
            }
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
        f.write_str("(input-facts (parameters")?;
        for (binding, (symbol, domain)) in self.parameters.declared.iter() {
            if self.parameters.invalidated.contains(binding) {
                write!(f, " (invalidated {binding} {domain})")?;
            } else {
                write!(f, " (input {binding} {domain} {symbol})")?;
            }
        }
        f.write_str(") (specialized")?;
        for (binding, domain, value) in &self.specialized {
            write!(f, " (input {binding} {domain} {value})")?;
        }
        f.write_str(") (types")?;
        for (binding, value) in &self.types {
            write!(f, " (input {binding} {value})")?;
        }
        write!(f, ") {} (values", self.named_types)?;
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
