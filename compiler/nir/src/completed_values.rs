//! Closed value reconstruction, not arbitrary initialization programs. Closure
//! bodies are immutable checked code; only their capture initializers execute
//! during reconstruction. Ordinary typing/phase checks still apply to that code.
use crate::normal_forms::NominalValueDeclaration;
use crate::program::{Expr, ExprKind as K, Literal, LocalId};
use crate::{Primitive, Term};
use std::collections::BTreeMap;

fn charge(fuel: &mut usize, amount: usize) -> Result<(), &'static str> {
    *fuel = fuel
        .checked_sub(amount)
        .ok_or("completed value budget exhausted")?;
    Ok(())
}
fn ty(ty: &Term, fuel: &mut usize) -> Result<(), &'static str> {
    let mut pending = vec![(ty, 0)];
    while let Some((ty, depth)) = pending.pop() {
        if depth >= 64 {
            return Err("completed value type depth exhausted");
        }
        charge(fuel, 1)?;
        let width = match ty {
            Term::ErrorResult { errors, .. } => errors.len().saturating_add(1),
            Term::Computation { effects, .. } => {
                charge(fuel, effects.len())?;
                effects.iter().fold(1usize, |width, operation| {
                    width.saturating_add(operation.arguments.len())
                })
            }
            Term::AssociatedType { arguments, .. } => arguments.len().saturating_add(1),
            Term::Nominal { arguments, .. }
            | Term::ClosureSnapshot { arguments, .. }
            | Term::FunctionValue { arguments, .. }
            | Term::NormalForm { arguments, .. }
            | Term::Tuple(arguments)
            | Term::ClosureContextValue(arguments)
            | Term::TypeListValue(arguments)
            | Term::TupleValue(arguments) => arguments.len(),
            Term::ContextualClosure { .. } => 2,
            Term::Closure { environment, .. } => environment.len().saturating_add(1),
            Term::Function { parameters, .. } => parameters.len().saturating_add(1),
            Term::OptionalValue { value, .. } => 1 + usize::from(value.is_some()),
            Term::ErrorValue { .. } => 3,
            Term::ListConcat { parts: fields, .. } | Term::NominalValue { fields, .. } => {
                fields.len().saturating_add(1)
            }
            Term::Record(fields) | Term::RecordValue(fields) => fields.len(),
            Term::Optional(_)
            | Term::Pointer(_)
            | Term::ComptimeFunction(_)
            | Term::NormalFormFunction(..)
            | Term::UnsafeFunction(_) => 1,
            _ => 0,
        };
        charge(fuel, width)?;
        let mut children = Vec::new();
        match ty {
            Term::ErrorResult { errors, result } => {
                children.push(result.as_ref());
                children.extend(errors);
            }
            Term::Computation { effects, result } => {
                children.push(result.as_ref());
                for operation in effects {
                    children.extend(&operation.arguments);
                }
            }
            Term::AssociatedType {
                subject, arguments, ..
            } => {
                children.push(subject.as_ref());
                children.extend(arguments);
            }
            Term::Nominal { arguments, .. }
            | Term::ClosureSnapshot { arguments, .. }
            | Term::FunctionValue { arguments, .. }
            | Term::NormalForm { arguments, .. }
            | Term::Tuple(arguments)
            | Term::ClosureContextValue(arguments)
            | Term::TypeListValue(arguments)
            | Term::TupleValue(arguments) => children.extend(arguments),
            Term::ContextualClosure { context, signature } => {
                children.push(context);
                children.push(signature);
            }
            Term::Closure {
                environment,
                signature,
            } => {
                children.extend(environment);
                children.push(signature);
            }
            Term::Optional(inner)
            | Term::Pointer(inner)
            | Term::ComptimeFunction(inner)
            | Term::NormalFormFunction(inner, _)
            | Term::UnsafeFunction(inner) => children.push(inner),
            Term::Function { parameters, result } => {
                children.extend(parameters.iter().map(|p| &p.ty));
                children.push(result);
            }
            Term::OptionalValue { ty, value } => {
                children.push(ty);
                if let Some(value) = value {
                    children.push(value);
                }
            }

            Term::ErrorValue { ty, label, value } => {
                children.push(ty);
                if let Some(label) = label {
                    children.push(label);
                }
                children.push(value);
            }
            Term::ListConcat { ty, parts: fields } | Term::NominalValue { ty, fields, .. } => {
                children.push(ty);
                children.extend(fields);
            }
            Term::Record(fields) | Term::RecordValue(fields) => {
                for (name, field) in fields {
                    charge(fuel, name.len())?;
                    children.push(field);
                }
            }
            Term::String(value) | Term::Symbol(value) => charge(fuel, value.len())?,
            Term::Integer(value) => charge(fuel, value.bit_len().div_ceil(32))?,
            Term::Float(value) => charge(
                fuel,
                value
                    .digits()
                    .len()
                    .saturating_add(value.exponent().bit_len().div_ceil(32)),
            )?,
            Term::BitVec(value) => charge(fuel, value.words().len())?,
            _ => {}
        }
        pending.extend(children.into_iter().map(|t| (t, depth + 1)));
    }
    Ok(())
}

