//! Pending input domains retain ordinary block and branch checking rules.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_interpretor::{Budget, Event, Value};

fn check_both(source: &str, calls: &[&str]) {
    check_both_in_budget(source, calls, sysroot::COMPTIME_BUDGET);
}

fn check_both_in_budget(source: &str, calls: &[&str], budget: Budget) {
    check_in_engines(source, calls, budget, &[false, true]);
}

fn check_in_engines(source: &str, calls: &[&str], budget: Budget, engines: &[bool]) {
    let package = sysroot::compile(source).unwrap_or_else(|failure| panic!("{failure}\n{source}"));
    let entry = package
        .functions
        .iter()
        .find(|f| f.name == "check")
        .unwrap()
        .declaration;
    for &tree in engines {
        sysroot::with_comptime(&package, budget, tree, |vm| {
            assert_eq!(
                vm.invoke(entry, vec![])
                    .unwrap_or_else(|failure| panic!("tree={tree}: {failure:?}")),
                Value::Bool(true),
                "tree={tree}"
            );
            let observed = vm
                .events()
                .iter()
                .filter_map(|event| match event {
                    Event::EnterFunction(id) if id.package == entry.package => package
                        .functions
                        .iter()
                        .find(|f| f.declaration == *id)
                        .map(|f| f.name.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(observed, calls, "tree={tree}");
            if budget.steps != sysroot::COMPTIME_BUDGET.steps {
                eprintln!(
                    "tree={tree}, invocation_work={}",
                    budget.steps - vm.remaining_steps()
                );
            }
        });
    }
}

#[test]
fn logical_regions_scope_type_premises_through_nested_children() {
    check_both(
        "
        comptime fn accept(value: bool) -> bool { value }
        comptime fn both(value: T) -> bool where T {
            T == bool and (do { if true { accept(value) } else { false } })
        }
        comptime fn either(value: T) -> bool where T {
            T != bool or (false or (true match { true => accept(value), false => false }))
        }
        comptime fn check() -> bool {
            both<bool>(true) and not both<u8>(7)
                and either<bool>(true) and either<u8>(7)
        }
        ",
        &[
            "check", "both", "accept", "both", "either", "accept", "either",
        ],
    );
    for condition in ["T == bool and value", "T != bool or value"] {
        let source = format!(
            "comptime fn invalid(value: T) -> bool where T {{
                let local = {condition};
                let escaped: bool = value;
                local and escaped
            }}"
        );
        let failure = sysroot::compile(&source).unwrap_err();
        assert!(failure.contains("type mismatch"), "{failure}");
    }
}

#[test]
fn logical_regions_preserve_short_circuit_and_early_return_order() {
    check_both(
        "
        comptime fn skipped() -> bool { false }
        comptime fn early() -> bool {
            let result = true and (false or (do { return true; false }));
            skipped()
        }
        comptime fn check() -> bool {
            var trace: Integer = 0;
            let left = (do { trace = trace * 10 + 1; false })
                and (do { trace = 99; true });
            let right = (do { trace = trace * 10 + 2; true })
                or (do { trace = 99; false });
            let nested = (left or (do { trace = trace * 10 + 3; true }))
                and (right and (do { trace = trace * 10 + 4; true }));
            nested and trace == 1234 and early()
        }
        ",
        &["check", "early"],
    );
    // Short-circuit execution does not skip source type checking.
    for expression in ["false and 7", "true or 7"] {
        assert!(sysroot::compile(&format!("fn invalid() -> bool {{ {expression} }}")).is_err());
    }
}

#[test]
fn logical_regions_recover_peer_context_without_replaying_runtime_inputs() {
    check_both(
        "
        comptime fn bits(value: BitVec<8>) -> BitVec<8> { value }
        comptime fn check() -> bool {
            var trace: Integer = 0;
            let mask = (do { trace = trace * 10 + 1; 15 })
                and bits(do { trace = trace * 10 + 2; 240 });
            let filled = (do { trace = trace * 10 + 3; 255 })
                or bits(do { trace = trace * 10 + 4; 0 });
            mask == 0 and filled == 255 and trace == 1234
        }
        ",
        &["check", "bits", "bits"],
    );
    for expression in ["15 and bits()", "15 or bits()"] {
        let failure = sysroot::compile(&format!(
            "fn bits() -> BitVec<3> {{ 0 }} fn invalid() -> bool {{ let value = {expression}; true }}"
        )).unwrap_err();
        assert!(!failure.contains("parsing failed"), "{failure}");
        assert!(
            failure.contains("range") || failure.contains("fit"),
            "{failure}"
        );
    }
}

#[test]
fn comparison_regions_keep_peer_context_and_producer_order_across_nested_blocks() {
    check_both(
        "
        comptime fn sample(value: u8) -> u8 { value }
        comptime fn check() -> bool {
            var trace: Integer = 0;
            let peer = (do { trace = trace * 10 + 1; 7 })
                == sample(do { trace = trace * 10 + 2; 7 });
            let nested = ((peer == true)
                == (do { trace = trace * 10 + 3; true })) == true;
            nested and trace == 123
        }
        ",
        &["check", "sample"],
    );
}

#[test]
fn comparison_regions_restore_branch_scopes_and_keep_original_return_targets() {
    check_both(
        "
        comptime fn skipped() -> bool { false }
        comptime fn early() -> bool {
            let unused = (do { return true; }) == skipped();
            false
        }
        comptime fn check() -> bool {
            var trace: Integer = 0;
            let known: u8 = 7;
            let candidate = (if true {
                do { trace = trace + 1; 6 }
            } else { 8 }) < known;
            let nested = do {
                let known: u8 = 8;
                (candidate == true) != (known == 7)
            };
            nested and early() and trace == 1 and known == 7
        }
        ",
        &["check", "early"],
    );
    for invalid in [
        "(do { return true; }) == wrong(true)",
        "(if true { 1 } else { 256 }) == wrong(1)",
    ] {
        let source = format!(
            "comptime fn wrong(value: u8) -> u8 {{ value }}
             comptime fn check() -> bool {{ {invalid} }}"
        );
        assert!(sysroot::compile(&source).is_err(), "accepted {invalid}");
    }
}

#[test]
fn pending_blocks_keep_continuation_type_facts_and_source_order() {
    check_both(
        "
        pure comptime fn Packet<T, .values: meta.List<T> = []> -> Type { bool }
        comptime fn Byte() -> Type { u8 }
        comptime fn mark(flag: bool) -> bool { flag }
        comptime fn Build(value: T) -> Type
        where T
        {
            Packet<.values = do {
                if T != bool { return Packet<u8, .values = []>; }
                let flag: bool = value;
                mark(flag);
                [1, 255]
            }, Byte()>
        }
        comptime fn check() -> bool {
            Build<bool>(true) == Packet<u8, .values = [1, 255]>
                and Build<u8>(9) == Packet<u8, .values = []>
        }
        ",
        &["check", "Build", "mark", "Byte", "Build"],
    );
}

#[test]
fn pending_branches_use_ordinary_type_facts() {
    for branch in [
        "if T == bool { let flag: bool = value; mark(flag); [1] } else { [] }",
        "when { T != bool => [], else => do { let flag: bool = value; mark(flag); [1] } }",
        "T match { 'bool => do { let flag: bool = value; mark(flag); [1] }, _ => [] }",
        "T match { 'bool if mark(value) => [1], _ => [] }",
        "when { T == bool => do { mark(value); [1] },
            do { let witness: Type = refines { Itself != bool } T; false } => [], else => [] }",
    ] {
        check_both(
            &format!(
                "
            pure comptime fn Packet<T, .values: meta.List<T> = []> -> Type {{ bool }}
            comptime fn Byte() -> Type {{ u8 }}
            comptime fn mark(flag: bool) -> bool {{ flag }}
            comptime fn Build(value: T) -> Type
            where T
            {{ Packet<.values = {branch}, Byte()> }}
            comptime fn check() -> bool {{
                Build<bool>(true) == Packet<u8, .values = [1]>
                    and Build<u8>(9) == Packet<u8, .values = []>
            }}"
            ),
            &["check", "Build", "mark", "Byte", "Build", "Byte"],
        );
    }
}

