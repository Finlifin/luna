//! NF inputs share contextual checking while retaining source evaluation slots.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_interpretor::{Budget, Event, Interpreter, Value};
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
    let mut vm = Interpreter::for_comptime(package, Budget::default()).unwrap();
    let actual = vm.invoke(id(package, name), vec![]).unwrap();
    let mut tree = Interpreter::for_comptime_tree(package, Budget::default()).unwrap();
    assert_eq!(tree.invoke(id(package, name), vec![]).unwrap(), actual);
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
        let mut vm = if tree {
            Interpreter::for_comptime_tree(&package, Budget::default())
        } else {
            Interpreter::for_comptime(&package, Budget::default())
        }
        .unwrap();
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
    for tree in [false, true] {
        let mut vm = if tree {
            Interpreter::for_comptime_tree(package, Budget::default())
        } else {
            Interpreter::for_comptime(package, Budget::default())
        }
        .unwrap();
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
        let mut vm = if tree {
            Interpreter::for_comptime_tree(&package, Budget::default())
        } else {
            Interpreter::for_comptime(&package, Budget::default())
        }
        .unwrap();
        assert!(vm.invoke(id(&package, "Wrong"), vec![]).is_err());
        assert_eq!(vm.invoke(id(&package, "Branch"), vec![]).unwrap(), direct);
    }
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
    trace_both(&package, "Build", &direct, &calls);
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
