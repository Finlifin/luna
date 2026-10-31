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
    let left = fold_input(heads, left, fuel, depth + 1)?;
    let mut fields = elements(heads, left, fuel, depth + 1)?;
    if let Some(right) = right {
        let right = fold_input(heads, right, fuel, depth + 1)?;
        let other = elements(heads, right, fuel, depth + 1)?;
        let count = fields
            .len()
            .checked_add(other.len())
            .filter(|count| *count <= 100_000)
            .ok_or("staged list representation size exceeded")?;
        charge(fuel, count.saturating_add(1))?;
        fields.extend(other);
        Ok(Expr {
            source: expr.source,
            ty: expr.ty.clone(),
            kind: K::TypeList(fields),
        })
    } else {
        Ok(Expr {
            source: expr.source,
            ty: expr.ty.clone(),
            kind: K::Literal(Literal::Integer(fields.len().to_string())),
        })
    }
}

// Read completed data through its declared domain. Frozen values are decoded
// by the caller; all operations use ordinary expressions.
pub(super) fn elements(
    heads: Layouts<'_>,
    value: Expr,
    fuel: &mut usize,
    depth: usize,
) -> Result<Vec<Expr>, String> {
    constant(heads, &value, fuel, depth)?;
    match value.kind {
        K::TypeList(fields) => Ok(fields),
        _ => Err("staged list operation requires completed list data".into()),
    }
}