#[test]
fn pending_region_facts_do_not_escape_or_cross_branches() {
    for body in [
        "let unused = Packet<.values = do {
            if T != bool { return Packet<u8>; }
            let flag: bool = value; [1]
        }, Byte()>; let escaped: bool = value; Packet<u8>",
        "Packet<.values = if T == bool { [] } else { let invalid: bool = value; [1] }, Byte()>",
        "Packet<.values = when {
            T == bool => [],
            else => do { let invalid: bool = value; [1] }
        }, Byte()>",
        "Packet<.values = T match {
            'bool => [], _ => do { let invalid: bool = value; [1] }
        }, Byte()>",
    ] {
        let failure = sysroot::compile(&format!(
            "
            pure comptime fn Packet<T, .values: meta.List<T> = []> -> Type {{ bool }}
            comptime fn Byte() -> Type {{ u8 }}
            comptime fn Bad(value: T) -> Type
            where T
            {{ {body} }}
        "
        ))
        .err()
        .expect("invalid branch evidence was accepted");
        assert!(failure.contains("type mismatch"), "{body}: {failure}");
    }
}

#[test]
fn pending_blocks_retain_mixed_pattern_producers_and_guard_reads() {
    for declaration in [
        "pure comptime fn Packet<T, .values: meta.List<T> = []> -> Type { bool }",
        "struct Packet where T, .values: meta.List<T> = [] {}",
    ] {
        check_both(
            &format!(
                "
            {declaration}
            comptime fn Byte() -> Type {{ u8 }}
            comptime fn mark() -> bool {{ true }}
            comptime fn check() -> bool {{
                let selected = Packet<.values = do {{
                    let (Kind, flag) = (u8, mark());
                    let Alias = Kind;
                    true match {{
                        _ if flag and Alias == u8 => [1],
                        _ => [],
                    }}
                }}, Byte()>;
                selected == Packet<u8, .values = [1]>
            }}"
            ),
            &["check", "mark", "Byte"],
        );
    }
}

#[test]
fn nested_lexical_exit_keeps_outer_writes_in_ordinary_and_pending_blocks() {
    for (input, expected_calls) in [
        (
            "do { do { changed = mark(); }; let inner = changed; inner }",
            vec!["check", "mark"],
        ),
        (
            "Packet<.values = do { do { changed = mark(); }; [1] }, Byte()> == Packet<u8, .values = [1]>",
            vec!["check", "mark", "Byte"],
        ),
    ] {
        check_both(
            &format!(
                "
            pure comptime fn Packet<T, .values: meta.List<T> = []> -> Type {{ bool }}
            comptime fn Byte() -> Type {{ u8 }}
            comptime fn mark() -> bool {{ true }}
            comptime fn check() -> bool {{
                var changed = false;
                let result = {input};
                result and changed
            }}
        "
            ),
            &expected_calls,
        );
    }
}

