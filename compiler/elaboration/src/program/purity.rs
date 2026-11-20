//! Source diagnostics and resource identities for NIR behavior verification.
use super::{Error, Protocols, n};
use crate::verification::purity;
pub(super) use purity::{ObservationScope, StableObservation};

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
        retry_after_capability_progress: false,
        incompatible_context: false,
    }
}
/// Open witnesses have their own declarations. A closed source and its
/// residual share an identity, so check both worlds without duplicate IDs.
/// The executable package is restored on success and on diagnostic return.
pub(super) fn with_definition_bodies(
    package: &mut n::Package,
    witnesses: &mut Vec<n::Function>,
    mut check: impl FnMut(&n::Package) -> Result<(), Error>,
) -> Result<(), Error> {
    if witnesses.is_empty() {
        return check(package);
    }
    let initial = package.functions.len();
    let slots: std::collections::BTreeMap<_, _> = package
        .functions
        .iter()
        .enumerate()
        .map(|(index, function)| (function.declaration, index))
        .collect();
    let mut overlays = Vec::new();
    for witness in witnesses.drain(..) {
        if let Some(&index) = slots.get(&witness.declaration) {
            overlays.push((index, witness));
        } else {
            package.functions.push(witness);
        }
    }
    let mut result = check(package);
    if result.is_ok() && !overlays.is_empty() {
        for (index, body) in &mut overlays {
            std::mem::swap(&mut package.functions[*index], body);
        }
        result = check(package);
        for (index, body) in &mut overlays {
            std::mem::swap(&mut package.functions[*index], body);
        }
    }
    witnesses.extend(package.functions.drain(initial..));
    witnesses.extend(overlays.into_iter().map(|(_, witness)| witness));
    result
}

pub(super) fn validate_runtime_promises(
    package: &n::Package,
    identities: &Protocols,
) -> Result<(), Error> {
    purity::validate_runtime_promises(package, &protocols(identities)).map_err(diagnostic)
}
pub(super) fn validate_defaults(
    package: &n::Package,
    obligations: &[StableObservation],
    identities: &Protocols,
) -> Result<(), Error> {
    purity::validate_defaults(package, obligations, &protocols(identities)).map_err(diagnostic)
}

pub(super) fn validate_predicates(
    package: &n::Package,
    obligations: &[StableObservation],
    identities: &Protocols,
) -> Result<(), Error> {
    // Both consumers need a stable, finitely completing observation. Reuse
    // the behavior proof, not a source `pure` marker or a bool result type.
    purity::validate_defaults(
        package,
        obligations,
        &protocols(identities),
    )
    .map_err(|mut failure| {
        failure.message = "assumes requires a supported effect-free program predicate; call stability or finite completion could not be certified".into();
        diagnostic(failure)
    })
}
