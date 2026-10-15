//! Compile the fixed-mask enum fragment into one constructor/discriminator plan.
use super::*;
use comptime_runtime::{Budget, Integer, bitvec::BitVec};
use nir::enum_encoding::{self as encoding, Field, Layout, Representation, Variant};

pub(super) struct EnumEncoding {
    pub bits: Option<Representation>,
    pub symbols: Option<encoding::SymbolRepresentation>,
    pub payloads: Vec<Vec<Term>>,
}

impl Checker<'_, '_> {
    fn enum_encoding_work<T>(
        &mut self,
        span: Span,
        work: impl FnOnce(&mut Budget) -> Result<T, encoding::Error>,
    ) -> Result<T, Error> {
        let steps = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let mut budget = Budget {
            steps,
            max_limbs: 4096,
        };
        let result = work(&mut budget);
        self.required_steps -= (steps - budget.steps) as u64;
        result.map_err(|failure| error(span, failure.to_string()))
    }

    pub(super) fn enum_encoding(
        &mut self,
        def: &hir::item::EnumDef<'_>,
        leaves: &[(&hir::item::Variant<'_>, String)],
        span: Span,
    ) -> Result<Option<EnumEncoding>, Error> {
        if def.configurations.is_empty() {
            return Ok(None);
        }
        let (mut base_width, mut explicit_width, mut discrete) = (None, None, false);
        let mut base_type = None;
        let mut seen = BTreeSet::new();
        for configuration in &def.configurations {
            let name = configuration.ident.name.as_str();
            if !seen.insert(name) {
                return Err(error(
                    configuration.span,
                    "duplicate enum representation configuration",
                ));
            }
            match name {
                "base_type" => {
                    let ty = self.type_expr(configuration.value)?;
                    let shape = self.structural_type(&ty, configuration.span)?;
                    let Term::Primitive(primitive) = shape else {
                        return Err(error(
                            configuration.span,
                            "enum bit-pattern base type must be an unsigned integer",
                        ));
                    };
                    if primitive == P::Symbol {
                        base_type = Some(primitive);
                        continue;
                    }
                    let Some((false, width)) = primitive.fixed_integer_layout() else {
                        return Err(error(
                            configuration.span,
                            "enum bit-pattern base type must be an unsigned integer",
                        ));
                    };
                    base_width = Some(width as usize);
                    base_type = Some(primitive);
                }
                "width" => {
                    let comptime_interpretor::Value::Integer(width) =
                        self.required_value(configuration.value, P::Integer)?
                    else {
                        unreachable!()
                    };
                    explicit_width = Some(
                        width
                            .to_u128()
                            .ok()
                            .and_then(|n| usize::try_from(n).ok())
                            .filter(|n| *n > 0)
                            .ok_or_else(|| {
                                error(
                                    configuration.span,
                                    "enum width requires a positive representable comptime integer",
                                )
                            })?,
                    );
                }
                "discrete_tag" => {
                    let comptime_interpretor::Value::Bool(value) =
                        self.required_value(configuration.value, P::Bool)?
                    else {
                        unreachable!()
                    };
                    discrete = value;
                }
                _ => {
                    return Err(error(
                        configuration.span,
                        "enum configuration key is not yet supported",
                    ));
                }
            }
        }
        if !discrete {
            return Err(error(
                span,
                "pattern encoding currently requires .discrete_tag true",
            ));
        }
        if base_type == Some(P::Symbol) {
            if explicit_width.is_some() {
                return Err(error(
                    span,
                    "Symbol enum representation has no fixed bit width",
                ));
            }
            let mut variants = Vec::new();
            for (variant, name) in leaves {
                let expr = match &variant.kind {
                    hir::item::VariantKind::Const(expr) => *expr,
                    hir::item::VariantKind::Pattern(pattern) => match &pattern.kind {
                        hir::PatternKind::Const(expr) => *expr,
                        _ => {
                            return Err(error(
                                pattern.span,
                                "Symbol enum patterns currently require one constant Symbol",
                            ));
                        }
                    },
                    _ => {
                        return Err(error(
                            variant.span,
                            "Symbol enum encoding requires an explicit value for every variant",
                        ));
                    }
                };
                let comptime_interpretor::Value::Symbol(value) =
                    self.required_value(expr, P::Symbol)?
                else {
                    return Err(error(expr.span, "enum expression encoding is not a Symbol"));
                };
                variants.push(encoding::SymbolVariant {
                    name: name.clone(),
                    value: value.to_string(),
                });
            }
            let symbols = self.enum_encoding_work(span, |budget| {
                encoding::SymbolRepresentation::new(variants, budget)
            })?;
            return Ok(Some(EnumEncoding {
                bits: None,
                symbols: Some(symbols),
                payloads: vec![vec![]; leaves.len()],
            }));
        }
        if base_width
            .zip(explicit_width)
            .is_some_and(|(base, width)| base != width)
        {
            return Err(error(
                span,
                "enum width conflicts with its base type representation",
            ));
        }
        let width = base_width.or(explicit_width).ok_or_else(|| {
            error(
                span,
                "enum pattern encoding requires base_type or whole representation width",
            )
        })?;
        // Check limits before building any width-sized buffers.
        let blank = self.enum_encoding_work(span, |budget| {
            let words = width / 32 + usize::from(width % 32 != 0);
            if words > budget.max_limbs {
                return Err(comptime_runtime::Error::SizeLimit.into());
            }
            budget.consume(words.saturating_add(1))?;
            Ok(BitVec::from_words(width, &vec![0; words], budget)?)
        })?;
        let mut variants = Vec::new();
        let mut payload_types = Vec::new();
        for (variant, name) in leaves {
            if let hir::item::VariantKind::Const(expr) = &variant.kind {
                // The representation chooses the input domain. Do not accept
                // arbitrary comptime values merely because this is `= expr`.
                let target = base_type.unwrap_or(P::BitVec(width));
                let value = self.required_value(expr, target)?;
                let bits = match value {
                    comptime_interpretor::Value::UnsignedInteger(value) if base_type.is_some() => {
                        self.enum_encoding_work(expr.span, |budget| {
                            Ok(Integer::from_u128(value).to_bitvec(width, budget)?)
                        })?
                    }
                    comptime_interpretor::Value::BitVec(bits)
                        if base_type.is_none() && bits.width() == width =>
                    {
                        bits
                    }
                    _ => {
                        return Err(error(
                            expr.span,
                            "enum expression encoding does not belong to its representation domain",
                        ));
                    }
                };
                variants.push(self.enum_encoding_work(expr.span, |budget| {
                    let mask = blank.not(budget)?;
                    Variant::new(name.clone(), mask, bits, vec![], budget)
                })?);
                payload_types.push(vec![]);
                continue;
            }
            let hir::item::VariantKind::Pattern(pattern) = &variant.kind else {
                return Err(error(
                    variant.span,
                    "fixed enum encoding requires an explicit representation pattern or value for every variant",
                ));
            };
            let hir::PatternKind::BitVec { radix, segments } = &pattern.kind else {
                return Err(error(
                    pattern.span,
                    "enum representation pattern requires fixed bit segments; guards and pattern protocols are not yet supported",
                ));
            };
            let mut mask = blank.clone();
            let mut value = blank.clone();
            let mut remaining = width;
            let mut fields = Vec::new();
            let mut types = Vec::new();
            for segment in segments {
                let (segment_width, constant, field) = match segment {
                    hir::pattern::BitSegment::Literal { spelling, span } => {
                        let (prefix, bits_per_digit) = match radix {
                            2 => ("0b", 1usize),
                            8 => ("0o", 3),
                            16 => ("0x", 4),
                            _ => return Err(error(*span, "invalid enum representation radix")),
                        };
                        let segment_width = spelling
                            .bytes()
                            .filter(|c| *c != b'_')
                            .count()
                            .checked_mul(bits_per_digit)
                            .ok_or_else(|| error(*span, "enum segment width overflow"))?;
                        if segment_width > remaining {
                            return Err(error(
                                *span,
                                "enum pattern width differs from the whole representation",
                            ));
                        }
                        let bits = self.enum_encoding_work(*span, |budget| {
                            Ok(
                                Integer::parse_literal(&format!("{prefix}{spelling}"), budget)?
                                    .to_bitvec(segment_width, budget)?,
                            )
                        })?;
                        (segment_width, Some(bits), None)
                    }
                    hir::pattern::BitSegment::Computed(expr) => {
                        let comptime_interpretor::Value::BitVec(bits) =
                            self.required_expression(expr, None)?
                        else {
                            return Err(error(
                                expr.span,
                                "computed enum encoding segment requires a concrete BitVec",
                            ));
                        };
                        (bits.width(), Some(bits), None)
                    }
                    hir::pattern::BitSegment::Typed { pattern, ty } => {
                        let ty = self.type_expr(ty)?;
                        let width = match &ty {
                            Term::Primitive(P::BitVec(width)) => Some(*width),
                            Term::Primitive(primitive) => primitive.unrestricted_scalar_width().map(|n|n as usize),
                            _ => None,
                        }.ok_or_else(||error(pattern.span,"encoded payload requires a completed scalar or BitVec representation"))?;
                        let hir::PatternKind::Binding(hir::BindingMode::ByValue, name, None) =
                            &pattern.kind
                        else {
                            return Err(error(
                                pattern.span,
                                "encoded payload currently requires a named unconstrained binding",
                            ));
                        };
                        types.push(ty);
                        (width, None, Some(name.name.to_string()))
                    }
                };
                let start = remaining.checked_sub(segment_width).ok_or_else(|| {
                    error(
                        pattern.span,
                        "enum pattern width differs from the whole representation",
                    )
                })?;
                if let Some(constant) = constant {
                    let (next_mask, next_value) =
                        self.enum_encoding_work(pattern.span, |budget| {
                            let ones = constant.not(budget)?.or(&constant, budget)?;
                            Ok((
                                mask.replaced(start, &ones, budget)?,
                                value.replaced(start, &constant, budget)?,
                            ))
                        })?;
                    mask = next_mask;
                    value = next_value;
                }
                if let Some(name) = field {
                    fields.push(Field {
                        name,
                        start,
                        width: segment_width,
                    });
                }
                remaining = start;
            }
            if remaining != 0 {
                return Err(error(
                    pattern.span,
                    "enum pattern width differs from the whole representation",
                ));
            }
            variants.push(self.enum_encoding_work(pattern.span, |budget| {
                Variant::new(name.clone(), mask, value, fields, budget)
            })?);
            payload_types.push(types);
        }
        let layout =
            self.enum_encoding_work(span, |budget| Layout::new(width, variants, budget))?;
        Ok(Some(EnumEncoding {
            bits: Some(Representation { base_type, layout }),
            symbols: None,
            payloads: payload_types,
        }))
    }
}