#[test]
fn block_exit_does_not_skip_checks_or_export_a_private_binding() {
    for source in [
        "comptime fn bad() -> bool { return true; let invalid: bool = 7; false }",
        "comptime fn bad() -> bool { let unused = do { let hidden_value = true; hidden_value }; hidden_value }",
    ] {
        assert!(sysroot::compile(source).is_err(), "accepted {source}");
    }
    check_both(
        "
        comptime fn mark() -> bool { true }
        comptime fn leave() -> bool { return true; mark(); false }
        comptime fn check() -> bool { leave() }
    ",
        &["check", "leave"],
    );
}

#[test]
fn ordinary_block_frames_keep_callee_delivery_shadowing_and_receiver_storage() {
    check_both(
        "
        struct Counter { count: Integer }
        impl Counter {
            comptime fn next(self) -> u8 { self.count += 1; 7 }
        }
        comptime fn identity(value: T) -> T where T { value }
        comptime fn check() -> bool {
            var counter = Counter { count: 0 };
            let value = (do { do { identity<u8> } })(do { do { counter.next() } });
            var outer = false;
            let shadow = do { let outer = true; do { outer } };
            do { do { outer = true; } };
            shadow and outer and value == 7 and counter.count == 1
        }
        ",
        &["check", "next", "identity"],
    );
}

#[test]
fn nested_block_exits_preserve_definition_checks_and_skip_unexecuted_calls() {
    check_both(
        "comptime fn check() -> bool { do { do { let hidden_value = true; hidden_value } } }",
        &["check"],
    );
    check_both(
        "
        comptime fn mark() -> bool { false }
        comptime fn leave() -> bool { do { do { return true; mark() } }; false }
        comptime fn check() -> bool { leave() }
        ",
        &["check", "leave"],
    );
    for source in [
        "comptime fn bad() -> bool { do { do { return true; let invalid: bool = 7; false } } }",
        "comptime fn bad() -> bool { do { do { let hidden_value = true; } }; hidden_value }",
    ] {
        assert!(sysroot::compile(source).is_err(), "accepted {source}");
    }
}

#[test]
fn contextual_block_frames_preserve_mixed_children_and_producer_order() {
    let mut body = "[1, 255]".to_owned();
    for _ in 0..24 {
        body = format!("do {{ do {{ trace += 1; }}; {body} }}");
    }
    check_both_in_budget(
        &format!(
            "
            pure comptime fn Packet<T, .values: meta.List<T> = []> -> Type {{ bool }}
            comptime fn Byte() -> Type {{ u8 }}
            comptime fn mark(flag: bool) -> bool {{ flag }}
            comptime fn Build(value: T) -> Type
            where T
            {{
                var trace: Integer = 0;
                let selected = Packet<.values = do {{
                    do {{
                        if T != bool {{ return Packet<u8>; }}
                        let flag: bool = value;
                        mark(flag);
                        {body}
                    }}
                }}, Byte()>;
                if trace != 24 {{ return Packet<u8>; }}
                selected
            }}
            comptime fn check() -> bool {{
                Build<bool>(true) == Packet<u8, .values = [1, 255]>
                    and Build<u8>(9) == Packet<u8>
            }}"
        ),
        &["check", "Build", "mark", "Byte", "Build"],
        // This tests deep source scopes and both execution engines, including
        // their charged cold admission work. Production limits stay unchanged.
        Budget {
            steps: 1_000_000,
            ..Default::default()
        },
    );
}

#[test]
fn contextual_nested_exit_keeps_facts_local_and_checks_unreachable_children() {
    for (body, diagnostic) in [
        (
            "let unused = Packet<.values = do { do {
            if T != bool { return Packet<u8>; }
            let flag: bool = value; [1]
        } }, Byte()>; let escaped: bool = value; Packet<u8>",
            "type mismatch",
        ),
        (
            "Packet<.values = do { do {
            return Packet<u8>; let invalid: bool = 7; [1]
        } }, Byte()>",
            "integer runtime proxy requires a supported fixed-width integer target",
        ),
    ] {
        let failure = sysroot::compile(&format!(
            "pure comptime fn Packet<T, .values: meta.List<T> = []> -> Type {{ bool }}
            comptime fn Byte() -> Type {{ u8 }}
            comptime fn Bad(value: T) -> Type
            where T
            {{ {body} }}"
        ))
        .unwrap_err();
        assert!(failure.contains(diagnostic), "{body}: {failure}");
    }
}

#[test]
fn contextual_if_frames_keep_condition_order_and_only_execute_selected_children() {
    // The reference tree engine separately caps native recursion at 64.
    // Keep the deep source/VM case and compare both engines within that bound.
    for (levels, engines) in [(4, &[false, true][..]), (12, &[false][..])] {
        let conditions = levels * 2;
        let mut body = "[1, 255]".to_owned();
        for _ in 0..levels {
            body = format!(
                "if do {{ trace += 1; select(false) }} {{ [2] }}
             else if do {{ trace += 1; select(true) }} {{ do {{ {body} }} }}
             else {{ [3] }}"
            );
        }
        let mut calls = vec!["check", "Build"];
        calls.extend(std::iter::repeat_n("select", levels * 2));
        calls.extend(["Byte", "Build", "Byte"]);
        check_in_engines(
            &format!(
                "
            pure comptime fn Packet<T, .values: meta.List<T> = []> -> Type {{ bool }}
            comptime fn Byte() -> Type {{ u8 }}
            comptime fn select(value: bool) -> bool {{ value }}
            comptime fn Build(value: T) -> Type
            where T
            {{
                var trace: Integer = 0;
                let result = Packet<.values = if T != bool {{ [] }} else {{
                    let flag: bool = value;
                    if flag {{ {body} }} else {{ [] }}
                }}, Byte()>;
                if T == bool and trace != {conditions} {{ return Packet<u8>; }}
                result
            }}
            comptime fn check() -> bool {{
                Build<bool>(true) == Packet<u8, .values = [1, 255]>
                    and Build<u8>(9) == Packet<u8>
            }}"
            ),
            &calls,
            Budget {
                steps: 1_000_000,
                ..Default::default()
            },
            engines,
        );
    }
}

