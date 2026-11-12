//! Leaf operands contain no user computation and can be read under a borrow.
use super::*;
pub(super) enum LocalSelector<'a> {
    Value(Option<&'a Value>, SourceRange),
    Literal {
        value: &'a Literal,
        ty: &'a Term,
        source: SourceRange,
    },
}

impl Interpreter<'_> {
    pub(super) fn compiled_leaf_selector<'s>(
        &self,
        selector: &'s comptime_compiler::leaf_inputs::Selector,
        code: &'s comptime_compiler::Program,
        slots: &'s [Option<Value>],
    ) -> Result<LocalSelector<'s>, Error> {
        use comptime_compiler::{constants::Kind, leaf_inputs::Selector};
        match selector {
            Selector::Local(slot, source) => Ok(LocalSelector::Value(
                slots.get(*slot).and_then(Option::as_ref),
                *source,
            )),
            Selector::Literal(id, source) => {
                let constant = code
                    .constants()
                    .get(id.0)
                    .ok_or_else(|| self.failure(ErrorKind::InvalidControlFlow, Some(*source)))?;
                let Kind::Literal(value) = &constant.kind else {
                    return Err(self.failure(ErrorKind::InvalidControlFlow, Some(*source)));
                };
                Ok(LocalSelector::Literal {
                    value,
                    ty: &constant.ty,
                    source: *source,
                })
            }
        }
    }

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
            LocalSelector::Literal { value, ty, source } => {
                self.tick(Some(source))?;
                self.literal_constant(value, ty, source).map(Some)
            }
        }
    }
}
