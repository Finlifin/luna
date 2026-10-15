use luna::{
    cli::{Emit, Options},
    frontend::{self, Artifact},
};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

fn compile(source: &str, emit: Emit) -> Result<Artifact, String> {
    let path = std::env::temp_dir().join(format!(
        "iteration-{}-{}.fl",
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
        optional_type: None,
        index_protocol: None,
        literal_into_protocol: None,
        arithmetic_protocols: Default::default(),
    });
    std::fs::remove_file(path).unwrap();
    result
}

#[test]
fn iteration_hir_preserves_protocol_input_pattern_and_loop_targets() {
    let source = "fn run(items:bool) {for:outer (left,right) in items {let saved=left;while right {continue outer;}break outer;}}";
    let Artifact::Hir(dump) = compile(source, Emit::Hir).unwrap() else {
        panic!("wrong stage")
    };
    assert!(dump.contains("(for (label"), "{dump}");
    assert!(dump.contains("(iterable (expr"), "{dump}");
    assert!(
        dump.contains("(break-to ") && dump.contains("(continue-to "),
        "{dump}"
    );
    let before = dump.split("(kind (for ").next().unwrap();
    let identity = before
        .rsplit("(expr (id ")
        .next()
        .unwrap()
        .strip_suffix(") ")
        .unwrap();
    assert!(
        dump.contains(&format!("(continue-to (target {identity}))")),
        "{dump}"
    );
    assert!(
        dump.contains(&format!("(break-to (target {identity}))")),
        "{dump}"
    );
    assert!(
        !dump.contains("__into_iter__") && !dump.contains("__next__") && !dump.contains("__iter__")
    );
    let failure = compile(source, Emit::Nir)
        .err()
        .expect("protocol must be selected before NIR");
    assert!(
        failure.contains("iteration protocol elaboration"),
        "{failure}"
    );
}

#[test]
fn iteration_bindings_do_not_capture_the_input_or_escape_the_body() {
    for source in [
        "fn run(item:bool) {for item in item {let copy=item;}let copy=item;}",
        "fn run(items:bool) {for item in items {for item in item {let copy=item;}}}",
        "fn run(items:bool) {for item if item in items {let copy=item;}}",
    ] {
        compile(source, Emit::Hir).unwrap();
    }
    for source in [
        "fn run(items:bool) {for item in items {} let escaped=item;}",
        "fn run(items:bool) {for:inside item in items {} break inside;}",
        "fn run(items:bool) {for:label item in items {let label=true;break label;}}",
    ] {
        assert!(compile(source, Emit::Hir).is_err(), "{source}");
    }
}
