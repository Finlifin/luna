//! Conservative normal-completion checks for structured control flow.
use super::*;

/// Definition-side counterpart of `boolean`: inspect only literal Boolean
/// trees, without interpreting staged inputs or invoking source computations.
pub(super) fn source_boolean(expr: &Expr<'_>, fuel: &mut u64, depth: usize) -> Option<bool> {
    *fuel = fuel.checked_sub(1)?;
    if depth >= 64 {
        return None;
    }
    match &expr.kind {
        H::Lit(literal) => match literal.kind {
            hir::common::LitKind::Bool(value) => Some(value),
            _ => None,
        },
        H::Unary(UnOp::Not, value) => Some(!source_boolean(value, fuel, depth + 1)?),
        H::Binary(BinOp::And, left, right) => {
            Some(source_boolean(left, fuel, depth + 1)? && source_boolean(right, fuel, depth + 1)?)
        }
        H::Binary(BinOp::Or, left, right) => {
            Some(source_boolean(left, fuel, depth + 1)? || source_boolean(right, fuel, depth + 1)?)
        }
        _ => None,
    }
}

pub(super) use crate::verification::control::{
    boolean, conditional_never, freshen_loop_sites, loop_may_complete, loop_may_continue,
    pattern_matches,
};
