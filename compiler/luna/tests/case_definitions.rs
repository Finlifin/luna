//! Frontend lowering and diagnostics for custom pattern view (`case`) definitions.
use luna::cli::{Emit, Options};
use luna::frontend::{self, Artifact};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn compile(source: &str, emit: Emit) -> Result<Artifact, String> {
    let path = std::env::temp_dir().join(format!(
        "luna-case-def-{}-{}.fl",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, source).unwrap();
    let result = frontend::compile(&Options {
        dispatch_fuel: None,
        comptime_intrinsics: Vec::new(),
        input: path.clone(),
        emit,
        output: None,
        build_modes: Default::default(),
        build_targets: Default::default(),
        sysroot: None,
        splice_protocol: None,
        meta_list_type: None,
        tuple_type: None,
        index_protocol: None,
        ord_protocol: None,
        eq_protocol: None,
        list_pattern_protocol: None,
        comptime_list_pattern_protocol: None,
        stable_list_pattern_protocol: None,
        iterator_protocol: None,
        into_iterator_protocol: None,
        literal_into_protocol: None,
        arithmetic_protocols: Default::default(),
        incremental: Default::default(),
    });
    std::fs::remove_file(path).unwrap();
    result
}

#[test]
fn case_definitions_lower_to_hir_across_containers_modifiers_and_bodies() {
    let source = r#"
typealias Type = .Type'builtin;
typealias bool = .bool'builtin;
typealias i32 = .i32'builtin;
typealias u32 = .u32'builtin;
typealias str = .str'builtin;
typealias Object = .Object'builtin;
trait Viewable {
    case req(*self) -> ?bool { null }
}
mod views {
    pub case positive(value: i32) -> ?i32 = if value > 0 { value } else { null };
}
struct Student {
    name: str,
    age: u32,
    pub case adult(*self) -> out: ?u32
    where ensures true {
        if self.age >= 18 { self.age } else { null }
    }
    private pure case has_age(self, .min: u32 = 0) -> ?u32 {
        if self.age >= min { self.age } else { null }
    }
}
enum Packet {
    empty,
    value(u32),
    pub inline case non_empty(*self) -> ?u32 {
        self.* match {
            .value(v) => v,
            .empty => null,
        }
    }
}
impl Student {
    comptime case ct_view(comptime flag: bool) -> ?Type {
        if flag { bool } else { null }
    }
    unsafe case raw_view(*self) -> ?u32 { self.age }
}
impl Viewable for Student {
    case req(*self) -> ?bool { self.age >= 18 }
}
comptime fn tag(name: str) -> Object { let meta = { tag: name }; meta }
^tag("student")
case annotated(student: Student) -> ?u32 { student.age }
"#;
    let Artifact::Hir(dump) = compile(source, Emit::Hir).unwrap() else {
        panic!("expected HIR artifact")
    };
    for name in [
        "req",
        "positive",
        "adult",
        "has_age",
        "non_empty",
        "ct_view",
        "raw_view",
        "annotated",
    ] {
        let needle = format!("(name (ident (name \"{name}\")");
        assert!(
            dump.lines()
                .any(|line| line.contains(&needle) && line.contains("(kind (case (signature")),
            "missing case HIR owner `{name}` in:\n{dump}"
        );
    }
    let req_count = dump
        .lines()
        .filter(|line| {
            line.contains("(name (ident (name \"req\")") && line.contains("(kind (case (signature")
        })
        .count();
    assert_eq!(req_count, 2, "expected Viewable + impl req cases:\n{dump}");
    for expected in [
        "(inline true)",
        "(pure true)",
        "(comptime true)",
        "(unsafe true)",
        "(deferred-attribute ",
    ] {
        assert!(dump.contains(expected), "missing `{expected}` in:\n{dump}");
    }
    assert!(
        !dump.contains("unsupported struct body entry"),
        "unexpected unsupported struct body entry pending marker:\n{dump}"
    );
    assert!(
        !dump.contains("pending \"top-level CaseDef\""),
        "CaseDef should lower cleanly to HIR:\n{dump}"
    );
}

#[test]
fn non_attribute_case_definitions_leave_no_pending_semantics_and_fail_cleanly_in_elaboration() {
    let source = r#"
typealias bool = .bool'builtin;
typealias i32 = .i32'builtin;
struct Pair where T {
    left: T,
    right: T,
    pub case equal(*self) -> ?T
    where .strict: bool = true {
        null
    }
}
case top(x: i32) -> ?i32 = x;
"#;
    let Artifact::Hir(dump) = compile(source, Emit::Hir).unwrap() else {
        panic!("expected HIR artifact")
    };
    assert!(
        dump.contains("(pending-semantics))"),
        "expected empty pending-semantics for CaseDef HIR:\n{dump}"
    );
    let err = compile(source, Emit::Nir)
        .err()
        .expect("elaboration should report unimplemented CaseDef cleanly");
    assert!(
        err.contains("elaboration is not implemented for case definition"),
        "{err}"
    );
}

#[test]
fn invalid_case_definitions_diagnose_cleanly() {
    for (source, expected) in [
        (
            "quote case bad(x: i32) -> ?i32 { null }",
            "quote modifier is not valid on case declarations",
        ),
        (
            "case dup(x: i32, x: i32) -> ?i32 { null }",
            "duplicate function parameter",
        ),
        (
            "case dup_ret(x: i32) -> x: ?i32 { null }",
            "result name duplicates a function parameter",
        ),
    ] {
        let err = compile(source, Emit::Hir)
            .err()
            .unwrap_or_else(|| panic!("expected HIR diagnostic for `{source}`"));
        assert!(err.contains(expected), "expected `{expected}` in `{err}`");
    }
}
