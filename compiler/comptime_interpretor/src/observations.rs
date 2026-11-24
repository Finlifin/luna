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
            K::DefinitionAttributes(operand) => (O::DefinitionAttributes, operand.as_ref()),
            K::StructFieldTypes(operand) => (O::StructFields { names: false }, operand.as_ref()),
            K::StructFieldNames(operand) => (O::StructFields { names: true }, operand.as_ref()),
            K::EnumStructure { subject, query } => (O::EnumStructure(*query), subject.as_ref()),
            K::EnumName(operand) => (O::EnumName, operand.as_ref()),
            K::SymbolName(operand) => (O::SymbolName, operand.as_ref()),
            K::TextQuery { subject, query } => (O::Text(*query), subject.as_ref()),
            K::TypeLayout { query, operand } => (O::TypeLayout(*query), operand.as_ref()),
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
            O::DefinitionAttributes => {
                if !self.comptime_only {
                    return Err(self.failure(ErrorKind::PhaseMismatch, Some(source)));
                }
                let never = Term::Primitive(Primitive::Never);
                let expected = if *input == never {
                    never
                } else {
                    Term::Primitive(Primitive::Object)
                };
                if *output != expected {
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
            O::TypeLayout(nir::TypeLayoutQuery::FieldOffset) => {
                let never = Term::Primitive(Primitive::Never);
                let expected = match input {
                    Term::Tuple(items) if items.len() == 2 => {
                        let (a, b) = (&items[0], &items[1]);
                        if *a != Term::Primitive(Primitive::Type) && *a != never {
                            return Err(self.failure(ErrorKind::Type, Some(source)));
                        }
                        if *b != Term::Primitive(Primitive::Symbol) && *b != never {
                            return Err(self.failure(ErrorKind::Type, Some(source)));
                        }
                        if *a == never || *b == never {
                            never.clone()
                        } else {
                            Term::Primitive(Primitive::Usize)
                        }
                    }
                    _ if *input == never => never.clone(),
                    _ => return Err(self.failure(ErrorKind::Type, Some(source))),
                };
                if *output != expected {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
            }
            O::TypeLayout(_) => {
                let never = Term::Primitive(Primitive::Never);
                let expected = if *input == never {
                    never.clone()
                } else {
                    Term::Primitive(Primitive::Usize)
                };
                if *output != expected
                    || (*input != never && *input != Term::Primitive(Primitive::Type))
                {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
            }
        }
        Ok(())
    }

    fn observe_definition_attributes(
        &mut self,
        value: Value,
        source: SourceRange,
    ) -> Result<Value, Error> {
        // A supplied nominal or normal form already identifies its declaration.
        // Only a projection at the root needs to ask for a selected Type.
        let declaration = match value {
            Value::Type(ty) => {
                let ty = if matches!(ty, Term::AssociatedType { .. }) {
                    self.selected_type_view(&ty, Some(source))?
                } else {
                    ty
                };
                match ty {
                    Term::Nominal { declaration, .. } => Some(declaration),
                    Term::NormalForm {
                        head,
                        sort: nir::Sort::Type,
                        ..
                    } => Some(head),
                    Term::Rigid { .. } | Term::Hole { .. } | Term::Bound { .. } => {
                        return Err(self.failure(ErrorKind::TypeContractUnresolved, Some(source)));
                    }
                    _ if ty.sort() == nir::Sort::Type => None,
                    _ => return Err(self.failure(ErrorKind::Type, Some(source))),
                }
            }
            Value::TypeParameter(_) | Value::AbstractValue { .. } | Value::Description(_) => {
                return Err(self.failure(ErrorKind::TypeContractUnresolved, Some(source)));
            }
            Value::Function(id)
            | Value::FunctionInstance {
                declaration: id, ..
            } => Some(id),
            Value::IirCallable(handle) => Some(handle.key.declaration),
            _ => return Err(self.failure(ErrorKind::Type, Some(source))),
        };
        if let Some(id) = declaration {
            let world = self
                .declaration_world
                .ok_or_else(|| self.failure(ErrorKind::TypeContractUnresolved, Some(source)))?;
            let table = &world.definition_attributes;
            let cost = u64::from(usize::BITS - table.len().leading_zeros()) + 1;
            self.budget.steps = self
                .budget
                .steps
                .checked_sub(cost)
                .ok_or_else(|| self.failure(ErrorKind::StepsExhausted, Some(source)))?;
            let supplied;
            let payload = match table.lookup(id) {
                Ok(frozen) => frozen,
                Err(target) => {
                    match self.attribute_resolver.as_ref().ok_or_else(|| {
                        self.failure(ErrorKind::PendingDefinitionAttributes(target), Some(source))
                    })? {
                        source_values::Resolver::Value(resolver) => {
                            supplied = host_budget::checked(
                                &mut self.budget.steps,
                                Some(source),
                                |fuel| resolver(target, fuel),
                            )?;
                        }
                        source_values::Resolver::Source(resolver) => {
                            let known = |id| {
                                self.structs.contains_key(&id)
                                    || self.enums.contains_key(&id)
                                    || self.newtypes.contains_key(&id)
                            };
                            let delivery = host_budget::checked(
                                &mut self.budget.steps,
                                Some(source),
                                |fuel| resolver(target, &known, fuel),
                            )?;
                            let Some(delivery) = delivery else {
                                return Ok(Value::Object(Object::from_entries(Vec::new())));
                            };
                            return delivery
                                .decode(self, &Term::Primitive(Primitive::Object))
                                .map_err(|mut e| {
                                    e.source = Some(source);
                                    e
                                });
                        }
                    }
                    supplied.as_deref()
                }
            };
            if let Some(frozen) = payload {
                return self
                    .decode_immutable_input(frozen, &Term::Primitive(Primitive::Object))
                    .map_err(|mut error| {
                        error.source = Some(source);
                        error
                    });
            }
        }
        Ok(Value::Object(Object::from_entries(vec![])))
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
            O::DefinitionAttributes => self.observe_definition_attributes(value, source),
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
                if query == nir::program::TextQueryKind::FromChar {
                    let [Value::Char(ch)] = values.as_slice() else {
                        return Err(self.failure(ErrorKind::Type, Some(source)));
                    };
                    let mut buf = [0u8; 4];
                    let encoded = ch.encode_utf8(&mut buf);
                    self.budget.steps = self
                        .budget
                        .steps
                        .checked_sub(encoded.len() as u64)
                        .ok_or_else(|| self.failure(ErrorKind::StepsExhausted, Some(source)))?;
                    return Ok(Value::String(encoded.into()));
                }
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
                    nir::program::TextQueryKind::Slice if values.len() == 3 => {
                        let (Value::UnsignedInteger(start), Value::UnsignedInteger(end)) =
                            (&values[1], &values[2])
                        else {
                            return Err(self.failure(ErrorKind::Type, Some(source)));
                        };
                        if *start > u64::MAX as u128 || *end > u64::MAX as u128 {
                            return Err(self.failure(ErrorKind::Type, Some(source)));
                        }
                        let sliced = (|| {
                            let start = usize::try_from(*start).ok()?;
                            let end = usize::try_from(*end).ok()?;
                            if start > end
                                || end > text.len()
                                || !text.is_char_boundary(start)
                                || !text.is_char_boundary(end)
                            {
                                return None;
                            }
                            Some(&text[start..end])
                        })();
                        let sliced = match sliced {
                            Some(slice) => {
                                self.budget.steps = self
                                    .budget
                                    .steps
                                    .checked_sub(slice.len() as u64)
                                    .ok_or_else(|| {
                                        self.failure(ErrorKind::StepsExhausted, Some(source))
                                    })?;
                                Some(Box::new(Value::String(slice.into())))
                            }
                            None => None,
                        };
                        Ok(Value::Optional {
                            ty: output.clone(),
                            value: sliced,
                        })
                    }
                    nir::program::TextQueryKind::Concat if values.len() == 2 => {
                        let Value::String(other) = &values[1] else {
                            return Err(self.failure(ErrorKind::Type, Some(source)));
                        };
                        let total_len = text.len().saturating_add(other.len()) as u64;
                        self.budget.steps =
                            self.budget.steps.checked_sub(total_len).ok_or_else(|| {
                                self.failure(ErrorKind::StepsExhausted, Some(source))
                            })?;
                        Ok(Value::String(format!("{text}{other}").into()))
                    }
                    _ => Err(self.failure(ErrorKind::Type, Some(source))),
                }
            }
            O::TypeLayout(query) => self.observe_type_layout(query, value, source),
        }
    }

    fn observe_type_layout(
        &mut self,
        query: nir::TypeLayoutQuery,
        value: Value,
        source: SourceRange,
    ) -> Result<Value, Error> {
        let (type_value, field_name) = if query == nir::TypeLayoutQuery::FieldOffset {
            let Value::Tuple(mut items) = value else {
                return Err(self.failure(ErrorKind::Type, Some(source)));
            };
            let (Some(field_val), Some(type_val)) = (items.pop(), items.pop()) else {
                return Err(self.failure(ErrorKind::Type, Some(source)));
            };
            if !items.is_empty() {
                return Err(self.failure(ErrorKind::Type, Some(source)));
            }
            let Value::Symbol(field) = field_val else {
                return Err(self.failure(ErrorKind::Type, Some(source)));
            };
            (type_val, Some(field))
        } else {
            (value, None)
        };
        let subject = self.type_identity_term(type_value, source)?;
        let subject = self.selected_type_view(&subject, Some(source))?;
        self.check_type_description(&subject, Some(source))?;
        self.admit_type(&subject, 0).map_err(|mut error| {
            if error.source.is_none() {
                error.source = Some(source);
            }
            error
        })?;
        let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let result = nir::compute_type_layout_query(
            &InterpreterLayoutWorld(self),
            query,
            &subject,
            field_name.as_deref(),
            &mut fuel,
        );
        self.budget.steps -= (before - fuel) as u64;
        let raw = result.map_err(|err| {
            let kind = match err {
                nir::LayoutError::BudgetExhausted => ErrorKind::StepsExhausted,
                nir::LayoutError::TargetRequired | nir::LayoutError::Unresolved(_) => {
                    ErrorKind::TypeContractUnresolved
                }
                nir::LayoutError::Overflow => ErrorKind::MachineOverflow,
                nir::LayoutError::ComptimeOnly(_)
                | nir::LayoutError::Cycle(_)
                | nir::LayoutError::Unsupported(_)
                | nir::LayoutError::UnknownField { .. }
                | nir::LayoutError::UnalignedBitField { .. } => ErrorKind::Type,
            };
            self.failure(kind, Some(source))
        })?;
        let val = u128::from(raw);
        if let Some(target) = self.target
            && target.pointer_bits() < 128
            && let Some(limit) = 1u128.checked_shl(target.pointer_bits())
            && val >= limit
        {
            return Err(self.failure(ErrorKind::MachineOverflow, Some(source)));
        }
        Ok(Value::UnsignedInteger(val))
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

impl Interpreter<'_> {
    pub(super) fn evaluate_type_kind(&self, kind: nir::TypeKind, ty: &Term) -> Option<bool> {
        nir::layout::evaluate_kind(&InterpreterLayoutWorld(self), kind, ty)
    }
}

