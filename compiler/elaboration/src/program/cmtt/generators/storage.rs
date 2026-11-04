//! Reuse comptime storage admission for generator data without inspecting code
//! ports or unfolding opaque heads. Nominal layouts come from the current world.
use super::*;
use nir::phases::StorageError;

impl Checker<'_, '_> {
    pub(super) fn generator_comptime_storage(
        &self,
        ty: &Term,
        fuel: &mut usize,
    ) -> Result<(), StorageError> {
        *fuel = fuel.checked_sub(1).ok_or(StorageError::Budget)?;
        let frames = vec![self.generator_storage_frame(fuel)?];
        let resolve = |ty: &Term, fuel: &mut usize| -> Result<Vec<Term>, StorageError> {
            if let Some(element) = self.list_element_type(ty) {
                nir::type_computations::charge_term(element, fuel)?;
                return Ok(vec![element.clone()]);
            }
            if let Some(fields) = self.bound_tuple_elements(ty) {
                return fields
                    .iter()
                    .map(|field| {
                        nir::type_computations::charge_term(field, fuel)?;
                        Ok(field.clone())
                    })
                    .collect();
            }
            let Term::Nominal {
                declaration,
                arguments,
            } = ty
            else {
                // An opaque NF head carries no storage view to unfold here.
                return Ok(vec![]);
            };
            if let Some(definition) = self.newtypes.get(declaration) {
                return Ok(vec![
                    definition.representation_budgeted(arguments, &frames, fuel)?,
                ]);
            }
            if let Some(definition) = self.structs.get(declaration) {
                definition
                    .instantiate_stored_fields(arguments, &frames, fuel)
                    .map_err(Into::into)
            } else if let Some(definition) = self.enums.get(declaration) {
                definition
                    .instantiate_stored_fields(arguments, &frames, fuel)
                    .map_err(Into::into)
            } else {
                // Abstract or not-yet-published descriptors are rechecked when
                // concrete generator arguments are admitted.
                Ok(vec![])
            }
        };
        nir::phases::validate_comptime_storage_with_layouts(ty, &frames, &resolve, fuel)
    }

    // Match call_inference_scope's known-prefix rules, but borrow only sorts.
    // Runtime type payloads in callable signatures are descriptions, not stored
    // generator data, so constructing this frame must not clone those payloads.
    fn generator_storage_frame(&self, fuel: &mut usize) -> Result<Vec<Sort>, StorageError> {
        if let Some(prefix) = self
            .signature_owner
            .and_then(|owner| self.signature_prefixes.get(&owner))
        {
            return collect_frame(prefix.iter().map(|p| p.sort), fuel);
        }
        if let Some(scope) = &self.default_input_scope {
            return collect_frame(
                scope
                    .iter()
                    .map_while(|(sort, ty)| ty.as_ref().map(|_| *sort)),
                fuel,
            );
        }
        if let Some(mut signature) = self.signature_owner.and_then(|owner| {
            self.signatures
                .get(&owner)
                .or_else(|| self.interface_signatures.get(&owner))
        }) {
            loop {
                *fuel = fuel.checked_sub(1).ok_or(StorageError::Budget)?;
                match signature {
                    Term::ComptimeFunction(inner)
                    | Term::NormalFormFunction(inner, _)
                    | Term::UnsafeFunction(inner)
                    | Term::Closure {
                        signature: inner, ..
                    }
                    | Term::ContextualClosure {
                        signature: inner, ..
                    } => signature = inner,
                    Term::Function { parameters, .. } => {
                        return collect_frame(parameters.iter().map(|p| p.sort), fuel);
                    }
                    _ => break,
                }
            }
        }
        if let Some(definition) = self
            .signature_owner
            .and_then(|owner| self.open_functions.get(&owner))
        {
            return collect_frame(definition.known_prefix_sorts(), fuel);
        }
        Ok(vec![])
    }
}

fn collect_frame(
    sorts: impl Iterator<Item = Sort>,
    fuel: &mut usize,
) -> Result<Vec<Sort>, StorageError> {
    let mut frame = Vec::new();
    for sort in sorts {
        *fuel = fuel.checked_sub(1).ok_or(StorageError::Budget)?;
        frame.push(sort);
    }
    Ok(frame)
}
