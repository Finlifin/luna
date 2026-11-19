//! Type observations must resume the original source execution, preserving its
//! earlier writes, captured values and nonlocal control exits.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;

use comptime_interpretor::{Event, Value};

fn check_twice(source: &str, expected_calls: &[&str]) {
    let package = sysroot::compile(source).unwrap_or_else(|failure| panic!("{failure}\n{source}"));
    let entry = package
        .functions
        .iter()
        .find(|function| function.name == "check")
        .unwrap()
        .declaration;
    for reference in [false, true] {
        sysroot::with_comptime(&package, sysroot::COMPTIME_BUDGET, reference, |vm| {
            for _ in 0..2 {
                let start = vm.events().len();
                assert_eq!(
                    vm.invoke(entry, vec![]).unwrap(),
                    Value::Bool(true),
                    "reference={reference}"
                );
                let calls = vm.events()[start..]
                    .iter()
                    .filter_map(|event| match event {
                        Event::EnterFunction(id) if id.package == entry.package => package
                            .functions
                            .iter()
                            .find(|function| function.declaration == *id)
                            .map(|function| function.name.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                assert_eq!(calls, expected_calls, "reference={reference}");
            }
        });
    }
}

fn ordered_collector(type_producer: &str) -> String {
    format!(
        "comptime fn Byte() -> Type {{ u8 }}
         comptime fn count() -> Integer
         where T, ...values: meta.List<T>
         {{ values.length() }}
         comptime fn check() -> bool {{
             var trace: Integer = 0;
             let size = count<
                 .values = do {{ trace = trace * 10 + 1; [1, 255] }},
                 do {{ trace = trace * 10 + 2; {type_producer} }}
             >();
             size == 2 and trace == 12
         }}"
    )
}

#[test]
fn observed_type_identity_keeps_collector_prefix_execution_in_place() {
    check_twice(&ordered_collector("u8"), &["check", "count"]);
}

#[test]
#[ignore = "source checking must resume the collector after its Type producer completes"]
fn computed_type_wait_preserves_earlier_collector_prefix_and_source_order() {
    check_twice(&ordered_collector("Byte()"), &["check", "Byte", "count"]);
}

fn exiting_collector(type_producer: &str) -> String {
    format!(
        "comptime fn Byte() -> Type {{ u8 }}
         comptime fn mark() -> bool {{ true }}
         comptime fn count() -> Integer
         where T, ...values: meta.List<T>
         {{ values.length() }}
         comptime fn check() -> bool {{
             let unused = count<
                 .values = do {{ return true; [1, 255] }},
                 do {{ mark(); {type_producer} }}
             >();
             false
         }}"
    )
}

#[test]
fn observed_type_identity_does_not_execute_inputs_after_return() {
    check_twice(&exiting_collector("u8"), &["check"]);
}

#[test]
#[ignore = "pending source checks must preserve return before the later Type producer"]
fn computed_type_wait_cannot_execute_later_producer_after_return() {
    check_twice(&exiting_collector("Byte()"), &["check"]);
}

fn captured_collector(type_producer: &str) -> String {
    format!(
        "comptime fn Byte() -> Type {{ u8 }}
         comptime fn first() -> T
         where T, ...values: meta.List<T>
         {{ values[0] }}
         comptime fn check() -> bool {{
             var byte: u8 = 7;
             let result = first<
                 .values = [byte],
                 do {{ byte = 9; {type_producer} }}
             >();
             result == 7 and byte == 9
         }}"
    )
}

#[test]
fn observed_type_identity_keeps_earlier_collected_value_snapshot() {
    check_twice(&captured_collector("u8"), &["check", "first"]);
}

#[test]
#[ignore = "pending source domains and callable signatures need the original execution continuation"]
fn computed_type_wait_preserves_earlier_value_before_later_write() {
    check_twice(&captured_collector("Byte()"), &["check", "Byte", "first"]);
}

#[test]
fn checked_packet_and_computed_type_share_the_original_invocation() {
    check_twice(
        "comptime fn Byte() -> Type { u8 }
         comptime fn count() -> Integer
         where T, ...values: meta.List<T>
         { values.length() }
         comptime fn check() -> bool {
             var trace: Integer = 0;
             let size = count<
                 .values = do {
                     trace = trace * 10 + 1;
                     let packet: meta.List<u8> = [1, 255];
                     packet
                 },
                 do { trace = trace * 10 + 2; Byte() }
             >();
             size == 2 and trace == 12
         }",
        &["check", "Byte", "count"],
    );
}

#[test]
fn checked_packet_keeps_earlier_snapshot_across_computed_type_input() {
    check_twice(
        "comptime fn Byte() -> Type { u8 }
         comptime fn first_is_seven() -> bool
         where T, ...values: meta.List<T>, requires T == u8
         { values[0] == 7 }
         comptime fn check() -> bool {
             var byte: u8 = 7;
             let result = first_is_seven<
                 .values = do { let packet: meta.List<u8> = [byte]; packet },
                 do { byte = 9; Byte() }
             >();
             result and byte == 9
         }",
        &["check", "Byte", "first_is_seven"],
    );
}

#[test]
fn completed_input_domains_reject_before_entry_and_recover_in_the_same_vm() {
    let package = sysroot::compile(
        "comptime fn Selected(flag: bool) -> Type { if flag { u8 } else { bool } }
         comptime fn packet() -> meta.List<u8> { [1, 255] }
         comptime fn count() -> Integer
         where T, ...values: meta.List<T>
         { values.length() }
         comptime fn check(flag: bool) -> bool {
             count<.values = packet(), Selected(flag)>() == 2
         }",
    )
    .unwrap();
    let id = |name: &str| {
        package
            .functions
            .iter()
            .find(|function| function.name == name)
            .unwrap()
            .declaration
    };
    for reference in [false, true] {
        sysroot::with_comptime(&package, sysroot::COMPTIME_BUDGET, reference, |vm| {
            for valid in [false, true, false, true] {
                let start = vm.events().len();
                let result = vm.invoke(id("check"), vec![Value::Bool(valid)]);
                if valid {
                    assert_eq!(result.unwrap(), Value::Bool(true), "reference={reference}");
                } else {
                    let error = result.unwrap_err();
                    assert!(
                        matches!(
                            error.kind,
                            comptime_interpretor::ErrorKind::FunctionInputRejected(_)
                        ),
                        "reference={reference}: {error:?}"
                    );
                }
                let entered = vm.events()[start..]
                    .iter()
                    .filter_map(|event| match event {
                        Event::EnterFunction(entered) if entered.package == id("check").package => {
                            Some(*entered)
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                let mut expected = vec![id("check"), id("packet"), id("Selected")];
                if valid {
                    expected.push(id("count"));
                }
                assert_eq!(entered, expected, "reference={reference}, valid={valid}");
            }
        });
    }
}

#[test]
fn pending_type_inputs_do_not_hide_known_domain_conflicts_in_unused_code() {
    for (domain, value) in [("u8", "true"), ("meta.List<T>", "true")] {
        let source = format!(
            "comptime fn Byte() -> Type {{ u8 }}
             comptime fn consume() -> bool
             where T, value: {domain}
             {{ true }}
             comptime fn unused() -> bool {{ consume<Byte(), {value}>() }}"
        );
        let failure = sysroot::compile(&source).unwrap_err();
        assert!(
            failure.contains("type mismatch"),
            "domain={domain}: {failure}"
        );
    }
}

#[test]
fn pending_domains_share_one_substitution_across_the_entire_call() {
    // Domain would fail at execution. Definition checking can already decide
    // whether both operands could have the same type without running it.
    let source = "
        comptime fn Domain() -> Type { assumes false; u8 }
        comptime fn consume() -> bool where T, first: T, second: T { true }
        comptime fn unused(byte: u8) -> bool { consume<Domain(), true, byte>() }
    ";
    let failure = sysroot::compile(source).unwrap_err();
    assert!(failure.contains("no common substitution"), "{failure}");
    sysroot::compile(&source.replace("true, byte>", "byte, byte>")).unwrap();

    // A second declaration slot gets its own identity, even for two uses of
    // the same Type-producing function.
    let independent = source
        .replace("T, first: T, second: T", "T, U, first: T, second: U")
        .replace("consume<Domain(),", "consume<Domain(), Domain(),");
    sysroot::compile(&independent).unwrap();
}

#[test]
fn known_value_indices_can_contradict_a_later_domain_before_type_delivery() {
    let source = "
        struct Ticket where T, flag: T {}
        comptime fn Domain() -> Type { bool }
        comptime fn consume() -> bool where T, flag: T, ticket: Ticket<T, flag> { true }
        comptime fn unused(ticket: Ticket<bool, false>) -> bool {
            consume<Domain(), true, ticket>()
        }
    ";
    let failure = sysroot::compile(source).unwrap_err();
    assert!(failure.contains("type mismatch"), "{failure}");
    sysroot::compile(&source.replace("Ticket<bool, false>", "Ticket<bool, true>")).unwrap();
}

#[test]
fn required_runtime_callable_checks_completed_input_domains() {
    for type_body in ["u8", "bool"] {
        let source = format!(
            "comptime fn Selected() -> Type {{ {type_body} }}
             comptime fn packet() -> meta.List<u8> {{ [1, 255] }}
             fn accepted() -> bool
             where T, ...values: meta.List<T>
             {{ true }}
             fn run() -> bool {{ accepted<.values = packet(), Selected()>() }}"
        );
        if type_body == "u8" {
            assert_eq!(sysroot::run(&source), Value::Bool(true));
        } else {
            assert!(sysroot::compile(&source).is_err(), "{source}");
        }
    }
}

#[test]
fn required_callable_packet_uses_computed_type_as_a_dependent_field_domain() {
    let source = "
        comptime fn Domain() -> Type { Type }
        comptime fn fields() -> meta.List<Type> { [u8, bool] }
        fn accepted() -> bool
        where T, field: T, ...values: meta.List<T>
        { true }
        fn run() -> bool { accepted<.values = fields(), Domain(), bool>() }
    ";
    assert_eq!(sysroot::run(source), Value::Bool(true));
    let failure = sysroot::compile(&source.replace("{ Type }", "{ bool }")).unwrap_err();
    assert!(
        failure.contains("clause input type mismatch: expected comptime-value, got type"),
        "{failure}"
    );
    let staged = source
        .replace("fn accepted()", "comptime fn accepted()")
        .replace("fn run()", "comptime fn check()");
    check_twice(&staged, &["check", "fields", "Domain", "accepted"]);
}

#[test]
fn type_inputs_transfer_control_to_the_enclosing_loop() {
    check_twice(
        "pure comptime fn Key<.first: Integer = 0, .last: Integer = 0> -> Type { bool }
         comptime fn mark() -> Integer { 8 }
         comptime fn check() -> bool {
             var trace: Integer = 0;
             while trace < 4 {
                 let key = Key<
                     .last = do {
                         trace = trace + 1;
                         if trace == 1 { continue; };
                         if trace == 3 { break; };
                         trace
                     },
                     .first = mark()
                 >;
             };
             trace == 3
         }",
        &["check", "mark"],
    );
}

#[test]
fn nested_type_plans_resume_only_the_lexical_target() {
    check_twice(
        "struct Outer where item: Type {}
         pure comptime fn Key<index: Integer> -> Type { bool }
         comptime fn mark() -> Integer { 7 }
         comptime fn check() -> bool {
             var visits: Integer = 0;
             var rounds: Integer = 0;
             while rounds < 4 {
                 let value = Outer<do {
                     while true {
                         let inner = Key<do { visits = visits + 1; break; 0 }>;
                         mark();
                     };
                     let key = Key<do {
                         rounds = rounds + 1;
                         if rounds == 1 { continue; };
                         if rounds == 3 { break; };
                         rounds
                     }>;
                     mark();
                     key
                 }>;
                 mark();
             };
             visits == 3 and rounds == 3
         }",
        &["check", "mark", "mark"],
    );
}

#[test]
fn exits_from_packet_preparation_and_staged_fields_skip_their_successors() {
    check_twice(
        "pure comptime fn Packet<T, tag: u8, ...items: meta.List<T>> -> Type { bool }
         comptime fn Byte() -> Type { u8 }
         comptime fn mark() -> u8 { 7 }
         comptime fn check() -> bool {
             var trace: Integer = 0;
             while trace < 4 {
                 let packet = Packet<...do {
                     trace = trace + 1;
                     if trace == 1 { continue; };
                     let tag: u8 = 9;
                     (u8, tag)
                 }, mark()>;
                 let staged = Packet<Byte(), 0, do {
                     if trace == 3 { break; };
                     255
                 }, mark()>;
             };
             trace == 3
         }",
        &["check", "mark", "Byte", "mark", "mark", "Byte"],
    );
}

#[test]
fn computed_clause_callee_precedes_collection_fields_in_the_original_invocation() {
    check_twice(
        "comptime fn select() -> bool { true }
         comptime fn collect(...items: Tuple<bool, bool>) -> bool
         where flag: bool
         {
             let (first, second) = items;
             flag and first and second
         }
         comptime fn check() -> bool {
             var step: u8 = 0;
             let result = collect<do { step = 1; select() }>(
                 do { let ready = step == 1; step = 2; ready },
                 do { let ready = step == 2; step = 3; ready }
             );
             result and step == 3
         }",
        &["check", "select", "collect"],
    );
}

#[test]
fn computed_callee_precedes_packets_and_interleaved_properties() {
    for (parameters, result, arguments, calls) in [
        (
            "...items: Tuple<bool, bool>",
            "let (a,b) = items; a and b",
            "(...do { let ready = step == 1; step = 2; (ready, true) })",
            vec!["check", "select", "collect"],
        ),
        (
            "...fields: Config",
            "fields.left and fields.right",
            "{ right: tick(), left: do { let ready = step == 1; step = 2; ready } }",
            vec!["check", "select", "tick", "collect"],
        ),
        (
            "...items: Tuple<bool, bool>, ...fields: Config",
            "let (a,b) = items; a and b and fields.left and fields.right",
            "{ right: do { let ready = step == 1; step = 2; ready }, tick(),
               left: do { let ready = step == 2; step = 3; ready }, step == 3 }",
            vec!["check", "select", "tick", "collect"],
        ),
    ] {
        let source = format!(
            "comptime fn schema(...fields: T) -> T where T {{ fields }}
             const model = schema {{ left: true, right: true }};
             typealias Config = model'type;
             comptime fn select() -> bool {{ true }}
             comptime fn tick() -> bool {{ true }}
             comptime fn collect({parameters}) -> bool where flag: bool {{
                 if not flag {{ return false; }};
                 {result}
             }}
             comptime fn check() -> bool {{
                 var step: u8 = 0;
                 collect<do {{ step = 1; select() }}>{arguments}
             }}"
        );
        check_twice(&source, &calls);
    }
}

#[test]
fn exit_during_callee_selection_skips_variadic_fields() {
    check_twice(
        "comptime fn stop() -> bool { true }
         comptime fn field() -> bool { false }
         comptime fn collect(...items: Tuple<bool, bool>) -> bool where flag: bool { flag }
         comptime fn check() -> bool {
             collect<do { if stop() { return true; }; false }>(field(), field())
         }",
        &["check", "stop"],
    );
}

#[test]
fn member_alias_spreads_run_in_order_with_only_contract_captures() {
    check_twice(
        r#"
        newtype Flags<tag: bool, first: bool, last: bool> = bool;
        struct Group where tag: bool {
            pub typealias Pair<first: bool, last: bool> = Flags<tag, first, last>;
        }
        comptime fn packet(bit: bool) -> Tuple<bool> { (bit,) }
        comptime fn check() -> bool {
            var bit = false;
            let result = Group<true>.Pair<
                ...packet(do { bit = not bit; bit }),
                ...packet(do { bit = not bit; bit })
            >;
            result == Flags<true, true, false> and not bit
        }
        "#,
        &["check", "packet", "packet"],
    );
}

#[test]
fn projected_newtype_uses_its_own_dependent_domain_after_the_parent() {
    check_twice(
        r#"
        struct Group where tag: bool {
            pub newtype Entry<.T: Type = bool, value: T> = bool;
        }
        comptime fn Byte() -> Type { u8 }
        comptime fn parent() -> bool { true }
        comptime fn check() -> bool {
            var trace: Integer = 0;
            let result = Group<parent()>.Entry<
                do { trace = trace * 10 + 1; 255 },
                .T = do { trace = trace * 10 + 2; Byte() }
            >;
            result == Group<true>.Entry<255, .T = u8> and trace == 12
        }
        "#,
        &["check", "parent", "Byte"],
    );
}

#[test]
fn member_alias_keeps_a_domain_capture_while_omitting_an_unrelated_parent_slot() {
    check_twice(
        r#"
        newtype Indexed<T, value: T, flag: bool> = bool;
        struct Group where spare: bool, Outer {
            pub typealias Entry<value: Outer, flag: bool> = Indexed<Outer, value, flag>;
        }
        comptime fn packet(value: u8) -> Tuple<u8, bool> { (value, true) }
        comptime fn check() -> bool {
            let byte: u8 = 17;
            let result = Group<false, u8>.Entry<...packet(byte)>;
            result == Indexed<u8, byte, true>
        }
        "#,
        &["check", "packet"],
    );
}
