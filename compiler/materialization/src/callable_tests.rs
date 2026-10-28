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
                        result: Some(Box::new(expr(K::Local(binding), signature.clone()))),
                    },
                    signature.clone(),
                ));
            }
        }
        let original = candidate.clone();
        Builder::resolve_conditional_callable(
            &mut candidate,
            false,
            &mut 100,
            &mut Vec::new(),
            &mut 1000,
        )
        .unwrap();
        assert_eq!(candidate, original, "boundary {boundary}");
    }
    assert!(
        Builder::resolve_conditional_callable(
            &mut base.clone(),
            false,
            &mut 100,
            &mut Vec::new(),
            &mut 0
        )
        .is_err()
    );
    let mut completed = base;
    Builder::resolve_conditional_callable(
        &mut completed,
        false,
        &mut 100,
        &mut Vec::new(),
        &mut 1000,
    )
    .unwrap();
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
                passing: Default::default(),
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
        Builder::resolve_immediate_callable(&mut candidate, false, &mut 1000).unwrap();
        assert_eq!(candidate, before);
    }
    let mut candidate = base;
    let K::Block { statements, .. } = &mut candidate.kind else {
        unreachable!()
    };
    statements.push(expr(K::Unit, Term::Primitive(Primitive::Void)));
    let before = candidate.clone();
    Builder::resolve_immediate_callable(&mut candidate, false, &mut 1000).unwrap();
    assert_eq!(candidate, before);
}

#[test]
fn callable_resolution_is_metered_and_keeps_arguments() {
    let (base, _, _) = fixture();
    assert!(Builder::resolve_immediate_callable(&mut base.clone(), false, &mut 0).is_err());
    let mut candidate = base.clone();
    let mut fuel = 100;
    Builder::resolve_immediate_callable(&mut candidate, false, &mut fuel).unwrap();
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

#[test]
fn runtime_alias_resolution_is_opt_in_and_preserves_argument_observations() {
    let (mut base, binding, old_signature) = fixture();
    let signature = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Runtime,
            ty: Term::Primitive(Primitive::Bool),
        }],
        result: Box::new(Term::Primitive(Primitive::Bool)),
    };
    base.rewrite_types(&mut |ty| {
        Ok::<_, String>(if *ty == old_signature {
            signature.clone()
        } else {
            ty.clone()
        })
    })
    .unwrap();
    let K::Block {
        result: Some(result),
        ..
    } = &mut base.kind
    else {
        unreachable!()
    };
    let K::Call { arguments, .. } = &mut result.kind else {
        unreachable!()
    };
    arguments[0].value = expr(
        K::Literal(n::Literal::Bool(true)),
        Term::Primitive(Primitive::Bool),
    );
    let mut candidate = base.clone();
    Builder::resolve_immediate_callable(&mut candidate, false, &mut 1000).unwrap();
    assert_eq!(candidate, base);
    Builder::resolve_immediate_callable(&mut candidate, true, &mut 1000).unwrap();
    let K::Block { statements, .. } = &candidate.kind else {
        unreachable!()
    };
    assert!(statements.is_empty());
    let K::Block {
        result: Some(result),
        ..
    } = &mut base.kind
    else {
        unreachable!()
    };
    let K::Call { arguments, .. } = &mut result.kind else {
        unreachable!()
    };
    arguments[0].value = expr(
        K::Block {
            statements: vec![expr(K::Local(binding), signature)],
            result: Some(Box::new(arguments[0].value.clone())),
        },
        Term::Primitive(Primitive::Bool),
    );
    let original = base.clone();
    Builder::resolve_immediate_callable(&mut base, true, &mut 1000).unwrap();
    assert_eq!(base, original);
}

