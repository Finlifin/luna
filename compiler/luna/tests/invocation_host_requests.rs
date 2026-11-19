//! A real source world supplies answers outside the suspended invocation driver.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_interpretor::{
    Budget, CodeSession, Event, Interpreter, InvocationProgress, SourceResolvers, Value,
};
use elaboration::materialization::{
    DeclarationSnapshot, DeclarationSource, SourceInvocationQueries,
};
use std::cell::Cell;

#[test]
fn source_selection_resumes_nested_calls_and_keeps_code_cache_separate_from_answers() {
    source_selection(false, false);
}

#[test]
fn source_initializers_execute_after_resume_and_do_not_cache_their_results() {
    source_selection(true, false);
}

#[test]
fn source_contracts_suspend_without_entering_the_synchronous_resolver() {
    source_selection(false, true);
}

#[test]
fn source_contracts_invoke_initializers_once_per_read_across_resume() {
    source_selection(true, true);
}

fn source_selection(deferred: bool, contracts: bool) {
    let implementations = if deferred {
        "impl Flag for T where T { assoc enabled: bool = initialize(T); }"
    } else {
        "impl Flag for bool { assoc enabled: bool = initialize(bool); }
         impl Flag for char { assoc enabled: bool = false; }"
    };
    let source = [
        "
        trait Flag { assoc enabled: bool; }
        pure comptime fn initialize(comptime T: Type) -> bool { T == bool }
        ",
        implementations,
        "
        comptime fn enabled(comptime T: Type) -> bool
        where requires T :- Flag
        { T.as(Flag).enabled }
        comptime fn check() -> bool {
            var trace: Integer = 0;
            trace = trace + 1;
            let first = enabled(bool);
            let second = enabled(char);
            first and not second and enabled(bool) and trace == 1
        }
        ",
    ]
    .concat();
    let source = if contracts {
        source.replace(
            "comptime fn enabled(comptime T: Type) -> bool\n        where requires T :- Flag\n        { T.as(Flag).enabled }",
            "comptime fn enabled(comptime T: Type) -> result: bool
             where requires T :- Flag, requires T.as(Flag).enabled,
                   ensures result and T.as(Flag).enabled
             { true }",
        ).replace("enabled(bool)", "true")
         .replace("enabled(char)", "false")
    } else {
        source
    };
    let package = sysroot::compile(&source).unwrap();
    assert!(!package.dispatch_instances.is_empty());
    let source = DeclarationSnapshot::new(std::sync::Arc::new(package));
    let package = source.package();
    let id = |name: &str| {
        package
            .functions
            .iter()
            .find(|function| function.name == name)
            .unwrap()
            .declaration
    };
    let context = elaboration::ptc::Context::from_package(package).unwrap();
    let queries =
        SourceInvocationQueries::in_source(DeclarationSource::Snapshot(&source), &context, &[]);
    let solver = |predicate: &nir::program::TypePredicate, fuel: &mut u64| {
        queries.check(predicate, Default::default(), fuel)
    };
    let methods = |callee: &nir::program::Expr, fuel: &mut u64| queries.resolve(callee, fuel);
    let fallback_calls = Cell::new(0);
    let fallback = |access: &nir::program::Expr, _: &mut u64| {
        fallback_calls.set(fallback_calls.get() + 1);
        Err(comptime_interpretor::Error {
            kind: comptime_interpretor::ErrorKind::UnmaterializedDispatch,
            source: Some(access.source),
        })
    };
    let mut code = CodeSession::new(package.clone());
    let mut cold_compilations = None;
    for _ in 0..2 {
        let mut vm = Interpreter::for_comptime_resolving_in_session(
            package,
            &mut code,
            SourceResolvers {
                types: &solver,
                methods: &methods,
                associated: &fallback,
            },
            &mut Budget {
                steps: 1_000_000,
                depth: 128,
            },
        )
        .unwrap();
        let mut requests = Vec::new();
        let value = vm
            .with_code_session(
                &mut code,
                |vm| -> Result<Value, comptime_interpretor::Error> {
                    let (entry, arguments) = if contracts {
                        (
                            "enabled",
                            vec![Value::Type(nir::Term::Primitive(nir::Primitive::Bool))],
                        )
                    } else {
                        ("check", vec![])
                    };
                    let mut active =
                        Box::new(vm.start_invocation(Value::Function(id(entry)), arguments)?);
                    loop {
                        match active.poll()? {
                            InvocationProgress::Running => {}
                            InvocationProgress::Associated(request) => {
                                assert!(std::ptr::eq(request.source_world(), package.as_ref()));
                                let nir::program::ExprKind::AssociatedValue { subject, .. } =
                                    &request.access().kind
                                else {
                                    panic!("associated access")
                                };
                                requests.push(subject.clone());
                                let state = active.to_string();
                                for _ in 0..3 {
                                    assert!(matches!(active.poll()?, InvocationProgress::Waiting));
                                    assert_eq!(active.remaining_steps(), 0);
                                    assert_eq!(active.to_string(), state);
                                }
                                active = Box::new(*active);
                                let response = request.resolve(|access, fuel| {
                                    let selection = queries.associated(access, fuel)?;
                                    assert_eq!(
                                        matches!(
                                            selection,
                                            comptime_interpretor::AssociatedSelection::Initializer(
                                                _
                                            )
                                        ),
                                        deferred
                                    );
                                    Ok(selection)
                                });
                                active.resume(response).unwrap();
                            }
                            InvocationProgress::Definition(request) => {
                                active
                                    .resume_definition(request.prepare())
                                    .map_err(|failure| failure.error().clone())?;
                            }
                            InvocationProgress::Method(request) => {
                                active
                                    .resume_method(request.prepare())
                                    .map_err(|failure| failure.error().clone())?;
                            }
                            InvocationProgress::Complete(value) => break Ok(value),
                            InvocationProgress::Waiting => {
                                panic!("an answered request must not remain waiting")
                            }
                        }
                    }
                },
            )
            .unwrap();
        assert_eq!(value, Value::Bool(true));
        if let Some(cold) = cold_compilations {
            assert!(
                vm.bytecode_compilations() < cold,
                "the second source VM must reuse compiled bodies"
            );
        } else {
            cold_compilations = Some(vm.bytecode_compilations());
        }
        assert_eq!(
            requests,
            if contracts {
                vec![nir::Term::Primitive(nir::Primitive::Bool)]
            } else {
                vec![
                    nir::Term::Primitive(nir::Primitive::Bool),
                    nir::Term::Primitive(nir::Primitive::Char),
                ]
            }
        );
        assert_eq!(fallback_calls.get(), 0);
        // Closed constants were evaluated during elaboration. A generic
        // initializer remains a program and runs on each read, including reads
        // whose implementation was already selected in the current invocation.
        for (name, expected) in [
            ("check", if contracts { 0 } else { 1 }),
            ("enabled", if contracts { 1 } else { 3 }),
            (
                "initialize",
                if deferred {
                    if contracts { 2 } else { 3 }
                } else {
                    0
                },
            ),
        ] {
            assert_eq!(vm.events().iter().filter(|event| matches!(event, Event::EnterFunction(declaration) if *declaration == id(name))).count(), expected, "{name}");
        }
    }
}

