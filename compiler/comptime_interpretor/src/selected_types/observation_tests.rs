use super::*;
use nir::program::{Function, FunctionPhase, Literal};

const SOURCE: SourceRange = SourceRange { start: 1, end: 2 };
fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 98, local }
}
fn ty(primitive: Primitive) -> Term {
    Term::Primitive(primitive)
}
fn expression(kind: K) -> Expr {
    Expr {
        source: SOURCE,
        ty: ty(Primitive::Type),
        kind,
    }
}
fn projection(member: u32) -> Term {
    Term::AssociatedType {
        origin: Box::new(nir::AssociatedTypeOrigin {
            interface: id(1),
            member: id(member),
        }),
        subject: Box::new(ty(Primitive::Bool)),
        arguments: vec![],
    }
}
fn access(member: u32) -> Expr {
    expression(K::AssociatedValue {
        interface: InterfaceInstance {
            declaration: id(1),
            arguments: vec![],
        },
        subject: ty(Primitive::Bool),
        member: id(member),
    })
}
fn call(declaration: u32) -> Expr {
    expression(K::Call {
        callee: Box::new(Expr {
            source: SOURCE,
            ty: Term::ComptimeFunction(Box::new(Term::Function {
                parameters: vec![],
                result: Box::new(ty(Primitive::Type)),
            })),
            kind: K::Function(id(declaration)),
        }),
        arguments: vec![],
        parameter_bindings: vec![],
    })
}
fn function(local: u32, body: Expr) -> Function {
    Function {
        declaration: id(local),
        name: format!("Domain{local}"),
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
        parameters: vec![],
        defaults: vec![],
        signature: Term::Function {
            parameters: vec![],
            result: Box::new(ty(Primitive::Type)),
        },
        body,
    }
}
fn package() -> Package {
    Package {
        traits: vec![nir::program::Trait {
            declaration: id(1),
            self_parameter: id(2),
            name: "Domains".into(),
            comptime_only: true,
            type_parameters: vec![],
            parameter_types: vec![],
            requirements: vec![],
            capabilities: vec![],
            parents: vec![],
            methods: vec![],
            associated: (3..=6)
                .map(|member| nir::program::AssociatedContract {
                    declaration: id(member),
                    name: format!("Member{member}"),
                    phase: nir::program::TraitPhase::Comptime,
                    ty: ty(Primitive::Type),
                    default: None,
                })
                .collect(),
        }],
        functions: vec![
            function(9, expression(K::TypeValue(ty(Primitive::Bool)))),
            function(
                10,
                expression(K::Block {
                    // This is a distinct source read inside another initializer. An
                    // outer type observation cannot replace it with an observed value.
                    statements: vec![access(3)],
                    result: Some(Box::new(expression(K::TypeValue(ty(Primitive::Char))))),
                }),
            ),
        ],
        ..Default::default()
    }
}
fn select(access: &Expr, fuel: &mut u64) -> Result<AssociatedSelection, Error> {
    let K::AssociatedValue { member, .. } = access.kind else {
        unreachable!()
    };
    let body = match member.local {
        3 => call(9),
        4 => call(10),
        5 => {
            return Err(Error {
                kind: ErrorKind::TypeContractUnresolved,
                source: Some(SOURCE),
            });
        }
        6 => Expr {
            source: SOURCE,
            ty: ty(Primitive::Bool),
            kind: K::Literal(Literal::Bool(true)),
        },
        _ => unreachable!(),
    };
    let mut allowance = usize::try_from(*fuel).unwrap();
    let initializer =
        nir::associated_initializers::AssociatedInitializer::new(body, &mut allowance).unwrap();
    *fuel = allowance as u64;
    Ok(AssociatedSelection {
        initializer,
        requires_invocation: true,
    })
}
fn entries(vm: &Interpreter<'_>, start: usize) -> Vec<DeclarationId> {
    vm.events[start..]
        .iter()
        .filter_map(|event| match event {
            Event::EnterFunction(declaration) => Some(*declaration),
            _ => None,
        })
        .collect()
}

