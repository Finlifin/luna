use luna::{
    cli::{Emit, Options},
    frontend::{self, Artifact},
};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

fn compile(source: &str, emit: Emit) -> Result<Artifact, String> {
    let path = std::env::temp_dir().join(format!(
        "enum-config-{}-{}.fl",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, source).unwrap();
    let result = frontend::compile(&Options {
        input: path.clone(),
        emit,
        output: None,
        sysroot: None,
        splice_protocol: None,
        meta_list_type: None,
        tuple_type: None,
        index_protocol: None,
        ord_protocol: None,
        eq_protocol: None,
        list_pattern_protocol: None,
        literal_into_protocol: None,
        arithmetic_protocols: Default::default(),
    });
    std::fs::remove_file(path).unwrap();
    result
}

#[test]
fn configurations_survive_resolution_and_hir_in_source_order() {
    let source = "typealias Word=.u32'builtin;const WIDTH=32;
        enum Instruction {.base_type Word,.width WIDTH,.discrete_tag true, add:0b00000000000000000000000000110011,}";
    let Artifact::Hir(dump) = compile(source, Emit::Hir).unwrap() else {
        panic!("wrong stage");
    };
    let base = dump
        .find("(configuration (name (ident (name \"base_type\")")
        .unwrap();
    let width = dump
        .find("(configuration (name (ident (name \"width\")")
        .unwrap();
    let discrete = dump
        .find("(configuration (name (ident (name \"discrete_tag\")")
        .unwrap();
    assert!(base < width && width < discrete);
    assert!(dump.contains("(configurations ("));
    assert!(dump.contains("(variant "));
    let Artifact::Nir(package) = compile(source, Emit::Nir).unwrap() else {
        panic!("wrong stage");
    };
    assert_eq!(package.enums[0].encoding.as_ref().unwrap().width(), 32);
    assert!(
        compile(
            &source.replace(".base_type Word", ".base_type Missing"),
            Emit::Hir
        )
        .is_err()
    );
}

#[test]
fn nested_enum_configurations_remain_with_their_own_definition() {
    let source = "mod A {enum E {.width 8,a,}} mod B {enum E {.width 16,b,}}";
    let Artifact::Hir(dump) = compile(source, Emit::Hir).unwrap() else {
        panic!("wrong stage");
    };
    assert_eq!(
        dump.matches("(configuration (name (ident (name \"width\")")
            .count(),
        2
    );
}
