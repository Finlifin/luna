//! Compilation is budgeted work even when no executable code can be published.
use super::*;

fn body() -> Expr {
    Expr {
        source: SourceRange { start: 1, end: 2 },
        ty: Term::Primitive(Primitive::Bool),
        kind: K::Literal(Literal::Bool(true)),
    }
}

#[test]
fn compilation_charges_actual_work_and_does_not_publish_on_exhaustion() {
    let package = Package::default();
    let body = body();
    let work = comptime_compiler::compile(&body, &mut 1000)
        .unwrap()
        .storage_work();
    assert!(work > 0);
    for steps in 0..=work as u64 {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        vm.budget.steps = steps;
        let mut reference_fuel = steps as usize;
        let _ = comptime_compiler::compile(&body, &mut reference_fuel);
        let spent = steps - reference_fuel as u64;
        let result = vm.compile_body(&body);
        if steps < work as u64 {
            assert_eq!(result.unwrap_err().kind, ErrorKind::StepsExhausted);
            assert!(vm.bytecode_cache.entries.is_empty());
        } else {
            assert!(result.unwrap().is_some());
        }
        assert_eq!(vm.budget.steps, reference_fuel as u64);
        assert_eq!(vm.bytecode_compilation_steps(), spent);
        assert_eq!(vm.bytecode_invocations(), 0);
        vm.budget.steps = 1000;
        assert!(vm.compile_body(&body).unwrap().is_some());
        assert_eq!(vm.budget.steps, 1000 - work as u64);
        assert_eq!(vm.bytecode_compilation_steps(), spent + work as u64);
    }
}

#[test]
fn unsupported_and_invalid_compilation_probes_keep_their_work_charges() {
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let invalid = Expr {
        kind: K::Break(LocalId { owner: 1, index: 0 }),
        ..body()
    };
    let before = vm.budget.steps;
    assert_eq!(
        vm.compile_body(&invalid).unwrap_err().kind,
        ErrorKind::InvalidControlFlow
    );
    assert!(vm.budget.steps < before);
    // A nested executable pattern is unsupported by the bytecode compiler.
    let unsupported = Expr {
        kind: K::Match {
            value: Box::new(body()),
            arms: vec![nir::program::MatchArm {
                pattern: nir::program::Pattern::Then {
                    first: Box::new(nir::program::Pattern::Wild),
                    value: Box::new(body()),
                    next: Box::new(nir::program::Pattern::Wild),
                },
                guard: None,
                body: body(),
            }],
        },
        ..body()
    };
    let before = vm.budget.steps;
    assert!(vm.compile_body(&unsupported).unwrap().is_none());
    assert!(vm.budget.steps < before);
    assert_eq!(vm.bytecode_invocations(), 0);
    let unsupported = Arc::new(unsupported);
    assert!(vm.owned_invocation_code(&unsupported).unwrap().is_none());
    let compiled = vm.bytecode_compilations();
    let steps = vm.bytecode_compilation_steps();
    assert!(vm.owned_invocation_code(&unsupported).unwrap().is_none());
    assert_eq!(vm.bytecode_compilations(), compiled);
    assert_eq!(vm.bytecode_compilation_steps(), steps);
}

#[test]
fn stable_code_cache_hits_do_not_repeat_compilation_charges() {
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let body = body();
    let code = Arc::new(vm.compile_body(&body).unwrap().unwrap());
    let declaration = DeclarationId {
        package: 1,
        local: 1,
    };
    vm.bytecode_cache.insert(declaration, Some(code.clone()));
    let before = vm.budget.steps;
    for _ in 0..3 {
        let cached = vm
            .invocation_code(&body, Some(declaration))
            .unwrap()
            .unwrap();
        assert!(Arc::ptr_eq(&code, &cached));
        assert_eq!(vm.budget.steps, before);
    }
    assert_eq!(vm.bytecode_compilations(), 1);
}

#[test]
fn owned_code_cache_tracks_live_identity_and_does_not_retain_source_bodies() {
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let original = Arc::new(body());
    let weak = Arc::downgrade(&original);
    let first = vm.owned_invocation_code(&original).unwrap().unwrap();
    let reused = vm.owned_invocation_code(&original).unwrap().unwrap();
    assert!(Arc::ptr_eq(&first, &reused));
    assert_eq!(vm.bytecode_compilations(), 1);
    let mut changed = original.clone();
    Arc::make_mut(&mut changed).kind = K::Literal(Literal::Bool(false));
    let second = vm.owned_invocation_code(&changed).unwrap().unwrap();
    assert!(!Arc::ptr_eq(&first, &second));
    assert_eq!(vm.bytecode_compilations(), 2);
    assert!(matches!(
        vm.execute_code(&first, &mut Environment::new()).unwrap(),
        Flow::Value(Value::Bool(true))
    ));
    assert!(matches!(
        vm.execute_code(&second, &mut Environment::new()).unwrap(),
        Flow::Value(Value::Bool(false))
    ));
    drop(original);
    assert!(weak.upgrade().is_none());
    assert!(vm.owned_invocation_code(&changed).unwrap().is_some());
    assert_eq!(vm.bytecode_compilations(), 2);
}

#[test]
fn owned_code_cache_does_not_publish_exhausted_compilations() {
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let source = Arc::new(body());
    vm.budget.steps = 0;
    assert_eq!(
        vm.owned_invocation_code(&source).unwrap_err().kind,
        ErrorKind::StepsExhausted
    );
    assert!(vm.bytecode_cache.owned.is_empty());
    vm.budget.steps = 10000;
    assert!(vm.owned_invocation_code(&source).unwrap().is_some());
    let attempts = vm.bytecode_compilations();
    assert!(vm.owned_invocation_code(&source).unwrap().is_some());
    assert_eq!(vm.bytecode_compilations(), attempts);
}

#[test]
fn owned_and_declaration_code_share_a_bounded_cache() {
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let source = Arc::new(Expr {
        kind: K::Block {
            statements: vec![body(); 1024],
            result: Some(Box::new(body())),
        },
        ..body()
    });
    let code = Arc::new(vm.compile_body(&source).unwrap().unwrap());
    let declaration = DeclarationId {
        package: 91,
        local: 1,
    };
    vm.bytecode_cache.insert(declaration, Some(code.clone()));
    vm.bytecode_cache.insert_owned(&source, Some(code.clone()));
    let weight = vm.bytecode_cache.weight;
    vm.bytecode_cache.insert_owned(&source, Some(code.clone()));
    vm.bytecode_cache.insert(declaration, Some(code.clone()));
    assert_eq!(vm.bytecode_cache.weight, weight);
    let mut sources = vec![source];
    for _ in 0..100 {
        let next = Arc::new(sources[0].as_ref().clone());
        vm.bytecode_cache.insert_owned(&next, Some(code.clone()));
        sources.push(next);
        assert!(vm.bytecode_cache.weight <= 100_000);
        if vm.bytecode_cache.entries.is_empty() {
            break;
        }
    }
    assert!(vm.bytecode_cache.entries.is_empty());
    assert!(
        vm.bytecode_cache
            .owned(sources.last().unwrap())
            .unwrap()
            .is_some()
    );
    assert!(vm.bytecode_cache.owned(&sources[0]).is_none());
}
