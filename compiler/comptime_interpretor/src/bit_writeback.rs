//! Commit a checked bit replacement against the storage after RHS evaluation.
use super::*;

impl Interpreter<'_> {
    pub(super) fn write_bit_place(
        &mut self,
        root: &mut Value,
        fields: &[(nir::program::FieldOwner, usize)],
        storage_type: &Term,
        start: usize,
        end: usize,
        rhs: comptime_runtime::bitvec::BitVec,
        output: &Term,
        source: SourceRange,
    ) -> Result<(), Error> {
        if start > end || end - start != rhs.width() || *output != Term::Primitive(Primitive::Void)
        {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        if let Value::ComptimeReference(reference) = root {
            let reference = reference
                .project(fields, &mut self.budget.steps)
                .map_err(|kind| self.failure(kind, Some(source)))?;
            // Read after RHS evaluation, through the same checked projection
            // used by ordinary receiver assignments. Release the guard before
            // committing; failed construction leaves completed RHS writes intact.
            let changed = {
                let contents = reference
                    .borrow(&mut self.budget.steps)
                    .map_err(|kind| self.failure(kind, Some(source)))?;
                self.replaced_bit_value(source, storage_type, contents.value(), start, &rhs)?
            };
            return reference
                .replace(changed, &mut self.budget.steps)
                .map_err(|kind| self.failure(kind, Some(source)));
        }
        let storage = match &*root {
            Value::Storage(storage) => Some(storage.clone()),
            _ => None,
        };
        let mut locked = storage
            .as_ref()
            .map(|storage| {
                storage
                    .0
                    .lock()
                    .map_err(|_| self.failure(ErrorKind::InvalidPointer, Some(source)))
            })
            .transpose()?;
        let mut target = match &mut locked {
            Some(value) => &mut **value,
            None => root,
        };
        for (expected, index) in fields {
            target = match (expected, target) {
                (
                    nir::program::FieldOwner::Nominal(expected),
                    Value::Record {
                        declaration,
                        fields,
                        ..
                    },
                ) => {
                    if declaration == expected {
                        fields.get_mut(*index)
                    } else {
                        None
                    }
                }
                (nir::program::FieldOwner::Anonymous, Value::AnonymousRecord(fields)) => {
                    fields.get_mut(*index).map(|(_, value)| value)
                }
                _ => None,
            }
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
        }
        let changed = self.replaced_bit_value(source, storage_type, target, start, &rhs)?;
        // Commit only after all checks and bounded construction succeed.
        *target = changed;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receiver_bit_updates_check_permissions_paths_and_budget_before_commit() {
        let package = Package::default();
        let source = SourceRange { start: 1, end: 2 };
        for case in 0..4 {
            let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
            let bits = comptime_runtime::bitvec::BitVec::from_words(
                4,
                &[5],
                &mut comptime_runtime::Budget::default(),
            )
            .unwrap();
            let reference = ValueReference::owned(Value::UnsignedInteger(0xa0));
            let mut restricted = reference.clone();
            if case == 0 {
                restricted.prohibit_rebind();
            }
            let fields = if case == 1 {
                vec![(nir::program::FieldOwner::Anonymous, 0)]
            } else {
                vec![]
            };
            if case == 2 {
                vm.budget.steps = 0;
            }
            let result = vm.write_bit_place(
                &mut Value::ComptimeReference(restricted),
                &fields,
                &Term::Primitive(Primitive::U8),
                0,
                4,
                bits,
                &Term::Primitive(Primitive::Void),
                source,
            );
            assert_eq!(result.is_ok(), case == 3);
            let observed = reference.borrow(&mut 100).unwrap();
            assert_eq!(
                observed.value(),
                &Value::UnsignedInteger(if case == 3 { 0xa5 } else { 0xa0 })
            );
        }
    }
}
