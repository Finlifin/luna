//! Source diagnostics and resource identities for NIR behavior verification.
use super::{Error, Protocols, Span, n, source};
use crate::verification::purity;
use std::collections::BTreeSet;

fn protocols(value: &Protocols) -> purity::Protocols {
    purity::Protocols {
        copy: value.copy,
        drop: value.drop,
    }
}
fn diagnostic(failure: purity::Error) -> Error {
    Error {
        source: failure.source,
        message: failure.message,
        notes: Vec::new(),
        needs_context: false,
        needs_tactic: false,
        needs_capability: false,
        incompatible_context: false,
    }
}
pub(super) fn validate_runtime_promises(
    package: &n::Package,
    identities: &Protocols,
) -> Result<(), Error> {
    purity::validate_runtime_promises(package, &protocols(identities)).map_err(diagnostic)
}
pub(super) fn validate_defaults(
    package: &n::Package,
    obligations: &[(n::Expr, Span, BTreeSet<n::LocalId>)],
    identities: &Protocols,
) -> Result<(), Error> {
    purity::validate_defaults(
        package,
        obligations
            .iter()
            .map(|(value, span, staged)| (value, source(*span), staged)),
        &protocols(identities),
    )
    .map_err(diagnostic)
}

pub(super) fn validate_predicates(
    package: &n::Package,
    obligations: &[(n::Expr, Span, BTreeSet<n::LocalId>)],
    identities: &Protocols,
) -> Result<(), Error> {
    // Both consumers need a stable, finitely completing observation. Reuse
    // the behavior proof, not a source `pure` marker or a bool result type.
    purity::validate_defaults(
        package,
        obligations.iter().map(|(value, span, staged)| (value, source(*span), staged)),
        &protocols(identities),
    )
    .map_err(|mut failure| {
        failure.message = "assumes requires a supported effect-free program predicate; call stability or finite completion could not be certified".into();
        diagnostic(failure)
    })
}
