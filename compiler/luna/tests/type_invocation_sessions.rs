//! Type publication follows the completed owned invocation, including host work.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;

use comptime_interpretor::{Budget, CodeSession, Event, SourceResolvers};
use elaboration::materialization::{DeclarationSource, SourceInvocationQueries};
use elaboration::type_computations::{
    Error, Input, Invocation, InvocationProgress as Progress, InvocationStart as Start, Session,
    State,
};
use elaboration::{InferenceContext, InferenceTerm as I};
use nir::{DeclarationId, Primitive as P, Sort, Term};

const SOURCE: &str = "
    trait Selected { assoc enabled: bool; }
    pure comptime fn initialize(comptime T: Type) -> bool { T == bool }
    impl Selected for T where T { assoc enabled: bool = initialize(T); }
    comptime fn Output() -> Type {
        var prefix: Integer = 0;
        prefix = prefix + 1;
        if bool.as(Selected).enabled and prefix == 1 { u8 } else { u16 }
    }
    comptime fn Pass(comptime T: Type) -> Type { T }
";

fn in_source(
    test: impl FnOnce(
        &nir::program::Package,
        &SourceInvocationQueries<'_>,
        InferenceContext,
        Session<'_>,
    ),
) {
    let package = sysroot::compile(SOURCE).unwrap();
    let source = DeclarationSource::Borrowed(&package)
        .retain_declarations(&mut 1_000_000)
        .unwrap();
    let package = source.package();
    let theory = elaboration::ptc::Context::from_package(package).unwrap();
    let queries =
        SourceInvocationQueries::in_source(DeclarationSource::Snapshot(&source), &theory, &[]);
    let types = |predicate: &nir::program::TypePredicate, fuel: &mut u64| {
        queries.check(predicate, Default::default(), fuel)
    };
    let methods = |callee: &nir::program::Expr, fuel: &mut u64| queries.resolve(callee, fuel);
    let associated =
        |_: &nir::program::Expr, _: &mut u64| panic!("body selection must leave the driver");
    let context = InferenceContext::default();
    let mut code = CodeSession::new(package.clone());
    let session = Session::new_resolving_in_scope(
        package,
        &context,
        &mut code,
        &mut Budget {
            steps: 1_000_000,
            depth: 128,
        },
        &[],
        &[],
        &[],
        SourceResolvers {
            types: &types,
            methods: &methods,
            associated: &associated,
        },
    )
    .unwrap();
    test(package, &queries, context, session);
}

fn id(package: &nir::program::Package, name: &str) -> DeclarationId {
    package
        .functions
        .iter()
        .find(|f| f.name == name)
        .unwrap()
        .declaration
}

fn request<'w>(active: &mut Invocation<'_, 'w>) -> comptime_interpretor::AssociatedHostRequest<'w> {
    loop {
        match active.poll() {
            Progress::Running => {}
            Progress::Associated(request) => return request,
            Progress::Settled(state) => panic!("settled before selection: {state}"),
            Progress::Waiting => panic!("request was not returned"),
        }
    }
}

fn complete(active: &mut Invocation<'_, '_>) -> Term {
    loop {
        match active.poll() {
            Progress::Running => {}
            Progress::Settled(State::Complete(ty)) => return ty.clone(),
            Progress::Settled(state) => panic!("Type producer failed: {state}"),
            _ => panic!("unexpected additional request"),
        }
    }
}

fn enters(session: &Session<'_>, target: DeclarationId) -> usize {
    session
        .events()
        .iter()
        .filter(|e| matches!(e, Event::EnterFunction(id) if *id == target))
        .count()
}

#[test]
fn source_type_producer_keeps_its_prefix_and_publishes_once_after_host_resume() {
    in_source(|package, queries, mut context, mut session| {
        let output = session
            .enqueue(&mut context, id(package, "Output"), vec![])
            .unwrap();
        assert!(!context.is_assigned(output).unwrap());
        {
            let Start::Invocation(mut active) =
                session.start(&mut context, output, &mut 100_000).unwrap()
            else {
                panic!("ready producer must start")
            };
            let request = request(&mut active);
            assert!(std::ptr::eq(request.source_world(), package));
            let dump = active.to_string();
            for _ in 0..4 {
                assert!(matches!(active.poll(), Progress::Waiting));
                assert_eq!(active.to_string(), dump);
            }
            let response = request.resolve(|access, fuel| queries.associated(access, fuel));
            active.resume(response).unwrap();
            assert_eq!(complete(&mut active), Term::Primitive(P::U8));
            assert_eq!(complete(&mut active), Term::Primitive(P::U8));
        }
        assert_eq!(
            context.freeze(&I::Meta(output)).unwrap(),
            Term::Primitive(P::U8)
        );
        assert!(matches!(
            session.start(&mut context, output, &mut 0).unwrap(),
            Start::Settled(State::Complete(_))
        ));
        assert_eq!(enters(&session, id(package, "Output")), 1);
        assert_eq!(enters(&session, id(package, "initialize")), 1);
    });
}

