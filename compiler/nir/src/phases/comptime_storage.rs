//! Stored comptime payloads cannot contain runtime pointers. Type arguments,
//! code ports and runtime callable signatures describe values rather than store
//! them; inspecting those as payloads would reject legal metaprogramming.
use super::StorageError;
use crate::{Sort, Term, program::Package};
use std::collections::HashSet;

pub(super) fn validate_resolved(
    ty: &Term,
    package: &Package,
    frames: &[Vec<Sort>],
    resolve: super::NominalStorageResolver<'_>,
    fuel: &mut usize,
) -> Result<(), StorageError> {
    match contains_pointer(
        ty,
        Some(package),
        frames,
        &mut HashSet::new(),
        fuel,
        0,
        resolve,
    ) {
        Ok(false) => Ok(()),
        Ok(true) => Err(StorageError::Invalid(
            "runtime pointer values cannot inhabit comptime storage",
        )),
        Err(error) => Err(error),
    }
}

/// Inspect concrete artifact layouts without reapplying source telescopes.
pub(super) fn validate_layouts(
    ty: &Term,
    frames: &[Vec<Sort>],
    resolve: &dyn Fn(&Term, &mut usize) -> Result<Vec<Term>, StorageError>,
    fuel: &mut usize,
) -> Result<(), StorageError> {
    match contains_pointer(
        ty,
        None,
        frames,
        &mut HashSet::new(),
        fuel,
        0,
        Some(resolve),
    ) {
        Ok(false) => Ok(()),
        Ok(true) => Err(StorageError::Invalid(
            "runtime pointer values cannot inhabit comptime storage",
        )),
        Err(error) => Err(error),
    }
}

fn contains_pointer(
    ty: &Term,
    package: Option<&Package>,
    frames: &[Vec<Sort>],
    seen: &mut HashSet<Term>,
    fuel: &mut usize,
    depth: usize,
    resolve: super::NominalStorageResolver<'_>,
) -> Result<bool, StorageError> {
    crate::type_computations::charge_term(ty, fuel)?;
    if depth > 128 {
        return Err(StorageError::Budget);
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
            if package.is_none() {
                resolve.ok_or(StorageError::Invalid("storage layout resolver unavailable"))?(
                    ty, fuel,
                )?
            } else if let Some(fields) = crate::normal_forms::tuple_fields(package, ty) {
                fields.to_vec()
            } else if let Some(element) =
                crate::normal_forms::list_element_budgeted(package.unwrap(), ty, fuel)?
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
            if let Some(resolve) = resolve {
                resolve(ty, fuel)?
            } else if let Some(definition) = package.and_then(|package| {
                package
                    .structs
                    .iter()
                    .find(|d| d.declaration == *declaration)
            }) {
                definition.instantiate_stored_fields(arguments, frames, fuel)?
            } else if let Some(definition) = package
                .and_then(|package| package.enums.iter().find(|d| d.declaration == *declaration))
            {
                definition.instantiate_stored_fields(arguments, frames, fuel)?
            } else if let Some(definition) = package.and_then(|package| {
                package
                    .newtypes
                    .iter()
                    .find(|d| d.declaration == *declaration)
            }) {
                vec![definition.representation_budgeted(arguments, frames, fuel)?]
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
        if contains_pointer(&child, package, frames, seen, fuel, depth + 1, resolve)? {
            return Ok(true);
        }
    }
    Ok(false)
}
