//! Branch joins retain provenance only for capture slots that may still exist.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::Value;

#[test]
fn conditionally_consumed_capture_is_cleaned_once_on_either_branch() {
    for conditional in ["if flag {callback();}", "if flag {} else {callback();}"] {
        let source = format!(
            "trait Duplicate {{}} trait Retire {{fn drop(*self);}}
             impl Duplicate for bool {{}} impl Duplicate for *T where T {{}}
             struct Watch {{target:*bool}}
             impl Retire for Watch {{fn drop(*self) {{
                 assumes not self.target.*.*;
                 self.target.*.* = true;
             }}}}
             fn consume(value:Watch) {{}}
             fn choose(flag:bool,target:*bool) {{
                 let watch=Watch{{target:target}};
                 let callback=|| {{consume(watch);}};
                 {conditional}
             }}
             fn run()->bool {{
                 var left=false;var right=false;
                 choose(true,left.ref);choose(false,right.ref);
                 left and right
             }}
             fn verify() {{let passed=run();assumes passed;}}"
        );
        let package = support::compile(&source);
        assert_eq!(
            luna::execution::interpret_with_protocols(
                &package,
                "run",
                "Duplicate",
                "Retire",
                1_000_000,
            )
            .unwrap(),
            Value::Bool(true)
        );
        let copy = package
            .traits
            .iter()
            .find(|item| item.name == "Duplicate")
            .unwrap()
            .declaration;
        let drop = package
            .traits
            .iter()
            .find(|item| item.name == "Retire")
            .unwrap()
            .declaration;
        let prepared =
            luna::host::prepare_with_protocols(&package, "verify", 1_000_000, copy, drop).unwrap();
        let ir = luna::host::llvm_ir_prepared(&prepared, 1_000_000).unwrap();
        for optimization in [
            luna::host::OptimizationLevel::O0,
            luna::host::OptimizationLevel::O3,
        ] {
            luna::host::Toolchain {
                optimization,
                ..Default::default()
            }
            .execute_test(&ir, std::time::Duration::from_secs(5))
            .unwrap();
        }
        let invalid = source.replace(conditional, &format!("{conditional} callback();"));
        let invalid = support::compile(&invalid);
        let error = luna::execution::interpret_with_protocols(
            &invalid,
            "run",
            "Duplicate",
            "Retire",
            1_000_000,
        )
        .unwrap_err();
        assert!(error.contains("unavailable"), "{error}");
    }
}
