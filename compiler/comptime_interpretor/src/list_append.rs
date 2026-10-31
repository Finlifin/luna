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
        // A closed structural summary already bounds this immutable payload.
        // Admission below still rechecks its current domain and every embedded
        // type. Only omit the redundant checkpoint walk when every node also
        // fits the current invocation depth; otherwise retain the original
        // traversal and its failure ordering. Scoped evidence is never reused.
        let summarized = self.comptime_only
            && match value {
                Value::List { values, .. } => values.summary().is_some_and(|summary| {
                    summary.height < 128
                        && self
                            .depth
                            .checked_add(summary.height)
                            .is_some_and(|depth| depth < self.budget.depth)
                }),
                _ => false,
            };
        if !summarized {
            self.check_borrowed_representation(value, source, 0)?;
        }
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
            let result =
                reference.append_list_checked(suffix, &mut remaining, |combined, moved, fuel| {
                    checked = true;
                    self.budget.steps = fuel;
                    self.charge_appended_list(combined, moved, source)
                });
            if !checked {
                self.budget.steps = remaining;
            }
            result.map_err(|kind| self.failure(kind, Some(source)))
        } else {
            memory::append_list_checked(target, suffix, |combined, moved| {
                self.charge_appended_list(combined, moved, source)
            })
            .map_err(|kind| self.failure(kind, Some(source)))
        }
    }
    fn charge_appended_list(
        &mut self,
        combined: &Value,
        moved: usize,
        source: SourceRange,
    ) -> Result<(), ErrorKind> {
        let (_, work) = combined
            .scoped_representation_work(
                &self.abstract_type_parameters,
                &self.abstract_value_parameters,
            )
            .ok_or(ErrorKind::RepresentationSizeLimit)?;
        self.integer_operation(Some(source), |budget| {
            budget.consume(moved.saturating_add(1))?;
            budget.consume(work)
        })
        .map_err(|error| error.kind)
    }
}