#[test]
fn observation_values_never_shadow_nested_program_reads() {
    let package = package();
    let input = Term::Tuple(vec![projection(3), projection(4), projection(3)]);
    let expected = Term::Tuple(vec![
        ty(Primitive::Bool),
        ty(Primitive::Char),
        ty(Primitive::Bool),
    ]);
    for compiled in [false, true] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .with_associated_resolver(&select);
        vm.bytecode_enabled = compiled;
        for _ in 0..3 {
            let start = vm.events.len();
            assert_eq!(
                vm.selected_type_view(&input, Some(SOURCE)).unwrap(),
                expected
            );
            assert_eq!(
                entries(&vm, start),
                [id(9), id(10), id(9)],
                "the outer observation reuses its own value; a nested source read must execute"
            );
            assert!(vm.associated_values.is_empty());
            assert!(vm.active_associated_initializers.is_empty());
        }
    }
}

#[test]
fn incomplete_observations_leave_no_values_for_a_later_query() {
    let package = package();
    for compiled in [false, true] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default())
            .unwrap()
            .with_associated_resolver(&select);
        vm.bytecode_enabled = compiled;
        for (member, expected) in [(5, ErrorKind::TypeContractUnresolved), (6, ErrorKind::Type)] {
            let start = vm.events.len();
            let input = Term::Tuple(vec![projection(3), projection(member)]);
            assert_eq!(
                vm.selected_type_view(&input, Some(SOURCE))
                    .unwrap_err()
                    .kind,
                expected
            );
            assert_eq!(entries(&vm, start), [id(9)]);
            assert!(vm.associated_values.is_empty());
            assert!(vm.active_associated_initializers.is_empty());
            let start = vm.events.len();
            assert_eq!(
                vm.selected_type_view(&projection(3), Some(SOURCE)).unwrap(),
                ty(Primitive::Bool)
            );
            assert_eq!(entries(&vm, start), [id(9)]);
        }
    }
}

#[test]
fn type_observation_runs_deep_initializers_on_the_driver_stack() {
    let mut package = package();
    package.traits[0]
        .associated
        .extend((100..196).map(|member| nir::program::AssociatedContract {
            declaration: id(member),
            name: format!("Link{member}"),
            phase: nir::program::TraitPhase::Comptime,
            ty: ty(Primitive::Type),
            default: None,
        }));
    let resolver = |request: &Expr, fuel: &mut u64| {
        let K::AssociatedValue { member, .. } = request.kind else {
            unreachable!()
        };
        let body = if member.local == 195 {
            call(9)
        } else {
            access(member.local + 1)
        };
        let mut allowance = usize::try_from(*fuel).unwrap();
        let initializer =
            nir::associated_initializers::AssociatedInitializer::new(body, &mut allowance).unwrap();
        *fuel = allowance as u64;
        Ok(AssociatedSelection {
            initializer,
            requires_invocation: true,
        })
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default())
        .unwrap()
        .with_associated_resolver(&resolver);
    vm.bytecode_enabled = false;
    vm.budget.depth = 8;
    assert_eq!(
        vm.selected_type_view(&projection(100), Some(SOURCE))
            .unwrap_err()
            .kind,
        ErrorKind::DepthExhausted
    );
    assert!(vm.active_associated_initializers.is_empty());
    assert_eq!(vm.depth, 0);
    vm.bytecode_enabled = true;
    for _ in 0..3 {
        let start = vm.events.len();
        assert_eq!(
            vm.selected_type_view(&projection(100), Some(SOURCE))
                .unwrap(),
            ty(Primitive::Bool)
        );
        assert_eq!(entries(&vm, start), [id(9)]);
        assert!(vm.associated_values.is_empty());
        assert!(vm.active_associated_initializers.is_empty());
        assert_eq!(vm.depth, 0);
    }
    vm.budget.steps = 100;
    assert_eq!(
        vm.selected_type_view(&projection(100), Some(SOURCE))
            .unwrap_err()
            .kind,
        ErrorKind::StepsExhausted
    );
    assert!(vm.active_associated_initializers.is_empty());
    assert_eq!(vm.depth, 0);
    vm.budget.steps = Budget::default().steps;
    assert_eq!(
        vm.selected_type_view(&projection(100), Some(SOURCE))
            .unwrap(),
        ty(Primitive::Bool)
    );
}

