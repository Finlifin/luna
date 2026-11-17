//! Type publication follows the completed owned invocation, including host work.
#[path = "../../elaboration/test_support/type_computations/mod.rs"]
#[allow(dead_code)]
mod type_computations;
use elaboration::gradual;

#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;

use comptime_interpretor::{Budget, CodeSession, Event, SourceResolvers};
use elaboration::materialization::{DeclarationSource, SourceInvocationQueries};
use elaboration::{InferenceContext, InferenceTerm as I};
use nir::{DeclarationId, Primitive as P, Sort, Term};
use type_computations::{
    Error, Input, Invocation, InvocationProgress as Progress, InvocationStart as Start, Session,
    State,
};

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
    in_sources(1, |package, queries, context, mut sessions| {
        test(package, queries, context, sessions.pop().unwrap());
    });
}

fn in_sources(
    count: usize,
    test: impl FnOnce(
        &nir::program::Package,
        &SourceInvocationQueries<'_>,
        InferenceContext,
        Vec<Session<'_>>,
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
    let sessions = (0..count)
        .map(|_| {
            Session::new_resolving_in_scope(
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
            .unwrap()
        })
        .collect();
    test(package, &queries, context, sessions);
}

fn id(package: &nir::program::Package, name: &str) -> DeclarationId {
    package
        .functions
        .iter()
        .find(|f| f.name == name)
        .unwrap()
        .declaration
}

fn request<'w>(
    active: &mut Invocation<'_, 'w>,
    context: &mut InferenceContext,
) -> comptime_interpretor::AssociatedHostRequest<'w> {
    loop {
        match active.poll(context).unwrap() {
            Progress::Running => {}
            Progress::Associated(request) => return request,
            Progress::Method(request) => active.resume_method(request.prepare()).unwrap(),
            Progress::Definition(_) => panic!("unexpected definition preparation"),
            Progress::Settled(state) => panic!("settled before selection: {state}"),
            Progress::Waiting => panic!("request was not returned"),
        }
    }
}

