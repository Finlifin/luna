//! Representation observations share required-computation fuel. These checks
//! do not prove formation, unfold opaque heads or admit runtime pointer storage.
use super::*;
use nir::type_computations::Error as QueryError;

impl Checker<'_, '_> {
    pub(super) fn with_input_domain_fuel(
        &mut self,
        span: Span,
        query: impl FnOnce(&Self, &mut usize) -> Result<bool, QueryError>,
    ) -> Result<bool, Error> {
        let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let result = query(self, &mut fuel);
        self.required_steps -= (before - fuel) as u64;
        result.map_err(|reason| match reason {
            QueryError::Budget => error(span, "comptime input domain inspection budget exhausted"),
            other => error(span, format!("invalid comptime input domain: {other}")),
        })
    }

    pub(super) fn input_domain_in(
        &self,
        ty: &Term,
        prior: &[nir::DeclarationParameter],
        fuel: &mut usize,
    ) -> Result<bool, QueryError> {
        nir::normal_forms::supported_input_type_with_collections_budgeted(
            ty,
            prior,
            |head| Some(head) == self.protocols.tuple,
            |head| Some(head) == self.protocols.meta_list,
            |id| {
                self.structs.contains_key(&id)
                    || self.enums.contains_key(&id)
                    || self.newtypes.contains_key(&id)
                    || self.context.is_nominal_constructor(id)
            },
            self.target,
            fuel,
        )
    }

    pub(super) fn normal_input_domain_supported(
        &mut self,
        ty: &Term,
        types: &[DeclarationId],
        values: &[(DeclarationId, n::LocalId, Term)],
        span: Span,
    ) -> Result<bool, Error> {
        self.with_input_domain_fuel(span, |this, fuel| {
            *fuel = fuel.checked_sub(types.len()).ok_or(QueryError::Budget)?;
            *fuel = fuel.checked_sub(values.len()).ok_or(QueryError::Budget)?;
            let prior: Vec<_> = types
                .iter()
                .map(|id| nir::DeclarationParameter {
                    declaration: *id,
                    sort: Sort::Type,
                })
                .chain(
                    values
                        .iter()
                        .filter(|(_, _, domain)| !matches!(domain, Term::Primitive(P::Type)))
                        .map(|(id, _, _)| nir::DeclarationParameter {
                            declaration: *id,
                            sort: Sort::ComptimeValue,
                        }),
                )
                .collect();
            this.input_domain_in(ty, &prior, fuel)
        })
    }

    pub(super) fn generator_clause_domain(
        &mut self,
        ty: &Term,
        parameters: &[DeclarationId],
        span: Span,
    ) -> Result<bool, Error> {
        self.with_input_domain_fuel(span, |this, fuel| {
            let prior = this.input_binders_in(parameters, fuel)?;
            this.input_domain_in(ty, &prior, fuel)
        })
    }
}
