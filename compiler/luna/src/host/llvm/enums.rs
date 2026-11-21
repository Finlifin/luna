use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum EnumRepr {
    Symbol(Vec<String>),
    FixedBits {
        width: u32,
        codes: Vec<String>,
        masks: Vec<String>,
        fields: Vec<Vec<(usize, Ty)>>,
    },
    Unit {
        width: u32,
    },
    SingleVariant {
        unwrapped: bool,
        carrier: Ty,
        fields: Vec<Ty>,
    },
    Niche {
        data_variant: usize,
        unwrapped: bool,
        data_fields: Vec<Ty>,
        carrier: Ty,
        niche: NicheEncoding,
    },
    TaggedUnion {
        tag_bits: u32,
        payload_size: u64,
        payload_align: u64,
        fields: Vec<Vec<Ty>>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct EnumLayout {
    pub names: Vec<String>,
    pub repr: EnumRepr,
}

fn build_record_from_values(emitter: &mut Emitter<'_>, record_ty: Ty, values: Vec<Value>) -> Value {
    let record_llvm = record_ty.llvm();
    let mut result = Value {
        ty: record_ty.clone(),
        text: "zeroinitializer".into(),
    };
    for (field_index, value) in values.into_iter().enumerate() {
        if value.ty == Ty::Unit {
            continue;
        }
        result = emitter.instruction(
            record_ty.clone(),
            format!(
                "insertvalue {record_llvm} {}, {} {}, {field_index}",
                result.text,
                value.ty.llvm(),
                value.text
            ),
        );
    }
    result
}

impl EnumLayout {
    pub fn variant_record_ty(&self, index: usize) -> Option<Ty> {
        let fields = match &self.repr {
            EnumRepr::TaggedUnion { fields, .. } => fields.get(index)?.clone(),
            EnumRepr::SingleVariant { fields, .. } if index == 0 => fields.clone(),
            EnumRepr::Niche {
                data_variant,
                data_fields,
                ..
            } if index == *data_variant => data_fields.clone(),
            _ => return None,
        };
        Some(Ty::Record(std::sync::Arc::new(fields)))
    }

    pub fn build(
        graph: &iir::Package,
        definition: &nir::program::Enum,
        depth: usize,
        checked: bool,
        remaining: &mut usize,
    ) -> Result<Self, String> {
        // Variants without payloads still allocate layout metadata.
        *remaining = remaining
            .checked_sub(definition.variants.len())
            .ok_or("host representation node budget exhausted")?;
        let names: Vec<String> = definition.variants.iter().map(|v| v.name.clone()).collect();
        if let Some(encoding) = &definition.symbol_encoding {
            // Symbols have no integer encoding or auxiliary discriminant.
            // Private host storage holds immutable UTF-8 bytes and their length;
            // equality compares content, never allocation addresses.
            return Ok(Self {
                names,
                repr: EnumRepr::Symbol(
                    encoding
                        .variants()
                        .iter()
                        .map(|v| v.value.clone())
                        .collect(),
                ),
            });
        }
        if let Some(encoding) = &definition.encoding {
            if !(1..=bits::MAX_BIT_WIDTH as usize).contains(&encoding.width()) {
                return Err("host fixed-width enum exceeds the host width budget".into());
            }
            let mut codes = Vec::new();
            let mut masks = Vec::new();
            let mut fields = Vec::new();
            for (variant, logical) in encoding.variants().iter().zip(&definition.variants) {
                codes.push(bits::decimal_words(variant.fixed_value().words()));
                masks.push(bits::decimal_words(variant.fixed_mask().words()));
                fields.push(
                    variant
                        .fields()
                        .iter()
                        .zip(&logical.fields)
                        .map(|(field, ty)| {
                            Ok((
                                field.start,
                                repr_at(graph, ty, depth + 1, checked, remaining)?,
                            ))
                        })
                        .collect::<Result<Vec<_>, String>>()?,
                );
            }
            return Ok(Self {
                names,
                repr: EnumRepr::FixedBits {
                    width: encoding.width() as u32,
                    codes,
                    masks,
                    fields,
                },
            });
        }
        u32::try_from(definition.variants.len()).map_err(|_| "too many host enum variants")?;
        if definition.variants.iter().all(|v| v.fields.is_empty()) {
            let width = nir::layout::unit_enum_tag_bits(definition.variants.len())
                .map_err(|e| e.to_string())?;
            return Ok(Self {
                names,
                repr: EnumRepr::Unit { width },
            });
        }

        let mut fields = definition
            .variants
            .iter()
            .map(|v| {
                v.fields
                    .iter()
                    .map(|ty| repr_at(graph, ty, depth + 1, checked, remaining))
                    .collect::<Result<Vec<_>, String>>()
            })
            .collect::<Result<Vec<_>, String>>()?;

        if definition.variants.len() == 1 {
            let single_fields = fields.into_iter().next().unwrap();
            let unwrapped = single_fields.len() == 1;
            let carrier = if unwrapped {
                single_fields[0].clone()
            } else {
                Ty::Record(std::sync::Arc::new(single_fields.clone()))
            };
            return Ok(Self {
                names,
                repr: EnumRepr::SingleVariant {
                    unwrapped,
                    carrier,
                    fields: single_fields,
                },
            });
        }

        let mut non_empty = definition
            .variants
            .iter()
            .enumerate()
            .filter(|(_, v)| !v.fields.is_empty());
        let single_data = match (non_empty.next(), non_empty.next()) {
            (Some((idx, _)), None) => Some(idx),
            _ => None,
        };
        if let Some(data_variant) = single_data {
            let empty_count = (definition.variants.len() - 1) as u64;
            let logical_fields = &definition.variants[data_variant].fields;
            let query_term = if logical_fields.len() == 1 {
                logical_fields[0].clone()
            } else {
                Term::Tuple(logical_fields.clone())
            };
            let computed = query_niche_layout(graph, &query_term, remaining)?;
            if let Some(niche) = computed.niche
                && niche.count >= empty_count
            {
                let data_fields = std::mem::take(&mut fields[data_variant]);
                let unwrapped = data_fields.len() == 1;
                let base_carrier = if unwrapped {
                    data_fields[0].clone()
                } else {
                    Ty::Record(std::sync::Arc::new(data_fields.clone()))
                };
                let carrier = if niche.needs_bool_widen {
                    widen_bool_ty(&base_carrier, &niche.path)?
                } else {
                    base_carrier
                };
                return Ok(Self {
                    names,
                    repr: EnumRepr::Niche {
                        data_variant,
                        unwrapped,
                        data_fields,
                        carrier,
                        niche: NicheEncoding {
                            llvm_path: niche.path,
                            scalar: niche.scalar,
                            start: niche.start,
                            widened_bool: niche.needs_bool_widen,
                        },
                    },
                });
            }
        }

        let tag_bits =
            nir::layout::sum_tag_bits(definition.variants.len()).map_err(|e| e.to_string())?;
        let pointer_bits = graph.target.map_or(64, |t| t.pointer_bits());
        let mut fuel = MAX_LAYOUT_NODES;
        let mut variant_layouts = Vec::with_capacity(fields.len());
        for variant_fields in &fields {
            let mut v_layouts = Vec::with_capacity(variant_fields.len());
            for field_ty in variant_fields {
                v_layouts.push(field_ty.storage_layout_at(pointer_bits, 0, &mut fuel)?);
            }
            variant_layouts
                .push(nir::layout::struct_layout(&v_layouts).map_err(|e| e.to_string())?);
        }
        let payload = nir::layout::union_layout(&variant_layouts).map_err(|e| e.to_string())?;
        if !matches!(payload.align, 1 | 2 | 4 | 8 | 16) || payload.size % payload.align != 0 {
            return Err("unsupported host enum union payload alignment".into());
        }
        Ok(Self {
            names,
            repr: EnumRepr::TaggedUnion {
                tag_bits,
                payload_size: payload.size,
                payload_align: payload.align,
                fields,
            },
        })
    }
}

impl Emitter<'_> {
    pub(super) fn variant(
        &mut self,
        source: nir::program::SourceRange,
        index: usize,
        fields: &[Expr],
        ty: &Term,
        depth: usize,
    ) -> Result<Value, String> {
        // A payload can exit before any enum value exists. Keep the declared
        // initialization order without requesting a storage layout for Never.
        if *ty == Term::Primitive(P::Never) {
            for (slot, field) in fields.iter().enumerate() {
                let value = self.expr(field, depth)?;
                if self.terminated {
                    return Ok(Value::unit());
                }
                self.retain_prepared_field(source, slot, field.source, &value)?;
            }
            return Err("host Never enum construction did not terminate".into());
        }
        let ty = repr(self.graph, ty, self.checked)?;
        let Ty::Enum(layout) = ty.clone() else {
            return Err("host variant has no enum representation".into());
        };
        if index >= layout.names.len() {
            return Err("host variant index out of range".into());
        }
        match &layout.repr {
            EnumRepr::Symbol(symbols) => {
                if !fields.is_empty() {
                    return Err("host Symbol enum cannot carry payloads".into());
                }
                let symbol = symbols
                    .get(index)
                    .ok_or("host Symbol variant index out of range")?;
                let mut value = self.text_literal(symbol, Ty::String)?;
                value.ty = ty;
                Ok(value)
            }
            EnumRepr::Unit { width } => {
                if !fields.is_empty() {
                    return Err("host unit enum cannot carry payloads".into());
                }
                self.finish_prepared_construction(source)?;
                Ok(Value {
                    ty,
                    text: if *width == 0 {
                        "zeroinitializer".into()
                    } else {
                        index.to_string()
                    },
                })
            }
            EnumRepr::SingleVariant {
                unwrapped,
                carrier,
                fields: expected,
            } => {
                if index != 0 {
                    return Err("host variant index out of range".into());
                }
                if fields.len() != expected.len() {
                    return Err("host enum payload count mismatch".into());
                }
                let mut evaluated = Vec::with_capacity(fields.len());
                for (field_index, (field, expected_ty)) in
                    fields.iter().zip(expected.iter()).enumerate()
                {
                    let field_source = field.source;
                    let val = self.expr(field, depth)?;
                    if self.terminated {
                        return Ok(Value::unit());
                    }
                    if val.ty != *expected_ty {
                        return Err("host enum payload representation mismatch".into());
                    }
                    self.retain_prepared_field(source, field_index, field_source, &val)?;
                    evaluated.push(val);
                }
                self.finish_prepared_construction(source)?;
                if *unwrapped {
                    let single = evaluated.into_iter().next().unwrap();
                    Ok(Value {
                        ty,
                        text: if *carrier == Ty::Unit {
                            "zeroinitializer".into()
                        } else {
                            single.text
                        },
                    })
                } else {
                    let rec = build_record_from_values(self, carrier.clone(), evaluated);
                    Ok(Value { ty, text: rec.text })
                }
            }
            EnumRepr::Niche {
                data_variant,
                unwrapped,
                data_fields,
                carrier,
                niche,
            } => {
                if index != *data_variant {
                    if !fields.is_empty() {
                        return Err("host enum payload count mismatch".into());
                    }
                    self.finish_prepared_construction(source)?;
                    let ord = if index < *data_variant {
                        index
                    } else {
                        index - 1
                    };
                    let code = niche.start + ord as u64;
                    return self.emit_niche_value(ty, carrier, niche, code);
                }
                if fields.len() != data_fields.len() {
                    return Err("host enum payload count mismatch".into());
                }
                let mut evaluated = Vec::with_capacity(fields.len());
                for (field_index, (field, expected_ty)) in
                    fields.iter().zip(data_fields.iter()).enumerate()
                {
                    let field_source = field.source;
                    let val = self.expr(field, depth)?;
                    if self.terminated {
                        return Ok(Value::unit());
                    }
                    if val.ty != *expected_ty {
                        return Err("host enum payload representation mismatch".into());
                    }
                    self.retain_prepared_field(source, field_index, field_source, &val)?;
                    evaluated.push(val);
                }
                self.finish_prepared_construction(source)?;
                let raw = if *unwrapped {
                    evaluated.into_iter().next().unwrap()
                } else {
                    let rec_ty = Ty::Record(std::sync::Arc::new(data_fields.clone()));
                    build_record_from_values(self, rec_ty, evaluated)
                };
                let carried = if niche.widened_bool {
                    self.emit_widen_bool_at_path(raw, carrier, &niche.llvm_path)?
                } else {
                    raw
                };
                Ok(Value {
                    ty,
                    text: carried.text,
                })
            }
            EnumRepr::FixedBits {
                width,
                codes,
                fields: all_fields,
                ..
            } => {
                let code = codes.get(index).ok_or("host variant index out of range")?;
                let payload = all_fields
                    .get(index)
                    .ok_or("host enum payload layout missing")?
                    .clone();
                if fields.len() != payload.len() {
                    return Err("host enum payload count mismatch".into());
                }
                let width = *width;
                let mut result = Value {
                    ty: ty.clone(),
                    text: code.to_string(),
                };
                for (slot, (field, (start, expected))) in fields.iter().zip(payload).enumerate() {
                    let field_source = field.source;
                    let field = self.expr(field, depth)?;
                    if self.terminated {
                        return Ok(Value::unit());
                    }
                    if field.ty != expected {
                        return Err("host enum payload representation mismatch".into());
                    }
                    self.retain_prepared_field(source, slot, field_source, &field)?;
                    // Empty fields still evaluate and participate in construction,
                    // but contribute no bits, including at start == total width.
                    if bit_width(&field.ty) == Some(0) {
                        continue;
                    }
                    let mut bits = self.to_bits(field)?;
                    let Ty::Int(bits_width, _) = bits.ty else {
                        unreachable!()
                    };
                    if bits_width < width {
                        bits = self.instruction(
                            Ty::Int(width, false),
                            format!("zext i{bits_width} {} to i{width}", bits.text),
                        );
                    }
                    if start > 0 {
                        bits = self.instruction(
                            Ty::Int(width, false),
                            format!("shl i{width} {}, {start}", bits.text),
                        );
                    }
                    result = self.instruction(
                        ty.clone(),
                        format!("or i{width} {}, {}", result.text, bits.text),
                    );
                }
                self.finish_prepared_construction(source)?;
                Ok(result)
            }
            EnumRepr::TaggedUnion {
                tag_bits,
                payload_size,
                fields: all_fields,
                ..
            } => {
                let tag_bits = *tag_bits;
                let payload_size = *payload_size;
                let payload = all_fields
                    .get(index)
                    .ok_or("host enum payload layout missing")?
                    .clone();
                if fields.len() != payload.len() {
                    return Err("host enum payload count mismatch".into());
                }
                let variant_llvm = layout
                    .variant_record_ty(index)
                    .ok_or("host enum payload layout missing")?
                    .llvm();
                let mut evaluated = Vec::with_capacity(fields.len());
                for (field_index, (field, expected)) in fields.iter().zip(payload).enumerate() {
                    let field_source = field.source;
                    let field = self.expr(field, depth)?;
                    if self.terminated {
                        return Ok(Value::unit());
                    }
                    if field.ty != expected {
                        return Err("host enum payload representation mismatch".into());
                    }
                    self.retain_prepared_field(source, field_index, field_source, &field)?;
                    evaluated.push(field);
                }
                if evaluated.is_empty()
                    || payload_size == 0
                    || evaluated.iter().all(|v| v.ty == Ty::Unit)
                {
                    let result = self.instruction(
                        ty.clone(),
                        format!(
                            "insertvalue {} zeroinitializer, i{tag_bits} {index}, 0",
                            ty.llvm()
                        ),
                    );
                    self.finish_prepared_construction(source)?;
                    return Ok(result);
                }
                let slot = self.slot(ty.clone());
                self.store(
                    &slot,
                    &Value {
                        ty: ty.clone(),
                        text: "zeroinitializer".into(),
                    },
                )?;
                let tag_ptr = self.instruction(
                    Ty::Pointer,
                    format!(
                        "getelementptr {}, ptr {}, i32 0, i32 0",
                        ty.llvm(),
                        slot.pointer
                    ),
                );
                self.line(format!("store i{tag_bits} {index}, ptr {}", tag_ptr.text));
                let payload_ptr = self.instruction(
                    Ty::Pointer,
                    format!(
                        "getelementptr {}, ptr {}, i32 0, i32 1",
                        ty.llvm(),
                        slot.pointer
                    ),
                );
                for (field_index, value) in evaluated.iter().enumerate() {
                    if value.ty == Ty::Unit {
                        continue;
                    }
                    let field_ptr = self.instruction(
                        Ty::Pointer,
                        format!(
                            "getelementptr {variant_llvm}, ptr {}, i32 0, i32 {field_index}",
                            payload_ptr.text
                        ),
                    );
                    let field_slot = Slot::new(
                        value.ty.clone(),
                        field_ptr.text,
                        matches!(value.ty, Ty::PackedRecord(_)).then_some(1),
                    );
                    self.store(&field_slot, value)?;
                }
                self.finish_prepared_construction(source)?;
                Ok(self.load(&slot))
            }
        }
    }

    pub(super) fn variant_field(
        &mut self,
        value: &Value,
        index: usize,
        field: usize,
    ) -> Result<Value, String> {
        let Ty::Enum(layout) = &value.ty else {
            return Err("host payload subject is not enum".into());
        };
        match &layout.repr {
            EnumRepr::Symbol(_) | EnumRepr::Unit { .. } => {
                Err("host enum variant carries no payload fields".into())
            }
            EnumRepr::SingleVariant {
                unwrapped,
                carrier,
                fields,
            } => {
                if index != 0 {
                    return Err("host enum field index out of range".into());
                }
                let field_ty = fields
                    .get(field)
                    .cloned()
                    .ok_or("host enum field index out of range")?;
                if field_ty == Ty::Unit {
                    return Ok(Value::unit());
                }
                if *unwrapped {
                    return Ok(Value {
                        ty: field_ty,
                        text: value.text.clone(),
                    });
                }
                Ok(self.instruction(
                    field_ty,
                    format!("extractvalue {} {}, {field}", carrier.llvm(), value.text),
                ))
            }
            EnumRepr::Niche {
                data_variant,
                unwrapped,
                data_fields,
                carrier,
                niche,
            } => {
                if index != *data_variant {
                    return Err("host enum field index out of range".into());
                }
                let field_ty = data_fields
                    .get(field)
                    .cloned()
                    .ok_or("host enum field index out of range")?;
                if field_ty == Ty::Unit {
                    return Ok(Value::unit());
                }
                if *unwrapped {
                    if niche.widened_bool {
                        let raw = Value {
                            ty: carrier.clone(),
                            text: value.text.clone(),
                        };
                        return self.emit_narrow_bool_at_path(raw, &field_ty, &niche.llvm_path);
                    }
                    return Ok(Value {
                        ty: field_ty,
                        text: value.text.clone(),
                    });
                }
                let Ty::Record(carrier_fields) = carrier else {
                    return Err("host niche enum carrier must be a record".into());
                };
                let carrier_field_ty = carrier_fields
                    .get(field)
                    .cloned()
                    .ok_or("host enum field index out of range")?;
                let extracted = self.instruction(
                    carrier_field_ty,
                    format!("extractvalue {} {}, {field}", carrier.llvm(), value.text),
                );
                if niche.widened_bool && niche.llvm_path.first().copied() == Some(field) {
                    self.emit_narrow_bool_at_path(extracted, &field_ty, &niche.llvm_path[1..])
                } else {
                    Ok(extracted)
                }
            }
            EnumRepr::FixedBits {
                width,
                fields: all_fields,
                ..
            } => {
                let (start, ty) = all_fields
                    .get(index)
                    .and_then(|fields| fields.get(field))
                    .cloned()
                    .ok_or("host enum field index out of range")?;
                let width = *width;
                let field_width =
                    bit_width(&ty).ok_or("host enum payload is not a bit representation")?;
                if field_width == 0 {
                    return Ok(Value {
                        ty,
                        text: "zeroinitializer".into(),
                    });
                }
                let mut bits = Value {
                    ty: Ty::Int(width, false),
                    text: value.text.clone(),
                };
                if start > 0 {
                    bits =
                        self.instruction(bits.ty, format!("lshr i{width} {}, {start}", bits.text));
                }
                if field_width < width {
                    bits = self.instruction(
                        Ty::Int(field_width, false),
                        format!("trunc i{width} {} to i{field_width}", bits.text),
                    );
                }
                self.from_bits(bits, ty)
            }
            EnumRepr::TaggedUnion {
                payload_size,
                fields: all_fields,
                ..
            } => {
                let ty = all_fields
                    .get(index)
                    .and_then(|fields| fields.get(field))
                    .cloned()
                    .ok_or("host enum field index out of range")?;
                if ty == Ty::Unit {
                    return Ok(Value::unit());
                }
                if *payload_size == 0 {
                    return Ok(Value {
                        ty,
                        text: "zeroinitializer".into(),
                    });
                }
                let variant_llvm = layout
                    .variant_record_ty(index)
                    .ok_or("host enum payload layout missing")?
                    .llvm();
                let slot = self.slot(value.ty.clone());
                self.store(&slot, value)?;
                let payload_ptr = self.instruction(
                    Ty::Pointer,
                    format!(
                        "getelementptr {}, ptr {}, i32 0, i32 1",
                        value.ty.llvm(),
                        slot.pointer
                    ),
                );
                let field_ptr = self.instruction(
                    Ty::Pointer,
                    format!(
                        "getelementptr {variant_llvm}, ptr {}, i32 0, i32 {field}",
                        payload_ptr.text
                    ),
                );
                let field_slot = Slot::new(
                    ty.clone(),
                    field_ptr.text,
                    matches!(ty, Ty::PackedRecord(_)).then_some(1),
                );
                Ok(self.load(&field_slot))
            }
        }
    }

    pub(super) fn variant_condition(
        &mut self,
        value: &Value,
        index: usize,
    ) -> Result<Value, String> {
        let Ty::Enum(layout) = &value.ty else {
            return Err("host variant pattern has no enum representation".into());
        };
        if index >= layout.names.len() {
            return Err("host variant pattern index out of range".into());
        }
        match &layout.repr {
            EnumRepr::Symbol(symbols) => {
                let symbol = symbols
                    .get(index)
                    .ok_or("host Symbol pattern index out of range")?;
                let expected = self.text_literal(symbol, Ty::String)?;
                let actual = Value {
                    ty: Ty::String,
                    text: value.text.clone(),
                };
                Ok(self.string_equal(&actual, &expected))
            }
            EnumRepr::Unit { width } => {
                if *width == 0 {
                    Ok(Value {
                        ty: Ty::Bool,
                        text: if index == 0 { "true" } else { "false" }.into(),
                    })
                } else {
                    Ok(self.instruction(
                        Ty::Bool,
                        format!("icmp eq i{width} {}, {index}", value.text),
                    ))
                }
            }
            EnumRepr::SingleVariant { .. } => Ok(Value {
                ty: Ty::Bool,
                text: if index == 0 { "true" } else { "false" }.into(),
            }),
            EnumRepr::Niche {
                data_variant,
                carrier,
                niche,
                ..
            } => {
                if index == *data_variant {
                    self.emit_niche_is_payload(
                        value,
                        carrier,
                        niche,
                        (layout.names.len() - 1) as u64,
                    )
                } else {
                    let ord = if index < *data_variant {
                        index
                    } else {
                        index - 1
                    };
                    self.emit_niche_equals_code(value, carrier, niche, niche.start + ord as u64)
                }
            }
            EnumRepr::FixedBits {
                width,
                codes,
                masks,
                ..
            } => {
                let code = codes
                    .get(index)
                    .ok_or("host variant pattern index out of range")?;
                let mask = &masks[index];
                let masked = self.instruction(
                    Ty::Int(*width, false),
                    format!("and {} {}, {mask}", value.ty.llvm(), value.text),
                );
                Ok(self.instruction(
                    Ty::Bool,
                    format!("icmp eq {} {}, {code}", value.ty.llvm(), masked.text),
                ))
            }
            EnumRepr::TaggedUnion { tag_bits, .. } => {
                let tag_bits = *tag_bits;
                let tag = self.instruction(
                    Ty::Int(tag_bits, false),
                    format!("extractvalue {} {}, 0", value.ty.llvm(), value.text),
                );
                Ok(self.instruction(
                    Ty::Bool,
                    format!("icmp eq i{tag_bits} {}, {index}", tag.text),
                ))
            }
        }
    }
}

impl Emitter<'_> {
    pub(super) fn enum_name(&mut self, operand: &Expr, depth: usize) -> Result<Value, String> {
        let value = self.expr(operand, depth)?;
        if self.terminated {
            return Ok(Value::unit());
        }
        self.retain_temporary(operand.source, &value)?;
        let Ty::Enum(layout) = &value.ty else {
            return Err("host tag_name operand requires an enum".into());
        };
        let work = layout.names.iter().fold(0usize, |sum, name| {
            sum.saturating_add(name.len()).saturating_add(1)
        });
        *self.fuel = self
            .fuel
            .checked_sub(work)
            .ok_or("host enum name lowering budget exhausted")?;
        let result = self.slot(Ty::String);
        let end = self.fresh();
        for (index, name) in layout.names.iter().enumerate() {
            let found = self.fresh();
            let next = self.fresh();
            let condition = self.variant_condition(&value, index)?;
            self.line(format!(
                "br i1 {}, label %{found}, label %{next}",
                condition.text
            ));
            self.label(&found);
            let name = self.text_literal(name, Ty::String)?;
            self.store(&result, &name)?;
            self.branch(&end);
            self.label(&next);
        }
        // The validated enum representation admits only declared variants.
        self.line("unreachable");
        self.terminated = true;
        self.label(&end);
        Ok(self.load(&result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enum_def(
        local: u32,
        name: &str,
        variants: Vec<(&str, Vec<Term>)>,
    ) -> (Term, iir::NominalInstance) {
        let declaration = nir::DeclarationId { package: 0, local };
        let ty = Term::Nominal {
            declaration,
            arguments: vec![],
        };
        let definition = nir::program::Enum {
            is_intrinsic: false,
            parameter_types: vec![],
            capabilities: vec![],
            requirements: vec![],
            type_parameters: vec![],
            declaration,
            name: name.into(),
            groups: vec![],
            encoding: None,
            symbol_encoding: None,
            variants: variants
                .into_iter()
                .map(|(vname, fields)| nir::program::Variant {
                    name: vname.into(),
                    shape: if fields.is_empty() {
                        nir::program::VariantShape::Unit
                    } else {
                        nir::program::VariantShape::Tuple
                    },
                    fields,
                })
                .collect(),
        };
        (
            ty.clone(),
            iir::NominalInstance {
                ty,
                definition: iir::NominalDefinition::Enum(definition),
            },
        )
    }

    #[test]
    fn aggregate_enum_uses_tagged_union_sum_layout() {
        let mut graph = iir::Package::default();
        let (small_ty, small_inst) = enum_def(
            1,
            "Small",
            vec![
                ("none", vec![]),
                ("byte", vec![Term::Primitive(P::U8)]),
                ("pair", vec![Term::Primitive(P::U8), Term::Primitive(P::U8)]),
            ],
        );
        let (rich_ty, rich_inst) = enum_def(
            2,
            "Rich",
            vec![
                ("none", vec![]),
                (
                    "pair",
                    vec![Term::Primitive(P::U64), Term::Primitive(P::U64)],
                ),
                (
                    "text",
                    vec![Term::Primitive(P::Str), Term::Primitive(P::U64)],
                ),
                (
                    "triple",
                    vec![
                        Term::Primitive(P::U8),
                        Term::Primitive(P::U64),
                        Term::Primitive(P::U8),
                    ],
                ),
            ],
        );
        let (wide_ty, wide_inst) = enum_def(
            3,
            "Wide",
            vec![
                ("small", vec![Term::Primitive(P::U8)]),
                ("huge", vec![Term::Primitive(P::U128)]),
            ],
        );
        let packed_decl = nir::DeclarationId {
            package: 0,
            local: 4,
        };
        let packed_ty = Term::Nominal {
            declaration: packed_decl,
            arguments: vec![],
        };
        let packed_inst = iir::NominalInstance {
            ty: packed_ty.clone(),
            definition: iir::NominalDefinition::Struct(nir::program::Struct {
                declaration: packed_decl,
                name: "Packed9".into(),
                type_parameters: vec![],
                parameter_types: vec![],
                requirements: vec![],
                capabilities: vec![],
                fields: vec![
                    ("head".into(), Term::Primitive(P::U64)),
                    ("tail".into(), Term::Primitive(P::U8)),
                ],
                packed: true,
            }),
        };
        let (mixed_packed_ty, mixed_packed_inst) = enum_def(
            5,
            "PackedPayload",
            vec![
                ("packed", vec![packed_ty]),
                ("word", vec![Term::Primitive(P::U64)]),
            ],
        );
        let (zero_ty, zero_inst) = enum_def(
            6,
            "ZeroPayload",
            vec![
                ("a", vec![Term::Primitive(P::Void)]),
                ("b", vec![Term::Tuple(vec![])]),
            ],
        );
        let many_decl = nir::DeclarationId {
            package: 0,
            local: 7,
        };
        let many_ty = Term::Nominal {
            declaration: many_decl,
            arguments: vec![],
        };
        let mut many_variants = vec![nir::program::Variant {
            name: "first".into(),
            shape: nir::program::VariantShape::Tuple,
            fields: vec![Term::Primitive(P::U8)],
        }];
        for idx in 1..257 {
            many_variants.push(nir::program::Variant {
                name: format!("v{idx}"),
                shape: nir::program::VariantShape::Unit,
                fields: vec![],
            });
        }
        let many_inst = iir::NominalInstance {
            ty: many_ty.clone(),
            definition: iir::NominalDefinition::Enum(nir::program::Enum {
                is_intrinsic: false,
                parameter_types: vec![],
                capabilities: vec![],
                requirements: vec![],
                type_parameters: vec![],
                declaration: many_decl,
                name: "Many".into(),
                groups: vec![],
                encoding: None,
                symbol_encoding: None,
                variants: many_variants,
            }),
        };
        graph.nominals = vec![
            small_inst,
            rich_inst,
            wide_inst,
            packed_inst,
            mixed_packed_inst,
            zero_inst,
            many_inst,
        ];

        for (ty, expected_llvm, expected_layout) in [
            (
                small_ty,
                "{ i8, [2 x i8] }",
                nir::TypeLayout { size: 3, align: 1 },
            ),
            (
                rich_ty,
                "{ i8, [3 x i64] }",
                nir::TypeLayout { size: 32, align: 8 },
            ),
            (
                wide_ty,
                "{ i8, [1 x i128] }",
                nir::TypeLayout {
                    size: 32,
                    align: 16,
                },
            ),
            (
                mixed_packed_ty,
                "{ i8, [2 x i64] }",
                nir::TypeLayout { size: 24, align: 8 },
            ),
            (zero_ty, "{ i8, {} }", nir::TypeLayout { size: 1, align: 1 }),
            (
                many_ty,
                "{ i16, [1 x i8] }",
                nir::TypeLayout { size: 4, align: 2 },
            ),
        ] {
            let llvm_ty = repr(&graph, &ty, true).unwrap();
            assert_eq!(llvm_ty.llvm(), expected_llvm);
            assert_eq!(
                llvm_ty.storage_layout_at(64, 0, &mut 1000).unwrap(),
                expected_layout
            );
        }
    }

    #[test]
    fn aggregate_enum_o3_eliminates_alloca_roundtrip() {
        let mut graph = iir::Package::default();
        let (enum_ty, inst) = enum_def(
            1,
            "Choice",
            vec![
                ("none", vec![]),
                (
                    "pair",
                    vec![Term::Primitive(P::U8), Term::Primitive(P::U64)],
                ),
                (
                    "wide",
                    vec![Term::Primitive(P::U64), Term::Primitive(P::U64)],
                ),
            ],
        );
        graph.nominals = vec![inst];
        let lambdas = BTreeMap::new();
        let template_identities = callables::TemplateIdentities::default();
        let mut declarations = BTreeSet::new();
        let mut fuel = 10_000;
        let mut emitter = Emitter {
            graph: &graph,
            cleanup: None,
            schedules: None,
            resource_unwinds: vec![],
            checked: true,
            function: None,
            closure_body: None,
            failure_result: None,
            effects: vec![],
            effect_resume: None,
            native_return: None,
            loop_destinations: Default::default(),
            outlined_return: None,
            outlined_loops: BTreeMap::new(),
            lambdas: &lambdas,
            template_identities: &template_identities,
            locals: BTreeMap::new(),
            live: BTreeMap::new(),
            temporaries: BTreeMap::new(),
            temporary_live: BTreeMap::new(),
            prepared: BTreeMap::new(),
            receipt_paths: vec![],
            pattern_orders: BTreeMap::new(),
            temporary_fields: BTreeMap::new(),
            loops: BTreeMap::new(),
            prologue: String::new(),
            body: String::new(),
            declarations: &mut declarations,
            text_constants: &mut strings::TextConstants::default(),
            attr_table: None,
            serial: 0,
            terminated: false,
            fuel: &mut fuel,
        };
        let x_id = LocalId { owner: 0, index: 0 };
        let y_id = LocalId { owner: 0, index: 1 };
        let x_slot = emitter.slot(Ty::Int(8, false));
        let y_slot = emitter.slot(Ty::Int(64, false));
        emitter
            .store(
                &x_slot,
                &Value {
                    ty: Ty::Int(8, false),
                    text: "%x".into(),
                },
            )
            .unwrap();
        emitter
            .store(
                &y_slot,
                &Value {
                    ty: Ty::Int(64, false),
                    text: "%y".into(),
                },
            )
            .unwrap();
        emitter.locals.insert(x_id, x_slot);
        emitter.locals.insert(y_id, y_slot);

        let packed = emitter
            .variant(
                nir::program::SourceRange::generated(0),
                1,
                &[
                    Expr {
                        ty: Term::Primitive(P::U8),
                        source: nir::program::SourceRange::generated(1),
                        kind: K::Local(x_id),
                    },
                    Expr {
                        ty: Term::Primitive(P::U64),
                        source: nir::program::SourceRange::generated(2),
                        kind: K::Local(y_id),
                    },
                ],
                &enum_ty,
                0,
            )
            .unwrap();
        let is_pair = emitter.variant_condition(&packed, 1).unwrap();
        let first = emitter.variant_field(&packed, 1, 0).unwrap();
        let second = emitter.variant_field(&packed, 1, 1).unwrap();
        let ext = emitter.instruction(Ty::Int(64, false), format!("zext i8 {} to i64", first.text));
        let sum = emitter.instruction(
            Ty::Int(64, false),
            format!("add i64 {}, {}", ext.text, second.text),
        );
        let result = emitter.instruction(
            Ty::Int(64, false),
            format!("select i1 {}, i64 {}, i64 0", is_pair.text, sum.text),
        );
        emitter.line(format!("ret i64 {}", result.text));
        let ir = format!(
            "define i64 @roundtrip(i8 %x, i64 %y) {{\nentry:\n{}{}}}\n",
            emitter.prologue, emitter.body
        );
        let toolchain = crate::host::Toolchain {
            optimization: crate::host::OptimizationLevel::O3,
            ..Default::default()
        };
        let optimized = toolchain.optimized_ir(&ir).unwrap();
        assert!(!optimized.contains("alloca"), "{optimized}");
        assert!(optimized.contains("add"), "{optimized}");
        assert!(optimized.contains("zext i8 %x to i64"), "{optimized}");
    }

    #[test]
    fn unit_single_variant_and_niche_enum_layouts_and_o3_lowering() {
        let mut graph = iir::Package::default();
        let (zero_ty, zero_inst) = enum_def(1, "Empty", vec![]);
        let (solo_ty, solo_inst) = enum_def(2, "Solo", vec![("only", vec![])]);
        let (tri_ty, tri_inst) =
            enum_def(3, "Tri", vec![("a", vec![]), ("b", vec![]), ("c", vec![])]);
        let many_decl = nir::DeclarationId {
            package: 0,
            local: 4,
        };
        let many_ty = Term::Nominal {
            declaration: many_decl,
            arguments: vec![],
        };
        let many_inst = iir::NominalInstance {
            ty: many_ty.clone(),
            definition: iir::NominalDefinition::Enum(nir::program::Enum {
                is_intrinsic: false,
                parameter_types: vec![],
                capabilities: vec![],
                requirements: vec![],
                type_parameters: vec![],
                declaration: many_decl,
                name: "Big257".into(),
                groups: vec![],
                encoding: None,
                symbol_encoding: None,
                variants: (0..257)
                    .map(|idx| nir::program::Variant {
                        name: format!("v{idx}"),
                        shape: nir::program::VariantShape::Unit,
                        fields: vec![],
                    })
                    .collect(),
            }),
        };
        let (single_u8_ty, single_u8_inst) =
            enum_def(5, "SingleU8", vec![("wrap", vec![Term::Primitive(P::U8)])]);
        let (single_pair_ty, single_pair_inst) = enum_def(
            6,
            "SinglePair",
            vec![(
                "wrap",
                vec![Term::Primitive(P::U8), Term::Primitive(P::U64)],
            )],
        );
        let (maybe_ptr_ty, maybe_ptr_inst) = enum_def(
            7,
            "MaybePtr",
            vec![
                ("empty", vec![]),
                ("ptr", vec![Term::Pointer(Box::new(Term::Primitive(P::U8)))]),
            ],
        );
        let (maybe_ptr2_ty, maybe_ptr2_inst) = enum_def(
            8,
            "MaybePtr2",
            vec![
                ("empty", vec![]),
                ("alt", vec![]),
                ("ptr", vec![Term::Pointer(Box::new(Term::Primitive(P::U8)))]),
            ],
        );
        let (three_bool_ty, three_bool_inst) = enum_def(
            9,
            "ThreeWithBool",
            vec![
                ("empty", vec![]),
                ("alt", vec![]),
                ("flag", vec![Term::Primitive(P::Bool)]),
            ],
        );
        let (pair_opt_ty, pair_opt_inst) = enum_def(
            10,
            "PairOpt",
            vec![
                ("empty", vec![]),
                (
                    "pair",
                    vec![Term::Primitive(P::Bool), Term::Primitive(P::U64)],
                ),
            ],
        );
        graph.nominals = vec![
            zero_inst,
            solo_inst,
            tri_inst,
            many_inst,
            single_u8_inst,
            single_pair_inst,
            maybe_ptr_inst,
            maybe_ptr2_inst,
            three_bool_inst,
            pair_opt_inst,
        ];

        for (ty, expected_llvm, expected_layout) in [
            (&zero_ty, "{}", nir::TypeLayout { size: 0, align: 1 }),
            (&solo_ty, "{}", nir::TypeLayout { size: 0, align: 1 }),
            (&tri_ty, "i8", nir::TypeLayout { size: 1, align: 1 }),
            (&many_ty, "i16", nir::TypeLayout { size: 2, align: 2 }),
            (&single_u8_ty, "i8", nir::TypeLayout { size: 1, align: 1 }),
            (
                &single_pair_ty,
                "{ i8, i64 }",
                nir::TypeLayout { size: 16, align: 8 },
            ),
            (&maybe_ptr_ty, "ptr", nir::TypeLayout { size: 8, align: 8 }),
            (
                &maybe_ptr2_ty,
                "{ i8, [1 x i64] }",
                nir::TypeLayout { size: 16, align: 8 },
            ),
            (&three_bool_ty, "i8", nir::TypeLayout { size: 1, align: 1 }),
            (
                &pair_opt_ty,
                "{ i8, i64 }",
                nir::TypeLayout { size: 16, align: 8 },
            ),
        ] {
            let llvm_ty = repr(&graph, ty, true).unwrap();
            assert_eq!(llvm_ty.llvm(), expected_llvm, "{ty}");
            assert_eq!(
                llvm_ty.storage_layout_at(64, 0, &mut 1000).unwrap(),
                expected_layout,
                "{ty}"
            );
        }

        let lambdas = BTreeMap::new();
        let template_identities = callables::TemplateIdentities::default();
        let mut declarations = BTreeSet::new();
        let mut fuel = 10_000;
        let mut emitter = Emitter {
            graph: &graph,
            cleanup: None,
            schedules: None,
            resource_unwinds: vec![],
            checked: true,
            function: None,
            closure_body: None,
            failure_result: None,
            effects: vec![],
            effect_resume: None,
            native_return: None,
            loop_destinations: Default::default(),
            outlined_return: None,
            outlined_loops: BTreeMap::new(),
            lambdas: &lambdas,
            template_identities: &template_identities,
            locals: BTreeMap::new(),
            live: BTreeMap::new(),
            temporaries: BTreeMap::new(),
            temporary_live: BTreeMap::new(),
            prepared: BTreeMap::new(),
            receipt_paths: vec![],
            pattern_orders: BTreeMap::new(),
            temporary_fields: BTreeMap::new(),
            loops: BTreeMap::new(),
            prologue: String::new(),
            body: String::new(),
            declarations: &mut declarations,
            text_constants: &mut strings::TextConstants::default(),
            attr_table: None,
            serial: 0,
            terminated: false,
            fuel: &mut fuel,
        };
        let b_id = LocalId { owner: 0, index: 0 };
        let y_id = LocalId { owner: 0, index: 1 };
        let b_slot = emitter.slot(Ty::Bool);
        let y_slot = emitter.slot(Ty::Int(64, false));
        emitter
            .store(
                &b_slot,
                &Value {
                    ty: Ty::Bool,
                    text: "%b".into(),
                },
            )
            .unwrap();
        emitter
            .store(
                &y_slot,
                &Value {
                    ty: Ty::Int(64, false),
                    text: "%y".into(),
                },
            )
            .unwrap();
        emitter.locals.insert(b_id, b_slot);
        emitter.locals.insert(y_id, y_slot);
        let constructed = emitter
            .variant(
                nir::program::SourceRange::generated(0),
                1,
                &[
                    Expr {
                        ty: Term::Primitive(P::Bool),
                        source: nir::program::SourceRange::generated(1),
                        kind: K::Local(b_id),
                    },
                    Expr {
                        ty: Term::Primitive(P::U64),
                        source: nir::program::SourceRange::generated(2),
                        kind: K::Local(y_id),
                    },
                ],
                &pair_opt_ty,
                0,
            )
            .unwrap();
        let is_pair = emitter.variant_condition(&constructed, 1).unwrap();
        let fb = emitter.variant_field(&constructed, 1, 0).unwrap();
        let fy = emitter.variant_field(&constructed, 1, 1).unwrap();
        let keep = emitter.instruction(Ty::Bool, format!("and i1 {}, {}", is_pair.text, fb.text));
        let out = emitter.instruction(
            Ty::Int(64, false),
            format!("select i1 {}, i64 {}, i64 0", keep.text, fy.text),
        );
        emitter.line(format!("ret i64 {}", out.text));
        let ir = format!(
            "define i64 @niche_roundtrip(i1 %b, i64 %y) {{\nentry:\n{}{}}}\n",
            emitter.prologue, emitter.body
        );
        let toolchain = crate::host::Toolchain {
            optimization: crate::host::OptimizationLevel::O3,
            ..Default::default()
        };
        let optimized = toolchain.optimized_ir(&ir).unwrap();
        assert!(!optimized.contains("alloca"), "{optimized}");
        assert!(!optimized.contains("icmp"), "{optimized}");
        assert!(optimized.contains("select"), "{optimized}");
    }
}