fn complete(active: &mut Invocation<'_, '_>, context: &mut InferenceContext) -> Term {
    loop {
        match active.poll(context).unwrap() {
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
            let request = request(&mut active, &mut context);
            assert!(std::ptr::eq(request.source_world(), package));
            let dump = active.to_string();
            for _ in 0..4 {
                assert!(matches!(
                    active.poll(&mut context).unwrap(),
                    Progress::Waiting
                ));
                assert_eq!(active.to_string(), dump);
            }
            let response = request.resolve(|access, fuel| queries.associated(access, fuel));
            active.resume(response).unwrap();
            assert_eq!(complete(&mut active, &mut context), Term::Primitive(P::U8));
            assert_eq!(complete(&mut active, &mut context), Term::Primitive(P::U8));
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
            let request = request(&mut active, &mut context);
            let response = request.resolve(|access, fuel| queries.associated(access, fuel));
            active.cancel();
            assert!(matches!(
                active.poll(&mut context).unwrap(),
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
            let current = request(&mut active, &mut context);
            let _rejected = active.resume(late).unwrap_err();
            assert!(matches!(
                active.poll(&mut context).unwrap(),
                Progress::Waiting
            ));
            active
                .resume(current.resolve(|access, fuel| queries.associated(access, fuel)))
                .unwrap();
            assert_eq!(complete(&mut active, &mut context), Term::Primitive(P::U8));
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
            drop(request(&mut active, &mut context));
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
                let request = request(&mut active, &mut context);
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
                    match active.poll(&mut context).unwrap() {
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
        drop(request(&mut active, &mut context));
    });
}

#[test]
fn live_input_admission_rejects_wrong_domains_before_execution_and_allows_next_work() {
    in_source(|package, _queries, mut context, mut session| {
        let invalid = session
            .enqueue_live(
                &mut context,
                id(package, "Pass"),
                vec![type_computations::LiveInput {
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
                vec![type_computations::LiveInput {
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
            assert_eq!(
                complete(&mut active, &mut context),
                Term::Primitive(P::Bool)
            );
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

#[test]
fn suspended_source_invocation_allows_other_inference_and_rejects_wrong_context_polls() {
    in_source(|package, queries, mut context, mut session| {
        let output = session
            .enqueue(&mut context, id(package, "Output"), vec![])
            .unwrap();
        let unrelated = context.fresh(Sort::Type);
        let Start::Invocation(mut active) =
            session.start(&mut context, output, &mut 100_000).unwrap()
        else {
            panic!("ready")
        };
        let mut foreign = InferenceContext::default();
        let before = active.to_string();
        assert!(matches!(
            active.poll(&mut foreign),
            Err(Error::ForeignContext)
        ));
        assert_eq!(active.to_string(), before);
        let request = request(&mut active, &mut context);
        let waiting = active.to_string();
        assert!(!context.is_assigned(output).unwrap());
        let mut candidate = context.trial();
        assert!(matches!(
            active.poll(&mut candidate),
            Err(Error::SpeculativeContext)
        ));
        assert!(matches!(
            active.poll(&mut foreign),
            Err(Error::ForeignContext)
        ));
        assert_eq!(active.to_string(), waiting);
        assert_eq!(
            candidate.assign(output, I::Stable(Term::Primitive(P::U16))),
            Err(elaboration::InferenceError::ReservedVariable(output))
        );
        candidate
            .assign(unrelated, I::Stable(Term::Primitive(P::Bool)))
            .unwrap();
        context.commit(candidate).unwrap();
        assert!(context.is_assigned(unrelated).unwrap());
        assert_eq!(
            context.assign(output, I::Stable(Term::Primitive(P::U16))),
            Err(elaboration::InferenceError::ReservedVariable(output))
        );
        assert!(matches!(
            active.poll(&mut context).unwrap(),
            Progress::Waiting
        ));
        active
            .resume(request.resolve(|access, fuel| queries.associated(access, fuel)))
            .unwrap();
        assert_eq!(complete(&mut active, &mut context), Term::Primitive(P::U8));
        assert!(matches!(
            active.poll(&mut foreign),
            Err(Error::ForeignContext)
        ));
        assert_eq!(complete(&mut active, &mut context), Term::Primitive(P::U8));
        drop(active);
        assert_eq!(enters(&session, id(package, "Output")), 1);
        assert_eq!(enters(&session, id(package, "initialize")), 1);
    });
}

#[test]
fn cancellation_after_independent_inference_preserves_the_unpublished_output() {
    in_source(|package, _queries, mut context, mut session| {
        let output = session
            .enqueue(&mut context, id(package, "Output"), vec![])
            .unwrap();
        let Start::Invocation(mut active) =
            session.start(&mut context, output, &mut 100_000).unwrap()
        else {
            panic!("ready")
        };
        drop(request(&mut active, &mut context));
        let other = context.fresh(Sort::Type);
        context
            .assign(other, I::Stable(Term::Primitive(P::Bool)))
            .unwrap();
        drop(active);
        assert!(!context.is_assigned(output).unwrap());
        assert!(context.is_assigned(other).unwrap());
        assert_eq!(
            context.assign(output, I::Stable(Term::Primitive(P::U8))),
            Err(elaboration::InferenceError::ReservedVariable(output))
        );
        assert!(matches!(
            session.start(&mut context, output, &mut 0).unwrap(),
            Start::Settled(State::Failed(Error::Cancelled))
        ));
    });
}

#[test]
fn two_source_producers_can_publish_into_one_context_without_sharing_execution() {
    in_sources(2, |package, queries, mut context, mut sessions| {
        let mut first = sessions.pop().unwrap();
        let mut second = sessions.pop().unwrap();
        let left = first
            .enqueue(&mut context, id(package, "Output"), vec![])
            .unwrap();
        let right = second
            .enqueue(&mut context, id(package, "Output"), vec![])
            .unwrap();
        let Start::Invocation(mut left_run) =
            first.start(&mut context, left, &mut 100_000).unwrap()
        else {
            panic!("left ready")
        };
        let left_request = request(&mut left_run, &mut context);
        let Start::Invocation(mut right_run) =
            second.start(&mut context, right, &mut 100_000).unwrap()
        else {
            panic!("right ready")
        };
        let right_request = request(&mut right_run, &mut context);
        assert!(!context.is_assigned(left).unwrap());
        assert!(!context.is_assigned(right).unwrap());
        let left_answer = left_request.resolve(|access, fuel| queries.associated(access, fuel));
        let left_answer = right_run.resume(left_answer).unwrap_err().into_response();
        assert!(matches!(
            right_run.poll(&mut context).unwrap(),
            Progress::Waiting
        ));
        right_run
            .resume(right_request.resolve(|access, fuel| queries.associated(access, fuel)))
            .unwrap();
        assert_eq!(
            complete(&mut right_run, &mut context),
            Term::Primitive(P::U8)
        );
        assert!(!context.is_assigned(left).unwrap());
        assert!(context.is_assigned(right).unwrap());
        assert!(matches!(
            left_run.poll(&mut context).unwrap(),
            Progress::Waiting
        ));
        left_run.resume(left_answer).unwrap();
        assert_eq!(
            complete(&mut left_run, &mut context),
            Term::Primitive(P::U8)
        );
        drop((left_run, right_run));
        for session in [&first, &second] {
            assert_eq!(enters(session, id(package, "Output")), 1);
            assert_eq!(enters(session, id(package, "initialize")), 1);
        }
    });
}

#[test]
fn generated_type_output_remains_reserved_until_its_receipt_is_installed() {
    use elaboration::definitions::AllocatedDefinitionPublisher;
    let package = sysroot::compile(
        "
        comptime fn Build() -> Type {
            let record = meta.StructBuilder.new(.name = .Record);
            record.add_field(.flag, bool);
            record.Build()
        }
    ",
    )
    .unwrap();
    let theory = elaboration::ptc::Context::from_package(&package).unwrap();
    let queries =
        SourceInvocationQueries::in_source(DeclarationSource::Borrowed(&package), &theory, &[]);
    let types = |predicate: &nir::program::TypePredicate, fuel: &mut u64| {
        queries.check(predicate, Default::default(), fuel)
    };
    let methods = |callee: &nir::program::Expr, fuel: &mut u64| queries.resolve(callee, fuel);
    let associated = |access: &nir::program::Expr, fuel: &mut u64| queries.associated(access, fuel);
    let publisher =
        AllocatedDefinitionPublisher::new_with_type_solver(&package, 9900, 1, 10_000_000, &types);
    let mut context = InferenceContext::default();
    let mut code = CodeSession::new(package.clone().into());
    let mut session = Session::new_resolving_in_scope(
        &package,
        &context,
        &mut code,
        &mut Budget {
            steps: 10_000_000,
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
    let output = session
        .enqueue(&mut context, id(&package, "Build"), vec![])
        .unwrap();
    let ty = {
        let Start::Invocation(mut active) =
            session.start(&mut context, output, &mut 100_000).unwrap()
        else {
            panic!("ready")
        };
        let mut request = loop {
            match active.poll(&mut context).unwrap() {
                Progress::Running => {}
                Progress::Definition(request) => break request,
                Progress::Associated(request) => {
                    active.resume(request.resolve(&associated)).unwrap();
                }
                Progress::Method(request) => active.resume_method(request.prepare()).unwrap(),
                _ => panic!("definition producer must suspend"),
            }
        };
        assert!(!context.is_assigned(output).unwrap());
        let comptime_interpretor::DefinitionBuildRequest::Struct(descriptor) = request.descriptor()
        else {
            panic!("expected struct construction")
        };
        let mut fuel = request.remaining_steps();
        let prepared = publisher
            .prepare_struct(descriptor, &mut fuel)
            .map(comptime_interpretor::PreparedDefinitionBuild::Struct);
        *request.remaining_steps_mut() = fuel;
        let response = request.respond(prepared);
        assert!(!context.is_assigned(output).unwrap());
        active.resume_definition(response).unwrap();
        assert!(!context.is_assigned(output).unwrap());
        complete(&mut active, &mut context)
    };
    assert_eq!(context.freeze(&I::Meta(output)).unwrap(), ty);
    assert_eq!(enters(&session, id(&package, "Build")), 1);
    drop(session);
    let (published, _) = publisher.finish();
    assert_eq!(published.structs.len(), 1);
    assert_eq!(
        ty,
        Term::Nominal {
            declaration: published.structs[0][0].declaration,
            arguments: vec![]
        }
    );
}

#[test]
fn synchronous_and_owned_type_demands_share_selection_failure_and_recovery() {
    use comptime_interpretor::{Error as VmError, ErrorKind};
    use elaboration::MetaId;
    use std::cell::Cell;

    fn drive(
        session: &mut Session<'_>,
        context: &mut InferenceContext,
        producer: MetaId,
        dependent: MetaId,
        mode: usize,
    ) {
        if mode == 2 {
            session
                .resume_dependencies(context, dependent, &mut 100_000)
                .unwrap();
            return;
        }
        for output in [producer, dependent] {
            match mode {
                0 => {
                    session.resume(context, output).unwrap();
                }
                1 => {
                    session
                        .resume_bounded(context, output, &mut 100_000)
                        .unwrap();
                }
                3 => {
                    if let Start::Invocation(mut active) =
                        session.start(context, output, &mut 100_000).unwrap()
                    {
                        loop {
                            match active.poll(context).unwrap() {
                                Progress::Running => {}
                                Progress::Associated(request) => {
                                    let response = request.prepare();
                                    // Resolving the request does not resume the producer or
                                    // grant permission to publish its Type ahead of execution.
                                    assert!(!context.is_assigned(output).unwrap());
                                    assert!(matches!(
                                        active.poll(context).unwrap(),
                                        Progress::Waiting
                                    ));
                                    active.resume(response).unwrap();
                                }
                                Progress::Definition(request) => {
                                    active.resume_definition(request.prepare()).unwrap();
                                }
                                Progress::Method(request) => {
                                    active.resume_method(request.prepare()).unwrap();
                                }
                                Progress::Settled(_) => break,
                                Progress::Waiting => panic!("no undelivered response"),
                            }
                        }
                    }
                }
                _ => unreachable!(),
            }
        }
    }

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
    for mode in 0..4 {
        let reject = Cell::new(true);
        let selections = Cell::new(0);
        let associated = |access: &nir::program::Expr, fuel: &mut u64| {
            selections.set(selections.get() + 1);
            if reject.get() {
                Err(VmError {
                    kind: ErrorKind::UnmaterializedDispatch,
                    source: Some(access.source),
                })
            } else {
                queries.associated(access, fuel)
            }
        };
        let mut context = InferenceContext::default();
        let mut code = CodeSession::new(package.clone());
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
        for rejecting in [true, false] {
            reject.set(rejecting);
            let output = session
                .enqueue(&mut context, id(package, "Output"), vec![])
                .unwrap();
            let dependent = session
                .enqueue(
                    &mut context,
                    id(package, "Pass"),
                    vec![Input {
                        ty: I::Stable(Term::Primitive(P::Type)),
                        value: I::Meta(output),
                    }],
                )
                .unwrap();
            // A single-output poll must not run its unresolved dependency.
            assert!(matches!(
                session.resume(&mut context, dependent).unwrap(),
                State::Waiting(_)
            ));
            let before = selections.get();
            drive(&mut session, &mut context, output, dependent, mode);
            assert_eq!(selections.get(), before + 1, "mode {mode}");
            if rejecting {
                assert!(matches!(
                    session.state(output),
                    Some(State::Failed(Error::Invocation(_)))
                ));
                assert!(
                    matches!(session.state(dependent), Some(State::Failed(Error::DependencyFailed(id))) if *id == output)
                );
                assert!(!context.is_assigned(output).unwrap());
                assert!(!context.is_assigned(dependent).unwrap());
                assert_eq!(enters(&session, id(package, "initialize")), 0);
                assert_eq!(enters(&session, id(package, "Pass")), 0);
            } else {
                assert_eq!(
                    context.freeze(&I::Meta(dependent)).unwrap(),
                    Term::Primitive(P::U8)
                );
                assert_eq!(enters(&session, id(package, "initialize")), 1);
                assert_eq!(enters(&session, id(package, "Pass")), 1);
            }
            let remaining = session.remaining_steps();
            let events = session.events().len();
            drive(&mut session, &mut context, output, dependent, mode);
            assert_eq!(selections.get(), before + 1);
            assert_eq!(session.remaining_steps(), remaining);
            assert_eq!(session.events().len(), events);
        }
        assert_eq!(enters(&session, id(package, "Output")), 2);
    }
}

#[test]
fn synchronous_type_host_unwind_cancels_the_owner_without_replaying_effects() {
    // This source helper deliberately configures a panicking selection service.
    // Unwinding a convenience drive must terminate the same owner as Drop on
    // an explicitly suspended handle, even before a host response exists.
    in_source(|package, _, mut context, mut session| {
        let output = session
            .enqueue(&mut context, id(package, "Output"), vec![])
            .unwrap();
        let dependent = session
            .enqueue(
                &mut context,
                id(package, "Pass"),
                vec![Input {
                    ty: I::Stable(Term::Primitive(P::Type)),
                    value: I::Meta(output),
                }],
            )
            .unwrap();
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            session.resume(&mut context, output).unwrap();
        }));
        assert!(unwound.is_err());
        assert!(matches!(
            session.state(output),
            Some(State::Failed(Error::Cancelled))
        ));
        assert!(!context.is_assigned(output).unwrap());
        assert_eq!(enters(&session, id(package, "Output")), 1);
        assert_eq!(enters(&session, id(package, "initialize")), 0);
        assert_eq!(
            session.remaining_steps(),
            0,
            "lost host fuel is not refunded"
        );
        assert!(matches!(
            session.resume(&mut context, output).unwrap(),
            State::Failed(Error::Cancelled)
        ));
        assert!(
            matches!(session.resume_dependencies(&mut context, dependent, &mut 100_000).unwrap(),
            State::Failed(Error::DependencyFailed(failed)) if *failed == output)
        );
        assert_eq!(enters(&session, id(package, "Output")), 1);
        assert_eq!(enters(&session, id(package, "Pass")), 0);
        assert!(!context.is_assigned(dependent).unwrap());
    });
}
