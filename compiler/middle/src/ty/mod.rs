//! Semantic type system.
//!
//! Re-exports everything from the sub-modules so callers can write
//! `use middle::ty::Ty` or `use middle::Ty` (via the crate-level
//! re-export).

mod context;
mod interner;
mod types;

pub use context::{CommonTypes, TyCtxt};
pub use interner::TyInterner;
pub use types::*;
