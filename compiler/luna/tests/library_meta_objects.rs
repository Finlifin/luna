//! Exercise the public meta Object interface with the repository library source.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::{Interpreter, Value};
use nir::{Primitive, Term};

const META: &str = include_str!("../../../library/meta/lib.fl");

fn compile(source: &str) -> Result<nir::program::Package, String> {
    support::compile_with_source_bindings(
        source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.meta.List"),
            ..Default::default()
        },
    )
}

#[test]
fn repository_package_exports_object_for_user_attribute_constructors() {
    let directory = std::env::temp_dir().join(format!("luna-meta-object-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let input = directory.join("input.fl");
    std::fs::write(
        &input,
        "use meta.prelude.*;
        pure comptime fn marker() -> Object { let value = { enabled: true }; value }
        ^marker() fn example() {}",
    )
    .unwrap();
    let library =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library/meta/lib.fl");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_luna"))
        .args(["--packages-v1", "--package", "1", "101", "app"])
        .arg(&input)
        .args(["--package", "2", "202", "meta"])
        .arg(&library)
        .args([
            "--dependency",
            "1",
            "meta",
            "2",
            "--primary",
            "1",
            "--meta-list-type",
            "meta.List",
            "--emit",
            "attributes",
        ])
        .output()
        .unwrap();
    std::fs::remove_dir_all(directory).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let dump = String::from_utf8_lossy(&output.stdout);
    assert_eq!(dump.matches("(evaluated-attribute ").count(), 1, "{dump}");
    assert!(dump.contains("(property \"enabled\")"), "{dump}");
    assert!(dump.contains("(bool true)"), "{dump}");
}

#[test]
fn public_object_preserves_order_duplicate_keys_and_nested_constants() {
    let package = compile(&format!(
        "mod meta {{ {META} }}
         use meta.prelude.*;
         const saved = {{ flag: true, 'x', flag: false }};
         comptime fn identity(value: meta.Object) -> Object {{ value }}
         comptime fn metadata() -> Object {{
             let result = {{ nested: saved, mode: .c }};
             identity(result)
         }}"
    ))
    .unwrap();
    let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
    let value = vm
        .invoke(support::id(&package, "metadata"), vec![])
        .unwrap();
    let Value::Object(object) = value else {
        panic!("expected Object")
    };
    assert_eq!(object.entries().len(), 2);
    let nested = &object.entries()[0];
    assert_eq!(nested.key(), Some("nested"));
    assert_eq!(nested.value().ty, Term::Primitive(Primitive::Object));
    let Value::Object(nested) = &nested.value().value else {
        panic!("expected nested Object")
    };
    assert_eq!(
        nested
            .entries()
            .iter()
            .map(|entry| entry.key())
            .collect::<Vec<_>>(),
        vec![Some("flag"), None, Some("flag")]
    );
    assert_eq!(nested.entries()[0].value().value, Value::Bool(true));
    assert_eq!(nested.entries()[1].value().value, Value::Char('x'));
    assert_eq!(nested.entries()[2].value().value, Value::Bool(false));
    assert_eq!(object.entries()[1].key(), Some("mode"));
    assert_eq!(
        object.entries()[1].value().ty,
        Term::Primitive(Primitive::Symbol)
    );
    assert_eq!(object.entries()[1].value().value, Value::Symbol("c".into()));
}

#[test]
fn public_alias_does_not_allow_runtime_transport_or_wrong_values() {
    for body in [
        "fn invalid(value: Object) {}",
        "fn invalid() -> Object { let value = { flag: true }; value }",
        "comptime fn invalid() -> Object { true }",
    ] {
        let source = format!("mod meta {{ {META} }} use meta.prelude.*; {body}");
        assert!(compile(&source).is_err(), "{body}");
    }
}
