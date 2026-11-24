//! Named callable values retain their phase and instance inputs across freezing.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;

use comptime_interpretor::Value;

#[test]
fn frozen_factory_results_preserve_both_callable_phases_and_instance_arguments() {
    for phase in ["", "comptime "] {
        let source = r#"
            PHASEfn selected(value: bool) -> bool where T { not value }
            comptime fn make() -> PHASEfn(bool) -> bool where T { selected<T> }
            fn apply(comptime action: PHASEfn(bool) -> bool) -> bool { RESULT }
            fn forward() -> bool where T { apply(make<T>()) }
            fn run() -> bool { not forward<bool>() and not forward<?bool>() }
        "#
        .replace("PHASE", phase)
        .replace(
            "RESULT",
            if phase.is_empty() {
                "action(true)"
            } else {
                "const result = action(true); result"
            },
        );
        assert_eq!(
            support::run_iir(&source, "run", vec![]),
            Value::Bool(true),
            "callable phase: {phase}"
        );
    }
}

#[test]
fn frozen_comptime_callbacks_execute_after_nominal_field_delivery() {
    let source = r#"
        struct Config { action: comptime fn(bool) -> bool }
        comptime fn selected(value: bool) -> bool where T { not value }
        comptime fn make() -> Config where T { Config { action: selected<T> } }
        fn apply(comptime config: Config) -> bool { const answer = config.action(true); answer }
        fn forward() -> bool where T { apply(make<T>()) }
        fn run() -> bool { not forward<bool>() and not forward<?bool>() }
    "#;
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn freezing_an_unused_callback_does_not_execute_its_body() {
    let source = r#"
        comptime fn selected() -> bool where T { assumes false; true }
        comptime fn make() -> comptime fn() -> bool where T { selected<T> }
        fn discard(comptime action: comptime fn() -> bool) -> bool { true }
        fn forward() -> bool where T { discard(make<T>()) }
        fn run() -> bool { forward<bool>() }
    "#;
    assert_eq!(support::run_iir(source, "run", vec![]), Value::Bool(true));
}

#[test]
fn freezing_does_not_convert_a_runtime_callback_to_comptime() {
    let source = r#"
        fn selected(value: bool) -> bool where T { not value }
        comptime fn make() -> fn(bool) -> bool where T { selected<T> }
        fn apply(comptime action: comptime fn(bool) -> bool) -> bool { action(true) }
        fn forward() -> bool where T { apply(make<T>()) }
        fn run() -> bool { forward<bool>() }
    "#;
    assert!(support::compile_with_protocols(source, &Default::default()).is_err());
}

#[test]
fn sysroot_splice_delivers_an_implicit_comptime_callback_result() {
    let source = r#"
        comptime fn selected(value: bool) -> bool where T { not value }
        comptime fn make() -> comptime fn(bool) -> bool where T { selected<T> }
        fn apply(comptime action: comptime fn(bool) -> bool) -> bool { action(true) }
        fn forward() -> bool where T { apply(make<T>()) }
        fn run() -> bool { forward<bool>() }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(false));
}

#[test]
fn value_clause_factory_inputs_wait_for_specialization() {
    assert_eq!(
        sysroot::run(include_str!(
            "../../../development/repros/staged-callable-value-input.fl"
        )),
        Value::Bool(true)
    );
}

#[test]
fn value_clause_identities_preserve_phase_and_chained_defaults() {
    for phase in ["", "comptime "] {
        let source = r#"
            PHASEfn selected() -> bool where flag: bool { flag }
            fn apply(comptime action: PHASEfn() -> bool,
                     comptime .alias: PHASEfn() -> bool = action) -> bool { alias() }
            fn forward() -> bool where flag: bool { apply(selected<flag>) }
            fn run() -> bool { forward<true>() and not forward<false>() }
        "#
        .replace("PHASE", phase);
        assert_eq!(sysroot::run(&source), Value::Bool(true), "phase: {phase}");
    }
}

