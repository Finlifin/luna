//! Invocation-local bindings and contextual normal-form construction.
use super::*;

impl Interpreter<'_> {
    pub(super) fn normal_list_domain<'a>(
        &mut self,
        expected: &'a Term,
        count: usize,
        source: SourceRange,
    ) -> Result<&'a Term, Error> {
        if count > 100_000 {
            return Err(self.failure(ErrorKind::RepresentationSizeLimit, Some(source)));
        }
        self.integer_operation(Some(source), |budget| {
            let before = budget.steps;
            let mut fuel = before;
            if nir::type_computations::charge_term(expected, &mut fuel).is_err() {
                return budget.consume(before.saturating_add(1));
            }
            budget.consume(before - fuel)
        })?;
        let element = self
            .list_element(expected)
            .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
        self.integer_operation(Some(source), |budget| budget.consume(count))?;
        Ok(element)
    }
    pub(super) fn prepare_normal_shape(
        &mut self,
        shape: &comptime_compiler::normal_types::Shape,
        expected: &Term,
        prepared: &mut BTreeMap<usize, Prepared>,
        depth: usize,
    ) -> Result<(), Error> {
        let complete =
            self.prepare_input_domain(input_domains::Node::Compiled(shape), expected, depth)?;
        for (key, value) in complete.into_entries() {
            let InputKey::Register(slot) = key else {
                unreachable!("compiled input identity")
            };
            prepared.insert(slot, value);
        }
        Ok(())
    }
    pub(super) fn append_compiled_normal_container(
        &mut self,
        container: &mut Prepared,
        value: Value,
        spread: bool,
        source: SourceRange,
    ) -> Result<(), Error> {
        match container {
            Prepared::List { ty, values } => {
                self.append_normal_list_item(ty, values, spread, value, source)
            }
            Prepared::Tuple { values, .. } => {
                values.push(value);
                Ok(())
            }
            _ => Err(self.failure(ErrorKind::Type, Some(source))),
        }
    }
    pub(super) fn finish_compiled_normal_container(
        &mut self,
        container: Prepared,
        source: SourceRange,
    ) -> Result<Value, Error> {
        match container {
            Prepared::List { ty, values } => self.finish_normal_list(ty, values),
            Prepared::Tuple { values } => Ok(Value::Tuple(values)),
            _ => Err(self.failure(ErrorKind::Type, Some(source))),
        }
    }
    pub(super) fn normal_unit_value(
        &mut self,
        expected: &Term,
        source: SourceRange,
    ) -> Result<Value, Error> {
        // Match ordinary expression elaboration: unit syntax may construct an
        // empty tuple value when the contextual domain supplies that layout.
        if self
            .tuple_input_elements(expected)
            .is_some_and(|fields| fields.is_empty())
        {
            return Ok(Value::Tuple(Vec::new()));
        }
        self.check_type_contract(
            &nir::program::TypePredicate::Equal(Term::Primitive(Primitive::Void), expected.clone()),
            source,
        )?;
        Ok(Value::Unit)
    }

    pub(super) fn normal_symbol_value(
        &mut self,
        expected: &Term,
        name: &str,
        source: SourceRange,
    ) -> Result<Value, Error> {
        let ty = self.prepare_aggregate(expected, source)?;
        let mut layers = Vec::new();
        let mut leaf = &ty;
        while let Term::Optional(inner) | Term::ErrorResult { result: inner, .. } = leaf {
            self.tick(Some(source))?;
            layers.push(leaf);
            leaf = inner;
        }
        let mut value = match leaf {
            Term::Primitive(Primitive::Symbol) => {
                self.integer_operation(Some(source), |budget| {
                    budget.consume(name.len().saturating_add(1))
                })?;
                Value::Symbol(name.into())
            }
            Term::Nominal { declaration, .. } => {
                let definition = self.selected_enum_definition(leaf, Some(source))?;
                let index = definition
                    .unit_variant_index(name, &mut self.budget.steps)
                    .map_err(|reason| {
                        let kind = match reason {
                            nir::program::UnitVariantError::Budget => ErrorKind::StepsExhausted,
                            _ => ErrorKind::EnumVariant(reason),
                        };
                        self.failure(kind, Some(source))
                    })?;
                self.construct_variant(leaf, *declaration, index, vec![], source)?
            }
            _ => return Err(self.failure(ErrorKind::Type, Some(source))),
        };
        // A bare symbol selects a payload. Restore every optional presence
        // and success tag in the same order as ordinary expression delivery.
        for layer in layers.into_iter().rev() {
            value = match layer {
                Term::Optional(_) => self.complete_optional(layer, Some(value), source)?,
                Term::ErrorResult { result, .. } => {
                    let plan = self.prepare_error_conversion(
                        nir::error_results::Conversion::Success,
                        result,
                        layer,
                        source,
                    )?;
                    self.complete_error_conversion(
                        plan.conversion,
                        &plan.input,
                        &plan.output,
                        value,
                        source,
                    )?
                }
                _ => unreachable!("only optional and error result layers were peeled"),
            };
        }
        Ok(value)
    }

    pub(super) fn normal_integer_value(
        &mut self,
        expected: &Term,
        spelling: &str,
        negative: bool,
        source: SourceRange,
    ) -> Result<Value, Error> {
        let integer = self.integer_literal(spelling, Some(source))?;
        let integer = if negative {
            self.integer_operation(Some(source), |budget| integer.negate(budget))?
        } else {
            integer
        };
        let value = match expected {
            Term::Primitive(Primitive::Integer) => Value::Integer(integer),
            Term::Primitive(Primitive::BitVec(width)) => Value::BitVec(
                self.bit_operation(source, |budget| integer.to_bitvec(*width, budget))?,
            ),
            Term::Primitive(primitive) if self.integer_layout(*primitive).is_some() => {
                let (signed, bits) = self.integer_layout(*primitive).unwrap();
                if signed {
                    Value::SignedInteger(
                        integer
                            .to_signed(bits)
                            .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?,
                    )
                } else {
                    Value::UnsignedInteger(
                        integer
                            .to_unsigned(bits)
                            .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?,
                    )
                }
            }
            _ => return Err(self.failure(ErrorKind::Type, Some(source))),
        };
        Ok(value)
    }
    pub(super) fn prepare_normal_value_type(
        &mut self,
        ty: &Term,
        expected: &Term,
        source: SourceRange,
    ) -> Result<(), Error> {
        if ty != expected && *ty != Term::Primitive(Primitive::Never) {
            self.check_type_contract(
                &nir::program::TypePredicate::Equal(ty.clone(), expected.clone()),
                source,
            )?;
        }
        Ok(())
    }
}

/// Prepared domains and accumulators are invocation-local, never code-cache data.
pub(super) enum Prepared {
    Literal(Value),
    List { ty: Term, values: Vec<Value> },
    Tuple { values: Vec<Value> },
}
impl fmt::Display for Prepared {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (kind, values) = match self {
            Self::Literal(value) => return write!(f, "(literal {value})"),
            Self::List { ty, values } => {
                write!(f, "(list (type {ty})")?;
                ("values", values)
            }
            Self::Tuple { values } => {
                write!(f, "(tuple")?;
                ("values", values)
            }
        };
        write!(f, " ({kind}")?;
        for value in values {
            write!(f, " {value}")?;
        }
        write!(f, "))")
    }
}
