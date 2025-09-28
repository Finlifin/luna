use crate::hir::{Expr, Hir, HirId, SExpr};

pub struct Candidate<'hir> {
    pub hir_id: HirId,
    pub ty: SExpr<'hir>,
}

/// For example,
/// given a type `Tuple<Vec<Tuple<i32, i32>>, Tuple<i32, i32>>`
/// returns a sequence of types:
/// Tuple<Any, Any>
/// Tuple<Vec<Any>, Tuple<Any, Any>>
/// Tuple<Vec<Tuple<Any, Any>>, Tuple<i32, i32>>
/// Tuple<Vec<Tuple<i32, i32>>, Tuple<i32, i32>>
/// the precision increases with each layer.
pub fn layer_sequence<'hir>(hir: &'hir Hir, ty: SExpr<'hir>) -> Vec<SExpr<'hir>> {
    let depth = max_depth(&ty);
    let mut result = Vec::new();

    for target_depth in 1..=depth {
        let generalized = generalize_beyond_depth(hir, &ty, 0, target_depth);
        result.push(hir.intern_expr(generalized));
    }

    result
}

// 计算类型的最大嵌套深度
fn max_depth(expr: &Expr) -> usize {
    match expr {
        Expr::TyTuple(exprs) => {
            if exprs.is_empty() {
                1
            } else {
                1 + exprs.iter().map(max_depth).max().unwrap_or(0)
            }
        }
        Expr::TyOptional(inner) | Expr::TyPointer(inner) => 1 + max_depth(inner),
        Expr::TyArray(inner, _) => 1 + max_depth(inner),
        _ => 1,
    }
}

// 在指定深度之后将所有类型替换为 TyAny
fn generalize_beyond_depth<'h>(
    hir: &'h Hir,
    expr: &Expr<'h>,
    current_depth: usize,
    target_depth: usize,
) -> Expr<'h> {
    if current_depth >= target_depth {
        return Expr::TyAny;
    }

    match expr {
        Expr::TyTuple(exprs) => {
            let new_exprs: Vec<Expr<'h>> = exprs
                .iter()
                .map(|e| generalize_beyond_depth(hir, e, current_depth + 1, target_depth))
                .collect();
            Expr::TyTuple(hir.intern_exprs(new_exprs))
        }
        Expr::TyOptional(inner) => {
            let new_inner = generalize_beyond_depth(hir, inner, current_depth + 1, target_depth);
            Expr::TyOptional(hir.intern_expr(new_inner))
        }
        Expr::TyPointer(inner) => {
            let new_inner = generalize_beyond_depth(hir, inner, current_depth + 1, target_depth);
            Expr::TyPointer(hir.intern_expr(new_inner))
        }
        Expr::TyArray(inner, size) => {
            let new_inner = generalize_beyond_depth(hir, inner, current_depth + 1, target_depth);
            Expr::TyArray(hir.intern_expr(new_inner), *size)
        }
        other => other.clone(),
    }
}

#[test]
pub fn test_layer_sequence() {
    use super::Expr::*;
    use super::*;
    let hir = Hir::new();
    // Tuple<Tuple<i32, i32>>
    // expected:
    // Tuple<Any>
    // Tuple<Tuple<Any, Any>>
    // Tuple<Tuple<i32, i32>>
    let ty = hir.intern_expr(TyTuple(hir.intern_exprs(vec![TyTuple(
        hir.intern_exprs(vec![TyInt(32, true), TyInt(32, true)]),
    )])));

    let layers = layer_sequence(&hir, ty);
    dbg!(&layers);

    assert_eq!(layers.len(), 3);
    assert_eq!(
        layers[0],
        hir.intern_expr(TyTuple(hir.intern_exprs(vec![TyAny])))
    );
    assert_eq!(
        layers[1],
        hir.intern_expr(TyTuple(
            hir.intern_exprs(vec![TyTuple(hir.intern_exprs(vec![TyAny, TyAny]))])
        ))
    );
    assert_eq!(
        layers[2],
        hir.intern_expr(TyTuple(hir.intern_exprs(vec![TyTuple(
            hir.intern_exprs(vec![TyInt(32, true), TyInt(32, true)])
        )])))
    );
}
