//! Optional, transactional removal of proven program checks. This preserves
//! predicate evaluation and only removes the redundant rejection operation.
use super::{Limits, PredicateProof, Unknown, flow};
use nir::{
    DeclarationId, Primitive, Term,
    program::{Expr, ExprKind as K, Package, Pattern, SourceRange},
};
use std::{collections::BTreeMap, fmt, sync::Arc};

#[derive(Default)]
pub struct ElisionReport {
    pub eliminated: Vec<ElidedCheck>,
    pub fallback_functions: usize,
}
pub struct ElidedCheck {
    pub function: DeclarationId,
    pub source: SourceRange,
    pub proof: PredicateProof,
}
impl fmt::Display for ElisionReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(check-elision (fallback-functions {}) (eliminated",
            self.fallback_functions
        )?;
        for check in &self.eliminated {
            write!(f, " {check}")?;
        }
        f.write_str("))")
    }
}
impl fmt::Display for ElidedCheck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(elided-check (function {}) (source {} {}) {})",
            self.function, self.source.start, self.source.end, self.proof
        )
    }
}

/// The baseline must already have passed ordinary frontend validation. An
/// unsupported analysis, exhausted budget or failed post-validation retains
/// that function exactly. No partial rewrite is published on failure.
pub fn elide_proven_checks(package: &mut Package, limits: &mut Limits) -> ElisionReport {
    let mut report = ElisionReport::default();
    for index in 0..package.functions.len() {
        if limits.arithmetic.steps == 0 {
            report.fallback_functions += package.functions.len() - index;
            break;
        }
        let mut candidate = package.functions[index].clone();
        let mut checks = 0;
        // Detach shared closure bodies before analysis. Distinct capture frames
        // must never rewrite one shared body based on just one frame's facts.
        if visit(&mut candidate.body, limits, 0, &mut |expr, _| {
            checks += usize::from(matches!(expr.kind, K::CheckPredicate(_)));
            Ok(())
        })
        .is_err()
        {
            report.fallback_functions += 1;
            continue;
        }
        if checks == 0 {
            continue;
        }
        let Ok(analysis) = flow::analyze_in_package(package, &candidate, limits) else {
            report.fallback_functions += 1;
            continue;
        };
        let mut proven = BTreeMap::<usize, Option<PredicateProof>>::new();
        for input in analysis
            .points
            .iter()
            .filter_map(|p| p.check_input.as_ref())
        {
            let proof = input
                .prove(limits)
                .ok()
                .filter(|p| p.replay(&input.facts, limits).is_ok());
            // If a node is visited in more than one analysis context, every
            // visit must justify removal; one successful visit is insufficient.
            proven
                .entry(input.site)
                .and_modify(|previous| {
                    if proof.is_none() {
                        *previous = None;
                    }
                })
                .or_insert(proof);
        }
        if !proven.values().any(Option::is_some) {
            continue;
        }
        let mut eliminated = Vec::new();
        let result = visit(&mut candidate.body, limits, 0, &mut |expr, _| {
            let site = expr as *const Expr as usize;
            if expr.ty != Term::Primitive(Primitive::Void) {
                return Ok(());
            }
            let K::CheckPredicate(condition) = &expr.kind else {
                return Ok(());
            };
            if !nir::program::checkable_predicate(condition) {
                return Ok(());
            }
            let Some(Some(proof)) = proven.remove(&site) else {
                return Ok(());
            };
            let K::CheckPredicate(condition) = std::mem::replace(&mut expr.kind, K::Unit) else {
                unreachable!()
            };
            expr.kind = K::Discard(condition);
            eliminated.push(ElidedCheck {
                function: candidate.declaration,
                source: expr.source,
                proof,
            });
            Ok(())
        });
        if result.is_err() || flow::analyze_in_package(package, &candidate, limits).is_err() {
            report.fallback_functions += 1;
            continue;
        }
        if !eliminated.is_empty() {
            package.functions[index] = candidate;
            report.eliminated.extend(eliminated);
        }
    }
    report
}

