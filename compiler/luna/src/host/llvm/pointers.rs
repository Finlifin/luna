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
        let (llvm_ty, llvm_init) = self.const_eval(value, &BTreeMap::new(), 0)?.to_llvm(true);
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
        let symbol = format!(
            "flurry_const_{}_{}_{hash:016x}",
            declaration.package, declaration.local
        );
        self.declarations.insert(format!(
            "@{symbol} = private unnamed_addr constant {llvm_ty} {llvm_init}"
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
                        llvm_ty: "{}".into(),
                        llvm_init: "zeroinitializer".into(),
                        zero_sized: true,
                    });
                }
                let value = self.literal(literal, ty)?;
                Ok(ConstValue::Atom {
                    llvm_ty: value.ty.llvm(),
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
                    fields: evaluated,
                })
            }
            K::OptionalAbsent => {
                if !matches!(ty, Ty::Optional(_)) {
                    return Err("host optional constant lacks optional layout".into());
                }
                Ok(ConstValue::Optional {
                    llvm_ty: ty.llvm(),
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
                    let mut active_fields = Vec::with_capacity(fields.len());
                    for (field, (_, expected)) in fields.iter().zip(expected_fields.iter()) {
                        if repr(self.graph, &field.ty, self.checked)? != *expected {
                            return Err("host enum payload representation mismatch".into());
                        }
                        let (ft, fi) = self.const_eval(field, env, depth + 1)?.to_llvm(false);
                        active_fields.push(format!("{ft} {fi}"));
                    }
                    let mut variant_inits = Vec::with_capacity(layout.fields.len());
                    for (variant_index, variant_fields) in layout.fields.iter().enumerate() {
                        let variant_ty = Ty::Record(std::sync::Arc::new(
                            variant_fields.iter().map(|(_, ty)| ty.clone()).collect(),
                        ))
                        .llvm();
                        if variant_index == *index && !active_fields.is_empty() {
                            variant_inits
                                .push(format!("{variant_ty} {{ {} }}", active_fields.join(", ")));
                        } else {
                            variant_inits.push(format!("{variant_ty} zeroinitializer"));
                        }
                    }
                    return Ok(ConstValue::Enum {
                        llvm_ty: ty.llvm(),
                        llvm_init: format!("{{ i32 {index}, {} }}", variant_inits.join(", ")),
                    });
                }
                if fields.is_empty() {
                    return Ok(ConstValue::Enum {
                        llvm_ty: ty.llvm(),
                        llvm_init: code.clone(),
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
                    llvm_init: combined.to_string(),
                })
            }
            _ => Err("host constant address initializer contains dynamic expression".into()),
        }
    }
}

#[derive(Clone)]
enum ConstValue {
    Unit,
    Atom {
        llvm_ty: String,
        llvm_init: String,
        zero_sized: bool,
    },
    Record {
        packed: bool,
        llvm_ty: String,
        fields: Vec<ConstValue>,
    },
    Optional {
        llvm_ty: String,
        payload: Option<Box<ConstValue>>,
    },
    Enum {
        llvm_ty: String,
        llvm_init: String,
    },
}

impl ConstValue {
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
            Self::Optional { llvm_ty, payload } => match payload {
                None => (llvm_ty.clone(), "zeroinitializer".into()),
                Some(inner) => {
                    let (pt, pi) = inner.to_llvm(false);
                    (llvm_ty.clone(), format!("{{ i1 true, {pt} {pi} }}"))
                }
            },
            Self::Enum { llvm_ty, llvm_init } => (llvm_ty.clone(), llvm_init.clone()),
        }
    }
}
