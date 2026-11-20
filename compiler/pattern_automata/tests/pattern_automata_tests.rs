use pattern_automata::{
    BitInterval, BitSegmentPattern, BitValue, BitVecPattern, CompileBudget, CompileError,
    CompileOutput, EnumType, ErrorRowType, ErrorTag, IntLit, IntType, MatchArm, MatchOutcome,
    MissingWitness, Pattern, PatternMatrix, RangeBound, TypeDesc, Value, VariantPayloadType,
    VariantSchema, VariantTag, compile, compile_with_budget, eval_dag, eval_linear,
};
use std::collections::BTreeMap;

fn assert_balanced_sexpr_no_newlines(rendered: &str) {
    assert!(
        !rendered.contains('\n') && !rendered.contains('\r'),
        "S-expression must not contain raw newlines: {rendered}"
    );
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escaped = false;
    for ch in rendered.chars() {
        if in_str {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_str = false;
            }
            continue;
        }
        match ch {
            '"' => in_str = true,
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                assert!(depth >= 0, "unbalanced ')' in S-expression: {rendered}");
            }
            _ => {}
        }
    }
    assert!(!in_str, "unterminated string in S-expression: {rendered}");
    assert_eq!(depth, 0, "unclosed '(' in S-expression: {rendered}");
}

fn assert_differential_with_trace<F>(
    output: &CompileOutput,
    arms: &[MatchArm],
    values: &[Value],
    mut oracle: F,
) where
    F: FnMut(usize, &BTreeMap<String, Value>) -> bool,
{
    for val in values {
        let mut linear_trace = Vec::new();
        let linear_res = eval_linear(arms, val, |arm_idx, binds| {
            let pass = oracle(arm_idx, binds);
            linear_trace.push((arm_idx, binds.clone(), pass));
            pass
        });

        let mut dag_trace = Vec::new();
        let dag_res = eval_dag(&output.dag, val, |arm_idx, binds| {
            let pass = oracle(arm_idx, binds);
            dag_trace.push((arm_idx, binds.clone(), pass));
            pass
        });

        assert_eq!(
            dag_res, linear_res,
            "eval_dag != eval_linear for value {val}\nDAG: {}",
            output.dag
        );
        assert_eq!(
            dag_trace, linear_trace,
            "guard evaluation trace mismatch for value {val}\nDAG: {}",
            output.dag
        );
    }
}

fn verify_witnesses_unmatched(output: &CompileOutput, arms: &[MatchArm]) {
    for witness in &output.missing_witnesses {
        assert_balanced_sexpr_no_newlines(&witness.to_string());
        if let Some(val) = witness.to_value() {
            let res = eval_linear(arms, &val, |_, _| false);
            assert!(
                res.is_none(),
                "witness {witness} ({val}) unexpectedly matched without guard pass: {res:?}"
            );
        }
    }
}

fn sample_closed_enum() -> TypeDesc {
    let mut variants = BTreeMap::new();
    variants.insert(
        VariantTag::named("Idle"),
        VariantSchema {
            discrete_code: Some(0),
            payload: VariantPayloadType::Unit,
        },
    );
    variants.insert(
        VariantTag::named("Pair"),
        VariantSchema {
            discrete_code: Some(1),
            payload: VariantPayloadType::Tuple(vec![TypeDesc::Bool, TypeDesc::Bool]),
        },
    );
    variants.insert(
        VariantTag::named("Move"),
        VariantSchema {
            discrete_code: Some(2),
            payload: VariantPayloadType::Record(BTreeMap::from([
                ("x".to_string(), TypeDesc::Int(IntType::UnsignedBits(8))),
                ("y".to_string(), TypeDesc::Bool),
            ])),
        },
    );
    TypeDesc::Enum(EnumType::closed("Msg", variants))
}

fn sample_error_row(is_open: bool) -> TypeDesc {
    let mut io_variants = BTreeMap::new();
    io_variants.insert(
        VariantTag::named("NotFound"),
        VariantSchema {
            discrete_code: None,
            payload: VariantPayloadType::Unit,
        },
    );
    io_variants.insert(
        VariantTag::named("Retry"),
        VariantSchema {
            discrete_code: None,
            payload: VariantPayloadType::Tuple(vec![TypeDesc::Bool]),
        },
    );
    let io_enum = TypeDesc::Enum(EnumType::closed("IoErr", io_variants));

    let mut parse_variants = BTreeMap::new();
    parse_variants.insert(
        VariantTag::named("Invalid"),
        VariantSchema {
            discrete_code: None,
            payload: VariantPayloadType::Unit,
        },
    );
    let parse_enum = TypeDesc::Enum(EnumType::closed("ParseErr", parse_variants));

    let mut error_tags = BTreeMap::new();
    error_tags.insert(ErrorTag::new("IoErr"), io_enum);
    error_tags.insert(ErrorTag::new("ParseErr"), parse_enum);

    TypeDesc::ErrorRow(ErrorRowType {
        ok_type: Box::new(TypeDesc::Optional(Box::new(TypeDesc::Bool))),
        error_tags,
        is_open,
    })
}

#[test]
fn differential_exhaustive_bool_tuple_optional_enum_and_error_row() {
    let err_ty = sample_error_row(false);
    let arms = vec![
        MatchArm::unguarded(0, Pattern::ok(Pattern::OptionalNull)).unwrap(),
        MatchArm::unguarded(
            1,
            Pattern::ok(Pattern::present(Pattern::bind_as("b", Pattern::Bool(true)))),
        )
        .unwrap(),
        MatchArm::unguarded(2, Pattern::ok(Pattern::present(Pattern::Bool(false)))).unwrap(),
        MatchArm::unguarded(
            3,
            Pattern::err_tag(
                "IoErr",
                Pattern::unit_variant(VariantTag::named("NotFound")),
            ),
        )
        .unwrap(),
        MatchArm::unguarded(
            4,
            Pattern::err_tag(
                "IoErr",
                Pattern::tuple_variant(VariantTag::named("Retry"), vec![Pattern::var("flag")]),
            ),
        )
        .unwrap(),
        MatchArm::unguarded(
            5,
            Pattern::err_tag(
                "ParseErr",
                Pattern::unit_variant(VariantTag::named("Invalid")),
            ),
        )
        .unwrap(),
    ];

    let matrix = PatternMatrix::new(err_ty, arms.clone()).unwrap();
    let out = compile(&matrix).unwrap();
    assert!(out.is_exhaustive);
    assert!(out.unreachable_arms.is_empty());
    assert!(out.missing_witnesses.is_empty());

    let values = vec![
        Value::ok(Value::OptionalNull),
        Value::ok(Value::present(Value::Bool(true))),
        Value::ok(Value::present(Value::Bool(false))),
        Value::err("IoErr", Value::unit_variant(VariantTag::named("NotFound"))),
        Value::err(
            "IoErr",
            Value::tuple_variant(VariantTag::named("Retry"), vec![Value::Bool(false)]),
        ),
        Value::err(
            "IoErr",
            Value::tuple_variant(VariantTag::named("Retry"), vec![Value::Bool(true)]),
        ),
        Value::err(
            "ParseErr",
            Value::unit_variant(VariantTag::named("Invalid")),
        ),
    ];
    assert_differential_with_trace(&out, &arms, &values, |_, _| false);
}

