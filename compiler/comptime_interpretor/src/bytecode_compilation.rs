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
            result.unwrap();
        }
        assert_eq!(vm.budget.steps, reference_fuel as u64);
        assert_eq!(vm.bytecode_compilation_steps(), spent);
        assert_eq!(vm.bytecode_invocations(), 0);
        vm.budget.steps = 1000;
        vm.compile_body(&body).unwrap();
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
    // Definition-only evidence is not executable bytecode.
    let evidence = Expr {
        kind: K::Match {
            value: Box::new(body()),
            arms: vec![nir::program::MatchArm {
                pattern: nir::program::Pattern::CodeEvidence { outputs: vec![] },
                guard: None,
                body: body(),
            }],
        },
        ..body()
    };
    let before = vm.budget.steps;
    assert_eq!(
        vm.compile_body(&evidence).unwrap_err().kind,
        ErrorKind::TypeContractUnresolved
    );
    assert!(vm.budget.steps < before);
    assert_eq!(vm.bytecode_invocations(), 0);
    // Rejected lowering is an error, not a request to select another engine.
    let unsupported = Arc::new(Expr {
        kind: K::EffectResume(Box::new(body())),
        ..body()
    });
    for _ in 0..2 {
        let before = vm.budget.steps;
        let failure = vm.owned_invocation_code(&unsupported).unwrap_err();
        assert_eq!(failure.kind, ErrorKind::Unsupported);
        assert_eq!(failure.source, Some(unsupported.source));
        assert!(vm.budget.steps < before);
        assert!(vm.bytecode_cache.owned.is_empty());
    }
    assert_eq!(vm.bytecode_compilations(), 4);
    assert_eq!(vm.bytecode_invocations(), 0);
    let valid = Arc::new(body());
    let code = vm.owned_invocation_code(&valid).unwrap().unwrap();
    assert!(matches!(
        vm.execute_code(&code, &mut Environment::new()).unwrap(),
        Flow::Value(Value::Bool(true))
    ));
}

#[test]
fn stable_code_cache_hits_do_not_repeat_compilation_charges() {
    let package = Package::default();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let body = body();
    let code = Arc::new(vm.compile_body(&body).unwrap());
    let declaration = CodeOrigin::Declaration(DeclarationId {
        package: 1,
        local: 1,
    });
    vm.bytecode_cache.insert(declaration, code.clone());
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
    let code = Arc::new(vm.compile_body(&source).unwrap());
    let declaration = CodeOrigin::Declaration(DeclarationId {
        package: 91,
        local: 1,
    });
    vm.bytecode_cache.insert(declaration, code.clone());
    vm.bytecode_cache.insert_owned(&source, code.clone());
    let weight = vm.bytecode_cache.weight;
    vm.bytecode_cache.insert_owned(&source, code.clone());
    vm.bytecode_cache.insert(declaration, code.clone());
    assert_eq!(vm.bytecode_cache.weight, weight);
    let mut sources = vec![source];
    for _ in 0..100 {
        let next = Arc::new(sources[0].as_ref().clone());
        vm.bytecode_cache.insert_owned(&next, code.clone());
        sources.push(next);
        assert!(vm.bytecode_cache.weight <= 100_000);
        if vm.bytecode_cache.entries.is_empty() {
            break;
        }
    }
    assert!(vm.bytecode_cache.entries.is_empty());
    assert!(vm.bytecode_cache.owned(sources.last().unwrap()).is_some());
    assert!(vm.bytecode_cache.owned(&sources[0]).is_none());
}

#[test]
fn typed_constants_preserve_formation_admission_and_diagnostics() {
    let package = Package::default();
    let cases = [
        (
            Primitive::U8,
            K::Literal(Literal::PendingInteger {
                spelling: "255".into(),
                negative: false,
            }),
        ),
        (
            Primitive::U8,
            K::Literal(Literal::PendingInteger {
                spelling: "256".into(),
                negative: false,
            }),
        ),
        (
            Primitive::F32,
            K::Literal(Literal::PendingFloat {
                spelling: "1.25".into(),
                negative: true,
            }),
        ),
        (
            Primitive::Bool,
            K::Literal(Literal::String("wrong domain".into())),
        ),
        (Primitive::Str, K::Literal(Literal::String("雪\n\"".into()))),
        (Primitive::Bool, K::ComptimeValue(Term::Bool(true))),
        (Primitive::U8, K::ComptimeValue(Term::Bool(true))),
        (
            Primitive::Type,
            K::TypeValue(Term::Primitive(Primitive::U8)),
        ),
        (
            Primitive::Bool,
            K::TypeValue(Term::Primitive(Primitive::U8)),
        ),
    ];
    for (domain, kind) in cases {
        let body = Expr {
            ty: Term::Primitive(domain),
            kind,
            ..body()
        };
        let code = Arc::new(comptime_compiler::compile(&body, &mut 10_000).unwrap());
        let mut reference = Interpreter::for_comptime_tree(&package, Budget::default()).unwrap();
        let mut compiled = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let expected = reference.eval(&body, &mut Environment::new());
        let actual = compiled.execute_code(&code, &mut Environment::new());
        match (expected, actual) {
            (Ok(Flow::Value(expected)), Ok(Flow::Value(actual))) => {
                assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
            }
            (Err(expected), Err(actual)) => {
                assert_eq!(actual.kind, expected.kind);
                assert_eq!(actual.source, expected.source);
            }
            values => panic!("constant execution diverged: {values:?}"),
        }
    }
}

