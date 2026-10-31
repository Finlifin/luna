//! Open NF parameter domains retain head identity until value admission.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Budget, Interpreter, Value};

fn compile(body: &str) -> Result<nir::program::Package, String> {
    support::compile_with_source_bindings(
        &format!(
            "typealias Type=.Type'builtin;
        pure comptime fn List<T>->Type {{T}}
        pure comptime fn Tuple<...Es:List<Type>>->Type {{bool}}
        {body}"
        ),
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.List"),
            tuple: Some("input.Tuple"),
            ..Default::default()
        },
    )
}

#[test]
fn dependent_tuple_domains_accept_completed_values_without_unfolding_heads() {
    let package = compile(
        "pure comptime fn Key<Es:List<Type>, value:Tuple<.Es=Es>>->Type {bool}
        comptime fn Build()->Type {Key<[bool],(true,)>}",
    )
    .unwrap();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let result = vm.invoke(support::id(&package, "Build"), vec![]).unwrap();
    assert!(
        matches!(result, Value::Type(nir::Term::NormalForm {head,..})
        if head == support::id(&package,"Key"))
    );
    let mut tree = Interpreter::for_comptime_tree(&package, Budget::default()).unwrap();
    assert_eq!(
        tree.invoke(support::id(&package, "Build"), vec![]).unwrap(),
        result
    );
}

#[test]
fn user_head_domains_can_be_declared_without_exposing_a_value_layout() {
    compile(
        "pure comptime fn Identity<T>->Type {T}
        pure comptime fn Key<T, value:Identity<T>>->Type {T}",
    )
    .unwrap();
    assert!(
        compile(
            "pure comptime fn Identity<T>->Type {T}
        pure comptime fn Key<T, value:Identity<T>>->Type {T}
        comptime fn Bad()->Type {Key<bool,true>}"
        )
        .is_err()
    );
}

#[test]
fn dependent_domains_check_each_completed_input() {
    for (types, value) in [
        ("[bool]", "(true,false)"),
        ("[bool]", "(7,)"),
        ("[bool]", "[true]"),
    ] {
        let source = format!(
            "pure comptime fn Key<Es:List<Type>, value:Tuple<.Es=Es>>->Type {{bool}}
            comptime fn Bad()->Type {{Key<{types},{value}>}}"
        );
        assert!(compile(&source).is_err(), "accepted {source}");
    }
    for (types, value) in [("[]", "()"), ("[bool,bool]", "(true,false)")] {
        let package = compile(&format!(
            "pure comptime fn Key<Es:List<Type>, value:Tuple<.Es=Es>>->Type {{bool}}
            comptime fn Build()->Type {{Key<{types},{value}>}}"
        ))
        .unwrap();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert!(matches!(
            vm.invoke(support::id(&package, "Build"), vec![]).unwrap(),
            Value::Type(_)
        ));
    }
}