// This traversal does not reallocate expression vectors or boxes. Arc bodies
// are detached during the initial pass, before ephemeral site IDs are recorded.
fn visit(
    expr: &mut Expr,
    limits: &mut Limits,
    depth: usize,
    action: &mut impl FnMut(&mut Expr, &mut Limits) -> Result<(), Unknown>,
) -> Result<(), Unknown> {
    limits.step(1)?;
    if depth >= 128 {
        return Err(Unknown::ConditionDepthLimit);
    }
    action(expr, limits)?;
    let depth = depth + 1;
    match &mut expr.kind {
        K::EffectHandle {
            computation,
            handlers,
            ..
        } => {
            visit(computation, limits, depth, action)?;
            for handler in handlers {
                for arm in &mut handler.arms {
                    visit_pattern(&mut arm.pattern, limits, depth, action)?;
                    if let Some(g) = &mut arm.guard {
                        visit(g, limits, depth, action)?;
                    }
                    visit(&mut arm.body, limits, depth, action)?;
                }
            }
        }
        K::EffectResume(body)
        | K::EffectComputation { body, .. }
        | K::EffectRun {
            computation: body, ..
        } => visit(body, limits, depth, action)?,
        K::EffectRequest { arguments, .. } => {
            for argument in arguments {
                visit(argument, limits, depth, action)?;
            }
        }
        K::Closure { captures, body, .. } => {
            for capture in captures {
                if let Some(value) = &mut capture.value {
                    visit(value, limits, depth, action)?;
                }
            }
            visit(Arc::make_mut(body), limits, depth, action)?;
        }
        K::NormalFormType {
            arguments, plan, ..
        } => {
            for p in plan.iter_mut().flat_map(|p| &mut p.preparations) {
                visit(&mut p.expression, limits, depth, action)?;
            }
            for argument in arguments {
                for pattern in argument.patterns_mut() {
                    visit_pattern(pattern, limits, depth, action)?;
                }
                for value in argument.expressions_mut() {
                    visit(value, limits, depth, action)?;
                }
            }
        }
        K::ClosureType {
            environment,
            signature,
        } => {
            for field in environment {
                visit(field, limits, depth, action)?;
            }
            visit(signature, limits, depth, action)?;
        }
        K::CheckTypeAlias {
            arguments: fields, ..
        }
        | K::NominalType {
            arguments: fields, ..
        }
        | K::FunctionType {
            parameters: fields, ..
        }
        | K::TypeList(fields)
        | K::TupleType(fields)
        | K::Tuple(fields)
        | K::AnonymousRecord { fields }
        | K::Record { fields, .. }
        | K::Variant { fields, .. } => {
            for field in fields {
                visit(field, limits, depth, action)?;
            }
        }
        K::AnonymousField { value, .. }
        | K::Field { value, .. }
        | K::Let { value, .. }
        | K::Assign { value, .. }
        | K::BitAssign { value, .. }
        | K::Discard(value)
        | K::RequirePredicate(value)
        | K::EnsurePredicate(value)
        | K::CheckPredicate(value)
        | K::BitSlice { value, .. }
        | K::AddressOf(value)
        | K::ReadPointer(value)
        | K::PointerAnonymousField { pointer: value, .. }
        | K::PointerField { pointer: value, .. }
        | K::PointerType(value)
        | K::BitVecType(value)
        | K::IntegerNegate(value)
        | K::FloatNegate(value)
        | K::BitConvert(value)
        | K::NewtypeConvert(value)
        | K::Snapshot(value)
        | K::TakeBits(value)
        | K::SymbolName(value)
        | K::EnumName(value)
        | K::Not(value)
        | K::Loop { body: value, .. } => visit(value, limits, depth, action)?,
        K::WritePointer {
            pointer: a,
            value: b,
        }
        | K::FunctionResultType {
            signature: a,
            result: b,
        }
        | K::IntegerBinary {
            left: a, right: b, ..
        }
        | K::FloatBinary {
            left: a, right: b, ..
        }
        | K::ScalarLess(a, b)
        | K::ScalarGreater(a, b)
        | K::ScalarGreaterEqual(a, b)
        | K::ScalarLessEqual(a, b)
        | K::ScalarEqual(a, b)
        | K::TypeEqual(a, b)
        | K::TypeListIndex(a, b)
        | K::BitEqual(a, b)
        | K::BitConcat(a, b)
        | K::BitIndex(a, b)
        | K::And(a, b)
        | K::Or(a, b) => {
            visit(a, limits, depth, action)?;
            visit(b, limits, depth, action)?;
        }
        K::BitWritePointer {
            pointer,
            selections,
            value,
        } => {
            visit(pointer, limits, depth, action)?;
            for input in selections
                .iter_mut()
                .flat_map(nir::program::BitSelection::expressions_mut)
            {
                visit(input, limits, depth, action)?;
            }
            visit(value, limits, depth, action)?;
        }
        K::BitUpdate {
            selections, value, ..
        } => {
            for input in selections
                .iter_mut()
                .flat_map(nir::program::BitSelection::expressions_mut)
            {
                visit(input, limits, depth, action)?;
            }
            visit(value, limits, depth, action)?;
        }
        K::BitWindow { value, start, end } => {
            visit(value, limits, depth, action)?;
            visit(start, limits, depth, action)?;
            visit(end, limits, depth, action)?;
        }
        K::Match { value, arms } => {
            visit(value, limits, depth, action)?;
            for arm in arms {
                visit_pattern(&mut arm.pattern, limits, depth, action)?;
                if let Some(guard) = &mut arm.guard {
                    visit(guard, limits, depth, action)?;
                }
                visit(&mut arm.body, limits, depth, action)?;
            }
        }
        K::Block { statements, result } | K::CallPreparation { statements, result } => {
            for statement in statements {
                visit(statement, limits, depth, action)?;
            }
            if let Some(result) = result {
                visit(result, limits, depth, action)?;
            }
        }
        K::LetPattern {
            pattern,
            value,
            guard,
            otherwise,
        } => {
            visit_pattern(pattern, limits, depth, action)?;
            visit(value, limits, depth, action)?;
            if let Some(guard) = guard {
                visit(guard, limits, depth, action)?;
            }
            if let Some(otherwise) = otherwise {
                visit(otherwise, limits, depth, action)?;
            }
        }
        K::If {
            condition,
            then_branch,
            else_branch,
        } => {
            visit(condition, limits, depth, action)?;
            visit(then_branch, limits, depth, action)?;
            if let Some(otherwise) = else_branch {
                visit(otherwise, limits, depth, action)?;
            }
        }
        K::Return(value) => {
            if let Some(value) = value {
                visit(value, limits, depth, action)?;
            }
        }
        K::Call {
            callee, arguments, ..
        } => {
            visit(callee, limits, depth, action)?;
            for argument in arguments {
                visit(&mut argument.value, limits, depth, action)?;
            }
        }
        K::AssociatedValue { .. }
        | K::TypeValue(_)
        | K::TypeListValue(_)
        | K::TraitMethod { .. }
        | K::FunctionInstance { .. }
        | K::Literal(_)
        | K::Local(_)
        | K::Function(_)
        | K::Unit
        | K::Break(_)
        | K::Continue(_) => {}
    }
    Ok(())
}
fn visit_pattern(
    pattern: &mut Pattern,
    limits: &mut Limits,
    depth: usize,
    action: &mut impl FnMut(&mut Expr, &mut Limits) -> Result<(), Unknown>,
) -> Result<(), Unknown> {
    limits.step(1)?;
    if depth >= 128 {
        return Err(Unknown::ConditionDepthLimit);
    }
    let depth = depth + 1;
    match pattern {
        Pattern::Then { first, value, next } => {
            visit_pattern(first, limits, depth, action)?;
            visit(value, limits, depth, action)?;
            visit_pattern(next, limits, depth, action)?;
        }
        Pattern::Guard { pattern, condition } => {
            visit_pattern(pattern, limits, depth, action)?;
            visit(condition, limits, depth, action)?;
        }
        Pattern::Observe { inner: pattern, .. }
        | Pattern::Remap { pattern, .. }
        | Pattern::Binding { inner: pattern, .. } => visit_pattern(pattern, limits, depth, action)?,
        Pattern::Record { fields, .. }
        | Pattern::AnonymousRecord { fields, .. }
        | Pattern::Variant { fields, .. }
        | Pattern::NormalForm {
            arguments: fields, ..
        }
        | Pattern::TypeTuple(fields)
        | Pattern::Tuple(fields)
        | Pattern::Or(fields) => {
            for field in fields {
                visit_pattern(field, limits, depth, action)?;
            }
        }
        Pattern::Wild | Pattern::Unit | Pattern::Literal(_) => {}
    }
    Ok(())
}
