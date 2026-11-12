//! Contextual-code storage and entry snapshots use the actual declaration world.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_interpretor::Value;

#[test]
fn repeated_pattern_invocations_fit_the_real_sysroot_preparation_budget() {
    let source =
        include_str!("../../../development/repros/pattern-generator-sysroot-preparation.fl");
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn single_hole_scalar_construction_fits_the_real_sysroot_budget() {
    let source = include_str!("../../../development/repros/scalar-code-construction.fl");
    // Exercise both branches and their interleaving, retaining eight quote layers.
    for value in [0, 165, 255] {
        let source = source.replace("byte_code(255)", &format!("byte_code({value})"));
        assert_eq!(sysroot::run(&source), Value::UnsignedInteger(value));
    }
}

#[test]
fn optional_branch_probes_do_not_repeat_generator_receiver_effects() {
    let source = "
        fn wrap(value: ?T) -> ?T where T { value }
        struct State { values: meta.List<bool> }
        impl State {
            comptime fn build(self) -> <|- bool> {
                self.values.append(true);
                '(true)
            }
        }
        comptime fn outer() -> <|- bool> {
            let state = State { values: [] };
            const code = '(do {
                let _ = wrap(if true { ~<state.build()> } else { null });
                true
            });
            if state.values.length() == 1 { code } else { '(false) }
        }
        fn run() -> bool { ~<outer()> }
    ";
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn contextual_retry_discards_effects_before_an_unresolved_optional_tail() {
    let source = "
        fn choose(value: ?T, fallback: T) -> T where T { fallback }
        struct State { values: meta.List<bool> }
        impl State {
            comptime fn build(self) -> <|- bool> {
                self.values.append(true);
                '(true)
            }
        }
        comptime fn outer() -> <|- bool> {
            let state = State { values: [] };
            const code = '(choose(
                if true { do { let _ = ~<state.build()>; null } } else { null },
                true
            ));
            if state.values.length() == 1 { code } else { '(false) }
        }
        fn run() -> bool { ~<outer()> }
    ";
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn entry_snapshot_retains_list_domains_and_isolates_later_receiver_updates() {
    let source = "
        struct State { flag: bool, values: meta.List<bool> }
        impl State {
            comptime fn build(self) -> <|- bool>
            where ensures self.flag and not self'old.flag
            {
                self.values.append(false);
                self.flag = true;
                '(true)
            }
        }
        comptime fn outer() -> <|- bool> {
            let state = State { flag: false, values: [true] };
            const code = state.build();
            let changed = state.values.length() == 2;
            '(changed and ~<code>)
        }
        fn run() -> bool { ~<outer()> }
    ";
    assert_eq!(sysroot::run(source), Value::Bool(true));
    let failure =
        sysroot::compile(&source.replace("not self'old.flag", "self'old.flag")).unwrap_err();
    assert!(failure.contains("postcondition rejected"), "{failure}");
}

#[test]
fn nominal_object_entries_use_the_generator_world_when_snapshotting_a_receiver() {
    let source = "
        struct Payload { flag: bool }
        struct State { flag: bool, values: meta.Object }
        impl State {
            comptime fn build(self) -> <|- bool>
            where ensures self.flag and not self'old.flag
            {
                self.values = self.values.append(Payload { flag: false });
                self.flag = true;
                '(true)
            }
        }
        comptime fn outer() -> <|- bool> {
            let state = State { flag: false, values: { item: Payload { flag: true } } };
            const code = state.build();
            let changed = state.values.length() == 2;
            '(changed and ~<code>)
        }
        fn run() -> bool { ~<outer()> }
    ";
    assert_eq!(sysroot::run(source), Value::Bool(true));
    let failure =
        sysroot::compile(&source.replace("self.flag = true", "self.flag = false")).unwrap_err();
    assert!(failure.contains("postcondition rejected"), "{failure}");
}

#[test]
fn nested_contextual_retries_restore_storage_before_retaining_later_code() {
    for input in [
        "if true { do { let _ = ~<state.build()>; null } } else { null }",
        "if true { if false { null } else {
            do { let _ = ~<state.build()>; null }
        } } else { null }",
        "if true { do {
            let local = true;
            if local { do { let _ = ~<state.build()>; null } } else { null }
        } } else { null }",
        "if false { null } else { do {
            let _ = ~<state.build()>;
            if true { null } else { null }
        } }",
    ] {
        let source = format!(
            "
            fn choose(value: ?T, fallback: T) -> T where T {{ fallback }}
            struct State {{ values: meta.List<bool> }}
            impl State {{
                comptime fn build(self) -> <|- bool> {{
                    self.values.append(true);
                    '(true)
                }}
            }}
            comptime fn outer() -> <|- bool> {{
                let state = State {{ values: [] }};
                const first = '(choose({input}, true));
                const next = state.build();
                if state.values.length() == 2 {{ first }} else {{ '(false) }}
            }}
            fn run() -> bool {{ ~<outer()> }}
            "
        );
        assert_eq!(sysroot::run(&source), Value::Bool(true), "{input}");
    }
}
