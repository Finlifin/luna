use super::*;
use nir::program::VariantShape;

#[derive(Clone, Debug, PartialEq)]
pub struct ReflectedVariant {
    pub name: String,
    pub shape: VariantShape,
    pub fields: Vec<Term>,
}

/// Ordered structure and explicitly declared Symbol encoding. Groups do not
/// occupy variant indices; no encoding or layout is inferred from order.
#[derive(Clone, Debug, PartialEq)]
pub struct EnumReflection {
    pub subject: Term,
    pub is_intrinsic: bool,
    pub variants: Vec<ReflectedVariant>,
    pub groups: Vec<String>,
    pub symbol_values: Option<Vec<String>>,
}

impl fmt::Display for ReflectedVariant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(variant (name {}) (shape ",
            nir::QuotedDisplay(&self.name)
        )?;
        match &self.shape {
            VariantShape::Unit => f.write_str("unit")?,
            VariantShape::Tuple => f.write_str("tuple")?,
            VariantShape::Named(names) => {
                f.write_str("(named")?;
                for name in names {
                    write!(f, " {}", nir::QuotedDisplay(name))?;
                }
                f.write_str(")")?;
            }
        }
        f.write_str(") (fields")?;
        for ty in &self.fields {
            write!(f, " {ty}")?;
        }
        f.write_str("))")
    }
}
impl fmt::Display for EnumReflection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(enum-reflection (subject {}) (intrinsic {}) (groups",
            self.subject, self.is_intrinsic
        )?;
        for name in &self.groups {
            write!(f, " {}", nir::QuotedDisplay(name))?;
        }
        f.write_str(") (variants")?;
        for variant in &self.variants {
            write!(f, " {variant}")?;
        }
        f.write_str(")")?;
        if let Some(values) = &self.symbol_values {
            f.write_str(" (symbol-values")?;
            for value in values {
                write!(f, " {}", nir::QuotedDisplay(value))?;
            }
            f.write_str(")")?;
        }
        f.write_str(")")
    }
}

impl Interpreter<'_> {
    /// Observe instantiated structure and declared Symbol codes without selecting
    /// a value's variant. The original subject identity is retained.
    pub fn reflect_enum(&mut self, ty: &Term) -> Result<EnumReflection, Error> {
        if !self.comptime_only {
            return Err(self.failure(ErrorKind::PhaseMismatch, None));
        }
        let subject = ty;
        let selected = self.reflection_type(ty)?;
        let ty = &selected;
        let Term::Nominal {
            declaration,
            arguments,
        } = ty
        else {
            return Err(self.failure(ErrorKind::Type, None));
        };
        let definition = if let Some(graph) = self.graph {
            let mut selected = None;
            for nominal in &graph.nominals {
                let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
                let mut fuel = before;
                let result = nir::type_computations::charge_term(&nominal.ty, &mut fuel);
                self.budget.steps -= (before - fuel) as u64;
                result.map_err(|_| self.failure(ErrorKind::StepsExhausted, None))?;
                if nominal.ty == *ty {
                    if let iir::NominalDefinition::Enum(definition) = &nominal.definition {
                        selected = Some(generated_enums::EnumDefinition::Borrowed(definition));
                    }
                    break;
                }
            }
            selected
        } else {
            self.enums.get(declaration).cloned()
        }
        .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
        self.integer_operation(None, |budget| {
            budget.consume(
                definition
                    .variants
                    .len()
                    .saturating_add(definition.groups.len()),
            )
        })?;
        let mut groups = Vec::with_capacity(definition.groups.len());
        for name in &definition.groups {
            self.integer_operation(None, |budget| budget.consume(name.len().saturating_add(1)))?;
            groups.push(name.clone());
        }
        let mut variants = Vec::with_capacity(definition.variants.len());
        for variant in &definition.variants {
            self.integer_operation(None, |budget| {
                budget.consume(variant.name.len().saturating_add(1))
            })?;
            match &variant.shape {
                VariantShape::Unit if !variant.fields.is_empty() => {
                    return Err(self.failure(ErrorKind::Type, None));
                }
                VariantShape::Named(names) => {
                    if names.len() != variant.fields.len() {
                        return Err(self.failure(ErrorKind::Type, None));
                    }
                    for name in names {
                        self.integer_operation(None, |budget| {
                            budget.consume(name.len().saturating_add(1))
                        })?;
                    }
                }
                _ => {}
            }
            let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
            let mut fuel = before;
            // Resolve the concrete descriptor once, rather than scanning the
            // complete IIR nominal table again for each variant.
            let fields = if self.graph.is_some() {
                nir::normal_forms::instantiate_value_fields(
                    &[],
                    &[],
                    &[],
                    variant.fields.iter(),
                    &mut fuel,
                )
            } else {
                nir::normal_forms::instantiate_value_fields(
                    &definition.type_parameters,
                    &definition.parameter_types,
                    arguments,
                    variant.fields.iter(),
                    &mut fuel,
                )
            };
            self.budget.steps -= (before - fuel) as u64;
            let mut fields = fields.ok_or_else(|| {
                self.failure(
                    if fuel == 0 {
                        ErrorKind::StepsExhausted
                    } else {
                        ErrorKind::Type
                    },
                    None,
                )
            })?;
            for field in &mut fields {
                *field = self.reflection_type(field)?;
            }
            variants.push(ReflectedVariant {
                name: variant.name.clone(),
                shape: variant.shape.clone(),
                fields,
            });
        }
        let symbol_values = if let Some(encoding) = &definition.symbol_encoding {
            let mut values = Vec::new();
            for variant in encoding.variants() {
                self.integer_operation(None, |budget| {
                    budget.consume(variant.value.len().saturating_add(1))
                })?;
                values.push(variant.value.clone());
            }
            Some(values)
        } else {
            None
        };
        Ok(EnumReflection {
            symbol_values,
            subject: subject.clone(),
            is_intrinsic: definition.is_intrinsic,
            variants,
            groups,
        })
    }
}

