//! Conservative fast-path evidence for default expressions.
use super::*;

fn stable_pattern(pattern: &n::Pattern) -> bool {
    match pattern {
        n::Pattern::Then { first, value, next } => {
            stable_pattern(first) && stable_observation(value) && stable_pattern(next)
        }
        n::Pattern::Guard { pattern, condition } => {
            stable_pattern(pattern) && stable_observation(condition)
        }
        n::Pattern::Binding { inner, .. } | n::Pattern::Remap { pattern: inner, .. } => {
            stable_pattern(inner)
        }
        n::Pattern::NormalForm {
            arguments: fields, ..
        }
        | n::Pattern::TypeTuple(fields)
        | n::Pattern::Tuple(fields)
        | n::Pattern::Or(fields)
        | n::Pattern::NominalType { fields, .. }
        | n::Pattern::Record { fields, .. }
        | n::Pattern::AnonymousRecord { fields, .. }
        | n::Pattern::Variant { fields, .. } => fields.iter().all(stable_pattern),
        _ => true,
    }
}
pub fn copy_scalar(ty: &Term) -> bool {
    match ty {
        Term::Primitive(
            P::Bool
            | P::Char
            | P::Void
            | P::I8
            | P::I16
            | P::I32
            | P::I64
            | P::I128
            | P::U8
            | P::U16
            | P::U32
            | P::U64
            | P::U128
            | P::Isize
            | P::Usize
            | P::F16
            | P::Bf16
            | P::F32
            | P::F64
            | P::BitVec(_),
        ) => true,
        // Copying an input pointer's representation neither observes its
        // pointee nor creates a new resource identity.
        Term::Pointer(_) => true,
        // An environment-free code value has no captured resource identity.
        // Copying a parameter's code reference does not invoke its body.
        Term::Function { .. } => true,
        Term::Tuple(items) => items.iter().all(copy_scalar),
        Term::Record(fields) => fields.iter().all(|(_, ty)| copy_scalar(ty)),
        _ => false,
    }
}
fn stable_observation(expr: &n::Expr) -> bool {
    copy_scalar(&expr.ty) && stable_default(expr)
}
// A later constructor field must not fail after an earlier owned field was
// acquired: that would execute hidden partial-construction cleanup. Until the
// general totality prover supplies evidence, use this finite infallible subset.
fn total_default_field(expr: &n::Expr) -> bool {
    match &expr.kind {
        // Forming a static text view is infallible and does not acquire an
        // external owner. This is construction evidence, not Str Copy evidence.
        K::Literal(n::Literal::String(_)) => expr.ty == p(P::Str),
        K::OptionalAbsent => true,
        K::OptionalPresent(value) => total_default_field(value),
        K::Function(_) | K::FunctionInstance { .. } => true,
        K::Closure { captures, .. } => captures.iter().all(|capture| {
            capture
                .value
                .as_ref()
                .is_none_or(|value| matches!(value.kind, K::Local(_)) || total_default_field(value))
        }),
        K::Literal(_) | K::Local(_) | K::Unit => copy_scalar(&expr.ty),
        K::Not(value) => total_default_field(value),
        K::Discard(value) | K::Let { value, .. } => {
            copy_scalar(&value.ty) && total_default_field(value)
        }
        K::And(a, b)
        | K::Or(a, b)
        | K::ScalarLess(a, b)
        | K::ScalarGreater(a, b)
        | K::ScalarLessEqual(a, b)
        | K::ScalarGreaterEqual(a, b)
        | K::TemplateEqual(a, b)
        | K::ScalarEqual(a, b)
        | K::BitEqual(a, b) => total_default_field(a) && total_default_field(b),
        K::AnonymousRecord { fields }
        | K::Record { fields, .. }
        | K::Variant { fields, .. }
        | K::Tuple(fields) => fields.iter().all(total_default_field),
        K::Block { statements, result } | K::CallPreparation { statements, result } => {
            statements
                .iter()
                .all(|statement| copy_scalar(&statement.ty) && total_default_field(statement))
                && result
                    .as_ref()
                    .is_none_or(|value| total_default_field(value))
        }
        K::If {
            condition,
            then_branch,
            else_branch,
        } => {
            total_default_field(condition)
                && total_default_field(then_branch)
                && else_branch
                    .as_ref()
                    .is_none_or(|value| total_default_field(value))
        }
        _ => false,
    }
}
pub fn stable_default(expr: &n::Expr) -> bool {
    match &expr.kind {
        K::Literal(n::Literal::String(_)) => return expr.ty == p(P::Str),
        K::OptionalAbsent => return true,
        K::OptionalPresent(value) => {
            return if copy_scalar(&value.ty) {
                stable_default(value)
            } else {
                total_default_field(value)
            };
        }
        _ => {}
    }
    // Direct constructors deliver all their fields to the result. They do not
    // invoke allocation, user code or Drop. A nominal result need not be Copy.
    // Local owned temporaries and projections are excluded below: their cleanup
    // can be observable even if all the field computations are stable.
    match &expr.kind {
        // Creating code/environment values does not invoke their bodies. Read
        // lexical captures by value; ownership admission checks their delivery.
        K::Function(_) | K::FunctionInstance { .. } | K::Closure { .. } => {
            return total_default_field(expr);
        }
        K::AnonymousRecord { fields }
        | K::Record { fields, .. }
        | K::Variant { fields, .. }
        | K::Tuple(fields) => {
            return if fields.iter().all(|field| copy_scalar(&field.ty)) {
                fields.iter().all(stable_default)
            } else {
                fields.iter().all(total_default_field)
            };
        }
        K::Block { statements, result } | K::CallPreparation { statements, result } => {
            return statements.iter().all(stable_observation)
                && result.as_ref().is_none_or(|value| stable_default(value));
        }
        K::If {
            condition,
            then_branch,
            else_branch,
        } => {
            return stable_observation(condition)
                && stable_default(then_branch)
                && else_branch
                    .as_ref()
                    .is_none_or(|value| stable_default(value));
        }
        K::Match { value, arms } => {
            return stable_observation(value)
                && arms.iter().all(|arm| {
                    stable_pattern(&arm.pattern)
                        && arm.guard.as_ref().is_none_or(stable_observation)
                        && stable_default(&arm.body)
                });
        }
        _ => {}
    }
    if !copy_scalar(&expr.ty) {
        return false;
    }
    match &expr.kind {
        K::Object { .. }
        | K::BuildMetadata { .. }
        | K::Intrinsic { .. }
        | K::ErrorConvert { .. }
        | K::NewtypeConvert(_)
        | K::BitWindow { .. }
        | K::BitIndex(..)
        | K::EffectResume(_)
        | K::EffectHandle { .. }
        | K::EffectRun { .. }
        | K::EffectComputation { .. }
        | K::EffectRequest { .. } => false,
        K::ComptimeValue(_) | K::AssociatedValue { .. } | K::Literal(_) | K::Local(_) | K::Unit => {
            true
        }
        K::BitSlice { value, .. }
        | K::BitConvert(value)
        | K::Snapshot(value)
        | K::TakeBits(value)
        | K::TextQuery { subject: value, .. }
        | K::IntegerRepresentation(value)
        | K::ComptimeIntegerConvert(value)
        | K::SymbolName(value)
        | K::EnumName(value)
        | K::FloatNegate(value)
        | K::Not(value) => stable_default(value),
        K::UnsafeScope(value) => stable_default(value),
        K::Discard(value) => stable_observation(value),
        // Structural scalar packets carry no owned field cleanup. Nominal
        // projections still require separate ownership/stability evidence.
        K::AnonymousField { value, .. } => stable_observation(value),
        K::FloatBinary {
            left: a, right: b, ..
        }
        | K::ScalarLess(a, b)
        | K::ScalarGreater(a, b)
        | K::ScalarGreaterEqual(a, b)
        | K::ScalarLessEqual(a, b)
        | K::TemplateEqual(a, b)
        | K::ScalarEqual(a, b)
        | K::BitEqual(a, b)
        | K::BitConcat(a, b)
        | K::And(a, b)
        | K::Or(a, b) => stable_default(a) && stable_default(b),
        K::TypeList(items) | K::TupleType(items) => items.iter().all(stable_default),
        K::Let { value, .. } => stable_observation(value),
        K::LetPattern {
            pattern,
            value,
            guard,
            otherwise,
        } => {
            stable_pattern(pattern)
                && otherwise.is_none()
                && stable_observation(value)
                && guard.as_ref().is_none_or(|g| stable_default(g))
        }
        K::IntegerNegate(value) => n::machine_integer_type(expr).is_some() && stable_default(value),
        K::IntegerBinary { left, right, .. } => {
            n::machine_integer_type(expr).is_some() && stable_default(left) && stable_default(right)
        }
        K::RequirePredicate(_)
        | K::EnsurePredicate(_)
        | K::CheckPredicate(_)
        | K::BitAssign { .. }
        | K::BitUpdate { .. }
        | K::Assign { .. }
        | K::Loop { .. }
        | K::Break(_)
        | K::Continue(_)
        | K::FunctionInstance { .. }
        | K::Function(_)
        | K::TraitMethod { .. }
        | K::Closure { .. }
        | K::Return(_)
        | K::Call { .. }
        | K::Field { .. }
        | K::TypeValue(_)
        | K::NormalFormType { .. }
        | K::BitVecType(_)
        | K::TypeEqual(..)
        | K::ListLength(_)
        | K::StructFieldTypes(_)
        | K::BuildStruct(_)
        | K::BuildEnum(_)
        | K::EnumStructure { .. }
        | K::ObjectQuery { .. }
        | K::ObjectEdit { .. }
        | K::ComptimeFunctionInstance { .. }
        | K::StructFieldNames(_)
        | K::ListConcat(..)
        | K::TypeListIndex(..)
        | K::NominalType { .. }
        | K::CheckTypeAlias { .. }
        | K::AddressOf(_)
        | K::ReadPointer(_)
        | K::PointerAnonymousField { .. }
        | K::PointerField { .. }
        | K::WritePointer { .. }
        | K::BitWritePointer { .. }
        | K::OptionalType(_)
        | K::PointerType(_)
        | K::ClosureType { .. }
        | K::FunctionType { .. }
        | K::FunctionResultType { .. } => false,
        K::AnonymousRecord { .. }
        | K::Record { .. }
        | K::Variant { .. }
        | K::Tuple(_)
        | K::Block { .. }
        | K::CallPreparation { .. }
        | K::If { .. }
        | K::Match { .. }
        | K::OptionalPresent(_)
        | K::OptionalAbsent => unreachable!(),
    }
}
