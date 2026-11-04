//! Session-bound HIR queries and owned query results.
//!
//! [`Db`] connects source inputs to registered lowering providers and preserves
//! invalidation through the generic [`query`] engine. [`HirPackageBox`] owns
//! each returned package and its arena independently of the database lifetime.

pub mod hir_package;
pub mod hir_query;
pub mod queries;

pub use hir_package::HirPackageBox;
pub use hir_query::HirQueryInput;
pub use queries::{Db, LunaDatabase, Providers, QueryCaches};
