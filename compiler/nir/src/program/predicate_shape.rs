//! Shape admission for program predicates, independent of truth and PTC.
use super::{Expr, ExprKind, Pattern, Term, intrinsic_pointer_value, pointer_operation_error};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PredicateShapeError {
    Budget,
}
impl std::fmt::Display for PredicateShapeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("program predicate inspection budget exhausted")
    }
}
impl std::error::Error for PredicateShapeError {}
impl From<PredicateShapeError> for String {
    fn from(error: PredicateShapeError) -> Self {
        error.to_string()
    }
}
fn charge(fuel: &mut usize, amount: usize) -> Result<(), PredicateShapeError> {
    *fuel = fuel
        .checked_sub(amount)
        .ok_or(PredicateShapeError::Budget)?;
    Ok(())
}
fn charge_type(ty: &Term, fuel: &mut usize) -> Result<(), PredicateShapeError> {
    crate::type_computations::charge_term(ty, fuel).map_err(|_| PredicateShapeError::Budget)
}
fn queue<'a, T: 'a, I: IntoIterator<Item = &'a T>>(
    work: &mut Vec<&'a T>,
    values: I,
    fuel: &mut usize,
) -> Result<(), PredicateShapeError>
where
    I::IntoIter: ExactSizeIterator,
{
    let values = values.into_iter();
    charge(fuel, values.len())?;
    work.extend(values);
    Ok(())
}

/// The current closed, read-only decision fragment for runtime checks. This
/// verifies executable shape, not truth or pointer validity; resource checking
/// establishes access permission separately. Calls need a predicate/effect contract
/// before admission; type propositions use a separate interface.
pub fn checkable_predicate(
    expression: &Expr,
    fuel: &mut usize,
) -> Result<bool, PredicateShapeError> {
    predicate_shape(expression, false, fuel)
}

/// Candidate shape only: calls still require separate stability and finite
/// completion evidence. Never use this as admission for a retained IR check.
pub fn predicate_call_candidate(
    expression: &Expr,
    fuel: &mut usize,
) -> Result<bool, PredicateShapeError> {
    predicate_shape(expression, true, fuel)
}

