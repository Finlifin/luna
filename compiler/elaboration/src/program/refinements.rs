//! Signature refinements can depend on the implementation world being built.
//! Retain their declaration context, never a caller's later assumptions.
use super::*;
pub(super) struct Pending {
    pub owner: OwnerId,
    predicate: n::TypePredicate,
    context: crate::ptc::Context,
    assumptions: Vec<n::TraitRequirement>,
    span: Span,
}
fn has_trait(predicate: &n::TypePredicate) -> bool {
    let mut work = vec![predicate];
    while let Some(predicate) = work.pop() {
        match predicate {
            n::TypePredicate::Trait(_) => return true,
            n::TypePredicate::Not(inner) => work.push(inner),
            n::TypePredicate::And(parts) | n::TypePredicate::Or(parts) => work.extend(parts),
            _ => {}
        }
    }
    false
}
// None means executable/program syntax; Some records whether the pure
// proposition actually observes a type. Classification must not change domains
// merely because a type atom is buried under a long Boolean prefix.
fn classify_refinement<'a>(
    expr: &Expr<'a>,
    mut type_operand: impl FnMut(&Expr<'a>) -> bool,
) -> Option<bool> {
    let mut observed = false;
    let mut pending = vec![expr];
    while let Some(expr) = pending.pop() {
        match &expr.kind {
            H::TraitBound(..) => observed = true,
            H::Lit(hir::Lit {
                kind: hir::LitKind::Bool(_),
                ..
            }) => {}
            H::Unary(UnOp::Not, value) => pending.push(value),
            H::Binary(BinOp::And | BinOp::Or, left, right) => {
                pending.push(right);
                pending.push(left);
            }
            H::Binary(BinOp::Eq | BinOp::Ne, left, right)
                if type_operand(left) && type_operand(right) =>
            {
                observed = true
            }
            _ => return None,
        }
    }
    Some(observed)
}

impl Checker<'_, '_> {
    /// Publish only consequences of normal completion, not a branch's entire
    /// local proof context. These formulas contain immutable Type identities.
    pub(super) fn assume_statement_type_facts(
        &mut self,
        statement: &n::Expr,
        span: Span,
    ) -> Result<(), Error> {
        let mut budget = crate::ptc::Budget::default();
        let fact = Self::continuation_type_fact(statement, &mut budget.steps, 0)
            .map_err(|message| error(span, message))?;
        if let Some(fact) = fact {
            self.context
                .validate_formula(&fact, &mut budget)
                .map_err(|e| error(span, e.to_string()))?;
            self.context.assume(fact);
        }
        Ok(())
    }
    fn continuation_type_fact(
        expr: &n::Expr,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<Option<crate::ptc::Formula>, String> {
        use crate::ptc::Formula as F;
        if depth >= 128 {
            return Err("continuation type-fact depth exhausted".into());
        }
        *fuel = fuel
            .checked_sub(1)
            .ok_or("continuation type-fact budget exhausted")?;
        if is_never(&expr.ty) {
            return Ok(None);
        }
        let and = |left, right| match (left, right) {
            (Some(left), Some(right)) => Some(F::And(vec![left, right])),
            (known, None) | (None, known) => known,
        };
        Ok(match &expr.kind {
            K::Discard(value) | K::Let { value, .. } => {
                Self::continuation_type_fact(value, fuel, depth + 1)?
            }
            K::Block { statements, result } | K::CallPreparation { statements, result } => {
                let mut facts = Vec::new();
                for statement in statements.iter().chain(result.as_deref()) {
                    if let Some(fact) = Self::continuation_type_fact(statement, fuel, depth + 1)? {
                        facts.push(fact);
                    }
                }
                if facts.is_empty() {
                    None
                } else {
                    Some(F::And(facts))
                }
            }
            K::LetPattern {
                pattern,
                guard,
                value,
                ..
            } => {
                let observed = match &value.kind {
                    K::TypeValue(ty) => Some(ty),
                    _ => None,
                };
                let facts = and(
                    Self::continuation_type_fact(value, fuel, depth + 1)?,
                    Self::pattern_type_fact(pattern, observed, fuel, depth + 1)?,
                );
                and(
                    facts,
                    match guard {
                        Some(guard) => Self::condition_type_fact(guard, true, fuel, depth + 1)?,
                        None => None,
                    },
                )
            }
            K::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let yes_exits = is_never(&then_branch.ty);
                let no_exits = else_branch
                    .as_ref()
                    .is_some_and(|branch| is_never(&branch.ty));
                let yes = Self::continuation_type_fact(then_branch, fuel, depth + 1)?;
                let no = match else_branch {
                    Some(branch) => Self::continuation_type_fact(branch, fuel, depth + 1)?,
                    None => None,
                };
                let paths = if yes_exits && no_exits {
                    None
                } else if yes_exits {
                    and(
                        no,
                        Self::condition_type_fact(condition, false, fuel, depth + 1)?,
                    )
                } else if no_exits {
                    and(
                        yes,
                        Self::condition_type_fact(condition, true, fuel, depth + 1)?,
                    )
                } else if yes.is_none() && no.is_none() {
                    None
                } else {
                    let yes = and(
                        yes,
                        Self::condition_type_fact(condition, true, fuel, depth + 1)?,
                    );
                    let no = and(
                        no,
                        Self::condition_type_fact(condition, false, fuel, depth + 1)?,
                    );
                    match (yes, no) {
                        (Some(yes), Some(no)) => Some(F::Or(vec![yes, no])),
                        _ => None, // A normal predecessor supplies no fact.
                    }
                };
                and(
                    Self::continuation_type_fact(condition, fuel, depth + 1)?,
                    paths,
                )
            }
            K::Match { value, arms } => {
                let observed = match &value.kind {
                    K::TypeValue(ty) => Some(ty),
                    _ => None,
                };
                let mut paths = Vec::new();
                let mut unknown = false;
                for arm in arms {
                    *fuel = fuel
                        .checked_sub(1)
                        .ok_or("continuation type-fact budget exhausted")?;
                    if is_never(&arm.body.ty) {
                        continue;
                    }
                    let path = and(
                        Self::pattern_type_fact(&arm.pattern, observed, fuel, depth + 1)?,
                        Self::continuation_type_fact(&arm.body, fuel, depth + 1)?,
                    );
                    let path = and(
                        path,
                        match &arm.guard {
                            Some(guard) => Self::condition_type_fact(guard, true, fuel, depth + 1)?,
                            None => None,
                        },
                    );
                    match path {
                        Some(fact) => paths.push(fact),
                        None => unknown = true,
                    }
                }
                // Positive success facts overapproximate selection. A failed
                // earlier candidate need not have evaluated its guard at all.
                let paths = if unknown || paths.is_empty() {
                    None
                } else {
                    Some(F::Or(paths))
                };
                and(Self::continuation_type_fact(value, fuel, depth + 1)?, paths)
            }
            // In particular, a Loop can exit via break: neither its body's
            // facts nor the negation of its entry condition describe all exits.
            _ => None,
        })
    }

