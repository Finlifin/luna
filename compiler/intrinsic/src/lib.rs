//! # Intrinsic crate – built-in content for the Luna/Flurry compiler.
//!
//! This crate defines and registers everything the compiler "just knows"
//! without needing a source definition: primitive types, built-in
//! functions, well-known symbols, and language items.
//!
//! ## Crate Dependency Graph
//!
//! ```text
//!   hir, ty                       ← foundational type crates
//!       ↑
//!   intrinsic                     ← THIS CRATE
//!       ↑
//!   interface                     ← CompilerInstance calls initialize()
//! ```
//!
//! ## Architecture
//!
//! ```text
//!   IntrinsicContext
//!   ├── lang_items : LangItems          ← concept → DefId mapping
//!   ├── builtin_fns: Vec<BuiltinFnId>   ← registered built-in functions
//!   └── initialized: bool
//!
//!   symbols   (module)   ← pre-interned well-known Symbol values
//!   builtins  (module)   ← BuiltinFn catalogue + lookup
//!   lang_item (module)   ← LangItem enum + LangItems table
//! ```
//!
//! The public entry point is [`initialize`], which is called by
//! `CompilerInstance::new()` to populate the type context and lang-item
//! table with compiler-provided definitions.

// pub mod builtins;
pub mod lang_item;
pub mod symbols;
pub mod sysroot;

// pub use builtins::{ALL_BUILTINS, BuiltinFn, BuiltinFnId, BuiltinTy};
pub use lang_item::{LangItem, LangItemDef, LangItems};
pub use sysroot::{PackageId, Sysroot, SysrootPackage};

use ty::TyCtxt;

/// Owns all intrinsic / built-in state for a single compilation.
///
/// Created by [`initialize`] and stored inside `CompilerInstance`.
pub struct IntrinsicContext {
    /// The language-item lookup table.
    pub lang_items: LangItems,
}

/// Initialise all compiler-provided built-in content.
///
/// This is the **single entry point** called by `CompilerInstance::new()`
/// to bootstrap the intrinsic environment. It:
///
/// 1. Pre-interns well-known symbols so later lookups are O(1).
/// 2. Registers primitive types as lang items.
/// 3. Registers built-in function type signatures into the [`TyCtxt`].
///
/// Returns an [`IntrinsicContext`] that should be stored in the compiler
/// instance for later queries.
pub fn initialize(_ty_ctxt: &TyCtxt) -> IntrinsicContext {
    // Register primitive types as lang items.
    let mut lang_items = LangItems::new();
    use LangItem::*;
    for item in [
        AnyType, U8, U16, U32, U64, U128, Usize, I8, I16, I32, I64, I128, Isize, Integer, F16, F32,
        F64, Real, Bool, Char, Str, Void, NoReturn, Type,
    ] {
        lang_items.set(item, LangItemDef::Builtin);
    }

    IntrinsicContext { lang_items }
}
