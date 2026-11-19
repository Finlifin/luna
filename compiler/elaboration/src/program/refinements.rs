//! Signature refinements can depend on the implementation world being built.
//! Retain their declaration context, never a caller's later assumptions.
use super::*;

/// Skolem subjects and premises exist only while checking an open definition.
#[derive(Default)]
pub(super) struct DefinitionRefinements {
    pub(super) subjects: BTreeMap<n::LocalId, nir::DeclarationParameter>,
    pub(super) requirements: Vec<n::TypePredicate>,
    pub(super) checked_tuple_loops: BTreeSet<n::LocalId>,
    pub(super) tuple_elements: Vec<DeclarationId>,
    pub(super) staged_values: Vec<(DeclarationId, Term)>,
}

impl Checker<'_, '_> {
    pub(super) fn complete_refined_type(
        &mut self,
        expr: &Expr<'_>,
        base: &Expr<'_>,
        requirement: &Expr<'_>,
    ) -> Result<CompletedType, Error> {
        match self.completed_type(base)? {
            CompletedType::Identity(base) => {
                self.refinement_subjects.push(base.clone());
                let checked = self.check_refinement_expression(requirement);
                self.refinement_subjects.pop();
                checked?;
                Ok(CompletedType::Identity(base))
            }
            CompletedType::Description(base) => {
                if !self.extracted_refinement(requirement) {
                    return Err(error(
                        requirement.span,
                        "storing an executable refinement requires a captured comptime computation",
                    ));
                }
                let binder = self.fresh_type_binder(expr.hir_id.owner, expr.span)?;
                self.refinement_subjects.push(Term::Rigid {
                    declaration: binder.declaration,
                    sort: binder.sort,
                });
                let predicate = self.type_predicate(requirement);
                self.refinement_subjects.pop();
                let data = self
                    .refinements
                    .publish(
                        base,
                        binder,
                        predicate?,
                        source(expr.span),
                        &mut self.required_steps,
                    )
                    .map_err(|message| error(expr.span, message))?;
                Ok(CompletedType::Description(data))
            }
        }
    }
    /// Concrete annotations supply their own checking direction. Gradual
    /// inputs evaluate the actual supplier once, then check its requirements
    /// in the receiving context before publishing a specialized signature.
    pub(super) fn admit_type_data(
        &mut self,
        data: CompletedType,
        span: Span,
        actual: impl FnOnce(&mut Self) -> Result<Term, Error>,
    ) -> Result<Term, Error> {
        let CompletedType::Description(description) = data else {
            return data.identity(span);
        };
        let actual = actual(self)?;
        self.check_description(&description, &actual, span)?;
        Ok(actual)
    }

