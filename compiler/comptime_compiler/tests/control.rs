use comptime_compiler::{Error, compile};
use nir::program::{Expr, ExprKind as K, LocalId, SourceRange};
use nir::{Primitive, Term};
fn expr(kind: K) -> Expr {
    Expr {
        source: SourceRange { start: 1, end: 2 },
        ty: Term::Primitive(Primitive::Void),
        kind,
    }
}
#[test]
fn linear_compilation_has_bounded_work_and_deterministic_dump() {
    let body = expr(K::Block {
        statements: vec![expr(K::Unit)],
        result: Some(Box::new(expr(K::Unit))),
    });
    let first = compile(&body, &mut 100).unwrap().to_string();
    assert!(first.contains("(enter-scope)"));
    assert!(first.contains("(leave-scope)"));
    assert_eq!(first, compile(&body, &mut 100).unwrap().to_string());
    assert!(matches!(compile(&body, &mut 1), Err(Error::Budget(_))));
}
#[test]
fn unsupported_work_and_invalid_transfers_never_publish_partial_code() {
    let body = expr(K::If {
        condition: Box::new(expr(K::Unit)),
        then_branch: Box::new(expr(K::Unit)),
        // Pattern-local executable code still needs suspended continuations.
        else_branch: Some(Box::new(expr(K::Match {
            value: Box::new(expr(K::Unit)),
            arms: vec![nir::program::MatchArm {
                pattern: nir::program::Pattern::Then {
                    first: Box::new(nir::program::Pattern::Wild),
                    value: Box::new(expr(K::Unit)),
                    next: Box::new(nir::program::Pattern::Wild),
                },
                guard: None,
                body: expr(K::Unit),
            }],
        }))),
    });
    assert!(matches!(
        compile(&body, &mut 100),
        Err(Error::Unsupported(_))
    ));
    let invalid = expr(K::Break(LocalId { owner: 1, index: 0 }));
    assert!(matches!(
        compile(&invalid, &mut 100),
        Err(Error::InvalidControl(_))
    ));
}
#[test]
fn compiler_control_nesting_does_not_use_the_host_stack() {
    let mut body = expr(K::Unit);
    for _ in 0..2048 {
        body = expr(K::Block {
            statements: vec![],
            result: Some(Box::new(body)),
        });
    }
    let code = compile(&body, &mut 30_000).unwrap();
    assert!(code.instructions().len() > 4096);
    drop(code);
    // Release the test input iteratively as well; Rust's generated recursive
    // destructor is unrelated to the compiler's traversal stack.
    loop {
        match body.kind {
            K::Block {
                result: Some(child),
                ..
            } => body = *child,
            _ => break,
        }
    }
}

#[test]
fn compiled_constants_outlive_their_source_and_charge_payload_copying() {
    let code = {
        let body = Expr {
            source: SourceRange { start: 1, end: 2 },
            ty: Term::Primitive(Primitive::Str),
            kind: K::Literal(nir::program::Literal::String("x".repeat(4096))),
        };
        assert!(matches!(compile(&body, &mut 100), Err(Error::Budget(_))));
        compile(&body, &mut 20_000).unwrap()
    };
    assert!(code.storage_work() >= 4096);
    let comptime_compiler::Op::Constant { value, .. } = &code.instructions()[0].op else {
        panic!("missing owned constant")
    };
    assert_eq!(
        value.kind,
        K::Literal(nir::program::Literal::String("x".repeat(4096)))
    );
}

#[test]
fn deeply_nested_constant_types_fail_before_recursive_copying() {
    let mut ty = Term::Primitive(Primitive::Bool);
    for _ in 0..192 {
        ty = Term::Pointer(Box::new(ty));
    }
    let body = Expr {
        source: SourceRange { start: 1, end: 2 },
        ty: Term::Primitive(Primitive::Type),
        kind: K::TypeValue(ty),
    };
    assert!(matches!(
        compile(&body, &mut 100_000),
        Err(Error::Budget(_))
    ));
}

