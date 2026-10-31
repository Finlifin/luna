use nir::program::{CallArgument, Capture, Expr, ExprKind as K, LocalId, Pattern, SourceRange};
use nir::{Primitive, Sort, Term};
use std::{collections::BTreeMap, sync::Arc};

fn id(index: u32) -> LocalId {
    LocalId { owner: 1, index }
}
fn expr(kind: K) -> Expr {
    Expr {
        kind,
        ty: Term::Primitive(Primitive::Bool),
        source: SourceRange { start: 10, end: 20 },
    }
}
fn local(index: u32) -> Expr {
    expr(K::Local(id(index)))
}
fn copy(body: &mut Expr, next: &mut u32, sources: &mut Vec<nir::program::ExpansionSource>) {
    nir::freshening::copied_expression(body, next, sources, &mut 10000).unwrap();
}

#[test]
fn copied_bindings_are_disjoint_but_lexical_inputs_stay_connected() {
    let template = expr(K::Block {
        statements: vec![expr(K::Let {
            binding: id(2),
            value: Box::new(local(1)),
        })],
        result: Some(Box::new(local(2))),
    });
    let mut next = 100;
    let mut sources = vec![];
    let mut copies = vec![template.clone(), template];
    let mut bindings = vec![];
    for body in &mut copies {
        copy(body, &mut next, &mut sources);
        let K::Block {
            statements,
            result: Some(result),
        } = &body.kind
        else {
            panic!()
        };
        let K::Let { binding, value } = &statements[0].kind else {
            panic!()
        };
        assert!(matches!(value.kind, K::Local(input) if input == id(1)));
        assert!(matches!(result.kind, K::Local(output) if output == *binding));
        bindings.push(*binding);
        assert_eq!(
            nir::free_bindings::collect(body, &mut 1000)
                .unwrap()
                .into_iter()
                .collect::<Vec<_>>(),
            vec![id(1)]
        );
    }
    assert_ne!(bindings[0], bindings[1]);
    assert_eq!(next, 102);
    assert_ne!(copies[0].source, copies[1].source);
    assert!(
        sources
            .iter()
            .all(|source| source.site == SourceRange { start: 10, end: 20 })
    );
}

#[test]
fn copied_loops_keep_external_transfers_and_separate_cleanup_scopes() {
    let mut body = expr(K::Block {
        statements: vec![expr(K::Loop {
            identity: id(3),
            body: Box::new(expr(K::Block {
                statements: vec![expr(K::Continue(id(3))), expr(K::Break(id(9)))],
                result: None,
            })),
        })],
        result: None,
    });
    let mut sources = vec![];
    copy(&mut body, &mut 100, &mut sources);
    let K::Block { statements, .. } = &body.kind else {
        panic!()
    };
    let K::Loop {
        identity,
        body: inner,
    } = &statements[0].kind
    else {
        panic!()
    };
    let K::Block { statements, .. } = &inner.kind else {
        panic!()
    };
    assert_ne!(*identity, id(3));
    assert!(matches!(statements[0].kind, K::Continue(target) if target == *identity));
    assert!(matches!(statements[1].kind, K::Break(target) if target == id(9)));
    assert_ne!(
        body.source, inner.source,
        "equal source spans cannot merge cleanup scopes"
    );
}

#[test]
fn closure_capture_cells_are_private_and_defaults_keep_the_callee_frame() {
    let mut closure = expr(K::Closure {
        parameters: vec![id(2)],
        captures: vec![Capture {
            binding: id(1),
            ty: Term::Primitive(Primitive::Bool),
            value: None,
        }],
        body: Arc::new(expr(K::Block {
            statements: vec![local(1)],
            result: Some(Box::new(local(2))),
        })),
    });
    copy(&mut closure, &mut 100, &mut vec![]);
    let K::Closure {
        parameters,
        captures,
        body,
    } = closure.kind
    else {
        panic!()
    };
    assert_ne!(captures[0].binding, id(1));
    assert!(matches!(captures[0].value.as_ref().unwrap().kind, K::Local(input) if input == id(1)));
    let K::Block {
        statements,
        result: Some(result),
    } = &body.kind
    else {
        panic!()
    };
    assert!(matches!(statements[0].kind, K::Local(input) if input == captures[0].binding));
    assert!(matches!(result.kind, K::Local(input) if input == parameters[0]));

    let mut call = expr(K::Call {
        callee: Box::new(local(8)),
        parameter_bindings: vec![id(1), id(2)],
        arguments: vec![
            CallArgument {
                passing: Default::default(),
                parameter: 0,
                value: local(1),
                is_default: false,
            },
            CallArgument {
                passing: Default::default(),
                parameter: 1,
                value: local(1),
                is_default: true,
            },
        ],
    });
    nir::freshening::expression(
        &mut call,
        BTreeMap::from([(id(1), id(99)), (id(8), id(8))]),
        &mut 100,
        &mut vec![],
        SourceRange { start: 1, end: 2 },
    )
    .unwrap();
    let K::Call {
        arguments,
        parameter_bindings,
        ..
    } = call.kind
    else {
        panic!()
    };
    assert!(matches!(arguments[0].value.kind, K::Local(input) if input == id(99)));
    assert!(matches!(arguments[1].value.kind, K::Local(input) if input == id(1)));
    assert_eq!(parameter_bindings, vec![id(1), id(2)]);
}

#[test]
fn pattern_bindings_and_refusals_preserve_the_existing_limits() {
    let site = SourceRange { start: 10, end: 20 };
    let mut pattern = Pattern::Binding {
        binding: id(1),
        inner: Box::new(Pattern::Wild),
    };
    nir::freshening::pattern(&mut pattern, BTreeMap::new(), &mut 100, &mut vec![], site).unwrap();
    assert!(
        matches!(pattern, Pattern::Binding { binding, .. } if binding == LocalId { owner: u32::MAX, index: 100 })
    );
    let input = expr(K::Let {
        binding: id(2),
        value: Box::new(local(1)),
    });
    assert!(
        nir::freshening::copied_expression(&mut input.clone(), &mut 100, &mut vec![], &mut 0)
            .is_err()
    );
    let mut exhausted_identity = u32::MAX;
    let error = nir::freshening::copied_expression(
        &mut input.clone(),
        &mut exhausted_identity,
        &mut vec![],
        &mut 1000,
    )
    .unwrap_err();
    assert!(error.starts_with("10..20: code expansion identity space exhausted"));
    copy(&mut input.clone(), &mut 100, &mut vec![]);
    let mut dependent = local(1);
    dependent.ty = Term::Bound {
        depth: 0,
        index: 0,
        sort: Sort::Runtime,
    };
    assert!(
        nir::freshening::expression(&mut dependent, BTreeMap::new(), &mut 100, &mut vec![], site)
            .unwrap_err()
            .contains("runtime-dependent quoted type")
    );
}