    fn check_description(
        &mut self,
        description: &n::DescriptionLiteral,
        actual: &Term,
        span: Span,
    ) -> Result<(), Error> {
        enum Check<'a> {
            Subject(&'a n::DescriptionLiteral, &'a Term),
            Predicate(nir::refinements::BoundRequirement<'a>, &'a Term),
        }
        let registry = self.refinements.clone();
        let mut work = vec![Check::Subject(description, actual)];
        let mut receiving_context = None;
        while let Some(check) = work.pop() {
            match check {
                Check::Subject(description, actual) => {
                    let subject = registry
                        .subject(description, &mut self.required_steps)
                        .map_err(|message| error(span, message))?;
                    let application = subject.application;
                    // LIFO order: check the structural base, then its inner
                    // predicates, then the predicates on its refinements.
                    work.extend(
                        subject
                            .requirements()
                            .rev()
                            .map(|r| Check::Predicate(r, actual)),
                    );
                    if let Some(application) = application {
                        let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                        let mut fuel = before;
                        let fields = application.fields_for(actual, &mut fuel);
                        self.required_steps -= (before - fuel) as u64;
                        work.extend(
                            fields
                                .map_err(|message| error(span, message))?
                                .into_iter()
                                .rev()
                                .map(|(field, value)| Check::Subject(field, value)),
                        );
                    }
                }
                Check::Predicate(requirement, actual) => {
                    let (frames, assumptions) = receiving_context.get_or_insert_with(|| {
                        (self.call_input_frames(), self.capability_assumptions())
                    });
                    let predicate =
                        self.instantiate_refinement(requirement, actual, frames, span)?;
                    self.prove_source_refinement(&predicate, &self.context, assumptions, span)?;
                }
            }
        }
        Ok(())
    }

    pub(super) fn assume_parameter_refinement(
        &mut self,
        parameter: hir::HirId,
        description: &n::DescriptionLiteral,
        span: Span,
    ) -> Result<Term, Error> {
        let binder = self.fresh_type_binder(parameter.owner, span)?;
        let subject = Term::Rigid {
            declaration: binder.declaration,
            sort: binder.sort,
        };
        let registry = self.refinements.clone();
        let is_tuple = registry
            .structure(description)
            .map_err(|message| error(span, message))?
            .is_some_and(|app| self.protocols.tuple == Some(app.head()));
        let view = registry
            .subject(description, &mut self.required_steps)
            .map_err(|message| error(span, message))?;
        let frames = self.call_input_frames();
        for requirement in view.requirements() {
            let predicate = self.instantiate_refinement(requirement, &subject, &frames, span)?;
            let fact = crate::ptc::Formula::from_predicate(&predicate);
            self.context
                .validate_formula(&fact, &mut crate::ptc::Budget::default())
                .map_err(|e| error(span, e.to_string()))?;
            self.context.assume(fact);
            self.definition_refinements.requirements.push(predicate);
        }
        if is_tuple {
            let predicate = n::TypePredicate::Kind(subject.clone(), nir::TypeKind::Tuple);
            let fact = crate::ptc::Formula::from_predicate(&predicate);
            self.context
                .validate_formula(&fact, &mut crate::ptc::Budget::default())
                .map_err(|e| error(span, e.to_string()))?;
            self.context.assume(fact);
            self.definition_refinements.requirements.push(predicate);
        }
        self.definition_refinements
            .subjects
            .insert(local(parameter), binder);
        Ok(subject)
    }

    pub(super) fn assume_deferred_result_guarantees(
        &mut self,
        parameter: hir::HirId,
        binder: DeclarationId,
        guarantees: &[n::TypePredicate],
        span: Span,
    ) -> Result<Term, Error> {
        let subject_binder = self.fresh_type_binder(parameter.owner, span)?;
        let subject = Term::Rigid {
            declaration: subject_binder.declaration,
            sort: subject_binder.sort,
        };
        let formal = [nir::DeclarationParameter {
            declaration: binder,
            sort: Sort::Type,
        }];
        let actual = [subject.clone()];
        for guarantee in guarantees {
            let predicate = super::required::map_requirement(
                guarantee,
                &mut |ty| {
                    ty.instantiate_parameters(&formal, &actual)
                        .map_err(|e| error(span, e.to_string()))
                },
                span,
                0,
            )?;
            let fact = crate::ptc::Formula::from_predicate(&predicate);
            self.context
                .validate_formula(&fact, &mut crate::ptc::Budget::default())
                .map_err(|e| error(span, e.to_string()))?;
            self.context.assume(fact);
            self.definition_refinements.requirements.push(predicate);
        }
        self.definition_refinements
            .subjects
            .insert(local(parameter), subject_binder);
        Ok(subject)
    }

    pub(super) fn is_proven_tuple_type(&mut self, ty: &Term, span: Span) -> Result<bool, Error> {
        if matches!(self.structural_type(ty, span)?, Term::Tuple(_)) {
            return Ok(true);
        }
        Ok(matches!(
            self.context.prove(
                crate::ptc::Formula::kind(ty.clone(), nir::TypeKind::Tuple),
                crate::ptc::Budget::default(),
            ),
            crate::ptc::Outcome::Entailed(_)
        ))
    }

    fn instantiate_refinement(
        &mut self,
        requirement: nir::refinements::BoundRequirement<'_>,
        actual: &Term,
        frames: &[Vec<Sort>],
        span: Span,
    ) -> Result<n::TypePredicate, Error> {
        let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let predicate = requirement.instantiate(actual, frames, &mut fuel);
        self.required_steps -= (before - fuel) as u64;
        predicate.map_err(|e| error(span, format!("refinement substitution failed: {e:?}")))
    }
}