#[test]
fn independent_compilations_share_constants_only_in_the_matching_code_session() {
    fn constant(program: &Program) -> &Arc<comptime_compiler::constants::Constant> {
        program
            .instructions()
            .iter()
            .find_map(|instruction| {
                if let Op::Constant { constant, .. } = &instruction.op {
                    Some(&program.constants()[constant.0])
                } else {
                    None
                }
            })
            .unwrap()
    }
    let package = Arc::new(Package::default());
    let mut session = CodeSession::new(package.clone());
    let first = {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        vm.with_code_session(&mut session, |vm| vm.compile_body(&body()).unwrap())
    };
    let mut other_body = body();
    other_body.source = SourceRange { start: 5, end: 7 };
    let second = {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        vm.with_code_session(&mut session, |vm| vm.compile_body(&other_body).unwrap())
    };
    assert!(Arc::ptr_eq(constant(&first), constant(&second)));
    let different_world = Package::default();
    let mut vm = Interpreter::for_comptime(&different_world, Budget::default()).unwrap();
    let isolated = vm.with_code_session(&mut session, |vm| vm.compile_body(&body()).unwrap());
    assert!(!Arc::ptr_eq(constant(&first), constant(&isolated)));
    let weak = Arc::downgrade(constant(&first));
    drop(first);
    drop(second);
    assert!(
        weak.upgrade().is_none(),
        "the session must not own constant payloads"
    );
}

#[test]
fn staged_values_are_not_executable_frozen_constants() {
    let package = Package::default();
    let expression = Expr {
        kind: K::StagedValue(Term::Bound {
            depth: 0,
            index: 0,
            sort: nir::Sort::ComptimeValue,
        }),
        ..body()
    };
    assert!(
        matches!(comptime_compiler::compile(&expression, &mut 1000), Err(comptime_compiler::Error::Unsupported(source)) if source == expression.source)
    );
    let mut tree = Interpreter::for_comptime_tree(&package, Budget::default()).unwrap();
    assert_eq!(
        tree.eval(&expression, &mut Environment::new())
            .unwrap_err()
            .kind,
        ErrorKind::PhaseMismatch
    );
    let mut completed = expression;
    completed
        .rewrite_types(&mut |term| term.substitute(&[Term::Bool(true)]))
        .unwrap();
    let code = Arc::new(comptime_compiler::compile(&completed, &mut 1000).unwrap());
    let mut compiled = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert!(matches!(
        compiled
            .execute_code(&code, &mut Environment::new())
            .unwrap(),
        Flow::Value(Value::Bool(true))
    ));
}

#[test]
fn rejected_input_lowering_never_starts_execution_or_publishes_code() {
    let package = Package::default();
    let trace = LocalId { owner: 1, index: 0 };
    let unsupported_source = SourceRange { start: 10, end: 20 };
    // Deliberately incomplete IR probes the engine boundary. This is not a
    // legal source program: staged values must be completed before execution.
    let expression = Expr {
        kind: K::Block {
            statements: vec![Expr {
                ty: Term::Primitive(Primitive::Void),
                kind: K::Assign {
                    place: nir::program::Place {
                        root: trace,
                        fields: vec![],
                    },
                    value: Box::new(Expr {
                        kind: K::Literal(Literal::Bool(false)),
                        ..body()
                    }),
                },
                ..body()
            }],
            result: Some(Box::new(Expr {
                source: unsupported_source,
                kind: K::StagedValue(Term::Bound {
                    depth: 0,
                    index: 0,
                    sort: nir::Sort::ComptimeValue,
                }),
                ..body()
            })),
        },
        ..body()
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut environment = Environment::from([(trace, Value::Bool(true))]);
    let failure = match vm.start_expression_input(&expression, &mut environment) {
        Err(error) => error,
        Ok(_) => panic!("rejected bytecode must not select tree execution"),
    };
    assert_eq!(failure.kind, ErrorKind::Unsupported);
    assert_eq!(failure.source, Some(unsupported_source));
    assert_eq!(environment.get(&trace), Some(&Value::Bool(true)));
    let declaration = CodeOrigin::Declaration(DeclarationId {
        package: 1,
        local: 1,
    });
    for _ in 0..2 {
        assert_eq!(
            vm.invocation_code(&expression, Some(declaration))
                .unwrap_err()
                .kind,
            ErrorKind::Unsupported
        );
        assert_eq!(
            vm.borrowed_invocation_code(&expression).unwrap_err().kind,
            ErrorKind::Unsupported
        );
    }
    assert!(vm.bytecode_cache.entries.is_empty());
    assert!(vm.borrowed_code_cache.entries.is_empty());
    let valid = body();
    let frame = vm
        .start_expression_input(&valid, &mut environment)
        .unwrap()
        .unwrap();
    let result = frame.run(&mut vm);
    assert!(matches!(result.flow, Ok(Flow::Value(Value::Bool(true)))));
    assert_eq!(result.environment.get(&trace), Some(&Value::Bool(true)));

    // Explicit reference execution remains available; it is not selected by
    // a failed compilation and therefore consumes no bytecode compiler work.
    let mut reference = Interpreter::for_comptime_tree(&package, Budget::default()).unwrap();
    assert!(
        reference
            .start_expression_input(&valid, &mut environment)
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        reference.eval(&valid, &mut environment).unwrap(),
        Flow::Value(Value::Bool(true))
    ));
    assert_eq!(reference.bytecode_compilations(), 0);
}
