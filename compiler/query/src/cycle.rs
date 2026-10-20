//! Cycle detection diagnostics.

use std::fmt;

use crate::stack::ActiveQuery;

/// A query cycle: some `InProgress` query was re-entered.
#[derive(Clone, Debug)]
pub struct CycleError {
    /// Active stack at the moment the cycle was detected, including the
    /// re-entrant frame as the last element.
    pub frames: Vec<ActiveQuery>,
}

impl CycleError {
    pub fn query_names(&self) -> impl Iterator<Item = &str> {
        self.frames.iter().map(|f| f.desc.name)
    }
}

impl fmt::Display for CycleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "query cycle:")?;
        for (i, frame) in self.frames.iter().enumerate() {
            writeln!(f, "  #{i}: {}({})", frame.desc.name, frame.key)?;
        }
        Ok(())
    }
}

impl std::error::Error for CycleError {}
