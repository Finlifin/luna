//! Type data can travel through ordinary values before an identity is needed.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::{ErrorKind, Value};
use nir::{Primitive, Term, program::DescriptionLiteral};

#[test]
fn descriptions_survive_calls_and_containers_without_supplying_identity() {
    let package = support::compile(
        r#"
        use builtin.Any as Field;
        struct Slot { domain: Type }
        const Unknown: Type = Field;
        const Stored: meta.List<Type> = [Unknown, u8];
        comptime fn Forward(value: Type) -> Type { value }
        comptime fn FromFields(choose: bool) -> Type {
            let fields: meta.List<Type> = [Forward(Stored[0]), bool];
            let slot = Slot { domain: fields[0] };
            if choose { slot.domain } else { fields[1] }
        }
        comptime fn compare(left: Type, right: Type) -> bool { left == right }
        "#,
    )
    .unwrap();
    let id = |name| {
        package
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .declaration
    };
    let any = Value::Description(DescriptionLiteral::Any);
    let boolean = Value::Type(Term::Primitive(Primitive::Bool));
    for reference in [false, true] {
        support::with_comptime(&package, support::COMPTIME_BUDGET, reference, |vm| {
            let select = id("FromFields");
            assert_eq!(vm.invoke(select, vec![Value::Bool(true)]).unwrap(), any);
            assert_eq!(
                vm.invoke(select, vec![Value::Bool(false)]).unwrap(),
                boolean
            );
            let compare = id("compare");
            let error = vm
                .invoke(compare, vec![any.clone(), boolean.clone()])
                .unwrap_err();
            assert_eq!(error.kind, ErrorKind::TypeContractUnresolved);
            assert_eq!(
                vm.invoke(compare, vec![boolean.clone(), boolean.clone()])
                    .unwrap(),
                Value::Bool(true)
            );
        });
    }
}

#[test]
fn a_gradual_type_constant_cannot_supply_a_concrete_field_layout() {
    let error = support::compile("const Field: Type = Any; struct Payload { data: Field }")
        .expect_err("a stored field requires an identity");
    assert!(error.contains("gradual description"), "{error}");
}

#[test]
fn alias_declarations_and_value_uses_preserve_the_producers_type_data() {
    let package = support::compile(
        r#"
        pub mod fields {
            pub typealias Slot = Any;
            pub typealias Slots<T> = T;
        }
        comptime fn Select(flag: bool) -> Type { if flag { fields.Slot } else { u8 } }
        typealias Field = Select(true);
        typealias Second<A, B> = B;
        comptime fn packet() -> Tuple<Type, Type> { (bool, fields.Slots<Field>) }
        typealias Forward = Second<...packet()>;
        typealias Byte = Select(false);
        comptime fn FromAlias() -> Type {
            let fields: meta.List<Type> = [fields.Slots<Forward>, Byte];
            fields[0]
        }
        fn run(value: Byte) -> u8 { value }
    "#,
    )
    .unwrap();
    let entry = package
        .functions
        .iter()
        .find(|item| item.name == "FromAlias")
        .unwrap()
        .declaration;
    for reference in [false, true] {
        support::with_comptime(&package, support::COMPTIME_BUDGET, reference, |vm| {
            assert_eq!(
                vm.invoke(entry, vec![]).unwrap(),
                Value::Description(DescriptionLiteral::Any)
            );
        });
    }
    let error =
        support::compile("typealias Field = Any; struct Payload { data: Field }").unwrap_err();
    assert!(error.contains("requires a type identity"), "{error}");
}

#[test]
fn closed_alias_production_is_shared_across_unrelated_substitutions() {
    let source = r#"
        comptime fn Make() -> Type {
            var builder = meta.StructBuilder.new(.name = .Generated);
            builder.add_field(.flag, bool);
            builder.Build()
        }
        typealias Declared = Make();
        typealias Through<T> = Declared;
        typealias Fresh = Make();

        struct Holder where T { pub typealias Element = T; }
        typealias Octet = Holder<u8>.Element;
        fn retain(value: Holder<bool>.Element) -> bool { value }
        fn select(value: Through<bool>) -> Through<u8> { value }
        comptime fn check() -> bool { Through<bool> == Declared and Declared != Fresh }
        fn run() -> bool {
            let value = Declared { flag: true };
            let byte: Octet = 1;
            byte == 1 and retain(check()) and select(value).flag
        }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));
}

