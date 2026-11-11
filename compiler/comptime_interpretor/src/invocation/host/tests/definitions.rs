use super::*;
use std::cell::Cell;

fn build() -> Expr {
    expr(
        ty(Primitive::Type),
        K::BuildNewtype(Box::new(expr(
            Term::Tuple(vec![ty(Primitive::Symbol), ty(Primitive::Type)]),
            K::Tuple(vec![
                expr(
                    ty(Primitive::Symbol),
                    K::Literal(Literal::Symbol("Generated".into())),
                ),
                expr(ty(Primitive::Type), K::TypeValue(ty(Primitive::Bool))),
            ]),
        ))),
    )
}
fn body(count: usize) -> Value {
    callable(
        expr(
            ty(Primitive::Bool),
            K::Block {
                statements: (0..count).map(|_| build()).collect(),
                result: Some(Box::new(expr(
                    ty(Primitive::Bool),
                    K::Literal(Literal::Bool(true)),
                ))),
            },
        ),
        None,
    )
}
fn request<'a>(active: &mut InvocationSession<'_, 'a>) -> DefinitionHostRequest<'a> {
    for _ in 0..1000 {
        match active.poll().unwrap() {
            InvocationProgress::Running => {}
            InvocationProgress::Definition(request) => return request,
            other => panic!("expected definition: {other}"),
        }
    }
    panic!("no definition request")
}
fn prepared<'a>(
    request: &NewtypeBuildRequest,
    next: u32,
    committed: &'a Cell<usize>,
    change: bool,
) -> PreparedNewtypeBuild<'a> {
    PreparedNewtypeBuild {
        definitions: vec![nir::newtypes::Newtype {
            declaration: id(next),
            name: request.name.clone(),
            type_parameters: vec![],
            parameter_types: vec![],
            capture_count: 0,
            requirements: vec![],
            capabilities: vec![],
            representation: if change {
                ty(Primitive::Char)
            } else {
                request.representation.clone()
            },
        }]
        .into(),
        result: id(next),
        commit: Box::new(move || {
            committed.set(committed.get() + 1);
            Ok(())
        }),
    }
}

#[test]
fn definitions_wait_prepare_and_install_once_in_original_execution_order() {
    let package = Package::default();
    let prepares = Cell::new(0);
    let commits = Cell::new(0);
    let builder = |r: &NewtypeBuildRequest, _: &mut u64| {
        prepares.set(prepares.get() + 1);
        Ok(prepared(r, 100 + prepares.get() as u32, &commits, false))
    };
    let mut vm = Interpreter::for_comptime(
        &package,
        Budget {
            steps: 100_000,
            depth: 128,
        },
    )
    .unwrap()
    .with_newtype_builder(&builder);
    let mut active = vm.start_invocation(body(2), vec![]).unwrap();
    for completed in 0..2 {
        let request = request(&mut active);
        assert_eq!(prepares.get(), completed);
        assert_eq!(commits.get(), completed);
        assert!(std::ptr::eq(request.source_world(), &package));
        assert!(request.to_string().contains("(newtype-build"));
        let paused = active.to_string();
        for _ in 0..3 {
            assert!(matches!(
                active.poll().unwrap(),
                InvocationProgress::Waiting
            ));
            assert_eq!(active.remaining_steps(), 0);
            assert_eq!(active.to_string(), paused);
        }
        let response = request.prepare();
        assert_eq!(prepares.get(), completed + 1);
        assert_eq!(commits.get(), completed);
        active.resume_definition(response).unwrap();
        assert_eq!(commits.get(), completed);
    }
    assert_eq!(finish(&mut active).unwrap(), Value::Bool(true));
    assert_eq!(commits.get(), 2);
}

