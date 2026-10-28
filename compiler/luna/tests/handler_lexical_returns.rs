//! Separate installer-relative captures from request-relative lexical results.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

const PREFIX: &str = "
    trait Duplicate {} trait Retire {fn drop(*self);}
    impl Duplicate for bool {} impl Duplicate for *T where T {}
    effect signal()->bool;
    effect relay(value:*bool)->bool;
";

fn execute(package: &nir::program::Package) -> Result<Value, String> {
    luna::execution::interpret_with_protocols(package, "run", "Duplicate", "Retire", 1_000_000)
}

#[test]
fn lexical_capture_returns_preserve_installer_lifetime() {
    let select = "fn select(pointer:*bool)->*bool {
        signal()# {signal()=>{return pointer;},};pointer
    }";
    let package = support::compile(&format!(
        "{PREFIX}{select}
        fn run()->bool {{var local=false;select(local.ref).* = true;local}}"
    ));
    assert_eq!(execute(&package).unwrap(), Value::Bool(true));
    let dangling = support::compile(&format!(
        "{PREFIX}{select}
        fn run()->*bool {{var local=false;select(local.ref)}}"
    ));
    assert!(
        execute(&dangling).is_err(),
        "installer-local storage must not escape"
    );
}

#[test]
fn request_pointer_lexical_returns_require_substitution_before_admission() {
    for (arm, unsupported) in [
        (
            "return value;",
            "pointer target provenance is not established",
        ),
        (
            "let observed=value.*;return value;",
            "unknown memory target",
        ),
    ] {
        let select = format!(
            "fn select(pointer:*bool)->*bool {{
            relay(pointer)# {{relay(value)=>{{{arm}}},}};pointer
        }}"
        );
        let source = format!(
            "{PREFIX}{select}
            fn run()->bool {{var local=false;select(local.ref).* = true;local}}"
        );
        let package = support::compile(&source);
        // This is a recorded implementation gap, not a source-language ban.
        // Remove the refusal assertion when request-side substitution is wired
        // through calls and lexical exits; never weaken the dangling check.
        let failure = execute(&package).unwrap_err();
        assert!(failure.contains(unsupported), "{failure}");
        let mut expanded = support::compile(&source.replace("effect relay", "inline effect relay"));
        let report = elaboration::program::inlining::expand(&mut expanded, &mut 1_000_000).unwrap();
        assert!(
            report.expanded_handlers > 0 && report.expanded_requests > 0,
            "{report}"
        );
        assert_eq!(execute(&expanded).unwrap(), Value::Bool(true));

        let mut dangling = support::compile(
            &format!(
                "{PREFIX}{select}
            fn run()->*bool {{var local=false;select(local.ref)}}"
            )
            .replace("effect relay", "inline effect relay"),
        );
        elaboration::program::inlining::expand(&mut dangling, &mut 1_000_000).unwrap();
        assert!(
            execute(&dangling).is_err(),
            "expansion must retain pointer lifetime checks"
        );
    }
}
