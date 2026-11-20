//! Storage observations belong to one borrowed, immutable declaration world.
//! A failed budget is not a reusable statement about a type's storage phase.
use super::{
    Package, Sort, StorageError, Term, storage_domains::Domains,
    validate_runtime_storage_with_domains,
};
use std::collections::HashMap;

type Entries = HashMap<Term, HashMap<usize, Result<(), StorageError>>>;
#[derive(Clone, Copy)]
enum Phase {
    Runtime,
    Comptime,
}

pub(super) struct StorageQueries<'a> {
    package: &'a Package,
    frames: HashMap<Vec<Vec<Sort>>, usize>,
    runtime_entries: Entries,
    comptime_entries: Entries,
    domains: Domains<'a>,
}

impl<'a> StorageQueries<'a> {
    #[cfg(test)]
    pub(super) fn new(package: &'a Package) -> Self {
        Self {
            package,
            frames: HashMap::new(),
            runtime_entries: HashMap::new(),
            comptime_entries: HashMap::new(),
            domains: Domains::new(package),
        }
    }

    pub(super) fn with_declarations(
        package: &'a Package,
        declarations: &'a super::declarations::Declarations,
    ) -> Self {
        Self {
            package,
            frames: HashMap::new(),
            runtime_entries: HashMap::new(),
            comptime_entries: HashMap::new(),
            domains: Domains::admitted(package, declarations),
        }
    }

    /// Intern a complete sort environment once at the callable boundary. The
    /// query handle borrows the frames that were actually admitted for this ID.
    pub(super) fn scope<'s>(
        &'s mut self,
        frames: &'s [Vec<Sort>],
        fuel: &mut usize,
    ) -> Result<StorageScope<'s, 'a>, StorageError> {
        *fuel = fuel
            .checked_sub(frames.len().saturating_add(1))
            .ok_or(StorageError::Budget)?;
        for frame in frames {
            *fuel = fuel.checked_sub(frame.len()).ok_or(StorageError::Budget)?;
        }
        let id = if let Some(id) = self.frames.get(frames) {
            *id
        } else {
            let id = self.frames.len();
            self.frames.insert(frames.to_vec(), id);
            id
        };
        Ok(StorageScope {
            queries: self,
            frames,
            id,
        })
    }
}

pub(super) struct StorageScope<'s, 'a> {
    queries: &'s mut StorageQueries<'a>,
    frames: &'s [Vec<Sort>],
    id: usize,
}

impl StorageScope<'_, '_> {
    pub(super) fn check_runtime(
        &mut self,
        ty: &Term,
        fuel: &mut usize,
    ) -> Result<(), StorageError> {
        self.check(Phase::Runtime, ty, fuel)
    }

    pub(super) fn check_comptime(
        &mut self,
        ty: &Term,
        fuel: &mut usize,
    ) -> Result<(), StorageError> {
        self.check(Phase::Comptime, ty, fuel)
    }

    fn check(&mut self, phase: Phase, ty: &Term, fuel: &mut usize) -> Result<(), StorageError> {
        crate::type_computations::charge_term(ty, fuel)?;
        let entries = match phase {
            Phase::Runtime => &mut self.queries.runtime_entries,
            Phase::Comptime => &mut self.queries.comptime_entries,
        };
        if let Some(result) = entries.get(ty).and_then(|scopes| scopes.get(&self.id)) {
            return *result;
        }
        let result = match phase {
            Phase::Runtime => validate_runtime_storage_with_domains(
                ty,
                self.queries.package,
                self.frames,
                None,
                &self.queries.domains,
                fuel,
            ),
            Phase::Comptime => super::comptime_storage::validate_resolved(
                ty,
                self.queries.package,
                self.frames,
                None,
                fuel,
            ),
        };
        if !matches!(result, Err(StorageError::Budget)) {
            entries
                .entry(ty.clone())
                .or_default()
                .insert(self.id, result);
        }
        result
    }
}

#[cfg(test)]
impl StorageQueries<'_> {
    pub(super) fn check_runtime(
        &mut self,
        ty: &Term,
        frames: &[Vec<Sort>],
        fuel: &mut usize,
    ) -> Result<(), StorageError> {
        self.scope(frames, fuel)?.check_runtime(ty, fuel)
    }

    pub(super) fn check_comptime(
        &mut self,
        ty: &Term,
        frames: &[Vec<Sort>],
        fuel: &mut usize,
    ) -> Result<(), StorageError> {
        self.scope(frames, fuel)?.check_comptime(ty, fuel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DeclarationId, Primitive as P, program::Struct};

    #[test]
    fn a_callable_checks_many_domains_in_the_same_admitted_sort_frame() {
        let package = Package::default();
        let mut queries = StorageQueries::new(&package);
        let mut slots = vec![Sort::Runtime; 512];
        slots[511] = Sort::Type;
        let frames = [slots];
        let bound = Term::Bound {
            depth: 0,
            index: 511,
            sort: Sort::Type,
        };
        let mut scope = queries.scope(&frames, &mut 1000).unwrap();
        let mut fuel = 64;
        for _ in 0..12 {
            scope.check_runtime(&bound, &mut fuel).unwrap();
        }
        let wrong_sort = [vec![Sort::Runtime; 512]];
        let mut other = queries.scope(&wrong_sort, &mut 1000).unwrap();
        assert!(matches!(
            other.check_runtime(&bound, &mut 64),
            Err(StorageError::Invalid(_))
        ));
    }

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
                packed: false,
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
                packed: false,
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
                packed: false,
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
