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
