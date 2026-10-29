//! Fused writes are allowed only when suffix evaluation cannot mutate the target.
use super::*;

impl Interpreter<'_> {
    pub(super) fn check_list_append(
        &mut self,
        target: &Value,
        ty: &Term,
        source: SourceRange,
    ) -> Result<(), Error> {
        self.prepare_list_operation(ty, ty, Some(ty), source)?;
        let reference = match target {
            Value::ComptimeReference(reference) => Some(reference.clone()),
            Value::Storage(storage) => Some(
                value_reference::ValueReference::projected(
                    storage.clone(),
                    &[],
                    &mut self.budget.steps,
                )
                .map_err(|kind| self.failure(kind, Some(source)))?,
            ),
            _ => None,
        };
        let borrowed = reference
            .as_ref()
            .map(|reference| reference.borrow(&mut self.budget.steps))
            .transpose()
            .map_err(|kind| self.failure(kind, Some(source)))?;
        let value = borrowed.as_ref().map(|v| v.value()).unwrap_or(target);
        self.check_borrowed_representation(value, source, 0)?;
        self.list_operand_length(value, ty, source)?;
        Ok(())
    }

    pub(super) fn complete_list_append(
        &mut self,
        target: &mut Value,
        suffix: Value,
        ty: &Term,
        source: SourceRange,
    ) -> Result<(), Error> {
        self.list_operand_length(&suffix, ty, source)?;
        let reference = match target {
            Value::ComptimeReference(reference) => Some(reference.clone()),
            Value::Storage(storage) => Some(
                value_reference::ValueReference::projected(
                    storage.clone(),
                    &[],
                    &mut self.budget.steps,
                )
                .map_err(|kind| self.failure(kind, Some(source)))?,
            ),
            _ => None,
        };
        // Checked extension restores the original length on every validation or
        // budget failure. No user computation runs while the target is borrowed.
        if let Some(reference) = reference {
            // The traversal budget must be threaded into validation, not reset
            // from the value it held before the reference path was walked.
            let mut remaining = self.budget.steps;
            let mut checked = false;
            let result = reference.append_list_checked(suffix, &mut remaining, |combined, fuel| {
                checked = true;
                self.budget.steps = fuel;
                self.charge_appended_list(combined, source)
            });
            if !checked {
                self.budget.steps = remaining;
            }
            result.map_err(|kind| self.failure(kind, Some(source)))
        } else {
            memory::append_list_checked(target, suffix, |combined| {
                self.charge_appended_list(combined, source)
            })
            .map_err(|kind| self.failure(kind, Some(source)))
        }
    }
    fn charge_appended_list(
        &mut self,
        combined: &Value,
        source: SourceRange,
    ) -> Result<(), ErrorKind> {
        let count = self
            .list_value_length(combined, source)
            .map_err(|error| error.kind)?;
        let work = combined
            .scoped_copy_work_with_evidence(
                &self.abstract_type_parameters,
                &self.abstract_value_parameters,
            )
            .ok_or(ErrorKind::RepresentationSizeLimit)?;
        self.integer_operation(Some(source), |budget| {
            budget.consume(count.saturating_add(1))?;
            budget.consume(work)
        })
        .map_err(|error| error.kind)
    }
}
