use comptime_interpretor::{Budget, Interpreter, ReflectedField, StructBuildRequest};
use elaboration::definitions::StructPublisher;
use nir::{Primitive, Term, program::SourceRange};
use std::collections::BTreeMap;

fn request(name: &str, ty: Term) -> StructBuildRequest {
    StructBuildRequest {
        name: name.into(),
        fields: vec![ReflectedField {
            name: "value".into(),
            ty,
        }],
        source: SourceRange { start: 1, end: 2 },
    }
}
fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}

#[test]
fn publisher_retains_checked_results_across_chained_dynamic_types() {
    let arena = hir::HirArena::new();
    let publisher = StructPublisher::new(
        &arena,
        hir::Package::new(),
        7,
        BTreeMap::from([(7, 879)]),
        Default::default(),
        None,
        100_000,
    )
    .unwrap();
    let (second_ty, first_ty) = {
        let baseline = nir::program::Package::default();
        let mut vm = Interpreter::for_comptime(&baseline, Budget::default()).unwrap();
        let mut steps = 1_000_000;
        let first = publisher
            .prepare(&request("First", boolean()), &mut steps)
            .unwrap();
        let first_ty = Term::Nominal {
            declaration: first.result,
            arguments: vec![],
        };
        assert_eq!(first.result.package, 879);
        assert_eq!(publisher.published_definition_count(), 0);
        vm.install_owned_generated_structs_with(first.definitions, first.commit)
            .unwrap();
        let second = publisher
            .prepare(&request("Second", first_ty.clone()), &mut steps)
            .unwrap();
        let second_ty = Term::Nominal {
            declaration: second.result,
            arguments: vec![],
        };
        vm.install_owned_generated_structs_with(second.definitions, second.commit)
            .unwrap();
        assert_eq!(
            vm.reflect_struct(&second_ty).unwrap().fields[0].ty,
            first_ty
        );
        assert_eq!(publisher.published_definition_count(), 2);
        assert!(publisher.remaining_structural_fuel() < 100_000);
        assert!(publisher.to_string().contains("(revision 2)"));
        drop(vm);
        (second_ty, first_ty)
    };
    let (hir, nir) = publisher.finish();
    assert_eq!(hir.num_defs(), 2);
    let nir = nir.unwrap();
    assert_eq!(nir.structs.len(), 2);
    let mut fresh = Interpreter::for_comptime(&nir, Budget::default()).unwrap();
    assert_eq!(
        fresh.reflect_struct(&second_ty).unwrap().fields[0].ty,
        first_ty
    );
}

#[test]
fn discarded_and_stale_candidates_never_overwrite_committed_definitions() {
    let arena = hir::HirArena::new();
    let publisher = StructPublisher::new(
        &arena,
        hir::Package::new(),
        0,
        BTreeMap::from([(0, 879)]),
        Default::default(),
        None,
        100_000,
    )
    .unwrap();
    {
        let mut steps = 1_000_000;
        let discarded = publisher
            .prepare(&request("Discarded", boolean()), &mut steps)
            .unwrap();
        drop(discarded);
        assert_eq!(publisher.published_definition_count(), 0);
        let first = publisher
            .prepare(&request("First", boolean()), &mut steps)
            .unwrap();
        let stale = publisher
            .prepare(&request("Stale", boolean()), &mut steps)
            .unwrap();
        (first.commit)().unwrap();
        assert!((stale.commit)().is_err());
        assert!(publisher.last_diagnostic().unwrap().contains("stale"));
        let rejected = publisher.prepare(
            &request(
                "Bad",
                Term::Nominal {
                    declaration: nir::DeclarationId {
                        package: 999,
                        local: 0,
                    },
                    arguments: vec![],
                },
            ),
            &mut steps,
        );
        assert!(rejected.is_err());
        drop(rejected);
        assert_eq!(publisher.published_definition_count(), 1);
        let repaired = publisher
            .prepare(&request("Good", boolean()), &mut steps)
            .unwrap();
        (repaired.commit)().unwrap();
    }
    let (_, checked) = publisher.finish();
    let names: Vec<_> = checked
        .unwrap()
        .structs
        .into_iter()
        .map(|item| item.name)
        .collect();
    assert_eq!(names, vec!["First", "Good"]);
}

#[test]
fn destination_and_structural_budget_fail_without_publication() {
    let arena = hir::HirArena::new();
    assert!(
        StructPublisher::new(
            &arena,
            hir::Package::new(),
            1,
            BTreeMap::new(),
            Default::default(),
            None,
            100
        )
        .is_err()
    );
    let publisher = StructPublisher::new(
        &arena,
        hir::Package::new(),
        0,
        BTreeMap::from([(0, 879)]),
        Default::default(),
        None,
        0,
    )
    .unwrap();
    let mut steps = 1000;
    assert!(
        publisher
            .prepare(&request("Never", boolean()), &mut steps)
            .is_err()
    );
    assert_eq!(publisher.published_definition_count(), 0);
    assert_eq!(steps, 1000);
    assert!(publisher.finish().1.is_none());
}

#[test]
fn deferred_publication_only_initializes_on_request_and_preserves_committed_state() {
    use elaboration::definitions::{DeferredStructPublisher, StructBuildService};
    let arena = hir::HirArena::new();
    let source = hir::Package::new();
    let identities = BTreeMap::from([(0, 879)]);
    let protocols = Default::default();
    let unused = DeferredStructPublisher::new(&arena, &source, 0, &identities, &protocols, None, 0);
    assert!(!unused.is_initialized());
    assert!(unused.to_string().contains("(initialized false)"));
    assert!(unused.finish().is_none());
    let publisher =
        DeferredStructPublisher::new(&arena, &source, 0, &identities, &protocols, None, 100_000);
    assert_eq!(publisher.remaining_structural_fuel(), 100_000);
    {
        let mut steps = 1_000_000;
        let first = publisher
            .prepare(&request("First", boolean()), &mut steps)
            .unwrap();
        assert!(publisher.is_initialized());
        let first_ty = Term::Nominal {
            declaration: first.result,
            arguments: vec![],
        };
        (first.commit)().unwrap();
        let second = publisher
            .prepare(&request("Second", first_ty), &mut steps)
            .unwrap();
        (second.commit)().unwrap();
    }
    assert_eq!(source.num_defs(), 0);
    assert_eq!(publisher.finish().unwrap().structs.len(), 2);
    let rejected =
        DeferredStructPublisher::new(&arena, &source, 0, &identities, &protocols, None, 0);
    assert!(
        rejected
            .prepare(&request("Fail", boolean()), &mut 1000)
            .is_err()
    );
    assert!(!rejected.is_initialized());
    assert!(rejected.last_diagnostic().unwrap().contains("preparation"));
    assert!(rejected.finish().is_none());
}