fn selected_package() -> Package {
    let mut package = package();
    package.traits[0]
        .associated
        .retain(|member| member.declaration.local == 3 || member.declaration.local == 4);
    let initializers: Vec<_> = [(3, 9), (4, 10)]
        .into_iter()
        .map(|(member, function)| {
            (
                id(member),
                nir::associated_initializers::AssociatedInitializer::new(call(function), &mut 1000)
                    .unwrap(),
            )
        })
        .collect();
    package.implementations.push(nir::program::Implementation {
        declaration: id(20),
        phases: nir::program::ImplementationPhases {
            runtime: false,
            comptime: true,
        },
        type_parameters: vec![],
        parameter_types: vec![],
        associated: vec![],
        associated_initializers: initializers.clone(),
        interface: Some(id(1)),
        interface_arguments: vec![],
        capabilities: vec![],
        requirements: vec![],
        subject: ty(Primitive::Bool),
        methods: vec![],
    });
    package
        .dispatch_instances
        .push(nir::program::DispatchInstance {
            interface: id(1),
            interface_arguments: vec![],
            subject: ty(Primitive::Bool),
            implementation: id(20),
            arguments: vec![],
            methods: vec![],
            associated: vec![],
            associated_initializers: initializers,
        });
    package
}

#[test]
fn selected_initializers_need_execution_permission_not_a_host_resolver() {
    let package = Arc::new(selected_package());
    let input = Term::Tuple(vec![projection(3), projection(4), projection(3)]);
    let expected = Term::Tuple(vec![
        ty(Primitive::Bool),
        ty(Primitive::Char),
        ty(Primitive::Bool),
    ]);
    for compiled in [false, true] {
        let mut session = CodeSession::new(package.clone());
        for _ in 0..2 {
            let mut vm = Interpreter::for_comptime_in_session(
                &package,
                &mut session,
                &mut Budget::default(),
            )
            .unwrap();
            vm.bytecode_enabled = compiled;
            assert!(vm.associated_resolver.is_none());
            for _ in 0..2 {
                let start = vm.events.len();
                assert_eq!(
                    vm.selected_type_view(&input, Some(SOURCE)).unwrap(),
                    expected
                );
                assert_eq!(entries(&vm, start), [id(9), id(10), id(9)]);
                assert!(vm.associated_values.is_empty());
                assert!(vm.active_associated_initializers.is_empty());
            }
            vm.budget.steps = 0;
            assert_eq!(
                vm.selected_type_view(&input, Some(SOURCE))
                    .unwrap_err()
                    .kind,
                ErrorKind::StepsExhausted
            );
            vm.budget.steps = Budget::default().steps;
            assert_eq!(
                vm.selected_type_view(&projection(5), Some(SOURCE))
                    .unwrap_err()
                    .kind,
                ErrorKind::UnmaterializedDispatch
            );
            assert_eq!(
                vm.selected_type_view(&projection(3), Some(SOURCE)).unwrap(),
                ty(Primitive::Bool)
            );
        }
    }
}

#[test]
fn observation_cannot_use_a_resolver_to_escape_runtime_or_graph_authority() {
    use std::cell::Cell;
    let package = selected_package();
    let calls = Cell::new(0);
    let resolver = |access: &Expr, fuel: &mut u64| {
        calls.set(calls.get() + 1);
        select(access, fuel)
    };
    let mut runtime = Interpreter::new(&package, Budget::default())
        .unwrap()
        .with_associated_resolver(&resolver);
    assert_eq!(
        runtime
            .selected_type_view(&projection(3), Some(SOURCE))
            .unwrap_err()
            .kind,
        ErrorKind::UnmaterializedDispatch
    );
    assert!(runtime.events.is_empty());
    assert!(runtime.active_associated_initializers.is_empty());
    let graph = iir::Package::default();
    for comptime in [false, true] {
        let mut vm = if comptime {
            Interpreter::from_iir_for_comptime(&graph, Budget::default()).unwrap()
        } else {
            Interpreter::from_iir(&graph, Budget::default()).unwrap()
        }
        .with_associated_resolver(&resolver);
        assert_eq!(
            vm.selected_type_view(&projection(3), Some(SOURCE))
                .unwrap_err()
                .kind,
            ErrorKind::UnmaterializedDispatch
        );
        assert!(vm.events.is_empty());
    }
    assert_eq!(calls.get(), 0);
}
