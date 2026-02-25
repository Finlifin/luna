//! Language items – well-known types and traits that the compiler must
//! know about in order to lower certain constructs.
//!
//! For example, the `for`-loop desugars into calls to `Iterator`, and the
//! `?` operator needs to know about the `Optional` type. Language items
//! connect these abstract compiler concepts to concrete user-visible
//! definitions.
//!
//! # How it works
//!
//! Each [`LangItem`] variant names a concept. During initialisation the
//! intrinsic crate registers placeholder `DefId`s for the built-in ones.
//! User code can also be tagged (e.g. via `#[lang = "..."]` annotations)
//! to provide the actual implementation; the resolver then calls
//! [`LangItems::set`] to record the mapping.

use std::fmt;

use rustc_data_structures::fx::FxHashMap;

// ── LangItem enum ────────────────────────────────────────────────────────────

/// Well-known items the compiler must be able to locate by concept rather
/// than by name alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LangItem {
    // ── Primitive types ──────────────────────────────────────────────────
    Int,
    Float,
    Bool,
    Char,
    Str,
    Unit,
    Never,

    // ── Built-in traits / protocols ──────────────────────────────────────
    Copy,
    Clone,
    Drop,
    Debug,
    Display,
    Eq,
    Ord,
    Hash,
    Iterator,
    Into,
    From,

    // ── Effect / control-flow ────────────────────────────────────────────
    /// The `Optional` (a.k.a. `?`) type.
    Optional,
    /// The `Result` type (if the language has one).
    Result,
    /// The `Future` / async trait.
    Future,
}

impl LangItem {
    /// The canonical string key for this lang item (used in annotations).
    pub fn name(self) -> &'static str {
        match self {
            LangItem::Int => "int",
            LangItem::Float => "float",
            LangItem::Bool => "bool",
            LangItem::Char => "char",
            LangItem::Str => "str",
            LangItem::Unit => "unit",
            LangItem::Never => "never",
            LangItem::Copy => "copy",
            LangItem::Clone => "clone",
            LangItem::Drop => "drop",
            LangItem::Debug => "debug",
            LangItem::Display => "display",
            LangItem::Eq => "eq",
            LangItem::Ord => "ord",
            LangItem::Hash => "hash",
            LangItem::Iterator => "iterator",
            LangItem::Into => "into",
            LangItem::From => "from",
            LangItem::Optional => "optional",
            LangItem::Result => "result",
            LangItem::Future => "future",
        }
    }

    /// Try to parse a lang-item key string back to the enum variant.
    pub fn from_name(s: &str) -> Option<Self> {
        Some(match s {
            "int" => LangItem::Int,
            "float" => LangItem::Float,
            "bool" => LangItem::Bool,
            "char" => LangItem::Char,
            "str" => LangItem::Str,
            "unit" => LangItem::Unit,
            "never" => LangItem::Never,
            "copy" => LangItem::Copy,
            "clone" => LangItem::Clone,
            "drop" => LangItem::Drop,
            "debug" => LangItem::Debug,
            "display" => LangItem::Display,
            "eq" => LangItem::Eq,
            "ord" => LangItem::Ord,
            "hash" => LangItem::Hash,
            "iterator" => LangItem::Iterator,
            "into" => LangItem::Into,
            "from" => LangItem::From,
            "optional" => LangItem::Optional,
            "result" => LangItem::Result,
            "future" => LangItem::Future,
            _ => return None,
        })
    }

    /// All known lang items, for iteration.
    pub const ALL: &'static [LangItem] = &[
        LangItem::Int,
        LangItem::Float,
        LangItem::Bool,
        LangItem::Char,
        LangItem::Str,
        LangItem::Unit,
        LangItem::Never,
        LangItem::Copy,
        LangItem::Clone,
        LangItem::Drop,
        LangItem::Debug,
        LangItem::Display,
        LangItem::Eq,
        LangItem::Ord,
        LangItem::Hash,
        LangItem::Iterator,
        LangItem::Into,
        LangItem::From,
        LangItem::Optional,
        LangItem::Result,
        LangItem::Future,
    ];
}

impl fmt::Display for LangItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

// ── LangItems table ──────────────────────────────────────────────────────────

/// A bidirectional mapping between [`LangItem`] concepts and their
/// concrete definition IDs.
///
/// Populated during intrinsic initialisation (for primitives) and later
/// during name resolution (for user-defined lang items).
#[derive(Debug, Default)]
pub struct LangItems {
    items: FxHashMap<LangItem, LangItemDef>,
}

/// What a lang item resolves to.
#[derive(Debug, Clone, Copy)]
pub enum LangItemDef {
    /// A compiler-synthesised built-in (no user-visible source location).
    Builtin,
    /// A user-provided definition (tagged via `#[lang = "..."]`).
    UserDef(hir::hir_id::LocalDefId),
}

impl LangItems {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register or overwrite a lang item mapping.
    pub fn set(&mut self, item: LangItem, def: LangItemDef) {
        self.items.insert(item, def);
    }

    /// Look up the definition of a lang item, if registered.
    pub fn get(&self, item: LangItem) -> Option<LangItemDef> {
        self.items.get(&item).copied()
    }

    /// Returns `true` if the lang item has been registered.
    pub fn has(&self, item: LangItem) -> bool {
        self.items.contains_key(&item)
    }

    /// Iterate over all registered lang items.
    pub fn iter(&self) -> impl Iterator<Item = (LangItem, LangItemDef)> + '_ {
        self.items.iter().map(|(&k, &v)| (k, v))
    }

    /// Number of registered lang items.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether no lang items have been registered yet.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}
