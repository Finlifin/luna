use nir::program::{Capture, Expr, ExprKind as K, LocalId, Pattern, SourceRange};
use nir::{DeclarationId, Primitive, Sort, Term};
use std::sync::Arc;
fn id(n: u32) -> DeclarationId {
    DeclarationId {
        package: 1,
        local: n,
    }
}
fn rigid() -> Term {
    Term::Rigid {
        declaration: id(1),
        sort: Sort::Type,
    }
}
fn expr(kind: K) -> Expr {
    Expr {
        ty: rigid(),
        source: SourceRange { start: 1, end: 2 },
        kind,
    }
}
#[test]
fn rewrite_visits_hidden_call_identity_and_pattern_expressions() {
    let method = expr(K::TraitMethod {
        interface: id(2),
        member: id(3),
        subject: rigid(),
        interface_arguments: vec![rigid()],
        method_arguments: vec![rigid()],
    });
    let mut pattern = Pattern::Then {
        first: Box::new(Pattern::Wild),
        value: Box::new(method),
        next: Box::new(Pattern::Guard {
            pattern: Box::new(Pattern::Wild),
            condition: Box::new(expr(K::TypeValue(rigid()))),
        }),
    };
    let mut visits = 0;
    pattern
        .rewrite_types(&mut |ty| {
            visits += 1;
            assert_eq!(*ty, rigid());
            Ok::<_, ()>(Term::Primitive(Primitive::Bool))
        })
        .unwrap();
    assert_eq!(visits, 6);
    let dump = pattern.to_string();
    assert!(!dump.contains("(rigid"));
    assert!(dump.contains("trait-method"));
}
#[test]
fn closure_rewrite_preserves_shared_template_and_capture_identity() {
    let binding = LocalId { owner: 5, index: 0 };
    let body = Arc::new(expr(K::Local(binding)));
    let mut closure = expr(K::Closure {
        captures: vec![Capture {
            binding,
            ty: rigid(),
            value: Some(Box::new(expr(K::TypeValue(rigid())))),
        }],
        body: body.clone(),
        parameters: vec![],
    });
    let original = closure.clone();
    let mut visits = 0;
    closure
        .rewrite_types(&mut |_| {
            visits += 1;
            Ok::<_, ()>(Term::Primitive(Primitive::Char))
        })
        .unwrap();
    assert_eq!(visits, 5);
    assert_eq!(body.ty, rigid());
    assert_ne!(closure, original);
    let K::Closure { captures, body, .. } = closure.kind else {
        panic!()
    };
    assert_eq!(captures[0].binding, binding);
    assert!(matches!(body.kind,K::Local(id) if id==binding));
}

#[test]
fn symbolic_type_lists_concatenate_after_substitution_without_changing_heads() {
    use nir::{DeclarationId, DeclarationParameter, Primitive, Sort, Term};
    let input = DeclarationId {
        package: 91,
        local: 1,
    };
    let head = DeclarationId {
        package: 91,
        local: 2,
    };
    let boolean = Term::Primitive(Primitive::Bool);
    let byte = Term::Primitive(Primitive::U8);
    let list = Term::type_list_concat(vec![
        Term::TypeListValue(vec![boolean.clone()]),
        Term::Rigid {
            declaration: input,
            sort: Sort::ComptimeValue,
        },
        Term::TypeListValue(vec![byte.clone()]),
    ]);
    list.check_scope(&[]).unwrap();
    assert!(list.to_string().contains("type-list-concat"));
    let template = Term::NormalForm {
        head,
        sort: Sort::Type,
        arguments: vec![list],
    };
    for middle in [vec![], vec![byte.clone(), boolean.clone()]] {
        let actual = template
            .instantiate_parameters(
                &[DeclarationParameter {
                    declaration: input,
                    sort: Sort::ComptimeValue,
                }],
                &[Term::TypeListValue(middle.clone())],
            )
            .unwrap();
        let mut elements = vec![boolean.clone()];
        elements.extend(middle);
        elements.push(byte.clone());
        assert_eq!(
            actual,
            Term::NormalForm {
                head,
                sort: Sort::Type,
                arguments: vec![Term::TypeListValue(elements)]
            }
        );
    }
    assert!(
        template
            .instantiate_parameters(
                &[DeclarationParameter {
                    declaration: input,
                    sort: Sort::ComptimeValue
                }],
                &[Term::Bool(true)]
            )
            .is_err()
    );
    assert_eq!(Term::type_list_concat(vec![]), Term::TypeListValue(vec![]));
}

