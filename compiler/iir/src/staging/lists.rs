//! Fold only completed immutable list data, retaining its nominal head.
use super::*;

pub(super) fn fold(
    heads: Layouts<'_>,
    expr: &Expr,
    fuel: &mut usize,
    depth: usize,
) -> Result<Expr, String> {
    if depth >= 128 {
        return Err("staged list depth exceeded".into());
    }
    charge(fuel, 1)?;
    let (left, right) = match &expr.kind {
        K::ListLength(left) => (left, None),
        K::ListConcat(left, right) => (left, Some(right)),
        _ => unreachable!(),
    };
    if heads.list_element(&left.ty, fuel)?.is_none()
        || right.is_some_and(|right| right.ty != left.ty)
        || expr.ty
            != if right.is_some() {
                left.ty.clone()
            } else {
                Term::Primitive(Primitive::Integer)
            }
    {
        return Err("invalid staged list operation types".into());
    }
    let mut result = fold_input(heads, left, fuel, depth + 1)?;
    constant(heads, &result, fuel, depth + 1)?;
    let length = match &result.kind {
        K::TypeListValue(values) => values.len(),
        K::TypeList(values) => values.len(),
        _ => return Err("staged list operation requires completed list data".into()),
    };
    if let Some(right) = right {
        let other = fold_input(heads, right, fuel, depth + 1)?;
        constant(heads, &other, fuel, depth + 1)?;
        let count = match &other.kind {
            K::TypeListValue(values) => values.len(),
            K::TypeList(values) => values.len(),
            _ => return Err("staged list operation requires completed list data".into()),
        };
        let count = length
            .checked_add(count)
            .filter(|count| *count <= 100_000)
            .ok_or("staged list representation size exceeded")?;
        charge(fuel, count.saturating_add(1))?;
        match (&mut result.kind, other.kind) {
            (K::TypeListValue(left), K::TypeListValue(right)) => left.extend(right),
            (K::TypeList(left), K::TypeList(right)) => left.extend(right),
            _ => return Err("staged list representation mismatch".into()),
        }
        result.source = expr.source;
        Ok(result)
    } else {
        Ok(Expr {
            source: expr.source,
            ty: expr.ty.clone(),
            kind: K::Literal(Literal::Integer(length.to_string())),
        })
    }
}
