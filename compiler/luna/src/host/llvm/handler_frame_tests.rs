//! Isolate per-arm cleanup lowering; source admission is tested in host.rs.
use super::*;
use iir::cleanup::{Action, Boundary, ContextKey, FunctionSchedule, Program, Site, Step, Target};
use nir::program::{FunctionPhase, MatchArm, Pattern, SourceRange};
#[test]
fn handler_arm_cleanup_restores_suspended_storage_on_value_and_resume() {
    let source = |start| SourceRange {
        start,
        end: start + 1,
    };
    let binding = LocalId { owner: 0, index: 0 };
    let declaration = nir::DeclarationId {
        package: 0,
        local: 0,
    };
    let key = iir::InstanceKey {
        declaration,
        arguments: vec![].into(),
    };
    let byte = Term::Primitive(P::U8);
    let destructor = Function {
        phase: FunctionPhase::Runtime,
        inline: false,
        pure: false,
        is_unsafe: false,
        preconditions: vec![],
        postconditions: vec![],
        result_binding: None,
        entry_snapshots: vec![],
        capabilities: vec![],
        requirements: vec![],
        type_parameters: vec![],
        declaration,
        name: "observe_cleanup".into(),
        signature: Term::Function {
            parameters: vec![nir::Parameter {
                sort: Sort::Runtime,
                ty: Term::Pointer(Box::new(byte.clone())),
            }],
            result: Box::new(Term::Primitive(P::Void)),
        },
        parameters: vec![binding],
        body: Expr {
            source: source(0),
            ty: Term::Primitive(P::Void),
            kind: K::Unit,
        },
        defaults: vec![],
    };
    let graph = iir::Package {
        functions: vec![iir::FunctionInstance {
            key: key.clone(),
            function: destructor,
        }],
        ..Default::default()
    };
    let program = Program {
        ty: byte.clone(),
        guards: vec![],
        steps: vec![Step::Invoke {
            path: vec![],
            subject: byte,
            method: key.clone(),
        }],
    };
    let schedule = |at, boundary, target| FunctionSchedule {
        instance: key.clone(),
        context: vec![],
        transfers: vec![],
        copies: vec![],
        pattern_transfers: vec![],
        pattern_orders: vec![],
        discards: vec![],
        sites: vec![Site {
            source: source(at),
            boundary,
            actions: vec![Action {
                target,
                program: program.clone(),
            }],
        }],
    };

    let local_target = || Target::Local {
        binding,
        fields: vec![],
    };
    let mut outer = schedule(10, Boundary::ScopeEnd, local_target());
    // The requester is suspended in a different context from installation.
    outer.context = vec![ContextKey::Default {
        construction: source(5),
        parameter: 0,
    }];
    let context = |arm| {
        vec![ContextKey::EffectHandler {
            installation: source(90),
            handler: 0,
            arm,
        }]
    };
    let lambdas = BTreeMap::new();
    let template_identities = callables::TemplateIdentities::default();
    outer.sites.push(Site {
        source: source(30),
        boundary: Boundary::PredicateFailure,
        actions: vec![Action {
            target: local_target(),
            program: program.clone(),
        }],
    });
    let prepared = Action {
        target: Target::Prepared {
            construction: source(30),
            slot: 0,
            value_source: source(47),
        },
        program: program.clone(),
    };
    outer.sites.push(Site {
        source: source(46),
        boundary: Boundary::PredicateFailure,
        actions: vec![
            prepared,
            Action {
                target: local_target(),
                program: program.clone(),
            },
        ],
    });
    let default_parent = schedule(5, Boundary::DefaultFailure(0), local_target());
    for (resume, failed, argument_failure, lexical_return, loop_exit) in [
        (false, false, false, false, None),
        (true, false, false, false, None),
        (false, true, false, false, None),
        (true, true, false, false, None),
        (false, false, true, false, None),
        (true, false, true, false, None),
        (false, false, false, true, None),
        (false, true, false, true, None),
        (true, false, false, true, None),
        (true, true, false, true, None),
        (false, false, false, false, Some(true)),
        (false, false, false, false, Some(false)),
        (true, false, false, false, Some(true)),
        (true, false, false, false, Some(false)),
    ] {
        let nonlocal = lexical_return || loop_exit.is_some();
        let nested_return = nonlocal && resume;
        let loop_id = LocalId {
            owner: 0,
            index: 99,
        };
        let mut intermediate = schedule(30, Boundary::EffectEscape, local_target());
        intermediate.context = vec![ContextKey::EffectHandler {
            installation: source(80),
            handler: 0,
            arm: 0,
        }];
        let mut outer = outer.clone();
        if nonlocal {
            outer.context.clear();
        }
        if let Some(is_break) = loop_exit {
            outer.sites.push(Site {
                source: source(if nested_return { 70 } else { 30 }),
                boundary: if is_break {
                    Boundary::EffectBreak(loop_id)
                } else {
                    Boundary::EffectContinue(loop_id)
                },
                actions: vec![Action {
                    target: local_target(),
                    program: program.clone(),
                }],
            });
        }
        let mut returning = graph.functions[0].function.clone();
        if lexical_return {
            outer.context.clear();
            outer.sites.push(Site {
                source: source(if nested_return { 70 } else { 30 }),
                boundary: Boundary::EffectReturn,
                actions: vec![Action {
                    target: local_target(),
                    program: program.clone(),
                }],
            });
            outer.sites.push(Site {
                source: source(60),
                boundary: Boundary::PostconditionFailure(source(if nested_return {
                    70
                } else {
                    30
                })),
                actions: vec![Action {
                    target: local_target(),
                    program: program.clone(),
                }],
            });
            returning.postconditions = vec![Expr {
                source: source(60),
                ty: Term::Primitive(P::Bool),
                kind: K::IntegerBinary {
                    op: Op::Equal,
                    left: Box::new(Expr {
                        source: source(61),
                        ty: Term::Primitive(P::U8),
                        kind: K::Local(binding),
                    }),
                    right: Box::new(Expr {
                        source: source(62),
                        ty: Term::Primitive(P::U8),
                        kind: K::Literal(Literal::UnsignedInteger(if failed { 0 } else { 65 })),
                    }),
                },
            }];
        }
        let mut rejected = schedule(41, Boundary::HandlerEnd, local_target());
        rejected.context = context(0);
        let mut selected = schedule(
            42,
            if let Some(is_break) = loop_exit {
                if is_break {
                    Boundary::Break
                } else {
                    Boundary::Continue
                }
            } else if lexical_return {
                Boundary::Return
            } else if resume {
                Boundary::EffectResume
            } else {
                Boundary::HandlerEnd
            },
            local_target(),
        );
        selected.context = context(1);
        selected.sites.push(Site {
            source: source(44),
            boundary: Boundary::PredicateFailure,
            actions: vec![Action {
                target: local_target(),
                program: program.clone(),
            }],
        });
        let published = iir::cleanup::Schedule {
            functions: vec![rejected, selected],
            ..Default::default()
        };
        let mut declarations = BTreeSet::new();
        let mut fuel = 10000;
        let mut emitter = Emitter {
            graph: &graph,
            cleanup: Some(&outer),
            schedules: Some(&published),
            resource_unwinds: vec![],
            checked: true,
            function: lexical_return.then_some(&returning),
            closure_body: None,
            failure_result: None,
            effects: vec![],
            effect_resume: None,
            native_return: None,
            loop_destinations: Default::default(),
            outlined_return: None,
            outlined_loops: BTreeMap::new(),
            lambdas: &lambdas,
            template_identities: &template_identities,
            locals: BTreeMap::new(),
            live: BTreeMap::new(),
            temporaries: BTreeMap::new(),
            temporary_live: BTreeMap::new(),
            prepared: BTreeMap::new(),
            receipt_paths: vec![],
            pattern_orders: BTreeMap::new(),
            temporary_fields: BTreeMap::new(),
            loops: BTreeMap::new(),
            prologue: String::new(),
            body: String::new(),
            declarations: &mut declarations,
            text_constants: &mut strings::TextConstants::default(),
            serial: 0,
            terminated: false,
            fuel: &mut fuel,
        };

        if !nonlocal {
            emitter.cleanup = Some(&default_parent);
            emitter
                .bind(
                    binding,
                    &Value {
                        ty: Ty::Int(8, false),
                        text: "67".into(),
                    },
                )
                .unwrap();
            let default_state = emitter.take_local_frame();
            emitter.resource_unwinds.push(frames::UnwindFrame {
                state: default_state,
                source: source(5),
                boundary: Boundary::DefaultFailure(0),
            });
        }
        emitter.cleanup = Some(&outer);
        emitter
            .bind(
                binding,
                &Value {
                    ty: Ty::Int(8, false),
                    text: "65".into(),
                },
            )
            .unwrap();
        let mut caller_pointer = emitter.locals[&binding].pointer.clone();
        let truth = |at| Expr {
            source: source(at),
            ty: Term::Primitive(P::Bool),
            kind: K::Literal(Literal::Bool(true)),
        };
        let selected = if let Some(is_break) = loop_exit {
            Expr {
                source: source(42),
                ty: Term::Primitive(P::Never),
                kind: if is_break {
                    K::Break(loop_id)
                } else {
                    K::Continue(loop_id)
                },
            }
        } else if lexical_return {
            Expr {
                source: source(42),
                ty: Term::Primitive(P::Never),
                kind: K::Return(Some(Box::new(Expr {
                    source: source(43),
                    ty: Term::Primitive(P::I32),
                    kind: K::Literal(Literal::SignedInteger(0)),
                }))),
            }
        } else if resume {
            Expr {
                source: source(42),
                ty: Term::Primitive(P::Never),
                kind: K::EffectResume(Box::new(truth(43))),
            }
        } else {
            truth(42)
        };
        let selected = if failed && !lexical_return {
            Expr {
                source: source(42),
                ty: selected.ty.clone(),
                kind: K::Block {
                    statements: vec![Expr {
                        source: source(45),
                        ty: Term::Primitive(P::Void),
                        kind: K::CheckPredicate(Box::new(Expr {
                            source: source(44),
                            ty: Term::Primitive(P::Bool),
                            kind: K::Literal(Literal::Bool(false)),
                        })),
                    }],
                    result: Some(Box::new(selected)),
                },
            }
        } else {
            selected
        };
        // A loop introduced inside the handler belongs to the arm, even when
        // the handler also captures an outer loop target.
        let local_loop = LocalId {
            owner: 0,
            index: 98,
        };
        let selected = Expr {
            source: source(42),
            ty: selected.ty.clone(),
            kind: K::Block {
                statements: vec![Expr {
                    source: source(64),
                    ty: Term::Primitive(P::Void),
                    kind: K::Loop {
                        identity: local_loop,
                        body: Box::new(Expr {
                            source: source(65),
                            ty: Term::Primitive(P::Never),
                            kind: K::Break(local_loop),
                        }),
                    },
                }],
                result: Some(Box::new(selected)),
            },
        };
        let arms = vec![
            MatchArm {
                pattern: Pattern::Tuple(vec![
                    Pattern::Binding {
                        binding,
                        inner: Box::new(Pattern::Wild),
                    },
                    Pattern::Wild,
                ]),
                guard: Some(Expr {
                    source: source(40),
                    ty: Term::Primitive(P::Bool),
                    kind: K::Literal(Literal::Bool(false)),
                }),
                body: truth(41),
            },
            MatchArm {
                pattern: Pattern::Tuple(vec![
                    Pattern::Binding {
                        binding,
                        inner: Box::new(Pattern::Wild),
                    },
                    Pattern::Wild,
                ]),
                guard: None,
                body: selected,
            },
        ];
        if loop_exit.is_some() {
            emitter.loops.insert(
                loop_id,
                frames::LoopTarget {
                    control: None,
                    head: "loop_head".into(),
                    end: "loop_end".into(),
                    owner: Some((outer.instance.clone(), outer.context.clone())),
                },
            );
        }
        let entry = std::sync::Arc::new(DirectHandler {
            return_storage: None,
            outer_loop_controls: BTreeMap::new(),
            scope: Some(HandlerScope {
                instance: key.clone(),
                context: vec![],
                installation: source(90),
                handler: 0,
            }),
            handler: nir::program::EffectHandler {
                operation: nir::effects::Operation {
                    declaration,
                    arguments: vec![],
                },
                abi: nir::effects::Abi::Direct,
                signature: Term::Function {
                    parameters: vec![
                        nir::Parameter {
                            sort: Sort::Runtime,
                            ty: Term::Primitive(P::U8),
                        },
                        nir::Parameter {
                            sort: Sort::Runtime,
                            ty: Term::Primitive(P::Bool),
                        },
                    ],
                    result: Box::new(Term::Primitive(P::Bool)),
                },
                arms,
                fallback: None,
            },
            provided: None,
            machine: None,
            locals: BTreeMap::new(),
            loops: emitter.loops.clone(),
            outer: vec![],
        });
        let operation = entry.handler.operation.clone();
        let signature = entry.handler.signature.clone();
        if nested_return {
            let root = emitter.take_local_frame();
            emitter.resource_unwinds.push(frames::UnwindFrame {
                state: root,
                source: source(70),
                boundary: Boundary::PredicateFailure,
            });
            emitter.cleanup = Some(&intermediate);
            emitter
                .bind(
                    binding,
                    &Value {
                        ty: Ty::Int(8, false),
                        text: "68".into(),
                    },
                )
                .unwrap();
            caller_pointer = emitter.locals[&binding].pointer.clone();
        }
        emitter.effects = vec![entry];
        let request = Expr {
            source: source(30),
            ty: Term::Primitive(P::Bool),
            kind: K::EffectRequest {
                operation,
                abi: nir::effects::Abi::Direct,
                allowed_index: 0,
                signature: Box::new(signature),
                arguments: vec![
                    Expr {
                        source: source(47),
                        ty: Term::Primitive(P::U8),
                        kind: K::Literal(Literal::UnsignedInteger(66)),
                    },
                    Expr {
                        source: source(48),
                        ty: Term::Primitive(P::Bool),
                        kind: K::Block {
                            statements: vec![Expr {
                                source: source(49),
                                ty: Term::Primitive(P::Void),
                                kind: K::CheckPredicate(Box::new(Expr {
                                    source: source(46),
                                    ty: Term::Primitive(P::Bool),
                                    kind: K::Literal(Literal::Bool(!argument_failure)),
                                })),
                            }],
                            result: Some(Box::new(truth(50))),
                        },
                    },
                ],
            },
        };
        let value = emitter.expr(&request, 0).unwrap();
        assert_eq!(
            emitter.resource_unwinds.len(),
            usize::from(!nonlocal) + usize::from(nested_return)
        );
        assert_eq!(emitter.locals[&binding].pointer, caller_pointer);
        assert!(std::ptr::eq(
            emitter.cleanup.unwrap(),
            if nested_return { &intermediate } else { &outer }
        ));
        assert!(emitter.effect_resume.is_none());
        if loop_exit.is_some() {
            emitter.line("ret i32 1");
            for (label, marker) in [("loop_head", 72), ("loop_end", 69)] {
                emitter.label(label);
                let slot = emitter.slot(Ty::Int(8, false));
                emitter
                    .store(
                        &slot,
                        &Value {
                            ty: Ty::Int(8, false),
                            text: marker.to_string(),
                        },
                    )
                    .unwrap();
                emitter.line(format!("call void @flurry_0(ptr {})", slot.pointer));
                emitter.line("ret i32 0");
            }
        } else {
            emitter
                .cleanup_boundary(source(10), Boundary::ScopeEnd)
                .unwrap();
            let status = emitter.instruction(
                Ty::Int(32, false),
                format!("select i1 {}, i32 0, i32 1", value.text),
            );
            emitter.line(format!("ret i32 {}", status.text));
        }
        let declarations = emitter
            .declarations
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        let ir = format!(
            "{declarations}\ndeclare void @llvm.trap()\ndeclare i64 @write(i32, ptr, i64)\ndefine void @flurry_0(ptr %p) {{\nentry:\n  %n = call i64 @write(i32 1, ptr %p, i64 1)\n  ret void\n}}\ndefine i32 @main() {{\nentry:\n{}{}}}\n",
            emitter.prologue, emitter.body
        );
        for optimization in [
            crate::host::OptimizationLevel::O0,
            crate::host::OptimizationLevel::O3,
        ] {
            let out = crate::host::Toolchain {
                optimization,
                ..Default::default()
            }
            .execute_test_ir(&ir)
            .unwrap();
            assert_eq!(
                out.status.success(),
                !(failed || argument_failure),
                "resume={resume} failed={failed} argument_failure={argument_failure} status={} stdout={:?} stderr={} IR={ir}",
                out.status,
                out.stdout,
                String::from_utf8_lossy(&out.stderr)
            );
            assert_eq!(
                out.stdout,
                if let Some(is_break) = loop_exit {
                    match (nested_return, is_break) {
                        (true, true) => b"BDAE".as_slice(),
                        (true, false) => b"BDAH".as_slice(),
                        (false, true) => b"BAE".as_slice(),
                        (false, false) => b"BAH".as_slice(),
                    }
                } else if nested_return {
                    b"BDA".as_slice()
                } else if lexical_return {
                    b"BA".as_slice()
                } else if failed || argument_failure {
                    b"BAC".as_slice()
                } else {
                    b"BA".as_slice()
                }
            );
        }
    }
}
