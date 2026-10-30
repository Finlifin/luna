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