#[test]
fn differential_enum_record_omitted_fields_and_discrete_tags() {
    let msg_ty = sample_closed_enum();
    let arms = vec![
        MatchArm::unguarded(
            0,
            Pattern::record_variant(
                VariantTag::named("Move"),
                BTreeMap::from([("y".to_string(), Pattern::Bool(true))]),
            ),
        )
        .unwrap(),
        MatchArm::unguarded(
            1,
            Pattern::bind_as(
                "whole",
                Pattern::record_variant(
                    VariantTag::named("Move"),
                    BTreeMap::from([(
                        "x".to_string(),
                        Pattern::IntRange {
                            start: RangeBound::Inclusive(IntLit::Unsigned(10)),
                            end: RangeBound::Exclusive(IntLit::Unsigned(20)),
                        },
                    )]),
                ),
            ),
        )
        .unwrap(),
        MatchArm::unguarded(
            2,
            Pattern::tuple_variant(
                VariantTag::named("Pair"),
                vec![Pattern::Bool(true), Pattern::var("p")],
            ),
        )
        .unwrap(),
        MatchArm::unguarded(3, Pattern::unit_variant(VariantTag::named("Idle"))).unwrap(),
        MatchArm::unguarded(4, Pattern::Wild).unwrap(),
    ];

    let matrix = PatternMatrix::new(msg_ty, arms.clone()).unwrap();
    let out = compile(&matrix).unwrap();
    assert!(out.is_exhaustive);
    assert!(out.unreachable_arms.is_empty());

    let mut values = vec![
        Value::unit_variant(VariantTag::named("Idle")),
        Value::tuple_variant(
            VariantTag::named("Pair"),
            vec![Value::Bool(false), Value::Bool(true)],
        ),
        Value::tuple_variant(
            VariantTag::named("Pair"),
            vec![Value::Bool(true), Value::Bool(false)],
        ),
        Value::tuple_variant(
            VariantTag::named("Pair"),
            vec![Value::Bool(true), Value::Bool(true)],
        ),
    ];
    for x in [0u128, 9, 10, 15, 19, 20, 255] {
        for y in [false, true] {
            values.push(Value::record_variant(
                VariantTag::named("Move"),
                BTreeMap::from([
                    ("x".to_string(), Value::unsigned(x)),
                    ("y".to_string(), Value::Bool(y)),
                ]),
            ));
        }
    }
    assert_differential_with_trace(&out, &arms, &values, |_, _| true);
}

#[test]
fn guard_fallthrough_preserves_exact_source_order_and_or_single_attempt() {
    let ty = TypeDesc::Optional(Box::new(TypeDesc::Int(IntType::SignedBits(32))));
    let arms = vec![
        MatchArm::guarded(0, Pattern::present(Pattern::var("x"))).unwrap(),
        MatchArm::unguarded(1, Pattern::present(Pattern::var("y"))).unwrap(),
        MatchArm::unguarded(2, Pattern::OptionalNull).unwrap(),
        MatchArm::unguarded(3, Pattern::Wild).unwrap(),
    ];
    let matrix = PatternMatrix::new(ty, arms.clone()).unwrap();
    let out = compile(&matrix).unwrap();
    assert!(out.is_exhaustive);
    // Arm 0 is guarded so Arm 1 IS reachable; Arms 1 & 2 exhaust ?i32 so Arm 3 IS unreachable.
    assert_eq!(out.unreachable_arms, vec![3]);

    let values = vec![
        Value::OptionalNull,
        Value::present(Value::signed(-5)),
        Value::present(Value::signed(0)),
        Value::present(Value::signed(10)),
    ];
    assert_differential_with_trace(&out, &arms, &values, |arm_idx, binds| {
        if arm_idx == 0 {
            matches!(binds.get("x"), Some(Value::Int(IntLit::Signed(n))) if *n > 0)
        } else {
            true
        }
    });

    // Guarded Or-pattern: failing guard must NOT retry second Or alternative of arm 0.
    let pair_ty = TypeDesc::Tuple(vec![
        TypeDesc::Int(IntType::UnsignedBits(8)),
        TypeDesc::Int(IntType::UnsignedBits(8)),
    ]);
    let or_arms = vec![
        MatchArm::guarded(
            0,
            Pattern::Or(vec![
                Pattern::Tuple(vec![Pattern::unsigned(1), Pattern::var("x")]),
                Pattern::Tuple(vec![Pattern::Wild, Pattern::var("x")]),
            ]),
        )
        .unwrap(),
        MatchArm::unguarded(1, Pattern::Wild).unwrap(),
    ];
    let or_matrix = PatternMatrix::new(pair_ty, or_arms.clone()).unwrap();
    let or_out = compile(&or_matrix).unwrap();
    assert!(or_out.is_exhaustive);
    assert!(or_out.unreachable_arms.is_empty());

    let pair_values = vec![
        Value::Tuple(vec![Value::unsigned(1), Value::unsigned(5)]),
        Value::Tuple(vec![Value::unsigned(1), Value::unsigned(25)]),
        Value::Tuple(vec![Value::unsigned(2), Value::unsigned(5)]),
        Value::Tuple(vec![Value::unsigned(2), Value::unsigned(25)]),
    ];
    assert_differential_with_trace(
        &or_out,
        &or_arms,
        &pair_values,
        |_arm_idx, binds| matches!(binds.get("x"), Some(Value::Int(IntLit::Unsigned(n))) if *n > 10),
    );
}

#[test]
fn multi_column_or_pattern_avoids_exponential_dag_blowup() {
    // 25 columns of (1 or 2), 2^25 = 33,554,432 combinations!
    let k = 25usize;
    let ty = TypeDesc::Tuple(vec![TypeDesc::Int(IntType::UnsignedBits(32)); k]);
    let one_or_two = Pattern::Or(vec![Pattern::unsigned(1), Pattern::unsigned(2)]);
    let arms = vec![
        MatchArm::unguarded(0, Pattern::Tuple(vec![one_or_two; k])).unwrap(),
        MatchArm::unguarded(1, Pattern::Wild).unwrap(),
    ];
    let matrix = PatternMatrix::new(ty, arms.clone()).unwrap();
    let out = compile(&matrix).unwrap();
    assert!(out.is_exhaustive);
    assert!(out.unreachable_arms.is_empty());
    assert!(
        out.dag.node_count() <= 40,
        "expected linear DAG sharing for 2^25 Or-patterns, got {} nodes",
        out.dag.node_count()
    );
    assert!(out.stats.memo_hits >= k - 1);

    let test_vals = vec![
        Value::Tuple(vec![Value::unsigned(1); k]),
        Value::Tuple(vec![Value::unsigned(2); k]),
        Value::Tuple(
            (0..k)
                .map(|i| Value::unsigned(if i % 2 == 0 { 1 } else { 2 }))
                .collect(),
        ),
        Value::Tuple({
            let mut v = vec![Value::unsigned(1); k];
            v[k - 1] = Value::unsigned(3);
            v
        }),
    ];
    assert_differential_with_trace(&out, &arms, &test_vals, |_, _| true);

    // Nested tuple Or-patterns: 18 columns of ((1, 2) or (3, 4)) -> 2^18 combinations.
    let m = 18usize;
    let pair_ty = TypeDesc::Tuple(vec![
        TypeDesc::Int(IntType::UnsignedBits(16)),
        TypeDesc::Int(IntType::UnsignedBits(16)),
    ]);
    let tuple_or = Pattern::Or(vec![
        Pattern::Tuple(vec![Pattern::unsigned(1), Pattern::unsigned(2)]),
        Pattern::Tuple(vec![Pattern::unsigned(3), Pattern::unsigned(4)]),
    ]);
    let nested_arms = vec![
        MatchArm::unguarded(0, Pattern::Tuple(vec![tuple_or; m])).unwrap(),
        MatchArm::unguarded(1, Pattern::Wild).unwrap(),
    ];
    let nested_matrix =
        PatternMatrix::new(TypeDesc::Tuple(vec![pair_ty; m]), nested_arms.clone()).unwrap();
    let nested_out = compile(&nested_matrix).unwrap();
    assert!(nested_out.is_exhaustive);
    assert!(
        nested_out.dag.node_count() <= 80,
        "expected linear DAG for 2^18 nested tuple Or-patterns, got {} nodes",
        nested_out.dag.node_count()
    );

    // Diverging-place bindings across 20 columns: ((true, x_i, _) or (false, _, x_i)).
    let d = 20usize;
    let triple_ty = TypeDesc::Tuple(vec![
        TypeDesc::Bool,
        TypeDesc::Int(IntType::UnsignedBits(8)),
        TypeDesc::Int(IntType::UnsignedBits(8)),
    ]);
    let div_pats: Vec<Pattern> = (0..d)
        .map(|i| {
            let name = format!("x{i}");
            Pattern::Or(vec![
                Pattern::Tuple(vec![
                    Pattern::Bool(true),
                    Pattern::var(name.clone()),
                    Pattern::Wild,
                ]),
                Pattern::Tuple(vec![
                    Pattern::Bool(false),
                    Pattern::Wild,
                    Pattern::var(name),
                ]),
            ])
        })
        .collect();
    let div_arms = vec![MatchArm::unguarded(0, Pattern::Tuple(div_pats)).unwrap()];
    let div_matrix =
        PatternMatrix::new(TypeDesc::Tuple(vec![triple_ty; d]), div_arms.clone()).unwrap();
    let div_out = compile(&div_matrix).unwrap();
    assert!(div_out.is_exhaustive);
    assert!(
        div_out.dag.node_count() <= 80,
        "expected O(d) Bind-frame sharing for diverging Or bindings, got {} nodes",
        div_out.dag.node_count()
    );

    let div_values = vec![
        Value::Tuple(
            (0..d)
                .map(|i| {
                    Value::Tuple(vec![
                        Value::Bool(i % 2 == 0),
                        Value::unsigned((i * 3) as u128),
                        Value::unsigned((i * 7 + 1) as u128),
                    ])
                })
                .collect(),
        ),
        Value::Tuple(
            (0..d)
                .map(|i| {
                    Value::Tuple(vec![
                        Value::Bool(i % 2 != 0),
                        Value::unsigned((i * 5) as u128),
                        Value::unsigned((i * 11) as u128),
                    ])
                })
                .collect(),
        ),
    ];
    assert_differential_with_trace(&div_out, &div_arms, &div_values, |_, _| true);
}

