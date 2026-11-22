//! Shipped attribute constructors in isolated and full sysroot configurations.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod sysroot;

use comptime_interpretor::{Interpreter, Value};
use nir::{Primitive, Term};

const ATTRS: &str = include!("support/attrs_source.rs");

#[test]
fn abi_constructor_preserves_symbol_and_object_entry_type() {
    let package = support::compile(&format!(
        "mod attrs {{ {ATTRS} }}
        use attrs.abi as calling_convention;
        comptime fn metadata_sample() -> attrs.bindings.Object {{ calling_convention(.c) }}"
    ));
    let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
    let value = vm
        .invoke(support::id(&package, "metadata_sample"), vec![])
        .unwrap();
    let Value::Object(object) = value else {
        panic!("expected attribute object")
    };
    assert_eq!(object.entries().len(), 1);
    let entry = &object.entries()[0];
    assert_eq!(entry.key(), Some("__flurry_abi"));
    assert_eq!(entry.value().ty, Term::Primitive(Primitive::Symbol));
    assert_eq!(entry.value().value, Value::Symbol("c".into()));
}

#[test]
fn abi_constructor_rejects_wrong_domain_and_runtime_transport() {
    for body in [
        "comptime fn wrong() -> attrs.bindings.Object { attrs.abi(true) }",
        "fn wrong() -> attrs.bindings.Object { attrs.abi(.c) }",
        "fn wrong(name: attrs.builtin.Symbol) { const value = attrs.abi(name); }",
    ] {
        let source = format!("mod attrs {{ {ATTRS} }} {body}");
        assert!(support::compile_with_protocols(&source, &Default::default()).is_err());
    }
}

