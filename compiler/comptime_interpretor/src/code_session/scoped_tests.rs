//! Metadata and code may persist; each invocation must earn its own authority.
use super::*;
use nir::program::{Function, FunctionPhase, Trait, TraitRequirement};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 91, local }
}
fn flag() -> Expr {
    Expr {
        source: SourceRange { start: 10, end: 20 },
        ty: Term::Primitive(Primitive::Bool),
        kind: K::Literal(Literal::Bool(true)),
    }
}
fn interface(local: u32) -> Trait {
    Trait {
        declaration: id(local),
        self_parameter: id(local + 10),
        name: format!("Protocol{local}"),
        comptime_only: false,
        type_parameters: vec![],
        parameter_types: vec![],
        requirements: vec![],
        capabilities: vec![],
        parents: vec![],
        methods: vec![],
        associated: vec![],
    }
}
fn premise(local: u32) -> TraitRequirement {
    TraitRequirement {
        interface: id(local),
        interface_arguments: vec![],
        subject: Term::Primitive(Primitive::Bool),
    }
}
fn source() -> Arc<Package> {
    let base = interface(1);
    let mut derived = interface(2);
    derived.parents.push(InterfaceInstance {
        declaration: base.declaration,
        arguments: vec![],
    });
    Arc::new(Package {
        traits: vec![base, derived],
        functions: vec![Function {
            interface: nir::program::FunctionDeclaration {
                declaration: id(3),
                name: "read".into(),
                phase: FunctionPhase::Comptime,
                inline: false,
                pure: false,
                is_unsafe: false,
                signature: Term::Function {
                    parameters: vec![],
                    result: Box::new(flag().ty),
                },
                parameters: vec![],
                type_parameters: vec![],
                defaults: vec![],
                preconditions: vec![],
                postconditions: vec![],
                result_binding: None,
                entry_snapshots: vec![],
                requirements: vec![],
                capabilities: vec![premise(1)],
            },
            body: flag(),
        }],
        normal_forms: vec![nir::normal_forms::Head {
            declaration: id(4),
            role: Default::default(),
            collector: None,
            parameters: vec![],
            requirements: vec![nir::program::TypePredicate::Bool(true); 2000],
        }],
        ..Default::default()
    })
}
fn unavailable<T>(_: &Expr, _: &mut u64) -> Result<T, Error> {
    Err(Error {
        kind: ErrorKind::UnmaterializedDispatch,
        source: None,
    })
}

#[test]
fn source_forks_share_only_completed_admission_and_keep_code_and_permissions_local() {
    check_source_forks(false);
    check_source_forks(true);
}

fn check_source_forks(phase_checked: bool) {
    let package = source();
    let phase = nir::phases::Inspection::new(&package)
        .check_package_with_evidence(&mut 100_000)
        .unwrap();
    let mut parent = if phase_checked {
        CodeSession::with_phase_checked_source(package.clone(), &phase).unwrap()
    } else {
        CodeSession::new(package.clone())
    };
    let mut sibling = parent.fork_for_source(package.clone());
    let solver = |_: &nir::program::TypePredicate, _: &mut u64| Ok(ContractOutcome::Unknown);
    let services = || SourceResolvers {
        types: &solver,
        methods: &unavailable,
        associated: &unavailable,
    };
    assert!(
        Interpreter::for_comptime_resolving_in_scope_in_session(
            &package,
            &mut parent,
            &[premise(999)],
            &[],
            &[],
            services(),
            &mut Budget::default()
        )
        .is_err()
    );
    assert!(sibling.resolving_indexes.get().is_none());
    let mut vm = Interpreter::for_comptime_resolving_in_scope_in_session(
        &package,
        &mut parent,
        &[premise(2)],
        &[],
        &[],
        services(),
        &mut Budget::default(),
    )
    .unwrap();
    let first = vm
        .with_code_session(&mut parent, |vm| vm.invoke(id(3), vec![]))
        .unwrap();
    assert_eq!(first, Value::Bool(true));
    assert!(vm.bytecode_compilations() > 0);
    assert!(Arc::ptr_eq(
        parent.resolving_indexes.get().unwrap(),
        sibling.resolving_indexes.get().unwrap()
    ));
    assert!(
        sibling.indexes.get().is_none(),
        "closed and resolving modes stay separate"
    );

    let mut without = Interpreter::for_comptime_resolving_in_session(
        &package,
        &mut sibling,
        services(),
        &mut Budget::default(),
    )
    .unwrap();
    assert_eq!(
        without.invoke(id(3), vec![]).unwrap_err().kind,
        ErrorKind::TypeContractUnresolved
    );
    let mut authorized = Interpreter::for_comptime_resolving_in_scope_in_session(
        &package,
        &mut sibling,
        &[premise(2)],
        &[],
        &[],
        services(),
        &mut Budget::default(),
    )
    .unwrap();
    assert_eq!(
        authorized
            .with_code_session(&mut sibling, |vm| vm.invoke(id(3), vec![]))
            .unwrap(),
        Value::Bool(true)
    );
    assert!(
        authorized.bytecode_compilations() > 0,
        "the sibling did not inherit compiled code"
    );

    let copied = Arc::new(package.as_ref().clone());
    let foreign = parent.fork_for_source(copied);
    assert!(foreign.indexes.get().is_none());
    assert!(foreign.resolving_indexes.get().is_none());
    drop(parent);
    drop(vm);
    drop(without);
    drop(authorized);
    let survivor = Interpreter::for_comptime_resolving_in_session(
        &package,
        &mut sibling,
        services(),
        &mut Budget::default(),
    )
    .unwrap();
    assert!(survivor.remaining_steps() > 0);
}