#[test]
fn callback_factories_observe_outer_value_inputs() {
    let source = r#"
        struct Settings where flag: bool {}
        comptime fn selected() -> bool where flag: bool { flag }
        comptime fn make() -> comptime fn() -> bool where flag: bool {
            if flag { selected<true> } else { selected<false> }
        }
        fn apply(comptime action: comptime fn() -> bool) -> bool { action() }
        impl Settings<flag> where flag: bool {
            fn forward() -> bool { apply(make<flag>()) }
        }
        fn run() -> bool { Settings<true>.forward() and not Settings<false>.forward() }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn pending_value_inputs_do_not_hide_producer_contract_failures() {
    let source = r#"
        comptime fn selected() -> bool where flag: bool { flag }
        comptime fn make() -> comptime fn() -> bool where flag: bool {
            assumes flag;
            selected<flag>
        }
        fn discard(comptime action: comptime fn() -> bool) -> bool { true }
        fn forward() -> bool where flag: bool { discard(make<flag>()) }
        fn run() -> bool { forward<false>() }
    "#;
    let package = sysroot::compile(source).unwrap();
    let failure = luna::instantiation::instantiate(&package, "run", 1_000_000)
        .err()
        .expect("the factory must run even when its callback is unused");
    assert!(failure.contains("ProgramContractRejected"), "{failure}");
    assert_eq!(
        sysroot::run(&source.replace("forward<false>()", "forward<true>()")),
        Value::Bool(true)
    );
}

#[test]
fn deferred_identity_inputs_cannot_read_runtime_parameters() {
    let source = r#"
        comptime fn selected() -> bool where flag: bool { flag }
        fn apply(comptime action: comptime fn() -> bool) -> bool { action() }
        fn invalid(flag: bool) -> bool { apply(selected<flag>) }
    "#;
    let failure = sysroot::compile(source)
        .err()
        .expect("runtime input is unavailable");
    assert!(
        failure.contains("runtime") || failure.contains("comptime"),
        "{failure}"
    );
}

#[test]
fn staged_nominal_domains_retain_value_inputs_without_payload_reads() {
    let source = r#"
        struct Marker where flag: bool {}
        fn consume(comptime marker: Marker<flag>) -> bool where flag: bool { flag }
        fn forward() -> bool where flag: bool { consume<flag>(Marker<flag> {}) }
        fn run() -> bool { forward<true>() and not forward<false>() }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn dependent_callable_domains_survive_comptime_container_transport() {
    let source = r#"
        struct Packet where T { payload: T }
        typealias Reader = comptime fn(comptime T: Type, Packet<T>) -> T;
        comptime fn read(comptime T: Type, packet: Packet<T>) -> T { packet.payload }
        comptime fn evaluate() -> bool {
            let readers: meta.List<Reader> = [read];
            let domain: meta.List<Type> = [Reader];
            domain[0] == Reader and readers[0](bool, Packet<bool> { payload: true })
                and readers[0](u8, Packet<u8> { payload: 7 }) == 7
        }
        fn run() -> bool { evaluate() }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn staged_closed_capturing_closures_deliver_distinct_captured_values() {
    let source = r#"
        comptime fn make(flag: bool) -> %{ captured: bool } comptime fn() -> bool {
            let captured = flag;
            || captured
        }
        comptime fn invoke(cb: %{ captured: bool } comptime fn() -> bool) -> bool {
            cb()
        }
        fn apply_with() -> bool where flag: bool {
            const answer = invoke(make(flag));
            answer
        }
        fn run() -> bool {
            apply_with<true>() and not apply_with<false>()
        }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn sysroot_splice_covers_str_tuples_records_callables_and_optionals() {
    let source = r#"
        comptime fn schema(...fields: T) -> T where T { fields }
        const REC_SAMPLE = schema { flag: true, text: "rec" };
        typealias Rec = REC_SAMPLE'type;

        comptime fn label() -> str { "ok" }
        comptime fn empty_tup() -> Tuple<> { () }
        comptime fn nested_tup() -> Tuple<bool, str, Tuple<u8, bool>> { (true, "hi", (7, false)) }
        comptime fn record_val() -> Rec { schema { flag: true, text: "rec" } }
        comptime fn opt_val() -> ?Tuple<bool, str> { (true, "opt") }
        fn target_fn(x: bool) -> bool { not x }
        comptime fn callback() -> fn(bool) -> bool { target_fn }

        fn accept_str(s: str) -> bool { s == "ok" }
        fn accept_empty(e: Tuple<>) -> bool { true }
        fn accept_tup(t: Tuple<bool, str, Tuple<u8, bool>>) -> bool {
            let (flag, text, inner) = t;
            let (count, neg) = inner;
            flag and text == "hi" and count == 7 and not neg
        }
        fn accept_rec(r: Rec) -> bool {
            r.flag and r.text == "rec"
        }
        fn accept_opt(o: ?Tuple<bool, str>) -> bool {
            o match {
                (ok, text)? => ok and text == "opt",
                null => false,
            }
        }
        fn accept_fn(f: fn(bool) -> bool) -> bool { not f(true) and f(false) }

        fn run() -> bool {
            accept_str(label())
                and accept_empty(empty_tup())
                and accept_tup(nested_tup())
                and accept_rec(record_val())
                and accept_opt(opt_val())
                and accept_fn(callback())
        }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn auto_splice_handles_finite_structs_generic_nominals_and_enums() {
    let source = r#"
        struct Pair where T {
            flag: bool,
            item: T,
            tag: str,
        }
        enum Message {
            Empty,
            Code(u8, bool),
            Named { label: str, ok: bool },
        }
        struct Wrapper {
            pair: Pair<Tuple<u16, str>>,
            msg: Message,
        }

        derive Splice for Pair<T> where T :- Splice;
        derive Splice for Message;

        comptime fn make_wrapper() -> Wrapper {
            Wrapper {
                pair: Pair<Tuple<u16, str>> {
                    flag: true,
                    item: (42, "inner"),
                    tag: "outer",
                },
                msg: Message.Named { label: "evt", ok: true },
            }
        }

        fn inspect_pair(p: Pair<Tuple<u16, str>>) -> bool {
            let (num, txt) = p.item;
            p.flag and num == 42 and txt == "inner" and p.tag == "outer"
        }
        fn inspect_msg(m: Message) -> bool {
            m match {
                Message.Empty => false,
                Message.Code(_, _) => false,
                Message.Named { label, ok } => label == "evt" and ok,
            }
        }
        fn inspect_wrapper(w: Wrapper) -> bool {
            inspect_pair(w.pair) and inspect_msg(w.msg)
        }

        fn run() -> bool {
            const p = Pair<str> { flag: true, item: "derived", tag: "s" };
            const u = Message.Empty;
            const c = Message.Code(9, true);
            let explicit_ok = (u, c) match {
                (Message.Empty, Message.Code(n, ok)) => n == 9 and ok,
                _ => false,
            };
            let direct_pair = p.flag and p.item == "derived" and p.tag == "s";
            direct_pair and explicit_ok and inspect_wrapper(make_wrapper())
        }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn auto_splice_rejects_shared_references_cycles_and_drop_types() {
    let ptr_derive = r#"
        struct Borrowed { ptr: *bool }
        derive Splice for Borrowed;
    "#;
    let err = sysroot::compile(ptr_derive).unwrap_err();
    assert!(
        err.contains("cannot auto-derive Splice") && err.contains("*T"),
        "{err}"
    );

    let cyclic_derive = r#"
        struct Node { next: ?Node }
        derive Splice for Node;
    "#;
    let err = sysroot::compile(cyclic_derive).unwrap_err();
    assert!(
        err.contains("cannot auto-derive Splice for cyclic type"),
        "{err}"
    );

    let drop_derive = r#"
        struct Guard { id: u8 }
        impl Drop for Guard { fn drop(*self) {} }
        derive Splice for Guard;
    "#;
    let err = sysroot::compile(drop_derive).unwrap_err();
    assert!(
        err.contains("cannot auto-derive Splice for Drop type"),
        "{err}"
    );

    let implicit_drop = r#"
        struct Guard { id: u8 }
        impl Drop for Guard { fn drop(*self) {} }
        struct Holder { guard: Guard }
        comptime fn make() -> Holder { Holder { guard: Guard { id: 1 } } }
        fn take(h: Holder) -> u8 { h.guard.id }
        fn run() -> bool { take(make()) == 1 }
    "#;
    let err = sysroot::compile(implicit_drop).unwrap_err();
    assert!(
        err.contains("cannot auto-derive Splice for Drop type"),
        "{err}"
    );
}

#[test]
fn auto_splice_scopes_drop_trait_and_rejects_invalid_closure_and_comptime_fn_fields() {
    let user_drop_source = r#"
        trait Drop {
            fn drop(*self);
        }
        struct LocalGuard { id: u8 }
        impl Drop for LocalGuard {
            fn drop(*self) {}
        }
        derive Splice for LocalGuard;
        comptime fn make() -> LocalGuard {
            LocalGuard { id: 7 }
        }
        fn run() -> bool {
            let g = make();
            g.id == 7
        }
    "#;
    assert_eq!(sysroot::run(user_drop_source), Value::Bool(true));

    let closure_ref_field = r#"
        struct ClosureWithRef {
            cb: %{ ptr: *bool } fn() -> bool,
        }
        derive Splice for ClosureWithRef;
    "#;
    let err = sysroot::compile(closure_ref_field).unwrap_err();
    assert!(err.contains("cannot auto-derive Splice"), "{err}");

    let comptime_fn_field = r#"
        struct ComptimeField {
            f: comptime fn(u8) -> u8,
        }
        derive Splice for ComptimeField;
    "#;
    let err = sysroot::compile(comptime_fn_field).unwrap_err();
    assert!(err.contains("cannot auto-derive Splice"), "{err}");
}
