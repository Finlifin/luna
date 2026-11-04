//! Bounded declaration field substitution. Layout selection and receiving-world
//! admission stay with the caller; this operation never unfolds an NF head.
use crate::{
    DeclarationId, Sort, Term,
    program::{Enum, Struct, Variant, VariantShape},
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
    /// Specialize a complete declaration without copying discarded generic
    /// fields or contracts. Formation and normalization remain with the caller.
    pub fn instantiate_definition_bounded(
        &self,
        arguments: &[Term],
        frames: &[Vec<Sort>],
        fuel: &mut usize,
    ) -> Result<Self, Error> {
        let substitution = prepare(
            &self.type_parameters,
            &self.parameter_types,
            arguments,
            frames,
            self.fields.len(),
            fuel,
        )?;
        charge_text(&self.name, fuel)?;
        let fields = self
            .fields
            .iter()
            .map(|(name, ty)| {
                charge_text(name, fuel)?;
                Ok((name.clone(), substitution.instantiate_bounded(ty, fuel)?))
            })
            .collect::<Result<_, Error>>()?;
        let (requirements, capabilities) =
            contracts(&substitution, &self.requirements, &self.capabilities, fuel)?;
        Ok(Self {
            declaration: self.declaration,
            name: self.name.clone(),
            fields,
            type_parameters: vec![],
            parameter_types: vec![],
            requirements,
            capabilities,
        })
    }

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
    /// Preserve representation metadata and logical branch positions while
    /// specializing payloads and contracts under the caller's shared budget.
    pub fn instantiate_definition_bounded(
        &self,
        arguments: &[Term],
        frames: &[Vec<Sort>],
        fuel: &mut usize,
    ) -> Result<Self, Error> {
        let substitution = self.prepare_stored_fields(arguments, frames, fuel)?;
        charge_text(&self.name, fuel)?;
        charge_names(&self.groups, fuel)?;
        if let Some(encoding) = &self.encoding {
            reserve(encoding.variants().len(), fuel)?;
            for variant in encoding.variants() {
                charge_text(variant.name(), fuel)?;
                // Each fixed bit plan owns a mask and value word buffer.
                reserve(
                    variant
                        .fixed_mask()
                        .words()
                        .len()
                        .saturating_add(variant.fixed_value().words().len())
                        .saturating_mul(std::mem::size_of::<u32>()),
                    fuel,
                )?;
                reserve(variant.fields().len(), fuel)?;
                for field in variant.fields() {
                    charge_text(&field.name, fuel)?;
                }
            }
        }
        if let Some(encoding) = &self.symbol_encoding {
            reserve(encoding.variants().len(), fuel)?;
            for variant in encoding.variants() {
                charge_text(&variant.name, fuel)?;
                charge_text(&variant.value, fuel)?;
            }
        }
        let variants = self
            .variants
            .iter()
            .map(|variant| {
                charge_text(&variant.name, fuel)?;
                if let VariantShape::Named(names) = &variant.shape {
                    charge_names(names, fuel)?;
                }
                Ok(Variant {
                    name: variant.name.clone(),
                    shape: variant.shape.clone(),
                    fields: variant
                        .fields
                        .iter()
                        .map(|ty| substitution.instantiate_bounded(ty, fuel))
                        .collect::<Result<_, _>>()?,
                })
            })
            .collect::<Result<_, Error>>()?;
        let (requirements, capabilities) =
            contracts(&substitution, &self.requirements, &self.capabilities, fuel)?;
        Ok(Self {
            declaration: self.declaration,
            name: self.name.clone(),
            variants,
            type_parameters: vec![],
            parameter_types: vec![],
            requirements,
            capabilities,
            is_intrinsic: self.is_intrinsic,
            groups: self.groups.clone(),
            encoding: self.encoding.clone(),
            symbol_encoding: self.symbol_encoding.clone(),
        })
    }

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

impl crate::newtypes::Newtype {
    /// Specialize a complete newtype without changing its nominal identity or
    /// using its representation as equality evidence. Completed domain admission
    /// and formation remain separate obligations of the receiving consumer.
    pub fn instantiate_definition_bounded(
        &self,
        arguments: &[Term],
        frames: &[Vec<Sort>],
        fuel: &mut usize,
    ) -> Result<Self, Error> {
        let substitution = self.representation_substitution(arguments, frames, fuel)?;
        charge_text(&self.name, fuel)?;
        let representation = substitution.instantiate_bounded(&self.representation, fuel)?;
        let (requirements, capabilities) =
            contracts(&substitution, &self.requirements, &self.capabilities, fuel)?;
        Ok(Self {
            declaration: self.declaration,
            name: self.name.clone(),
            representation,
            type_parameters: vec![],
            parameter_types: vec![],
            capture_count: 0,
            requirements,
            capabilities,
        })
    }
}

fn reserve(work: usize, fuel: &mut usize) -> Result<(), Error> {
    *fuel = fuel.checked_sub(work).ok_or(Error::Budget)?;
    Ok(())
}

fn charge_text(text: &str, fuel: &mut usize) -> Result<(), Error> {
    reserve(text.len().saturating_add(1), fuel)
}

fn charge_names(names: &[String], fuel: &mut usize) -> Result<(), Error> {
    reserve(names.len(), fuel)?;
    for name in names {
        charge_text(name, fuel)?;
    }
    Ok(())
}

fn contracts(
    substitution: &crate::DeclarationSubstitution<'_>,
    requirements: &[crate::program::TypePredicate],
    capabilities: &[crate::program::TraitRequirement],
    fuel: &mut usize,
) -> Result<
    (
        Vec<crate::program::TypePredicate>,
        Vec<crate::program::TraitRequirement>,
    ),
    Error,
> {
    reserve(requirements.len().saturating_add(capabilities.len()), fuel)?;
    let requirements = requirements
        .iter()
        .map(|r| substitution.instantiate_predicate_bounded(r, fuel))
        .collect::<Result<_, _>>()?;
    let capabilities = capabilities
        .iter()
        .map(|r| substitution.instantiate_requirement_bounded(r, fuel))
        .collect::<Result<_, _>>()?;
    Ok((requirements, capabilities))
}
