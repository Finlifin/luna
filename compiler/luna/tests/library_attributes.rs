//! Attribute construction uses the repository package, not fixture definitions.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::{Interpreter, Value};
use nir::{Primitive, Term};

const ATTRS: &str = include_str!("../../../library/attrs/lib.fl");

#[test]
fn abi_constructor_preserves_symbol_and_object_entry_type() {
    let package = support::compile(&format!(
        "mod attrs {{ {ATTRS} }}
        use attrs.abi as calling_convention;
        comptime fn metadata() -> .Object'builtin {{ calling_convention(.c) }}"
    ));
    let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
    let value = vm
        .invoke(support::id(&package, "metadata"), vec![])
        .unwrap();
    let Value::Object(object) = value else {
        panic!("expected attribute object")
    };
    assert_eq!(object.entries().len(), 1);
    let entry = &object.entries()[0];
    assert_eq!(entry.key(), Some("abi"));
    assert_eq!(entry.value().ty, Term::Primitive(Primitive::Symbol));
    assert_eq!(entry.value().value, Value::Symbol("c".into()));
}

#[test]
fn abi_constructor_rejects_wrong_domain_and_runtime_transport() {
    for body in [
        "comptime fn wrong() -> .Object'builtin { attrs.abi(true) }",
        "fn wrong() -> .Object'builtin { attrs.abi(.c) }",
        "fn wrong(name: .Symbol'builtin) { const value = attrs.abi(name); }",
    ] {
        let source = format!("mod attrs {{ {ATTRS} }} {body}");
        assert!(support::compile_with_protocols(&source, &Default::default()).is_err());
    }
}

#[test]
fn repository_attribute_package_inspects_abi_without_claiming_abi_lowering() {
    let directory = std::env::temp_dir().join(format!("luna-library-attrs-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("input.fl"),
        "use attrs.abi as convention; ^convention(.c) fn sample() {} ",
    )
    .unwrap();
    let run = |arguments: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_luna"))
            .current_dir(&directory)
            .args(arguments)
            .output()
            .unwrap()
    };
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library/attrs/lib.fl");
    let arguments = [
        "--packages-v1",
        "--package",
        "1",
        "101",
        "app",
        "input.fl",
        "--package",
        "2",
        "202",
        "attrs",
        path.to_str().unwrap(),
        "--dependency",
        "1",
        "attrs",
        "2",
        "--primary",
        "1",
        "--emit",
        "attributes",
    ];
    let output = run(&arguments);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let dump = String::from_utf8_lossy(&output.stdout);
    assert_eq!(dump.matches("(evaluated-attribute ").count(), 1, "{dump}");
    assert!(dump.contains("(property \"abi\")"), "{dump}");
    assert!(dump.contains("(symbol \"c\")"), "{dump}");

    let mut merged = arguments;
    *merged.last_mut().unwrap() = "merged-attributes";
    let selected = run(&merged);
    assert!(
        selected.status.success(),
        "{}",
        String::from_utf8_lossy(&selected.stderr)
    );
    let selected = String::from_utf8_lossy(&selected.stdout);
    assert!(
        selected.contains("(attribute-inspection (evaluated"),
        "{selected}"
    );
    assert_eq!(
        selected.matches("(merged-attributes ").count(),
        1,
        "{selected}"
    );
    assert!(selected.contains("(key \"abi\") (entry 0)"), "{selected}");

    let mut ordinary = arguments;
    *ordinary.last_mut().unwrap() = "nir";
    let rejected = run(&ordinary);
    assert_eq!(rejected.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("attribute"));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn source_merge_preserves_application_order_and_failed_output() {
    let directory =
        std::env::temp_dir().join(format!("luna-attribute-merge-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let input = directory.join("input.fl");
    let output = directory.join("attributes.lisp");
    let run = |mode| {
        std::process::Command::new(env!("CARGO_BIN_EXE_luna"))
            .arg(&input)
            .args(["--emit", mode, "--output"])
            .arg(&output)
            .output()
            .unwrap()
    };
    std::fs::write(
        &input,
        "^{mode:false, mode:true} ^{mode:false} struct First { .mode false }
         ^{mode:false} struct Second {}",
    )
    .unwrap();
    let checked = run("merged-attributes");
    assert!(
        checked.status.success(),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let report = std::fs::read_to_string(&output).unwrap();
    assert_eq!(
        report.matches("(evaluated-attribute ").count(),
        4,
        "{report}"
    );
    assert_eq!(report.matches("(merged-attributes ").count(), 2, "{report}");
    let selected = report.split(") (selected").nth(1).unwrap();
    assert_eq!(
        selected.matches("(key \"mode\") (entry 1)").count(),
        1,
        "{report}"
    );
    assert_eq!(
        selected.matches("(key \"mode\") (entry 0)").count(),
        1,
        "{report}"
    );
    assert!(
        !selected.contains("(bool"),
        "selection must refer to original objects"
    );

    std::fs::write(&input, "^{true} fn sample() {}").unwrap();
    let rejected = run("merged-attributes");
    assert_eq!(rejected.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("positional"));
    assert_eq!(std::fs::read_to_string(&output).unwrap(), report);
    assert!(
        run("attributes").status.success(),
        "raw Object inspection remains valid"
    );

    std::fs::write(&input, "fn sample() {}").unwrap();
    assert!(run("merged-attributes").status.success());
    assert_eq!(
        std::fs::read_to_string(&output).unwrap().trim(),
        "(attribute-inspection (evaluated) (selected))"
    );
    std::fs::remove_dir_all(directory).unwrap();
}
