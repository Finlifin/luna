//! Type equality uses host proofs and retains opaque normal-form identity.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Budget, ContractOutcome, ErrorKind, Interpreter, Value};
use nir::{Primitive, Term};

#[test]
fn type_equality_uses_proofs_without_unfolding_heads() {
    let package = support::compile(
        "typealias Type = .Type'builtin;
        typealias char = .char'builtin;
        pure comptime fn Identity<T> -> Type { T }
        comptime fn compare(left: Type, right: Type) -> bool { left == right }
        comptime fn Opaque() -> Type { Identity<bool> }
        comptime fn ordered() -> bool {
            var trace = false;
            let same = (do { trace = true; bool }) == (do { if trace { bool } else { char } });
            same and trace
        }
        comptime fn early() -> bool {
            (do { return true; bool }) == (do { while true {} bool });
            false
        }",
    );
    let context = elaboration::ptc::Context::from_package(&package).unwrap();
    let solver = |predicate: &nir::program::TypePredicate| {
        use elaboration::ptc::{Budget, Formula, Outcome};
        match context.prove(Formula::from_predicate(predicate), Budget::default()) {
            Outcome::Entailed(_) => ContractOutcome::Satisfied,
            Outcome::NotEntailed(_) => ContractOutcome::Rejected,
            _ => ContractOutcome::Unknown,
        }
    };
    let mut vm = Interpreter::for_comptime(&package, Budget::default())
        .unwrap()
        .with_type_solver(&solver);
    let compare = support::id(&package, "compare");
    for (right, expected) in [(Primitive::Bool, true), (Primitive::Char, false)] {
        let before = vm.bytecode_invocations();
        assert_eq!(
            vm.invoke(
                compare,
                vec![
                    Value::Type(Term::Primitive(Primitive::Bool)),
                    Value::Type(Term::Primitive(right))
                ]
            )
            .unwrap(),
            Value::Bool(expected)
        );
        assert_eq!(vm.bytecode_invocations() - before, 1);
    }
    // Construction and comparison retain the same opaque head identity.
    let opaque = vm.invoke(support::id(&package, "Opaque"), vec![]).unwrap();
    let before = vm.bytecode_invocations();
    assert_eq!(
        vm.invoke(
            compare,
            vec![opaque, Value::Type(Term::Primitive(Primitive::Bool))]
        )
        .unwrap_err()
        .kind,
        ErrorKind::TypeContractUnresolved
    );
    assert_eq!(vm.bytecode_invocations() - before, 1);
    for name in ["ordered", "early"] {
        let before = vm.bytecode_invocations();
        assert_eq!(
            vm.invoke(support::id(&package, name), vec![]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(vm.bytecode_invocations() - before, 1);
    }
}

#[test]
fn structural_type_factories_preserve_phase_sorts_and_environment() {
    let package = support::compile(
        "typealias Type = .Type'builtin;
        comptime fn Mapper(input: Type, output: Type) -> Type { fn(input) -> output }
        comptime fn Staged(input: Type, output: Type) -> Type { comptime fn(comptime input, output) -> output }
        comptime fn Reader(input: Type, output: Type) -> Type { %{saved: input} comptime fn() -> output }
        comptime fn Pointer(input: Type) -> Type { *input }
        comptime fn Optional(input: Type) -> Type { ?input }
        comptime fn Empty(input: Type) -> Type { %{} input }",
    );
    let boolean = Term::Primitive(Primitive::Bool);
    let character = Term::Primitive(Primitive::Char);
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut tree = Interpreter::new(&package, Budget::default()).unwrap();
    let function = |parameters, result| Term::Function {
        parameters,
        result: Box::new(result),
    };
    for (name, inputs, expected) in [
        (
            "Mapper",
            vec![boolean.clone(), character.clone()],
            function(
                vec![nir::Parameter {
                    sort: nir::Sort::Runtime,
                    ty: boolean.clone(),
                }],
                character.clone(),
            ),
        ),
        (
            "Staged",
            vec![Term::Primitive(Primitive::Type), character.clone()],
            function(
                vec![
                    nir::Parameter {
                        sort: nir::Sort::Type,
                        ty: Term::Primitive(Primitive::Type),
                    },
                    nir::Parameter {
                        sort: nir::Sort::Runtime,
                        ty: character.clone(),
                    },
                ],
                character.clone(),
            )
            .with_comptime_phase(true),
        ),
        (
            "Staged",
            vec![boolean.clone(), character.clone()],
            function(
                vec![
                    nir::Parameter {
                        sort: nir::Sort::ComptimeValue,
                        ty: boolean.clone(),
                    },
                    nir::Parameter {
                        sort: nir::Sort::Runtime,
                        ty: character.clone(),
                    },
                ],
                character.clone(),
            )
            .with_comptime_phase(true),
        ),
        (
            "Reader",
            vec![boolean.clone(), character.clone()],
            Term::Closure {
                environment: vec![boolean.clone()],
                signature: Box::new(function(vec![], character.clone()).with_comptime_phase(true)),
            },
        ),
        (
            "Pointer",
            vec![boolean.clone()],
            Term::Pointer(Box::new(boolean.clone())),
        ),
        (
            "Optional",
            vec![boolean.clone()],
            Term::Optional(Box::new(boolean.clone())),
        ),
        ("Empty", vec![boolean.clone()], boolean.clone()),
    ] {
        let before = vm.bytecode_invocations();
        let inputs: Vec<_> = inputs.into_iter().map(Value::Type).collect();
        let id = support::id(&package, name);
        let result = vm.invoke(id, inputs.clone()).unwrap();
        assert_eq!(result, Value::Type(expected), "{name}");
        assert_eq!(result, tree.invoke(id, inputs).unwrap(), "{name}");
        assert_eq!(vm.bytecode_invocations() - before, 1, "{name}");
    }
}

#[test]
fn opaque_type_applications_execute_as_bytecode_without_body_reduction() {
    let package = support::compile(
        "typealias Type = .Type'builtin;
        typealias char = .char'builtin;
        pure comptime fn Identity<T> -> Type { T }
        comptime fn Wrapped(input: Type) -> Type { Identity<input> }
        comptime fn Nested(input: Type) -> Type { Identity<Identity<input>> }",
    );
    let head = support::id(&package, "Identity");
    let wrapped = support::id(&package, "Wrapped");
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for primitive in [Primitive::Bool, Primitive::Char, Primitive::Bool] {
        let before = vm.bytecode_invocations();
        let result = vm
            .invoke(wrapped, vec![Value::Type(Term::Primitive(primitive))])
            .unwrap();
        assert_eq!(
            result,
            Value::Type(Term::NormalForm {
                head,
                sort: nir::Sort::Type,
                arguments: vec![Term::Primitive(primitive)],
            })
        );
        assert_eq!(vm.bytecode_invocations() - before, 1);
        let before = vm.bytecode_invocations();
        let result = vm
            .invoke(
                support::id(&package, "Nested"),
                vec![Value::Type(Term::Primitive(primitive))],
            )
            .unwrap();
        assert_eq!(
            result,
            Value::Type(Term::NormalForm {
                head,
                sort: nir::Sort::Type,
                arguments: vec![Term::NormalForm {
                    head,
                    sort: nir::Sort::Type,
                    arguments: vec![Term::Primitive(primitive)],
                }],
            })
        );
        assert_eq!(vm.bytecode_invocations() - before, 1);
    }
}

#[test]
fn contextual_integer_normal_inputs_execute_as_bytecode() {
    let package = support::compile(
        "typealias Type = .Type'builtin;
        typealias Integer = .Integer'builtin;
        pure comptime fn Shape<T, count: Integer> -> Type { T }
        comptime fn Sized(input: Type) -> Type { Shape<input, 17> }",
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut tree = Interpreter::new(&package, Budget::default()).unwrap();
    for primitive in [Primitive::Bool, Primitive::Char] {
        let before = vm.bytecode_invocations();
        let arguments = vec![Value::Type(Term::Primitive(primitive))];
        let id = support::id(&package, "Sized");
        let result = vm.invoke(id, arguments.clone()).unwrap();
        assert_eq!(result, tree.invoke(id, arguments).unwrap());
        let Value::Type(Term::NormalForm {
            head, arguments, ..
        }) = result
        else {
            panic!("opaque type")
        };
        assert_eq!(head, support::id(&package, "Shape"));
        assert_eq!(arguments[0], Term::Primitive(primitive));
        assert_eq!(vm.bytecode_invocations() - before, 1);
    }
}

#[test]
fn contextual_normal_containers_execute_with_ordinary_list_identity() {
    let package = support::compile_with_tuple(
        "typealias Type = .Type'builtin;
        typealias Integer = .Integer'builtin;
        typealias List<T> = TupleElements<T>;
        pure comptime fn Shape<T, values: List<List<Integer>>> -> Type { T }
        comptime fn NestedLists(input: Type) -> Type {
            Shape<input, [[17], [23, 29]]>
        }
        comptime fn Pair(input: Type) -> Type { Tuple<.Es=[input, bool]> }
        pure comptime fn Payload<T, value: T> -> Type { T }
        comptime fn WithList(element: Type) -> Type { Payload<List<element>, [17, 23]> }",
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for name in ["NestedLists", "Pair"] {
        for primitive in [Primitive::Bool, Primitive::Char] {
            let arguments = vec![Value::Type(Term::Primitive(primitive))];
            let id = support::id(&package, name);
            let before = vm.bytecode_invocations();
            let actual = vm.invoke(id, arguments.clone()).unwrap();
            let list_head = support::id(&package, "TupleElements");
            let list_type = |element| Term::NormalForm {
                head: list_head,
                sort: nir::Sort::Type,
                arguments: vec![element],
            };
            let integer_list = list_type(Term::Primitive(Primitive::Integer));
            let row = |numbers: &[u128]| Term::NominalValue {
                ty: Box::new(integer_list.clone()),
                variant: None,
                fields: numbers
                    .iter()
                    .map(|number| Term::Integer(comptime_runtime::Integer::from_u128(*number)))
                    .collect(),
            };
            let arguments = if name == "Pair" {
                vec![Term::TypeListValue(vec![
                    Term::Primitive(primitive),
                    Term::Primitive(Primitive::Bool),
                ])]
            } else {
                vec![
                    Term::Primitive(primitive),
                    Term::NominalValue {
                        ty: Box::new(list_type(integer_list.clone())),
                        variant: None,
                        fields: vec![row(&[17]), row(&[23, 29])],
                    },
                ]
            };
            assert_eq!(
                actual,
                Value::Type(Term::NormalForm {
                    head: support::id(&package, if name == "Pair" { "Tuple" } else { "Shape" }),
                    sort: nir::Sort::Type,
                    arguments,
                })
            );
            assert_eq!(vm.bytecode_invocations() - before, 1, "{name}");
        }
    }
    for element in [Primitive::U8, Primitive::Integer] {
        let before = vm.bytecode_invocations();
        let actual = vm
            .invoke(
                support::id(&package, "WithList"),
                vec![Value::Type(Term::Primitive(element))],
            )
            .unwrap();
        let list = Term::NormalForm {
            head: support::id(&package, "TupleElements"),
            sort: nir::Sort::Type,
            arguments: vec![Term::Primitive(element)],
        };
        assert_eq!(
            actual,
            Value::Type(Term::NormalForm {
                head: support::id(&package, "Payload"),
                sort: nir::Sort::Type,
                arguments: vec![
                    list.clone(),
                    Term::NominalValue {
                        ty: Box::new(list),
                        variant: None,
                        fields: vec![
                            Term::Integer(comptime_runtime::Integer::from_u128(17)),
                            Term::Integer(comptime_runtime::Integer::from_u128(23))
                        ],
                    }
                ],
            })
        );
        assert_eq!(vm.bytecode_invocations() - before, 1);
    }
}

#[test]
fn contextual_normal_control_flow_executes_in_the_vm() {
    let package = support::compile(
        "typealias Type = .Type'builtin;
        pure comptime fn Payload<T, value: T> -> Type { T }
        comptime fn Conditional(input: Type, flag: bool) -> Type {
            Payload<input, if flag { 17 } else { do { let local = false; 23 } }>
        }
        comptime fn Limited(input: Type, flag: bool) -> Type {
            Payload<input, if flag { 17 } else { 256 }>
        }
        comptime fn Early(input: Type) -> Type {
            Payload<input, do { return bool; 17 }>
        }",
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for primitive in [Primitive::U8, Primitive::Integer] {
        for flag in [false, true] {
            let before = vm.bytecode_invocations();
            let result = vm
                .invoke(
                    support::id(&package, "Conditional"),
                    vec![Value::Type(Term::Primitive(primitive)), Value::Bool(flag)],
                )
                .unwrap();
            assert_eq!(
                result,
                Value::Type(Term::NormalForm {
                    head: support::id(&package, "Payload"),
                    sort: nir::Sort::Type,
                    arguments: vec![
                        Term::Primitive(primitive),
                        Term::Integer(comptime_runtime::Integer::from_u128(if flag {
                            17
                        } else {
                            23
                        }))
                    ],
                })
            );
            assert_eq!(vm.bytecode_invocations() - before, 1);
        }
    }
    let before = vm.bytecode_invocations();
    assert_eq!(
        vm.invoke(
            support::id(&package, "Limited"),
            vec![
                Value::Type(Term::Primitive(Primitive::U8)),
                Value::Bool(true)
            ],
        )
        .unwrap_err()
        .kind,
        ErrorKind::Type
    );
    assert_eq!(vm.bytecode_invocations() - before, 1);
    let before = vm.bytecode_invocations();
    assert_eq!(
        vm.invoke(
            support::id(&package, "Early"),
            vec![Value::Type(Term::Primitive(Primitive::U8))]
        )
        .unwrap(),
        Value::Type(Term::Primitive(Primitive::Bool))
    );
    assert_eq!(vm.bytecode_invocations() - before, 1);
}

#[test]
fn contextual_normal_matches_execute_selected_arms_in_bytecode() {
    let package = support::compile(
        "typealias Type = .Type'builtin;
        pure comptime fn Payload<T, value: T> -> Type { T }
        comptime fn Matched(input: Type, flag: bool) -> Type {
            Payload<input, flag match { bound if not bound => 17, _ => 23, }>
        }
        comptime fn LimitedMatch(input: Type, flag: bool) -> Type {
            Payload<input, flag match { true => 17, false => 256, }>
        }
        comptime fn EarlyMatch(input: Type) -> Type {
            Payload<input, true match { true => do { return bool; 17 }, false => 23, }>
        }",
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for primitive in [Primitive::U8, Primitive::Integer] {
        for flag in [false, true] {
            let before = vm.bytecode_invocations();
            let value = vm
                .invoke(
                    support::id(&package, "Matched"),
                    vec![Value::Type(Term::Primitive(primitive)), Value::Bool(flag)],
                )
                .unwrap();
            assert_eq!(
                value,
                Value::Type(Term::NormalForm {
                    head: support::id(&package, "Payload"),
                    sort: nir::Sort::Type,
                    arguments: vec![
                        Term::Primitive(primitive),
                        Term::Integer(comptime_runtime::Integer::from_u128(if flag {
                            23
                        } else {
                            17
                        }))
                    ],
                })
            );
            assert_eq!(vm.bytecode_invocations() - before, 1);
        }
    }
    let before = vm.bytecode_invocations();
    assert_eq!(
        vm.invoke(
            support::id(&package, "LimitedMatch"),
            vec![
                Value::Type(Term::Primitive(Primitive::U8)),
                Value::Bool(true)
            ]
        )
        .unwrap_err()
        .kind,
        ErrorKind::Type
    );
    assert_eq!(vm.bytecode_invocations() - before, 1);
    let before = vm.bytecode_invocations();
    assert_eq!(
        vm.invoke(
            support::id(&package, "EarlyMatch"),
            vec![Value::Type(Term::Primitive(Primitive::U8))]
        )
        .unwrap(),
        Value::Type(Term::Primitive(Primitive::Bool))
    );
    assert_eq!(vm.bytecode_invocations() - before, 1);
}

#[test]
fn deferred_normal_inputs_run_bytecode_before_their_domains_arrive() {
    let package = support::compile_with_tuple(
        "typealias Type = .Type'builtin;
        typealias List<T> = TupleElements<T>;
        pure comptime fn Payload<.T: Type = Type, value: T> -> Type { T }
        comptime fn enabled(.flag: bool = true) -> bool { flag }
        comptime fn Deferred(element: Type) -> Type {
            var touched = false;
            let result = Payload<do {
                touched = not touched;
                if enabled() { [17, 23] } else { [29] }
            }, .T=if touched { List<element> } else { bool }>;
            if touched { result } else { bool }
        }
        comptime fn Ready(element: Type) -> Type {
            var touched = false;
            let result = Payload<.T=List<element>, do {
                touched = not touched;
                if enabled() { [17, 23] } else { [29] }
            }>;
            if touched { result } else { bool }
        }
        comptime fn ReadyEarly(element: Type) -> Type {
            Payload<.T=List<element>, do { return bool; [17, 23] }>
        }
        comptime fn Early(element: Type) -> Type {
            Payload<do { return bool; [17, 23] }, .T=do { while true {} List<element> }>
        }",
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for primitive in [Primitive::U8, Primitive::Integer, Primitive::U8] {
        let list = Term::NormalForm {
            head: support::id(&package, "TupleElements"),
            sort: nir::Sort::Type,
            arguments: vec![Term::Primitive(primitive)],
        };
        let before = vm.bytecode_staged_inputs();
        let contextual = vm.bytecode_contextual_inputs();
        let defaults = vm.bytecode_default_evaluations();
        assert_eq!(
            vm.invoke(
                support::id(&package, "Ready"),
                vec![Value::Type(Term::Primitive(primitive))]
            )
            .unwrap(),
            Value::Type(Term::NormalForm {
                head: support::id(&package, "Payload"),
                sort: nir::Sort::Type,
                arguments: vec![
                    list.clone(),
                    Term::NominalValue {
                        ty: Box::new(list.clone()),
                        variant: None,
                        fields: [17, 23]
                            .into_iter()
                            .map(|n| Term::Integer(comptime_runtime::Integer::from_u128(n)))
                            .collect(),
                    }
                ],
            })
        );
        assert_eq!(vm.bytecode_staged_inputs() - before, 0);
        assert_eq!(vm.bytecode_contextual_inputs() - contextual, 2);
        assert_eq!(vm.bytecode_default_evaluations() - defaults, 1);
        let before = vm.bytecode_staged_inputs();
        let defaults = vm.bytecode_default_evaluations();
        assert_eq!(
            vm.invoke(
                support::id(&package, "Deferred"),
                vec![Value::Type(Term::Primitive(primitive))]
            )
            .unwrap(),
            Value::Type(Term::NormalForm {
                head: support::id(&package, "Payload"),
                sort: nir::Sort::Type,
                arguments: vec![
                    list.clone(),
                    Term::NominalValue {
                        ty: Box::new(list.clone()),
                        variant: None,
                        fields: [17, 23]
                            .into_iter()
                            .map(|n| Term::Integer(comptime_runtime::Integer::from_u128(n)))
                            .collect(),
                    }
                ],
            })
        );
        assert_eq!(vm.bytecode_staged_inputs() - before, 1);
        assert_eq!(vm.bytecode_default_evaluations() - defaults, 1);
        let before = vm.bytecode_staged_inputs();
        let invocations = vm.bytecode_invocations();
        assert_eq!(
            vm.invoke(
                support::id(&package, "Early"),
                vec![Value::Type(Term::Primitive(primitive))]
            )
            .unwrap(),
            Value::Type(Term::Primitive(Primitive::Bool))
        );
        assert_eq!(vm.bytecode_staged_inputs() - before, 1);
        assert_eq!(vm.bytecode_invocations() - invocations, 1);
        let invocations = vm.bytecode_invocations();
        let contextual = vm.bytecode_contextual_inputs();
        assert_eq!(
            vm.invoke(
                support::id(&package, "ReadyEarly"),
                vec![Value::Type(Term::Primitive(primitive))]
            )
            .unwrap(),
            Value::Type(Term::Primitive(Primitive::Bool))
        );
        assert_eq!(vm.bytecode_contextual_inputs() - contextual, 2);
        assert_eq!(vm.bytecode_invocations() - invocations, 1);
    }
}

#[test]
fn nested_reordered_normal_plans_suspend_compiled_callers() {
    let package = support::compile(
        "typealias Type = .Type'builtin;
        typealias u8 = .u8'builtin;
        pure comptime fn Tag<.T: Type = Type, value: T> -> Type { T }
        comptime fn Build() -> Type {
            var touched = false;
            let result = Tag<Tag<do { touched = not touched; 7 }, .T=u8>, .T=Type>;
            if touched { result } else { bool }
        }",
    );
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut tree = Interpreter::new(&package, Budget::default()).unwrap();
    let head = support::id(&package, "Tag");
    let expected = Value::Type(Term::NormalForm {
        head,
        sort: nir::Sort::Type,
        arguments: vec![
            Term::Primitive(Primitive::Type),
            Term::NormalForm {
                head,
                sort: nir::Sort::Type,
                arguments: vec![
                    Term::Primitive(Primitive::U8),
                    Term::Integer(comptime_runtime::Integer::from_u128(7)),
                ],
            },
        ],
    });
    for _ in 0..3 {
        let invocations = vm.bytecode_invocations();
        let staged = vm.bytecode_staged_inputs();
        assert_eq!(
            vm.invoke(support::id(&package, "Build"), vec![]).unwrap(),
            expected
        );
        assert_eq!(
            tree.invoke(support::id(&package, "Build"), vec![]).unwrap(),
            expected
        );
        assert_eq!(vm.bytecode_invocations() - invocations, 1);
        assert_eq!(vm.bytecode_staged_inputs() - staged, 2);
    }
}
