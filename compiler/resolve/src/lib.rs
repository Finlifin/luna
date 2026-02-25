//! # Early Name Resolution
//!
//! This crate implements **early name resolution** for the Flurry compiler.
//! It runs after parsing (AST construction) and before HIR lowering, building
//! the scope tree and resolving `use` imports so that every name in the program
//! maps to a unique [`DefId`].
//!
//! ## Architecture
//!
//! ```text
//!   resolve crate
//!   ├── ids        – DefId, NodeId, ScopeId and related identifiers
//!   ├── namespace  – Namespace enum and PerNs resolution bucket
//!   ├── binding    – Binding / Resolution descriptors
//!   ├── scope      – Scope tree nodes
//!   ├── rib        – Rib & RibStack for lexical resolution
//!   ├── import     – ImportDirective and import-resolution logic
//!   ├── item_scope – Per-scope item/import collection (the "namespace manager")
//!   ├── scanner    – VFS scanner + AST scanner (migrated from luna/src/scan)
//!   ├── resolver   – The main Resolver orchestrator
//!   └── error      – ResolveError diagnostics
//! ```
//!
//! The public entry point is [`Resolver::new`] followed by [`Resolver::resolve_package`].

pub mod binding;
pub mod error;
pub mod ids;
pub mod import;
pub mod item_scope;
pub mod namespace;
pub mod resolver;
pub mod rib;
pub mod scanner;
pub mod scope;

pub use binding::{Binding, BindingKind, Resolution};
pub use error::{ResolveError, ResolveResult};
pub use ids::{DefId, ModuleId, ScopeId};
pub use import::{ImportDirective, ImportKind};
pub use item_scope::ItemScope;
pub use namespace::{Namespace, PerNs};
pub use resolver::Resolver;
pub use rib::{Rib, RibKind, RibStack};
pub use scope::Scope;
