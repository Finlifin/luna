use super::*;
use nir::normal_forms::{Head, Kind, Parameter, Role};
use nir::program::{NormalFormInputPlan, Place};
use std::cell::RefCell;

const SOURCE: SourceRange = SourceRange { start: 4, end: 9 };
fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 94, local }
}
fn local(index: u32) -> LocalId {
    LocalId { owner: 94, index }
}
fn expression(ty: Term, kind: K) -> Expr {
    Expr {
        ty,
        kind,
        source: SOURCE,
    }
}

// Observe the real providers, without replacing their domain, admission, or
// completion rules. Counts expose repeated observation of unrelated inputs.
struct Observed<S> {
    inner: S,
    queries: RefCell<Vec<usize>>,
    bindings: Vec<usize>,
}
impl<S: Source> fmt::Display for Observed<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.fmt(f)
    }
}
impl<S: Source> Source for Observed<S> {
    fn expression(&self) -> &Expr {
        self.inner.expression()
    }
    fn substitutions(&self) -> Option<&input_bindings::Bindings> {
        self.inner.substitutions()
    }
    fn pending_inputs(
        &self,
        vm: &mut Interpreter<'_>,
        slot: usize,
    ) -> Result<BTreeSet<usize>, Error> {
        self.queries.borrow_mut().push(slot);
        self.inner.pending_inputs(vm, slot)
    }
    fn expected(&self, vm: &mut Interpreter<'_>, slot: usize) -> Result<Term, Error> {
        self.inner.expected(vm, slot)
    }
    fn check_input(
        &self,
        vm: &mut Interpreter<'_>,
        slot: usize,
        operand: &Argument,
        substitutions: Option<&input_bindings::Bindings>,
        expected: Option<&Term>,
    ) -> Result<(), Error> {
        self.inner
            .check_input(vm, slot, operand, substitutions, expected)
    }
    fn bind(&mut self, vm: &mut Interpreter<'_>, slot: usize, value: Value) -> Result<(), Error> {
        self.inner.bind(vm, slot, value)?;
        self.bindings.push(slot);
        Ok(())
    }
    fn complete(&mut self, vm: &mut Interpreter<'_>) -> Result<Progress, Error> {
        self.inner.complete(vm)
    }
    fn cleanup(&self, env: &mut Environment) {
        self.inner.cleanup(env);
    }
}
fn observed<S: Source>(source: S) -> Frame<Observed<S>> {
    Frame::from_source(Observed {
        inner: source,
        queries: RefCell::new(vec![]),
        bindings: vec![],
    })
}
fn run<S: Source>(
    frame: &mut Frame<S>,
    vm: &mut Interpreter<'_>,
    env: &mut Environment,
) -> Result<Progress, Error> {
    loop {
        match frame.next_input(vm, env)? {
            Progress::DomainDelivered => continue,
            Progress::Input(bytecode::InputFrame { mut code, finish }) => {
                let flow = vm.run_fragment(&mut code);
                let result = finish.complete(vm, code, flow);
                if let Some(flow) = frame.accept_input(vm, env, result)? {
                    return Ok(Progress::Complete(flow));
                }
            }
            result => return Ok(result),
        }
    }
}

