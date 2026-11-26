//! Structure observation for comptime library adapters, independent of layout.
use super::*;

mod newtypes;

pub use newtypes::NewtypeReflection;

mod enums;

pub use enums::{EnumReflection, ReflectedVariant};

mod callables;

pub use callables::CallableReflection;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReflectedField {
    pub name: String,
    pub ty: Term,
}

/// An observation of an applied struct, not a replacement for its type identity.
/// This carries no visibility, access, offset, Copy or layout permission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructReflection {
    pub subject: Term,
    pub fields: Vec<ReflectedField>,
}

/// An observation of an anonymous record Type, not a replacement for its type identity.
/// This carries no access, Copy or layout permission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordReflection {
    pub subject: Term,
    pub fields: Vec<ReflectedField>,
}

impl fmt::Display for ReflectedField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(field (name {}) (type {}))",
            nir::QuotedDisplay(&self.name),
            self.ty
        )
    }
}

impl fmt::Display for StructReflection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(struct-reflection (subject {}) (fields", self.subject)?;
        for field in &self.fields {
            write!(f, " {field}")?;
        }
        f.write_str("))")
    }
}

impl fmt::Display for RecordReflection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(record-reflection (subject {}) (fields", self.subject)?;
        for field in &self.fields {
            write!(f, " {field}")?;
        }
        f.write_str("))")
    }
}

