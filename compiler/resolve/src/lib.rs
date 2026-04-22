//! # Early Name Resolution
//!
//! This crate implements **early name resolution** for the Flurry compiler.
//! It runs after parsing (AST construction) and before HIR lowering.
//!
//! ## Two-phase design
//!
//! 1. **Build phase** ([`build_module_tree`]) – scans the VFS, parses `.fl`
//!    files, constructs the scope tree, and resolves `use` imports in a
//!    fixpoint loop.  Produces a [`ModuleTree`].
//!
//! 2. **Query phase** ([`Resolver`]) – borrows the immutable `ModuleTree`
//!    and answers name/path resolution queries from the AST lowering pass.
//!    Unresolvable names are reported as errors immediately.
//!
//! ## Architecture
//!
//! ```text
//!   resolve crate
//!   ├── ids            – DefId, ScopeId, ModuleId and related identifiers
//!   ├── binding        – Binding / Resolution descriptors
//!   ├── scope          – Scope tree nodes
//!   ├── rib            – Rib & RibStack for lexical resolution
//!   ├── import         – ImportDirective / ResolvedImport types
//!   ├── item_scope     – Per-scope item/import collection
//!   ├── scanner        – VFS scanner + AST scanner
//!   ├── module_builder – Build phase: scope tree construction + import resolution
//!   ├── resolver       – Query phase: name resolution for AST lowering
//!   └── error          – ResolveError diagnostics
//! ```
//!
//! The public entry point is [`build_module_tree`] followed by
//! [`Resolver::new`].

pub mod binding;
pub mod error;
pub mod ids;
pub mod import;
pub mod item_scope;
pub mod module_builder;
pub mod resolver;
pub mod rib;
pub mod scanner;
pub mod scope;

pub use binding::{Binding, BindingKind, Resolution};
pub use error::{ResolveError, ResolveResult};
pub use ids::{DefId, ModuleId, ScopeId};
pub use import::{ImportDirective, ImportKind};
pub use item_scope::ItemScope;
pub use module_builder::{ModuleTree, build_module_tree};
pub use resolver::Resolver;
pub use rib::{Rib, RibKind, RibStack};
pub use scope::Scope;