fn fixture(count: usize) -> (Package, Expr, Vec<nir::DeclarationParameter>, Vec<Term>) {
    let ty = Term::Primitive(Primitive::Type);
    let boolean = Term::Primitive(Primitive::Bool);
    let parameters: Vec<_> = (0..count)
        .map(|slot| nir::DeclarationParameter {
            declaration: id(slot as u32),
            sort: if slot == 1 {
                nir::Sort::ComptimeValue
            } else {
                nir::Sort::Type
            },
        })
        .collect();
    let domains: Vec<_> = (0..count)
        .map(|slot| {
            if slot == 1 {
                Term::Rigid {
                    declaration: id(0),
                    sort: nir::Sort::Type,
                }
            } else {
                ty.clone()
            }
        })
        .collect();
    let arguments = (0..count)
        .map(|slot| {
            if slot == 1 {
                Argument::Block {
                    source: SOURCE,
                    statements: vec![expression(
                        Term::Primitive(Primitive::Void),
                        K::Assign {
                            place: Place {
                                root: local(999),
                                fields: vec![],
                            },
                            value: Box::new(expression(
                                boolean.clone(),
                                K::Not(Box::new(expression(boolean.clone(), K::Local(local(999))))),
                            )),
                        },
                    )],
                    result: Some(Box::new(Argument::Integer {
                        source: SOURCE,
                        spelling: "7".into(),
                        negative: false,
                    })),
                }
            } else {
                Argument::Value(expression(
                    ty.clone(),
                    K::TypeValue(if slot == 0 {
                        Term::Primitive(Primitive::U8)
                    } else {
                        boolean.clone()
                    }),
                ))
            }
        })
        .collect();
    let package = Package {
        normal_forms: vec![Head {
            declaration: id(1000),
            collector: None,
            role: Role::Opaque,
            requirements: vec![],
            parameters: parameters
                .iter()
                .zip(&domains)
                .enumerate()
                .map(|(slot, (parameter, domain))| Parameter {
                    declaration: parameter.declaration,
                    binding: local(slot as u32),
                    kind: if slot == 1 { Kind::Value } else { Kind::Type },
                    ty: domain.clone(),
                })
                .collect(),
        }],
        ..Default::default()
    };
    let input = expression(
        ty,
        K::NormalFormType {
            head: id(1000),
            arguments,
            plan: Some(NormalFormInputPlan {
                order: (1..count).chain([0]).collect(),
                bindings: (0..count).map(|slot| local(slot as u32)).collect(),
                preparations: vec![],
            }),
        },
    );
    (package, input, parameters, domains)
}

fn check_unrelated<S: Source>(source: S, vm: &mut Interpreter<'_>, count: usize) {
    let mut frame = observed(source);
    let mut env = Environment::from([(local(999), Value::Bool(false))]);
    let result = run(&mut frame, vm, &mut env).unwrap();
    assert!(matches!(
        result,
        Progress::InputsComplete | Progress::Complete(Flow::Value(Value::Type(_)))
    ));
    assert_eq!(
        &*frame.source.queries.borrow(),
        &(1..count).chain([0]).collect::<Vec<_>>()
    );
    assert_eq!(
        frame.source.bindings,
        (2..count).chain([0, 1]).collect::<Vec<_>>()
    );
    assert_eq!(env.get(&local(1)), Some(&Value::UnsignedInteger(7)));
    assert_eq!(env.get(&local(999)), Some(&Value::Bool(true)));
    assert!(frame.pending.is_empty() && frame.dependents.is_empty() && frame.ready.is_empty());
    frame.cleanup(&mut env);
    assert_eq!(env, Environment::from([(local(999), Value::Bool(true))]));
}

#[test]
fn only_observed_completions_recheck_nominal_and_normal_domains() {
    for count in [4, 32] {
        let (package, input, parameters, domains) = fixture(count);
        for compiled in [false, true] {
            let mut vm = if compiled {
                Interpreter::for_comptime(&package, Budget::default()).unwrap()
            } else {
                Interpreter::for_comptime_tree(&package, Budget::default()).unwrap()
            };
            let source =
                normal_plan::Frame::new(&mut vm, Expression::Borrowed(&input), &Environment::new())
                    .unwrap()
                    .source;
            check_unrelated(source, &mut vm, count);
            let K::NormalFormType {
                arguments,
                plan: Some(plan),
                ..
            } = &input.kind
            else {
                unreachable!()
            };
            let nominal = expression(
                input.ty.clone(),
                K::NominalType {
                    declaration: id(1000),
                    arguments: arguments.clone(),
                    bindings: Some(plan.bindings.clone()),
                    order: Some(plan.order.clone()),
                    preparations: vec![],
                },
            );
            let locals = Expression::Borrowed(&nominal)
                .admit_scope(&mut vm, &Environment::new())
                .unwrap();
            let source = nominal_inputs::Source::new(
                input_collection::Expression::Borrowed(&nominal),
                parameters.clone(),
                domains.clone(),
                true,
                locals,
            );
            check_unrelated(source, &mut vm, count);
        }
    }
}

