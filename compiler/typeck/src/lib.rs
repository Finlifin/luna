//! # Type checking for the Luna/Flurry compiler.
//!
//! This crate implements type checking and type inference for the HIR.
//! It traverses `Package<'hir>` and assigns `Ty<'tcx>` to every HIR
//! expression, pattern, and definition.
//!
//! ## Architecture
//!
//! ```text
//!   Package<'hir>  (from ast_lowering)
//!       ↓
//!   TypeChecker  (this crate)
//!       ├── resolve type annotations  (HIR Expr → Ty)
//!       ├── infer expression types    (literal, binop, call, etc.)
//!       └── register types in TyCtxt  (side tables)
//!       ↓
//!   TyCtxt  (populated with def_types + node_types)
//! ```

mod check;
mod resolve_ty;

pub use check::typeck_package;