#[test]
fn invocation_code_distinguishes_normal_form_application() {
    let signature = Term::Function {
        parameters: vec![],
        result: Box::new(Term::Primitive(Primitive::Type)),
    };
    let mut body = Expr {
        source: SourceRange { start: 1, end: 2 },
        ty: Term::Primitive(Primitive::Type),
        kind: K::Call {
            callee: Box::new(Expr {
                source: SourceRange { start: 1, end: 2 },
                ty: Term::ComptimeFunction(Box::new(signature.clone())),
                kind: K::Local(LocalId { owner: 1, index: 0 }),
            }),
            arguments: vec![],
            parameter_bindings: vec![],
        },
    };
    let dump = compile(&body, &mut 1000).unwrap().to_string();
    assert!(dump.contains("(begin-call "));
    assert!(dump.contains("(invoke "));
    assert_eq!(dump, compile(&body, &mut 1000).unwrap().to_string());
    let K::Call { callee, .. } = &mut body.kind else {
        unreachable!()
    };
    callee.ty = Term::NormalFormFunction(Box::new(signature), None);
    assert!(matches!(
        compile(&body, &mut 1000),
        Err(Error::Unsupported(_))
    ));
}

#[test]
fn default_templates_are_owned_and_copy_work_is_bounded() {
    let code = {
        let source = SourceRange { start: 1, end: 2 };
        let ty = Term::Primitive(Primitive::Str);
        let body = Expr {
            source,
            ty: ty.clone(),
            kind: K::Call {
                callee: Box::new(Expr {
                    source,
                    ty: Term::ComptimeFunction(Box::new(Term::Function {
                        parameters: vec![nir::Parameter {
                            sort: nir::Sort::Runtime,
                            ty: ty.clone(),
                        }],
                        result: Box::new(ty.clone()),
                    })),
                    kind: K::Local(LocalId { owner: 1, index: 0 }),
                }),
                parameter_bindings: vec![LocalId { owner: 2, index: 0 }],
                arguments: vec![nir::program::CallArgument {
                    passing: nir::program::ArgumentPassing::Value,
                    parameter: 0,
                    is_default: true,
                    value: Expr {
                        source,
                        ty,
                        kind: K::Literal(nir::program::Literal::String("\"\n".repeat(2048))),
                    },
                }],
            },
        };
        assert!(matches!(compile(&body, &mut 100), Err(Error::Budget(_))));
        compile(&body, &mut 20_000).unwrap()
    };
    assert!(code.storage_work() >= 4096);
    let dump = code.to_string();
    assert!(dump.contains("(default-argument "));
    assert!(dump.contains("(prepare-defaults true)"));
    assert!(dump.contains("\\\"\\n"));
}

#[test]
fn reference_dump_preserves_field_ownership_and_rebind_permission() {
    use nir::program::{ArgumentPassing, CallArgument};
    let source = SourceRange { start: 1, end: 2 };
    let boolean = Term::Primitive(Primitive::Bool);
    let body = Expr {
        source,
        ty: boolean.clone(),
        kind: K::Call {
            callee: Box::new(Expr {
                source,
                ty: Term::ComptimeFunction(Box::new(Term::Function {
                    parameters: vec![nir::Parameter {
                        sort: nir::Sort::Runtime,
                        ty: boolean.clone(),
                    }],
                    result: Box::new(boolean.clone()),
                })),
                kind: K::Local(LocalId { owner: 1, index: 0 }),
            }),
            arguments: vec![CallArgument {
                passing: ArgumentPassing::ComptimeReferenceNoRebind,
                parameter: 0,
                is_default: false,
                value: Expr {
                    source,
                    ty: boolean.clone(),
                    kind: K::AnonymousField {
                        value: Box::new(Expr {
                            source,
                            ty: Term::Record(vec![("flag".into(), boolean)]),
                            kind: K::Local(LocalId { owner: 1, index: 1 }),
                        }),
                        name: "flag".into(),
                        index: 0,
                    },
                },
            }],
            parameter_bindings: vec![],
        },
    };
    let dump = compile(&body, &mut 1000).unwrap().to_string();
    assert!(dump.contains("(value-reference "));
    assert!(dump.contains("(local true)"));
    assert!(dump.contains("(field (anonymous-record) 0)"));
    assert!(dump.contains("(rebind false)"));
    assert_eq!(dump, compile(&body, &mut 1000).unwrap().to_string());
}

