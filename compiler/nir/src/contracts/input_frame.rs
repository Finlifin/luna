//! Structural admission of a callable's ordinary input bindings. This observes
//! declarations and defaults, never the executable body or invocation values.
use super::CallableProgram;
use crate::{Parameter, Term, program::SourceRange};
use std::collections::{BTreeMap, BTreeSet};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputFrameErrorKind {
    Signature,
    Arity,
    DuplicateBinding,
    DefaultSlot,
    DefaultScope,
    UnavailableDefaultBinding,
    Budget,
    DefaultBudget,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputFrameError {
    pub source: SourceRange,
    pub kind: InputFrameErrorKind,
}

impl InputFrameError {
    pub fn is_budget(self) -> bool {
        matches!(
            self.kind,
            InputFrameErrorKind::Budget | InputFrameErrorKind::DefaultBudget
        )
    }

    pub fn message(self, quote: bool) -> &'static str {
        use InputFrameErrorKind::*;
        match (self.kind, quote) {
            (Signature, false) => "function parameter frame has no callable signature",
            (Signature, true) => "quote template parameter frame has no callable signature",
            (Arity, false) => "function parameter frame arity mismatch",
            (Arity, true) => "quote template parameter frame arity mismatch",
            (DuplicateBinding, false) => "duplicate function parameter identity",
            (DuplicateBinding, true) => "duplicate quote template parameter identity",
            (DefaultSlot, false) => "invalid function parameter default slot",
            (DefaultSlot, true) => "invalid quote template parameter default slot",
            (DefaultScope, _) => "invalid function default binding scope",
            (UnavailableDefaultBinding, _) => {
                "function default references an unavailable parameter binding"
            }
            (Budget, _) => "function parameter frame validation budget exhausted",
            (DefaultBudget, _) => "function default binding analysis budget exhausted",
        }
    }
}

impl std::fmt::Display for InputFrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} at {}..{}",
            self.message(false),
            self.source.start,
            self.source.end
        )
    }
}
impl std::error::Error for InputFrameError {}

impl<'a> CallableProgram<'a> {
    /// Admit parameter identity, arity and default visibility without requiring
    /// a body. Success does not check types, predicates, phases or executable
    /// code. The returned telescope is borrowed from this exact declaration.
    pub fn validate_input_frame(
        self,
        fuel: &mut usize,
    ) -> Result<&'a [Parameter], InputFrameError> {
        use InputFrameErrorKind as K;
        let failure = |kind| InputFrameError {
            source: self.parts.source,
            kind,
        };
        *fuel = fuel
            .checked_sub(self.parts.parameters.len().saturating_add(1))
            .ok_or_else(|| failure(K::Budget))?;
        *fuel = fuel
            .checked_sub(self.parts.defaults.len())
            .ok_or_else(|| failure(K::Budget))?;
        crate::type_computations::charge_term(self.signature, fuel)
            .map_err(|_| failure(K::Budget))?;
        let Term::Function { parameters, .. } = self.signature.callable_signature() else {
            return Err(failure(K::Signature));
        };
        if parameters.len() != self.parts.parameters.len() {
            return Err(failure(K::Arity));
        }
        let mut positions = BTreeMap::new();
        for (slot, binding) in self.parts.parameters.iter().enumerate() {
            if positions.insert(*binding, slot).is_some() {
                return Err(failure(K::DuplicateBinding));
            }
        }
        let mut defaults = BTreeSet::new();
        if self
            .parts
            .defaults
            .iter()
            .any(|(slot, _)| *slot >= parameters.len() || !defaults.insert(*slot))
        {
            return Err(failure(K::DefaultSlot));
        }
        for (slot, value) in self.parts.defaults {
            let free =
                crate::free_bindings::collect(value, fuel).map_err(|reason| InputFrameError {
                    source: value.source,
                    kind: match reason {
                        crate::free_bindings::Error::Budget(_) => K::DefaultBudget,
                        crate::free_bindings::Error::Invalid(_) => K::DefaultScope,
                    },
                })?;
            if free.iter().any(|binding| {
                positions
                    .get(binding)
                    .is_none_or(|position| position >= slot)
            }) {
                return Err(InputFrameError {
                    source: value.source,
                    kind: K::UnavailableDefaultBinding,
                });
            }
        }
        Ok(parameters)
    }
}
