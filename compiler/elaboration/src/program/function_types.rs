//! Scoped callable telescopes. Temporary rigid inputs are abstracted before
//! publishing the type; names do not become part of callable type identity.
use super::*;

const INPUT_NAMESPACE: u64 = u64::MAX - 8;

pub(super) fn phase(kind: &H<'_>) -> n::FunctionPhase {
    match kind {
        H::TyNFFn(_) => n::FunctionPhase::NormalForm,
        H::TyComptimeFn(_) => n::FunctionPhase::Comptime,
        H::TyFn(_) => n::FunctionPhase::Runtime,
        _ => unreachable!("plain callable type"),
    }
}

pub(super) struct StagedParameter {
    pub(super) declaration: DeclarationId,
    pub(super) domain: Term,
}

pub(super) fn normal_form_inputs<'a, 'h>(
    expr: &'a Expr<'h>,
) -> Option<&'h [hir::common::TyParam<'h>]> {
    let input = match expr.kind {
        H::TyFnArrow(input, _) => input,
        _ => expr,
    };
    match input.kind {
        H::TyNFFn(inputs) => Some(inputs),
        _ => None,
    }
}

impl Checker<'_, '_> {
    /// A name is not part of callable identity. Only actual references to this
    /// telescope require its binders during construction; outer Type values
    /// otherwise use the ordinary executable callable constructors.
    pub(super) fn callable_type_needs_binders(&mut self, expr: &Expr<'_>) -> Result<bool, Error> {
        let input = if let H::TyFnArrow(input, _) = expr.kind {
            input
        } else {
            expr
        };
        let (H::TyFn(parameters) | H::TyComptimeFn(parameters) | H::TyModifiedFn(_, parameters)) =
            input.kind
        else {
            return Ok(false);
        };
        self.required_steps = self
            .required_steps
            .checked_sub(parameters.len() as u64)
            .ok_or_else(|| {
                error(
                    expr.span,
                    "callable dependency observation budget exhausted",
                )
            })?;
        let bindings = parameters
            .iter()
            .filter_map(|parameter| {
                matches!(parameter.kind, TyParamKind::PositionalDependencyCatched(..))
                    .then_some(local(parameter.hir_id))
            })
            .collect();
        self.source_may_use_bindings(expr.hir_id.owner, expr, &bindings)
    }

    pub(super) fn computed_callable_type(
        &mut self,
        inputs: Vec<Term>,
        staged: &[bool],
        comptime: bool,
        span: Span,
    ) -> Result<Term, Error> {
        let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let result = nir::callable_types::from_outer(inputs, staged, comptime, &mut fuel);
        self.required_steps -= (before - fuel) as u64;
        result.map_err(|reason| error(span, reason.to_string()))
    }

    pub(super) fn computed_callable_result(
        &mut self,
        signature: Term,
        result: Term,
        span: Span,
    ) -> Result<Term, Error> {
        let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let result = nir::callable_types::with_result_from_outer(signature, result, &mut fuel);
        self.required_steps -= (before - fuel) as u64;
        result.map_err(|reason| error(span, reason.to_string()))
    }

    /// Named callable-type parameters are abstract values in the active
    /// telescope, not locals belonging to an interpreter invocation.
    pub(super) fn callable_type_input_expression(
        &mut self,
        expr: &Expr<'_>,
        binding: n::LocalId,
        expected: Option<&Term>,
    ) -> Result<Option<n::Expr>, Error> {
        let Some(type_input) = self.callable_type_inputs.get(&binding) else {
            return Ok(None);
        };
        let type_domain = p(P::Type);
        let Some(StagedParameter {
            declaration,
            domain,
        }) = type_input
        else {
            return Err(error(
                expr.span,
                if expected == Some(&type_domain) {
                    "function type dependency requires an earlier explicit comptime Type parameter"
                } else {
                    "normal-form argument requires an explicit comptime value input in this function type"
                },
            ));
        };
        if expected == Some(&type_domain) && *domain != type_domain {
            return Err(error(
                expr.span,
                "function type dependency requires an earlier explicit comptime Type parameter",
            ));
        }
        let ty =
            declaration_inputs::copy_input_domain(domain, &mut self.required_steps, expr.span)?;
        let symbol = Term::Rigid {
            declaration: *declaration,
            sort: Sort::of_comptime_domain(&ty),
        };
        if let Some(expected) = expected {
            self.require_expression(&ty, expected, expr.span)?;
        }
        let kind = if ty == type_domain {
            K::TypeValue(symbol)
        } else {
            K::ComptimeValue(symbol)
        };
        Ok(Some(n::Expr {
            source: source(expr.span),
            ty,
            kind,
        }))
    }

