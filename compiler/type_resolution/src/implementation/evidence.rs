//! Replay failures belong to declaration/world evidence, not solely to PTC.
use super::TypeAssessment;
use crate::ptc::ProofError;
use nir::NominalAdmissionError as QueryError;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EvidenceError {
    Query(QueryError),
    Proof(ProofError),
}
impl From<QueryError> for EvidenceError {
    fn from(error: QueryError) -> Self {
        Self::Query(error)
    }
}
impl From<nir::type_computations::Error> for EvidenceError {
    fn from(error: nir::type_computations::Error) -> Self {
        Self::Query(error.into())
    }
}
impl From<ProofError> for EvidenceError {
    fn from(error: ProofError) -> Self {
        Self::Proof(error)
    }
}
impl EvidenceError {
    pub(crate) fn into_assessment(self) -> TypeAssessment {
        match self {
            Self::Query(QueryError::Budget) | Self::Proof(ProofError::BudgetExhausted) => {
                TypeAssessment::BudgetExhausted
            }
            Self::Query(QueryError::Unresolved(reason)) => TypeAssessment::Pending(reason),
            Self::Query(QueryError::Invalid(reason)) => TypeAssessment::Invalid(reason),
            Self::Proof(ProofError::ArithmeticRange) => TypeAssessment::Pending(
                "dependent head replay exceeds the arithmetic proof range".into(),
            ),
            Self::Proof(ProofError::Invalid) => TypeAssessment::Invalid(
                "dependent head evidence failed replay against the original declaration".into(),
            ),
        }
    }
}

impl fmt::Display for EvidenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Query(QueryError::Budget) => {
                f.write_str("(evidence-query-error (budget-exhausted))")
            }
            Self::Query(QueryError::Unresolved(reason)) => {
                write!(f, "(evidence-query-error (unresolved {reason:?}))")
            }
            Self::Query(QueryError::Invalid(reason)) => {
                write!(f, "(evidence-query-error (invalid {reason:?}))")
            }
            Self::Proof(error) => write!(
                f,
                "(evidence-proof-error ({}))",
                match error {
                    ProofError::BudgetExhausted => "budget-exhausted",
                    ProofError::Invalid => "invalid",
                    ProofError::ArithmeticRange => "arithmetic-range",
                }
            ),
        }
    }
}
impl std::error::Error for EvidenceError {}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn display_keeps_query_and_proof_failures_distinct_and_escapes_payloads() {
        assert_eq!(
            EvidenceError::Query(QueryError::Budget).to_string(),
            "(evidence-query-error (budget-exhausted))"
        );
        assert_eq!(
            EvidenceError::Proof(ProofError::BudgetExhausted).to_string(),
            "(evidence-proof-error (budget-exhausted))"
        );
        assert_eq!(
            EvidenceError::Proof(ProofError::ArithmeticRange).to_string(),
            "(evidence-proof-error (arithmetic-range))"
        );
        assert_eq!(
            EvidenceError::Query(QueryError::Unresolved("a\n\"b".into())).to_string(),
            "(evidence-query-error (unresolved \"a\\n\\\"b\"))"
        );
    }
}
