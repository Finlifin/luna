//! `middle` — the central crate of the Luna compiler.
//!
//! Analogous to `rustc_middle`. Contains:
//!
//! - [`ty`] — the semantic type system (`Ty`, `TyKind`, `TyCtxt`, …).
//! - [`queries`] — the salsa query-group trait, the concrete [`Db`], and
//!   all query provider stubs.
//!
//! # Dependency position
//!
//! ```text
//!   symbol, hir, query          ← foundational crates
//!               ↑
//!            middle              ← THIS CRATE
//!               ↑
//!   typeck, mir_build, …        ← analysis passes
//!               ↑
//!           interface            ← compiler coordinator
//! ```
//!
//! Every analysis pass that needs the type context or wants to call
//! a query depends on `middle`, not on `interface`.

pub mod hir_package;
pub mod hir_query;
pub mod queries;
pub mod ty;

// Convenience re-exports for the most commonly used items.
pub use hir_package::HirPackageBox;
pub use hir_query::HirQueryInput;
pub use queries::{Db, LunaDatabase, Providers};
pub use ty::{
    AdtDef, CommonTypes, FieldDef, InferTy, NFId, PrimTy, Ty, TyCtxt, TyInterner, TyKind,
};
