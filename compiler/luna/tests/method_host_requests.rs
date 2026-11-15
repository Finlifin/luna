//! Method choice leaves the source VM after checking its prefix, and resumes
//! before evaluating ordinary arguments or entering the chosen implementation.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_interpretor::{
    Budget, CodeSession, ErrorKind, Event, Interpreter, InvocationProgress as Progress,
    InvocationSession, MethodHostRequest, SourceResolvers, Value,
};
use elaboration::materialization::{
    DeclarationSnapshot, DeclarationSource, SourceInvocationQueries,
};
use nir::{Primitive, Term};
use std::cell::Cell;

fn in_world(
    test: impl for<'world> FnOnce(
        &'world nir::program::Package,
        &SourceInvocationQueries<'_>,
        &mut Interpreter<'world>,
    ),
) {
    in_worlds(1, |package, queries, mut vms| {
        test(package, queries, &mut vms.pop().unwrap())
    });
}
fn in_worlds(
    count: usize,
    test: impl for<'world> FnOnce(
        &'world nir::program::Package,
        &SourceInvocationQueries<'_>,
        Vec<Interpreter<'world>>,
    ),
) {
    let package = sysroot::compile(
        "
        pure comptime trait Select {
            comptime fn choose(value: bool, .fallback: bool = not value) -> bool;
        }
        comptime impl Select for T where T {
            comptime fn choose(value: bool, fallback: bool) -> bool { fallback }
        }
        comptime fn run() -> bool {
            var trace: Integer = 0;
            trace = trace + 1;
            let first = bool.as(Select).choose(do { trace = trace * 10 + 2; true });
            let second = char.as(Select).choose(false);
            let method = u8.as(Select).choose;
            not first and second and method(false, true) and trace == 12
        }
    ",
    )
    .unwrap();
    assert!(!package.dispatch_instances.is_empty());
    let source = DeclarationSnapshot::new(std::sync::Arc::new(package));
    let package = source.package();
    let context = elaboration::ptc::Context::from_package(package).unwrap();
    let queries =
        SourceInvocationQueries::in_source(DeclarationSource::Snapshot(&source), &context, &[]);
    let types = |predicate: &nir::program::TypePredicate, fuel: &mut u64| {
        queries.check(predicate, Default::default(), fuel)
    };
    let fallback_calls = Cell::new(0);
    let methods = |access: &nir::program::Expr, _: &mut u64| {
        fallback_calls.set(fallback_calls.get() + 1);
        Err(comptime_interpretor::Error {
            kind: ErrorKind::UnmaterializedDispatch,
            source: Some(access.source),
        })
    };
    let associated = |access: &nir::program::Expr, fuel: &mut u64| queries.associated(access, fuel);
    let mut code = CodeSession::new(package.clone());
    let vms = (0..count)
        .map(|_| {
            Interpreter::for_comptime_resolving_in_session(
                package,
                &mut code,
                SourceResolvers {
                    types: &types,
                    methods: &methods,
                    associated: &associated,
                },
                &mut Budget {
                    steps: 1_000_000,
                    depth: 128,
                },
            )
            .unwrap()
        })
        .collect();
    test(package, &queries, vms);
    assert_eq!(fallback_calls.get(), 0, "selection must leave the driver");
}
fn entry(package: &nir::program::Package) -> nir::DeclarationId {
    package
        .functions
        .iter()
        .find(|f| f.name == "run")
        .unwrap()
        .declaration
}
fn take_method<'a>(active: &mut InvocationSession<'_, 'a>) -> MethodHostRequest<'a> {
    loop {
        match active.poll().unwrap() {
            Progress::Running => {}
            Progress::Method(request) => return request,
            other => panic!("expected method request: {other}"),
        }
    }
}
fn finish(active: &mut InvocationSession<'_, '_>) -> Result<Value, comptime_interpretor::Error> {
    loop {
        match active.poll()? {
            Progress::Running => {}
            Progress::Complete(value) => return Ok(value),
            other => panic!("unexpected remaining work: {other}"),
        }
    }
}

