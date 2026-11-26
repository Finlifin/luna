//! Comptime reflection span queries, field-type/value access, and nominal construction.
use super::*;

impl Interpreter<'_> {
    fn encode_source_span_value(
        &self,
        span_ty: &Term,
        range: nir::program::SourceRange,
    ) -> Result<Value, Error> {
        let Term::Nominal {
            declaration,
            arguments,
        } = span_ty
        else {
            return Err(self.failure(ErrorKind::Type, Some(range)));
        };
        if !arguments.is_empty() {
            return Err(self.failure(ErrorKind::Type, Some(range)));
        }
        let resolved = self
            .declaration_world
            .and_then(|world| range.original(&world.expansion_sources))
            .unwrap_or(range);
        let expansion_id = range
            .expansion_index()
            .and_then(|idx| isize::try_from(idx).ok())
            .unwrap_or(0);
        let file: Arc<str> = self
            .implicit_call_site
            .as_ref()
            .map(|site| site.file.clone())
            .unwrap_or_else(|| Arc::from(""));
        Ok(Value::Record {
            arguments: Vec::new(),
            declaration: *declaration,
            fields: vec![
                Value::String(file),
                Value::UnsignedInteger(u128::from(resolved.start)),
                Value::UnsignedInteger(u128::from(resolved.end)),
                Value::SignedInteger(expansion_id as i128),
            ],
        })
    }

    fn span_elem_type<'t>(&self, list_ty: &'t Term) -> Option<&'t Term> {
        self.list_element(list_ty)
    }

    pub(super) fn check_field_reflection_observation(
        &self,
        kind: comptime_compiler::observations::Kind,
        input: &Term,
        output: &Term,
        source: SourceRange,
    ) -> Result<(), Error> {
        use comptime_compiler::observations::Kind as O;
        if !self.comptime_only {
            return Err(self.failure(ErrorKind::PhaseMismatch, Some(source)));
        }
        let type_err = || Error {
            kind: ErrorKind::Type,
            source: Some(source),
        };
        match kind {
            O::StructFieldSpans | O::RecordFieldSpans | O::EnumVariantSpans => {
                if *input != Term::Primitive(Primitive::Type)
                    || self.list_element(output).is_none()
                    || !matches!(
                        self.span_elem_type(output),
                        Some(Term::Nominal { arguments, .. }) if arguments.is_empty()
                    )
                {
                    return Err(type_err());
                }
            }
            O::EnumVariantFieldSpans => {
                if *input != Term::Primitive(Primitive::Type) {
                    return Err(type_err());
                }
                let Some(inner_list) = self.list_element(output) else {
                    return Err(type_err());
                };
                if self.list_element(inner_list).is_none()
                    || !matches!(
                        self.span_elem_type(inner_list),
                        Some(Term::Nominal { arguments, .. }) if arguments.is_empty()
                    )
                {
                    return Err(type_err());
                }
            }
            O::DeclarationSpan => {
                if *input != Term::Primitive(Primitive::Type)
                    || !matches!(
                        output,
                        Term::Nominal { arguments, .. } if arguments.is_empty()
                    )
                {
                    return Err(type_err());
                }
            }
            O::ComptimeEnumVariantIndex => {
                if *output != Term::Primitive(Primitive::Integer) {
                    return Err(type_err());
                }
            }
            O::ComptimeEnumVariantFieldValues => {
                if *output != Term::Primitive(Primitive::Object) {
                    return Err(type_err());
                }
            }
            _ => return Err(type_err()),
        }
        Ok(())
    }

    pub(super) fn observe_field_reflection_query(
        &mut self,
        kind: comptime_compiler::observations::Kind,
        value: Value,
        output: &Term,
        source: Option<SourceRange>,
    ) -> Result<Value, Error> {
        use comptime_compiler::observations::Kind as O;
        let site = source.unwrap_or(SourceRange { start: 0, end: 0 });
        let world = self
            .declaration_world
            .ok_or_else(|| self.failure(ErrorKind::Type, source))?;
        match kind {
            O::StructFieldSpans => {
                let Value::Type(ty) = value else {
                    return Err(self.failure(ErrorKind::Type, source));
                };
                let Term::Nominal { declaration, .. } = &ty else {
                    return Err(self.failure(ErrorKind::Type, source));
                };
                let field_count = world
                    .structs
                    .iter()
                    .find(|s| s.declaration == *declaration)
                    .map(|s| s.fields.len())
                    .ok_or_else(|| self.failure(ErrorKind::Type, source))?;
                let span_ty = self
                    .span_elem_type(output)
                    .ok_or_else(|| self.failure(ErrorKind::Type, source))?;
                let spans = world.reflection_spans.struct_fields.get(declaration);
                let mut items = Vec::with_capacity(field_count);
                for i in 0..field_count {
                    let r = spans
                        .and_then(|list| list.get(i).copied())
                        .unwrap_or(SourceRange { start: 0, end: 0 });
                    items.push(self.encode_source_span_value(span_ty, r)?);
                }
                Ok(Value::List {
                    ty: output.clone(),
                    values: ListValues::from(items),
                })
            }
            O::RecordFieldSpans => {
                let Value::Type(ty) = value else {
                    return Err(self.failure(ErrorKind::Type, source));
                };
                let Term::Record(fields) = &ty else {
                    return Err(self.failure(ErrorKind::Type, source));
                };
                let span_ty = self
                    .span_elem_type(output)
                    .ok_or_else(|| self.failure(ErrorKind::Type, source))?;
                let key: Vec<String> = fields.iter().map(|(n, _)| n.clone()).collect();
                let spans = world.reflection_spans.record_fields.get(&key);
                let mut items = Vec::with_capacity(fields.len());
                for i in 0..fields.len() {
                    let r = spans
                        .and_then(|list| list.get(i).copied())
                        .unwrap_or(SourceRange { start: 0, end: 0 });
                    items.push(self.encode_source_span_value(span_ty, r)?);
                }
                Ok(Value::List {
                    ty: output.clone(),
                    values: ListValues::from(items),
                })
            }
            O::DeclarationSpan => {
                let Value::Type(ty) = value else {
                    return Err(self.failure(ErrorKind::Type, source));
                };
                let range = match &ty {
                    Term::Nominal { declaration, .. } => world
                        .reflection_spans
                        .structs
                        .get(declaration)
                        .or_else(|| world.reflection_spans.enums.get(declaration))
                        .copied()
                        .unwrap_or(SourceRange { start: 0, end: 0 }),
                    _ => SourceRange { start: 0, end: 0 },
                };
                self.encode_source_span_value(output, range)
            }
            O::EnumVariantSpans => {
                let Value::Type(ty) = value else {
                    return Err(self.failure(ErrorKind::Type, source));
                };
                let Term::Nominal { declaration, .. } = &ty else {
                    return Err(self.failure(ErrorKind::Type, source));
                };
                let variant_count = world
                    .enums
                    .iter()
                    .find(|e| e.declaration == *declaration)
                    .map(|e| e.variants.len())
                    .ok_or_else(|| self.failure(ErrorKind::Type, source))?;
                let span_ty = self
                    .span_elem_type(output)
                    .ok_or_else(|| self.failure(ErrorKind::Type, source))?;
                let spans = world.reflection_spans.enum_variants.get(declaration);
                let mut items = Vec::with_capacity(variant_count);
                for i in 0..variant_count {
                    let r = spans
                        .and_then(|list| list.get(i).copied())
                        .unwrap_or(SourceRange { start: 0, end: 0 });
                    items.push(self.encode_source_span_value(span_ty, r)?);
                }
                Ok(Value::List {
                    ty: output.clone(),
                    values: ListValues::from(items),
                })
            }
            O::EnumVariantFieldSpans => {
                let Value::Type(ty) = value else {
                    return Err(self.failure(ErrorKind::Type, source));
                };
                let Term::Nominal { declaration, .. } = &ty else {
                    return Err(self.failure(ErrorKind::Type, source));
                };
                let enum_def = world
                    .enums
                    .iter()
                    .find(|e| e.declaration == *declaration)
                    .ok_or_else(|| self.failure(ErrorKind::Type, source))?;
                let inner_list_ty = self
                    .span_elem_type(output)
                    .ok_or_else(|| self.failure(ErrorKind::Type, source))?;
                let span_ty = self
                    .span_elem_type(inner_list_ty)
                    .ok_or_else(|| self.failure(ErrorKind::Type, source))?;
                let spans = world.reflection_spans.enum_variant_fields.get(declaration);
                let mut outer = Vec::with_capacity(enum_def.variants.len());
                for (vi, variant) in enum_def.variants.iter().enumerate() {
                    let mut inner = Vec::with_capacity(variant.fields.len());
                    for fi in 0..variant.fields.len() {
                        let r = spans
                            .and_then(|v| v.get(vi))
                            .and_then(|f| f.get(fi).copied())
                            .unwrap_or(SourceRange { start: 0, end: 0 });
                        inner.push(self.encode_source_span_value(span_ty, r)?);
                    }
                    outer.push(Value::List {
                        ty: inner_list_ty.clone(),
                        values: ListValues::from(inner),
                    });
                }
                Ok(Value::List {
                    ty: output.clone(),
                    values: ListValues::from(outer),
                })
            }
            O::ComptimeEnumVariantIndex => {
                let Value::Variant { index, .. } = value else {
                    return Err(self.failure(ErrorKind::Type, Some(site)));
                };
                Ok(Value::Integer(comptime_runtime::Integer::from_u128(
                    index as u128,
                )))
            }
            O::ComptimeEnumVariantFieldValues => {
                let Value::Variant {
                    declaration,
                    arguments,
                    index,
                    fields,
                } = value
                else {
                    return Err(self.failure(ErrorKind::Type, Some(site)));
                };
                let enum_def = world
                    .enums
                    .iter()
                    .find(|e| e.declaration == declaration)
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(site)))?;
                let variant = enum_def
                    .variants
                    .get(index)
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(site)))?;
                let instantiated_types = if enum_def.type_parameters.is_empty() {
                    variant.fields.clone()
                } else {
                    let mut fuel = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
                    let binders = enum_def
                        .parameter_binders()
                        .map_err(|_| self.failure(ErrorKind::Type, Some(site)))?;
                    let types: Vec<Term> = variant
                        .fields
                        .iter()
                        .map(|ft| {
                            fuel = fuel.checked_sub(1).unwrap_or(0);
                            ft.instantiate_parameters(&binders, &arguments)
                        })
                        .collect::<Result<_, _>>()
                        .map_err(|_| self.failure(ErrorKind::Type, Some(site)))?;
                    types
                };
                let names: Option<&Vec<String>> = match &variant.shape {
                    nir::program::VariantShape::Named(names) => Some(names),
                    _ => None,
                };
                let mut entries = Vec::with_capacity(fields.len());
                for (i, (val, ty)) in fields.into_iter().zip(instantiated_types).enumerate() {
                    let obj_val = ObjectValue { ty, value: val };
                    let entry = match names.and_then(|list| list.get(i)) {
                        Some(name) => ObjectEntry::Property {
                            key: name.clone(),
                            value: obj_val,
                        },
                        None => ObjectEntry::Value(obj_val),
                    };
                    entries.push(entry);
                }
                Ok(Value::Object(Object::from_entries(entries)))
            }
            _ => Err(self.failure(ErrorKind::Type, source)),
        }
    }

    pub(super) fn resolve_field_in_type(
        &self,
        ty: &Term,
        field_name: &str,
        source: SourceRange,
    ) -> Result<Option<(usize, Term)>, Error> {
        match ty {
            Term::Record(fields) => Ok(fields
                .iter()
                .enumerate()
                .find(|(_, (n, _))| n == field_name)
                .map(|(i, (_, t))| (i, t.clone()))),
            Term::Nominal {
                declaration,
                arguments,
            } => {
                let Some(world) = self.declaration_world else {
                    return Ok(None);
                };
                let Some(struct_def) = world.structs.iter().find(|s| s.declaration == *declaration)
                else {
                    return Ok(None);
                };
                let Some((i, (_, raw_ty))) = struct_def
                    .fields
                    .iter()
                    .enumerate()
                    .find(|(_, (n, _))| n == field_name)
                else {
                    return Ok(None);
                };
                if struct_def.type_parameters.is_empty() {
                    return Ok(Some((i, raw_ty.clone())));
                }
                let binders = struct_def
                    .parameter_binders()
                    .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?;
                let instantiated = raw_ty
                    .instantiate_parameters(&binders, arguments)
                    .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?;
                Ok(Some((i, instantiated)))
            }
            _ => Ok(None),
        }
    }

    pub(super) fn complete_field_operation(
        &mut self,
        op: &comptime_compiler::observations::FieldOp,
        operands: Vec<Value>,
        output_ty: &Term,
        source: SourceRange,
    ) -> Result<Value, Error> {
        use comptime_compiler::observations::FieldOp as F;
        if !self.comptime_only {
            return Err(self.failure(ErrorKind::PhaseMismatch, Some(source)));
        }
        match op {
            F::TypeHasField => {
                let [subject, field] = operands.as_slice() else {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                };
                let Value::Type(ty) = subject else {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                };
                let Value::Symbol(name) = field else {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                };
                let found = self
                    .resolve_field_in_type(ty, name.as_ref(), source)?
                    .is_some();
                Ok(Value::Bool(found))
            }
            F::TypeFieldType => {
                let [subject, field] = operands.as_slice() else {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                };
                let Value::Type(ty) = subject else {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                };
                let Value::Symbol(name) = field else {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                };
                Ok(
                    match self.resolve_field_in_type(ty, name.as_ref(), source)? {
                        Some((_, ft)) => Value::Optional {
                            ty: Term::Primitive(Primitive::Type),
                            value: Some(Box::new(Value::Type(ft))),
                        },
                        None => Value::Optional {
                            ty: Term::Primitive(Primitive::Type),
                            value: None,
                        },
                    },
                )
            }
            F::StructFieldType => {
                let [subject, field] = operands.as_slice() else {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                };
                let Value::Type(ty) = subject else {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                };
                let Value::Symbol(name) = field else {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                };
                let Some((_, ft)) = self.resolve_field_in_type(ty, name.as_ref(), source)? else {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                };
                Ok(Value::Type(ft))
            }
            F::FieldValue => {
                let mut iter = operands.into_iter();
                let target = iter
                    .next()
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
                let field = iter
                    .next()
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
                let Value::Symbol(name) = field else {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                };
                self.extract_named_field(target, name.as_ref(), source)
            }
            F::ComptimeFieldEntry => {
                let mut iter = operands.into_iter();
                let target = iter
                    .next()
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
                let field = iter
                    .next()
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
                let ty_val = iter
                    .next()
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
                let Value::Symbol(name) = field else {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                };
                let Value::Type(field_ty) = ty_val else {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                };
                let extracted = self.extract_named_field(target, name.as_ref(), source)?;
                self.check_value_type(&extracted, &field_ty, Some(source))?;
                Ok(Value::Object(Object::from_entries(vec![
                    ObjectEntry::Property {
                        key: name.to_string(),
                        value: ObjectValue {
                            ty: field_ty,
                            value: extracted,
                        },
                    },
                ])))
            }
            F::NamedField(name) => {
                let mut iter = operands.into_iter();
                let target = iter
                    .next()
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
                self.extract_named_field(target, name.as_ref(), source)
            }
            F::TupleAppend => {
                let mut iter = operands.into_iter();
                let tup = iter
                    .next()
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
                let elem = iter
                    .next()
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
                let mut items = match tup {
                    Value::Unit => Vec::new(),
                    Value::Tuple(items) => items,
                    _ => return Err(self.failure(ErrorKind::Type, Some(source))),
                };
                items.push(elem);
                Ok(Value::Tuple(items))
            }
            F::ConstructStruct => {
                let mut iter = operands.into_iter();
                let arg = iter
                    .next()
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
                let Term::Nominal {
                    declaration,
                    arguments,
                } = output_ty
                else {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                };
                let world = self
                    .declaration_world
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
                let struct_def = world
                    .structs
                    .iter()
                    .find(|s| s.declaration == *declaration)
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
                let fields = self.unpack_struct_fields(struct_def, arguments, arg, source)?;
                Ok(Value::Record {
                    arguments: arguments.clone(),
                    declaration: *declaration,
                    fields,
                })
            }
            F::ConstructEnum => {
                let mut iter = operands.into_iter();
                let var_val = iter
                    .next()
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
                let arg = iter
                    .next()
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
                let var_idx = match var_val {
                    Value::UnsignedInteger(v) => usize::try_from(v)
                        .map_err(|_| self.failure(ErrorKind::IndexOutOfBounds, Some(source)))?,
                    Value::Integer(var_int) => var_int
                        .to_unsigned(64)
                        .map_err(|_| self.failure(ErrorKind::IndexOutOfBounds, Some(source)))?
                        as usize,
                    _ => return Err(self.failure(ErrorKind::Type, Some(source))),
                };
                let Term::Nominal {
                    declaration,
                    arguments,
                } = output_ty
                else {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                };
                let world = self
                    .declaration_world
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
                let enum_def = world
                    .enums
                    .iter()
                    .find(|e| e.declaration == *declaration)
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
                let variant = enum_def
                    .variants
                    .get(var_idx)
                    .ok_or_else(|| self.failure(ErrorKind::IndexOutOfBounds, Some(source)))?;
                let fields =
                    self.unpack_variant_fields(enum_def, variant, arguments, arg, source)?;
                Ok(Value::Variant {
                    arguments: arguments.clone(),
                    declaration: *declaration,
                    index: var_idx,
                    fields,
                })
            }
        }
    }

    fn extract_named_field(
        &self,
        target: Value,
        name: &str,
        source: SourceRange,
    ) -> Result<Value, Error> {
        match target {
            Value::AnonymousRecord(fields) => fields
                .into_iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v)
                .ok_or_else(|| self.failure(ErrorKind::Type, Some(source))),
            Value::Record {
                declaration,
                mut fields,
                ..
            } => {
                let world = self
                    .declaration_world
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
                let struct_def = world
                    .structs
                    .iter()
                    .find(|s| s.declaration == declaration)
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
                let idx = struct_def
                    .fields
                    .iter()
                    .position(|(n, _)| n == name)
                    .ok_or_else(|| self.failure(ErrorKind::Type, Some(source)))?;
                if idx >= fields.len() {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
                Ok(fields.swap_remove(idx))
            }
            _ => Err(self.failure(ErrorKind::Type, Some(source))),
        }
    }

    fn unpack_struct_fields(
        &mut self,
        struct_def: &nir::program::Struct,
        arguments: &[Term],
        arg: Value,
        source: SourceRange,
    ) -> Result<Vec<Value>, Error> {
        let expected_types: Vec<Term> = if struct_def.type_parameters.is_empty() {
            struct_def.fields.iter().map(|(_, t)| t.clone()).collect()
        } else {
            let binders = struct_def
                .parameter_binders()
                .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?;
            struct_def
                .fields
                .iter()
                .map(|(_, t)| t.instantiate_parameters(&binders, arguments))
                .collect::<Result<_, _>>()
                .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?
        };
        let names: Vec<&str> = struct_def.fields.iter().map(|(n, _)| n.as_str()).collect();
        self.unpack_fields_from_value(arg, &names, &expected_types, source)
    }

    fn unpack_variant_fields(
        &mut self,
        enum_def: &nir::program::Enum,
        variant: &nir::program::Variant,
        arguments: &[Term],
        arg: Value,
        source: SourceRange,
    ) -> Result<Vec<Value>, Error> {
        let expected_types: Vec<Term> = if enum_def.type_parameters.is_empty() {
            variant.fields.clone()
        } else {
            let binders = enum_def
                .parameter_binders()
                .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?;
            variant
                .fields
                .iter()
                .map(|t| t.instantiate_parameters(&binders, arguments))
                .collect::<Result<_, _>>()
                .map_err(|_| self.failure(ErrorKind::Type, Some(source)))?
        };
        match arg {
            Value::Unit if expected_types.is_empty() => Ok(Vec::new()),
            Value::Tuple(vals) => {
                if vals.len() != expected_types.len() {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
                for (val, ty) in vals.iter().zip(&expected_types) {
                    self.check_value_type(val, ty, Some(source))?;
                }
                Ok(vals)
            }
            Value::Object(obj) => {
                let entries = obj.into_entries();
                if entries.len() != expected_types.len() {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
                let mut out = Vec::with_capacity(entries.len());
                for (i, (entry, ty)) in entries.into_iter().zip(&expected_types).enumerate() {
                    if let nir::program::VariantShape::Named(names) = &variant.shape
                        && let Some(expected_name) = names.get(i)
                        && entry.key().map(|k| k.as_ref()) != Some(expected_name.as_str())
                    {
                        return Err(self.failure(ErrorKind::Type, Some(source)));
                    }
                    let val = entry.into_value().value;
                    self.check_value_type(&val, ty, Some(source))?;
                    out.push(val);
                }
                Ok(out)
            }
            _ => Err(self.failure(ErrorKind::Type, Some(source))),
        }
    }

    fn unpack_fields_from_value(
        &mut self,
        arg: Value,
        names: &[&str],
        expected_types: &[Term],
        source: SourceRange,
    ) -> Result<Vec<Value>, Error> {
        match arg {
            Value::Unit if expected_types.is_empty() => Ok(Vec::new()),
            Value::Tuple(vals) => {
                if vals.len() != expected_types.len() {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
                for (val, ty) in vals.iter().zip(expected_types) {
                    self.check_value_type(val, ty, Some(source))?;
                }
                Ok(vals)
            }
            Value::Object(obj) => {
                let entries = obj.into_entries();
                if entries.len() != expected_types.len() {
                    return Err(self.failure(ErrorKind::Type, Some(source)));
                }
                let mut out = Vec::with_capacity(entries.len());
                for ((entry, name), ty) in entries.into_iter().zip(names).zip(expected_types) {
                    if entry.key().map(|k| k.as_ref()) != Some(*name) {
                        return Err(self.failure(ErrorKind::Type, Some(source)));
                    }
                    let val = entry.into_value().value;
                    self.check_value_type(&val, ty, Some(source))?;
                    out.push(val);
                }
                Ok(out)
            }
            _ => Err(self.failure(ErrorKind::Type, Some(source))),
        }
    }

    pub(super) fn eval_field_operation_expr(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        use comptime_compiler::observations::FieldOp as F;
        let (op, operand_exprs): (F, Vec<&Expr>) = match &expr.kind {
            K::TypeHasField { subject, field } => (F::TypeHasField, vec![subject, field]),
            K::TypeFieldType { subject, field } => (F::TypeFieldType, vec![subject, field]),
            K::StructFieldType { subject, field } => (F::StructFieldType, vec![subject, field]),
            K::FieldValue { value, field } => (F::FieldValue, vec![value, field]),
            K::ComptimeFieldEntry { value, field, ty } => {
                (F::ComptimeFieldEntry, vec![value, field, ty])
            }
            K::NamedField { value, name } => (F::NamedField(Arc::from(name.as_str())), vec![value]),
            K::ConstructStruct { fields } => (F::ConstructStruct, vec![fields]),
            K::ConstructEnum { variant, fields } => (F::ConstructEnum, vec![variant, fields]),
            K::TupleAppend { tuple, element } => (F::TupleAppend, vec![tuple, element]),
            _ => return Err(self.failure(ErrorKind::Type, Some(expr.source))),
        };
        let mut values = Vec::with_capacity(operand_exprs.len());
        for operand in operand_exprs {
            match self.eval(operand, environment)? {
                Flow::Value(v) => values.push(v),
                flow => return Ok(flow),
            }
        }
        let result = self.complete_field_operation(&op, values, &expr.ty, expr.source)?;
        Ok(Flow::Value(result))
    }
}
