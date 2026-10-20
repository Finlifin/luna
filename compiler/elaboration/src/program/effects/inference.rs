//! Bidirectional argument checking for an operation whose Type arguments were
//! omitted. The allowed row validates the inferred identity; it is not searched
//! for a convenient overload or used to equate unrelated operation instances.
use super::*;

impl Checker<'_, '_> {
    pub(super) fn infer_effect_request(
        &mut self,
        head: &Expr<'_>,
        arguments: &[Arg<'_>],
        expected: Option<&Term>,
        structural_result: Option<&crate::calls::InputContext>,
        span: Span,
    ) -> Result<Option<(nir::effects::Operation, Vec<n::Expr>)>, Error> {
        let owner = match &head.kind {
            H::ResolvedDef { .. } => self.effect_head_owner(head).ok(),
            H::Projection(group, member) if matches!(group.kind, H::ResolvedDef { .. }) => {
                self.effect_head_owner(group).ok().and_then(|owner| {
                    let ItemKind::EffectGroup(group) = &self.package.item(owner)?.kind else {
                        return None;
                    };
                    group.operations.iter().copied().find(|owner| {
                        self.package
                            .item(*owner)
                            .is_some_and(|item| item.ident.name == member.name)
                    })
                })
            }
            _ => None,
        };
        let Some(owner) = owner.filter(|owner| {
            matches!(
                self.package.item(*owner).map(|i| &i.kind),
                Some(ItemKind::Effect(..))
            )
        }) else {
            return Ok(None);
        };
        if self.type_parameters[&owner].is_empty() {
            return Ok(None);
        }
        self.ensure_signature(owner, span)?;
        self.ensure_contract(owner, span)?;
        let binders = self.type_parameters[&owner].clone();
        let signature = &self.signatures[&owner];
        crate::associated::charge_projection_copy(signature, &mut 100_000)
            .map_err(|m| error(span, m))?;
        let Term::Function { parameters, .. } = signature else {
            return Err(error(span, "invalid effect signature"));
        };
        if parameters.len() != arguments.len() {
            return Err(error(span, "effect request argument count mismatch"));
        }
        let inputs = arguments
            .iter()
            .map(|arg| match arg {
                Arg::Positional(value) => Ok(*value),
                _ => Err(error(span, "effect request requires positional arguments")),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let scheme = crate::calls::Scheme {
            parameter_sorts: vec![],
            type_parameters: binders.len(),
            signature: signature
                .abstract_types(&binders)
                .map_err(|e| error(span, e.to_string()))?,
        };
        let caller_scope = self.call_inference_domains();
        let mut checked: Vec<Option<n::Expr>> = vec![None; inputs.len()];
        let type_arguments = loop {
            let actual: Vec<_> = checked
                .iter()
                .map(|value| {
                    value
                        .as_ref()
                        .filter(|v| !is_never(&v.ty))
                        .map(|v| v.ty.clone())
                })
                .collect();
            let mut candidate = scheme
                .infer_sparse_scoped_types_mode(
                    &actual,
                    expected,
                    None,
                    &self.context,
                    crate::ptc::Budget::default(),
                    self.active_effect_environment().is_some()
                        && !matches!(expected, Some(Term::Computation { .. })),
                    &caller_scope,
                )
                .map_err(|e| call_inference_error(span, e))?;
            if let Some(context) = structural_result {
                candidate
                    .constrain_result_context(context, crate::ptc::Budget::default())
                    .map_err(|e| call_inference_error(span, e))?;
            }
            candidate
                .refine_type_views(
                    |ty, budget| {
                        self.callable_input_view_budgeted(ty, budget, span)
                            .map_err(|e| e.message)
                    },
                    crate::ptc::Budget::default(),
                )
                .map_err(|e| call_inference_error(span, e))?;
            if checked.iter().all(Option::is_some) {
                break candidate
                    .inferred_type_arguments()
                    .map_err(|e| call_inference_error(span, e))?;
            }
            let mut progress = false;
            for (index, input) in inputs.iter().enumerate() {
                if checked[index].is_some() {
                    continue;
                }
                let context = candidate
                    .parameter_type(index)
                    .map_err(|e| call_inference_error(input.span, e))?;
                let partial = if context.is_none() {
                    candidate
                        .parameter_callable_context(index)
                        .map_err(|e| call_inference_error(input.span, e))?
                } else {
                    None
                };
                // Preserve computation-valued inputs, just as ordinary generic
                // calls do, until their contextual row is determined.
                if context.is_none()
                    && matches!(&scheme.signature,Term::Function {parameters,..} if matches!(parameters[index].ty,Term::Computation {..}))
                {
                    self.preserve_effect_value = true;
                }
                let input_context = candidate
                    .parameter_input_context(index)
                    .map_err(|e| call_inference_error(input.span, e))?;
                checked[index] = self.argument_attempt_with_input(
                    input,
                    context.as_ref(),
                    partial.as_ref(),
                    false,
                    Some(&input_context),
                )?;
                progress |= checked[index].is_some();
            }
            if !progress && self.numeric_defaults {
                for (index, input) in inputs.iter().enumerate() {
                    if checked[index].is_some() {
                        continue;
                    }
                    let context = candidate
                        .parameter_type(index)
                        .map_err(|e| call_inference_error(input.span, e))?;
                    let partial = candidate
                        .parameter_callable_context(index)
                        .map_err(|e| call_inference_error(input.span, e))?;
                    let input_context = candidate
                        .parameter_input_context(index)
                        .map_err(|e| call_inference_error(input.span, e))?;
                    checked[index] = self.argument_attempt_with_input(
                        input,
                        context.as_ref(),
                        partial.as_ref(),
                        true,
                        Some(&input_context),
                    )?;
                    if checked[index].is_some() {
                        progress = true;
                        break;
                    }
                }
            }
            if !progress {
                return Err(context_error(
                    span,
                    "generic effect argument context is unresolved; add a type annotation or explicit type arguments",
                ));
            }
        };
        // No full signature is copied by inference. Formation is responsible for
        // bounded substitution and proving declaration/parent-group contracts.
        if let Some(group) = self.effect_parents.get(&owner).copied() {
            self.ensure_contract(group, span)?;
            self.check_type_application(group, &type_arguments, span)?;
        }
        self.check_type_application(owner, &type_arguments, span)?;
        Ok(Some((
            nir::effects::Operation {
                declaration: declaration(self.package, &self.package_identities, owner),
                arguments: type_arguments,
            },
            checked.into_iter().map(Option::unwrap).collect(),
        )))
    }
}