#[test]
fn transitive_completions_wake_dependents_and_cycles_remain_unresolved() {
    let ty = Term::Primitive(Primitive::Type);
    let rigid = |slot, sort| Term::Rigid {
        declaration: id(slot),
        sort,
    };
    let parameters: Vec<_> = (0..3)
        .map(|slot| nir::DeclarationParameter {
            declaration: id(slot),
            sort: if slot == 2 {
                nir::Sort::Type
            } else {
                nir::Sort::ComptimeValue
            },
        })
        .collect();
    let (_, donor, _, _) = fixture(3);
    let K::NormalFormType {
        arguments: donor_arguments,
        ..
    } = donor.kind
    else {
        unreachable!()
    };
    for compiled in [false, true] {
        for cyclic in [false, true] {
            let package = Package::default();
            let mut vm = if compiled {
                Interpreter::for_comptime(&package, Budget::default()).unwrap()
            } else {
                Interpreter::for_comptime_tree(&package, Budget::default()).unwrap()
            };
            let domains = vec![
                rigid(1, nir::Sort::ComptimeValue),
                if cyclic {
                    rigid(0, nir::Sort::ComptimeValue)
                } else {
                    rigid(2, nir::Sort::Type)
                },
                ty.clone(),
            ];
            let input = expression(
                ty.clone(),
                K::NominalType {
                    declaration: id(1000),
                    preparations: vec![],
                    order: None,
                    bindings: Some((0..3).map(local).collect()),
                    arguments: vec![
                        donor_arguments[1].clone(),
                        Argument::Value(expression(
                            ty.clone(),
                            K::TypeValue(Term::Primitive(Primitive::U8)),
                        )),
                        Argument::Value(expression(ty.clone(), K::TypeValue(ty.clone()))),
                    ],
                },
            );
            let locals = Expression::Borrowed(&input)
                .admit_scope(&mut vm, &Environment::new())
                .unwrap();
            let source = nominal_inputs::Source::new(
                input_collection::Expression::Borrowed(&input),
                parameters.clone(),
                domains,
                true,
                locals,
            );
            let mut frame = observed(source);
            let mut env = Environment::from([(local(999), Value::Bool(false))]);
            let result = run(&mut frame, &mut vm, &mut env);
            if cyclic {
                assert_eq!(result.err().unwrap().kind, ErrorKind::Type);
                assert_eq!(&*frame.source.queries.borrow(), &[0, 1, 2]);
                assert_eq!(frame.source.bindings, [2]);
            } else {
                assert!(matches!(result, Ok(Progress::InputsComplete)));
                assert_eq!(&*frame.source.queries.borrow(), &[0, 1, 2]);
                assert_eq!(frame.source.bindings, [2, 1, 0]);
                assert_eq!(env.get(&local(0)), Some(&Value::UnsignedInteger(7)));
            }
            assert_eq!(
                frame.next_input(&mut vm, &mut env).err().unwrap().kind,
                ErrorKind::InvalidControlFlow
            );
            frame.cleanup(&mut env);
            assert_eq!(env, Environment::from([(local(999), Value::Bool(true))]));
        }
    }
}

