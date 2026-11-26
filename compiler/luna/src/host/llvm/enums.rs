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

    pub(super) fn emit_unsigned_ranges_condition(
        &mut self,
        scalar_text: &str,
        width: u32,
        ranges: &[(u128, u128)],
    ) -> Value {
        if ranges.is_empty() {
            return Value {
                ty: Ty::Bool,
                text: "false".into(),
            };
        }
        let full_mask = if width == 128 {
            u128::MAX
        } else {
            (1u128 << width) - 1
        };
        let mut combined: Option<Value> = None;
        for &(lo, hi) in ranges {
            if lo == 0 && hi == full_mask {
                return Value {
                    ty: Ty::Bool,
                    text: "true".into(),
                };
            }
            let cond = if lo == 0 {
                self.instruction(Ty::Bool, format!("icmp ule i{width} {scalar_text}, {hi}"))
            } else if hi == full_mask {
                self.instruction(Ty::Bool, format!("icmp uge i{width} {scalar_text}, {lo}"))
            } else if lo == hi {
                self.instruction(Ty::Bool, format!("icmp eq i{width} {scalar_text}, {lo}"))
            } else {
                let shifted = self.instruction(
                    Ty::Int(width, false),
                    format!("sub i{width} {scalar_text}, {lo}"),
                );
                let span = hi - lo;
                self.instruction(
                    Ty::Bool,
                    format!("icmp ule i{width} {}, {span}", shifted.text),
                )
            };
            combined = Some(match combined {
                None => cond,
                Some(prev) => {
                    self.instruction(Ty::Bool, format!("or i1 {}, {}", prev.text, cond.text))
                }
            });
        }
        combined.unwrap()
    }

    pub(super) fn variant_set_condition(
        &mut self,
        value: &Value,
        indices: &[usize],
    ) -> Result<Option<Value>, String> {
        let Ty::Enum(layout) = value.ty.clone() else {
            return Err("host variant set pattern has no enum representation".into());
        };
        *self.fuel = self
            .fuel
            .checked_sub(indices.len().max(1))
            .ok_or("host variant set lowering budget exhausted")?;
        if indices.iter().any(|&index| index >= layout.names.len()) {
            return Err("host variant pattern index out of range".into());
        }
        let mut unique = indices.to_vec();
        unique.sort_unstable();
        unique.dedup();
        if unique.is_empty() {
            return Ok(Some(Value {
                ty: Ty::Bool,
                text: "false".into(),
            }));
        }
        match &layout.repr {
            EnumRepr::Unit { width: 0 } | EnumRepr::SingleVariant { .. } => Ok(Some(Value {
                ty: Ty::Bool,
                text: if unique.contains(&0) { "true" } else { "false" }.into(),
            })),
            EnumRepr::Unit { width } => {
                let ranges = nir::enum_encoding::merge_contiguous_ranges(
                    unique.iter().map(|&i| (i as u128, i as u128)).collect(),
                );
                Ok(Some(self.emit_unsigned_ranges_condition(
                    &value.text,
                    *width,
                    &ranges,
                )))
            }
            EnumRepr::TaggedUnion { tag_bits, .. } => {
                let tag_bits = *tag_bits;
                let tag = self.instruction(
                    Ty::Int(tag_bits, false),
                    format!("extractvalue {} {}, 0", value.ty.llvm(), value.text),
                );
                let ranges = nir::enum_encoding::merge_contiguous_ranges(
                    unique.iter().map(|&i| (i as u128, i as u128)).collect(),
                );
                Ok(Some(self.emit_unsigned_ranges_condition(
                    &tag.text, tag_bits, &ranges,
                )))
            }
            EnumRepr::Niche {
                data_variant,
                carrier,
                niche,
                ..
            } => match niche.scalar {
                nir::layout::NicheScalar::Int { bits } => {
                    if !unique.contains(data_variant) {
                        let raw = unique
                            .iter()
                            .map(|&idx| {
                                let ord = if idx < *data_variant { idx } else { idx - 1 };
                                let code = u128::from(niche.start) + ord as u128;
                                (code, code)
                            })
                            .collect();
                        let ranges = nir::enum_encoding::merge_contiguous_ranges(raw);
                        let leaf = self.extract_niche_leaf(value, carrier, niche)?;
                        Ok(Some(
                            self.emit_unsigned_ranges_condition(&leaf.text, bits, &ranges),
                        ))
                    } else {
                        let excluded: Vec<(u128, u128)> = (0..layout.names.len())
                            .filter(|idx| *idx != *data_variant && !unique.contains(idx))
                            .map(|idx| {
                                let ord = if idx < *data_variant { idx } else { idx - 1 };
                                let code = u128::from(niche.start) + ord as u128;
                                (code, code)
                            })
                            .collect();
                        if excluded.is_empty() {
                            return Ok(Some(Value {
                                ty: Ty::Bool,
                                text: "true".into(),
                            }));
                        }
                        let ranges = nir::enum_encoding::merge_contiguous_ranges(excluded);
                        let leaf = self.extract_niche_leaf(value, carrier, niche)?;
                        let is_excluded =
                            self.emit_unsigned_ranges_condition(&leaf.text, bits, &ranges);
                        Ok(Some(self.instruction(
                            Ty::Bool,
                            format!("xor i1 {}, true", is_excluded.text),
                        )))
                    }
                }
                nir::layout::NicheScalar::Pointer => {
                    if unique.len() == layout.names.len() {
                        Ok(Some(Value {
                            ty: Ty::Bool,
                            text: "true".into(),
                        }))
                    } else if unique.len() == 1 {
                        Ok(Some(self.variant_condition(value, unique[0])?))
                    } else {
                        Ok(None)
                    }
                }
            },
            EnumRepr::FixedBits {
                width,
                codes,
                masks,
                ..
            } if *width <= 128 => {
                let full_mask = if *width == 128 {
                    u128::MAX
                } else {
                    (1u128 << *width) - 1
                };
                let mut intervals = Vec::new();
                let mut residual = Vec::new();
                for &idx in &unique {
                    let code = codes[idx]
                        .parse::<u128>()
                        .map_err(|_| "host fixed-bit variant code overflow")?;
                    let mask = masks[idx]
                        .parse::<u128>()
                        .map_err(|_| "host fixed-bit variant mask overflow")?;
                    let free = !mask & full_mask;
                    let val = code & mask;
                    if free & free.wrapping_add(1) == 0 {
                        intervals.push((val, val | free));
                    } else {
                        residual.push(idx);
                    }
                }
                let ranges = nir::enum_encoding::merge_contiguous_ranges(intervals);
                let mut combined = if ranges.is_empty() {
                    None
                } else {
                    Some(self.emit_unsigned_ranges_condition(&value.text, *width, &ranges))
                };
                for idx in residual {
                    let cond = self.variant_condition(value, idx)?;
                    combined = Some(match combined {
                        None => cond,
                        Some(prev) => self
                            .instruction(Ty::Bool, format!("or i1 {}, {}", prev.text, cond.text)),
                    });
                }
                Ok(Some(combined.unwrap_or(Value {
                    ty: Ty::Bool,
                    text: "false".into(),
                })))
            }
            EnumRepr::FixedBits { .. } | EnumRepr::Symbol(_) => Ok(None),
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

    pub(super) fn construct_enum(
        &mut self,
        source: nir::program::SourceRange,
        variant_expr: &Expr,
        fields_expr: &Expr,
        ty: &Term,
        depth: usize,
    ) -> Result<Value, String> {
        let variant_val = self.expr(variant_expr, depth)?;
        if self.terminated {
            return Ok(Value::unit());
        }
        let tuple_val = self.expr(fields_expr, depth)?;
        if self.terminated {
            return Ok(Value::unit());
        }
        let evaluated = self.extract_tuple_values(&tuple_val)?;
        if let K::Literal(nir::program::Literal::UnsignedInteger(raw)) = &variant_expr.kind {
            if let Ok(index) = usize::try_from(*raw) {
                return self.variant_from_values(source, index, evaluated, ty, false);
            }
        }
        let enum_ty = repr(self.graph, ty, self.checked)?;
        let Ty::Enum(layout) = enum_ty.clone() else {
            return Err("host construct_enum requires an enum type".into());
        };
        let Ty::Int(tag_bits, _) = variant_val.ty else {
            return Err("host construct_enum variant index must be an integer".into());
        };
        let result_slot = self.slot(enum_ty);
        let end = self.fresh();
        for index in 0..layout.names.len() {
            let expected_tys = match &layout.repr {
                EnumRepr::Symbol(_) | EnumRepr::Unit { .. } => Vec::new(),
                EnumRepr::SingleVariant { fields, .. } if index == 0 => fields.clone(),
                EnumRepr::SingleVariant { .. } => continue,
                EnumRepr::Niche {
                    data_variant,
                    data_fields,
                    ..
                } => {
                    if index == *data_variant {
                        data_fields.clone()
                    } else {
                        Vec::new()
                    }
                }
                EnumRepr::FixedBits { fields, .. } => {
                    fields[index].iter().map(|(_, fty)| fty.clone()).collect()
                }
                EnumRepr::TaggedUnion { fields, .. } => fields[index].clone(),
            };
            if expected_tys.len() != evaluated.len()
                || !expected_tys
                    .iter()
                    .zip(evaluated.iter())
                    .all(|(a, b)| *a == b.ty)
            {
                continue;
            }
            let found = self.fresh();
            let next = self.fresh();
            let cond = self.instruction(
                Ty::Bool,
                format!("icmp eq i{tag_bits} {}, {index}", variant_val.text),
            );
            self.line(format!(
                "br i1 {}, label %{found}, label %{next}",
                cond.text
            ));
            self.label(&found);
            let constructed =
                self.variant_from_values(source, index, evaluated.clone(), ty, false)?;
            self.store(&result_slot, &constructed)?;
            self.branch(&end);
            self.label(&next);
        }
        self.line("unreachable");
        self.terminated = true;
        self.label(&end);
        Ok(self.load(&result_slot))
    }

    fn variant_from_values(
        &mut self,
        source: nir::program::SourceRange,
        index: usize,
        evaluated: Vec<Value>,
        ty: &Term,
        finish_prepared: bool,
    ) -> Result<Value, String> {
        let ty = repr(self.graph, ty, self.checked)?;
        let Ty::Enum(layout) = ty.clone() else {
            return Err("host variant has no enum representation".into());
        };
        if index >= layout.names.len() {
            return Err("host variant index out of range".into());
        }
        match &layout.repr {
            EnumRepr::Symbol(symbols) => {
                if !evaluated.is_empty() {
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
                if !evaluated.is_empty() {
                    return Err("host unit enum cannot carry payloads".into());
                }
                if finish_prepared {
                    self.finish_prepared_construction(source)?;
                }
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
                if index != 0 || evaluated.len() != expected.len() {
                    return Err("host enum payload count mismatch".into());
                }
                if finish_prepared {
                    self.finish_prepared_construction(source)?;
                }
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
                    if !evaluated.is_empty() {
                        return Err("host enum payload count mismatch".into());
                    }
                    if finish_prepared {
                        self.finish_prepared_construction(source)?;
                    }
                    let ord = if index < *data_variant {
                        index
                    } else {
                        index - 1
                    };
                    let code = niche.start + ord as u64;
                    return self.emit_niche_value(ty, carrier, niche, code);
                }
                if evaluated.len() != data_fields.len() {
                    return Err("host enum payload count mismatch".into());
                }
                if finish_prepared {
                    self.finish_prepared_construction(source)?;
                }
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
                    .ok_or("host enum payload layout missing")?;
                if evaluated.len() != payload.len() {
                    return Err("host enum payload count mismatch".into());
                }
                let width = *width;
                let mut result = Value {
                    ty: ty.clone(),
                    text: code.to_string(),
                };
                for (field, (start, _expected)) in evaluated.into_iter().zip(payload.iter()) {
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
                    if *start > 0 {
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
                if finish_prepared {
                    self.finish_prepared_construction(source)?;
                }
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
                    .ok_or("host enum payload layout missing")?;
                if evaluated.len() != payload.len() {
                    return Err("host enum payload count mismatch".into());
                }
                let variant_llvm = layout
                    .variant_record_ty(index)
                    .ok_or("host enum payload layout missing")?
                    .llvm();
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
                    if finish_prepared {
                        self.finish_prepared_construction(source)?;
                    }
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
                if finish_prepared {
                    self.finish_prepared_construction(source)?;
                }
                Ok(self.load(&slot))
            }
        }
    }

    pub(super) fn enum_variant_index(
        &mut self,
        pointer: &Expr,
        result_ty: &Term,
        depth: usize,
    ) -> Result<Value, String> {
        let pointee = pointers::pointee(&pointer.ty)?;
        let ptr_val = self.expr(pointer, depth)?;
        if self.terminated {
            return Ok(Value::unit());
        }
        let usize_ty = repr(self.graph, result_ty, self.checked)?;
        let Ty::Int(usize_bits, false) = usize_ty else {
            return Err("host enum_variant_index result must be unsigned usize".into());
        };
        let enum_ty = repr(self.graph, pointee, self.checked)?;
        let Ty::Enum(layout) = &enum_ty else {
            return Err("host enum_variant_index operand must point to an enum".into());
        };
        match &layout.repr {
            EnumRepr::Unit { width: 0 } | EnumRepr::SingleVariant { .. } => Ok(Value {
                ty: usize_ty,
                text: "0".into(),
            }),
            EnumRepr::Unit { width } => {
                let tag = self.load(&Slot::new(Ty::Int(*width, false), ptr_val.text, None));
                if *width == usize_bits {
                    Ok(tag)
                } else if *width < usize_bits {
                    Ok(self.instruction(
                        usize_ty,
                        format!("zext i{width} {} to i{usize_bits}", tag.text),
                    ))
                } else {
                    Ok(self.instruction(
                        usize_ty,
                        format!("trunc i{width} {} to i{usize_bits}", tag.text),
                    ))
                }
            }
            EnumRepr::TaggedUnion { tag_bits, .. } => {
                let tag_ptr = self.instruction(
                    Ty::Pointer,
                    format!(
                        "getelementptr {}, ptr {}, i32 0, i32 0",
                        enum_ty.llvm(),
                        ptr_val.text
                    ),
                );
                let tag = self.load(&Slot::new(Ty::Int(*tag_bits, false), tag_ptr.text, None));
                if *tag_bits == usize_bits {
                    Ok(tag)
                } else if *tag_bits < usize_bits {
                    Ok(self.instruction(
                        usize_ty,
                        format!("zext i{tag_bits} {} to i{usize_bits}", tag.text),
                    ))
                } else {
                    Ok(self.instruction(
                        usize_ty,
                        format!("trunc i{tag_bits} {} to i{usize_bits}", tag.text),
                    ))
                }
            }
            EnumRepr::Symbol(_) | EnumRepr::Niche { .. } | EnumRepr::FixedBits { .. } => {
                let slot = self.pointer_slot(&ptr_val, pointee)?;
                let loaded = self.load(&slot);
                let result = self.slot(usize_ty.clone());
                let end = self.fresh();
                for index in 0..layout.names.len() {
                    let found = self.fresh();
                    let next = self.fresh();
                    let cond = self.variant_condition(&loaded, index)?;
                    self.line(format!(
                        "br i1 {}, label %{found}, label %{next}",
                        cond.text
                    ));
                    self.label(&found);
                    self.store(
                        &result,
                        &Value {
                            ty: usize_ty.clone(),
                            text: index.to_string(),
                        },
                    )?;
                    self.branch(&end);
                    self.label(&next);
                }
                self.line("unreachable");
                self.terminated = true;
                self.label(&end);
                Ok(self.load(&result))
            }
        }
    }

    pub(super) fn enum_variant_field_ptr(
        &mut self,
        pointer: &Expr,
        variant_expr: &Expr,
        field_expr: &Expr,
        result_ty: &Term,
        depth: usize,
    ) -> Result<Value, String> {
        let pointee = pointers::pointee(&pointer.ty)?;
        let ptr_val = self.expr(pointer, depth)?;
        if self.terminated {
            return Ok(Value::unit());
        }
        let var_val = self.expr(variant_expr, depth)?;
        if self.terminated {
            return Ok(Value::unit());
        }
        let fld_val = self.expr(field_expr, depth)?;
        if self.terminated {
            return Ok(Value::unit());
        }
        let opt_ty = repr(self.graph, result_ty, self.checked)?;
        let enum_ty = repr(self.graph, pointee, self.checked)?;
        let Ty::Enum(layout) = &enum_ty else {
            return Err("host enum_variant_field_ptr requires a pointer to enum".into());
        };
        let Ty::Int(var_bits, _) = var_val.ty else {
            return Err("host enum_variant_field_ptr variant index must be integer".into());
        };
        let Ty::Int(fld_bits, _) = fld_val.ty else {
            return Err("host enum_variant_field_ptr field index must be integer".into());
        };
        let const_v = match &variant_expr.kind {
            K::Literal(nir::program::Literal::UnsignedInteger(v)) => usize::try_from(*v).ok(),
            _ => None,
        };
        let const_f = match &field_expr.kind {
            K::Literal(nir::program::Literal::UnsignedInteger(f)) => usize::try_from(*f).ok(),
            _ => None,
        };
        let result_slot = self.slot(opt_ty.clone());
        let end = self.fresh();

        // Emit tag checks lazily depending on EnumRepr.
        let mut loaded_enum = None;
        let mut loaded_tag = None;
        for v_idx in 0..layout.names.len() {
            if const_v.is_some_and(|v| v != v_idx) {
                continue;
            }
            let field_tys: Vec<Ty> = match &layout.repr {
                EnumRepr::Symbol(_) | EnumRepr::Unit { .. } => Vec::new(),
                EnumRepr::SingleVariant { fields, .. } if v_idx == 0 => fields.clone(),
                EnumRepr::SingleVariant { .. } => Vec::new(),
                EnumRepr::Niche {
                    data_variant,
                    data_fields,
                    ..
                } if v_idx == *data_variant => data_fields.clone(),
                EnumRepr::Niche { .. } => Vec::new(),
                EnumRepr::TaggedUnion { fields, .. } => fields[v_idx].clone(),
                EnumRepr::FixedBits { .. } => {
                    return Err(
                        "host enum_variant_field_ptr cannot address packed bit fields".into(),
                    );
                }
            };
            if field_tys.is_empty() {
                continue;
            }
            for (f_idx, field_ty) in field_tys.iter().enumerate() {
                if const_f.is_some_and(|f| f != f_idx) {
                    continue;
                }
                let tag_cond = match &layout.repr {
                    EnumRepr::SingleVariant { .. } => Value {
                        ty: Ty::Bool,
                        text: "true".into(),
                    },
                    EnumRepr::TaggedUnion { tag_bits, .. } => {
                        let tag = if let Some(tag) = loaded_tag.clone() {
                            tag
                        } else {
                            let tag_ptr = self.instruction(
                                Ty::Pointer,
                                format!(
                                    "getelementptr {}, ptr {}, i32 0, i32 0",
                                    enum_ty.llvm(),
                                    ptr_val.text
                                ),
                            );
                            let t = self.load(&Slot::new(
                                Ty::Int(*tag_bits, false),
                                tag_ptr.text,
                                None,
                            ));
                            loaded_tag = Some(t.clone());
                            t
                        };
                        self.instruction(
                            Ty::Bool,
                            format!("icmp eq i{tag_bits} {}, {v_idx}", tag.text),
                        )
                    }
                    EnumRepr::Niche { .. } => {
                        let val = if let Some(val) = loaded_enum.clone() {
                            val
                        } else {
                            let slot = self.pointer_slot(&ptr_val, pointee)?;
                            let v = self.load(&slot);
                            loaded_enum = Some(v.clone());
                            v
                        };
                        self.variant_condition(&val, v_idx)?
                    }
                    EnumRepr::Symbol(_) | EnumRepr::Unit { .. } | EnumRepr::FixedBits { .. } => {
                        unreachable!()
                    }
                };
                let mut cond = tag_cond;
                if const_v.is_none() {
                    let v_eq = self.instruction(
                        Ty::Bool,
                        format!("icmp eq i{var_bits} {}, {v_idx}", var_val.text),
                    );
                    cond =
                        self.instruction(Ty::Bool, format!("and i1 {}, {}", cond.text, v_eq.text));
                }
                if const_f.is_none() {
                    let f_eq = self.instruction(
                        Ty::Bool,
                        format!("icmp eq i{fld_bits} {}, {f_idx}", fld_val.text),
                    );
                    cond =
                        self.instruction(Ty::Bool, format!("and i1 {}, {}", cond.text, f_eq.text));
                }
                let matched = self.fresh();
                let next = self.fresh();
                self.line(format!(
                    "br i1 {}, label %{matched}, label %{next}",
                    cond.text
                ));
                self.label(&matched);
                let raw_ptr = match &layout.repr {
                    EnumRepr::SingleVariant {
                        unwrapped, carrier, ..
                    }
                    | EnumRepr::Niche {
                        unwrapped, carrier, ..
                    } => {
                        if *unwrapped || *field_ty == Ty::Unit {
                            Value {
                                ty: Ty::Pointer,
                                text: ptr_val.text.clone(),
                            }
                        } else {
                            self.instruction(
                                Ty::Pointer,
                                format!(
                                    "getelementptr {}, ptr {}, i32 0, i32 {f_idx}",
                                    carrier.llvm(),
                                    ptr_val.text
                                ),
                            )
                        }
                    }
                    EnumRepr::TaggedUnion { payload_size, .. } => {
                        if *payload_size == 0 || *field_ty == Ty::Unit {
                            Value {
                                ty: Ty::Pointer,
                                text: ptr_val.text.clone(),
                            }
                        } else {
                            let payload_ptr = self.instruction(
                                Ty::Pointer,
                                format!(
                                    "getelementptr {}, ptr {}, i32 0, i32 1",
                                    enum_ty.llvm(),
                                    ptr_val.text
                                ),
                            );
                            let variant_llvm = layout
                                .variant_record_ty(v_idx)
                                .ok_or("host enum payload layout missing")?
                                .llvm();
                            self.instruction(
                                Ty::Pointer,
                                format!(
                                    "getelementptr {variant_llvm}, ptr {}, i32 0, i32 {f_idx}",
                                    payload_ptr.text
                                ),
                            )
                        }
                    }
                    EnumRepr::Symbol(_) | EnumRepr::Unit { .. } | EnumRepr::FixedBits { .. } => {
                        unreachable!()
                    }
                };
                let present = self.optional_present_value(&opt_ty, raw_ptr)?;
                self.store(&result_slot, &present)?;
                self.branch(&end);
                self.label(&next);
            }
        }
        let absent = self.optional_absent_value(&opt_ty)?;
        self.store(&result_slot, &absent)?;
        self.branch(&end);
        self.label(&end);
        Ok(self.load(&result_slot))
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
                packed: nir::program::PackedMode::Byte,
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
            emitted_effects: attributes::EmittedBodyEffects::default(),
            target_triple: None,
            declarations: &mut declarations,
            text_constants: &mut strings::TextConstants::default(),
            attr_table: None,
            bitvec_cache: None,
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
            emitted_effects: attributes::EmittedBodyEffects::default(),
            target_triple: None,
            declarations: &mut declarations,
            text_constants: &mut strings::TextConstants::default(),
            attr_table: None,
            bitvec_cache: None,
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

    #[test]
    fn variant_set_condition_lowers_contiguous_ranges_for_unit_tagged_niche_and_fixed_bits() {
        use comptime_runtime::{Budget, bitvec::BitVec};
        use nir::enum_encoding::{
            Field as EncField, Layout as EncLayout, Representation, Variant as EncVariant,
        };

        let mut graph = iir::Package::default();
        let (unit_ty, unit_inst) = enum_def(
            1,
            "Unit4",
            vec![
                ("u0", vec![]),
                ("u1", vec![]),
                ("u2", vec![]),
                ("u3", vec![]),
            ],
        );
        let (tagged_ty, tagged_inst) = enum_def(
            2,
            "Tagged4",
            vec![
                ("a", vec![Term::Primitive(P::U64)]),
                ("b", vec![Term::Primitive(P::U32)]),
                ("c", vec![]),
                ("d", vec![]),
            ],
        );
        let (niche_ty, niche_inst) = enum_def(
            3,
            "Niche4",
            vec![
                ("e0", vec![]),
                ("e1", vec![]),
                ("e2", vec![]),
                ("flag", vec![Term::Primitive(P::Bool)]),
            ],
        );

        let fixed_decl = nir::DeclarationId {
            package: 0,
            local: 4,
        };
        let fixed_ty = Term::Nominal {
            declaration: fixed_decl,
            arguments: vec![],
        };
        let mut budget = Budget::default();
        let bv8 = |v: u32, budget: &mut Budget| BitVec::from_words(8, &[v], budget).unwrap();
        let mk_const = |name: &str, val: u32, budget: &mut Budget| {
            (
                nir::program::Variant {
                    name: name.into(),
                    shape: nir::program::VariantShape::Unit,
                    fields: vec![],
                },
                EncVariant::new(
                    name.into(),
                    bv8(0xff, budget),
                    bv8(val, budget),
                    vec![],
                    budget,
                )
                .unwrap(),
            )
        };
        let (c0_v, c0_e) = mk_const("c10", 0x10, &mut budget);
        let (c1_v, c1_e) = mk_const("c11", 0x11, &mut budget);
        let (c2_v, c2_e) = mk_const("c12", 0x12, &mut budget);
        let (c3_v, c3_e) = mk_const("c13", 0x13, &mut budget);
        let p2_v = nir::program::Variant {
            name: "p2".into(),
            shape: nir::program::VariantShape::Named(vec!["x".into()]),
            fields: vec![Term::Primitive(P::BitVec(4))],
        };
        let p2_e = EncVariant::new(
            "p2".into(),
            bv8(0xf0, &mut budget),
            bv8(0x20, &mut budget),
            vec![EncField {
                name: "x".into(),
                start: 0,
                width: 4,
            }],
            &mut budget,
        )
        .unwrap();
        let p3_v = nir::program::Variant {
            name: "p3".into(),
            shape: nir::program::VariantShape::Named(vec!["y".into()]),
            fields: vec![Term::Primitive(P::BitVec(4))],
        };
        let p3_e = EncVariant::new(
            "p3".into(),
            bv8(0xf0, &mut budget),
            bv8(0x30, &mut budget),
            vec![EncField {
                name: "y".into(),
                start: 0,
                width: 4,
            }],
            &mut budget,
        )
        .unwrap();
        let mid_v = nir::program::Variant {
            name: "mid".into(),
            shape: nir::program::VariantShape::Named(vec!["z".into()]),
            fields: vec![Term::Primitive(P::BitVec(4))],
        };
        let mid_e = EncVariant::new(
            "mid".into(),
            bv8(0xc3, &mut budget),
            bv8(0x81, &mut budget),
            vec![EncField {
                name: "z".into(),
                start: 2,
                width: 4,
            }],
            &mut budget,
        )
        .unwrap();
        let enc_layout = EncLayout::new(
            8,
            vec![c0_e, c1_e, c2_e, c3_e, p2_e, p3_e, mid_e],
            &mut budget,
        )
        .unwrap();
        let fixed_inst = iir::NominalInstance {
            ty: fixed_ty.clone(),
            definition: iir::NominalDefinition::Enum(nir::program::Enum {
                is_intrinsic: false,
                parameter_types: vec![],
                capabilities: vec![],
                requirements: vec![],
                type_parameters: vec![],
                declaration: fixed_decl,
                name: "Mixed8".into(),
                groups: vec![],
                encoding: Some(Representation {
                    base_type: Some(P::U8),
                    layout: enc_layout,
                }),
                symbol_encoding: None,
                variants: vec![c0_v, c1_v, c2_v, c3_v, p2_v, p3_v, mid_v],
            }),
        };
        graph.nominals = vec![unit_inst, tagged_inst, niche_inst, fixed_inst];

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
            emitted_effects: attributes::EmittedBodyEffects::default(),
            target_triple: None,
            declarations: &mut declarations,
            text_constants: &mut strings::TextConstants::default(),
            attr_table: None,
            bitvec_cache: None,
            serial: 0,
            terminated: false,
            fuel: &mut fuel,
        };

        let unit_llvm = repr(&graph, &unit_ty, true).unwrap();
        let u_cond = emitter
            .variant_set_condition(
                &Value {
                    ty: unit_llvm,
                    text: "%u".into(),
                },
                &[1, 2],
            )
            .unwrap()
            .unwrap();
        assert!(emitter.body.contains("sub i8 %u, 1"), "{}", emitter.body);
        assert!(emitter.body.contains("icmp ule i8"), "{}", emitter.body);
        assert!(!emitter.body.contains("icmp eq"), "{}", emitter.body);
        let _ = u_cond;

        let body_before_tagged = emitter.body.len();
        let tagged_llvm = repr(&graph, &tagged_ty, true).unwrap();
        let _ = emitter
            .variant_set_condition(
                &Value {
                    ty: tagged_llvm,
                    text: "%tg".into(),
                },
                &[1, 2],
            )
            .unwrap()
            .unwrap();
        let tagged_ir = &emitter.body[body_before_tagged..];
        assert!(
            tagged_ir.matches("extractvalue").count() == 1,
            "{tagged_ir}"
        );
        assert!(tagged_ir.contains("sub i8"), "{tagged_ir}");
        assert!(tagged_ir.contains("icmp ule i8"), "{tagged_ir}");

        let body_before_niche = emitter.body.len();
        let niche_llvm = repr(&graph, &niche_ty, true).unwrap();
        let _ = emitter
            .variant_set_condition(
                &Value {
                    ty: niche_llvm.clone(),
                    text: "%ni".into(),
                },
                &[0, 1],
            )
            .unwrap()
            .unwrap();
        let _ = emitter
            .variant_set_condition(
                &Value {
                    ty: niche_llvm,
                    text: "%ni".into(),
                },
                &[1, 2, 3],
            )
            .unwrap()
            .unwrap();
        let niche_ir = &emitter.body[body_before_niche..];
        assert!(niche_ir.contains("sub i8 %ni, 2"), "{niche_ir}");
        assert!(niche_ir.contains("xor i1"), "{niche_ir}");

        emitter.body.clear();
        emitter.prologue.clear();
        let fixed_llvm = repr(&graph, &fixed_ty, true).unwrap();
        let arg = Value {
            ty: fixed_llvm.clone(),
            text: "%raw".into(),
        };
        let c_range = emitter
            .variant_set_condition(&arg, &[0, 1, 2, 3])
            .unwrap()
            .unwrap();
        assert!(emitter.body.contains("sub i8 %raw, 16"), "{}", emitter.body);
        assert!(emitter.body.contains("icmp ule i8"), "{}", emitter.body);
        assert!(!emitter.body.contains("icmp eq"), "{}", emitter.body);

        let before_prefix = emitter.body.len();
        let p_range = emitter
            .variant_set_condition(&arg, &[4, 5])
            .unwrap()
            .unwrap();
        let prefix_ir = &emitter.body[before_prefix..];
        assert!(prefix_ir.contains("sub i8 %raw, 32"), "{prefix_ir}");
        assert!(prefix_ir.contains("icmp ule i8"), "{prefix_ir}");
        assert!(!prefix_ir.contains("icmp eq"), "{prefix_ir}");

        let before_mixed = emitter.body.len();
        let m_range = emitter
            .variant_set_condition(&arg, &[4, 5, 6])
            .unwrap()
            .unwrap();
        let mixed_ir = &emitter.body[before_mixed..];
        assert!(mixed_ir.contains("sub i8 %raw, 32"), "{mixed_ir}");
        assert!(mixed_ir.contains("and i8 %raw, 195"), "{mixed_ir}");
        assert!(mixed_ir.contains("icmp eq i8"), "{mixed_ir}");
        assert!(mixed_ir.contains("or i1"), "{mixed_ir}");

        let pack = emitter.instruction(
            Ty::Int(8, false),
            format!("select i1 {}, i8 1, i8 0", c_range.text),
        );
        let p_bit = emitter.instruction(
            Ty::Int(8, false),
            format!("select i1 {}, i8 2, i8 0", p_range.text),
        );
        let m_bit = emitter.instruction(
            Ty::Int(8, false),
            format!("select i1 {}, i8 4, i8 0", m_range.text),
        );
        let p01 = emitter.instruction(
            Ty::Int(8, false),
            format!("or i8 {}, {}", pack.text, p_bit.text),
        );
        let all = emitter.instruction(
            Ty::Int(8, false),
            format!("or i8 {}, {}", p01.text, m_bit.text),
        );
        emitter.line(format!("ret i8 {}", all.text));

        let check_cases = [
            (0x0F, 0),
            (0x10, 1),
            (0x13, 1),
            (0x14, 0),
            (0x1F, 0),
            (0x20, 2 | 4),
            (0x3F, 2 | 4),
            (0x40, 0),
            (0x81, 4),
            (0xBD, 4),
            (0x80, 0),
        ];
        let mut checks = String::new();
        for (input, expected) in check_cases {
            checks.push_str(&format!(
                "  %r_{input} = call i8 @classify(i8 {input})\n  %ok_{input} = icmp eq i8 %r_{input}, {expected}\n  br i1 %ok_{input}, label %next_{input}, label %fail\nnext_{input}:\n"
            ));
        }
        let ir = format!(
            "declare void @llvm.trap()\ndefine i8 @classify(i8 %raw) {{\nentry:\n{}{}}}\ndefine i32 @main() {{\nentry:\n{checks}  ret i32 0\nfail:\n  call void @llvm.trap()\n  unreachable\n}}\n",
            emitter.prologue, emitter.body
        );
        for optimization in [
            crate::host::OptimizationLevel::O0,
            crate::host::OptimizationLevel::O3,
        ] {
            let toolchain = crate::host::Toolchain {
                optimization,
                ..Default::default()
            };
            let output = toolchain.execute_test_ir(&ir).unwrap();
            assert!(output.status.success(), "{optimization:?}: {ir}");
        }
    }
}