#[test]
fn contextual_when_frames_preserve_ordered_conditions_and_nested_exits() {
    for (levels, engines) in [(3, &[false, true][..]), (10, &[false][..])] {
        let mut body = "[1, 255]".to_owned();
        for _ in 0..levels {
            body = format!(
                "when {{
                    do {{ trace += 1; select(false) }} => [2],
                    do {{ trace += 1; select(true) }} => do {{ {body} }},
                    select(true) => [3],
                    else => [4],
                }}"
            );
        }
        let mut calls = vec!["check", "Build"];
        calls.extend(std::iter::repeat_n("select", levels * 2));
        calls.extend(["Byte", "Build", "Byte"]);
        check_in_engines(
            &format!(
                "
                pure comptime fn Packet<T, .values: meta.List<T> = []> -> Type {{ bool }}
                comptime fn Byte() -> Type {{ u8 }}
                comptime fn select(value: bool) -> bool {{ value }}
                comptime fn Build(value: T) -> Type
                where T
                {{
                    var trace: Integer = 0;
                    let result = Packet<.values = when {{
                        T != bool => [],
                        else => do {{
                            let flag: bool = value;
                            if flag {{ {body} }} else {{ [] }}
                        }},
                    }}, Byte()>;
                    if T == bool and trace != {} {{ return Packet<u8>; }}
                    result
                }}
                comptime fn check() -> bool {{
                    Build<bool>(true) == Packet<u8, .values = [1, 255]>
                        and Build<u8>(9) == Packet<u8>
                }}",
                levels * 2,
            ),
            &calls,
            Budget {
                steps: 1_000_000,
                ..Default::default()
            },
            engines,
        );
    }
}

#[test]
fn contextual_when_frames_check_all_arms_and_restore_ordered_premises() {
    for (body, diagnostic) in [
        (
            "Packet<.values = when { T != bool => [], else => do {
                let flag: bool = value; [1]
            } }, Byte()>; let escaped: bool = value; Packet<u8>",
            "type mismatch",
        ),
        (
            "Packet<.values = when { true => [1], else => when {
                false => do { let invalid: bool = 7; [] }, else => []
            } }, Byte()>",
            "integer runtime proxy requires a supported fixed-width integer target",
        ),
        (
            "Packet<.values = when { T == bool => [], else => when {
                true => do { let invalid: bool = value; [] }, else => []
            } }, Byte()>",
            "type mismatch",
        ),
        (
            "Packet<.values = when { true => [1], else => when {
                else => [], true => []
            } }, Byte()>",
            "when fallback must be the final arm",
        ),
    ] {
        let failure = sysroot::compile(&format!(
            "pure comptime fn Packet<T, .values: meta.List<T> = []> -> Type {{ bool }}
             comptime fn Byte() -> Type {{ u8 }}
             comptime fn Bad(value: T) -> Type where T {{ {body} }}"
        ))
        .err()
        .expect("invalid contextual arm was accepted");
        assert!(failure.contains(diagnostic), "{failure}\n{body}");
    }
}

#[test]
fn contextual_when_returns_exit_the_original_invocation_without_later_inputs() {
    check_both(
        "
        pure comptime fn Packet<T, .values: meta.List<T> = []> -> Type { bool }
        comptime fn Byte() -> Type { u8 }
        comptime fn mark(flag: bool) -> bool { flag }
        comptime fn select() -> bool { false }
        comptime fn Build(value: T) -> Type
        where T
        {
            Packet<.values = when {
                T == bool => do { mark(value); return Packet<u8>; [1] },
                select() => [2],
                else => do { mark(false); [255] },
            }, Byte()>
        }
        comptime fn ExitCondition() -> Type {
            Packet<.values = when {
                do { mark(true); return Packet<u8>; false } => [1],
                else => [2],
            }, Byte()>
        }
        comptime fn check() -> bool {
            Build<bool>(true) == Packet<u8>
                and Build<u8>(9) == Packet<u8, .values = [255]>
                and ExitCondition() == Packet<u8>
        }",
        &[
            "check",
            "Build",
            "mark",
            "Build",
            "select",
            "mark",
            "Byte",
            "ExitCondition",
            "mark",
        ],
    );
}

