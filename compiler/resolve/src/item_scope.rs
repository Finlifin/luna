//! Item scope – the per-scope "namespace manager".
//!
//! An [`ItemScope`] stores all the names visible in a single scope, grouped
//! by [`Namespace`]. It also records the resolved imports that feed names
//! into the scope.

use std::collections::HashMap;

use crate::binding::Binding;
use crate::import::ResolvedImport;

/// The names visible in a single scope.
///
/// Each module / ADT body / impl block / trait owns an `ItemScope`.
/// Function bodies use the [`RibStack`](crate::rib::RibStack) instead.
#[derive(Debug, Clone)]
pub struct ItemScope {
    /// Locally defined names (direct definitions in this scope).
    declarations: HashMap<String, Binding>,
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
    pub name: String,
    pub binding: Binding,
}

impl ItemScope {
    pub fn new() -> Self {
        Self {
            declarations: HashMap::new(),
            imports: Vec::new(),
            reexports: Vec::new(),
            clauses: Vec::new(),
        }
    }

    /// Define a name in this scope. Returns `Err` with the old binding if the
    /// name already exists.
    pub fn define(&mut self, name: String, binding: Binding) -> Result<(), Binding> {
        if self.declarations.contains_key(&name) {
            return Err(self.declarations[&name].clone());
        }
        self.declarations.insert(name, binding);
        Ok(())
    }

    /// Define a name, allowing shadowing (overwrites previous binding).
    pub fn define_or_overwrites(&mut self, name: String, binding: Binding) {
        self.declarations.insert(name, binding);
    }

    /// Look up a name among **direct** declarations only (no imports).
    pub fn get_direct(&self, name: &str) -> Option<&Binding> {
        self.declarations.get(name)
    }

    /// Look up a name including both direct declarations and clauses.
    pub fn get_local(&self, name: &str) -> Option<&Binding> {
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
    pub fn declarations(&self) -> &HashMap<String, Binding> {
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
    pub fn add_clause(&mut self, name: String, binding: Binding) {
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