#[test]
fn conditional_copy_failure_restores_expression_and_expansion_metadata() {
    let (base, _, _) = fixture();
    let K::Block {
        result: Some(result),
        ..
    } = base.kind
    else {
        unreachable!()
    };
    let mut candidate = *result;
    let K::Call {
        callee, arguments, ..
    } = &mut candidate.kind
    else {
        unreachable!()
    };
    let Term::Function { parameters, .. } = &mut callee.ty else {
        unreachable!()
    };
    parameters.push(Parameter {
        sort: Sort::Runtime,
        ty: Term::Primitive(Primitive::Bool),
    });
    let signature = callee.ty.clone();
    let target = Box::new(expr(
        K::Function(Id {
            package: 0,
            local: 1,
        }),
        signature.clone(),
    ));
    *callee = Box::new(expr(
        K::If {
            condition: Box::new(expr(
                K::Literal(n::Literal::Bool(true)),
                Term::Primitive(Primitive::Bool),
            )),
            then_branch: target.clone(),
            else_branch: Some(target),
        },
        signature,
    ));
    let local = n::LocalId { owner: 2, index: 3 };
    arguments.push(n::CallArgument {
        passing: Default::default(),
        parameter: 1,
        is_default: false,
        value: expr(
            K::Block {
                statements: vec![expr(
                    K::Let {
                        binding: local,
                        value: Box::new(expr(
                            K::Literal(n::Literal::Bool(true)),
                            Term::Primitive(Primitive::Bool),
                        )),
                    },
                    Term::Primitive(Primitive::Void),
                )],
                result: Some(Box::new(expr(
                    K::Local(local),
                    Term::Primitive(Primitive::Bool),
                ))),
            },
            Term::Primitive(Primitive::Bool),
        ),
    });
    for matched in [false, true] {
        let mut candidate = candidate.clone();
        if matched {
            let K::Call { callee, .. } = &mut candidate.kind else {
                unreachable!()
            };
            let K::If {
                condition,
                then_branch,
                else_branch: Some(other),
            } = std::mem::replace(&mut callee.kind, K::Unit)
            else {
                unreachable!()
            };
            callee.kind = K::Match {
                value: condition,
                arms: vec![
                    n::MatchArm {
                        pattern: P::Literal(n::Literal::Bool(true)),
                        guard: None,
                        body: *then_branch,
                    },
                    n::MatchArm {
                        pattern: P::Wild,
                        guard: None,
                        body: *other,
                    },
                ],
            };
        }
        let original = candidate.clone();
        let mut sources = Vec::new();
        let initial_next = u32::MAX - u32::from(matched);
        let mut next = initial_next;
        let failure = Builder::resolve_conditional_callable(
            &mut candidate,
            false,
            &mut next,
            &mut sources,
            &mut 10_000,
        )
        .unwrap_err();
        assert!(failure.contains("identity space exhausted"), "{failure}");
        assert_eq!(candidate, original);
        assert_eq!(next, initial_next);
        assert!(sources.is_empty());
        next = 100;
        Builder::resolve_conditional_callable(
            &mut candidate,
            false,
            &mut next,
            &mut sources,
            &mut 10_000,
        )
        .unwrap();
        assert!(next > 100);
        assert!(!sources.is_empty());
        assert!(matches!(candidate.kind, K::If { .. } | K::Block { .. }));
    }
}

#[test]
fn runtime_conditional_calls_only_expand_in_the_opted_in_context() {
    let (base, _, _) = fixture();
    let K::Block {
        result: Some(result),
        ..
    } = base.kind
    else {
        unreachable!()
    };
    let mut candidate = *result;
    let K::Call {
        callee, arguments, ..
    } = &mut candidate.kind
    else {
        unreachable!()
    };
    let signature = Term::Function {
        parameters: vec![Parameter {
            sort: Sort::Runtime,
            ty: Term::Primitive(Primitive::Bool),
        }],
        result: Box::new(Term::Primitive(Primitive::Bool)),
    };
    let target = Box::new(expr(
        K::Function(Id {
            package: 0,
            local: 1,
        }),
        signature.clone(),
    ));
    *callee = Box::new(expr(
        K::If {
            condition: Box::new(expr(
                K::Literal(n::Literal::Bool(true)),
                Term::Primitive(Primitive::Bool),
            )),
            then_branch: target.clone(),
            else_branch: Some(target),
        },
        signature,
    ));
    arguments[0].value = expr(
        K::Literal(n::Literal::Bool(true)),
        Term::Primitive(Primitive::Bool),
    );
    let original = candidate.clone();
    let mut sources = Vec::new();
    let mut next = 100;
    Builder::resolve_conditional_callable(
        &mut candidate,
        false,
        &mut next,
        &mut sources,
        &mut 10_000,
    )
    .unwrap();
    assert_eq!(candidate, original);
    assert!(sources.is_empty());
    assert_eq!(next, 100);
    Builder::resolve_conditional_callable(
        &mut candidate,
        true,
        &mut next,
        &mut sources,
        &mut 10_000,
    )
    .unwrap();
    assert!(matches!(candidate.kind, K::If { .. }));
    assert!(!sources.is_empty());
}

