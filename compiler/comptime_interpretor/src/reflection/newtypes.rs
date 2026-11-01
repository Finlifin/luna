//! Explicit observation of one newtype representation edge, never normalization.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewtypeReflection {
    pub subject: Term,
    pub representation: Term,
}
impl fmt::Display for NewtypeReflection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(newtype-reflection (subject {}) (representation {}))",
            self.subject, self.representation
        )
    }
}

impl Interpreter<'_> {
    /// Observe the direct representation of an admitted application. Pointer
    /// Types are metadata here, not pointer storage in the comptime universe.
    pub fn reflect_newtype(&mut self, ty: &Term) -> Result<NewtypeReflection, Error> {
        if !self.comptime_only {
            return Err(self.failure(ErrorKind::PhaseMismatch, None));
        }
        let selected = self.reflection_type(ty)?;
        let Term::Nominal {
            declaration,
            arguments,
        } = &selected
        else {
            return Err(self.failure(ErrorKind::Type, None));
        };
        let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let result = (|| {
            nir::type_computations::charge_term(ty, &mut fuel)
                .map_err(|_| nir::newtypes::ConversionError::Budget)?;
            if self.graph.is_some() {
                nir::type_computations::charge_term(&selected, &mut fuel)
                    .map_err(|_| nir::newtypes::ConversionError::Budget)?;
                let Some(iir::NominalDefinition::Newtype(definition)) =
                    self.concrete_nominal(&selected)
                else {
                    return Ok(None);
                };
                // Concrete IIR descriptors already substituted the telescope.
                nir::type_computations::charge_term(&definition.representation, &mut fuel)
                    .map_err(|_| nir::newtypes::ConversionError::Budget)?;
                Ok(Some(definition.representation.clone()))
            } else {
                let Some(definition) = self.newtypes.handle(declaration) else {
                    return Ok(None);
                };
                definition
                    .representation_budgeted(arguments, &[], &mut fuel)
                    .map(Some)
            }
        })();
        self.budget.steps -= (before - fuel) as u64;
        let representation = result
            .map_err(|error| {
                self.failure(
                    match error {
                        nir::newtypes::ConversionError::Budget => ErrorKind::StepsExhausted,
                        _ => ErrorKind::Type,
                    },
                    None,
                )
            })?
            .ok_or_else(|| self.failure(ErrorKind::Type, None))?;
        let representation = self.reflection_type(&representation)?;
        Ok(NewtypeReflection {
            subject: ty.clone(),
            representation,
        })
    }
}

#[cfg(test)]
mod tests;
