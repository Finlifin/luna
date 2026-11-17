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
fn unobserved_default_dependencies_report_the_missing_continuation() {
    let failure = sysroot::compile(&format!(
        "{FLAG}
        comptime fn mark() -> bool {{ true }}
        comptime fn accept(comptime .T: Type = bool,
            comptime .U: Type = T,
            comptime .V: Type = do {{ mark(); U }}, value: V) -> bool
        where requires V :- Flag
        {{ true }}
        comptime fn check() -> bool {{ accept(true) }}"
    ))
    .unwrap_err();
    assert!(
        failure.contains("dependent call input needs its original comptime invocation"),
        "{failure}"
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
#[ignore = "unknown default prefixes need source execution continuation, not eager freezing"]
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
