//! Checked initialization programs, distinct from completed associated values.
//! Formation, protocol evidence and full phase checking remain caller duties.
use crate::{
    DeclarationParameter, Term,
    program::{Expr, ExprKind as K},
};
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub struct AssociatedInitializer {
    expression: Expr,
}
impl AssociatedInitializer {
    pub fn new(expression: Expr, fuel: &mut usize) -> Result<Self, String> {
        if !crate::free_bindings::collect(&expression, fuel)?.is_empty() {
            return Err("associated initializer cannot capture caller locals".into());
        }
        check_calls(&expression, fuel, 0)?;
        Ok(Self { expression })
    }
    pub fn expression(&self) -> &Expr {
        &self.expression
    }
    pub fn into_expression(self) -> Expr {
        self.expression
    }

    /// Substitution preserves a program; it does not certify a completed value.
    /// The caller must establish declaration argument domains before invocation.
    pub fn instantiate(
        &self,
        parameters: &[DeclarationParameter],
        arguments: &[Term],
        fuel: &mut usize,
    ) -> Result<Self, String> {
        crate::copy_budget::charge_substitution(&self.expression, parameters, arguments, fuel)
            .map_err(|_| "associated initializer substitution budget exhausted")?;
        let expression = self
            .expression
            .instantiate_parameters(parameters, arguments)
            .map_err(|e| e.to_string())?;
        Self::new(expression, fuel)
    }
}
fn check_calls(expression: &Expr, fuel: &mut usize, depth: usize) -> Result<(), String> {
    if depth >= 128 {
        return Err("associated initializer nesting exhausted".into());
    }
    crate::visit::expression_pruned(expression, fuel, |node, fuel| {
        if let K::Call { callee, .. } = &node.kind {
            if !callee.ty.is_comptime_callable() {
                return Err("associated initializer cannot invoke a runtime callable".into());
            }
        }
        if let K::Closure { captures, .. } = &node.kind {
            // Constructing a closure evaluates captures, not its body. The body
            // is checked independently at its declared phase.
            for capture in captures {
                if let Some(value) = &capture.value {
                    check_calls(value, fuel, depth + 1)?;
                }
            }
            return Ok(false);
        }
        Ok(true)
    })
}
impl fmt::Display for AssociatedInitializer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(associated-initializer (result {}) (body {}))",
            self.expression.ty, self.expression
        )
    }
}