#[test]
fn contextual_match_frames_keep_selector_and_failed_guard_execution_order() {
    for (levels, engines) in [(3, &[false, true][..]), (10, &[false][..])] {
        let mut body = "[1, 255]".to_owned();
        for _ in 0..levels {
            body = format!(
                "do {{ trace += 1; select() }} match {{
                    flag if do {{ trace += 1; reject(flag) }} => [2],
                    flag if do {{ trace += 1; accept(flag) }} => do {{ {body} }},
                    _ => [3],
                }}"
            );
        }
        let mut calls = vec!["check", "Build"];
        for _ in 0..levels {
            calls.extend(["select", "reject", "accept"]);
        }
        calls.extend(["Byte", "Build", "Byte"]);
        check_in_engines(
            &format!(
                "
                pure comptime fn Packet<T, .values: meta.List<T> = []> -> Type {{ bool }}
                comptime fn Byte() -> Type {{ u8 }}
                comptime fn select() -> bool {{ true }}
                comptime fn reject(value: bool) -> bool {{ not value }}
                comptime fn accept(value: bool) -> bool {{ value }}
                comptime fn Build(value: T) -> Type
                where T
                {{
                    var trace: Integer = 0;
                    let result = Packet<.values = T match {{
                        'bool => do {{
                            let flag: bool = value;
                            if flag {{ {body} }} else {{ [] }}
                        }},
                        _ => [],
                    }}, Byte()>;
                    if T == bool and trace != {} {{ return Packet<u8>; }}
                    result
                }}
                comptime fn check() -> bool {{
                    Build<bool>(true) == Packet<u8, .values = [1, 255]>
                        and Build<u8>(9) == Packet<u8>
                }}",
                levels * 3,
            ),
            &calls,
            Budget {
                steps: 1_000_000,
                ..Default::default()
            },
            engines,
        );
    }
}

#[test]
fn contextual_match_frames_keep_coverage_and_branch_fact_checks() {
    for (body, diagnostic) in [
        (
            "Packet<.values = T match { 'bool => do {
                let flag: bool = value; [1]
            }, _ => [] }, Byte()>; let escaped: bool = value; Packet<u8>",
            "type mismatch",
        ),
        (
            "Packet<.values = true match { true => [1], false => do {
                let invalid: bool = 7; []
            } }, Byte()>",
            "integer runtime proxy requires a supported fixed-width integer target",
        ),
        (
            "Packet<.values = T match { 'bool => [], _ => do {
                let invalid: bool = value; []
            } }, Byte()>",
            "type mismatch",
        ),
        (
            "Packet<.values = value match { flag if guard(flag) => [1] }, Byte()>",
            "type mismatch",
        ),
        (
            "Packet<.values = guard(true) match { flag if guard(flag) => [1] }, Byte()>",
            "exhaust",
        ),
        (
            "Packet<.values = true match { flag if 7 => [1], _ => [] }, Byte()>",
            "integer runtime proxy requires a supported fixed-width integer target",
        ),
    ] {
        let failure = sysroot::compile(&format!(
            "pure comptime fn Packet<T, .values: meta.List<T> = []> -> Type {{ bool }}
             comptime fn Byte() -> Type {{ u8 }}
             comptime fn guard(flag: bool) -> bool {{ flag }}
             comptime fn Bad(value: T) -> Type where T {{ {body} }}"
        ))
        .err()
        .expect("invalid contextual match was accepted");
        assert!(failure.contains(diagnostic), "{failure}\n{body}");
    }
}

#[test]
fn contextual_match_returns_keep_the_original_activation_and_skip_later_inputs() {
    check_both(
        "
        pure comptime fn Packet<T, .values: meta.List<T> = []> -> Type { bool }
        comptime fn Byte() -> Type { u8 }
        comptime fn mark(flag: bool) -> bool { flag }
        comptime fn Build(value: T) -> Type
        where T
        {
            Packet<.values = T match {
                'bool => do { mark(value); return Packet<u8>; [1] },
                _ => do { mark(false); [255] },
            }, Byte()>
        }
        comptime fn ExitSelector() -> Type {
            Packet<.values = do { mark(true); return Packet<u8>; true } match {
                _ => [1],
            }, Byte()>
        }
        comptime fn check() -> bool {
            Build<bool>(true) == Packet<u8>
                and Build<u8>(9) == Packet<u8, .values = [255]>
                and ExitSelector() == Packet<u8>
        }",
        &[
            "check",
            "Build",
            "mark",
            "Build",
            "mark",
            "Byte",
            "ExitSelector",
            "mark",
        ],
    );
}

#[test]
fn contextual_if_frames_do_not_skip_invalid_siblings_or_leak_negative_premises() {
    for (body, diagnostic) in [
        (
            "Packet<.values = if T != bool { [] } else {
                do { let flag: bool = value; [1] }
            }, Byte()>; let escaped: bool = value; Packet<u8>",
            "type mismatch",
        ),
        (
            "Packet<.values = if true { [1] } else {
                do { let invalid: bool = 7; [] }
            }, Byte()>",
            "integer runtime proxy requires a supported fixed-width integer target",
        ),
        (
            "Packet<.values = if T == bool { [] } else if true {
                do { let invalid: bool = value; [] }
            } else { [] }, Byte()>",
            "type mismatch",
        ),
    ] {
        let failure = sysroot::compile(&format!(
            "pure comptime fn Packet<T, .values: meta.List<T> = []> -> Type {{ bool }}
            comptime fn Byte() -> Type {{ u8 }}
            comptime fn Bad(value: T) -> Type
            where T
            {{ {body} }}"
        ))
        .unwrap_err();
        assert!(failure.contains(diagnostic), "{body}: {failure}");
    }
}

