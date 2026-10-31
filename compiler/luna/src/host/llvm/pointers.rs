use super::*;

pub(super) fn pointee(ty: &Term) -> Result<&Term, String> {
    match ty {
        Term::Pointer(pointee) => Ok(pointee),
        _ => Err("host memory access lacks a logical pointer type".into()),
    }
}

impl Emitter<'_> {
    pub(super) fn pointer_slot(&self, pointer: &Value, pointee: &Term) -> Result<Slot, String> {
        if !self.checked {
            return Err("host data pointers require paired Copy/Drop resource verification".into());
        }
        let Ty::Pointer = &pointer.ty else {
            return Err("host memory access requires a data pointer".into());
        };
        Ok(Slot {
            ty: repr(self.graph, pointee, self.checked)?,
            pointer: pointer.text.clone(),
        })
    }
    pub(super) fn address(&mut self, place: &Expr, depth: usize) -> Result<Value, String> {
        if !self.checked {
            return Err("host data pointers require paired Copy/Drop resource verification".into());
        }
        if depth >= 128 || *self.fuel == 0 {
            return Err("host pointer path budget exhausted".into());
        }
        *self.fuel -= 1;
        match &place.kind {
            K::Local(binding) => {
                let slot = self
                    .locals
                    .get(binding)
                    .ok_or("host address binding missing")?;
                Ok(Value {
                    ty: Ty::Pointer,
                    text: slot.pointer.clone(),
                })
            }
            K::AnonymousField { value, index, .. } | K::Field { value, index, .. } => {
                let base = self.address(value, depth + 1)?;
                if self.terminated {
                    Ok(Value::unit())
                } else {
                    self.project_pointer(base, *index, &value.ty)
                }
            }
            K::ReadPointer(pointer) => self.expr(pointer, depth + 1),
            _ => Err("host address requires a supported local or field place".into()),
        }
    }
    pub(super) fn project_pointer(
        &mut self,
        pointer: Value,
        index: usize,
        pointee: &Term,
    ) -> Result<Value, String> {
        let slot = self.pointer_slot(&pointer, pointee)?;
        let Ty::Record(fields) = &slot.ty else {
            return Err("host pointer projection requires an aggregate layout".into());
        };
        fields
            .get(index)
            .ok_or("host pointer field index out of range")?;
        Ok(self.instruction(
            Ty::Pointer,
            format!(
                "getelementptr {}, ptr {}, i32 0, i32 {index}",
                slot.ty.llvm(),
                pointer.text
            ),
        ))
    }
}