/// A branch premise lasts only until its child check finishes or fails. This
/// boundary changes type facts, not runtime input facts or lexical bindings.
pub(super) struct TypeConditionScope {
    parent: Option<crate::ptc::Context>,
}

impl TypeConditionScope {
    pub(super) fn leave(self, checker: &mut Checker<'_, '_>) {
        if let Some(parent) = self.parent {
            checker.context = parent;
        }
    }
}

impl std::fmt::Display for TypeConditionScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(type-condition-parent")?;
        if let Some(parent) = &self.parent {
            write!(f, " {parent}")?;
        }
        f.write_str(")")
    }
}

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
    expr: &'a Expr<'a>,
    mut type_operand: impl FnMut(&'a Expr<'a>) -> bool,
    is_metatype: impl Fn(&Expr<'_>) -> bool,
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
            _ if Checker::syntactic_type_predicate_kind_with(expr, &is_metatype)
                .is_some_and(|(_, arg)| type_operand(arg)) =>
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
            P::OptionalPresent(_) | P::ErrorResult { .. } | P::CodeEvidence { .. } => None,
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
            P::OptionalAbsent | P::Wild | P::Unit | P::Literal(_) => None,
        })
    }

    pub(super) fn under_matched_pattern_type_facts<R>(
        &mut self,
        pattern: &n::Pattern,
        value: &n::Expr,
        span: Span,
        check: impl FnOnce(&mut Self) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let scope = self.enter_matched_pattern_type_facts(pattern, value, span)?;
        let result = check(self);
        scope.leave(self);
        result
    }

    pub(super) fn enter_matched_pattern_type_facts(
        &mut self,
        pattern: &n::Pattern,
        value: &n::Expr,
        span: Span,
    ) -> Result<TypeConditionScope, Error> {
        let observed = match &value.kind {
            K::TypeValue(ty) => Some(ty),
            _ => None,
        };
        self.enter_observed_pattern_type_facts(pattern, observed, span)
    }
    pub(super) fn under_observed_pattern_type_facts<R>(
        &mut self,
        pattern: &n::Pattern,
        observed: Option<&Term>,
        span: Span,
        check: impl FnOnce(&mut Self) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let scope = self.enter_observed_pattern_type_facts(pattern, observed, span)?;
        let result = check(self);
        scope.leave(self);
        result
    }

    fn enter_observed_pattern_type_facts(
        &mut self,
        pattern: &n::Pattern,
        observed: Option<&Term>,
        span: Span,
    ) -> Result<TypeConditionScope, Error> {
        let Some(fact) = self.checked_pattern_type_fact(pattern, observed, span)? else {
            return Ok(TypeConditionScope { parent: None });
        };
        let parent = self.context.clone();
        self.context.assume(fact);
        Ok(TypeConditionScope {
            parent: Some(parent),
        })
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
            K::TypeKind { kind, operand } => match &operand.kind {
                K::TypeValue(ty) => {
                    nir::type_computations::charge_term(ty, fuel).map_err(|e| e.to_string())?;
                    Some(F::kind(ty.clone(), *kind))
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
        let scope = self.enter_type_condition(condition, positive, span)?;
        let result = check(self);
        scope.leave(self);
        result
    }

    pub(super) fn enter_type_condition(
        &mut self,
        condition: &n::Expr,
        positive: bool,
        span: Span,
    ) -> Result<TypeConditionScope, Error> {
        self.enter_source_type_condition(None, condition, positive, span)
    }

    pub(super) fn enter_source_type_condition(
        &mut self,
        source_expr: Option<&Expr<'_>>,
        condition: &n::Expr,
        positive: bool,
        span: Span,
    ) -> Result<TypeConditionScope, Error> {
        let Some(fact) =
            self.checked_source_type_condition(source_expr, condition, positive, span)?
        else {
            return Ok(TypeConditionScope { parent: None });
        };
        let parent = self.context.clone();
        self.context.assume(fact);
        Ok(TypeConditionScope {
            parent: Some(parent),
        })
    }

    pub(super) fn assume_source_type_condition(
        &mut self,
        source_expr: Option<&Expr<'_>>,
        condition: &n::Expr,
        positive: bool,
        span: Span,
    ) -> Result<(), Error> {
        if let Some(fact) =
            self.checked_source_type_condition(source_expr, condition, positive, span)?
        {
            self.context.assume(fact);
        }
        Ok(())
    }

    fn checked_source_type_condition(
        &mut self,
        source_expr: Option<&Expr<'_>>,
        condition: &n::Expr,
        positive: bool,
        span: Span,
    ) -> Result<Option<crate::ptc::Formula>, Error> {
        let mut budget = crate::ptc::Budget::default();
        let fact = self.condition_source_type_fact(
            source_expr,
            condition,
            positive,
            &mut budget.steps,
            0,
            span,
        )?;
        let Some(fact) = fact else {
            return Ok(None);
        };
        self.context
            .validate_formula(&fact, &mut budget)
            .map_err(|e| error(span, e.to_string()))?;
        Ok(Some(fact))
    }

    fn condition_source_type_fact(
        &mut self,
        source_expr: Option<&Expr<'_>>,
        checked: &n::Expr,
        positive: bool,
        fuel: &mut usize,
        depth: usize,
        span: Span,
    ) -> Result<Option<crate::ptc::Formula>, Error> {
        use crate::ptc::Formula as F;
        if depth >= 128 {
            return Err(error(span, "type condition depth exhausted"));
        }
        *fuel = fuel
            .checked_sub(1)
            .ok_or_else(|| error(span, "type condition budget exhausted"))?;
        if let Some(source_expr) = source_expr {
            if matches!(source_expr.kind, H::TraitBound(..)) {
                let predicate = self.type_predicate(source_expr)?;
                let fact = F::from_predicate(&predicate);
                let mut budget = crate::ptc::Budget {
                    steps: *fuel,
                    max_depth: 128,
                };
                self.context
                    .validate_formula(&fact, &mut budget)
                    .map_err(|e| error(source_expr.span, e.to_string()))?;
                *fuel = budget.steps;
                return Ok(Some(if positive { fact } else { fact.negate() }));
            }
            match (&source_expr.kind, &checked.kind) {
                (H::Unary(UnOp::Not, inner), K::Not(c_inner)) => {
                    return self.condition_source_type_fact(
                        Some(inner),
                        c_inner,
                        !positive,
                        fuel,
                        depth + 1,
                        span,
                    );
                }
                (H::Binary(BinOp::And, l, r), K::And(cl, cr))
                | (H::Binary(BinOp::Or, l, r), K::Or(cl, cr))
                    if checked.ty == p(P::Bool) =>
                {
                    let and = matches!(checked.kind, K::And(..));
                    let left = self.condition_source_type_fact(
                        Some(l),
                        cl,
                        positive,
                        fuel,
                        depth + 1,
                        span,
                    )?;
                    let right = self.condition_source_type_fact(
                        Some(r),
                        cr,
                        positive,
                        fuel,
                        depth + 1,
                        span,
                    )?;
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
                _ => {}
            }
        }
        Self::condition_type_fact(checked, positive, fuel, depth).map_err(|m| error(span, m))
    }

    fn refinement_type_operand(&self, expr: &Expr<'_>) -> bool {
        match &expr.kind {
            H::ResolvedFnParam(_, id) => self.input_facts.type_identity(&local(*id)).is_some(),
            H::ResolvedLocal(_, id) => {
                let id = local(*id);
                (self.input_facts.derived.contains(&id)
                    && self.locals.get(&id) == Some(&p(P::Type))
                    && self.input_facts.type_identity(&id).is_some())
                    || matches!(
                        self.generator_data.get(&id),
                        Some(Some(n::Expr {
                            kind: K::TypeValue(_),
                            ..
                        }))
                    )
            }
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
        classify_refinement(
            expr,
            |value| self.refinement_type_operand(value),
            |base| self.is_metatype_base(base),
        )
        .is_some()
    }
    pub(super) fn is_type_assumption(&self, expr: &Expr<'_>) -> bool {
        classify_refinement(
            expr,
            |value| self.refinement_type_operand(value),
            |base| self.is_metatype_base(base),
        ) == Some(true)
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

    pub(super) fn verify_returned_type_guarantees(
        &mut self,
        expr: &n::Expr,
        span: Span,
    ) -> Result<(), Error> {
        let Some((binder, guarantees)) = self.control.result_guarantees.clone() else {
            return Ok(());
        };
        if guarantees.is_empty() {
            return Ok(());
        }
        let context = self.context.clone();
        let bindings = BTreeMap::new();
        self.verify_type_guarantees_expr(expr, binder, &guarantees, &context, &bindings, span, 0)
    }

    fn verify_type_guarantees_expr<'e>(
        &mut self,
        expr: &'e n::Expr,
        binder: DeclarationId,
        guarantees: &[n::TypePredicate],
        context: &crate::ptc::Context,
        bindings: &BTreeMap<n::LocalId, &'e n::Expr>,
        span: Span,
        depth: usize,
    ) -> Result<(), Error> {
        if depth >= 128 {
            return Err(error(span, "result guarantee verification depth exhausted"));
        }
        if is_never(&expr.ty) {
            return Ok(());
        }
        match &expr.kind {
            K::Block { statements, result } | K::CallPreparation { statements, result } => {
                let mut local_context = context.fork();
                let mut local_bindings = bindings.clone();
                let frames = self.call_input_frames();
                for stmt in statements {
                    if let K::Let { binding, value } = &stmt.kind
                        && !is_never(&value.ty)
                    {
                        local_bindings.insert(*binding, value);
                    }
                    let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                    let before = fuel;
                    let fact = Self::continuation_type_fact(stmt, &mut fuel, 0)
                        .map_err(|message| error(span, message))?;
                    self.required_steps -= (before - fuel) as u64;
                    if let Some(fact) = fact {
                        local_context
                            .validate_formula(&fact, &mut crate::ptc::Budget::default())
                            .map_err(|failure| error(span, failure.to_string()))?;
                        local_context.assume(fact);
                    }
                    if is_never(&stmt.ty) {
                        return Ok(());
                    }
                }
                let _ = frames;
                if let Some(result) = result {
                    self.verify_type_guarantees_expr(
                        result,
                        binder,
                        guarantees,
                        &local_context,
                        &local_bindings,
                        span,
                        depth + 1,
                    )
                } else {
                    Ok(())
                }
            }
            K::UnsafeScope(inner)
            | K::EffectComputation { body: inner, .. }
            | K::CheckTypeAlias {
                body: Some(inner), ..
            } => self.verify_type_guarantees_expr(
                inner,
                binder,
                guarantees,
                context,
                bindings,
                span,
                depth + 1,
            ),
            K::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let before = fuel;
                let then_fact = Self::condition_type_fact(condition, true, &mut fuel, 0)
                    .map_err(|message| error(span, message))?;
                let else_fact = Self::condition_type_fact(condition, false, &mut fuel, 0)
                    .map_err(|message| error(span, message))?;
                self.required_steps -= (before - fuel) as u64;
                let mut then_context = context.fork();
                if let Some(fact) = then_fact {
                    then_context
                        .validate_formula(&fact, &mut crate::ptc::Budget::default())
                        .map_err(|failure| error(span, failure.to_string()))?;
                    then_context.assume(fact);
                }
                self.verify_type_guarantees_expr(
                    then_branch,
                    binder,
                    guarantees,
                    &then_context,
                    bindings,
                    span,
                    depth + 1,
                )?;
                if let Some(else_branch) = else_branch {
                    let mut else_context = context.fork();
                    if let Some(fact) = else_fact {
                        else_context
                            .validate_formula(&fact, &mut crate::ptc::Budget::default())
                            .map_err(|failure| error(span, failure.to_string()))?;
                        else_context.assume(fact);
                    }
                    self.verify_type_guarantees_expr(
                        else_branch,
                        binder,
                        guarantees,
                        &else_context,
                        bindings,
                        span,
                        depth + 1,
                    )?;
                }
                Ok(())
            }
            K::Match { arms, .. } => {
                for arm in arms {
                    self.verify_type_guarantees_expr(
                        &arm.body,
                        binder,
                        guarantees,
                        context,
                        bindings,
                        span,
                        depth + 1,
                    )?;
                }
                Ok(())
            }
            K::Closure { .. } => Ok(()),
            K::Local(id) => {
                if let Some(rhs) = bindings.get(id).copied() {
                    return self.verify_type_guarantees_expr(
                        rhs,
                        binder,
                        guarantees,
                        context,
                        bindings,
                        span,
                        depth + 1,
                    );
                }
                let target = self
                    .input_facts
                    .type_identity(id)
                    .cloned()
                    .unwrap_or(Term::Rigid {
                        declaration: binder,
                        sort: Sort::Type,
                    });
                self.prove_guarantees_for_term(context, &target, binder, guarantees, span)
            }
            K::TypeValue(ty) => {
                let mut local_context = context.fork();
                if let Term::NormalForm {
                    head, arguments, ..
                } = ty
                    && let Some(contract) = self.alias_contracts.get(head).cloned()
                    && contract.parameters.len() == arguments.len()
                {
                    let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                    let before = fuel;
                    let frames = self.call_input_frames();
                    let sub = nir::DeclarationSubstitution::bounded(
                        &contract.parameters,
                        &contract.parameter_types,
                        arguments,
                        &frames,
                        &mut fuel,
                    )
                    .map_err(|e| error(span, e.to_string()))?;
                    for req in &contract.requirements {
                        let pred = sub
                            .instantiate_predicate_bounded(req, &mut fuel)
                            .map_err(|e| error(span, e.to_string()))?;
                        let fact = crate::ptc::Formula::from_predicate(&pred);
                        local_context
                            .validate_formula(&fact, &mut crate::ptc::Budget::default())
                            .map_err(|failure| error(span, failure.to_string()))?;
                        local_context.assume(fact);
                    }
                    self.required_steps -= (before - fuel) as u64;
                }
                self.prove_guarantees_for_term(&local_context, ty, binder, guarantees, span)
            }
            K::Literal(n::Literal::Description(desc)) => {
                let mut local_context = context.fork();
                let subject_term = Term::Rigid {
                    declaration: binder,
                    sort: Sort::Type,
                };
                let subject = self
                    .refinements
                    .subject(desc, &mut self.required_steps)
                    .map_err(|message| error(span, message))?;
                if subject
                    .application
                    .is_some_and(|app| Some(app.head()) == self.protocols.tuple)
                {
                    let fact =
                        crate::ptc::Formula::kind(subject_term.clone(), nir::TypeKind::Tuple);
                    local_context
                        .validate_formula(&fact, &mut crate::ptc::Budget::default())
                        .map_err(|failure| error(span, failure.to_string()))?;
                    local_context.assume(fact);
                }
                let frames = self.call_input_frames();
                let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let before = fuel;
                for req in subject.requirements() {
                    let pred = req
                        .instantiate(&subject_term, &frames, &mut fuel)
                        .map_err(|e| error(span, e.to_string()))?;
                    let fact = crate::ptc::Formula::from_predicate(&pred);
                    local_context
                        .validate_formula(&fact, &mut crate::ptc::Budget::default())
                        .map_err(|failure| error(span, failure.to_string()))?;
                    local_context.assume(fact);
                }
                self.required_steps -= (before - fuel) as u64;
                self.prove_guarantees_for_term(
                    &local_context,
                    &subject_term,
                    binder,
                    guarantees,
                    span,
                )
            }
            K::NormalFormType { head, .. } => {
                let mut local_context = context.fork();
                let subject_term = Term::Rigid {
                    declaration: binder,
                    sort: Sort::Type,
                };
                if Some(*head) == self.protocols.tuple {
                    let fact =
                        crate::ptc::Formula::kind(subject_term.clone(), nir::TypeKind::Tuple);
                    local_context
                        .validate_formula(&fact, &mut crate::ptc::Budget::default())
                        .map_err(|failure| error(span, failure.to_string()))?;
                    local_context.assume(fact);
                }
                self.prove_guarantees_for_term(
                    &local_context,
                    &subject_term,
                    binder,
                    guarantees,
                    span,
                )
            }
            K::Call { callee, .. } => {
                let mut local_context = context.fork();
                let subject_term = Term::Rigid {
                    declaration: binder,
                    sort: Sort::Type,
                };
                let (callee_id, type_args) = match &callee.kind {
                    K::Function(id) => (Some(*id), None),
                    K::FunctionInstance {
                        declaration,
                        arguments,
                        ..
                    } => (Some(*declaration), Some(arguments.as_slice())),
                    _ => (None, None),
                };
                if let Some(callee_owner) =
                    callee_id.and_then(|id| self.source_declarations.owner(id))
                    && let Some((callee_binder, callee_guarantees)) =
                        self.result_guarantees.get(&callee_owner).cloned()
                {
                    let frames = self.call_input_frames();
                    let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                    let before = fuel;
                    let owner_params = self.type_parameters.get(&callee_owner).cloned();
                    for guarantee in &callee_guarantees {
                        let mut pred = guarantee.clone();
                        if let (Some(params), Some(args)) = (&owner_params, type_args)
                            && params.len() == args.len()
                            && !params.is_empty()
                        {
                            let domains: Vec<_> = params
                                .iter()
                                .map(|id| {
                                    self.trait_input_domain(*id).unwrap_or_else(|| p(P::Type))
                                })
                                .collect();
                            let sub = nir::DeclarationSubstitution::bounded(
                                params, &domains, args, &frames, &mut fuel,
                            )
                            .map_err(|e| error(span, e.to_string()))?;
                            pred = sub
                                .instantiate_predicate_bounded(&pred, &mut fuel)
                                .map_err(|e| error(span, e.to_string()))?;
                        }
                        if callee_binder != binder {
                            let sub = nir::DeclarationSubstitution::bounded(
                                &[callee_binder],
                                &[p(P::Type)],
                                std::slice::from_ref(&subject_term),
                                &frames,
                                &mut fuel,
                            )
                            .map_err(|e| error(span, e.to_string()))?;
                            pred = sub
                                .instantiate_predicate_bounded(&pred, &mut fuel)
                                .map_err(|e| error(span, e.to_string()))?;
                        }
                        let fact = crate::ptc::Formula::from_predicate(&pred);
                        local_context
                            .validate_formula(&fact, &mut crate::ptc::Budget::default())
                            .map_err(|failure| error(span, failure.to_string()))?;
                        local_context.assume(fact);
                    }
                    self.required_steps -= (before - fuel) as u64;
                }
                self.prove_guarantees_for_term(
                    &local_context,
                    &subject_term,
                    binder,
                    guarantees,
                    span,
                )
            }
            _ => {
                let subject_term = Term::Rigid {
                    declaration: binder,
                    sort: Sort::Type,
                };
                self.prove_guarantees_for_term(context, &subject_term, binder, guarantees, span)
            }
        }
    }

    fn prove_guarantees_for_term(
        &self,
        context: &crate::ptc::Context,
        target: &Term,
        binder: DeclarationId,
        guarantees: &[n::TypePredicate],
        span: Span,
    ) -> Result<(), Error> {
        let frames = self.call_input_frames();
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        for guarantee in guarantees {
            let instantiated = if *target
                == (Term::Rigid {
                    declaration: binder,
                    sort: Sort::Type,
                }) {
                guarantee.clone()
            } else {
                let sub = nir::DeclarationSubstitution::bounded(
                    &[binder],
                    &[p(P::Type)],
                    std::slice::from_ref(target),
                    &frames,
                    &mut fuel,
                )
                .map_err(|e| error(span, e.to_string()))?;
                sub.instantiate_predicate_bounded(guarantee, &mut fuel)
                    .map_err(|e| error(span, e.to_string()))?
            };
            if matches!(
                context.prove(
                    crate::ptc::Formula::from_predicate(&instantiated),
                    crate::ptc::Budget::default(),
                ),
                crate::ptc::Outcome::Entailed(_)
            ) {
                continue;
            }
            if has_trait(&instantiated) || matches!(instantiated, n::TypePredicate::Equal(..)) {
                let assumptions = self.capability_assumptions();
                if self
                    .prove_source_refinement(&instantiated, context, &assumptions, span)
                    .is_ok()
                {
                    continue;
                }
            }
            return Err(error(
                span,
                format!("result guarantee contract is not established: {instantiated}"),
            ));
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
        let classify = |value| {
            classify_refinement(
                value,
                |operand| matches!(operand.kind, H::TyType),
                |base| matches!(base.kind, H::TyType),
            )
        };
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
