//! Host binding of target operations before resource preparation.
pub use nir::intrinsics::{Checked, Error, Operation, Registry, Tag};

/// Consume a private package transaction. On error no partially bound artifact
/// is returned. This pass supplies target behavior, not source unsafe permission.
pub fn bind(
    mut package: nir::program::Package,
    registry: &Registry,
    fuel: &mut usize,
) -> Result<nir::program::Package, String> {
    // Recheck family identities in this package; the registry may have been
    // constructed from another snapshot with identical session handles.
    for function in package.functions.iter().chain(&package.quote_templates) {
        nir::copy_budget::charge_function_substitution(function, &[], &[], fuel)
            .map_err(|_| "intrinsic binding copy budget exhausted")?;
        nir::visit::expressions(function, fuel, |expr, fuel| {
            if let nir::program::ExprKind::Intrinsic { tag, .. } = &expr.kind {
                *fuel = fuel
                    .checked_sub(package.enums.len())
                    .ok_or("intrinsic family lookup budget exhausted")?;
                nir::intrinsics::validate_tag(&package, *tag).map_err(str::to_owned)?;
            }
            Ok(())
        })?;
    }
    for function in package
        .functions
        .iter_mut()
        .chain(&mut package.quote_templates)
    {
        for root in std::iter::once(&mut function.body)
            .chain(&mut function.preconditions)
            .chain(&mut function.postconditions)
            .chain(function.defaults.iter_mut().map(|(_, value)| value))
        {
            nir::visit::expression_mut(root, fuel, |expr, _| {
                if let nir::program::ExprKind::Intrinsic {
                    tag,
                    inputs,
                    binding,
                } = &mut expr.kind
                {
                    let types: Vec<_> = inputs.iter().map(|input| input.ty.clone()).collect();
                    *binding = match registry.check(*tag, &types, &expr.ty) {
                        Ok(checked) => Some(checked),
                        Err(Error::OutputType(_)) => {
                            // Source elaboration may already have proved the
                            // operation type domain of an open generic signature.
                            // Retain only exactly that pending evidence. A
                            // driver rebind cannot manufacture its PTC proof or
                            // transfer it to a different operation/signature.
                            let pending =
                                registry.defer(*tag, &types, &expr.ty).map_err(|error| {
                                    format!("{}..{}: {error}", expr.source.start, expr.source.end)
                                })?;
                            if binding.as_ref() != Some(&pending) {
                                return Err(format!(
                                    "{}..{}: generic intrinsic rebinding requires matching source admission",
                                    expr.source.start, expr.source.end
                                ));
                            }
                            Some(pending)
                        }
                        // Unreachable unsupported operations need no target
                        // implementation. If materialized, they still fail.
                        // Rebinding must not retain an older registry's proof.
                        Err(Error::UnsupportedTag(_)) => None,
                        Err(e) => {
                            return Err(format!("{}..{}: {e}", expr.source.start, expr.source.end));
                        }
                    };
                }
                Ok(())
            })?;
        }
    }
    nir::phases::validate(&package).map_err(|e| e.message.to_owned())?;
    Ok(package)
}
