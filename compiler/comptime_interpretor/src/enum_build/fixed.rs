//! Fixed-mask builder requests use the same partition and overlap checks as source enums.
use super::*;
use nir::enum_encoding::{Error as EncodingError, Field, Layout, Representation, Variant};

impl Interpreter<'_> {
    pub(super) fn fixed_enum_encoding(
        &mut self,
        parts: Vec<Value>,
        variants: &[nir::program::Variant],
        source: SourceRange,
    ) -> Result<Representation, Error> {
        let fail = || Error {
            kind: ErrorKind::Type,
            source: Some(source),
        };
        let mut parts = parts.into_iter();
        let (
            Some(Value::Integer(width)),
            Some(Value::Optional { ty, value: base }),
            Some(masks),
            Some(values),
            Some(Value::List {
                ty: offsets_ty,
                values: offsets,
            }),
            None,
        ) = (
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
        )
        else {
            return Err(fail());
        };
        if ty != Term::Optional(Box::new(Term::Primitive(Primitive::Type)))
            || self
                .list_element(&offsets_ty)
                .and_then(|ty| self.list_element(ty))
                != Some(&Term::Primitive(Primitive::Integer))
        {
            return Err(fail());
        }
        let width = width
            .to_u128()
            .ok()
            .and_then(|width| usize::try_from(width).ok())
            .filter(|width| *width > 0)
            .ok_or_else(|| {
                self.fixed_enum_error("width must be a positive addressable Integer", source)
            })?;
        let base_type = match base {
            None => None,
            Some(base) => match *base {
                Value::Type(Term::Primitive(base))
                    if self
                        .target
                        .map_or_else(
                            || base.fixed_integer_layout(),
                            |target| target.integer_layout(base),
                        )
                        .is_some_and(|(signed, bits)| !signed && bits as usize == width) =>
                {
                    Some(base)
                }
                _ => {
                    return Err(self.fixed_enum_error(
                        "base type must be an unsigned integer with the specified width",
                        source,
                    ));
                }
            },
        };
        let masks = self.fixed_enum_integer_list(masks, source)?;
        let values = self.fixed_enum_integer_list(values, source)?;
        if masks.len() != variants.len()
            || values.len() != variants.len()
            || offsets.len() != variants.len()
        {
            return Err(self.fixed_enum_error(
                "masks, values and offset lists must match the leaf variant count",
                source,
            ));
        }
        self.integer_operation(Some(source), |budget| budget.consume(offsets.len()))?;
        let offsets = offsets
            .into_iter()
            .map(|value| self.fixed_enum_integer_list(value, source))
            .collect::<Result<Vec<_>, _>>()?;
        let target = self.target;
        let result = self.integer_operation(Some(source), |budget| {
            let build = || -> Result<Representation, EncodingError> {
                let mut encoded = Vec::with_capacity(variants.len());
                for (((variant, mask), value), offsets) in
                    variants.iter().zip(masks).zip(values).zip(offsets)
                {
                    let names = match &variant.shape {
                        nir::program::VariantShape::Unit if variant.fields.is_empty() => &[][..],
                        nir::program::VariantShape::Named(names) => names.as_slice(),
                        _ => {
                            return Err(EncodingError::Invalid(
                                "fixed encoding requires unit or named payload variants",
                            ));
                        }
                    };
                    if offsets.len() != variant.fields.len() {
                        return Err(EncodingError::Invalid(
                            "payload offsets must match the variant field count",
                        ));
                    }
                    budget.consume(names.len().saturating_add(variant.name.len()))?;
                    let mut fields = Vec::with_capacity(names.len());
                    for ((name, ty), offset) in names.iter().zip(&variant.fields).zip(offsets) {
                        let width = match ty {
                            Term::Primitive(Primitive::BitVec(width)) => Some(*width),
                            Term::Primitive(primitive) => primitive
                                .unrestricted_scalar_width_in(target)
                                .map(|width| width as usize),
                            _ => None,
                        }
                        .ok_or(EncodingError::Invalid(
                            "payload requires a fixed unrestricted scalar or BitVec type",
                        ))?;
                        let start = offset
                            .to_u128()
                            .ok()
                            .and_then(|value| usize::try_from(value).ok())
                            .ok_or(EncodingError::Invalid(
                                "payload offset must be a nonnegative addressable Integer",
                            ))?;
                        budget.consume(name.len())?;
                        fields.push(Field {
                            name: name.clone(),
                            start,
                            width,
                        });
                    }
                    encoded.push(Variant::new(
                        variant.name.clone(),
                        mask.to_bitvec(width, budget)?,
                        value.to_bitvec(width, budget)?,
                        fields,
                        budget,
                    )?);
                }
                Ok(Representation {
                    base_type,
                    layout: Layout::new(width, encoded, budget)?,
                })
            };
            match build() {
                Err(EncodingError::Bits(comptime_runtime::bitvec::Error::Resource(error))) => {
                    Err(error)
                }
                other => Ok(other),
            }
        })?;
        result.map_err(|error| self.fixed_enum_error(&error.to_string(), source))
    }

    fn fixed_enum_integer_list(
        &mut self,
        value: Value,
        source: SourceRange,
    ) -> Result<Vec<comptime_runtime::Integer>, Error> {
        let fail = || Error {
            kind: ErrorKind::Type,
            source: Some(source),
        };
        let Value::List { ty, values } = value else {
            return Err(fail());
        };
        if self.list_element(&ty) != Some(&Term::Primitive(Primitive::Integer)) {
            return Err(fail());
        }
        self.integer_operation(Some(source), |budget| budget.consume(values.len()))?;
        values
            .into_iter()
            .map(|value| match value {
                Value::Integer(value) => Ok(value),
                _ => Err(fail()),
            })
            .collect()
    }

    fn fixed_enum_error(&self, reason: &str, source: SourceRange) -> Error {
        self.failure(
            ErrorKind::InvalidDefinition(DefinitionBuildError::FixedEncoding {
                reason: reason.into(),
            }),
            Some(source),
        )
    }
}
