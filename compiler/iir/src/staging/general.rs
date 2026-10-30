//! General scalar/block folding with bounded recursive frame sizes.
use super::*;

#[inline(never)]
pub(super) fn fold_general_input(
    heads: Layouts<'_>,
    expr: &Expr,
    fuel: &mut usize,
    depth: usize,
) -> Result<Expr, String> {
    if depth >= 128 {
        return Err("staged expression depth exceeded".into());
    }
    charge(fuel, 1)?;
    let mut result = fold_kind(heads, expr, fuel, depth)?;
    if result.ty != expr.ty {
        return Err("staged selected value type differs from expression type".into());
    }
    constant(heads, &result, fuel, depth)?;
    result.source = expr.source;
    Ok(result)
}

// Dispatch forwards the return slot; it does not retain aggregate temporaries
// for every other branch while recursively evaluating the selected branch.
#[inline(never)]
fn fold_kind(
    heads: Layouts<'_>,
    expr: &Expr,
    fuel: &mut usize,
    depth: usize,
) -> Result<Expr, String> {
    match &expr.kind {
        K::SymbolName(value) => symbol_name(heads, expr, value, fuel, depth),
        K::Discard(value) => discard(heads, expr, value, fuel, depth),
        K::AnonymousField { .. } | K::Field { .. } => fold_record_field(heads, expr, fuel, depth),
        K::IntegerBinary { op, .. } if op.is_comparison() => {
            fold_integer_comparison(heads, expr, fuel, depth)
        }
        K::IntegerNegate(_) | K::IntegerBinary { .. } => {
            fold_machine_input(heads, expr, fuel, depth)
        }
        K::Block { statements, result } => {
            block(heads, expr, statements, result.as_deref(), fuel, depth)
        }
        _ => {
            constant(heads, expr, fuel, depth)?;
            Ok(expr.clone())
        }
    }
}

#[inline(never)]
fn symbol_name(
    heads: Layouts<'_>,
    expr: &Expr,
    value: &Expr,
    fuel: &mut usize,
    depth: usize,
) -> Result<Expr, String> {
    if value.ty != Term::Primitive(Primitive::Symbol) || expr.ty != Term::Primitive(Primitive::Str)
    {
        return Err("invalid Symbol tag_name types".into());
    }
    let value = fold_input(heads, value, fuel, depth + 1)?;
    let Term::Symbol(name) = constant(heads, &value, fuel, depth + 1)? else {
        return Err("tag_name requires a completed Symbol".into());
    };
    charge(fuel, name.len())?;
    Ok(Expr {
        source: expr.source,
        ty: expr.ty.clone(),
        kind: K::Literal(Literal::String(name.to_string())),
    })
}

#[inline(never)]
fn discard(
    heads: Layouts<'_>,
    expr: &Expr,
    value: &Expr,
    fuel: &mut usize,
    depth: usize,
) -> Result<Expr, String> {
    // Discard the result, not the evaluation or its failures.
    fold_input(heads, value, fuel, depth + 1)?;
    Ok(Expr {
        kind: K::Unit,
        ty: Term::Primitive(Primitive::Void),
        source: expr.source,
    })
}

#[inline(never)]
fn block(
    heads: Layouts<'_>,
    expr: &Expr,
    statements: &[Expr],
    result: Option<&Expr>,
    fuel: &mut usize,
    depth: usize,
) -> Result<Expr, String> {
    let mut locals = std::collections::BTreeMap::new();
    for statement in statements {
        if let K::Let { binding, value } = &statement.kind {
            let value = fold_local_input(heads, value, &locals, fuel, depth + 1)?;
            if locals.insert(*binding, value).is_some() {
                return Err("duplicate staged block binding".into());
            }
        } else if let K::LetPattern {
            pattern,
            value,
            guard: None,
            otherwise: None,
        } = &statement.kind
        {
            let value = fold_local_input(heads, value, &locals, fuel, depth + 1)?;
            bind_staged_pattern(heads, pattern, &value, &mut locals, fuel, depth + 1)?;
        } else {
            fold_local_input(heads, statement, &locals, fuel, depth + 1)?;
        }
    }
    if let Some(result) = result {
        fold_local_input(heads, result, &locals, fuel, depth + 1)
    } else {
        Ok(Expr {
            ty: Term::Primitive(Primitive::Void),
            source: expr.source,
            kind: K::Unit,
        })
    }
}