#[test]
fn exhaustiveness_witnesses_across_open_and_closed_domains() {
    // 1. Bool missing true.
    let bool_arms = vec![MatchArm::unguarded(0, Pattern::Bool(false)).unwrap()];
    let bool_out =
        compile(&PatternMatrix::new(TypeDesc::Bool, bool_arms.clone()).unwrap()).unwrap();
    assert!(!bool_out.is_exhaustive);
    assert_eq!(bool_out.missing_witnesses, vec![MissingWitness::Bool(true)]);
    verify_witnesses_unmatched(&bool_out, &bool_arms);

    // 2. ?bool missing Present(false).
    let opt_arms = vec![
        MatchArm::unguarded(0, Pattern::OptionalNull).unwrap(),
        MatchArm::unguarded(1, Pattern::present(Pattern::Bool(true))).unwrap(),
    ];
    let opt_out = compile(
        &PatternMatrix::new(
            TypeDesc::Optional(Box::new(TypeDesc::Bool)),
            opt_arms.clone(),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(!opt_out.is_exhaustive);
    assert_eq!(
        opt_out.missing_witnesses,
        vec![MissingWitness::OptionalPresent(Box::new(
            MissingWitness::Bool(false)
        ))]
    );
    verify_witnesses_unmatched(&opt_out, &opt_arms);

    // 3. Closed !E T missing ParseErr.Invalid.
    let err_arms = vec![
        MatchArm::unguarded(0, Pattern::ok(Pattern::Wild)).unwrap(),
        MatchArm::unguarded(1, Pattern::err_tag("IoErr", Pattern::Wild)).unwrap(),
    ];
    let err_out =
        compile(&PatternMatrix::new(sample_error_row(false), err_arms.clone()).unwrap()).unwrap();
    assert!(!err_out.is_exhaustive);
    assert_eq!(
        err_out.missing_witnesses,
        vec![MissingWitness::ErrorErr {
            tag: ErrorTag::new("ParseErr"),
            inner: Box::new(MissingWitness::EnumVariant {
                tag: VariantTag::named("Invalid"),
                payload: pattern_automata::WitnessPayload::Unit,
            }),
        }]
    );
    verify_witnesses_unmatched(&err_out, &err_arms);

    // 4. Open error row without wildcard error branch reports ErrorOtherTag.
    let open_err_arms = vec![
        MatchArm::unguarded(0, Pattern::ok(Pattern::Wild)).unwrap(),
        MatchArm::unguarded(1, Pattern::err_tag("IoErr", Pattern::Wild)).unwrap(),
        MatchArm::unguarded(2, Pattern::err_tag("ParseErr", Pattern::Wild)).unwrap(),
    ];
    let open_err_out =
        compile(&PatternMatrix::new(sample_error_row(true), open_err_arms.clone()).unwrap())
            .unwrap();
    assert!(!open_err_out.is_exhaustive);
    assert!(
        open_err_out
            .missing_witnesses
            .iter()
            .any(|w| matches!(w, MissingWitness::ErrorOtherTag { .. }))
    );

    // Open error row WITH `error _` and `ok _` IS exhaustive.
    let open_err_complete = vec![
        MatchArm::unguarded(0, Pattern::ok(Pattern::Wild)).unwrap(),
        MatchArm::unguarded(1, Pattern::err_any(Pattern::Wild)).unwrap(),
    ];
    let open_err_complete_out =
        compile(&PatternMatrix::new(sample_error_row(true), open_err_complete).unwrap()).unwrap();
    assert!(open_err_complete_out.is_exhaustive);

    // 5. Integer ranges with gaps & unbounded open integer domains.
    let u8_arms = vec![
        MatchArm::unguarded(
            0,
            Pattern::IntRange {
                start: RangeBound::Inclusive(IntLit::Unsigned(0)),
                end: RangeBound::Inclusive(IntLit::Unsigned(9)),
            },
        )
        .unwrap(),
        MatchArm::unguarded(
            1,
            Pattern::IntRange {
                start: RangeBound::Inclusive(IntLit::Unsigned(20)),
                end: RangeBound::Inclusive(IntLit::Unsigned(255)),
            },
        )
        .unwrap(),
    ];
    let u8_out = compile(
        &PatternMatrix::new(TypeDesc::Int(IntType::UnsignedBits(8)), u8_arms.clone()).unwrap(),
    )
    .unwrap();
    assert!(!u8_out.is_exhaustive);
    assert_eq!(
        u8_out.missing_witnesses,
        vec![MissingWitness::Int(IntLit::Unsigned(10))]
    );
    verify_witnesses_unmatched(&u8_out, &u8_arms);

    // Open signed integer: `..0` + `0..` IS exhaustive; `..0` + `1..` misses `0`.
    let open_exhaustive = vec![
        MatchArm::unguarded(
            0,
            Pattern::IntRange {
                start: RangeBound::Unbounded,
                end: RangeBound::Exclusive(IntLit::Signed(0)),
            },
        )
        .unwrap(),
        MatchArm::unguarded(
            1,
            Pattern::IntRange {
                start: RangeBound::Inclusive(IntLit::Signed(0)),
                end: RangeBound::Unbounded,
            },
        )
        .unwrap(),
    ];
    assert!(
        compile(&PatternMatrix::new(TypeDesc::Int(IntType::OpenSigned), open_exhaustive).unwrap())
            .unwrap()
            .is_exhaustive
    );
    let open_gap = vec![
        MatchArm::unguarded(
            0,
            Pattern::IntRange {
                start: RangeBound::Unbounded,
                end: RangeBound::Exclusive(IntLit::Signed(0)),
            },
        )
        .unwrap(),
        MatchArm::unguarded(
            1,
            Pattern::IntRange {
                start: RangeBound::Inclusive(IntLit::Signed(1)),
                end: RangeBound::Unbounded,
            },
        )
        .unwrap(),
    ];
    let open_gap_out =
        compile(&PatternMatrix::new(TypeDesc::Int(IntType::OpenSigned), open_gap.clone()).unwrap())
            .unwrap();
    assert!(!open_gap_out.is_exhaustive);
    assert_eq!(
        open_gap_out.missing_witnesses,
        vec![MissingWitness::Int(IntLit::Signed(0))]
    );
    verify_witnesses_unmatched(&open_gap_out, &open_gap);

    // 6. Char surrogate gap handling: covering 0..=D7FF and E000..=10FFFF is exhaustive!
    let char_full = vec![
        MatchArm::unguarded(
            0,
            Pattern::CharRange {
                start: RangeBound::Inclusive('\u{0}'),
                end: RangeBound::Inclusive('\u{D7FF}'),
            },
        )
        .unwrap(),
        MatchArm::unguarded(
            1,
            Pattern::CharRange {
                start: RangeBound::Inclusive('\u{E000}'),
                end: RangeBound::Inclusive('\u{10FFFF}'),
            },
        )
        .unwrap(),
    ];
    assert!(
        compile(&PatternMatrix::new(TypeDesc::Char, char_full).unwrap())
            .unwrap()
            .is_exhaustive
    );

    // 7. Open discrete tag enum without `_` reports EnumOtherVariant.
    let mut disc_variants = BTreeMap::new();
    disc_variants.insert(
        VariantTag::discrete(0x01),
        VariantSchema {
            discrete_code: Some(0x01),
            payload: VariantPayloadType::Unit,
        },
    );
    disc_variants.insert(
        VariantTag::discrete(0x02),
        VariantSchema {
            discrete_code: Some(0x02),
            payload: VariantPayloadType::Unit,
        },
    );
    let open_enum = TypeDesc::Enum(EnumType::open("Opcode", disc_variants));
    let open_enum_arms = vec![
        MatchArm::unguarded(0, Pattern::unit_variant(VariantTag::discrete(0x01))).unwrap(),
        MatchArm::unguarded(1, Pattern::unit_variant(VariantTag::discrete(0x02))).unwrap(),
    ];
    let open_enum_out = compile(&PatternMatrix::new(open_enum, open_enum_arms).unwrap()).unwrap();
    assert!(!open_enum_out.is_exhaustive);
    assert_eq!(
        open_enum_out.missing_witnesses,
        vec![MissingWitness::EnumOtherVariant]
    );

    // 8. Tuple product missing specific combination.
    let tup_arms = vec![
        MatchArm::unguarded(0, Pattern::Tuple(vec![Pattern::Bool(true), Pattern::Wild])).unwrap(),
        MatchArm::unguarded(
            1,
            Pattern::Tuple(vec![Pattern::Bool(false), Pattern::Bool(false)]),
        )
        .unwrap(),
    ];
    let tup_out = compile(
        &PatternMatrix::new(
            TypeDesc::Tuple(vec![TypeDesc::Bool, TypeDesc::Bool]),
            tup_arms.clone(),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(!tup_out.is_exhaustive);
    assert_eq!(
        tup_out.missing_witnesses,
        vec![MissingWitness::Tuple(vec![
            MissingWitness::Bool(false),
            MissingWitness::Bool(true),
        ])]
    );
    verify_witnesses_unmatched(&tup_out, &tup_arms);
}

#[test]
fn list_patterns_rest_slices_and_differential_execution() {
    let list_ty = TypeDesc::List(Box::new(TypeDesc::Int(IntType::UnsignedBits(8))));

    // Missing len == 1 (`[_]`) when only `[]` and `[a, b, ...rest]` exist.
    let gap_arms = vec![
        MatchArm::unguarded(
            0,
            Pattern::List {
                prefix: vec![],
                rest: None,
            },
        )
        .unwrap(),
        MatchArm::unguarded(
            1,
            Pattern::List {
                prefix: vec![Pattern::var("a"), Pattern::var("b")],
                rest: Some(Box::new(Pattern::var("rest"))),
            },
        )
        .unwrap(),
    ];
    let gap_out = compile(&PatternMatrix::new(list_ty.clone(), gap_arms.clone()).unwrap()).unwrap();
    assert!(!gap_out.is_exhaustive);
    assert_eq!(
        gap_out.missing_witnesses,
        vec![MissingWitness::List(vec![MissingWitness::Int(
            IntLit::Unsigned(0)
        )])]
    );
    verify_witnesses_unmatched(&gap_out, &gap_arms);

    // Exhaustive list match with overlapping prefix & rest slices + nested list rest.
    let arms = vec![
        MatchArm::unguarded(
            0,
            Pattern::List {
                prefix: vec![],
                rest: None,
            },
        )
        .unwrap(),
        MatchArm::unguarded(
            1,
            Pattern::List {
                prefix: vec![Pattern::unsigned(7), Pattern::var("only_tail")],
                rest: None,
            },
        )
        .unwrap(),
        MatchArm::unguarded(
            2,
            Pattern::List {
                prefix: vec![Pattern::var("head")],
                rest: Some(Box::new(Pattern::List {
                    prefix: vec![Pattern::unsigned(9)],
                    rest: Some(Box::new(Pattern::var("tail"))),
                })),
            },
        )
        .unwrap(),
        MatchArm::unguarded(
            3,
            Pattern::List {
                prefix: vec![Pattern::var("first")],
                rest: Some(Box::new(Pattern::var("rest"))),
            },
        )
        .unwrap(),
    ];
    let out = compile(&PatternMatrix::new(list_ty, arms.clone()).unwrap()).unwrap();
    assert!(out.is_exhaustive);
    assert!(out.unreachable_arms.is_empty());

    let values = vec![
        Value::List(vec![]),
        Value::List(vec![Value::unsigned(7)]),
        Value::List(vec![Value::unsigned(7), Value::unsigned(9)]),
        Value::List(vec![Value::unsigned(7), Value::unsigned(4)]),
        Value::List(vec![Value::unsigned(1), Value::unsigned(9)]),
        Value::List(vec![
            Value::unsigned(1),
            Value::unsigned(9),
            Value::unsigned(2),
            Value::unsigned(3),
        ]),
        Value::List(vec![
            Value::unsigned(1),
            Value::unsigned(2),
            Value::unsigned(3),
        ]),
    ];
    assert_differential_with_trace(&out, &arms, &values, |_, _| true);
}

#[test]
fn bitvec_segment_alignment_exhaustiveness_witnesses_and_differential_eval() {
    // 1. 0b(kind: u3)(flags: u5)(length: u8) covers all 16-bit values without wildcard.
    let bv16_arms = vec![
        MatchArm::unguarded(
            0,
            Pattern::BitVec(BitVecPattern::with_width(
                16,
                vec![
                    BitSegmentPattern::typed_u(3, Pattern::var("kind")),
                    BitSegmentPattern::typed_u(5, Pattern::var("flags")),
                    BitSegmentPattern::typed_u(8, Pattern::var("length")),
                ],
            )),
        )
        .unwrap(),
    ];
    let bv16_out =
        compile(&PatternMatrix::new(TypeDesc::BitVec(16), bv16_arms.clone()).unwrap()).unwrap();
    assert!(bv16_out.is_exhaustive);
    assert!(bv16_out.unreachable_arms.is_empty());
    let sample16: Vec<Value> = [0u16, 1, 0xABCD, 0xFFFF]
        .into_iter()
        .map(|v| Value::BitVec(BitValue::from_u64(16, v as u64).unwrap()))
        .collect();
    assert_differential_with_trace(&bv16_out, &bv16_arms, &sample16, |_, _| true);

    // 2. 0b10(rest: u6) leaves 0b00_000000 witness.
    let prefix_arms = vec![
        MatchArm::unguarded(
            0,
            Pattern::BitVec(BitVecPattern::with_width(
                8,
                vec![
                    BitSegmentPattern::from_radix_str(2, "10").unwrap(),
                    BitSegmentPattern::typed_u(6, Pattern::var("rest")),
                ],
            )),
        )
        .unwrap(),
    ];
    let prefix_out =
        compile(&PatternMatrix::new(TypeDesc::BitVec(8), prefix_arms.clone()).unwrap()).unwrap();
    assert!(!prefix_out.is_exhaustive);
    assert_eq!(
        prefix_out.missing_witnesses,
        vec![MissingWitness::BitVec(BitValue::zero(8))]
    );
    verify_witnesses_unmatched(&prefix_out, &prefix_arms);

    // 3. Heterogeneous overlapping segments + range split inside typed slice across ALL 256 bytes.
    let hetero_arms = vec![
        // Arm 0: 0b 10 (mid: u4) 01
        MatchArm::unguarded(
            0,
            Pattern::BitVec(BitVecPattern::new(vec![
                BitSegmentPattern::from_radix_str(2, "10").unwrap(),
                BitSegmentPattern::typed_u(4, Pattern::var("mid")),
                BitSegmentPattern::from_radix_str(2, "01").unwrap(),
            ])),
        )
        .unwrap(),
        // Arm 1: 0b (hi: u3) 111 (lo: u2)
        MatchArm::unguarded(
            1,
            Pattern::BitVec(BitVecPattern::new(vec![
                BitSegmentPattern::typed_u(3, Pattern::var("hi")),
                BitSegmentPattern::from_radix_str(2, "111").unwrap(),
                BitSegmentPattern::typed_u(2, Pattern::var("lo")),
            ])),
        )
        .unwrap(),
        // Arm 2: 0b (x @ 0..=127 : u8) (covers all 0xxxxxxx)
        MatchArm::unguarded(
            2,
            Pattern::BitVec(BitVecPattern::new(vec![BitSegmentPattern::typed_u(
                8,
                Pattern::bind_as(
                    "x",
                    Pattern::IntRange {
                        start: RangeBound::Inclusive(IntLit::Unsigned(0)),
                        end: RangeBound::Inclusive(IntLit::Unsigned(127)),
                    },
                ),
            )])),
        )
        .unwrap(),
        // Arm 3: 0b 11 (tail: BitVec<6>)
        MatchArm::unguarded(
            3,
            Pattern::BitVec(BitVecPattern::new(vec![
                BitSegmentPattern::from_radix_str(2, "11").unwrap(),
                BitSegmentPattern::typed_bitvec(6, Pattern::var("tail")),
            ])),
        )
        .unwrap(),
    ];
    let hetero_out =
        compile(&PatternMatrix::new(TypeDesc::BitVec(8), hetero_arms.clone()).unwrap()).unwrap();
    assert!(!hetero_out.is_exhaustive);
    assert!(hetero_out.unreachable_arms.is_empty());
    verify_witnesses_unmatched(&hetero_out, &hetero_arms);

    // Exhaustive differential verification over all 256 possible BitVec<8> values!
    let all_u8_vals: Vec<Value> = (0u64..=255)
        .map(|v| Value::BitVec(BitValue::from_u64(8, v).unwrap()))
        .collect();
    assert_differential_with_trace(&hetero_out, &hetero_arms, &all_u8_vals, |_, _| true);

    // 4. Exhaustive BitVec<8> using 0b11(_: u6) + (0..=191 : u8).
    let split_exhaustive = vec![
        MatchArm::unguarded(
            0,
            Pattern::BitVec(BitVecPattern::new(vec![
                BitSegmentPattern::from_radix_str(2, "11").unwrap(),
                BitSegmentPattern::typed_u(6, Pattern::Wild),
            ])),
        )
        .unwrap(),
        MatchArm::unguarded(
            1,
            Pattern::BitVec(BitVecPattern::new(vec![BitSegmentPattern::typed_u(
                8,
                Pattern::IntRange {
                    start: RangeBound::Inclusive(IntLit::Unsigned(0)),
                    end: RangeBound::Inclusive(IntLit::Unsigned(191)),
                },
            )])),
        )
        .unwrap(),
    ];
    let split_out =
        compile(&PatternMatrix::new(TypeDesc::BitVec(8), split_exhaustive.clone()).unwrap())
            .unwrap();
    assert!(split_out.is_exhaustive);
    assert_differential_with_trace(&split_out, &split_exhaustive, &all_u8_vals, |_, _| true);

    // 5. IEEE-754 f32 BitVec<32> match from docs/control-flow/pattern-matching/bitvec.md.
    let f32_arms = vec![
        MatchArm::guarded(
            0,
            Pattern::BitVec(BitVecPattern::with_width(
                32,
                vec![
                    BitSegmentPattern::typed_u(1, Pattern::Wild),
                    BitSegmentPattern::from_radix_str(2, "11111111").unwrap(),
                    BitSegmentPattern::typed_bitvec(23, Pattern::var("mantissa")),
                ],
            )),
        )
        .unwrap(),
        MatchArm::unguarded(
            1,
            Pattern::BitVec(BitVecPattern::with_width(
                32,
                vec![
                    BitSegmentPattern::typed_u(1, Pattern::Wild),
                    BitSegmentPattern::from_radix_str(2, "11111111").unwrap(),
                    BitSegmentPattern::from_radix_str(2, "00000000000000000000000").unwrap(),
                ],
            )),
        )
        .unwrap(),
        MatchArm::unguarded(2, Pattern::Wild).unwrap(),
    ];
    let f32_out =
        compile(&PatternMatrix::new(TypeDesc::BitVec(32), f32_arms.clone()).unwrap()).unwrap();
    assert!(f32_out.is_exhaustive);
    assert!(f32_out.unreachable_arms.is_empty());
    let f32_vals: Vec<Value> = [
        13.625_f32.to_bits(),
        f32::INFINITY.to_bits(),
        f32::NEG_INFINITY.to_bits(),
        f32::NAN.to_bits(),
        0.0_f32.to_bits(),
    ]
    .into_iter()
    .map(|bits| Value::BitVec(BitValue::from_u64(32, bits as u64).unwrap()))
    .collect();
    assert_differential_with_trace(
        &f32_out,
        &f32_arms,
        &f32_vals,
        |_arm, binds| matches!(binds.get("mantissa"), Some(Value::BitVec(bv)) if !bv.is_zero()),
    );

    // 6. Zero-width BitVec<0> & wide cross-machine-word BitVec<256>.
    let zero_arms = vec![
        MatchArm::unguarded(0, Pattern::BitVec(BitVecPattern::with_width(0, vec![]))).unwrap(),
    ];
    let zero_out =
        compile(&PatternMatrix::new(TypeDesc::BitVec(0), zero_arms.clone()).unwrap()).unwrap();
    assert!(zero_out.is_exhaustive);
    assert_differential_with_trace(
        &zero_out,
        &zero_arms,
        &[Value::BitVec(BitValue::empty())],
        |_, _| true,
    );

    let wide_arms = vec![
        MatchArm::unguarded(
            0,
            Pattern::BitVec(BitVecPattern::with_width(
                256,
                vec![
                    BitSegmentPattern::from_radix_str(16, "DEAD_BEEF").unwrap(),
                    BitSegmentPattern::typed_bitvec(224, Pattern::var("payload")),
                ],
            )),
        )
        .unwrap(),
        MatchArm::unguarded(1, Pattern::Wild).unwrap(),
    ];
    let wide_out =
        compile(&PatternMatrix::new(TypeDesc::BitVec(256), wide_arms.clone()).unwrap()).unwrap();
    assert!(wide_out.is_exhaustive);
    let mut wide_match = BitValue::zero(256);
    wide_match.splice_from(224, &BitValue::from_radix_str(16, "DEADBEEF").unwrap());
    let wide_other = BitValue::zero(256);
    assert_differential_with_trace(
        &wide_out,
        &wide_arms,
        &[Value::BitVec(wide_match), Value::BitVec(wide_other)],
        |_, _| true,
    );
}

#[test]
fn bitvec_signed_slice_and_or_inside_segment_differential() {
    // Signed 4-bit slice inside BitVec<6> alongside fixed-bit overlaps.
    let arms = vec![
        MatchArm::unguarded(
            0,
            Pattern::BitVec(BitVecPattern::new(vec![
                BitSegmentPattern::from_radix_str(2, "10").unwrap(),
                BitSegmentPattern::typed_i(
                    4,
                    Pattern::IntRange {
                        start: RangeBound::Inclusive(IntLit::Signed(-3)),
                        end: RangeBound::Inclusive(IntLit::Signed(2)),
                    },
                ),
            ])),
        )
        .unwrap(),
        MatchArm::unguarded(
            1,
            Pattern::BitVec(BitVecPattern::new(vec![
                BitSegmentPattern::typed_bool(Pattern::Bool(false)),
                BitSegmentPattern::typed_u(
                    5,
                    Pattern::Or(vec![Pattern::unsigned(3), Pattern::unsigned(19)]),
                ),
            ])),
        )
        .unwrap(),
        MatchArm::unguarded(2, Pattern::Wild).unwrap(),
    ];
    let out = compile(&PatternMatrix::new(TypeDesc::BitVec(6), arms.clone()).unwrap()).unwrap();
    assert!(out.is_exhaustive);
    assert!(out.unreachable_arms.is_empty());

    let all_6bit: Vec<Value> = (0u64..64)
        .map(|v| Value::BitVec(BitValue::from_u64(6, v).unwrap()))
        .collect();
    assert_differential_with_trace(&out, &arms, &all_6bit, |_, _| true);
}

#[test]
fn reachability_detection_with_guards_and_shadowing() {
    let pair_bool = TypeDesc::Tuple(vec![TypeDesc::Bool, TypeDesc::Bool]);

    // Case A: Unguarded (true, _) + (false, _) makes (_, true) unreachable.
    let shadowed_arms = vec![
        MatchArm::unguarded(0, Pattern::Tuple(vec![Pattern::Bool(true), Pattern::Wild])).unwrap(),
        MatchArm::unguarded(1, Pattern::Tuple(vec![Pattern::Bool(false), Pattern::Wild])).unwrap(),
        MatchArm::unguarded(2, Pattern::Tuple(vec![Pattern::Wild, Pattern::Bool(true)])).unwrap(),
    ];
    let shadowed_out =
        compile(&PatternMatrix::new(pair_bool.clone(), shadowed_arms).unwrap()).unwrap();
    assert!(shadowed_out.is_exhaustive);
    assert_eq!(shadowed_out.unreachable_arms, vec![2]);

    // Case B: Guarded (false, _) does NOT shadow (_, true) or (false, false).
    let guarded_arms = vec![
        MatchArm::unguarded(0, Pattern::Tuple(vec![Pattern::Bool(true), Pattern::Wild])).unwrap(),
        MatchArm::guarded(1, Pattern::Tuple(vec![Pattern::Bool(false), Pattern::Wild])).unwrap(),
        MatchArm::unguarded(2, Pattern::Tuple(vec![Pattern::Wild, Pattern::Bool(true)])).unwrap(),
    ];
    let guarded_out = compile(&PatternMatrix::new(pair_bool, guarded_arms).unwrap()).unwrap();
    assert!(!guarded_out.is_exhaustive);
    assert!(guarded_out.unreachable_arms.is_empty());
    assert_eq!(
        guarded_out.missing_witnesses,
        vec![MissingWitness::Tuple(vec![
            MissingWitness::Bool(false),
            MissingWitness::Bool(false),
        ])]
    );

    // Case C: Empty integer range is unreachable.
    let empty_range_arms = vec![
        MatchArm::unguarded(
            0,
            Pattern::IntRange {
                start: RangeBound::Inclusive(IntLit::Signed(10)),
                end: RangeBound::Inclusive(IntLit::Signed(5)),
            },
        )
        .unwrap(),
        MatchArm::unguarded(1, Pattern::Wild).unwrap(),
    ];
    let empty_range_out = compile(
        &PatternMatrix::new(TypeDesc::Int(IntType::SignedBits(32)), empty_range_arms).unwrap(),
    )
    .unwrap();
    assert!(empty_range_out.is_exhaustive);
    assert_eq!(empty_range_out.unreachable_arms, vec![0]);
}

#[test]
fn validation_and_budget_exhaustion_errors() {
    // 1. Inconsistent Or bindings.
    let bad_or = Pattern::Or(vec![Pattern::var("a"), Pattern::var("b")]);
    assert!(matches!(
        MatchArm::unguarded(0, bad_or),
        Err(CompileError::InconsistentOrBindings { .. })
    ));

    // 2. Empty Or pattern.
    assert_eq!(
        MatchArm::unguarded(0, Pattern::Or(vec![])),
        Err(CompileError::EmptyOrPattern)
    );

    // 3. Duplicate binding inside product pattern.
    let dup = Pattern::Tuple(vec![Pattern::var("x"), Pattern::var("x")]);
    assert_eq!(
        MatchArm::unguarded(0, dup),
        Err(CompileError::DuplicateBinding {
            name: "x".to_string()
        })
    );

    // 4. BitVec width mismatch.
    let bad_bv = MatchArm::unguarded(
        0,
        Pattern::BitVec(BitVecPattern::new(vec![
            BitSegmentPattern::from_radix_str(2, "101").unwrap(),
        ])),
    )
    .unwrap();
    assert!(matches!(
        PatternMatrix::new(TypeDesc::BitVec(8), vec![bad_bv]),
        Err(CompileError::InvalidBitVecWidth {
            expected: 8,
            found: 3
        })
    ));

    // 5. BudgetExceeded on steps and on nodes.
    let ty = TypeDesc::Tuple(vec![TypeDesc::Bool; 6]);
    let arms = vec![
        MatchArm::unguarded(0, Pattern::Tuple(vec![Pattern::Bool(true); 6])).unwrap(),
        MatchArm::unguarded(1, Pattern::Wild).unwrap(),
    ];
    let matrix = PatternMatrix::new(ty, arms).unwrap();

    let tiny_steps = CompileBudget {
        max_steps: 2,
        max_nodes: 1000,
        max_witnesses: 8,
    };
    assert!(matches!(
        compile_with_budget(&matrix, tiny_steps),
        Err(CompileError::BudgetExceeded { .. })
    ));

    let tiny_nodes = CompileBudget {
        max_steps: 10_000,
        max_nodes: 2,
        max_witnesses: 8,
    };
    assert!(matches!(
        compile_with_budget(&matrix, tiny_nodes),
        Err(CompileError::BudgetExceeded { .. })
    ));
}

#[test]
fn deterministic_lisp_sexpr_display_and_escaping() {
    let weird_name = "a\"b\\c\n\r\t\0";
    let pat = Pattern::Record(BTreeMap::from([
        (weird_name.to_string(), Pattern::Char('\n')),
        ("k".to_string(), Pattern::var("v\"1\n2")),
    ]));
    let arm = MatchArm::guarded(0, pat).unwrap();
    let ty = TypeDesc::Record(BTreeMap::from([
        (weird_name.to_string(), TypeDesc::Char),
        ("k".to_string(), TypeDesc::Bool),
    ]));
    let matrix = PatternMatrix::new(ty, vec![arm.clone()]).unwrap();
    let out = compile(&matrix).unwrap();
    let out2 = compile(&matrix).unwrap();
    assert_eq!(out.to_string(), out2.to_string());

    assert_balanced_sexpr_no_newlines(&arm.to_string());
    assert_balanced_sexpr_no_newlines(&matrix.to_string());
    assert_balanced_sexpr_no_newlines(&out.to_string());
    assert_balanced_sexpr_no_newlines(&out.dag.to_string());

    let val = Value::Record(BTreeMap::from([
        (weird_name.to_string(), Value::Char('\n')),
        ("k".to_string(), Value::Bool(true)),
    ]));
    let outcome = eval_dag(&out.dag, &val, |_, _| true).unwrap();
    assert_balanced_sexpr_no_newlines(&val.to_string());
    assert_balanced_sexpr_no_newlines(&outcome.to_string());
    assert_eq!(
        outcome,
        MatchOutcome {
            arm_index: 0,
            bindings: BTreeMap::from([("v\"1\n2".to_string(), Value::Bool(true))]),
        }
    );

    let err = CompileError::BudgetExceeded {
        steps_used: 5,
        nodes_created: 3,
        budget: CompileBudget::default(),
    };
    assert_balanced_sexpr_no_newlines(&err.to_string());
}

#[test]
fn bit_interval_splitting_and_128bit_extremes() {
    // Verify split_bit_interval covers exact original range for arbitrary splits.
    let iv = BitInterval::new(
        BitValue::from_u64(8, 5).unwrap(),
        BitValue::from_u64(8, 200).unwrap(),
    );
    let splits = pattern_automata::bitvec::split_bit_interval(&iv, 6);
    assert!(splits.len() <= 3);
    let mut covered = vec![false; 256];
    for (hi_iv, lo_iv) in &splits {
        let h_lo = hi_iv.lo.to_u128().unwrap() as usize;
        let h_hi = hi_iv.hi.to_u128().unwrap() as usize;
        let l_lo = lo_iv.lo.to_u128().unwrap() as usize;
        let l_hi = lo_iv.hi.to_u128().unwrap() as usize;
        for h in h_lo..=h_hi {
            for l in l_lo..=l_hi {
                let full = (h << 6) | l;
                assert!(!covered[full], "duplicate value {full} in split");
                covered[full] = true;
            }
        }
    }
    for (v, &seen) in covered.iter().enumerate() {
        assert_eq!(seen, (5..=200).contains(&v), "mismatch at {v}");
    }

    // Verify i128::MIN..=i128::MAX and 0..=u128::MAX exhaustiveness & witness precision.
    let i128_arms = vec![
        MatchArm::unguarded(
            0,
            Pattern::IntRange {
                start: RangeBound::Inclusive(IntLit::Signed(i128::MIN)),
                end: RangeBound::Inclusive(IntLit::Signed(i128::MAX)),
            },
        )
        .unwrap(),
    ];
    assert!(
        compile(&PatternMatrix::new(TypeDesc::Int(IntType::SignedBits(128)), i128_arms).unwrap())
            .unwrap()
            .is_exhaustive
    );

    let u128_gap_arms = vec![
        MatchArm::unguarded(
            0,
            Pattern::IntRange {
                start: RangeBound::Inclusive(IntLit::Unsigned(0)),
                end: RangeBound::Exclusive(IntLit::Unsigned(u128::MAX)),
            },
        )
        .unwrap(),
    ];
    let u128_out = compile(
        &PatternMatrix::new(
            TypeDesc::Int(IntType::UnsignedBits(128)),
            u128_gap_arms.clone(),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(!u128_out.is_exhaustive);
    assert_eq!(
        u128_out.missing_witnesses,
        vec![MissingWitness::Int(IntLit::Unsigned(u128::MAX))]
    );
    verify_witnesses_unmatched(&u128_out, &u128_gap_arms);
}

#[test]
fn char_surrogates_error_any_discrete_tags_and_uninhabited_enum() {
    // 1. Char range spanning surrogate gap 0xD800..=0xDFFF.
    let char_arms = vec![
        MatchArm::unguarded(
            0,
            Pattern::CharRange {
                start: RangeBound::Inclusive('a'),
                end: RangeBound::Inclusive('\u{E000}'),
            },
        )
        .unwrap(),
        MatchArm::unguarded(1, Pattern::Char('A')).unwrap(),
    ];
    let char_out =
        compile(&PatternMatrix::new(TypeDesc::Char, char_arms.clone()).unwrap()).unwrap();
    assert!(!char_out.is_exhaustive);
    verify_witnesses_unmatched(&char_out, &char_arms);
    let char_vals: Vec<Value> = [
        '\0',
        '@',
        'A',
        'B',
        'a',
        'z',
        '\u{D7FF}',
        '\u{E000}',
        '\u{E001}',
        '\u{10FFFF}',
    ]
    .into_iter()
    .map(Value::Char)
    .collect();
    assert_differential_with_trace(&char_out, &char_arms, &char_vals, |_, _| true);

    // 2. Untagged `error e` matching multiple error tags and binding inner payload.
    let multi_err_ty = TypeDesc::ErrorRow(ErrorRowType {
        ok_type: Box::new(TypeDesc::Bool),
        error_tags: BTreeMap::from([
            (
                ErrorTag::new("DiskErr"),
                TypeDesc::Int(IntType::UnsignedBits(8)),
            ),
            (
                ErrorTag::new("NetErr"),
                TypeDesc::Int(IntType::UnsignedBits(8)),
            ),
        ]),
        is_open: false,
    });
    let err_arms = vec![
        MatchArm::unguarded(0, Pattern::ok(Pattern::var("ok_v"))).unwrap(),
        MatchArm::unguarded(1, Pattern::err_tag("DiskErr", Pattern::unsigned(42))).unwrap(),
        MatchArm::guarded(2, Pattern::err_any(Pattern::var("code"))).unwrap(),
        MatchArm::unguarded(3, Pattern::err_any(Pattern::Wild)).unwrap(),
    ];
    let err_out = compile(&PatternMatrix::new(multi_err_ty, err_arms.clone()).unwrap()).unwrap();
    assert!(err_out.is_exhaustive);
    assert!(err_out.unreachable_arms.is_empty());
    let err_vals = vec![
        Value::ok(Value::Bool(true)),
        Value::err("DiskErr", Value::unsigned(42)),
        Value::err("DiskErr", Value::unsigned(7)),
        Value::err("NetErr", Value::unsigned(200)),
        Value::err("NetErr", Value::unsigned(3)),
    ];
    assert_differential_with_trace(
        &err_out,
        &err_arms,
        &err_vals,
        |_arm, binds| matches!(binds.get("code"), Some(Value::Int(IntLit::Unsigned(c))) if *c >= 100),
    );

    // 3. Negative discrete enum tags & zero-variant uninhabited closed enum.
    let disc_ty = TypeDesc::Enum(EnumType::closed(
        "TriCode",
        BTreeMap::from([
            (
                VariantTag::discrete(-1),
                VariantSchema {
                    discrete_code: Some(-1),
                    payload: VariantPayloadType::Unit,
                },
            ),
            (
                VariantTag::discrete(0),
                VariantSchema {
                    discrete_code: Some(0),
                    payload: VariantPayloadType::Unit,
                },
            ),
            (
                VariantTag::discrete(10),
                VariantSchema {
                    discrete_code: Some(10),
                    payload: VariantPayloadType::Unit,
                },
            ),
        ]),
    ));
    let disc_arms = vec![
        MatchArm::unguarded(0, Pattern::unit_variant(VariantTag::discrete(0))).unwrap(),
        MatchArm::unguarded(1, Pattern::unit_variant(VariantTag::discrete(10))).unwrap(),
    ];
    let disc_out = compile(&PatternMatrix::new(disc_ty, disc_arms.clone()).unwrap()).unwrap();
    assert!(!disc_out.is_exhaustive);
    assert_eq!(
        disc_out.missing_witnesses,
        vec![MissingWitness::EnumVariant {
            tag: VariantTag::discrete(-1),
            payload: pattern_automata::WitnessPayload::Unit,
        }]
    );
    verify_witnesses_unmatched(&disc_out, &disc_arms);

    let empty_enum = TypeDesc::Enum(EnumType::closed("Empty", BTreeMap::new()));
    let empty_out = compile(&PatternMatrix::new(empty_enum.clone(), vec![]).unwrap()).unwrap();
    assert!(empty_out.is_exhaustive);
    assert!(empty_out.unreachable_arms.is_empty());

    let product_empty = TypeDesc::Tuple(vec![TypeDesc::Bool, empty_enum]);
    let prod_empty_out = compile(&PatternMatrix::new(product_empty, vec![]).unwrap()).unwrap();
    assert!(prod_empty_out.is_exhaustive);
}

#[test]
fn bitvec_nested_typed_segments_and_128bit_slices() {
    // Nested BitVec<4> inside BitVec<8>, plus 128-bit slice matching.
    let nested_arms = vec![
        MatchArm::unguarded(
            0,
            Pattern::BitVec(BitVecPattern::with_width(
                8,
                vec![
                    BitSegmentPattern::typed_bitvec(
                        4,
                        Pattern::BitVec(BitVecPattern::with_width(
                            4,
                            vec![
                                BitSegmentPattern::from_radix_str(2, "10").unwrap(),
                                BitSegmentPattern::typed_u(2, Pattern::var("a")),
                            ],
                        )),
                    ),
                    BitSegmentPattern::typed_u(
                        4,
                        Pattern::bind_as(
                            "b",
                            Pattern::IntRange {
                                start: RangeBound::Inclusive(IntLit::Unsigned(1)),
                                end: RangeBound::Inclusive(IntLit::Unsigned(3)),
                            },
                        ),
                    ),
                ],
            )),
        )
        .unwrap(),
        MatchArm::unguarded(
            1,
            Pattern::BitVec(BitVecPattern::with_width(
                8,
                vec![
                    BitSegmentPattern::from_radix_str(2, "10").unwrap(),
                    BitSegmentPattern::typed_u(6, Pattern::var("rest")),
                ],
            )),
        )
        .unwrap(),
    ];
    let nested_out =
        compile(&PatternMatrix::new(TypeDesc::BitVec(8), nested_arms.clone()).unwrap()).unwrap();
    assert!(!nested_out.is_exhaustive);
    verify_witnesses_unmatched(&nested_out, &nested_arms);
    let all_8: Vec<Value> = (0u64..=255)
        .map(|v| Value::BitVec(BitValue::from_u64(8, v).unwrap()))
        .collect();
    assert_differential_with_trace(&nested_out, &nested_arms, &all_8, |_, _| true);

    // 128-bit BitVec with u64 + i64 segments.
    let bv128_arms = vec![
        MatchArm::unguarded(
            0,
            Pattern::BitVec(BitVecPattern::with_width(
                128,
                vec![
                    BitSegmentPattern::from_radix_str(16, "FFFF_FFFF_FFFF_FFFF").unwrap(),
                    BitSegmentPattern::typed_i(64, Pattern::signed(-1)),
                ],
            )),
        )
        .unwrap(),
        MatchArm::unguarded(1, Pattern::Wild).unwrap(),
    ];
    let bv128_out =
        compile(&PatternMatrix::new(TypeDesc::BitVec(128), bv128_arms.clone()).unwrap()).unwrap();
    assert!(bv128_out.is_exhaustive);
    let vals128 = vec![
        Value::BitVec(BitValue::all_ones(128)),
        Value::BitVec(BitValue::zero(128)),
        Value::BitVec(BitValue::from_u128(128, u128::MAX - 1).unwrap()),
    ];
    assert_differential_with_trace(&bv128_out, &bv128_arms, &vals128, |_, _| true);
}

struct XorShift64(u64);

impl XorShift64 {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn gen_range(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}

#[test]
fn differential_fuzz_random_matrices_and_bitvec_splits() {
    let mut rng = XorShift64(0x9E37_79B9_7F4A_7C15);

    // 1. Fuzz 40 random 6-bit BitVec matrices with heterogeneous segment widths, ranges, and guards.
    let all_6bit: Vec<Value> = (0u64..64)
        .map(|v| Value::BitVec(BitValue::from_u64(6, v).unwrap()))
        .collect();
    for seed_idx in 0..40 {
        let arm_count = 3 + (rng.gen_range(5) as usize);
        let mut arms = Vec::with_capacity(arm_count);
        for arm_i in 0..arm_count {
            let mut rem = 6u32;
            let mut segs = Vec::new();
            let mut bind_idx = 0usize;
            while rem > 0 {
                let w = 1 + (rng.gen_range(rem as u64) as u32);
                rem -= w;
                match rng.gen_range(4) {
                    0 => {
                        let v = rng.gen_range(1u64 << w);
                        segs.push(BitSegmentPattern::fixed(BitValue::from_u64(w, v).unwrap()));
                    }
                    1 => {
                        let name = format!("b{bind_idx}");
                        bind_idx += 1;
                        segs.push(BitSegmentPattern::typed_u(w as u8, Pattern::var(name)));
                    }
                    2 => {
                        let a = rng.gen_range(1u64 << w) as u128;
                        let b = rng.gen_range(1u64 << w) as u128;
                        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                        segs.push(BitSegmentPattern::typed_u(
                            w as u8,
                            Pattern::IntRange {
                                start: RangeBound::Inclusive(IntLit::Unsigned(lo)),
                                end: RangeBound::Inclusive(IntLit::Unsigned(hi)),
                            },
                        ));
                    }
                    _ => {
                        let v1 = rng.gen_range(1u64 << w) as u128;
                        let v2 = rng.gen_range(1u64 << w) as u128;
                        segs.push(BitSegmentPattern::typed_u(
                            w as u8,
                            Pattern::Or(vec![Pattern::unsigned(v1), Pattern::unsigned(v2)]),
                        ));
                    }
                }
            }
            let has_guard = rng.gen_range(4) == 0;
            arms.push(
                MatchArm::new(
                    arm_i,
                    Pattern::BitVec(BitVecPattern::with_width(6, segs)),
                    has_guard,
                )
                .unwrap(),
            );
        }
        let matrix = PatternMatrix::new(TypeDesc::BitVec(6), arms.clone()).unwrap();
        let out = compile(&matrix).unwrap();
        assert_balanced_sexpr_no_newlines(&out.to_string());
        assert_differential_with_trace(&out, &arms, &all_6bit, |arm_idx, binds| {
            ((arm_idx + binds.len() + seed_idx) % 2) == 0
        });
        if out.is_exhaustive {
            for val in &all_6bit {
                assert!(
                    eval_dag(&out.dag, val, |arm_idx, binds| {
                        ((arm_idx + binds.len() + seed_idx) % 2) == 0
                    })
                    .is_some()
                        || arms.iter().any(|a| a.has_guard),
                    "exhaustive match missed {val}"
                );
            }
        } else {
            verify_witnesses_unmatched(&out, &arms);
        }
    }

    // 2. Fuzz 40 random nested (bool, ?u4-like u8, List<bool>) matrices with Or and guards.
    let nested_ty = TypeDesc::Tuple(vec![
        TypeDesc::Bool,
        TypeDesc::Optional(Box::new(TypeDesc::Int(IntType::UnsignedBits(8)))),
        TypeDesc::List(Box::new(TypeDesc::Bool)),
    ]);
    let mut sample_vals = Vec::new();
    for b in [false, true] {
        for opt in [
            Value::OptionalNull,
            Value::present(Value::unsigned(0)),
            Value::present(Value::unsigned(5)),
            Value::present(Value::unsigned(12)),
            Value::present(Value::unsigned(255)),
        ] {
            for list in [
                Value::List(vec![]),
                Value::List(vec![Value::Bool(true)]),
                Value::List(vec![Value::Bool(false), Value::Bool(true)]),
                Value::List(vec![
                    Value::Bool(true),
                    Value::Bool(false),
                    Value::Bool(true),
                ]),
            ] {
                sample_vals.push(Value::Tuple(vec![b.into_val(), opt.clone(), list]));
            }
        }
    }

    for seed_idx in 0..40 {
        let arm_count = 4 + (rng.gen_range(5) as usize);
        let mut arms = Vec::with_capacity(arm_count);
        for arm_i in 0..arm_count {
            let p_bool = match rng.gen_range(3) {
                0 => Pattern::Bool(false),
                1 => Pattern::Bool(true),
                _ => Pattern::Wild,
            };
            let p_opt = match rng.gen_range(4) {
                0 => Pattern::OptionalNull,
                1 => Pattern::present(Pattern::IntRange {
                    start: RangeBound::Inclusive(IntLit::Unsigned(0)),
                    end: RangeBound::Inclusive(IntLit::Unsigned(10)),
                }),
                2 => Pattern::present(Pattern::Or(vec![
                    Pattern::unsigned(5),
                    Pattern::unsigned(12),
                ])),
                _ => Pattern::Wild,
            };
            let p_list = match rng.gen_range(4) {
                0 => Pattern::List {
                    prefix: vec![],
                    rest: None,
                },
                1 => Pattern::List {
                    prefix: vec![Pattern::Bool(true)],
                    rest: Some(Box::new(Pattern::Wild)),
                },
                2 => Pattern::List {
                    prefix: vec![Pattern::Wild, Pattern::Bool(true)],
                    rest: None,
                },
                _ => Pattern::Wild,
            };
            let pat = if rng.gen_range(5) == 0 {
                Pattern::Or(vec![
                    Pattern::Tuple(vec![p_bool.clone(), p_opt.clone(), Pattern::Wild]),
                    Pattern::Tuple(vec![Pattern::Wild, p_opt, p_list]),
                ])
            } else {
                Pattern::Tuple(vec![p_bool, p_opt, p_list])
            };
            let has_guard = rng.gen_range(4) == 0;
            arms.push(MatchArm::new(arm_i, pat, has_guard).unwrap());
        }
        let matrix = PatternMatrix::new(nested_ty.clone(), arms.clone()).unwrap();
        let out = compile(&matrix).unwrap();
        assert_differential_with_trace(&out, &arms, &sample_vals, |arm_idx, _| {
            ((arm_idx + seed_idx) % 3) != 0
        });
        verify_witnesses_unmatched(&out, &arms);
    }
}

trait BoolExt {
    fn into_val(self) -> Value;
}

impl BoolExt for bool {
    fn into_val(self) -> Value {
        Value::Bool(self)
    }
}
