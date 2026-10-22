use luna::{
    cli::{Emit, Options},
    frontend::{self, Artifact},
};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

fn compile(source: &str, emit: Emit) -> Result<Artifact, String> {
    let path = std::env::temp_dir().join(format!(
        "error-surface-{}-{}.fl",
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
fn error_surface_retains_rows_propagation_and_catch_roles() {
    let source = "enum InputErr {Missing,}
        fn read()->![InputErr] bool {error InputErr.Missing}
        fn forward()->!InputErr bool {read()!}
        fn run()->bool {read()! {
            catch before => {before;},
            InputErr.Missing => false,
            catch after => {after;true},
        }}";
    let Artifact::Hir(dump) = compile(source, Emit::Hir).unwrap() else {
        panic!("HIR")
    };
    for marker in [
        "error-qualified-type",
        "(errors ",
        "(result ",
        "error-new",
        "error-propagate",
        "error-handle",
        "(before (",
        "(after (",
    ] {
        assert!(dump.contains(marker), "missing {marker}: {dump}");
    }
    assert!(!dump.contains("invalid"), "{dump}");
}

#[test]
fn single_catch_is_trailing_and_accepts_patterns() {
    for pattern in ["_", "failure", "error payload", "value!"] {
        let source = format!("fn run(input:bool)->bool {{input! {{catch {pattern} => true,}}}}");
        let Artifact::Hir(dump) = compile(&source, Emit::Hir).unwrap() else {
            panic!("HIR")
        };
        assert!(dump.contains("error-handle"), "{dump}");
        assert!(!dump.contains("invalid"), "{dump}");
    }
}

#[test]
fn ordinary_match_preserves_success_and_error_patterns_and_bindings() {
    let source =
        "fn run(input:bool)->bool {input match {value! => value, error failure => failure,}}";
    let Artifact::Hir(dump) = compile(source, Emit::Hir).unwrap() else {
        panic!("HIR")
    };
    assert!(dump.contains("error-ok"), "{dump}");
    assert!(dump.contains("error-err"), "{dump}");
    assert!(!dump.contains("invalid"), "{dump}");
}

#[test]
fn invalid_catch_positions_and_counts_are_diagnosed() {
    for arms in [
        "catch _ => true, _ => false,",
        "_ => false, catch _ => true, _ => false,",
        "catch _ => true, catch _ => true, catch _ => false,",
    ] {
        let source = format!("fn run(input:bool)->bool {{input! {{{arms}}}}}");
        let failure = compile(&source, Emit::Hir)
            .err()
            .expect("expected rejection");
        assert!(failure.contains("catch"), "{failure}");
    }
}

#[test]
fn catch_bindings_do_not_leak_to_other_arms() {
    let failure = compile(
        "fn run(input:bool)->bool {input! {catch first => first, _ => first, catch last => last,}}",
        Emit::Hir,
    )
    .err()
    .expect("expected rejection");
    assert!(failure.contains("first"), "{failure}");
}