#[test]
fn direct_calls_method_values_and_defaults_resume_without_replaying_prefixes() {
    in_world(|package, queries, vm| {
        let mut active = vm
            .start_invocation(Value::Function(entry(package)), vec![])
            .unwrap();
        for subject in [Primitive::Bool, Primitive::Char, Primitive::U8] {
            let request = take_method(&mut active);
            assert!(std::ptr::eq(request.source_world(), package));
            let nir::program::ExprKind::TraitMethod {
                subject: actual, ..
            } = &request.access().kind
            else {
                panic!("method")
            };
            assert_eq!(*actual, Term::Primitive(subject));
            let paused = active.to_string();
            for _ in 0..3 {
                assert!(matches!(active.poll().unwrap(), Progress::Waiting));
                assert_eq!(active.remaining_steps(), 0);
                assert_eq!(active.to_string(), paused);
            }
            let response = request.resolve(|access, fuel| queries.resolve(access, fuel));
            active.resume_method(response).unwrap();
        }
        assert_eq!(finish(&mut active).unwrap(), Value::Bool(true));
        drop(active);
        let entered: Vec<_> = vm
            .events()
            .iter()
            .filter_map(|event| match event {
                Event::EnterFunction(id) => Some(*id),
                _ => None,
            })
            .collect();
        assert_eq!(
            entered.iter().filter(|id| **id == entry(package)).count(),
            1
        );
        let body = package
            .functions
            .iter()
            .find(|f| f.name == "choose")
            .unwrap()
            .declaration;
        assert_eq!(entered.iter().filter(|id| **id == body).count(), 3);
        // A second invocation reuses only admitted selection metadata. It still
        // executes every method body, argument, and default at its own position.
        let mut active = vm
            .start_invocation(Value::Function(entry(package)), vec![])
            .unwrap();
        assert_eq!(finish(&mut active).unwrap(), Value::Bool(true));
    });
}

#[test]
fn malformed_or_refilled_method_responses_never_enter_the_selected_body() {
    for alteration in 0..3 {
        in_world(|package, queries, vm| {
            let mut active = vm
                .start_invocation(Value::Function(entry(package)), vec![])
                .unwrap();
            let mut request = take_method(&mut active);
            let selected = queries.resolve(request.access(), &mut 100_000).unwrap();
            let mut selected = selected;
            if alteration == 0 {
                selected.member.local += 1;
            }
            if alteration == 1 {
                selected
                    .method_arguments
                    .push(Term::Primitive(Primitive::Bool));
            }
            if alteration == 2 {
                *request.remaining_steps_mut() += 1;
            }
            active.resume_method(request.respond(Ok(selected))).unwrap();
            assert_eq!(
                finish(&mut active).unwrap_err().kind,
                if alteration == 2 {
                    ErrorKind::StepsExhausted
                } else {
                    ErrorKind::InvalidCallPlan
                }
            );
            drop(active);
            let body = package
                .functions
                .iter()
                .find(|f| f.name == "choose")
                .unwrap()
                .declaration;
            assert!(
                !vm.events()
                    .iter()
                    .any(|e| matches!(e,Event::EnterFunction(id) if *id==body))
            );
            if alteration != 2 {
                let mut retry = vm
                    .start_invocation(Value::Function(entry(package)), vec![])
                    .unwrap();
                for _ in 0..3 {
                    let response = take_method(&mut retry)
                        .resolve(|access, fuel| queries.resolve(access, fuel));
                    retry.resume_method(response).unwrap();
                }
                assert_eq!(finish(&mut retry).unwrap(), Value::Bool(true));
            }
        });
    }
}

#[test]
fn cancelled_selection_rejects_late_answers_and_does_not_refund_work() {
    in_world(|package, queries, vm| {
        let mut active = vm
            .start_invocation(Value::Function(entry(package)), vec![])
            .unwrap();
        let response =
            take_method(&mut active).resolve(|access, fuel| queries.resolve(access, fuel));
        active.cancel();
        assert!(active.resume_method(response).is_err());
        assert_eq!(active.remaining_steps(), 0);
        assert!(active.poll().is_err());
        drop(active);
        assert_eq!(
            vm.start_invocation(Value::Function(entry(package)), vec![])
                .unwrap_err()
                .kind,
            ErrorKind::StepsExhausted
        );
    });
}

#[test]
fn foreign_method_response_leaves_both_waiters_available_for_correct_delivery() {
    in_worlds(2, |package, queries, mut vms| {
        let mut other = vms.pop().unwrap();
        let mut vm = vms.pop().unwrap();
        let mut first = vm
            .start_invocation(Value::Function(entry(package)), vec![])
            .unwrap();
        let mut second = other
            .start_invocation(Value::Function(entry(package)), vec![])
            .unwrap();
        let a = take_method(&mut first).resolve(|access, fuel| queries.resolve(access, fuel));
        let b = take_method(&mut second).resolve(|access, fuel| queries.resolve(access, fuel));
        let paused = second.to_string();
        let a = second.resume_method(a).unwrap_err().into_response();
        assert_eq!(second.to_string(), paused);
        assert_eq!(second.remaining_steps(), 0);
        first.resume_method(a).unwrap();
        second.resume_method(b).unwrap();
        for active in [&mut first, &mut second] {
            for _ in 0..2 {
                let response =
                    take_method(active).resolve(|access, fuel| queries.resolve(access, fuel));
                active.resume_method(response).unwrap();
            }
            assert_eq!(finish(active).unwrap(), Value::Bool(true));
        }
    });
}
