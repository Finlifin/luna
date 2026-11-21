//! Static input identity is evidence, not a replacement for its producer.
use comptime_interpretor::{Budget, Event, Value};

#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;

fn check_both(source: &str, entered: usize) {
    check_events(source, &[("accept", entered)]);
}

fn check_events(source: &str, events: &[(&str, usize)]) {
    let package = sysroot::compile(source).unwrap();
    let id = |name: &str| {
        package
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .declaration
    };
    for tree in [false, true] {
        let budget = Budget {
            steps: 1_000_000,
            depth: 128,
        };
        sysroot::with_comptime(&package, budget, tree, |vm| {
            for invocation in 1..=2 {
                assert_eq!(
                    vm.invoke(id("check"), vec![]).unwrap(),
                    Value::Bool(true),
                    "tree={tree}"
                );
                for (name, entered) in events {
                    assert_eq!(vm.events().iter().filter(|event| matches!(event, Event::EnterFunction(declaration) if *declaration == id(name))).count(), entered * invocation, "{name}, tree={tree}");
                }
            }
        });
    }
}

const FLAG: &str = "trait Flag {} impl Flag for bool {}";

#[test]
fn unobserved_dependencies_do_not_start_an_isolated_invocation() {
    // These remain valid source-checking requests, not language rejection
    // cases. Until source/VM continuation exists, fail at the missing boundary
    // instead of running Selected (which would index past the list) while checking.
    for (parameters, call) in [
        ("comptime T: Type, value: T", "accept(Selected(), true)"),
        (
            "comptime T: Type, value: T",
            "do { let target = accept; target(Selected(), true) }",
        ),
        (
            "comptime .T: Type = bool, value: T",
            "accept(do { return true; true }, .T = Selected())",
        ),
    ] {
        let failure = sysroot::compile(&format!(
            "comptime fn Selected() -> Type {{
                let items: meta.List<Type> = [bool];
                items[1]
            }}
            comptime fn accept({parameters}) -> bool {{ true }}
            comptime fn check() -> bool {{ {call} }}"
        ))
        .unwrap_err();
        assert!(
            failure.contains("dependent call input needs its original comptime invocation"),
            "{call}: {failure}"
        );
    }
}

#[test]
fn direct_type_results_keep_argument_effects_and_the_selected_binding() {
    let source = "
        struct Left {}
        typealias First = Left;
        comptime fn mark() -> bool { true }
        comptime fn Selected(flag: bool, tick: bool) -> Type {
            if flag { First } else { char }
        }
        comptime fn Forward(ty: Type) -> Type { ty }
        comptime fn empty(comptime ty: Type) -> ?ty { null }
        comptime fn indirect(target: comptime fn(comptime ty: Type) -> ?ty) -> bool {
            let value: ?char = target(Selected(false, mark()));
            value matches null
        }
        comptime fn check() -> bool {
            let left: ?Left = empty(Selected(true, mark()));
            let right: ?char = empty(Selected(false, mark()));
            let again: ?Left = empty(Forward(Left));
            (left matches null) and (right matches null) and (again matches null) and indirect(empty)
        }
    ";
    check_events(
        source,
        &[
            ("mark", 3),
            ("Selected", 3),
            ("Forward", 1),
            ("empty", 4),
            ("indirect", 1),
        ],
    );
    let wrong = source.replace("let right: ?char", "let right: ?Left");
    let failure = sysroot::compile(&wrong).unwrap_err();
    assert!(
        failure.contains("type mismatch") && failure.contains("(primitive char)"),
        "{failure}"
    );
}

#[test]
fn type_return_evidence_joins_only_agreeing_possible_exits() {
    let source = "
        typealias First = char;
        comptime fn Selected(flag: bool) -> Type { if flag { First } else { char } }
        comptime fn empty(comptime ty: Type) -> ?ty { null }
        comptime fn relay(flag: bool) -> bool {
            let value: ?char = empty(Selected(flag));
            value matches null
        }
        comptime fn check() -> bool { relay(true) and relay(false) }
    ";
    check_events(source, &[("Selected", 2), ("empty", 2)]);
    for divergent in [
        source.replace("else { char }", "else { bool }"),
        source.replace(
            "if flag { First }",
            "if do { if flag { return bool; }; true } { First }",
        ),
    ] {
        let failure = sysroot::compile(&divergent).unwrap_err();
        assert!(
            failure.contains("needs its original comptime invocation"),
            "{failure}"
        );
    }
}

#[test]
fn an_explicit_type_consumer_uses_each_original_invocation_once() {
    check_events(
        "
        comptime fn Selected(flag: bool) -> Type { if flag { bool } else { char } }
        comptime fn empty(comptime ty: Type) -> ?ty { null }
        comptime fn consume(comptime ty: Type, flag: bool) -> bool {
            empty(ty) match {
                null => if flag { ty == bool } else { ty == char },
                _ => false,
            }
        }
        comptime fn relay(flag: bool) -> bool { consume(Selected(flag), flag) }
        comptime fn check() -> bool { relay(true) and relay(false) }
        ",
        &[("Selected", 2), ("consume", 2), ("empty", 2)],
    );
}

#[test]
fn a_default_keeps_its_scalar_prefix_across_an_independent_call() {
    check_events(
        &format!(
            "{FLAG}
        comptime fn mark() -> bool {{ true }}
        comptime fn accept(comptime .T: Type = bool,
            comptime .U: Type = T,
            comptime .V: Type = do {{ mark(); U }}, value: V) -> bool
        where requires V :- Flag
        {{ true }}
        comptime fn check() -> bool {{ accept(true) }}"
        ),
        &[("accept", 1), ("mark", 1)],
    );
}