#[test]
fn repository_attribute_package_preserves_abi_through_nir() {
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
    let library = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let arguments = [
        "dump",
        "input.fl",
        "--sysroot",
        library.to_str().unwrap(),
        "--build-target",
        "x86_64",
        "linux",
        "64",
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
    assert!(dump.contains("(property \"__flurry_abi\")"), "{dump}");
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
    assert!(
        selected.contains("(key \"__flurry_abi\") (entry 0)"),
        "{selected}"
    );

    let mut ordinary = arguments;
    *ordinary.last_mut().unwrap() = "nir";
    let nir = run(&ordinary);
    assert!(
        nir.status.success(),
        "{}",
        String::from_utf8_lossy(&nir.stderr)
    );
    assert!(String::from_utf8_lossy(&nir.stdout).contains("__flurry_abi"));
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

#[test]
fn comptime_phase_prelude_value_preserves_boolean_metadata() {
    let package = support::compile(&format!(
        "mod attrs {{{ATTRS}}} use attrs.prelude.*; comptime fn metadata_sample()->attrs.bindings.Object {{comptime_phase}}"
    ));
    let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
    let Value::Object(object) = vm
        .invoke(support::id(&package, "metadata_sample"), vec![])
        .unwrap()
    else {
        panic!("expected Object")
    };
    assert_eq!(object.entries().len(), 1);
    assert_eq!(object.entries()[0].key(), Some("__flurry_comptime_phase"));
    assert_eq!(
        object.entries()[0].value().ty,
        Term::Primitive(Primitive::Bool)
    );
    assert_eq!(object.entries()[0].value().value, Value::Bool(true));
}

#[test]
fn phase_attribute_alias_is_inspected_through_the_repository_package() {
    let directory = std::env::temp_dir().join(format!("luna-library-phase-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("input.fl"),
        "use builtin.bool; use attrs.comptime_phase as staged; trait Code {^staged comptime fn value()->bool;}",
    )
    .unwrap();
    let library = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    for emit in ["attributes", "merged-attributes"] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_luna"))
            .current_dir(&directory)
            .args([
                "dump",
                "input.fl",
                "--sysroot",
                library.to_str().unwrap(),
                "--build-target",
                "x86_64",
                "linux",
                "64",
                "--emit",
                emit,
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(text.contains("comptime_phase"), "{text}");
        if emit == "attributes" {
            assert!(text.contains("(bool true)"), "{text}");
        }
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn sysroot_attribute_constructor_returns_the_public_meta_domain() {
    let package = sysroot::compile(
        "use meta.Object as Metadata;
         use attrs.abi as convention;
         comptime fn metadata() -> Metadata { convention(.c) }",
    )
    .unwrap();
    let mut vm = Interpreter::for_comptime(&package, sysroot::COMPTIME_BUDGET).unwrap();
    let Value::Object(object) = vm
        .invoke(support::id(&package, "metadata"), vec![])
        .unwrap()
    else {
        panic!("expected meta.Object");
    };
    assert_eq!(object.entries().len(), 1);
    let entry = &object.entries()[0];
    assert_eq!(entry.key(), Some("__flurry_abi"));
    assert_eq!(entry.value().ty, Term::Primitive(Primitive::Symbol));
    assert_eq!(entry.value().value, Value::Symbol("c".into()));
}

#[test]
fn stable_since_constructor_preserves_str_and_object_entry_type() {
    let package = support::compile(&format!(
        "mod attrs {{ {ATTRS} }}
        use attrs.prelude.*;
        comptime fn metadata_sample() -> attrs.bindings.Object {{ stable_since(\"1.0.0\") }}"
    ));
    let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
    let value = vm
        .invoke(support::id(&package, "metadata_sample"), vec![])
        .unwrap();
    let Value::Object(object) = value else {
        panic!("expected attribute object")
    };
    assert_eq!(object.entries().len(), 1);
    let entry = &object.entries()[0];
    assert_eq!(entry.key(), Some("__flurry_stable_since"));
    assert_eq!(entry.value().ty, Term::Primitive(Primitive::Str));
    assert_eq!(entry.value().value, Value::String("1.0.0".into()));
}

#[test]
fn stable_since_constructor_rejects_wrong_domain_and_runtime_transport() {
    for body in [
        "comptime fn wrong() -> attrs.bindings.Object { attrs.stable_since(true) }",
        "fn wrong() -> attrs.bindings.Object { attrs.stable_since(\"1.0.0\") }",
        "fn wrong(version: attrs.bindings.str) { const value = attrs.stable_since(version); }",
    ] {
        let source = format!("mod attrs {{ {ATTRS} }} {body}");
        assert!(support::compile_with_protocols(&source, &Default::default()).is_err());
    }
}

#[test]
fn repository_attribute_package_inspects_stable_since_without_claiming_abi_stability() {
    let directory =
        std::env::temp_dir().join(format!("luna-library-stable-since-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("input.fl"),
        "use attrs.stable_since as stable; ^stable(\"1.0.0\") struct Sample {} ",
    )
    .unwrap();
    let run = |arguments: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_luna"))
            .current_dir(&directory)
            .args(arguments)
            .output()
            .unwrap()
    };
    let library = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let arguments = [
        "dump",
        "input.fl",
        "--sysroot",
        library.to_str().unwrap(),
        "--build-target",
        "x86_64",
        "linux",
        "64",
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
    assert!(
        dump.contains("(property \"__flurry_stable_since\")"),
        "{dump}"
    );
    assert!(dump.contains("(string \"1.0.0\")"), "{dump}");

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
    assert!(
        selected.contains("(key \"__flurry_stable_since\") (entry 0)"),
        "{selected}"
    );

    let mut ordinary = arguments;
    *ordinary.last_mut().unwrap() = "nir";
    let admitted = run(&ordinary);
    assert!(
        admitted.status.success(),
        "{}",
        String::from_utf8_lossy(&admitted.stderr)
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn packed_mode_constants_preserve_symbol_and_object_entry_type() {
    let package = support::compile(&format!(
        "mod attrs {{ {ATTRS} }}
        use attrs.packed;
        comptime fn byte_sample() -> attrs.bindings.Object {{ packed.byte }}
        comptime fn bit_sample() -> attrs.bindings.Object {{ packed.bit }}"
    ));
    let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
    for (entry_fn, expected_mode) in [("byte_sample", "byte"), ("bit_sample", "bit")] {
        let value = vm.invoke(support::id(&package, entry_fn), vec![]).unwrap();
        let Value::Object(object) = value else {
            panic!("expected attribute object for {entry_fn}")
        };
        assert_eq!(object.entries().len(), 1);
        let entry = &object.entries()[0];
        assert_eq!(entry.key(), Some("__flurry_packed_mode"));
        assert_eq!(entry.value().ty, Term::Primitive(Primitive::Symbol));
        assert_eq!(entry.value().value, Value::Symbol(expected_mode.into()));
    }
}

#[test]
fn repository_attribute_package_inspects_packed_byte_and_bit_with_merge_override() {
    let directory =
        std::env::temp_dir().join(format!("luna-library-packed-modes-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("input.fl"),
        "^attrs.packed.byte struct BytePacked { a: u8, b: u64 }
         ^attrs.packed.bit struct BitPacked { flag: bool, tag: u8 }
         ^attrs.packed.bit ^attrs.packed.byte struct Overridden { a: u8 }",
    )
    .unwrap();
    let run = |arguments: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_luna"))
            .current_dir(&directory)
            .args(arguments)
            .output()
            .unwrap()
    };
    let library = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let arguments = [
        "dump",
        "input.fl",
        "--sysroot",
        library.to_str().unwrap(),
        "--build-target",
        "x86_64",
        "linux",
        "64",
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
    assert_eq!(dump.matches("(evaluated-attribute ").count(), 4, "{dump}");
    assert_eq!(
        dump.matches("(property \"__flurry_packed_mode\")").count(),
        4,
        "{dump}"
    );
    assert_eq!(dump.matches("(symbol \"byte\")").count(), 2, "{dump}");
    assert_eq!(dump.matches("(symbol \"bit\")").count(), 2, "{dump}");

    let mut merged = arguments;
    *merged.last_mut().unwrap() = "merged-attributes";
    let selected = run(&merged);
    assert!(
        selected.status.success(),
        "{}",
        String::from_utf8_lossy(&selected.stderr)
    );
    let selected = String::from_utf8_lossy(&selected.stdout);
    assert_eq!(
        selected.matches("(merged-attributes ").count(),
        3,
        "{selected}"
    );
    assert_eq!(
        selected.matches("(key \"__flurry_packed_mode\")").count(),
        3,
        "{selected}"
    );

    let mut ordinary = arguments;
    *ordinary.last_mut().unwrap() = "nir";
    let rejected = run(&ordinary);
    assert_eq!(rejected.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("__flurry_packed_mode"));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn volatile_constants_preserve_bool_and_object_entry_type() {
    let package = support::compile(&format!(
        "mod attrs {{ {ATTRS} }}
        use attrs.mem;
        comptime fn mem_sample() -> attrs.bindings.Object {{ mem.volatile }}
        comptime fn root_sample() -> attrs.bindings.Object {{ attrs.volatile }}"
    ));
    let mut vm = Interpreter::for_comptime(&package, Default::default()).unwrap();
    for entry_fn in ["mem_sample", "root_sample"] {
        let value = vm.invoke(support::id(&package, entry_fn), vec![]).unwrap();
        let Value::Object(object) = value else {
            panic!("expected attribute object for {entry_fn}")
        };
        assert_eq!(object.entries().len(), 1);
        let entry = &object.entries()[0];
        assert_eq!(entry.key(), Some("__flurry_volatile"));
        assert_eq!(entry.value().ty, Term::Primitive(Primitive::Bool));
        assert_eq!(entry.value().value, Value::Bool(true));
    }
}

#[test]
fn repository_attribute_package_validates_and_rejects_invalid_volatile_targets() {
    let directory =
        std::env::temp_dir().join(format!("luna-library-volatile-attr-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let library = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let run = |file: &str, emit: &str| {
        std::fs::write(directory.join("input.fl"), file).unwrap();
        std::process::Command::new(env!("CARGO_BIN_EXE_luna"))
            .current_dir(&directory)
            .args([
                "dump",
                "input.fl",
                "--sysroot",
                library.to_str().unwrap(),
                "--build-target",
                "x86_64",
                "linux",
                "64",
                "--emit",
                emit,
            ])
            .output()
            .unwrap()
    };

    let valid_source = "^attrs.mem.volatile struct MmioReg { value: u32 }
        ^attrs.volatile newtype VolU32 = u32;
        struct Uart {
            ^attrs.mem.volatile status: u32,
            normal: u32,
        }";
    let attrs_out = run(valid_source, "attributes");
    assert!(
        attrs_out.status.success(),
        "{}",
        String::from_utf8_lossy(&attrs_out.stderr)
    );
    let dump = String::from_utf8_lossy(&attrs_out.stdout);
    assert_eq!(
        dump.matches("(property \"__flurry_volatile\")").count(),
        3,
        "{dump}"
    );

    let merged_out = run(valid_source, "merged-attributes");
    assert!(
        merged_out.status.success(),
        "{}",
        String::from_utf8_lossy(&merged_out.stderr)
    );
    let merged = String::from_utf8_lossy(&merged_out.stdout);
    assert_eq!(
        merged.matches("(key \"__flurry_volatile\")").count(),
        3,
        "{merged}"
    );

    let nir_out = run(valid_source, "nir");
    assert!(
        nir_out.status.success(),
        "{}",
        String::from_utf8_lossy(&nir_out.stderr)
    );

    for (invalid_source, expected_diag) in [
        (
            "struct ConfigReg { .__flurry_volatile true, value: u32 }",
            "configuration",
        ),
        (
            "^attrs.mem.volatile fn bad_fn() -> u32 { 1 }",
            "__flurry_volatile",
        ),
        (
            "^{ __flurry_volatile: 1 } struct BadDomain { value: u32 }",
            "Bool",
        ),
        (
            "^attrs.layout.packed ^attrs.mem.volatile struct PackedVol { value: u32 }",
            "packed",
        ),
    ] {
        let rejected = run(invalid_source, "nir");
        assert_eq!(
            rejected.status.code(),
            Some(1),
            "expected failure for `{invalid_source}`"
        );
        let stderr = String::from_utf8_lossy(&rejected.stderr);
        assert!(
            stderr.contains(expected_diag),
            "expected `{expected_diag}` in stderr for `{invalid_source}`:\n{stderr}"
        );
    }

    std::fs::remove_dir_all(&directory).unwrap();
}