#[test]
fn direct_source_entries_check_closed_capabilities_without_a_formation_receipt() {
    let package = source();
    let mode = std::cell::Cell::new(ContractOutcome::Unknown);
    let solver = |_: &nir::program::TypePredicate, _: &mut u64| Ok(mode.get());
    let services = || SourceResolvers {
        types: &solver,
        methods: &unavailable,
        associated: &unavailable,
    };
    for compiled in [false, true] {
        let mut vm = Interpreter::for_comptime_resolving_in_budget(
            &package,
            &[],
            &[],
            &[],
            services(),
            &mut Budget::default(),
        )
        .unwrap();
        vm.bytecode_enabled = compiled;
        for route in 0..3 {
            for outcome in [
                ContractOutcome::Unknown,
                ContractOutcome::Satisfied,
                ContractOutcome::Rejected,
            ] {
                mode.set(outcome);
                let result = match route {
                    0 => vm.invoke(id(3), vec![]),
                    1 => vm.invoke_instantiated(id(3), vec![], vec![]),
                    _ => vm.invoke_comptime_expression(&Expr {
                        kind: K::Call {
                            callee: Box::new(Expr {
                                ty: package.functions[0].callable_type(),
                                kind: K::Function(id(3)),
                                ..flag()
                            }),
                            arguments: vec![],
                            parameter_bindings: vec![],
                        },
                        ..flag()
                    }),
                };
                match outcome {
                    ContractOutcome::Satisfied => assert_eq!(result.unwrap(), Value::Bool(true)),
                    ContractOutcome::Unknown => {
                        assert_eq!(result.unwrap_err().kind, ErrorKind::TypeContractUnresolved)
                    }
                    ContractOutcome::Rejected => {
                        assert_eq!(result.unwrap_err().kind, ErrorKind::TypeContractRejected)
                    }
                }
                assert_eq!(vm.depth, 0);
                assert!(vm.prepared_frames.is_empty());
            }
        }
    }
}

#[test]
fn warm_metadata_and_code_cannot_retain_premises_or_host_answers() {
    let package = source();
    let mut session = CodeSession::new(package.clone());
    let mut cold_cost = 0;
    for (index, supplied) in [true, false, true, false].into_iter().enumerate() {
        let calls = std::cell::Cell::new(0);
        let solver = |_: &nir::program::TypePredicate, _: &mut u64| {
            calls.set(calls.get() + 1);
            Ok(ContractOutcome::Unknown)
        };
        let assumptions = if supplied { vec![premise(2)] } else { vec![] };
        let mut budget = Budget::default();
        let mut vm = Interpreter::for_comptime_resolving_in_scope_in_session(
            &package,
            &mut session,
            &assumptions,
            &[],
            &[],
            SourceResolvers {
                types: &solver,
                methods: &unavailable,
                associated: &unavailable,
            },
            &mut budget,
        )
        .unwrap();
        let cost = Budget::default().steps - budget.steps;
        if index == 0 {
            cold_cost = cost;
        } else {
            assert!(cost < cold_cost);
        }
        assert_eq!(
            calls.get(),
            0,
            "construction does not invoke host proof services"
        );
        let result = vm.with_code_session(&mut session, |vm| vm.invoke(id(3), vec![]));
        if supplied {
            assert_eq!(result.unwrap(), Value::Bool(true));
            assert_eq!(calls.get(), 0);
            assert!(
                vm.assumed_interfaces
                    .contains(&(premise(1).interface_instance(), premise(1).subject))
            );
            assert_eq!(vm.bytecode_compilations() > 0, index == 0);
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::TypeContractUnresolved);
            assert!(
                calls.get() > 0,
                "each failed demand must reach this VM's service"
            );
            assert!(vm.assumed_interfaces.is_empty());
        }
        assert!(vm.method_bodies.is_empty());
    }
}

