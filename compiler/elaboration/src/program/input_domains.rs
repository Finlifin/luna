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
        visible: impl FnMut(DeclarationId, Sort, &mut usize) -> Result<bool, QueryError>,
        fuel: &mut usize,
    ) -> Result<bool, QueryError> {
        nir::normal_forms::supported_input_type_in_scope(
            ty,
            visible,
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

    fn visible_input_has_sort(
        &self,
        parameters: &[DeclarationId],
        id: DeclarationId,
        sort: Sort,
        fuel: &mut usize,
    ) -> Result<bool, QueryError> {
        *fuel = fuel
            .checked_sub(parameters.len())
            .ok_or(QueryError::Budget)?;
        if !parameters.contains(&id) {
            return Ok(false);
        }
        let domain = self.observe_input_domain(id, || {
            *fuel = fuel.checked_sub(1).ok_or(QueryError::Budget)?;
            Ok(())
        })?;
        Ok(domain.map_or(Sort::Type, Sort::of_comptime_domain) == sort)
    }

    pub(super) fn normal_input_domain_supported(
        &mut self,
        ty: &Term,
        inherited: &[DeclarationId],
        prefix: &BTreeMap<DeclarationId, Sort>,
        span: Span,
    ) -> Result<bool, Error> {
        self.with_input_domain_fuel(span, |this, fuel| {
            this.input_domain_in(
                ty,
                |id, sort, fuel| match prefix.get(&id) {
                    Some(declared) => Ok(*declared == sort),
                    None => this.visible_input_has_sort(inherited, id, sort, fuel),
                },
                fuel,
            )
        })
    }

    pub(super) fn input_domain_with_visible_ids(
        &self,
        ty: &Term,
        parameters: &[DeclarationId],
        fuel: &mut usize,
    ) -> Result<bool, QueryError> {
        self.input_domain_in(
            ty,
            |id, sort, fuel| self.visible_input_has_sort(parameters, id, sort, fuel),
            fuel,
        )
    }

    pub(super) fn generator_clause_domain(
        &mut self,
        ty: &Term,
        parameters: &[DeclarationId],
        span: Span,
    ) -> Result<bool, Error> {
        self.with_input_domain_fuel(span, |this, fuel| {
            this.input_domain_with_visible_ids(ty, parameters, fuel)
        })
    }
}