#[test]
fn template_alias_forwarding_keeps_writes_addresses_and_captures() {
    let (base, binding, signature) = fixture();
    let observers = [
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
            K::AddressOf(Box::new(expr(K::Local(binding), signature.clone()))),
            Term::Pointer(Box::new(signature.clone())),
        ),
        expr(
            K::Closure {
                captures: vec![n::Capture {
                    binding,
                    ty: signature.clone(),
                    value: None,
                }],
                parameters: vec![],
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
        let K::Block { statements, .. } = &mut candidate.kind else {
            unreachable!()
        };
        statements.push(observer);
        let original = candidate.clone();
        staged::forward_template_aliases(&mut candidate, &mut 100, &mut vec![], &mut 10000)
            .unwrap();
        assert_eq!(candidate, original);
    }
    let mut candidate = base.clone();
    staged::forward_template_aliases(&mut candidate, &mut 100, &mut vec![], &mut 10000).unwrap();
    let K::Block {
        statements,
        result: Some(result),
    } = candidate.kind
    else {
        unreachable!()
    };
    assert!(matches!(statements[0].kind, K::Unit));
    let K::Call { callee, .. } = result.kind else {
        unreachable!()
    };
    assert!(matches!(callee.kind, K::Function(_)));
    assert!(
        staged::forward_template_aliases(&mut base.clone(), &mut 100, &mut vec![], &mut 0).is_err()
    );
}

#[test]
fn mutable_template_storage_keeps_address_escape_opaque() {
    let (mut candidate, binding, signature) = fixture();
    let K::Block { statements, .. } = &mut candidate.kind else {
        unreachable!()
    };
    statements.push(expr(
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
    ));
    statements.push(expr(
        K::AddressOf(Box::new(expr(K::Local(binding), signature.clone()))),
        Term::Pointer(Box::new(signature)),
    ));
    let original = candidate.clone();
    mutable_callbacks::lower(&mut candidate, &mut 100, &mut vec![], &mut 10000).unwrap();
    assert_eq!(candidate, original);
    assert!(mutable_callbacks::lower(&mut candidate, &mut 100, &mut vec![], &mut 0).is_err());
}

#[test]
fn mutable_template_components_preserve_escaping_aliases_and_unknown_sources() {
    let (base, binding, signature) = fixture();
    let other = n::LocalId {
        owner: binding.owner,
        index: binding.index + 1,
    };
    for escaping in [false, true] {
        let mut candidate = base.clone();
        let K::Block { statements, .. } = &mut candidate.kind else {
            unreachable!()
        };
        if escaping {
            statements.push(expr(
                K::Let {
                    binding: other,
                    value: Box::new(expr(K::Local(binding), signature.clone())),
                },
                Term::Primitive(Primitive::Void),
            ));
            statements.push(expr(
                K::AddressOf(Box::new(expr(K::Local(other), signature.clone()))),
                Term::Pointer(Box::new(signature.clone())),
            ));
        }
        statements.push(expr(
            K::Assign {
                place: n::Place {
                    root: binding,
                    fields: vec![],
                },
                value: Box::new(expr(K::Local(other), signature.clone())),
            },
            Term::Primitive(Primitive::Void),
        ));
        let original = candidate.clone();
        mutable_callbacks::lower(&mut candidate, &mut 100, &mut vec![], &mut 10000).unwrap();
        assert_eq!(candidate, original);
    }
}

#[test]
fn preparation_pruning_removes_only_completed_void_placeholders() {
    let void = Term::Primitive(Primitive::Void);
    let binding = n::LocalId { owner: 1, index: 0 };
    let runtime_binding = expr(
        K::Let {
            binding,
            value: Box::new(expr(
                K::Block {
                    statements: vec![],
                    result: None,
                },
                void.clone(),
            )),
        },
        void.clone(),
    );
    let result = Some(Box::new(expr(K::Local(binding), void.clone())));
    let mut preparation = expr(
        K::CallPreparation {
            statements: vec![
                expr(K::Unit, void.clone()),
                runtime_binding.clone(),
                expr(K::Unit, void.clone()),
            ],
            result: result.clone(),
        },
        void.clone(),
    );
    dead_tails::prune_type_preparations(&mut preparation, &mut 1000).unwrap();
    assert_eq!(
        preparation,
        expr(
            K::CallPreparation {
                statements: vec![runtime_binding],
                result,
            },
            void.clone()
        )
    );
    // Ordinary blocks retain their sequencing nodes; this repair is scoped to
    // the binding-only protocol of materialized call preparations.
    let mut block = expr(
        K::Block {
            statements: vec![expr(K::Unit, void.clone())],
            result: None,
        },
        void,
    );
    let original = block.clone();
    dead_tails::prune_type_preparations(&mut block, &mut 1000).unwrap();
    assert_eq!(block, original);
}

#[test]
fn mutable_callback_reads_have_distinct_cleanup_sources() {
    let (mut candidate, binding, signature) = fixture();
    let K::Block { statements, .. } = &mut candidate.kind else {
        unreachable!()
    };
    statements.push(expr(
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
    ));
    statements.push(expr(K::Local(binding), signature));
    let original = candidate.clone();
    let mut sources = vec![];
    mutable_callbacks::lower(&mut candidate, &mut 100, &mut sources, &mut 10000).unwrap();
    let mut generated = BTreeSet::new();
    let mut matches = 0;
    nir::visit::expression(&candidate, &mut 10000, |expression, _| {
        if let K::Match { value, arms } = &expression.kind {
            matches += 1;
            assert_ne!(expression.source, value.source);
            for source in
                std::iter::once(value.source).chain(arms.iter().map(|arm| arm.body.source))
            {
                assert_eq!(source.start, u32::MAX);
                assert!(
                    generated.insert(source.end),
                    "generated cleanup sources must be unique"
                );
                assert!((source.end as usize) < sources.len());
            }
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(matches, 2);
    let mut repeated = original;
    let mut repeated_sources = vec![];
    mutable_callbacks::lower(&mut repeated, &mut 100, &mut repeated_sources, &mut 10000).unwrap();
    assert_eq!(candidate, repeated);
    assert_eq!(sources, repeated_sources);
}

#[test]
fn factory_dispatch_separates_wrapper_and_copied_target_sources() {
    let (_, binding, signature) = fixture();
    let value = expr(K::Local(binding), Term::Primitive(Primitive::U64));
    let source = value.source;
    let target = expr(
        K::Function(Id {
            package: 0,
            local: 1,
        }),
        signature.clone(),
    );
    let mut sources = vec![];
    let dispatch = template_factories::dispatch(
        value,
        &signature,
        &[target.clone(), target],
        &mut 100,
        &mut sources,
        &mut 10000,
    )
    .unwrap();
    assert_eq!(dispatch.source, source);
    let K::Match { value, arms } = dispatch.kind else {
        unreachable!()
    };
    let mut generated = BTreeSet::new();
    for source in std::iter::once(value.source).chain(arms.iter().map(|arm| arm.body.source)) {
        assert_eq!(source.start, u32::MAX);
        assert!(generated.insert(source.end));
        assert!((source.end as usize) < sources.len());
    }
}
