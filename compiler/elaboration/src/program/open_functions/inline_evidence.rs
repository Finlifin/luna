//! Scoped evidence for inline operations in an open function definition.
use super::*;

impl Checker<'_, '_> {
    pub(super) fn computed_tuple_input(&mut self, input: &Expr<'_>) -> Result<bool, Error> {
        let (H::ResolvedFnParam(_, binding) | H::ResolvedLocal(_, binding)) = input.kind else {
            return Ok(false);
        };
        let Some(head) = self.protocols.tuple else {
            return Ok(false);
        };
        let owner = binding.owner;
        let Some(ItemKind::Fn(sig, _)) = self.package.item(owner).map(|item| &item.kind) else {
            return Ok(false);
        };
        let slot = sig.params.iter().position(|p| p.hir_id == binding);
        let annotation =
            slot.and_then(|slot| self.open_functions.get(&owner)?.parameters.get(slot));
        match annotation {
            Some((_, Annotation::Description(description))) => Ok(self
                .refinements
                .structure(description)
                .map_err(|message| error(input.span, message))?
                .is_some_and(|app| app.head() == head)),
            Some((_, Annotation::Pending(code))) if !self.locals.contains_key(&local(binding)) => {
                if code.returns_head(head, &mut self.required_steps, input.span)? {
                    Ok(true)
                } else {
                    Err(error(
                        input.span,
                        "tuple_enumerate requires a tuple head on every normal return of the type producer",
                    ))
                }
            }
            _ => Ok(false),
        }
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
