//! Remove unreachable sequence suffixes before template dependency discovery.
//! The terminating expression is retained, including its effects and cleanup.
use super::*;

fn terminates(expression: &n::Expr, fuel: &mut usize) -> Result<bool, String> {
    let mut current = expression;
    loop {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("dead-tail analysis budget exhausted")?;
        if current.ty == Term::Primitive(nir::Primitive::Never) {
            return Ok(true);
        }
        match &current.kind {
            K::Return(_) | K::Break(_) | K::Continue(_) => return Ok(true),
            K::Discard(inner) => current = inner,
            _ => return Ok(false),
        }
    }
}

pub(super) fn prune(expression: &mut n::Expr, fuel: &mut usize) -> Result<(), String> {
    iir::visit::expression_mut_pruned(expression, fuel, |expr, fuel| {
        if let K::Block { statements, result } | K::CallPreparation { statements, result } =
            &mut expr.kind
        {
            for (index, statement) in statements.iter().enumerate() {
                if terminates(statement, fuel)? {
                    statements.truncate(index + 1);
                    *result = None;
                    expr.ty = Term::Primitive(nir::Primitive::Never);
                    break;
                }
            }
        }
        Ok(true)
    })
}
