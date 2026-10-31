//! Exact compile-time values and bounded operations.
//!
//! This crate does not authorize comptime execution, select a runtime proxy,
//! or implement a language operator protocol. Callers must establish those
//! contracts before invoking the value operations.
pub mod decimal;
pub mod float16;
pub mod integer;
pub use integer::{Budget, Error, Integer};
pub mod bitvec;
pub mod machine;
pub mod object;