#[test]
fn dependency_registration_and_wakeup_fail_terminally_without_replaying_inputs() {
    let (package, input, _, _) = fixture(2);
    for fail_at_registration in [true, false] {
        let mut vm = Interpreter::for_comptime_tree(&package, Budget::default()).unwrap();
        let source =
            normal_plan::Frame::new(&mut vm, Expression::Borrowed(&input), &Environment::new())
                .unwrap()
                .source;
        let mut frame = observed(source);
        let mut env = Environment::from([(local(999), Value::Bool(false))]);
        let Request::Stage(argument) = frame.advance(&mut vm, &mut env).unwrap() else {
            panic!("pending domain")
        };
        let std::ops::ControlFlow::Continue(staged) = vm
            .interpret_staged_argument(&argument, &mut env, 0)
            .unwrap()
        else {
            panic!("staged input")
        };
        if fail_at_registration {
            vm.budget.steps = 0;
        }
        let registered = frame.receive(&mut vm, &mut env, Completion::Staged(staged));
        let failure = if fail_at_registration {
            registered.unwrap_err()
        } else {
            registered.unwrap();
            let Request::Ready(argument, expected) = frame.advance(&mut vm, &mut env).unwrap()
            else {
                panic!("ready Type")
            };
            let Flow::Value(value) = vm
                .interpret_contextual_input(&argument, &expected, &mut env)
                .unwrap()
            else {
                panic!("Type value")
            };
            frame
                .receive(&mut vm, &mut env, Completion::Value(value))
                .unwrap();
            assert!(frame.to_string().contains("(ready 1)"));
            vm.budget.steps = 0;
            frame.advance(&mut vm, &mut env).err().unwrap()
        };
        assert_eq!(failure.kind, ErrorKind::StepsExhausted);
        assert!(!env.contains_key(&local(1)));
        vm.budget = Budget::default();
        assert_eq!(
            frame.next_input(&mut vm, &mut env).err().unwrap().kind,
            ErrorKind::InvalidControlFlow
        );
        frame.cancel(&mut env);
        assert_eq!(env, Environment::from([(local(999), Value::Bool(true))]));
        let source =
            normal_plan::Frame::new(&mut vm, Expression::Borrowed(&input), &Environment::new())
                .unwrap()
                .source;
        check_unrelated(source, &mut vm, 2);
    }
}

#[test]
fn waking_a_staged_block_is_independent_of_its_executed_prefix() {
    for compiled in [false, true] {
        let wake_cost = |statement_count| {
            let (package, mut input, _, _) = fixture(2);
            let K::NormalFormType { arguments, .. } = &mut input.kind else {
                unreachable!()
            };
            let Argument::Block { statements, .. } = &mut arguments[1] else {
                unreachable!()
            };
            statements.resize(statement_count, statements[0].clone());
            let mut vm = if compiled {
                Interpreter::for_comptime(&package, Budget::default()).unwrap()
            } else {
                Interpreter::for_comptime_tree(&package, Budget::default()).unwrap()
            };
            let mut env = Environment::from([(local(999), Value::Bool(false))]);
            let mut frame =
                normal_plan::Frame::new(&mut vm, Expression::Borrowed(&input), &env).unwrap();
            let Request::Stage(argument) = frame.advance(&mut vm, &mut env).unwrap() else {
                panic!("expected input before its domain")
            };
            let std::ops::ControlFlow::Continue(staged) =
                vm.stage_normal_argument(&argument, &mut env, 0).unwrap()
            else {
                panic!("unexpected transfer from staged block")
            };
            frame
                .receive(&mut vm, &mut env, Completion::Staged(staged))
                .unwrap();
            let Request::Ready(argument, expected) = frame.advance(&mut vm, &mut env).unwrap()
            else {
                panic!("expected the later Type input")
            };
            let Flow::Value(value) = vm
                .evaluate_contextual_argument(&argument, &expected, &mut env)
                .unwrap()
            else {
                panic!("unexpected transfer from Type input")
            };
            frame
                .receive(&mut vm, &mut env, Completion::Value(value))
                .unwrap();
            let before = vm.budget.steps;
            assert!(matches!(
                frame.advance(&mut vm, &mut env).unwrap(),
                Request::DomainDelivered
            ));
            assert!(matches!(
                frame.advance(&mut vm, &mut env).unwrap(),
                Request::Complete
            ));
            assert_eq!(env.get(&local(1)), Some(&Value::UnsignedInteger(7)));
            assert_eq!(env.get(&local(999)), Some(&Value::Bool(true)));
            before - vm.budget.steps
        };
        assert_eq!(wake_cost(1), wake_cost(129));
    }
}

