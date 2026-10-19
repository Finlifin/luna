//! Contextual meta.List<Type> construction retains expression evaluation.
#[path = "support/execution.rs"]
mod support;
use comptime_interpretor::{Budget, Interpreter, Value};
use nir::{Primitive as P, Term};

const PREFIX: &str = "typealias Type=.Type'builtin;pure comptime fn List<T>->Type {T}";

fn compile(body: &str) -> Result<nir::program::Package, String> {
    support::compile_with_source_bindings(
        &format!("{PREFIX}{body}"),
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input::List"),
            ..Default::default()
        },
    )
}

#[test]
fn type_list_literals_construct_empty_and_ordered_values() {
    for (body, types) in [
        ("[]", vec![]),
        ("[bool,.u8'builtin,bool]", vec![P::Bool, P::U8, P::Bool]),
    ] {
        let package = compile(&format!("comptime fn make()->List<Type> {{{body}}}")).unwrap();
        assert!(package.to_string().contains("(type-list (elements"));
        assert_eq!(
            luna::invocation::invoke(&package, "make", 100_000).unwrap(),
            Value::TypeList(types.into_iter().map(Term::Primitive).collect())
        );
    }
}

#[test]
fn type_list_elements_execute_once_and_stop_at_return() {
    let package = compile(
        "comptime fn First()->Type {bool}
        comptime fn Second()->Type {.u8'builtin}
        comptime fn make()->List<Type> {[First(),Second(),First()]}
        comptime fn early()->List<Type> {[First(),do {return [];},Second()]}",
    )
    .unwrap();
    for (entry, expected, calls) in [
        (
            "make",
            vec![P::Bool, P::U8, P::Bool],
            vec!["make", "First", "Second", "First"],
        ),
        ("early", vec![], vec!["early", "First"]),
    ] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, entry), vec![]).unwrap(),
            Value::TypeList(expected.into_iter().map(Term::Primitive).collect())
        );
        assert_eq!(support::calls(&package, &vm), calls);
    }
}

#[test]
fn type_list_literal_requires_type_elements_and_bound_domain() {
    for source in [
        "comptime fn bad()->List<Type> {[true]}",
        "comptime fn bad()->List<Type> {[7]}",
        "comptime fn bad()->List<bool> {[true]}",
        "comptime fn bad()->(Type,Type) {[bool,bool]}",
        "fn bad()->List<Type> {[bool]}",
        "fn runtime()->Type {bool} comptime fn bad()->List<Type> {[runtime()]}",
    ] {
        assert!(compile(source).is_err(), "{source}");
    }
    assert!(
        support::compile_with_protocols(
            &format!("{PREFIX}comptime fn bad()->List<Type> {{[bool]}}"),
            &Default::default()
        )
        .is_err()
    );
}

