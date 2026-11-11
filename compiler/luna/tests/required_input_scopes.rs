//! Nested type formation observes supplied clause inputs in their owning region.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;
use comptime_interpretor::Value;

const SOURCE: &str = "
    struct Flag where enabled: bool {}
    comptime fn selected() -> <|-bool>
    where flag: bool, .witness: Flag<flag> = (Flag<flag>{})
    {
        if flag {'(true)} else {'(false)}
    }
    fn run() -> bool {
        const yes = selected<true>();
        const no = selected<false>();
        const again = selected<true>();
        ~<yes> and not ~<no> and ~<again>
    }
";

#[test]
fn nested_required_inputs_remain_separate_across_instances() {
    assert_eq!(sysroot::run(SOURCE), Value::Bool(true));
}

#[test]
fn nested_required_calls_capture_supplied_inputs_for_helpers() {
    let source = SOURCE
        .replace(
            "struct Flag",
            "pure comptime fn same(value: bool) -> bool { value }\nstruct Flag",
        )
        .replace("= (Flag<flag>{})", "= (Flag<same(flag)>{})");
    assert_eq!(sysroot::run(&source), Value::Bool(true));
}

#[test]
fn nested_required_domains_reject_wrong_and_unavailable_inputs() {
    for source in [
        SOURCE.replace("= (Flag<flag>{})", "= (Flag<not flag>{})"),
        SOURCE.replace(
            "flag: bool, .witness: Flag<flag> = (Flag<flag>{})",
            ".witness: Flag<flag> = (Flag<flag>{}), flag: bool",
        ),
        SOURCE
            .replace("fn run() -> bool", "fn run(value: bool) -> bool")
            .replace("selected<true>()", "selected<value>()"),
    ] {
        assert!(sysroot::compile(&source).is_err());
    }
    assert_eq!(sysroot::run(SOURCE), Value::Bool(true));
}

#[test]
fn ordinary_functions_use_the_same_input_scope() {
    let ordinary = SOURCE
        .replace("-> <|-bool>", "-> bool")
        .replace("if flag {'(true)} else {'(false)}", "flag")
        .replace(
            "~<yes> and not ~<no> and ~<again>",
            "yes and not no and again",
        );
    assert_eq!(sysroot::run(&ordinary), Value::Bool(true));
}

#[test]
fn library_collectors_use_the_same_input_scope() {
    let collected = SOURCE.replace(
        ".witness: Flag<flag> = (Flag<flag>{})",
        ".witness: meta.List<Flag<flag>> = [Flag<flag>{}]",
    );
    assert_eq!(sysroot::run(&collected), Value::Bool(true));
}

#[test]
fn forwarded_generic_clauses_keep_their_symbolic_identity() {
    let source = SOURCE
        .replace(
            "fn run()",
            "comptime fn forward() -> <|-bool> where value: bool { selected<value>() }\nfn run()",
        )
        .replace("selected<true>()", "forward<true>()")
        .replace("selected<false>()", "forward<false>()");
    assert_eq!(sysroot::run(&source), Value::Bool(true));
}

#[test]
fn observed_clause_identities_do_not_erase_input_effects() {
    assert_eq!(
        sysroot::run(
            "
        struct Flag where enabled: bool {}
        comptime fn select() -> bool
        where flag: bool, .witness: Flag<flag> = (Flag<flag>{})
        {
            flag
        }
        comptime fn check() -> bool {
            var ticks: Integer = 0;
            let value = select<do { ticks = ticks + 1; true }>();
            value and ticks == 1
        }
        fn run() -> bool { const value = check(); value }
    "
        ),
        Value::Bool(true)
    );
}