#[test]
fn ordinary_if_frames_retain_callee_positions_and_inferred_sibling_destinations() {
    check_both(
        "
        struct Counter { count: Integer }
        impl Counter {
            comptime fn choose(self) -> bool { self.count += 1; true }
            comptime fn next(self) -> u8 { self.count += 1; 7 }
        }
        comptime fn identity(value: T) -> T where T { value }
        comptime fn check() -> bool {
            var counter = Counter { count: 0 };
            let selected = (if counter.choose() {
                do { if true { identity<u8> } else { identity<u8> } }
            } else { identity<u8> })(counter.next());
            let inferred = if true { do { selected == 7 } } else { false };
            let other = if false { counter.next() } else { counter.next() };
            inferred and selected == 7 and other == 7 and counter.count == 3
        }
        ",
        &["check", "choose", "next", "identity", "next"],
    );
}

#[test]
fn ordinary_if_joins_use_branch_premises_and_restore_them_after_early_exits() {
    check_both(
        "
        comptime fn identity(value: T) -> T
        where T
        {
            if T == bool { if value { true } else { false } } else { value }
        }
        comptime fn choose(value: T) -> bool
        where T
        {
            if T != bool { return false; }
            if value { do { return true; false } } else { false }
        }
        comptime fn check() -> bool {
            identity<bool>(true) and identity<u8>(7) == 7
                and choose<bool>(true) and not choose<u8>(7)
        }
        ",
        &["check", "identity", "identity", "choose", "choose"],
    );
    let failure = sysroot::compile(
        "comptime fn invalid(value: T) -> bool
        where T
        {
            let local = if T == bool { let valid: bool = value; valid } else { false };
            let escaped: bool = value;
            local and escaped
        }",
    )
    .unwrap_err();
    assert!(failure.contains("type mismatch"), "{failure}");
}

#[test]
fn partial_branch_search_prefers_typed_evidence_after_nested_missing_inputs() {
    for body in [
        "if false { 1 } else { byte() }",
        "if false { do { if true { null } else { null } } } else { byte() }",
        "if false { do { let local = true; if local { null } else { null } } } else { do { byte() } }",
        "false match { true => null, false => byte() }",
        "false match { true => do { let local = true; local match { true => null, false => null } }, false => byte() }",
        "when { false => 1, else => byte() }",
        "when { false => do { let local = true; local match { true => null, false => null } }, else => byte() }",
        "false match { true => when { false => null, else => null }, false => do { byte() } }",
        "if false { when { false => null, else => null } } else { when { false => 1, else => byte() } }",
    ] {
        check_both(
            &format!(
                "
                comptime fn keep(value: ?T) -> ?T where T {{ value }}
                comptime fn byte() -> u8 {{ 9 }}
                comptime fn payload_is_byte(value: ?T) -> bool where T {{ T == u8 }}
                comptime fn check() -> bool {{
                    let value = keep({body});
                    payload_is_byte(value)
                }}
                "
            ),
            &["check", "byte", "keep", "payload_is_byte"],
        );
    }
}

#[test]
fn partial_probe_recovery_does_not_hide_errors_or_publish_branch_locals() {
    for body in [
        "if false { do { let private_value: bool = true; null } } else { private_value }",
        "if false { do { let invalid: bool = 9; null } } else { true }",
        "if false { do { true match { true => null } } } else { true }",
        "when { false => do { let private_value: bool = true; null }, else => private_value }",
        "true match { false => do { let private_value: bool = true; null }, true => private_value }",
        "when { false => do { let invalid: bool = 9; null }, else => true }",
        "true match { false => do { let invalid: bool = 9; null }, true => true }",
        "when { false => null, else => true match { true => null } }",
        "true match { true => when { false => null, else => true match { true => null } } }",
    ] {
        let source = format!(
            "fn keep(value: ?T) -> ?T where T {{ value }}
            fn run() -> bool {{ let result = keep({body}); true }}"
        );
        let failure = sysroot::compile(&source).expect_err("invalid branch must be checked");
        assert!(!failure.contains("parsing failed"), "{failure}");
        assert!(!failure.contains("budget exhausted"), "{failure}");
    }
}

#[test]
fn partial_arm_probes_keep_type_facts_private_and_preserve_execution_order() {
    for body in [
        "when { T != bool => null, else => do { let flag: bool = value; mark(flag) } }",
        "T match { 'bool => do { let flag: bool = value; mark(flag) }, _ => null }",
        "when { T != bool => null, else => T match { 'bool => mark(value), _ => null } }",
    ] {
        check_both(
            &format!(
                "comptime fn keep(value: ?T) -> ?T where T {{ value }}
                comptime fn mark(value: bool) -> bool {{ value }}
                comptime fn select(value: T) -> bool where T {{
                    let result = keep({body});
                    result match {{ flag? => flag, null => false }}
                }}
                comptime fn check() -> bool {{ select<bool>(true) }}"
            ),
            &["check", "select", "mark", "keep"],
        );
    }
    for body in [
        "when { T != bool => null, else => true }",
        "T match { 'bool => true, _ => null }",
    ] {
        let source = format!(
            "fn keep(value: ?T) -> ?T where T {{ value }}
            fn choose(value: T) -> bool where T {{
                let result = keep({body});
                let flag: bool = value;
                flag
            }}"
        );
        let failure = sysroot::compile(&source).expect_err("probe facts must not escape the arms");
        assert!(failure.contains("type mismatch"), "{failure}");
    }
}

