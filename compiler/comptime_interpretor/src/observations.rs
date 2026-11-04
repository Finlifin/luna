//! Scalar and metadata observations kept outside the recursive dispatcher.
use super::*;

impl Interpreter<'_> {
    pub(super) fn check_build_metadata(&self, source: SourceRange) -> Result<(), Error> {
        if !self.comptime_only {
            return Err(self.failure(ErrorKind::PhaseMismatch, Some(source)));
        }
        Ok(())
    }

    pub(super) fn read_build_metadata(
        &mut self,
        owner: u64,
        output: &Term,
        entries: &[comptime_compiler::metadata::Entry],
        selector: Value,
        source: SourceRange,
    ) -> Result<Value, Error> {
        let Value::String(alias) = selector else {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        };
        for (name, provider, value) in entries {
            self.integer_operation(Some(source), |budget| {
                budget.consume(name.len().min(alias.len()).saturating_add(1))
            })?;
            if name == alias.as_ref() {
                let value = value.as_ref().ok_or_else(|| {
                    self.failure(
                        ErrorKind::BuildMetadataUnavailable {
                            owner,
                            provider: Some(*provider),
                            alias: alias.to_string(),
                        },
                        Some(source),
                    )
                })?;
                return self
                    .decode_immutable_input(value, output)
                    .map_err(|mut error| {
                        error.source = Some(source);
                        error
                    });
            }
        }
        Err(self.failure(
            ErrorKind::BuildMetadataUnavailable {
                owner,
                provider: None,
                alias: alias.to_string(),
            },
            Some(source),
        ))
    }

    pub(super) fn eval_build_metadata(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let K::BuildMetadata {
            owner,
            selector,
            entries,
        } = &expr.kind
        else {
            unreachable!()
        };
        self.check_build_metadata(expr.source)?;
        let selector = value!(self.eval(selector, environment));
        self.read_build_metadata(*owner, &expr.ty, entries, selector, expr.source)
            .map(Flow::Value)
    }
    pub(super) fn eval_observation(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        use comptime_compiler::observations::{Kind as O, Observation};
        let (kind, operand) = match &expr.kind {
            K::NewtypeRepresentation(operand) => (O::NewtypeRepresentation, operand.as_ref()),
            K::StructFieldTypes(operand) => (O::StructFields { names: false }, operand.as_ref()),
            K::StructFieldNames(operand) => (O::StructFields { names: true }, operand.as_ref()),
            K::EnumStructure { subject, query } => (O::EnumStructure(*query), subject.as_ref()),
            K::EnumName(operand) => (O::EnumName, operand.as_ref()),
            K::SymbolName(operand) => (O::SymbolName, operand.as_ref()),
            K::TextQuery { subject, query } => (O::Text(*query), subject.as_ref()),
            _ => unreachable!("observation dispatcher"),
        };
        let observation = Observation {
            kind,
            input: &operand.ty,
            output: &expr.ty,
            operand_source: operand.source,
        };
        self.check_observation(&observation, expr.source)?;
        let value = value!(self.eval(operand, environment));
        self.observe_value(&observation, value, expr.source)
            .map(Flow::Value)
    }
    pub(super) fn check_observation(
        &self,
        observation: &comptime_compiler::observations::Observation<impl std::borrow::Borrow<Term>>,
        source: SourceRange,
    ) -> Result<(), Error> {
        use comptime_compiler::observations::Kind as O;
        let input: &Term = observation.input.borrow();
        let output: &Term = observation.output.borrow();
        match observation.kind {
            O::EnumStructure(nir::program::EnumStructureQuery::BitEncoding(query)) => {
                use nir::program::EnumBitQuery as B;
                if !self.comptime_only {
                    return Err(self.failure(ErrorKind::PhaseMismatch, Some(source)));
                }
                let never = Term::Primitive(Primitive::Never);
                let integer = Term::Primitive(Primitive::Integer);
                let valid = match query {
                    B::Width => *output == integer,
                    B::BaseType => {
                        *output == Term::Optional(Box::new(Term::Primitive(Primitive::Type)))
                    }
                    B::Masks | B::Values => self.list_element(output) == Some(&integer),
                    B::Offsets => {
                        self.list_element(output)
                            .and_then(|ty| self.list_element(ty))
                            == Some(&integer)
                    }
                };
                if (*input == never && *output != never)
                    || (*input != never && (*input != Term::Primitive(Primitive::Type) || !valid))
                {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
            }
            O::NewtypeRepresentation => {
                if !self.comptime_only {
                    return Err(self.failure(ErrorKind::PhaseMismatch, Some(source)));
                }
                let type_ty = Term::Primitive(Primitive::Type);
                let never = Term::Primitive(Primitive::Never);
                if !((*input == type_ty && *output == type_ty)
                    || (*input == never && *output == never))
                {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
            }
            O::StructFields { .. } | O::EnumStructure(_) => {
                if !self.comptime_only {
                    return Err(self.failure(ErrorKind::PhaseMismatch, Some(source)));
                }
                let never = Term::Primitive(Primitive::Never);
                let element = Term::Primitive(match observation.kind {
                    O::StructFields { names: false }
                    | O::EnumStructure(nir::program::EnumStructureQuery::FieldTypes) => {
                        Primitive::Type
                    }
                    _ => Primitive::Symbol,
                });
                let result_element = self.list_element(output);
                let expected_element = match observation.kind {
                    O::EnumStructure(query)
                        if !matches!(
                            query,
                            nir::program::EnumStructureQuery::Shapes
                                | nir::program::EnumStructureQuery::SymbolValues
                        ) =>
                    {
                        result_element.and_then(|ty| self.list_element(ty))
                    }
                    _ => result_element,
                };
                if (*input != Term::Primitive(Primitive::Type) && *input != never)
                    || (*input == never && *output != never)
                    || (*input != never && expected_element != Some(&element))
                {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
            }

            O::EnumName => {
                if *output != Term::Primitive(Primitive::Str)
                    || !matches!(*input, Term::Nominal { .. })
                {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
                let Term::Nominal { declaration, .. } = input else {
                    unreachable!()
                };
                let is_enum = if self.graph.is_some() {
                    matches!(self.concrete_nominal(input), Some(ConcreteNominal::Enum(_)))
                } else {
                    self.enums.contains_key(declaration)
                };
                if !is_enum {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
            }
            O::SymbolName => {
                let never = Term::Primitive(Primitive::Never);
                let expected = if *input == never {
                    never.clone()
                } else {
                    Term::Primitive(Primitive::Str)
                };
                if *output != expected
                    || (*input != never && *input != Term::Primitive(Primitive::Symbol))
                {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
            }
            O::Text(query) => {
                if !query.validate(input, output) {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
            }
        }
        Ok(())
    }
    pub(super) fn observe_value(
        &mut self,
        observation: &comptime_compiler::observations::Observation<impl std::borrow::Borrow<Term>>,
        value: Value,
        source: SourceRange,
    ) -> Result<Value, Error> {
        use comptime_compiler::observations::Kind as O;
        let input: &Term = observation.input.borrow();
        let output: &Term = observation.output.borrow();
        match observation.kind {
            O::NewtypeRepresentation => {
                let Value::Type(subject) = value else {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                };
                self.reflect_newtype(&subject)
                    .map(|view| Value::Type(view.representation))
                    .map_err(|mut error| {
                        error.source = Some(source);
                        error
                    })
            }
            O::StructFields { names } => self.observe_struct_fields(value, output, names, source),
            O::EnumStructure(query) => self.observe_enum_structure(value, output, query, source),
            O::EnumName => {
                self.check_value_type(&value, input, Some(source))?;
                self.retain_residual(observation.operand_source, &value)?;
                let Value::Variant {
                    declaration,
                    index,
                    arguments,
                    ..
                } = value
                else {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                };
                let definition = if self.graph.is_some() {
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
                let name = definition
                    .as_ref()
                    .and_then(|definition| definition.variants.get(index))
                    .map(|variant| variant.name.as_str())
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
                self.budget.steps = self
                    .budget
                    .steps
                    .checked_sub(name.len() as u64)
                    .ok_or_else(|| self.failure(ErrorKind::StepsExhausted, Some(source)))?;
                Ok(Value::String(name.into()))
            }
            O::SymbolName => {
                let Value::Symbol(name) = value else {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                };
                Ok(Value::String(name))
            }
            O::Text(query) => {
                let Value::Tuple(values) = value else {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                };
                let text = match values.first() {
                    Some(Value::String(text)) if !query.is_symbol() => text,
                    Some(Value::Symbol(text)) if query.is_symbol() => text,
                    _ => return Err(self.failure(ErrorKind::Type, Some(source))),
                };
                match query {
                    nir::program::TextQueryKind::ByteLength
                    | nir::program::TextQueryKind::SymbolByteLength
                        if values.len() == 1 =>
                    {
                        Ok(Value::UnsignedInteger(text.len() as u128))
                    }
                    nir::program::TextQueryKind::Byte | nir::program::TextQueryKind::SymbolByte
                        if values.len() == 2 =>
                    {
                        let Value::UnsignedInteger(index) = values[1] else {
                            return Err(self.failure(ErrorKind::Type, Some(source)));
                        };
                        if index > u64::MAX as u128 {
                            return Err(self.failure(ErrorKind::Type, Some(source)));
                        }
                        let byte = usize::try_from(index)
                            .ok()
                            .and_then(|index| text.as_bytes().get(index));
                        Ok(Value::Optional {
                            ty: output.clone(),
                            value: byte
                                .map(|byte| Box::new(Value::UnsignedInteger(u128::from(*byte)))),
                        })
                    }
                    _ => Err(self.failure(ErrorKind::Type, Some(source))),
                }
            }
        }
    }
    #[inline(never)]
    pub(super) fn prepare_snapshot(
        &self,
        input: &Term,
        output: &Term,
        source: SourceRange,
    ) -> Result<(), Error> {
        if input != output {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        }
        Ok(())
    }

    pub(super) fn eval_snapshot(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        match &expr.kind {
            K::Snapshot(value) => {
                self.prepare_snapshot(&value.ty, &expr.ty, expr.source)?;
                self.eval(value, environment)
            }
            _ => unreachable!("expression dispatcher selected an incompatible observation"),
        }
    }
}

#[cfg(test)]
#[path = "metadata_tests.rs"]
mod metadata_tests;
