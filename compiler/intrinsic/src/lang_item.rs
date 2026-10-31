//! Language items – well-known types and traits that the compiler must
//! know about in order to lower certain constructs.
//!
//! For example, `for` uses the `Iter` and `IntoIterator` protocols, and the
//! `?` operator needs to know about the `Optional` type. Language items
//! connect these abstract compiler concepts to concrete user-visible
//! definitions.
//!
//! Primitive entries are installed at bootstrap. Source protocol declarations
//! are registered by their package-qualified semantic identity; absence remains
//! absence until a trusted language-item binding supplies the declaration.

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
    Bf16,
    F32,
    F64,
    Real,
    Bool,
    Char,
    Str,
    Void,
    NoReturn,
    Type,
    /// Source-defined pure comptime normal-form head; not a primitive Type.
    Tuple,

    Undefined,
    AnyValue,

    Once,
    Copy,
    Clone,
    Drop,
    Debug,
    Display,
    Eq,
    Ord,
    Hash,
    Iter,
    IntoIterator,
    Into,
    From,
    Splice,
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
            LangItem::Bf16 => "bf16",
            LangItem::F32 => "f32",
            LangItem::F64 => "f64",
            LangItem::Real => "Real",
            LangItem::Bool => "bool",
            LangItem::Char => "char",
            LangItem::Str => "str",
            LangItem::Void => "void",
            LangItem::NoReturn => "NoReturn",
            LangItem::Type => "type",
            LangItem::Tuple => "Tuple",
            LangItem::Undefined => "undefined",
            LangItem::AnyValue => "any",
            LangItem::Once => "Once",
            LangItem::Copy => "Copy",
            LangItem::Clone => "Clone",
            LangItem::Drop => "Drop",
            LangItem::Debug => "Debug",
            LangItem::Display => "Display",
            LangItem::Eq => "Eq",
            LangItem::Ord => "Ord",
            LangItem::Hash => "Hash",
            LangItem::Iter => "Iter",
            LangItem::IntoIterator => "IntoIterator",
            LangItem::Into => "Into",
            LangItem::From => "From",
            LangItem::Splice => "Splice",
        }
    }
}

impl fmt::Display for LangItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A mapping between [`LangItem`] concepts and their
/// concrete definition IDs.
///
/// Populated during intrinsic initialisation (for primitives), then through
/// explicit driver bindings against elaborated source declarations.
#[derive(Debug, Default, Clone)]
pub struct LangItems {
    items: FxHashMap<LangItem, LangItemDef>,
}

/// What a lang item resolves to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LangItemDef {
    /// A compiler-synthesised built-in (no user-visible source location).
    Builtin,
    /// A user-provided definition
    UserDef(nir::DeclarationId),
}

impl LangItems {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind source trait identities supplied by the trusted compilation driver.
    /// Names, imports and aliases never implicitly designate language items.
    /// Validate the entire batch before publishing any registration. This checks
    /// identity/kind only; protocol-specific method and resource contracts must
    /// still be checked by each consuming pass.
    pub fn bind_source_traits(
        &mut self,
        package: &nir::program::Package,
        bindings: &[(LangItem, nir::DeclarationId)],
    ) -> Result<(), SourceBindingError> {
        let mut staged = self.clone();
        for &(item, declaration) in bindings {
            if !matches!(
                item,
                LangItem::Once
                    | LangItem::Clone
                    | LangItem::Copy
                    | LangItem::Drop
                    | LangItem::Debug
                    | LangItem::Display
                    | LangItem::Eq
                    | LangItem::Ord
                    | LangItem::Hash
                    | LangItem::Iter
                    | LangItem::IntoIterator
                    | LangItem::Into
                    | LangItem::From
                    | LangItem::Splice
            ) {
                return Err(SourceBindingError::NotTraitItem(item));
            }
            let count = package
                .traits
                .iter()
                .filter(|t| t.declaration == declaration)
                .count();
            if count != 1 {
                return Err(SourceBindingError::InvalidTraitIdentity { item, declaration });
            }
            staged
                .set(item, LangItemDef::UserDef(declaration))
                .map_err(SourceBindingError::Conflict)?;
        }
        *self = staged;
        Ok(())
    }

