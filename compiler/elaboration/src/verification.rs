//! Bounded mathematical integer reasoning about versioned program values.
//! Independent of PTC; this module grants neither memory access nor ownership.
pub mod contracts;
pub mod copy;
pub mod drop;
mod effect_transfer;
pub mod effect_usage;
pub mod flow;
pub mod memory;
pub mod moves;
mod observation_region;
pub mod optimization;
mod pattern_facts;
mod rejection;
pub use verification_core::*;
