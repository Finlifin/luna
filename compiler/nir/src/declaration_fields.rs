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
    let substitution = prepare(parameters, domains, arguments, frames, fields.len(), fuel)?;
    fields
        .map(|field| substitution.instantiate_bounded(field, fuel))
        .collect()
}

fn prepare<'a>(
    parameters: &[DeclarationId],
    domains: &[Term],
    arguments: &'a [Term],
    frames: &'a [Vec<Sort>],
    field_count: usize,
    fuel: &mut usize,
) -> Result<crate::DeclarationSubstitution<'a>, Error> {
    *fuel = fuel
        .checked_sub(
            field_count
                .saturating_add(parameters.len())
                .saturating_add(arguments.len()),
        )
        .ok_or(Error::Budget)?;
    for domain in domains {
        crate::type_computations::charge_term(domain, fuel)?;
    }
    crate::DeclarationSubstitution::bounded(parameters, domains, arguments, frames, fuel)
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
    // Reserve traversal and output slots before collecting either view. The
    // complete parameter frame is validated once, including payload-free enums.
    fn prepare_stored_fields<'a>(
        &self,
        arguments: &'a [Term],
        frames: &'a [Vec<Sort>],
        fuel: &mut usize,
    ) -> Result<crate::DeclarationSubstitution<'a>, Error> {
        *fuel = fuel.checked_sub(self.variants.len()).ok_or(Error::Budget)?;
        let count = self.variants.iter().try_fold(0usize, |count, variant| {
            count.checked_add(variant.fields.len()).ok_or(Error::Budget)
        })?;
        prepare(
            &self.type_parameters,
            &self.parameter_types,
            arguments,
            frames,
            count,
            fuel,
        )
    }

    /// All possible payload types. Names and variant metadata are not copied.
    pub fn instantiate_stored_fields(
        &self,
        arguments: &[Term],
        frames: &[Vec<Sort>],
        fuel: &mut usize,
    ) -> Result<Vec<Term>, Error> {
        let substitution = self.prepare_stored_fields(arguments, frames, fuel)?;
        self.variants
            .iter()
            .flat_map(|variant| &variant.fields)
            .map(|field| substitution.instantiate_bounded(field, fuel))
            .collect()
    }

    /// Retain every variant position, including empty alternatives, for branch
    /// consumers such as cleanup. This does not reinterpret the source tag ABI.
    pub fn instantiate_variant_fields(
        &self,
        arguments: &[Term],
        frames: &[Vec<Sort>],
        fuel: &mut usize,
    ) -> Result<Vec<Vec<Term>>, Error> {
        let substitution = self.prepare_stored_fields(arguments, frames, fuel)?;
        self.variants
            .iter()
            .map(|variant| {
                variant
                    .fields
                    .iter()
                    .map(|field| substitution.instantiate_bounded(field, fuel))
                    .collect()
            })
            .collect()
    }
}
