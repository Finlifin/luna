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

/// Specialization embeds staged Type arguments in instance identities. Their
/// preparation aliases can become dead only after the residual call is built.
/// Forwarding callable aliases also leaves void Unit placeholders to remove.
/// Do not erase general values: they may still have cleanup responsibilities.
pub(super) fn prune_type_preparations(
    expression: &mut n::Expr,
    fuel: &mut usize,
) -> Result<(), String> {
    iir::visit::expression_mut_pruned(expression, fuel, |expr, fuel| {
        let K::CallPreparation { statements, result } = &mut expr.kind else {
            return Ok(true);
        };
        let mut live = match result {
            Some(result) => nir::preparation::references(result, fuel)?,
            None => BTreeSet::new(),
        };
        let mut remove = Vec::new();
        for (index, statement) in statements.iter().enumerate().rev() {
            *fuel = fuel
                .checked_sub(1)
                .ok_or("type preparation pruning budget exhausted")?;
            // Forwarded staged callable aliases leave a void placeholder.
            // It has no runtime evaluation or ownership responsibility, and
            // cannot remain in a preparation's binding-only statement list.
            if statement.ty == Term::Primitive(nir::Primitive::Void)
                && matches!(statement.kind, K::Unit)
            {
                remove.push(index);
                continue;
            }
            if let K::Let { binding, value } = &statement.kind {
                if !live.contains(binding)
                    && value.ty == Term::Primitive(nir::Primitive::Type)
                    && matches!(value.kind, K::TypeValue(_))
                {
                    remove.push(index);
                    continue;
                }
            }
            live.extend(nir::preparation::references(statement, fuel)?);
        }
        for index in remove {
            statements.remove(index);
        }
        Ok(true)
    })
}