#[test]
fn one_domain_delivery_can_be_cancelled_before_a_later_field_fails() {
    let (mut package, mut input, _, _) = fixture(3);
    let head = &mut package.normal_forms[0];
    head.parameters[2].kind = Kind::Value;
    head.parameters[2].ty = head.parameters[1].ty.clone();
    let invalid_source = SourceRange { start: 21, end: 24 };
    let K::NormalFormType { arguments, .. } = &mut input.kind else {
        unreachable!()
    };
    arguments[2] = Argument::Integer {
        source: invalid_source,
        spelling: "256".into(),
        negative: false,
    };

    for tree in [false, true] {
        let mut vm = if tree {
            Interpreter::for_comptime_tree(&package, Budget::default()).unwrap()
        } else {
            Interpreter::for_comptime(&package, Budget::default()).unwrap()
        };
        // Re-enter the same VM after cancellation and again after failure.
        for cancel in [true, false, true] {
            let mut env = Environment::from([(local(999), Value::Bool(false))]);
            let mut frame =
                normal_plan::Frame::new(&mut vm, Expression::Borrowed(&input), &env).unwrap();
            loop {
                match frame.next_input(&mut vm, &mut env).unwrap() {
                    Progress::DomainDelivered => break,
                    Progress::Input(bytecode::InputFrame { mut code, finish }) => {
                        let flow = vm.run_fragment(&mut code);
                        let result = finish.complete(&mut vm, code, flow);
                        assert!(
                            frame
                                .accept_input(&mut vm, &mut env, result)
                                .unwrap()
                                .is_none()
                        );
                    }
                    _ => panic!("completed before delivering the first field"),
                }
            }
            assert_eq!(env.get(&local(1)), Some(&Value::UnsignedInteger(7)));
            assert!(!env.contains_key(&local(2)));
            if cancel {
                frame.cancel(&mut env);
            } else {
                let failure = frame.next_input(&mut vm, &mut env).err().unwrap();
                assert_eq!(failure.kind, ErrorKind::Type);
                assert_eq!(failure.source, Some(invalid_source));
                frame.cleanup(&mut env);
            }
            assert_eq!(env, Environment::from([(local(999), Value::Bool(true))]));
            assert_eq!(
                frame.next_input(&mut vm, &mut env).err().unwrap().kind,
                ErrorKind::InvalidControlFlow
            );
        }
    }
}

#[test]
fn shared_and_partial_dependencies_keep_each_pending_value_until_its_domain_is_ready() {
    for shared in [false, true] {
        let count = if shared { 24 } else { 3 };
        let (mut package, mut input, _, _) = fixture(count);
        let K::NormalFormType {
            arguments,
            plan: Some(plan),
            ..
        } = &mut input.kind
        else {
            unreachable!()
        };
        if shared {
            let domain = package.normal_forms[0].parameters[1].ty.clone();
            for slot in 2..count {
                package.normal_forms[0].parameters[slot].kind = Kind::Value;
                package.normal_forms[0].parameters[slot].ty = domain.clone();
                arguments[slot] = Argument::Integer {
                    source: SOURCE,
                    spelling: "7".into(),
                    negative: false,
                };
            }
        } else {
            package.normal_forms[0].parameters[1].kind = Kind::Type;
            package.normal_forms[0].parameters[1].ty = Term::Primitive(Primitive::Type);
            package.normal_forms[0].parameters[2].kind = Kind::Value;
            package.normal_forms[0].parameters[2].ty = Term::Tuple(vec![
                Term::Rigid {
                    declaration: id(0),
                    sort: nir::Sort::Type,
                },
                Term::Rigid {
                    declaration: id(1),
                    sort: nir::Sort::Type,
                },
            ]);
            plan.order = vec![2, 0, 1];
            arguments.swap(1, 2);
            let Argument::Block {
                result: Some(result),
                ..
            } = &mut arguments[2]
            else {
                unreachable!()
            };
            **result = Argument::Tuple {
                source: SOURCE,
                items: vec![*result.clone(), *result.clone()],
            };
            arguments[1] = arguments[0].clone();
        }
        for compiled in [false, true] {
            let mut vm = if compiled {
                Interpreter::for_comptime(&package, Budget::default()).unwrap()
            } else {
                Interpreter::for_comptime_tree(&package, Budget::default()).unwrap()
            };
            let source =
                normal_plan::Frame::new(&mut vm, Expression::Borrowed(&input), &Environment::new())
                    .unwrap()
                    .source;
            let mut frame = observed(source);
            let mut env = Environment::from([(local(999), Value::Bool(false))]);
            assert!(matches!(
                run(&mut frame, &mut vm, &mut env).unwrap(),
                Progress::Complete(Flow::Value(Value::Type(_)))
            ));
            if shared {
                assert_eq!(
                    &*frame.source.queries.borrow(),
                    &(1..count).chain([0]).collect::<Vec<_>>()
                );
                for slot in 1..count {
                    assert_eq!(
                        env.get(&local(slot as u32)),
                        Some(&Value::UnsignedInteger(7))
                    );
                }
            } else {
                assert_eq!(&*frame.source.queries.borrow(), &[2, 0, 1]);
                assert_eq!(
                    env.get(&local(2)),
                    Some(&Value::Tuple(vec![
                        Value::UnsignedInteger(7),
                        Value::UnsignedInteger(7)
                    ]))
                );
            }
            assert_eq!(env.get(&local(999)), Some(&Value::Bool(true)));
            assert!(
                frame.pending.is_empty() && frame.dependents.is_empty() && frame.ready.is_empty()
            );
        }
    }
}

