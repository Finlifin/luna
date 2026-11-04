//! Bounded declaration field substitution. Layout selection and receiving-world
//! admission stay with the caller; this operation never unfolds an NF head.
use crate::{
    DeclarationId, Sort, Term,
    program::{Enum, Struct},
    type_computations::Error,
};

/// Instantiate the selected field types in the caller's binder frames. Even an
/// empty selection checks the complete parameter frame before returning.
pub fn instantiate<'a>(
    parameters: &[DeclarationId],
    domains: &[Term],
    arguments: &[Term],
    fields: impl ExactSizeIterator<Item = &'a Term>,
    frames: &[Vec<Sort>],
    fuel: &mut usize,
) -> Result<Vec<Term>, Error> {
    *fuel = fuel
        .checked_sub(
            fields
                .len()
                .saturating_add(parameters.len())
                .saturating_add(arguments.len()),
        )
        .ok_or(Error::Budget)?;
    for domain in domains {
        crate::type_computations::charge_term(domain, fuel)?;
    }
    let substitution =
        crate::DeclarationSubstitution::bounded(parameters, domains, arguments, frames, fuel)?;
    fields
        .map(|field| substitution.instantiate_bounded(field, fuel))
        .collect()
}

impl Struct {
    /// Stored payload types only; names are not part of storage-phase inspection.
    pub fn instantiate_stored_fields(
        &self,
        arguments: &[Term],
        frames: &[Vec<Sort>],
        fuel: &mut usize,
    ) -> Result<Vec<Term>, Error> {
        instantiate(
            &self.type_parameters,
            &self.parameter_types,
            arguments,
            self.fields.iter().map(|(_, ty)| ty),
            frames,
            fuel,
        )
    }
}

impl Enum {
    /// All possible payloads, including inactive variants. Reserve traversal and
    /// reference storage before collecting; empty alternatives still cost work.
    pub fn instantiate_stored_fields(
        &self,
        arguments: &[Term],
        frames: &[Vec<Sort>],
        fuel: &mut usize,
    ) -> Result<Vec<Term>, Error> {
        *fuel = fuel.checked_sub(self.variants.len()).ok_or(Error::Budget)?;
        let count = self.variants.iter().try_fold(0usize, |count, variant| {
            count.checked_add(variant.fields.len()).ok_or(Error::Budget)
        })?;
        *fuel = fuel.checked_sub(count).ok_or(Error::Budget)?;
        let fields: Vec<_> = self
            .variants
            .iter()
            .flat_map(|variant| &variant.fields)
            .collect();
        instantiate(
            &self.type_parameters,
            &self.parameter_types,
            arguments,
            fields.into_iter(),
            frames,
            fuel,
        )
    }
}