impl Interpreter<'_> {
    fn reflected_field_list(
        &mut self,
        fields: Vec<ReflectedField>,
        output: &Term,
        names: bool,
    ) -> Result<Value, Error> {
        let value = if names {
            Value::List {
                ty: output.clone(),
                values: fields
                    .into_iter()
                    .map(|field| Value::Symbol(field.name.into()))
                    .collect(),
            }
        } else {
            Value::List {
                ty: output.clone(),
                values: fields
                    .into_iter()
                    .map(|field| Value::Type(field.ty))
                    .collect(),
            }
        };
        self.validate_immutable_input(&value, output)?;
        Ok(value)
    }

    pub(super) fn observe_struct_fields(
        &mut self,
        subject: Value,
        output: &Term,
        names: bool,
        source: SourceRange,
    ) -> Result<Value, Error> {
        let Value::Type(subject) = subject else {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        };
        let reflected = self.reflect_struct(&subject).map_err(|mut error| {
            if error.source.is_none() {
                error.source = Some(source);
            }
            error
        })?;
        self.reflected_field_list(reflected.fields, output, names)
    }

    pub(super) fn observe_record_fields(
        &mut self,
        subject: Value,
        output: &Term,
        names: bool,
        source: SourceRange,
    ) -> Result<Value, Error> {
        let Value::Type(subject) = subject else {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        };
        let reflected = self.reflect_record(&subject).map_err(|mut error| {
            if error.source.is_none() {
                error.source = Some(source);
            }
            error
        })?;
        self.reflected_field_list(reflected.fields, output, names)
    }

    pub(super) fn observe_callable(
        &mut self,
        subject: Value,
        source: SourceRange,
    ) -> Result<Value, Error> {
        let Value::Type(subject) = subject else {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        };
        let reflected = self.reflect_callable(&subject).map_err(|mut error| {
            if error.source.is_none() {
                error.source = Some(source);
            }
            error
        })?;
        let signature = reflected.signature().clone();
        let is_comptime = reflected.is_comptime();
        let is_unsafe = reflected.is_unsafe();
        let CallableReflection { subject, callable } = reflected;
        let value = Value::Object(Object::from_entries(vec![
            ObjectEntry::Property {
                key: "subject".into(),
                value: ObjectValue {
                    ty: Term::Primitive(Primitive::Type),
                    value: Value::Type(subject),
                },
            },
            ObjectEntry::Property {
                key: "callable".into(),
                value: ObjectValue {
                    ty: Term::Primitive(Primitive::Type),
                    value: Value::Type(callable),
                },
            },
            ObjectEntry::Property {
                key: "signature".into(),
                value: ObjectValue {
                    ty: Term::Primitive(Primitive::Type),
                    value: Value::Type(signature),
                },
            },
            ObjectEntry::Property {
                key: "is_comptime".into(),
                value: ObjectValue {
                    ty: Term::Primitive(Primitive::Bool),
                    value: Value::Bool(is_comptime),
                },
            },
            ObjectEntry::Property {
                key: "is_unsafe".into(),
                value: ObjectValue {
                    ty: Term::Primitive(Primitive::Bool),
                    value: Value::Bool(is_unsafe),
                },
            },
        ]));
        self.validate_immutable_input(&value, &Term::Primitive(Primitive::Object))?;
        Ok(value)
    }

    pub(super) fn observe_type_id(
        &mut self,
        subject: Value,
        source: SourceRange,
    ) -> Result<Value, Error> {
        if !self.comptime_only {
            return Err(self.failure(ErrorKind::PhaseMismatch, Some(source)));
        }
        let Value::Type(subject) = subject else {
            return Err(self.failure(ErrorKind::Type, Some(source)));
        };
        let ty = self.reflection_type(&subject).map_err(|mut error| {
            if error.source.is_none() {
                error.source = Some(source);
            }
            error
        })?;
        let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let result = nir::compute_type_id_with_world(&InterpreterTypeIdWorld(self), &ty, &mut fuel);
        self.budget.steps = self.budget.steps.saturating_sub((before - fuel) as u64);
        let id = result.map_err(|err| match err {
            nir::TypeIdError::BudgetExhausted => {
                self.failure(ErrorKind::StepsExhausted, Some(source))
            }
            nir::TypeIdError::PendingDefinitionAttributes(decl) => {
                self.failure(ErrorKind::PendingDefinitionAttributes(decl), Some(source))
            }
            nir::TypeIdError::MissingTarget
            | nir::TypeIdError::Unresolved(_)
            | nir::TypeIdError::OpenGeneric(_) => {
                self.failure(ErrorKind::TypeContractUnresolved, Some(source))
            }
            nir::TypeIdError::Layout(nir::LayoutError::BudgetExhausted) => {
                self.failure(ErrorKind::StepsExhausted, Some(source))
            }
            nir::TypeIdError::Layout(nir::LayoutError::Unresolved(_)) => {
                self.failure(ErrorKind::TypeContractUnresolved, Some(source))
            }
            nir::TypeIdError::Layout(nir::LayoutError::Overflow) => {
                self.failure(ErrorKind::MachineOverflow, Some(source))
            }
            nir::TypeIdError::InvalidStableSince { .. }
            | nir::TypeIdError::InvalidAbiMetadata { .. }
            | nir::TypeIdError::UnsupportedGlobalExpr(_)
            | nir::TypeIdError::NonGlobalRuntimeOrigin(_)
            | nir::TypeIdError::NotAType(_)
            | nir::TypeIdError::ComptimeOnly(_)
            | nir::TypeIdError::Layout(_)
            | nir::TypeIdError::Unsupported(_) => self.failure(
                ErrorKind::InvalidTypeContract(err.to_string()),
                Some(source),
            ),
        })?;
        Ok(Value::UnsignedInteger(id))
    }

    pub(super) fn reflection_type(&mut self, ty: &Term) -> Result<Term, Error> {
        let ty = self.selected_type_view(ty, None)?;
        self.check_type_description(&ty, None)?;
        self.admit_type(&ty, 0)?;
        Ok(ty)
    }

    /// Inspect an anonymous record Type in this invocation's declaration world.
    /// Formation predicates and nested field descriptions are checked under the
    /// invocation budget; opaque normal-form heads are not unfolded.
    pub fn reflect_record(&mut self, ty: &Term) -> Result<RecordReflection, Error> {
        if !self.comptime_only {
            return Err(self.failure(ErrorKind::PhaseMismatch, None));
        }
        let subject = ty;
        let selected = self.reflection_type(ty)?;
        let Term::Record(raw_fields) = &selected else {
            return Err(self.failure(ErrorKind::Type, None));
        };
        self.integer_operation(None, |budget| budget.consume(raw_fields.len()))?;
        let mut fields = Vec::with_capacity(raw_fields.len());
        for (name, field_ty) in raw_fields {
            self.integer_operation(None, |budget| budget.consume(name.len().saturating_add(1)))?;
            let ty = self.reflection_type(field_ty)?;
            fields.push(ReflectedField {
                name: name.clone(),
                ty,
            });
        }
        Ok(RecordReflection {
            subject: subject.clone(),
            fields,
        })
    }

    /// Inspect a completed struct application in this invocation's declaration
    /// world. Formation predicates are checked even for phantom parameters and
    /// empty structs. All substitution/copy work consumes the invocation budget.
    /// No user code executes and no normal-form head is unfolded.
    pub fn reflect_struct(&mut self, ty: &Term) -> Result<StructReflection, Error> {
        if !self.comptime_only {
            return Err(self.failure(ErrorKind::PhaseMismatch, None));
        }
        let subject = ty;
        let selected = self.reflection_type(ty)?;
        let ty = &selected;
        let Term::Nominal { declaration, .. } = ty else {
            return Err(self.failure(ErrorKind::Type, None));
        };
        let definition = if self.graph.is_some() {
            match self.concrete_nominal_metered(ty)? {
                Some(ConcreteNominal::Struct(definition)) => Some(definition),
                _ => None,
            }
        } else {
            self.structs.get(declaration).cloned()
        }
        .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
        self.integer_operation(None, |budget| budget.consume(definition.fields.len()))?;
        // The same parameter telescope and bounded substitution used by value
        // admission, without performing any size/alignment query.
        let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let types = self.admitted_nominal_layout(ty, None, &mut fuel);
        self.budget.steps -= (before - fuel) as u64;
        let types = types.map_err(|error| {
            self.failure(
                match error {
                    nir::NominalAdmissionError::Budget => ErrorKind::StepsExhausted,
                    nir::NominalAdmissionError::Unresolved(_) => ErrorKind::TypeContractUnresolved,
                    nir::NominalAdmissionError::Invalid(_) => ErrorKind::Type,
                },
                None,
            )
        })?;
        if types.len() != definition.fields.len() {
            return Err(self.failure(ErrorKind::Type, None));
        }
        let mut fields = Vec::with_capacity(types.len());
        for ((name, _), ty) in definition.fields.iter().zip(types) {
            self.integer_operation(None, |budget| budget.consume(name.len().saturating_add(1)))?;
            let ty = self.reflection_type(&ty)?;
            fields.push(ReflectedField {
                name: name.clone(),
                ty,
            });
        }
        Ok(StructReflection {
            subject: subject.clone(),
            fields,
        })
    }
}

