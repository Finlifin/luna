//! Bounded traversal of the structured operations shared with normalization.
use crate::program::{Expr, ExprKind as K, Function, Pattern as P};

/// The callback may retain references into the borrowed immutable function,
/// allowing analyses to index nodes without cloning expression subtrees.
pub fn expressions<'a>(
    function: &'a Function,
    fuel: &mut usize,
    visit: impl FnMut(&'a Expr, &mut usize) -> Result<(), String>,
) -> Result<(), String> {
    walk(function, fuel, visit, true)
}
/// Visit one runtime environment, excluding defaults, closure bodies and
/// handler arms evaluated in their own resource contexts.
pub fn expressions_in_scope<'a>(
    function: &'a Function,
    fuel: &mut usize,
    visit: impl FnMut(&'a Expr, &mut usize) -> Result<(), String>,
) -> Result<(), String> {
    walk(function, fuel, visit, false)
}
/// Include candidate computations belonging to this runtime scope, without
/// treating them as preconditions or entering nested handler/closure scopes.
pub fn expressions_in_scope_with_observations<'a>(
    function: &'a Function,
    observations: &'a [Expr],
    fuel: &mut usize,
    mut visit: impl FnMut(&'a Expr, &mut usize) -> Result<(), String>,
) -> Result<(), String> {
    let roots = std::iter::once(&function.body)
        .chain(&function.preconditions)
        .chain(&function.postconditions)
        .chain(observations);
    walk_roots(
        roots,
        fuel,
        |expr, fuel| visit(expr, fuel).map(|_| true),
        false,
        false,
    )
}

