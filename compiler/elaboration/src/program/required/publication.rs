//! Receive a completed invocation's definitions in the source checking world.
//! VM publication does not itself publish those descriptions into the caller.
use super::*;
use crate::definitions::PublishedDefinitions;

pub(super) struct Receiver<'a> {
    pub context: &'a mut crate::ptc::Context,
    pub structs: &'a mut BTreeMap<DeclarationId, n::Struct>,
    pub enums: &'a mut BTreeMap<DeclarationId, n::Enum>,
    pub newtypes: &'a mut BTreeMap<DeclarationId, nir::newtypes::Newtype>,
    pub implementation_index: &'a mut crate::implementation::Index,
}

impl Receiver<'_> {
    pub(super) fn register(
        self,
        definitions: &PublishedDefinitions,
        steps: &mut u64,
        span: Span,
    ) -> Result<(), Error> {
        if definitions.structs.is_empty()
            && definitions.enums.is_empty()
            && definitions.newtypes.is_empty()
        {
            return Ok(());
        }
        // Every fallible reception step precedes all caller-world writes. The
        // descriptors have already passed builder admission in the invocation;
        // this pays their transfer cost without granting partial visibility.
        for batch in &definitions.structs {
            for definition in batch.iter() {
                let before = usize::try_from(*steps).unwrap_or(usize::MAX);
                let mut fuel = before;
                let copied = (|| {
                    fuel = fuel
                        .checked_sub(
                            definition
                                .name
                                .len()
                                .saturating_add(definition.fields.len()),
                        )
                        .ok_or(())?;
                    for (name, ty) in &definition.fields {
                        fuel = fuel.checked_sub(name.len()).ok_or(())?;
                        nir::type_computations::charge_term(ty, &mut fuel).map_err(|_| ())?;
                    }
                    Ok::<_, ()>(())
                })();
                *steps -= (before - fuel) as u64;
                copied
                    .map_err(|_| error(span, "generated struct registration budget exhausted"))?;
            }
        }
        for batch in &definitions.enums {
            for definition in batch.iter() {
                let before = usize::try_from(*steps).unwrap_or(usize::MAX);
                let mut fuel = before;
                let copied = comptime_interpretor::EnumBuildRequest::charge_descriptor(
                    &definition.name,
                    &definition.groups,
                    &definition.variants,
                    definition.encoding.as_ref(),
                    definition.symbol_encoding.as_ref(),
                    &mut fuel,
                );
                *steps -= (before - fuel) as u64;
                copied.map_err(|_| error(span, "generated enum registration budget exhausted"))?;
            }
        }
        for batch in &definitions.newtypes {
            for definition in batch.iter() {
                let before = usize::try_from(*steps).unwrap_or(usize::MAX);
                let mut fuel = before;
                let copied = (|| {
                    fuel = fuel
                        .checked_sub(definition.name.len().saturating_add(1))
                        .ok_or(())?;
                    nir::type_computations::charge_term(&definition.representation, &mut fuel)
                        .map_err(|_| ())
                })();
                *steps -= (before - fuel) as u64;
                copied
                    .map_err(|_| error(span, "generated newtype registration budget exhausted"))?;
            }
        }
        // No recoverable checks or callbacks remain. Install descriptions and
        // constructor authority together, then synchronize the selection view.
        // Reserved builder identities and spent invocation work are not undone.
        for batch in &definitions.structs {
            for definition in batch.iter() {
                self.context
                    .admit_nominal_constructor(definition.declaration);
                self.structs
                    .insert(definition.declaration, definition.clone());
            }
        }
        for batch in &definitions.enums {
            for definition in batch.iter() {
                self.context
                    .admit_nominal_constructor(definition.declaration);
                self.enums
                    .insert(definition.declaration, definition.clone());
            }
        }
        for batch in &definitions.newtypes {
            for definition in batch.iter() {
                self.context
                    .admit_nominal_constructor(definition.declaration);
                self.newtypes
                    .insert(definition.declaration, definition.clone());
            }
        }
        self.implementation_index.sync_nominal_layouts(
            self.structs.values(),
            self.enums.values(),
            self.newtypes.values(),
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests;
