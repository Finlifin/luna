//! Required body computations wait for declaration-clause inputs.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

fn compile(source: &str) -> Result<nir::program::Package, String> {
    support::compile_with_source_bindings(
        &format!(
            "mod meta {{{}}} {source}",
            include!("support/meta_source.rs")
        ),
        &support::meta_fixture_protocols(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.meta.List"),
            ..Default::default()
        },
    )
}

#[test]
fn local_constants_specialize_separate_clause_instances_before_runtime_storage() {
    let source = "fn selected()->bool where flags:meta.List<bool> {
            const flag=flags[0]; flag
        }
        fn scalar()->bool where input:bool {const flag=not input; flag}
        fn run()->bool {
            let first=selected<[true,false]>;
            let second=selected<[false,true]>;
            first() and not second() and scalar<false>() and not scalar<true>()
        }";
    let package = compile(source).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    for invalid in [
        source.replace("selected<[true,false]>", "selected<[]>"),
        source.replace("const flag=flags[0]", "const flag=flags[9]"),
        "fn selected(value:bool)->bool where flag:bool {const x=value; x}
         fn run()->bool {selected<true>(true)}"
            .into(),
    ] {
        assert!(compile(&invalid).is_err());
    }
}

#[test]
fn generic_type_constants_keep_existing_receiver_and_comptime_paths() {
    let source = "struct Holder where T {}
        impl Holder<T> where T {fn keep(value:bool)->bool {const Item=T; value}}
        comptime fn invert()->bool where flag:bool {const result=not flag; result}
        fn run()->bool {Holder<bool>.keep(true)}";
    let package = compile(source).unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}