#[test]
fn partial_arm_probes_preserve_error_delivery_after_domain_selection() {
    for body in [
        "if fail { error true } else { byte() }",
        "when { fail => error true, else => byte() }",
        "fail match { true => error true, false => byte() }",
        "when { fail => error true, else => false match { true => error false, false => byte() } }",
        "fail match { true => error true, false => when { false => error false, else => byte() } }",
    ] {
        for fail in [false, true] {
            let source = format!(
                "comptime fn keep(value: !bool T) -> !bool T where T {{ value }}
                comptime fn byte() -> u8 {{ 9 }}
                comptime fn select(fail: bool) -> bool {{
                    let result = keep({body});
                    is_byte(result)
                }}
                comptime fn is_byte(value: !bool T) -> bool where T {{
                    T == u8 and (value match {{ _! => true, error flag => flag }})
                }}
                comptime fn check() -> bool {{ select({fail}) }}"
            );
            let calls: &[&str] = if fail {
                &["check", "select", "keep", "is_byte"]
            } else {
                &["check", "select", "byte", "keep", "is_byte"]
            };
            check_both(&source, calls);
        }
    }
}

#[test]
fn partial_arm_probes_restore_return_targets_and_reject_foreign_error_rows() {
    for body in [
        "when { flag => do { return true; null }, else => byte() }",
        "flag match { true => do { return true; null }, false => byte() }",
    ] {
        check_both(
            &format!(
                "comptime fn keep(value: ?T) -> ?T where T {{ value }}
                comptime fn byte() -> u8 {{ 9 }}
                comptime fn select(flag: bool) -> bool {{
                    let result = keep({body}); false
                }}
                comptime fn check() -> bool {{ select(true) }}"
            ),
            &["check", "select"],
        );
    }
    for body in [
        "if true { error failure } else { true }",
        "when { true => error failure, else => true }",
        "true match { true => error failure, false => true }",
    ] {
        let source = format!(
            "fn keep(value: !bool T) -> !bool T where T {{ value }}
            fn run() -> bool {{
                let failure: u8 = 7;
                let result = keep({body}); true
            }}"
        );
        let failure = sysroot::compile(&source).expect_err("foreign error row must be rejected");
        assert!(!failure.contains("parsing failed"), "{failure}");
        assert!(!failure.contains("budget exhausted"), "{failure}");
        assert!(
            failure.contains("type mismatch: expected (primitive bool), found (primitive u8)"),
            "{failure}"
        );
    }
}

#[test]
fn declaration_defaults_specialize_each_completed_prefix_without_reexecution() {
    check_both(
        r#"
        struct Config where enabled: bool { flag: bool }
        comptime fn prefix(value: bool) -> bool { value }
        comptime fn field(value: bool) -> bool { value }
        pure comptime fn Packet<
            .enabled: bool = true,
            .first: Config<enabled> = defer Config<enabled> { flag: field(enabled) },
            .second: Config<enabled> = defer do {
                let previous = first;
                let read = || previous.flag;
                Config<enabled> { flag: read() }
            }> -> Type { bool }
        comptime fn check() -> bool {
            let yes = Packet<.enabled = prefix(true)>;
            let no = Packet<.enabled = prefix(false)>;
            (yes match { Packet<true, first, second> => first.flag and second.flag, _ => false }) and
            (no match { Packet<false, first, second> => not first.flag and not second.flag, _ => false })
        }
        "#,
        &["check", "prefix", "field", "prefix", "field"],
    );
}

#[test]
fn declaration_default_overrides_preserve_source_order_and_skip_default_effects() {
    check_both(
        r#"
        struct Config where enabled: bool { flag: bool }
        comptime fn prefix(value: bool) -> bool { value }
        comptime fn field(value: bool) -> bool { value }
        comptime fn unused(value: bool) -> bool { value }
        pure comptime fn Packet<
            .enabled: bool = true,
            .first: Config<enabled> = defer Config<enabled> { flag: unused(enabled) },
            .second: Config<enabled> = defer do {
                let previous = first;
                let read = || previous.flag;
                Config<enabled> { flag: read() }
            }> -> Type { bool }
        comptime fn check() -> bool {
            Packet<.first = Config<false> { flag: field(true) }, .enabled = prefix(false)> match {
                Packet<false, first, second> => first.flag and second.flag,
                _ => false,
            }
        }
        "#,
        &["check", "field", "prefix"],
    );
}

#[test]
fn declaration_defaults_still_reject_forward_references_and_invalid_domains() {
    for (default, message) in [
        (
            "first",
            "normal-form default references itself or a later parameter",
        ),
        (
            "second",
            "normal-form default references itself or a later parameter",
        ),
        ("Config<true> { flag: enabled }", "unresolved type equality"),
        ("Config<enabled> { flag: Type }", "type mismatch"),
    ] {
        let source = format!(
            "struct Config where enabled: bool {{ flag: bool }}
             pure comptime fn Packet<.enabled: bool = true,
                .first: Config<enabled> = defer {default},
                .second: Config<enabled> = defer first> -> Type {{ bool }}"
        );
        let failure = sysroot::compile(&source).unwrap_err();
        assert!(failure.contains(message), "{default}: {failure}");
    }
}

