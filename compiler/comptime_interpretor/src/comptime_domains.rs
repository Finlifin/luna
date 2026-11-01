//! Recheck dependent storage domains after invocation inputs are substituted.
use super::*;

impl Interpreter<'_> {
    pub(super) fn check_comptime_domain(
        &mut self,
        ty: &Term,
        source: SourceRange,
    ) -> Result<(), Error> {
        if !self.comptime_only {
            return Ok(());
        }
        let package = self
            .declaration_world
            .ok_or_else(|| self.failure(ErrorKind::PhaseMismatch, Some(source)))?;
        let before = usize::try_from(self.budget.steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let checked = nir::phases::validate_comptime_storage_with_nominals(
            ty,
            package,
            &[],
            &|ty, fuel| {
                let Term::Nominal {
                    declaration,
                    arguments,
                } = ty
                else {
                    return None;
                };
                *fuel = fuel.checked_sub(1)?;
                if let Some(definition) = self.structs.get(declaration) {
                    Some(
                        definition
                            .instantiate_fields_in_scope(arguments, &[])
                            .ok()?
                            .into_iter()
                            .map(|(_, field)| field)
                            .collect(),
                    )
                } else if let Some(definition) = self.enums.get(declaration) {
                    Some(
                        definition
                            .instantiate_variants_in_scope(arguments, &[])
                            .ok()?
                            .into_iter()
                            .flat_map(|variant| variant.fields)
                            .collect(),
                    )
                } else if let Some(definition) = self.newtypes.get(declaration) {
                    Some(vec![
                        definition
                            .representation_budgeted(arguments, &[], fuel)
                            .ok()?,
                    ])
                } else {
                    None
                }
            },
            &mut fuel,
        );
        self.budget.steps -= (before - fuel) as u64;
        checked.map_err(|_| {
            self.failure(
                if fuel == 0 {
                    ErrorKind::StepsExhausted
                } else {
                    ErrorKind::PhaseMismatch
                },
                Some(source),
            )
        })
    }
}
