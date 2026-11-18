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
