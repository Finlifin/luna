//! Shared preflight for materialized constants and contextual input templates.
use crate::{Term, program as n, program::ExprKind as K};

// Quote expansion retains preparation, parameter and lexical cleanup scopes.
// Its supported 32-frame chain can exceed 128 expression nodes in depth.
// Keep a finite pre-clone bound without conflating this with type term depth.
pub const MAX_COPY_DEPTH: usize = 160;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyBudgetExhausted {
    Steps,
    Depth,
}

pub fn charge_expression(value: &n::Expr, steps: &mut u64) -> Result<(), CopyBudgetExhausted> {
    charge_tree(value, steps, false)
}

pub fn charge_pattern_rewrite(
    value: &n::Pattern,
    steps: &mut u64,
) -> Result<(), CopyBudgetExhausted> {
    charge_node(Node::Pattern(value), steps, true, None)
}

/// Type rewriting enters shared closure bodies and may copy them through Arc::make_mut.
pub fn charge_type_rewrite(value: &n::Expr, steps: &mut u64) -> Result<(), CopyBudgetExhausted> {
    charge_tree(value, steps, true)
}

fn charge_tree(value: &n::Expr, steps: &mut u64, rewrite: bool) -> Result<(), CopyBudgetExhausted> {
    charge_node(Node::Expr(value), steps, rewrite, None)
}

/// Charge actual declaration occurrences in the expanded copy, including shared
/// closure bodies. Inputs are inspected once; unused inputs are not multiplied
/// by expression size. Scope and sort admission remain substitution's duty.
pub fn charge_substitution(
    value: &n::Expr,
    parameters: &[crate::DeclarationParameter],
    arguments: &[Term],
    fuel: &mut usize,
) -> Result<(), CopyBudgetExhausted> {
    charge_node_substitution(Node::Expr(value), parameters, arguments, fuel)
}

/// Preflight every copied function payload, including type-only contracts and
/// signature metadata. Empty substitution inputs also cover staged templates.
pub fn charge_function_substitution(
    value: &n::Function,
    parameters: &[crate::DeclarationParameter],
    arguments: &[Term],
    fuel: &mut usize,
) -> Result<(), CopyBudgetExhausted> {
    charge_node_substitution(Node::Function(value), parameters, arguments, fuel)
}

pub fn charge_predicate_substitution(
    value: &n::TypePredicate,
    parameters: &[crate::DeclarationParameter],
    arguments: &[Term],
    fuel: &mut usize,
) -> Result<(), CopyBudgetExhausted> {
    charge_node_substitution(Node::Predicate(value), parameters, arguments, fuel)
}

pub fn charge_type_substitution(
    value: &Term,
    parameters: &[crate::DeclarationParameter],
    arguments: &[Term],
    fuel: &mut usize,
) -> Result<(), CopyBudgetExhausted> {
    charge_node_substitution(Node::Type(value), parameters, arguments, fuel)
}

fn charge_node_substitution(
    root: Node<'_>,
    parameters: &[crate::DeclarationParameter],
    arguments: &[Term],
    fuel: &mut usize,
) -> Result<(), CopyBudgetExhausted> {
    if parameters.len() != arguments.len() {
        return Err(CopyBudgetExhausted::Steps);
    }
    *fuel = fuel
        .checked_sub(parameters.len())
        .ok_or(CopyBudgetExhausted::Steps)?;
    for argument in arguments {
        crate::type_computations::charge_term(argument, fuel)
            .map_err(|_| CopyBudgetExhausted::Steps)?;
    }
    let replacements = parameters
        .iter()
        .zip(arguments)
        .map(|(parameter, argument)| (parameter.declaration, argument))
        .collect();
    let mut steps = *fuel as u64;
    let result = charge_node(root, &mut steps, true, Some(&replacements));
    *fuel = steps as usize;
    result
}