#[test]
fn a_computed_layout_consumer_reports_a_description_at_the_use() {
    let error =
        support::compile("comptime fn Field() -> Type { Any } struct Payload { value: Field() }")
            .unwrap_err();
    assert!(error.contains("requires a type identity"), "{error}");
}

#[test]
fn runtime_inputs_admit_completed_descriptions_at_each_call() {
    let source = r#"
        typealias Field = Any;
        comptime fn Select(flag: bool) -> Type { if flag { Field } else { u8 } }
        fn observe(value: Field) -> bool { true }
        fn selected(comptime flag: bool, value: defer Select(flag)) -> bool { observe(value) }
        fn count(calls: *u8) -> bool {
            let before = calls.*;
            if before < 255 { calls.* = before + 1; }
            true
        }
        fn run() -> bool {
            var calls: u8 = 0;
            let byte: u8 = 3;
            selected(true, count(calls.ref)) and selected(true, byte) and
                selected(false, byte) and calls == 1
        }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));
}

#[test]
fn a_description_does_not_grant_unchecked_body_operations() {
    let failure = support::compile("fn bad(value: Any) -> bool { value.missing }").unwrap_err();
    assert!(
        failure.contains("missing") || failure.contains("projection"),
        "{failure}"
    );
}

#[test]
fn a_gradual_result_checks_its_body_against_each_delivery_domain() {
    let source = r#"
        fn pass(value: Any) -> Any { value }
        fn run() -> bool {
            let integer: u32 = 42;
            let result: u32 = pass(integer);
            let flag: bool = pass(true);
            result == integer and flag
        }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));
    let incompatible = source.replace("pass(true)", "pass(integer)");
    let error = support::compile(&incompatible).unwrap_err();
    assert!(error.contains("bool") && error.contains("u32"), "{error}");
}

#[test]
fn an_identity_produced_by_a_deferred_domain_still_checks_the_actual() {
    let failure = support::compile(
        r#"
        comptime fn Select(flag: bool) -> Type { if flag { Any } else { u8 } }
        fn consume(comptime flag: bool, value: defer Select(flag)) {}
        fn run() { consume(false, true); }
    "#,
    )
    .unwrap_err();
    assert!(
        failure.contains("bool") && failure.contains("u8"),
        "{failure}"
    );
}

#[test]
fn distinct_gradual_inputs_preserve_named_order_defaults_and_inferred_clauses() {
    let source = r#"
        comptime fn Domain(flag: bool) -> Type { if flag { Any } else { u8 } }
        fn pick(comptime flag: bool, witness: T, .first: defer Domain(flag) = false,
                .second: Any = true) -> T where T { witness }
        fn record(state: *bool, next: bool) -> bool { state.* = next; next }
        fn run() -> bool {
            var state = false;
            let byte: u8 = 12;
            let result = pick(true, byte,
                .second = record(state.ref, false), .first = record(state.ref, true));
            result == byte and state and pick(true, true)
        }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));
}

#[test]
fn a_callable_target_instantiates_the_gradual_input_and_result_by_slot() {
    let source = r#"
        fn pass(value: Any) -> Any { value }
        fn call(f: fn(bool) -> bool) -> bool { f(true) }
        fn run() -> bool {
            let byte: fn(u8) -> u8 = pass;
            call(pass) and byte(7) == 7
        }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));
}

#[test]
fn an_annotation_completed_for_an_earlier_source_argument_runs_once() {
    let source = r#"
        typealias Selector = %{used: bool} comptime fn() -> Type;
        comptime fn make() -> Selector {
            var used = false;
            || { used = not used; if used { Any } else { bool } }
        }
        comptime fn consume(select: Selector, comptime .gate: bool = true,
                            .field: defer select() = false) -> bool {
            select() == bool
        }
        const result = consume(make(), .field = true, .gate = true);
        fn run() -> bool { result }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));
}

