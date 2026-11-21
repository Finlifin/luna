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

const DEPENDENT_SUBJECT: &str = "
comptime fn Selected(flag: bool) -> Type { if flag { bool } else { char } }
fn empty(comptime ty: Type) -> ?ty { null }
fn forwarded() -> bool where flag: bool { from_parameter(flag) }
fn discarded(comptime flag: bool) -> bool { let unused = empty(Selected(flag)); true }
fn from_clause() -> bool where flag: bool {
    empty(Selected(flag)) match { null => true, _ => false }
}
fn from_parameter(comptime flag: bool) -> bool {
    empty(Selected(flag)) match { null => true, _ => false }
}
";

#[test]
fn computed_type_operands_remain_abstract_until_their_enclosing_call() {
    let source = format!(
        "{DEPENDENT_SUBJECT}
        fn run() -> bool {{
            from_clause<true>() and from_clause<false>()
                and from_parameter(true) and from_parameter(false)
                and forwarded<true>() and forwarded<false>() and discarded(false)
        }}"
    );
    let package = sysroot::compile(&source).unwrap();
    let graph = luna::instantiation::instantiate(&package, "run", 5_000_000).unwrap();
    let domains: std::collections::HashSet<_> = graph
        .functions
        .iter()
        .filter(|instance| instance.function.name == "empty")
        .map(|instance| instance.key.arguments.to_vec())
        .collect();
    assert_eq!(
        domains,
        [nir::Primitive::Bool, nir::Primitive::Char]
            .into_iter()
            .map(|p| vec![nir::Term::Primitive(p)])
            .collect()
    );
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn abstract_type_operands_still_check_unused_definitions() {
    for (source, expected) in [
        (
            DEPENDENT_SUBJECT.replace(", _ => false", ""),
            "non-exhaustive",
        ),
        (
            DEPENDENT_SUBJECT.replace("_ => false", "payload? => not payload"),
            "no-applicable-candidate",
        ),
        (
            DEPENDENT_SUBJECT.replace("from_parameter(comptime flag", "from_parameter(flag"),
            "runtime",
        ),
    ] {
        let error = sysroot::compile(&source)
            .err()
            .expect("invalid open body accepted");
        assert!(
            error.contains(expected) && !error.contains("AbstractValueUnresolved"),
            "{error}"
        );
    }
}
