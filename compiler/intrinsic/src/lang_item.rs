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

/// Well-known items the compiler must be able to locate by concept rather
/// than by name alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LangItem {
    AnyType,
    U8,
    U16,
    U32,
    U64,
    U128,
    Usize,
    I8,
    I16,
    I32,
    I64,
    I128,
    Isize,
    Integer,
    F16,
    F32,
    F64,
    Real,
    Bool,
    Char,
    Str,
    Void,
    NoReturn,
    Type,

    Undefined,
    AnyValue,

    Copy,
    Clone,
    Finalze,
    Debug,
    Display,
    Eq,
    Ord,
    Hash,
    Iterator,
    Into,
    From,
}

impl LangItem {
    /// The canonical string key for this lang item (used in annotations).
    pub fn name(self) -> &'static str {
        match self {
            LangItem::AnyType => "Any",
            LangItem::U8 => "u8",
            LangItem::U16 => "u16",
            LangItem::U32 => "u32",
            LangItem::U64 => "u64",
            LangItem::U128 => "u128",
            LangItem::Usize => "usize",
            LangItem::I8 => "i8",
            LangItem::I16 => "i16",
            LangItem::I32 => "i32",
            LangItem::I64 => "i64",
            LangItem::I128 => "i128",
            LangItem::Isize => "isize",
            LangItem::Integer => "Integer",
            LangItem::F16 => "f16",
            LangItem::F32 => "f32",
            LangItem::F64 => "f64",
            LangItem::Real => "Real",
            LangItem::Bool => "bool",
            LangItem::Char => "char",
            LangItem::Str => "str",
            LangItem::Void => "void",
            LangItem::NoReturn => "NoReturn",
            LangItem::Type => "type",
            LangItem::Undefined => "undefined",
            LangItem::AnyValue => "any",
            LangItem::Copy => "Copy",
            LangItem::Clone => "Clone",
            LangItem::Finalze => "Finalize",
            LangItem::Debug => "Debug",
            LangItem::Display => "Display",
            LangItem::Eq => "Eq",
            LangItem::Ord => "Ord",
            LangItem::Hash => "Hash",
            LangItem::Iterator => "Iterator",
            LangItem::Into => "Into",
            LangItem::From => "From",
        }
    }
}

impl fmt::Display for LangItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

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
    /// A user-provided definition
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
