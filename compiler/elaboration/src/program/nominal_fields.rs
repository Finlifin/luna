//! Scoped payload observations charged to the existing elaboration budget.
//! Field routing does not establish formation, resource rights or target layout.
use super::*;
use nir::type_computations::Error as QueryError;

pub(super) fn metered<T>(
    steps: &mut u64,
    query: impl FnOnce(&mut usize) -> Result<T, QueryError>,
) -> Result<T, QueryError> {
    let before = usize::try_from(*steps).unwrap_or(usize::MAX);
    let mut fuel = before;
    let result = query(&mut fuel);
    *steps -= (before - fuel) as u64;
    result
}

// Retain only the declaration header needed by omitted-field defaults. The
// source field bodies and contracts need no second copy across expression work.
pub(super) fn constructor_fields(
    definition: &n::Struct,
    arguments: &[Term],
    frames: &[Vec<Sort>],
    steps: &mut u64,
) -> Result<(Vec<(String, Term)>, Vec<DeclarationId>, Vec<Term>), QueryError> {
    metered(steps, |fuel| {
        *fuel = fuel
            .checked_sub(
                definition
                    .type_parameters
                    .len()
                    .saturating_add(definition.parameter_types.len()),
            )
            .ok_or(QueryError::Budget)?;
        for domain in &definition.parameter_types {
            nir::type_computations::charge_term(domain, fuel)?;
        }
        let fields = definition.instantiate_fields_bounded(arguments, frames, fuel)?;
        Ok((
            fields,
            definition.type_parameters.clone(),
            definition.parameter_types.clone(),
        ))
    })
}

impl Checker<'_, '_> {
    pub(super) fn instantiated_variants(
        &mut self,
        ty: &Term,
        span: Span,
    ) -> Result<Vec<n::Variant>, Error> {
        let Term::Nominal {
            declaration,
            arguments,
        } = ty
        else {
            return Err(error(span, "variant requires an enum type"));
        };
        let frames = self.call_input_frames();
        let definition = self
            .enums
            .get(declaration)
            .ok_or_else(|| error(span, "missing enum descriptor"))?;
        metered(&mut self.required_steps, |fuel| {
            definition.instantiate_variants_bounded(arguments, &frames, fuel)
        })
        .map_err(|e| error(span, e.to_string()))
    }

    pub(super) fn instantiated_fields(
        &mut self,
        ty: &Term,
        span: Span,
    ) -> Result<Vec<(String, Term)>, Error> {
        let Term::Nominal {
            declaration,
            arguments,
        } = ty
        else {
            return Err(error(span, "fields require a struct type"));
        };
        let frames = self.call_input_frames();
        let definition = self
            .structs
            .get(declaration)
            .ok_or_else(|| error(span, "missing struct descriptor"))?;
        metered(&mut self.required_steps, |fuel| {
            definition.instantiate_fields_bounded(arguments, &frames, fuel)
        })
        .map_err(|e| error(span, e.to_string()))
    }

    pub(super) fn validate_enum_payloads(&mut self, ty: &Term, span: Span) -> Result<(), Error> {
        let Term::Nominal {
            declaration,
            arguments,
        } = ty
        else {
            return Err(error(span, "variant requires an enum type"));
        };
        let frames = self.call_input_frames();
        let definition = self
            .enums
            .get(declaration)
            .ok_or_else(|| error(span, "missing enum descriptor"))?;
        metered(&mut self.required_steps, |fuel| {
            definition
                .instantiate_stored_fields(arguments, &frames, fuel)
                .map(|_| ())
        })
        .map_err(|e| error(span, e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn constructor_observation_debits_failures_and_recovers_without_copying_contracts() {
        let declaration = DeclarationId {
            package: 97,
            local: 1,
        };
        let parameter = DeclarationId {
            package: 97,
            local: 2,
        };
        let mut definition = n::Struct {
            declaration,
            name: "ignored".repeat(10_000),
            type_parameters: vec![parameter],
            parameter_types: vec![Term::Primitive(P::Type)],
            fields: vec![(
                "field".repeat(1000),
                Term::Rigid {
                    declaration: parameter,
                    sort: Sort::Type,
                },
            )],
            requirements: vec![],
            capabilities: vec![],
        };
        let arguments = [Term::Primitive(P::Bool)];
        let mut steps = 500;
        assert_eq!(
            constructor_fields(&definition, &arguments, &[], &mut steps),
            Err(QueryError::Budget)
        );
        assert!(steps > 0 && steps < 500);
        definition.fields[0].0 = "value".into();
        let remaining = steps;
        let (fields, parameters, domains) =
            constructor_fields(&definition, &arguments, &[], &mut steps).unwrap();
        assert_eq!(fields, vec![("value".into(), arguments[0].clone())]);
        assert_eq!(parameters, definition.type_parameters);
        assert_eq!(domains, definition.parameter_types);
        assert!(steps < remaining);
    }

    #[test]
    fn failed_observations_charge_completed_work_and_preserve_remaining_fuel() {
        let mut steps = 7;
        let result: Result<(), QueryError> = metered(&mut steps, |fuel| {
            *fuel -= 2;
            *fuel = fuel.checked_sub(8).ok_or(QueryError::Budget)?;
            Ok(())
        });
        assert_eq!(result, Err(QueryError::Budget));
        assert_eq!(steps, 5);
        assert_eq!(
            metered(&mut steps, |fuel| {
                *fuel -= 3;
                Ok(9)
            }),
            Ok(9)
        );
        assert_eq!(steps, 2);
    }
}
