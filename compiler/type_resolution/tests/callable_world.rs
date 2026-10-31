//! Callable evidence is indexed per immutable declaration world, not memoized invocation.
use nir::program::{Expr, ExprKind, Function, FunctionPhase, LocalId, Package, SourceRange};
use nir::{DeclarationId, Primitive, Sort, Term};
use type_resolution::dispatch::DeclarationWorld;
use type_resolution::ptc::Budget;

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 11, local }
}
fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}
fn callable(domain: Term) -> Term {
    Term::Function {
        parameters: vec![nir::Parameter {
            sort: Sort::Runtime,
            ty: domain.clone(),
        }],
        result: Box::new(domain),
    }
}
fn function(local: u32) -> Function {
    let binding = LocalId {
        owner: local,
        index: 0,
    };
    Function {
        declaration: id(local),
        name: format!("f{local}"),
        phase: FunctionPhase::Comptime,
        pure: false,
        inline: false,
        is_unsafe: false,
        type_parameters: vec![],
        parameters: vec![binding],
        signature: callable(boolean()),
        requirements: vec![],
        capabilities: vec![],
        defaults: vec![],
        preconditions: vec![],
        postconditions: vec![],
        result_binding: None,
        entry_snapshots: vec![],
        body: Expr {
            source: SourceRange { start: 0, end: 1 },
            ty: boolean(),
            kind: ExprKind::Local(binding),
        },
    }
}
fn package(noise: u32) -> Package {
    let mut generic = function(1);
    generic.type_parameters = vec![id(10)];
    let t = Term::Rigid {
        declaration: id(10),
        sort: Sort::Type,
    };
    generic.signature = callable(t.clone());
    generic.body.ty = t;
    let mut functions = vec![generic];
    functions.extend((1000..1000 + noise).map(function));
    Package {
        functions,
        runtime_function_identities: vec![nir::program::RuntimeFunctionIdentity {
            declaration: id(2),
            type_parameters: vec![],
            signature: callable(boolean()),
        }],
        normal_forms: vec![nir::normal_forms::Head {
            declaration: id(3),
            collector: None,
            role: nir::normal_forms::Role::Opaque,
            parameters: vec![nir::normal_forms::Parameter {
                declaration: id(30),
                binding: LocalId { owner: 3, index: 0 },
                kind: nir::normal_forms::Kind::Type,
                ty: Term::Primitive(Primitive::Type),
            }],
            requirements: vec![],
        }],
        ..Default::default()
    }
}

#[test]
fn evidence_lookup_work_does_not_scale_with_unrelated_declarations() {
    let mut baseline = None;
    for noise in [0, 100, 2000] {
        let package = package(noise);
        let (world, remaining) = DeclarationWorld::new(
            &package,
            Budget {
                steps: 1_000_000,
                ..Budget::default()
            },
            None,
        )
        .unwrap();
        assert!(remaining < 1_000_000);
        let mut observed = vec![];
        for (declaration, arguments) in [(id(1), vec![boolean()]), (id(2), vec![]), (id(3), vec![])]
        {
            let mut fuel = 100;
            let signature = world
                .callable_signature(declaration, &arguments, &[], &mut fuel)
                .unwrap();
            signature.check_scope(&[]).unwrap();
            observed.push((signature, 100 - fuel));
        }
        assert_eq!(observed[0].0, callable(boolean()).with_comptime_phase(true));
        assert_eq!(observed[1].0, callable(boolean()));
        assert!(observed[2].0.is_normal_form_callable());
        if let Some(baseline) = &baseline {
            assert_eq!(&observed, baseline);
        } else {
            baseline = Some(observed);
        }
        assert!(
            world
                .callable_signature(id(9999), &[], &[], &mut 10)
                .is_none()
        );
        assert!(
            world
                .callable_signature(id(1), &[], &[], &mut 100)
                .is_none()
        );
        assert!(
            world
                .callable_signature(id(1), &[boolean()], &[], &mut 1)
                .is_none()
        );
        assert!(
            world
                .callable_signature(id(1), &[boolean()], &[], &mut 100)
                .is_some()
        );
    }
}

#[test]
fn worlds_keep_signature_and_input_evidence_isolated() {
    let first = package(0);
    let mut second = package(0);
    second.functions[0].is_unsafe = true;
    let (a, _) = DeclarationWorld::new(&first, Budget::default(), None).unwrap();
    let (b, _) = DeclarationWorld::new(&second, Budget::default(), None).unwrap();
    let left = a
        .callable_signature(id(1), &[boolean()], &[], &mut 100)
        .unwrap();
    let right = b
        .callable_signature(id(1), &[boolean()], &[], &mut 100)
        .unwrap();
    assert!(!left.is_unsafe_callable());
    assert!(right.is_unsafe_callable());
    assert!(
        a.callable_signature(id(1), &[Term::Bool(true)], &[], &mut 100)
            .is_none()
    );
    assert_eq!(
        a.callable_signature(id(1), &[boolean()], &[], &mut 100),
        Some(left)
    );
}

#[test]
fn ambiguous_runtime_metadata_does_not_publish_an_index() {
    let mut package = package(0);
    package
        .runtime_function_identities
        .push(package.runtime_function_identities[0].clone());
    assert!(DeclarationWorld::new(&package, Budget::default(), None).is_err());
    package.runtime_function_identities.pop();
    assert!(DeclarationWorld::new(&package, Budget::default(), None).is_ok());
    assert!(
        DeclarationWorld::new(
            &package,
            Budget {
                steps: 0,
                ..Budget::default()
            },
            None
        )
        .is_err()
    );
}
