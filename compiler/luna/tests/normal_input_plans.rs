//! NF inputs share contextual checking while retaining source evaluation slots.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_interpretor::{Budget, Event, Value};
use nir::program::{ExprKind as K, NormalFormArgument, Package};

fn id(package: &Package, name: &str) -> nir::DeclarationId {
    package
        .functions
        .iter()
        .find(|f| f.name == name)
        .unwrap()
        .declaration
}

fn invoke_both(package: &Package, name: &str) -> Value {
    let actual = sysroot::with_comptime(package, sysroot::COMPTIME_BUDGET, false, |vm| {
        vm.invoke(id(package, name), vec![]).unwrap()
    });
    let expected = sysroot::with_comptime(package, sysroot::COMPTIME_BUDGET, true, |vm| {
        vm.invoke(id(package, name), vec![]).unwrap()
    });
    assert_eq!(expected, actual);
    actual
}

fn assert_ordinary_reordered_inputs(package: &Package, name: &str) {
    let function = package.functions.iter().find(|f| f.name == name).unwrap();
    let mut count = 0;
    nir::visit::expressions(function, &mut 10_000, |expression, _| {
        if let K::NormalFormType {
            arguments,
            plan: Some(plan),
            ..
        } = &expression.kind
            && plan.order == [1, 0]
        {
            // A ready domain must use ordinary checked expressions, not an
            // NF-specific If/Block/Match tree interpreted again at invocation.
            assert!(
                arguments
                    .iter()
                    .all(|argument| matches!(argument, NormalFormArgument::Value(_))),
                "{expression}"
            );
            count += 1;
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(count, 1, "{name}: {}", function.body);
}

#[test]
fn late_static_domains_contextualize_whole_collections_as_ordinary_expressions() {
    let package = sysroot::compile(
        "
        pure comptime fn Packet<T, ...values: meta.List<T>> -> Type { bool }
        comptime fn Direct() -> Type { Packet<u8, 1, 255> }
        comptime fn Branch() -> Type {
            Packet<.values = if true { [1, 255] } else { [] }, u8>
        }
        comptime fn Block() -> Type {
            Packet<.values = do { let selected = true; if selected { [1, 255] } else { [] } }, u8>
        }
        comptime fn Matched() -> Type {
            Packet<.values = true match { true => [1, 255], false => [] }, u8>
        }
        comptime fn Nested() -> Type {
            Packet<.values = [[1], []], meta.List<u8>>
        }
        comptime fn NestedDirect() -> Type {
            Packet<meta.List<u8>, [1], []>
        }
        comptime fn Product() -> Type {
            Packet<.values = [(1, true)], Tuple<u8, bool>>
        }
        comptime fn ProductDirect() -> Type {
            Packet<Tuple<u8, bool>, (1, true)>
        }
    ",
    )
    .unwrap();
    let direct = invoke_both(&package, "Direct");
    for name in ["Branch", "Block", "Matched"] {
        assert_ordinary_reordered_inputs(&package, name);
        assert_eq!(invoke_both(&package, name), direct);
    }
    for (name, reference) in [("Nested", "NestedDirect"), ("Product", "ProductDirect")] {
        assert_ordinary_reordered_inputs(&package, name);
        assert_eq!(
            invoke_both(&package, name),
            invoke_both(&package, reference)
        );
    }
}

#[test]
fn nested_normal_regions_retain_opaque_heads_and_source_effects() {
    let body = (0..8).fold("u8".to_owned(), |body, index| match index % 3 {
        0 => format!("Layer<do {{ mark(); {body} }}>"),
        1 => format!("Layer<if true {{ mark(); {body} }} else {{ bool }}>"),
        _ => format!("Layer<when {{ false => bool, else => do {{ mark(); {body} }} }}>"),
    });
    let package = sysroot::compile(&format!(
        "pure comptime fn Layer<T> -> Type {{ T }}
        comptime fn mark() -> bool {{ true }}
        comptime fn Build() -> Type {{ {body} }}"
    ))
    .unwrap();
    let mut expected = nir::Term::Primitive(nir::Primitive::U8);
    for _ in 0..8 {
        expected = nir::Term::NormalForm {
            head: id(&package, "Layer"),
            sort: nir::Sort::Type,
            arguments: vec![expected],
        };
    }
    let mut calls = vec!["Build"];
    calls.extend(std::iter::repeat_n("mark", 8));
    trace_both(&package, "Build", &Value::Type(expected), &calls);
}

#[test]
fn nested_normal_input_failures_retain_domain_and_formation_diagnostics() {
    for (input, expected) in [
        ("Layer<Layer<true>>", "type mismatch"),
        (
            "Layer<Restricted<bool>>",
            "type requirement is not satisfied",
        ),
        ("Layer<Layer<>>", "argument count mismatch"),
    ] {
        let source = format!(
            "pure comptime fn Layer<T> -> Type {{ T }}
            pure comptime fn Restricted<T> -> Type
            where requires T == u8
            {{ T }}
            comptime fn Build() -> Type {{ {input} }}"
        );
        let failure = match sysroot::compile(&source) {
            Err(failure) => failure,
            Ok(_) => panic!("accepted {input}"),
        };
        assert!(failure.contains(expected), "{input}: {failure}");
    }
}

#[test]
fn fixed_inputs_and_defaults_share_the_contextual_plan() {
    let package = sysroot::compile(
        "
        pure comptime fn Key<T, .value: ?T = null> -> Type { bool }
        comptime fn Explicit() -> Type { Key<.value = null, u8> }
        comptime fn Default() -> Type { Key<u8> }
        pure comptime fn Point<Es: meta.List<Type>, value: Tuple<.Es = Es>> -> Type { bool }
        comptime fn Product() -> Type { Point<[u8, bool], (1, true)> }
        typealias ProductAlias = Point<[u8, bool], (1, true)>;
        comptime fn Alias() -> Type { ProductAlias }
    ",
    )
    .unwrap();
    assert_ordinary_reordered_inputs(&package, "Explicit");
    assert_eq!(
        invoke_both(&package, "Explicit"),
        invoke_both(&package, "Default")
    );
    assert_eq!(
        invoke_both(&package, "Product"),
        invoke_both(&package, "Alias")
    );
}

#[test]
fn default_domain_preparation_preserves_explicit_order_and_default_execution() {
    let package = sysroot::compile(
        "
        comptime fn first() -> bool { true }
        comptime fn second() -> bool { true }
        comptime fn mark() -> bool { true }
        pure comptime fn Tagged<flag: bool, .T: Type = do { mark(); u8 }, value: T> -> Type { bool }
        comptime fn Build() -> Type { Tagged<first(), do { second(); 255 }> }
        comptime fn Override() -> Type { Tagged<first(), do { second(); true }, .T = bool> }
        comptime fn Exit() -> Type {
            Tagged<first(), do { second(); return bool; 1 }>
        }
        comptime fn Direct() -> Type { Tagged<true, .T = u8, 255> }
        comptime fn DirectOverride() -> Type { Tagged<true, .T = bool, true> }
    ",
    )
    .unwrap();
    let expected = invoke_both(&package, "Direct");
    trace_both(
        &package,
        "Build",
        &expected,
        &["Build", "first", "second", "mark"],
    );
    let expected = invoke_both(&package, "DirectOverride");
    trace_both(
        &package,
        "Override",
        &expected,
        &["Override", "first", "second"],
    );
    trace_both(
        &package,
        "Exit",
        &Value::Type(nir::Term::Primitive(nir::Primitive::Bool)),
        &["Exit", "first", "second"],
    );
}

#[test]
fn observed_domains_retain_effectful_producers_in_their_source_slots() {
    let package = sysroot::compile(
        "
        pure comptime fn Packet<T, ...values: meta.List<T>> -> Type { bool }
        comptime fn mark() -> bool { true }
        comptime fn Direct() -> Type { Packet<u8, 1, 255> }
        comptime fn Build() -> Type {
            Packet<.values = if true { [1, 255] } else { [] },
                do { mark(); let selected = u8; selected }>
        }
        comptime fn Twice() -> Type { Build(); Build() }
        comptime fn check() -> bool {
            var trace: Integer = 0;
            let actual = Packet<do { trace = trace * 10 + 1; u8 },
                do { trace = trace * 10 + 2; 1 }, 255>;
            actual == Packet<u8, 1, 255> and trace == 12
        }
    ",
    )
    .unwrap();
    assert_ordinary_reordered_inputs(&package, "Build");
    let direct = invoke_both(&package, "Direct");
    trace_both(&package, "Build", &direct, &["Build", "mark"]);
    trace_both(
        &package,
        "Twice",
        &direct,
        &["Twice", "Build", "mark", "Build", "mark"],
    );
    trace_both(&package, "check", &Value::Bool(true), &["check"]);
}

#[test]
fn observed_domains_check_unexecuted_branches_before_invocation() {
    for value in [
        "if true { [1] } else { [256] }",
        "if true { [1] } else { [false] }",
    ] {
        let source = format!(
            "
            pure comptime fn Packet<T, ...values: meta.List<T>> -> Type {{ bool }}
            comptime fn mark() -> bool {{ true }}
            comptime fn Bad() -> Type {{
                Packet<.values = {value}, do {{ mark(); u8 }}>
            }}
        "
        );
        assert!(sysroot::compile(&source).is_err(), "accepted {source}");
    }
}

#[test]
fn observed_packet_fields_and_defaults_retain_their_producers() {
    let package = sysroot::compile(
        "
        pure comptime fn Packet<T, tag: bool, ...values: meta.List<T>> -> Type { bool }
        pure comptime fn Defaulted<T, .U: Type = T, .value: ?U = null> -> Type { bool }
        comptime fn mark() -> bool { true }
        comptime fn Direct() -> Type { Packet<u8, true, 1, 255> }
        comptime fn Build() -> Type {
            Packet<...do { mark(); (u8, true) }, 1, 255>
        }
        comptime fn check() -> bool {
            var trace: Integer = 0;
            let actual = Defaulted<do { trace = trace + 1; u8 }>;
            actual == Defaulted<u8> and trace == 1
        }
        comptime fn Exit() -> Type {
            Packet<...do { return Direct(); (u8, true) },
                do { mark(); 1 }, 255>
        }
    ",
    )
    .unwrap();
    let direct = invoke_both(&package, "Direct");
    trace_both(&package, "Build", &direct, &["Build", "mark"]);
    trace_both(&package, "check", &Value::Bool(true), &["check"]);
    trace_both(&package, "Exit", &direct, &["Exit", "Direct"]);
}

#[test]
fn observed_collection_temporaries_keep_formation_checks_and_effects() {
    let prefix = "
        pure comptime fn Packet<...values: meta.List<u8>> -> Type { bool }
        pure comptime fn Checked<T, .tag: bool = true, ...values: meta.List<u8>> -> Type
        where requires T == Packet<.values = values>
        {
            T
        }
        comptime fn mark() -> bool { true }
    ";
    let package = sysroot::compile(&format!(
        "{prefix}
        comptime fn Direct() -> Type {{ Checked<Packet<1, 2>, 1, 2> }}
        comptime fn Build() -> Type {{
            Checked<Packet<1, 2>, do {{ mark(); 1 }}, .tag = mark(), 2>
        }}
    "
    ))
    .unwrap();
    let direct = invoke_both(&package, "Direct");
    trace_both(&package, "Build", &direct, &["Build", "mark", "mark"]);
    let failure = sysroot::compile(&format!(
        "{prefix}
        comptime fn Bad() -> Type {{
            Checked<Packet<1, 2>, do {{ mark(); 1 }}, .tag = mark(), 3>
        }}
    "
    ))
    .unwrap_err();
    assert!(
        failure.contains("type requirement is not satisfied"),
        "{failure}"
    );
}

#[test]
fn dynamic_domains_do_not_move_or_replay_input_effects() {
    let package = sysroot::compile(
        "
        pure comptime fn Packet<T, ...values: meta.List<T>> -> Type { bool }
        comptime fn choose() -> bool { true }
        comptime fn Byte() -> Type { u8 }
        comptime fn Direct() -> Type { Packet<u8, 1, 255> }
        comptime fn Build() -> Type {
            Packet<.values = if choose() { [1, 255] } else { [] }, Byte()>
        }
        comptime fn Wrong() -> Type {
            Packet<.values = if choose() { [256] } else { [] }, Byte()>
        }
    ",
    )
    .unwrap();
    let direct = invoke_both(&package, "Direct");
    for tree in [false, true] {
        sysroot::with_comptime(&package, sysroot::COMPTIME_BUDGET, tree, |vm| {
            assert!(vm.invoke(id(&package, "Wrong"), vec![]).is_err());
            let start = vm.events().len();
            assert_eq!(vm.invoke(id(&package, "Build"), vec![]).unwrap(), direct);
            let calls: Vec<_> = vm.events()[start..]
                .iter()
                .filter_map(|event| match event {
                    Event::EnterFunction(declaration) => package
                        .functions
                        .iter()
                        .find(|f| f.declaration == *declaration)
                        .map(|f| f.name.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(calls, ["Build", "choose", "Byte"]);
        });
    }
}

#[test]
fn ready_domains_reject_invalid_values_in_every_branch() {
    for value in [
        "[256]",
        "[true]",
        "if true { [1] } else { [256] }",
        "true match { true => [1], false => [false] }",
    ] {
        let source = format!(
            "
            pure comptime fn Packet<T, ...values: meta.List<T>> -> Type {{ bool }}
            comptime fn Bad() -> Type {{ Packet<.values = {value}, u8> }}
        "
        );
        assert!(sysroot::compile(&source).is_err(), "accepted {source}");
    }
}

fn trace_both(package: &Package, name: &str, expected: &Value, calls: &[&str]) {
    trace_both_in_budget(package, name, expected, calls, sysroot::COMPTIME_BUDGET);
}

fn trace_both_in_budget(
    package: &Package,
    name: &str,
    expected: &Value,
    calls: &[&str],
    budget: Budget,
) {
    for tree in [false, true] {
        sysroot::with_comptime(package, budget, tree, |vm| {
            assert_eq!(vm.invoke(id(package, name), vec![]).unwrap(), *expected);
            let actual: Vec<_> = vm
                .events()
                .iter()
                .filter_map(|event| match event {
                    Event::EnterFunction(declaration) => package
                        .functions
                        .iter()
                        .find(|f| f.declaration == *declaration)
                        .map(|f| f.name.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(actual, calls, "{name}, tree={tree}");
        });
    }
}

#[test]
fn collected_inputs_use_late_static_domains_and_ordinary_collection_expressions() {
    let package = sysroot::compile("
        pure comptime fn Packet<.T: Type = u8, .tag: bool = true, ...values: meta.List<T>> -> Type { bool }
        comptime fn choose() -> bool { true }
        comptime fn mark() -> bool { true }
        comptime fn Direct() -> Type { Packet<.T = u8, 1, 255> }
        comptime fn Build() -> Type {
            Packet<if choose() { 1 } else { 0 }, .tag = mark(), .T = u8,
                ...if true { [255] } else { [] }>
        }
    ").unwrap();
    let direct = invoke_both(&package, "Direct");
    trace_both(&package, "Build", &direct, &["Build", "choose", "mark"]);
    for name in ["Direct", "Build"] {
        let function = package.functions.iter().find(|f| f.name == name).unwrap();
        let mut collections = 0;
        nir::visit::expressions(function, &mut 10_000, |expression, _| {
            if let K::NormalFormType {
                head,
                arguments,
                plan,
                ..
            } = &expression.kind
                && *head == id(&package, "Packet")
            {
                assert!(plan.is_some(), "every head expression uses the shared plan");
                assert!(
                    arguments
                        .iter()
                        .all(|argument| matches!(argument, NormalFormArgument::Value(_))),
                    "{expression}"
                );
                collections += 1;
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(collections, 1);
    }
}

#[test]
fn dynamic_contiguous_collections_keep_contextual_branches_and_mixed_producer_order() {
    let package = sysroot::compile(
        "
        pure comptime fn Packet<T, .tag: bool = true, ...values: meta.List<T>> -> Type { bool }
        comptime fn Byte() -> Type { u8 }
        comptime fn first() -> u8 { 1 }
        comptime fn mark() -> bool { true }
        comptime fn choose() -> bool { true }
        comptime fn empty() -> meta.List<u8> { [] }
        comptime fn Direct() -> Type { Packet<u8, 1, 255> }
        comptime fn Branch() -> Type {
            Packet<Byte(), .tag = mark(), first(), if choose() { 255 } else { 0 }>
        }
        comptime fn Matched() -> Type {
            Packet<Byte(), first(), ...empty(), .tag = mark(),
                choose() match { true => 255, false => 0 }>
        }
        comptime fn Block() -> Type {
            Packet<Byte(), .tag = mark(), do { let yes = choose(); if yes { 1 } else { 0 } }, 255>
        }
        comptime fn Wrong() -> Type {
            Packet<Byte(), .tag = mark(), if choose() { 256 } else { 0 }>
        }
    ",
    )
    .unwrap();
    let direct = invoke_both(&package, "Direct");
    trace_both(
        &package,
        "Branch",
        &direct,
        &["Branch", "Byte", "mark", "first", "choose"],
    );
    trace_both(
        &package,
        "Matched",
        &direct,
        &["Matched", "Byte", "first", "empty", "mark", "choose"],
    );
    trace_both(
        &package,
        "Block",
        &direct,
        &["Block", "Byte", "mark", "choose"],
    );
    for tree in [false, true] {
        sysroot::with_comptime(&package, sysroot::COMPTIME_BUDGET, tree, |vm| {
            assert!(vm.invoke(id(&package, "Wrong"), vec![]).is_err());
            assert_eq!(vm.invoke(id(&package, "Branch"), vec![]).unwrap(), direct);
        });
    }
}

#[test]
fn collection_child_handoffs_preserve_probes_empty_spreads_and_source_exits() {
    let package = sysroot::compile(
        "
        pure comptime fn Packet<T, .tag: bool = true, ...values: meta.List<T>> -> Type { bool }
        comptime fn Byte() -> Type { u8 }
        comptime fn first() -> u8 { 5 }
        comptime fn second() -> u8 { 6 }
        comptime fn empty() -> meta.List<u8> { [] }
        comptime fn mark() -> bool { true }
        comptime fn Direct() -> Type { Packet<u8, 1, 5, 2, 3, 6, 4> }
        comptime fn Build() -> Type {
            Packet<Byte(), 1, first(), ...empty(), 2, .tag = mark(), 3, second(), 4>
        }
        comptime fn Exit() -> Type {
            Packet<Byte(), 1, do { return Direct(); first() }, .tag = mark(), second()>
        }
        ",
    )
    .unwrap();
    let direct = invoke_both(&package, "Direct");
    trace_both(
        &package,
        "Build",
        &direct,
        &["Build", "Byte", "first", "empty", "mark", "second"],
    );
    trace_both(&package, "Exit", &direct, &["Exit", "Byte", "Direct"]);
}

#[test]
fn collection_probe_does_not_turn_domain_failures_into_missing_context() {
    let failure = sysroot::compile(
        "
        pure comptime fn Packet<T, .tag: bool = true, ...values: meta.List<T>> -> Type { bool }
        comptime fn Byte() -> Type { u8 }
        comptime fn wrong() -> u8 { 1 }
        comptime fn accept(value: bool) -> u8 { 1 }
        comptime fn Bad() -> Type { Packet<Byte(), accept(wrong()), .tag = true, 2> }
        ",
    )
    .unwrap_err();
    assert!(failure.contains("type mismatch"), "{failure}");
    assert!(
        !failure.contains("structured collection input requires"),
        "{failure}"
    );
}

#[test]
fn collection_preparations_snapshot_reads_and_keep_empty_spread_effects() {
    let package = sysroot::compile(
        "
        pure comptime fn Packet<T, .tag: bool = true, ...values: meta.List<T>> -> Type { bool }
        comptime fn check() -> bool {
            var state: u8 = 1;
            let captured = Packet<u8, state,
                ...do { state = 2; let items: meta.List<u8> = []; items },
                .tag = do { state = 3; true }, state>;
            state == 3 and captured == Packet<u8, 1, 3>
        }
    ",
    )
    .unwrap();
    assert_eq!(invoke_both(&package, "check"), Value::Bool(true));
}

#[test]
fn collection_early_returns_skip_remaining_elements_and_named_inputs() {
    let package = sysroot::compile(
        "
        pure comptime fn Packet<T, .tag: bool = true, ...values: meta.List<T>> -> Type { bool }
        comptime fn Byte() -> Type { u8 }
        comptime fn mark() -> bool { true }
        comptime fn first() -> u8 { 1 }
        comptime fn Direct() -> Type { Packet<u8, 1> }
        comptime fn Tail() -> Type {
            Packet<Byte(), do { return Direct(); 255 }, first()>
        }
        comptime fn Interrupted() -> Type {
            Packet<u8, do { return Direct(); 255 }, .tag = mark(), first()>
        }
        comptime fn ReadyTail() -> Type {
            Packet<u8, do { return Direct(); 255 }, first()>
        }
        comptime fn SpreadExit() -> Type {
            Packet<u8, ...do { return Direct(); let values: meta.List<u8> = []; values }, first()>
        }
    ",
    )
    .unwrap();
    let direct = invoke_both(&package, "Direct");
    trace_both(&package, "Tail", &direct, &["Tail", "Byte", "Direct"]);
    trace_both(&package, "Interrupted", &direct, &["Interrupted", "Direct"]);
    trace_both(&package, "ReadyTail", &direct, &["ReadyTail", "Direct"]);
    trace_both(&package, "SpreadExit", &direct, &["SpreadExit", "Direct"]);
}

#[test]
fn collected_static_domains_check_all_branches_before_invocation() {
    for value in [
        "256",
        "true",
        "if true { 1 } else { 256 }",
        "true match { true => 1, false => false }",
    ] {
        let source = format!(
            "
            pure comptime fn Packet<.T: Type = u8, ...values: meta.List<T>> -> Type {{ bool }}
            comptime fn Bad() -> Type {{ Packet<{value}, .T = u8> }}
        "
        );
        assert!(sysroot::compile(&source).is_err(), "accepted {source}");
    }
}

#[test]
fn ordinary_collection_assembly_preserves_many_inline_spread_producers() {
    let count = 192;
    let inputs = (0..count)
        .map(|n| format!("...part({n})"))
        .collect::<Vec<_>>()
        .join(", ");
    let values = (0..count)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let package = sysroot::compile(&format!(
        "
        pure comptime fn Packet<T, ...values: meta.List<T>> -> Type {{ bool }}
        comptime fn part(value: u8) -> meta.List<u8> {{ [value] }}
        comptime fn Direct() -> Type {{ Packet<u8, .values = [{values}]> }}
        comptime fn Build() -> Type {{ Packet<u8, {inputs}> }}
    "
    ))
    .unwrap();
    let direct = invoke_both(&package, "Direct");
    let mut calls = vec!["Build"];
    calls.extend(std::iter::repeat_n("part", count));
    /*
     * Preserve the existing combined admission/execution ceiling. With the
     * complete sysroot query services, the source declaration view avoids a
     * whole-package copy while preserving query and admission costs. Do not
     * relax this ceiling or skip any of the 192 calls to hide regressions.
     * Scaling is also checked separately in contextual_input_costs.
     */
    trace_both_in_budget(
        &package,
        "Build",
        &direct,
        &calls,
        Budget {
            steps: 200_000,
            ..Default::default()
        },
    );
}

#[test]
fn static_spread_assembly_keeps_definition_time_formation_checks() {
    let prefix = "
        pure comptime fn Packet<...values: meta.List<u8>> -> Type { bool }
        pure comptime fn Checked<T, ...values: meta.List<u8>> -> Type
        where requires T == Packet<.values = values>
        {
            T
        }
    ";
    let package = sysroot::compile(&format!(
        "{prefix}
        comptime fn Good() -> Type {{ Checked<Packet<1, 2>, ...do {{ [1] }}, ...do {{ [2] }}> }}
        comptime fn Direct() -> Type {{ Checked<Packet<1, 2>, 1, 2> }}
        comptime fn unknown_items() -> meta.List<u8> {{ unknown_items() }}
        comptime fn Pending() -> Type {{ Checked<Packet<1, 2>, ...unknown_items(), ...do {{ [2] }}> }}
    "
    ))
    .unwrap();
    assert_eq!(
        invoke_both(&package, "Good"),
        invoke_both(&package, "Direct")
    );
    let failure = sysroot::compile(&format!(
        "{prefix}
        comptime fn Bad() -> Type {{ Checked<Packet<1, 2>, ...do {{ [1] }}, ...do {{ [3] }}> }}
    "
    ))
    .err()
    .expect("unused function must reject its statically invalid formation");
    assert!(failure.contains("type requirement"), "{failure}");
}

#[test]
fn type_annotations_share_named_defaults_packets_and_collections_with_values() {
    let package = sysroot::compile(
        "
        pure comptime fn Shape<T, .U: Type = bool> -> Type { T }
        pure comptime fn Pack<...Es: meta.List<Type>> -> Type { bool }
        typealias Named = Shape<.U = u8, bool>;
        typealias Defaulted = Shape<u8>;
        typealias Packet = Shape<...(bool,), .U = u8>;
        typealias Collected = Pack<...do { [bool] }, ...do { [u8] }>;
        typealias Whole = Pack<.Es = if true { [bool, u8] } else { [] }>;
        comptime fn NamedValue() -> Type { Shape<.U = u8, bool> }
        comptime fn DefaultValue() -> Type { Shape<u8> }
        comptime fn CollectedValue() -> Type { Pack<bool, u8> }
        comptime fn NamedAlias() -> Type { Named }
        comptime fn DefaultAlias() -> Type { Defaulted }
        comptime fn PacketAlias() -> Type { Packet }
        comptime fn CollectedAlias() -> Type { Collected }
        comptime fn WholeAlias() -> Type { Whole }
        ",
    )
    .unwrap();
    for (alias, reference) in [
        ("NamedAlias", "NamedValue"),
        ("PacketAlias", "NamedValue"),
        ("DefaultAlias", "DefaultValue"),
        ("CollectedAlias", "CollectedValue"),
        ("WholeAlias", "CollectedValue"),
    ] {
        assert_eq!(
            invoke_both(&package, alias),
            invoke_both(&package, reference)
        );
    }
    for position in [
        "typealias Bad = Shape<.U = bool, .U = u8, bool>;",
        "comptime fn Bad() -> Type { Shape<.U = bool, .U = u8, bool> }",
    ] {
        let error = sysroot::compile(&format!(
            "pure comptime fn Shape<T, .U: Type = bool> -> Type {{ T }} {position}"
        ))
        .unwrap_err();
        assert!(error.contains("duplicate normal-form argument"), "{error}");
    }
}

#[test]
fn annotated_and_computed_callbacks_keep_outer_and_named_binders() {
    let package = sysroot::compile(
        "
        pure comptime fn Identity<T> -> Type { T }
        pure comptime fn Indexed<T, count: usize> -> Type { T }
        fn direct(comptime T: Type, callback: Identity<fn(T) -> T>) {}
        fn observed(comptime T: Type, callback: Identity<fn(do { T }) -> do { T }>) {}
        fn nested(comptime T: Type, callback: Identity<fn(fn(T) -> T) -> fn(T) -> T>) {}
        typealias Named = fn(comptime T: Type, comptime count: usize,
            value: Indexed<T, count>) -> Indexed<T, count>;
        comptime fn NamedType() -> Type { Named }
        comptime fn NamedExpression() -> Type {
            fn(comptime T: Type, comptime count: usize,
                value: Indexed<T, count>) -> Indexed<T, count>
        }
        comptime fn Domain() -> Type { bool }
        comptime fn Constructed() -> Type { Identity<fn(Domain()) -> Domain()> }
        comptime fn Literal() -> Type { Identity<fn(bool) -> bool> }
        ",
    )
    .unwrap();
    let signature = |name| {
        &package
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .signature
    };
    assert_eq!(signature("direct"), signature("observed"));
    for name in ["direct", "observed", "nested"] {
        signature(name).check_scope(&[]).unwrap();
    }
    assert_eq!(
        invoke_both(&package, "NamedType"),
        invoke_both(&package, "NamedExpression")
    );
    assert_eq!(
        invoke_both(&package, "Constructed"),
        invoke_both(&package, "Literal")
    );
    let Value::Type(named) = invoke_both(&package, "NamedType") else {
        panic!()
    };
    named.check_scope(&[]).unwrap();
    let nir::Term::Function { parameters, result } = named else {
        panic!()
    };
    assert_eq!(parameters[0].sort, nir::Sort::Type);
    assert_eq!(parameters[1].sort, nir::Sort::ComptimeValue);
    assert_eq!(parameters[2].ty, *result);
    let nir::Term::NormalForm { arguments, .. } = *result else {
        panic!()
    };
    assert_eq!(
        arguments,
        vec![
            nir::Term::Bound {
                depth: 0,
                index: 0,
                sort: nir::Sort::Type
            },
            nir::Term::Bound {
                depth: 0,
                index: 1,
                sort: nir::Sort::ComptimeValue
            },
        ]
    );
    for source in [
        "typealias Bad = fn(T: Type, callback: Identity<T>);",
        "typealias Bad = fn(count: usize, value: Indexed<bool, count>);",
    ] {
        let error = sysroot::compile(&format!(
            "pure comptime fn Identity<T> -> Type {{ T }}
             pure comptime fn Indexed<T, count: usize> -> Type {{ T }} {source}"
        ))
        .unwrap_err();
        assert!(error.contains("explicit comptime"), "{error}");
    }
}

#[test]
fn signature_packets_preserve_residual_value_binders_without_local_captures() {
    let source = "struct Key where T, tag: T { value: T }
        comptime fn singleton(value: Key<T, tag>) -> Tuple<Key<T, tag>>
        where T, tag: T, .unused: T = tag
        {
            (value,)
        }
        comptime fn build() -> <|- bool> {
            let (yes,) = singleton(Key<bool, true> { value: true });
            let (no,) = singleton(Key<bool, false> { value: false });
            let (number,) = singleton(Key<u8, 7> { value: 7 });
            let result = yes.value and not no.value and number.value == 7;
            '(result)
        }
        fn run() -> bool { ~<build()> }";
    assert_eq!(sysroot::run(source), Value::Bool(true));
    let observing = "struct Flag where tag: bool {}
        comptime fn choose(flag: bool) -> bool { if flag { true } else { false } }
        comptime fn singleton(value: Flag<tag>) -> Tuple<Flag<choose(tag)>>
        where tag: bool
        {
            (value,)
        }";
    let failure = sysroot::compile(observing).unwrap_err();
    assert!(failure.contains("AbstractValueUnresolved"), "{failure}");
}

#[test]
fn nested_nominal_inputs_retain_abstract_values_and_formation_obligations() {
    let prefix = "
        pure comptime fn Selected<enabled: bool> -> Type { bool }
        pure comptime fn Wrap<T> -> Type { T }
        struct Flag where enabled: bool {}
        struct Checked where enabled: bool,
            requires Selected<enabled> == Selected<true> {}
        ";
    let package = sysroot::compile(&format!(
        "{prefix}
        fn packet(value: Flag<enabled>) -> Tuple<Flag<enabled>, bool>
        where enabled: bool
        {{ (value, enabled) }}
        fn forward(value: Checked<enabled>) -> Tuple<Checked<enabled>, bool>
        where enabled: bool, requires Selected<enabled> == Selected<true>
        {{ (value, enabled) }}
        typealias Accepted = Wrap<Checked<true>>;
        comptime fn Alias() -> Type {{ Accepted }}
        comptime fn Expression() -> Type {{ Wrap<Checked<true>> }}
        "
    ))
    .unwrap();
    assert_eq!(
        invoke_both(&package, "Alias"),
        invoke_both(&package, "Expression")
    );
    for source in [
        "typealias Bad = Wrap<Checked<false>>;",
        "comptime fn Bad() -> Type { Wrap<Checked<false>> }",
        "typealias Bad = Wrap<Flag<9>>;",
    ] {
        assert!(
            sysroot::compile(&format!("{prefix}{source}")).is_err(),
            "{source}"
        );
    }
}

#[test]
fn unobserved_invocations_keep_failure_and_dead_branch_behavior() {
    // A library callable is unavailable to static observation. Its error belongs
    // to invocation; observing an NF input must not execute that producer early.
    let package = sysroot::compile(
        "pure comptime fn Indexed<value: Integer> -> Type { bool }
        comptime fn Fail() -> Type { Indexed<meta.div_trunc(1, 0)> }
        comptime fn Skipped() -> Type {
            Indexed<if false { meta.div_trunc(1, 0) } else { 7 }>
        }
        comptime fn Reference() -> Type { Indexed<7> }",
    )
    .unwrap();
    for tree in [false, true] {
        sysroot::with_comptime(&package, sysroot::COMPTIME_BUDGET, tree, |vm| {
            let failure = vm.invoke(id(&package, "Fail"), vec![]).unwrap_err();
            assert_eq!(
                failure.kind,
                comptime_interpretor::ErrorKind::DivisionByZero
            );
            let actual = vm.invoke(id(&package, "Skipped"), vec![]).unwrap();
            assert_eq!(
                actual,
                vm.invoke(id(&package, "Reference"), vec![]).unwrap()
            );
        });
    }
}

#[test]
fn caller_generic_domains_contextualize_collected_optional_values() {
    let package = sysroot::compile(
        "pure comptime fn Packet<T, ...values: meta.List<?T>> -> Type { bool }
        comptime fn Direct(value: T) -> Type where T { Packet<T, value> }
        comptime fn Named(value: T) -> Type where T { Packet<.values = [value], T> }
        comptime fn Collected() -> Type { Direct<bool>(true) }
        comptime fn Reordered() -> Type { Named<bool>(true) }
        comptime fn Expected() -> Type { Packet<bool, true> }",
    )
    .unwrap();
    let expected = invoke_both(&package, "Expected");
    assert_eq!(invoke_both(&package, "Collected"), expected);
    assert_eq!(invoke_both(&package, "Reordered"), expected);
    assert_ordinary_reordered_inputs(&package, "Named");
}

#[test]
fn caller_generic_domains_contextualize_nominal_optional_inputs() {
    let package = sysroot::compile(
        "struct Stamp where T, .value: ?T = null { flag: bool }
        comptime fn Direct(value: T) -> Type where T { Stamp<T, .value = value> }
        comptime fn Named(value: T) -> Type where T { Stamp<.value = value, T> }
        comptime fn Positional() -> Type { Direct<bool>(true) }
        comptime fn Reordered() -> Type { Named<bool>(true) }
        comptime fn Expected() -> Type { Stamp<bool, .value = true> }",
    )
    .unwrap();
    let expected = invoke_both(&package, "Expected");
    assert_eq!(invoke_both(&package, "Positional"), expected);
    assert_eq!(invoke_both(&package, "Reordered"), expected);
}

#[test]
fn recursive_input_domains_substitute_caller_binders_once() {
    let package = sysroot::compile(
        "struct Stamp where T, .value: ?T = null { flag: bool }
        comptime fn Relay(stop: bool) -> Type
        where A, B, .left: ?A = null, .right: ?B = null
        {
            if stop { Stamp<A, .value = left> }
            else { Relay<B, .right = left, A, .left = right>(true) }
        }
        comptime fn Reordered() -> Type { Relay<bool, u8, .left = true, .right = 9>(false) }
        comptime fn Expected() -> Type { Stamp<u8, .value = 9> }",
    )
    .unwrap();
    assert_eq!(
        invoke_both(&package, "Reordered"),
        invoke_both(&package, "Expected")
    );
}

#[test]
fn supplied_generic_domains_do_not_authorize_unrelated_payloads() {
    for body in ["Stamp<T, .value = true>", "Stamp<.value = true, T>"] {
        let source = format!(
            "struct Stamp where T, .value: ?T = null {{ flag: bool }}
            comptime fn Invalid() -> Type where T {{ {body} }}"
        );
        let failure = sysroot::compile(&source)
            .err()
            .expect("unrelated payload must fail at definition checking");
        assert!(
            failure.contains("type mismatch") || failure.contains("optional operation"),
            "{source}: {failure}"
        );
    }
}
