use super::*;
use nir::program::{Function, FunctionPhase, Struct};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 81, local }
}
fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}
fn ty(local: u32) -> Term {
    Term::Nominal {
        declaration: id(local),
        arguments: vec![],
    }
}
fn structure(local: u32, fields: Vec<(&str, Term)>) -> Struct {
    Struct {
        declaration: id(local),
        name: format!("Generated{local}"),
        type_parameters: vec![],
        parameter_types: vec![],
        capabilities: vec![],
        requirements: vec![],
        fields: fields
            .into_iter()
            .map(|(name, ty)| (name.into(), ty))
            .collect(),
    }
}
fn package() -> Package {
    let binding = LocalId { owner: 1, index: 0 };
    Package {
        structs: vec![structure(10, vec![("flag", boolean())])],
        functions: vec![Function {
            declaration: id(1),
            name: "read".into(),
            phase: FunctionPhase::Comptime,
            inline: false,
            pure: false,
            is_unsafe: false,
            type_parameters: vec![],
            requirements: vec![],
            capabilities: vec![],
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            entry_snapshots: vec![],
            parameters: vec![binding],
            defaults: vec![],
            signature: Term::Function {
                parameters: vec![nir::Parameter {
                    sort: nir::Sort::Runtime,
                    ty: Term::Pointer(Box::new(boolean())),
                }],
                result: Box::new(boolean()),
            },
            body: Expr {
                source: SourceRange { start: 1, end: 2 },
                ty: boolean(),
                kind: K::ReadPointer(Box::new(Expr {
                    source: SourceRange { start: 1, end: 2 },
                    ty: Term::Pointer(Box::new(boolean())),
                    kind: K::Local(binding),
                })),
            },
        }],
        ..Default::default()
    }
}

#[test]
fn install_retains_existing_calls_storage_and_events_and_admits_forward_pointer_fields() {
    let package = package();
    let definitions = [
        structure(20, vec![("next", Term::Pointer(Box::new(ty(21))))]),
        structure(
            21,
            vec![
                ("previous", Term::Pointer(Box::new(ty(20)))),
                ("flag", boolean()),
            ],
        ),
    ];
    let later = [structure(22, vec![("entry", ty(20))])];
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let storage = memory::Storage::new(Value::Bool(true));
    let pointer = Value::Pointer(storage.pointer(boolean(), vec![]));
    assert_eq!(
        vm.invoke(id(1), vec![pointer.clone()]).unwrap(),
        Value::Bool(true)
    );
    let events = vm.events().to_vec();
    let before = vm.remaining_steps();
    vm.install_generated_structs(&definitions).unwrap();
    assert!(vm.remaining_steps() < before);
    assert_eq!(vm.events(), events);
    assert_eq!(vm.invoke(id(1), vec![pointer]).unwrap(), Value::Bool(true));
    assert_eq!(storage.read().unwrap(), Value::Bool(true));
    assert_eq!(
        vm.reflect_struct(&ty(20)).unwrap().fields[0].ty,
        Term::Pointer(Box::new(ty(21)))
    );
    assert_eq!(vm.reflect_struct(&ty(10)).unwrap().fields[0].ty, boolean());
    vm.install_generated_structs(&later).unwrap();
    assert_eq!(vm.reflect_struct(&ty(22)).unwrap().fields[0].ty, ty(20));
}

#[test]
fn failed_batches_preserve_the_old_catalog_and_allow_corrected_retry() {
    let package = package();
    let invalid = [
        structure(20, vec![("flag", boolean())]),
        structure(21, vec![("unknown", ty(99))]),
    ];
    let repaired = [
        structure(20, vec![("flag", boolean())]),
        structure(21, vec![("known", ty(20))]),
    ];
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let before = vm.remaining_steps();
    assert!(vm.install_generated_structs(&invalid).is_err());
    assert!(vm.remaining_steps() < before);
    assert!(vm.reflect_struct(&ty(20)).is_err());
    assert!(vm.reflect_struct(&ty(21)).is_err());
    assert!(vm.reflect_struct(&ty(10)).is_ok());
    vm.install_generated_structs(&repaired).unwrap();
    assert!(vm.reflect_struct(&ty(20)).is_ok());
    assert!(vm.reflect_struct(&ty(21)).is_ok());
}

#[test]
fn collisions_open_inputs_and_duplicate_fields_never_replace_published_identity() {
    let package = package();
    let mut generic = structure(20, vec![]);
    generic.type_parameters.push(id(100));
    let batches = [
        vec![structure(10, vec![])],
        vec![structure(1, vec![])],
        vec![structure(20, vec![]), structure(20, vec![])],
        vec![generic],
        vec![structure(
            20,
            vec![("same", boolean()), ("same", boolean())],
        )],
        vec![structure(20, vec![("value", Term::Bool(true))])],
        vec![structure(
            20,
            vec![(
                "open",
                Term::Rigid {
                    declaration: id(100),
                    sort: nir::Sort::Type,
                },
            )],
        )],
    ];
    for batch in &batches {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert!(vm.install_generated_structs(batch).is_err());
        assert_eq!(vm.reflect_struct(&ty(10)).unwrap().fields.len(), 1);
        assert!(vm.reflect_struct(&ty(20)).is_err());
    }
}