#[test]
fn cancellation_blocks_dependents_and_late_answers_cannot_resume_another_producer() {
    in_source(|package, queries, mut context, mut session| {
        let first = session
            .enqueue(&mut context, id(package, "Output"), vec![])
            .unwrap();
        let dependent = session
            .enqueue(
                &mut context,
                id(package, "Pass"),
                vec![Input {
                    ty: I::Stable(Term::Primitive(P::Type)),
                    value: I::Meta(first),
                }],
            )
            .unwrap();
        assert!(matches!(
            session
                .start(&mut context, dependent, &mut 100_000)
                .unwrap(),
            Start::Settled(State::Waiting(_))
        ));
        assert!(session.events().is_empty());
        let late = {
            let Start::Invocation(mut active) =
                session.start(&mut context, first, &mut 100_000).unwrap()
            else {
                panic!("ready")
            };
            let request = request(&mut active);
            let response = request.resolve(|access, fuel| queries.associated(access, fuel));
            active.cancel();
            assert!(matches!(
                active.poll(),
                Progress::Settled(State::Failed(Error::Cancelled))
            ));
            // A terminal wrapper must return the complete rejected answer to its owner.
            active.resume(response).unwrap_err().into_response()
        };
        assert!(!context.is_assigned(first).unwrap());
        assert!(
            matches!(session.start(&mut context, dependent, &mut 100_000).unwrap(), Start::Settled(State::Failed(Error::DependencyFailed(failed))) if *failed == first)
        );
        assert_eq!(
            session.remaining_steps(),
            0,
            "undelivered fuel is not refunded"
        );
        assert!(!context.is_assigned(first).unwrap());
        assert!(!context.is_assigned(dependent).unwrap());
        assert_eq!(enters(&session, id(package, "Output")), 1);
        assert_eq!(enters(&session, id(package, "Pass")), 0);
        assert_eq!(enters(&session, id(package, "initialize")), 0);

        // A new independently budgeted session in the same source world must
        // reject the old answer without consuming its own outstanding waiter.
        let types = |predicate: &nir::program::TypePredicate, fuel: &mut u64| {
            queries.check(predicate, Default::default(), fuel)
        };
        let methods = |callee: &nir::program::Expr, fuel: &mut u64| queries.resolve(callee, fuel);
        let associated = |_: &nir::program::Expr, _: &mut u64| panic!("external selection");
        let mut code = CodeSession::new(package.clone().into());
        let mut context = InferenceContext::default();
        let mut session = Session::new_resolving_in_scope(
            package,
            &context,
            &mut code,
            &mut Budget {
                steps: 1_000_000,
                depth: 128,
            },
            &[],
            &[],
            &[],
            SourceResolvers {
                types: &types,
                methods: &methods,
                associated: &associated,
            },
        )
        .unwrap();
        let second = session
            .enqueue(&mut context, id(package, "Output"), vec![])
            .unwrap();
        {
            let Start::Invocation(mut active) =
                session.start(&mut context, second, &mut 100_000).unwrap()
            else {
                panic!("ready")
            };
            let current = request(&mut active);
            let _rejected = active.resume(late).unwrap_err();
            assert!(matches!(active.poll(), Progress::Waiting));
            active
                .resume(current.resolve(|access, fuel| queries.associated(access, fuel)))
                .unwrap();
            assert_eq!(complete(&mut active), Term::Primitive(P::U8));
        }
        assert!(context.is_assigned(second).unwrap());
        assert_eq!(enters(&session, id(package, "Output")), 1);
        assert_eq!(enters(&session, id(package, "Pass")), 0);
        assert_eq!(enters(&session, id(package, "initialize")), 1);
    });
}

#[test]
fn dropping_an_active_producer_is_terminal_without_publishing_a_type() {
    in_source(|package, _queries, mut context, mut session| {
        let output = session
            .enqueue(&mut context, id(package, "Output"), vec![])
            .unwrap();
        {
            let Start::Invocation(mut active) =
                session.start(&mut context, output, &mut 100_000).unwrap()
            else {
                panic!("ready")
            };
            drop(request(&mut active));
        }
        assert!(!context.is_assigned(output).unwrap());
        assert!(matches!(
            session.start(&mut context, output, &mut 0).unwrap(),
            Start::Settled(State::Failed(Error::Cancelled))
        ));
        assert_eq!(enters(&session, id(package, "Output")), 1);
    });
}

