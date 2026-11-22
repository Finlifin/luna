//! Read, encode, decode, and validate bit representations for scalars, enums,
//! and `attrs.packed.bit` structs across tree execution and bytecode.
use super::*;
use comptime_runtime::bitvec::{BitVec, Error as BitError};

const MAX_BIT_PACK_DEPTH: usize = 128;

enum BitPackedNominal {
    Struct {
        fields: Vec<Term>,
    },
    Newtype {
        repr: Term,
    },
    Enum {
        encoding: nir::enum_encoding::Layout,
        variant_fields: Vec<Vec<Term>>,
    },
}

impl Interpreter<'_> {
    fn resolve_bit_packed_nominal(
        &mut self,
        ty: &Term,
        source: SourceRange,
    ) -> Result<Option<BitPackedNominal>, Error> {
        let Term::Nominal {
            declaration,
            arguments,
        } = ty
        else {
            return Ok(None);
        };
        if self.graph.is_some() {
            let Some(concrete) = self.concrete_nominal_metered(ty)? else {
                return Ok(None);
            };
            return Ok(match concrete {
                ConcreteNominal::Struct(def) if def.packed == nir::program::PackedMode::Bit => {
                    Some(BitPackedNominal::Struct {
                        fields: def.fields.iter().map(|(_, t)| t.clone()).collect(),
                    })
                }
                ConcreteNominal::Newtype(def) => Some(BitPackedNominal::Newtype {
                    repr: def.representation.clone(),
                }),
                ConcreteNominal::Enum(def) => {
                    def.encoding
                        .as_ref()
                        .map(|encoding| BitPackedNominal::Enum {
                            encoding: encoding.layout.clone(),
                            variant_fields: def
                                .variants
                                .iter()
                                .map(|variant| variant.fields.clone())
                                .collect(),
                        })
                }
                _ => None,
            });
        }
        let mut fuel = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let before = fuel;
        let result = (|| -> Result<Option<BitPackedNominal>, ErrorKind> {
            if let Some(def) = self.structs.get(declaration) {
                if def.packed != nir::program::PackedMode::Bit {
                    return Ok(None);
                }
                let fields = nir::normal_forms::instantiate_value_fields_checked(
                    &def.type_parameters,
                    &def.parameter_types,
                    arguments,
                    def.fields.iter().map(|(_, t)| t),
                    &mut fuel,
                )
                .map_err(|err| match err {
                    nir::NominalAdmissionError::Budget => ErrorKind::StepsExhausted,
                    _ => ErrorKind::Type,
                })?;
                return Ok(Some(BitPackedNominal::Struct { fields }));
            }
            if let Some(def) = self.newtypes.get(declaration) {
                let repr = def
                    .representation_budgeted(arguments, &[], &mut fuel)
                    .map_err(|err| match err {
                        nir::newtypes::ConversionError::Budget => ErrorKind::StepsExhausted,
                        _ => ErrorKind::Type,
                    })?;
                return Ok(Some(BitPackedNominal::Newtype { repr }));
            }
            if let Some(def) = self.enums.get(declaration) {
                let Some(encoding) = &def.encoding else {
                    return Ok(None);
                };
                let mut variant_fields = Vec::with_capacity(def.variants.len());
                for variant in &def.variants {
                    let fields = nir::normal_forms::instantiate_value_fields_checked(
                        &def.type_parameters,
                        &def.parameter_types,
                        arguments,
                        variant.fields.iter(),
                        &mut fuel,
                    )
                    .map_err(|err| match err {
                        nir::NominalAdmissionError::Budget => ErrorKind::StepsExhausted,
                        _ => ErrorKind::Type,
                    })?;
                    variant_fields.push(fields);
                }
                return Ok(Some(BitPackedNominal::Enum {
                    encoding: encoding.layout.clone(),
                    variant_fields,
                }));
            }
            Ok(None)
        })();
        self.budget.steps -= (before - fuel) as u64;
        result.map_err(|kind| self.failure(kind, Some(source)))
    }

    pub(super) fn unrestricted_bit_storage_width(
        &mut self,
        storage: &Term,
        source: SourceRange,
    ) -> Result<Option<usize>, Error> {
        match storage {
            Term::Primitive(Primitive::BitVec(width)) => return Ok(Some(*width)),
            Term::Primitive(p) => {
                return Ok(p
                    .unrestricted_scalar_width_in(self.target)
                    .map(|n| n as usize));
            }
            Term::Nominal { .. } => {}
            _ => return Ok(None),
        }
        let Some(BitPackedNominal::Struct { fields }) =
            self.resolve_bit_packed_nominal(storage, source)?
        else {
            return Ok(None);
        };
        let mut fuel = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let before = fuel;
        let layout = nir::bit_packed_struct_layout(
            &observations::InterpreterLayoutWorld(self),
            &fields,
            &mut fuel,
        );
        self.budget.steps -= (before - fuel) as u64;
        match layout {
            Ok(layout) => Ok((!layout.restricted).then_some(layout.total_bits as usize)),
            Err(nir::BitPackedError::Layout(nir::LayoutError::BudgetExhausted)) => {
                Err(self.failure(ErrorKind::StepsExhausted, Some(source)))
            }
            Err(_) => Ok(None),
        }
    }

    pub(super) fn check_bit_update_selections(
        &mut self,
        storage: &Term,
        selections: &[nir::program::BitSelection],
        value: &Term,
        result: &Term,
        source: SourceRange,
    ) -> Result<usize, Error> {
        let mut fuel = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let before = fuel;
        let err = nir::program::bit_update_error_with_world(
            &observations::InterpreterLayoutWorld(self),
            storage,
            selections,
            value,
            result,
            &mut fuel,
        );
        self.budget.steps -= (before - fuel) as u64;
        if err.is_some() {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        self.unrestricted_bit_storage_width(storage, source)?
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))
    }

    pub(super) fn check_bit_update_plan_layouts<'a>(
        &mut self,
        storage: &Term,
        layouts: impl IntoIterator<Item = nir::program::BitSelectionLayout<&'a Term>>,
        value: &Term,
        result: &Term,
        source: SourceRange,
    ) -> Result<usize, Error> {
        let mut fuel = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let before = fuel;
        let err = nir::program::bit_update_layout_error_with_world(
            &observations::InterpreterLayoutWorld(self),
            storage,
            layouts,
            value,
            result,
            &mut fuel,
        );
        self.budget.steps -= (before - fuel) as u64;
        if err.is_some() {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        self.unrestricted_bit_storage_width(storage, source)?
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))
    }

    pub(super) fn decode_unrestricted_scalar(
        &mut self,
        bits: &BitVec,
        primitive: Primitive,
        allow_widening: bool,
        source: SourceRange,
    ) -> Result<Value, Error> {
        let Some(width) = primitive.unrestricted_scalar_width_in(self.target) else {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        };
        let widening = allow_widening
            && self
                .integer_layout(primitive)
                .is_some_and(|(signed, _)| !signed);
        if (width as usize) < bits.width() || (!widening && width as usize != bits.width()) {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        let value = self.bit_operation(source, |budget| {
            budget.consume(bits.words().len().saturating_add(1))?;
            Ok(bits
                .words()
                .iter()
                .rev()
                .fold(0u128, |acc, word| (acc << 32) | u128::from(*word)))
        })?;
        Ok(match primitive {
            Primitive::F16 => Value::Float16(value as u16),
            Primitive::Bf16 => Value::BFloat16(value as u16),
            Primitive::F32 => Value::Float32(value as u32),
            Primitive::F64 => Value::Float64(value as u64),
            _ if self
                .integer_layout(primitive)
                .is_some_and(|(signed, _)| signed) =>
            {
                let shift = 128 - width;
                Value::SignedInteger(((value << shift) as i128) >> shift)
            }
            _ => Value::UnsignedInteger(value),
        })
    }

    pub(super) fn encode_bit_packed_value(
        &mut self,
        value: &Value,
        ty: &Term,
        source: SourceRange,
    ) -> Result<BitVec, Error> {
        self.encode_bit_packed_at(value, ty, 0, source)
    }

    fn encode_bit_packed_at(
        &mut self,
        value: &Value,
        ty: &Term,
        depth: usize,
        source: SourceRange,
    ) -> Result<BitVec, Error> {
        if depth >= MAX_BIT_PACK_DEPTH {
            return Err(self.failure(ErrorKind::StepsExhausted, Some(source)));
        }
        self.charge_preparation(1)?;
        match (value, ty) {
            (Value::Unit, Term::Primitive(Primitive::Void)) => {
                self.bit_operation(source, |budget| BitVec::from_words(0, &[], budget))
            }
            (Value::Bool(flag), Term::Primitive(Primitive::Bool)) => self
                .bit_operation(source, |budget| {
                    BitVec::from_words(1, &[u32::from(*flag)], budget)
                }),
            (Value::BitVec(bits), Term::Primitive(Primitive::BitVec(width)))
                if bits.width() == *width =>
            {
                self.bit_operation(source, |budget| {
                    BitVec::from_words(bits.width(), bits.words(), budget)
                })
            }
            (Value::Char(ch), Term::Primitive(Primitive::Char)) => {
                let raw = u32::from(*ch);
                self.bit_operation(source, |budget| BitVec::from_words(32, &[raw], budget))
            }
            (
                Value::SignedInteger(_)
                | Value::UnsignedInteger(_)
                | Value::Float16(_)
                | Value::BFloat16(_)
                | Value::Float32(_)
                | Value::Float64(_),
                Term::Primitive(primitive),
            ) => {
                let Some(width) = primitive.unrestricted_scalar_width_in(self.target) else {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                };
                let raw = match value {
                    Value::SignedInteger(v) => *v as u128,
                    Value::UnsignedInteger(v) => *v,
                    Value::Float16(bits) | Value::BFloat16(bits) => u128::from(*bits),
                    Value::Float32(bits) => u128::from(*bits),
                    Value::Float64(bits) => u128::from(*bits),
                    _ => unreachable!(),
                };
                let raw = if width == 128 {
                    raw
                } else {
                    raw & ((1u128 << width) - 1)
                };
                self.bit_operation(source, |budget| {
                    comptime_runtime::Integer::from_u128(raw).to_bitvec(width as usize, budget)
                })
            }
            (Value::Newtype { ty: actual, value }, Term::Nominal { .. }) if actual == ty => {
                let Some(BitPackedNominal::Newtype { repr }) =
                    self.resolve_bit_packed_nominal(ty, source)?
                else {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                };
                self.encode_bit_packed_at(value, &repr, depth + 1, source)
            }
            (
                Value::Variant {
                    declaration,
                    index,
                    fields,
                    ..
                },
                Term::Nominal {
                    declaration: ty_decl,
                    ..
                },
            ) if declaration == ty_decl => {
                let Some(BitPackedNominal::Enum {
                    encoding,
                    variant_fields,
                }) = self.resolve_bit_packed_nominal(ty, source)?
                else {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                };
                let variant = encoding
                    .variants()
                    .get(*index)
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
                let field_types = variant_fields
                    .get(*index)
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
                if fields.len() != field_types.len() {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
                let mut payloads = Vec::with_capacity(fields.len());
                for (field_val, field_ty) in fields.iter().zip(field_types) {
                    payloads.push(self.encode_bit_packed_at(
                        field_val,
                        field_ty,
                        depth + 1,
                        source,
                    )?);
                }
                self.bit_operation(source, |budget| {
                    variant
                        .construct(&payloads, budget)
                        .map_err(|error| match error {
                            nir::enum_encoding::Error::Bits(error) => error,
                            _ => BitError::InvalidRepresentation,
                        })
                })
            }
            (
                Value::Record {
                    declaration,
                    fields,
                    ..
                },
                Term::Nominal {
                    declaration: ty_decl,
                    ..
                },
            ) if declaration == ty_decl => {
                let Some(BitPackedNominal::Struct {
                    fields: field_types,
                }) = self.resolve_bit_packed_nominal(ty, source)?
                else {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                };
                if fields.len() != field_types.len() {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
                let mut acc =
                    self.bit_operation(source, |budget| BitVec::from_words(0, &[], budget))?;
                for (field_val, field_ty) in fields.iter().zip(&field_types) {
                    let field_bits =
                        self.encode_bit_packed_at(field_val, field_ty, depth + 1, source)?;
                    acc = self.bit_operation(source, |budget| field_bits.concat(&acc, budget))?;
                }
                Ok(acc)
            }
            _ => Err(self.failure(ErrorKind::Type, Some(source))),
        }
    }

    pub(super) fn decode_bit_packed_value(
        &mut self,
        bits: &BitVec,
        ty: &Term,
        source: SourceRange,
    ) -> Result<Value, Error> {
        self.decode_bit_packed_at(bits, ty, 0, source)
    }

    fn decode_bit_packed_at(
        &mut self,
        bits: &BitVec,
        ty: &Term,
        depth: usize,
        source: SourceRange,
    ) -> Result<Value, Error> {
        if depth >= MAX_BIT_PACK_DEPTH {
            return Err(self.failure(ErrorKind::StepsExhausted, Some(source)));
        }
        self.charge_preparation(1)?;
        match ty {
            Term::Primitive(Primitive::Void) => {
                if bits.width() != 0 {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
                Ok(Value::Unit)
            }
            Term::Primitive(Primitive::Bool) => {
                if bits.width() != 1 {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
                let bit = bits.words().first().copied().unwrap_or(0) & 1 != 0;
                Ok(Value::Bool(bit))
            }
            Term::Primitive(Primitive::BitVec(width)) => {
                if bits.width() != *width {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
                let cloned = self.bit_operation(source, |budget| {
                    BitVec::from_words(bits.width(), bits.words(), budget)
                })?;
                Ok(Value::BitVec(cloned))
            }
            Term::Primitive(Primitive::Char) => {
                if bits.width() != 32 {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
                let raw = bits.words().first().copied().unwrap_or(0);
                let ch = char::from_u32(raw)
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
                Ok(Value::Char(ch))
            }
            Term::Primitive(primitive) => {
                self.decode_unrestricted_scalar(bits, *primitive, false, source)
            }
            Term::Nominal {
                declaration,
                arguments,
            } => match self.resolve_bit_packed_nominal(ty, source)? {
                Some(BitPackedNominal::Newtype { repr }) => {
                    let inner = self.decode_bit_packed_at(bits, &repr, depth + 1, source)?;
                    Ok(Value::Newtype {
                        ty: ty.clone(),
                        value: Box::new(inner),
                    })
                }
                Some(BitPackedNominal::Enum {
                    encoding,
                    variant_fields,
                }) => {
                    if encoding.width() != bits.width() {
                        return Err(self.failure(ErrorKind::Type, Some(source)));
                    }
                    let extracted = self.bit_operation(source, |budget| {
                        for (index, variant) in encoding.variants().iter().enumerate() {
                            let accepted =
                                variant.accepts(bits, budget).map_err(|err| match err {
                                    nir::enum_encoding::Error::Bits(err) => err,
                                    _ => BitError::InvalidRepresentation,
                                })?;
                            if accepted {
                                let slices =
                                    variant.extract(bits, budget).map_err(|err| match err {
                                        nir::enum_encoding::Error::Bits(err) => err,
                                        _ => BitError::InvalidRepresentation,
                                    })?;
                                return Ok(Some((index, slices)));
                            }
                        }
                        Ok(None)
                    })?;
                    let Some((index, slices)) = extracted else {
                        return Err(self.failure(ErrorKind::Type, Some(source)));
                    };
                    let field_types = variant_fields
                        .get(index)
                        .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
                    if slices.len() != field_types.len() {
                        return Err(self.failure(ErrorKind::Type, Some(source)));
                    }
                    let mut decoded_fields = Vec::with_capacity(slices.len());
                    for (slice, field_ty) in slices.iter().zip(field_types) {
                        decoded_fields.push(self.decode_bit_packed_at(
                            slice,
                            field_ty,
                            depth + 1,
                            source,
                        )?);
                    }
                    Ok(Value::Variant {
                        declaration: *declaration,
                        arguments: arguments.clone(),
                        index,
                        fields: decoded_fields,
                    })
                }
                Some(BitPackedNominal::Struct { fields }) => {
                    let mut fuel = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
                    let before = fuel;
                    let layout = nir::bit_packed_struct_layout(
                        &observations::InterpreterLayoutWorld(self),
                        &fields,
                        &mut fuel,
                    );
                    self.budget.steps -= (before - fuel) as u64;
                    let layout = layout.map_err(|err| match err {
                        nir::BitPackedError::Layout(nir::LayoutError::BudgetExhausted) => {
                            self.failure(ErrorKind::StepsExhausted, Some(source))
                        }
                        _ => self.failure(ErrorKind::Type, Some(source)),
                    })?;
                    if layout.total_bits as usize != bits.width()
                        || layout.fields.len() != fields.len()
                    {
                        return Err(self.failure(ErrorKind::Type, Some(source)));
                    }
                    let mut decoded_fields = Vec::with_capacity(fields.len());
                    for (field_layout, field_ty) in layout.fields.iter().zip(&fields) {
                        let start = field_layout.bit_offset as usize;
                        let end = start + field_layout.bit_width as usize;
                        let slice =
                            self.bit_operation(source, |budget| bits.slice(start, end, budget))?;
                        decoded_fields.push(self.decode_bit_packed_at(
                            &slice,
                            field_ty,
                            depth + 1,
                            source,
                        )?);
                    }
                    Ok(Value::Record {
                        arguments: arguments.clone(),
                        declaration: *declaration,
                        fields: decoded_fields,
                    })
                }
                None => Err(self.failure(ErrorKind::Type, Some(source))),
            },
            _ => Err(self.failure(ErrorKind::Type, Some(source))),
        }
    }

    pub(super) fn take_bits_value(
        &mut self,
        value: Value,
        input: &Term,
        output: &Term,
        source: SourceRange,
    ) -> Result<Value, Error> {
        let Term::Primitive(Primitive::BitVec(width)) = *output else {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        };
        Ok(match value {
            Value::BitVec(bits) if bits.width() == width => Value::BitVec(bits),
            value @ Value::Variant { .. } => {
                self.check_retained_value(&value, input, Some(source))?;
                let Value::Variant {
                    declaration,
                    index,
                    fields,
                    arguments,
                } = value
                else {
                    unreachable!()
                };
                let definition = if self.graph.is_some() {
                    // The value check may have selected an associated input view. Use
                    // the checked value's full concrete identity for lookup.
                    let subject = Term::Nominal {
                        declaration,
                        arguments,
                    };
                    match self.concrete_nominal_metered(&subject)? {
                        Some(ConcreteNominal::Enum(definition)) => Some(definition),
                        _ => None,
                    }
                } else {
                    self.enums.get(&declaration).cloned()
                };
                let layout = definition
                    .as_ref()
                    .and_then(|definition| definition.encoding.as_ref())
                    .filter(|layout| layout.width() == width)
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
                let variant = layout
                    .variants()
                    .get(index)
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
                Value::BitVec(self.bit_operation(source, |budget| {
                    let mut payloads = Vec::new();
                    for (field, value) in variant.fields().iter().zip(&fields) {
                        let bits = if let Value::BitVec(bits) = value {
                            BitVec::from_words(bits.width(), bits.words(), budget)?
                        } else {
                            let raw = match value {
                                Value::UnsignedInteger(value) => *value,
                                Value::SignedInteger(value) => *value as u128,
                                Value::Float32(value) => u128::from(*value),
                                Value::Float64(value) => u128::from(*value),
                                _ => return Err(BitError::InvalidRepresentation),
                            };
                            if field.width > 128 {
                                return Err(BitError::InvalidRepresentation);
                            }
                            let raw = if field.width == 128 {
                                raw
                            } else {
                                raw & ((1u128 << field.width) - 1)
                            };
                            comptime_runtime::Integer::from_u128(raw)
                                .to_bitvec(field.width, budget)?
                        };
                        payloads.push(bits);
                    }
                    variant
                        .construct(&payloads, budget)
                        .map_err(|error| match error {
                            nir::enum_encoding::Error::Bits(error) => error,
                            _ => BitError::InvalidRepresentation,
                        })
                })?)
            }
            value @ Value::Record { .. } => {
                self.check_retained_value(&value, input, Some(source))?;
                let bits = self.encode_bit_packed_value(&value, input, source)?;
                if bits.width() != width {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
                Value::BitVec(bits)
            }
            Value::SignedInteger(_)
            | Value::UnsignedInteger(_)
            | Value::Float16(_)
            | Value::BFloat16(_)
            | Value::Float32(_)
            | Value::Float64(_)
            | Value::Char(_) => {
                let Term::Primitive(primitive) = *input else {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                };
                if primitive
                    .readable_scalar_width_in(self.target)
                    .map(|w| w as usize)
                    != Some(width)
                {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
                self.check_retained_value(&value, input, Some(source))?;
                let raw = match value {
                    Value::SignedInteger(v) => v as u128,
                    Value::UnsignedInteger(v) => v,
                    Value::Float16(bits) | Value::BFloat16(bits) => u128::from(bits),
                    Value::Float32(bits) => u128::from(bits),
                    Value::Float64(bits) => u128::from(bits),
                    Value::Char(value) => u128::from(u32::from(value)),
                    _ => unreachable!(),
                };
                let raw = if width == 128 {
                    raw
                } else {
                    raw & ((1u128 << width) - 1)
                };
                Value::BitVec(self.bit_operation(source, |budget| {
                    comptime_runtime::Integer::from_u128(raw).to_bitvec(width, budget)
                })?)
            }
            _ => return Err(self.failure(ErrorKind::Type, Some(source))),
        })
    }
}
