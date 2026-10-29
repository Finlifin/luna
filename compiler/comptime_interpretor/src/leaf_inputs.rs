//! Leaf operands contain no user computation and can be read under a borrow.
use super::*;
pub(super) enum LocalSelector<'a> {
    Value(Option<&'a Value>, SourceRange),
    Literal(&'a Expr),
}

impl Interpreter<'_> {
    pub(super) fn read_leaf_selector(
        &mut self,
        selector: Option<LocalSelector<'_>>,
    ) -> Result<Option<Value>, Error> {
        let Some(selector) = selector else {
            return Ok(None);
        };
        match selector {
            LocalSelector::Value(value, source) => {
                self.tick(Some(source))?;
                let value =
                    value.ok_or_else(|| self.failure(ErrorKind::MissingBinding, Some(source)))?;
                self.invocation_slot_value(value, Some(source)).map(Some)
            }
            LocalSelector::Literal(expr) => {
                self.tick(Some(expr.source))?;
                let Flow::Value(value) = self.eval_literal(expr, &mut Environment::new())? else {
                    return Err(self.failure(ErrorKind::InvalidControlFlow, Some(expr.source)));
                };
                Ok(Some(value))
            }
        }
    }
}
