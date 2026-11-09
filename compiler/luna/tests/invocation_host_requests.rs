//! A real source world supplies answers outside the suspended invocation driver.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_interpretor::{
    Budget, CodeSession, Event, Interpreter, InvocationProgress, SourceResolvers, Value,
};
use elaboration::materialization::{DeclarationSource, SourceInvocationQueries};
use std::cell::Cell;

#[test]
fn source_selection_resumes_nested_calls_and_keeps_code_cache_separate_from_answers() {
    source_selection(false);
}

#[test]
fn source_initializers_execute_after_resume_and_do_not_cache_their_results() {
    source_selection(true);
}

fn source_selection(deferred: bool) {
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
    let package = sysroot::compile(&source).unwrap();
    let source = DeclarationSource::Borrowed(&package)
        .retain_declarations(&mut 1_000_000)
        .unwrap();
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
                    let mut active =
                        Box::new(vm.start_invocation(Value::Function(id("check")), vec![])?);
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
                                    assert_eq!(selection.requires_invocation, deferred);
                                    Ok(selection)
                                });
                                active.resume(response).unwrap();
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
            vec![
                nir::Term::Primitive(nir::Primitive::Bool),
                nir::Term::Primitive(nir::Primitive::Char)
            ]
        );
        assert_eq!(fallback_calls.get(), 0);
        // Closed constants were evaluated during elaboration. A generic
        // initializer remains a program and runs on each read, including reads
        // whose implementation was already selected in the current invocation.
        for (name, expected) in [
            ("check", 1),
            ("enabled", 3),
            ("initialize", if deferred { 3 } else { 0 }),
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
