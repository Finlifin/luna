//! Scoped evidence for inline operations in an open function definition.
use super::*;

impl Checker<'_, '_> {
    pub(super) fn computed_tuple_input(&mut self, input: &Expr<'_>) -> Result<bool, Error> {
        Ok(match input.kind {
            H::ResolvedFnParam(_, binding) | H::ResolvedLocal(_, binding)
                if !self.locals.contains_key(&local(binding)) =>
            {
                let owner = binding.owner;
                let ItemKind::Fn(sig, _) = &self.package.item(owner).unwrap().kind else {
                    return Err(error(
                        input.span,
                        "tuple iteration input has no function signature",
                    ));
                };
                let slot = sig.params.iter().position(|p| p.hir_id == binding);
                let annotation =
                    slot.and_then(|slot| self.open_functions.get(&owner)?.parameters.get(slot));
                match (annotation, self.protocols.tuple) {
                    (Some((_, Annotation::Pending(code))), Some(head)) => {
                        if !code.returns_head(head, &mut self.required_steps, input.span)? {
                            return Err(error(
                                input.span,
                                "tuple_enumerate requires a tuple head on every normal return of the type producer",
                            ));
                        }
                        true
                    }
                    _ => false,
                }
            }
            _ => false,
        })
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
        let saved = self.context.clone();
        self.context.assume(fact.clone());
        self.definition_refinements
            .requirements
            .push(predicate.clone());
        let yes = self.describe_open_block(yes, &mut locals.clone(), returns, expected, depth + 1);
        self.definition_refinements.requirements.pop();
        self.context = saved.clone();
        let yes = yes?;
        let no = if let Some(no) = no {
            self.context.assume(fact.negate());
            self.definition_refinements
                .requirements
                .push(n::TypePredicate::Not(Box::new(predicate)));
            let result = self.describe_open_expression(
                no,
                &mut locals.clone(),
                returns,
                expected,
                depth + 1,
            );
            self.definition_refinements.requirements.pop();
            self.context = saved;
            result?
        } else {
            D::Known(p(P::Void))
        };
        let result = match (&yes, &no) {
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