    /// Reserve the private declaration frame before checking its domains.
    /// Nested callable types allocate disjoint ranges from the same cursor.
    pub(super) fn fresh_callable_inputs(
        &mut self,
        count: usize,
        span: Span,
    ) -> Result<impl ExactSizeIterator<Item = DeclarationId> + use<>, Error> {
        if self
            .package_identities
            .values()
            .any(|id| *id == INPUT_NAMESPACE)
        {
            return Err(error(
                span,
                "package identity conflicts with callable input namespace",
            ));
        }
        let first = self.next_temporary_binding;
        let end = u32::try_from(count)
            .ok()
            .and_then(|width| first.checked_add(width))
            .ok_or_else(|| error(span, "callable input identity space exhausted"))?;
        self.required_steps = self
            .required_steps
            .checked_sub(u64::from(end - first))
            .ok_or_else(|| error(span, "callable input formation budget exhausted"))?;
        self.next_temporary_binding = end;
        Ok((first..end).map(|local| DeclarationId {
            package: INPUT_NAMESPACE,
            local,
        }))
    }

    pub(super) fn form_modified_callable_type<'hir>(
        &mut self,
        modifiers: hir::expr::FnTypeModifiers,
        inputs: &[hir::common::TyParam<'hir>],
        output: Option<&Expr<'hir>>,
        span: Span,
    ) -> Result<Term, Error> {
        if modifiers.is_inline || modifiers.is_spec || modifiers.is_verified {
            return Err(error(
                span,
                "modified function type admission and compatibility are not implemented",
            ));
        }
        if modifiers.is_pure && modifiers.is_unsafe {
            return Err(error(span, "pure function type cannot be unsafe"));
        }
        let phase = if modifiers.is_comptime {
            n::FunctionPhase::Comptime
        } else {
            n::FunctionPhase::Runtime
        };
        let signature = self.form_callable_type(inputs, output, phase, span)?;
        Ok(if modifiers.is_unsafe {
            Term::UnsafeFunction(Box::new(signature))
        } else if modifiers.is_pure {
            Term::PureFunction(Box::new(signature))
        } else {
            signature
        })
    }

    pub(super) fn form_callable_type<'hir>(
        &mut self,
        inputs: &[hir::common::TyParam<'hir>],
        output: Option<&Expr<'hir>>,
        phase: n::FunctionPhase,
        span: Span,
    ) -> Result<Term, Error> {
        let declarations = self.fresh_callable_inputs(inputs.len(), span)?;
        let mut previous_types = Vec::new();
        let result = (|| {
            let mut telescope = nir::callable_types::DeclarationTelescope::default();
            let mut collector = None;
            for (slot, (input, declaration)) in inputs.iter().zip(declarations).enumerate() {
                if input.flags & !hir::common::TPARAM_COMPTIME != 0 {
                    return Err(error(
                        input.span,
                        "function type parameter protocols are not implemented",
                    ));
                }
                let annotation = match input.kind {
                    TyParamKind::Positional(ty)
                    | TyParamKind::PositionalDependencyCatched(_, ty) => ty,
                    TyParamKind::Varadic(_, ty) if phase == n::FunctionPhase::NormalForm => {
                        if slot + 1 != inputs.len() {
                            return Err(error(
                                input.span,
                                "normal-form callable collector must be the final input",
                            ));
                        }
                        collector = Some(slot);
                        ty
                    }
                    _ => {
                        return Err(error(
                            input.span,
                            "function type parameter form is not implemented",
                        ));
                    }
                };
                let ty = self.type_expr(annotation)?;
                let sort = if phase == n::FunctionPhase::NormalForm || input.is_comptime() {
                    Sort::of_comptime_domain(&ty)
                } else {
                    Sort::Runtime
                };
                nominal_fields::metered(&mut self.required_steps, |fuel| {
                    telescope.push(declaration, sort, &ty, fuel)
                })
                .map_err(|reason| error(annotation.span, reason.to_string()))?;
                if matches!(
                    input.kind,
                    TyParamKind::PositionalDependencyCatched(..) | TyParamKind::Varadic(..)
                ) {
                    let id = local(input.hir_id);
                    let previous = self.callable_type_inputs.insert(
                        id,
                        (sort != Sort::Runtime).then_some(StagedParameter {
                            declaration,
                            domain: ty,
                        }),
                    );
                    previous_types.push((id, previous));
                }
            }
            let result = output
                .map(|output| self.type_expr(output))
                .transpose()?
                .unwrap_or(p(P::Void));
            let signature = nominal_fields::metered(&mut self.required_steps, |fuel| {
                telescope.finish(&result, fuel)
            })
            .map_err(|reason| error(span, reason.to_string()))?;
            if phase == n::FunctionPhase::NormalForm {
                let ty = Term::NormalFormFunction(Box::new(signature), collector);
                ty.check_scope(&[])
                    .map_err(|reason| error(span, reason.to_string()))?;
                Ok(ty)
            } else {
                Ok(signature.with_comptime_phase(phase == n::FunctionPhase::Comptime))
            }
        })();
        for (id, previous) in previous_types.into_iter().rev() {
            if let Some(value) = previous {
                self.callable_type_inputs.insert(id, value);
            } else {
                self.callable_type_inputs.remove(&id);
            }
        }
        result
    }
}
