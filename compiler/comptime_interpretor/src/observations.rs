//! Scalar and metadata observations kept outside the recursive dispatcher.
use super::*;

impl Interpreter<'_> {
    #[inline(never)]
    pub(super) fn eval_build_metadata(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        match &expr.kind {
            K::BuildMetadata {
                owner,
                selector,
                entries,
            } => {
                if !self.comptime_only {
                    return Err(self.failure(ErrorKind::PhaseMismatch, Some(expr.source)));
                }
                let Value::String(alias) = value!(self.eval(selector, environment)) else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                for (name, provider, value) in entries {
                    self.integer_operation(Some(expr.source), |budget| {
                        budget.consume(name.len().min(alias.len()).saturating_add(1))
                    })?;
                    if name == alias.as_ref() {
                        let value = value.as_ref().ok_or_else(|| {
                            self.failure(
                                ErrorKind::BuildMetadataUnavailable {
                                    owner: *owner,
                                    provider: Some(*provider),
                                    alias: alias.to_string(),
                                },
                                Some(expr.source),
                            )
                        })?;
                        return self
                            .decode_immutable_input(value, &expr.ty)
                            .map(Flow::Value)
                            .map_err(|mut error| {
                                error.source = Some(expr.source);
                                error
                            });
                    }
                }
                Err(self.failure(
                    ErrorKind::BuildMetadataUnavailable {
                        owner: *owner,
                        provider: None,
                        alias: alias.to_string(),
                    },
                    Some(expr.source),
                ))
            }
            _ => unreachable!("expression dispatcher selected an incompatible observation"),
        }
    }
    pub(super) fn eval_observation(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        use comptime_compiler::observations::{Kind as O, Observation};
        let (kind, operand) = match &expr.kind {
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
            O::EnumName => {
                if *output != Term::Primitive(Primitive::Str)
                    || !matches!(*input, Term::Nominal { .. })
                {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
                let Term::Nominal { declaration, .. } = input else {
                    unreachable!()
                };
                let is_enum = if let Some(graph) = self.graph {
                    graph.nominals.iter().any(|nominal| {
                        nominal.ty == *input
                            && matches!(nominal.definition, iir::NominalDefinition::Enum(_))
                    })
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
            O::EnumName => {
                if !self.has_type(&value, input) {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
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
                let definition = if let Some(graph) = self.graph {
                    graph
                        .nominals
                        .iter()
                        .find(|nominal| {
                            matches!(&nominal.ty,
                        Term::Nominal { declaration: actual, arguments: actual_args }
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
    pub(super) fn eval_snapshot(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        match &expr.kind {
            K::Snapshot(value) => {
                if value.ty != expr.ty {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                self.eval(value, environment)
            }
            _ => unreachable!("expression dispatcher selected an incompatible observation"),
        }
    }
    #[inline(never)]
    pub(super) fn eval_comptime_integer_convert(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        match &expr.kind {
            K::ComptimeIntegerConvert(operand) => {
                if !self.comptime_only {
                    return Err(self.failure(ErrorKind::PhaseMismatch, Some(expr.source)));
                }
                if !nir::program::comptime_integer_conversion_valid(&operand.ty, &expr.ty) {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                let value = value!(self.eval(operand, environment));
                if !self.has_type(&value, &operand.ty) {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                let integer = match value {
                    Value::Integer(value) => value,
                    Value::UnsignedInteger(value) => comptime_runtime::Integer::from_u128(value),
                    Value::SignedInteger(value) => comptime_runtime::Integer::from_i128(value),
                    _ => return Err(self.failure(ErrorKind::Type, Some(expr.source))),
                };
                let Term::Primitive(target) = expr.ty else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                if target == Primitive::Integer {
                    return Ok(Flow::Value(Value::Integer(integer)));
                }
                let (signed, width) = self
                    .integer_layout(target)
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(expr.source)))?;
                self.tick(Some(expr.source))?;
                let converted = if signed {
                    integer.to_signed(width).map(Value::SignedInteger)
                } else {
                    integer.to_unsigned(width).map(Value::UnsignedInteger)
                }
                .map_err(|_| self.failure(ErrorKind::MachineOverflow, Some(expr.source)))?;
                Ok(Flow::Value(converted))
            }
            _ => unreachable!("expression dispatcher selected an incompatible observation"),
        }
    }
    #[inline(never)]
    pub(super) fn eval_integer_representation(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        match &expr.kind {
            K::IntegerRepresentation(operand) => {
                if !nir::program::integer_representation_valid(&operand.ty, &expr.ty) {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                let value = value!(self.eval(operand, environment));
                let Term::Primitive(primitive) = operand.ty else {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                };
                let (_, width) = self
                    .integer_layout(primitive)
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(expr.source)))?;
                if !self.has_type(&value, &operand.ty) {
                    return Err(self.failure(ErrorKind::Type, Some(expr.source)));
                }
                let bits = match value {
                    Value::UnsignedInteger(value) => value,
                    Value::SignedInteger(value) => value as u128,
                    _ => return Err(self.failure(ErrorKind::Type, Some(expr.source))),
                };
                Ok(Flow::Value(Value::UnsignedInteger(
                    bits & (u128::MAX >> (128 - width)),
                )))
            }
            _ => unreachable!("expression dispatcher selected an incompatible observation"),
        }
    }
}
