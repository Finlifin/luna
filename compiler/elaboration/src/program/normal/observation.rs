//! Outcomes of bounded static observation, distinct from a negative verdict.
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::program) enum Failure {
    Unavailable,
    Steps,
    Depth,
    Size,
    Structure,
    Proof,
    Evaluation(String),
    Invalid(String),
}
impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => f.write_str("(observation-unavailable)"),
            Self::Steps => f.write_str("(observation-limit steps)"),
            Self::Depth => f.write_str("(observation-limit depth)"),
            Self::Size => f.write_str("(observation-limit size)"),
            Self::Structure => f.write_str("(observation-limit structure)"),
            Self::Proof => f.write_str("(observation-limit proof)"),
            Self::Evaluation(message) => write!(f, "(observation-evaluation {message:?})"),
            Self::Invalid(message) => write!(f, "(observation-invalid {message:?})"),
        }
    }
}
impl From<comptime_runtime::Error> for Failure {
    fn from(error: comptime_runtime::Error) -> Self {
        use comptime_runtime::Error as E;
        match error {
            E::BudgetExhausted => Self::Steps,
            E::SizeLimit => Self::Size,
            E::DivisionByZero | E::OutOfRange => Self::Evaluation(error.to_string()),
            E::InvalidLiteral => Self::Invalid(error.to_string()),
        }
    }
}
impl From<comptime_runtime::bitvec::Error> for Failure {
    fn from(error: comptime_runtime::bitvec::Error) -> Self {
        match error {
            comptime_runtime::bitvec::Error::Resource(error) => error.into(),
            error => Self::Invalid(error.to_string()),
        }
    }
}
impl From<nir::copy_budget::CopyBudgetExhausted> for Failure {
    fn from(error: nir::copy_budget::CopyBudgetExhausted) -> Self {
        match error {
            nir::copy_budget::CopyBudgetExhausted::Steps => Self::Steps,
            nir::copy_budget::CopyBudgetExhausted::Depth => Self::Depth,
        }
    }
}

impl From<nir::free_bindings::Error> for Failure {
    fn from(error: nir::free_bindings::Error) -> Self {
        match error {
            nir::free_bindings::Error::Budget(_) => Self::Steps,
            nir::free_bindings::Error::Invalid(message) => Self::Invalid(message.into()),
        }
    }
}

impl From<nir::type_computations::Error> for Failure {
    fn from(error: nir::type_computations::Error) -> Self {
        match error {
            nir::type_computations::Error::Budget => Self::Structure,
            error => Self::Invalid(error.to_string()),
        }
    }
}
impl From<nir::comptime_values::Error> for Failure {
    fn from(error: nir::comptime_values::Error) -> Self {
        match error {
            nir::comptime_values::Error::Budget => Self::Structure,
            error => Self::Invalid(error.to_string()),
        }
    }
}

/// A hint reads checked code ahead of execution. A value-dependent trap
/// supplies no identity on that path. Resource and representation errors still
/// belong to the observation, even when its caller can use partial evidence.
pub(in crate::program) fn hint<T>(result: Result<T, Failure>) -> Result<Option<T>, Failure> {
    match result {
        Err(Failure::Evaluation(_)) => Ok(None),
        other => optional(other),
    }
}

/// Checking may defer an unavailable value, but a failed observation is not
/// permission to retry with invocation or to publish a weaker obligation.
pub(in crate::program) fn optional<T>(result: Result<T, Failure>) -> Result<Option<T>, Failure> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(Failure::Unavailable) => Ok(None),
        Err(failure) => Err(failure),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_faults_can_wait_for_invocation_while_limits_stop_observation() {
        for error in [
            comptime_runtime::Error::DivisionByZero,
            comptime_runtime::Error::OutOfRange,
        ] {
            let failure = Failure::from(error);
            assert_eq!(hint::<u8>(Err(failure.clone())), Ok(None));
            assert_eq!(optional::<u8>(Err(failure.clone())), Err(failure));
        }
        for error in [
            comptime_runtime::Error::BudgetExhausted,
            comptime_runtime::Error::SizeLimit,
            comptime_runtime::Error::InvalidLiteral,
        ] {
            let failure = Failure::from(error);
            assert_eq!(hint::<u8>(Err(failure.clone())), Err(failure));
        }
    }

    #[test]
    fn only_unavailable_observation_allows_deferred_checking() {
        assert_eq!(optional(Ok(42)), Ok(Some(42)));
        assert_eq!(optional::<u8>(Err(Failure::Unavailable)), Ok(None));
        for failure in [
            Failure::Steps,
            Failure::Depth,
            Failure::Size,
            Failure::Structure,
            Failure::Proof,
            Failure::Invalid("bad value".into()),
        ] {
            assert_eq!(optional::<u8>(Err(failure.clone())), Err(failure));
        }
    }
}