#[test]
fn abstract_authority_is_fresh_and_failed_construction_cannot_publish_it() {
    let package = source();
    let mut session = CodeSession::new(package.clone());
    let solver = |_: &nir::program::TypePredicate, _: &mut u64| Ok(ContractOutcome::Unknown);
    let services = || SourceResolvers {
        types: &solver,
        methods: &unavailable,
        associated: &unavailable,
    };
    let rigid = Term::Rigid {
        declaration: id(90),
        sort: nir::Sort::Type,
    };
    let expression = Expr {
        ty: Term::Primitive(Primitive::Type),
        kind: K::TypeValue(rigid.clone()),
        ..flag()
    };
    for grant in [true, false, true] {
        let parameters = if grant { vec![id(90)] } else { vec![] };
        let mut vm = Interpreter::for_comptime_resolving_in_scope_in_session(
            &package,
            &mut session,
            &[],
            &parameters,
            &[],
            services(),
            &mut Budget::default(),
        )
        .unwrap();
        let result = vm.with_code_session(&mut session, |vm| {
            vm.invoke_comptime_expression(&expression)
        });
        if grant {
            assert_eq!(result.unwrap(), Value::TypeParameter(id(90)));
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::Type);
        }
    }
    // Both cold and warm constructors reject conflicting type/value authority.
    for warm in [false, true] {
        let mut cold = CodeSession::new(package.clone());
        let selected = if warm { &mut session } else { &mut cold };
        let mut budget = Budget::default();
        assert_eq!(
            Interpreter::for_comptime_resolving_in_scope_in_session(
                &package,
                selected,
                &[],
                &[id(90)],
                &[(id(90), Term::Primitive(Primitive::Bool))],
                services(),
                &mut budget,
            )
            .err()
            .unwrap()
            .kind,
            ErrorKind::Type
        );
        assert_eq!(budget.steps, 0);
        assert_eq!(selected.resolving_indexes.get().is_some(), warm);
        let vm = Interpreter::for_comptime_resolving_in_session(
            &package,
            selected,
            services(),
            &mut Budget::default(),
        )
        .unwrap();
        assert!(vm.abstract_type_parameters.is_empty());
        assert!(vm.abstract_value_parameters.is_empty());
    }
}

#[test]
fn scoped_session_rechecks_world_identity_invalid_premises_and_budget() {
    let package = source();
    let mut session = CodeSession::new(package.clone());
    let solver = |_: &nir::program::TypePredicate, _: &mut u64| Ok(ContractOutcome::Unknown);
    let services = || SourceResolvers {
        types: &solver,
        methods: &unavailable,
        associated: &unavailable,
    };
    Interpreter::for_comptime_resolving_in_session(
        &package,
        &mut session,
        services(),
        &mut Budget::default(),
    )
    .unwrap();
    let mut budget = Budget::default();
    Interpreter::for_comptime_resolving_in_scope_in_session(
        &package,
        &mut session,
        &[premise(2)],
        &[],
        &[],
        services(),
        &mut budget,
    )
    .unwrap();
    let cost = Budget::default().steps - budget.steps;
    for steps in 0..cost {
        let mut budget = Budget {
            steps,
            ..Default::default()
        };
        assert_eq!(
            Interpreter::for_comptime_resolving_in_scope_in_session(
                &package,
                &mut session,
                &[premise(2)],
                &[],
                &[],
                services(),
                &mut budget,
            )
            .err()
            .unwrap()
            .kind,
            ErrorKind::StepsExhausted
        );
        assert_eq!(budget.steps, 0);
    }
    for invalid in [
        premise(999),
        TraitRequirement {
            interface_arguments: vec![Term::Primitive(Primitive::Bool)],
            ..premise(2)
        },
    ] {
        assert_eq!(
            Interpreter::for_comptime_resolving_in_scope_in_session(
                &package,
                &mut session,
                &[invalid],
                &[],
                &[],
                services(),
                &mut Budget::default(),
            )
            .err()
            .unwrap()
            .kind,
            ErrorKind::InvalidCallPlan
        );
    }
    let copied = package.as_ref().clone();
    assert_eq!(
        Interpreter::for_comptime_resolving_in_scope_in_session(
            &copied,
            &mut session,
            &[premise(2)],
            &[],
            &[],
            services(),
            &mut Budget {
                steps: cost,
                ..Default::default()
            },
        )
        .err()
        .unwrap()
        .kind,
        ErrorKind::StepsExhausted
    );
    let mut changed = copied;
    changed.functions[0].defaults.push((99, flag()));
    assert!(matches!(
        Interpreter::for_comptime_resolving_in_scope_in_session(
            &changed,
            &mut session,
            &[premise(2)],
            &[],
            &[],
            services(),
            &mut Budget::default(),
        )
        .err()
        .unwrap()
        .kind,
        ErrorKind::InvalidPhase(_)
    ));
    let mut recovered = Interpreter::for_comptime_resolving_in_scope_in_session(
        &package,
        &mut session,
        &[premise(2)],
        &[],
        &[],
        services(),
        &mut Budget::default(),
    )
    .unwrap();
    assert_eq!(
        recovered
            .with_code_session(&mut session, |vm| vm.invoke(id(3), vec![]))
            .unwrap(),
        Value::Bool(true)
    );
}
