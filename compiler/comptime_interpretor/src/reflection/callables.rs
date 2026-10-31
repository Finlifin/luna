//! Callable type observation; no callable value or captured storage is inspected.
use super::*;

/// The original subject and its admitted callable view. Opaque normal-form heads
/// are not unfolded. Closure environments, phase and unsafe responsibility stay
/// in `callable`; it is not an identity-preserving coercion from `subject`.
///
/// A signature is a complete telescope. Individual parameter/result terms may
/// refer to earlier binders and must not be treated as closed standalone Types.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallableReflection {
    pub subject: Term,
    pub callable: Term,
}
impl CallableReflection {
    pub fn signature(&self) -> &Term {
        self.callable.callable_signature()
    }
    pub fn is_comptime(&self) -> bool {
        self.callable.is_comptime_callable()
    }
    pub fn is_unsafe(&self) -> bool {
        self.callable.is_unsafe_callable()
    }
}
impl fmt::Display for CallableReflection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(callable-reflection (subject {}) (callable {}))",
            self.subject, self.callable
        )
    }
}

impl Interpreter<'_> {
    /// Observe a callable Type in this invocation's declaration world. Already
    /// selected associated views may be used; this does not search implementations,
    /// execute a function or expose names/defaults absent from its function Type.
    pub fn reflect_callable(&mut self, ty: &Term) -> Result<CallableReflection, Error> {
        if !self.comptime_only {
            return Err(self.failure(ErrorKind::PhaseMismatch, None));
        }
        let callable = self.reflection_type(ty)?;
        if !matches!(callable.callable_signature(), Term::Function { .. }) {
            return Err(self.failure(ErrorKind::Type, None));
        }
        let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let copied = nir::copy_budget::charge_type_substitution(ty, &[], &[], &mut fuel);
        self.budget.steps -= (before - fuel) as u64;
        copied.map_err(|_| self.failure(ErrorKind::StepsExhausted, None))?;
        Ok(CallableReflection {
            subject: ty.clone(),
            callable,
        })
    }
}

#[cfg(test)]
mod tests;
