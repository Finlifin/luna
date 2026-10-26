//! Source comptime reflection feeds ordinary list transformations and type inputs.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::Value;

fn compile(source: &str) -> Result<nir::program::Package, String> {
    support::compile_with_source_bindings(
        &format!(
            "mod meta {{ {} }} {source}",
            include_str!("../../../library/meta/lib.fl")
        ),
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.meta.List"),
            ..Default::default()
        },
    )
}

#[test]
fn struct_fields_are_observed_in_order_with_applied_generic_types() {
    let package = compile(
        "
        typealias Type = .Type'builtin;
        typealias Symbol = .Symbol'builtin;
        struct Sample where T { first: T, second: bool }
        struct Empty {}
        comptime fn run() -> bool {
            let types = meta.struct_field_types(Sample<.u32'builtin>);
            let names = meta.struct_field_names(Sample<.u32'builtin>);
            meta.length(types) == 2 and types[0] == .u32'builtin and types[1] == bool
                and meta.length(names) == 2 and names[0] == .first and names[1] == .second
                and meta.length(meta.struct_field_types(Empty)) == 0
                and meta.length(meta.struct_field_names(Empty)) == 0
        }
        pure comptime fn Packet<...fields: meta.List<Type>> -> Type {Packet<...fields>}
        pure comptime fn Generated() -> Type {
            Packet<...meta.struct_field_types(Sample<bool>)>
        }
        comptime fn Direct() -> Type {Packet<bool,bool>}
    ",
    )
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 500_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::invocation::invoke(&package, "Generated", 500_000).unwrap(),
        luna::invocation::invoke(&package, "Direct", 500_000).unwrap()
    );
    let dump = package.to_string();
    assert!(dump.contains("(struct-field-types "), "{dump}");
    assert!(dump.contains("(struct-field-names "), "{dump}");
}

#[test]
fn reflection_evaluates_the_subject_once_and_preserves_opaque_fields() {
    let package = compile(
        "
        typealias Type = .Type'builtin;
        typealias Integer = .Integer'builtin;
        pure comptime fn Identity<T> -> Type {T}
        struct Sample { field: Identity<bool> }
        comptime fn Selected(count: *Integer) -> Type {
            count.* = count.* + 1; Sample
        }
        comptime fn run() -> bool {
            var count: Integer = 0;
            let types = meta.struct_field_types(Selected(count.ref));
            let names = meta.struct_field_names(Selected(count.ref));
            count == 2 and types[0] == Identity<bool> and names[0] == .field
        }
        comptime fn bad() -> Type {meta.struct_field_types(Identity<Sample>)[0]}
    ",
    )
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 500_000).unwrap(),
        Value::Bool(true)
    );
    assert!(luna::invocation::invoke(&package, "bad", 500_000).is_err());
}

#[test]
fn reflection_rejects_non_structs_wrong_operands_and_runtime_invocation() {
    for source in [
        "comptime fn bad() {meta.struct_field_types(true);}",
        "fn bad() {meta.struct_field_types(bool);}",
        "comptime fn bad() {.struct_field_names'builtin();}",
        "comptime fn bad() {.struct_field_types'builtin(bool, bool);}",
    ] {
        assert!(compile(source).is_err(), "{source}");
    }
    let package = compile(
        "
        comptime fn bad() {meta.struct_field_types(bool);}
        struct Good { flag: bool }
        comptime fn good() -> bool {meta.struct_field_types(Good)[0] == bool}
    ",
    )
    .unwrap();
    assert!(luna::invocation::invoke(&package, "bad", 100_000).is_err());
    assert_eq!(
        luna::invocation::invoke(&package, "good", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn reflected_types_participate_in_required_elaboration_and_runtime_results() {
    let package = compile(
        "
        typealias Type = .Type'builtin;
        struct Flags { flag: bool }
        pure comptime fn First() -> Type {meta.struct_field_types(Flags)[0]}
        typealias Reflected = First();
        fn entry() -> Reflected {true}
        comptime fn early() -> bool {
            .struct_field_types'builtin(do {return true;}); false
        }
    ",
    )
    .unwrap();
    assert_eq!(
        luna::execution::interpret(&package, "entry", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::invocation::invoke(&package, "early", 100_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn ordinary_package_observes_fields_through_meta_without_builtin_access() {
    let directory =
        std::env::temp_dir().join(format!("luna-source-reflection-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let input = directory.join("input.fl");
    std::fs::write(
        &input,
        "
        struct Flags { enabled: bool }
        comptime fn run() -> bool {
            let names = meta.struct_field_names(Flags);
            let types = meta.struct_field_types(Flags);
            meta.length(names) == 1 and names[0] == .enabled and types[0] == bool
        }
    ",
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
            "--invoke-comptime",
            "run",
        ])
        .output()
        .unwrap();
    std::fs::remove_dir_all(directory).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "(bool true)"
    );
}