fn predicate_shape(
    expression: &Expr,
    calls: bool,
    remaining: &mut usize,
) -> Result<bool, PredicateShapeError> {
    charge_type(&expression.ty, remaining)?;
    if expression.ty != Term::Primitive(crate::Primitive::Bool) {
        return Ok(false);
    }
    let mut pending = vec![expression];
    while let Some(expr) = pending.pop() {
        charge(remaining, 1)?;
        charge_type(&expr.ty, remaining)?;
        if let ExprKind::UnsafeScope(body) = &expr.kind {
            charge_type(&body.ty, remaining)?;
        }
        if matches!(&expr.kind, ExprKind::UnsafeScope(body) if body.ty != expr.ty) {
            return Ok(false);
        }
        match &expr.kind {
            ExprKind::Call {
                callee, arguments, ..
            } if calls => {
                pending.push(callee);
                queue(
                    &mut pending,
                    arguments.iter().map(|argument| &argument.value),
                    remaining,
                )?;
            }
            ExprKind::Function(_)
            | ExprKind::FunctionInstance { .. }
            | ExprKind::TraitMethod { .. }
                if calls => {}
            ExprKind::BitWindow { value, start, end } => queue(
                &mut pending,
                [value.as_ref(), start.as_ref(), end.as_ref()],
                remaining,
            )?,
            ExprKind::Literal(_)
            | ExprKind::Local(_)
            | ExprKind::OptionalAbsent
            | ExprKind::Unit => {}
            ExprKind::Intrinsic {
                tag,
                inputs,
                binding,
            } => {
                // Target evidence establishes a scalar operation with no memory
                // or owning-result effects. It never grants permission for its
                // operands: continue visiting every input, including unsafe scopes.
                // Pending evidence only admits template shape; concrete execution
                // still calls intrinsics::checked after specialization.
                charge(remaining, inputs.len())?;
                for input in inputs {
                    charge_type(&input.ty, remaining)?;
                }
                if !binding.as_ref().is_some_and(|binding| {
                    binding.validate_template(*tag, inputs, &expr.ty).is_ok()
                }) {
                    return Ok(false);
                }
                queue(&mut pending, inputs, remaining)?;
            }
            // A checked associated value is initialized at comptime. Reading
            // it is a predicate operand; invoking a callable stored in it is
            // still subject to the separate call admission rules below.
            ExprKind::AssociatedValue { .. } => {}
            ExprKind::ComptimeValue(value) => {
                match crate::comptime_values::validate(value, &expr.ty, remaining) {
                    Ok(()) => (),
                    Err(crate::comptime_values::Error::Budget) => {
                        return Err(PredicateShapeError::Budget);
                    }
                    Err(crate::comptime_values::Error::Invalid(_)) => return Ok(false),
                }
            }
            ExprKind::ReadPointer(value) => {
                charge_type(&value.ty, remaining)?;
                if !intrinsic_pointer_value(&expr.ty) || pointer_operation_error(expr).is_some() {
                    return Ok(false);
                }
                pending.push(value);
            }
            ExprKind::AddressOf(value)
            | ExprKind::PointerField { pointer: value, .. }
            | ExprKind::PointerAnonymousField { pointer: value, .. } => {
                charge_type(&value.ty, remaining)?;
                if pointer_operation_error(expr).is_some() {
                    return Ok(false);
                }
                pending.push(value);
            }
            ExprKind::Not(value)
            | ExprKind::IntegerNegate(value)
            | ExprKind::FloatNegate(value)
            | ExprKind::BitConvert(value)
            | ExprKind::OptionalPresent(value)
            | ExprKind::ErrorConvert { value: value, .. }
            | ExprKind::NewtypeConvert(value)
            | ExprKind::Snapshot(value)
            | ExprKind::TakeBits(value)
            | ExprKind::TextQuery { subject: value, .. }
            | ExprKind::IntegerRepresentation(value)
            | ExprKind::ComptimeIntegerConvert(value)
            | ExprKind::SymbolName(value)
            | ExprKind::EnumName(value)
            | ExprKind::UnsafeScope(value)
            | ExprKind::Discard(value)
            | ExprKind::AnonymousField { value, .. }
            | ExprKind::Field { value, .. }
            | ExprKind::BitSlice { value, .. }
            | ExprKind::Let { value, .. } => pending.push(value),
            ExprKind::ScalarEqual(a, b)
            | ExprKind::ScalarLess(a, b)
            | ExprKind::ScalarLessEqual(a, b)
            | ExprKind::ScalarGreater(a, b)
            | ExprKind::ScalarGreaterEqual(a, b)
            | ExprKind::BitEqual(a, b)
            | ExprKind::BitConcat(a, b)
            | ExprKind::BitIndex(a, b)
            | ExprKind::And(a, b)
            | ExprKind::Or(a, b)
            | ExprKind::IntegerBinary {
                left: a, right: b, ..
            }
            | ExprKind::FloatBinary {
                left: a, right: b, ..
            } => queue(&mut pending, [a.as_ref(), b.as_ref()], remaining)?,
            // Staging reconstructs immutable aggregate inputs inside each
            // contract scope. These nodes only assemble data; calls, writes,
            // and cleanup remain rejected when visiting their children.
            ExprKind::Tuple(fields)
            | ExprKind::Record { fields, .. }
            | ExprKind::Object { fields, .. }
            | ExprKind::AnonymousRecord { fields }
            | ExprKind::Variant { fields, .. } => queue(&mut pending, fields, remaining)?,
            ExprKind::Match { value, arms } => {
                charge(remaining, arms.len())?;
                pending.push(value);
                for arm in arms {
                    if !predicate_pattern(&arm.pattern, &mut pending, remaining)? {
                        return Ok(false);
                    }
                    queue(&mut pending, arm.guard.as_ref(), remaining)?;
                    pending.push(&arm.body);
                }
            }
            ExprKind::LetPattern {
                pattern,
                value,
                guard: None,
                otherwise: None,
            } => {
                if !predicate_pattern(pattern, &mut pending, remaining)? {
                    return Ok(false);
                }
                pending.push(value);
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                queue(
                    &mut pending,
                    [condition.as_ref(), then_branch.as_ref()],
                    remaining,
                )?;
                queue(&mut pending, else_branch.as_deref(), remaining)?;
            }
            ExprKind::Block { statements, result }
            | ExprKind::CallPreparation { statements, result } => {
                queue(&mut pending, statements, remaining)?;
                queue(&mut pending, result.as_deref(), remaining)?;
            }
            _ => return Ok(false),
        }
    }
    Ok(true)
}

// Source elaboration proves let-pattern irrefutability; this traversal checks
// every embedded observation using the same expression budget as match.
fn predicate_pattern<'a>(
    pattern: &'a Pattern,
    pending: &mut Vec<&'a Expr>,
    remaining: &mut usize,
) -> Result<bool, PredicateShapeError> {
    let mut patterns = vec![pattern];
    while let Some(pattern) = patterns.pop() {
        charge(remaining, 1)?;
        match pattern {
            Pattern::OptionalAbsent | Pattern::Wild | Pattern::Unit | Pattern::Literal(_) => {}
            Pattern::Then { first, value, next } => {
                queue(&mut patterns, [first.as_ref(), next.as_ref()], remaining)?;
                pending.push(value);
            }
            Pattern::Remap { pattern, bindings } => {
                charge(remaining, bindings.len())?;
                patterns.push(pattern);
            }
            Pattern::Guard { pattern, condition } => {
                patterns.push(pattern);
                pending.push(condition);
            }
            Pattern::OptionalPresent(inner)
            | Pattern::ErrorResult { inner, .. }
            | Pattern::Observe { inner, .. }
            | Pattern::Binding { inner, .. } => patterns.push(inner),
            Pattern::Tuple(fields)
            | Pattern::Record { fields, .. }
            | Pattern::AnonymousRecord { fields, .. }
            | Pattern::Or(fields)
            | Pattern::Variant { fields, .. } => {
                queue(&mut patterns, fields, remaining)?;
            }
            _ => return Ok(false),
        }
    }
    Ok(true)
}
