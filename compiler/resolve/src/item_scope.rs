//! Item scope – the per-scope "namespace manager".
//!
//! An [`ItemScope`] stores all the names visible in a single scope.
//!  It also records the resolved imports that feed names
//! into the scope.

use std::collections::HashMap;

use symbol::Symbol;

use crate::binding::Binding;
use crate::import::ResolvedImport;

/// The names visible in a single scope.
///
/// Each module / ADT body / impl block / trait owns an `ItemScope`.
/// Function bodies use the [`RibStack`](crate::rib::RibStack) instead.
#[derive(Debug, Clone)]
pub struct ItemScope {
    /// Locally defined names (direct definitions in this scope).
    declarations: HashMap<Symbol, Binding>,
    pub(crate) glob_entries: Vec<(crate::ids::ScopeId, crate::binding::Visibility)>,
    pub(crate) glob_candidates: HashMap<Symbol, Vec<Binding>>,
    /// Private imports (`use …`): bring names into this scope but are not
    /// visible to the outside world.
    imports: Vec<ResolvedImport>,
    /// Re-exported imports (`pub use …`): bring names into this scope *and*
    /// expose them as part of this scope's public API.
    reexports: Vec<ResolvedImport>,
    /// Clauses (type parameters, bounds) associated with this scope's owner.
    clauses: Vec<ClauseBinding>,
}

/// A clause-level binding (type parameter / bounded type param / value param).
#[derive(Debug, Clone)]
pub struct ClauseBinding {
    pub name: Symbol,
    pub binding: Binding,
}

impl ItemScope {
    pub fn new() -> Self {
        Self {
            declarations: HashMap::new(),
            glob_candidates: HashMap::new(),
            glob_entries: Vec::new(),
            imports: Vec::new(),
            reexports: Vec::new(),
            clauses: Vec::new(),
        }
    }

    pub fn ambiguous_glob(&self, name: Symbol) -> bool {
        self.get_local(name).is_none()
            && self
                .glob_candidates
                .get(&name)
                .is_some_and(|xs| xs.len() > 1)
    }
    pub fn unique_glob(&self, name: Symbol) -> Option<&Binding> {
        self.glob_candidates
            .get(&name)
            .filter(|xs| xs.len() == 1)
            .map(|xs| &xs[0])
    }
    pub(crate) fn selected_candidates(&self) -> HashMap<Symbol, Vec<Binding>> {
        let mut names = self.glob_candidates.clone();
        for (name, binding) in &self.declarations {
            names.insert(*name, vec![binding.clone()]);
        }
        names
    }

    /// Define a name in this scope. Returns `Err` with the old binding if the
    /// name already exists.
    pub fn define(&mut self, name: Symbol, binding: Binding) -> Result<(), Binding> {
        if self.declarations.contains_key(&name) {
            return Err(self.declarations[&name].clone());
        }
        self.declarations.insert(name, binding);
        Ok(())
    }

    /// Define a name, allowing shadowing (overwrites previous binding).
    pub fn define_or_overwrites(&mut self, name: Symbol, binding: Binding) {
        self.declarations.insert(name, binding);
    }

    /// Look up a name among **direct** declarations only (no imports).
    pub fn get_direct(&self, name: Symbol) -> Option<&Binding> {
        self.declarations.get(&name)
    }

    /// Look up a name including both direct declarations and clauses.
    pub fn get_local(&self, name: Symbol) -> Option<&Binding> {
        // Direct declarations first
        if let Some(b) = self.get_direct(name) {
            return Some(b);
        }
        // Then clauses
        self.clauses.iter().rev().find_map(|cb| {
            if cb.name == name {
                Some(&cb.binding)
            } else {
                None
            }
        })
    }

    /// All direct declarations.
    pub fn declarations(&self) -> &HashMap<Symbol, Binding> {
        &self.declarations
    }

    /// Number of declared names (not counting imports).
    pub fn declaration_count(&self) -> usize {
        self.declarations.len()
    }

    /// Record a private resolved import (`use …`).
    pub fn add_import(&mut self, import: ResolvedImport) {
        self.imports.push(import);
    }

    /// Record a re-exported resolved import (`pub use …`).
    pub fn add_reexport(&mut self, import: ResolvedImport) {
        self.reexports.push(import);
    }

    /// The list of private resolved imports.
    pub fn imports(&self) -> &[ResolvedImport] {
        &self.imports
    }

    /// The list of re-exported resolved imports.
    pub fn reexports(&self) -> &[ResolvedImport] {
        &self.reexports
    }

    /// All resolved imports visible in this scope (private + re-exported).
    pub fn all_imports(&self) -> impl Iterator<Item = &ResolvedImport> {
        self.imports.iter().chain(self.reexports.iter())
    }

    /// Add a clause-level binding (type parameter, bounded param, etc.).
    pub fn add_clause(&mut self, name: Symbol, binding: Binding) {
        self.clauses.push(ClauseBinding { name, binding });
    }

    /// The clause bindings.
    pub fn clauses(&self) -> &[ClauseBinding] {
        &self.clauses
    }
}

impl Default for ItemScope {
    fn default() -> Self {
        Self::new()
    }
}