fn literal_payload(literal: &Literal, fuel: &mut usize) -> Result<(), &'static str> {
    use comptime_runtime::{Budget, Error, Integer};
    let mut budget = Budget {
        steps: *fuel,
        max_limbs: *fuel,
    };
    let checked = match literal {
        Literal::Integer(spelling) => {
            Integer::parse_literal(spelling.strip_prefix('-').unwrap_or(spelling), &mut budget)
                .map(|_| ())
        }
        Literal::Float(spelling) => {
            comptime_runtime::decimal::DecimalValue::parse_literal(spelling, &mut budget)
                .map(|_| ())
        }
        Literal::BitVec { width, words } => {
            comptime_runtime::bitvec::BitVec::from_words(*width, words, &mut budget)
                .map(|_| ())
                .map_err(|error| match error {
                    comptime_runtime::bitvec::Error::Resource(error) => error,
                    _ => Error::InvalidLiteral,
                })
        }
        _ => return Ok(()),
    };
    // Failed parsing consumes work too; never reset the caller's budget.
    *fuel = budget.steps;
    checked.map_err(|error| match error {
        Error::BudgetExhausted | Error::SizeLimit => "completed value budget exhausted",
        _ => "invalid completed literal payload",
    })
}

/// Validate and bound the serialized value graph before copying/substitution.
/// The only permitted call reconstructs a private graph through a zero-input,
/// captureless comptime closure. Its body must itself be a completed value graph.
pub fn validate(value: &Expr, fuel: &mut usize) -> Result<(), &'static str> {
    validate_with_newtypes(value, std::iter::empty(), fuel)
}

pub fn validate_in(
    value: &Expr,
    target: crate::target::TargetData,
    fuel: &mut usize,
) -> Result<(), &'static str> {
    validate_with_declarations_in(
        value,
        std::iter::empty(),
        &|_| false,
        &|_, _| None,
        Some(target),
        fuel,
    )
}

/// Validate frozen nominal wrappers using declaration-owned representation
/// evidence. No representation is guessed from nominal arguments or names.
pub fn validate_with_newtypes<'d>(
    value: &Expr,
    definitions: impl Iterator<Item = &'d crate::newtypes::Newtype> + Clone,
    fuel: &mut usize,
) -> Result<(), &'static str> {
    validate_with_layouts(value, definitions, &|_| false, fuel)
}

/// Tuple views must come from the registered protocol, never a head's shape.
pub fn validate_with_layouts<'d>(
    value: &Expr,
    definitions: impl Iterator<Item = &'d crate::newtypes::Newtype> + Clone,
    is_tuple: &impl Fn(crate::DeclarationId) -> bool,
    fuel: &mut usize,
) -> Result<(), &'static str> {
    validate_with_declarations(value, definitions, is_tuple, &|_, _| None, fuel)
}

pub fn lookup_nominal<'a>(
    package: &'a crate::program::Package,
    id: crate::DeclarationId,
    fuel: &mut usize,
) -> Option<NominalValueDeclaration<'a>> {
    let mut found = None;
    for definition in &package.structs {
        charge(fuel, 1).ok()?;
        if definition.declaration == id {
            if found.is_some() {
                return None;
            }
            found = Some(NominalValueDeclaration::Struct(definition));
        }
    }
    for definition in &package.enums {
        charge(fuel, 1).ok()?;
        if definition.declaration == id {
            if found.is_some() {
                return None;
            }
            found = Some(NominalValueDeclaration::Enum(definition));
        }
    }
    found
}

/// Declaration layouts validate constructors without unfolding opaque heads.
/// Generic template arguments remain symbolic; invocation checks their domains.
pub fn validate_with_declarations<'d>(
    value: &Expr,
    definitions: impl Iterator<Item = &'d crate::newtypes::Newtype> + Clone,
    is_tuple: &impl Fn(crate::DeclarationId) -> bool,
    lookup: &impl Fn(crate::DeclarationId, &mut usize) -> Option<NominalValueDeclaration<'d>>,
    fuel: &mut usize,
) -> Result<(), &'static str> {
    validate_with_declarations_in(value, definitions, is_tuple, lookup, None, fuel)
}