pub(super) struct InterpreterLayoutWorld<'a, 'b>(pub(super) &'a Interpreter<'b>);

impl nir::LayoutWorld for InterpreterLayoutWorld<'_, '_> {
    fn target(&self) -> Option<nir::target::TargetData> {
        self.0.target
    }

    fn is_tuple_head(&self, head: DeclarationId) -> bool {
        self.0
            .normal_forms
            .get(&head)
            .is_some_and(|meta| meta.role == nir::normal_forms::Role::Tuple)
    }

    fn is_list_head(&self, head: DeclarationId) -> bool {
        self.0
            .normal_forms
            .get(&head)
            .is_some_and(|meta| meta.role == nir::normal_forms::Role::TypeList)
    }

    fn nominal_kind(&self, ty: &Term) -> Option<nir::NominalKind> {
        let Term::Nominal { declaration, .. } = ty else {
            return None;
        };
        if self.0.graph.is_some() {
            return Some(match self.0.concrete_nominal(ty)? {
                ConcreteNominal::Struct(_) => nir::NominalKind::Struct,
                ConcreteNominal::Enum(_) => nir::NominalKind::Enum,
                ConcreteNominal::Newtype(_) => nir::NominalKind::Newtype,
            });
        }
        if self.0.structs.contains_key(declaration) {
            Some(nir::NominalKind::Struct)
        } else if self.0.enums.contains_key(declaration) {
            Some(nir::NominalKind::Enum)
        } else if self.0.newtypes.contains_key(declaration) {
            Some(nir::NominalKind::Newtype)
        } else {
            None
        }
    }

