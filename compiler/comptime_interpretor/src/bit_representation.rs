//! Read defined scalar and enum representations without retaining a place.
use super::*;

impl Interpreter<'_> {
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
                if !self.has_type(&value, input) {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
                let Value::Variant {
                    declaration,
                    index,
                    fields,
                    arguments,
                } = value
                else {
                    unreachable!()
                };
                let definition = if let Some(graph) = self.graph {
                    graph
                        .nominals
                        .iter()
                        .find(|nominal| {
                            matches!(&nominal.ty,
                                Term::Nominal {declaration:actual,arguments:actual_args}
                                    if *actual == declaration && *actual_args == arguments)
                        })
                        .and_then(|nominal| match &nominal.definition {
                            iir::NominalDefinition::Enum(definition) => {
                                Some(generated_enums::EnumDefinition::Borrowed(definition))
                            }
                            _ => None,
                        })
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
                    use comptime_runtime::bitvec::{BitVec, Error as BitError};
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
                    || !self.has_type(&value, input)
                {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
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
