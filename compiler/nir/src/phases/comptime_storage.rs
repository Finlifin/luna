//! Stored comptime payloads cannot contain runtime pointers. Type arguments,
//! code ports and runtime callable signatures describe values rather than store
//! them; inspecting those as payloads would reject legal metaprogramming.
use crate::{Sort, Term, program::Package};
use std::collections::HashSet;

pub(super) fn validate(
    ty: &Term,
    package: &Package,
    frames: &[Vec<Sort>],
) -> Result<(), &'static str> {
    match contains_pointer(ty, package, frames, &mut HashSet::new(), &mut 100_000, 0) {
        Ok(false) => Ok(()),
        Ok(true) => Err("runtime pointer values cannot inhabit comptime storage"),
        Err(()) => Err("comptime storage inspection limit or invalid payload layout"),
    }
}

fn contains_pointer(
    ty: &Term,
    package: &Package,
    frames: &[Vec<Sort>],
    seen: &mut HashSet<Term>,
    fuel: &mut usize,
    depth: usize,
) -> Result<bool, ()> {
    crate::type_computations::charge_term(ty, fuel).map_err(|_| ())?;
    if depth > 128 {
        return Err(());
    }
    if !seen.insert(ty.clone()) {
        return Ok(false);
    }
    let children = match ty {
        Term::Pointer(_) => return Ok(true),
        Term::Tuple(fields) => fields.clone(),
        Term::Record(fields) => fields.iter().map(|(_, ty)| ty.clone()).collect(),
        Term::Optional(inner) | Term::Computation { result: inner, .. } => vec![(**inner).clone()],
        Term::ErrorResult { errors, result } => errors
            .iter()
            .cloned()
            .chain(std::iter::once((**result).clone()))
            .collect(),
        Term::Closure { environment, .. } => environment.clone(),
        Term::ContextualClosure { context, .. } => match context.as_ref() {
            Term::ClosureContextValue(fields) => fields.clone(),
            // A scoped context is completed and checked on specialization.
            _ => vec![],
        },
        Term::NormalForm { .. } => {
            if let Some(fields) = crate::normal_forms::tuple_fields(Some(package), ty) {
                fields.to_vec()
            } else if let Some(element) =
                crate::normal_forms::list_element_budgeted(package, ty, fuel).map_err(|_| ())?
            {
                vec![element]
            } else {
                // Opaque heads are not unfolded to guess a representation.
                vec![]
            }
        }
        Term::Nominal {
            declaration,
            arguments,
        } => {
            if let Some(definition) = package
                .structs
                .iter()
                .find(|d| d.declaration == *declaration)
            {
                definition
                    .instantiate_fields_in_scope(arguments, frames)
                    .map_err(|_| ())?
                    .into_iter()
                    .map(|(_, field)| field)
                    .collect()
            } else if let Some(definition) =
                package.enums.iter().find(|d| d.declaration == *declaration)
            {
                definition
                    .instantiate_variants_in_scope(arguments, frames)
                    .map_err(|_| ())?
                    .into_iter()
                    .flat_map(|variant| variant.fields)
                    .collect()
            } else if let Some(definition) = package
                .newtypes
                .iter()
                .find(|d| d.declaration == *declaration)
            {
                vec![
                    definition
                        .representation_budgeted(arguments, frames, fuel)
                        .map_err(|_| ())?,
                ]
            } else {
                vec![]
            }
        }
        // Function identities and quoted code contain descriptions of runtime
        // inputs/results. Their signatures are not captured storage. Similarly,
        // Type values can describe pointers without holding a pointer value.
        _ => vec![],
    };
    for child in children {
        if contains_pointer(&child, package, frames, seen, fuel, depth + 1)? {
            return Ok(true);
        }
    }
    Ok(false)
}
