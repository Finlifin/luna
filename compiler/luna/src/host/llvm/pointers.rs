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
        let ty = repr(self.graph, pointee, self.checked)?;
        let align = matches!(ty, Ty::PackedRecord(_)).then_some(1);
        Ok(Slot {
            ty,
            pointer: pointer.text.clone(),
            align,
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
        let (Ty::Record(fields) | Ty::PackedRecord(fields)) = &slot.ty else {
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

    pub(super) fn const_address(
        &mut self,
        declaration: nir::DeclarationId,
        value: &Expr,
    ) -> Result<Value, String> {
        if !self.checked {
            return Err("host data pointers require paired Copy/Drop resource verification".into());
        }
        let pointer_bits = self.graph.target.map_or(64, |t| t.pointer_bits());
        let const_val = self.const_eval(value, &BTreeMap::new(), 0)?;
        let (llvm_ty, llvm_init, align) = const_val.to_llvm_global(pointer_bits, self.fuel)?;
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let mut feed = |bytes: &[u8]| {
            for &byte in bytes {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0100_0000_01b3);
            }
            hash ^= 0xff;
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        };
        feed(&declaration.package.to_le_bytes());
        feed(&declaration.local.to_le_bytes());
        feed(llvm_ty.as_bytes());
        feed(llvm_init.as_bytes());
        feed(&align.to_le_bytes());
        let symbol = format!(
            "flurry_const_{}_{}_{hash:016x}",
            declaration.package, declaration.local
        );
        self.declarations.insert(format!(
            "@{symbol} = private unnamed_addr constant {llvm_ty} {llvm_init}, align {align}"
        ));
        Ok(Value {
            ty: Ty::Pointer,
            text: format!("@{symbol}"),
        })
    }

    fn const_eval(
        &mut self,
        expr: &Expr,
        env: &BTreeMap<LocalId, ConstValue>,
        depth: usize,
    ) -> Result<ConstValue, String> {
        if depth >= 128 {
            return Err("host constant initializer nesting budget exhausted".into());
        }
        *self.fuel = self
            .fuel
            .checked_sub(1)
            .ok_or("host constant initializer budget exhausted")?;
        let ty = repr(self.graph, &expr.ty, self.checked)?;
        match &expr.kind {
            K::Unit => {
                if ty != Ty::Unit {
                    return Err("host unit constant representation mismatch".into());
                }
                Ok(ConstValue::Unit)
            }
            K::Literal(literal) => {
                if ty == Ty::Int(0, false) {
                    let _ = self.literal(literal, ty)?;
                    return Ok(ConstValue::Atom {
                        ty: Ty::Int(0, false),
                        llvm_ty: "{}".into(),
                        llvm_init: "zeroinitializer".into(),
                        zero_sized: true,
                    });
                }
                let value = self.literal(literal, ty)?;
                Ok(ConstValue::Atom {
                    llvm_ty: value.ty.llvm(),
                    ty: value.ty,
                    llvm_init: value.text,
                    zero_sized: false,
                })
            }
            K::Local(binding) => env
                .get(binding)
                .cloned()
                .ok_or_else(|| "host constant initializer references unbound local".into()),
            K::Field { value, index, .. } | K::AnonymousField { value, index, .. } => {
                let ConstValue::Record { fields, .. } = self.const_eval(value, env, depth + 1)?
                else {
                    return Err("host constant field projection requires a record value".into());
                };
                fields
                    .get(*index)
                    .cloned()
                    .ok_or_else(|| "host constant field index out of range".into())
            }
            K::Block {
                statements,
                result: Some(result),
            }
            | K::CallPreparation {
                statements,
                result: Some(result),
            } => {
                let mut inner_env = env.clone();
                for statement in statements {
                    let K::Let { binding, value } = &statement.kind else {
                        return Err(
                            "host constant initializer contains non-binding statement".into()
                        );
                    };
                    let val = self.const_eval(value, &inner_env, depth + 1)?;
                    inner_env.insert(*binding, val);
                }
                self.const_eval(result, &inner_env, depth + 1)
            }
            K::Call {
                callee,
                arguments,
                env_arguments,
                ..
            } => {
                if !arguments.is_empty() || !env_arguments.is_empty() {
                    return Err(
                        "host constant call requires a specialized zero-argument target".into(),
                    );
                }
                let key = match &callee.kind {
                    K::Function(declaration) => iir::InstanceKey {
                        declaration: *declaration,
                        arguments: vec![].into(),
                    },
                    K::FunctionInstance {
                        declaration,
                        arguments,
                    } => iir::InstanceKey {
                        declaration: *declaration,
                        arguments: arguments.clone().into(),
                    },
                    _ => {
                        return Err(
                            "host constant call requires a concrete specialized function".into(),
                        );
                    }
                };
                let target = self
                    .graph
                    .function(&key)
                    .ok_or("host constant call target instance missing")?;
                if !target.function.parameters.is_empty()
                    || !target.function.env_parameters.is_empty()
                    || !target.function.preconditions.is_empty()
                    || !target.function.postconditions.is_empty()
                    || !target.function.entry_snapshots.is_empty()
                    || !matches!(target.function.signature, Term::Function { .. })
                    || !effects::row_result(&target.function.signature).0.is_empty()
                {
                    return Err("host constant call target must be pure and zero-argument".into());
                }
                if repr(self.graph, &target.function.body.ty, self.checked)? != ty {
                    return Err("host constant call representation mismatch".into());
                }
                self.const_eval(&target.function.body, &BTreeMap::new(), depth + 1)
            }
            K::NewtypeConvert(inner) | K::UnsafeScope(inner) | K::Snapshot(inner) => {
                if repr(self.graph, &inner.ty, self.checked)? != ty {
                    return Err("host constant conversion changes representation".into());
                }
                self.const_eval(inner, env, depth + 1)
            }
            K::ConstAddress { declaration, value } => {
                if ty != Ty::Pointer {
                    return Err("host constant address representation mismatch".into());
                }
                let ptr = self.const_address(*declaration, value)?;
                Ok(ConstValue::Atom {
                    ty: Ty::Pointer,
                    llvm_ty: "ptr".into(),
                    llvm_init: ptr.text,
                    zero_sized: false,
                })
            }
            K::Tuple(fields) | K::AnonymousRecord { fields } | K::Record { fields, .. } => {
                if ty == Ty::Unit && fields.is_empty() {
                    return Ok(ConstValue::Unit);
                }
                let (Ty::Record(layout) | Ty::PackedRecord(layout)) = &ty else {
                    return Err("host constant record lacks a record layout".into());
                };
                if fields.len() != layout.len() {
                    return Err("host constant record field count mismatch".into());
                }
                let mut evaluated = Vec::with_capacity(fields.len());
                for (field, expected) in fields.iter().zip(layout.iter()) {
                    if repr(self.graph, &field.ty, self.checked)? != *expected {
                        return Err("host constant record field representation mismatch".into());
                    }
                    evaluated.push(self.const_eval(field, env, depth + 1)?);
                }
                Ok(ConstValue::Record {
                    packed: matches!(ty, Ty::PackedRecord(_)),
                    llvm_ty: ty.llvm(),
                    ty,
                    fields: evaluated,
                })
            }
            K::OptionalAbsent => {
                if !matches!(ty, Ty::Optional(_)) {
                    return Err("host optional constant lacks optional layout".into());
                }
                Ok(ConstValue::Optional {
                    llvm_ty: ty.llvm(),
                    ty,
                    payload: None,
                })
            }
            K::OptionalPresent(payload) => {
                let Ty::Optional(expected) = &ty else {
                    return Err("host optional constant lacks optional layout".into());
                };
                if payload.ty == Term::Primitive(P::Never)
                    || repr(self.graph, &payload.ty, self.checked)? != **expected
                {
                    return Err("host optional constant payload representation mismatch".into());
                }
                let inner = self.const_eval(payload, env, depth + 1)?;
                Ok(ConstValue::Optional {
                    llvm_ty: ty.llvm(),
                    ty,
                    payload: Some(Box::new(inner)),
                })
            }
            K::Variant { index, fields, .. } => {
                let Ty::Enum(layout) = &ty else {
                    return Err("host variant constant lacks enum layout".into());
                };
                if let Some(symbols) = &layout.symbols {
                    if !fields.is_empty() {
                        return Err("host Symbol enum cannot carry payloads".into());
                    }
                    let symbol = symbols
                        .get(*index)
                        .ok_or("host Symbol variant index out of range")?;
                    let value = self.text_literal(symbol, Ty::String)?;
                    return Ok(ConstValue::Enum {
                        llvm_ty: ty.llvm(),
                        ty,
                        llvm_init: value.text,
                    });
                }
                let code = layout
                    .codes
                    .get(*index)
                    .ok_or("host variant index out of range")?;
                let expected_fields = layout
                    .fields
                    .get(*index)
                    .ok_or("host enum payload layout missing")?;
                if fields.len() != expected_fields.len() {
                    return Err("host enum payload count mismatch".into());
                }
                if layout.aggregate {
                    let mut evaluated = Vec::with_capacity(fields.len());
                    for (field, (_, expected)) in fields.iter().zip(expected_fields.iter()) {
                        if repr(self.graph, &field.ty, self.checked)? != *expected {
                            return Err("host enum payload representation mismatch".into());
                        }
                        evaluated.push(self.const_eval(field, env, depth + 1)?);
                    }
                    return Ok(ConstValue::AggregateEnum {
                        ty,
                        variant: *index,
                        fields: evaluated,
                    });
                }
                if fields.is_empty() {
                    let llvm_init = code.clone();
                    return Ok(ConstValue::Enum {
                        llvm_ty: ty.llvm(),
                        ty,
                        llvm_init,
                    });
                }
                if layout.width > 128 {
                    return Err(
                        "host constant bit-packed enum exceeds 128-bit initializer budget".into(),
                    );
                }
                let mut combined = u128::from_str_radix(code, 10)
                    .map_err(|_| "host bit-packed enum discriminant is not an integer")?;
                for (field, (start, expected)) in fields.iter().zip(expected_fields.iter()) {
                    if repr(self.graph, &field.ty, self.checked)? != *expected {
                        return Err("host enum payload representation mismatch".into());
                    }
                    let (_, fi) = self.const_eval(field, env, depth + 1)?.to_llvm(false);
                    let Some(field_width) = bits::bit_width(expected) else {
                        return Err("host constant bit-packed enum payload lacks bit width".into());
                    };
                    if field_width == 0 {
                        continue;
                    }
                    if field_width > 128 || *start + (field_width as usize) > 128 {
                        return Err("host constant bit-packed enum field exceeds 128 bits".into());
                    }
                    let raw = if fi == "true" {
                        1u128
                    } else if fi == "false" {
                        0u128
                    } else if let Ok(v) = fi.parse::<u128>() {
                        v
                    } else if let Ok(v) = fi.parse::<i128>() {
                        v as u128
                    } else {
                        return Err(
                            "host constant bit-packed enum payload requires integer initializer"
                                .into(),
                        );
                    };
                    let mask = if field_width == 128 {
                        u128::MAX
                    } else {
                        (1u128 << field_width) - 1
                    };
                    combined |= (raw & mask) << *start;
                }
                Ok(ConstValue::Enum {
                    llvm_ty: ty.llvm(),
                    ty,
                    llvm_init: combined.to_string(),
                })
            }
            _ => Err("host constant address initializer contains dynamic expression".into()),
        }
    }
}

fn push_zero_padding(bytes: u64, types: &mut Vec<String>, inits: &mut Vec<String>) {
    if bytes > 0 {
        let pad_ty = format!("[{bytes} x i8]");
        inits.push(format!("{pad_ty} zeroinitializer"));
        types.push(pad_ty);
    }
}

#[derive(Clone)]
enum ConstValue {
    Unit,
    Atom {
        ty: Ty,
        llvm_ty: String,
        llvm_init: String,
        zero_sized: bool,
    },
    Record {
        ty: Ty,
        packed: bool,
        llvm_ty: String,
        fields: Vec<ConstValue>,
    },
    Optional {
        ty: Ty,
        llvm_ty: String,
        payload: Option<Box<ConstValue>>,
    },
    Enum {
        ty: Ty,
        llvm_ty: String,
        llvm_init: String,
    },
    AggregateEnum {
        ty: Ty,
        variant: usize,
        fields: Vec<ConstValue>,
    },
}

impl ConstValue {
    fn ty(&self) -> &Ty {
        match self {
            Self::Unit => &Ty::Unit,
            Self::Atom { ty, .. }
            | Self::Record { ty, .. }
            | Self::Optional { ty, .. }
            | Self::Enum { ty, .. }
            | Self::AggregateEnum { ty, .. } => ty,
        }
    }

    fn contains_aggregate_enum(&self) -> bool {
        match self {
            Self::Unit | Self::Atom { .. } | Self::Enum { .. } => false,
            Self::AggregateEnum { .. } => true,
            Self::Record { fields, .. } => fields.iter().any(Self::contains_aggregate_enum),
            Self::Optional { payload, .. } => payload
                .as_deref()
                .is_some_and(Self::contains_aggregate_enum),
        }
    }

    fn to_llvm(&self, top_level: bool) -> (String, String) {
        match self {
            Self::Unit => {
                if top_level {
                    ("i8".into(), "0".into())
                } else {
                    ("{}".into(), "zeroinitializer".into())
                }
            }
            Self::Atom {
                llvm_ty,
                llvm_init,
                zero_sized,
                ..
            } => {
                if top_level && *zero_sized {
                    ("i8".into(), "0".into())
                } else {
                    (llvm_ty.clone(), llvm_init.clone())
                }
            }
            Self::Record {
                packed,
                llvm_ty,
                fields,
                ..
            } => {
                if fields.is_empty() {
                    if top_level {
                        ("i8".into(), "0".into())
                    } else {
                        (llvm_ty.clone(), "zeroinitializer".into())
                    }
                } else {
                    let parts = fields
                        .iter()
                        .map(|field| {
                            let (ft, fi) = field.to_llvm(false);
                            format!("{ft} {fi}")
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    let init = if *packed {
                        format!("<{{ {parts} }}>")
                    } else {
                        format!("{{ {parts} }}")
                    };
                    (llvm_ty.clone(), init)
                }
            }
            Self::Optional {
                llvm_ty, payload, ..
            } => match payload {
                None => (llvm_ty.clone(), "zeroinitializer".into()),
                Some(inner) => {
                    let (pt, pi) = inner.to_llvm(false);
                    (llvm_ty.clone(), format!("{{ i1 true, {pt} {pi} }}"))
                }
            },
            Self::Enum {
                llvm_ty, llvm_init, ..
            } => (llvm_ty.clone(), llvm_init.clone()),
            Self::AggregateEnum { ty, .. } => (ty.llvm(), "zeroinitializer".into()),
        }
    }

    fn to_llvm_global(
        &self,
        pointer_bits: u32,
        fuel: &mut usize,
    ) -> Result<(String, String, u64), String> {
        let root_layout = self.ty().storage_layout_at(pointer_bits, 0, fuel)?;
        if !self.contains_aggregate_enum() {
            let (llvm_ty, llvm_init) = self.to_llvm(true);
            return Ok((llvm_ty, llvm_init, root_layout.align));
        }
        let mut types = Vec::new();
        let mut inits = Vec::new();
        let written = self.append_packed_parts(pointer_bits, 0, fuel, &mut types, &mut inits)?;
        if written != root_layout.size {
            return Err("host constant initializer layout mismatch".into());
        }
        if types.is_empty() {
            Ok(("i8".into(), "0".into(), 1))
        } else {
            Ok((
                format!("<{{ {} }}>", types.join(", ")),
                format!("<{{ {} }}>", inits.join(", ")),
                root_layout.align,
            ))
        }
    }

    fn append_packed_parts(
        &self,
        pointer_bits: u32,
        depth: usize,
        fuel: &mut usize,
        types: &mut Vec<String>,
        inits: &mut Vec<String>,
    ) -> Result<u64, String> {
        if depth >= 128 {
            return Err("host constant initializer nesting budget exhausted".into());
        }
        match self {
            Self::Unit => Ok(0),
            Self::Atom {
                ty,
                llvm_ty,
                llvm_init,
                zero_sized,
            } => {
                let layout = ty.storage_layout_at(pointer_bits, 0, fuel)?;
                if *zero_sized || layout.size == 0 {
                    return Ok(0);
                }
                types.push(llvm_ty.clone());
                inits.push(format!("{llvm_ty} {llvm_init}"));
                Ok(layout.size)
            }
            Self::Enum {
                ty,
                llvm_ty,
                llvm_init,
            } => {
                let layout = ty.storage_layout_at(pointer_bits, 0, fuel)?;
                if layout.size == 0 {
                    return Ok(0);
                }
                types.push(llvm_ty.clone());
                inits.push(format!("{llvm_ty} {llvm_init}"));
                Ok(layout.size)
            }
            Self::Record {
                ty, packed, fields, ..
            } => {
                let total = ty.storage_layout_at(pointer_bits, 0, fuel)?;
                if fields.is_empty() {
                    return Ok(total.size);
                }
                let mut offset = 0u64;
                for field in fields {
                    let f_layout = field.ty().storage_layout_at(pointer_bits, 0, fuel)?;
                    if !*packed {
                        let aligned = nir::layout::align_up(offset, f_layout.align)
                            .map_err(|e| e.to_string())?;
                        push_zero_padding(aligned - offset, types, inits);
                        offset = aligned;
                    }
                    let written =
                        field.append_packed_parts(pointer_bits, depth + 1, fuel, types, inits)?;
                    offset = offset
                        .checked_add(written)
                        .ok_or("host constant initializer size overflow")?;
                }
                let tail = total
                    .size
                    .checked_sub(offset)
                    .ok_or("host constant initializer layout mismatch")?;
                push_zero_padding(tail, types, inits);
                Ok(total.size)
            }
            Self::Optional { ty, payload, .. } => {
                let total = ty.storage_layout_at(pointer_bits, 0, fuel)?;
                match payload {
                    None => {
                        push_zero_padding(total.size, types, inits);
                        Ok(total.size)
                    }
                    Some(inner) => {
                        types.push("i1".into());
                        inits.push("i1 true".into());
                        let mut offset = 1u64;
                        let inner_layout = inner.ty().storage_layout_at(pointer_bits, 0, fuel)?;
                        let aligned = nir::layout::align_up(offset, inner_layout.align)
                            .map_err(|e| e.to_string())?;
                        push_zero_padding(aligned - offset, types, inits);
                        offset = aligned;
                        let written = inner.append_packed_parts(
                            pointer_bits,
                            depth + 1,
                            fuel,
                            types,
                            inits,
                        )?;
                        offset = offset
                            .checked_add(written)
                            .ok_or("host constant initializer size overflow")?;
                        let tail = total
                            .size
                            .checked_sub(offset)
                            .ok_or("host constant initializer layout mismatch")?;
                        push_zero_padding(tail, types, inits);
                        Ok(total.size)
                    }
                }
            }
            Self::AggregateEnum {
                ty,
                variant,
                fields,
            } => {
                let Ty::Enum(layout) = ty else {
                    return Err("host aggregate enum constant lacks enum layout".into());
                };
                let total = ty.storage_layout_at(pointer_bits, 0, fuel)?;
                let tag_layout =
                    nir::layout::integer_layout(layout.tag_bits).map_err(|e| e.to_string())?;
                types.push(format!("i{}", layout.tag_bits));
                inits.push(format!("i{} {variant}", layout.tag_bits));
                let payload_offset = nir::layout::align_up(tag_layout.size, layout.payload_align)
                    .map_err(|e| e.to_string())?;
                push_zero_padding(payload_offset - tag_layout.size, types, inits);
                let mut v_offset = 0u64;
                for field in fields {
                    let f_layout = field.ty().storage_layout_at(pointer_bits, 0, fuel)?;
                    let aligned = nir::layout::align_up(v_offset, f_layout.align)
                        .map_err(|e| e.to_string())?;
                    push_zero_padding(aligned - v_offset, types, inits);
                    v_offset = aligned;
                    let written =
                        field.append_packed_parts(pointer_bits, depth + 1, fuel, types, inits)?;
                    v_offset = v_offset
                        .checked_add(written)
                        .ok_or("host constant initializer size overflow")?;
                }
                let total_written = payload_offset
                    .checked_add(v_offset)
                    .ok_or("host constant initializer size overflow")?;
                let tail = total
                    .size
                    .checked_sub(total_written)
                    .ok_or("host constant initializer layout mismatch")?;
                push_zero_padding(tail, types, inits);
                Ok(total.size)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn const_address_emits_byte_exact_packed_initializers_for_aggregate_enums() {
        let mut graph = iir::Package::default();
        let enum_decl = nir::DeclarationId {
            package: 0,
            local: 1,
        };
        let enum_ty = Term::Nominal {
            declaration: enum_decl,
            arguments: vec![],
        };
        let enum_inst = iir::NominalInstance {
            ty: enum_ty.clone(),
            definition: iir::NominalDefinition::Enum(nir::program::Enum {
                is_intrinsic: false,
                parameter_types: vec![],
                capabilities: vec![],
                requirements: vec![],
                type_parameters: vec![],
                declaration: enum_decl,
                name: "Rich".into(),
                groups: vec![],
                encoding: None,
                symbol_encoding: None,
                variants: vec![
                    nir::program::Variant {
                        name: "none".into(),
                        shape: nir::program::VariantShape::Unit,
                        fields: vec![],
                    },
                    nir::program::Variant {
                        name: "text".into(),
                        shape: nir::program::VariantShape::Tuple,
                        fields: vec![Term::Primitive(P::Str), Term::Primitive(P::U64)],
                    },
                    nir::program::Variant {
                        name: "triple".into(),
                        shape: nir::program::VariantShape::Tuple,
                        fields: vec![
                            Term::Primitive(P::U8),
                            Term::Primitive(P::U64),
                            Term::Primitive(P::U8),
                        ],
                    },
                ],
            }),
        };
        let wrap_decl = nir::DeclarationId {
            package: 0,
            local: 2,
        };
        let wrap_ty = Term::Nominal {
            declaration: wrap_decl,
            arguments: vec![],
        };
        let wrap_inst = iir::NominalInstance {
            ty: wrap_ty.clone(),
            definition: iir::NominalDefinition::Struct(nir::program::Struct {
                declaration: wrap_decl,
                name: "Wrap".into(),
                type_parameters: vec![],
                parameter_types: vec![],
                requirements: vec![],
                capabilities: vec![],
                fields: vec![
                    ("lead".into(), Term::Primitive(P::U8)),
                    ("item".into(), enum_ty.clone()),
                    ("tail".into(), Term::Primitive(P::U8)),
                ],
                packed: false,
            }),
        };
        graph.nominals = vec![enum_inst, wrap_inst];

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
            serial: 0,
            terminated: false,
            fuel: &mut fuel,
        };

        let src = nir::program::SourceRange::generated(0);
        let const_text_enum = Expr {
            ty: enum_ty.clone(),
            source: src,
            kind: K::Variant {
                declaration: enum_decl,
                index: 1,
                fields: vec![
                    Expr {
                        ty: Term::Primitive(P::Str),
                        source: src,
                        kind: K::Literal(Literal::String("union-const".into())),
                    },
                    Expr {
                        ty: Term::Primitive(P::U64),
                        source: src,
                        kind: K::Literal(Literal::UnsignedInteger(777)),
                    },
                ],
            },
        };
        let p_enum = emitter
            .const_address(
                nir::DeclarationId {
                    package: 0,
                    local: 10,
                },
                &const_text_enum,
            )
            .unwrap();

        let const_wrap = Expr {
            ty: wrap_ty,
            source: src,
            kind: K::Record {
                declaration: wrap_decl,
                fields: vec![
                    Expr {
                        ty: Term::Primitive(P::U8),
                        source: src,
                        kind: K::Literal(Literal::UnsignedInteger(11)),
                    },
                    Expr {
                        ty: enum_ty.clone(),
                        source: src,
                        kind: K::Variant {
                            declaration: enum_decl,
                            index: 2,
                            fields: vec![
                                Expr {
                                    ty: Term::Primitive(P::U8),
                                    source: src,
                                    kind: K::Literal(Literal::UnsignedInteger(3)),
                                },
                                Expr {
                                    ty: Term::Primitive(P::U64),
                                    source: src,
                                    kind: K::Literal(Literal::UnsignedInteger(999)),
                                },
                                Expr {
                                    ty: Term::Primitive(P::U8),
                                    source: src,
                                    kind: K::Literal(Literal::UnsignedInteger(7)),
                                },
                            ],
                        },
                    },
                    Expr {
                        ty: Term::Primitive(P::U8),
                        source: src,
                        kind: K::Literal(Literal::UnsignedInteger(22)),
                    },
                ],
            },
        };
        let p_wrap = emitter
            .const_address(
                nir::DeclarationId {
                    package: 0,
                    local: 11,
                },
                &const_wrap,
            )
            .unwrap();

        let const_opt = Expr {
            ty: Term::Optional(Box::new(enum_ty.clone())),
            source: src,
            kind: K::OptionalPresent(Box::new(Expr {
                ty: enum_ty,
                source: src,
                kind: K::Variant {
                    declaration: enum_decl,
                    index: 0,
                    fields: vec![],
                },
            })),
        };
        let p_opt = emitter
            .const_address(
                nir::DeclarationId {
                    package: 0,
                    local: 12,
                },
                &const_opt,
            )
            .unwrap();

        let decls = emitter
            .declarations
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        assert!(decls.contains(", align 8"), "{decls}");
        assert!(
            decls.contains("<{ i8, [7 x i8], { ptr, i64 }, i64 }>"),
            "{decls}"
        );
        assert!(
            decls.contains(
                "<{ i8, [7 x i8], i8, [7 x i8], i8, [7 x i8], i64, i8, [7 x i8], i8, [7 x i8] }>"
            ),
            "{decls}"
        );
        assert!(
            decls.contains("<{ i1, [7 x i8], i8, [7 x i8], [24 x i8] }>"),
            "{decls}"
        );

        let enum_llvm = repr(&graph, &const_text_enum.ty, true).unwrap();
        let loaded_enum = emitter.load(&Slot {
            ty: enum_llvm.clone(),
            pointer: p_enum.text,
            align: None,
        });
        let cond_enum = emitter.variant_condition(&loaded_enum, 1).unwrap();
        emitter.guard(&cond_enum).unwrap();
        let f_num = emitter.variant_field(&loaded_enum, 1, 1).unwrap();
        let eq_enum = emitter.instruction(Ty::Bool, format!("icmp eq i64 {}, 777", f_num.text));
        emitter.guard(&eq_enum).unwrap();

        let wrap_llvm = repr(&graph, &const_wrap.ty, true).unwrap();
        let loaded_wrap = emitter.load(&Slot {
            ty: wrap_llvm,
            pointer: p_wrap.text,
            align: None,
        });
        let wrap_lead = emitter.record_field(&loaded_wrap, 0).unwrap();
        let eq_lead = emitter.instruction(Ty::Bool, format!("icmp eq i8 {}, 11", wrap_lead.text));
        emitter.guard(&eq_lead).unwrap();
        let wrap_item = emitter.record_field(&loaded_wrap, 1).unwrap();
        let cond_item = emitter.variant_condition(&wrap_item, 2).unwrap();
        emitter.guard(&cond_item).unwrap();
        let item_0 = emitter.variant_field(&wrap_item, 2, 0).unwrap();
        let item_1 = emitter.variant_field(&wrap_item, 2, 1).unwrap();
        let item_2 = emitter.variant_field(&wrap_item, 2, 2).unwrap();
        let eq_i0 = emitter.instruction(Ty::Bool, format!("icmp eq i8 {}, 3", item_0.text));
        let eq_i1 = emitter.instruction(Ty::Bool, format!("icmp eq i64 {}, 999", item_1.text));
        let eq_i2 = emitter.instruction(Ty::Bool, format!("icmp eq i8 {}, 7", item_2.text));
        emitter.guard(&eq_i0).unwrap();
        emitter.guard(&eq_i1).unwrap();
        emitter.guard(&eq_i2).unwrap();
        let wrap_tail = emitter.record_field(&loaded_wrap, 2).unwrap();
        let eq_tail = emitter.instruction(Ty::Bool, format!("icmp eq i8 {}, 22", wrap_tail.text));
        emitter.guard(&eq_tail).unwrap();

        let opt_llvm = repr(&graph, &const_opt.ty, true).unwrap();
        let loaded_opt = emitter.load(&Slot {
            ty: opt_llvm,
            pointer: p_opt.text,
            align: None,
        });
        let opt_tag = emitter.optional_tag(&loaded_opt).unwrap();
        emitter.guard(&opt_tag).unwrap();
        let opt_val = emitter.optional_payload(&loaded_opt).unwrap();
        let cond_opt_enum = emitter.variant_condition(&opt_val, 0).unwrap();
        emitter.guard(&cond_opt_enum).unwrap();

        emitter.line("ret i32 0");
        let decls = emitter
            .declarations
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        let ir = format!(
            "declare void @llvm.trap()\n{decls}\ndefine i32 @main() {{\nentry:\n{}{}}}\n",
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
