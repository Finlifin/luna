//! Shipped text methods execute through ordinary comptime control flow.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Budget, Interpreter, Value};

#[test]
fn text_search_agrees_with_reference_for_boundaries_utf8_and_overlap() {
    let source = format!(
        "{}{}",
        include!("support/text_source.rs"),
        r#"
        pure comptime fn check(haystack: str, needle: str, expected: u64) -> bool {
            haystack.contains(needle) and (haystack.find(needle) match {
                offset? => offset == expected,
                null => false,
            })
        }
        pure comptime fn run() -> bool {
            "abc".starts_with("ab") and not "abc".starts_with("bc")
                and "abc".ends_with("bc") and not "abc".ends_with("ab")
                and "".starts_with("") and "".ends_with("")
                and "abc".starts_with("") and "abc".ends_with("")
                and not "a".starts_with("ab") and not "a".ends_with("ba")
                and check("", "", 0) and check("abc", "", 0)
                and check("abc", "abc", 0) and check("ababa", "aba", 0)
                and check("aaaab", "aab", 2) and check("abc", "c", 2)
                and check("é🦀a", "🦀", 2) and check("é🦀a", "a", 6)
                and "é🦀".starts_with("é") and "é🦀".ends_with("🦀")
                and not "é".contains("é")
                and not "abc".contains("abcd") and ("".find("a") matches null)
                and ("abc".find("d") matches null)
        }
        comptime fn order() -> bool {
            var trace: u64 = 0;
            let found = do { trace = trace * 10 + 1; "abc" }.find(
                do { trace = trace * 10 + 2; "b" });
            trace == 12 and (found matches 1?)
        }
        "#
    );
    let package =
        support::compile_with_protocols(&source, &support::meta_fixture_protocols()).unwrap();
    let mut bytecode = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut reference = Interpreter::for_comptime_tree(&package, Budget::default()).unwrap();
    for name in ["run", "order"] {
        let id = support::id(&package, name);
        assert_eq!(
            bytecode.invoke(id, vec![]).unwrap(),
            Value::Bool(true),
            "{name}"
        );
        assert_eq!(
            reference.invoke(id, vec![]).unwrap(),
            Value::Bool(true),
            "{name}"
        );
    }
    assert!(bytecode.bytecode_invocations() > 2);
    assert_eq!(reference.bytecode_invocations(), 0);
}
