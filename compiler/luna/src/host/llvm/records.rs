use super::*;
use nir::program::{FieldOwner, Place};

pub(super) enum AssignmentTarget {
    Direct(Slot),
    BitField {
        slot: Slot,
        bit_offset: u32,
        bit_width: u32,
        field_ty: Ty,
    },
}

impl Emitter<'_> {
    pub(super) fn record(
        &mut self,
        source: nir::program::SourceRange,
        fields: &[Expr],
        ty: &Term,
        depth: usize,
    ) -> Result<Value, String> {
        if *ty == Term::Primitive(P::Never) {
            for (index, field) in fields.iter().enumerate() {
                let value = self.expr(field, depth)?;
                if self.terminated {
                    return Ok(Value::unit());
                }
                self.retain_prepared_field(source, index, field.source, &value)?;
            }
            return Err("host Never product construction did not terminate".into());
        }
        let ty = repr(self.graph, ty, self.checked)?;
        if ty == Ty::Unit && fields.is_empty() {
            return Ok(Value::unit());
        }
        if let Ty::BitPackedRecord(layout) = &ty {
            if fields.len() != layout.fields.len() {
                return Err("host bit-packed record field count mismatch".into());
            }
            let mut field_bits = Vec::with_capacity(fields.len());
            for (index, (field, expected)) in fields.iter().zip(layout.fields.iter()).enumerate() {
                let field_source = field.source;
                let value = self.expr(field, depth)?;
                if self.terminated {
                    return Ok(Value::unit());
                }
                if value.ty != expected.ty {
                    return Err("host bit-packed field representation mismatch".into());
                }
                self.retain_prepared_field(source, index, field_source, &value)?;
                field_bits.push(self.to_bits(value)?);
            }
            let mut acc = None;
            for bits_val in field_bits {
                if bits::bit_width(&bits_val.ty) == Some(0) {
                    continue;
                }
                acc = Some(match acc {
                    None => bits_val,
                    Some(lo) => self.bit_concat(bits_val, lo)?,
                });
            }
            self.finish_prepared_construction(source)?;
            let packed_bits = acc.unwrap_or(Value {
                ty: Ty::Int(0, false),
                text: "zeroinitializer".into(),
            });
            return self.from_bits(packed_bits, ty);
        }
        if let Ty::AlignedRecord(layout) = &ty {
            if fields.len() != layout.fields.len() {
                return Err("host aligned record field count mismatch".into());
            }
            let mut value = Value {
                ty: ty.clone(),
                text: "zeroinitializer".into(),
            };
            for (index, (field, expected)) in fields.iter().zip(layout.fields.iter()).enumerate() {
                let field_source = field.source;
                let field = self.expr(field, depth)?;
                if self.terminated {
                    return Ok(Value::unit());
                }
                if field.ty != *expected {
                    return Err("host aligned record field representation mismatch".into());
                }
                self.retain_prepared_field(source, index, field_source, &field)?;
                let (storage, text) = if field.ty == Ty::Unit {
                    ("{}".into(), "zeroinitializer".into())
                } else {
                    (field.ty.llvm(), field.text)
                };
                let llvm_index = layout.field_llvm_indices[index];
                value = self.instruction(
                    ty.clone(),
                    format!(
                        "insertvalue {} {}, {storage} {text}, {llvm_index}",
                        ty.llvm(),
                        value.text
                    ),
                );
            }
            self.finish_prepared_construction(source)?;
            return Ok(value);
        }
        let (Ty::Record(layout) | Ty::PackedRecord(layout)) = &ty else {
            return Err("host anonymous construction lacks a record layout".into());
        };
        if fields.len() != layout.len() {
            return Err("host anonymous field count mismatch".into());
        }
        let mut value = Value {
            ty: ty.clone(),
            text: "zeroinitializer".into(),
        };
        for (index, (field, expected)) in fields.iter().zip(layout.iter()).enumerate() {
            let field_source = field.source;
            let field = self.expr(field, depth)?;
            if self.terminated {
                return Ok(Value::unit());
            }
            if field.ty != *expected {
                return Err("host record field representation mismatch".into());
            }
            self.retain_prepared_field(source, index, field_source, &field)?;
            let (storage, text) = if field.ty == Ty::Unit {
                ("{}".into(), "zeroinitializer".into())
            } else {
                (field.ty.llvm(), field.text)
            };
            value = self.instruction(
                ty.clone(),
                format!(
                    "insertvalue {} {}, {storage} {text}, {index}",
                    ty.llvm(),
                    value.text
                ),
            );
        }
        self.finish_prepared_construction(source)?;
        Ok(value)
    }

    pub(super) fn extract_tuple_values(&mut self, value: &Value) -> Result<Vec<Value>, String> {
        match &value.ty {
            Ty::Unit => Ok(Vec::new()),
            Ty::Record(fields) | Ty::PackedRecord(fields) => (0..fields.len())
                .map(|idx| self.record_field(value, idx))
                .collect(),
            _ => Err("host tuple operation requires a tuple or unit value".into()),
        }
    }

    pub(super) fn record_from_values(
        &mut self,
        prepared_source: Option<nir::program::SourceRange>,
        evaluated: Vec<Value>,
        ty: &Term,
    ) -> Result<Value, String> {
        let ty = repr(self.graph, ty, self.checked)?;
        if ty == Ty::Unit && evaluated.is_empty() {
            return Ok(Value::unit());
        }
        if let Ty::BitPackedRecord(layout) = &ty {
            if evaluated.len() != layout.fields.len() {
                return Err("host bit-packed record field count mismatch".into());
            }
            let mut field_bits = Vec::with_capacity(evaluated.len());
            for (value, expected) in evaluated.into_iter().zip(layout.fields.iter()) {
                if value.ty != expected.ty {
                    return Err("host bit-packed field representation mismatch".into());
                }
                field_bits.push(self.to_bits(value)?);
            }
            let mut acc = None;
            for bits_val in field_bits {
                if bits::bit_width(&bits_val.ty) == Some(0) {
                    continue;
                }
                acc = Some(match acc {
                    None => bits_val,
                    Some(lo) => self.bit_concat(bits_val, lo)?,
                });
            }
            if let Some(source) = prepared_source {
                self.finish_prepared_construction(source)?;
            }
            let packed_bits = acc.unwrap_or(Value {
                ty: Ty::Int(0, false),
                text: "zeroinitializer".into(),
            });
            return self.from_bits(packed_bits, ty);
        }
        let (Ty::Record(layout) | Ty::PackedRecord(layout)) = &ty else {
            return Err("host anonymous construction lacks a record layout".into());
        };
        if evaluated.len() != layout.len() {
            return Err("host anonymous field count mismatch".into());
        }
        let mut value = Value {
            ty: ty.clone(),
            text: "zeroinitializer".into(),
        };
        for (index, (field, expected)) in evaluated.into_iter().zip(layout.iter()).enumerate() {
            if field.ty != *expected {
                return Err("host record field representation mismatch".into());
            }
            let (storage, text) = if field.ty == Ty::Unit {
                ("{}".into(), "zeroinitializer".into())
            } else {
                (field.ty.llvm(), field.text)
            };
            value = self.instruction(
                ty.clone(),
                format!(
                    "insertvalue {} {}, {storage} {text}, {index}",
                    ty.llvm(),
                    value.text
                ),
            );
        }
        if let Some(source) = prepared_source {
            self.finish_prepared_construction(source)?;
        }
        Ok(value)
    }

    pub(super) fn construct_struct(
        &mut self,
        fields: &Expr,
        ty: &Term,
        depth: usize,
    ) -> Result<Value, String> {
        let tuple_val = self.expr(fields, depth)?;
        if self.terminated {
            return Ok(Value::unit());
        }
        let evaluated = self.extract_tuple_values(&tuple_val)?;
        self.record_from_values(None, evaluated, ty)
    }

    pub(super) fn tuple_append(
        &mut self,
        tuple: &Expr,
        element: &Expr,
        ty: &Term,
        depth: usize,
    ) -> Result<Value, String> {
        let tuple_val = self.expr(tuple, depth)?;
        if self.terminated {
            return Ok(Value::unit());
        }
        let elem_val = self.expr(element, depth)?;
        if self.terminated {
            return Ok(Value::unit());
        }
        let mut evaluated = self.extract_tuple_values(&tuple_val)?;
        evaluated.push(elem_val);
        self.record_from_values(None, evaluated, ty)
    }

    pub(super) fn array(
        &mut self,
        source: nir::program::SourceRange,
        elements: &[Expr],
        ty: &Term,
        depth: usize,
    ) -> Result<Value, String> {
        let repr_ty = repr(self.graph, ty, self.checked)?;
        let Ty::Array { elem, len, .. } = &repr_ty else {
            return Err("host array construction lacks an array layout".into());
        };
        if elements.len() != *len {
            return Err("host array element count mismatch".into());
        }
        let mut value = Value {
            ty: repr_ty.clone(),
            text: "zeroinitializer".into(),
        };
        for (index, element) in elements.iter().enumerate() {
            let elem_source = element.source;
            let val = self.expr(element, depth)?;
            if self.terminated {
                return Ok(Value::unit());
            }
            if val.ty != **elem {
                return Err("host array element representation mismatch".into());
            }
            self.retain_prepared_field(source, index, elem_source, &val)?;
            let (storage, text) = if val.ty == Ty::Unit {
                ("{}".into(), "zeroinitializer".into())
            } else {
                (val.ty.llvm(), val.text)
            };
            value = self.instruction(
                repr_ty.clone(),
                format!(
                    "insertvalue {} {}, {storage} {text}, {index}",
                    repr_ty.llvm(),
                    value.text
                ),
            );
        }
        self.finish_prepared_construction(source)?;
        Ok(value)
    }

    pub(super) fn record_field(&mut self, value: &Value, index: usize) -> Result<Value, String> {
        if let Ty::BitPackedRecord(layout) = &value.ty {
            let field = layout
                .fields
                .get(index)
                .ok_or("host bit-packed projection index out of range")?;
            if field.ty == Ty::Unit {
                return Ok(Value::unit());
            }
            if field.bit_width == 0 {
                return self.from_bits(
                    Value {
                        ty: Ty::Int(0, false),
                        text: "zeroinitializer".into(),
                    },
                    field.ty.clone(),
                );
            }
            let raw = self.to_bits(value.clone())?;
            let sliced = self.bit_slice(
                raw,
                field.bit_offset as usize,
                (field.bit_offset + field.bit_width) as usize,
            )?;
            return self.from_bits(sliced, field.ty.clone());
        }
        if let Ty::AlignedRecord(layout) = &value.ty {
            let field = layout
                .fields
                .get(index)
                .ok_or("host aligned record projection index out of range")?;
            if *field == Ty::Unit {
                return Ok(Value::unit());
            }
            let llvm_index = layout.field_llvm_indices[index];
            return Ok(self.instruction(
                field.clone(),
                format!(
                    "extractvalue {} {}, {llvm_index}",
                    value.ty.llvm(),
                    value.text
                ),
            ));
        }
        let (Ty::Record(fields) | Ty::PackedRecord(fields)) = &value.ty else {
            return Err("host field projection requires an anonymous record".into());
        };
        let field = fields
            .get(index)
            .ok_or("host record projection index out of range")?;
        if *field == Ty::Unit {
            return Ok(Value::unit());
        }
        Ok(self.instruction(
            field.clone(),
            format!("extractvalue {} {}, {index}", value.ty.llvm(), value.text),
        ))
    }

    pub(super) fn resolve_assignment_target(
        &mut self,
        place: &Place,
    ) -> Result<AssignmentTarget, String> {
        let root = self
            .locals
            .get(&place.root)
            .cloned()
            .ok_or("host assignment binding is unavailable")?;
        if place.fields.is_empty() {
            return Ok(AssignmentTarget::Direct(root));
        }
        let mut ty = root.ty.clone();
        let mut packed = root.align == Some(1) || matches!(ty, Ty::PackedRecord(_));
        let mut volatile = root.volatile;
        let mut path = String::from("i32 0");
        let mut byte_prefix_len = 0usize;
        let mut bit_target: Option<(u32, u32, Ty)> = None;

        for (owner, index) in &place.fields {
            if *owner != FieldOwner::Anonymous && !self.checked {
                return Err("host nominal field assignment requires resource verification".into());
            }
            if let FieldOwner::Nominal(decl) = owner {
                volatile |= self.graph.volatile_storage.is_field_volatile(*decl, *index);
            }
            if let Some((ref mut offset, _, ref mut field_ty)) = bit_target {
                let Ty::BitPackedRecord(layout) = field_ty.clone() else {
                    return Err("host bit-packed field assignment path mismatch".into());
                };
                let field = layout
                    .fields
                    .get(*index)
                    .ok_or("host assignment field index out of range")?;
                *offset = offset
                    .checked_add(field.bit_offset)
                    .ok_or("host bit-packed assignment offset overflow")?;
                bit_target = Some((*offset, field.bit_width, field.ty.clone()));
                continue;
            }
            let next_ty = match &ty {
                Ty::Record(fields) | Ty::PackedRecord(fields) => {
                    byte_prefix_len += 1;
                    let next = fields
                        .get(*index)
                        .cloned()
                        .ok_or("host assignment field index out of range")?;
                    packed |= matches!(next, Ty::PackedRecord(_));
                    path.push_str(&format!(", i32 {index}"));
                    Some(next)
                }
                Ty::AlignedRecord(layout) => {
                    byte_prefix_len += 1;
                    let next = layout
                        .fields
                        .get(*index)
                        .cloned()
                        .ok_or("host assignment field index out of range")?;
                    let llvm_index = layout.field_llvm_indices[*index];
                    path.push_str(&format!(", i32 {llvm_index}"));
                    Some(next)
                }
                Ty::BitPackedRecord(layout) => {
                    let field = layout
                        .fields
                        .get(*index)
                        .ok_or("host assignment field index out of range")?;
                    bit_target = Some((field.bit_offset, field.bit_width, field.ty.clone()));
                    None
                }
                _ => return Err("host assignment path is not an anonymous record".into()),
            };
            if let Some(next_ty) = next_ty {
                ty = next_ty;
            }
        }

        let pointer = if byte_prefix_len == 0 {
            root.pointer
        } else {
            let name = self.fresh();
            self.line(format!(
                "%{name} = getelementptr {}, ptr {}, {path}",
                root.ty.llvm(),
                root.pointer
            ));
            format!("%{name}")
        };
        let slot = Slot {
            ty,
            pointer,
            align: if packed { Some(1) } else { root.align },
            volatile,
        };
        match bit_target {
            Some((bit_offset, bit_width, field_ty)) => Ok(AssignmentTarget::BitField {
                slot,
                bit_offset,
                bit_width,
                field_ty,
            }),
            None => Ok(AssignmentTarget::Direct(slot)),
        }
    }
}
