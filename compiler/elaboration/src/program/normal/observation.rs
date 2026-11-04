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
            error => Self::Invalid(error.to_string()),
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
