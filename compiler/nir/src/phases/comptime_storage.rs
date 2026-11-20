//! Stored comptime payloads cannot contain runtime pointers. Type arguments,
//! code ports and runtime callable signatures describe values rather than store
//! them; inspecting those as payloads would reject legal metaprogramming.
use super::{NominalStorageResolver, StorageError};
use crate::{Sort, Term, program::Package};
use std::collections::HashSet;

pub(super) fn validate_resolved(
    ty: &Term,
    package: &Package,
    frames: &[Vec<Sort>],
    resolve: NominalStorageResolver<'_>,
    fuel: &mut usize,
) -> Result<(), StorageError> {
    Query::new(Some(package), frames, resolve).check(ty, fuel, 0)
}

/// Inspect concrete artifact layouts without reapplying source telescopes.
pub(super) fn validate_layouts(
    ty: &Term,
    frames: &[Vec<Sort>],
    resolve: &dyn Fn(&Term, &mut usize) -> Result<Vec<Term>, StorageError>,
    fuel: &mut usize,
) -> Result<(), StorageError> {
    Query::new(None, frames, Some(resolve)).check(ty, fuel, 0)
}

struct Query<'a> {
    package: Option<&'a Package>,
    frames: &'a [Vec<Sort>],
    resolve: NominalStorageResolver<'a>,
    /// Only expansion of a named layout can revisit a type recursively.
    layouts: Option<HashSet<Term>>,
}

impl<'a> Query<'a> {
    fn new(
        package: Option<&'a Package>,
        frames: &'a [Vec<Sort>],
        resolve: NominalStorageResolver<'a>,
    ) -> Self {
        Self {
            package,
            frames,
            resolve,
            layouts: None,
        }
    }

    fn check(&mut self, ty: &Term, fuel: &mut usize, depth: usize) -> Result<(), StorageError> {
        crate::type_computations::charge_term(ty, fuel)?;
        if depth > 128 {
            return Err(StorageError::Budget);
        }
        let next = depth + 1;
        match ty {
            Term::Pointer(_) => Err(StorageError::Invalid(
                "runtime pointer values cannot inhabit comptime storage",
            )),
            Term::Tuple(fields)
            | Term::Closure {
                environment: fields,
                ..
            } => self.fields(fields, fuel, next),
            Term::Record(fields) => self.fields(fields.iter().map(|(_, ty)| ty), fuel, next),
            Term::Optional(inner) | Term::Computation { result: inner, .. } => {
                self.check(inner, fuel, next)
            }
            Term::ErrorResult { errors, result } => {
                self.fields(errors, fuel, next)?;
                self.check(result, fuel, next)
            }
            Term::ContextualClosure { context, .. } => match context.as_ref() {
                Term::ClosureContextValue(fields) => self.fields(fields, fuel, next),
                // A scoped context is completed and checked on specialization.
                _ => Ok(()),
            },
            Term::Nominal { .. } | Term::NormalForm { .. } => {
                let layouts = self.layouts.get_or_insert_with(HashSet::new);
                if layouts.contains(ty) {
                    return Ok(());
                }
                layouts.insert(ty.clone());
                self.check_layout(ty, fuel, next)
            }
            // A callable signature or a Type value can describe pointers.
            _ => Ok(()),
        }
    }

    fn fields<'f>(
        &mut self,
        fields: impl IntoIterator<Item = &'f Term>,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<(), StorageError> {
        for field in fields {
            self.check(field, fuel, depth)?;
        }
        Ok(())
    }

    fn check_layout(
        &mut self,
        ty: &Term,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<(), StorageError> {
        if let (Some(package), Term::NormalForm { .. }) = (self.package, ty) {
            if let Some(fields) = crate::normal_forms::tuple_fields(Some(package), ty) {
                return self.fields(fields, fuel, depth);
            }
            if let Some(element) = crate::normal_forms::list_element_budgeted(package, ty, fuel)? {
                return self.check(&element, fuel, depth);
            }
            // Other opaque heads do not expose a stored representation here.
            return Ok(());
        }
        if let Some(resolve) = self.resolve {
            return self.fields(&resolve(ty, fuel)?, fuel, depth);
        }
        let (
            Some(package),
            Term::Nominal {
                declaration,
                arguments,
            },
        ) = (self.package, ty)
        else {
            return Err(StorageError::Invalid("storage layout resolver unavailable"));
        };
        if let Some(definition) = package
            .structs
            .iter()
            .find(|d| d.declaration == *declaration)
        {
            let fields = definition.instantiate_stored_fields(arguments, self.frames, fuel)?;
            self.fields(&fields, fuel, depth)
        } else if let Some(definition) =
            package.enums.iter().find(|d| d.declaration == *declaration)
        {
            let fields = definition.instantiate_stored_fields(arguments, self.frames, fuel)?;
            self.fields(&fields, fuel, depth)
        } else if let Some(definition) = package
            .newtypes
            .iter()
            .find(|d| d.declaration == *declaration)
        {
            let field = definition.representation_budgeted(arguments, self.frames, fuel)?;
            self.check(&field, fuel, depth)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DeclarationId, Primitive};
    use std::cell::Cell;

    #[test]
    fn recursive_layouts_visit_each_instance_and_check_later_fields() {
        let instance = |flag| Term::Nominal {
            declaration: DeclarationId {
                package: 1,
                local: 0,
            },
            arguments: vec![Term::Bool(flag)],
        };
        let safe = instance(false);
        let unsafe_field = instance(true);
        let root = Term::Tuple(vec![safe.clone(), safe.clone(), unsafe_field.clone()]);
        let package = Package::default();
        for source in [None, Some(&package)] {
            let visits = Cell::new(0);
            let resolve = |ty: &Term, _: &mut usize| {
                visits.set(visits.get() + 1);
                if ty == &safe {
                    Ok(vec![Term::Optional(Box::new(safe.clone()))])
                } else {
                    assert_eq!(ty, &unsafe_field);
                    Ok(vec![Term::Pointer(Box::new(Term::Primitive(
                        Primitive::Bool,
                    )))])
                }
            };
            let error = Query::new(source, &[], Some(&resolve))
                .check(&root, &mut 1_000, 0)
                .unwrap_err();
            assert_eq!(visits.get(), 2);
            assert_eq!(
                error,
                StorageError::Invalid("runtime pointer values cannot inhabit comptime storage")
            );
        }
    }
}
