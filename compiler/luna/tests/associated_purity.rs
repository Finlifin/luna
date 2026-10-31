//! Source and host-pipeline coverage for scalar associated values in P4 code.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Budget, Interpreter, Value};

const SOURCE: &str = "
    typealias Char=.char'builtin;
    trait Flag {assoc enabled:bool=true;}
    impl Flag for bool {}
    impl Flag for Char {assoc enabled:bool=false;}
    pure fn enabled()->bool where T:-Flag {T.as(Flag).enabled}
    pure fn branch()->bool {
        if bool.as(Flag).enabled {not Char.as(Flag).enabled} else {false}
    }
    pure fn run()->bool {enabled<bool>() and not enabled<Char>() and branch()}
";

#[test]
fn pure_associated_defaults_overrides_and_generic_guards_execute_in_iir() {
    let package = support::compile(SOURCE);
    for function in &package.functions {
        assert!(function.pure);
        assert_eq!(function.phase, nir::program::FunctionPhase::Runtime);
    }
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = Interpreter::from_iir(&graph, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn associated_observations_do_not_authorize_external_writes_or_wrong_domains() {
    let writes = format!(
        "{SOURCE} pure fn bad(flag:bool,p:*bool)->bool {{
        if flag and bool.as(Flag).enabled {{p.* = false;}} true
    }}"
    );
    for (source, purity_failure) in [
        (writes, true),
        (
            SOURCE.replace("assoc enabled:bool=false;", "assoc enabled:bool='x';"),
            false,
        ),
    ] {
        let failure = support::compile_with_protocols(&source, &Default::default()).unwrap_err();
        if purity_failure {
            assert!(failure.contains("P4 evidence"), "{failure}");
            support::compile(&source.replace("pure fn bad", "fn bad"));
        }
        let package = support::compile(SOURCE);
        assert_eq!(
            luna::execution::interpret(&package, "run", 100_000).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn pure_associated_values_execute_through_llvm_at_o0_and_o3() {
    use std::os::unix::fs::PermissionsExt;
    let package = support::compile(SOURCE);
    let ir = luna::host::llvm_ir(&package, "run", 100_000).unwrap();
    let directory =
        std::env::temp_dir().join(format!("luna-associated-purity-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let executable = directory.join("run");
    for optimization in [
        luna::host::OptimizationLevel::O0,
        luna::host::OptimizationLevel::O3,
    ] {
        let binary = luna::host::executable(
            &ir,
            &luna::host::Toolchain {
                optimization,
                ..Default::default()
            },
        )
        .unwrap();
        std::fs::write(&executable, binary).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let output = std::process::Command::new(&executable).output().unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(output.stdout, b"(bool true)\n");
    }
    std::fs::remove_dir_all(directory).unwrap();
}
