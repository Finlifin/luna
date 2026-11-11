//! Demand planning is bounded independently of invocation, and never replays it.
use comptime_interpretor::{Budget, Event};
use elaboration::type_computations::{Error, Input, Session, State};
use elaboration::{InferenceContext, InferenceTerm as I};
use nir::program::{Expr, ExprKind as K, Function, FunctionPhase, LocalId, Package, SourceRange};
use nir::{DeclarationId, Primitive as P, Sort, Term};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 93, local }
}
fn input(value: I) -> Input {
    Input {
        ty: I::Stable(Term::Primitive(P::Type)),
        value,
    }
}
fn package() -> Package {
    let source = SourceRange { start: 1, end: 9 };
    let functions = (0..3)
        .map(|count| {
            let parameters = (0..count)
                .map(|index| LocalId {
                    owner: count,
                    index,
                })
                .collect::<Vec<_>>();
            Function {
                declaration: id(count),
                name: format!("Factory{count}"),
                phase: FunctionPhase::Comptime,
                inline: false,
                pure: false,
                is_unsafe: false,
                preconditions: vec![],
                postconditions: vec![],
                result_binding: None,
                entry_snapshots: vec![],
                type_parameters: vec![],
                requirements: vec![],
                capabilities: vec![],
                signature: Term::Function {
                    parameters: (0..count)
                        .map(|_| nir::Parameter {
                            sort: Sort::Type,
                            ty: Term::Primitive(P::Type),
                        })
                        .collect(),
                    result: Box::new(Term::Primitive(P::Type)),
                },
                body: Expr {
                    ty: Term::Primitive(P::Type),
                    source,
                    kind: if count == 0 {
                        K::TypeValue(Term::Primitive(P::Bool))
                    } else {
                        K::Local(parameters[0])
                    },
                },
                parameters,
                defaults: vec![],
            }
        })
        .collect();
    Package {
        functions,
        ..Default::default()
    }
}

#[test]
fn direct_resume_has_one_graph_allowance_and_exhaustion_remains_retryable() {
    let package = package();
    let mut context = InferenceContext::default();
    let external = context.fresh(Sort::Type);
    let mut session = Session::new(&package, &context, Budget::default()).unwrap();
    let mut output = external;
    for _ in 0..20_000 {
        output = session
            .enqueue(&mut context, id(1), vec![input(I::Meta(output))])
            .unwrap();
    }
    let before = session.remaining_steps();
    assert_eq!(
        session.resume(&mut context, output),
        Err(Error::SchedulingBudget)
    );
    assert!(matches!(session.state(output), Some(State::Waiting(_))));
    assert!(!context.is_assigned(output).unwrap());
    assert_eq!(session.remaining_steps(), before);
    assert!(session.events().is_empty());
    // More scheduling allowance can observe the same retained graph. This
    // does not grant an invocation input or replenish the VM's execution fuel.
    let mut fuel = 1_000_000;
    assert!(
        matches!(session.resume_bounded(&mut context, output, &mut fuel).unwrap(),
        State::Waiting(inputs) if inputs.len() == 1 && !inputs.contains(&external))
    );
    assert!(fuel < 1_000_000);
    assert!(session.events().is_empty());
    let mut fuel = 1_000_000;
    assert!(
        matches!(session.resume_dependencies(&mut context, output, &mut fuel).unwrap(),
        State::Waiting(inputs) if inputs == &std::collections::BTreeSet::from([external]))
    );
    assert!(fuel < 1_000_000);
    assert_eq!(session.remaining_steps(), before);
    assert!(session.events().is_empty());
}

