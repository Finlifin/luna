//! Storage observations belong to one borrowed, immutable declaration world.
//! A failed budget is not a reusable statement about a type's storage phase.
use super::{Package, Sort, StorageError, Term, validate_runtime_storage_inner};
use std::collections::HashMap;

pub(super) struct RuntimeStorageQueries<'a> {
    package: &'a Package,
    entries: HashMap<Term, HashMap<Vec<Vec<Sort>>, Result<(), StorageError>>>,
}

impl<'a> RuntimeStorageQueries<'a> {
    pub(super) fn new(package: &'a Package) -> Self {
        Self {
            package,
            entries: HashMap::new(),
        }
    }

    pub(super) fn check(
        &mut self,
        ty: &Term,
        frames: &[Vec<Sort>],
        fuel: &mut usize,
    ) -> Result<(), StorageError> {
        // Pay for the key before hashing or retaining it, including warm hits.
        // Borrowed lookup avoids allocating a candidate frame just to query it.
        *fuel = fuel
            .checked_sub(frames.len().saturating_add(1))
            .ok_or(StorageError::Budget)?;
        for frame in frames {
            *fuel = fuel.checked_sub(frame.len()).ok_or(StorageError::Budget)?;
        }
        crate::type_computations::charge_term(ty, fuel)?;
        if let Some(result) = self.entries.get(ty).and_then(|scopes| scopes.get(frames)) {
            return *result;
        }
        let result = validate_runtime_storage_inner(ty, self.package, frames, None, fuel);
        if !matches!(result, Err(StorageError::Budget)) {
            self.entries
                .entry(ty.clone())
                .or_default()
                .insert(frames.to_vec(), result);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DeclarationId, Primitive as P, program::Struct};

    #[test]
    fn failed_expansions_are_not_cached_and_warm_keys_still_cost_work() {
        let parameter = DeclarationId {
            package: 98,
            local: 1,
        };
        let declaration = DeclarationId {
            package: 98,
            local: 2,
        };
        let package = Package {
            structs: vec![Struct {
                declaration,
                name: "Container".into(),
                type_parameters: vec![parameter],
                parameter_types: vec![Term::Primitive(P::Type)],
                fields: (0..64)
                    .map(|i| {
                        (
                            format!("field{i}"),
                            Term::Rigid {
                                declaration: parameter,
                                sort: Sort::Type,
                            },
                        )
                    })
                    .collect(),
                requirements: vec![],
                capabilities: vec![],
            }],
            ..Default::default()
        };
        let ty = Term::Nominal {
            declaration,
            arguments: vec![Term::Record(
                (0..32)
                    .map(|i| (format!("input{i}"), Term::Primitive(P::Bool)))
                    .collect(),
            )],
        };
        let mut queries = RuntimeStorageQueries::new(&package);
        assert_eq!(
            queries.check(&ty, &[], &mut 2000),
            Err(StorageError::Budget)
        );
        assert!(queries.entries.is_empty());
        queries.check(&ty, &[], &mut 100_000).unwrap();
        assert_eq!(queries.entries.len(), 1);
        let mut fuel = 2000;
        queries.check(&ty, &[], &mut fuel).unwrap();
        assert!(fuel < 2000);
        assert_eq!(queries.check(&ty, &[], &mut 0), Err(StorageError::Budget));
        queries.check(&ty, &[], &mut 2000).unwrap();
    }

    #[test]
    fn frame_and_world_identity_are_part_of_storage_evidence() {
        let package = Package::default();
        let mut queries = RuntimeStorageQueries::new(&package);
        let bound = Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        };
        queries
            .check(&bound, &[vec![Sort::Type]], &mut 1000)
            .unwrap();
        assert!(matches!(
            queries.check(&bound, &[], &mut 1000),
            Err(StorageError::Invalid(_))
        ));
        queries
            .check(&bound, &[vec![Sort::Type]], &mut 1000)
            .unwrap();
        assert_eq!(queries.entries[&bound].len(), 2);
        let mut fuel = 16;
        assert_eq!(
            queries.check(&bound, &[vec![Sort::Type; 100]], &mut fuel),
            Err(StorageError::Budget)
        );
        assert!(fuel > 0 && fuel < 16);
        assert_eq!(queries.entries[&bound].len(), 2);

        let declaration = DeclarationId {
            package: 98,
            local: 3,
        };
        let a = Package {
            structs: vec![Struct {
                declaration,
                name: "Value".into(),
                type_parameters: vec![],
                parameter_types: vec![],
                fields: vec![("value".into(), Term::Primitive(P::Bool))],
                requirements: vec![],
                capabilities: vec![],
            }],
            ..Default::default()
        };
        let mut b = a.clone();
        b.structs[0].fields[0].1 = Term::Primitive(P::Type);
        let nominal = Term::Nominal {
            declaration,
            arguments: vec![],
        };
        RuntimeStorageQueries::new(&a)
            .check(&nominal, &[], &mut 1000)
            .unwrap();
        assert!(matches!(
            RuntimeStorageQueries::new(&b).check(&nominal, &[], &mut 1000),
            Err(StorageError::Invalid(_))
        ));
    }
}
