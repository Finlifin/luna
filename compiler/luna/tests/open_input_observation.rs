//! Inference support follows actual unknown operands without inventing evidence.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;

use comptime_interpretor::Value;

const RUNTIME: &str = "
comptime fn Pick(flag: bool) -> Type { if flag { bool } else { u8 } }
comptime fn Select(flag: bool, T: Type) -> Type { if flag { T } else { bool } }
fn inner(comptime flag: bool, value: T) -> defer Select(flag, V)
where T, .U: Type = T, .V: Type = U
{
    value
}
fn outer(comptime flag: bool, value: defer Pick(flag)) -> defer Pick(flag) {
    inner(true, value)
}
fn run() -> bool { outer(true, true) and outer(false, 9) == 9 }
";

const COMPTIME: &str = "
comptime fn Domain(flag: bool) -> Type { if flag { Type } else { bool } }
comptime fn Identity(T: Type) -> Type { T }
comptime fn retain(value: T) -> T
where T, .U: Type = Identity(T), .V: Type = Identity(U)
{
    value
}
fn choose(comptime flag: bool, comptime seed: defer Domain(flag),
    value: defer do { let saved = retain(seed); retain(saved) })
    -> defer do { let saved = retain(seed); retain(saved) }
{
    value
}
fn run() -> bool { choose(true, bool, true) and choose(true, u8, 9) == 9 }
";

#[test]
fn unknown_operand_support_survives_default_chains_and_distinct_instances() {
    for source in [RUNTIME, COMPTIME] {
        let package = sysroot::compile(source).unwrap();
        assert_eq!(
            luna::execution::interpret(&package, "run", 200_000).unwrap(),
            Value::Bool(true)
        );
        let graph = luna::instantiation::instantiate(&package, "run", 200_000).unwrap();
        let mut vm =
            comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
        assert_eq!(
            vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
            Value::Bool(true)
        );
    }
}

#[test]
fn unknown_operand_support_cannot_seed_unrelated_declaration_inputs() {
    for source in [RUNTIME, COMPTIME] {
        let source = source.replace("where T,", "where T, Phantom,");
        let error = sysroot::compile(&source)
            .err()
            .expect("phantom input accepted");
        assert!(error.contains("inference evidence"), "{error}");
    }
}

#[test]
fn pending_inference_still_checks_operand_contents() {
    for source in [
        RUNTIME.replace("inner(true, value)", "inner(true, value.missing)"),
        COMPTIME.replace("retain(seed)", "retain(seed.missing)"),
    ] {
        assert!(
            sysroot::compile(&source).is_err(),
            "unchecked pending operand accepted"
        );
    }
}
