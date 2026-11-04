//! Builtin symbol bindings, language items and sysroot discovery.
//!
//! [`initialize`] creates the bootstrap language-item table. Source protocol
//! bindings are resolved separately through their declaration identities.

pub mod lang_item;
pub mod symbols;
pub mod sysroot;

pub use lang_item::{LangItem, LangItemDef, LangItems, SourceBindingError};
pub use sysroot::{PackageId, Sysroot, SysrootPackage};

/// Owns all intrinsic / built-in state for a single compilation.
///
/// Created by [`initialize`] and stored inside `CompilerInstance`.
pub struct IntrinsicContext {
    /// The language-item lookup table.
    pub lang_items: LangItems,
}

/// Create the bootstrap table of compiler-provided primitive language items.
pub fn initialize() -> IntrinsicContext {
    // Register primitive types as lang items.
    let mut lang_items = LangItems::new();
    use LangItem::*;
    for item in [
        AnyType, U8, U16, U32, U64, U128, Usize, I8, I16, I32, I64, I128, Isize, Integer, F16,
        Bf16, F32, F64, Real, Bool, Char, Str, Void, NoReturn, Type,
    ] {
        lang_items
            .set(item, LangItemDef::Builtin)
            .expect("bootstrap language items are unique");
    }

    IntrinsicContext { lang_items }
}
