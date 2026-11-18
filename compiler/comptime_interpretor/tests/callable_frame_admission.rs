//! Raw VM registration and phase inspection share body-free input frame rules.
use comptime_interpretor::{Budget, ErrorKind, Interpreter, Value};
use nir::{DeclarationId, Parameter, Primitive, Sort, Term, program::*};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 71, local }
}
fn binding(index: u32) -> LocalId {
    LocalId { owner: 1, index }
}
fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}
fn source() -> SourceRange {
    SourceRange { start: 10, end: 20 }
}
fn local(index: u32) -> Expr {
    Expr {
        source: source(),
        ty: boolean(),
        kind: ExprKind::Local(binding(index)),
    }
}
fn function() -> Function {
    Function {
        interface: FunctionDeclaration {
            declaration: id(1),
            name: "choose".into(),
            phase: FunctionPhase::Comptime,
            inline: false,
            pure: false,
            is_unsafe: false,
            signature: Term::Function {
                parameters: vec![
                    Parameter {
                        sort: Sort::Runtime,
                        ty: boolean()
                    };
                    2
                ],
                result: Box::new(boolean()),
            },
            parameters: vec![binding(0), binding(1)],
            defaults: vec![(1, local(0))],
            preconditions: vec![],
            postconditions: vec![],
            entry_snapshots: vec![],
            result_binding: None,
            type_parameters: vec![],
            requirements: vec![],
            capabilities: vec![],
        },
        body: local(1),
    }
}
fn package() -> Package {
    Package {
        functions: vec![function()],
        ..Default::default()
    }
}

fn reject(package: &Package) {
    let error = match Interpreter::new(package, Budget::default()) {
        Ok(_) => panic!("malformed callable was registered"),
        Err(error) => error,
    };
    assert_eq!(error.kind, ErrorKind::InvalidCallPlan);
    let inspection = nir::phases::Inspection::new(package);
    assert!(inspection.check_package(&mut 100_000).is_err());
    assert!(
        inspection.callables_for(package).is_none(),
        "failed input admission published a reusable directory fact"
    );
}

#[test]
fn raw_registration_rejects_invalid_frames_before_invocation() {
    for case in 0..6 {
        let mut package = package();
        let function = &mut package.functions[0];
        match case {
            0 => {
                function.interface.parameters.pop();
            }
            1 => function.interface.parameters[1] = binding(0),
            2 => function.interface.defaults[0].0 = 2,
            3 => function.interface.defaults.push((1, local(0))),
            4 => function.interface.defaults[0].1 = local(1),
            _ => function.interface.defaults[0].1 = local(99),
        }
        reject(&package);
    }
    let package = package();
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert!(vm.events().is_empty(), "registration executed user code");
    assert_eq!(
        vm.invoke(id(1), vec![Value::Bool(false), Value::Bool(true)])
            .unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn quote_metadata_uses_the_same_frame_validation_without_becoming_executable() {
    let mut quote = function();
    quote.interface.phase = FunctionPhase::Runtime;
    let mut package = Package {
        quote_templates: vec![quote],
        ..Default::default()
    };
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert!(
        vm.invoke(id(1), vec![Value::Bool(false), Value::Bool(true)])
            .is_err()
    );
    package.quote_templates[0].interface.defaults[0].1 = local(1);
    reject(&package);
}

#[test]
fn bodyless_trait_contracts_cannot_alias_or_omit_input_bindings() {
    let function = function();
    let mut package = Package {
        traits: vec![Trait {
            declaration: id(2),
            self_parameter: id(3),
            name: "Read".into(),
            comptime_only: false,
            type_parameters: vec![],
            parameter_types: vec![],
            requirements: vec![],
            capabilities: vec![],
            parents: vec![],
            associated: vec![],
            methods: vec![MethodContract {
                declaration: id(4),
                name: "read".into(),
                phase: Default::default(),
                is_quote: false,
                pure: false,
                is_unsafe: false,
                type_parameters: vec![],
                type_arguments: vec![],
                signature: function.signature.clone().into(),
                parameter_bindings: function.parameters.clone(),
                capabilities: vec![],
                requirements: vec![],
                by_value_receiver: false,
                by_reference_receiver: false,
                default: None,
                program: Default::default(),
            }],
        }],
        ..Default::default()
    };
    Interpreter::new(&package, Budget::default()).unwrap();
    nir::phases::Inspection::new(&package)
        .check_package(&mut 100_000)
        .unwrap();
    package.traits[0].methods[0].parameter_bindings[1] = binding(0);
    reject(&package);
    package.traits[0].methods[0].parameter_bindings.pop();
    reject(&package);
}

#[test]
fn frame_registration_budget_is_independent_of_body_size_and_retry_is_fresh() {
    let package = package();
    let allowance = Budget {
        steps: 100_000,
        ..Budget::default()
    };
    let vm = Interpreter::new(&package, allowance).unwrap();
    let cost = allowance.steps - vm.remaining_steps();
    assert!(cost > 0);
    for steps in 0..cost {
        let error = match Interpreter::new(&package, Budget { steps, ..allowance }) {
            Ok(_) => panic!("registration bypassed its budget at {steps}/{cost}"),
            Err(error) => error,
        };
        assert_eq!(
            error.kind,
            ErrorKind::StepsExhausted,
            "steps={steps}/{cost}: {error:?}"
        );
    }
    let mut vm = Interpreter::new(&package, allowance).unwrap();
    assert_eq!(
        vm.invoke(id(1), vec![Value::Bool(false), Value::Bool(true)])
            .unwrap(),
        Value::Bool(true)
    );
    let mut large = package.clone();
    large.functions[0].body.kind = ExprKind::Block {
        statements: vec![
            Expr {
                source: source(),
                ty: boolean(),
                kind: ExprKind::Literal(Literal::Bool(false))
            };
            10_000
        ],
        result: Some(Box::new(local(1))),
    };
    let vm = Interpreter::new(&large, allowance).unwrap();
    assert_eq!(
        allowance.steps - vm.remaining_steps(),
        cost,
        "input frame admission traversed an executable body"
    );
}

#[test]
fn a_source_identity_cannot_denote_two_different_function_bodies() {
    let mut package = package();
    let mut other = function();
    other.interface.name = "other".into();
    other.body = local(0);
    package.functions.push(other);
    reject(&package);
}