    /// Bind opaque source heads without inspecting their bodies or using names.
    /// This validates declaration kind and purity only. Consumers must validate
    /// the particular head's input telescope and collection protocol separately.
    /// External head metadata is allowed without transporting executable bodies.
    pub fn bind_source_normal_form_heads(
        &mut self,
        package: &nir::program::Package,
        bindings: &[(LangItem, nir::DeclarationId)],
    ) -> Result<(), SourceBindingError> {
        let mut staged = self.clone();
        for &(item, declaration) in bindings {
            if item != LangItem::Tuple {
                return Err(SourceBindingError::NotNormalFormItem(item));
            }
            let count = package
                .normal_forms
                .iter()
                .filter(|head| head.declaration == declaration)
                .count();
            let functions: Vec<_> = package
                .functions
                .iter()
                .filter(|function| function.declaration == declaration)
                .collect();
            if count != 1
                || functions.len() > 1
                || functions.first().is_some_and(|function| {
                    function.phase != nir::program::FunctionPhase::NormalForm || !function.pure
                })
            {
                return Err(SourceBindingError::InvalidNormalFormIdentity { item, declaration });
            }
            staged
                .set(item, LangItemDef::UserDef(declaration))
                .map_err(SourceBindingError::Conflict)?;
        }
        *self = staged;
        Ok(())
    }

    /// Register one identity. Re-registering the same binding is idempotent;
    /// conflicting bindings never replace a previously accepted declaration.
    pub fn set(&mut self, item: LangItem, def: LangItemDef) -> Result<(), RegistrationConflict> {
        if let Some(previous) = self.items.get(&item) {
            if *previous != def {
                return Err(RegistrationConflict {
                    item,
                    previous: *previous,
                    attempted: def,
                });
            }
            return Ok(());
        }
        self.items.insert(item, def);
        Ok(())
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
        let mut entries: Vec<_> = self.items.iter().map(|(&k, &v)| (k, v)).collect();
        entries.sort_by_key(|(item, _)| item.name());
        entries.into_iter()
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegistrationConflict {
    pub item: LangItem,
    pub previous: LangItemDef,
    pub attempted: LangItemDef,
}
impl fmt::Display for LangItemDef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Builtin => f.write_str("builtin"),
            Self::UserDef(id) => write!(f, "(definition {id})"),
        }
    }
}
impl fmt::Display for LangItems {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(language-items")?;
        for (item, definition) in self.iter() {
            write!(f, " ({} {})", item.name(), definition)?;
        }
        f.write_str(")")
    }
}
impl fmt::Display for RegistrationConflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "conflicting language item {}: existing {}, attempted {}",
            self.item, self.previous, self.attempted
        )
    }
}
impl std::error::Error for RegistrationConflict {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceBindingError {
    NotTraitItem(LangItem),
    NotNormalFormItem(LangItem),
    InvalidNormalFormIdentity {
        item: LangItem,
        declaration: nir::DeclarationId,
    },
    InvalidTraitIdentity {
        item: LangItem,
        declaration: nir::DeclarationId,
    },
    Conflict(RegistrationConflict),
}
impl fmt::Display for SourceBindingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotNormalFormItem(item) => {
                write!(f, "language item {item} is not a source normal-form item")
            }
            Self::InvalidNormalFormIdentity { item, declaration } => write!(
                f,
                "language item {item} requires exactly one pure normal-form head at {declaration}"
            ),
            Self::NotTraitItem(item) => {
                write!(f, "language item {item} is not a source trait item")
            }
            Self::InvalidTraitIdentity { item, declaration } => write!(
                f,
                "language item {item} requires exactly one declared trait at {declaration}"
            ),
            Self::Conflict(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for SourceBindingError {}