#[test]
fn supplied_abstract_identity_is_complete_even_when_its_declaration_is_unchanged() {
    let (package, mut input, _, _) = fixture(2);
    let abstract_type = Term::Rigid {
        declaration: id(0),
        sort: nir::Sort::Type,
    };
    let abstract_value = Value::AbstractValue {
        declaration: id(2000),
        ty: abstract_type.clone(),
    };
    let K::NormalFormType { arguments, .. } = &mut input.kind else {
        unreachable!()
    };
    arguments[0] = Argument::Value(expression(
        Term::Primitive(Primitive::Type),
        K::TypeValue(abstract_type.clone()),
    ));
    arguments[1] = Argument::Value(expression(abstract_type.clone(), K::Local(local(998))));
    for compiled in [false, true] {
        let mut vm = if compiled {
            Interpreter::for_comptime(&package, Budget::default()).unwrap()
        } else {
            Interpreter::for_comptime_tree(&package, Budget::default()).unwrap()
        };
        vm.authorize_abstract_type_parameters(&[id(0)]).unwrap();
        vm.authorize_abstract_value_parameters(&[(id(2000), abstract_type.clone())])
            .unwrap();
        let mut env = Environment::from([(local(998), abstract_value.clone())]);
        let source = normal_plan::Frame::new(&mut vm, Expression::Borrowed(&input), &env)
            .unwrap()
            .source;
        let mut frame = observed(source);
        let Progress::Complete(Flow::Value(Value::Type(result))) =
            run(&mut frame, &mut vm, &mut env).unwrap()
        else {
            panic!("opaque Type result")
        };
        assert_eq!(
            result,
            Term::NormalForm {
                head: id(1000),
                sort: nir::Sort::Type,
                arguments: vec![
                    abstract_type.clone(),
                    Term::Rigid {
                        declaration: id(2000),
                        sort: nir::Sort::ComptimeValue
                    }
                ],
            }
        );
        assert_eq!(&*frame.source.queries.borrow(), &[1, 0]);
        assert_eq!(env.get(&local(1)), Some(&abstract_value));
        frame.cleanup(&mut env);
        assert_eq!(
            env,
            Environment::from([(local(998), abstract_value.clone())])
        );
    }
}