#[test]
fn type_list_index_rewriting_reaches_both_operands_and_keeps_head_identity() {
    let list_head = id(40);
    let tuple_head = id(41);
    let list_type = Term::NormalForm {
        head: list_head,
        sort: Sort::Type,
        arguments: vec![Term::Primitive(Primitive::Type)],
    };
    let nested = Term::NormalForm {
        head: tuple_head,
        sort: Sort::Type,
        arguments: vec![Term::TypeListValue(vec![rigid()])],
    };
    let mut index = Expr {
        ty: Term::Primitive(Primitive::Type),
        source: SourceRange { start: 10, end: 20 },
        kind: K::TypeListIndex(
            Box::new(Expr {
                ty: list_type.clone(),
                source: SourceRange { start: 11, end: 12 },
                kind: K::TypeListValue(vec![nested]),
            }),
            Box::new(Expr {
                ty: Term::Primitive(Primitive::Integer),
                source: SourceRange { start: 13, end: 14 },
                kind: K::Local(LocalId { owner: 2, index: 0 }),
            }),
        ),
    };
    index = index
        .instantiate_types(&[id(1)], &[Term::Primitive(Primitive::U8)])
        .unwrap();
    let mut sources = Vec::new();
    nir::visit::expression(&index, &mut 100, |expr, _| {
        sources.push(expr.source.start);
        Ok(())
    })
    .unwrap();
    sources.sort();
    assert_eq!(sources, [10, 11, 13]);
    nir::visit::expression_mut(&mut index, &mut 100, |expr, _| {
        if matches!(expr.kind, K::Local(_)) {
            expr.kind = K::Literal(nir::program::Literal::Integer("0".into()));
        }
        Ok(())
    })
    .unwrap();
    let K::TypeListIndex(list, position) = &index.kind else {
        panic!("index")
    };
    assert_eq!(list.ty, list_type);
    assert_eq!(
        list.kind,
        K::TypeListValue(vec![Term::NormalForm {
            head: tuple_head,
            sort: Sort::Type,
            arguments: vec![Term::TypeListValue(vec![Term::Primitive(Primitive::U8)])]
        }])
    );
    assert!(matches!(position.kind, K::Literal(_)));
    assert!(index.to_string().contains("(type-list-index (values"));
}

