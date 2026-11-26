//! Structural expansion runs only after the visitor returns. A scope-aware
//! callback may recursively traverse a child; it must not retain the stack
//! storage of this operation dispatcher for every lexical frame.
use super::*;

// Keep this stack boundary when callers instantiate the small callback driver.
#[inline(never)]
pub(super) fn expand<'a, T: From<Node<'a>>>(
    node: Node<'a>,
    work: &mut Work<'_, T>,
    fuel: &mut usize,
    include_defaults: bool,
    include_closure_bodies: bool,
    allow_pattern_evidence: bool,
) -> Result<(), Error> {
    match node {
        Node::Argument(argument) => {
            use crate::program::NormalFormArgument as A;
            match argument {
                A::ListSpread(value) | A::Value(value) => push!(work, fuel, Node::Expr(value)),
                A::Literal { .. } => {}
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
                    extend!(work, fuel, statements.iter().map(Node::Expr));
                    extend!(work, fuel, result.as_deref().map(Node::Argument));
                }
                A::List { items, .. } | A::Tuple { items, .. } => {
                    extend!(work, fuel, items.iter().map(Node::Argument))
                }
                A::Match { value, arms, .. } => {
                    work::charge_entries(fuel, arms.len())?;
                    push!(work, fuel, Node::Expr(value));
                    for arm in arms {
                        push!(work, fuel, Node::Pattern(&arm.pattern));
                        extend!(work, fuel, arm.guard.iter().map(Node::Expr));
                        push!(work, fuel, Node::Argument(&arm.body));
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
                extend!(work, fuel, fields.iter().map(Node::Pattern));
            }
            P::NominalType { fields, .. }
            | P::Record { fields, .. }
            | P::Variant { fields, .. }
            | P::NormalForm {
                arguments: fields, ..
            }
            | P::Tuple(fields)
            | P::TypeTuple(fields)
            | P::Or(fields) => extend!(work, fuel, fields.iter().map(Node::Pattern)),
            P::CodeEvidence { .. } => {
                if !allow_pattern_evidence {
                    return Err(Error::Invalid("unresolved pattern-code evidence"));
                }
            }
            P::OptionalAbsent | P::Wild | P::Unit | P::Literal(_) => {}
        },
        Node::Expr(expr) => match &expr.kind {
            K::EffectHandle {
                computation,
                handlers,
                ..
            } => {
                push!(work, fuel, Node::Expr(computation));
                if include_defaults {
                    work::charge_entries(fuel, handlers.len())?;
                    for handler in handlers {
                        work::charge_entries(fuel, handler.arms.len())?;
                        for arm in &handler.arms {
                            push!(work, fuel, Node::Pattern(&arm.pattern));
                            if let Some(guard) = &arm.guard {
                                push!(work, fuel, Node::Expr(guard));
                            }
                            push!(work, fuel, Node::Expr(&arm.body));
                        }
                    }
                }
            }
            K::EffectResume(body)
            | K::EffectComputation { body, .. }
            | K::EffectRun {
                computation: body, ..
            } => push!(work, fuel, Node::Expr(body)),
            K::EffectRequest { arguments, .. } => {
                extend!(work, fuel, arguments.iter().map(Node::Expr))
            }
            K::Closure { captures, body, .. } => {
                work::charge_entries(fuel, captures.len())?;
                if include_closure_bodies {
                    push!(work, fuel, Node::Expr(body));
                }
                extend!(
                    work,
                    fuel,
                    captures
                        .iter()
                        .filter_map(|c| c.value.as_deref())
                        .map(Node::Expr),
                );
            }
            K::PlannedFunctionInstance {
                arguments, plan, ..
            }
            | K::NormalFormType {
                arguments, plan, ..
            }
            | K::ApplyNormalForm {
                arguments, plan, ..
            } => {
                extend!(
                    work,
                    fuel,
                    plan.iter()
                        .flat_map(|p| &p.preparations)
                        .map(|p| Node::Expr(&p.expression)),
                );
                extend!(work, fuel, arguments.iter().map(Node::Argument));
                if let K::ApplyNormalForm { callee, .. } = &expr.kind {
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
                    preparations.iter().map(|p| Node::Expr(&p.expression))
                );
                extend!(work, fuel, arguments.iter().map(Node::Argument));
            }
            K::ClosureType {
                environment,
                signature,
            } => {
                extend!(work, fuel, environment.iter().map(Node::Expr));
                push!(work, fuel, Node::Expr(signature));
            }
            K::WithTypeInputs {
                preparations,
                arguments,
                body,
                ..
            } => {
                extend!(
                    work,
                    fuel,
                    preparations.iter().map(|p| Node::Expr(&p.expression))
                );
                extend!(work, fuel, arguments.iter().map(Node::Argument));
                push!(work, fuel, Node::Expr(body));
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
                    preparations.iter().map(|p| Node::Expr(&p.expression))
                );
                extend!(work, fuel, arguments.iter().map(Node::Argument));
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
            | K::Tuple(fields) => extend!(work, fuel, fields.iter().map(Node::Expr)),
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
            | K::ConstAddress { value, .. }
            | K::ReadPointer(value)
            | K::PointerAnonymousField { pointer: value, .. }
            | K::PointerField { pointer: value, .. }
            | K::OptionalPresent(value)
            | K::OptionalType(value)
            | K::PointerType(value)
            | K::ListLength(value)
            | K::NewtypeRepresentation(value)
            | K::DefinitionAttributes(value)
            | K::StructFieldTypes(value)
            | K::RecordFieldTypes(value)
            | K::ReflectCallable(value)
            | K::TypeId(value)
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
            | K::RecordFieldNames(value)
            | K::ImplicitQuery { operand: value, .. }
            | K::NamedField { value, .. }
            | K::ConstructStruct { fields: value }
            | K::EnumVariantIndex(value)
            | K::StructFieldSpans(value)
            | K::RecordFieldSpans(value)
            | K::DeclarationSpan(value)
            | K::EnumVariantSpans(value)
            | K::EnumVariantFieldSpans(value)
            | K::ComptimeEnumVariantIndex(value)
            | K::ComptimeEnumVariantFieldValues(value)
            | K::BitVecType(value)
            | K::IntegerNegate(value)
            | K::FloatNegate(value)
            | K::BitConvert(value)
            | K::ErrorConvert { value, .. }
            | K::NewtypeConvert(value)
            | K::Snapshot(value)
            | K::TakeBits(value)
            | K::TextQuery { subject: value, .. }
            | K::IntegerRepresentation(value)
            | K::TryIntegerConvert(value)
            | K::ComptimeIntegerConvert(value)
            | K::SymbolName(value)
            | K::EnumName(value)
            | K::TypeKind { operand: value, .. }
            | K::TypeNotSized { operand: value }
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
                    .ok_or(Error::Budget("bit selection traversal budget exhausted"))?;
                push!(work, fuel, Node::Expr(pointer));
                extend!(
                    work,
                    fuel,
                    selections
                        .iter()
                        .flat_map(crate::program::BitSelection::expressions)
                        .map(Node::Expr),
                );
                push!(work, fuel, Node::Expr(value));
            }
            K::BitUpdate {
                selections, value, ..
            } => {
                *fuel = fuel
                    .checked_sub(selections.len())
                    .ok_or(Error::Budget("bit selection traversal budget exhausted"))?;
                extend!(
                    work,
                    fuel,
                    selections
                        .iter()
                        .flat_map(crate::program::BitSelection::expressions)
                        .map(Node::Expr),
                );
                push!(work, fuel, Node::Expr(value));
            }
            K::BitWindow { value, start, end }
            | K::EnumVariantFieldPtr {
                pointer: value,
                variant: start,
                field: end,
            }
            | K::ComptimeFieldEntry {
                value,
                field: start,
                ty: end,
            } => {
                push!(work, fuel, Node::Expr(value));
                push!(work, fuel, Node::Expr(start));
                push!(work, fuel, Node::Expr(end));
            }
            K::TypeHasField {
                subject: left,
                field: right,
            }
            | K::TypeFieldType {
                subject: left,
                field: right,
            }
            | K::StructFieldType {
                subject: left,
                field: right,
            }
            | K::FieldPtr {
                pointer: left,
                field: right,
            }
            | K::FieldValue {
                value: left,
                field: right,
            }
            | K::ConstructEnum {
                variant: left,
                fields: right,
            }
            | K::TupleAppend {
                tuple: left,
                element: right,
            } => {
                extend!(work, fuel, [Node::Expr(right), Node::Expr(left)]);
            }
            K::Match { value, arms } => {
                work::charge_entries(fuel, arms.len())?;
                push!(work, fuel, Node::Expr(value));
                for arm in arms {
                    push!(work, fuel, Node::Pattern(&arm.pattern));
                    extend!(work, fuel, arm.guard.iter().map(Node::Expr));
                    push!(work, fuel, Node::Expr(&arm.body));
                }
            }
            K::ComptimeFunctionInstance { inputs, .. } => {
                extend!(work, fuel, inputs.expressions().map(Node::Expr));
            }
            K::Block { statements, result } | K::CallPreparation { statements, result } => {
                extend!(work, fuel, statements.iter().map(Node::Expr));
                extend!(work, fuel, result.as_deref().map(Node::Expr));
            }
            K::LetPattern {
                pattern,
                value,
                guard,
                otherwise,
            } => {
                push!(work, fuel, Node::Pattern(pattern));
                push!(work, fuel, Node::Expr(value));
                extend!(work, fuel, guard.as_deref().map(Node::Expr));
                extend!(work, fuel, otherwise.as_deref().map(Node::Expr));
            }
            K::If {
                condition,
                then_branch,
                else_branch,
            } => {
                push!(work, fuel, Node::Expr(condition));
                push!(work, fuel, Node::Expr(then_branch));
                extend!(work, fuel, else_branch.as_deref().map(Node::Expr));
            }
            K::Return(value) => extend!(work, fuel, value.as_deref().map(Node::Expr)),
            K::Call {
                callee,
                arguments,
                env_arguments,
                ..
            } => {
                work::charge_entries(fuel, arguments.len())?;
                work::charge_entries(fuel, env_arguments.len())?;
                push!(work, fuel, Node::Expr(callee));
                extend!(
                    work,
                    fuel,
                    arguments
                        .iter()
                        .filter(|a| include_defaults || !a.is_default)
                        .map(|a| Node::Expr(&a.value)),
                );
                extend!(work, fuel, env_arguments.iter().map(Node::Expr));
            }
            K::TraitObjectType { arguments, .. } => {
                extend!(work, fuel, arguments.iter().map(Node::Expr));
            }
            K::DynCast(cast) => {
                work::charge_entries(fuel, cast.env_arguments.len())?;
                push!(work, fuel, Node::Expr(&cast.value));
                extend!(work, fuel, cast.env_arguments.iter().map(Node::Expr));
            }
            K::DynCall(call) => {
                work::charge_entries(fuel, call.arguments.len())?;
                push!(work, fuel, Node::Expr(&call.receiver));
                extend!(
                    work,
                    fuel,
                    call.arguments
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
            | K::TraitMethod { .. }
            | K::StaticAddress(_) => {}
        },
    }
    Ok(())
}
