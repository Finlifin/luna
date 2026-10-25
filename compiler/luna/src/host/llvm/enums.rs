use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct EnumLayout {
    pub names: Vec<String>,
    pub width: u32,
    pub symbols: Option<Vec<String>>,
    pub aggregate: bool,
    pub codes: Vec<String>,
    pub masks: Vec<String>,
    pub fields: Vec<Vec<(usize, Ty)>>,
}
impl EnumLayout {
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
        if let Some(encoding) = &definition.symbol_encoding {
            // Symbols have no integer encoding or auxiliary discriminant.
            // Private host storage holds immutable UTF-8 bytes and their length;
            // equality compares content, never allocation addresses.
            return Ok(Self {
                names: definition.variants.iter().map(|v| v.name.clone()).collect(),
                width: 0,
                symbols: Some(
                    encoding
                        .variants()
                        .iter()
                        .map(|v| v.value.clone())
                        .collect(),
                ),
                aggregate: false,
                codes: Vec::new(),
                masks: Vec::new(),
                fields: vec![Vec::new(); definition.variants.len()],
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
            Ok(Self {
                names: definition.variants.iter().map(|v| v.name.clone()).collect(),
                symbols: None,
                width: encoding.width() as u32,
                aggregate: false,
                codes,
                masks,
                fields,
            })
        } else {
            // Ordinary enum has no specified bit encoding. This private host ABI
            // uses a u32 discriminant; configured enums never enter this branch.
            u32::try_from(definition.variants.len()).map_err(|_| "too many host enum variants")?;
            Ok(Self {
                names: definition.variants.iter().map(|v| v.name.clone()).collect(),
                symbols: None,
                width: 32,
                aggregate: definition.variants.iter().any(|v| !v.fields.is_empty()),
                masks: vec![u32::MAX.to_string(); definition.variants.len()],
                fields: definition
                    .variants
                    .iter()
                    .map(|v| {
                        v.fields
                            .iter()
                            .map(|ty| Ok((0, repr_at(graph, ty, depth + 1, checked, remaining)?)))
                            .collect::<Result<Vec<_>, String>>()
                    })
                    .collect::<Result<Vec<_>, String>>()?,
                codes: (0..definition.variants.len())
                    .map(|i| i.to_string())
                    .collect(),
            })
        }
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
        let Ty::Enum(layout) = &ty else {
            return Err("host variant has no enum representation".into());
        };
        if let Some(symbols) = &layout.symbols {
            if !fields.is_empty() {
                return Err("host Symbol enum cannot carry payloads".into());
            }
            let symbol = symbols
                .get(index)
                .ok_or("host Symbol variant index out of range")?;
            let mut value = self.text_literal(symbol, Ty::String)?;
            value.ty = ty;
            return Ok(value);
        }
        let code = layout
            .codes
            .get(index)
            .ok_or("host variant index out of range")?;
        let payload = layout
            .fields
            .get(index)
            .ok_or("host enum payload layout missing")?
            .clone();
        if fields.len() != payload.len() {
            return Err("host enum payload count mismatch".into());
        }
        if layout.aggregate {
            let mut result = self.instruction(
                ty.clone(),
                format!("insertvalue {} zeroinitializer, i32 {index}, 0", ty.llvm()),
            );
            for (field_index, (field, (_, expected))) in fields.iter().zip(payload).enumerate() {
                let field_source = field.source;
                let field = self.expr(field, depth)?;
                if self.terminated {
                    return Ok(Value::unit());
                }
                if field.ty != expected {
                    return Err("host enum payload representation mismatch".into());
                }
                self.retain_prepared_field(source, field_index, field_source, &field)?;
                let (storage, text) = if field.ty == Ty::Unit {
                    ("{}".into(), "zeroinitializer".into())
                } else {
                    (field.ty.llvm(), field.text)
                };
                result = self.instruction(
                    ty.clone(),
                    format!(
                        "insertvalue {} {}, {storage} {text}, {}, {field_index}",
                        ty.llvm(),
                        result.text,
                        index + 1
                    ),
                );
            }
            self.finish_prepared_construction(source)?;
            return Ok(result);
        }
        let width = layout.width;
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
    pub(super) fn variant_field(
        &mut self,
        value: &Value,
        index: usize,
        field: usize,
    ) -> Result<Value, String> {
        let Ty::Enum(layout) = &value.ty else {
            return Err("host payload subject is not enum".into());
        };
        let (start, ty) = layout
            .fields
            .get(index)
            .and_then(|fields| fields.get(field))
            .cloned()
            .ok_or("host enum field index out of range")?;
        if layout.aggregate {
            if ty == Ty::Unit {
                return Ok(Value::unit());
            }
            return Ok(self.instruction(
                ty,
                format!(
                    "extractvalue {} {}, {}, {field}",
                    value.ty.llvm(),
                    value.text,
                    index + 1
                ),
            ));
        }
        let width = layout.width;
        let field_width = bit_width(&ty).ok_or("host enum payload is not a bit representation")?;
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
            bits = self.instruction(bits.ty, format!("lshr i{width} {}, {start}", bits.text));
        }
        if field_width < width {
            bits = self.instruction(
                Ty::Int(field_width, false),
                format!("trunc i{width} {} to i{field_width}", bits.text),
            );
        }
        self.from_bits(bits, ty)
    }

    pub(super) fn variant_condition(
        &mut self,
        value: &Value,
        index: usize,
    ) -> Result<Value, String> {
        let Ty::Enum(layout) = &value.ty else {
            return Err("host variant pattern has no enum representation".into());
        };
        if let Some(symbols) = &layout.symbols {
            let symbol = symbols
                .get(index)
                .ok_or("host Symbol pattern index out of range")?;
            let expected = self.text_literal(symbol, Ty::String)?;
            let actual = Value {
                ty: Ty::String,
                text: value.text.clone(),
            };
            return Ok(self.string_equal(&actual, &expected));
        }
        if layout.aggregate {
            let tag = self.instruction(
                Ty::Int(32, false),
                format!("extractvalue {} {}, 0", value.ty.llvm(), value.text),
            );
            return Ok(self.instruction(Ty::Bool, format!("icmp eq i32 {}, {index}", tag.text)));
        }
        let code = layout
            .codes
            .get(index)
            .ok_or("host variant pattern index out of range")?;
        let mask = &layout.masks[index];
        let masked = self.instruction(
            Ty::Int(layout.width, false),
            format!("and {} {}, {mask}", value.ty.llvm(), value.text),
        );
        Ok(self.instruction(
            Ty::Bool,
            format!("icmp eq {} {}, {code}", value.ty.llvm(), masked.text),
        ))
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