#[test]
fn type_list_literals_keep_generic_elements_constants_and_indexing() {
    let package = compile(
        "comptime fn collect(T:Type)->List<Type> {[T,bool]}
        comptime fn generic()->List<Type> where T {[T,bool]}
        const fixed:List<Type> = [bool,.u8'builtin];
        comptime fn Selected()->Type {let values=collect(.u8'builtin);values[0]}
        comptime fn GenericSelected()->Type {let values=generic<.u8'builtin>();values[0]}
        comptime fn ConstantSelected()->Type {fixed[1]}
        comptime fn nested(T:Type)->List<Type> {const values:List<Type> = [T,bool];values}",
    )
    .unwrap();
    for name in ["Selected", "GenericSelected", "ConstantSelected"] {
        assert_eq!(
            luna::invocation::invoke(&package, name, 100_000).unwrap(),
            Value::Type(Term::Primitive(P::U8))
        );
    }
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(
            support::id(&package, "nested"),
            vec![Value::Type(Term::Primitive(P::U8))]
        )
        .unwrap(),
        Value::TypeList(vec![Term::Primitive(P::U8), Term::Primitive(P::Bool)])
    );
}

#[test]
fn constructed_type_lists_feed_normal_heads_without_unfolding() {
    let package = compile(
        "pure comptime fn Product<...Es:List<Type>>->Type {bool}
        comptime fn First()->Type {.u8'builtin}
        comptime fn Second()->Type {bool}
        comptime fn Build()->Type {Product<...[First(),Second()]>}
        comptime fn Named()->Type {Product<.Es=[First(),Second()]>}",
    )
    .unwrap();
    let expected = Value::Type(Term::NormalForm {
        head: support::id(&package, "Product"),
        sort: nir::Sort::Type,
        arguments: vec![Term::TypeListValue(vec![
            Term::Primitive(P::U8),
            Term::Primitive(P::Bool),
        ])],
    });
    for name in ["Build", "Named"] {
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        assert_eq!(
            vm.invoke(support::id(&package, name), vec![]).unwrap(),
            expected
        );
        assert_eq!(support::calls(&package, &vm), vec![name, "First", "Second"]);
    }
}

#[test]
fn type_list_elements_preserve_state_changes_and_reject_bad_later_elements() {
    let source = "comptime fn Selected()->Type {
        var seen=false;
        let values:List<Type> = [do {seen=true;bool},do {assumes seen;.u8'builtin}];
        assumes seen;values[1]
    }";
    let package = compile(source).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "Selected", 100_000).unwrap(),
        Value::Type(Term::Primitive(P::U8))
    );
    let invalid = compile(&source.replace("seen=true;bool", "seen=false;bool")).unwrap();
    assert!(luna::invocation::invoke(&invalid, "Selected", 100_000).is_err());
    assert!(compile("comptime fn bad()->List<Type> {[do {return [];},true]}").is_err());
}

#[test]
fn generic_aliases_preserve_type_list_inputs_and_opaque_heads() {
    let package = compile(
        "pure comptime fn Product<...Es:List<Type>>->Type {bool}
        typealias Mixed<T> = Product<.Es=[T,bool,T]>;
        comptime fn Chosen()->Type {Mixed<.u8'builtin>}
        comptime fn Forward()->Type where T {Mixed<T>}
        comptime fn Forwarded()->Type {Forward<.u8'builtin>()}",
    )
    .unwrap();
    let expected = Value::Type(Term::NormalForm {
        head: support::id(&package, "Product"),
        sort: nir::Sort::Type,
        arguments: vec![Term::TypeListValue(vec![
            Term::Primitive(P::U8),
            Term::Primitive(P::Bool),
            Term::Primitive(P::U8),
        ])],
    });
    for name in ["Chosen", "Forwarded"] {
        assert_eq!(
            luna::invocation::invoke(&package, name, 100_000).unwrap(),
            expected
        );
    }
}

#[test]
fn quoted_code_generators_use_type_list_data_at_construction_time() {
    let package = compile(
        "pure comptime fn build(T:Type)->< |- bool> {
            let types:List<Type> = [T,bool];
            if types[0] == types[1] {'(true)} else {'(false)}
        }
        fn run()->bool {~<build(bool)> and not ~<build(.u8'builtin)>}",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn type_list_defaults_and_generator_updates_keep_invocation_state() {
    let package = compile(
        "comptime fn choose(.types:List<Type> = [bool,.u8'builtin])->Type {types[0]}
        comptime fn Defaulted()->Type {choose()}
        comptime fn Explicit()->Type {choose(.types=[.u8'builtin])}
        pure comptime fn build(T:Type)->< |- bool> {
            var types:List<Type> = [bool];
            types=[T,types[0]];
            if types[0] == types[1] {'(true)} else {'(false)}
        }
        fn run()->bool {~<build(bool)> and not ~<build(.u8'builtin)>}",
    )
    .unwrap();
    for (name, expected) in [("Defaulted", P::Bool), ("Explicit", P::U8)] {
        assert_eq!(
            luna::invocation::invoke(&package, name, 100_000).unwrap(),
            Value::Type(Term::Primitive(expected))
        );
    }
    assert_eq!(
        luna::execution::interpret(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn type_list_index_failures_leave_later_invocations_usable() {
    let package = compile(
        "comptime fn pick(index:.Integer'builtin)->Type {
            let values:List<Type> = [bool,.u8'builtin];values[index]
        }",
    )
    .unwrap();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let pick = support::id(&package, "pick");
    for position in [-1, 2, 17] {
        let error = vm
            .invoke(
                pick,
                vec![Value::Integer(comptime_runtime::Integer::from_i128(
                    position,
                ))],
            )
            .unwrap_err();
        assert_eq!(
            error.kind,
            comptime_interpretor::ErrorKind::IndexOutOfBounds
        );
        assert!(error.source.is_some());
        assert_eq!(
            vm.invoke(
                pick,
                vec![Value::Integer(comptime_runtime::Integer::from_u128(1))]
            )
            .unwrap(),
            Value::Type(Term::Primitive(P::U8))
        );
    }
}