#[test]
fn explicit_constant_evaluation_remains_an_execution_boundary() {
    assert_eq!(
        sysroot::run(&format!(
            "{FLAG}
            comptime fn Selected() -> Type {{ bool }}
            comptime fn accept(comptime T: Type, value: T) -> bool
            where requires T :- Flag
            {{ true }}
            fn run() -> bool {{
                const selected = Selected();
                const result = accept(selected, true);
                result
            }}"
        )),
        Value::Bool(true),
    );
}

#[test]
fn immutable_local_identities_do_not_need_required_execution() {
    check_both(
        &format!(
            "{FLAG}
            trait Enabled where flag: bool {{}}
            impl Enabled<true> for bool {{}}
            comptime fn accept(comptime T: Type, comptime flag: bool, value: T) -> bool
            where requires T :- Flag and bool :- Enabled<flag>
            {{ flag }}
            comptime fn check() -> bool {{
                let selected = bool;
                let enabled = true;
                accept(selected, enabled, true)
            }}"
        ),
        1,
    );
}

#[test]
fn dependent_default_unknown_prefix_runs_in_the_original_invocation() {
    check_events(
        &format!(
            "{FLAG}
        comptime fn mark() -> bool {{ true }}
        comptime fn accept(comptime .T: Type = bool,
            comptime .U: Type = T,
            comptime .V: Type = do {{ mark(); U }}, value: V) -> bool
        where requires V :- Flag
        {{ true }}
        comptime fn check() -> bool {{
            var trace: Integer = 0;
            let result = accept(
                do {{ trace = trace * 10 + 1; true }},
                .T = do {{ trace = trace * 10 + 2; bool }}
            );
            result and trace == 12
        }}
    "
        ),
        &[("accept", 1), ("mark", 1)],
    );
}

#[test]
fn only_selected_defaults_write_the_type_seen_by_later_parameters() {
    let source = format!(
        "{FLAG}
        comptime fn mark() -> bool {{ true }}
        comptime fn accept(comptime .T: Type = bool,
            .change: bool = do {{ T = char; true }},
            comptime .U: Type = do {{ mark(); T }}, value: U) -> bool
        where requires U :- Flag {{ true }}
        comptime fn check() -> bool {{ CALL }}"
    );
    let failure = sysroot::compile(&source.replace("CALL", "accept(true)")).unwrap_err();
    assert!(
        failure.contains("not established") && failure.contains("(primitive char)"),
        "{failure}"
    );
    check_events(
        &source
            .replace("impl Flag for bool", "impl Flag for char")
            .replace("CALL", "accept('z')"),
        &[("accept", 1), ("mark", 1)],
    );
    check_events(
        &source.replace("CALL", "accept(true, .change = false)"),
        &[("accept", 1), ("mark", 1)],
    );
}

#[test]
fn each_default_reads_the_version_at_its_declaration_slot() {
    check_events(
        "comptime fn Initial() -> Type { bool }
        comptime fn note() -> bool { true }
        comptime fn accept(comptime .T: Type = bool,
            .first: bool = do { T = do { note(); char }; true },
            comptime .U: Type = T,
            .second: bool = do { T = u8; true },
            comptime .V: Type = T, x: U, y: V) -> bool {
            U == char and V == u8
        }
        comptime fn check() -> bool { accept('a', 7, .T = Initial()) }",
        &[("accept", 1), ("Initial", 1), ("note", 1)],
    );
}

#[test]
fn stored_values_depend_on_the_inputs_of_each_call() {
    let source = "
        trait Enabled where flag: bool {}
        impl Enabled<true> for bool {}
        comptime fn accept(comptime seed: bool, comptime .flag: bool = false,
            .change: bool = do { let inverted = not seed; flag = inverted; true },
            comptime .copy: bool = flag) -> bool
        where requires bool :- Enabled<copy> { copy }
        comptime fn check() -> bool { accept(false) }";
    check_both(source, 1);
    let failure = sysroot::compile(&source.replace("accept(false)", "accept(true)")).unwrap_err();
    assert!(
        failure.contains("not established") || failure.contains("not satisfied"),
        "{failure}"
    );
}