#[test]
fn lexical_rewriting_keeps_order_pruning_and_failure_on_an_ordinary_stack() {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            struct Rewrite {
                visits: Vec<u32>,
                types: usize,
                stop: Option<(u32, bool)>,
            }
            impl nir::program::Rewriter for Rewrite {
                type Error = [u64; 8];
                fn expression(&mut self, expression: &mut Expr) -> Result<bool, Self::Error> {
                    let index = expression.source.start;
                    self.visits.push(index);
                    if let Some((stop, fail)) = self.stop {
                        if stop == index {
                            return if fail { Err([42; 8]) } else { Ok(false) };
                        }
                    }
                    Ok(true)
                }
                fn ty(&mut self, _: &Term) -> Result<Term, Self::Error> {
                    self.types += 1;
                    Ok(Term::Primitive(Primitive::Bool))
                }
            }
            let node = |index, kind| Expr {
                source: SourceRange {
                    start: index,
                    end: index + 1,
                },
                ty: rigid(),
                kind,
            };
            for stop in [None, Some((64, false)), Some((64, true))] {
                let mut tree = node(384, K::Unit);
                for index in (0..192).rev() {
                    let statements = vec![node(index * 2 + 1, K::Unit)];
                    let result = Some(Box::new(tree));
                    tree = node(
                        index * 2,
                        if index % 2 == 0 {
                            K::Block { statements, result }
                        } else {
                            K::CallPreparation { statements, result }
                        },
                    );
                }
                let mut rewrite = Rewrite {
                    visits: vec![],
                    types: 0,
                    stop,
                };
                let result = tree.rewrite_with(&mut rewrite);
                assert_eq!(result.is_err(), stop.is_some_and(|(_, fail)| fail));
                let count = if stop.is_some() { 65 } else { 385 };
                assert_eq!(rewrite.visits, (0..count).collect::<Vec<_>>());
                assert_eq!(rewrite.types, if stop.is_some() { 64 } else { 385 });
                assert_eq!(tree.ty, Term::Primitive(Primitive::Bool));
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn work_stack_preserves_preorder_pruning_and_failure_boundaries() {
    fn node(index: u32, kind: K) -> Expr {
        let mut value = expr(kind);
        value.source.start = index;
        value
    }
    fn tree() -> Expr {
        node(
            1,
            K::Block {
                statements: vec![node(2, K::Discard(Box::new(node(3, K::Unit))))],
                result: Some(Box::new(node(
                    4,
                    K::And(
                        Box::new(node(5, K::Not(Box::new(node(6, K::Unit))))),
                        Box::new(node(7, K::Unit)),
                    ),
                ))),
            },
        )
    }
    struct Rewrite {
        visits: Vec<u32>,
        stop: Option<u32>,
    }
    impl nir::program::Rewriter for Rewrite {
        type Error = u32;
        fn expression(&mut self, value: &mut Expr) -> Result<bool, u32> {
            let index = value.source.start;
            self.visits.push(index);
            if self.stop == Some(index) {
                return Err(index);
            }
            Ok(index != 5)
        }
        fn ty(&mut self, _: &Term) -> Result<Term, u32> {
            Ok(Term::Primitive(Primitive::Bool))
        }
    }
    let mut rewrite = Rewrite {
        visits: vec![],
        stop: Some(4),
    };
    assert_eq!(tree().rewrite_with(&mut rewrite), Err(4));
    assert_eq!(rewrite.visits, [1, 2, 3, 4]);
    rewrite.visits.clear();
    rewrite.stop = None;
    let mut value = tree();
    value.rewrite_with(&mut rewrite).unwrap();
    assert_eq!(rewrite.visits, [1, 2, 3, 4, 5, 7]);
    let K::Block {
        result: Some(result),
        ..
    } = value.kind
    else {
        unreachable!()
    };
    let K::And(left, right) = result.kind else {
        unreachable!()
    };
    assert_eq!(left.ty, rigid());
    assert_eq!(right.ty, Term::Primitive(Primitive::Bool));
}

#[test]
fn conditional_call_and_aggregate_children_keep_source_order() {
    fn node(index: u32, kind: K) -> Expr {
        let mut value = expr(kind);
        value.source.start = index;
        value
    }
    struct Order(Vec<u32>);
    impl nir::program::Rewriter for Order {
        type Error = ();
        fn expression(&mut self, value: &mut Expr) -> Result<bool, ()> {
            self.0.push(value.source.start);
            Ok(true)
        }
        fn ty(&mut self, ty: &Term) -> Result<Term, ()> {
            Ok(ty.clone())
        }
    }
    let mut tree = node(
        1,
        K::If {
            condition: Box::new(node(2, K::Unit)),
            then_branch: Box::new(node(
                3,
                K::Call {
                    callee: Box::new(node(4, K::Function(id(1)))),
                    arguments: vec![
                        nir::program::CallArgument {
                            parameter: 1,
                            value: node(5, K::Tuple(vec![node(6, K::Unit), node(7, K::Unit)])),
                            is_default: false,
                        },
                        nir::program::CallArgument {
                            parameter: 0,
                            value: node(8, K::Return(Some(Box::new(node(9, K::Unit))))),
                            is_default: true,
                        },
                    ],
                    parameter_bindings: vec![],
                },
            )),
            else_branch: Some(Box::new(node(
                10,
                K::ClosureType {
                    environment: vec![node(11, K::Unit), node(12, K::Unit)],
                    signature: Box::new(node(13, K::Unit)),
                },
            ))),
        },
    );
    let mut order = Order(vec![]);
    tree.rewrite_with(&mut order).unwrap();
    assert_eq!(order.0, (1..=13).collect::<Vec<_>>());
}

#[test]
fn capture_failure_does_not_rewrite_later_captures_or_clone_shared_body() {
    let body = Arc::new(expr(K::Unit));
    let mut value = expr(K::Closure {
        parameters: vec![],
        captures: (0..2)
            .map(|index| Capture {
                binding: LocalId { owner: 1, index },
                ty: rigid(),
                value: Some(Box::new(expr(K::Unit))),
            })
            .collect(),
        body: body.clone(),
    });
    struct Stop;
    impl nir::program::Rewriter for Stop {
        type Error = ();
        fn expression(&mut self, value: &mut Expr) -> Result<bool, ()> {
            if matches!(value.kind, K::Unit) {
                Err(())
            } else {
                Ok(true)
            }
        }
        fn ty(&mut self, _: &Term) -> Result<Term, ()> {
            Ok(Term::Primitive(Primitive::Bool))
        }
    }
    assert_eq!(value.rewrite_with(&mut Stop), Err(()));
    let K::Closure {
        captures,
        body: current,
        ..
    } = &value.kind
    else {
        unreachable!()
    };
    assert_eq!(captures[0].ty, Term::Primitive(Primitive::Bool));
    assert_eq!(captures[0].value.as_ref().unwrap().ty, rigid());
    assert_eq!(captures[1].ty, rigid());
    assert!(Arc::ptr_eq(current, &body));
    value
        .rewrite_types(&mut |_| Ok::<_, ()>(Term::Primitive(Primitive::Bool)))
        .unwrap();
    let K::Closure {
        captures,
        body: current,
        ..
    } = &value.kind
    else {
        unreachable!()
    };
    assert!(
        captures
            .iter()
            .all(|capture| capture.value.as_ref().unwrap().ty == Term::Primitive(Primitive::Bool))
    );
    assert!(!Arc::ptr_eq(current, &body));
    assert_eq!(current.ty, Term::Primitive(Primitive::Bool));
    assert_eq!(body.ty, rigid());
}

#[test]
fn struct_reflection_rewrites_and_visits_its_subject() {
    for names in [false, true] {
        let subject = Box::new(expr(K::TypeValue(rigid())));
        let mut query = expr(if names {
            K::StructFieldNames(subject)
        } else {
            K::StructFieldTypes(subject)
        });
        let original = query.clone();
        let mut visits = 0;
        query
            .rewrite_types(&mut |_| {
                visits += 1;
                Ok::<_, ()>(Term::Primitive(Primitive::Bool))
            })
            .unwrap();
        assert_eq!(visits, 3);
        assert!(!query.to_string().contains("(rigid"));
        assert_ne!(query, original);
        let mut nodes = 0;
        nir::visit::expression(&query, &mut 100, |_, _| {
            nodes += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(nodes, 2);
    }
}