struct InterpreterTypeIdWorld<'a, 'b>(&'a Interpreter<'b>);

impl nir::LayoutWorld for InterpreterTypeIdWorld<'_, '_> {
    fn target(&self) -> Option<nir::target::TargetData> {
        super::observations::InterpreterLayoutWorld(self.0).target()
    }

    fn is_tuple_head(&self, head: DeclarationId) -> bool {
        super::observations::InterpreterLayoutWorld(self.0).is_tuple_head(head)
    }

    fn is_list_head(&self, head: DeclarationId) -> bool {
        super::observations::InterpreterLayoutWorld(self.0).is_list_head(head)
    }

    fn nominal_kind(&self, ty: &Term) -> Option<nir::NominalKind> {
        super::observations::InterpreterLayoutWorld(self.0).nominal_kind(ty)
    }

    fn is_template_identity(&self, ty: &Term) -> bool {
        super::observations::InterpreterLayoutWorld(self.0).is_template_identity(ty)
    }

    fn nominal_layout(
        &self,
        ty: &Term,
        fuel: &mut usize,
    ) -> Result<Option<nir::NominalLayout>, nir::LayoutError> {
        super::observations::InterpreterLayoutWorld(self.0).nominal_layout(ty, fuel)
    }

    fn nominal_field_index(
        &self,
        ty: &Term,
        field: &str,
        fuel: &mut usize,
    ) -> Result<nir::FieldLookup, nir::LayoutError> {
        super::observations::InterpreterLayoutWorld(self.0).nominal_field_index(ty, field, fuel)
    }
}

impl nir::TypeIdWorld for InterpreterTypeIdWorld<'_, '_> {
    fn package_metadata(&self, package: u64) -> Option<&nir::program::PackageMetadata> {
        self.0
            .declaration_world
            .and_then(|world| world.package_metadata.get(&package))
    }

    fn declaration_path(
        &self,
        declaration: DeclarationId,
    ) -> Option<&nir::program::CanonicalDeclarationPath> {
        self.0
            .declaration_world
            .and_then(|world| world.declaration_paths.get(&declaration))
    }

    fn definition_attributes(&self) -> Option<&nir::program::DefinitionAttributes> {
        self.0
            .declaration_world
            .map(|world| &world.definition_attributes)
    }

