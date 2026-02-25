//! # Semantic type system for the Luna/Flurry compiler.
//!
//! This crate defines the **internal (semantic) representation** of types,
//! which is distinct from the *syntactic* type expressions in [`hir::ty`].
//!
//! ## Key concepts
//!
//! | Concept       | Type            | Description                                      |
//! |---------------|-----------------|--------------------------------------------------|
//! | Semantic type  | [`Ty`]         | A thin, `Copy` pointer into the interning arena  |
//! | Type kind      | [`TyKind`]     | The payload: what a type *actually is*            |
//! | Type context   | [`TyCtxt`]     | Owns the interning arena; creates & retrieves Ty |
//!
//! ## Architecture
//!
//! ```text
//!   TyCtxt  (owned by CompilerInstance)
//!   ├── arena: TypedArena<TyKind>     ← backing memory
//!   ├── interner: FxHashSet<Ty>       ← deduplication table
//!   └── common_types: CommonTypes     ← cached primitives / Unit / Never
//! ```
//!
//! All [`Ty`] values are interned: structurally identical types share the
//! same allocation. This makes type equality a **pointer comparison** and
//! makes `Ty` both `Copy` and `Hash`.
//!
//! The `'tcx` lifetime ties every `Ty<'tcx>` to the [`TyCtxt`] that
//! created it, analogous to how `'hir` ties HIR nodes to the
//! [`HirArena`](hir::HirArena).

mod context;
mod interner;
mod types;

pub use context::TyCtxt;
pub use interner::TyInterner;
pub use types::*;
