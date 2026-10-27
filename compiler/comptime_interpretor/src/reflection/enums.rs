use super::*;
use nir::program::VariantShape;

#[derive(Clone, Debug, PartialEq)]
pub struct ReflectedVariant {
    pub name: String,
    pub shape: VariantShape,
    pub fields: Vec<Term>,
}

/// Ordered structural information only. Groups do not occupy variant indices;
/// no discriminant value, storage encoding or layout is inferred from order.
#[derive(Clone, Debug, PartialEq)]
pub struct EnumReflection {
    pub subject: Term,
    pub is_intrinsic: bool,
    pub variants: Vec<ReflectedVariant>,
    pub groups: Vec<String>,
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
        f.write_str("))")
    }
}

impl Interpreter<'_> {
    /// Observe instantiated enum structure without selecting a value's variant
    /// or requesting its encoding. The original subject identity is retained.
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
        Ok(EnumReflection {
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
    pub(crate) fn eval_enum_structure(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        use nir::program::EnumStructureQuery as Q;
        let K::EnumStructure { subject, query } = &expr.kind else {
            unreachable!()
        };
        let fail = || Error {
            kind: ErrorKind::Type,
            source: Some(expr.source),
        };
        if !self.comptime_only {
            return Err(self.failure(ErrorKind::PhaseMismatch, Some(expr.source)));
        }
        let never = Term::Primitive(Primitive::Never);
        let element = Term::Primitive(if *query == Q::FieldTypes {
            Primitive::Type
        } else {
            Primitive::Symbol
        });
        let result_element = self.list_element(&expr.ty);
        let expected_element = if *query == Q::Shapes {
            result_element
        } else {
            result_element.and_then(|ty| self.list_element(ty))
        };
        if (subject.ty != Term::Primitive(Primitive::Type) && subject.ty != never)
            || (subject.ty == never && expr.ty != never)
            || (subject.ty != never && expected_element != Some(&element))
        {
            return Err(fail());
        }
        let value = match self.eval(subject, environment)? {
            Flow::Value(value) => value,
            flow => return Ok(flow),
        };
        let Value::Type(subject) = value else {
            return Err(fail());
        };
        let inner = self.list_element(&expr.ty).ok_or_else(fail)?.clone();
        let reflected = self.reflect_enum(&subject).map_err(|mut error| {
            error.source = Some(expr.source);
            error
        })?;
        let count = if *query == Q::GroupPaths {
            reflected.groups.len()
        } else {
            reflected.variants.len()
        };
        self.integer_operation(Some(expr.source), |budget| budget.consume(count))?;
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
                let paths = if *query == Q::GroupPaths {
                    reflected.groups
                } else {
                    reflected
                        .variants
                        .into_iter()
                        .map(|variant| variant.name)
                        .collect()
                };
                for path in paths {
                    self.integer_operation(Some(expr.source), |budget| {
                        budget.consume(path.len().saturating_add(1))
                    })?;
                    values.push(symbols(path.split('.').map(str::to_owned).collect()));
                }
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
                    values.push(Value::TypeList(variant.fields));
                }
            }
        }
        let value = Value::List {
            ty: expr.ty.clone(),
            values,
        };
        self.validate_immutable_input(&value, &expr.ty)?;
        Ok(Flow::Value(value))
    }
}
