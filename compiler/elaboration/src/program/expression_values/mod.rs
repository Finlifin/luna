//! Small expression dispatcher. Category frames are entered only after special
//! contextual/type-value handling returns, keeping unrelated recursive paths off
//! one another's native stack. Phase and expected-type checks retain their order.
use super::*;
mod aggregates;
mod bindings;
mod literals;
mod places;
mod propagation;
mod special;
mod statements;
mod views;

impl Checker<'_, '_> {
    #[inline(never)]
    pub(super) fn expr_context_inner(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        if let Some(value) = self.special_value_expression(expr, expected, partial)? {
            return Ok(value);
        }
        match &expr.kind {
            H::LiteralConversion { .. } | H::Lit(_) | H::Null | H::Unit => {
                self.literals_value_expression(expr, expected, partial)
            }
            H::ErrorNew(_) | H::ErrorPropagate(_) | H::OptionPropagate(_) => {
                self.propagation_value_expression(expr, expected, partial)
            }
            H::SelfValue
            | H::TacticArgument(_)
            | H::PostLambdaBody(_)
            | H::TacticParameter(_)
            | H::ResolvedLocal(..)
            | H::ResolvedFnParam(..)
            | H::NFApplication(..)
            | H::ResolvedDef { .. }
            | H::Closure(..)
            | H::Item(_) => self.bindings_value_expression(expr, expected, partial),
            H::Ref(_) | H::Deref(_) | H::AssignOp(..) | H::Assign(..) => {
                self.places_value_expression(expr, expected, partial)
            }
            H::Semi(_)
            | H::For { .. }
            | H::Loop(_)
            | H::BreakTo(_)
            | H::ContinueTo(_)
            | H::Break(_)
            | H::Continue(_)
            | H::Let(_)
            | H::Defer(_)
            | H::Assumes(_)
            | H::Return(_) => self.control_value_expression(expr, expected, partial),
            H::CollectedProperties(_)
            | H::Object(_)
            | H::Tuple(_)
            | H::If(..)
            | H::InlineControl(_) => self.aggregates_value_expression(expr, expected, partial),
            H::Index(..)
            | H::Cast(..)
            | H::TakeOld(_)
            | H::TakeTupleEnumerate(_)
            | H::TakeIntrinsic(_)
            | H::TakeTagName(_)
            | H::TakeBitVec(_) => self.views_value_expression(expr, expected, partial),
            _ => {
                return Err(error(
                    expr.span,
                    "expression elaboration requires a protocol or construct not yet implemented",
                ));
            }
        }
    }

    fn finish_value_expression(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
        ty: Term,
        kind: K,
    ) -> Result<n::Expr, Error> {
        if let Some(expected) = expected {
            self.require_expression(&ty, expected, expr.span)?;
        }
        Ok(n::Expr {
            ty,
            source: source(expr.span),
            kind,
        })
    }
}