    fn pattern_type_fact(
        pattern: &n::Pattern,
        observed: Option<&Term>,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<Option<crate::ptc::Formula>, String> {
        use crate::ptc::Formula as F;
        use n::Pattern as P;
        if depth >= 128 {
            return Err("pattern type-fact depth exhausted".into());
        }
        *fuel = fuel
            .checked_sub(1)
            .ok_or("pattern type-fact budget exhausted")?;
        let combine = |left, right| match (left, right) {
            (Some(left), Some(right)) => Some(F::And(vec![left, right])),
            (known, None) | (None, known) => known,
        };
        Ok(match pattern {
            P::ErrorResult { .. } | P::CodeEvidence { .. } => None,
            P::Observe { .. } => None,
            P::Guard { pattern, condition } => {
                // The canonical constant Type pattern binds its input and
                // immediately compares that snapshot with a TypeValue. No
                // intervening guard code or mutable alias is substituted.
                let observed_fact = match (observed, pattern.as_ref(), &condition.kind) {
                    (Some(subject), P::Binding { binding, inner }, K::TypeEqual(left, right))
                        if matches!(inner.as_ref(), P::Wild)
                            && matches!(&left.kind,K::Local(id) if id==binding) =>
                    {
                        if let K::TypeValue(expected) = &right.kind {
                            nir::type_computations::charge_term(subject, fuel)
                                .map_err(|e| e.to_string())?;
                            nir::type_computations::charge_term(expected, fuel)
                                .map_err(|e| e.to_string())?;
                            Some(F::equal(subject.clone(), expected.clone()))
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                combine(
                    Self::pattern_type_fact(pattern, observed, fuel, depth + 1)?,
                    combine(
                        observed_fact,
                        Self::condition_type_fact(condition, true, fuel, depth + 1)?,
                    ),
                )
            }
            P::Binding { inner, .. } | P::Remap { pattern: inner, .. } => {
                Self::pattern_type_fact(inner, observed, fuel, depth + 1)?
            }
            P::Then { first, value, next } => combine(
                Self::pattern_type_fact(first, observed, fuel, depth + 1)?,
                Self::pattern_type_fact(
                    next,
                    match &value.kind {
                        K::TypeValue(ty) => Some(ty),
                        _ => None,
                    },
                    fuel,
                    depth + 1,
                )?,
            ),
            P::Or(alternatives) => {
                let mut facts = Vec::new();
                let mut unknown = false;
                for alternative in alternatives {
                    match Self::pattern_type_fact(alternative, observed, fuel, depth + 1)? {
                        Some(fact) => facts.push(fact),
                        None => unknown = true,
                    }
                }
                if unknown || facts.is_empty() {
                    None
                } else {
                    Some(F::Or(facts))
                }
            }
            P::Tuple(fields)
            | P::TypeTuple(fields)
            | P::NominalType { fields, .. }
            | P::Record { fields, .. }
            | P::AnonymousRecord { fields, .. }
            | P::Variant { fields, .. }
            | P::NormalForm {
                arguments: fields, ..
            } => {
                let mut facts = Vec::new();
                for field in fields {
                    if let Some(fact) = Self::pattern_type_fact(field, None, fuel, depth + 1)? {
                        facts.push(fact);
                    }
                }
                if facts.is_empty() {
                    None
                } else {
                    Some(F::And(facts))
                }
            }
            P::Wild | P::Unit | P::Literal(_) => None,
        })
    }

    pub(super) fn under_matched_pattern_type_facts<R>(
        &mut self,
        pattern: &n::Pattern,
        value: &n::Expr,
        span: Span,
        check: impl FnOnce(&mut Self) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let observed = match &value.kind {
            K::TypeValue(ty) => Some(ty),
            _ => None,
        };
        self.under_observed_pattern_type_facts(pattern, observed, span, check)
    }
    pub(super) fn under_observed_pattern_type_facts<R>(
        &mut self,
        pattern: &n::Pattern,
        observed: Option<&Term>,
        span: Span,
        check: impl FnOnce(&mut Self) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let Some(fact) = self.checked_pattern_type_fact(pattern, observed, span)? else {
            return check(self);
        };
        let parent = self.context.clone();
        self.context.assume(fact);
        let result = check(self);
        self.context = parent;
        result
    }

    fn checked_pattern_type_fact(
        &self,
        pattern: &n::Pattern,
        observed: Option<&Term>,
        span: Span,
    ) -> Result<Option<crate::ptc::Formula>, Error> {
        let mut budget = crate::ptc::Budget::default();
        let fact = Self::pattern_type_fact(pattern, observed, &mut budget.steps, 0)
            .map_err(|message| error(span, message))?;
        let Some(fact) = fact else {
            return Ok(None);
        };
        self.context
            .validate_formula(&fact, &mut budget)
            .map_err(|e| error(span, e.to_string()))?;
        Ok(Some(fact))
    }

    /// Extract only audited type observations from checked immutable operands.
    /// A Boolean call or mutable Type-valued local is not type evidence.
    fn condition_type_fact(
        condition: &n::Expr,
        positive: bool,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<Option<crate::ptc::Formula>, String> {
        use crate::ptc::Formula as F;
        if depth >= 128 {
            return Err("type condition depth exhausted".into());
        }
        *fuel = fuel
            .checked_sub(1)
            .ok_or("type condition budget exhausted")?;
        let full = match &condition.kind {
            K::TypeEqual(left, right) => match (&left.kind, &right.kind) {
                (K::TypeValue(left), K::TypeValue(right)) => {
                    nir::type_computations::charge_term(left, fuel).map_err(|e| e.to_string())?;
                    nir::type_computations::charge_term(right, fuel).map_err(|e| e.to_string())?;
                    Some(F::equal(left.clone(), right.clone()))
                }
                _ => None,
            },
            K::Not(value) => return Self::condition_type_fact(value, !positive, fuel, depth + 1),
            // Normal completion delivers the tail's Boolean. Prefix statements
            // remain in the program; immutable TypeValue facts do not depend on
            // their mutable state or imply that their effects can be removed.
            K::Block {
                result: Some(value),
                ..
            }
            | K::CallPreparation {
                result: Some(value),
                ..
            } => return Self::condition_type_fact(value, positive, fuel, depth + 1),
            K::And(left, right) | K::Or(left, right) if condition.ty == p(P::Bool) => {
                let and = matches!(condition.kind, K::And(..));
                let left = Self::condition_type_fact(left, positive, fuel, depth + 1)?;
                let right = Self::condition_type_fact(right, positive, fuel, depth + 1)?;
                // True conjunctions and false disjunctions imply every known
                // component, even if other components are runtime Booleans.
                return Ok(if and == positive {
                    match (left, right) {
                        (Some(left), Some(right)) => Some(F::And(vec![left, right])),
                        (known, None) | (None, known) => known,
                    }
                } else {
                    match (left, right) {
                        (Some(left), Some(right)) => Some(F::Or(vec![left, right])),
                        _ => None,
                    }
                });
            }
            _ => None,
        };
        Ok(full.map(|fact| if positive { fact } else { fact.negate() }))
    }

    pub(super) fn under_type_condition<T>(
        &mut self,
        condition: &n::Expr,
        positive: bool,
        span: Span,
        check: impl FnOnce(&mut Self) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let Some(fact) = self.checked_type_condition(condition, positive, span)? else {
            return check(self);
        };
        let parent = self.context.clone();
        self.context.assume(fact);
        let result = check(self);
        self.context = parent;
        result
    }

    pub(super) fn assume_type_condition(
        &mut self,
        condition: &n::Expr,
        positive: bool,
        span: Span,
    ) -> Result<(), Error> {
        if let Some(fact) = self.checked_type_condition(condition, positive, span)? {
            self.context.assume(fact);
        }
        Ok(())
    }
    fn checked_type_condition(
        &self,
        condition: &n::Expr,
        positive: bool,
        span: Span,
    ) -> Result<Option<crate::ptc::Formula>, Error> {
        let mut budget = crate::ptc::Budget::default();
        let fact = Self::condition_type_fact(condition, positive, &mut budget.steps, 0)
            .map_err(|message| error(span, message))?;
        let Some(fact) = fact else {
            return Ok(None);
        };
        self.context
            .validate_formula(&fact, &mut budget)
            .map_err(|e| error(span, e.to_string()))?;
        Ok(Some(fact))
    }

    fn refinement_type_operand(&self, expr: &Expr<'_>) -> bool {
        match &expr.kind {
            H::ResolvedFnParam(_, id) => self.type_input_bindings.contains_key(&local(*id)),
            H::ResolvedClauseParam(_, id) => {
                let declaration = DeclarationId {
                    package: semantic_package(&self.package_identities, id.pkg),
                    local: id.index,
                };
                self.type_parameters
                    .values()
                    .any(|parameters| parameters.contains(&declaration))
            }
            _ => self.is_type_head(expr),
        }
    }
    // Classify before checking: a failed type proof is never a request to run
    // arbitrary code under another meaning. Calls/control flow are executable
    // predicates and remain outside PTC's type-proposition language.
    pub(super) fn extracted_refinement(&self, expr: &Expr<'_>) -> bool {
        classify_refinement(expr, |value| self.refinement_type_operand(value)).is_some()
    }
    pub(super) fn is_type_assumption(&self, expr: &Expr<'_>) -> bool {
        classify_refinement(expr, |value| self.refinement_type_operand(value)) == Some(true)
    }

    pub(super) fn check_refinement_expression(&mut self, expr: &Expr<'_>) -> Result<(), Error> {
        if self.extracted_refinement(expr) {
            let predicate = self.type_predicate(expr)?;
            return self.check_source_refinement(predicate, expr.span);
        }
        let previous = self.executable_refinement_owner.replace(expr.hir_id.owner);
        let value = self.required_value(expr, P::Bool);
        self.executable_refinement_owner = previous;
        match value? {
            comptime_interpretor::Value::Bool(true) => Ok(()),
            comptime_interpretor::Value::Bool(false) => Err(error(
                expr.span,
                "type refinement predicate evaluated to false",
            )),
            _ => Err(error(
                expr.span,
                "type refinement predicate must produce bool",
            )),
        }
    }
    pub(super) fn refinement_trait_value(&mut self, expr: &Expr<'_>) -> Result<bool, Error> {
        let predicate = self.type_predicate(expr)?;
        let formula = crate::ptc::Formula::from_predicate(&predicate);
        if matches!(
            self.context
                .prove(formula.clone(), crate::ptc::Budget::default()),
            crate::ptc::Outcome::Entailed(_)
        ) {
            return Ok(true);
        }
        if matches!(
            self.context
                .prove(formula.negate(), crate::ptc::Budget::default()),
            crate::ptc::Outcome::Entailed(_)
        ) {
            return Ok(false);
        }
        if !self.interfaces_ready {
            return Err(error(
                expr.span,
                "executable refinement trait query is waiting for the implementation world",
            ));
        }
        let assumptions = self.capability_assumptions();
        let world = crate::capabilities::World {
            index: &self.implementation_index,
            implementations: &self.checked_implementations,
            traits: &self.checked_traits,
            assumptions: &assumptions,
            context: &self.context,
        };
        let positive = world
            .check_predicate(&predicate, crate::ptc::Budget::default())
            .map_err(|e| error(expr.span, e))?;
        if matches!(positive, crate::ptc::Outcome::Entailed(_)) {
            return Ok(true);
        }
        let negative = world
            .check_predicate(
                &n::TypePredicate::Not(Box::new(predicate.clone())),
                crate::ptc::Budget::default(),
            )
            .map_err(|e| error(expr.span, e))?;
        if matches!(negative, crate::ptc::Outcome::Entailed(_)) {
            return Ok(false);
        }
        Err(error(
            expr.span,
            format!(
                "refinement trait predicate has no established Boolean value: {predicate}; positive: {positive}; negative: {negative}"
            ),
        ))
    }
    pub(super) fn check_source_refinement(
        &mut self,
        predicate: n::TypePredicate,
        span: Span,
    ) -> Result<(), Error> {
        let outcome = self.context.prove(
            crate::ptc::Formula::from_predicate(&predicate),
            crate::ptc::Budget::default(),
        );
        if matches!(outcome, crate::ptc::Outcome::Entailed(_)) {
            return Ok(());
        }
        let assumptions = self.capability_assumptions();
        if !self.interfaces_ready && has_trait(&predicate) {
            if let Some(owner) = self
                .signature_owner
                .filter(|owner| self.active_signatures.contains(owner))
            {
                if self.in_required_comptime {
                    return Err(error(
                        span,
                        "required comptime evaluation is waiting for a signature type refinement",
                    ));
                }
                self.pending_refinements.push(Pending {
                    owner,
                    predicate,
                    context: self.context.clone(),
                    assumptions,
                    span,
                });
                return Ok(());
            }
        }
        self.prove_source_refinement(&predicate, &self.context, &assumptions, span)
    }
    fn prove_source_refinement(
        &self,
        predicate: &n::TypePredicate,
        context: &crate::ptc::Context,
        assumptions: &[n::TraitRequirement],
        span: Span,
    ) -> Result<(), Error> {
        let outcome = crate::capabilities::World {
            index: &self.implementation_index,
            implementations: &self.checked_implementations,
            traits: &self.checked_traits,
            assumptions,
            context,
        }
        .check_predicate(predicate, crate::ptc::Budget::default())
        .map_err(|e| error(span, e))?;
        if !matches!(outcome, crate::ptc::Outcome::Entailed(_)) {
            return Err(error(
                span,
                format!("type refinement requirement is not established: {predicate}: {outcome}"),
            ));
        }
        Ok(())
    }
    pub(super) fn finish_signature_refinements(&mut self) -> Result<(), Error> {
        for pending in std::mem::take(&mut self.pending_refinements) {
            self.prove_source_refinement(
                &pending.predicate,
                &pending.context,
                &pending.assumptions,
                pending.span,
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod classification_tests {
    use super::*;

    #[test]
    fn deep_type_observations_keep_their_domain_without_native_recursion() {
        let arena = hir::HirArena::new();
        let node = |kind| {
            arena.alloc_expr(Expr {
                hir_id: hir::HirId::INVALID,
                span: rustc_span::DUMMY_SP,
                kind,
            })
        };
        let ty = node(H::TyType);
        let literal = node(H::Lit(hir::Lit {
            kind: hir::LitKind::Bool(true),
            span: rustc_span::DUMMY_SP,
        }));
        let mut value = node(H::Binary(BinOp::Eq, ty, ty));
        for _ in 0..4096 {
            value = node(H::Binary(
                BinOp::And,
                node(H::Unary(UnOp::Not, value)),
                literal,
            ));
        }
        let classify =
            |value| classify_refinement(value, |operand| matches!(operand.kind, H::TyType));
        assert_eq!(classify(value), Some(true));
        assert_eq!(classify(literal), Some(false));
        // A runtime operand remains program syntax even behind a known type
        // observation; classification does not perform Boolean simplification.
        let runtime = node(H::Unit);
        assert_eq!(classify(node(H::Binary(BinOp::And, value, runtime))), None);
        assert_eq!(classify(node(H::Binary(BinOp::Eq, ty, runtime))), None);
        assert_eq!(classify(node(H::Binary(BinOp::Or, runtime, value))), None);
    }
}
