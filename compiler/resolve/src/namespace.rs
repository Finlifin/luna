//! Namespaces – the three "slots" a name can live in.
//!
//! Flurry follows a scheme similar to Rust: the same textual name can
//! simultaneously refer to a *type*, a *value*, and a *macro* without
//! ambiguity because each lives in its own namespace.

use crate::binding::Binding;

/// The three namespaces a name can inhabit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Namespace {
    /// Types: struct, enum, union, type alias, type parameter.
    Type,
    /// Values: function, constant, local variable, enum variant constructor.
    Value,
    /// Macros: compile-time macros and attributes.
    Macro,
}

impl Namespace {
    /// All namespaces, in canonical order.
    pub const ALL: [Namespace; 3] = [Namespace::Type, Namespace::Value, Namespace::Macro];
}

impl std::fmt::Display for Namespace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Namespace::Type => write!(f, "type"),
            Namespace::Value => write!(f, "value"),
            Namespace::Macro => write!(f, "macro"),
        }
    }
}

// ── PerNs ────────────────────────────────────────────────────────────────────

/// A per-namespace resolution bucket.
///
/// When looking up a name in a scope we may find up to three independent
/// bindings – one in each namespace.
#[derive(Debug, Clone, Default)]
pub struct PerNs {
    pub type_ns: Option<Binding>,
    pub value_ns: Option<Binding>,
    pub macro_ns: Option<Binding>,
}

impl PerNs {
    pub fn none() -> Self {
        Self::default()
    }

    /// Create a `PerNs` with a single binding in the given namespace.
    pub fn from_binding(ns: Namespace, binding: Binding) -> Self {
        let mut per_ns = Self::none();
        per_ns.set(ns, binding);
        per_ns
    }

    /// Get the binding in a specific namespace.
    pub fn get(&self, ns: Namespace) -> Option<&Binding> {
        match ns {
            Namespace::Type => self.type_ns.as_ref(),
            Namespace::Value => self.value_ns.as_ref(),
            Namespace::Macro => self.macro_ns.as_ref(),
        }
    }

    /// Set the binding for a specific namespace.
    pub fn set(&mut self, ns: Namespace, binding: Binding) {
        match ns {
            Namespace::Type => self.type_ns = Some(binding),
            Namespace::Value => self.value_ns = Some(binding),
            Namespace::Macro => self.macro_ns = Some(binding),
        }
    }

    /// Returns `true` if all namespace slots are `None`.
    pub fn is_empty(&self) -> bool {
        self.type_ns.is_none() && self.value_ns.is_none() && self.macro_ns.is_none()
    }

    /// Merge another `PerNs` into this one. Existing bindings are **not** overwritten.
    pub fn merge_from(&mut self, other: &PerNs) {
        if self.type_ns.is_none() {
            self.type_ns = other.type_ns.clone();
        }
        if self.value_ns.is_none() {
            self.value_ns = other.value_ns.clone();
        }
        if self.macro_ns.is_none() {
            self.macro_ns = other.macro_ns.clone();
        }
    }

    /// Iterate over `(Namespace, &Binding)` for the populated slots.
    pub fn iter(&self) -> impl Iterator<Item = (Namespace, &Binding)> {
        Namespace::ALL.iter().filter_map(move |&ns| {
            self.get(ns).map(|b| (ns, b))
        })
    }
}
