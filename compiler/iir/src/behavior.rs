//! Certified runtime behavior and memory-effect summaries for IIR instances and callsites.
//! Every flag defaults conservatively to `false` and `MemoryEffect::Unknown`; certification
//! passes may only refine behavior when soundness invariants are explicitly proven.
use nir::program::SourceRange;
use std::fmt;

/// Conservative summary of observable memory side effects for LLVM optimization.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MemoryEffect {
    None,
    ArgMemRead,
    ArgMemReadWrite,
    ReadOnly,
    #[default]
    Unknown,
}

impl MemoryEffect {
    /// Compute the least upper bound of two memory effects.
    pub fn join(self, other: Self) -> Self {
        use MemoryEffect as M;
        match (self, other) {
            (M::None, x) | (x, M::None) => x,
            (a, b) if a == b => a,
            (M::ArgMemRead, M::ArgMemReadWrite) | (M::ArgMemReadWrite, M::ArgMemRead) => {
                M::ArgMemReadWrite
            }
            (M::ArgMemRead, M::ReadOnly) | (M::ReadOnly, M::ArgMemRead) => M::ReadOnly,
            _ => M::Unknown,
        }
    }
}

impl fmt::Display for MemoryEffect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::None => "none",
            Self::ArgMemRead => "argmem-read",
            Self::ArgMemReadWrite => "argmem-readwrite",
            Self::ReadOnly => "readonly",
            Self::Unknown => "unknown",
        })
    }
}

/// Conservative LLVM parameter-level alias and provenance attributes for a logical pointer slot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PointerParamAttrs {
    pub readonly: bool,
    pub noalias: bool,
    pub captures_none: bool,
    pub nonnull: bool,
}

impl PointerParamAttrs {
    /// Intersect two proven pointer parameter attribute sets conservatively.
    pub fn join(self, other: Self) -> Self {
        Self {
            readonly: self.readonly && other.readonly,
            noalias: self.noalias && other.noalias,
            captures_none: self.captures_none && other.captures_none,
            nonnull: self.nonnull && other.nonnull,
        }
    }
}

impl fmt::Display for PointerParamAttrs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(pointer-param (readonly {}) (noalias {}) (captures-none {}) (nonnull {}))",
            self.readonly, self.noalias, self.captures_none, self.nonnull
        )
    }
}

/// Certified orthogonal function or callsite behavior attributes mapped to LLVM 23 IR.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct BehaviorAttributes {
    pub memory: MemoryEffect,
    pub terminates: bool,
    pub no_trap_or_unwind: bool,
    pub no_free: bool,
    pub no_sync: bool,
    pub pointer_params: Vec<PointerParamAttrs>,
}

impl BehaviorAttributes {
    /// Fully conservative default matching P0 unknown behavior.
    pub fn conservative() -> Self {
        Self::default()
    }

    pub fn is_conservative(&self) -> bool {
        self.memory == MemoryEffect::Unknown
            && !self.terminates
            && !self.no_trap_or_unwind
            && !self.no_free
            && !self.no_sync
            && self
                .pointer_params
                .iter()
                .all(|param| *param == PointerParamAttrs::default())
    }

    /// Conservatively intersect two behavior summaries (sound across all call targets).
    pub fn join(&self, other: &Self) -> Self {
        let len = self.pointer_params.len().min(other.pointer_params.len());
        let pointer_params = self.pointer_params[..len]
            .iter()
            .zip(&other.pointer_params[..len])
            .map(|(a, b)| a.join(*b))
            .collect();
        Self {
            memory: self.memory.join(other.memory),
            terminates: self.terminates && other.terminates,
            no_trap_or_unwind: self.no_trap_or_unwind && other.no_trap_or_unwind,
            no_free: self.no_free && other.no_free,
            no_sync: self.no_sync && other.no_sync,
            pointer_params,
        }
    }

    /// Degrade attributes when runtime failure state or `@llvm.trap` is accessed.
    pub fn degrade_for_runtime_failure_access(&mut self) {
        self.no_trap_or_unwind = false;
        self.memory = MemoryEffect::Unknown;
    }

    /// Degrade attributes when volatile memory accesses are present.
    pub fn degrade_for_volatile_access(&mut self) {
        self.memory = MemoryEffect::Unknown;
        self.no_sync = false;
    }
}

impl fmt::Display for BehaviorAttributes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(behavior (memory {}) (terminates {}) (no-trap-or-unwind {}) (no-free {}) (no-sync {}) (pointer-params",
            self.memory, self.terminates, self.no_trap_or_unwind, self.no_free, self.no_sync
        )?;
        for param in &self.pointer_params {
            write!(f, " {param}")?;
        }
        f.write_str("))")
    }
}

/// Callsite-specific certified behavior attached for indirect or direct-effect invocations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallsiteBehavior {
    pub source: SourceRange,
    pub behavior: BehaviorAttributes,
}

impl fmt::Display for CallsiteBehavior {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(callsite-behavior (source {}) {})",
            self.source, self.behavior
        )
    }
}