#[test]
fn store_annotations_follow_the_callers_type_inputs() {
    let source = "
        comptime fn Selected(comptime .S: Type = bool, comptime .T: Type = char,
            .write: bool = do { T = do { let items: meta.List<S> = []; items'type }; true },
            comptime .U: Type = T,
            comptime .V: Type = if T == U { U } else { char }, x: V) -> Type { V }
        comptime fn forwarded(comptime element: Type) -> bool {
            Selected([], .S = element) == meta.List<element>
        }
        comptime fn check() -> bool {
            Selected([]) == meta.List<bool> and Selected([], .S = char) == meta.List<char>
                and forwarded(char)
        }";
    check_events(source, &[("Selected", 3), ("forwarded", 1)]);
    let error =
        sysroot::compile(&source.replace("Selected([], .S = char)", "Selected([true], .S = char)"))
            .unwrap_err();
    assert!(
        error.contains("expected (primitive char), found (primitive bool)"),
        "{error}"
    );
}

#[test]
fn an_opaque_store_drops_the_old_version_until_a_definite_overwrite() {
    let source = "
        trait Flag {} impl Flag for char {}
        comptime fn Hidden() -> Type { bool }
        comptime fn accept(comptime .T: Type = char,
            .write: bool = do { T = Hidden(); true },
            .overwrite: bool = do { T = char; true },
            comptime .U: Type = T, x: U) -> bool
        where requires U :- Flag { true }
        comptime fn check() -> bool { accept('z') }";
    check_events(source, &[("Hidden", 1), ("accept", 1)]);
    let failure =
        sysroot::compile(&source.replace("accept('z')", "accept('z', .overwrite = false)"))
            .unwrap_err();
    assert!(failure.contains("original invocation"), "{failure}");
}

#[test]
fn writes_inside_a_default_replace_its_entry_snapshot() {
    let source = "
        trait Flag {} impl Flag for char {}
        comptime fn mark() -> bool { true }
        comptime fn accept(comptime .T: Type = bool,
            comptime .U: Type = do { T = char; mark(); T }, value: U) -> bool
        where requires U :- Flag { true }
        comptime fn check() -> bool { accept('z') }";
    check_events(source, &[("accept", 1), ("mark", 1)]);
    let error =
        sysroot::compile(&source.replace("impl Flag for char", "impl Flag for bool")).unwrap_err();
    assert!(
        error.contains("not established") && error.contains("(primitive char)"),
        "{error}"
    );
}

#[test]
fn a_default_can_determine_its_result_without_observing_the_overwritten_input() {
    let source = "
        trait Flag {} impl Flag for char {}
        comptime fn Initial() -> Type { bool }
        comptime fn note(t: Type) -> bool { t == bool }
        comptime fn accept(comptime .T: Type = bool, comptime .S: Type = char,
            comptime .U: Type = do { note(T); T = S; T }, x: U) -> bool
        where requires U :- Flag { true }
        comptime fn check() -> bool { accept('z', .T = Initial()) }";
    check_events(source, &[("Initial", 1), ("note", 1), ("accept", 1)]);
    let failure =
        sysroot::compile(&source.replace("impl Flag for char", "impl Flag for bool")).unwrap_err();
    assert!(
        failure.contains("not established") && failure.contains("(primitive char)"),
        "{failure}"
    );
    // A self-assignment still reads the entry value.
    let failure = sysroot::compile(&source.replace("T = S", "T = T")).unwrap_err();
    assert!(
        failure.contains("original comptime invocation"),
        "{failure}"
    );
}

#[test]
fn earlier_local_reads_keep_their_version_in_a_default_result() {
    let source = "
        comptime fn Initial() -> Type { bool }
        comptime fn accept(comptime .T: Type = bool, comptime .S: Type = char,
            comptime .U: Type = do { let saved = S; S = T; saved }, x: U) -> bool { true }
        comptime fn check() -> bool { accept('z', .T = Initial()) }";
    check_events(source, &[("Initial", 1), ("accept", 1)]);
}

#[test]
fn an_unobserved_runtime_default_restores_captures_for_required_execution() {
    let source = "
        comptime fn check_flag(flag: bool) -> bool { assumes flag; true }
        comptime fn Selected() -> Type { char }
        fn accept(comptime .flag: bool = true,
            comptime .T: Type = do { check_flag(flag); Selected() }, x: T) -> bool { true }
        fn run() -> bool { accept('z') }";
    assert_eq!(sysroot::run(source), Value::Bool(true));
    let failure =
        sysroot::compile(&source.replace("accept('z')", "accept('z', .flag = false)")).unwrap_err();
    assert!(failure.contains("ProgramContractRejected"), "{failure}");
}

#[test]
fn scalar_default_snapshots_cover_value_requirements_and_literal_overrides() {
    let source = "
        trait Enabled where flag: bool {}
        impl Enabled<true> for bool {}
        comptime fn mark(input: bool) -> bool { not input }
        comptime fn accept(comptime flag: bool,
            comptime .next: bool = do { mark(flag); flag }) -> bool
        where requires bool :- Enabled<next> { next }
        comptime fn check() -> bool { accept(true) and accept(false, .next = true) }";
    check_events(source, &[("accept", 2), ("mark", 1)]);
    let error = sysroot::compile(&source.replace("accept(true)", "accept(false)")).unwrap_err();
    assert!(
        error.contains("not established") || error.contains("not satisfied"),
        "{error}"
    );
}

#[test]
fn nested_dependent_defaults_keep_checked_inputs_and_source_order() {
    check_events(
        &format!(
            "{FLAG}
        comptime fn mark() -> bool {{ true }}
        comptime fn accept(comptime .T: Type = bool,
            comptime .U: Type = T, comptime .V: Type = U,
            value: V, .last: bool = mark()) -> bool
        where requires V :- Flag
        {{ last }}
        comptime fn check() -> bool {{
            var trace: Integer = 0;
            let result = accept(
                do {{ trace = trace * 10 + 1; true }},
                .T = do {{ trace = trace * 10 + 2; bool }}
            );
            result and trace == 12
        }}
    "
        ),
        &[("accept", 1), ("mark", 1)],
    );
}

#[test]
fn receiver_and_named_type_inputs_keep_their_positions_across_early_return() {
    check_events(
        "
        struct Gate { enabled: bool }
        comptime fn fallback() -> bool { true }
        impl Gate {
            comptime fn accept(self, comptime .T: Type = bool, value: T,
                .last: bool = fallback()) -> bool { self.enabled and last }
        }
        comptime fn exercise(exit: bool) -> bool {
            var trace: Integer = 0;
            let result = (do { trace = trace * 10 + 1; Gate { enabled: true } }).accept(
                do { if exit { return trace == 1; }; trace = trace * 10 + 2; true },
                .T = do { trace = trace * 10 + 3; bool }
            );
            result and trace == 123
        }
        comptime fn check() -> bool { exercise(false) and exercise(true) }
        ",
        &[("exercise", 2), ("fallback", 1)],
    );
}

#[test]
fn new_facts_after_effects_support_both_type_and_value_inputs() {
    check_both(
        &format!(
            "{FLAG}
        trait Enabled where flag: bool {{}}
        impl Enabled<true> for bool {{}}
        comptime fn accept(comptime T: Type, comptime flag: bool) -> bool
        where requires T :- Flag and bool :- Enabled<flag>
        {{ flag }}
        comptime fn check() -> bool {{
            var trace: Integer = 0;
            let result = accept(
                do {{ trace = trace * 10 + 1; let selected = bool; selected }},
                do {{ trace = trace * 10 + 2; let selected = true; selected }}
            );
            result and trace == 12
        }}
    "
        ),
        1,
    );
}

#[test]
fn default_prefixes_execute_per_invocation_before_new_local_facts() {
    check_events(&format!("{FLAG}
        comptime fn mark() -> bool {{ true }}
        comptime fn accept(comptime .T: Type = do {{ mark(); let selected = bool; selected }}) -> bool
        where requires T :- Flag
        {{ true }}
        comptime fn check() -> bool {{ accept() }}
    "), &[("accept", 1), ("mark", 1)]);
}

#[test]
fn pattern_bindings_after_effects_share_the_ordinary_static_observer() {
    check_both(
        &format!(
            "{FLAG}
        comptime fn accept(comptime T: Type) -> bool
        where requires T :- Flag
        {{ true }}
        comptime fn check() -> bool {{
            var trace: Integer = 0;
            let result = accept(do {{
                trace = trace + 1;
                let (selected, flag) = (bool, true);
                if flag {{ selected }} else {{ char }}
            }});
            result and trace == 1
        }}
    "
        ),
        1,
    );
}

#[test]
fn default_expressions_observe_only_previously_supplied_input_facts() {
    check_both(
        "
        trait Enabled where flag: bool {}
        impl Enabled<true> for bool {}
        comptime fn accept(comptime flag: bool, comptime .next: bool = do {
            let result = not flag; result
        }) -> bool
        where requires bool :- Enabled<next>
        { next }
        comptime fn check() -> bool {
            var trace: Integer = 0;
            let result = accept(do { trace = trace + 1; false });
            result and trace == 1
        }
    ",
        1,
    );
}

#[test]
fn effectful_prefixes_invalidate_old_facts_and_keep_shadowing_lexical() {
    for body in [
        "trace = trace + 1; var selected: Type = bool; selected = char; selected",
        "trace = trace + 1; let selected = bool; do { let selected = char; selected }",
    ] {
        let failure = sysroot::compile(&format!(
            "{FLAG}
            comptime fn accept(comptime T: Type) -> bool
            where requires T :- Flag
            {{ true }}
            comptime fn check() -> bool {{
                var trace: Integer = 0;
                accept(do {{ {body} }})
            }}
        "
        ))
        .unwrap_err();
        assert!(
            failure.contains("dependent call input needs its original comptime invocation")
                || failure.contains("not established"),
            "{failure}"
        );
    }
}

#[test]
fn trait_evidence_preserves_source_order_and_nested_result_prefixes() {
    check_both(
        &format!(
            "{FLAG}
        comptime fn accept(comptime T: Type, value: Integer) -> bool
        where requires T :- Flag
        {{ value == 123 }}
        comptime fn check() -> bool {{
            var trace: Integer = 0;
            let result = accept(
                do {{ trace = trace * 10 + 1; do {{ trace = trace * 10 + 2; bool }} }},
                do {{ trace = trace * 10 + 3; trace }}
            );
            result and trace == 123
        }}
    "
        ),
        1,
    );
}

#[test]
fn ordinary_value_dependencies_use_the_same_identity_observation() {
    check_both(
        "
        trait Enabled where flag: bool {}
        impl Enabled<true> for bool {}
        comptime fn accept(comptime flag: bool) -> bool
        where requires bool :- Enabled<flag>
        { flag }
        comptime fn check() -> bool {
            var trace: Integer = 0;
            let result = accept(do { trace = trace + 1; not false });
            result and trace == 1
        }
    ",
        1,
    );
}

#[test]
fn identity_contextualizes_later_arguments_without_running_the_producer() {
    check_both(
        "
        comptime fn accept(comptime T: Type, value: T) -> T { value }
        comptime fn check() -> bool {
            var trace: Integer = 0;
            let result: u8 = accept(do { trace = trace + 1; u8 }, 7);
            result == 7 and trace == 1
        }
    ",
        1,
    );
}

#[test]
fn default_forwarding_does_not_replay_the_explicit_producer() {
    check_both(
        &format!(
            "{FLAG}
        comptime fn accept(comptime T: Type, comptime .U: Type = T) -> bool
        where requires U :- Flag
        {{ T == U }}
        comptime fn check() -> bool {{
            var trace: Integer = 0;
            let result = accept(do {{ trace = trace + 1; bool }});
            result and trace == 1
        }}
    "
        ),
        1,
    );
}

#[test]
fn result_identity_does_not_erase_control_transfer() {
    check_both(
        &format!(
            "{FLAG}
        comptime fn accept(comptime T: Type) -> bool
        where requires T :- Flag
        {{ false }}
        comptime fn check() -> bool {{
            accept(do {{ return true; bool }})
        }}
    "
        ),
        0,
    );
}

#[test]
fn observed_identities_still_need_trait_evidence_and_argument_domain_checks() {
    let failure = sysroot::compile(&format!(
        "{FLAG}
        comptime fn accept(comptime T: Type) -> bool
        where requires T :- Flag
        {{ true }}
        comptime fn check() -> bool {{
            var trace: Integer = 0;
            accept(do {{ trace = trace + 1; if true {{ char }} else {{ bool }} }})
        }}
    "
    ))
    .unwrap_err();
    assert!(failure.contains("not established"), "{failure}");
    let failure = sysroot::compile(
        "
        comptime fn accept(comptime T: Type, value: T) -> T { value }
        comptime fn check() -> u8 {
            var trace: Integer = 0;
            accept(do { trace = trace + 1; u8 }, 256)
        }
    ",
    )
    .unwrap_err();
    assert!(
        failure.contains("does not fit the requested fixed-width representation"),
        "{failure}"
    );
}

#[test]
fn tail_observation_cannot_reuse_a_local_value_from_before_the_prefix() {
    let failure = sysroot::compile(&format!(
        "{FLAG}
        comptime fn accept(comptime T: Type) -> bool
        where requires T :- Flag
        {{ true }}
        comptime fn check() -> bool {{
            var selected: Type = bool;
            accept(do {{ selected = char; selected }})
        }}
    "
    ))
    .unwrap_err();
    // This source is invalid, but the current conservative path can still
    // stop before proving the missing Flag implementation for char.
    assert!(
        failure.contains("dependent call input needs its original comptime invocation")
            || failure.contains("not established"),
        "{failure}"
    );
}

#[test]
fn prefix_obligations_are_checked_before_observing_the_tail() {
    let failure = sysroot::compile(&format!(
        "{FLAG}
        comptime fn accept(comptime T: Type) -> bool
        where requires T :- Flag
        {{ true }}
        comptime fn check() -> bool {{
            accept(do {{ accept(char); bool }})
        }}
    "
    ))
    .unwrap_err();
    assert!(failure.contains("not established"), "{failure}");
}

#[test]
fn required_runtime_inputs_resume_their_own_call_and_default_slots() {
    let declarations = "
        comptime fn Select(flag: bool) -> Type {
            if flag { bool } else { u8 }
        }
        fn choose(comptime .flag: bool = true, comptime .T: Type = Select(flag),
                  value: T) -> T { value }
    ";
    let source = format!(
        "{declarations}
        fn run() -> bool {{
            let a = choose(true);
            let b = choose(255, .flag = false);
            let c = choose(7, .T = Select(false));
            a and b == 255 and c == 7
        }}"
    );
    assert_eq!(sysroot::run(&source), Value::Bool(true));

    let checked = declarations.replace("if flag", "assumes flag; if flag");
    let failure = sysroot::compile(&format!(
        "{checked}
        fn run() -> u8 {{ choose(255, .flag = false) }}"
    ))
    .unwrap_err();
    assert!(failure.contains("ProgramContractRejected"), "{failure}");

    // Supplying T removes the failing default from this invocation.
    let source = format!(
        "{checked}
        fn run() -> bool {{ choose(true, .flag = false, .T = bool) }}"
    );
    assert_eq!(sysroot::run(&source), Value::Bool(true));
}

#[test]
fn dependent_inputs_preserve_optional_error_and_object_values() {
    let source = r#"
        enum ProbeErr { Missing }
        enum OtherErr { Failed }
        struct Stamp where T, key: T { flag: bool }
        comptime fn accept(comptime T: Type, comptime key: T, item: Stamp<T, key>) -> Stamp<T, key> { item }

        comptime fn check() -> bool {
            let empty: ?bool = null;
            let present: ?bool = true;
            let ok: !ProbeErr bool = true;
            let failed: !ProbeErr bool = error ProbeErr.Missing;
            let wide: ![OtherErr, ProbeErr] bool = failed;
            let object: Object = {ready: true};
            accept(?bool, empty, Stamp<?bool, null> { flag: true }).flag and
                accept(?bool, present, Stamp<?bool, true> { flag: true }).flag and
                accept(!ProbeErr bool, ok, Stamp<!ProbeErr bool, true> { flag: true }).flag and
                accept(!ProbeErr bool, failed, Stamp<!ProbeErr bool, error ProbeErr.Missing> { flag: true }).flag and
                accept(![OtherErr, ProbeErr] bool, wide, Stamp<![OtherErr, ProbeErr] bool, error ProbeErr.Missing> { flag: true }).flag and
                accept(Object, object, Stamp<Object, {ready: true}> { flag: true }).flag
        }
    "#;
    check_both(source, 6);
    let mismatch = source.replace("Stamp<?bool, null> {", "Stamp<?bool, true> {");
    assert!(sysroot::compile(&mismatch).is_err());
}

#[test]
fn staged_runtime_defaults_share_writes_but_keep_delivered_arguments() {
    let source = "
        comptime fn Select() -> Type { bool }
        fn gate(comptime .T: Type = bool,
            comptime .U: Type = do { T = char; Select() },
            comptime .V: Type = T, x: U, y: V) -> bool {
            const valid = T == bool and U == bool and V == char;
            valid
        }
        fn run() -> bool {
            gate(true, 'z') and not gate(true, false, .U = bool)
        }";
    assert_eq!(sysroot::run(source), Value::Bool(true));
    let error =
        sysroot::compile(&source.replace("gate(true, 'z')", "gate(true, false)")).unwrap_err();
    assert!(
        error.contains("expected (primitive char), found (primitive bool)"),
        "{error}"
    );
}

#[test]
fn selected_defaults_publish_only_the_executed_branch() {
    let source = "
        fn readback(item: A, comptime .flag: bool = true,
            comptime .seed: bool = false,
            comptime .update: bool = do { if flag { seed = not seed; }; seed },
            comptime .next: bool = seed) -> bool where A {
            const checked = not seed and update == flag and next == flag;
            checked
        }
        fn run() -> bool {
            readback(true) and readback('z', .flag = false)
        }";
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn closed_staged_default_writes_survive_required_invocation_and_argument_erasure() {
    let source = "
        comptime fn Select() -> Type { bool }
        fn deliver(comptime .T: Type = bool,
            comptime .U: Type = do { T = char; Select() },
            comptime .V: Type = T, x: U, y: V) -> V { y }
        fn run() -> bool {
            deliver(true, 'z') == 'z' and deliver(false, true, .U = bool)
        }";
    assert_eq!(sysroot::run(source), Value::Bool(true));
    let error = sysroot::compile(&source.replace("deliver(true, 'z')", "deliver(true, false)"))
        .unwrap_err();
    assert!(
        error.contains("expected (primitive char), found (primitive bool)"),
        "{error}"
    );
}

#[test]
fn runtime_defaults_read_staged_slots_at_their_own_position() {
    let source = "
        fn selected(comptime .seed: bool = false,
            .before: bool = seed,
            comptime .update: bool = do { seed = not seed; seed },
            .after: bool = seed) -> bool {
            before == seed and after != seed and after == update
        }
        fn forward(comptime first: bool) -> bool { selected(.seed = first) }
        fn run() -> bool {
            selected() and not selected(.update = true) and forward(true) and forward(false)
        }";
    assert_eq!(sysroot::run(source), Value::Bool(true));
    let specialized = source.replace(
        "before == seed",
        "const original = seed; before == original",
    );
    assert_eq!(sysroot::run(&specialized), Value::Bool(true));
}

#[test]
fn inference_delivers_the_selected_comptime_default_once() {
    let source = "
        typealias Selector = %{flag: bool} comptime fn() -> Type;
        comptime fn make() -> Selector {
            var flag = false;
            || { flag = not flag; if flag { bool } else { char } }
        }
        comptime fn select_once(.select: Selector = make(), .chosen: Type = select(),
            .value: defer chosen = true) -> T where T, requires T == bool {
            select() == char
        }
        comptime fn forward() -> bool { select_once() }
        comptime fn override_chosen() -> bool { select_once(.chosen = char, .value = 'z') }
        const chosen = forward();
        const skipped = override_chosen();
        fn run() -> bool { chosen and not skipped }";
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn open_comptime_defaults_keep_mutable_slots_through_the_residual_suffix() {
    let source = "
        comptime fn gather(.seed: bool = false,
            .flip: bool = do { seed = not seed; seed },
            .later: bool = seed,
            .value: defer if later { char } else { bool } = 'z',
            .before: bool = seed, .clear: bool = do { seed = false; seed },
            .after: bool = seed) -> bool {
            not seed and flip and later and before and not clear and not after
        }
        const written = gather();
        const skipped = gather(.seed = true, .flip = false);
        const overridden = gather(.flip = true, .value = true);
        fn run() -> bool { written and not skipped and not overridden }";
    assert_eq!(sysroot::run(source), Value::Bool(true));
    let error = sysroot::compile(&source.replace("= 'z'", "= true")).unwrap_err();
    assert!(
        error.contains("expected (primitive char), found (primitive bool)"),
        "{error}"
    );
}

#[test]
fn explicit_operands_wait_for_default_dependent_comptime_domains() {
    let source = r#"
        comptime fn flag() -> bool { true }
        comptime fn select(.wide: bool = true,
            .enabled: bool = wide,
            .value: defer if enabled { ?bool } else { ?char } = false)
            -> defer if enabled { ?bool } else { ?char } {
            value
        }
        comptime fn widths(.wide: bool = true,
            ...fields: defer if wide { Tuple<u16, bool> } else { Tuple<u8, bool> }) -> bool {
            true
        }
        comptime fn describe(.pair: bool = true,
            ...fields: defer if pair { Tuple<Any, Any> } else { Tuple<char> }) -> bool { true }
        const described = describe(flag(), 255);
        const present = do { let item = select(.value = flag()); item matches true? };
        const pack = widths(256, flag());
        fn run() -> bool { present and pack and described }"#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
    let wrong_width = source.replace(
        "fn widths(.wide: bool = true",
        "fn widths(.wide: bool = false",
    );
    let error = sysroot::compile(&wrong_width).unwrap_err();
    assert!(error.contains("integer does not fit"), "{error}");
    let wrong_field = source.replace("select(.value = flag())", "select(.value = 'x')");
    let error = sysroot::compile(&wrong_field).unwrap_err();
    assert!(
        error.contains("expected (primitive bool), found (primitive char)"),
        "{error}"
    );
}

#[test]
fn pending_comptime_inputs_resume_selected_control_with_the_declared_width() {
    let source = "
        comptime fn enabled() -> bool { true }
        comptime fn fail() -> u16 { assumes false; 0 }
        comptime fn consume(.wide: bool = enabled(),
            .value: defer if wide { u16 } else { u8 } = 0)
            -> defer if wide { u16 } else { u8 } { value }
        comptime fn describe(.wide: bool = enabled(),
            ...fields: defer if wide { Tuple<Any> } else { Tuple<char> }) -> bool { true }
        const described = describe(if enabled() { 321 } else { 123 });
        const selected = consume(.value = if enabled() { 256 } else { fail() });
        const from_match = consume(.value = do {
            let value: u16 = 357;
            enabled() match { true => value, false => 1 }
        });
        fn run() -> bool { selected == 256 and from_match == 357 and described }";
    assert_eq!(sysroot::run(source), Value::Bool(true));
    let wrong_branch = source.replace("else { fail() }", "else { 65536 }");
    let error = sysroot::compile(&wrong_branch).unwrap_err();
    assert!(error.contains("integer does not fit"), "{error}");
}

#[test]
fn pending_comptime_selection_checks_aggregate_fields_and_preserves_aliases() {
    let source = "
        typealias Switch = %{flag: bool} comptime fn() -> bool;
        comptime fn toggle() -> Switch { var flag = false; || { flag = not flag; flag } }
        comptime fn branch() -> bool { true }
        comptime fn accept(.wide: bool = branch(), ...fields: defer if wide {
            Tuple<Tuple<u16, Switch, Switch>>
        } else { Tuple<Tuple<u8, Switch, Switch>> })
            -> defer if wide { Tuple<Tuple<u16, Switch, Switch>> } else { Tuple<Tuple<u8, Switch, Switch>> } { fields }
        const ok = do {
            let answer = accept(do {
                let flip = toggle();
                (if branch() { 511 } else { 0 }, flip, flip)
            });
            let ((width, first, second),) = answer;
            width == 511 and first() and not second()
        };
        fn run() -> bool { ok }";
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn literal_signature_inputs_leave_control_transfers_and_effects_in_the_caller() {
    let source = "
        comptime fn bad() -> bool { assumes false; true }
        comptime fn enabled() -> bool { true }
        comptime fn consume(.wide: bool = true,
            .item: defer if wide { u16 } else { u8 } = 0) -> bool { false }
        comptime fn unused() -> bool {
            consume(.item = if bad() { 512 } else { 0 })
        }
        comptime fn outer(flag: bool) -> bool {
            consume(.item = do {
                if flag { return true; }
                if enabled() { 256 } else { 0 }
            })
        }
        const ok = outer(true) and not outer(false);
        const skipped = if false { consume(.item = if bad() { 256 } else { 0 }) } else { true };
        fn run() -> bool { ok and skipped }";
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn unobserved_open_calls_do_not_start_contextual_producers() {
    // The general effectful default still needs the caller's VM continuation.
    // Reporting that boundary must not execute either the selector or default.
    let source = "
        comptime fn trap() -> bool { assumes false; true }
        comptime fn consume(.wide: bool = trap(),
            .item: defer if wide { u16 } else { u8 } = 0) -> bool { true }
        const answer = if false { consume(.item = if trap() { 256 } else { 0 }) } else { true };
        fn run() -> bool { answer }";
    let error = sysroot::compile(source).unwrap_err();
    assert!(
        error.contains("awaits control reaching its original comptime invocation"),
        "{error}"
    );
    assert!(!error.contains("ProgramContractRejected"), "{error}");
}

#[test]
fn selected_comptime_producers_publish_once_in_source_order() {
    let source = r#"
        comptime fn observed(name: Symbol, result: bool) -> bool {
            let generated = meta.StructBuilder.new(.name = name).Build();
            result
        }
        comptime fn consume(.wide: bool = observed(.DomainDefault, true),
            .value: defer if wide { ?bool } else { ?char } = true,
            .other: bool = false) -> bool { other }
        const ok = consume(.value = if observed(.Condition, true) {
            observed(.Selected, true)
        } else { observed(.Skipped, false); null }, .other = observed(.NextOperand, true));
        fn run() -> bool { ok }
    "#;
    let package = sysroot::compile(source).unwrap();
    let names = [
        "Condition",
        "Selected",
        "Skipped",
        "NextOperand",
        "DomainDefault",
    ];
    let mut observed: Vec<_> = package
        .structs
        .iter()
        .filter(|item| names.contains(&item.name.as_str()))
        .map(|item| (item.declaration, item.name.as_str()))
        .collect();
    observed.sort_by_key(|(id, _)| *id);
    assert_eq!(
        observed
            .into_iter()
            .map(|(_, name)| name)
            .collect::<Vec<_>>(),
        ["Condition", "Selected", "NextOperand", "DomainDefault"]
    );
}

#[test]
fn pending_variadic_operands_preserve_shared_closure_captures() {
    let source = "
        typealias Switch = %{flag: bool} comptime fn() -> bool;
        comptime fn toggle() -> Switch { var flag = false; || { flag = not flag; flag } }
        comptime fn pair() -> Tuple<Switch, Switch> { let flip = toggle(); (flip, flip) }
        comptime fn preserve(.two: bool = true,
            ...fields: defer if two { Tuple<Switch, Switch> } else { Tuple<Switch> })
            -> defer if two { Tuple<Switch, Switch> } else { Tuple<Switch> } { fields }
        const ok = do { let (first, second) = preserve(...pair()); first() and not second() };
        fn run() -> bool { ok }";
    assert_eq!(sysroot::run(source), Value::Bool(true));
}

#[test]
fn pending_operands_execute_before_an_omitted_comptime_default() {
    let source = "
        comptime fn bad_input() -> bool { assumes false; true }
        comptime fn bad_default() -> bool {
            let items: meta.List<bool> = [];
            items[0]
        }
        comptime fn accept(.selector: bool = bad_default(),
            ...fields: defer if selector { Tuple<bool> } else { Tuple<char> }) -> bool { true }
        const rejected = accept(bad_input());
        fn run() -> bool { rejected }";
    let error = sysroot::compile(source).unwrap_err();
    assert!(error.contains("ProgramContractRejected"), "{error}");
}

#[test]
fn rebound_comptime_callbacks_share_their_capture_graph_with_residual_defaults() {
    let source = "
        typealias Switch = %{flag: bool} comptime fn() -> bool;
        comptime fn make() -> Switch {
            var flag = false;
            || { flag = not flag; flag }
        }
        comptime fn sample(.first: Switch = make(), .second: Switch = make(),
            .enable: bool = do { first = second; true },
            .field: defer if enable { bool } else { char } = true,
            .via_first: bool = first(), .via_second: bool = second()) -> bool {
            via_first and not via_second and first() and second()
        }
        comptime fn check() -> bool { sample() }";
    check_events(source, &[("check", 1)]);
}

#[test]
fn projected_type_inputs_use_the_public_normal_form_interface() {
    let declarations = r#"
        struct Holder where T { pub typealias Payload = T; }
        comptime fn Make(T: Type) -> Type { Holder<T> }
        const Renamed: pure comptime fn<...Fields: meta.List<Type>> -> Type = Tuple;
        fn pair(comptime T: Type, value: defer T) -> defer T {
            const Container = Make(T);
            let both: Renamed<.Fields = [Container.Payload, bool]> = (value, true);
            let (answer, _) = both;
            answer
        }
    "#;
    let entry = r#"
        fn direct(comptime Container: Type, comptime T: Type, value: defer T) -> defer T {
            let both: Tuple<Container.Payload, bool> = (value, true);
            let (answer, _) = both;
            answer
        }
        fn run() -> bool {
            pair(bool, true) and pair(u32, 7) == 7 and direct(Holder<u32>, u32, 42) == 42
        }
    "#;
    assert_eq!(
        sysroot::run(&format!("{declarations}{entry}")),
        Value::Bool(true)
    );

    // The public name and the independent second element can already be
    // checked in an open definition, without an invocation of pair.
    for (before, after, message) in [
        (".Fields =", ".Es =", "unknown named normal-form argument"),
        (
            "Container.Payload, bool",
            "Container.Payload, true",
            "expected (primitive Type), found (primitive bool)",
        ),
    ] {
        let error = sysroot::compile(&declarations.replace(before, after)).unwrap_err();
        assert!(error.contains(message), "{error}");
    }
    let invalid = declarations.replace("Container.Payload", "Container.Missing");
    assert!(sysroot::compile(&format!("{invalid}{entry}")).is_err());
}

#[test]
fn const_opaque_heads_keep_projected_member_inputs_until_instantiation() {
    let source = r#"
        struct Holder where T { pub typealias Payload = T; }
        comptime fn Make(T: Type) -> Type { Holder<T> }
        pure comptime fn Mark<T, flag: bool> -> Type { T }
        fn check(comptime T: Type) -> bool {
            const C = Make(T);
            const actual = do {
                const inner = Mark<C.Payload, true>;
                inner
            };
            const expected = Mark<T, true>;
            const same = actual == expected;
            same
        }
        fn run() -> bool { check(bool) and check(u32) }
    "#;
    assert_eq!(sysroot::run(source), Value::Bool(true));
    let invalid = source.replace("const same =", "let wrong: actual = true; const same =");
    let diagnostic = sysroot::compile(&invalid).unwrap_err();
    assert!(diagnostic.contains("normal-form"), "{diagnostic}");
}

#[test]
fn only_normal_operand_exits_supply_the_original_comptime_call() {
    check_events(
        r#"
        comptime fn select(flag: bool) -> bool { flag }
        comptime fn accept(comptime ty: Type, value: ty) -> ty { value }
        comptime fn relay(flag: bool) -> bool {
            var trace: Integer = 0;
            let result = accept(
                if select(flag) { trace = trace + 1; return trace == 0; }
                else { trace = trace + 1; bool },
                do { trace = trace * 10 + 2; true }
            );
            result and trace == 12
        }
        comptime fn check() -> bool { relay(false) and not relay(true) }
        "#,
        &[("select", 2), ("accept", 1)],
    );
}

#[test]
fn matching_operand_exits_keep_guard_effects_and_reject_disagreement() {
    let definitions = r#"
        comptime fn select(flag: bool) -> bool { flag }
        comptime fn accept(comptime ty: Type, value: ty) -> ty { value }
        comptime fn relay(flag: bool) -> bool {
            var trace: Integer = 0;
            let result = accept(select(flag) match {
                true if do { trace = trace + 1; false } => bool,
                true => do { trace = trace + 2; bool },
                false => do { trace = trace + 3; bool },
            }, true);
            result and trace == 3
        }
        comptime fn check() -> bool { relay(false) and relay(true) }
    "#;
    check_events(definitions, &[("select", 2), ("accept", 2)]);
    let error = sysroot::compile(&definitions.replace("trace + 2; bool", "trace + 2; char"))
        .expect_err("inconsistent operand domains were accepted");
    assert!(error.contains("original comptime invocation"), "{error}");
}

#[test]
fn an_opaque_selector_cannot_preserve_a_type_from_before_its_write() {
    let failure = sysroot::compile(
        r#"
        comptime fn choose(flag: bool) -> bool { flag }
        comptime fn accept(comptime ty: Type, value: ty) -> ty { value }
        comptime fn check() -> bool {
            accept(do {
                var saved: Type = bool;
                if do { saved = char; choose(false) } { saved } else { saved }
            }, true)
        }
    "#,
    )
    .expect_err("a write in the selector cannot leave bool as the payload type");
    assert!(
        failure.contains("original comptime invocation") || failure.contains("type mismatch"),
        "{failure}"
    );
}
