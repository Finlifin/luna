//! Clause defaults wait for free declaration inputs, including captured inputs.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_interpretor::Value;

const DEFAULTS: &[&str] = &[
    "do { let capture = || T == bool; capture() }",
    "(|candidate: Type| candidate == T)(bool)",
    "do { pure comptime fn capture() -> bool { T == bool } capture() }",
    "do { pure comptime fn compare(candidate: Type) -> bool where U { candidate == U and U == T } compare<bool>(bool) }",
];

#[test]
fn ordinary_value_defaults_wait_for_captured_type_inference() {
    for default in DEFAULTS {
        let source = format!(
            "comptime fn choose(value: T) -> bool
            where T, .flag: bool = {default}, .opposite: bool = not flag
            {{ opposite }}
            const yes = choose(true);
            const no = choose('x');
            const override = choose<bool, .flag = false>(true);
            comptime fn run() -> bool {{ not yes and no and override }}"
        );
        let package =
            sysroot::compile(&source).unwrap_or_else(|error| panic!("{default}: {error}"));
        let run = package
            .functions
            .iter()
            .find(|function| function.name == "run")
            .unwrap()
            .declaration;
        for tree in [false, true] {
            let mut vm = if tree {
                comptime_interpretor::Interpreter::for_comptime_tree(
                    &package,
                    sysroot::COMPTIME_BUDGET,
                )
                .unwrap()
            } else {
                comptime_interpretor::Interpreter::for_comptime(&package, sysroot::COMPTIME_BUDGET)
                    .unwrap()
            };
            assert_eq!(
                vm.invoke(run, vec![]).unwrap(),
                Value::Bool(true),
                "{default}"
            );
        }
    }
}

