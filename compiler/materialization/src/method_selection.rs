//! Typed source-selection responses at the VM host boundary.
use super::*;

pub fn resolve_method_for_invocation(
    package: &n::Package,
    callee: &n::Expr,
    context: &Context,
    assumptions: &[n::TraitRequirement],
    remaining: &mut u64,
) -> Result<n::MethodInstance, comptime_interpretor::Error> {
    let before = usize::try_from(*remaining).unwrap_or(usize::MAX);
    let mut fuel = before;
    let result =
        resolve_method_identity_in_context(package, callee, context, assumptions, &mut fuel);
    *remaining -= (before - fuel) as u64;
    result.map_err(|failure| comptime_interpretor::Error {
        kind: match failure {
            nir::NominalAdmissionError::Budget => comptime_interpretor::ErrorKind::StepsExhausted,
            nir::NominalAdmissionError::Unresolved(_) => {
                comptime_interpretor::ErrorKind::TypeContractUnresolved
            }
            nir::NominalAdmissionError::Invalid(_) => {
                comptime_interpretor::ErrorKind::UnmaterializedDispatch
            }
        },
        source: Some(callee.source),
    })
}
