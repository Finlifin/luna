//! Stored comptime payloads cannot contain runtime pointers. Type arguments,
//! code ports and runtime callable signatures describe values rather than store
//! them; inspecting those as payloads would reject legal metaprogramming.
use super::{NominalStorageResolver, StorageError, storage_key::OrderedTerm};
use crate::{Sort, Term, program::Package};
use std::collections::BTreeSet;

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
    layouts: Option<BTreeSet<OrderedTerm>>,
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
        // Pay once for this complete input, including descriptive payloads and
        // keys used to recognize recursive layouts. A resolved field supplies
        // a new input. Ordinary structural children borrow this same tree.
        crate::type_computations::charge_term(ty, fuel)?;
        self.inspect(ty, fuel, depth)
    }

    fn inspect(&mut self, ty: &Term, fuel: &mut usize, depth: usize) -> Result<(), StorageError> {
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
            } => self.inspect_fields(fields, fuel, next),
            Term::Record(fields) => {
                self.inspect_fields(fields.iter().map(|(_, ty)| ty), fuel, next)
            }
            Term::Optional(inner) | Term::Computation { result: inner, .. } => {
                self.inspect(inner, fuel, next)
            }
            Term::ErrorResult { errors, result } => {
                self.inspect_fields(errors, fuel, next)?;
                self.inspect(result, fuel, next)
            }
            Term::ContextualClosure { context, .. } => match context.as_ref() {
                Term::ClosureContextValue(fields) => self.inspect_fields(fields, fuel, next),
                // A scoped context is completed and checked on specialization.
                _ => Ok(()),
            },
            Term::Nominal { .. } | Term::NormalForm { .. } => {
                let layouts = self.layouts.get_or_insert_with(BTreeSet::new);
                let key = OrderedTerm(ty.clone());
                if layouts.contains(&key) {
                    return Ok(());
                }
                layouts.insert(key);
                self.check_layout(ty, fuel, next)
            }
            // A callable signature or a Type value can describe pointers.
            _ => Ok(()),
        }
    }

    fn inspect_fields<'f>(
        &mut self,
        fields: impl IntoIterator<Item = &'f Term>,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<(), StorageError> {
        for field in fields {
            self.inspect(field, fuel, depth)?;
        }
        Ok(())
    }

    fn check_layout_fields(
        &mut self,
        fields: &[Term],
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
                return self.check_layout_fields(fields, fuel, depth);
            }
            if let Some(element) = crate::normal_forms::list_element_budgeted(package, ty, fuel)? {
                return self.check(&element, fuel, depth);
            }
            // Other opaque heads do not expose a stored representation here.
            return Ok(());
        }
        if let Some(resolve) = self.resolve {
            return self.check_layout_fields(&resolve(ty, fuel)?, fuel, depth);
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
            self.check_layout_fields(&fields, fuel, depth)
        } else if let Some(definition) =
            package.enums.iter().find(|d| d.declaration == *declaration)
        {
            let fields = definition.instantiate_stored_fields(arguments, self.frames, fuel)?;
            self.check_layout_fields(&fields, fuel, depth)
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
    fn nested_structural_storage_has_linear_cost_and_checks_the_innermost_value() {
        let levels = 64;
        let nest = |leaf| (0..levels).fold(leaf, |ty, _| Term::Optional(Box::new(ty)));
        let leaf = Term::Primitive(Primitive::Bool);
        let resolve = |_: &Term, _: &mut usize| unreachable!("no named layouts");
        let safe = nest(leaf.clone());
        let check = |ty: &Term, mut fuel| validate_layouts(ty, &[], &resolve, &mut fuel);
        let linear_budget = 3 * (levels + 2);
        check(&safe, linear_budget).unwrap();
        assert_eq!(check(&safe, 10), Err(StorageError::Budget));
        let pointer = nest(Term::Pointer(Box::new(leaf)));
        assert_eq!(
            check(&pointer, linear_budget),
            Err(StorageError::Invalid(
                "runtime pointer values cannot inhabit comptime storage",
            )),
        );
    }

    #[test]
    fn newly_resolved_field_payloads_have_their_own_budget() {
        let nominal = Term::Nominal {
            declaration: DeclarationId {
                package: 1,
                local: 7,
            },
            arguments: vec![],
        };
        let resolve = |_: &Term, _: &mut usize| {
            Ok(vec![Term::Record(vec![(
                "x".repeat(100),
                Term::Optional(Box::new(Term::Primitive(Primitive::Bool))),
            )])])
        };
        assert_eq!(
            validate_layouts(&nominal, &[], &resolve, &mut 100),
            Err(StorageError::Budget),
        );
        validate_layouts(&nominal, &[], &resolve, &mut 110).unwrap();
    }

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