/// Extract only expression roots from a pattern. Descendants remain attached
/// to their root so a later expression walk observes every runtime node once.
pub fn into_pattern_observations(pattern: P, fuel: &mut usize) -> Result<Vec<Expr>, String> {
    let mut pending = vec![pattern];
    let mut roots = vec![];
    while let Some(pattern) = pending.pop() {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("pattern observation budget exhausted")?;
        match pattern {
            P::Then { first, value, next } => {
                roots.push(*value);
                pending.push(*next);
                pending.push(*first);
            }
            P::Guard { pattern, condition } => {
                roots.push(*condition);
                pending.push(*pattern);
            }
            P::OptionalPresent(pattern)
            | P::ErrorResult { inner: pattern, .. }
            | P::Observe { inner: pattern, .. }
            | P::Remap { pattern, .. }
            | P::Binding { inner: pattern, .. } => pending.push(*pattern),
            P::Tuple(fields)
            | P::TypeTuple(fields)
            | P::Or(fields)
            | P::NominalType { fields, .. }
            | P::Record { fields, .. }
            | P::AnonymousRecord { fields, .. }
            | P::Variant { fields, .. }
            | P::NormalForm {
                arguments: fields, ..
            } => {
                *fuel = fuel
                    .checked_sub(fields.len())
                    .ok_or("pattern observation field budget exhausted")?;
                pending.extend(fields.into_iter().rev());
            }
            P::CodeEvidence { .. } => return Err("unresolved pattern-code evidence".into()),
            P::OptionalAbsent | P::Wild | P::Unit | P::Literal(_) => (),
        }
    }
    Ok(roots)
}
/// Borrow pattern computation roots in structural evaluation order. This does
/// not decide which alternatives execute; callers retain that obligation.
pub fn pattern_observations<'a>(pattern: &'a P, fuel: &mut usize) -> Result<Vec<&'a Expr>, String> {
    enum Node<'a> {
        Pattern(&'a P),
        Expr(&'a Expr),
    }
    let mut pending = vec![Node::Pattern(pattern)];
    let mut roots = vec![];
    while let Some(node) = pending.pop() {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("pattern observation budget exhausted")?;
        let pattern = match node {
            Node::Expr(expr) => {
                roots.push(expr);
                continue;
            }
            Node::Pattern(pattern) => pattern,
        };
        match pattern {
            P::Then { first, value, next } => {
                pending.push(Node::Pattern(next));
                pending.push(Node::Expr(value));
                pending.push(Node::Pattern(first));
            }
            P::Guard { pattern, condition } => {
                pending.push(Node::Expr(condition));
                pending.push(Node::Pattern(pattern));
            }
            P::OptionalPresent(pattern)
            | P::ErrorResult { inner: pattern, .. }
            | P::Observe { inner: pattern, .. }
            | P::Remap { pattern, .. }
            | P::Binding { inner: pattern, .. } => pending.push(Node::Pattern(pattern)),
            P::Tuple(fields)
            | P::TypeTuple(fields)
            | P::Or(fields)
            | P::NominalType { fields, .. }
            | P::Record { fields, .. }
            | P::AnonymousRecord { fields, .. }
            | P::Variant { fields, .. }
            | P::NormalForm {
                arguments: fields, ..
            } => {
                *fuel = fuel
                    .checked_sub(fields.len())
                    .ok_or("pattern observation field budget exhausted")?;
                pending.extend(fields.iter().rev().map(Node::Pattern));
            }
            P::CodeEvidence { .. } => return Err("unresolved pattern-code evidence".into()),
            P::OptionalAbsent | P::Wild | P::Unit | P::Literal(_) => (),
        }
    }
    Ok(roots)
}
/// Visit the evaluations in one expression's current runtime scope. Closure
/// bodies, handler arms and call defaults have independent analysis contexts.
pub fn expression_in_scope<'a>(
    expression: &'a Expr,
    fuel: &mut usize,
    mut visit: impl FnMut(&'a Expr, &mut usize) -> Result<(), String>,
) -> Result<(), String> {
    walk_roots(
        std::iter::once(expression),
        fuel,
        |expr, fuel| visit(expr, fuel).map(|_| true),
        false,
        false,
    )
}
/// Visit accesses to the current closure environment. Handler arms, defaults,
/// and capture initializers remain visible, but newly constructed closure bodies
/// operate on their own owned capture cells and are not entered.
pub fn expression_in_closure_environment<'a>(
    expression: &'a Expr,
    fuel: &mut usize,
    mut visit: impl FnMut(&'a Expr, &mut usize) -> Result<(), String>,
) -> Result<(), String> {
    walk_roots(
        std::iter::once(expression),
        fuel,
        |expr, fuel| visit(expr, fuel).map(|_| true),
        true,
        false,
    )
}
/// Visit a complete expression tree, including nested closure bodies and defaults.
pub fn expression<'a>(
    expression: &'a Expr,
    fuel: &mut usize,
    mut visit: impl FnMut(&'a Expr, &mut usize) -> Result<(), String>,
) -> Result<(), String> {
    walk_roots(
        std::iter::once(expression),
        fuel,
        |expr, fuel| visit(expr, fuel).map(|_| true),
        true,
        true,
    )
}
/// Returning false delegates this subtree to a scope-aware validator.
pub fn expression_pruned<'a>(
    expression: &'a Expr,
    fuel: &mut usize,
    visit: impl FnMut(&'a Expr, &mut usize) -> Result<bool, String>,
) -> Result<(), String> {
    walk_roots(std::iter::once(expression), fuel, visit, true, true)
}
fn walk<'a>(
    function: &'a Function,
    fuel: &mut usize,
    visit: impl FnMut(&'a Expr, &mut usize) -> Result<(), String>,
    include_defaults: bool,
) -> Result<(), String> {
    let roots = std::iter::once(&function.body)
        .chain(&function.preconditions)
        .chain(&function.postconditions)
        .chain(
            function
                .defaults
                .iter()
                .filter(|_| include_defaults)
                .map(|(_, expression)| expression),
        );
    let mut visit = visit;
    walk_roots(
        roots,
        fuel,
        |expr, fuel| visit(expr, fuel).map(|_| true),
        include_defaults,
        include_defaults,
    )
}
fn walk_roots<'a>(
    roots: impl Iterator<Item = &'a Expr>,
    fuel: &mut usize,
    mut visit: impl FnMut(&'a Expr, &mut usize) -> Result<bool, String>,
    include_defaults: bool,
    include_closure_bodies: bool,
) -> Result<(), String> {
    enum Node<'a> {
        Expr(&'a Expr),
        Pattern(&'a P),
    }
    let mut work: Vec<_> = roots.map(Node::Expr).collect();
    while let Some(node) = work.pop() {
        if *fuel == 0 {
            return Err("IIR traversal budget exhausted".into());
        }
        *fuel -= 1;
        match node {
            Node::Pattern(pattern) => match pattern {
                P::Then { first, value, next } => {
                    work.push(Node::Pattern(first));
                    work.push(Node::Expr(value));
                    work.push(Node::Pattern(next));
                }
                P::Guard { pattern, condition } => {
                    work.push(Node::Pattern(pattern));
                    work.push(Node::Expr(condition));
                }
                P::OptionalPresent(pattern)
                | P::ErrorResult { inner: pattern, .. }
                | P::Observe { inner: pattern, .. }
                | P::Remap { pattern, .. }
                | P::Binding { inner: pattern, .. } => work.push(Node::Pattern(pattern)),
                P::AnonymousRecord { names, fields } => {
                    validate_record_pattern(names, fields.len(), fuel)?;
                    work.extend(fields.iter().map(Node::Pattern));
                }
                P::NominalType { fields, .. }
                | P::Record { fields, .. }
                | P::Variant { fields, .. }
                | P::NormalForm {
                    arguments: fields, ..
                }
                | P::Tuple(fields)
                | P::TypeTuple(fields)
                | P::Or(fields) => work.extend(fields.iter().map(Node::Pattern)),
                P::CodeEvidence { .. } => return Err("unresolved pattern-code evidence".into()),
                P::OptionalAbsent | P::Wild | P::Unit | P::Literal(_) => {}
            },
            Node::Expr(expr) => {
                if !visit(expr, fuel)? {
                    continue;
                }
                match &expr.kind {
                    K::EffectHandle {
                        computation,
                        handlers,
                        ..
                    } => {
                        work.push(Node::Expr(computation));
                        if include_defaults {
                            for handler in handlers {
                                for arm in &handler.arms {
                                    work.push(Node::Pattern(&arm.pattern));
                                    if let Some(guard) = &arm.guard {
                                        work.push(Node::Expr(guard));
                                    }
                                    work.push(Node::Expr(&arm.body));
                                }
                            }
                        }
                    }
                    K::EffectResume(body)
                    | K::EffectComputation { body, .. }
                    | K::EffectRun {
                        computation: body, ..
                    } => work.push(Node::Expr(body)),
                    K::EffectRequest { arguments, .. } => {
                        work.extend(arguments.iter().map(Node::Expr))
                    }
                    K::Closure { captures, body, .. } => {
                        if include_closure_bodies {
                            work.push(Node::Expr(body));
                        }
                        work.extend(
                            captures
                                .iter()
                                .filter_map(|c| c.value.as_deref())
                                .map(Node::Expr),
                        );
                    }
                    K::NormalFormType {
                        arguments, plan, ..
                    } => {
                        work.extend(
                            plan.iter()
                                .flat_map(|p| &p.preparations)
                                .map(|p| Node::Expr(&p.expression)),
                        );
                        work.extend(
                            arguments
                                .iter()
                                .flat_map(|a| a.patterns())
                                .map(Node::Pattern),
                        );
                        work.extend(
                            arguments
                                .iter()
                                .flat_map(|a| a.expressions())
                                .map(Node::Expr),
                        );
                    }
                    K::NominalType {
                        arguments,
                        preparations,
                        ..
                    } => {
                        work.extend(preparations.iter().map(|p| Node::Expr(&p.expression)));
                        work.extend(
                            arguments
                                .iter()
                                .flat_map(|a| a.patterns())
                                .map(Node::Pattern),
                        );
                        work.extend(
                            arguments
                                .iter()
                                .flat_map(|a| a.expressions())
                                .map(Node::Expr),
                        );
                    }
                    K::ClosureType {
                        environment,
                        signature,
                    } => {
                        work.extend(environment.iter().map(Node::Expr));
                        work.push(Node::Expr(signature));
                    }
                    K::CheckTypeAlias {
                        arguments,
                        body,
                        preparations,
                        ..
                    } => {
                        work.extend(preparations.iter().map(|p| Node::Expr(&p.expression)));
                        work.extend(
                            arguments
                                .iter()
                                .flat_map(|a| a.patterns())
                                .map(Node::Pattern),
                        );
                        work.extend(
                            arguments
                                .iter()
                                .flat_map(|a| a.expressions())
                                .map(Node::Expr),
                        );
                        if let Some(body) = body {
                            work.push(Node::Expr(body));
                        }
                    }
                    K::Variant { fields, .. }
                    | K::Object { fields, .. }
                    | K::AnonymousRecord { fields }
                    | K::Record { fields, .. }
                    | K::FunctionType {
                        parameters: fields, ..
                    }
                    | K::TypeList(fields)
                    | K::TupleType(fields)
                    | K::Intrinsic { inputs: fields, .. }
                    | K::Tuple(fields) => work.extend(fields.iter().map(Node::Expr)),
                    K::BitAssign { value, .. }
                    | K::AnonymousField { value, .. }
                    | K::Field { value, .. }
                    | K::Let { value, .. }
                    | K::Assign { value, .. }
                    | K::UnsafeScope(value)
                    | K::Discard(value)
                    | K::RequirePredicate(value)
                    | K::EnsurePredicate(value)
                    | K::CheckPredicate(value)
                    | K::BitSlice { value, .. }
                    | K::AddressOf(value)
                    | K::ReadPointer(value)
                    | K::PointerAnonymousField { pointer: value, .. }
                    | K::PointerField { pointer: value, .. }
                    | K::OptionalPresent(value)
                    | K::OptionalType(value)
                    | K::PointerType(value)
                    | K::BitVecType(value)
                    | K::IntegerNegate(value)
                    | K::FloatNegate(value)
                    | K::BitConvert(value)
                    | K::ErrorConvert { value: value, .. }
                    | K::NewtypeConvert(value)
                    | K::Snapshot(value)
                    | K::TakeBits(value)
                    | K::SymbolName(value)
                    | K::EnumName(value)
                    | K::Not(value)
                    | K::Loop { body: value, .. } => work.push(Node::Expr(value)),
                    K::BitWritePointer {
                        pointer,
                        selections,
                        value,
                    } => {
                        *fuel = fuel
                            .checked_sub(selections.len())
                            .ok_or("bit selection traversal budget exhausted")?;
                        work.push(Node::Expr(pointer));
                        work.extend(
                            selections
                                .iter()
                                .flat_map(crate::program::BitSelection::expressions)
                                .map(Node::Expr),
                        );
                        work.push(Node::Expr(value));
                    }
                    K::BitUpdate {
                        selections, value, ..
                    } => {
                        *fuel = fuel
                            .checked_sub(selections.len())
                            .ok_or("bit selection traversal budget exhausted")?;
                        work.extend(
                            selections
                                .iter()
                                .flat_map(crate::program::BitSelection::expressions)
                                .map(Node::Expr),
                        );
                        work.push(Node::Expr(value));
                    }
                    K::BitWindow { value, start, end } => {
                        work.push(Node::Expr(value));
                        work.push(Node::Expr(start));
                        work.push(Node::Expr(end));
                    }
                    K::Match { value, arms } => {
                        work.push(Node::Expr(value));
                        for arm in arms {
                            work.push(Node::Pattern(&arm.pattern));
                            work.extend(arm.guard.iter().map(Node::Expr));
                            work.push(Node::Expr(&arm.body));
                        }
                    }
                    K::Block { statements, result } | K::CallPreparation { statements, result } => {
                        work.extend(statements.iter().map(Node::Expr));
                        work.extend(result.as_deref().map(Node::Expr));
                    }
                    K::LetPattern {
                        pattern,
                        value,
                        guard,
                        otherwise,
                    } => {
                        work.push(Node::Pattern(pattern));
                        work.push(Node::Expr(value));
                        work.extend(guard.as_deref().map(Node::Expr));
                        work.extend(otherwise.as_deref().map(Node::Expr));
                    }
                    K::If {
                        condition,
                        then_branch,
                        else_branch,
                    } => {
                        work.push(Node::Expr(condition));
                        work.push(Node::Expr(then_branch));
                        work.extend(else_branch.as_deref().map(Node::Expr));
                    }
                    K::Return(value) => work.extend(value.as_deref().map(Node::Expr)),
                    K::Call {
                        callee, arguments, ..
                    } => {
                        work.push(Node::Expr(callee));
                        work.extend(
                            arguments
                                .iter()
                                .filter(|a| include_defaults || !a.is_default)
                                .map(|a| Node::Expr(&a.value)),
                        );
                    }
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
                    | K::TemplateEqual(a, b)
                    | K::ScalarEqual(a, b)
                    | K::TypeEqual(a, b)
                    | K::TypeListIndex(a, b)
                    | K::BitEqual(a, b)
                    | K::BitConcat(a, b)
                    | K::BitIndex(a, b)
                    | K::And(a, b)
                    | K::Or(a, b) => {
                        work.push(Node::Expr(a));
                        work.push(Node::Expr(b));
                    }
                    K::AssociatedValue { .. }
                    | K::ComptimeValue(_)
                    | K::TypeValue(_)
                    | K::TypeListValue(_)
                    | K::Literal(_)
                    | K::Local(_)
                    | K::OptionalAbsent
                    | K::Unit
                    | K::Break(_)
                    | K::Continue(_)
                    | K::Function(_)
                    | K::FunctionInstance { .. }
                    | K::TraitMethod { .. } => {}
                }
            }
        }
    }
    Ok(())
}

/// Rewrite a private expression tree with a bounded explicit work stack.
/// Newly substituted descendants are visited; failures leave a private partial
/// result, which callers must discard. Shared closure bodies use copy-on-write.
pub fn expression_mut(
    expression: &mut Expr,
    fuel: &mut usize,
    mut visit: impl FnMut(&mut Expr, &mut usize) -> Result<(), String>,
) -> Result<(), String> {
    expression_mut_pruned(expression, fuel, |expr, fuel| {
        visit(expr, fuel).map(|_| true)
    })
}

/// Bounded rewriting with explicit subtree ownership. Returning false leaves
/// newly substituted descendants untouched, permitting simultaneous substitution
/// when inserted references belong to the caller's frame.
pub fn expression_mut_pruned(
    expression: &mut Expr,
    fuel: &mut usize,
    visit: impl FnMut(&mut Expr, &mut usize) -> Result<bool, String>,
) -> Result<(), String> {
    walk_mut(expression, fuel, visit, true, true)
}

/// Rewrite formation-time evaluations without entering newly constructed
/// closure bodies. Capture initializers, defaults and handler arms remain
/// visible, matching expression_in_closure_environment. Failures leave a
/// private partial result, which the caller must discard.
pub fn expression_mut_in_closure_environment(
    expression: &mut Expr,
    fuel: &mut usize,
    mut visit: impl FnMut(&mut Expr, &mut usize) -> Result<(), String>,
) -> Result<(), String> {
    walk_mut(
        expression,
        fuel,
        |expr, fuel| visit(expr, fuel).map(|_| true),
        false,
        true,
    )
}

/// Rewrite the current handler's evaluations without entering nested handler
/// arms or newly formed closure bodies. A nested handler's computation and
/// closure capture initializers are still evaluated in the enclosing scope.
pub fn expression_mut_in_handler_scope(
    expression: &mut Expr,
    fuel: &mut usize,
    mut visit: impl FnMut(&mut Expr, &mut usize) -> Result<(), String>,
) -> Result<(), String> {
    walk_mut(
        expression,
        fuel,
        |expr, fuel| visit(expr, fuel).map(|_| true),
        false,
        false,
    )
}

fn walk_mut(
    expression: &mut Expr,
    fuel: &mut usize,
    mut visit: impl FnMut(&mut Expr, &mut usize) -> Result<bool, String>,
    include_closure_bodies: bool,
    include_handler_arms: bool,
) -> Result<(), String> {
    enum Node<'a> {
        Expr(&'a mut Expr),
        Pattern(&'a mut P),
        Argument(&'a mut crate::program::NormalFormArgument),
    }
    let mut work: Vec<_> = vec![Node::Expr(expression)];
    while let Some(node) = work.pop() {
        if *fuel == 0 {
            return Err("IIR traversal budget exhausted".into());
        }
        *fuel -= 1;
        match node {
            Node::Argument(argument) => {
                use crate::program::NormalFormArgument as A;
                match argument {
                    A::TypeListSpread(value) | A::Value(value) => work.push(Node::Expr(value)),
                    A::Unit { .. } | A::Integer { .. } => {}
                    A::If {
                        condition,
                        then_input,
                        else_input,
                        ..
                    } => {
                        work.push(Node::Expr(condition));
                        work.push(Node::Argument(then_input));
                        work.push(Node::Argument(else_input));
                    }
                    A::Block {
                        statements, result, ..
                    } => {
                        work.extend(statements.iter_mut().map(Node::Expr));
                        work.extend(result.as_deref_mut().map(Node::Argument));
                    }
                    A::TypeList { items, .. } | A::Tuple { items, .. } => {
                        work.extend(items.iter_mut().map(Node::Argument))
                    }
                    A::Match { value, arms, .. } => {
                        work.push(Node::Expr(value));
                        for arm in arms {
                            work.push(Node::Pattern(&mut arm.pattern));
                            work.extend(arm.guard.iter_mut().map(Node::Expr));
                            work.push(Node::Argument(&mut arm.body));
                        }
                    }
                }
            }
            Node::Pattern(pattern) => match pattern {
                P::Then { first, value, next } => {
                    work.push(Node::Pattern(first));
                    work.push(Node::Expr(value));
                    work.push(Node::Pattern(next));
                }
                P::Guard { pattern, condition } => {
                    work.push(Node::Pattern(pattern));
                    work.push(Node::Expr(condition));
                }
                P::OptionalPresent(pattern)
                | P::ErrorResult { inner: pattern, .. }
                | P::Observe { inner: pattern, .. }
                | P::Remap { pattern, .. }
                | P::Binding { inner: pattern, .. } => work.push(Node::Pattern(pattern)),
                P::AnonymousRecord { names, fields } => {
                    validate_record_pattern(names, fields.len(), fuel)?;
                    work.extend(fields.iter_mut().map(Node::Pattern));
                }
                P::NominalType { fields, .. }
                | P::Record { fields, .. }
                | P::Variant { fields, .. }
                | P::NormalForm {
                    arguments: fields, ..
                }
                | P::Tuple(fields)
                | P::TypeTuple(fields)
                | P::Or(fields) => work.extend(fields.iter_mut().map(Node::Pattern)),
                P::CodeEvidence { .. } => return Err("unresolved pattern-code evidence".into()),
                P::OptionalAbsent | P::Wild | P::Unit | P::Literal(_) => {}
            },
            Node::Expr(expr) => {
                if !visit(expr, fuel)? {
                    continue;
                }
                match &mut expr.kind {
                    K::EffectHandle {
                        computation,
                        handlers,
                        ..
                    } => {
                        work.push(Node::Expr(computation));
                        for handler in handlers.iter_mut().filter(|_| include_handler_arms) {
                            for arm in &mut handler.arms {
                                work.push(Node::Pattern(&mut arm.pattern));
                                if let Some(guard) = &mut arm.guard {
                                    work.push(Node::Expr(guard));
                                }
                                work.push(Node::Expr(&mut arm.body));
                            }
                        }
                    }
                    K::EffectResume(body)
                    | K::EffectComputation { body, .. }
                    | K::EffectRun {
                        computation: body, ..
                    } => work.push(Node::Expr(body)),
                    K::EffectRequest { arguments, .. } => {
                        work.extend(arguments.iter_mut().map(Node::Expr))
                    }
                    K::Closure { captures, body, .. } => {
                        if include_closure_bodies {
                            work.push(Node::Expr(std::sync::Arc::make_mut(body)));
                        }
                        work.extend(
                            captures
                                .iter_mut()
                                .filter_map(|c| c.value.as_deref_mut())
                                .map(Node::Expr),
                        );
                    }
                    K::NormalFormType {
                        arguments, plan, ..
                    } => {
                        work.extend(
                            plan.iter_mut()
                                .flat_map(|p| &mut p.preparations)
                                .map(|p| Node::Expr(&mut p.expression)),
                        );
                        work.extend(arguments.iter_mut().map(Node::Argument));
                    }
                    K::NominalType {
                        arguments,
                        preparations,
                        ..
                    } => {
                        work.extend(
                            preparations
                                .iter_mut()
                                .map(|p| Node::Expr(&mut p.expression)),
                        );
                        work.extend(arguments.iter_mut().map(Node::Argument));
                    }
                    K::ClosureType {
                        environment,
                        signature,
                    } => {
                        work.extend(environment.iter_mut().map(Node::Expr));
                        work.push(Node::Expr(signature));
                    }
                    K::CheckTypeAlias {
                        arguments,
                        body,
                        preparations,
                        ..
                    } => {
                        work.extend(
                            preparations
                                .iter_mut()
                                .map(|p| Node::Expr(&mut p.expression)),
                        );
                        work.extend(arguments.iter_mut().map(Node::Argument));
                        if let Some(body) = body {
                            work.push(Node::Expr(body));
                        }
                    }
                    K::Variant { fields, .. }
                    | K::Object { fields, .. }
                    | K::AnonymousRecord { fields }
                    | K::Record { fields, .. }
                    | K::FunctionType {
                        parameters: fields, ..
                    }
                    | K::TypeList(fields)
                    | K::TupleType(fields)
                    | K::Intrinsic { inputs: fields, .. }
                    | K::Tuple(fields) => work.extend(fields.iter_mut().map(Node::Expr)),
                    K::BitAssign { value, .. }
                    | K::AnonymousField { value, .. }
                    | K::Field { value, .. }
                    | K::Let { value, .. }
                    | K::Assign { value, .. }
                    | K::UnsafeScope(value)
                    | K::Discard(value)
                    | K::RequirePredicate(value)
                    | K::EnsurePredicate(value)
                    | K::CheckPredicate(value)
                    | K::BitSlice { value, .. }
                    | K::AddressOf(value)
                    | K::ReadPointer(value)
                    | K::PointerAnonymousField { pointer: value, .. }
                    | K::PointerField { pointer: value, .. }
                    | K::OptionalPresent(value)
                    | K::OptionalType(value)
                    | K::PointerType(value)
                    | K::BitVecType(value)
                    | K::IntegerNegate(value)
                    | K::FloatNegate(value)
                    | K::BitConvert(value)
                    | K::ErrorConvert { value: value, .. }
                    | K::NewtypeConvert(value)
                    | K::Snapshot(value)
                    | K::TakeBits(value)
                    | K::SymbolName(value)
                    | K::EnumName(value)
                    | K::Not(value)
                    | K::Loop { body: value, .. } => work.push(Node::Expr(value)),
                    K::BitWritePointer {
                        pointer,
                        selections,
                        value,
                    } => {
                        *fuel = fuel
                            .checked_sub(selections.len())
                            .ok_or("bit selection traversal budget exhausted")?;
                        work.push(Node::Expr(pointer));
                        work.extend(
                            selections
                                .iter_mut()
                                .flat_map(crate::program::BitSelection::expressions_mut)
                                .map(Node::Expr),
                        );
                        work.push(Node::Expr(value));
                    }
                    K::BitUpdate {
                        selections, value, ..
                    } => {
                        *fuel = fuel
                            .checked_sub(selections.len())
                            .ok_or("bit selection traversal budget exhausted")?;
                        work.extend(
                            selections
                                .iter_mut()
                                .flat_map(crate::program::BitSelection::expressions_mut)
                                .map(Node::Expr),
                        );
                        work.push(Node::Expr(value));
                    }
                    K::BitWindow { value, start, end } => {
                        work.push(Node::Expr(value));
                        work.push(Node::Expr(start));
                        work.push(Node::Expr(end));
                    }
                    K::Match { value, arms } => {
                        work.push(Node::Expr(value));
                        for arm in arms {
                            work.push(Node::Pattern(&mut arm.pattern));
                            work.extend(arm.guard.iter_mut().map(Node::Expr));
                            work.push(Node::Expr(&mut arm.body));
                        }
                    }
                    K::Block { statements, result } | K::CallPreparation { statements, result } => {
                        work.extend(statements.iter_mut().map(Node::Expr));
                        work.extend(result.as_deref_mut().map(Node::Expr));
                    }
                    K::LetPattern {
                        pattern,
                        value,
                        guard,
                        otherwise,
                    } => {
                        work.push(Node::Pattern(pattern));
                        work.push(Node::Expr(value));
                        work.extend(guard.as_deref_mut().map(Node::Expr));
                        work.extend(otherwise.as_deref_mut().map(Node::Expr));
                    }
                    K::If {
                        condition,
                        then_branch,
                        else_branch,
                    } => {
                        work.push(Node::Expr(condition));
                        work.push(Node::Expr(then_branch));
                        work.extend(else_branch.as_deref_mut().map(Node::Expr));
                    }
                    K::Return(value) => work.extend(value.as_deref_mut().map(Node::Expr)),
                    K::Call {
                        callee, arguments, ..
                    } => {
                        work.push(Node::Expr(callee));
                        work.extend(arguments.iter_mut().map(|a| Node::Expr(&mut a.value)));
                    }
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
                    | K::TemplateEqual(a, b)
                    | K::ScalarEqual(a, b)
                    | K::TypeEqual(a, b)
                    | K::TypeListIndex(a, b)
                    | K::BitEqual(a, b)
                    | K::BitConcat(a, b)
                    | K::BitIndex(a, b)
                    | K::And(a, b)
                    | K::Or(a, b) => {
                        work.push(Node::Expr(a));
                        work.push(Node::Expr(b));
                    }
                    K::AssociatedValue { .. }
                    | K::ComptimeValue(_)
                    | K::TypeValue(_)
                    | K::TypeListValue(_)
                    | K::Literal(_)
                    | K::Local(_)
                    | K::OptionalAbsent
                    | K::Unit
                    | K::Break(_)
                    | K::Continue(_)
                    | K::Function(_)
                    | K::FunctionInstance { .. }
                    | K::TraitMethod { .. } => {}
                }
            }
        }
    }
    Ok(())
}

fn validate_record_pattern(
    names: &[String],
    fields: usize,
    fuel: &mut usize,
) -> Result<(), String> {
    if names.len() != fields {
        return Err("anonymous record pattern arity mismatch".into());
    }
    let mut seen = std::collections::BTreeSet::new();
    for name in names {
        *fuel = fuel
            .checked_sub(name.len().saturating_add(1))
            .ok_or("anonymous record pattern budget exhausted")?;
        if !seen.insert(name) {
            return Err("duplicate anonymous record pattern label".into());
        }
    }
    Ok(())
}

/// Types embedded directly in patterns. Embedded expression trees are visited
/// by the ordinary expression walker, so they are not charged twice here.
pub fn pattern_types(
    root: &P,
    fuel: &mut usize,
    mut visit: impl FnMut(&crate::Term, &mut usize) -> Result<(), String>,
) -> Result<(), String> {
    let mut pending = vec![root];
    while let Some(pattern) = pending.pop() {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("pattern type traversal budget exhausted")?;
        match pattern {
            P::ErrorResult { label, inner } => {
                if let Some(label) = label {
                    visit(label, fuel)?;
                }
                pending.push(inner);
            }
            P::CodeEvidence { outputs } => {
                for (_, ty) in outputs {
                    visit(ty, fuel)?;
                }
            }
            P::OptionalPresent(inner)
            | P::Observe { inner, .. }
            | P::Binding { inner, .. }
            | P::Remap { pattern: inner, .. }
            | P::Guard { pattern: inner, .. } => pending.push(inner),
            P::Then { first, next, .. } => {
                pending.push(first);
                pending.push(next);
            }
            P::Tuple(fields)
            | P::TypeTuple(fields)
            | P::Or(fields)
            | P::NominalType { fields, .. }
            | P::Record { fields, .. }
            | P::AnonymousRecord { fields, .. }
            | P::Variant { fields, .. }
            | P::NormalForm {
                arguments: fields, ..
            } => {
                *fuel = fuel
                    .checked_sub(fields.len())
                    .ok_or("pattern type field budget exhausted")?;
                pending.extend(fields);
            }
            P::OptionalAbsent | P::Wild | P::Unit | P::Literal(_) => (),
        }
    }
    Ok(())
}