#[cfg(test)]
mod tests;

impl Interpreter<'_> {
    pub(crate) fn observe_enum_structure(
        &mut self,
        value: Value,
        output: &Term,
        query: nir::program::EnumStructureQuery,
        source: SourceRange,
    ) -> Result<Value, Error> {
        use nir::program::EnumStructureQuery as Q;
        let fail = || Error {
            kind: ErrorKind::Type,
            source: Some(source),
        };
        let Value::Type(subject) = value else {
            return Err(fail());
        };
        let inner = self.list_element(output).ok_or_else(fail)?.clone();
        let reflected = self.reflect_enum(&subject).map_err(|mut error| {
            error.source = Some(source);
            error
        })?;
        let count = if query == Q::GroupPaths {
            reflected.groups.len()
        } else {
            reflected.variants.len()
        };
        self.integer_operation(Some(source), |budget| budget.consume(count))?;
        let symbols = |names: Vec<String>| Value::List {
            ty: inner.clone(),
            values: names
                .into_iter()
                .map(|name| Value::Symbol(name.into()))
                .collect(),
        };
        let mut values = Vec::with_capacity(count);
        match query {
            Q::GroupPaths | Q::VariantPaths => {
                let paths = if query == Q::GroupPaths {
                    reflected.groups
                } else {
                    reflected
                        .variants
                        .into_iter()
                        .map(|variant| variant.name)
                        .collect()
                };
                for path in paths {
                    self.integer_operation(Some(source), |budget| {
                        budget.consume(path.len().saturating_add(1))
                    })?;
                    values.push(symbols(path.split('.').map(str::to_owned).collect()));
                }
            }
            Q::SymbolValues => {
                values.extend(
                    reflected
                        .symbol_values
                        .ok_or_else(fail)?
                        .into_iter()
                        .map(|value| Value::Symbol(value.into())),
                );
            }
            Q::Shapes => {
                for variant in reflected.variants {
                    values.push(Value::Symbol(
                        match variant.shape {
                            VariantShape::Unit => "unit",
                            VariantShape::Tuple => "tuple",
                            VariantShape::Named(_) => "named",
                        }
                        .into(),
                    ));
                }
            }
            Q::FieldNames => {
                for variant in reflected.variants {
                    values.push(symbols(match variant.shape {
                        VariantShape::Named(names) => names,
                        _ => vec![],
                    }));
                }
            }
            Q::FieldTypes => {
                for variant in reflected.variants {
                    values.push(Value::List {
                        ty: inner.clone(),
                        values: variant.fields.into_iter().map(Value::Type).collect(),
                    });
                }
            }
        }
        let value = Value::List {
            ty: output.clone(),
            values: values.into(),
        };
        self.validate_immutable_input(&value, output)?;
        Ok(value)
    }
}