#[test]
fn explicit_comptime_type_arguments_can_use_the_active_callers_storage() {
    assert_eq!(
        sysroot::run(
            "
        trait Flag { assoc enabled: bool; }
        impl Flag for bool { assoc enabled: bool = true; }
        comptime fn enabled(comptime T: Type) -> bool
        where requires T :- Flag
        { T.as(Flag).enabled }
        comptime fn check() -> bool {
            var trace: Integer = 0;
            let result = enabled(do { trace = trace + 1; bool });
            result and trace == 1
        }
        fn run() -> bool { const result = check(); result }
        "
        ),
        Value::Bool(true)
    );
}

#[test]
fn public_builders_suspend_cross_kind_publication_and_reuse_completed_types() {
    use elaboration::definitions::AllocatedDefinitionPublisher;
    let package = sysroot::compile(
        "
        comptime fn Build() -> Type {
            let wrapper = meta.NewtypeBuilder.new(bool, .name = .Flag).Build();
            let record = meta.StructBuilder.new(.name = .Record);
            record.add_field(.flag, wrapper);
            let choice = meta.EnumBuilder.new(.name = .Choice);
            choice.add_tuple(.some, [record.Build()]);
            choice.Build()
        }
    ",
    )
    .unwrap();
    let entry = package
        .functions
        .iter()
        .find(|f| f.name == "Build")
        .unwrap()
        .declaration;
    let context = elaboration::ptc::Context::from_package(&package).unwrap();
    let queries =
        SourceInvocationQueries::in_source(DeclarationSource::Borrowed(&package), &context, &[]);
    let solver = |predicate: &nir::program::TypePredicate, fuel: &mut u64| {
        queries.check(predicate, Default::default(), fuel)
    };
    let publisher =
        AllocatedDefinitionPublisher::new_with_type_solver(&package, 9900, 1, 10_000_000, &solver);
    let structs = |r: &comptime_interpretor::StructBuildRequest, steps: &mut u64| {
        publisher.prepare_struct(r, steps)
    };
    let enums = |r: &comptime_interpretor::EnumBuildRequest, steps: &mut u64| {
        publisher.prepare_enum(r, steps)
    };
    let newtypes = |r: &comptime_interpretor::NewtypeBuildRequest, steps: &mut u64| {
        publisher.prepare_newtype(r, steps)
    };
    let methods = |callee: &nir::program::Expr, fuel: &mut u64| queries.resolve(callee, fuel);
    let associated = |access: &nir::program::Expr, fuel: &mut u64| queries.associated(access, fuel);
    let mut vm = Interpreter::for_comptime_resolving_in_budget(
        &package,
        &[],
        &[],
        &[],
        SourceResolvers {
            types: &solver,
            methods: &methods,
            associated: &associated,
        },
        &mut Budget {
            steps: 10_000_000,
            depth: 128,
        },
    )
    .unwrap()
    .with_struct_builder(&structs)
    .with_enum_builder(&enums)
    .with_newtype_builder(&newtypes);
    let mut results = Vec::new();
    for _ in 0..2 {
        let mut active = vm.start_invocation(Value::Function(entry), vec![]).unwrap();
        let mut kinds = Vec::new();
        loop {
            match active.poll().unwrap() {
                InvocationProgress::Running => {}
                InvocationProgress::Definition(request) => {
                    kinds.push(match request.descriptor() {
                        comptime_interpretor::DefinitionBuildRequest::Struct(_) => "struct",
                        comptime_interpretor::DefinitionBuildRequest::Enum(_) => "enum",
                        comptime_interpretor::DefinitionBuildRequest::Newtype(_) => "newtype",
                    });
                    assert!(matches!(
                        active.poll().unwrap(),
                        InvocationProgress::Waiting
                    ));
                    assert_eq!(active.remaining_steps(), 0);
                    let response = request.prepare();
                    assert!(matches!(
                        active.poll().unwrap(),
                        InvocationProgress::Waiting
                    ));
                    active.resume_definition(response).unwrap();
                }
                InvocationProgress::Complete(value) => {
                    results.push(value);
                    break;
                }
                InvocationProgress::Method(request) => {
                    active.resume_method(request.prepare()).unwrap();
                }
                InvocationProgress::Associated(request) => {
                    active.resume(request.resolve(&associated)).unwrap();
                }
                other => panic!("unexpected host work: {other}"),
            }
        }
        assert_eq!(kinds, ["newtype", "struct", "enum"]);
    }
    assert_ne!(results[0], results[1]);
    assert_eq!(
        vm.events()
            .iter()
            .filter(|e| matches!(e,Event::EnterFunction(id) if *id == entry))
            .count(),
        2
    );
    drop(vm);
    let (published, _) = publisher.finish();
    assert_eq!(published.structs.len(), 2);
    assert_eq!(published.enums.len(), 2);
    assert_eq!(published.newtypes.len(), 2);
}
