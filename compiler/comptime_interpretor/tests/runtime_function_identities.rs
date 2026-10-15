use comptime_interpretor::{Budget, ErrorKind, Interpreter, Value};
use nir::program::{
    Expr, ExprKind as K, Function, FunctionPhase, Package, RuntimeFunctionIdentity, SourceRange,
};
use nir::{DeclarationId, Primitive, Sort, Term};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 1, local }
}
fn signature(result: Term) -> Term {
    Term::Function {
        parameters: vec![],
        result: Box::new(result),
    }
}
fn package() -> Package {
    let callable = signature(Term::Primitive(Primitive::Bool));
    Package {
        runtime_function_identities: vec![RuntimeFunctionIdentity {
            declaration: id(2),
            type_parameters: vec![id(3)],
            signature: signature(Term::Rigid {
                declaration: id(3),
                sort: Sort::Type,
            }),
        }],
        functions: vec![Function {
            declaration: id(1),
            name: "obtain".into(),
            phase: FunctionPhase::Comptime,
            inline: false,
            pure: false,
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            entry_snapshots: vec![],
            capabilities: vec![],
            requirements: vec![],
            type_parameters: vec![],
            signature: signature(callable.clone()),
            parameters: vec![],
            defaults: vec![],
            body: Expr {
                source: SourceRange { start: 1, end: 2 },
                ty: callable,
                kind: K::FunctionInstance {
                    declaration: id(2),
                    arguments: vec![Term::Primitive(Primitive::Bool)],
                },
            },
        }],
        ..Default::default()
    }
}

#[test]
fn runtime_identity_is_typed_transport_without_a_body() {
    let package = package();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let value = vm.invoke(id(1), vec![]).unwrap();
    assert_eq!(
        value,
        Value::RuntimeFunction {
            declaration: id(2),
            arguments: vec![Term::Primitive(Primitive::Bool)]
        }
    );
    assert!(format!("{package}").contains("(runtime-function-identity"));
    assert!(format!("{value}").starts_with("(runtime-function "));
    assert_eq!(
        vm.invoke(id(2), vec![]).unwrap_err().kind,
        ErrorKind::PhaseMismatch
    );
}

#[test]
fn malformed_identity_catalogs_and_instances_are_rejected() {
    let mut duplicate = package();
    duplicate
        .runtime_function_identities
        .push(duplicate.runtime_function_identities[0].clone());
    assert!(nir::phases::validate(&duplicate).is_err());
    assert!(Interpreter::new(&duplicate, Budget::default()).is_err());
    let mut collision = package();
    collision.runtime_function_identities[0].declaration = id(1);
    assert!(Interpreter::for_comptime(&collision, Budget::default()).is_err());
    let mut wrong_arity = package();
    if let K::FunctionInstance { arguments, .. } = &mut wrong_arity.functions[0].body.kind {
        arguments.clear();
    }
    let mut vm = Interpreter::for_comptime(&wrong_arity, Budget::default()).unwrap();
    assert_eq!(vm.invoke(id(1), vec![]).unwrap_err().kind, ErrorKind::Type);
}

#[test]
fn an_identity_cannot_be_called_even_without_the_comptime_entry_gate() {
    let mut package = package();
    let callee = package.functions[0].body.clone();
    package.functions[0].signature = signature(Term::Primitive(Primitive::Bool));
    package.functions[0].body = Expr {
        source: callee.source,
        ty: Term::Primitive(Primitive::Bool),
        kind: K::Call {
            callee: Box::new(callee),
            arguments: vec![],
            parameter_bindings: vec![],
        },
    };
    assert!(Interpreter::for_comptime(&package, Budget::default()).is_err());
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(1), vec![]).unwrap_err().kind,
        ErrorKind::PhaseMismatch
    );
}

fn trait_package() -> Package {
    let mut package = package();
    package.traits.push(nir::program::Trait {
        parameter_types: Vec::new(),
        associated: vec![],
        requirements: vec![],
        capabilities: vec![],
        type_parameters: vec![],
        declaration: id(4),
        parents: vec![],
        self_parameter: id(4),
        name: "Factory".into(),
        methods: vec![nir::program::MethodContract {
            is_quote: false,
            pure: false,
            type_arguments: vec![],
            type_parameters: vec![id(3)],
            capabilities: vec![],
            program: Default::default(),
            parameter_bindings: vec![],
            by_value_receiver: false,
            by_reference_receiver: false,
            declaration: id(5),
            name: "make".into(),
            signature: package.runtime_function_identities[0].signature.clone(),
            requirements: vec![],
            default: None,
        }],
    });
    package.implementations.push(nir::program::Implementation {
        associated: vec![],
        interface_arguments: vec![],
        capabilities: vec![],
        type_parameters: vec![],
        requirements: vec![],
        declaration: id(6),
        subject: Term::Primitive(Primitive::Bool),
        interface: Some(id(4)),
        methods: vec![("make".into(), id(2))],
    });
    package.functions[0].body.kind = K::TraitMethod {
        interface: id(4),
        interface_arguments: vec![],
        subject: Term::Primitive(Primitive::Bool),
        member: id(5),
        method_arguments: vec![Term::Primitive(Primitive::Bool)],
    };
    package
}

#[test]
fn body_free_trait_method_retains_the_interface_origin() {
    let package = trait_package();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let value = vm.invoke(id(1), vec![]).unwrap();
    assert_eq!(
        value,
        Value::RuntimeTraitMethod(Box::new(comptime_interpretor::TraitMethodIdentity {
            interface: id(4).into(),
            subject: Term::Primitive(Primitive::Bool),
            member: id(5),
            method_arguments: vec![Term::Primitive(Primitive::Bool)],
        }))
    );
    assert!(format!("{value}").starts_with("(runtime-trait-method "));
}

#[test]
fn body_free_trait_method_cannot_execute_or_erase_a_precondition() {
    let mut package = trait_package();
    let callee = package.functions[0].body.clone();
    package.functions[0].signature = signature(Term::Primitive(Primitive::Bool));
    package.functions[0].body = Expr {
        source: callee.source,
        ty: Term::Primitive(Primitive::Bool),
        kind: K::Call {
            callee: Box::new(callee),
            arguments: vec![],
            parameter_bindings: vec![],
        },
    };
    assert!(Interpreter::for_comptime(&package, Budget::default()).is_err());
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(1), vec![]).unwrap_err().kind,
        ErrorKind::PhaseMismatch
    );
    let mut package = trait_package();
    package.traits[0].methods[0]
        .program
        .preconditions
        .push(Expr {
            source: SourceRange { start: 1, end: 2 },
            ty: Term::Primitive(Primitive::Bool),
            kind: K::Literal(nir::program::Literal::Bool(true)),
        });
    assert!(Interpreter::for_comptime(&package, Budget::default()).is_err());
    let mut vm = Interpreter::new(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(id(1), vec![]).unwrap_err().kind,
        ErrorKind::InvalidCallPlan
    );
}