#[test]
fn code_generator_defaults_wait_for_captured_type_inference() {
    for default in DEFAULTS {
        let source = format!(
            "comptime fn choose(value: T) -> < |- bool>
            where T, .flag: bool = {default}, .opposite: bool = not flag
            {{ '(opposite) }}
            fn run() -> bool {{
                const yes = choose(true);
                const no = choose('x');
                const override = choose<bool, .flag = false>(true);
                not ~<yes> and ~<no> and ~<override>
            }}"
        );
        assert_eq!(sysroot::run(&source), Value::Bool(true), "{default}");
    }
}

#[test]
fn explicit_overrides_do_not_hide_invalid_closure_defaults() {
    for generator in [false, true] {
        let (output, body, call) = if generator {
            (
                "< |- bool>",
                "'(flag)",
                "fn run() -> bool { const code = choose<bool, .flag = true>(true); ~<code> }",
            )
        } else {
            (
                "bool",
                "flag",
                "const value = choose<bool, .flag = true>(true);",
            )
        };
        let source = format!(
            "comptime fn choose(value: T) -> {output}
            where T, .flag: bool = do {{ let capture = || 'x'; capture() }}
            {{ {body} }}
            {call}"
        );
        let failure = sysroot::compile(&source)
            .err()
            .expect("invalid default accepted");
        assert!(failure.contains("type"), "{failure}");
    }
}

#[test]
fn named_clause_overrides_keep_caller_identity_and_infer_missing_slots() {
    let source = r#"
        fn inspect(x: T, witness: *Flag, done: bool) -> bool
        where T, .Flag: Type = bool
        {
            if done { true }
            else {
                let byte: u8 = 7;
                inspect<.Flag = T>(byte, x.ref, true)
            }
        }

        fn validate(value: T) -> bool
        where T, .Marker: Type = u32, .chosen: bool = do { let capture = || T == bool; capture() }
        {
            chosen
        }

        fn run() -> bool {
            let flag = true;
            inspect(flag, flag.ref, false)
                and validate<.Marker = u16>(flag)
                and not validate<.Marker = u16>('x')
                and validate<.chosen = true>('x')
        }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
    let invalid = source.replace("byte, x.ref, true", "byte, byte.ref, true");
    let error = sysroot::compile(&invalid).unwrap_err();
    assert!(
        error.contains("type constraint") || error.contains("type mismatch"),
        "{error}"
    );
}

#[test]
fn open_calls_infer_missing_clauses_alongside_explicit_overrides() {
    let source = r#"
        fn choose(value: T, comptime keep: bool) -> T
        where T, .Witness: Type = bool
        {
            inline if keep { value } else { value }
        }

        fn wrapper(value: T, comptime keep: bool) -> T where T {
            choose<.Witness = u16>(value, keep)
        }

        fn run() -> bool {
            let byte: u8 = 42;
            wrapper(byte, true) == byte and choose<.Witness = char>(byte, false) == byte
        }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn named_clause_literals_wait_for_their_inferred_domain() {
    let source = r#"
        fn present(value: T) -> bool
        where T, .tag: ?T = null, .flag: bool = tag matches _?
        {
            flag
        }

        fn tagged(value: T, comptime keep: bool) -> bool
        where T, .tag: ?T = null
        {
            inline if keep { tag matches _? } else { false }
        }

        fn run() -> bool {
            let byte: u8 = 1;
            not present<.tag = null>(byte)
                and present<.tag = 255>(byte)
                and not present<.tag = if true { null } else { 1 }>(byte)
                and tagged<.tag = 7>(byte, true)
        }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
    let invalid = source.replace("else { 1 }", "else { 256 }");
    let error = sysroot::compile(&invalid).unwrap_err();
    assert!(error.contains("does not fit"), "{error}");
}

#[test]
fn computed_clause_operands_keep_their_value_until_delivery() {
    let source = r#"
        enum TestErr { Missing }
        comptime fn make() -> u8 { 42 }
        comptime fn absent() -> ?u8 { null }
        comptime fn fail() -> !TestErr u8 { error TestErr.Missing }

        fn present(value: T) -> bool where T, .tag: ?T = null { tag matches _? }
        fn failed(value: T) -> bool
        where T, .out: !TestErr T = error TestErr.Missing
        {
            out matches error _
        }

        fn forward(value: U) -> bool
        where U, .outer: !TestErr U = error TestErr.Missing
        {
            failed<.out = outer>(value)
        }

        fn run() -> bool {
            let byte: u8 = 1;
            present<.tag = make()>(byte)
                and not present<.tag = absent()>(byte)
                and not failed<.out = make()>(byte)
                and failed<.out = fail()>(byte)
                and forward(byte)
                and not forward<.outer = make()>(byte)
        }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
    let error = sysroot::compile(&source.replace("make() -> u8", "make() -> u16")).unwrap_err();
    assert!(
        error.contains("expected (primitive u8), found (primitive u16)"),
        "{error}"
    );
}

#[test]
fn contextual_clause_control_runs_in_order_before_ordinary_type_inference() {
    let source = r#"
        comptime fn choose(name: Symbol, result: bool) -> bool {
            let generated = meta.StructBuilder.new(.name = name).Build();
            result
        }

        fn present(value: T) -> bool
        where T, .tag: ?T = null, .stamp: bool = false
        {
            stamp and (tag matches _?)
        }

        fn run() -> bool {
            let byte: u8 = 0;
            present<.tag = do {
                let flag = choose(.FirstSelector, false);
                flag match {
                    true => null,
                    false => when {
                        choose(.SecondSelector, true) => 255,
                        choose(.SkippedSelector, false) => 1,
                        else => null,
                    },
                }
            }, .stamp = choose(.FollowingInput, true)>(byte)
        }
    "#;
    let package = sysroot::compile(source).unwrap();
    let mut events: Vec<_> = package
        .structs
        .iter()
        .filter_map(|definition| {
            let name = definition.name.as_str();
            matches!(
                name,
                "FirstSelector" | "SecondSelector" | "SkippedSelector" | "FollowingInput"
            )
            .then_some((definition.declaration, name))
        })
        .collect();
    events.sort_by_key(|(id, _)| *id);
    assert_eq!(
        events.iter().map(|(_, name)| *name).collect::<Vec<_>>(),
        ["FirstSelector", "SecondSelector", "FollowingInput"]
    );
    let graph = luna::instantiation::instantiate(&package, "run", 1_000_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
    let wrong = source.replace("true => null", "true => 256");
    assert!(
        sysroot::compile(&wrong)
            .unwrap_err()
            .contains("does not fit")
    );
}

#[test]
fn runtime_wrappers_specialize_clause_operands_from_staged_parameters() {
    let source = r#"
        fn outer(value: u8, comptime flag: bool) -> bool { nested(value, flag) }
        fn nested(value: u8, comptime flag: bool) -> bool {
            let alias = flag;
            present<.tag = if choose(alias) { null } else { 42 }>(value)
        }
        fn explicit(value: u8, comptime flag: bool) -> bool {
            present<u8, .tag = if choose(flag) { null } else { 42 }>(value)
        }
        comptime fn choose(value: bool) -> bool { value }
        fn present(value: T) -> bool where T, .tag: ?T = null { tag matches _? }
        fn run() -> bool {
            let byte: u8 = 0;
            not outer(byte, true) and outer(byte, false)
                and not explicit(byte, true) and explicit(byte, false)
        }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
    for (from, to, diagnostic) in [
        ("choose(alias)", "choose(value == 0)", "runtime"),
        ("else { 42 }", "else { 256 }", "does not fit"),
    ] {
        let error = sysroot::compile(&source.replace(from, to)).unwrap_err();
        assert!(error.contains(diagnostic), "{error}");
    }
}

#[test]
fn a_returned_specialization_retains_its_own_clause_inputs() {
    let source = r#"
        fn present(value: T) -> bool where T, .tag: ?T = null { tag matches _? }
        comptime fn choose(flag: bool) -> bool { flag }
        fn select(comptime flag: bool) -> fn(value: u8) -> bool {
            present<u8, .tag = if choose(flag) { null } else { 42 }>
        }
        fn run() -> bool {
            let byte: u8 = 0;
            let empty = select(true);
            let full = select(false);
            not empty(byte) and full(byte) and not empty(byte)
        }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn mixed_clause_results_keep_lexical_values_and_check_unselected_producers() {
    let source = r#"
        comptime fn event(name: Symbol, value: u8) -> u8 {
            let generated = meta.StructBuilder.new(.name = name).Build();
            value
        }
        fn check(value: T) -> ?T where T, .tag: ?T = null, .stamp: u8 = 0 {
            if stamp == 9 { tag } else { null }
        }
        fn run() -> bool {
            let byte: u8 = 0;
            let result = check<.tag = do {
                let value = event(.BeforeMatch, 42);
                value match {
                    0 => null,
                    1 => event(.Skipped, value),
                    payload => event(.Selected, payload),
                }
            }, .stamp = event(.AfterMatch, 9)>(byte);
            result matches 42?
        }
    "#;
    let package = sysroot::compile(source).unwrap();
    let mut events: Vec<_> = package
        .structs
        .iter()
        .filter_map(|def| {
            let name = def.name.as_str();
            matches!(name, "BeforeMatch" | "Skipped" | "Selected" | "AfterMatch")
                .then_some((def.declaration, name))
        })
        .collect();
    events.sort_unstable();
    assert_eq!(
        events.iter().map(|(_, name)| *name).collect::<Vec<_>>(),
        ["BeforeMatch", "Selected", "AfterMatch"]
    );
    let graph = luna::instantiation::instantiate(&package, "run", 1_000_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
    let bad = source.replace("event(.Skipped, value)", "event(.Skipped, value) == 1");
    let error = sysroot::compile(&bad).unwrap_err();
    assert!(error.contains("bool") && error.contains("u8"), "{error}");
}

#[test]
fn mixed_clause_results_distinguish_absence_from_an_absent_payload() {
    let source = r#"
        comptime fn choose(value: bool) -> bool { value }
        comptime fn absent() -> ?u8 { null }
        fn read(value: T) -> ?T where T, .tag: ?T = null { tag }
        fn staged(value: ?u8, comptime flag: bool) -> ??u8 {
            read<.tag = if choose(flag) { null } else { absent() }>(value)
        }
        fn run() -> bool {
            let witness: ?u8 = null;
            let outer = staged(witness, true);
            let inner = staged(witness, false);
            (outer matches null) and (inner matches null?)
        }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn computed_clause_delivery_preserves_error_tags_and_optional_success() {
    let source = r#"
        enum ReadErr { Missing }
        comptime fn failed() -> !ReadErr ?u8 { error .Missing }
        comptime fn byte() -> u8 { 42 }
        fn read(value: T) -> !ReadErr ?T where T, .tag: !ReadErr ?T = null { tag }
        fn staged(value: u8, comptime branch: u8) -> !ReadErr ?u8 {
            read<.tag = branch match {
                0 => null,
                1 => failed(),
                _ => byte(),
            }>(value)
        }
        fn run() -> bool {
            let value: u8 = 0;
            (staged(value, 0) matches null!) and (staged(value, 1) matches error .Missing)
                and (staged(value, 2) matches 42?!)
        }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn mixed_aggregate_clause_stages_lexical_fields_before_following_inputs() {
    let source = r#"
        comptime fn event(name: Symbol, value: u8) -> u8 {
            let generated = meta.StructBuilder.new(.name = name).Build();
            value
        }
        fn keep(value: T) -> ?T where T, .tag: ?T = null, .stamp: u8 = 0 {
            if stamp == 9 { tag } else { null }
        }
        fn run() -> bool {
            let witness: Tuple<?u8, u8> = (null, 0);
            let chosen = keep<.tag = if event(.Outer, 1) == 1 {
                (if event(.Guard, 0) == 0 { null } else { event(.Skipped, 3) },
                 do { let a = event(.Chosen, 42); a })
            } else { null }, .stamp = event(.Following, 9)>(witness);
            chosen matches (null, 42)?
        }
    "#;
    let package = sysroot::compile(source).unwrap();
    let mut events: Vec<_> = package
        .structs
        .iter()
        .filter_map(|def| {
            let name = def.name.as_str();
            matches!(name, "Outer" | "Guard" | "Skipped" | "Chosen" | "Following")
                .then_some((def.declaration, name))
        })
        .collect();
    events.sort_unstable();
    assert_eq!(
        events.into_iter().map(|(_, name)| name).collect::<Vec<_>>(),
        ["Outer", "Guard", "Chosen", "Following"]
    );
    let graph = luna::instantiation::instantiate(&package, "run", 1_000_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );

    // The other branch delivers a u8 to the optional first field. Reducing
    // the selected path must retain the field context inside the outer tag.
    let present = source
        .replace("event(.Guard, 0)", "event(.Guard, 1)")
        .replace("(null, 42)?", "(3?, 42)?");
    assert_eq!(sysroot::run(&present), Value::Bool(true));

    // Check a skipped field both when its containing tuple is constructed
    // and when control skips the entire tuple.
    let wrong = source.replace("event(.Skipped, 3)", "event(.Skipped, 3) == 3");
    for invalid in [
        wrong.clone(),
        wrong.replace("event(.Outer, 1)", "event(.Outer, 0)"),
    ] {
        let error = sysroot::compile(&invalid).unwrap_err();
        assert!(error.contains("bool") && error.contains("u8"), "{error}");
    }
}

#[test]
fn contextual_lists_use_the_inferred_element_domain_for_each_staged_field() {
    let source = r#"
        comptime fn byte() -> u8 { 42 }
        fn check(value: T) -> bool where T, .values: meta.List<?T> = [] {
            const ok = values.length() == 2 and (values[0] matches null)
                and (values[1] matches _?);
            ok
        }
        fn run() -> bool {
            let byte_witness: u8 = 0;
            let pair_witness: Tuple<?u8, u8> = (null, 0);
            check<.values = [null, byte()]>(byte_witness) and
                check<.values = [null, (null, byte())]>(pair_witness)
        }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
    let wrong = source.replace("(null, byte())]", "(null, byte() == 0)]");
    let error = sysroot::compile(&wrong).unwrap_err();
    assert!(error.contains("bool") && error.contains("u8"), "{error}");
}

#[test]
fn computed_fields_receive_payload_context_through_error_and_optional_tags() {
    let source = r#"
        enum ReadErr { Missing }
        comptime fn byte() -> u8 { 42 }
        fn keep(witness: T) -> !ReadErr ?T where T, .value: !ReadErr ?T = null { value }
        fn run() -> bool {
            let witness: Tuple<?u8, u8> = (null, 0);
            let inferred = keep<.value = (byte(), byte())>(witness);
            let explicit = keep<Tuple<?u8, u8>, .value = (byte(), byte())>(witness);
            (inferred matches (42?, 42)?!) and (explicit matches (42?, 42)?!)
        }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn enum_clause_payloads_wait_for_their_namespace_and_preserve_source_effects() {
    let source = r#"
        enum Packet {
            waiting,
            ready(u8),
            pair(u8, u8),
            record { first: u8, second: u8, defaulted: u8 = 7 },
        }
        comptime fn event(name: Symbol, value: u8) -> u8 {
            let generated = meta.StructBuilder.new(.name = name).Build();
            value
        }
        comptime fn pair() -> Tuple<u8, u8> { (event(.Spread, 1), 2) }
        fn keep(witness: T) -> ?T where T, .tag: ?T = null, .stamp: u8 = 0 {
            if stamp == 9 { tag } else { null }
        }
        fn run() -> bool {
            let witness = Packet.waiting;
            let record = keep<.tag = .record {
                second: event(.Second, 2), first: event(.First, 1)
            }, .stamp = event(.AfterRecord, 9)>(witness);
            let another = Packet.waiting;
            let value = keep<.tag = if event(.Guard, 1) == 1 {
                .pair(...pair())
            } else { .ready(event(.Skipped, 0)) }, .stamp = event(.AfterPair, 9)>(another);
            (record matches Packet.record { first: 1, second: 2, defaulted: 7 }?) and
                (value matches Packet.pair(1, 2)?)
        }
    "#;
    for program in [
        source.to_string(),
        source.replace("keep<.tag", "keep<Packet, .tag"),
    ] {
        let package = sysroot::compile(&program).unwrap();
        let mut events: Vec<_> = package
            .structs
            .iter()
            .filter_map(|item| {
                let name = item.name.as_str();
                matches!(
                    name,
                    "Second"
                        | "First"
                        | "AfterRecord"
                        | "Guard"
                        | "Spread"
                        | "Skipped"
                        | "AfterPair"
                )
                .then_some((item.declaration, name))
            })
            .collect();
        events.sort_unstable();
        assert_eq!(
            events.into_iter().map(|(_, name)| name).collect::<Vec<_>>(),
            [
                "Second",
                "First",
                "AfterRecord",
                "Guard",
                "Spread",
                "AfterPair"
            ]
        );
        let graph = luna::instantiation::instantiate(&package, "run", 1_000_000).unwrap();
        let mut vm =
            comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
        assert_eq!(
            vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
            Value::Bool(true)
        );
    }
    let wrong = source.replace(".ready(event(.Skipped, 0))", ".ready(false)");
    let error = sysroot::compile(&wrong).unwrap_err();
    assert!(error.contains("u8") && error.contains("bool"), "{error}");
}

#[test]
fn list_elements_resolve_enum_constructors_from_the_inferred_domain() {
    let source = r#"
        enum State { ready(u8), waiting }
        comptime fn byte() -> u8 { 42 }
        fn check(witness: T) -> T where T, .values: meta.List<T> = [] {
            const first = values[0];
            first
        }
        fn run() -> bool {
            let witness = State.waiting;
            check<.values = [.ready(byte())]>(witness) matches State.ready(42)
        }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn ordinary_type_evidence_selects_the_clause_namespace_before_invocation() {
    let source = r#"
        enum Packet { group.{byte(u8)}, waiting }
        fn first(witness: T) -> T where T, .items: meta.List<T> = [] {
            const value = items[0];
            value
        }
        fn nested(witness: *T) -> T where T, .items: meta.List<T> = [] {
            const value = items[0];
            value
        }
        struct Slot where T { value: T }
        fn slot(witness: T) -> Slot<T> where T, .items: meta.List<T> = [] {
            const value = items[0];
            Slot<T> { value: value }
        }
        fn independent(left: T, right: U) -> T where T, U, .items: meta.List<T> = [] {
            const value = items[0];
            value
        }
        fn run() -> bool {
            let empty: ?u8 = null;
            let byte = first<.items = [.group.byte(0)]>(empty);
            let selected = first<.items = [.group.byte(0)]>(Packet.waiting);
            let packet = Packet.waiting;
            let other = nested<.items = [.group.byte(7)]>(packet.ref);
            let another = independent<.items = [.group.byte(9)]>(Packet.group.byte(4), true);
            let wrapped: Slot<?Packet> = slot<.items = [.group.byte(5)]>(null);
            let lifted: Slot<?Packet> = slot<.items = [.group.byte(6)]>(Packet.waiting);
            (wrapped.value matches .group.byte(5)?) and (lifted.value matches .group.byte(6)?)
                and (another matches .group.byte(9)) and (byte matches 103?) and (selected matches .group.byte(0))
                and (other matches .group.byte(7))
        }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
    let invalid = source.replace("Packet.group.byte(4)", "Packet.group.byte(false)");
    let error = sysroot::compile(&invalid).unwrap_err();
    assert!(error.contains("u8") && error.contains("bool"), "{error}");
}

#[test]
fn clause_evidence_reads_signatures_without_invoking_an_ordinary_producer() {
    let source = r#"
        enum Packet { group.{record_mark(u8)}, waiting }
        comptime fn event(name: Symbol, value: u8) -> u8 {
            let generated = meta.StructBuilder.new(.name = name).Build();
            value
        }
        pure comptime trait MarkSymbol { comptime fn record_mark(self, value: u8) -> ?u8; }
        comptime impl MarkSymbol for Symbol {
            comptime fn record_mark(self, value: u8) -> ?u8 { event(.EvidenceMethod, value) }
        }
        comptime fn absent() -> ?u8 { null }
        comptime fn witness() -> Packet {
            let mark = event(.EvidenceWitness, 0);
            Packet.waiting
        }
        fn take(comptime w: T) -> T
        where T, .items: meta.List<T> = [], .stamp: u8 = 0
        {
            const first = items[0];
            first
        }
        fn from_result() -> T where T, .items: meta.List<T> = [] {
            const first = items[0];
            first
        }
        fn run() -> bool {
            let value = take<.stamp = event(.EvidenceStamp, 1),
                .items = [.group.record_mark(event(.EvidenceField, 7))]>(witness());
            let other: Packet = from_result<.items = [.group.record_mark(8)]>();
            let ordinary = take<.items = [.group.record_mark(event(.EvidenceArgument, 4))],
                .stamp = event(.EvidenceFollowing, 0)>(absent());
            (ordinary matches 4?) and (value matches .group.record_mark(7)) and (other matches .group.record_mark(8))
        }
    "#;
    let package = sysroot::compile(source).unwrap();
    let mut events: Vec<_> = package
        .structs
        .iter()
        .filter(|item| item.name.starts_with("Evidence"))
        .map(|item| (item.declaration, item.name.as_str()))
        .collect();
    events.sort_unstable();
    assert_eq!(
        events.into_iter().map(|(_, name)| name).collect::<Vec<_>>(),
        [
            "EvidenceStamp",
            "EvidenceField",
            "EvidenceWitness",
            "EvidenceArgument",
            "EvidenceMethod",
            "EvidenceFollowing"
        ]
    );
    let graph = luna::instantiation::instantiate(&package, "run", 1_000_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn later_type_suppliers_contextualize_earlier_clauses_without_running_early() {
    let source = r#"
        enum Packet { group.{byte(u8)} }
        typealias Alias = Packet;
        comptime fn mark(name: Symbol, value: u8) -> u8 {
            let generated = meta.StructBuilder.new(.name = name).Build();
            value
        }
        fn read() -> T where .T: Type = bool, .items: meta.List<T> = [], .stamp: u8 = 0 {
            const item = items[0];
            item
        }
        fn positional() -> T where T, .items: meta.List<T> = [] {
            const item = items[0];
            item
        }
        fn run() -> bool {
            let selected = read<.items = [.group.byte(mark(.ForwardField, 7))],
                .stamp = mark(.ForwardStamp, 0),
                .T = if mark(.ForwardGuard, 1) == 1 { Packet } else { Alias }>();
            let other = positional<.items = [.group.byte(9)], ...(Packet,)>();
            let ordinary = read<.items = [.group.byte(0)], .T = ?u8>();
            (selected matches .group.byte(7)) and (other matches .group.byte(9))
                and (ordinary matches 103?)
        }
    "#;
    check_struct_events(
        source,
        "Forward",
        &["ForwardField", "ForwardStamp", "ForwardGuard"],
    );
    let invalid = source.replace("mark(.ForwardGuard, 1) == 1", "mark(.ForwardGuard, 1)");
    assert!(sysroot::compile(&invalid).is_err());
}

fn check_struct_events(source: &str, prefix: &str, expected: &[&str]) {
    let package = sysroot::compile(source).unwrap();
    let mut events: Vec<_> = package
        .structs
        .iter()
        .filter(|item| item.name.starts_with(prefix))
        .map(|item| (item.declaration, item.name.as_str()))
        .collect();
    events.sort_unstable();
    assert_eq!(
        events.into_iter().map(|(_, name)| name).collect::<Vec<_>>(),
        expected
    );
    let graph = luna::instantiation::instantiate(&package, "run", 1_000_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn selected_type_defaults_contextualize_inputs_before_default_evaluation() {
    let source = r#"
        enum Packet { group.{byte(u8)} }

        comptime fn mark(name: Symbol, value: u8) -> u8 {
            let generated = meta.StructBuilder.new(.name = name).Build();
            value
        }
        fn read() -> T
        where .A: Type = Packet,
            .T: Type = if mark(.DefaultGuard, 1) == 1 { A } else { A },
            .items: meta.List<T> = [], .stamp: u8 = 0
        {
            const item = items[0];
            item
        }
        fn chained() -> T where .A: Type = bool, .B: Type = A,
            .T: Type = B, .items: meta.List<T> = []
        {
            const item = items[0];
            item
        }
        fn run() -> bool {
            let first = read<.items = [.group.byte(mark(.DefaultField, 7))],
                .stamp = mark(.DefaultStamp, 0)>();
            let ordinary = read<.items = [.group.byte(0)], .T = ?u8>();
            let last = chained<.items = [.group.byte(9)], .A = Packet>();
            (first matches .group.byte(7)) and (ordinary matches 103?)
                and (last matches .group.byte(9))
        }
    "#;
    check_struct_events(
        source,
        "Default",
        &["DefaultField", "DefaultStamp", "DefaultGuard"],
    );
    let failing = source.replace(
        "let generated =",
        "assumes name != .DefaultGuard; let generated =",
    );
    let error = sysroot::compile(&failing).unwrap_err();
    assert!(error.contains("ProgramContractRejected"), "{error}");
}

#[test]
fn computed_type_suppliers_select_context_without_replaying_call_operands() {
    let source = r#"
        enum Packet { group.{byte(u8)} }
        pure comptime fn Choose(flag: bool, stamp: u8, .yes: Type = bool, .no: Type = bool) -> Type {
            if flag { yes } else { no }
        }
        pure comptime fn Relay(flag: bool, stamp: u8) -> Type {
            Choose(flag, stamp, .no = ?u8, .yes = Packet)
        }
        pure comptime fn Fixed(stamp: u8) -> Type { Packet }
        pure comptime fn Escape(flag: bool) -> Type {
            Fixed(do { if flag { return ?u8; }; 0 })
        }
        pure comptime fn InGuard(flag: bool) -> Type {
            if do { if flag { return ?u8; }; true } { Packet } else { Packet }
        }
        comptime fn mark(name: Symbol, value: u8) -> u8 {
            let generated = meta.StructBuilder.new(.name = name).Build();
            value
        }
        fn read() -> T where .T: Type = bool, .items: meta.List<T> = [], .stamp: u8 = 0 {
            const item = items[0];
            item
        }
        fn run() -> bool {
            let a = read<.items = [.group.byte(mark(.CallField, 7))], .stamp = mark(.CallMiddle, 1),
                .T = Relay(true, mark(.CallSupplier, 2))>();
            let b = read<.items = [.group.byte(0)], .T = Relay(false, 3)>();
            let c = read<.items = [.group.byte(0)], .T = Escape(true)>();
            let d = read<.items = [.group.byte(0)], .T = InGuard(true)>();
            (a matches .group.byte(7)) and (b matches 103?) and (c matches 103?) and (d matches 103?)
        }
    "#;
    check_struct_events(source, "Call", &["CallField", "CallMiddle", "CallSupplier"]);
    let invalid = source.replace(
        "let generated =",
        "assumes name != .CallSupplier; let generated =",
    );
    let error = sysroot::compile(&invalid).unwrap_err();
    assert!(error.contains("ProgramContractRejected"), "{error}");
}

#[test]
fn type_producer_defaults_observe_only_the_selected_declaration_prefix() {
    let source = r#"
        enum Packet { group.{byte(u8)} }
        pure comptime fn Select(seed: Type, .first: Type = seed, .picked: Type = first) -> Type {
            picked
        }
        pure comptime fn Checked(flag: bool) -> Type {
            assumes flag;
            Packet
        }
        comptime fn Opaque() -> Type { ?u8 }
        pure comptime fn Override(.picked: Type = Checked(false)) -> Type { picked }
        fn read() -> T where .T: Type = bool, .items: meta.List<T> = [] {
            const item = items[0];
            item
        }
        fn run() -> bool {
            let first = read<.items = [.group.byte(7)], .T = Select(Packet)>();
            let second = read<.items = [.group.byte(0)], .T = Select(Packet, .first = ?u8)>();
            let third = read<.items = [.group.byte(3)], .T = Override(.picked = Packet)>();
            let last = read<.items = [.group.byte(0)], .T = Select(Packet, .first = Opaque())>();
            (first matches .group.byte(7)) and (second matches 103?)
                and (third matches .group.byte(3)) and (last matches 103?)
        }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
    let invalid = source.replace("Override(.picked = Packet)", "Override()");
    let error = sysroot::compile(&invalid).unwrap_err();
    assert!(error.contains("ProgramContractRejected"), "{error}");
}

#[test]
fn lexical_returns_and_fallthrough_supply_the_same_type_evidence() {
    let source = r#"
        enum Packet { group.{byte(u8)} }
        comptime fn note(name: Symbol, value: bool) -> bool {
            let generated = meta.StructBuilder.new(.name = name).Build();
            value
        }
        pure comptime fn Choose(flag: bool, yes: Type, no: Type) -> Type {
            if flag { return yes; };
            return no;
        }
        pure comptime fn Agrees(flag: bool) -> Type {
            when {
                flag => { return Packet; },
                else => {},
            };
            Packet
        }
        pure comptime fn InGuard(flag: bool) -> Type {
            if do { if flag { return ?u8; }; true } { return Packet; };
            ?u8
        }
        pure comptime fn Short(flag: bool) -> Type {
            if not flag or do { return Packet; false } { return ?u8; };
            Packet
        }
        fn read() -> T where .T: Type = bool, .items: meta.List<T> = [] {
            const item = items[0];
            item
        }
        fn run() -> bool {
            let a = read<.items = [.group.byte(7)], .T = Choose(true, Packet, ?u8)>();
            let b = read<.items = [.group.byte(0)], .T = Choose(false, Packet, ?u8)>();
            let c = read<.items = [.group.byte(3)], .T = Agrees(note(.TypeExitMerge, false))>();
            let d = read<.items = [.group.byte(5)], .T = InGuard(false)>();
            let e = read<.items = [.group.byte(0)], .T = InGuard(true)>();
            let f = read<.items = [.group.byte(0)], .T = Choose(note(.TypeExitSplit, false), Packet, ?u8)>();
            let g = read<.items = [.group.byte(9)], .T = Short(true)>();
            let h = read<.items = [.group.byte(0)], .T = Short(false)>();
            (g matches .group.byte(9)) and (h matches 103?) and (a matches .group.byte(7)) and (b matches 103?) and (c matches .group.byte(3))
                and (d matches .group.byte(5)) and (e matches 103?) and (f matches 103?)
        }
    "#;
    check_struct_events(source, "TypeExit", &["TypeExitMerge", "TypeExitSplit"]);
}

#[test]
fn returns_in_call_operands_target_the_caller_in_written_order() {
    let source = r#"
        enum Packet { group.{byte(u8)} }
        pure comptime fn Ignore(.first: bool = true, .second: bool = true) -> Type {
            return ?u8;
        }
        pure comptime fn Select(first: bool, second: bool) -> Type {
            Ignore(.second = do { if second { return Packet; }; false },
                   .first = do { if first { return ?u8; }; true })
        }
        pure comptime fn Discard(flag: bool) -> Type {
            Ignore(.first = do { if flag { return ?u8; }; true });
            Packet
        }
        pure comptime fn Agrees(flag: bool) -> Type {
            Ignore(.second = do { if flag { return Packet; }; false },
                   .first = do { return Packet; false })
        }
        pure comptime fn Byte(value: bool) -> u8 { 0 }
        pure comptime fn BeforeDelivery() -> Type { Byte(do { return Packet; true }) }
        pure comptime fn Group(...items: Tuple<bool, bool>, .named: bool = false) -> Type { ?u8 }
        pure comptime fn Mixed(flag: bool) -> Type {
            Group(false, .named = do { if flag { return Packet; }; true }, do { return ?u8; false })
        }
        comptime fn flag() -> bool { false }
        fn pick() -> T where .T: Type = bool, .items: meta.List<T> = [] {
            const item = items[0];
            item
        }
        fn run() -> bool {
            let a = pick<.items = [.group.byte(9)], .T = Select(true, true)>();
            let b = pick<.items = [.group.byte(0)], .T = Select(true, false)>();
            let c = pick<.items = [.group.byte(3)], .T = Discard(false)>();
            let d = pick<.items = [.group.byte(0)], .T = Discard(true)>();
            let e = pick<.items = [.group.byte(5)], .T = Agrees(flag())>();
            let f = pick<.items = [.group.byte(4)], .T = BeforeDelivery()>();
            let g = pick<.items = [.group.byte(6)], .T = Mixed(true)>();
            let h = pick<.items = [.group.byte(0)], .T = Mixed(false)>();
            (f matches .group.byte(4)) and (g matches .group.byte(6)) and (h matches 103?)
                and (a matches .group.byte(9)) and (b matches 103?) and (c matches .group.byte(3))
                and (d matches 103?) and (e matches .group.byte(5))
        }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
}
