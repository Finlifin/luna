//! Native optional layers preserve code identity without granting purity by type.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

fn compile(source: &str) -> Result<nir::program::Package, String> {
    support::compile_with_source_bindings(
        &format!("trait Copy {{}} trait Drop {{fn drop(*self);}} {source}"),
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            copy: Some("input.Copy"),
            drop: Some("input.Drop"),
            ..Default::default()
        },
    )
}

#[test]
fn optional_callbacks_keep_construction_matching_and_loop_evidence() {
    for body in [
        "let action: ?Action = invert; action match { f? => f(false), null => false }",
        "let action = make(); action match { f? => f(false), null => false }",
        "let action: ?Action = null; action match { f? => f(false), null => true }",
        "let inner: ?Action = null; let outer: ??Action = inner; outer match { f?? => f(false), null? => true, null => false }",
        "var action: ?Action = keep; var again = true; while again { action = invert; again = false; } action match { f? => f(false), null => false }",
    ] {
        let source = format!(
            "typealias Action=fn(bool)->bool; fn keep(value:bool)->bool {{value}} fn invert(value:bool)->bool {{not value}} fn make()->?Action {{invert}} pure fn run()->bool {{{body}}}"
        );
        let package = compile(&source).unwrap_or_else(|error| panic!("{body}: {error}"));
        assert_eq!(
            luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 500_000)
                .unwrap(),
            Value::Bool(true),
            "{body}",
        );
    }
}

#[test]
fn optional_callback_identity_does_not_hide_impure_or_unknown_calls() {
    for body in [
        "let action: ?Action = bad; action match { f? => f(pointer), null => true }",
        "let action: ?Action = unknown; action match { f? => f(pointer), null => true }",
        "let action = factory(pointer); action match { f? => f(pointer), null => true }",
        "var action: ?Action = good; var again = true; while again { action = bad; again = false; } action match { f? => f(pointer), null => true }",
    ] {
        let source = format!(
            "typealias Action=fn(*bool)->bool; fn good(pointer:*bool)->bool {{true}} fn bad(pointer:*bool)->bool {{pointer.*}} fn factory(pointer:*bool)->?Action {{pointer.* = true; good}} pure fn run(pointer:*bool,unknown:fn(*bool)->bool)->bool {{{body}}}"
        );
        let error = compile(&source).unwrap_err();
        assert!(error.contains("P4 evidence"), "{body}: {error}");
    }
}
