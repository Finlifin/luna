//! Bit-vector logical operations evaluate both operands in the caller's store.
use super::flow::Returns;
use super::*;

impl Checker<'_, '_> {
    pub(super) fn observe_bit_operands(
        &mut self,
        operator: &K,
        left: &n::Expr,
        right: Option<&n::Expr>,
        scope: &mut ObservationScope,
        returns: &mut Option<Returns>,
        span: Span,
        depth: usize,
    ) -> Result<Completion, Error> {
        let step = self.observe_factory_expr(left, scope, returns, span, depth)?;
        let Completion::Next(left) = step else {
            return Ok(step);
        };
        let right = if let Some(expr) = right {
            let step = self.observe_factory_expr(expr, scope, returns, span, depth)?;
            let Completion::Next(value) = step else {
                return Ok(step);
            };
            Some(value)
        } else {
            None
        };
        let Some(Term::BitVec(left)) = left.known_term() else {
            return Ok(Completion::Next(PartialStagedValue::unknown()));
        };
        self.schema_fuel(span, |_, fuel| {
            let mut budget = comptime_runtime::Budget {
                steps: *fuel,
                max_limbs: 4096,
            };
            let calculated = match right.as_deref() {
                None => left.not(&mut budget).map(Term::BitVec).map_err(Into::into),
                Some(value) => match value.known_term() {
                    Some(Term::BitVec(right)) => {
                        normal::known_scalars::binary_bits(operator, left, right, &mut budget)
                    }
                    _ => return Ok(Completion::Next(PartialStagedValue::unknown())),
                },
            };
            *fuel = budget.steps;
            let known =
                normal::observation::hint(calculated).map_err(|reason| reason.to_string())?;
            Ok(Completion::Next(known.map_or_else(
                PartialStagedValue::unknown,
                PartialStagedValue::known,
            )))
        })
    }
}