    fn global_runtime_origin(&self, id: nir::RuntimeId) -> Option<&nir::program::Expr> {
        self.0
            .declaration_world
            .and_then(|world| world.global_runtime_deliveries.get(&id))
    }

    fn nominal_name_fallback(&self, declaration: DeclarationId) -> Option<&str> {
        if let Some(def) = self.0.structs.get(&declaration) {
            return Some(def.name.as_str());
        }
        if let Some(def) = self.0.enums.get(&declaration) {
            return Some(def.name.as_str());
        }
        if let Some(def) = self.0.newtypes.get(&declaration) {
            return Some(def.name.as_str());
        }
        None
    }

    fn normal_form_name_fallback(&self, _head: DeclarationId) -> Option<&str> {
        None
    }

    fn concrete_struct_info(
        &self,
        ty: &Term,
        fuel: &mut usize,
    ) -> Result<Option<nir::ConcreteStructInfo>, nir::LayoutError> {
        let Term::Nominal {
            declaration,
            arguments,
        } = ty
        else {
            return Ok(None);
        };
        if self.0.graph.is_some() {
            return Ok(match self.0.concrete_nominal(ty) {
                Some(ConcreteNominal::Struct(def)) => Some(nir::ConcreteStructInfo {
                    packed: def.packed,
                    fields: def.fields.clone(),
                }),
                _ => None,
            });
        }
        let Some(def) = self.0.structs.get(declaration) else {
            return Ok(None);
        };
        let field_types = nir::normal_forms::instantiate_value_fields_checked(
            &def.type_parameters,
            &def.parameter_types,
            arguments,
            def.fields.iter().map(|(_, t)| t),
            fuel,
        )
        .map_err(|err| match err {
            nir::NominalAdmissionError::Budget => nir::LayoutError::BudgetExhausted,
            nir::NominalAdmissionError::Unresolved(_) => nir::LayoutError::Unresolved(ty.clone()),
            nir::NominalAdmissionError::Invalid(_) => nir::LayoutError::Unsupported(ty.clone()),
        })?;
        Ok(Some(nir::ConcreteStructInfo {
            packed: def.packed,
            fields: def
                .fields
                .iter()
                .map(|(name, _)| name.clone())
                .zip(field_types)
                .collect(),
        }))
    }

    fn concrete_enum_info(
        &self,
        ty: &Term,
        fuel: &mut usize,
    ) -> Result<Option<nir::ConcreteEnumInfo>, nir::LayoutError> {
        let Term::Nominal {
            declaration,
            arguments,
        } = ty
        else {
            return Ok(None);
        };
        if self.0.graph.is_some() {
            return Ok(match self.0.concrete_nominal(ty) {
                Some(ConcreteNominal::Enum(def)) => Some(nir::ConcreteEnumInfo {
                    variants: def
                        .variants
                        .iter()
                        .map(|v| nir::ConcreteVariantInfo {
                            name: v.name.clone(),
                            shape: v.shape.clone(),
                            fields: v.fields.clone(),
                        })
                        .collect(),
                    encoding: def.encoding.clone(),
                    symbol_encoding: def.symbol_encoding.clone(),
                }),
                _ => None,
            });
        }
        let Some(def) = self.0.enums.get(declaration) else {
            return Ok(None);
        };
        let mut variants = Vec::with_capacity(def.variants.len());
        for variant in &def.variants {
            let fields = nir::normal_forms::instantiate_value_fields_checked(
                &def.type_parameters,
                &def.parameter_types,
                arguments,
                variant.fields.iter(),
                fuel,
            )
            .map_err(|err| match err {
                nir::NominalAdmissionError::Budget => nir::LayoutError::BudgetExhausted,
                nir::NominalAdmissionError::Unresolved(_) => {
                    nir::LayoutError::Unresolved(ty.clone())
                }
                nir::NominalAdmissionError::Invalid(_) => nir::LayoutError::Unsupported(ty.clone()),
            })?;
            variants.push(nir::ConcreteVariantInfo {
                name: variant.name.clone(),
                shape: variant.shape.clone(),
                fields,
            });
        }
        Ok(Some(nir::ConcreteEnumInfo {
            variants,
            encoding: def.encoding.clone(),
            symbol_encoding: def.symbol_encoding.clone(),
        }))
    }
}

#[cfg(test)]
mod tests;