#[test]
fn recursive_application_keeps_caller_binders_in_ready_and_resumed_operands() {
    let ty = Term::Primitive(Primitive::Type);
    let boolean = Term::Primitive(Primitive::Bool);
    let byte = Term::Primitive(Primitive::U8);
    let rigid = |n| Term::Rigid {
        declaration: id(n),
        sort: nir::Sort::Type,
    };
    // Outer B and inner B have the same declaration. This application supplies
    // A = outer B, B = bool, T = u8, and checks its last operand at (A, T).
    let template = Term::Tuple(vec![rigid(0), rigid(2)]);
    let domains = vec![ty.clone(), ty.clone(), ty.clone(), template];
    let parameters = domains
        .iter()
        .enumerate()
        .map(|(slot, domain)| Parameter {
            declaration: id(slot as u32),
            binding: local(slot as u32),
            kind: if slot < 3 { Kind::Type } else { Kind::Value },
            ty: domain.clone(),
        })
        .collect();
    let package = Package {
        normal_forms: vec![Head {
            declaration: id(100),
            collector: None,
            role: Role::Opaque,
            requirements: vec![],
            parameters,
        }],
        alias_contracts: vec![nir::type_aliases::Contract {
            declaration: id(101),
            parameters: (0..4).map(id).collect(),
            parameter_types: domains,
            requirements: vec![],
            capabilities: vec![],
        }],
        ..Default::default()
    };
    let outer = Value::AbstractValue {
        declaration: id(42),
        ty: rigid(1),
    };
    for tree in [true, false] {
        let mut vm = if tree {
            Interpreter::for_comptime_tree(&package, Budget::default()).unwrap()
        } else {
            Interpreter::for_comptime(&package, Budget::default()).unwrap()
        };
        vm.authorize_abstract_type_parameters(&[id(1)]).unwrap();
        vm.authorize_abstract_value_parameters(&[(id(42), rigid(1))])
            .unwrap();
        for staged in [false, true] {
            let type_value = |value| expression(ty.clone(), K::TypeValue(value));
            let mut inner_b = type_value(boolean.clone());
            if !staged {
                // T already arrived. Reading the exported operand before
                // supplying inner B exposes any spurious wait on its identity.
                inner_b = expression(
                    ty.clone(),
                    K::Block {
                        statements: vec![expression(
                            Term::Tuple(vec![rigid(0), byte.clone()]),
                            K::Local(local(3)),
                        )],
                        result: Some(Box::new(inner_b)),
                    },
                );
            }
            let arguments = vec![
                Argument::Value(type_value(rigid(1))),
                Argument::Value(inner_b),
                Argument::Value(type_value(byte.clone())),
                Argument::Tuple {
                    source: SOURCE,
                    items: vec![
                        Argument::Value(expression(rigid(0), K::Local(local(42)))),
                        Argument::Integer {
                            source: SOURCE,
                            spelling: "7".into(),
                            negative: false,
                        },
                    ],
                },
            ];
            let plan = NormalFormInputPlan {
                order: if staged {
                    vec![0, 3, 1, 2]
                } else {
                    vec![0, 2, 3, 1]
                },
                bindings: (0..4).map(local).collect(),
                preparations: vec![],
            };
            let normal = expression(
                ty.clone(),
                K::NormalFormType {
                    head: id(100),
                    arguments: arguments.clone(),
                    plan: Some(plan.clone()),
                },
            );
            let alias = expression(
                ty.clone(),
                K::CheckTypeAlias {
                    declaration: id(101),
                    body: Some(Box::new(type_value(rigid(0)))),
                    arguments,
                    order: Some(plan.order),
                    bindings: Some(plan.bindings),
                    preparations: vec![],
                },
            );
            let mut env = Environment::from([(local(42), outer.clone())]);
            for (input, expected) in [
                (
                    normal,
                    Value::Type(Term::NormalForm {
                        head: id(100),
                        sort: nir::Sort::Type,
                        arguments: vec![
                            rigid(1),
                            boolean.clone(),
                            byte.clone(),
                            Term::TupleValue(vec![
                                Term::Rigid {
                                    declaration: id(42),
                                    sort: nir::Sort::ComptimeValue,
                                },
                                Term::Integer(comptime_runtime::Integer::from_u128(7)),
                            ]),
                        ],
                    }),
                ),
                (alias, Value::TypeParameter(id(1))),
            ] {
                let flow = vm
                    .eval(&input, &mut env)
                    .unwrap_or_else(|err| panic!("tree={tree} staged={staged} {input}: {err:?}"));
                let Flow::Value(actual) = flow else {
                    panic!("unexpected transfer")
                };
                assert_eq!(actual, expected, "tree={tree} staged={staged}: {input}");
                assert_eq!(env, Environment::from([(local(42), outer.clone())]));
            }
        }
    }
}
