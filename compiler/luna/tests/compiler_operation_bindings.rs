//! Compiler operation bindings preserve declaration resolution and call checks.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

fn compile(source: &str) -> Result<nir::program::Package, String> {
    support::compile_with_source_bindings(
        &format!(
            "typealias Type=.Type'builtin;typealias Integer=.Integer'builtin;pure comptime fn List<T>->Type {{T}} {source}"
        ),
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.List"),
            ..Default::default()
        },
    )
}

#[test]
fn compiler_operations_follow_imports_and_constant_aliases() {
    let package=compile(
        "mod operations {pub const length=.list_length'builtin;pub const concat=.list_concat'builtin;}
        use operations.length as size;
        const count=size;
        comptime fn run()->bool {
            let a:List<bool> = [true];let b:List<bool> = [false,true];
            let joined=operations.concat(a,b);
            count(joined) == 3 and joined[0] and not joined[1] and joined[2]
        }"
    ).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn compiler_operation_arguments_keep_source_order_and_single_evaluation() {
    let package=compile(
        "const concat=.list_concat'builtin;const length=.list_length'builtin;
        comptime fn input(trace:*Integer,digit:Integer)->List<bool> {trace.* = trace.* * 10 + digit;[true]}
        comptime fn run()->bool {var trace:Integer=0;let values=concat(input(trace.ref,1),input(trace.ref,2));trace == 12 and length(values) == 2}"
    ).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn compiler_operation_bindings_retain_arity_domain_and_phase_errors() {
    for (body, diagnostic) in [
        (
            "comptime fn bad() {let xs:List<bool> = [];length(xs,xs);}",
            "positional list operands",
        ),
        ("comptime fn bad() {length(true);}", "bound meta.List"),
        (
            "fn bad()->Integer {divide(4,2)}",
            "requires comptime invocation",
        ),
        (
            "comptime fn bad() {let escaped=length;}",
            "requires a direct call",
        ),
    ] {
        let error=compile(&format!("const length=.list_length'builtin;const divide=.comptime_integer_div_trunc'builtin;{body}")).unwrap_err();
        assert!(error.contains(diagnostic), "{body}: {error}");
    }
    assert!(
        compile("const a=b;const b=a;")
            .unwrap_err()
            .contains("cyclic")
    );
    assert!(compile("const ordinary=3;comptime fn run()->bool {ordinary == 3}").is_ok());
}

#[test]
fn compiler_operation_binding_can_supply_type_observation_without_replaying_calls() {
    let package=compile(
        "const concat=.list_concat'builtin;const length=.list_length'builtin;
        comptime fn Other()->Type {Integer}
        comptime fn run()->bool {let a:List<Type> = [Other()];let b:List<Type> = [bool];let joined=concat(a,b);let T=joined[length(a)];let value:T=true;value}"
    ).unwrap();
    let mut vm =
        comptime_interpretor::Interpreter::for_comptime(&package, Default::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        support::calls(&package, &vm)
            .iter()
            .filter(|name| **name == "Other")
            .count(),
        1
    );
}

#[test]
fn ordinary_callable_parameters_shadow_operation_bindings() {
    let package = compile(
        "const length=.list_length'builtin;
        comptime fn negate(value:bool)->bool {not value}
        comptime fn apply(length:comptime fn(bool)->bool)->bool {length(false)}
        comptime fn run()->bool {apply(negate)}",
    )
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    let mut source = "const op0=.list_length'builtin;".to_owned();
    for index in 1..=65 {
        source.push_str(&format!("const op{index}=op{};", index - 1));
    }
    assert!(compile(&source).unwrap_err().contains("excessively deep"));
}
