//! Ribs – the lexical resolution stack.
//!
//! A *rib* captures a snapshot of which names are visible at a particular
//! point in a function body. Ribs are pushed/popped as the resolver walks
//! into / out of blocks, match arms, closures, etc.
//!
//! Ribs are only relevant for *ordered* scopes (function bodies, blocks).
//! Module-level resolution does not use the rib stack.

use std::collections::HashMap;

use crate::binding::Binding;
use crate::ids::ScopeId;

// ── RibKind ──────────────────────────────────────────────────────────────────

/// What syntactic construct pushed this rib.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RibKind {
    /// A normal block `{ … }`.
    Normal,
    /// A function parameter list.
    FnParams,
    /// A closure / lambda capture boundary.
    Closure,
    /// A `let` / `const` binding.
    LetBinding,
    /// A `for` loop pattern.
    ForLoop,
    /// A match arm pattern.
    MatchArm,
    /// An `if-let` pattern binding.
    IfLet,
    /// Type parameter scope (clause-level).
    TypeParam,
}

// ── Rib ──────────────────────────────────────────────────────────────────────

/// A rib on the lexical resolution stack.
///
/// Each rib records the bindings introduced by one syntactic construct
/// (a block, a parameter list, a match arm pattern, etc.).
#[derive(Debug, Clone)]
pub struct Rib {
    pub kind: RibKind,
    /// Bindings introduced by this rib, keyed by name.
    pub bindings: HashMap<String, Binding>,
    /// The scope this rib is associated with.
    pub scope_id: ScopeId,
}

impl Rib {
    pub fn new(kind: RibKind, scope_id: ScopeId) -> Self {
        Rib {
            kind,
            bindings: HashMap::new(),
            scope_id,
        }
    }

    /// Introduce a binding into this rib.
    pub fn define(&mut self, name: String, binding: Binding) {
        self.bindings.insert(name, binding);
    }

    /// Look up a name in this rib only.
    pub fn get(&self, name: &str) -> Option<&Binding> {
        self.bindings.get(name)
    }
}

// ── RibStack ─────────────────────────────────────────────────────────────────

/// A stack of ribs for lexical resolution inside ordered scopes
/// (function bodies, blocks).
///
/// The resolver pushes a rib when entering a new block / pattern and pops
/// it when leaving. Name look-ups walk the stack from top to bottom.
#[derive(Debug)]
pub struct RibStack {
    ribs: Vec<Rib>,
}

impl RibStack {
    pub fn new() -> Self {
        Self { ribs: Vec::new() }
    }

    /// Push a new rib onto the stack.
    pub fn push(&mut self, rib: Rib) {
        self.ribs.push(rib);
    }

    /// Pop the top rib.
    pub fn pop(&mut self) -> Option<Rib> {
        self.ribs.pop()
    }

    /// The current (top) rib, if any.
    pub fn current(&self) -> Option<&Rib> {
        self.ribs.last()
    }

    /// The current (top) rib, mutable.
    pub fn current_mut(&mut self) -> Option<&mut Rib> {
        self.ribs.last_mut()
    }

    /// Introduce a binding into the current (top) rib.
    pub fn define(&mut self, name: String, binding: Binding) {
        if let Some(rib) = self.current_mut() {
            rib.define(name, binding);
        }
    }

    /// Look up a name by walking the rib stack top-to-bottom.
    pub fn lookup(&self, name: &str) -> Option<&Binding> {
        for rib in self.ribs.iter().rev() {
            if let Some(binding) = rib.get(name) {
                return Some(binding);
            }
        }
        None
    }

    /// Depth of the stack.
    pub fn depth(&self) -> usize {
        self.ribs.len()
    }

    /// Is the stack empty?
    pub fn is_empty(&self) -> bool {
        self.ribs.is_empty()
    }
}

impl Default for RibStack {
    fn default() -> Self {
        Self::new()
    }
}