/// Target facts apply to every reconstructed child, including closure captures.
/// No target means platform integers remain unadmitted.
pub fn validate_with_declarations_in<'d>(
    value: &Expr,
    definitions: impl Iterator<Item = &'d crate::newtypes::Newtype> + Clone,
    is_tuple: &impl Fn(crate::DeclarationId) -> bool,
    lookup: &impl Fn(crate::DeclarationId, &mut usize) -> Option<NominalValueDeclaration<'d>>,
    target: Option<crate::target::TargetData>,
    fuel: &mut usize,
) -> Result<(), &'static str> {
    validate_with_layout_evidence_in(
        value,
        definitions,
        &|ty| {
            let Term::NormalForm {
                head,
                sort: crate::Sort::Type,
                arguments,
            } = ty
            else {
                return None;
            };
            if !is_tuple(*head) {
                return None;
            }
            let [Term::TypeListValue(fields)] = arguments.as_slice() else {
                return None;
            };
            Some(fields.as_slice())
        },
        lookup,
        target,
        fuel,
    )
}

/// Reconstruct using borrowed declaration-owned tuple layouts. Callers supply
/// the exact head's metadata; neither names nor payload shape confer authority.
pub fn validate_with_layout_evidence_in<'d>(
    value: &Expr,
    definitions: impl Iterator<Item = &'d crate::newtypes::Newtype> + Clone,
    tuple_fields: &impl for<'t> Fn(&'t Term) -> Option<&'t [Term]>,
    lookup: &impl Fn(crate::DeclarationId, &mut usize) -> Option<NominalValueDeclaration<'d>>,
    target: Option<crate::target::TargetData>,
    fuel: &mut usize,
) -> Result<(), &'static str> {
    value_in(
        value,
        fuel,
        &mut BTreeMap::new(),
        0,
        &definitions,
        tuple_fields,
        lookup,
        target,
    )
}
fn value_in<'a, 'd>(
    value: &'a Expr,
    fuel: &mut usize,
    scope: &mut BTreeMap<LocalId, &'a Term>,
    depth: usize,
    definitions: &(impl Iterator<Item = &'d crate::newtypes::Newtype> + Clone),
    tuple_fields: &impl for<'t> Fn(&'t Term) -> Option<&'t [Term]>,
    lookup: &impl Fn(crate::DeclarationId, &mut usize) -> Option<NominalValueDeclaration<'d>>,
    target: Option<crate::target::TargetData>,
) -> Result<(), &'static str> {
    if depth >= 64 {
        return Err("completed value representation depth exhausted");
    }
    charge(fuel, 1)?;
    ty(&value.ty, fuel)?;
    if let K::Literal(literal) = &value.kind {
        if !literal.has_primitive_type_in(&value.ty, target) {
            return Err("completed literal disagrees with its declared type");
        }
    }
    match &value.kind {
        K::OptionalAbsent if matches!(value.ty, Term::Optional(_)) => Ok(()),
        K::OptionalPresent(payload) => {
            let Term::Optional(inner) = &value.ty else {
                return Err("optional value requires optional type");
            };
            if payload.ty != **inner {
                return Err("optional payload type mismatch");
            }
            value_in(
                payload,
                fuel,
                scope,
                depth + 1,
                definitions,
                tuple_fields,
                lookup,
                target,
            )
        }
        K::Literal(Literal::Symbol(s) | Literal::String(s)) => charge(fuel, s.len()),
        K::Literal(
            literal @ (Literal::Integer(_) | Literal::Float(_) | Literal::BitVec { .. }),
        ) => literal_payload(literal, fuel),
        K::Literal(_) | K::Function(_) => Ok(()),
        K::Unit if matches!(value.ty, Term::Primitive(Primitive::Unit | Primitive::Void)) => Ok(()),
        K::TypeValue(t)
            if value.ty == Term::Primitive(Primitive::Type) && t.sort() == crate::Sort::Type =>
        {
            ty(t, fuel)
        }
        // A captured data parameter is already a value, not a deferred program.
        // Specialization supplies its domain-checked concrete representation.
        K::ComptimeValue(t) => crate::comptime_values::validate_in(t, &value.ty, target, fuel)
            .map_err(|error| error.message()),
        K::FunctionInstance { arguments, .. } => {
            for t in arguments {
                ty(t, fuel)?;
            }
            Ok(())
        }
        K::TraitMethod {
            subject,
            interface_arguments,
            method_arguments,
            ..
        } => {
            ty(subject, fuel)?;
            for t in interface_arguments.iter().chain(method_arguments) {
                ty(t, fuel)?;
            }
            Ok(())
        }
        K::Tuple(fields) => {
            let expected = match &value.ty {
                Term::Tuple(types) => types.as_slice(),
                Term::NormalForm { .. } => tuple_fields(&value.ty)
                    .ok_or("completed tuple lacks registered layout evidence")?,
                _ => return Err("completed tuple lacks registered layout evidence"),
            };
            if fields.len() != expected.len() {
                return Err("completed tuple field count mismatch");
            }
            for (field, expected) in fields.iter().zip(expected) {
                if &field.ty != expected {
                    return Err("completed tuple field type mismatch");
                }
                value_in(
                    field,
                    fuel,
                    scope,
                    depth + 1,
                    definitions,
                    tuple_fields,
                    lookup,
                    target,
                )?;
            }
            Ok(())
        }
        K::AnonymousRecord { fields } => {
            let Term::Record(expected) = &value.ty else {
                return Err("completed record lacks a structural record type");
            };
            if fields.len() != expected.len() {
                return Err("completed record field count mismatch");
            }
            let mut names = std::collections::BTreeSet::new();
            for (field, (name, expected)) in fields.iter().zip(expected) {
                charge(fuel, 1)?;
                if !names.insert(name) || &field.ty != expected {
                    return Err("completed record field identity or type mismatch");
                }
                value_in(
                    field,
                    fuel,
                    scope,
                    depth + 1,
                    definitions,
                    tuple_fields,
                    lookup,
                    target,
                )?;
            }
            Ok(())
        }
        K::Record {
            declaration,
            fields,
        }
        | K::Variant {
            declaration,
            fields,
            ..
        } => {
            let Term::Nominal {
                declaration: owner,
                arguments,
            } = &value.ty
            else {
                return Err("completed nominal constructor lacks a nominal type");
            };
            if declaration != owner {
                return Err("completed nominal constructor identity mismatch");
            }
            let definition = lookup(*owner, fuel).ok_or("completed nominal layout unavailable")?;
            let (parameters, domains, expected): (_, _, Vec<&Term>) =
                match (definition, &value.kind) {
                    (NominalValueDeclaration::Struct(definition), K::Record { .. })
                        if definition.declaration == *owner =>
                    {
                        charge(
                            fuel,
                            definition.type_parameters.len() + definition.fields.len(),
                        )?;
                        (
                            &definition.type_parameters,
                            &definition.parameter_types,
                            definition.fields.iter().map(|(_, ty)| ty).collect(),
                        )
                    }
                    (NominalValueDeclaration::Enum(definition), K::Variant { index, .. })
                        if definition.declaration == *owner =>
                    {
                        charge(fuel, definition.type_parameters.len() + 1)?;
                        let variant = definition
                            .variants
                            .get(*index)
                            .ok_or("completed enum variant out of bounds")?;
                        charge(fuel, variant.fields.len())?;
                        (
                            &definition.type_parameters,
                            &definition.parameter_types,
                            variant.fields.iter().collect(),
                        )
                    }
                    _ => return Err("completed nominal constructor kind mismatch"),
                };
            if fields.len() != expected.len() || arguments.len() != parameters.len() {
                return Err("completed nominal field or argument count mismatch");
            }
            let expected = crate::normal_forms::instantiate_value_fields(
                parameters,
                domains,
                arguments,
                expected.into_iter(),
                fuel,
            )
            .ok_or("completed nominal layout instantiation failed")?;
            for (field, expected) in fields.iter().zip(expected) {
                if field.ty != expected {
                    return Err("completed nominal field type mismatch");
                }
                value_in(
                    field,
                    fuel,
                    scope,
                    depth + 1,
                    definitions,
                    tuple_fields,
                    lookup,
                    target,
                )?;
            }
            Ok(())
        }
        K::ErrorConvert {
            conversion,
            value: payload,
        } => {
            // Completed payloads can undergo checked row and product layout
            // transport. No call or other deferred computation is introduced.
            crate::error_results::validate_conversion_with_layouts(
                *conversion,
                &payload.ty,
                &value.ty,
                fuel,
                &mut |_, _| false,
                tuple_fields,
            )
            .map_err(|error| match error {
                crate::error_results::Error::Budget => "completed value budget exhausted",
                _ => "completed error payload type mismatch",
            })?;
            value_in(
                payload,
                fuel,
                scope,
                depth + 1,
                definitions,
                tuple_fields,
                lookup,
                target,
            )
        }
        K::NewtypeConvert(payload) => {
            let conversion = crate::newtypes::conversion_from(
                definitions.clone(),
                &payload.ty,
                &value.ty,
                &[],
                fuel,
            )
            .map_err(|_| "invalid completed newtype representation")?;
            if !conversion.is_some_and(|edge| edge.direction() == crate::newtypes::Direction::Wrap)
            {
                return Err("completed newtype requires a declared wrapping edge");
            }
            value_in(
                payload,
                fuel,
                scope,
                depth + 1,
                definitions,
                tuple_fields,
                lookup,
                target,
            )
        }
        K::Local(binding) => {
            if scope.get(binding).is_some_and(|ty| *ty == &value.ty) {
                Ok(())
            } else {
                Err("completed value references an unbound or mistyped local")
            }
        }
        K::Closure {
            parameters,
            captures,
            body,
        } => {
            charge(fuel, parameters.len() + captures.len())?;
            let environment: &[Term] = match &value.ty {
                Term::Closure { environment, .. } => environment,
                _ => &[],
            };
            let Term::Function {
                parameters: inputs,
                result,
            } = value.ty.callable_signature()
            else {
                return Err("completed closure lacks a callable signature");
            };
            if environment.len() != captures.len()
                || parameters.len() != inputs.len()
                || result.as_ref() != &body.ty
            {
                return Err("completed closure shape mismatch");
            }
            let mut bindings = std::collections::BTreeSet::new();
            for binding in parameters {
                if !bindings.insert(*binding) {
                    return Err("duplicate completed closure binding");
                }
            }
            for (capture, expected) in captures.iter().zip(environment) {
                if !bindings.insert(capture.binding) || &capture.ty != expected {
                    return Err("completed closure capture mismatch");
                }
                let captured = capture
                    .value
                    .as_deref()
                    .ok_or("completed closure has an implicit capture")?;
                if captured.ty != capture.ty {
                    return Err("completed closure capture type mismatch");
                }
                value_in(
                    captured,
                    fuel,
                    scope,
                    depth + 1,
                    definitions,
                    tuple_fields,
                    lookup,
                    target,
                )?;
            }
            Ok(())
        }
        K::Call {
            callee,
            arguments,
            parameter_bindings,
        } => {
            let K::Closure {
                parameters,
                captures,
                body,
            } = &callee.kind
            else {
                return Err("completed value cannot invoke a declaration");
            };
            if !arguments.is_empty()
                || !parameter_bindings.is_empty()
                || !parameters.is_empty()
                || !captures.is_empty()
                || !matches!(callee.ty, Term::ComptimeFunction(_))
            {
                return Err("invalid completed value reconstruction call");
            }
            let Term::Function {
                parameters: inputs,
                result,
            } = callee.ty.callable_signature()
            else {
                return Err("invalid reconstruction signature");
            };
            if !inputs.is_empty() || result.as_ref() != &value.ty || body.ty != value.ty {
                return Err("reconstruction result mismatch");
            }
            ty(&callee.ty, fuel)?;
            value_in(
                body,
                fuel,
                &mut BTreeMap::new(),
                depth + 1,
                definitions,
                tuple_fields,
                lookup,
                target,
            )
        }
        K::Block { statements, result } => {
            let mut introduced = Vec::new();
            let checked = (|| {
                for statement in statements {
                    charge(fuel, 1)?;
                    let K::Let { binding, value } = &statement.kind else {
                        return Err("completed graph contains an executable statement");
                    };
                    if statement.ty != Term::Primitive(Primitive::Void)
                        || scope.contains_key(binding)
                    {
                        return Err("invalid completed graph binding");
                    }
                    value_in(
                        value,
                        fuel,
                        scope,
                        depth + 1,
                        definitions,
                        tuple_fields,
                        lookup,
                        target,
                    )?;
                    scope.insert(*binding, &value.ty);
                    introduced.push(*binding);
                }
                let result = result.as_deref().ok_or("completed graph lacks a result")?;
                if result.ty != value.ty {
                    return Err("completed graph result type mismatch");
                }
                value_in(
                    result,
                    fuel,
                    scope,
                    depth + 1,
                    definitions,
                    tuple_fields,
                    lookup,
                    target,
                )
            })();
            for binding in introduced {
                scope.remove(&binding);
            }
            checked
        }
        _ => Err("associated value requires a completed constant representation"),
    }
}
