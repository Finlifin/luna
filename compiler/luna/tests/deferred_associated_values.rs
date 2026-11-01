//! Associated values selected after ordinary Type inputs become available.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

const SOURCE: &str = "typealias Type=.Type'builtin; typealias char=.char'builtin;
    trait Flag { assoc enabled:bool; }
    impl Flag for bool { assoc enabled:bool=true; }
    impl Flag for char { assoc enabled:bool=false; }
    comptime fn enabled(comptime T:Type)->bool
    where requires T :- Flag
    { T.as(Flag).enabled }
    comptime fn run()->bool { enabled(bool) and not enabled(char) and enabled(bool) }";

#[test]
fn ordinary_type_inputs_select_associated_values_during_invocation() {
    let package = support::compile_with_protocols(SOURCE, &Default::default()).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn associated_contracts_reject_missing_or_wrong_sort_caller_binders() {
    for (index, sort) in [(1, nir::Sort::Type), (0, nir::Sort::ComptimeValue)] {
        let mut package = support::compile_with_protocols(SOURCE, &Default::default()).unwrap();
        let function = package
            .functions
            .iter_mut()
            .find(|f| f.name == "enabled")
            .unwrap();
        let mut changed = false;
        function
            .body
            .rewrite_types(&mut |ty| {
                Ok::<_, ()>(if matches!(ty, nir::Term::Bound { .. }) {
                    changed = true;
                    nir::Term::Bound {
                        depth: 0,
                        index,
                        sort,
                    }
                } else {
                    ty.clone()
                })
            })
            .unwrap();
        assert!(changed);
        let failure = nir::phases::validate(&package).unwrap_err();
        assert_eq!(
            failure.message,
            "associated value contract substitution failed"
        );
    }
}

#[test]
fn associated_domains_keep_ordinary_interface_argument_scope() {
    let source = "typealias Type=.Type'builtin;
        trait Source where U { assoc initial:U; }
        impl Source<bool> for bool { assoc initial:bool=true; }
        comptime fn initial(comptime T:Type)->T
        where requires bool :- Source<T>
        { bool.as(Source<T>).initial }
        comptime fn run()->bool { initial(bool) }";
    let package = support::compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn required_computations_select_associated_values_after_type_binding() {
    let source = include_str!("../../../development/repros/deferred-associated-required.fl");
    let package = support::compile_with_protocols(source, &Default::default()).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "result", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn demanded_associated_initializers_execute_in_the_required_world() {
    let source = include_str!("../../../development/repros/deferred-associated-required.fl")
        .replace("assoc enabled:bool=true;", "assoc enabled:bool=initial();")
        + "pure comptime fn initial()->bool { true }";
    let package = support::compile_with_protocols(&source, &Default::default()).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "result", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn required_type_results_preserve_selected_associated_types() {
    let source = "typealias Type=.Type'builtin;
        trait Shape { assoc Item:Type; }
        impl Shape for bool { assoc Item:Type=bool; }
        comptime fn Selected(comptime T:Type)->Type
        where requires T :- Shape
        { T.as(Shape).Item }
        typealias SelectedBool=Selected(bool);
        comptime fn run()->Type { SelectedBool }";
    for source in [
        source.to_owned(),
        source.replace("assoc Item:Type=bool;", "assoc Item:Type=Initialized();")
            + "pure comptime fn Initialized()->Type {bool}",
    ] {
        let package = support::compile_with_protocols(&source, &Default::default()).unwrap();
        assert_eq!(
            luna::invocation::invoke(&package, "run", 100_000).unwrap(),
            Value::Type(nir::Term::Primitive(nir::Primitive::Bool))
        );
    }
}
