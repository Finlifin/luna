//! Concrete enum construction requests for checked host publication.
use super::*;

/// Ordinary concrete variants preserve their source shape and payload order.
/// Generic and encoded enum construction require separate publication contracts.
#[derive(Debug)]
pub struct EnumBuildRequest {
    pub name: String,
    pub groups: Vec<String>,
    pub variants: Vec<nir::program::Variant>,
    pub source: SourceRange,
}
impl EnumBuildRequest {
    /// Charge a borrowed descriptor before copying it into another compiler world.
    pub fn charge_descriptor(
        name: &str,
        groups: &[String],
        variants: &[nir::program::Variant],
        fuel: &mut usize,
    ) -> Result<(), ()> {
        charge_enum_parts(name, groups, variants, fuel)
    }
    /// Charge descriptor assembly before cloning names or nested payload types.
    pub fn charge_assembly(&self, fuel: &mut usize) -> Result<(), ()> {
        charge_enum_parts(&self.name, &self.groups, &self.variants, fuel)
    }
}
fn charge_enum_parts(
    name: &str,
    groups: &[String],
    variants: &[nir::program::Variant],
    fuel: &mut usize,
) -> Result<(), ()> {
    let charge = |fuel: &mut usize, work: usize| {
        *fuel = fuel.checked_sub(work).ok_or(())?;
        Ok::<_, ()>(())
    };
    charge(fuel, name.len().saturating_add(1))?;
    for group in groups {
        charge(fuel, group.len().saturating_add(1))?;
    }
    for variant in variants {
        charge(fuel, variant.name.len().saturating_add(1))?;
        if let nir::program::VariantShape::Named(names) = &variant.shape {
            for name in names {
                charge(fuel, name.len().saturating_add(1))?;
            }
        }
        for field in &variant.fields {
            nir::type_computations::charge_term(field, fuel).map_err(|_| ())?;
        }
    }
    Ok(())
}

impl fmt::Display for EnumBuildRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(enum-build (name {}) (groups",
            nir::QuotedDisplay(&self.name)
        )?;
        for group in &self.groups {
            write!(f, " {}", nir::QuotedDisplay(group))?;
        }
        f.write_str(") (variants")?;
        for variant in &self.variants {
            write!(
                f,
                " (variant (name {}) (shape ",
                nir::QuotedDisplay(&variant.name)
            )?;
            match &variant.shape {
                nir::program::VariantShape::Unit => f.write_str("unit")?,
                nir::program::VariantShape::Tuple => f.write_str("tuple")?,
                nir::program::VariantShape::Named(names) => {
                    f.write_str("(named")?;
                    for name in names {
                        write!(f, " {}", nir::QuotedDisplay(name))?;
                    }
                    f.write_str(")")?;
                }
            }
            f.write_str(") (fields")?;
            for field in &variant.fields {
                write!(f, " {field}")?;
            }
            f.write_str("))")?;
        }
        f.write_str("))")
    }
}

/// The enclosing host owns identity reservation and checked publication. The
/// live VM must admit this exact batch before calling commit. Dropping abandons
/// the request; commit errors must leave the host publication unchanged.
pub struct PreparedEnumBuild<'a> {
    pub definitions: Arc<[nir::program::Enum]>,
    pub result: DeclarationId,
    pub commit: Box<dyn FnOnce() -> Result<(), Error> + 'a>,
}

impl<'a> Interpreter<'a> {
    pub fn with_enum_builder(
        mut self,
        builder: &'a dyn Fn(&EnumBuildRequest, &mut u64) -> Result<PreparedEnumBuild<'a>, Error>,
    ) -> Self {
        self.enum_builder = Some(builder);
        self
    }

