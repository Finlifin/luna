//! Scoped callable telescopes. Temporary rigid inputs are abstracted before
//! publishing the type; names do not become part of callable type identity.
use super::*;

const INPUT_NAMESPACE: u64 = u64::MAX - 8;

pub(super) fn has_named_inputs(expr: &Expr<'_>) -> bool {
    let input = if let H::TyFnArrow(input, _) = &expr.kind {
        *input
    } else {
        expr
    };
    match &input.kind {
        H::TyFn(inputs) | H::TyComptimeFn(inputs) | H::TyModifiedFn(_, inputs) => inputs
            .iter()
            .any(|input| matches!(input.kind, TyParamKind::PositionalDependencyCatched(..))),
        _ => false,
    }
}

impl Checker<'_, '_> {
    pub(super) fn form_modified_callable_type<'hir>(
        &mut self,
        modifiers: hir::expr::FnTypeModifiers,
        inputs: &[hir::common::TyParam<'hir>],
        output: Option<&Expr<'hir>>,
        span: Span,
    ) -> Result<Term, Error> {
        if modifiers.is_pure || modifiers.is_inline || modifiers.is_spec || modifiers.is_verified {
            return Err(error(
                span,
                "modified function type admission and compatibility are not implemented",
            ));
        }
        let signature = self.form_callable_type(inputs, output, modifiers.is_comptime, span)?;
        Ok(if modifiers.is_unsafe {
            Term::UnsafeFunction(Box::new(signature))
        } else {
            signature
        })
    }

    pub(super) fn form_callable_type<'hir>(
        &mut self,
        inputs: &[hir::common::TyParam<'hir>],
        output: Option<&Expr<'hir>>,
        comptime: bool,
        span: Span,
    ) -> Result<Term, Error> {
        self.required_steps = self
            .required_steps
            .checked_sub(
                u64::try_from(
                    inputs
                        .len()
                        .saturating_add(self.callable_type_inputs.len())
                        .saturating_add(1),
                )
                .map_err(|_| error(span, "callable type construction budget exhausted"))?,
            )
            .ok_or_else(|| error(span, "callable type construction budget exhausted"))?;
        if self
            .package_identities
            .values()
            .any(|identity| *identity == INPUT_NAMESPACE)
        {
            return Err(error(
                span,
                "package identity conflicts with callable input namespace",
            ));
        }
        let mut previous_types = Vec::new();
        let mut previous_values = Vec::new();
        let result = (|| {
            let mut binders = Vec::new();
            let mut parameters = Vec::new();
            for input in inputs {
                if input.flags & !hir::common::TPARAM_COMPTIME != 0 {
                    return Err(error(
                        input.span,
                        "function type parameter protocols are not implemented",
                    ));
                }
                let annotation = match input.kind {
                    TyParamKind::Positional(ty)
                    | TyParamKind::PositionalDependencyCatched(_, ty) => ty,
                    _ => {
                        return Err(error(
                            input.span,
                            "function type parameter form is not implemented",
                        ));
                    }
                };
                let ty = self.type_expr(annotation)?;
                let sort = if input.is_comptime() {
                    if ty == p(P::Type) {
                        Sort::Type
                    } else {
                        Sort::ComptimeValue
                    }
                } else {
                    Sort::Runtime
                };
                let abstract_ty = ty
                    .abstract_parameters(&binders)
                    .map_err(|reason| error(annotation.span, reason.to_string()))?;
                parameters.push(Parameter {
                    sort,
                    ty: abstract_ty,
                });
                let declaration = DeclarationId {
                    package: INPUT_NAMESPACE,
                    local: self.next_temporary_binding,
                };
                self.next_temporary_binding = self
                    .next_temporary_binding
                    .checked_add(1)
                    .ok_or_else(|| error(input.span, "callable input identity space exhausted"))?;
                binders.push(nir::DeclarationParameter { declaration, sort });
                if matches!(input.kind, TyParamKind::PositionalDependencyCatched(..)) {
                    let id = local(input.hir_id);
                    if sort == Sort::ComptimeValue {
                        let previous = self
                            .callable_value_inputs
                            .insert(id, (Term::Rigid { declaration, sort }, ty));
                        previous_values.push((id, previous));
                    }
                    let previous = self.callable_type_inputs.insert(
                        id,
                        (sort == Sort::Type).then_some(Term::Rigid { declaration, sort }),
                    );
                    previous_types.push((id, previous));
                }
            }
            let result = output
                .map(|output| self.type_expr(output))
                .transpose()?
                .unwrap_or(p(P::Void))
                .abstract_parameters(&binders)
                .map_err(|reason| error(span, reason.to_string()))?;
            Ok(Term::Function {
                parameters,
                result: Box::new(result),
            }
            .with_comptime_phase(comptime))
        })();
        for (id, previous) in previous_types.into_iter().rev() {
            if let Some(value) = previous {
                self.callable_type_inputs.insert(id, value);
            } else {
                self.callable_type_inputs.remove(&id);
            }
        }
        for (id, previous) in previous_values.into_iter().rev() {
            if let Some(value) = previous {
                self.callable_value_inputs.insert(id, value);
            } else {
                self.callable_value_inputs.remove(&id);
            }
        }
        result
    }
}