#[test]
fn variant_dump_retains_nested_heads_and_selected_members() {
    let source = SourceRange { start: 1, end: 2 };
    let inner = nir::DeclarationId {
        package: 1,
        local: 0,
    };
    let outer = nir::DeclarationId {
        package: 1,
        local: 1,
    };
    let inner_type = Term::Nominal {
        declaration: inner,
        arguments: vec![],
    };
    let body = Expr {
        source,
        ty: Term::Nominal {
            declaration: outer,
            arguments: vec![inner_type.clone()],
        },
        kind: K::Variant {
            declaration: outer,
            index: 2,
            fields: vec![Expr {
                source,
                ty: inner_type,
                kind: K::Variant {
                    declaration: inner,
                    index: 0,
                    fields: vec![],
                },
            }],
        },
    };
    let code = compile(&body, &mut 1000).unwrap();
    let dump = code.to_string();
    assert_eq!(dump.matches("(variant ").count(), 2);
    assert!(dump.contains("(index 2)"));
    assert!(dump.contains("(index 0)"));
    assert_eq!(dump, compile(&body, &mut 1000).unwrap().to_string());
}

#[test]
fn match_dump_keeps_pattern_bindings_guards_and_exhaustion_explicit() {
    use nir::program::{MatchArm, Pattern};
    let binding = LocalId { owner: 1, index: 3 };
    let body = expr(K::Match {
        value: Box::new(expr(K::Unit)),
        arms: vec![MatchArm {
            pattern: Pattern::Binding {
                binding,
                inner: Box::new(Pattern::Unit),
            },
            guard: Some(expr(K::Literal(nir::program::Literal::Bool(true)))),
            body: expr(K::Local(binding)),
        }],
    });
    let dump = compile(&body, &mut 1000).unwrap().to_string();
    assert!(dump.contains("(match-pattern "));
    assert!(dump.contains("(bindings (binding "));
    assert!(dump.contains("(non-exhaustive-match)"));
    assert_eq!(dump.matches("(branch ").count(), 2);
    assert_eq!(dump, compile(&body, &mut 1000).unwrap().to_string());
    assert!(matches!(compile(&body, &mut 5), Err(Error::Budget(_))));
}

#[test]
fn deep_owned_patterns_refuse_the_carrier_without_misreporting_fuel() {
    use nir::program::{MatchArm, Pattern};
    let mut pattern = Pattern::Wild;
    for _ in 0..200 {
        pattern = Pattern::Or(vec![pattern]);
    }
    let body = expr(K::Match {
        value: Box::new(expr(K::Unit)),
        arms: vec![MatchArm {
            pattern,
            guard: None,
            body: expr(K::Unit),
        }],
    });
    assert!(matches!(
        compile(&body, &mut 10000),
        Err(Error::Unsupported(_))
    ));
    assert!(matches!(compile(&body, &mut 5), Err(Error::Budget(_))));
}

#[test]
fn declaration_dump_distinguishes_scope_commit_and_invalid_else_completion() {
    use nir::program::Pattern;
    let body = expr(K::LetPattern {
        pattern: Pattern::Binding {
            binding: LocalId { owner: 1, index: 0 },
            inner: Box::new(Pattern::Wild),
        },
        value: Box::new(expr(K::Unit)),
        guard: None,
        otherwise: Some(Box::new(expr(K::Unit))),
    });
    let dump = compile(&body, &mut 1000).unwrap().to_string();
    assert!(dump.contains("(commit-scope)"));
    assert!(dump.contains("(leave-scope)"));
    assert!(dump.contains("(invalid-control-flow)"));
    assert_eq!(dump, compile(&body, &mut 1000).unwrap().to_string());
}

