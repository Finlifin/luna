//! Fold only completed error constructors and representation transports. No
//! producer invocation or cleanup is performed by this constant-data boundary.
use super::*;
use nir::error_results::{Conversion as C, RepresentationTransport as Plan};

#[inline(never)]
pub(super) fn fold(
    heads: Layouts<'_>,
    expr: &Expr,
    fuel: &mut usize,
    depth: usize,
) -> Result<Expr, String> {
    if depth >= 128 {
        return Err("staged error depth exceeded".into());
    }
    charge(fuel, 1)?;
    let K::ErrorConvert { conversion, value } = &expr.kind else {
        unreachable!()
    };
    let value = fold_input(heads, value, fuel, depth + 1)?;
    let prepared = Expr {
        ty: expr.ty.clone(),
        source: expr.source,
        kind: K::ErrorConvert {
            conversion: *conversion,
            value: Box::new(value),
        },
    };
    // Validate the complete typed constant before reconstructing its syntax.
    constant(heads, &prepared, fuel, depth)?;
    if matches!(conversion, C::Success | C::Failure) {
        return Ok(prepared);
    }
    let K::ErrorConvert { value, .. } = prepared.kind else {
        unreachable!()
    };
    charge(fuel, heads.len())?;
    let tuples: BTreeSet<_> = heads
        .iter()
        .filter(|head| head.role == nir::normal_forms::Role::Tuple)
        .map(|head| head.declaration)
        .collect();
    let plan = nir::error_results::transport_plan(
        &value.ty,
        &expr.ty,
        *conversion == C::Product,
        fuel,
        &mut |a, b| a == b,
        &|head| tuples.contains(&head),
    )
    .map_err(|error| error.to_string())?;
    let mut result = transport(heads, &plan, *value, &expr.ty, fuel, depth)?;
    result.source = expr.source;
    Ok(result)
}

#[inline(never)]
fn transport(
    heads: Layouts<'_>,
    plan: &Plan,
    value: Expr,
    target: &Term,
    fuel: &mut usize,
    depth: usize,
) -> Result<Expr, String> {
    if depth >= 128 {
        return Err("staged error transport depth exceeded".into());
    }
    charge(fuel, 1)?;
    let source = value.source;
    let kind = match (plan, value.kind) {
        (Plan::Identity, kind) if value.ty == *target => kind,
        (
            Plan::Error {
                source: expected,
                target: output,
                ..
            },
            K::ErrorConvert {
                conversion,
                value: payload,
            },
        ) if value.ty == *expected
            && output == target
            && matches!(conversion, C::Success | C::Failure) =>
        {
            K::ErrorConvert {
                conversion,
                value: payload,
            }
        }
        (Plan::Tuple(plans), K::Tuple(values)) if plans.len() == values.len() => {
            let types = match target {
                Term::Tuple(types) => Some(types.as_slice()),
                Term::NormalForm { head, .. } => {
                    charge(fuel, heads.len())?;
                    heads
                        .iter()
                        .find(|metadata| metadata.declaration == *head)
                        .and_then(|metadata| metadata.tuple_elements(target))
                }
                _ => None,
            }
            .ok_or("staged error transport requires a trusted tuple layout")?;
            if types.len() != plans.len() {
                return Err("staged tuple transport arity mismatch".into());
            }
            let mut result = Vec::new();
            for ((plan, value), ty) in plans.iter().zip(values).zip(types) {
                result.push(transport(heads, plan, value, ty, fuel, depth + 1)?);
            }
            K::Tuple(result)
        }
        (Plan::Record(plans), K::AnonymousRecord { fields }) if plans.len() == fields.len() => {
            let Term::Record(types) = target else {
                return Err("staged record transport type mismatch".into());
            };
            if types.len() != plans.len() {
                return Err("staged record transport arity mismatch".into());
            }
            let mut result = Vec::new();
            for (((name, plan), value), (expected, ty)) in plans.iter().zip(fields).zip(types) {
                charge(fuel, name.len())?;
                if name != expected {
                    return Err("staged record transport field mismatch".into());
                }
                result.push(transport(heads, plan, value, ty, fuel, depth + 1)?);
            }
            K::AnonymousRecord { fields: result }
        }
        _ => return Err("staged error transport requires canonical constructors".into()),
    };
    Ok(Expr {
        kind,
        ty: target.clone(),
        source,
    })
}

pub(super) fn payload<'a>(
    value: &'a Expr,
    label: Option<&Term>,
    fuel: &mut usize,
) -> Result<Option<&'a Expr>, String> {
    charge(fuel, 1)?;
    let Term::ErrorResult { errors, .. } = &value.ty else {
        return Err("staged error pattern domain mismatch".into());
    };
    if let Some(label) = label {
        nir::type_computations::charge_term(label, fuel).map_err(|error| error.to_string())?;
        nir::type_computations::charge_term(&value.ty, fuel).map_err(|error| error.to_string())?;
        if !errors.contains(label) {
            return Err("staged error pattern label is absent".into());
        }
    }
    let K::ErrorConvert {
        conversion,
        value: inner,
    } = &value.kind
    else {
        return Err("staged error pattern requires a canonical result".into());
    };
    match conversion {
        C::Success => Ok(label.is_none().then_some(inner)),
        C::Failure => Ok((label == Some(&inner.ty)).then_some(inner)),
        _ => Err("staged error pattern requires completed transport".into()),
    }
}
