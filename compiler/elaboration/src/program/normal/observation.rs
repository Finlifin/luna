//! Outcomes of bounded static observation, distinct from a negative verdict.
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::program) enum Failure {
    Unavailable,
    Steps,
    Depth,
    Size,
    Invalid(String),
}
impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => f.write_str("(observation-unavailable)"),
            Self::Steps => f.write_str("(observation-limit steps)"),
            Self::Depth => f.write_str("(observation-limit depth)"),
            Self::Size => f.write_str("(observation-limit size)"),
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
