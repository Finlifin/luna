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
            if matches!(self.structural_type(&ty, input.span)?, Term::Tuple(_)) {
                return Ok(false);
            }
            if self.is_proven_tuple_type(&ty, input.span)? {
                return Ok(true);
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
        let has_else = no.is_some();
        let saved = self.context.clone();
        self.context.assume(fact.clone());
        self.definition_refinements
            .requirements
            .push(predicate.clone());
        let yes = self.describe_open_block(
            yes,
            &mut locals.clone(),
            returns,
            if has_else { expected } else { None },
            depth + 1,
        );
        self.definition_refinements.requirements.pop();
        self.context = saved.clone();
        let yes = yes?;
        let yes = if !has_else && !matches!(&yes, D::Known(ty) if is_never(ty)) {
            D::Known(p(P::Void))
        } else {
            yes
        };
        let no = if let Some(no) = no {
            self.context.assume(fact.negate());
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
                    returns,
                    expected,
                    depth + 1,
                )
            } else {
                self.describe_open_expression(no, &mut locals.clone(), returns, expected, depth + 1)
            };
            self.definition_refinements.requirements.pop();
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
                self.require_expression(a, b, expr.span)?;
                yes
            }
            _ => D::Unknown,
        };
        self.require_description(&result, expected, expr.span)?;
        Ok(result)
    }
}