#[test]
fn ordered_object_dump_escapes_keys_and_retains_duplicate_positions() {
    let body = Expr {
        ty: Term::Primitive(Primitive::Object),
        ..expr(K::Object {
            names: vec![Some("quote\"\n".into()), None, Some("quote\"\n".into())],
            fields: vec![expr(K::Unit), expr(K::Unit), expr(K::Unit)],
        })
    };
    let dump = compile(&body, &mut 1000).unwrap().to_string();
    assert!(dump.contains("(name \"quote\\\"\\n\") (positional) (name \"quote\\\"\\n\")"));
    assert_eq!(dump.matches("(check-object-entry ").count(), 3);
    assert_eq!(dump, compile(&body, &mut 1000).unwrap().to_string());
}

#[test]
fn type_comparison_validates_each_operand_before_the_next() {
    use comptime_compiler::Op;
    let type_value = Expr {
        source: SourceRange { start: 3, end: 4 },
        ty: Term::Primitive(Primitive::Type),
        kind: K::TypeValue(Term::Primitive(Primitive::Bool)),
    };
    let body = Expr {
        source: SourceRange { start: 1, end: 6 },
        ty: Term::Primitive(Primitive::Bool),
        kind: K::TypeEqual(Box::new(type_value.clone()), Box::new(type_value)),
    };
    let code = compile(&body, &mut 1000).unwrap();
    let operations = code
        .instructions()
        .iter()
        .filter_map(|instruction| match instruction.op {
            Op::PrepareTypeEqual { .. } => Some("prepare"),
            Op::Constant { .. } => Some("operand"),
            Op::TypeDescription(_) => Some("description"),
            Op::TypeEqual { .. } => Some("compare"),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        operations,
        [
            "prepare",
            "operand",
            "description",
            "operand",
            "description",
            "compare"
        ]
    );
    let dump = code.to_string();
    assert!(dump.contains("(prepare-type-equal (type (primitive bool)) (left (primitive Type)) (right (primitive Type)))"), "{dump}");
    assert_eq!(dump, compile(&body, &mut 1000).unwrap().to_string());
    assert!(matches!(compile(&body, &mut 1), Err(Error::Budget(_))));
}

#[test]
fn function_type_dump_retains_phase_and_parameter_staging() {
    use comptime_compiler::Op;
    let source = SourceRange { start: 1, end: 4 };
    let body = Expr {
        source,
        ty: Term::Primitive(Primitive::Type),
        kind: K::FunctionType {
            comptime: true,
            comptime_parameters: vec![true, false],
            parameters: [Primitive::Type, Primitive::Bool]
                .into_iter()
                .map(|ty| Expr {
                    source,
                    ty: Term::Primitive(Primitive::Type),
                    kind: K::TypeValue(Term::Primitive(ty)),
                })
                .collect(),
        },
    };
    let code = compile(&body, &mut 1000).unwrap();
    let dump = code.to_string();
    assert!(
        dump.contains("(function (comptime true) (parameters true false))"),
        "{dump}"
    );
    assert_eq!(dump, compile(&body, &mut 1000).unwrap().to_string());
    let order = code
        .instructions()
        .iter()
        .filter_map(|instruction| match instruction.op {
            Op::PrepareTypeConstructor(_) => Some("prepare"),
            Op::Constant { .. } => Some("operand"),
            Op::CheckTypeConstructorInput { .. } => Some("check"),
            Op::TypeConstructor { .. } => Some("construct"),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        order,
        [
            "prepare",
            "operand",
            "check",
            "operand",
            "check",
            "construct"
        ]
    );
    assert!(matches!(compile(&body, &mut 1), Err(Error::Budget(_))));
}

#[test]
fn object_query_compilation_keeps_computed_selectors_on_the_snapshot_path() {
    use nir::program::{Literal, ObjectQueryKind};
    let source = SourceRange { start: 1, end: 3 };
    let object = Expr {
        source,
        ty: Term::Primitive(Primitive::Object),
        kind: K::Local(LocalId { owner: 1, index: 0 }),
    };
    let key = Expr {
        source,
        ty: Term::Primitive(Primitive::Symbol),
        kind: K::Literal(Literal::Symbol("key\"\\\n".into())),
    };
    for computed in [false, true] {
        let selector = if computed {
            Expr {
                source,
                ty: key.ty.clone(),
                kind: K::Block {
                    statements: vec![],
                    result: Some(Box::new(key.clone())),
                },
            }
        } else {
            key.clone()
        };
        let packet = Expr {
            source,
            ty: Term::Tuple(vec![object.ty.clone(), selector.ty.clone()]),
            kind: K::Tuple(vec![object.clone(), selector]),
        };
        let body = Expr {
            source,
            ty: Term::Optional(Box::new(Term::Primitive(Primitive::Bool))),
            kind: K::ObjectQuery {
                query: ObjectQueryKind::NamedValue,
                subject: Box::new(packet),
            },
        };
        let dump = compile(&body, &mut 1000).unwrap().to_string();
        assert_eq!(dump.contains("(local-object-query "), !computed, "{dump}");
        assert_eq!(dump.contains("(object-access "), computed, "{dump}");
        assert!(dump.contains("key\\\"\\\\\\n"), "{dump}");
        assert_eq!(dump, compile(&body, &mut 1000).unwrap().to_string());
        assert!(matches!(compile(&body, &mut 1), Err(Error::Budget(_))));
    }
}

#[test]
fn list_code_retains_declared_domains_and_checks_concat_before_right_input() {
    use comptime_compiler::Op;
    let source = SourceRange { start: 1, end: 8 };
    let ty = Term::NormalForm {
        head: nir::DeclarationId {
            package: 3,
            local: 91,
        },
        sort: nir::Sort::Type,
        arguments: vec![Term::Primitive(Primitive::Bool)],
    };
    let empty = Expr {
        source,
        ty: ty.clone(),
        kind: K::TypeList(vec![]),
    };
    let body = Expr {
        source,
        ty,
        kind: K::ListConcat(Box::new(empty.clone()), Box::new(empty)),
    };
    let code = compile(&body, &mut 1000).unwrap();
    let order = code
        .instructions()
        .iter()
        .filter_map(|instruction| match instruction.op {
            Op::PrepareListOperation(_) => Some("prepare-concat"),
            Op::PrepareList(_) => Some("prepare-list"),
            Op::List { .. } => Some("list"),
            Op::CheckListOperand { .. } => Some("check"),
            Op::ListConcat { .. } => Some("concat"),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        order,
        [
            "prepare-concat",
            "prepare-list",
            "list",
            "check",
            "prepare-list",
            "list",
            "check",
            "concat"
        ]
    );
    let dump = code.to_string();
    assert!(dump.contains(&body.ty.to_string()), "{dump}");
    assert_eq!(dump, compile(&body, &mut 1000).unwrap().to_string());
    assert!(matches!(compile(&body, &mut 1), Err(Error::Budget(_))));
}

#[test]
fn list_index_compilation_distinguishes_leaf_reads_from_computed_indices() {
    let source = SourceRange { start: 1, end: 5 };
    let list = Expr {
        source,
        ty: Term::NormalForm {
            head: nir::DeclarationId {
                package: 3,
                local: 91,
            },
            sort: nir::Sort::Type,
            arguments: vec![Term::Primitive(Primitive::Bool)],
        },
        kind: K::Local(LocalId { owner: 1, index: 0 }),
    };
    for mode in 0..3 {
        let literal = Expr {
            source,
            ty: Term::Primitive(Primitive::Integer),
            kind: K::Literal(nir::program::Literal::Integer("0".into())),
        };
        let index = match mode {
            0 => literal,
            1 => Expr {
                kind: K::Local(LocalId { owner: 1, index: 1 }),
                ..literal
            },
            _ => Expr {
                source,
                ty: literal.ty.clone(),
                kind: K::Block {
                    statements: vec![],
                    result: Some(Box::new(literal)),
                },
            },
        };
        let body = Expr {
            source,
            ty: Term::Primitive(Primitive::Bool),
            kind: K::TypeListIndex(Box::new(list.clone()), Box::new(index)),
        };
        let dump = compile(&body, &mut 1000).unwrap().to_string();
        assert_eq!(dump.contains("(local-list-index "), mode != 2, "{dump}");
        assert_eq!(dump.contains("(check-indexed-list "), mode == 2, "{dump}");
        assert!(
            dump.contains("(prepare-list-index (list-index-contract"),
            "{dump}"
        );
        assert_eq!(dump, compile(&body, &mut 1000).unwrap().to_string());
        assert!(matches!(compile(&body, &mut 1), Err(Error::Budget(_))));
    }
}

#[test]
fn append_fusion_requires_an_inert_suffix_and_matching_local() {
    let binding = LocalId { owner: 1, index: 0 };
    let ty = Term::NormalForm {
        head: nir::DeclarationId {
            package: 7,
            local: 99,
        },
        sort: nir::Sort::Type,
        arguments: vec![Term::Primitive(Primitive::Bool)],
    };
    for computed in [false, true] {
        let mut suffix = Expr {
            ty: ty.clone(),
            ..expr(K::TypeList(vec![]))
        };
        if computed {
            suffix = Expr {
                ty: ty.clone(),
                ..expr(K::Block {
                    statements: vec![],
                    result: Some(Box::new(suffix)),
                })
            };
        }
        let body = expr(K::Assign {
            place: nir::program::Place {
                root: binding,
                fields: vec![],
            },
            value: Box::new(Expr {
                ty: ty.clone(),
                ..expr(K::ListConcat(
                    Box::new(Expr {
                        ty: ty.clone(),
                        ..expr(K::Local(binding))
                    }),
                    Box::new(suffix),
                ))
            }),
        });
        let code = compile(&body, &mut 1000).unwrap();
        let dump = code.to_string();
        assert_eq!(dump.contains("(list-append "), !computed, "{dump}");
        assert_eq!(dump, compile(&body, &mut 1000).unwrap().to_string());
        assert!(matches!(compile(&body, &mut 1), Err(Error::Budget(_))));
    }
}

#[test]
fn normal_plan_templates_own_inputs_and_capture_only_free_bindings() {
    use nir::program::{NormalFormArgument as A, NormalFormInputPlan};
    let external = LocalId {
        owner: 90,
        index: 0,
    };
    let first = LocalId {
        owner: 90,
        index: 1,
    };
    let second = LocalId {
        owner: 90,
        index: 2,
    };
    let code = {
        let mut body = Expr {
            source: SourceRange { start: 3, end: 8 },
            ty: Term::Primitive(Primitive::Type),
            kind: K::NormalFormType {
                head: nir::DeclarationId {
                    package: 90,
                    local: 0,
                },
                arguments: [external, first]
                    .into_iter()
                    .map(|binding| {
                        A::Value(Expr {
                            source: SourceRange { start: 4, end: 7 },
                            ty: Term::Primitive(Primitive::Type),
                            kind: K::Local(binding),
                        })
                    })
                    .collect(),
                plan: Some(NormalFormInputPlan {
                    order: vec![0, 1],
                    bindings: vec![first, second],
                    preparations: vec![],
                }),
            },
        };
        let code = compile(&body, &mut 10000).unwrap();
        assert_eq!(
            code.locals().iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![external]
        );
        assert!(matches!(
            compile(&body, &mut (code.storage_work() - 1)),
            Err(Error::Budget(_))
        ));
        let K::NormalFormType {
            plan: Some(plan), ..
        } = &mut body.kind
        else {
            unreachable!()
        };
        plan.order = vec![0, 0];
        assert!(matches!(
            compile(&body, &mut 10000),
            Err(Error::Unsupported(_))
        ));
        code
    };
    let dump = code.to_string();
    assert!(dump.contains("(type-input-plan %"));
    assert!(dump.contains("(order 0 1)"));
    assert_eq!(dump, code.to_string());
}
