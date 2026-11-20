//! Fused writes are allowed only when suffix evaluation cannot mutate the target.
use super::*;

impl Interpreter<'_> {
    pub(super) fn check_list_append(
        &mut self,
        target: &Value,
        ty: &Term,
        source: SourceRange,
    ) -> Result<Term, Error> {
        let (ty, _) = self.prepare_list_operation(ty, ty, Some(ty), source)?;
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
        self.list_operand_length(value, &ty, source)?;
        Ok(ty)
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
        self.query_descriptions(Some(source), |vm, fuel| {
            let check = |combined: &Value, moved: usize, fuel: &mut usize| {
                type_descriptions::spend(fuel, moved.saturating_add(1))?;
                combined
                    .inspect_representation(
                        &vm.abstract_type_parameters,
                        &vm.abstract_value_parameters,
                        vm.runtime_domains(),
                        fuel,
                    )
                    .map(|_| ())
            };
            if let Some(reference) = reference {
                reference.append_list_checked(suffix, fuel, check)
            } else {
                memory::append_list_checked(target, suffix, |value, moved| {
                    check(value, moved, fuel)
                })
            }
        })
    }
}
