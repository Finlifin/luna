//! Active query stack for cycle detection and diagnostics.

use std::fmt;

/// Static description of a query kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct QueryDesc {
    /// Stable name used in diagnostics (e.g. `"hir_package"`).
    pub name: &'static str,
}

impl QueryDesc {
    pub const fn new(name: &'static str) -> Self {
        QueryDesc { name }
    }
}

/// One frame on the active query stack.
#[derive(Clone, Debug)]
pub struct ActiveQuery {
    pub desc: QueryDesc,
    /// Human-readable key (already formatted).
    pub key: String,
}

impl fmt::Display for ActiveQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}({})", self.desc.name, self.key)
    }
}

/// Stack of currently-evaluating queries (outermost first).
#[derive(Clone, Debug, Default)]
pub struct QueryStack {
    frames: Vec<ActiveQuery>,
}

impl QueryStack {
    pub fn new() -> Self {
        QueryStack { frames: Vec::new() }
    }

    pub fn push(&mut self, frame: ActiveQuery) {
        self.frames.push(frame);
    }

    pub fn pop(&mut self) -> Option<ActiveQuery> {
        self.frames.pop()
    }

    pub fn frames(&self) -> &[ActiveQuery] {
        &self.frames
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }
}
