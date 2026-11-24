//! Signature refinements can depend on the implementation world being built.
//! Retain their declaration context, never a caller's later assumptions.
use super::*;

mod match_evidence;

/// Definition evidence belongs to the lexical declaration being checked.
#[derive(Default)]
pub(super) struct DefinitionRefinements {
    pub(super) subjects: BTreeMap<n::LocalId, nir::DeclarationParameter>,
    pub(super) requirements: Vec<n::TypePredicate>,
    pub(super) closed_source_body: bool,
    pub(super) block_contains_conditional_const: bool,
    pub(super) regions: open_functions::CheckedRegions,
    pub(super) tuple_elements: Vec<DeclarationId>,
    pub(super) behavior_values: LocalBindings<n::Expr>,
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
        let frames = self.call_input_frames();
        for guarantee in guarantees {
            let predicate = nominal_fields::metered(&mut self.required_steps, |fuel| {
                let sub = nir::DeclarationSubstitution::bounded(
                    &[binder],
                    &[p(P::Type)],
                    std::slice::from_ref(&subject),
                    &frames,
                    fuel,
                )?;
                sub.instantiate_predicate_bounded(guarantee, fuel)
            })
            .map_err(|failure| error(span, failure.to_string()))?;
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

fn combine_condition_facts(
    conjunctive: bool,
    left: Option<crate::ptc::Formula>,
    right: Option<crate::ptc::Formula>,
) -> Option<crate::ptc::Formula> {
    use crate::ptc::Formula;
    match (left, right) {
        (Some(left), Some(right)) if conjunctive => Some(Formula::And(vec![left, right])),
        (Some(left), Some(right)) => Some(Formula::Or(vec![left, right])),
        (known, None) | (None, known) if conjunctive => known,
        _ => None,
    }
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
    is_int_comparison: impl Fn(&Expr<'_>) -> bool,
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
            _ if is_int_comparison(expr) => observed = true,
            H::Binary(BinOp::Eq | BinOp::Ne, left, right)
                if type_operand(left) && type_operand(right) =>
            {
                observed = true
            }
            _ if Checker::syntactic_type_predicate_not_sized_with(expr, &is_metatype)
                .is_some_and(&mut type_operand) =>
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
        let fact = self
            .continuation_type_fact(statement, &mut budget.steps, 0)
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
        &self,
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
                self.continuation_type_fact(value, fuel, depth + 1)?
            }
            K::Block { statements, result } | K::CallPreparation { statements, result } => {
                let mut facts = Vec::new();
                for statement in statements.iter().chain(result.as_deref()) {
                    if let Some(fact) = self.continuation_type_fact(statement, fuel, depth + 1)? {
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
                    self.continuation_type_fact(value, fuel, depth + 1)?,
                    self.pattern_type_fact(pattern, observed, fuel, depth + 1)?,
                );
                and(
                    facts,
                    match guard {
                        Some(guard) => self.condition_type_fact(guard, true, fuel, depth + 1)?,
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
                let yes = self.continuation_type_fact(then_branch, fuel, depth + 1)?;
                let no = match else_branch {
                    Some(branch) => self.continuation_type_fact(branch, fuel, depth + 1)?,
                    None => None,
                };
                let paths = if yes_exits && no_exits {
                    None
                } else if yes_exits {
                    and(
                        no,
                        self.condition_type_fact(condition, false, fuel, depth + 1)?,
                    )
                } else if no_exits {
                    and(
                        yes,
                        self.condition_type_fact(condition, true, fuel, depth + 1)?,
                    )
                } else if yes.is_none() && no.is_none() {
                    None
                } else {
                    let yes = and(
                        yes,
                        self.condition_type_fact(condition, true, fuel, depth + 1)?,
                    );
                    let no = and(
                        no,
                        self.condition_type_fact(condition, false, fuel, depth + 1)?,
                    );
                    match (yes, no) {
                        (Some(yes), Some(no)) => Some(F::Or(vec![yes, no])),
                        _ => None, // A normal predecessor supplies no fact.
                    }
                };
                and(
                    self.continuation_type_fact(condition, fuel, depth + 1)?,
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
                        self.pattern_type_fact(&arm.pattern, observed, fuel, depth + 1)?,
                        self.continuation_type_fact(&arm.body, fuel, depth + 1)?,
                    );
                    let path = and(
                        path,
                        match &arm.guard {
                            Some(guard) => {
                                self.condition_type_fact(guard, true, fuel, depth + 1)?
                            }
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
                and(self.continuation_type_fact(value, fuel, depth + 1)?, paths)
            }
            K::Loop { identity, body } => {
                match self.collect_loop_break_facts(body, *identity, None, fuel, depth + 1)? {
                    LoopExitFacts::Known(mut facts) if facts.len() == 1 => facts.pop(),
                    LoopExitFacts::Known(facts) if !facts.is_empty() => Some(F::Or(facts)),
                    LoopExitFacts::Known(_) | LoopExitFacts::Unknown => None,
                }
            }
            _ => None,
        })
    }

    fn collect_loop_break_facts(
        &self,
        expr: &n::Expr,
        target: n::LocalId,
        prefix: Option<crate::ptc::Formula>,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<LoopExitFacts, String> {
        use crate::ptc::Formula as F;
        if depth >= 128 {
            return Err("loop exit type-fact depth exhausted".into());
        }
        *fuel = fuel
            .checked_sub(1)
            .ok_or("loop exit type-fact budget exhausted")?;
        let and = |left: Option<F>, right: Option<F>| match (left, right) {
            (Some(left), Some(right)) => Some(F::And(vec![left, right])),
            (known, None) | (None, known) => known,
        };
        match &expr.kind {
            K::Break(id) => Ok(if *id == target {
                match prefix {
                    Some(fact) => LoopExitFacts::Known(vec![fact]),
                    None => LoopExitFacts::Unknown,
                }
            } else {
                LoopExitFacts::Known(Vec::new())
            }),
            K::Continue(_) | K::Closure { .. } | K::Return(None) => {
                Ok(LoopExitFacts::Known(Vec::new()))
            }
            K::Discard(inner)
            | K::Let { value: inner, .. }
            | K::UnsafeScope(inner)
            | K::EffectComputation { body: inner, .. } => {
                self.collect_loop_break_facts(inner, target, prefix, fuel, depth + 1)
            }
            K::CheckTypeAlias {
                preparations,
                arguments,
                body,
                ..
            } => {
                let mut exits = LoopExitFacts::Known(Vec::new());
                let mut current = prefix;
                for preparation in preparations {
                    let prep_exits = self.collect_loop_break_facts(
                        &preparation.expression,
                        target,
                        current.clone(),
                        fuel,
                        depth + 1,
                    )?;
                    exits = exits.merge(prep_exits);
                    if is_never(&preparation.expression.ty) {
                        return Ok(exits);
                    }
                    current = and(
                        current,
                        self.continuation_type_fact(&preparation.expression, fuel, depth + 1)?,
                    );
                }
                for argument in arguments {
                    if normal_argument_contains_target_break(argument, target, fuel)? {
                        return Ok(LoopExitFacts::Unknown);
                    }
                }
                if let Some(inner) = body {
                    exits = exits.merge(self.collect_loop_break_facts(
                        inner,
                        target,
                        current,
                        fuel,
                        depth + 1,
                    )?);
                }
                Ok(exits)
            }
            K::Return(Some(inner)) => {
                self.collect_loop_break_facts(inner, target, prefix, fuel, depth + 1)
            }
            K::LetPattern {
                pattern,
                guard,
                otherwise,
                value,
            } => {
                let mut exits =
                    self.collect_loop_break_facts(value, target, prefix.clone(), fuel, depth + 1)?;
                if is_never(&value.ty) {
                    return Ok(exits);
                }
                let after_value = and(prefix, self.continuation_type_fact(value, fuel, depth + 1)?);
                if let Some(guard) = guard {
                    let observed = match &value.kind {
                        K::TypeValue(ty) => Some(ty),
                        _ => None,
                    };
                    let guard_prefix = and(
                        after_value.clone(),
                        self.pattern_type_fact(pattern, observed, fuel, depth + 1)?,
                    );
                    let guard_exits = self.collect_loop_break_facts(
                        guard,
                        target,
                        guard_prefix,
                        fuel,
                        depth + 1,
                    )?;
                    exits = exits.merge(guard_exits);
                }
                if let Some(otherwise) = otherwise {
                    let otherwise_exits = self.collect_loop_break_facts(
                        otherwise,
                        target,
                        after_value,
                        fuel,
                        depth + 1,
                    )?;
                    exits = exits.merge(otherwise_exits);
                }
                Ok(exits)
            }
            K::Block { statements, result } | K::CallPreparation { statements, result } => {
                let mut exits = LoopExitFacts::Known(Vec::new());
                let mut current = prefix;
                for stmt in statements {
                    let stmt_exits = self.collect_loop_break_facts(
                        stmt,
                        target,
                        current.clone(),
                        fuel,
                        depth + 1,
                    )?;
                    exits = exits.merge(stmt_exits);
                    if is_never(&stmt.ty) {
                        return Ok(exits);
                    }
                    current = and(current, self.continuation_type_fact(stmt, fuel, depth + 1)?);
                }
                if let Some(result) = result {
                    let tail_exits =
                        self.collect_loop_break_facts(result, target, current, fuel, depth + 1)?;
                    exits = exits.merge(tail_exits);
                }
                Ok(exits)
            }
            K::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let mut exits = self.collect_loop_break_facts(
                    condition,
                    target,
                    prefix.clone(),
                    fuel,
                    depth + 1,
                )?;
                if is_never(&condition.ty) {
                    return Ok(exits);
                }
                let base = and(
                    prefix,
                    self.continuation_type_fact(condition, fuel, depth + 1)?,
                );
                let then_prefix = and(
                    base.clone(),
                    self.condition_type_fact(condition, true, fuel, depth + 1)?,
                );
                exits = exits.merge(self.collect_loop_break_facts(
                    then_branch,
                    target,
                    then_prefix,
                    fuel,
                    depth + 1,
                )?);
                if let Some(else_branch) = else_branch {
                    let else_prefix = and(
                        base,
                        self.condition_type_fact(condition, false, fuel, depth + 1)?,
                    );
                    exits = exits.merge(self.collect_loop_break_facts(
                        else_branch,
                        target,
                        else_prefix,
                        fuel,
                        depth + 1,
                    )?);
                }
                Ok(exits)
            }
            K::Match { value, arms } => {
                let mut exits =
                    self.collect_loop_break_facts(value, target, prefix.clone(), fuel, depth + 1)?;
                if is_never(&value.ty) {
                    return Ok(exits);
                }
                let observed = match &value.kind {
                    K::TypeValue(ty) => Some(ty),
                    _ => None,
                };
                let base = and(prefix, self.continuation_type_fact(value, fuel, depth + 1)?);
                for arm in arms {
                    *fuel = fuel
                        .checked_sub(1)
                        .ok_or("loop exit type-fact budget exhausted")?;
                    let mut arm_prefix = and(
                        base.clone(),
                        self.pattern_type_fact(&arm.pattern, observed, fuel, depth + 1)?,
                    );
                    if let Some(guard) = &arm.guard {
                        exits = exits.merge(self.collect_loop_break_facts(
                            guard,
                            target,
                            arm_prefix.clone(),
                            fuel,
                            depth + 1,
                        )?);
                        if is_never(&guard.ty) {
                            continue;
                        }
                        arm_prefix = and(
                            arm_prefix,
                            and(
                                self.continuation_type_fact(guard, fuel, depth + 1)?,
                                self.condition_type_fact(guard, true, fuel, depth + 1)?,
                            ),
                        );
                    }
                    exits = exits.merge(self.collect_loop_break_facts(
                        &arm.body,
                        target,
                        arm_prefix,
                        fuel,
                        depth + 1,
                    )?);
                }
                Ok(exits)
            }
            K::Loop {
                identity: inner_id,
                body: inner_body,
            } => {
                if *inner_id == target {
                    Ok(LoopExitFacts::Known(Vec::new()))
                } else {
                    self.collect_loop_break_facts(inner_body, target, prefix, fuel, depth + 1)
                }
            }
            _ => Ok(if expression_contains_target_break(expr, target, fuel)? {
                LoopExitFacts::Unknown
            } else {
                LoopExitFacts::Known(Vec::new())
            }),
        }
    }

    fn pattern_type_fact(
        &self,
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
                        if let Some(expected) = self.observed_condition_type_value(right, fuel)? {
                            nir::type_computations::charge_term(subject, fuel)
                                .map_err(|e| e.to_string())?;
                            Some(F::equal(subject.clone(), expected))
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                combine(
                    self.pattern_type_fact(pattern, observed, fuel, depth + 1)?,
                    combine(
                        observed_fact,
                        self.condition_type_fact(condition, true, fuel, depth + 1)?,
                    ),
                )
            }
            P::Binding { inner, .. } | P::Remap { pattern: inner, .. } => {
                self.pattern_type_fact(inner, observed, fuel, depth + 1)?
            }
            P::Then { first, value, next } => combine(
                self.pattern_type_fact(first, observed, fuel, depth + 1)?,
                self.pattern_type_fact(
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
                    match self.pattern_type_fact(alternative, observed, fuel, depth + 1)? {
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
                    if let Some(fact) = self.pattern_type_fact(field, None, fuel, depth + 1)? {
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
        let fact = self
            .pattern_type_fact(pattern, observed, &mut budget.steps, 0)
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
    fn observed_condition_type_value(
        &self,
        expr: &n::Expr,
        fuel: &mut usize,
    ) -> Result<Option<Term>, String> {
        if let K::TypeValue(ty) = &expr.kind {
            nir::type_computations::charge_term(ty, fuel).map_err(|e| e.to_string())?;
            return Ok(Some(ty.clone()));
        }
        let mut budget = comptime_runtime::Budget {
            steps: *fuel,
            max_limbs: 4096,
        };
        let before = budget.steps;
        let observed = self
            .normal_input_hint(expr, &mut budget)
            .map_err(|failure| failure.to_string())?;
        *fuel -= before - budget.steps;
        match observed {
            Some(ty) if ty.sort() == Sort::Type && ty.check_scope(self.context.scope()).is_ok() => {
                nir::type_computations::charge_term(&ty, fuel).map_err(|e| e.to_string())?;
                Ok(Some(ty))
            }
            _ => Ok(None),
        }
    }

    fn condition_type_fact(
        &self,
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
            K::TypeEqual(left, right) => {
                match (
                    self.observed_condition_type_value(left, fuel)?,
                    self.observed_condition_type_value(right, fuel)?,
                ) {
                    (Some(left), Some(right)) => Some(F::equal(left, right)),
                    _ => None,
                }
            }
            K::TypeKind { kind, operand } => {
                match self.observed_condition_type_value(operand, fuel)? {
                    Some(ty) => Some(F::kind(ty, *kind)),
                    None => None,
                }
            }
            K::TypeNotSized { operand } => self
                .observed_condition_type_value(operand, fuel)?
                .map(F::not_sized),
            K::Not(value) => return self.condition_type_fact(value, !positive, fuel, depth + 1),
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
            } => return self.condition_type_fact(value, positive, fuel, depth + 1),
            K::And(left, right) | K::Or(left, right) if condition.ty == p(P::Bool) => {
                let and = matches!(condition.kind, K::And(..));
                let left = self.condition_type_fact(left, positive, fuel, depth + 1)?;
                let right = self.condition_type_fact(right, positive, fuel, depth + 1)?;
                // True conjunctions and false disjunctions imply every known
                // component, even if other components are runtime Booleans.
                return Ok(combine_condition_facts(and == positive, left, right));
            }
            _ => None,
        };
        Ok(full.map(|fact| if positive { fact } else { fact.negate() }))
    }

    /// The Boolean spine has already been checked by open expression
    /// description. Only immutable type observations can supply branch facts.
    /// In particular an ordinary staged bool supplies no PTC formula.
    pub(super) fn staged_condition_fact(
        &mut self,
        expr: &Expr<'_>,
        positive: bool,
        depth: usize,
    ) -> Result<Option<crate::ptc::Formula>, Error> {
        if depth >= 128 {
            return Err(error(expr.span, "staged condition depth exhausted"));
        }
        self.required_steps = self
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(expr.span, "staged condition budget exhausted"))?;
        match expr.kind {
            H::Unary(UnOp::Not, inner) => self.staged_condition_fact(inner, !positive, depth + 1),
            H::Binary(op @ (BinOp::And | BinOp::Or), left, right) => {
                let left = self.staged_condition_fact(left, positive, depth + 1)?;
                let right = self.staged_condition_fact(right, positive, depth + 1)?;
                Ok(combine_condition_facts(
                    (op == BinOp::And) == positive,
                    left,
                    right,
                ))
            }
            _ if self.is_type_assumption(expr) => {
                let predicate = self.type_predicate(expr)?;
                let fact = crate::ptc::Formula::from_predicate(&predicate);
                self.context
                    .validate_formula(&fact, &mut crate::ptc::Budget::default())
                    .map_err(|e| error(expr.span, e.to_string()))?;
                Ok(Some(if positive { fact } else { fact.negate() }))
            }
            _ => Ok(None),
        }
    }

    pub(super) fn enter_source_type_condition(
        &mut self,
        source_expr: Option<&Expr<'_>>,
        condition: &n::Expr,
        positive: bool,
        span: Span,
    ) -> Result<TypeConditionScope, Error> {
        let fact = self.checked_source_type_condition(source_expr, condition, positive, span)?;
        Ok(self.enter_condition_fact(fact))
    }

    pub(super) fn enter_condition_fact(
        &mut self,
        fact: Option<crate::ptc::Formula>,
    ) -> TypeConditionScope {
        let parent = fact.map(|fact| {
            let parent = self.context.clone();
            self.context.assume(fact);
            parent
        });
        TypeConditionScope { parent }
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
                    return Ok(combine_condition_facts(and == positive, left, right));
                }
                _ => {}
            }
        }
        self.condition_type_fact(checked, positive, fuel, depth)
            .map_err(|m| error(span, m))
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
            |expr| self.ptc_integer_comparison(expr).is_some(),
        )
        .is_some()
    }

    pub(super) fn is_type_assumption(&self, expr: &Expr<'_>) -> bool {
        classify_refinement(
            expr,
            |value| self.refinement_type_operand(value),
            |base| self.is_metatype_base(base),
            |expr| self.ptc_integer_comparison(expr).is_some(),
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
        let mut context = self.context.fork();
        self.admit_not_sized_nominals(&predicate, &mut context, span)?;
        let outcome = context.prove(
            crate::ptc::Formula::from_predicate(&predicate),
            crate::ptc::Budget::default(),
        );
        if matches!(outcome, crate::ptc::Outcome::Entailed(_)) {
            return Ok(());
        }
        if let crate::ptc::Outcome::IllFormed(message) = outcome {
            return Err(error(span, message));
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
                    context,
                    assumptions,
                    span,
                });
                return Ok(());
            }
        }
        self.prove_source_refinement(&predicate, &context, &assumptions, span)
    }

