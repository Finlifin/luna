//! Error rows consume ordinary comptime Type values or collected Type values.
use super::*;

impl Checker<'_, '_> {
    pub(in crate::program) fn error_type_labels(
        &mut self,
        source: &Expr<'_>,
    ) -> Result<Vec<Term>, Error> {
        // Literal rows also work before a collection protocol is installed,
        // and may contain the current signature's symbolic Type parameters.
        if let H::List(items) = &source.kind {
            return items.iter().map(|item| self.type_expr(item)).collect();
        }
        let checked = self.with_staged_input(true, |this| this.expr(source, None))?;
        if checked.ty == p(P::Type) {
            // Preserve normal-form identity: observing a Type in a row must
            // not turn a head application into invocation of its body.
            return Ok(vec![self.type_expr(source)?]);
        }
        if self.list_element_type(&checked.ty) != Some(&p(P::Type)) {
            return Err(error(
                source.span,
                "error row requires Type or a comptime list of Type values",
            ));
        }
        let domain = checked.ty.clone();
        let (value, _) = self.required_normal_input(source, checked, Vec::new())?;
        let comptime_interpretor::Value::List { ty, values } = value else {
            return Err(error(
                source.span,
                "error row did not produce a comptime list",
            ));
        };
        if ty != domain {
            return Err(error(
                source.span,
                "error row list domain changed during invocation",
            ));
        }
        self.required_steps = self
            .required_steps
            .checked_sub(values.len() as u64)
            .ok_or_else(|| error(source.span, "error row label budget exhausted"))?;
        values
            .into_iter()
            .map(|value| match value {
                comptime_interpretor::Value::Type(ty) if ty.sort() == Sort::Type => Ok(ty),
                _ => Err(error(
                    source.span,
                    "error row list contains a non-Type value",
                )),
            })
            .collect()
    }
}