    fn is_template_identity(&self, ty: &Term) -> bool {
        self.0
            .graph
            .is_some_and(|graph| graph.templates.iter().any(|t| t.logical_type() == ty))
    }

    fn nominal_layout(
        &self,
        ty: &Term,
        fuel: &mut usize,
    ) -> Result<Option<nir::NominalLayout>, nir::LayoutError> {
        let Term::Nominal {
            declaration,
            arguments,
        } = ty
        else {
            return Ok(None);
        };
        let admission_err = |err: nir::NominalAdmissionError| match err {
            nir::NominalAdmissionError::Budget => nir::LayoutError::BudgetExhausted,
            nir::NominalAdmissionError::Unresolved(_) => nir::LayoutError::Unresolved(ty.clone()),
            nir::NominalAdmissionError::Invalid(_) => nir::LayoutError::Unsupported(ty.clone()),
        };
        if self.0.graph.is_some() {
            let Some(concrete) = self.0.concrete_nominal(ty) else {
                return Ok(None);
            };
            return Ok(Some(match concrete {
                ConcreteNominal::Struct(def) => {
                    let fields = def.fields.iter().map(|(_, t)| t.clone()).collect();
                    match def.packed {
                        nir::program::PackedMode::None => nir::NominalLayout::Struct(fields),
                        nir::program::PackedMode::Byte => nir::NominalLayout::PackedStruct(fields),
                        nir::program::PackedMode::Bit => {
                            nir::NominalLayout::BitPackedStruct(fields)
                        }
                    }
                }
                ConcreteNominal::Newtype(def) => {
                    nir::NominalLayout::Newtype(def.representation.clone())
                }
                ConcreteNominal::Enum(def) => {
                    if def.symbol_encoding.is_some() {
                        nir::NominalLayout::EnumSymbol
                    } else if let Some(encoding) = &def.encoding {
                        let bits = u32::try_from(encoding.width())
                            .map_err(|_| nir::LayoutError::Unsupported(ty.clone()))?;
                        nir::NominalLayout::EnumFixedBits {
                            width: bits,
                            niche: encoding.layout.contiguous_niche(),
                            exhaustive: encoding.layout.is_exhaustive(),
                        }
                    } else if def.variants.iter().all(|v| v.fields.is_empty()) {
                        nir::NominalLayout::EnumUnit(def.variants.len())
                    } else {
                        nir::NominalLayout::EnumAggregate(
                            def.variants.iter().map(|v| v.fields.clone()).collect(),
                        )
                    }
                }
            }));
        }
        if let Some(def) = self.0.structs.get(declaration) {
            let fields = nir::normal_forms::instantiate_value_fields_checked(
                &def.type_parameters,
                &def.parameter_types,
                arguments,
                def.fields.iter().map(|(_, t)| t),
                fuel,
            )
            .map_err(admission_err)?;
            return Ok(Some(match def.packed {
                nir::program::PackedMode::None => nir::NominalLayout::Struct(fields),
                nir::program::PackedMode::Byte => nir::NominalLayout::PackedStruct(fields),
                nir::program::PackedMode::Bit => nir::NominalLayout::BitPackedStruct(fields),
            }));
        }
        if let Some(def) = self.0.newtypes.get(declaration) {
            let repr =
                def.representation_budgeted(arguments, &[], fuel)
                    .map_err(|err| match err {
                        nir::newtypes::ConversionError::Budget => nir::LayoutError::BudgetExhausted,
                        _ => nir::LayoutError::Unsupported(ty.clone()),
                    })?;
            return Ok(Some(nir::NominalLayout::Newtype(repr)));
        }
        if let Some(def) = self.0.enums.get(declaration) {
            if def.symbol_encoding.is_some() {
                return Ok(Some(nir::NominalLayout::EnumSymbol));
            }
            if let Some(encoding) = &def.encoding {
                let bits = u32::try_from(encoding.width())
                    .map_err(|_| nir::LayoutError::Unsupported(ty.clone()))?;
                return Ok(Some(nir::NominalLayout::EnumFixedBits {
                    width: bits,
                    niche: encoding.layout.contiguous_niche(),
                    exhaustive: encoding.layout.is_exhaustive(),
                }));
            }
            if def.variants.iter().all(|v| v.fields.is_empty()) {
                return Ok(Some(nir::NominalLayout::EnumUnit(def.variants.len())));
            }
            let mut variants = Vec::with_capacity(def.variants.len());
            for variant in &def.variants {
                let fields = nir::normal_forms::instantiate_value_fields_checked(
                    &def.type_parameters,
                    &def.parameter_types,
                    arguments,
                    variant.fields.iter(),
                    fuel,
                )
                .map_err(admission_err)?;
                variants.push(fields);
            }
            return Ok(Some(nir::NominalLayout::EnumAggregate(variants)));
        }
        Ok(None)
    }

    fn nominal_field_index(
        &self,
        ty: &Term,
        field: &str,
        fuel: &mut usize,
    ) -> Result<nir::FieldLookup, nir::LayoutError> {
        let Some(next) = fuel.checked_sub(1) else {
            *fuel = 0;
            return Err(nir::LayoutError::BudgetExhausted);
        };
        *fuel = next;
        let Term::Nominal { declaration, .. } = ty else {
            return Ok(nir::FieldLookup::NotFound);
        };
        if self.0.graph.is_some() {
            return Ok(match self.0.concrete_nominal(ty) {
                Some(ConcreteNominal::Struct(def)) => def
                    .fields
                    .iter()
                    .position(|(name, _)| name == field)
                    .map_or(nir::FieldLookup::NotFound, nir::FieldLookup::Index),
                _ => nir::FieldLookup::NotFound,
            });
        }
        if let Some(def) = self.0.structs.get(declaration) {
            return Ok(def
                .fields
                .iter()
                .position(|(name, _)| name == field)
                .map_or(nir::FieldLookup::NotFound, nir::FieldLookup::Index));
        }
        Ok(nir::FieldLookup::NotFound)
    }
}
