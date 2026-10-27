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
        iterator_protocol: None,
        into_iterator_protocol: None,
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
}

#[test]
fn unknown_configuration_values_are_preserved_in_hir_and_rejected_by_elaboration() {
    let valid = "typealias Word=.u32'builtin;const WIDTH=32;
        enum Instruction {.base_type Word,.width WIDTH,.discrete_tag true,add:0b00000000000000000000000000110011,}";
    compile(valid, Emit::Nir).unwrap();
    for (configuration, unknown) in [
        (".base_type Word", ".base_type Missing"),
        (".width WIDTH", ".width Missing"),
        (".discrete_tag true", ".discrete_tag Missing"),
    ] {
        let source = valid.replace(configuration, unknown);
        let Artifact::Hir(dump) = compile(&source, Emit::Hir).unwrap() else {
            panic!("expected HIR before attribute elaboration");
        };
        assert!(
            dump.contains("(unresolved-ident (name \"Missing\"))"),
            "{dump}"
        );
        let failure = compile(&source, Emit::Nir)
            .err()
            .expect("unknown configuration value must not become a default");
        assert!(failure.contains("elaboration failed"), "{failure}");
    }
}

#[test]
fn deferred_configuration_names_do_not_hide_access_errors() {
    let source = "mod hidden {private typealias Word=.u8'builtin;}
        enum Instruction {.base_type hidden.Word,.width 8,.discrete_tag true,add:0b00110011,}";
    assert!(compile(source, Emit::Hir).is_err());
    let visible = source.replace("private typealias", "pub typealias");
    assert!(compile(&visible, Emit::Hir).is_ok());
    compile(&visible, Emit::Nir).unwrap();
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
