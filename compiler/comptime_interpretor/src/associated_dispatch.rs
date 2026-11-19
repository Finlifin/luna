//! Check that each concrete member supply comes from its selected implementation.
use super::*;
use nir::associated_initializers::MemberSource;
use nir::program::{DispatchInstance, Implementation, Trait};

pub(super) fn validate<'a>(
    instance: &DispatchInstance,
    implementation: &Implementation,
    implementation_inputs: &nir::DeclarationSubstitution<'_>,
    contract: &Trait,
    values: &associated_catalog::Catalog<'_>,
    package: &Package,
    head: &impl Fn(DeclarationId) -> Option<&'a nir::normal_forms::Head>,
    fuel: &mut usize,
) -> Result<(), Error> {
    if instance.associated.is_empty() && instance.associated_initializers.is_empty() {
        return Ok(());
    }
    let inputs =
        nir::member_types::arguments(&instance.subject, &instance.interface_arguments, &[], fuel)
            .map_err(constructor_premises::failure)?;
    let interface_inputs = contract
        .input_substitution(&inputs, &[], fuel)
        .map_err(constructor_premises::failure)?;
    let members = instance
        .associated
        .iter()
        .map(|(member, expr)| (member, MemberSource::Value(expr)))
        .chain(
            instance
                .associated_initializers
                .iter()
                .map(|(member, init)| (member, MemberSource::Initializer(init))),
        );
    for (member, supplied) in members {
        let value = supplied.expression();
        let source = Some(value.source);
        let invalid = || Error {
            kind: ErrorKind::InvalidCallPlan,
            source,
        };
        let failure = |reason: nir::NominalAdmissionError| Error {
            kind: match reason {
                nir::NominalAdmissionError::Budget => ErrorKind::StepsExhausted,
                nir::NominalAdmissionError::Unresolved(_) => ErrorKind::TypeContractUnresolved,
                nir::NominalAdmissionError::Invalid(_) => ErrorKind::InvalidCallPlan,
            },
            source,
        };
        let declaration = contract
            .associated
            .iter()
            .find(|item| item.declaration == *member)
            .ok_or_else(invalid)?;
        if declaration.phase == nir::program::TraitPhase::Comptime
            && !implementation.phases.comptime
        {
            return Err(invalid());
        }
        let expected = interface_inputs
            .instantiate_bounded(&declaration.ty, fuel)
            .map_err(|e| failure(e.into()))?;
        if !selected_types::same(&value.ty, &expected, values, fuel)
            .map_err(|kind| Error { kind, source })?
        {
            return Err(invalid());
        }
        let original = implementation
            .associated_source(*member, fuel)
            .map_err(failure)?;
        let template = match (original, supplied) {
            (Some(MemberSource::Initializer(template)), MemberSource::Initializer(_)) => {
                template.expression()
            }
            (Some(MemberSource::Value(template)), MemberSource::Value(data)) => {
                for expression in [data, template] {
                    nir::completed_values::validate_in_layouts(expression, package, head, fuel)
                        .map_err(failure)?;
                }
                template
            }
            _ => return Err(invalid()),
        };
        let specialized = implementation_inputs
            .instantiate_expression_bounded(template, fuel)
            .map_err(|reason| match reason {
                nir::ExpressionInstantiationError::Copy(
                    nir::copy_budget::CopyBudgetExhausted::Depth,
                ) => Error {
                    kind: ErrorKind::DepthExhausted,
                    source,
                },
                reason => failure(nir::type_computations::Error::from(reason).into()),
            })?;
        // Selection may expose the normalized result type. Its source and
        // executable tree (including every child annotation) still come from
        // this exact implementation under its selected declaration arguments.
        if specialized.source != value.source
            || specialized.kind != value.kind
            || !selected_types::same(&specialized.ty, &value.ty, values, fuel)
                .map_err(|kind| Error { kind, source })?
        {
            return Err(invalid());
        }
    }
    Ok(())
}
