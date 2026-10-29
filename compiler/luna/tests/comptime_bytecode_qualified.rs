//! Optional layers and error rows keep their own identities during invocation.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Budget, Interpreter, Value};
use nir::{Primitive, Term};

#[test]
fn compiled_optional_construction_preserves_each_absence_layer() {
    let package = support::compile(
        "comptime fn absent() -> ? ?bool { null }
        comptime fn inner_absent() -> ? ?bool { let inner: ?bool = null; inner }
        comptime fn present() -> ? ?bool { let inner: ?bool = true; inner }",
    );
    let inner = Term::Optional(Box::new(Term::Primitive(Primitive::Bool)));
    let outer = Term::Optional(Box::new(inner.clone()));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut tree = Interpreter::new(&package, Budget::default()).unwrap();
    for (entry, value) in [
        ("absent", None),
        (
            "inner_absent",
            Some(Box::new(Value::Optional {
                ty: inner.clone(),
                value: None,
            })),
        ),
        (
            "present",
            Some(Box::new(Value::Optional {
                ty: inner,
                value: Some(Box::new(Value::Bool(true))),
            })),
        ),
    ] {
        let expected = Value::Optional {
            ty: outer.clone(),
            value,
        };
        let id = support::id(&package, entry);
        let before = vm.bytecode_invocations();
        assert_eq!(vm.invoke(id, vec![]).unwrap(), expected);
        assert_eq!(tree.invoke(id, vec![]).unwrap(), expected);
        assert_eq!(vm.bytecode_invocations() - before, 1);
    }
}

#[test]
fn compiled_error_construction_and_embedding_preserve_labels_and_payloads() {
    let package = support::compile(
        "enum Failure { bad(bool) }
        enum Extra { other }
        comptime fn narrow(flag: bool) -> ![Failure] bool {
            if flag { true } else { error Failure.bad(false) }
        }
        comptime fn wide(flag: bool) -> ![Extra, Failure] bool { narrow(flag) }",
    );
    let failure = package
        .enums
        .iter()
        .find(|e| e.name == "Failure")
        .unwrap()
        .declaration;
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let mut tree = Interpreter::new(&package, Budget::default()).unwrap();
    let root = support::id(&package, "wide");
    for succeeds in [false, true] {
        let before = vm.bytecode_invocations();
        let result = vm.invoke(root, vec![Value::Bool(succeeds)]).unwrap();
        assert_eq!(
            result,
            tree.invoke(root, vec![Value::Bool(succeeds)]).unwrap()
        );
        let Value::ErrorResult { ty, error, value } = result else {
            panic!("missing error carrier")
        };
        let Term::ErrorResult { errors, result } = ty else {
            panic!("missing row")
        };
        assert_eq!(errors.len(), 2);
        assert_eq!(*result, Term::Primitive(Primitive::Bool));
        if succeeds {
            assert_eq!(error, None);
            assert_eq!(*value, Value::Bool(true));
        } else {
            assert_eq!(
                errors[error.unwrap()],
                Term::Nominal {
                    declaration: failure,
                    arguments: vec![]
                }
            );
            assert_eq!(
                *value,
                Value::Variant {
                    declaration: failure,
                    arguments: vec![],
                    index: 0,
                    fields: vec![Value::Bool(false)]
                }
            );
        }
        assert_eq!(vm.bytecode_invocations() - before, 2);
    }
}