#[test]
fn produced_refinements_bind_each_call_subject_in_the_receiving_world() {
    let source = r#"
        trait Mark {}
        impl Mark for bool {}
        impl Mark for u8 {}
        impl Mark for u16 {}
        comptime fn Marked() -> Type { refines { Itself :- Mark } Any }
        comptime fn Small() -> Type {
            refines { Itself == u8 or Itself == bool } Marked()
        }
        typealias Field = Small();
        const fields: meta.List<Type> = [Field];
        comptime fn Load() -> Type { fields[0] }
        fn accept(value: defer Load()) -> bool { true }
        fn run() -> bool { let byte: u8 = 42; accept(byte) and accept(true) }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));
    for invalid in [
        source.replace("byte: u8", "byte: u16"),
        source.replace("impl Mark for bool {}", ""),
    ] {
        let error = support::compile(&invalid).unwrap_err();
        assert!(
            error.contains("refinement requirement is not established"),
            "{error}"
        );
    }
}

#[test]
fn foreign_refinement_at_the_same_slot_cannot_cross_vm_worlds() {
    let source = r#"
        trait Valid {}
        comptime fn Field() -> Type { refines { Itself :- Valid } Any }
        comptime fn Echo(value: Type) -> Type { value }
    "#;
    let first = support::compile(source).unwrap();
    let second = support::compile(source).unwrap();
    let entry = |pkg: &nir::program::Package, name| {
        pkg.functions
            .iter()
            .find(|f| f.name == name)
            .unwrap()
            .declaration
    };
    for reference in [false, true] {
        let field = support::with_comptime(&first, support::COMPTIME_BUDGET, reference, |vm| {
            vm.invoke(entry(&first, "Field"), vec![]).unwrap()
        });
        support::with_comptime(&second, support::COMPTIME_BUDGET, reference, |vm| {
            let local = vm.invoke(entry(&second, "Field"), vec![]).unwrap();
            assert_ne!(field, local);
            assert!(vm.invoke(entry(&second, "Echo"), vec![field]).is_err());
            assert_eq!(
                vm.invoke(entry(&second, "Echo"), vec![local.clone()])
                    .unwrap(),
                local
            );
        });
    }
}

#[test]
fn a_parameter_refinement_supplies_body_evidence_without_proving_the_caller() {
    let source = r#"
        trait Flag { fn flag(self) -> bool; }
        impl Flag for bool { fn flag(self) -> bool { self } }
        typealias Field = refines { Itself :- Flag } Any;
        fn read(value: Field) -> bool { value.flag() }
        fn run() -> bool { read(true) }
    "#;
    assert_eq!(support::run(source), Value::Bool(true));
    let invalid = source.replace("read(true)", "do { let byte: u8 = 1; read(byte) }");
    let error = support::compile(&invalid).unwrap_err();
    assert!(
        error.contains("refinement requirement is not established"),
        "{error}"
    );
}

#[test]
fn constructed_descriptions_keep_the_head_formation_requirements() {
    let source = r#"
        pure comptime fn Checked<T, Gate> -> Type where requires Gate == bool { bool }
        comptime fn Make(gate: Type) -> Type { Checked<Any, gate> }
    "#;
    let package = support::compile(source).unwrap();
    let entry = package
        .functions
        .iter()
        .find(|f| f.name == "Make")
        .unwrap()
        .declaration;
    for tree in [false, true] {
        support::with_comptime(&package, support::COMPTIME_BUDGET, tree, |vm| {
            assert!(
                vm.invoke(
                    entry,
                    vec![Value::Type(nir::Term::Primitive(nir::Primitive::U8))]
                )
                .is_err()
            );
            assert!(matches!(
                vm.invoke(
                    entry,
                    vec![Value::Type(nir::Term::Primitive(nir::Primitive::Bool))]
                )
                .unwrap(),
                Value::Description(_)
            ));
        });
    }
}