#[test]
fn declaration_defaults_keep_actual_domain_checks_and_recover_after_rejection() {
    let package = sysroot::compile(
        r#"
        struct Config where enabled: bool { flag: bool }
        comptime fn prefix(value: bool) -> bool { value }
        comptime fn field(value: bool) -> bool { value }
        comptime fn later() -> bool { true }
        pure comptime fn Packet<
            .enabled: bool = true,
            .value: Config<enabled> = defer Config<enabled> { flag: field(enabled) },
            .last: bool = defer later()> -> Type { bool }
        comptime fn Invalid() -> Type {
            Packet<.enabled = prefix(false), .value = Config<true> { flag: true }>
        }
        comptime fn Valid() -> Type { Packet<.enabled = prefix(true)> }
        "#,
    )
    .unwrap();
    let id = |name| {
        package
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .declaration
    };
    for reference in [false, true] {
        sysroot::with_comptime(&package, sysroot::COMPTIME_BUDGET, reference, |vm| {
            let failure = vm.invoke(id("Invalid"), vec![]).unwrap_err();
            assert_eq!(
                failure.kind,
                comptime_interpretor::ErrorKind::TypeContractRejected
            );
            assert!(matches!(
                vm.invoke(id("Valid"), vec![]).unwrap(),
                Value::Type(_)
            ));
            let calls = vm
                .events()
                .iter()
                .filter_map(|event| {
                    let Event::EnterFunction(id) = event else {
                        return None;
                    };
                    package
                        .functions
                        .iter()
                        .find(|f| f.declaration == *id)
                        .map(|f| f.name.as_str())
                })
                .collect::<Vec<_>>();
            assert_eq!(
                calls,
                ["Invalid", "prefix", "Valid", "prefix", "field", "later"]
            );
        });
    }
}

#[test]
fn ordinary_when_frames_keep_nested_callee_delivery_and_single_evaluation() {
    check_both(
        "
        struct Counter { count: Integer }
        impl Counter {
            comptime fn no(self) -> bool { self.count += 1; false }
            comptime fn yes(self) -> bool { self.count += 1; true }
            comptime fn next(self) -> u8 { self.count += 1; 7 }
        }
        comptime fn identity(value: T) -> T where T { value }
        comptime fn check() -> bool {
            var counter = Counter { count: 0 };
            let selected = (when {
                counter.no() => identity<u8>,
                counter.yes() => when { true => identity<u8>, else => identity<u8>, },
                counter.yes() => identity<u8>,
                else => identity<u8>,
            })(counter.next());
            selected == 7 and counter.count == 3
        }
        ",
        &["check", "no", "yes", "next", "identity"],
    );
}

#[test]
fn ordinary_when_frames_join_under_ordered_premises_without_exporting_them() {
    check_both(
        "
        comptime fn choose(value: T) -> T
        where T
        {
            when {
                T == bool => when { value => true, else => false, },
                else => value,
            }
        }
        comptime fn check() -> bool { choose<bool>(true) and choose<u8>(7) == 7 }
        ",
        &["check", "choose", "choose"],
    );
    let failure = sysroot::compile(
        "comptime fn invalid(value: T) -> bool
        where T
        {
            let selected = when { T == bool => do { let valid: bool = value; valid }, else => false, };
            let escaped: bool = value;
            selected and escaped
        }",
    ).unwrap_err();
    assert!(failure.contains("type mismatch"), "{failure}");
}

#[test]
fn nested_ordinary_when_frames_preserve_the_original_return_target() {
    // The reference engine has a separate 64-level native recursion guard.
    // Compare both engines inside it; keep 32 source levels for bytecode and
    // frontend observation on the default host thread stack.
    for (levels, engines) in [(8, &[false, true][..]), (32, &[false][..])] {
        let mut nested = "do { return true; false }".to_string();
        for _ in 0..levels {
            nested = format!("when {{ false => false, else => {nested}, }}");
        }
        check_in_engines(
            &format!("comptime fn check() -> bool {{ let value: bool = {nested}; not value }}"),
            &["check"],
            sysroot::COMPTIME_BUDGET,
            engines,
        );
    }
}

#[test]
fn ordinary_match_frames_keep_nested_callee_selection_and_guard_order() {
    check_both(
        "
        comptime fn select() -> bool { true }
        comptime fn reject(value: bool) -> bool { not value }
        comptime fn supply() -> u8 { 7 }
        comptime fn identity(value: T) -> T where T { value }
        comptime fn check() -> bool {
            let answer = (select() match {
                value if reject(value) => identity<u8>,
                true => false match { false => identity<u8>, true => identity<u8>, },
                false => identity<u8>,
            })(supply());
            answer == 7
        }
        ",
        &["check", "select", "reject", "supply", "identity"],
    );
}

#[test]
fn ordinary_match_frames_restore_pattern_facts_after_nested_bodies() {
    check_both(
        "
        comptime fn accepts(value: T) -> bool
        where T
        {
            T match {
                'bool => value match { true => true, false => false, },
                _ => false,
            }
        }
        comptime fn check() -> bool { accepts<bool>(true) and not accepts<u8>(7) }
        ",
        &["check", "accepts", "accepts"],
    );
    for body in [
        "let selected = T match { 'bool => value, _ => false }; let escaped: bool = value; selected and escaped",
        "T match { 'bool => value, _ => do { let escaped: bool = value; escaped } }",
    ] {
        let failure = sysroot::compile(&format!(
            "comptime fn invalid(value: T) -> bool where T {{ {body} }}"
        ))
        .unwrap_err();
        assert!(failure.contains("type mismatch"), "{failure}");
    }
}

#[test]
fn nested_ordinary_match_frames_preserve_the_original_return_target() {
    // Keep the 32-level source case; the reference evaluator independently
    // caps native recursion, so compare engines below that boundary.
    for (levels, engines) in [(8, &[false, true][..]), (32, &[false][..])] {
        let mut nested = "do { return true; false }".to_string();
        for _ in 0..levels {
            nested = format!("true match {{ false => false, true => {nested}, }}");
        }
        check_in_engines(
            &format!("comptime fn check() -> bool {{ let value: bool = {nested}; not value }}"),
            &["check"],
            sysroot::COMPTIME_BUDGET,
            engines,
        );
    }
}