#[test]
fn every_exhausted_preflight_preserves_ready_producers_for_one_later_execution() {
    let package = package();
    let mut exhausted = 0;
    let mut complete = 0;
    for mut allowance in 0..128 {
        let mut context = InferenceContext::default();
        let mut session = Session::new(&package, &context, Budget::default()).unwrap();
        let first = session.enqueue(&mut context, id(0), vec![]).unwrap();
        let left = session
            .enqueue(&mut context, id(1), vec![input(I::Meta(first))])
            .unwrap();
        let right = session
            .enqueue(&mut context, id(1), vec![input(I::Meta(first))])
            .unwrap();
        let output = session
            .enqueue(
                &mut context,
                id(2),
                vec![input(I::Meta(left)), input(I::Meta(right))],
            )
            .unwrap();
        match session.resume_dependencies(&mut context, output, &mut allowance) {
            Err(Error::SchedulingBudget) => {
                exhausted += 1;
                assert!(session.events().is_empty());
                for output in [first, left, right, output] {
                    assert!(matches!(session.state(output), Some(State::Waiting(_))));
                    assert!(!context.is_assigned(output).unwrap());
                }
                assert!(
                    matches!(session.resume_dependencies(&mut context, output, &mut 1000).unwrap(), State::Complete(ty) if *ty == Term::Primitive(P::Bool))
                );
            }
            Ok(State::Complete(ty)) => {
                complete += 1;
                assert_eq!(*ty, Term::Primitive(P::Bool));
            }
            result => panic!("unexpected demand result: {result:?}"),
        }
        let entered = session
            .events()
            .iter()
            .filter_map(|event| match event {
                Event::EnterFunction(id) => Some(*id),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(entered, vec![id(0), id(1), id(1), id(2)]);
        let events = session.events().len();
        session
            .resume_dependencies(&mut context, output, &mut 0)
            .unwrap();
        session.resume(&mut context, output).unwrap();
        assert_eq!(session.events().len(), events);
    }
    assert!(exhausted > 0 && complete > 0);
}

#[test]
fn cycle_preflight_is_bounded_without_terminalizing_budget_exhaustion() {
    let package = package();
    let mut context = InferenceContext::default();
    let mut session = Session::new(&package, &context, Budget::default()).unwrap();
    let alias = context.fresh(Sort::Type);
    let first = session
        .enqueue(&mut context, id(1), vec![input(I::Meta(alias))])
        .unwrap();
    let second = session
        .enqueue(&mut context, id(1), vec![input(I::Meta(first))])
        .unwrap();
    context.assign(alias, I::Meta(second)).unwrap();
    let ready = session.enqueue(&mut context, id(0), vec![]).unwrap();
    let output = session
        .enqueue(
            &mut context,
            id(2),
            vec![input(I::Meta(ready)), input(I::Meta(first))],
        )
        .unwrap();
    assert_eq!(
        session.resume_dependencies(&mut context, output, &mut 3),
        Err(Error::SchedulingBudget)
    );
    assert!(matches!(session.state(output), Some(State::Waiting(_))));
    assert!(session.events().is_empty());
    assert!(matches!(
        session
            .resume_dependencies(&mut context, output, &mut 1000)
            .unwrap(),
        State::Failed(Error::DependencyCycle(_))
    ));
    assert!(session.events().is_empty());
    assert!(matches!(session.state(ready), Some(State::Waiting(_))));
    assert!(matches!(
        session.resume(&mut context, ready).unwrap(),
        State::Complete(_)
    ));
}

#[test]
fn bounded_single_resume_observes_but_never_drives_dependency_bodies() {
    let package = package();
    let mut context = InferenceContext::default();
    let mut session = Session::new(&package, &context, Budget::default()).unwrap();
    let dependency = session.enqueue(&mut context, id(0), vec![]).unwrap();
    let output = session
        .enqueue(&mut context, id(1), vec![input(I::Meta(dependency))])
        .unwrap();
    assert!(
        matches!(session.resume_bounded(&mut context, output, &mut 1000).unwrap(),
        State::Waiting(inputs) if inputs == &std::collections::BTreeSet::from([dependency]))
    );
    assert!(session.events().is_empty());
    assert!(matches!(
        session
            .resume_bounded(&mut context, dependency, &mut 1000)
            .unwrap(),
        State::Complete(_)
    ));
    let events = session.events().len();
    assert_eq!(
        session.resume_bounded(&mut context, output, &mut 0),
        Err(Error::SchedulingBudget)
    );
    assert_eq!(session.events().len(), events);
    assert!(
        matches!(session.resume_bounded(&mut context, output, &mut 1000).unwrap(),
        State::Complete(ty) if *ty == Term::Primitive(P::Bool))
    );
    assert_eq!(
        session
            .events()
            .iter()
            .filter_map(|event| match event {
                Event::EnterFunction(id) => Some(*id),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![id(0), id(1)]
    );
}
