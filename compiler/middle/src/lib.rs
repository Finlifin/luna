//! Session-bound HIR and mid-end queries with owned query results.
//!
//! [`Db`] connects source inputs to registered lowering and elaboration
//! providers and preserves invalidation through the generic [`query`] engine.
//! [`HirPackageBox`] owns each returned HIR package and its arena independently
//! of the database lifetime.

pub mod hir_package;
pub mod hir_query;
pub mod package_interface;
pub mod queries;

pub use ast;
pub use hir_package::HirPackageBox;
pub use hir_query::HirQueryInput;
pub use package_interface::{
    ElaboratedPackage, ElaborationQueryInput, ItemInterfaceSummary, ItemKindSummary,
    PackageInterface, PackageTraitImpls,
};
pub use queries::{Db, LunaDatabase, Providers, QueryCaches};
pub use resolve;
