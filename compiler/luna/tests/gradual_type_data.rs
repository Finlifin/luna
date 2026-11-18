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
