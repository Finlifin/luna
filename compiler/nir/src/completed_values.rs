//! Closed value reconstruction, not arbitrary initialization programs. Closure
//! bodies are immutable checked code; only their capture initializers execute
//! during reconstruction. Ordinary typing/phase checks still apply to that code.
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
            Term::Computation { effects, .. } => {
                charge(fuel, effects.len())?;
                effects.iter().fold(1usize, |width, operation| width.saturating_add(operation.arguments.len()))
            }
            Term::AssociatedType { arguments, .. } => arguments.len().saturating_add(1),
            Term::Nominal { arguments, .. }
            | Term::NormalForm { arguments, .. }
            | Term::Tuple(arguments)
            | Term::TypeListValue(arguments) | Term::TypeListConcat(arguments) | Term::TupleValue(arguments) => arguments.len(),
            Term::Closure { environment, .. } => environment.len().saturating_add(1),
            Term::Function { parameters, .. } => parameters.len().saturating_add(1),
            Term::NominalValue { fields, .. } => fields.len().saturating_add(1),
            Term::Record(fields) | Term::RecordValue(fields) => fields.len(),
            Term::Pointer(_) | Term::ComptimeFunction(_) => 1,
            _ => 0,
        };
        charge(fuel, width)?;
        let mut children = Vec::new();
        match ty {
            Term::Computation { effects, result } => {
                children.push(result.as_ref());
                for operation in effects { children.extend(&operation.arguments); }
            }
            Term::AssociatedType {
                subject, arguments, ..
            } => {
                children.push(subject.as_ref());
                children.extend(arguments);
            }
            Term::Nominal { arguments, .. }
            | Term::NormalForm { arguments, .. }
            | Term::Tuple(arguments)
            | Term::TypeListValue(arguments) | Term::TypeListConcat(arguments) | Term::TupleValue(arguments) => children.extend(arguments),
            Term::Closure {
                environment,
                signature,
            } => {
                children.extend(environment);
                children.push(signature);
            }
            Term::Pointer(inner) | Term::ComptimeFunction(inner) => children.push(inner),
            Term::Function { parameters, result } => {
                children.extend(parameters.iter().map(|p| &p.ty));
                children.push(result);
            }
            Term::NominalValue { ty, fields, .. } => { children.push(ty); children.extend(fields); },
            Term::Record(fields) | Term::RecordValue(fields) => {
                for (name, field) in fields {
                    charge(fuel, name.len())?;
                    children.push(field);
                }
            }
            Term::String(value) | Term::Symbol(value) => charge(fuel, value.len())?,
            Term::Integer(value) => charge(fuel, value.bit_len().div_ceil(32))?,
            Term::BitVec(value) => charge(fuel, value.words().len())?,
            _ => {}
        }
        pending.extend(children.into_iter().map(|t| (t, depth + 1)));
    }
    Ok(())
}

/// Validate and bound the serialized value graph before copying/substitution.
/// The only permitted call reconstructs a private graph through a zero-input,
/// captureless comptime closure. Its body must itself be a completed value graph.
pub fn validate(value: &Expr, fuel: &mut usize) -> Result<(), &'static str> {
    value_in(value, fuel, &mut BTreeMap::new(), 0)
}
fn value_in<'a>(
    value: &'a Expr,
    fuel: &mut usize,
    scope: &mut BTreeMap<LocalId, &'a Term>,
    depth: usize,
) -> Result<(), &'static str> {
    if depth >= 64 {
        return Err("completed value representation depth exhausted");
    }
    charge(fuel, 1)?;
    ty(&value.ty, fuel)?;
    match &value.kind {
        K::Literal(Literal::Symbol(s) | Literal::String(s) | Literal::Integer(s) | Literal::Float(s)) => {
            charge(fuel, s.len())
        }
        K::Literal(Literal::BitVec { words, .. }) => charge(fuel, words.len()),
        K::Literal(_) | K::Unit | K::Function(_) => Ok(()),
        K::TypeValue(t) => ty(t, fuel),
        K::TypeListValue(types) => {
            for element in types {
                ty(element, fuel)?;
                if element.sort()!=crate::Sort::Type {return Err("completed type-list element is not Type");}
            }
            Ok(())
        }
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
        K::Tuple(fields)
        | K::Record { fields, .. }
        | K::Variant { fields, .. }
        | K::AnonymousRecord { fields } => {
            for field in fields {
                value_in(field, fuel, scope, depth + 1)?;
            }
            Ok(())
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
                value_in(captured, fuel, scope, depth + 1)?;
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
            value_in(body, fuel, &mut BTreeMap::new(), depth + 1)
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
                    value_in(value, fuel, scope, depth + 1)?;
                    scope.insert(*binding, &value.ty);
                    introduced.push(*binding);
                }
                let result = result.as_deref().ok_or("completed graph lacks a result")?;
                if result.ty != value.ty {
                    return Err("completed graph result type mismatch");
                }
                value_in(result, fuel, scope, depth + 1)
            })();
            for binding in introduced {
                scope.remove(&binding);
            }
            checked
        }
        _ => Err("associated value requires a completed constant representation"),
    }
}
