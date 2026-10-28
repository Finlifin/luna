//! Source intrinsic formation, separate from ordinary callable argument routing.
use super::*;

pub(super) fn is_application(expr: &Expr<'_>) -> bool {
    let call = if let H::Cast(call, _) = expr.kind {
        call
    } else {
        expr
    };
    matches!(call.kind, H::Application(callee, _) if matches!(callee.kind, H::TakeIntrinsic(_)))
}

impl Checker<'_, '_> {
    pub(super) fn intrinsic_expression(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let (call, output) = match expr.kind {
            H::Cast(call, target) => (call, self.type_expr(target)?),
            _ => (expr, p(P::Void)),
        };
        let H::Application(callee, arguments) = call.kind else {
            unreachable!()
        };
        let H::TakeIntrinsic(tag_source) = callee.kind else {
            unreachable!()
        };
        let Some((ty, variant)) = self.variant_target(tag_source, None)? else {
            return Err(error(
                tag_source.span,
                "intrinsic requires an enum variant tag",
            ));
        };
        let Term::Nominal {
            declaration,
            arguments: family_arguments,
        } = ty
        else {
            unreachable!()
        };
        if !family_arguments.is_empty() {
            return Err(error(
                tag_source.span,
                "intrinsic tag family instances are not implemented",
            ));
        }
        let tag = nir::intrinsics::Tag {
            family: declaration,
            variant,
        };
        let family = self
            .enums
            .get(&declaration)
            .ok_or_else(|| error(tag_source.span, "unknown intrinsic tag family"))?;
        nir::intrinsics::validate_family(family, tag)
            .map_err(|reason| error(tag_source.span, reason))?;
        let mut inputs = Vec::with_capacity(arguments.len());
        for argument in arguments {
            let Arg::Positional(value) = argument else {
                return Err(error(
                    call.span,
                    "intrinsic named, spread and implicit input protocols are not implemented",
                ));
            };
            // No expected type from the requested result or a fabricated fn
            // signature: operation-specific operand checks belong to the target.
            inputs.push(self.expr(value, None)?);
        }
        if let Some(expected) = expected {
            self.require_expression(&output, expected, expr.span)?;
        }
        let family = &self.enums[&declaration];
        let comptime = self
            .protocols
            .comptime_intrinsics
            .iter()
            .any(|((id, name), _)| *id == declaration && *name == family.variants[variant].name);
        let selected = self
            .protocols
            .host_bindings()
            .chain(
                self.protocols
                    .comptime_intrinsics
                    .iter()
                    .map(|(tag, op)| (tag, *op)),
            )
            .filter(|((id, name), _)| *id == declaration && *name == family.variants[variant].name)
            .map(|(_, operation)| operation)
            .collect::<Vec<_>>();
        if selected
            .iter()
            .any(|operation| Some(operation) != selected.first())
        {
            return Err(error(
                expr.span,
                "intrinsic tag has conflicting target operations",
            ));
        }
        let binding = if let Some(operation) = selected.first() {
            let registry = nir::intrinsics::Registry::for_family(family, variant, *operation)
                .map_err(|reason| error(expr.span, reason))?;
            let types: Vec<_> = inputs.iter().map(|input| input.ty.clone()).collect();
            let scalar = if operation.is_float_predicate() {
                types.first().unwrap_or(&output)
            } else {
                &output
            };
            let admission = if matches!(
                scalar,
                Term::Rigid {
                    sort: Sort::Type,
                    ..
                }
            ) {
                // Only type propositions enter PTC. This is not evidence of
                // any program behavior or permission to execute the node.
                self.establish_type_requirement(
                    &n::TypePredicate::Or(
                        operation
                            .scalar_types()
                            .iter()
                            .map(|ty| n::TypePredicate::Equal(scalar.clone(), p(*ty)))
                            .collect(),
                    ),
                    expr.span,
                )?;
                registry.defer(tag, &types, &output)
            } else {
                registry.check(tag, &types, &output)
            };
            let admission = admission.map_err(|reason| error(expr.span, reason.to_string()))?;
            admission
                .validate_target_layout(self.target)
                .map_err(|reason| error(expr.span, reason))?;
            Some(if comptime {
                admission.for_comptime()
            } else {
                admission
            })
        } else {
            None
        };
        Ok(n::Expr {
            source: source(expr.span),
            ty: output,
            kind: K::Intrinsic {
                tag,
                inputs,
                binding,
            },
        })
    }
}