    pub(super) fn eval_enum_build(
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
            Some(groups),
            Some(names),
            Some(shapes),
            Some(field_names),
            Some(field_types),
            None,
        ) = (
            values.next(),
            values.next(),
            values.next(),
            values.next(),
            values.next(),
            values.next(),
            values.next(),
        )
        else {
            return Err(fail());
        };
        let groups = self.enum_paths(groups, expr.source)?;
        let names = self.enum_paths(names, expr.source)?;
        let shapes = self.enum_symbols(shapes, expr.source)?;
        let nested = |value, element| -> Result<Vec<Value>, Error> {
            let Value::List { ty, values } = value else {
                return Err(fail());
            };
            if self.list_element(&ty).and_then(|ty| self.list_element(ty))
                != Some(&Term::Primitive(element))
            {
                return Err(fail());
            }
            Ok(values)
        };
        let field_names = nested(field_names, Primitive::Symbol)?;
        let field_types = nested(field_types, Primitive::Type)?;
        if names.len() != shapes.len()
            || names.len() != field_names.len()
            || names.len() != field_types.len()
        {
            return Err(fail());
        }
        self.integer_operation(Some(expr.source), |budget| {
            budget.consume(name.len().saturating_add(names.len()))
        })?;
        let mut variants = Vec::with_capacity(names.len());
        let mut seen = BTreeSet::new();
        for (((name, shape), field_names), field_types) in names
            .into_iter()
            .zip(shapes)
            .zip(field_names)
            .zip(field_types)
        {
            if !seen.insert(name.clone()) {
                return Err(self.failure(
                    ErrorKind::InvalidDefinition(DefinitionBuildError::DuplicateVariant { name }),
                    Some(expr.source),
                ));
            }
            let names = self.enum_symbols(field_names, expr.source)?;
            let Value::TypeList(fields) = field_types else {
                return Err(fail());
            };
            let shape = match shape.as_str() {
                "unit" if names.is_empty() && fields.is_empty() => nir::program::VariantShape::Unit,
                "tuple" if names.is_empty() => nir::program::VariantShape::Tuple,
                "named" if names.len() == fields.len() => {
                    let mut unique = BTreeSet::new();
                    for field in &names {
                        if !unique.insert(field) {
                            return Err(self.failure(
                                ErrorKind::InvalidDefinition(
                                    DefinitionBuildError::DuplicateField {
                                        owner: name,
                                        name: field.clone(),
                                    },
                                ),
                                Some(expr.source),
                            ));
                        }
                    }
                    nir::program::VariantShape::Named(names)
                }
                _ => {
                    return Err(self.failure(
                        ErrorKind::InvalidDefinition(DefinitionBuildError::VariantShape {
                            name,
                            shape,
                            names: names.len(),
                            types: fields.len(),
                        }),
                        Some(expr.source),
                    ));
                }
            };
            for field in &fields {
                self.admit_type(field, 0)?;
            }
            variants.push(nir::program::Variant {
                name,
                shape,
                fields,
            });
        }
        let request = EnumBuildRequest {
            name: name.to_string(),
            groups,
            variants,
            source: expr.source,
        };
        let builder = self.enum_builder.ok_or_else(fail)?;
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
            .find(|definition| definition.declaration == prepared.result)
            .ok_or_else(fail)?;
        // Prepay comparison of the returned descriptor before traversing trees.
        let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let charged = charge_enum_parts(
            &definition.name,
            &definition.groups,
            &definition.variants,
            &mut fuel,
        );
        self.budget.steps -= (before - fuel) as u64;
        charged.map_err(|_| self.failure(ErrorKind::StepsExhausted, Some(expr.source)))?;
        if definition.name != request.name
            || definition.groups != request.groups
            || definition.variants != request.variants
        {
            return Err(fail());
        }
        let result = Term::Nominal {
            declaration: prepared.result,
            arguments: vec![],
        };
        self.install_owned_generated_enums_with(prepared.definitions, prepared.commit)
            .map_err(|error| match error {
                EnumPublicationError::Admission(error) | EnumPublicationError::Commit(error) => {
                    error
                }
            })?;
        Ok(Flow::Value(Value::Type(result)))
    }

    fn enum_paths(&mut self, value: Value, source: SourceRange) -> Result<Vec<String>, Error> {
        let fail = || Error {
            kind: ErrorKind::Type,
            source: Some(source),
        };
        let Value::List { ty, values } = value else {
            return Err(fail());
        };
        if self.list_element(&ty) == Some(&Term::Primitive(Primitive::Symbol)) {
            return self.enum_symbols(Value::List { ty, values }, source);
        }
        if self
            .list_element(&ty)
            .and_then(|element| self.list_element(element))
            != Some(&Term::Primitive(Primitive::Symbol))
        {
            return Err(fail());
        }
        self.integer_operation(Some(source), |budget| budget.consume(values.len()))?;
        let mut paths = Vec::with_capacity(values.len());
        for value in values {
            let segments = self.enum_symbols(value, source)?;
            if segments.is_empty()
                || segments
                    .iter()
                    .any(|segment| segment.is_empty() || segment.contains('.'))
            {
                return Err(self.failure(
                    ErrorKind::InvalidDefinition(DefinitionBuildError::InvalidPath),
                    Some(source),
                ));
            }
            let work = segments.iter().fold(segments.len() - 1, |work, segment| {
                work.saturating_add(segment.len())
            });
            self.integer_operation(Some(source), |budget| budget.consume(work))?;
            paths.push(segments.join("."));
        }
        Ok(paths)
    }

    fn enum_symbols(&mut self, value: Value, source: SourceRange) -> Result<Vec<String>, Error> {
        let fail = || Error {
            kind: ErrorKind::Type,
            source: Some(source),
        };
        let Value::List { ty, values } = value else {
            return Err(fail());
        };
        if self.list_element(&ty) != Some(&Term::Primitive(Primitive::Symbol)) {
            return Err(fail());
        }
        self.integer_operation(Some(source), |budget| budget.consume(values.len()))?;
        values
            .into_iter()
            .map(|value| {
                let Value::Symbol(name) = value else {
                    return Err(fail());
                };
                self.integer_operation(Some(source), |budget| {
                    budget.consume(name.len().saturating_add(1))
                })?;
                Ok(name.to_string())
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
