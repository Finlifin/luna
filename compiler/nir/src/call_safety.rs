//! Lexical admission of responsibility-bearing calls. This is independent of
//! phase, purity, resource legality and the callee's program contract.
use crate::program::{ExprKind as K, Function};

pub fn validate(function: &Function, fuel: &mut usize) -> Result<(), String> {
    if function.signature.is_unsafe_callable() {
        return Err(
            "function declaration requires an unqualified parameter/result telescope".into(),
        );
    }
    let mut pending: Vec<_> = std::iter::once(&function.body)
        .chain(&function.preconditions)
        .chain(&function.postconditions)
        .chain(function.defaults.iter().map(|(_, value)| value))
        .map(|expr| (expr, function.is_unsafe))
        .collect();
    while let Some((root, permitted)) = pending.pop() {
        crate::visit::expression_pruned(root, fuel, |expr, _| {
            if let K::UnsafeScope(body) = &expr.kind {
                pending.push((body, true));
                return Ok(false);
            }
            if matches!(expr.kind, K::Intrinsic { .. }) && !permitted {
                return Err(format!(
                    "intrinsic requires an unsafe boundary at {}..{}",
                    expr.source.start, expr.source.end
                ));
            }
            if let K::Call { callee, .. } = &expr.kind {
                if callee.ty.is_unsafe_callable() && !permitted {
                    return Err(format!(
                        "unsafe function call requires an unsafe boundary at {}..{}",
                        expr.source.start, expr.source.end
                    ));
                }
            }
            Ok(true)
        })?;
    }
    Ok(())
}
