//! Structure observation for comptime library adapters, independent of layout.
use super::*;
mod enums;
pub use enums::{EnumReflection, ReflectedVariant};

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
    fn reflection_type(&mut self, ty: &Term) -> Result<Term, Error> {
        let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let selected = selected_types::view_budgeted(ty, &self.associated_values, &mut fuel);
        self.budget.steps -= (before - fuel) as u64;
        let ty = selected.map_err(|kind| self.failure(kind, None))?;
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
                        selected = Some(definition);
                    }
                    break;
                }
            }
            selected
        } else {
            self.structs.get(declaration).copied()
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
