//! Reconstruct inert frozen data using checked declaration layouts.
use super::*;

pub(super) fn fold(
    heads: Layouts<'_>,
    expr: &Expr,
    value: &Term,
    fuel: &mut usize,
    depth: usize,
) -> Result<Expr, String> {
    nir::type_computations::charge_term(value, fuel).map_err(|e| e.to_string())?;
    crate::completed_type(value, &BTreeSet::new(), fuel)?;
    nir::type_computations::charge_term(&expr.ty, fuel).map_err(|e| e.to_string())?;
    let result = decode(heads, value, &expr.ty, expr.source, fuel, depth)?;
    // Reuse admission for ordinary constructors, including every element,
    // before allowing an index or projection to discard unselected data.
    constant(heads, &result, fuel, depth)?;
    Ok(result)
}

fn decode(
    heads: Layouts<'_>,
    value: &Term,
    ty: &Term,
    source: nir::program::SourceRange,
    fuel: &mut usize,
    depth: usize,
) -> Result<Expr, String> {
    if depth >= 128 {
        return Err("staged frozen value depth exceeded".into());
    }
    charge(fuel, 1)?;
    // Each reconstructed field owns its domain; repeated nested domains must
    // pay for every copy rather than only the outer list's type description.
    nir::type_computations::charge_term(ty, fuel).map_err(|e| e.to_string())?;
    let child = |value: &Term, ty: &Term, fuel: &mut usize| {
        decode(heads, value, ty, source, fuel, depth + 1)
    };
    if value.scalar_index_domain().is_some() {
        return nir::scalar_construction::expression(value, ty, heads.target, source, fuel)
            .map_err(|error| error.to_string());
    }
    let kind = if *ty == Term::Primitive(Primitive::Type) && value.sort() == Sort::Type {
        K::TypeValue(value.clone())
    } else {
        match value {
            Term::FunctionValue {
                declaration,
                arguments,
            } if matches!(
                ty,
                Term::NormalFormFunction(..)
                    | Term::Function { .. }
                    | Term::ComptimeFunction(_)
                    | Term::UnsafeFunction(_)
            ) =>
            {
                if arguments.is_empty() {
                    K::Function(*declaration)
                } else {
                    for argument in arguments {
                        crate::completed_type(argument, &std::collections::BTreeSet::new(), fuel)?;
                    }
                    K::FunctionInstance {
                        declaration: *declaration,
                        arguments: arguments.clone(),
                    }
                }
            }
            Term::MethodValue(method)
                if matches!(
                    ty,
                    Term::Function { .. } | Term::ComptimeFunction(_) | Term::UnsafeFunction(_)
                ) =>
            {
                crate::completed_type(&method.subject, &std::collections::BTreeSet::new(), fuel)?;
                for argument in method
                    .interface
                    .arguments
                    .iter()
                    .chain(&method.method_arguments)
                {
                    crate::completed_type(argument, &std::collections::BTreeSet::new(), fuel)?;
                }
                K::TraitMethod {
                    interface: method.interface.declaration,
                    interface_arguments: method.interface.arguments.clone(),
                    member: method.member,
                    subject: method.subject.clone(),
                    method_arguments: method.method_arguments.clone(),
                }
            }
            Term::ClosureValue { .. } => {
                return Err("staged closure value requires its source closure expression".into());
            }
            Term::ClosureSnapshot { .. } => {
                return Err(
                    "staged closure snapshot decoding requires factory lookup outside staging layouts"
                        .into(),
                );
            }

            Term::NominalValue {
                ty: domain,
                variant: None,
                fields,
            } if domain.as_ref() == ty => {
                let element = heads
                    .list_element(ty, fuel)?
                    .ok_or("frozen list requires a bound list layout")?;
                charge(fuel, fields.len())?;
                K::TypeList(
                    fields
                        .iter()
                        .map(|value| child(value, &element, fuel))
                        .collect::<Result<_, _>>()?,
                )
            }
            Term::TupleValue(fields) => {
                let types = match ty {
                    Term::Tuple(types) => types.as_slice(),
                    _ => {
                        let mut selected = None;
                        for head in heads {
                            charge(fuel, 1)?;
                            if let Some(types) = head.tuple_elements(ty) {
                                selected = Some(types);
                                break;
                            }
                        }
                        selected.ok_or("frozen tuple requires a bound tuple layout")?
                    }
                };
                if fields.len() != types.len() {
                    return Err("frozen tuple arity mismatch".into());
                }
                K::Tuple(
                    fields
                        .iter()
                        .zip(types)
                        .map(|(value, ty)| child(value, ty, fuel))
                        .collect::<Result<_, _>>()?,
                )
            }
            Term::RecordValue(fields) => {
                let Term::Record(types) = ty else {
                    return Err("frozen record requires its field domains".into());
                };
                if fields.len() != types.len() {
                    return Err("frozen record arity mismatch".into());
                }
                K::AnonymousRecord {
                    fields: fields
                        .iter()
                        .zip(types)
                        .map(|((name, value), (expected, ty))| {
                            if name != expected {
                                return Err("frozen record label mismatch".into());
                            }
                            child(value, ty, fuel)
                        })
                        .collect::<Result<_, String>>()?,
                }
            }
            Term::OptionalValue { ty: domain, value } if domain.as_ref() == ty => {
                let Term::Optional(payload) = ty else {
                    return Err("frozen optional requires an optional domain".into());
                };
                match value {
                    Some(value) => K::OptionalPresent(Box::new(child(value, payload, fuel)?)),
                    None => K::OptionalAbsent,
                }
            }
            _ => return Err("staged frozen value requires supported immutable data".into()),
        }
    };
    Ok(Expr {
        source,
        ty: ty.clone(),
        kind,
    })
}
