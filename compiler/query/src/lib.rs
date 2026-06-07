//! Luna query system — generic infrastructure layer.
//!
//! This crate is the **infrastructure** tier of Luna's query system,
//! analogous to `rustc_query_system`.  It re-exports the
//! [salsa](https://crates.io/crates/salsa) framework and defines the
//! [`LunaDbBase`] marker super-trait.
//!
//! # Crate responsibility split
//!
//! | Crate       | Responsibility                                          |
//! |-------------|----------------------------------------------------------|
//! | **query**   | Generic infra: salsa re-export, `LunaDbBase` marker     |
//! | `middle`    | Concrete queries: `LunaDatabase` trait + `Db` struct    |
//! | `interface` | Coordinator: owns `Db`, wires up passes                 |
//!
//! Analysis passes that only need to *call* queries import `middle`;
//! crates that need raw salsa primitives (e.g. custom `#[salsa::input]`
//! structs) import `query` directly.

/// Re-export salsa so downstream crates can write `query::salsa::…`
/// or use salsa proc-macro attributes via `use query::salsa`.
pub use salsa;

/// Marker super-trait for all Luna query databases.
///
/// Every concrete database struct (currently only `middle::Db`) must
/// implement this trait in addition to [`salsa::Database`].  It acts as
/// a single extension point for Luna-wide database invariants that are
/// too generic for `middle` but too domain-specific for plain salsa.
pub trait LunaDbBase: salsa::Database + Send {}
