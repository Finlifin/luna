//! Preserve implementation phase before phase-aware selection is available.
use luna::cli::{Emit, Options};
use luna::frontend::{self, Artifact};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

fn compile(source: &str, emit: Emit) -> Result<Artifact, String> {
    let path = std::env::temp_dir().join(format!(
        "luna-impl-phase-{}-{}.fl",
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
fn complementary_implementation_phases_survive_hir_lowering() {
    let Artifact::Hir(text)=compile("trait Mark {} struct Subject {} impl Mark for Subject {} comptime impl Mark for Subject {}",Emit::Hir).unwrap() else {panic!()};
    assert_eq!(
        text.matches("(impl-definition (phase runtime)").count(),
        1,
        "{text}"
    );
    assert_eq!(
        text.matches("(impl-definition (phase comptime)").count(),
        1,
        "{text}"
    );
    assert!(!text.contains("__flurry_kw_comptime"), "{text}");
}

#[test]
fn inherent_implementation_phase_is_retained_independently_of_members() {
    let Artifact::Hir(text) = compile(
        "struct Subject {} comptime impl Subject {fn value()->bool {true}}",
        Emit::Hir,
    )
    .unwrap() else {
        panic!()
    };
    assert!(text.contains("(impl-definition (phase comptime)"), "{text}");
    assert!(text.contains("(comptime false)"), "{text}");
}

#[test]
fn staged_implementations_cannot_silently_enter_runtime_selection() {
    for source in [
        "trait Mark {} struct Subject {} comptime impl Mark for Subject {}",
        "struct Subject {} comptime impl Subject {fn value()->bool {true}}",
    ] {
        let error = compile(source, Emit::Nir)
            .err()
            .expect("phase selection remains unfinished");
        assert!(
            error.contains("comptime implementation phase selection is not yet implemented"),
            "{error}"
        );
    }
}

#[test]
fn ordinary_implementations_continue_to_elaborate() {
    assert!(
        compile(
            "trait Mark {} struct Subject {} impl Mark for Subject {} fn run()->bool {true}",
            Emit::Nir
        )
        .is_ok()
    );
}
