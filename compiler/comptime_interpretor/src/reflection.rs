//! Structure observation for comptime library adapters, independent of layout.
use super::*;
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

impl Interpreter<'_> {
    pub(super) fn eval_struct_fields(
        &mut self,
        expr: &Expr,
        environment: &mut Environment,
    ) -> Result<Flow, Error> {
        if !self.comptime_only {
            return Err(self.failure(ErrorKind::PhaseMismatch, Some(expr.source)));
        }
        let (subject, names) = match &expr.kind {
            K::StructFieldTypes(subject) => (subject, false),
            K::StructFieldNames(subject) => (subject, true),
            _ => unreachable!(),
        };
        let never = Term::Primitive(Primitive::Never);
        let element = Term::Primitive(if names {
            Primitive::Symbol
        } else {
            Primitive::Type
        });
        if (subject.ty != Term::Primitive(Primitive::Type) && subject.ty != never)
            || (subject.ty == never && expr.ty != never)
            || (subject.ty != never && self.list_element(&expr.ty) != Some(&element))
        {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        }
        let subject = match self.eval(subject, environment)? {
            Flow::Value(value) => value,
            flow => return Ok(flow),
        };
        let Value::Type(subject) = subject else {
            return Err(self.failure(ErrorKind::Type, Some(expr.source)));
        };
        let reflected = self.reflect_struct(&subject).map_err(|mut error| {
            if error.source.is_none() {
                error.source = Some(expr.source);
            }
            error
        })?;
        let value = if names {
            Value::List {
                ty: expr.ty.clone(),
                values: reflected
                    .fields
                    .into_iter()
                    .map(|field| Value::Symbol(field.name.into()))
                    .collect(),
            }
        } else {
            Value::TypeList(reflected.fields.into_iter().map(|field| field.ty).collect())
        };
        self.validate_immutable_input(&value, &expr.ty)?;
        Ok(Flow::Value(value))
    }

    fn reflection_type(&mut self, ty: &Term) -> Result<Term, Error> {
        let ty = self.selected_type_view(ty, None)?;
        let work = self
            .type_description_work(&ty)
            .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
        self.integer_operation(None, |budget| budget.consume(work))?;
        self.admit_type(&ty, 0)?;
        Ok(ty)
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
        let definition = if let Some(graph) = self.graph {
            let mut selected = None;
            for nominal in &graph.nominals {
                let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
                let mut fuel = before;
                let result = nir::type_computations::charge_term(&nominal.ty, &mut fuel);
                self.budget.steps -= (before - fuel) as u64;
                result.map_err(|_| self.failure(ErrorKind::StepsExhausted, None))?;
                if nominal.ty == *ty {
                    if let iir::NominalDefinition::Struct(definition) = &nominal.definition {
                        selected = Some(generated_structs::StructDefinition::Borrowed(definition));
                    }
                    break;
                }
            }
            selected
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

#[cfg(test)]
mod tests;
