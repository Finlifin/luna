use super::*;
const SOURCE: SourceRange = SourceRange { start: 10, end: 20 };
fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 96, local }
}
fn wide(width: usize) -> Term {
    Term::Record(vec![("x".repeat(width), Term::Primitive(Primitive::Bool))])
}

#[test]
fn closed_copy_preserves_limits_without_granting_scope_or_export_authority() {
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let value = Value::Type(wide(512));
    vm.budget.steps = 100;
    let error = vm.copy_representation(&value, SOURCE, 0).unwrap_err();
    assert_eq!(error.kind, ErrorKind::StepsExhausted);
    assert_eq!(error.source, Some(SOURCE));
    assert!(vm.remaining_steps() > 0 && vm.remaining_steps() < 100);
    vm.budget.steps = 10_000;
    assert_eq!(vm.copy_representation(&value, SOURCE, 0).unwrap(), value);
    vm.budget.steps = 1_000_000;
    assert_eq!(
        vm.copy_representation(&Value::Type(wide(100_001)), SOURCE, 0)
            .unwrap_err()
            .kind,
        ErrorKind::RepresentationSizeLimit
    );

    vm.authorize_abstract_type_parameters(&[id(1)]).unwrap();
    let scoped = Value::Type(Term::Rigid {
        declaration: id(1),
        sort: nir::Sort::Type,
    });
    assert_eq!(
        vm.checkpoint_representation(&scoped, SOURCE).unwrap(),
        scoped
    );
    assert_eq!(
        vm.copy_representation(&scoped, SOURCE, 0).unwrap_err().kind,
        ErrorKind::Type
    );
    vm.validate_immutable_input(&scoped, &Term::Primitive(Primitive::Type))
        .unwrap();
    assert!(vm.events.is_empty());
}

#[test]
fn copied_instance_inputs_share_fuel_and_accept_ordinary_scoped_values() {
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let boolean = Term::Primitive(Primitive::Bool);
    vm.authorize_abstract_value_parameters(&[(id(1), boolean)])
        .unwrap();
    let arguments = vec![
        Term::Bool(true),
        Term::Rigid {
            declaration: id(1),
            sort: nir::Sort::ComptimeValue,
        },
        wide(512),
    ];
    vm.budget.steps = 100;
    assert_eq!(
        vm.copy_instance_arguments(&arguments, SOURCE)
            .unwrap_err()
            .kind,
        ErrorKind::StepsExhausted
    );
    assert!(vm.remaining_steps() > 0 && vm.remaining_steps() < 95);
    vm.budget.steps = 10_000;
    assert_eq!(
        vm.copy_instance_arguments(&arguments, SOURCE).unwrap(),
        arguments
    );
    assert_eq!(
        vm.check_closed_term_description(&arguments[1], Some(SOURCE))
            .unwrap_err()
            .kind,
        ErrorKind::Type
    );
    vm.check_closed_term_description(&arguments[0], Some(SOURCE))
        .unwrap();
    assert_eq!(
        vm.check_closed_type_description(&arguments[0], Some(SOURCE))
            .unwrap_err()
            .kind,
        ErrorKind::Type
    );
    assert!(vm.events.is_empty());
}

#[test]
fn cleanup_effect_index_preserves_typed_failures_and_publishes_only_a_complete_map() {
    use nir::effects::{Abi, Operation};
    let operation = |n, argument| Operation {
        declaration: id(n),
        arguments: vec![argument],
    };
    let small = operation(1, Term::Bool(true));
    let large = operation(2, wide(512));
    let boolean = Term::Primitive(Primitive::Bool);
    let request_signature = Term::Function {
        parameters: vec![],
        result: Box::new(boolean.clone()),
    };
    let request = |operation: Operation, allowed_index| Expr {
        source: SOURCE,
        ty: boolean.clone(),
        kind: K::EffectRequest {
            operation,
            abi: Abi::Direct,
            allowed_index,
            signature: Box::new(request_signature.clone()),
            arguments: vec![],
        },
    };
    let result = Term::Computation {
        effects: vec![small.clone(), large.clone()],
        result: Box::new(boolean.clone()),
    };
    let function = nir::program::Function {
        interface: nir::program::FunctionDeclaration {
            source: SOURCE,
            declaration: id(3),
            name: "inspect".into(),
            phase: nir::program::FunctionPhase::Runtime,
            inline: false,
            pure: false,
            is_unsafe: false,
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            result_type_binder: None,
            result_guarantees: vec![],
            entry_snapshots: vec![],
            capabilities: vec![],
            requirements: vec![],
            type_parameters: vec![],
            parameters: vec![],
            env_parameters: vec![],
            defaults: vec![],
            signature: Term::Function {
                parameters: vec![],
                result: Box::new(result.clone()),
            },
        },
        runtime_deliveries: Default::default(),
        body: Expr {
            source: SOURCE,
            ty: result,
            kind: K::EffectComputation {
                effects: vec![small.clone(), large.clone()],
                body: Box::new(Expr {
                    source: SOURCE,
                    ty: boolean.clone(),
                    kind: K::Block {
                        statements: vec![request(small.clone(), 0)],
                        result: Some(Box::new(request(large.clone(), 1))),
                    },
                }),
            },
        },
    };
    let graph = iir::Package {
        functions: vec![iir::FunctionInstance {
            key: iir::InstanceKey {
                declaration: id(3),
                arguments: vec![].into(),
            },
            function,
        }],
        ..Default::default()
    };
    let package = Package::default();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    vm.graph = Some(&graph);
    let retained = operation(9, Term::Bool(false));
    vm.cleanup_effect_context.insert(retained.clone(), true);
    let before = vm.cleanup_effect_context.clone();
    for allowance in [0, 100] {
        vm.budget.steps = allowance;
        assert_eq!(
            vm.index_cleanup_effect_context().unwrap_err().kind,
            ErrorKind::StepsExhausted
        );
        assert_eq!(vm.cleanup_effect_context, before);
        if allowance != 0 {
            assert!(vm.remaining_steps() > 0 && vm.remaining_steps() < allowance);
        }
    }
    vm.budget.steps = 10_000;
    vm.index_cleanup_effect_context().unwrap();
    assert_eq!(vm.cleanup_effect_context.len(), 2);
    assert_eq!(vm.cleanup_effect_context.get(&small), Some(&false));
    assert_eq!(vm.cleanup_effect_context.get(&large), Some(&false));
    assert!(!vm.cleanup_effect_context.contains_key(&retained));
    assert!(vm.events.is_empty());
}
