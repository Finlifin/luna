//! Explicit target-dependent scalar representation facts.
//!
//! These facts describe storage, not type equality: `usize` remains `usize`
//! even on a target where its representation matches `u64`.
use crate::Primitive;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TargetData {
    pointer_bits: u32,
}

impl TargetData {
    /// Current machine-integer values support at most 128 bits. Reject other
    /// widths instead of truncating a target fact to the host's representation.
    pub fn new(pointer_bits: u32) -> Result<Self, &'static str> {
        if pointer_bits == 0 || pointer_bits > 128 || pointer_bits % 8 != 0 {
            return Err("unsupported target pointer width (expected byte-aligned 8..=128 bits)");
        }
        Ok(Self { pointer_bits })
    }

    pub fn pointer_bits(self) -> u32 {
        self.pointer_bits
    }

    pub fn integer_layout(self, primitive: Primitive) -> Option<(bool, u32)> {
        match primitive {
            Primitive::Isize => Some((true, self.pointer_bits)),
            Primitive::Usize => Some((false, self.pointer_bits)),
            other => other.fixed_integer_layout(),
        }
    }
}

impl std::fmt::Display for TargetData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(target-data (pointer-bits {}))", self.pointer_bits)
    }
}