#[test]
fn host_failure_and_exhaustion_are_terminal_without_replaying_the_prefix() {
    for exhausted in [false, true] {
        in_source(|package, queries, mut context, mut session| {
            let output = session
                .enqueue(&mut context, id(package, "Output"), vec![])
                .unwrap();
            {
                let Start::Invocation(mut active) =
                    session.start(&mut context, output, &mut 100_000).unwrap()
                else {
                    panic!("ready")
                };
                let request = request(&mut active);
                let response = if exhausted {
                    request.resolve(|access, fuel| {
                        let selection = queries.associated(access, fuel)?;
                        *fuel = 0;
                        Ok(selection)
                    })
                } else {
                    request.respond(Err(comptime_interpretor::Error {
                        kind: comptime_interpretor::ErrorKind::UnmaterializedDispatch,
                        source: None,
                    }))
                };
                active.resume(response).unwrap();
                loop {
                    match active.poll() {
                        Progress::Running => {}
                        Progress::Settled(State::Failed(Error::Invocation(error))) => {
                            assert_eq!(
                                error.kind,
                                if exhausted {
                                    comptime_interpretor::ErrorKind::StepsExhausted
                                } else {
                                    comptime_interpretor::ErrorKind::UnmaterializedDispatch
                                }
                            );
                            break;
                        }
                        _ => panic!("host failure must settle"),
                    }
                }
            }
            assert!(!context.is_assigned(output).unwrap());
            assert!(matches!(
                session.start(&mut context, output, &mut 0).unwrap(),
                Start::Settled(State::Failed(_))
            ));
            assert_eq!(enters(&session, id(package, "Output")), 1);
            assert_eq!(enters(&session, id(package, "initialize")), 0);
        });
    }
}

#[test]
fn production_required_type_path_resolves_associated_initializers() {
    assert_eq!(
        sysroot::run(&format!(
            "{SOURCE}
        fn run() -> bool {{ const T = Output(); let value: T = 7; value == 7 }}
    "
        )),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn scheduling_exhaustion_and_foreign_context_do_not_consume_the_producer() {
    in_source(|package, _queries, mut context, mut session| {
        let output = session
            .enqueue(&mut context, id(package, "Output"), vec![])
            .unwrap();
        assert!(matches!(
            session.start(&mut context, output, &mut 0),
            Err(Error::SchedulingBudget)
        ));
        let mut other = InferenceContext::default();
        other.fresh(Sort::Type);
        assert!(matches!(
            session.start(&mut other, output, &mut 100_000),
            Err(Error::ForeignContext)
        ));
        assert!(session.events().is_empty());
        assert!(matches!(session.state(output), Some(State::Waiting(_))));
        let Start::Invocation(mut active) =
            session.start(&mut context, output, &mut 100_000).unwrap()
        else {
            panic!("ready")
        };
        drop(request(&mut active));
    });
}

#[test]
fn live_input_admission_rejects_wrong_domains_before_execution_and_allows_next_work() {
    in_source(|package, _queries, mut context, mut session| {
        let invalid = session
            .enqueue_live(
                &mut context,
                id(package, "Pass"),
                vec![elaboration::type_computations::LiveInput {
                    ty: I::Stable(Term::Primitive(P::Type)),
                    value: comptime_interpretor::Value::Bool(true),
                }],
            )
            .unwrap();
        assert!(matches!(
            session.start(&mut context, invalid, &mut 100_000).unwrap(),
            Start::Settled(State::Failed(Error::Invocation(_)))
        ));
        assert!(!context.is_assigned(invalid).unwrap());
        assert!(session.events().is_empty());
        let domain = context.fresh(Sort::Type);
        let valid = session
            .enqueue_live(
                &mut context,
                id(package, "Pass"),
                vec![elaboration::type_computations::LiveInput {
                    ty: I::Meta(domain),
                    value: comptime_interpretor::Value::Type(Term::Primitive(P::Bool)),
                }],
            )
            .unwrap();
        assert!(matches!(
            session.start(&mut context, valid, &mut 100_000).unwrap(),
            Start::Settled(State::Waiting(_))
        ));
        assert!(session.events().is_empty());
        context
            .assign(domain, I::Stable(Term::Primitive(P::Type)))
            .unwrap();
        {
            let Start::Invocation(mut active) =
                session.start(&mut context, valid, &mut 100_000).unwrap()
            else {
                panic!("supplied input")
            };
            assert_eq!(complete(&mut active), Term::Primitive(P::Bool));
        }
        assert_eq!(
            context.freeze(&I::Meta(valid)).unwrap(),
            Term::Primitive(P::Bool)
        );
        assert_eq!(enters(&session, id(package, "Pass")), 1);
        assert!(matches!(
            session.start(&mut context, invalid, &mut 0).unwrap(),
            Start::Settled(State::Failed(_))
        ));
    });
}
