//! Scoped evidence for inline operations in an open function definition.
use super::*;

impl Checker<'_, '_> {
    pub(super) fn computed_tuple_input(&mut self, input: &Expr<'_>) -> Result<bool, Error> {
        let (H::ResolvedFnParam(_, binding) | H::ResolvedLocal(_, binding)) = input.kind else {
            return Ok(false);
        };
        let key = local(binding);
        let owner = binding.owner;
        let is_deferred_annotation = matches!(
            self.package.item(owner).and_then(|item| match &item.kind {
                ItemKind::Fn(sig, _) => sig
                    .params
                    .iter()
                    .position(|p| p.hir_id == binding)
                    .and_then(|slot| self.open_functions.get(&owner)?.parameters.get(slot)),
                _ => None,
            }),
            Some((_, Annotation::Pending(_) | Annotation::Suspended { .. }))
        );
        if let Some(ty) = self.locals.get(&key).cloned() {
            if self.bound_tuple_elements(&ty).is_some() || matches!(ty, Term::Tuple(_)) {
                return Ok(false);
            }
            if self.is_proven_tuple_type(&ty, input.span)? {
                return Ok(!matches!(
                    self.structural_type(&ty, input.span),
                    Ok(Term::Tuple(_))
                ));
            }
            if is_deferred_annotation {
                return Err(error(
                    input.span,
                    "tuple_enumerate requires a proven tuple type guarantee (e.g. `ensures Type.predicates.is_tuple(R)` or `assumes Type.predicates.is_tuple(...)`)",
                ));
            }
            return Ok(false);
        }
        if let Some(subject) = self.definition_refinements.subjects.get(&key).copied() {
            let ty = Term::Rigid {
                declaration: subject.declaration,
                sort: subject.sort,
            };
            if matches!(self.structural_type(&ty, input.span)?, Term::Tuple(_)) {
                return Ok(false);
            }
            if self.is_proven_tuple_type(&ty, input.span)? {
                return Ok(true);
            }
        }
        if is_deferred_annotation {
            return Err(error(
                input.span,
                "tuple_enumerate requires a proven tuple type guarantee (e.g. `ensures Type.predicates.is_tuple(R)` or `assumes Type.predicates.is_tuple(...)`)",
            ));
        }
        Ok(false)
    }

    pub(super) fn describe_inline_trait_guard(
        &mut self,
        expr: &Expr<'_>,
        condition: &Expr<'_>,
        yes: &hir::expr::Block<'_>,
        no: Option<&Expr<'_>>,
        locals: &BTreeMap<n::LocalId, D>,
        returns: Option<&D>,
        expected: Option<&D>,
        depth: usize,
    ) -> Result<D, Error> {
        // The selector is a staged type proposition. Both branches are
        // checked under their own premise before a concrete input exists.
        let predicate = self.type_predicate(condition)?;
        let fact = crate::ptc::Formula::from_predicate(&predicate);
        self.context
            .validate_formula(&fact, &mut crate::ptc::Budget::default())
            .map_err(|e| error(condition.span, e.to_string()))?;
        let simplify_opt =
            |this: &mut Self, value: Option<&D>, span: Span| -> Result<Option<D>, Error> {
                match value {
                    Some(D::Known(ty)) if matches!(ty, Term::DeferredErrorResult { .. }) => Ok(
                        Some(D::Known(this.simplify_deferred_error_result(ty, span)?)),
                    ),
                    Some(other) => Ok(Some(other.clone())),
                    None => Ok(None),
                }
            };
        let has_else = no.is_some();
        let saved = self.context.clone();
        let saved_control_result = self.control.result.clone();
        self.context.assume(fact.clone());
        let yes_returns = simplify_opt(self, returns, expr.span)?;
        let yes_expected = if has_else {
            simplify_opt(self, expected, expr.span)?
        } else {
            None
        };
        if let Some(target) = saved_control_result.as_ref()
            && matches!(target, Term::DeferredErrorResult { .. })
        {
            self.control.result = Some(self.simplify_deferred_error_result(target, expr.span)?);
        }
        self.definition_refinements
            .requirements
            .push(predicate.clone());
        let yes = self.describe_open_block(
            yes,
            &mut locals.clone(),
            yes_returns.as_ref(),
            yes_expected.as_ref(),
            depth + 1,
        );
        self.definition_refinements.requirements.pop();
        self.control.result = saved_control_result.clone();
        self.context = saved.clone();
        let yes = yes?;
        let yes = if !has_else && !matches!(&yes, D::Known(ty) if is_never(ty)) {
            D::Known(p(P::Void))
        } else {
            yes
        };
        let no = if let Some(no) = no {
            self.context.assume(fact.negate());
            let no_returns = simplify_opt(self, returns, expr.span)?;
            let no_expected = simplify_opt(self, expected, expr.span)?;
            if let Some(target) = saved_control_result.as_ref()
                && matches!(target, Term::DeferredErrorResult { .. })
            {
                self.control.result = Some(self.simplify_deferred_error_result(target, expr.span)?);
            }
            self.definition_refinements
                .requirements
                .push(n::TypePredicate::Not(Box::new(predicate)));
            let result = if let H::If(cond2, yes2, no2) = no.kind
                && self.is_type_assumption(cond2)
            {
                self.describe_inline_trait_guard(
                    no,
                    cond2,
                    yes2,
                    no2,
                    locals,
                    no_returns.as_ref(),
                    no_expected.as_ref(),
                    depth + 1,
                )
            } else {
                self.describe_open_expression(
                    no,
                    &mut locals.clone(),
                    no_returns.as_ref(),
                    no_expected.as_ref(),
                    depth + 1,
                )
            };
            self.definition_refinements.requirements.pop();
            self.control.result = saved_control_result;
            self.context = saved;
            result?
        } else {
            D::Known(p(P::Void))
        };
        let result = match (&yes, &no) {
            (D::Known(a), D::Known(b)) if is_never(a) && is_never(b) => D::Known(p(P::Never)),
            (D::Known(_), D::Known(_)) if has_else && matches!(expected, Some(D::Known(_))) => {
                // Both branches are known and already justified `expected`
                // under their own premises; do not re-check equality outside.
                expected.unwrap().clone()
            }
            (D::Known(t), _) if is_never(t) => no,
            (_, D::Known(t)) if is_never(t) => yes,
            (D::Known(a), D::Known(b)) => {
                if matches!(expected, None | Some(D::Unknown)) {
                    if self.require_expression(a, b, expr.span).is_ok() {
                        yes
                    } else {
                        D::Unknown
                    }
                } else {
                    self.require_expression(a, b, expr.span)?;
                    yes
                }
            }
            _ => D::Unknown,
        };
        self.require_description(&result, expected, expr.span)?;
        Ok(result)
    }
}