    pub(super) fn check_not_sized_operand(
        &mut self,
        operand: &Term,
        span: Span,
    ) -> Result<(), Error> {
        let mut context = self.context.fork();
        let mut checked = std::collections::HashSet::new();
        self.admit_not_sized_nominals_in_term(operand, &mut context, &mut checked, span, 0)?;
        context
            .check_not_sized_domain(operand)
            .map_err(|message| error(span, message))
    }

    fn admit_not_sized_nominals(
        &mut self,
        predicate: &n::TypePredicate,
        context: &mut crate::ptc::Context,
        span: Span,
    ) -> Result<(), Error> {
        let mut pending = vec![predicate];
        let mut checked = std::collections::HashSet::new();
        while let Some(pred) = pending.pop() {
            match pred {
                n::TypePredicate::NotSized(ty) => {
                    self.admit_not_sized_nominals_in_term(ty, context, &mut checked, span, 0)?;
                }
                n::TypePredicate::Not(inner) => pending.push(inner),
                n::TypePredicate::And(parts) | n::TypePredicate::Or(parts) => pending.extend(parts),
                _ => {}
            }
        }
        Ok(())
    }

    fn admit_not_sized_nominals_in_term(
        &mut self,
        ty: &Term,
        context: &mut crate::ptc::Context,
        checked: &mut std::collections::HashSet<Term>,
        span: Span,
        depth: usize,
    ) -> Result<(), Error> {
        if depth >= 128 {
            return Ok(());
        }
        match ty {
            Term::Nominal {
                declaration,
                arguments,
            } => {
                if !checked.insert(ty.clone()) {
                    return Ok(());
                }
                let is_active = self
                    .source_declarations
                    .owner(*declaration)
                    .is_some_and(|owner| self.active_nominals.contains(&owner));
                if !is_active {
                    self.ensure_nominal_type(ty, span)?;
                }
                let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let before = fuel;
                let layout = nir::compute_layout(self, ty, &mut fuel);
                self.required_steps -= (before - fuel) as u64;
                match layout {
                    Ok(_) => {
                        if arguments.is_empty() {
                            context.admit_nominal_sized(*declaration);
                        }
                    }
                    Err(nir::LayoutError::ComptimeOnly(_)) => {
                        if arguments.is_empty() {
                            context.admit_nominal_comptime_only(*declaration);
                        }
                        return Err(error(
                            span,
                            "comptime-only type has no runtime representation for Type.predicates.is_not_sized",
                        ));
                    }
                    Err(_) => {}
                }
            }
            Term::Optional(inner) | Term::UnsafeFunction(inner) => {
                self.admit_not_sized_nominals_in_term(inner, context, checked, span, depth + 1)?;
            }
            Term::Tuple(elements) => {
                for element in elements {
                    self.admit_not_sized_nominals_in_term(
                        element,
                        context,
                        checked,
                        span,
                        depth + 1,
                    )?;
                }
            }
            Term::Record(fields) => {
                for (_, field_ty) in fields {
                    self.admit_not_sized_nominals_in_term(
                        field_ty,
                        context,
                        checked,
                        span,
                        depth + 1,
                    )?;
                }
            }
            Term::ErrorResult { errors, result } => {
                for err_ty in errors {
                    self.admit_not_sized_nominals_in_term(
                        err_ty,
                        context,
                        checked,
                        span,
                        depth + 1,
                    )?;
                }
                self.admit_not_sized_nominals_in_term(result, context, checked, span, depth + 1)?;
            }
            Term::Closure { environment, .. } => {
                for env_ty in environment {
                    self.admit_not_sized_nominals_in_term(
                        env_ty,
                        context,
                        checked,
                        span,
                        depth + 1,
                    )?;
                }
            }
            Term::NormalForm {
                head,
                sort: nir::Sort::Type,
                arguments,
            } if context.is_tuple_head(*head) => {
                if let Some(elements) =
                    nir::normal_forms::type_list_fields_with_layouts(arguments, &|id| {
                        context.is_list_inference_head(id)
                    })
                {
                    for element in elements {
                        self.admit_not_sized_nominals_in_term(
                            element,
                            context,
                            checked,
                            span,
                            depth + 1,
                        )?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
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

    pub(super) fn invalidate_mutated_type_bindings<T>(
        expr: &n::Expr,
        bindings: &mut BTreeMap<n::LocalId, T>,
        fuel: &mut usize,
    ) -> Result<(), nir::visit::Error> {
        nir::free_bindings::retain_unmodified(expr, bindings, fuel)
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
                    let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                    let before = fuel;
                    let mutated = match &stmt.kind {
                        K::Let { binding, value } => {
                            let res = Self::invalidate_mutated_type_bindings(
                                value,
                                &mut local_bindings,
                                &mut fuel,
                            );
                            if res.is_ok() {
                                local_bindings.remove(binding);
                                if !is_never(&value.ty) {
                                    local_bindings.insert(*binding, value);
                                }
                            }
                            res
                        }
                        _ => Self::invalidate_mutated_type_bindings(
                            stmt,
                            &mut local_bindings,
                            &mut fuel,
                        ),
                    };
                    self.required_steps -= (before - fuel) as u64;
                    mutated.map_err(|message| error(span, message))?;
                    let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                    let before = fuel;
                    let fact = self
                        .continuation_type_fact(stmt, &mut fuel, 0)
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
                let mut branch_bindings = bindings.clone();
                let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let before = fuel;
                let invalidated = Self::invalidate_mutated_type_bindings(
                    condition,
                    &mut branch_bindings,
                    &mut fuel,
                );
                let then_fact = self.condition_type_fact(condition, true, &mut fuel, 0);
                let else_fact = self.condition_type_fact(condition, false, &mut fuel, 0);
                self.required_steps -= (before - fuel) as u64;
                invalidated.map_err(|message| error(span, message))?;
                let then_fact = then_fact.map_err(|message| error(span, message))?;
                let else_fact = else_fact.map_err(|message| error(span, message))?;
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
                    &branch_bindings,
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
                        &branch_bindings,
                        span,
                        depth + 1,
                    )?;
                }
                Ok(())
            }
            K::Match { value, arms } => {
                let mut match_bindings = bindings.clone();
                let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let before = fuel;
                let invalidated =
                    Self::invalidate_mutated_type_bindings(value, &mut match_bindings, &mut fuel);
                self.required_steps -= (before - fuel) as u64;
                invalidated.map_err(|message| error(span, message))?;
                for arm in arms {
                    let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                    let before = fuel;
                    let arm_res = match_evidence::arm_bindings(arm, &mut match_bindings, &mut fuel);
                    self.required_steps -= (before - fuel) as u64;
                    let arm_bindings = arm_res.map_err(|message| error(span, message))?;
                    self.verify_type_guarantees_expr(
                        &arm.body,
                        binder,
                        guarantees,
                        context,
                        &arm_bindings,
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
            K::TypeValue(ty) => self.prove_returned_identity(context, ty, binder, guarantees, span),
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
            K::Call { callee, .. } => {
                let mut local_context = context.fork();
                let subject_term = Term::Rigid {
                    declaration: binder,
                    sort: Sort::Type,
                };
                if let Some((id, operands)) = CallableClauseView::direct(callee)
                    && let Some(owner) = self.source_declarations.owner(id)
                {
                    let frames = self.call_input_frames();
                    let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                    let before = fuel;
                    let checked = (|| -> Result<(), Error> {
                        let Some((callee_binder, facts)) = self
                            .source_result_facts(owner, Some((operands, expr)), &frames, &mut fuel)
                            .map_err(|e| error(span, e))?
                        else {
                            return Ok(());
                        };
                        for mut pred in facts {
                            if callee_binder != binder {
                                let sub = nir::DeclarationSubstitution::bounded(
                                    std::slice::from_ref(&callee_binder),
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
                        Ok(())
                    })();
                    self.required_steps -= (before - fuel) as u64;
                    checked?;
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
                let subject = Term::Rigid {
                    declaration: binder,
                    sort: Sort::Type,
                };
                let mut local_context = context.fork();
                if let K::NormalFormType { head, .. } = &expr.kind
                    && Some(*head) == self.protocols.tuple
                {
                    // The head already proves tuple shape, even when a field
                    // is a description or requires an invocation.
                    let fact = crate::ptc::Formula::kind(subject.clone(), nir::TypeKind::Tuple);
                    local_context
                        .validate_formula(&fact, &mut crate::ptc::Budget::default())
                        .map_err(|failure| error(span, failure.to_string()))?;
                    local_context.assume(fact);
                    if guarantees.iter().all(|guarantee| {
                        matches!(
                            local_context.prove(
                                crate::ptc::Formula::from_predicate(guarantee),
                                crate::ptc::Budget::default()
                            ),
                            crate::ptc::Outcome::Entailed(_)
                        )
                    }) {
                        return Ok(());
                    }
                }
                // A checked construction has its ordinary identity on normal
                // completion. Observation neither runs a supplier nor unfolds
                // an opaque head; an unavailable operand leaves just the facts
                // already known about the result.
                let identity = self.with_observation_budget(span, |this, budget| {
                    this.normal_input_hint(expr, budget)
                })?;
                match identity {
                    Some(ty) if ty.sort() == Sort::Type => {
                        self.prove_returned_identity(&local_context, &ty, binder, guarantees, span)
                    }
                    _ => self.prove_guarantees_for_term(
                        &local_context,
                        &subject,
                        binder,
                        guarantees,
                        span,
                    ),
                }
            }
        }
    }

    fn prove_returned_identity(
        &mut self,
        context: &crate::ptc::Context,
        ty: &Term,
        binder: DeclarationId,
        guarantees: &[n::TypePredicate],
        span: Span,
    ) -> Result<(), Error> {
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

    fn prove_guarantees_for_term(
        &mut self,
        context: &crate::ptc::Context,
        target: &Term,
        binder: DeclarationId,
        guarantees: &[n::TypePredicate],
        span: Span,
    ) -> Result<(), Error> {
        let frames = self.call_input_frames();
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let result = (|| -> Result<(), Error> {
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
        })();
        self.required_steps -= (before - fuel) as u64;
        result
    }

    pub(in crate::program) fn elaborate_loop_body_with_barrier(
        &mut self,
        span: Span,
        identity: n::LocalId,
        mut build_body: impl FnMut(&mut Self) -> Result<n::Expr, Error>,
    ) -> Result<n::Expr, Error> {
        let entry_locals: BTreeSet<n::LocalId> = self.locals.keys().copied().collect();
        let entry_write_versions = self.local_write_versions.clone();
        let entry_input_facts = self.input_facts.clone();
        let staged = self.in_required_comptime || self.in_staged_input;
        let mut barrier_mutated = BTreeSet::new();
        let max_passes = entry_locals
            .len()
            .saturating_add(entry_write_versions.len())
            .clamp(2, 16);
        for pass in 0..max_passes {
            let attempt = checking_attempts::Attempt::begin(
                self,
                span,
                self.numeric_defaults,
                checking_attempts::Disposition::Commit,
            )?;
            let checked = build_body(self);
            let body = match checked {
                Ok(body) => body,
                Err(failure) => return attempt.finish_checked(self, Err(failure)),
            };
            let mut continue_fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
            let before_continue_fuel = continue_fuel;
            let may_continue = if !is_never(&body.ty) || self.in_code_body {
                Ok(true)
            } else {
                control::loop_may_continue(&body, identity, &mut continue_fuel)
                    .map_err(|reason| error(span, reason))
            };
            self.required_steps -= (before_continue_fuel - continue_fuel) as u64;
            let may_continue = match may_continue {
                Ok(flag) => flag,
                Err(failure) => return attempt.finish_checked(self, Err(failure)),
            };
            let mut newly_mutated = BTreeSet::new();
            if may_continue {
                for binding in entry_locals.iter().chain(entry_write_versions.keys()) {
                    if barrier_mutated.contains(binding) {
                        continue;
                    }
                    let before = entry_write_versions.get(binding).copied().unwrap_or(0);
                    let after = self.local_write_versions.get(binding).copied().unwrap_or(0);
                    if after > before {
                        newly_mutated.insert(*binding);
                    }
                }
                for binding in entry_input_facts
                    .types
                    .keys()
                    .filter(|id| !self.input_facts.types.contains_key(id))
                    .chain(
                        entry_input_facts
                            .values
                            .keys()
                            .filter(|id| !self.input_facts.values.contains_key(id)),
                    )
                    .chain(
                        entry_input_facts
                            .partial
                            .keys()
                            .filter(|id| !self.input_facts.partial.contains_key(id)),
                    )
                    .chain(
                        entry_input_facts
                            .specialized
                            .iter()
                            .map(|(id, _, _)| id)
                            .filter(|id| {
                                !self
                                    .input_facts
                                    .specialized
                                    .iter()
                                    .any(|(s, _, _)| s == *id)
                            }),
                    )
                    .chain(
                        entry_input_facts
                            .parameters
                            .identities()
                            .map(|(id, _)| id)
                            .filter(|id| !self.input_facts.has_identity(id)),
                    )
                {
                    if !barrier_mutated.contains(binding) {
                        newly_mutated.insert(*binding);
                    }
                }
                if !entry_input_facts.supplied.is_empty() {
                    let supplied_check: Result<(), Error> = (|| {
                        for binding in entry_locals
                            .iter()
                            .chain(entry_input_facts.parameters.identities().map(|(id, _)| id))
                        {
                            if barrier_mutated.contains(binding) {
                                continue;
                            }
                            let had_value = matches!(
                                entry_input_facts.supplied.lookup(
                                    *binding,
                                    &mut self.required_steps,
                                    span,
                                )?,
                                Some((_, Some(_)))
                            );
                            if had_value {
                                let still_has_value = matches!(
                                    self.input_facts.supplied.lookup(
                                        *binding,
                                        &mut self.required_steps,
                                        span,
                                    )?,
                                    Some((_, Some(_)))
                                );
                                if !still_has_value {
                                    newly_mutated.insert(*binding);
                                }
                            }
                        }
                        Ok(())
                    })();
                    if let Err(failure) = supplied_check {
                        return attempt.finish_checked(self, Err(failure));
                    }
                }
            }
            if newly_mutated.is_empty() {
                return attempt.finish_checked(self, Ok(body));
            }
            if pass + 1 == max_passes {
                return attempt.finish_checked(
                    self,
                    Err(error(
                        span,
                        "loop back-edge invalidation barrier did not converge",
                    )),
                );
            }
            let _ = attempt.finish_checked::<n::Expr>(
                self,
                Err(error(
                    span,
                    "re-elaborating loop under back-edge invalidation barrier",
                )),
            );
            barrier_mutated.extend(newly_mutated);
            for binding in &barrier_mutated {
                let version = self.local_write_versions.entry(*binding).or_insert(0);
                *version = entry_write_versions
                    .get(binding)
                    .copied()
                    .unwrap_or(0)
                    .saturating_add(1);
            }
            self.input_facts.invalidate(
                barrier_mutated.clone(),
                staged,
                &mut self.required_steps,
                span,
            )?;
        }
        unreachable!()
    }
}

enum LoopExitFacts {
    Known(Vec<crate::ptc::Formula>),
    Unknown,
}

impl LoopExitFacts {
    fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::Known(mut left), Self::Known(right)) => {
                left.extend(right);
                Self::Known(left)
            }
            _ => Self::Unknown,
        }
    }
}

impl std::fmt::Display for LoopExitFacts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Known(facts) => write!(f, "(loop-exit-facts (known {}))", facts.len()),
            Self::Unknown => f.write_str("(loop-exit-facts unknown)"),
        }
    }
}

fn expression_contains_target_break(
    expr: &n::Expr,
    target: n::LocalId,
    fuel: &mut usize,
) -> Result<bool, String> {
    let mut contains_target_break = false;
    nir::visit::expression_pruned(expr, fuel, |sub, _| {
        if matches!(sub.kind, K::Closure { .. }) {
            return Ok(false);
        }
        if matches!(sub.kind, K::Break(id) if id == target) {
            contains_target_break = true;
            return Ok(false);
        }
        Ok(true)
    })?;
    Ok(contains_target_break)
}

fn normal_argument_contains_target_break(
    argument: &n::NormalFormArgument,
    target: n::LocalId,
    fuel: &mut usize,
) -> Result<bool, String> {
    *fuel = fuel
        .checked_sub(1)
        .ok_or("loop exit type-fact budget exhausted")?;
    match argument {
        n::NormalFormArgument::Value(expr) | n::NormalFormArgument::ListSpread(expr) => {
            expression_contains_target_break(expr, target, fuel)
        }
        n::NormalFormArgument::List { items, .. } | n::NormalFormArgument::Tuple { items, .. } => {
            for item in items {
                if normal_argument_contains_target_break(item, target, fuel)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        n::NormalFormArgument::If {
            condition,
            then_input,
            else_input,
            ..
        } => Ok(expression_contains_target_break(condition, target, fuel)?
            || normal_argument_contains_target_break(then_input, target, fuel)?
            || normal_argument_contains_target_break(else_input, target, fuel)?),
        n::NormalFormArgument::Block {
            statements, result, ..
        } => {
            for statement in statements {
                if expression_contains_target_break(statement, target, fuel)? {
                    return Ok(true);
                }
            }
            if let Some(result) = result {
                normal_argument_contains_target_break(result, target, fuel)
            } else {
                Ok(false)
            }
        }
        n::NormalFormArgument::Match { value, arms, .. } => {
            if expression_contains_target_break(value, target, fuel)? {
                return Ok(true);
            }
            for arm in arms {
                if let Some(guard) = &arm.guard
                    && expression_contains_target_break(guard, target, fuel)?
                {
                    return Ok(true);
                }
                if normal_argument_contains_target_break(&arm.body, target, fuel)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        n::NormalFormArgument::Literal { .. } => Ok(false),
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
                |_| false,
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