pub fn charge_argument_type_rewrite(
    value: &n::NormalFormArgument,
    steps: &mut u64,
) -> Result<(), CopyBudgetExhausted> {
    charge_node(Node::Argument(value), steps, true, None)
}
enum Node<'a> {
    Function(&'a n::Function),
    Predicate(&'a n::TypePredicate),
    Requirement(&'a n::TraitRequirement),
    Expr(&'a n::Expr),
    Type(&'a Term),
    Argument(&'a n::NormalFormArgument),
    Pattern(&'a n::Pattern),
    Arm(&'a n::MatchArm),
    Operation(&'a crate::effects::Operation),
}

fn charge_node(
    root: Node<'_>,
    steps: &mut u64,
    rewrite: bool,
    replacements: Option<&std::collections::BTreeMap<crate::DeclarationId, &Term>>,
) -> Result<(), CopyBudgetExhausted> {
    fn charge_width(steps: &mut u64, width: usize) -> Result<(), CopyBudgetExhausted> {
        *steps = steps
            .checked_sub(width as u64)
            .ok_or_else(|| CopyBudgetExhausted::Steps)?;
        Ok(())
    }
    fn literal_size(literal: &n::Literal) -> usize {
        match literal {
            n::Literal::PendingFloat { spelling, .. }
            | n::Literal::PendingInteger { spelling, .. } => spelling.len(),
            n::Literal::Symbol(s)
            | n::Literal::String(s)
            | n::Literal::Integer(s)
            | n::Literal::Float(s) => s.len(),
            n::Literal::BitVec { words, .. } => words.len(),
            _ => 0,
        }
    }
    let mut work = vec![(root, 0usize)];
    while let Some((node, depth)) = work.pop() {
        if depth >= MAX_COPY_DEPTH {
            return Err(CopyBudgetExhausted::Depth);
        }
        if *steps == 0 {
            return Err(CopyBudgetExhausted::Steps);
        }
        if let Node::Type(ty) = node {
            let mut fuel = usize::try_from(*steps).unwrap_or(usize::MAX);
            let before = fuel;
            let result = if let Some(replacements) = replacements {
                crate::type_computations::charge_expanded_term(
                    ty,
                    &mut fuel,
                    (MAX_COPY_DEPTH - depth).min(128),
                    replacements,
                )
            } else {
                crate::type_computations::charge_term_with_depth(
                    ty,
                    &mut fuel,
                    (MAX_COPY_DEPTH - depth).min(128),
                )
            };
            *steps -= (before - fuel) as u64;
            result.map_err(|_| CopyBudgetExhausted::Steps)?;
            continue;
        }
        *steps -= 1;
        let size = match node {
            Node::Function(function) => {
                for width in [
                    function.name.len(),
                    function.parameters.len(),
                    function.type_parameters.len(),
                    function.entry_snapshots.len(),
                    function.preconditions.len(),
                    function.postconditions.len(),
                    function.defaults.len(),
                    function.requirements.len(),
                    function.capabilities.len(),
                ] {
                    charge_width(steps, width)?;
                }
                work.push((Node::Type(&function.signature), depth));
                work.push((Node::Expr(&function.body), depth));
                work.extend(
                    function
                        .preconditions
                        .iter()
                        .chain(&function.postconditions)
                        .chain(function.defaults.iter().map(|(_, expr)| expr))
                        .map(|expr| (Node::Expr(expr), depth)),
                );
                work.extend(
                    function
                        .requirements
                        .iter()
                        .map(|p| (Node::Predicate(p), depth)),
                );
                work.extend(
                    function
                        .capabilities
                        .iter()
                        .map(|r| (Node::Requirement(r), depth)),
                );
                0
            }
            Node::Predicate(predicate) => {
                use n::TypePredicate as P;
                match predicate {
                    P::Bool(_) => {}
                    P::Equal(a, b) => {
                        work.push((Node::Type(a), depth + 1));
                        work.push((Node::Type(b), depth + 1));
                    }
                    P::Trait(requirement) => work.push((Node::Requirement(requirement), depth + 1)),
                    P::Not(inner) => work.push((Node::Predicate(inner), depth + 1)),
                    P::And(items) | P::Or(items) => {
                        charge_width(steps, items.len())?;
                        work.extend(items.iter().map(|p| (Node::Predicate(p), depth + 1)));
                    }
                }
                0
            }
            Node::Requirement(requirement) => {
                charge_width(steps, requirement.interface_arguments.len())?;
                work.push((Node::Type(&requirement.subject), depth + 1));
                work.extend(
                    requirement
                        .interface_arguments
                        .iter()
                        .map(|ty| (Node::Type(ty), depth + 1)),
                );
                0
            }
            Node::Expr(expr) => {
                work.push((Node::Type(&expr.ty), depth + 1));
                match &expr.kind {
                    K::NominalType {
                        preparations,
                        arguments,
                        order,
                        bindings,
                        ..
                    } => {
                        charge_width(steps, preparations.len())?;
                        work.extend(
                            preparations
                                .iter()
                                .map(|p| (Node::Expr(&p.expression), depth + 1)),
                        );
                        charge_width(
                            steps,
                            arguments
                                .len()
                                .saturating_add(order.as_ref().map_or(0, Vec::len))
                                .saturating_add(bindings.as_ref().map_or(0, Vec::len)),
                        )?;
                        work.extend(
                            arguments
                                .iter()
                                .map(|argument| (Node::Argument(argument), depth + 1)),
                        );
                        0
                    }
                    K::NormalFormType {
                        arguments, plan, ..
                    } => {
                        charge_width(steps, arguments.len())?;
                        work.extend(
                            arguments
                                .iter()
                                .map(|argument| (Node::Argument(argument), depth + 1)),
                        );
                        if let Some(plan) = plan {
                            charge_width(
                                steps,
                                plan.order
                                    .len()
                                    .saturating_add(plan.bindings.len())
                                    .saturating_add(plan.preparations.len()),
                            )?;
                            work.extend(
                                plan.preparations
                                    .iter()
                                    .map(|p| (Node::Expr(&p.expression), depth + 1)),
                            );
                        }
                        0
                    }
                    K::Call {
                        callee,
                        arguments,
                        parameter_bindings,
                    } => {
                        charge_width(steps, arguments.len())?;
                        work.push((Node::Expr(callee), depth + 1));
                        work.extend(
                            arguments
                                .iter()
                                .map(|argument| (Node::Expr(&argument.value), depth + 1)),
                        );
                        parameter_bindings.len()
                    }
                    K::Block { statements, result } | K::CallPreparation { statements, result } => {
                        charge_width(steps, statements.len())?;
                        work.extend(statements.iter().map(|expr| (Node::Expr(expr), depth + 1)));
                        if let Some(result) = result {
                            work.push((Node::Expr(result), depth + 1));
                        }
                        0
                    }
                    K::Let { value, .. } => {
                        work.push((Node::Expr(value), depth + 1));
                        0
                    }
                    K::Closure {
                        parameters,
                        captures,
                        body,
                    } => {
                        if rewrite {
                            work.push((Node::Expr(body), depth + 1));
                        }
                        charge_width(steps, captures.len().saturating_mul(2))?;
                        for capture in captures {
                            work.push((Node::Type(&capture.ty), depth + 1));
                            if let Some(value) = &capture.value {
                                work.push((Node::Expr(value), depth + 1));
                            }
                        }
                        // Expr::clone shares the immutable Arc body. Captured
                        // initializers and parameter bindings are actual copies.
                        parameters.len()
                    }
                    K::FunctionInstance { arguments, .. } => {
                        charge_width(steps, arguments.len())?;
                        work.extend(arguments.iter().map(|ty| (Node::Type(ty), depth + 1)));
                        0
                    }
                    K::TraitMethod {
                        subject,
                        interface_arguments,
                        method_arguments,
                        ..
                    } => {
                        charge_width(
                            steps,
                            interface_arguments
                                .len()
                                .saturating_add(method_arguments.len()),
                        )?;
                        work.push((Node::Type(subject), depth + 1));
                        work.extend(
                            interface_arguments
                                .iter()
                                .chain(method_arguments)
                                .map(|ty| (Node::Type(ty), depth + 1)),
                        );
                        0
                    }
                    K::TypeValue(ty) | K::ComptimeValue(ty) => {
                        work.push((Node::Type(ty), depth + 1));
                        0
                    }
                    K::Object { names, fields } => {
                        charge_width(steps, names.len().saturating_add(fields.len()))?;
                        for name in names.iter().flatten() {
                            charge_width(steps, name.len())?;
                        }
                        work.extend(fields.iter().map(|field| (Node::Expr(field), depth + 1)));
                        0
                    }
                    K::TypeListValue(types) => {
                        charge_width(steps, types.len())?;
                        work.extend(types.iter().map(|ty| (Node::Type(ty), depth + 1)));
                        0
                    }
                    K::CheckTypeAlias {
                        arguments,
                        body,
                        preparations,
                        bindings,
                        order,
                        ..
                    } => {
                        charge_width(
                            steps,
                            arguments
                                .len()
                                .saturating_add(preparations.len())
                                .saturating_add(bindings.as_ref().map_or(0, Vec::len))
                                .saturating_add(order.as_ref().map_or(0, Vec::len)),
                        )?;
                        work.extend(arguments.iter().map(|e| (Node::Argument(e), depth + 1)));
                        work.extend(
                            preparations
                                .iter()
                                .map(|p| (Node::Expr(&p.expression), depth + 1)),
                        );
                        if let Some(body) = body {
                            work.push((Node::Expr(body), depth + 1));
                        }
                        0
                    }
                    K::FunctionType {
                        parameters,
                        comptime_parameters,
                        ..
                    } => {
                        charge_width(steps, parameters.len())?;
                        charge_width(steps, comptime_parameters.len())?;
                        work.extend(parameters.iter().map(|e| (Node::Expr(e), depth + 1)));
                        0
                    }
                    K::TypeList(items)
                    | K::TupleType(items)
                    | K::Intrinsic { inputs: items, .. }
                    | K::Tuple(items)
                    | K::AnonymousRecord { fields: items }
                    | K::Record { fields: items, .. }
                    | K::Variant { fields: items, .. } => {
                        charge_width(steps, items.len())?;
                        work.extend(items.iter().map(|e| (Node::Expr(e), depth + 1)));
                        0
                    }
                    K::Literal(value) => literal_size(value),
                    K::OptionalAbsent
                    | K::Unit
                    | K::Local(_)
                    | K::Function(_)
                    | K::Break(_)
                    | K::Continue(_) => 0,
                    K::If {
                        condition,
                        then_branch,
                        else_branch,
                    } => {
                        work.push((Node::Expr(condition), depth + 1));
                        work.push((Node::Expr(then_branch), depth + 1));
                        if let Some(branch) = else_branch {
                            work.push((Node::Expr(branch), depth + 1));
                        }
                        0
                    }
                    K::Match { value, arms } => {
                        charge_width(steps, arms.len())?;
                        work.push((Node::Expr(value), depth + 1));
                        work.extend(arms.iter().map(|arm| (Node::Arm(arm), depth + 1)));
                        0
                    }
                    K::LetPattern {
                        pattern,
                        value,
                        guard,
                        otherwise,
                    } => {
                        work.push((Node::Pattern(pattern), depth + 1));
                        work.push((Node::Expr(value), depth + 1));
                        work.extend(
                            guard
                                .iter()
                                .chain(otherwise)
                                .map(|value| (Node::Expr(value), depth + 1)),
                        );
                        0
                    }
                    K::Return(value) => {
                        if let Some(value) = value {
                            work.push((Node::Expr(value), depth + 1));
                        }
                        0
                    }
                    K::EffectResume(value)
                    | K::OptionalPresent(value)
                    | K::OptionalType(value)
                    | K::PointerType(value)
                    | K::AddressOf(value)
                    | K::ReadPointer(value)
                    | K::ListLength(value)
                    | K::BitVecType(value)
                    | K::FloatNegate(value)
                    | K::IntegerNegate(value)
                    | K::RequirePredicate(value)
                    | K::EnsurePredicate(value)
                    | K::CheckPredicate(value)
                    | K::UnsafeScope(value)
                    | K::Discard(value)
                    | K::BitConvert(value)
                    | K::ErrorConvert { value: value, .. }
                    | K::NewtypeConvert(value)
                    | K::Snapshot(value)
                    | K::TakeBits(value)
                    | K::SymbolName(value)
                    | K::EnumName(value)
                    | K::Not(value)
                    | K::Loop { body: value, .. }
                    | K::Field { value, .. }
                    | K::BitSlice { value, .. }
                    | K::PointerField { pointer: value, .. } => {
                        work.push((Node::Expr(value), depth + 1));
                        0
                    }
                    K::ListConcat(left, right)
                    | K::TypeListIndex(left, right)
                    | K::TypeEqual(left, right)
                    | K::TemplateEqual(left, right)
                    | K::ScalarEqual(left, right)
                    | K::ScalarLessEqual(left, right)
                    | K::ScalarLess(left, right)
                    | K::ScalarGreater(left, right)
                    | K::ScalarGreaterEqual(left, right)
                    | K::BitIndex(left, right)
                    | K::BitEqual(left, right)
                    | K::BitConcat(left, right)
                    | K::And(left, right)
                    | K::Or(left, right)
                    | K::IntegerBinary { left, right, .. }
                    | K::FloatBinary { left, right, .. }
                    | K::FunctionResultType {
                        signature: left,
                        result: right,
                    }
                    | K::WritePointer {
                        pointer: left,
                        value: right,
                    } => {
                        work.push((Node::Expr(left), depth + 1));
                        work.push((Node::Expr(right), depth + 1));
                        0
                    }
                    K::AnonymousField { value, name, .. }
                    | K::PointerAnonymousField {
                        pointer: value,
                        name,
                        ..
                    } => {
                        work.push((Node::Expr(value), depth + 1));
                        name.len()
                    }
                    K::BitWindow { value, start, end } => {
                        for value in [value, start, end] {
                            work.push((Node::Expr(value), depth + 1));
                        }
                        0
                    }
                    K::Assign { place, value } => {
                        work.push((Node::Expr(value), depth + 1));
                        place.fields.len()
                    }
                    K::BitAssign {
                        place,
                        storage_type,
                        value,
                        ..
                    } => {
                        work.push((Node::Expr(value), depth + 1));
                        work.push((Node::Type(storage_type), depth + 1));
                        place.fields.len()
                    }
                    K::BitUpdate {
                        place,
                        storage_type,
                        selections,
                        value,
                    } => {
                        charge_width(steps, selections.len().saturating_mul(2))?;
                        work.push((Node::Expr(value), depth + 1));
                        work.push((Node::Type(storage_type), depth + 1));
                        work.extend(
                            selections
                                .iter()
                                .flat_map(n::BitSelection::expressions)
                                .map(|value| (Node::Expr(value), depth + 1)),
                        );
                        place.fields.len()
                    }
                    K::BitWritePointer {
                        pointer,
                        selections,
                        value,
                    } => {
                        charge_width(steps, selections.len().saturating_mul(2))?;
                        work.push((Node::Expr(pointer), depth + 1));
                        work.push((Node::Expr(value), depth + 1));
                        work.extend(
                            selections
                                .iter()
                                .flat_map(n::BitSelection::expressions)
                                .map(|value| (Node::Expr(value), depth + 1)),
                        );
                        0
                    }
                    K::AssociatedValue {
                        interface, subject, ..
                    } => {
                        charge_width(steps, interface.arguments.len())?;
                        work.push((Node::Type(subject), depth + 1));
                        work.extend(
                            interface
                                .arguments
                                .iter()
                                .map(|ty| (Node::Type(ty), depth + 1)),
                        );
                        0
                    }
                    K::ClosureType {
                        environment,
                        signature,
                    } => {
                        charge_width(steps, environment.len())?;
                        work.push((Node::Expr(signature), depth + 1));
                        work.extend(
                            environment
                                .iter()
                                .map(|value| (Node::Expr(value), depth + 1)),
                        );
                        0
                    }
                    K::EffectRun { computation, slots } => {
                        work.push((Node::Expr(computation), depth + 1));
                        slots.len()
                    }
                    K::EffectComputation { effects, body } => {
                        charge_width(steps, effects.len())?;
                        work.push((Node::Expr(body), depth + 1));
                        work.extend(effects.iter().map(|op| (Node::Operation(op), depth + 1)));
                        0
                    }
                    K::EffectRequest {
                        operation,
                        signature,
                        arguments,
                        ..
                    } => {
                        charge_width(steps, arguments.len())?;
                        work.push((Node::Operation(operation), depth + 1));
                        work.push((Node::Type(signature), depth + 1));
                        work.extend(arguments.iter().map(|value| (Node::Expr(value), depth + 1)));
                        0
                    }
                    K::EffectHandle {
                        computation,
                        handlers,
                        outer,
                    } => {
                        charge_width(steps, handlers.len().saturating_add(outer.len()))?;
                        work.push((Node::Expr(computation), depth + 1));
                        work.extend(outer.iter().map(|op| (Node::Operation(op), depth + 1)));
                        for handler in handlers {
                            charge_width(steps, handler.arms.len())?;
                            work.push((Node::Operation(&handler.operation), depth + 1));
                            work.push((Node::Type(&handler.signature), depth + 1));
                            work.extend(handler.arms.iter().map(|arm| (Node::Arm(arm), depth + 1)));
                        }
                        0
                    }
                }
            }
            Node::Argument(argument) => {
                use n::NormalFormArgument as A;
                match argument {
                    A::Value(value) | A::TypeListSpread(value) => {
                        work.push((Node::Expr(value), depth + 1));
                        0
                    }
                    A::Integer { spelling, .. } => spelling.len(),
                    A::Unit { .. } => 0,
                    A::Tuple { items, .. } | A::TypeList { items, .. } => {
                        charge_width(steps, items.len())?;
                        work.extend(items.iter().map(|value| (Node::Argument(value), depth + 1)));
                        0
                    }
                    A::If {
                        condition,
                        then_input,
                        else_input,
                        ..
                    } => {
                        work.push((Node::Expr(condition), depth + 1));
                        work.push((Node::Argument(then_input), depth + 1));
                        work.push((Node::Argument(else_input), depth + 1));
                        0
                    }
                    A::Block {
                        statements, result, ..
                    } => {
                        charge_width(steps, statements.len())?;
                        work.extend(
                            statements
                                .iter()
                                .map(|value| (Node::Expr(value), depth + 1)),
                        );
                        if let Some(result) = result {
                            work.push((Node::Argument(result), depth + 1));
                        }
                        0
                    }
                    A::Match { value, arms, .. } => {
                        charge_width(steps, arms.len().saturating_mul(3))?;
                        work.push((Node::Expr(value), depth + 1));
                        for arm in arms {
                            work.push((Node::Pattern(&arm.pattern), depth + 1));
                            if let Some(guard) = &arm.guard {
                                work.push((Node::Expr(guard), depth + 1));
                            }
                            work.push((Node::Argument(&arm.body), depth + 1));
                        }
                        0
                    }
                }
            }
            Node::Pattern(pattern) => {
                use n::Pattern as P;
                match pattern {
                    P::ErrorResult { label, inner } => {
                        if let Some(label) = label {
                            work.push((Node::Type(label), depth + 1));
                        }
                        work.push((Node::Pattern(inner), depth + 1));
                        0
                    }
                    P::OptionalPresent(inner)
                    | P::Observe { inner, .. }
                    | P::Binding { inner, .. } => {
                        work.push((Node::Pattern(inner), depth + 1));
                        0
                    }
                    P::Then { first, value, next } => {
                        work.push((Node::Pattern(first), depth + 1));
                        work.push((Node::Expr(value), depth + 1));
                        work.push((Node::Pattern(next), depth + 1));
                        0
                    }
                    P::Guard { pattern, condition } => {
                        work.push((Node::Pattern(pattern), depth + 1));
                        work.push((Node::Expr(condition), depth + 1));
                        0
                    }
                    P::Remap { pattern, bindings } => {
                        work.push((Node::Pattern(pattern), depth + 1));
                        bindings.len()
                    }
                    P::AnonymousRecord { names, fields } => {
                        charge_width(steps, fields.len().saturating_add(names.len()))?;
                        work.extend(fields.iter().map(|value| (Node::Pattern(value), depth + 1)));
                        names
                            .iter()
                            .fold(0usize, |size, name| size.saturating_add(name.len()))
                    }
                    P::NominalType { fields, .. }
                    | P::Record { fields, .. }
                    | P::Variant { fields, .. }
                    | P::Tuple(fields)
                    | P::TypeTuple(fields)
                    | P::Or(fields)
                    | P::NormalForm {
                        arguments: fields, ..
                    } => {
                        charge_width(steps, fields.len())?;
                        work.extend(fields.iter().map(|value| (Node::Pattern(value), depth + 1)));
                        0
                    }
                    P::Literal(value) => literal_size(value),
                    P::CodeEvidence { outputs } => {
                        charge_width(steps, outputs.len())?;
                        work.extend(outputs.iter().map(|(_, ty)| (Node::Type(ty), depth + 1)));
                        0
                    }
                    P::OptionalAbsent | P::Wild | P::Unit => 0,
                }
            }
            Node::Type(_) => unreachable!(),
            Node::Arm(arm) => {
                work.push((Node::Pattern(&arm.pattern), depth + 1));
                if let Some(guard) = &arm.guard {
                    work.push((Node::Expr(guard), depth + 1));
                }
                work.push((Node::Expr(&arm.body), depth + 1));
                0
            }
            Node::Operation(operation) => {
                charge_width(steps, operation.arguments.len())?;
                work.extend(
                    operation
                        .arguments
                        .iter()
                        .map(|ty| (Node::Type(ty), depth + 1)),
                );
                0
            }
        };
        *steps = steps
            .checked_sub(size as u64)
            .ok_or_else(|| CopyBudgetExhausted::Steps)?;
        if work.len() as u64 > *steps {
            return Err(CopyBudgetExhausted::Steps);
        }
    }
    Ok(())
}
