//! Storage observations belong to one borrowed, immutable declaration world.
//! A failed budget is not a reusable statement about a type's storage phase.
use super::{Package, Sort, StorageError, Term, validate_runtime_storage_inner};
use std::collections::HashMap;

type Entries = HashMap<Term, HashMap<Vec<Vec<Sort>>, Result<(), StorageError>>>;
#[derive(Clone, Copy)]
enum Phase {
    Runtime,
    Comptime,
}

pub(super) struct StorageQueries<'a> {
    package: &'a Package,
    runtime_entries: Entries,
    comptime_entries: Entries,
}

impl<'a> StorageQueries<'a> {
    pub(super) fn new(package: &'a Package) -> Self {
        Self {
            package,
            runtime_entries: HashMap::new(),
            comptime_entries: HashMap::new(),
        }
    }

    pub(super) fn check_runtime(
        &mut self,
        ty: &Term,
        frames: &[Vec<Sort>],
        fuel: &mut usize,
    ) -> Result<(), StorageError> {
        self.check(Phase::Runtime, ty, frames, fuel)
    }

    pub(super) fn check_comptime(
        &mut self,
        ty: &Term,
        frames: &[Vec<Sort>],
        fuel: &mut usize,
    ) -> Result<(), StorageError> {
        self.check(Phase::Comptime, ty, frames, fuel)
    }

    fn check(
        &mut self,
        phase: Phase,
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
        let entries = match phase {
            Phase::Runtime => &mut self.runtime_entries,
            Phase::Comptime => &mut self.comptime_entries,
        };
        if let Some(result) = entries.get(ty).and_then(|scopes| scopes.get(frames)) {
            return *result;
        }
        let result = match phase {
            Phase::Runtime => validate_runtime_storage_inner(ty, self.package, frames, None, fuel),
            Phase::Comptime => {
                super::comptime_storage::validate_resolved(ty, self.package, frames, None, fuel)
            }
        };
        if !matches!(result, Err(StorageError::Budget)) {
            entries
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
        let mut queries = StorageQueries::new(&package);
        assert_eq!(
            queries.check_runtime(&ty, &[], &mut 2000),
            Err(StorageError::Budget)
        );
        assert!(queries.runtime_entries.is_empty());
        queries.check_runtime(&ty, &[], &mut 100_000).unwrap();
        assert_eq!(queries.runtime_entries.len(), 1);
        // The same expansion is bounded in comptime storage too. A failed
        // query cannot reuse the successful runtime-phase evidence.
        assert_eq!(
            queries.check_comptime(&ty, &[], &mut 2000),
            Err(StorageError::Budget)
        );
        assert!(queries.comptime_entries.is_empty());
        queries.check_comptime(&ty, &[], &mut 100_000).unwrap();
        assert_eq!(queries.comptime_entries.len(), 1);
        assert_eq!(
            queries.check_comptime(&ty, &[], &mut 0),
            Err(StorageError::Budget)
        );
        queries.check_comptime(&ty, &[], &mut 2000).unwrap();
        let mut fuel = 2000;
        queries.check_runtime(&ty, &[], &mut fuel).unwrap();
        assert!(fuel < 2000);
        assert_eq!(
            queries.check_runtime(&ty, &[], &mut 0),
            Err(StorageError::Budget)
        );
        queries.check_runtime(&ty, &[], &mut 2000).unwrap();
    }

    #[test]
    fn frame_and_world_identity_are_part_of_storage_evidence() {
        let package = Package::default();
        let mut queries = StorageQueries::new(&package);
        let bound = Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        };
        queries
            .check_runtime(&bound, &[vec![Sort::Type]], &mut 1000)
            .unwrap();
        assert!(matches!(
            queries.check_runtime(&bound, &[], &mut 1000),
            Err(StorageError::Invalid(_))
        ));
        queries
            .check_runtime(&bound, &[vec![Sort::Type]], &mut 1000)
            .unwrap();
        assert_eq!(queries.runtime_entries[&bound].len(), 2);
        let mut fuel = 16;
        assert_eq!(
            queries.check_runtime(&bound, &[vec![Sort::Type; 100]], &mut fuel),
            Err(StorageError::Budget)
        );
        assert!(fuel > 0 && fuel < 16);
        assert_eq!(queries.runtime_entries[&bound].len(), 2);

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
        StorageQueries::new(&a)
            .check_runtime(&nominal, &[], &mut 1000)
            .unwrap();
        assert!(matches!(
            StorageQueries::new(&b).check_runtime(&nominal, &[], &mut 1000),
            Err(StorageError::Invalid(_))
        ));
    }
    #[test]
    fn storage_phase_and_world_cannot_share_incompatible_evidence() {
        let package = Package::default();
        let mut queries = StorageQueries::new(&package);
        let pointer = Term::Pointer(Box::new(Term::Primitive(P::Bool)));
        queries.check_runtime(&pointer, &[], &mut 1000).unwrap();
        assert!(matches!(
            queries.check_comptime(&pointer, &[], &mut 1000),
            Err(StorageError::Invalid(_))
        ));
        queries
            .check_comptime(&Term::Primitive(P::Type), &[], &mut 1000)
            .unwrap();
        assert!(matches!(
            queries.check_runtime(&Term::Primitive(P::Type), &[], &mut 1000),
            Err(StorageError::Invalid(_))
        ));
        queries.check_runtime(&pointer, &[], &mut 1000).unwrap();
        let mut fuel = 10;
        assert_eq!(
            queries.check_comptime(&pointer, &[vec![Sort::Type; 100]], &mut fuel),
            Err(StorageError::Budget)
        );
        assert!(fuel > 0);
        assert_eq!(queries.comptime_entries[&pointer].len(), 1);

        let declaration = DeclarationId {
            package: 98,
            local: 4,
        };
        let a = Package {
            structs: vec![Struct {
                declaration,
                name: "Payload".into(),
                type_parameters: vec![],
                parameter_types: vec![],
                requirements: vec![],
                capabilities: vec![],
                fields: vec![("data".into(), Term::Primitive(P::Bool))],
            }],
            ..Default::default()
        };
        let mut b = a.clone();
        b.structs[0].fields[0].1 = pointer;
        let nominal = Term::Nominal {
            declaration,
            arguments: vec![],
        };
        StorageQueries::new(&a)
            .check_comptime(&nominal, &[], &mut 1000)
            .unwrap();
        assert!(matches!(
            StorageQueries::new(&b).check_comptime(&nominal, &[], &mut 1000),
            Err(StorageError::Invalid(_))
        ));
    }
}
