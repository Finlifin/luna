//! NIR program verification: control flow, memory views, ownership and effects.
//! Type propositions remain in PTC; program facts use verification_core.
pub mod contracts;
pub mod control;
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
pub mod purity;
mod rejection;
pub use verification_core::*;

use type_resolution::{associated, capabilities, implementation, ptc};
