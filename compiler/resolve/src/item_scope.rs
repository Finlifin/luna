//! Item scope – the per-scope "namespace manager".
//!
//! An [`ItemScope`] stores all the names visible in a single scope, grouped
//! by [`Namespace`]. It also records the resolved imports that feed names
//! into the scope.

use std::collections::HashMap;

use crate::binding::Binding;
use crate::import::ResolvedImport;
use crate::namespace::{Namespace, PerNs};

// ── ItemScope ────────────────────────────────────────────────────────────────

/// The names visible in a single scope.
///
/// Each module / ADT body / impl block / trait owns an `ItemScope`.
/// Function bodies use the [`RibStack`](crate::rib::RibStack) instead.
#[derive(Debug, Clone)]
pub struct ItemScope {
    /// Locally defined names (direct definitions in this scope).
    declarations: HashMap<String, PerNs>,
    /// Resolved imports that bring names into this scope.
    imports: Vec<ResolvedImport>,
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
            clauses: Vec::new(),
        }
    }

    // ── Declarations ─────────────────────────────────────────────────────

    /// Define a name in this scope. Returns `Err` with the old binding if the
    /// name already exists in the same namespace.
    pub fn define(
        &mut self,
        name: String,
        ns: Namespace,
        binding: Binding,
    ) -> Result<(), Binding> {
        let per_ns = self.declarations.entry(name).or_default();
        if per_ns.get(ns).is_some() {
            return Err(binding);
        }
        per_ns.set(ns, binding);
        Ok(())
    }

    /// Define a name, allowing shadowing (overwrites previous binding).
    pub fn define_or_shadow(&mut self, name: String, ns: Namespace, binding: Binding) {
        self.declarations.entry(name).or_default().set(ns, binding);
    }

    /// Look up a name among **direct** declarations only (no imports).
    pub fn get_direct(&self, name: &str, ns: Namespace) -> Option<&Binding> {
        self.declarations.get(name).and_then(|per_ns| per_ns.get(ns))
    }

    /// Look up a name including both direct declarations and clauses.
    pub fn get_local(&self, name: &str, ns: Namespace) -> Option<&Binding> {
        // Direct declarations first
        if let Some(b) = self.get_direct(name, ns) {
            return Some(b);
        }
        // Then clauses
        self.clauses.iter().rev().find_map(|cb| {
            if cb.name == name && cb.binding.ns == ns {
                Some(&cb.binding)
            } else {
                None
            }
        })
    }

    /// All direct declarations.
    pub fn declarations(&self) -> &HashMap<String, PerNs> {
        &self.declarations
    }

    /// Number of declared names (not counting imports).
    pub fn declaration_count(&self) -> usize {
        self.declarations.len()
    }

    // ── Imports ──────────────────────────────────────────────────────────

    /// Record a resolved import.
    pub fn add_import(&mut self, import: ResolvedImport) {
        self.imports.push(import);
    }

    /// The list of resolved imports.
    pub fn imports(&self) -> &[ResolvedImport] {
        &self.imports
    }

    // ── Clauses ──────────────────────────────────────────────────────────

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
