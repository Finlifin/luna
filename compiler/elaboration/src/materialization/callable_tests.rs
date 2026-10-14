use super::*;
use nir::{Parameter, Primitive, Sort};

#[test]
fn conditional_call_resolution_preserves_uncompleted_inputs_and_scopes() {
    let (base, binding, signature) = fixture();
    let K::Block {
        result: Some(result),
        ..
    } = base.kind
    else {
        unreachable!()
    };
    let mut base = *result;
    let K::Call { callee, .. } = &mut base.kind else {
        unreachable!()
    };
    let left = Box::new(expr(
        K::Function(Id {
            package: 0,
            local: 1,
        }),
        signature.clone(),
    ));
    let right = Box::new(expr(
        K::Function(Id {
            package: 0,
            local: 2,
        }),
        signature.clone(),
    ));
    *callee = Box::new(expr(
        K::If {
            condition: Box::new(expr(
                K::Literal(n::Literal::Bool(true)),
                Term::Primitive(Primitive::Bool),
            )),
            then_branch: left,
            else_branch: Some(right),
        },
        signature.clone(),
    ));
    for boundary in 0..3 {
        let mut candidate = base.clone();
        let K::Call {
            callee, arguments, ..
        } = &mut candidate.kind
        else {
            unreachable!()
        };
        match boundary {
            0 => arguments[0].value = expr(K::Local(binding), Term::Primitive(Primitive::Type)),
            1 => arguments[0].is_default = true,
            _ => {
                let K::If { then_branch, .. } = &mut callee.kind else {
                    unreachable!()
                };
                *then_branch = Box::new(expr(
                    K::Block {
                        statements: vec![expr(K::Unit, Term::Primitive(Primitive::Void))],
                        result: Some(then_branch.clone()),
                    },
                    signature.clone(),
                ));
            }
        }
        let original = candidate.clone();
        Builder::resolve_conditional_callable(&mut candidate, &mut 1000).unwrap();
        assert_eq!(candidate, original, "boundary {boundary}");
    }
    assert!(Builder::resolve_conditional_callable(&mut base.clone(), &mut 0).is_err());
    let mut completed = base;
    Builder::resolve_conditional_callable(&mut completed, &mut 1000).unwrap();
    assert!(matches!(completed.kind, K::If { .. }));
}

fn expr(kind: K, ty: Term) -> n::Expr {
    n::Expr {
        source: n::SourceRange { start: 0, end: 1 },
        kind,
        ty,
    }
}

fn fixture() -> (n::Expr, n::LocalId, Term) {
    let binding = n::LocalId { owner: 1, index: 0 };
    let signature = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Type,
            ty: Term::Primitive(Primitive::Type),
        }],
        result: Box::new(Term::Primitive(Primitive::Bool)),
    };
    let result = expr(
        K::Call {
            callee: Box::new(expr(K::Local(binding), signature.clone())),
            parameter_bindings: vec![],
            arguments: vec![n::CallArgument {
                parameter: 0,
                is_default: false,
                value: expr(
                    K::TypeValue(Term::Primitive(Primitive::Bool)),
                    Term::Primitive(Primitive::Type),
                ),
            }],
        },
        Term::Primitive(Primitive::Bool),
    );
    (
        expr(
            K::Block {
                statements: vec![expr(
                    K::Let {
                        binding,
                        value: Box::new(expr(
                            K::Function(Id {
                                package: 0,
                                local: 1,
                            }),
                            signature.clone(),
                        )),
                    },
                    Term::Primitive(Primitive::Void),
                )],
                result: Some(Box::new(result)),
            },
            Term::Primitive(Primitive::Bool),
        ),
        binding,
        signature,
    )
}

#[test]
fn callable_resolution_preserves_observable_bindings_and_statement_order() {
    let (base, binding, signature) = fixture();
    let observers = [
        expr(K::Local(binding), signature.clone()),
        expr(
            K::Assign {
                place: n::Place {
                    root: binding,
                    fields: vec![],
                },
                value: Box::new(expr(
                    K::Function(Id {
                        package: 0,
                        local: 2,
                    }),
                    signature.clone(),
                )),
            },
            Term::Primitive(Primitive::Void),
        ),
        expr(
            K::Closure {
                parameters: vec![],
                captures: vec![n::Capture {
                    binding,
                    ty: signature.clone(),
                    value: None,
                }],
                body: std::sync::Arc::new(expr(K::Local(binding), signature.clone())),
            },
            Term::Closure {
                environment: vec![signature.clone()],
                signature: Box::new(Term::Function {
                    parameters: vec![],
                    result: Box::new(signature),
                }),
            },
        ),
    ];
    for observer in observers {
        let mut candidate = base.clone();
        let K::Block {
            result: Some(result),
            ..
        } = &mut candidate.kind
        else {
            unreachable!()
        };
        let K::Call { arguments, .. } = &mut result.kind else {
            unreachable!()
        };
        // The ordinary argument contains the observation in a nested scope.
        let value = arguments[0].value.clone();
        arguments[0].value = expr(
            K::Block {
                statements: vec![observer],
                result: Some(Box::new(value.clone())),
            },
            value.ty,
        );
        let before = candidate.clone();
        Builder::resolve_immediate_callable(&mut candidate, &mut 1000).unwrap();
        assert_eq!(candidate, before);
    }
    let mut candidate = base;
    let K::Block { statements, .. } = &mut candidate.kind else {
        unreachable!()
    };
    statements.push(expr(K::Unit, Term::Primitive(Primitive::Void)));
    let before = candidate.clone();
    Builder::resolve_immediate_callable(&mut candidate, &mut 1000).unwrap();
    assert_eq!(candidate, before);
}

#[test]
fn callable_resolution_is_metered_and_keeps_arguments() {
    let (base, _, _) = fixture();
    assert!(Builder::resolve_immediate_callable(&mut base.clone(), &mut 0).is_err());
    let mut candidate = base.clone();
    let mut fuel = 100;
    Builder::resolve_immediate_callable(&mut candidate, &mut fuel).unwrap();
    assert!(fuel < 100);
    let K::Block {
        statements,
        result: Some(result),
    } = &candidate.kind
    else {
        unreachable!()
    };
    assert!(statements.is_empty());
    let K::Call {
        callee, arguments, ..
    } = &result.kind
    else {
        unreachable!()
    };
    assert!(matches!(
        callee.kind,
        K::Function(Id {
            package: 0,
            local: 1
        })
    ));
    let K::Block {
        result: Some(original),
        ..
    } = &base.kind
    else {
        unreachable!()
    };
    let K::Call {
        arguments: original,
        ..
    } = &original.kind
    else {
        unreachable!()
    };
    assert_eq!(arguments, original);
}
