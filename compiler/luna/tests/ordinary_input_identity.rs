//! Static input identity is evidence, not a replacement for its producer.
use comptime_interpretor::{Budget, Event, Interpreter, Value};
use elaboration::materialization::SourceInvocationQueries;
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;

fn check_both(source: &str, entered: usize) {
    let package = sysroot::compile(source).unwrap();
    let id = |name: &str| {
        package
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .declaration
    };
    let context = elaboration::ptc::Context::from_package(&package).unwrap();
    let queries = SourceInvocationQueries::new(&package, &context, &[]);
    let solver = |predicate: &nir::program::TypePredicate, fuel: &mut u64| {
        queries.check(predicate, Default::default(), fuel)
    };
    let methods = |callee: &nir::program::Expr, fuel: &mut u64| queries.resolve(callee, fuel);
    let associated = |access: &nir::program::Expr, fuel: &mut u64| queries.associated(access, fuel);
    for tree in [false, true] {
        let budget = Budget {
            steps: 1_000_000,
            depth: 128,
        };
        let mut vm = if tree {
            Interpreter::for_comptime_tree(&package, budget)
        } else {
            Interpreter::for_comptime(&package, budget)
        }
        .unwrap()
        .with_budgeted_type_solver(&solver)
        .with_method_resolver(&methods)
        .with_associated_resolver(&associated);
        for invocation in 1..=2 {
            assert_eq!(
                vm.invoke(id("check"), vec![]).unwrap(),
                Value::Bool(true),
                "tree={tree}"
            );
            assert_eq!(vm.events().iter().filter(|event| matches!(event, Event::EnterFunction(declaration) if *declaration == id("accept"))).count(), entered * invocation, "tree={tree}");
        }
    }
}

const FLAG: &str = "trait Flag {} impl Flag for bool {}";

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
        failure.contains("not available in this invocation") || failure.contains("not established"),
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