#[test]
fn definition_responses_reject_wrong_owners_and_remain_deliverable() {
    let package = Package::default();
    let commits = Cell::new(0);
    let builder = |r: &NewtypeBuildRequest, _: &mut u64| Ok(prepared(r, 101, &commits, false));
    let mut first = Interpreter::for_comptime(
        &package,
        Budget {
            steps: 100_000,
            depth: 128,
        },
    )
    .unwrap()
    .with_newtype_builder(&builder);
    let mut second = Interpreter::for_comptime(
        &package,
        Budget {
            steps: 100_000,
            depth: 128,
        },
    )
    .unwrap()
    .with_newtype_builder(&builder);
    let mut a = first.start_invocation(body(1), vec![]).unwrap();
    let mut b = second.start_invocation(body(1), vec![]).unwrap();
    let ra = request(&mut a).prepare();
    let rb = request(&mut b).prepare();
    let state = b.to_string();
    let ra = b.resume_definition(ra).unwrap_err().into_response();
    assert_eq!(b.to_string(), state);
    assert_eq!(commits.get(), 0);
    a.resume_definition(ra).unwrap();
    assert_eq!(finish(&mut a).unwrap(), Value::Bool(true));
    b.resume_definition(rb).unwrap();
    assert_eq!(finish(&mut b).unwrap(), Value::Bool(true));
    assert_eq!(commits.get(), 2);
}

#[test]
fn cancellation_before_or_after_response_never_installs_a_definition() {
    for received in [false, true] {
        let package = Package::default();
        let commits = Cell::new(0);
        let builder = |r: &NewtypeBuildRequest, _: &mut u64| Ok(prepared(r, 101, &commits, false));
        let mut vm = Interpreter::for_comptime(
            &package,
            Budget {
                steps: 100_000,
                depth: 128,
            },
        )
        .unwrap()
        .with_newtype_builder(&builder);
        let mut active = vm.start_invocation(body(1), vec![]).unwrap();
        let response = request(&mut active).prepare();
        if received {
            active.resume_definition(response).unwrap();
            active.cancel();
        } else {
            active.cancel();
            assert!(active.resume_definition(response).is_err());
        }
        assert_eq!(commits.get(), 0);
        assert!(active.poll().is_err());
    }
}

#[test]
fn wrong_descriptors_and_inflated_allowances_fail_before_publication() {
    for inflate in [false, true] {
        let package = Package::default();
        let commits = Cell::new(0);
        let builder = |r: &NewtypeBuildRequest, fuel: &mut u64| {
            if inflate {
                *fuel += 1;
            }
            Ok(prepared(r, 101, &commits, !inflate))
        };
        let mut vm = Interpreter::for_comptime(
            &package,
            Budget {
                steps: 100_000,
                depth: 128,
            },
        )
        .unwrap()
        .with_newtype_builder(&builder);
        let mut active = vm.start_invocation(body(1), vec![]).unwrap();
        let response = request(&mut active).prepare();
        active.resume_definition(response).unwrap();
        let failure = finish(&mut active).unwrap_err();
        assert_eq!(
            failure.kind,
            if inflate {
                ErrorKind::StepsExhausted
            } else {
                ErrorKind::Type
            }
        );
        assert_eq!(failure.source, Some(SOURCE));
        assert_eq!(commits.get(), 0);
        if inflate {
            assert_eq!(active.remaining_steps(), 0);
        }
    }
}

#[test]
fn host_rejection_does_not_reach_later_definition_producers() {
    let package = Package::default();
    let prepares = Cell::new(0);
    let builder =
        |r: &NewtypeBuildRequest, _: &mut u64| -> Result<PreparedNewtypeBuild<'_>, Error> {
            prepares.set(prepares.get() + 1);
            Err(Error {
                kind: ErrorKind::Type,
                source: Some(r.source),
            })
        };
    let mut vm = Interpreter::for_comptime(
        &package,
        Budget {
            steps: 100_000,
            depth: 128,
        },
    )
    .unwrap()
    .with_newtype_builder(&builder);
    let mut active = vm.start_invocation(body(2), vec![]).unwrap();
    let response = request(&mut active).prepare();
    active.resume_definition(response).unwrap();
    assert_eq!(finish(&mut active).unwrap_err().kind, ErrorKind::Type);
    assert_eq!(prepares.get(), 1);
}
