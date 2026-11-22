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
            volatile: false,
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

    pub(super) fn static_address(
        &mut self,
        declaration: nir::DeclarationId,
    ) -> Result<Value, String> {
        if !self.checked {
            return Err("host data pointers require paired Copy/Drop resource verification".into());
        }
        let static_def = self
            .graph
            .statics
            .iter()
            .find(|s| s.declaration == declaration)
            .cloned()
            .ok_or_else(|| format!("missing static declaration {declaration}"))?;
        let pointer_bits = self.graph.target.map_or(64, |t| t.pointer_bits());
        let (llvm_ty, llvm_init, align) = if let Some(init) = &static_def.initializer {
            let const_val = self.const_eval(init, &BTreeMap::new(), 0)?;
            const_val.to_llvm_global(pointer_bits, self.fuel)?
        } else {
            let ty = repr(self.graph, &static_def.ty, self.checked)?;
            let layout = ty.storage_layout_at(pointer_bits, 0, self.fuel)?;
            if ty == Ty::Unit || layout.size == 0 {
                ("i8".into(), "0".into(), 1)
            } else {
                (ty.llvm(), "zeroinitializer".into(), layout.align)
            }
        };
        let symbol = format!(
            "flurry_static_{}_{}",
            declaration.package, declaration.local
        );
        self.declarations.insert(format!(
            "@{symbol} = internal global {llvm_ty} {llvm_init}, align {align}"
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
                let base = self.const_eval(value, env, depth + 1)?;
                match base {
                    ConstValue::Record { fields, .. } => fields
                        .get(*index)
                        .cloned()
                        .ok_or_else(|| "host constant field index out of range".into()),
                    other => {
                        if let Ty::BitPackedRecord(layout) = other.ty().clone() {
                            let field = layout
                                .fields
                                .get(*index)
                                .ok_or("host constant bit-packed field index out of range")?;
                            if field.ty == Ty::Unit {
                                return Ok(ConstValue::Unit);
                            }
                            let words = const_value_to_bit_words(&other, layout.total_bits)?;
                            let sliced = slice_bits_from(&words, field.bit_offset, field.bit_width);
                            return const_value_from_bit_words(&field.ty, &sliced);
                        }
                        Err("host constant field projection requires a record value".into())
                    }
                }
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
            K::StaticAddress(declaration) => {
                if ty != Ty::Pointer {
                    return Err("host static address representation mismatch".into());
                }
                let ptr = self.static_address(*declaration)?;
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
                if let Ty::BitPackedRecord(layout) = &ty {
                    if fields.len() != layout.fields.len() {
                        return Err("host constant bit-packed record field count mismatch".into());
                    }
                    let mut total_words = vec![0u32; layout.total_bits.div_ceil(32) as usize];
                    for (field, expected) in fields.iter().zip(layout.fields.iter()) {
                        if repr(self.graph, &field.ty, self.checked)? != expected.ty {
                            return Err(
                                "host constant bit-packed field representation mismatch".into()
                            );
                        }
                        let val = self.const_eval(field, env, depth + 1)?;
                        let f_words = const_value_to_bit_words(&val, expected.bit_width)?;
                        pack_bits_into(
                            &mut total_words,
                            &f_words,
                            expected.bit_offset,
                            expected.bit_width,
                        );
                    }
                    return const_value_from_bit_words(&ty, &total_words);
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
            K::TakeBits(inner) | K::BitConvert(inner) => {
                let src = self.const_eval(inner, env, depth + 1)?;
                let src_w = bits::bit_width(src.ty())
                    .ok_or("host constant bit conversion source has no width")?;
                let dst_w = bits::bit_width(&ty)
                    .ok_or("host constant bit conversion target has no width")?;
                if matches!(expr.kind, K::TakeBits(_)) {
                    if src_w != dst_w {
                        return Err("host constant take-bits changes width".into());
                    }
                } else if dst_w < src_w || (dst_w > src_w && !matches!(ty, Ty::Int(_, false))) {
                    return Err("host constant bit conversion width mismatch".into());
                }
                let mut words = const_value_to_bit_words(&src, src_w)?;
                words.resize(dst_w.div_ceil(32) as usize, 0);
                const_value_from_bit_words(&ty, &words)
            }
            K::BitSlice { value, start, end } => {
                let src = self.const_eval(value, env, depth + 1)?;
                let src_w = bits::bit_width(src.ty())
                    .ok_or("host constant bit slice source has no width")?;
                if *start > *end || *end > src_w as usize {
                    return Err("host constant bit slice out of bounds".into());
                }
                let words = const_value_to_bit_words(&src, src_w)?;
                let sliced = slice_bits_from(&words, *start as u32, (*end - *start) as u32);
                const_value_from_bit_words(&ty, &sliced)
            }
            K::BitConcat(high, low) => {
                let hi = self.const_eval(high, env, depth + 1)?;
                let lo = self.const_eval(low, env, depth + 1)?;
                let hi_w = bits::bit_width(hi.ty())
                    .ok_or("host constant bit concat operand has no width")?;
                let lo_w = bits::bit_width(lo.ty())
                    .ok_or("host constant bit concat operand has no width")?;
                let total_w = hi_w
                    .checked_add(lo_w)
                    .ok_or("host constant bit concat width overflow")?;
                let hi_words = const_value_to_bit_words(&hi, hi_w)?;
                let lo_words = const_value_to_bit_words(&lo, lo_w)?;
                let mut out = vec![0u32; total_w.div_ceil(32) as usize];
                pack_bits_into(&mut out, &lo_words, 0, lo_w);
                pack_bits_into(&mut out, &hi_words, lo_w, hi_w);
                const_value_from_bit_words(&ty, &out)
            }
            K::OptionalAbsent => {
                let Ty::Optional(opt) = ty.clone() else {
                    return Err("host optional constant lacks optional layout".into());
                };
                if let Some(niche) = &opt.niche {
                    let inner = const_niche_value(
                        &opt.carrier,
                        &niche.llvm_path,
                        niche.scalar,
                        niche.start,
                    )?;
                    Ok(ConstValue::NicheCarrier {
                        ty,
                        inner: Box::new(inner),
                    })
                } else {
                    Ok(ConstValue::Optional {
                        llvm_ty: ty.llvm(),
                        ty,
                        payload: None,
                    })
                }
            }
            K::OptionalPresent(payload) => {
                let Ty::Optional(opt) = ty.clone() else {
                    return Err("host optional constant lacks optional layout".into());
                };
                if payload.ty == Term::Primitive(P::Never)
                    || repr(self.graph, &payload.ty, self.checked)? != opt.payload
                {
                    return Err("host optional constant payload representation mismatch".into());
                }
                let mut inner = self.const_eval(payload, env, depth + 1)?;
                if let Some(niche) = &opt.niche {
                    if niche.widened_bool {
                        inner = const_widen_bool(inner, &opt.payload, &niche.llvm_path)?;
                    }
                    Ok(ConstValue::NicheCarrier {
                        ty,
                        inner: Box::new(inner),
                    })
                } else {
                    Ok(ConstValue::Optional {
                        llvm_ty: ty.llvm(),
                        ty,
                        payload: Some(Box::new(inner)),
                    })
                }
            }
            K::Variant { index, fields, .. } => {
                let Ty::Enum(layout) = ty.clone() else {
                    return Err("host variant constant lacks enum layout".into());
                };
                if *index >= layout.names.len() {
                    return Err("host variant index out of range".into());
                }
                match &layout.repr {
                    enums::EnumRepr::Symbol(symbols) => {
                        if !fields.is_empty() {
                            return Err("host Symbol enum cannot carry payloads".into());
                        }
                        let symbol = symbols
                            .get(*index)
                            .ok_or("host Symbol variant index out of range")?;
                        let value = self.text_literal(symbol, Ty::String)?;
                        Ok(ConstValue::Enum {
                            llvm_ty: ty.llvm(),
                            ty,
                            llvm_init: value.text,
                        })
                    }
                    enums::EnumRepr::Unit { width } => {
                        if !fields.is_empty() {
                            return Err("host enum payload count mismatch".into());
                        }
                        let llvm_init = if *width == 0 {
                            "zeroinitializer".into()
                        } else {
                            index.to_string()
                        };
                        Ok(ConstValue::Enum {
                            llvm_ty: ty.llvm(),
                            ty,
                            llvm_init,
                        })
                    }
                    enums::EnumRepr::SingleVariant {
                        unwrapped,
                        carrier,
                        fields: expected_fields,
                    } => {
                        if *index != 0 {
                            return Err("host variant index out of range".into());
                        }
                        if fields.len() != expected_fields.len() {
                            return Err("host enum payload count mismatch".into());
                        }
                        let mut evaluated = Vec::with_capacity(fields.len());
                        for (field, expected) in fields.iter().zip(expected_fields.iter()) {
                            if repr(self.graph, &field.ty, self.checked)? != *expected {
                                return Err("host enum payload representation mismatch".into());
                            }
                            evaluated.push(self.const_eval(field, env, depth + 1)?);
                        }
                        let inner = if *unwrapped {
                            evaluated.into_iter().next().unwrap()
                        } else {
                            ConstValue::Record {
                                ty: carrier.clone(),
                                packed: false,
                                llvm_ty: carrier.llvm(),
                                fields: evaluated,
                            }
                        };
                        Ok(ConstValue::NicheCarrier {
                            ty,
                            inner: Box::new(inner),
                        })
                    }
                    enums::EnumRepr::Niche {
                        data_variant,
                        unwrapped,
                        data_fields,
                        carrier,
                        niche,
                    } => {
                        if *index != *data_variant {
                            if !fields.is_empty() {
                                return Err("host enum payload count mismatch".into());
                            }
                            let ord = if *index < *data_variant {
                                *index
                            } else {
                                *index - 1
                            };
                            let inner = const_niche_value(
                                carrier,
                                &niche.llvm_path,
                                niche.scalar,
                                niche.start + ord as u64,
                            )?;
                            return Ok(ConstValue::NicheCarrier {
                                ty,
                                inner: Box::new(inner),
                            });
                        }
                        if fields.len() != data_fields.len() {
                            return Err("host enum payload count mismatch".into());
                        }
                        let mut evaluated = Vec::with_capacity(fields.len());
                        for (field, expected) in fields.iter().zip(data_fields.iter()) {
                            if repr(self.graph, &field.ty, self.checked)? != *expected {
                                return Err("host enum payload representation mismatch".into());
                            }
                            evaluated.push(self.const_eval(field, env, depth + 1)?);
                        }
                        let raw_ty = if *unwrapped {
                            data_fields[0].clone()
                        } else {
                            Ty::Record(std::sync::Arc::new(data_fields.clone()))
                        };
                        let mut inner = if *unwrapped {
                            evaluated.into_iter().next().unwrap()
                        } else {
                            ConstValue::Record {
                                ty: raw_ty.clone(),
                                packed: false,
                                llvm_ty: raw_ty.llvm(),
                                fields: evaluated,
                            }
                        };
                        if niche.widened_bool {
                            inner = const_widen_bool(inner, &raw_ty, &niche.llvm_path)?;
                        }
                        Ok(ConstValue::NicheCarrier {
                            ty,
                            inner: Box::new(inner),
                        })
                    }
                    enums::EnumRepr::FixedBits {
                        width,
                        codes,
                        fields: expected_fields,
                        ..
                    } => {
                        let code = codes.get(*index).ok_or("host variant index out of range")?;
                        let expected_fields = expected_fields
                            .get(*index)
                            .ok_or("host enum payload layout missing")?;
                        if fields.len() != expected_fields.len() {
                            return Err("host enum payload count mismatch".into());
                        }
                        if fields.is_empty() {
                            return Ok(ConstValue::Enum {
                                llvm_ty: ty.llvm(),
                                ty,
                                llvm_init: code.clone(),
                            });
                        }
                        if *width > 128 {
                            return Err(
                                "host constant bit-packed enum exceeds 128-bit initializer budget"
                                    .into(),
                            );
                        }
                        let mut combined = u128::from_str_radix(code, 10)
                            .map_err(|_| "host bit-packed enum discriminant is not an integer")?;
                        for (field, (start, expected)) in fields.iter().zip(expected_fields.iter())
                        {
                            if repr(self.graph, &field.ty, self.checked)? != *expected {
                                return Err("host enum payload representation mismatch".into());
                            }
                            let (_, fi) = self.const_eval(field, env, depth + 1)?.to_llvm(false);
                            let Some(field_width) = bits::bit_width(expected) else {
                                return Err(
                                    "host constant bit-packed enum payload lacks bit width".into(),
                                );
                            };
                            if field_width == 0 {
                                continue;
                            }
                            if field_width > 128 || *start + (field_width as usize) > 128 {
                                return Err(
                                    "host constant bit-packed enum field exceeds 128 bits".into()
                                );
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
                    enums::EnumRepr::TaggedUnion {
                        fields: all_fields, ..
                    } => {
                        let expected_fields = all_fields
                            .get(*index)
                            .ok_or("host enum payload layout missing")?;
                        if fields.len() != expected_fields.len() {
                            return Err("host enum payload count mismatch".into());
                        }
                        let mut evaluated = Vec::with_capacity(fields.len());
                        for (field, expected) in fields.iter().zip(expected_fields.iter()) {
                            if repr(self.graph, &field.ty, self.checked)? != *expected {
                                return Err("host enum payload representation mismatch".into());
                            }
                            evaluated.push(self.const_eval(field, env, depth + 1)?);
                        }
                        Ok(ConstValue::AggregateEnum {
                            ty,
                            variant: *index,
                            fields: evaluated,
                        })
                    }
                }
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
    NicheCarrier {
        ty: Ty,
        inner: Box<ConstValue>,
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
            | Self::NicheCarrier { ty, .. }
            | Self::AggregateEnum { ty, .. } => ty,
        }
    }

    fn contains_aggregate_enum(&self) -> bool {
        match self {
            Self::Unit | Self::Atom { .. } | Self::Enum { .. } => false,
            Self::AggregateEnum { .. } => true,
            Self::NicheCarrier { inner, .. } => inner.contains_aggregate_enum(),
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
                ty,
                llvm_ty,
                llvm_init,
            } => {
                if top_level
                    && matches!(
                        ty,
                        Ty::Enum(layout) if matches!(layout.repr, enums::EnumRepr::Unit { width: 0 })
                    )
                {
                    ("i8".into(), "0".into())
                } else {
                    (llvm_ty.clone(), llvm_init.clone())
                }
            }
            Self::NicheCarrier { ty, inner } => {
                let (inner_ty, inner_init) = inner.to_llvm(top_level);
                if top_level && inner_ty == "i8" && inner_init == "0" && ty.llvm() == "{}" {
                    ("i8".into(), "0".into())
                } else {
                    (ty.llvm(), inner_init)
                }
            }
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
            Self::NicheCarrier { inner, .. } => {
                inner.append_packed_parts(pointer_bits, depth + 1, fuel, types, inits)
            }
            Self::AggregateEnum {
                ty,
                variant,
                fields,
            } => {
                let Ty::Enum(layout) = ty else {
                    return Err("host aggregate enum constant lacks enum layout".into());
                };
                let enums::EnumRepr::TaggedUnion {
                    tag_bits,
                    payload_align,
                    ..
                } = layout.repr
                else {
                    return Err("host aggregate enum constant lacks tagged-union layout".into());
                };
                let total = ty.storage_layout_at(pointer_bits, 0, fuel)?;
                let tag_layout =
                    nir::layout::integer_layout(tag_bits).map_err(|e| e.to_string())?;
                types.push(format!("i{tag_bits}"));
                inits.push(format!("i{tag_bits} {variant}"));
                let payload_offset = nir::layout::align_up(tag_layout.size, payload_align)
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

fn const_zero_value(ty: &Ty) -> Result<ConstValue, String> {
    match ty {
        Ty::Unit => Ok(ConstValue::Unit),
        Ty::Bool => Ok(ConstValue::Atom {
            ty: ty.clone(),
            llvm_ty: "i1".into(),
            llvm_init: "false".into(),
            zero_sized: false,
        }),
        Ty::Int(0, _) => Ok(ConstValue::Atom {
            ty: ty.clone(),
            llvm_ty: "{}".into(),
            llvm_init: "zeroinitializer".into(),
            zero_sized: true,
        }),
        Ty::Int(..)
        | Ty::BFloat16
        | Ty::Float16
        | Ty::Float32
        | Ty::Float64
        | Ty::Pointer
        | Ty::FunctionPointer
        | Ty::TemplateIdentity(_)
        | Ty::String
        | Ty::Symbol
        | Ty::ErrorResult(_)
        | Ty::Closure(_) => Ok(ConstValue::Atom {
            ty: ty.clone(),
            llvm_ty: ty.llvm(),
            llvm_init: if matches!(ty, Ty::Pointer | Ty::FunctionPointer) {
                "null".into()
            } else if matches!(ty, Ty::Float16 | Ty::Float32 | Ty::Float64) {
                "0.0".into()
            } else if matches!(
                ty,
                Ty::String | Ty::Symbol | Ty::ErrorResult(_) | Ty::Closure(_)
            ) {
                "zeroinitializer".into()
            } else {
                "0".into()
            },
            zero_sized: false,
        }),
        Ty::Record(fields) | Ty::PackedRecord(fields) => {
            let zeroed = fields
                .iter()
                .map(const_zero_value)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(ConstValue::Record {
                packed: matches!(ty, Ty::PackedRecord(_)),
                llvm_ty: ty.llvm(),
                ty: ty.clone(),
                fields: zeroed,
            })
        }
        Ty::BitPackedRecord(layout) => {
            if layout.total_bits == 0 {
                Ok(ConstValue::Atom {
                    ty: ty.clone(),
                    llvm_ty: "{}".into(),
                    llvm_init: "zeroinitializer".into(),
                    zero_sized: true,
                })
            } else {
                Ok(ConstValue::Atom {
                    ty: ty.clone(),
                    llvm_ty: ty.llvm(),
                    llvm_init: "0".into(),
                    zero_sized: false,
                })
            }
        }
        Ty::Optional(opt) => {
            if opt.niche.is_some() {
                Ok(ConstValue::NicheCarrier {
                    ty: ty.clone(),
                    inner: Box::new(const_zero_value(&opt.carrier)?),
                })
            } else {
                Ok(ConstValue::Optional {
                    ty: ty.clone(),
                    llvm_ty: ty.llvm(),
                    payload: None,
                })
            }
        }
        Ty::Enum(layout) => match &layout.repr {
            enums::EnumRepr::Unit { width: 0 } => Ok(ConstValue::Enum {
                ty: ty.clone(),
                llvm_ty: "{}".into(),
                llvm_init: "zeroinitializer".into(),
            }),
            enums::EnumRepr::Unit { .. } | enums::EnumRepr::FixedBits { .. } => {
                Ok(ConstValue::Enum {
                    ty: ty.clone(),
                    llvm_ty: ty.llvm(),
                    llvm_init: "0".into(),
                })
            }
            enums::EnumRepr::Symbol(_) => Ok(ConstValue::Enum {
                ty: ty.clone(),
                llvm_ty: ty.llvm(),
                llvm_init: "zeroinitializer".into(),
            }),
            enums::EnumRepr::SingleVariant { carrier, .. } => Ok(ConstValue::NicheCarrier {
                ty: ty.clone(),
                inner: Box::new(const_zero_value(carrier)?),
            }),
            enums::EnumRepr::Niche { carrier, .. } => Ok(ConstValue::NicheCarrier {
                ty: ty.clone(),
                inner: Box::new(const_zero_value(carrier)?),
            }),
            enums::EnumRepr::TaggedUnion { .. } => Ok(ConstValue::AggregateEnum {
                ty: ty.clone(),
                variant: 0,
                fields: Vec::new(),
            }),
        },
    }
}

fn parse_decimal_to_words(decimal: &str, width: u32) -> Result<Vec<u32>, String> {
    if width == 0 {
        return Ok(Vec::new());
    }
    let word_count = width.div_ceil(32) as usize;
    let trimmed = decimal.trim();
    let (negative, digits) = if let Some(rest) = trimmed.strip_prefix('-') {
        (true, rest)
    } else {
        (false, trimmed)
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!(
            "host constant bit initializer is not an integer: `{trimmed}`"
        ));
    }
    let mut words = vec![0u32; word_count];
    for byte in digits.bytes() {
        let mut carry = u64::from(byte - b'0');
        for word in &mut words {
            let val = u64::from(*word) * 10 + carry;
            *word = val as u32;
            carry = val >> 32;
        }
    }
    if negative {
        let mut carry = 1u64;
        for word in &mut words {
            let val = u64::from(!*word) + carry;
            *word = val as u32;
            carry = val >> 32;
        }
    }
    let rem = width % 32;
    if rem != 0
        && let Some(last) = words.last_mut()
    {
        *last &= (1u32 << rem) - 1;
    }
    Ok(words)
}

fn pack_bits_into(dst: &mut [u32], src: &[u32], bit_offset: u32, bit_width: u32) {
    for bit in 0..bit_width {
        let word_idx = (bit / 32) as usize;
        let bit_val = (src.get(word_idx).copied().unwrap_or(0) >> (bit % 32)) & 1;
        if bit_val != 0 {
            let dst_bit = bit_offset + bit;
            dst[(dst_bit / 32) as usize] |= 1u32 << (dst_bit % 32);
        }
    }
}

fn slice_bits_from(src: &[u32], bit_offset: u32, bit_width: u32) -> Vec<u32> {
    if bit_width == 0 {
        return Vec::new();
    }
    let mut dst = vec![0u32; bit_width.div_ceil(32) as usize];
    for bit in 0..bit_width {
        let src_bit = bit_offset + bit;
        let bit_val =
            (src.get((src_bit / 32) as usize).copied().unwrap_or(0) >> (src_bit % 32)) & 1;
        if bit_val != 0 {
            dst[(bit / 32) as usize] |= 1u32 << (bit % 32);
        }
    }
    dst
}

fn const_value_to_bit_words(val: &ConstValue, width: u32) -> Result<Vec<u32>, String> {
    if width == 0 {
        return Ok(Vec::new());
    }
    match val {
        ConstValue::Unit => Err("host constant unit value has 0 bit width".into()),
        ConstValue::Atom {
            ty,
            llvm_init,
            zero_sized,
            ..
        } => {
            if *zero_sized {
                return Ok(vec![0u32; width.div_ceil(32) as usize]);
            }
            match ty {
                Ty::Bool => {
                    let v = match llvm_init.as_str() {
                        "true" => 1u32,
                        "false" => 0u32,
                        _ => return Err("host constant bool initializer invalid".into()),
                    };
                    Ok(vec![v])
                }
                Ty::Float16 | Ty::Float32 | Ty::Float64 => {
                    if llvm_init == "0.0" {
                        return Ok(vec![0u32; width.div_ceil(32) as usize]);
                    }
                    let rest = llvm_init
                        .strip_prefix("bitcast (")
                        .and_then(|s| s.split_once(' '))
                        .map(|(_, r)| r)
                        .and_then(|r| r.split_once(" to "))
                        .map(|(num, _)| num)
                        .ok_or("host constant float initializer format unsupported")?;
                    parse_decimal_to_words(rest, width)
                }
                Ty::Int(..) | Ty::BFloat16 | Ty::BitPackedRecord(_) => {
                    parse_decimal_to_words(llvm_init, width)
                }
                _ => Err("host constant atom has no bit-packed representation".into()),
            }
        }
        ConstValue::Enum { llvm_init, .. } => parse_decimal_to_words(llvm_init, width),
        ConstValue::NicheCarrier { inner, .. } => const_value_to_bit_words(inner, width),
        _ => Err("host constant aggregate has no bit-packed representation".into()),
    }
}

fn const_value_from_bit_words(ty: &Ty, words: &[u32]) -> Result<ConstValue, String> {
    let width = bits::bit_width(ty).ok_or("host constant target has no bit width")?;
    let mut normalized = words.to_vec();
    normalized.resize(width.div_ceil(32) as usize, 0);
    let rem = width % 32;
    if rem != 0
        && let Some(last) = normalized.last_mut()
    {
        *last &= (1u32 << rem) - 1;
    }
    match ty {
        Ty::Unit => Ok(ConstValue::Unit),
        Ty::Bool => Ok(ConstValue::Atom {
            ty: ty.clone(),
            llvm_ty: "i1".into(),
            llvm_init: if (normalized.first().copied().unwrap_or(0) & 1) != 0 {
                "true"
            } else {
                "false"
            }
            .into(),
            zero_sized: false,
        }),
        Ty::Int(0, _) => Ok(ConstValue::Atom {
            ty: ty.clone(),
            llvm_ty: "{}".into(),
            llvm_init: "zeroinitializer".into(),
            zero_sized: true,
        }),
        Ty::Int(..) => Ok(ConstValue::Atom {
            ty: ty.clone(),
            llvm_ty: ty.llvm(),
            llvm_init: bits::decimal_words(&normalized),
            zero_sized: false,
        }),
        Ty::BFloat16 => Ok(ConstValue::Atom {
            ty: ty.clone(),
            llvm_ty: "i16".into(),
            llvm_init: (normalized.first().copied().unwrap_or(0) & 0xFFFF).to_string(),
            zero_sized: false,
        }),
        Ty::Float16 => Ok(ConstValue::Atom {
            ty: ty.clone(),
            llvm_ty: "half".into(),
            llvm_init: format!(
                "bitcast (i16 {} to half)",
                normalized.first().copied().unwrap_or(0) & 0xFFFF
            ),
            zero_sized: false,
        }),
        Ty::Float32 => Ok(ConstValue::Atom {
            ty: ty.clone(),
            llvm_ty: "float".into(),
            llvm_init: format!(
                "bitcast (i32 {} to float)",
                normalized.first().copied().unwrap_or(0)
            ),
            zero_sized: false,
        }),
        Ty::Float64 => {
            let lo = u64::from(normalized.first().copied().unwrap_or(0));
            let hi = u64::from(normalized.get(1).copied().unwrap_or(0));
            let combined = lo | (hi << 32);
            Ok(ConstValue::Atom {
                ty: ty.clone(),
                llvm_ty: "double".into(),
                llvm_init: format!("bitcast (i64 {combined} to double)"),
                zero_sized: false,
            })
        }
        Ty::BitPackedRecord(layout) => {
            if layout.total_bits == 0 {
                Ok(ConstValue::Atom {
                    ty: ty.clone(),
                    llvm_ty: "{}".into(),
                    llvm_init: "zeroinitializer".into(),
                    zero_sized: true,
                })
            } else {
                Ok(ConstValue::Atom {
                    ty: ty.clone(),
                    llvm_ty: ty.llvm(),
                    llvm_init: bits::decimal_words(&normalized),
                    zero_sized: false,
                })
            }
        }
        Ty::Enum(layout)
            if matches!(
                layout.repr,
                enums::EnumRepr::FixedBits { .. } | enums::EnumRepr::Unit { .. }
            ) =>
        {
            if width == 0 {
                Ok(ConstValue::Enum {
                    ty: ty.clone(),
                    llvm_ty: "{}".into(),
                    llvm_init: "zeroinitializer".into(),
                })
            } else {
                Ok(ConstValue::Enum {
                    ty: ty.clone(),
                    llvm_ty: ty.llvm(),
                    llvm_init: bits::decimal_words(&normalized),
                })
            }
        }
        _ => Err("host constant bit conversion target is unsupported".into()),
    }
}

fn const_niche_value(
    carrier: &Ty,
    path: &[usize],
    scalar: nir::layout::NicheScalar,
    code: u64,
) -> Result<ConstValue, String> {
    if path.is_empty() {
        let init = match scalar {
            nir::layout::NicheScalar::Pointer => "null".to_string(),
            nir::layout::NicheScalar::Int { .. } => code.to_string(),
        };
        return Ok(ConstValue::Atom {
            ty: carrier.clone(),
            llvm_ty: carrier.llvm(),
            llvm_init: init,
            zero_sized: false,
        });
    }
    let idx = path[0];
    let rest = &path[1..];
    match carrier {
        Ty::Record(fields) => {
            if idx >= fields.len() {
                return Err("host constant niche record index out of range".into());
            }
            let mut built = Vec::with_capacity(fields.len());
            for (j, field_ty) in fields.iter().enumerate() {
                if j == idx {
                    built.push(const_niche_value(field_ty, rest, scalar, code)?);
                } else {
                    built.push(const_zero_value(field_ty)?);
                }
            }
            Ok(ConstValue::Record {
                ty: carrier.clone(),
                packed: false,
                llvm_ty: carrier.llvm(),
                fields: built,
            })
        }
        Ty::Optional(opt) if opt.niche.is_some() => Ok(ConstValue::NicheCarrier {
            ty: carrier.clone(),
            inner: Box::new(const_niche_value(&opt.carrier, path, scalar, code)?),
        }),
        Ty::Enum(layout) => match &layout.repr {
            enums::EnumRepr::SingleVariant {
                carrier: inner_carrier,
                ..
            }
            | enums::EnumRepr::Niche {
                carrier: inner_carrier,
                ..
            } => Ok(ConstValue::NicheCarrier {
                ty: carrier.clone(),
                inner: Box::new(const_niche_value(inner_carrier, path, scalar, code)?),
            }),
            enums::EnumRepr::TaggedUnion { .. } if idx == 0 && rest.is_empty() => {
                Ok(ConstValue::AggregateEnum {
                    ty: carrier.clone(),
                    variant: usize::try_from(code)
                        .map_err(|_| "host constant niche tag overflows usize")?,
                    fields: Vec::new(),
                })
            }
            _ => Err("host constant niche unsupported enum carrier".into()),
        },
        _ => Err("host constant niche carrier path mismatch".into()),
    }
}

fn const_widen_bool(val: ConstValue, orig_ty: &Ty, path: &[usize]) -> Result<ConstValue, String> {
    if path.is_empty() {
        if let ConstValue::NicheCarrier { inner, .. } = val {
            return const_widen_bool(*inner, orig_ty, &[]);
        }
        let ConstValue::Atom { llvm_init, .. } = val else {
            return Err("host constant bool-widen leaf must be atom".into());
        };
        let widened = match llvm_init.as_str() {
            "true" | "1" => "1",
            "false" | "0" => "0",
            _ => return Err("host constant bool-widen encountered non-boolean init".into()),
        };
        return Ok(ConstValue::Atom {
            ty: Ty::Int(8, false),
            llvm_ty: "i8".into(),
            llvm_init: widened.into(),
            zero_sized: false,
        });
    }
    let idx = path[0];
    let rest = &path[1..];
    match (val, orig_ty) {
        (ConstValue::Record { mut fields, .. }, _) => {
            if idx >= fields.len() {
                return Err("host constant bool-widen field out of range".into());
            }
            let child_orig = match orig_ty {
                Ty::Record(orig_fields) => orig_fields
                    .get(idx)
                    .ok_or("host constant bool-widen orig field out of range")?
                    .clone(),
                Ty::Enum(layout) => match &layout.repr {
                    enums::EnumRepr::SingleVariant { carrier, .. } => {
                        let Ty::Record(orig_fields) = carrier else {
                            return Err("host constant bool-widen expected record carrier".into());
                        };
                        orig_fields
                            .get(idx)
                            .ok_or("host constant bool-widen orig field out of range")?
                            .clone()
                    }
                    _ => return Err("host constant bool-widen unsupported enum carrier".into()),
                },
                _ => return Err("host constant bool-widen type mismatch".into()),
            };
            let target = fields.remove(idx);
            let widened = const_widen_bool(target, &child_orig, rest)?;
            fields.insert(idx, widened);
            let new_ty = Ty::Record(std::sync::Arc::new(
                fields.iter().map(|f| f.ty().clone()).collect(),
            ));
            Ok(ConstValue::Record {
                llvm_ty: new_ty.llvm(),
                ty: new_ty,
                packed: false,
                fields,
            })
        }
        (ConstValue::Optional { payload, .. }, Ty::Optional(opt))
            if opt.niche.is_none() && idx == 0 && rest.is_empty() =>
        {
            let tag_init = if payload.is_some() { "1" } else { "0" };
            let tag_const = ConstValue::Atom {
                ty: Ty::Int(8, false),
                llvm_ty: "i8".into(),
                llvm_init: tag_init.into(),
                zero_sized: false,
            };
            let pay_const = match payload {
                Some(p) => *p,
                None => const_zero_value(&opt.payload)?,
            };
            let new_ty = Ty::Record(std::sync::Arc::new(vec![
                Ty::Int(8, false),
                opt.payload.clone(),
            ]));
            Ok(ConstValue::Record {
                llvm_ty: new_ty.llvm(),
                ty: new_ty,
                packed: false,
                fields: vec![tag_const, pay_const],
            })
        }
        (ConstValue::NicheCarrier { inner, .. }, Ty::Enum(layout)) => match &layout.repr {
            enums::EnumRepr::SingleVariant { carrier, .. } => {
                const_widen_bool(*inner, carrier, path)
            }
            _ => Err("host constant bool-widen unsupported enum".into()),
        },
        _ => Err("host constant bool-widen path mismatch".into()),
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
                packed: nir::program::PackedMode::None,
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
            attr_table: None,
            bitvec_cache: None,
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
        assert!(decls.contains("<{ i8, [7 x i8], [24 x i8] }>"), "{decls}");

        let enum_llvm = repr(&graph, &const_text_enum.ty, true).unwrap();
        let loaded_enum = emitter.load(&Slot::new(enum_llvm.clone(), p_enum.text, None));
        let cond_enum = emitter.variant_condition(&loaded_enum, 1).unwrap();
        emitter.guard(&cond_enum).unwrap();
        let f_num = emitter.variant_field(&loaded_enum, 1, 1).unwrap();
        let eq_enum = emitter.instruction(Ty::Bool, format!("icmp eq i64 {}, 777", f_num.text));
        emitter.guard(&eq_enum).unwrap();

        let wrap_llvm = repr(&graph, &const_wrap.ty, true).unwrap();
        let loaded_wrap = emitter.load(&Slot::new(wrap_llvm, p_wrap.text, None));
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
        let loaded_opt = emitter.load(&Slot::new(opt_llvm, p_opt.text, None));
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

    #[test]
    fn const_address_emits_niche_optionals_and_enums_correctly() {
        let mut graph = iir::Package::default();
        let rich_decl = nir::DeclarationId {
            package: 0,
            local: 1,
        };
        let rich_ty = Term::Nominal {
            declaration: rich_decl,
            arguments: vec![],
        };
        let rich_inst = iir::NominalInstance {
            ty: rich_ty.clone(),
            definition: iir::NominalDefinition::Enum(nir::program::Enum {
                is_intrinsic: false,
                parameter_types: vec![],
                capabilities: vec![],
                requirements: vec![],
                type_parameters: vec![],
                declaration: rich_decl,
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
        let choice_decl = nir::DeclarationId {
            package: 0,
            local: 2,
        };
        let choice_ty = Term::Nominal {
            declaration: choice_decl,
            arguments: vec![],
        };
        let choice_inst = iir::NominalInstance {
            ty: choice_ty.clone(),
            definition: iir::NominalDefinition::Enum(nir::program::Enum {
                is_intrinsic: false,
                parameter_types: vec![],
                capabilities: vec![],
                requirements: vec![],
                type_parameters: vec![],
                declaration: choice_decl,
                name: "FlagChoice".into(),
                groups: vec![],
                encoding: None,
                symbol_encoding: None,
                variants: vec![
                    nir::program::Variant {
                        name: "empty".into(),
                        shape: nir::program::VariantShape::Unit,
                        fields: vec![],
                    },
                    nir::program::Variant {
                        name: "alt".into(),
                        shape: nir::program::VariantShape::Unit,
                        fields: vec![],
                    },
                    nir::program::Variant {
                        name: "flag".into(),
                        shape: nir::program::VariantShape::Tuple,
                        fields: vec![Term::Primitive(P::Bool)],
                    },
                ],
            }),
        };
        graph.nominals = vec![rich_inst, choice_inst];

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
            bitvec_cache: None,
            serial: 0,
            terminated: false,
            fuel: &mut fuel,
        };

        let src = nir::program::SourceRange::generated(0);
        let opt_rich_ty = Term::Optional(Box::new(rich_ty));
        let null_rich = Expr {
            ty: opt_rich_ty.clone(),
            source: src,
            kind: K::OptionalAbsent,
        };
        let p_null_rich = emitter
            .const_address(
                nir::DeclarationId {
                    package: 0,
                    local: 20,
                },
                &null_rich,
            )
            .unwrap();

        let opt_bool_ty = Term::Optional(Box::new(Term::Primitive(P::Bool)));
        let opt2_bool_ty = Term::Optional(Box::new(opt_bool_ty.clone()));
        let make_opt_bool = |e: Expr| Expr {
            ty: opt2_bool_ty.clone(),
            source: src,
            kind: K::OptionalPresent(Box::new(e)),
        };
        let p_b_outer_null = emitter
            .const_address(
                nir::DeclarationId {
                    package: 0,
                    local: 21,
                },
                &Expr {
                    ty: opt2_bool_ty.clone(),
                    source: src,
                    kind: K::OptionalAbsent,
                },
            )
            .unwrap();
        let p_b_inner_null = emitter
            .const_address(
                nir::DeclarationId {
                    package: 0,
                    local: 22,
                },
                &make_opt_bool(Expr {
                    ty: opt_bool_ty.clone(),
                    source: src,
                    kind: K::OptionalAbsent,
                }),
            )
            .unwrap();
        let p_b_true = emitter
            .const_address(
                nir::DeclarationId {
                    package: 0,
                    local: 23,
                },
                &make_opt_bool(Expr {
                    ty: opt_bool_ty,
                    source: src,
                    kind: K::OptionalPresent(Box::new(Expr {
                        ty: Term::Primitive(P::Bool),
                        source: src,
                        kind: K::Literal(Literal::Bool(true)),
                    })),
                }),
            )
            .unwrap();

        let opt_u64_ty = Term::Optional(Box::new(Term::Primitive(P::U64)));
        let opt2_u64_ty = Term::Optional(Box::new(opt_u64_ty.clone()));
        let p_u_outer_null = emitter
            .const_address(
                nir::DeclarationId {
                    package: 0,
                    local: 24,
                },
                &Expr {
                    ty: opt2_u64_ty.clone(),
                    source: src,
                    kind: K::OptionalAbsent,
                },
            )
            .unwrap();
        let p_u_42 = emitter
            .const_address(
                nir::DeclarationId {
                    package: 0,
                    local: 25,
                },
                &Expr {
                    ty: opt2_u64_ty.clone(),
                    source: src,
                    kind: K::OptionalPresent(Box::new(Expr {
                        ty: opt_u64_ty,
                        source: src,
                        kind: K::OptionalPresent(Box::new(Expr {
                            ty: Term::Primitive(P::U64),
                            source: src,
                            kind: K::Literal(Literal::UnsignedInteger(42)),
                        })),
                    })),
                },
            )
            .unwrap();

        let p_choice_alt = emitter
            .const_address(
                nir::DeclarationId {
                    package: 0,
                    local: 26,
                },
                &Expr {
                    ty: choice_ty.clone(),
                    source: src,
                    kind: K::Variant {
                        declaration: choice_decl,
                        index: 1,
                        fields: vec![],
                    },
                },
            )
            .unwrap();
        let p_choice_flag = emitter
            .const_address(
                nir::DeclarationId {
                    package: 0,
                    local: 27,
                },
                &Expr {
                    ty: choice_ty.clone(),
                    source: src,
                    kind: K::Variant {
                        declaration: choice_decl,
                        index: 2,
                        fields: vec![Expr {
                            ty: Term::Primitive(P::Bool),
                            source: src,
                            kind: K::Literal(Literal::Bool(true)),
                        }],
                    },
                },
            )
            .unwrap();

        let decls = emitter
            .declarations
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        assert!(decls.contains("<{ i8, [7 x i8], [24 x i8] }>"), "{decls}");
        assert!(decls.contains("i8 3"), "{decls}");

        let opt_rich_llvm = repr(&graph, &opt_rich_ty, true).unwrap();
        let loaded_null_rich = emitter.load(&Slot::new(opt_rich_llvm, p_null_rich.text, None));
        let null_tag = emitter.optional_tag(&loaded_null_rich).unwrap();
        let not_present = emitter.instruction(Ty::Bool, format!("xor i1 {}, true", null_tag.text));
        emitter.guard(&not_present).unwrap();

        let opt2_bool_llvm = repr(&graph, &opt2_bool_ty, true).unwrap();
        assert_eq!(opt2_bool_llvm.llvm(), "i8");
        let l_out_null = emitter.load(&Slot::new(
            opt2_bool_llvm.clone(),
            p_b_outer_null.text,
            None,
        ));
        let t_out_null = emitter.optional_tag(&l_out_null).unwrap();
        let not_out = emitter.instruction(Ty::Bool, format!("xor i1 {}, true", t_out_null.text));
        emitter.guard(&not_out).unwrap();

        let l_in_null = emitter.load(&Slot::new(
            opt2_bool_llvm.clone(),
            p_b_inner_null.text,
            None,
        ));
        let t_in_null = emitter.optional_tag(&l_in_null).unwrap();
        emitter.guard(&t_in_null).unwrap();
        let pay_in_null = emitter.optional_payload(&l_in_null).unwrap();
        let t_inner = emitter.optional_tag(&pay_in_null).unwrap();
        let not_inner = emitter.instruction(Ty::Bool, format!("xor i1 {}, true", t_inner.text));
        emitter.guard(&not_inner).unwrap();

        let l_b_true = emitter.load(&Slot::new(opt2_bool_llvm, p_b_true.text, None));
        let t_b_true = emitter.optional_tag(&l_b_true).unwrap();
        emitter.guard(&t_b_true).unwrap();
        let p1 = emitter.optional_payload(&l_b_true).unwrap();
        let t_p1 = emitter.optional_tag(&p1).unwrap();
        emitter.guard(&t_p1).unwrap();
        let b_val = emitter.optional_payload(&p1).unwrap();
        emitter.guard(&b_val).unwrap();

        let opt2_u64_llvm = repr(&graph, &opt2_u64_ty, true).unwrap();
        assert_eq!(opt2_u64_llvm.llvm(), "{ i8, i64 }");
        let l_u_null = emitter.load(&Slot::new(opt2_u64_llvm.clone(), p_u_outer_null.text, None));
        let t_u_null = emitter.optional_tag(&l_u_null).unwrap();
        let not_u = emitter.instruction(Ty::Bool, format!("xor i1 {}, true", t_u_null.text));
        emitter.guard(&not_u).unwrap();

        let l_u_42 = emitter.load(&Slot::new(opt2_u64_llvm, p_u_42.text, None));
        let t_u_42 = emitter.optional_tag(&l_u_42).unwrap();
        emitter.guard(&t_u_42).unwrap();
        let inner_u42 = emitter.optional_payload(&l_u_42).unwrap();
        let t_in42 = emitter.optional_tag(&inner_u42).unwrap();
        emitter.guard(&t_in42).unwrap();
        let u42 = emitter.optional_payload(&inner_u42).unwrap();
        let eq42 = emitter.instruction(Ty::Bool, format!("icmp eq i64 {}, 42", u42.text));
        emitter.guard(&eq42).unwrap();

        let choice_llvm = repr(&graph, &choice_ty, true).unwrap();
        assert_eq!(choice_llvm.llvm(), "i8");
        let l_alt = emitter.load(&Slot::new(choice_llvm.clone(), p_choice_alt.text, None));
        let is_alt = emitter.variant_condition(&l_alt, 1).unwrap();
        emitter.guard(&is_alt).unwrap();
        let l_flag = emitter.load(&Slot::new(choice_llvm, p_choice_flag.text, None));
        let is_flag = emitter.variant_condition(&l_flag, 2).unwrap();
        emitter.guard(&is_flag).unwrap();
        let extracted_flag = emitter.variant_field(&l_flag, 2, 0).unwrap();
        emitter.guard(&extracted_flag).unwrap();

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
