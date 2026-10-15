use nir::program::{CallArgument, Expr, ExprKind as K, Literal, LocalId, Pattern, SourceRange};
use nir::{DeclarationId, Parameter, Primitive, Sort, Term};

fn boolean() -> Term {
    Term::Primitive(Primitive::Bool)
}
fn local(index: u32) -> LocalId {
    LocalId { owner: 1, index }
}
fn expr(kind: K, ty: Term) -> Expr {
    Expr {
        kind,
        ty,
        source: SourceRange { start: 0, end: 1 },
    }
}
fn read(index: u32) -> Expr {
    expr(K::Local(local(index)), boolean())
}
fn value(flag: bool) -> Expr {
    expr(K::Literal(Literal::Bool(flag)), boolean())
}
fn bind(index: u32, value: Expr) -> Expr {
    expr(
        K::Let {
            binding: local(index),
            value: Box::new(value),
        },
        Term::Primitive(Primitive::Void),
    )
}
fn callable(id: u32, stages: &[Sort], result: Term, ct: bool) -> Expr {
    let ty = Term::Function {
        parameters: stages
            .iter()
            .map(|&sort| Parameter {
                sort,
                ty: boolean(),
            })
            .collect(),
        result: Box::new(result),
    };
    expr(
        K::Function(DeclarationId {
            package: 1,
            local: id,
        }),
        if ct {
            Term::ComptimeFunction(Box::new(ty))
        } else {
            ty
        },
    )
}
fn call(callee: Expr, values: Vec<Expr>, ty: Term) -> Expr {
    expr(
        K::Call {
            callee: Box::new(callee),
            arguments: values
                .into_iter()
                .enumerate()
                .map(|(parameter, value)| CallArgument {
                    parameter,
                    value,
                    is_default: false,
                })
                .collect(),
            parameter_bindings: vec![],
        },
        ty,
    )
}

#[test]
fn selected_packet_has_one_producer_and_preserves_runtime_siblings() {
    let pair = Term::Tuple(vec![boolean(), boolean()]);
    let producer = call(
        callable(10, &[Sort::Runtime], pair.clone(), true),
        vec![read(0)],
        pair,
    );
    let unpack = expr(
        K::LetPattern {
            pattern: Pattern::Tuple(
                vec![1, 2]
                    .into_iter()
                    .map(|id| Pattern::Binding {
                        binding: local(id),
                        inner: Box::new(Pattern::Wild),
                    })
                    .collect(),
            ),
            value: Box::new(producer),
            guard: None,
            otherwise: None,
        },
        Term::Primitive(Primitive::Void),
    );
    let runtime = bind(
        3,
        call(callable(11, &[], boolean(), false), vec![], boolean()),
    );
    let mut input = expr(
        K::CallPreparation {
            statements: vec![
                bind(0, value(true)),
                unpack,
                runtime.clone(),
                bind(4, read(1)),
                bind(5, read(2)),
            ],
            result: Some(Box::new(call(
                callable(
                    12,
                    &[Sort::ComptimeValue, Sort::Runtime, Sort::Runtime],
                    boolean(),
                    false,
                ),
                vec![read(4), read(5), read(3)],
                boolean(),
            ))),
        },
        boolean(),
    );
    let plan = iir::staging::preparation(&input, &mut 10000)
        .unwrap()
        .unwrap();
    assert_eq!(plan.indices.into_iter().collect::<Vec<_>>(), vec![0, 1, 3]);
    let plan = iir::staging::preparation(&input, &mut 10000)
        .unwrap()
        .unwrap();
    let mut producers = 0;
    iir::visit::expression(&plan.expression, &mut 10000, |expr, _| {
        if matches!(expr.kind,K::Function(id) if id.local==10) {
            producers += 1;
        }
        assert!(!matches!(expr.kind,K::Function(id) if id.local==11));
        Ok(())
    })
    .unwrap();
    assert_eq!(producers, 1);
    let completed = expr(
        K::Tuple(vec![value(false), value(true)]),
        plan.expression.ty.clone(),
    );
    plan.apply(&mut input, completed, &mut 10000).unwrap();
    assert!(
        iir::staging::preparation(&input, &mut 10000)
            .unwrap()
            .is_none()
    );
    let K::CallPreparation {
        statements,
        result: Some(result),
    } = &input.kind
    else {
        panic!()
    };
    assert_eq!(statements.len(), 3);
    assert_eq!(statements[1], runtime);
    assert!(matches!(&statements[0].kind,K::Let {binding,..} if *binding==local(2)));
    let K::Call { arguments, .. } = &result.kind else {
        panic!()
    };
    assert_eq!(arguments[0].value, value(true));
    assert_eq!(arguments[1].value, read(5));
    assert!(iir::staging::preparation(&input, &mut 0).is_err());
}

#[test]
fn selection_only_type_bindings_do_not_become_runtime_storage() {
    let ty = Term::Primitive(Primitive::Type);
    let type_value = expr(K::TypeValue(boolean()), ty.clone());
    let callee = expr(
        K::Function(DeclarationId {
            package: 1,
            local: 30,
        }),
        Term::Function {
            parameters: vec![Parameter {
                sort: Sort::ComptimeValue,
                ty: ty.clone(),
            }],
            result: Box::new(boolean()),
        },
    );
    let mut input = expr(
        K::CallPreparation {
            statements: vec![bind(0, type_value.clone())],
            result: Some(Box::new(call(
                callee,
                vec![expr(K::Local(local(0)), ty.clone())],
                boolean(),
            ))),
        },
        boolean(),
    );
    let plan = iir::staging::preparation(&input, &mut 1000)
        .unwrap()
        .unwrap();
    let completed = expr(K::Tuple(vec![type_value]), Term::Tuple(vec![ty]));
    plan.apply(&mut input, completed, &mut 1000).unwrap();
    let K::CallPreparation { statements, .. } = input.kind else {
        panic!()
    };
    assert!(statements.is_empty());
}
