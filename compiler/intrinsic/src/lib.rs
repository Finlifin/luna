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

pub mod builtins;
pub mod lang_item;
pub mod symbols;
pub mod sysroot;

pub use builtins::{ALL_BUILTINS, BuiltinFn, BuiltinFnId, BuiltinTy};
pub use lang_item::{LangItem, LangItemDef, LangItems};
pub use sysroot::{PackageId, Sysroot, SysrootPackage};

use ty::TyCtxt;

// ── IntrinsicContext ─────────────────────────────────────────────────────────

/// Owns all intrinsic / built-in state for a single compilation.
///
/// Created by [`initialize`] and stored inside `CompilerInstance`.
pub struct IntrinsicContext {
    /// The language-item lookup table.
    pub lang_items: LangItems,
    /// IDs of built-in functions that have been registered.
    pub builtin_fn_ids: Vec<BuiltinFnId>,
}

impl IntrinsicContext {
    /// How many built-in functions have been registered.
    pub fn num_builtins(&self) -> usize {
        self.builtin_fn_ids.len()
    }

    /// Look up a [`BuiltinFn`] descriptor by name.
    pub fn lookup_builtin(&self, name: &str) -> Option<&'static BuiltinFn> {
        builtins::lookup_builtin(name)
    }
}

// ── Initialisation entry point ───────────────────────────────────────────────

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
pub fn initialize(ty_ctxt: &TyCtxt) -> IntrinsicContext {
    // ── 1. Pre-intern all well-known symbols ─────────────────────────────
    for (_, intern_fn) in symbols::all() {
        intern_fn();
    }

    // ── 2. Register primitive-type lang items ────────────────────────────
    let mut lang_items = LangItems::new();

    lang_items.set(LangItem::Int, LangItemDef::Builtin);
    lang_items.set(LangItem::Float, LangItemDef::Builtin);
    lang_items.set(LangItem::Bool, LangItemDef::Builtin);
    lang_items.set(LangItem::Char, LangItemDef::Builtin);
    lang_items.set(LangItem::Str, LangItemDef::Builtin);
    lang_items.set(LangItem::Unit, LangItemDef::Builtin);
    lang_items.set(LangItem::Never, LangItemDef::Builtin);

    // ── 3. Register built-in function signatures ─────────────────────────
    let mut builtin_fn_ids = Vec::with_capacity(ALL_BUILTINS.len());

    for builtin in ALL_BUILTINS {
        register_builtin_fn(ty_ctxt, builtin);
        builtin_fn_ids.push(builtin.id);
    }

    IntrinsicContext {
        lang_items,
        builtin_fn_ids,
    }
}

/// Resolve a [`BuiltinTy`] descriptor to a real interned [`ty::Ty`].
fn resolve_builtin_ty<'tcx>(ty_ctxt: &'tcx TyCtxt, bty: &BuiltinTy) -> ty::Ty<'tcx> {
    use ty::PrimTy;

    match bty {
        BuiltinTy::Int => ty_ctxt.mk_primitive(PrimTy::Int),
        BuiltinTy::Float => ty_ctxt.mk_primitive(PrimTy::Float),
        BuiltinTy::Bool => ty_ctxt.mk_primitive(PrimTy::Bool),
        BuiltinTy::Char => ty_ctxt.mk_primitive(PrimTy::Char),
        BuiltinTy::Str => ty_ctxt.mk_primitive(PrimTy::Str),
        BuiltinTy::Unit => ty_ctxt.mk_unit(),
        BuiltinTy::Never => ty_ctxt.mk_never(),
        // For generic / polymorphic builtins we use an inference variable
        // that will be unified during type checking.
        BuiltinTy::Any => ty_ctxt.mk_infer(),
    }
}

/// Register a single built-in function's type signature into the
/// [`TyCtxt`]. The resulting `Fn` type is interned but not (yet)
/// associated with a `LocalDefId` – that mapping will be established
/// by the resolver when it creates synthetic definitions for builtins.
fn register_builtin_fn<'tcx>(ty_ctxt: &'tcx TyCtxt, builtin: &BuiltinFn) {
    let param_tys: Vec<ty::Ty<'tcx>> = builtin
        .params
        .iter()
        .map(|p| resolve_builtin_ty(ty_ctxt, &p.ty))
        .collect();
    let ret_ty = resolve_builtin_ty(ty_ctxt, &builtin.ret);

    // Intern the function type so it's ready for later lookup.
    let _fn_ty = ty_ctxt.mk_fn(&param_tys, ret_ty);
}
