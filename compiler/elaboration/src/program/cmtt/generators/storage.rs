//! Reuse comptime storage admission for generator data without inspecting code
//! ports or unfolding opaque heads. Nominal layouts come from the current world.
use super::*;

impl Checker<'_, '_> {
    pub(super) fn generator_comptime_storage(&self, ty: &Term, fuel: &mut usize) -> bool {
        let frames = vec![self.call_inference_scope().iter().map(|p| p.sort).collect()];
        let resolve = |ty: &Term, fuel: &mut usize| -> Option<Vec<Term>> {
            if let Some(element) = self.list_element_type(ty) {
                nir::type_computations::charge_term(element, fuel).ok()?;
                return Some(vec![element.clone()]);
            }
            if let Some(fields) = self.bound_tuple_elements(ty) {
                return fields
                    .iter()
                    .map(|field| {
                        nir::type_computations::charge_term(field, fuel).ok()?;
                        Some(field.clone())
                    })
                    .collect();
            }
            let Term::Nominal {
                declaration,
                arguments,
            } = ty
            else {
                // An opaque NF head carries no storage view to unfold here.
                return Some(vec![]);
            };
            if let Some(definition) = self.newtypes.get(declaration) {
                return Some(vec![
                    definition
                        .representation_budgeted(arguments, &frames, fuel)
                        .ok()?,
                ]);
            }
            let (parameters, domains, fields): (_, _, Vec<_>) =
                if let Some(definition) = self.structs.get(declaration) {
                    *fuel = fuel.checked_sub(definition.fields.len())?;
                    (
                        &definition.type_parameters,
                        &definition.parameter_types,
                        definition.fields.iter().map(|(_, field)| field).collect(),
                    )
                } else if let Some(definition) = self.enums.get(declaration) {
                    for variant in &definition.variants {
                        *fuel = fuel.checked_sub(variant.fields.len().checked_add(1)?)?;
                    }
                    (
                        &definition.type_parameters,
                        &definition.parameter_types,
                        definition
                            .variants
                            .iter()
                            .flat_map(|variant| &variant.fields)
                            .collect(),
                    )
                } else {
                    // Abstract or not-yet-published descriptors are rechecked
                    // when concrete generator arguments are admitted.
                    return Some(vec![]);
                };
            let substitution = nir::DeclarationSubstitution::bounded(
                parameters, domains, arguments, &frames, fuel,
            )
            .ok()?;
            fields
                .into_iter()
                .map(|field| substitution.instantiate_bounded(field, fuel).ok())
                .collect()
        };
        nir::phases::validate_comptime_storage_with_layouts(ty, &frames, &resolve, fuel).is_ok()
    }
}
