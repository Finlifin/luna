//! Read a local record field without copying unrelated builder state.
use super::*;

impl Interpreter<'_> {
    pub(super) fn eval_local_field(
        &mut self,
        expr: &Expr,
        environment: &Environment,
    ) -> Option<Result<Flow, Error>> {
        if !self.comptime_only || self.cleanup_schedule.is_some() {
            return None;
        }
        let (base, index, nominal, name) = match &expr.kind {
            K::Field {
                value,
                declaration,
                index,
            } => (value, *index, Some(*declaration), None),
            K::AnonymousField { value, name, index } => (value, *index, None, Some(name)),
            _ => return None,
        };
        let K::Local(binding) = base.kind else {
            return None;
        };
        Some((|| {
            // No user code runs while this local or its addressable storage is
            // borrowed. Preserve the ordinary local-read tick and lock errors.
            self.tick(Some(base.source))?;
            let local = environment
                .get(&binding)
                .ok_or_else(|| self.failure(ErrorKind::MissingBinding, Some(base.source)))?;
            self.read_record_field(
                local,
                nominal,
                name.map(String::as_str),
                index,
                &expr.ty,
                base.source,
                expr.source,
            )
            .map(Flow::Value)
        })())
    }

    pub(super) fn read_record_field(
        &mut self,
        local: &Value,
        nominal: Option<DeclarationId>,
        name: Option<&str>,
        index: usize,
        ty: &Term,
        base_source: SourceRange,
        source: SourceRange,
    ) -> Result<Value, Error> {
        let guard = if let Value::Storage(storage) = local {
            Some(
                storage
                    .0
                    .try_lock()
                    .map_err(|_| self.failure(ErrorKind::InvalidPointer, Some(base_source)))?,
            )
        } else {
            None
        };
        let referenced = if let Value::ComptimeReference(reference) = local {
            Some(
                reference
                    .borrow(&mut self.budget.steps)
                    .map_err(|kind| self.failure(kind, Some(base_source)))?,
            )
        } else {
            None
        };
        let record = referenced
            .as_ref()
            .map(|value| value.value())
            .unwrap_or_else(|| guard.as_deref().unwrap_or(local));
        let selected = match (record, nominal, name) {
            (
                Value::Record {
                    declaration,
                    fields,
                    ..
                },
                Some(expected),
                None,
            ) if *declaration == expected => fields.get(index),
            (Value::AnonymousRecord(fields), None, Some(expected)) => fields
                .get(index)
                .and_then(|(actual, value)| (actual == expected).then_some(value)),
            _ => None,
        }
        .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
        let value = self.checkpoint_representation(selected, base_source)?;
        if name.is_some() && !self.has_type(&value, ty) {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests;
