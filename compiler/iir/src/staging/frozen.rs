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
    let kind = if *ty == Term::Primitive(Primitive::Type) && value.sort() == Sort::Type {
        K::TypeValue(value.clone())
    } else {
        match value {
            Term::Bool(value) => K::Literal(Literal::Bool(*value)),
            Term::Char(value) => K::Literal(Literal::Char(*value)),
            Term::Symbol(value) => K::Literal(Literal::Symbol(value.to_string())),
            Term::String(value) => K::Literal(Literal::String(value.to_string())),
            Term::Unit => K::Unit,
            Term::Float16(value) => K::Literal(Literal::Float16(*value)),
            Term::BFloat16(value) => K::Literal(Literal::BFloat16(*value)),
            Term::Float32(value) => K::Literal(Literal::Float32(*value)),
            Term::Float64(value) => K::Literal(Literal::Float64(*value)),
            Term::BitVec(value) => K::Literal(Literal::BitVec {
                width: value.width(),
                words: value.words().to_vec(),
            }),
            Term::Float(value) => {
                let mut budget = comptime_runtime::Budget {
                    steps: *fuel,
                    max_limbs: 4096,
                };
                let spelling = value.to_literal(&mut budget);
                *fuel = budget.steps;
                K::Literal(Literal::Float(spelling.map_err(|e| e.to_string())?))
            }
            Term::Integer(value) => {
                if *ty == Term::Primitive(Primitive::Integer) {
                    let mut budget = comptime_runtime::Budget {
                        steps: *fuel,
                        max_limbs: 4096,
                    };
                    let spelling = value.to_hex_literal(&mut budget);
                    *fuel = budget.steps;
                    K::Literal(Literal::Integer(spelling.map_err(|e| e.to_string())?))
                } else {
                    let Term::Primitive(primitive) = ty else {
                        return Err("frozen integer requires an integer domain".into());
                    };
                    let (signed, width) = heads
                        .integer_layout(*primitive)
                        .ok_or("frozen integer requires a target-resolved integer domain")?;
                    if signed {
                        K::Literal(Literal::SignedInteger(
                            value.to_signed(width).map_err(|e| e.to_string())?,
                        ))
                    } else {
                        K::Literal(Literal::UnsignedInteger(
                            value.to_unsigned(width).map_err(|e| e.to_string())?,
                        ))
                    }
                }
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
