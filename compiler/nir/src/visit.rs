//! Bounded traversal of the structured operations shared with normalization.
use crate::program::{Expr, ExprKind as K, Function, Pattern as P};
#[cfg(test)]
mod tests;
mod work;
use work::Work;

// Queue space is bounded by the remaining minimum node work. Actual node
// charges stay at visitation so pruning and callback budgets retain ownership.
macro_rules! push {
    ($work:expr, $fuel:expr, $node:expr) => {
        $work.push($node, *$fuel)?
    };
}
macro_rules! extend {
    ($work:expr, $fuel:expr, $nodes:expr $(,)?) => {
        $work.extend($nodes, *$fuel)?
    };
}

mod children;

/// Traversal failures remain distinct from errors returned by a visitor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Budget(&'static str),
    Invalid(&'static str),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Budget(message) | Self::Invalid(message) => f.write_str(message),
        }
    }
}
impl std::error::Error for Error {}
impl From<Error> for String {
    fn from(error: Error) -> Self {
        error.to_string()
    }
}

/// The callback may retain references into the borrowed immutable function,
/// allowing analyses to index nodes without cloning expression subtrees.
pub fn expressions<'a>(
    function: &'a Function,
    fuel: &mut usize,
    visit: impl FnMut(&'a Expr, &mut usize) -> Result<(), String>,
) -> Result<(), String> {
    walk(function, fuel, visit, true)
}
/// Preserve typed visitor and traversal failures across all function roots.
pub fn expressions_checked<'a, E: From<Error>>(
    function: &'a Function,
    fuel: &mut usize,
    visit: impl FnMut(&'a Expr, &mut usize) -> Result<(), E>,
) -> Result<(), E> {
    walk(function, fuel, visit, true)
}
/// Inspect original declaration/contract expressions without materializing a
/// synthetic function. Root references are consumed lazily, in work-stack order.
pub fn program_expressions<'a>(
    program: crate::contracts::CallableProgram<'a>,
    fuel: &mut usize,
    mut visit: impl FnMut(&'a Expr, &mut usize) -> Result<(), String>,
) -> Result<(), String> {
    walk_roots(
        program.roots(),
        fuel,
        |expr, fuel| visit(expr, fuel).map(|_| true),
        true,
        true,
    )
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
/// Preserve the callback's error type and explicitly convert traversal failures.
pub fn expression_checked<'a, E: From<Error>>(
    expression: &'a Expr,
    fuel: &mut usize,
    mut visit: impl FnMut(&'a Expr, &mut usize) -> Result<(), E>,
) -> Result<(), E> {
    expression_pruned_checked(expression, fuel, |expr, fuel| {
        visit(expr, fuel).map(|_| true)
    })
}
/// The typed counterpart of `expression_pruned`; both use the same traversal.
pub fn expression_pruned_checked<'a, E: From<Error>>(
    expression: &'a Expr,
    fuel: &mut usize,
    visit: impl FnMut(&'a Expr, &mut usize) -> Result<bool, E>,
) -> Result<(), E> {
    walk_roots(std::iter::once(expression), fuel, visit, true, true)
}
fn walk<'a, E: From<Error>>(
    function: &'a Function,
    fuel: &mut usize,
    visit: impl FnMut(&'a Expr, &mut usize) -> Result<(), E>,
    include_defaults: bool,
) -> Result<(), E> {
    let defaults = if include_defaults {
        function.defaults.as_slice()
    } else {
        &[]
    };
    let roots = std::iter::once(&function.body)
        .chain(&function.preconditions)
        .chain(&function.postconditions)
        .chain(defaults.iter().map(|(_, expression)| expression));
    let mut visit = visit;
    walk_roots(
        roots,
        fuel,
        |expr, fuel| visit(expr, fuel).map(|_| true),
        include_defaults,
        include_defaults,
    )
}
pub(crate) enum Node<'a> {
    Expr(&'a Expr),
    Pattern(&'a P),
    Argument(&'a crate::program::NormalFormArgument),
}

/// A scope-aware analysis owns the order of its binding operations. For all
/// other nodes it can append the same structural children as the full visitor,
/// in stack insertion order, without retaining a recursive visitor callback.
pub(crate) fn append_expression_children<'a, T: From<Node<'a>>>(
    expr: &'a Expr,
    steps: &mut Vec<T>,
    fuel: &mut usize,
) -> Result<(), Error> {
    let mut children = Work::new(steps);
    children::expand(Node::Expr(expr), &mut children, fuel, true, true, false)?;
    work::charge_entries(fuel, children.len())
}

/// Inspect a pending input with the ordinary expression/pattern visitor.
/// Structural placeholders consume fuel even when they contain no typed leaf.
pub fn argument_checked<'a, E: From<Error>>(
    argument: &'a crate::program::NormalFormArgument,
    fuel: &mut usize,
    mut visit: impl FnMut(&'a Expr, &mut usize) -> Result<(), E>,
) -> Result<(), E> {
    walk_nodes(
        std::iter::once(Node::Argument(argument)),
        fuel,
        |expr, fuel| visit(expr, fuel).map(|_| true),
        true,
        true,
        false,
    )
}

/// Discover dependencies in retained code, including defaults and closures.
/// Pattern evidence has no executable expression children and remains opaque
/// here. This does not certify it for execution; ordinary visitors still reject
/// unresolved evidence. All structural placeholders consume the same budget.
pub fn retained_expressions_checked<'a, E: From<Error>>(
    roots: impl DoubleEndedIterator<Item = &'a Expr>,
    fuel: &mut usize,
    mut visit: impl FnMut(&'a Expr, &mut usize) -> Result<(), E>,
) -> Result<(), E> {
    walk_nodes(
        roots.map(Node::Expr),
        fuel,
        |expr, fuel| visit(expr, fuel).map(|_| true),
        true,
        true,
        true,
    )
}

fn walk_roots<'a, E: From<Error>>(
    roots: impl DoubleEndedIterator<Item = &'a Expr>,
    fuel: &mut usize,
    visit: impl FnMut(&'a Expr, &mut usize) -> Result<bool, E>,
    include_defaults: bool,
    include_closure_bodies: bool,
) -> Result<(), E> {
    walk_nodes(
        roots.map(Node::Expr),
        fuel,
        visit,
        include_defaults,
        include_closure_bodies,
        false,
    )
}

fn walk_nodes<'a, E: From<Error>>(
    roots: impl DoubleEndedIterator<Item = Node<'a>>,
    fuel: &mut usize,
    mut visit: impl FnMut(&'a Expr, &mut usize) -> Result<bool, E>,
    include_defaults: bool,
    include_closure_bodies: bool,
    allow_pattern_evidence: bool,
) -> Result<(), E> {
    // Keep root order without allocating one slot per contract/default. The
    // same lazy root stream serves ordinary functions and borrowed summaries.
    let mut roots = roots.rev();
    let mut nodes = Vec::new();
    let mut work = Work::new(&mut nodes);
    while let Some(node) = work.pop().or_else(|| roots.next()) {
        if *fuel == 0 {
            return Err(Error::Budget("IIR traversal budget exhausted").into());
        }
        *fuel -= 1;
        if let Node::Expr(expr) = &node
            && !visit(expr, fuel)?
        {
            continue;
        }
        children::expand(
            node,
            &mut work,
            fuel,
            include_defaults,
            include_closure_bodies,
            allow_pattern_evidence,
        )?;
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
    let mut nodes = Vec::new();
    let mut work = Work::new(&mut nodes);
    push!(work, fuel, Node::Expr(expression));
    while let Some(node) = work.pop() {
        if *fuel == 0 {
            return Err("IIR traversal budget exhausted".into());
        }
        *fuel -= 1;
        match node {
            Node::Argument(argument) => {
                use crate::program::NormalFormArgument as A;
                match argument {
                    A::ListSpread(value) | A::Value(value) => push!(work, fuel, Node::Expr(value)),
                    A::Unit { .. } | A::Null { .. } | A::Integer { .. } => {}
                    A::If {
                        condition,
                        then_input,
                        else_input,
                        ..
                    } => {
                        push!(work, fuel, Node::Expr(condition));
                        push!(work, fuel, Node::Argument(then_input));
                        push!(work, fuel, Node::Argument(else_input));
                    }
                    A::Block {
                        statements, result, ..
                    } => {
                        extend!(work, fuel, statements.iter_mut().map(Node::Expr));
                        extend!(work, fuel, result.as_deref_mut().map(Node::Argument));
                    }
                    A::List { items, .. } | A::Tuple { items, .. } => {
                        extend!(work, fuel, items.iter_mut().map(Node::Argument))
                    }
                    A::Match { value, arms, .. } => {
                        work::charge_entries(fuel, arms.len())?;
                        push!(work, fuel, Node::Expr(value));
                        for arm in arms {
                            push!(work, fuel, Node::Pattern(&mut arm.pattern));
                            extend!(work, fuel, arm.guard.iter_mut().map(Node::Expr));
                            push!(work, fuel, Node::Argument(&mut arm.body));
                        }
                    }
                }
            }
            Node::Pattern(pattern) => match pattern {
                P::Then { first, value, next } => {
                    push!(work, fuel, Node::Pattern(first));
                    push!(work, fuel, Node::Expr(value));
                    push!(work, fuel, Node::Pattern(next));
                }
                P::Guard { pattern, condition } => {
                    push!(work, fuel, Node::Pattern(pattern));
                    push!(work, fuel, Node::Expr(condition));
                }
                P::OptionalPresent(pattern)
                | P::ErrorResult { inner: pattern, .. }
                | P::Observe { inner: pattern, .. }
                | P::Remap { pattern, .. }
                | P::Binding { inner: pattern, .. } => push!(work, fuel, Node::Pattern(pattern)),
                P::AnonymousRecord { names, fields } => {
                    validate_record_pattern(names, fields.len(), fuel)?;
                    extend!(work, fuel, fields.iter_mut().map(Node::Pattern));
                }
                P::NominalType { fields, .. }
                | P::Record { fields, .. }
                | P::Variant { fields, .. }
                | P::NormalForm {
                    arguments: fields, ..
                }
                | P::Tuple(fields)
                | P::TypeTuple(fields)
                | P::Or(fields) => extend!(work, fuel, fields.iter_mut().map(Node::Pattern)),
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
                        push!(work, fuel, Node::Expr(computation));
                        if include_handler_arms {
                            work::charge_entries(fuel, handlers.len())?;
                        }
                        // Avoid scanning an excluded handler list through a filter.
                        let handlers = if include_handler_arms {
                            handlers.as_mut_slice()
                        } else {
                            &mut []
                        };
                        for handler in handlers {
                            work::charge_entries(fuel, handler.arms.len())?;
                            for arm in &mut handler.arms {
                                push!(work, fuel, Node::Pattern(&mut arm.pattern));
                                if let Some(guard) = &mut arm.guard {
                                    push!(work, fuel, Node::Expr(guard));
                                }
                                push!(work, fuel, Node::Expr(&mut arm.body));
                            }
                        }
                    }
                    K::EffectResume(body)
                    | K::EffectComputation { body, .. }
                    | K::EffectRun {
                        computation: body, ..
                    } => push!(work, fuel, Node::Expr(body)),
                    K::EffectRequest { arguments, .. } => {
                        extend!(work, fuel, arguments.iter_mut().map(Node::Expr))
                    }
                    K::Closure { captures, body, .. } => {
                        work::charge_entries(fuel, captures.len())?;
                        if include_closure_bodies {
                            push!(work, fuel, Node::Expr(std::sync::Arc::make_mut(body)));
                        }
                        extend!(
                            work,
                            fuel,
                            captures
                                .iter_mut()
                                .filter_map(|c| c.value.as_deref_mut())
                                .map(Node::Expr),
                        );
                    }
                    kind @ (K::NormalFormType { .. } | K::ApplyNormalForm { .. }) => {
                        let (callee, arguments, plan) = match kind {
                            K::NormalFormType {
                                arguments, plan, ..
                            } => (None, arguments, plan),
                            K::ApplyNormalForm {
                                callee,
                                arguments,
                                plan,
                                ..
                            } => (Some(std::sync::Arc::make_mut(callee)), arguments, plan),
                            _ => unreachable!(),
                        };
                        extend!(
                            work,
                            fuel,
                            plan.iter_mut()
                                .flat_map(|p| &mut p.preparations)
                                .map(|p| Node::Expr(&mut p.expression)),
                        );
                        extend!(work, fuel, arguments.iter_mut().map(Node::Argument));
                        if let Some(callee) = callee {
                            push!(work, fuel, Node::Expr(callee));
                        }
                    }
                    K::NominalType {
                        arguments,
                        preparations,
                        ..
                    } => {
                        extend!(
                            work,
                            fuel,
                            preparations
                                .iter_mut()
                                .map(|p| Node::Expr(&mut p.expression)),
                        );
                        extend!(work, fuel, arguments.iter_mut().map(Node::Argument));
                    }
                    K::ClosureType {
                        environment,
                        signature,
                    } => {
                        extend!(work, fuel, environment.iter_mut().map(Node::Expr));
                        push!(work, fuel, Node::Expr(signature));
                    }
                    K::CheckTypeAlias {
                        arguments,
                        body,
                        preparations,
                        ..
                    } => {
                        extend!(
                            work,
                            fuel,
                            preparations
                                .iter_mut()
                                .map(|p| Node::Expr(&mut p.expression)),
                        );
                        extend!(work, fuel, arguments.iter_mut().map(Node::Argument));
                        if let Some(body) = body {
                            push!(work, fuel, Node::Expr(body));
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
                    | K::Intrinsic { inputs: fields, .. }
                    | K::Tuple(fields) => extend!(work, fuel, fields.iter_mut().map(Node::Expr)),
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
                    | K::ListLength(value)
                    | K::NewtypeRepresentation(value)
                    | K::StructFieldTypes(value)
                    | K::BuildStruct(value)
                    | K::BuildNewtype(value)
                    | K::BuildMetadata {
                        selector: value, ..
                    }
                    | K::BuildEnum(value)
                    | K::EnumStructure { subject: value, .. }
                    | K::ObjectQuery { subject: value, .. }
                    | K::ObjectEdit { subject: value, .. }
                    | K::StructFieldNames(value)
                    | K::BitVecType(value)
                    | K::IntegerNegate(value)
                    | K::FloatNegate(value)
                    | K::BitConvert(value)
                    | K::ErrorConvert { value: value, .. }
                    | K::NewtypeConvert(value)
                    | K::Snapshot(value)
                    | K::TakeBits(value)
                    | K::TextQuery { subject: value, .. }
                    | K::IntegerRepresentation(value)
                    | K::ComptimeIntegerConvert(value)
                    | K::SymbolName(value)
                    | K::EnumName(value)
                    | K::TypeKind { operand: value, .. }
                    | K::TypeLayout { operand: value, .. }
                    | K::Not(value)
                    | K::Loop { body: value, .. } => push!(work, fuel, Node::Expr(value)),
                    K::BitWritePointer {
                        pointer,
                        selections,
                        value,
                    } => {
                        *fuel = fuel
                            .checked_sub(selections.len())
                            .ok_or("bit selection traversal budget exhausted")?;
                        push!(work, fuel, Node::Expr(pointer));
                        extend!(
                            work,
                            fuel,
                            selections
                                .iter_mut()
                                .flat_map(crate::program::BitSelection::expressions_mut)
                                .map(Node::Expr),
                        );
                        push!(work, fuel, Node::Expr(value));
                    }
                    K::BitUpdate {
                        selections, value, ..
                    } => {
                        *fuel = fuel
                            .checked_sub(selections.len())
                            .ok_or("bit selection traversal budget exhausted")?;
                        extend!(
                            work,
                            fuel,
                            selections
                                .iter_mut()
                                .flat_map(crate::program::BitSelection::expressions_mut)
                                .map(Node::Expr),
                        );
                        push!(work, fuel, Node::Expr(value));
                    }
                    K::BitWindow { value, start, end } => {
                        push!(work, fuel, Node::Expr(value));
                        push!(work, fuel, Node::Expr(start));
                        push!(work, fuel, Node::Expr(end));
                    }
                    K::Match { value, arms } => {
                        work::charge_entries(fuel, arms.len())?;
                        push!(work, fuel, Node::Expr(value));
                        for arm in arms {
                            push!(work, fuel, Node::Pattern(&mut arm.pattern));
                            extend!(work, fuel, arm.guard.iter_mut().map(Node::Expr));
                            push!(work, fuel, Node::Expr(&mut arm.body));
                        }
                    }
                    K::ComptimeFunctionInstance { inputs, .. } => {
                        extend!(work, fuel, inputs.expressions_mut().map(Node::Expr));
                    }
                    K::Block { statements, result } | K::CallPreparation { statements, result } => {
                        extend!(work, fuel, statements.iter_mut().map(Node::Expr));
                        extend!(work, fuel, result.as_deref_mut().map(Node::Expr));
                    }
                    K::LetPattern {
                        pattern,
                        value,
                        guard,
                        otherwise,
                    } => {
                        push!(work, fuel, Node::Pattern(pattern));
                        push!(work, fuel, Node::Expr(value));
                        extend!(work, fuel, guard.as_deref_mut().map(Node::Expr));
                        extend!(work, fuel, otherwise.as_deref_mut().map(Node::Expr));
                    }
                    K::If {
                        condition,
                        then_branch,
                        else_branch,
                    } => {
                        push!(work, fuel, Node::Expr(condition));
                        push!(work, fuel, Node::Expr(then_branch));
                        extend!(work, fuel, else_branch.as_deref_mut().map(Node::Expr));
                    }
                    K::Return(value) => extend!(work, fuel, value.as_deref_mut().map(Node::Expr)),
                    K::Call {
                        callee, arguments, ..
                    } => {
                        work::charge_entries(fuel, arguments.len())?;
                        push!(work, fuel, Node::Expr(callee));
                        extend!(
                            work,
                            fuel,
                            arguments.iter_mut().map(|a| Node::Expr(&mut a.value))
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
                    | K::ListConcat(a, b)
                    | K::TypeListIndex(a, b)
                    | K::BitEqual(a, b)
                    | K::BitConcat(a, b)
                    | K::BitIndex(a, b)
                    | K::And(a, b)
                    | K::Or(a, b) => {
                        push!(work, fuel, Node::Expr(a));
                        push!(work, fuel, Node::Expr(b));
                    }
                    K::AssociatedValue { .. }
                    | K::ComptimeValue(_)
                    | K::StagedValue(_)
                    | K::TypeValue(_)
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

fn validate_record_pattern(names: &[String], fields: usize, fuel: &mut usize) -> Result<(), Error> {
    if names.len() != fields {
        return Err(Error::Invalid("anonymous record pattern arity mismatch"));
    }
    let mut seen = std::collections::BTreeSet::new();
    for name in names {
        *fuel = fuel
            .checked_sub(name.len().saturating_add(1))
            .ok_or(Error::Budget("anonymous record pattern budget exhausted"))?;
        if !seen.insert(name) {
            return Err(Error::Invalid("duplicate anonymous record pattern label"));
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