#[test]
fn exhausted_installation_and_solver_unwind_keep_the_old_world() {
    let package = package();
    let definitions = [
        structure(20, vec![("flag", boolean())]),
        structure(21, vec![("known", ty(20))]),
    ];
    let mut observed_failure = false;
    let mut observed_success = false;
    for steps in 1..256 {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        vm.budget.steps = steps;
        let success = vm.install_generated_structs(&definitions).is_ok();
        observed_failure |= !success;
        observed_success |= success;
        assert!(vm.remaining_steps() <= steps);
        assert_eq!(vm.structs.contains_key(&id(20)), success);
        assert_eq!(vm.structs.contains_key(&id(21)), success);
        vm.budget.steps = 100_000;
        if !success {
            vm.install_generated_structs(&definitions).unwrap();
        }
        assert!(vm.reflect_struct(&ty(21)).is_ok());
    }
    assert!(observed_failure && observed_success);
    let mut conditional = definitions.clone();
    conditional[1]
        .requirements
        .push(nir::program::TypePredicate::Equal(
            boolean(),
            Term::Primitive(Primitive::Char),
        ));
    let solver =
        |_: &nir::program::TypePredicate| -> ContractOutcome { panic!("host solver aborted") };
    let mut vm = Interpreter::for_comptime(&package, Budget::default())
        .unwrap()
        .with_type_solver(&solver);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        vm.install_generated_structs(&conditional)
    }));
    assert!(result.is_err());
    assert!(!vm.structs.contains_key(&id(20)));
    assert!(!vm.structs.contains_key(&id(21)));
    assert!(vm.structs.contains_key(&id(10)));
    let mut runtime = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        runtime
            .install_generated_structs(&definitions)
            .unwrap_err()
            .kind,
        ErrorKind::PhaseMismatch
    );
}

#[test]
fn empty_structs_still_check_formation_and_large_contract_preflight() {
    let package = package();
    let mut rejected = structure(20, vec![]);
    rejected
        .requirements
        .push(nir::program::TypePredicate::Bool(false));
    let rejected = [rejected];
    let mut large = structure(20, vec![]);
    large.requirements.push(nir::program::TypePredicate::And(
        vec![nir::program::TypePredicate::Bool(true); 2000],
    ));
    let large = [large];
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.install_generated_structs(&rejected).unwrap_err().kind,
        ErrorKind::TypeContractRejected
    );
    assert!(!vm.structs.contains_key(&id(20)));
    vm.budget.steps = 128;
    assert_eq!(
        vm.install_generated_structs(&large).unwrap_err().kind,
        ErrorKind::StepsExhausted
    );
    assert!(!vm.structs.contains_key(&id(20)));
    assert!(vm.structs.contains_key(&id(10)));
}

#[test]
fn a_small_install_does_not_recharge_the_existing_struct_catalog() {
    let added = [structure(10_000, vec![("flag", boolean())])];
    let mut costs = Vec::new();
    for count in [1, 4096] {
        let package = Package {
            structs: (1..=count)
                .map(|local| structure(local, vec![("flag", boolean())]))
                .collect(),
            ..Default::default()
        };
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let before = vm.remaining_steps();
        vm.install_generated_structs(&added).unwrap();
        costs.push(before - vm.remaining_steps());
        assert_eq!(vm.structs.len(), count as usize + 1);
        assert!(vm.reflect_struct(&ty(count)).is_ok());
    }
    assert!(
        costs[0] > 0 && costs[1] <= costs[0] + 32,
        "installation costs: {costs:?}"
    );
}

#[test]
fn repeated_installation_and_late_failure_preserve_prior_batches_with_a_bounded_allowance() {
    let package = Package {
        structs: (1..=4096)
            .map(|local| structure(local, vec![("flag", boolean())]))
            .collect(),
        ..Default::default()
    };
    let additions: Vec<_> = (10_000..10_128)
        .map(|local| structure(local, vec![("flag", boolean())]))
        .collect();
    let failed = [
        structure(20_000, vec![("previous", ty(10_127))]),
        structure(20_001, vec![("unknown", ty(99_999))]),
    ];
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    vm.budget.steps = 10_000;
    for definition in &additions {
        vm.install_generated_structs(std::slice::from_ref(definition))
            .unwrap();
    }
    let count = vm.structs.len();
    assert_eq!(count, 4096 + additions.len());
    assert!(vm.install_generated_structs(&failed).is_err());
    assert_eq!(vm.structs.len(), count);
    assert!(!vm.structs.contains_key(&id(20_000)));
    assert!(!vm.structs.contains_key(&id(20_001)));
    assert!(vm.reflect_struct(&ty(1)).is_ok());
    assert!(vm.reflect_struct(&ty(10_000)).is_ok());
    assert!(vm.reflect_struct(&ty(10_127)).is_ok());
}
