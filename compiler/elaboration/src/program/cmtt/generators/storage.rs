//! Reuse comptime storage admission for generator data without inspecting code
//! ports or unfolding opaque heads. Nominal layouts come from the current world.
use super::*;
use nir::phases::StorageError;

impl Checker<'_, '_> {
    pub(super) fn generator_comptime_storage(&self, ty: &Term, fuel: &mut usize) -> bool {
        let frames = vec![self.call_inference_scope().iter().map(|p| p.sort).collect()];
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
        nir::phases::validate_comptime_storage_with_layouts(ty, &frames, &resolve, fuel).is_ok()
    }
}
