//! Source construction requests dispatched to the ordinary checked publisher.
use super::*;

#[derive(Debug)]
pub struct StructBuildRequest {
    pub name: String,
    pub fields: Vec<ReflectedField>,
    pub source: SourceRange,
}
impl StructBuildRequest {
    /// Charge descriptor assembly before copying names or field type trees.
    pub fn charge_assembly(&self, fuel: &mut usize) -> Result<(), ()> {
        *fuel = fuel
            .checked_sub(
                self.name
                    .len()
                    .saturating_add(self.fields.len())
                    .saturating_add(1),
            )
            .ok_or(())?;
        for field in &self.fields {
            *fuel = fuel
                .checked_sub(field.name.len().saturating_add(1))
                .ok_or(())?;
            nir::type_computations::charge_term(&field.ty, fuel).map_err(|_| ())?;
        }
        Ok(())
    }
}
impl fmt::Display for StructBuildRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(struct-build (name {}) (fields",
            nir::QuotedDisplay(&self.name)
        )?;
        for field in &self.fields {
            write!(f, " {field}")?;
        }
        f.write_str("))")
    }
}

/// The host must reserve identities and perform ordinary semantic checking.
/// Commit must leave publication unchanged on error or unwind, and perform no
/// fallible work after making the candidate visible. Dropping it abandons the
/// candidate. VM admission is an additional check, not semantic elaboration.
pub struct PreparedStructBuild<'a> {
    pub definitions: Arc<[nir::program::Struct]>,
    pub result: DeclarationId,
    pub commit: Box<dyn FnOnce() -> Result<(), Error> + 'a>,
}

impl<'a> Interpreter<'a> {
    pub fn with_struct_builder(
        mut self,
        builder: &'a dyn Fn(
            &StructBuildRequest,
            &mut u64,
        ) -> Result<PreparedStructBuild<'a>, Error>,
    ) -> Self {
        self.struct_builder = Some(builder);
        self
    }

    pub(super) fn eval_struct_build(
        &mut self,
        expr: &Expr,
        request: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        let fail = || Error {
            kind: ErrorKind::Type,
            source: Some(expr.source),
        };
        if !self.comptime_only {
            return Err(self.failure(ErrorKind::PhaseMismatch, Some(expr.source)));
        }
        if expr.ty != Term::Primitive(Primitive::Type)
            && expr.ty != Term::Primitive(Primitive::Never)
        {
            return Err(fail());
        }
        let value = match self.eval(request, environment)? {
            Flow::Value(value) => value,
            flow => return Ok(flow),
        };
        if expr.ty != Term::Primitive(Primitive::Type) {
            return Err(fail());
        }
        let Value::Tuple(values) = value else {
            return Err(fail());
        };
        let mut values = values.into_iter();
        let (
            Some(Value::Symbol(name)),
            Some(Value::List {
                ty: names_ty,
                values: names,
            }),
            Some(Value::TypeList(types)),
            None,
        ) = (values.next(), values.next(), values.next(), values.next())
        else {
            return Err(fail());
        };
        if self.list_element(&names_ty) != Some(&Term::Primitive(Primitive::Symbol)) {
            return Err(fail());
        }
        if names.len() != types.len() {
            return Err(self.failure(
                ErrorKind::InvalidDefinition(DefinitionBuildError::FieldCount {
                    names: names.len(),
                    types: types.len(),
                }),
                Some(expr.source),
            ));
        }
        self.integer_operation(Some(expr.source), |budget| {
            budget.consume(name.len().saturating_add(names.len()))
        })?;
        let mut fields = Vec::with_capacity(names.len());
        let mut seen = BTreeSet::new();
        for (field_name, ty) in names.into_iter().zip(types) {
            let Value::Symbol(field_name) = field_name else {
                return Err(fail());
            };
            self.integer_operation(Some(expr.source), |budget| {
                budget.consume(field_name.len().saturating_add(1))
            })?;
            if !seen.insert(field_name.clone()) {
                return Err(self.failure(
                    ErrorKind::InvalidDefinition(DefinitionBuildError::DuplicateField {
                        owner: name.to_string(),
                        name: field_name.to_string(),
                    }),
                    Some(expr.source),
                ));
            }
            self.admit_type(&ty, 0)?;
            fields.push(ReflectedField {
                name: field_name.to_string(),
                ty,
            });
        }
        let request = StructBuildRequest {
            name: name.to_string(),
            fields,
            source: expr.source,
        };
        // Reserve the allowance before entering host code. Unwind cannot refund
        // unknown work, and a successful host cannot replenish the VM's budget.
        let builder = self.struct_builder.ok_or_else(fail)?;
        let before = self.budget.steps;
        self.budget.steps = 0;
        let mut remaining = before;
        let prepared = builder(&request, &mut remaining);
        self.budget.steps = before.min(remaining);
        let prepared = prepared?;
        self.integer_operation(Some(expr.source), |budget| {
            budget.consume(prepared.definitions.len())
        })?;
        let definition = prepared
            .definitions
            .iter()
            .find(|item| item.declaration == prepared.result)
            .ok_or_else(fail)?;
        self.integer_operation(Some(expr.source), |budget| {
            budget.consume(
                definition
                    .name
                    .len()
                    .saturating_add(definition.fields.len()),
            )
        })?;
        for (name, ty) in &definition.fields {
            self.integer_operation(Some(expr.source), |budget| budget.consume(name.len()))?;
            let before = self.budget.steps as usize;
            let mut fuel = before;
            let charged = nir::type_computations::charge_term(ty, &mut fuel);
            self.budget.steps -= (before - fuel) as u64;
            charged.map_err(|_| self.failure(ErrorKind::StepsExhausted, Some(expr.source)))?;
        }
        // A concrete field descriptor cannot authorize extra declaration inputs
        // or constraints merely because the host returned an admissible type.
        if definition.name != request.name
            || !definition.type_parameters.is_empty()
            || !definition.parameter_types.is_empty()
            || !definition.requirements.is_empty()
            || !definition.capabilities.is_empty()
            || definition.fields.len() != request.fields.len()
            || definition
                .fields
                .iter()
                .zip(&request.fields)
                .any(|((name, ty), field)| name != &field.name || ty != &field.ty)
        {
            return Err(fail());
        }
        let result = Term::Nominal {
            declaration: prepared.result,
            arguments: vec![],
        };
        self.install_owned_generated_structs_with(prepared.definitions, prepared.commit)
            .map_err(|error| match error {
                StructPublicationError::Admission(error)
                | StructPublicationError::Commit(error) => error,
            })?;
        Ok(Flow::Value(Value::Type(result)))
    }
}
