//! End-to-end integration tests for `meta.patterns` (`PatternViewErr`, `PatternShape`,
//! `TypePatternView`), `--type-pattern-protocol`, normal-form head opacity, and
//! contextual dependent normal-form value pattern elaboration (`GAP-03-30`).
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::Value;
use luna::{
    cli::{self, Command},
    frontend,
};
use std::sync::atomic::{AtomicUsize, Ordering};

fn invoke_with_args(source: &str, entry: &str, extra_args: &[&str]) -> Result<Value, String> {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "luna-type-pattern-view-{}-{}.fl",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, source).unwrap();
    let library = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let mut args = vec![
        "check".into(),
        path.clone().into_os_string(),
        "--sysroot".into(),
        library.into_os_string(),
        "--build-target".into(),
        "x86_64".into(),
        "linux".into(),
        "64".into(),
    ];
    for arg in extra_args {
        args.push((*arg).into());
    }
    let Command::Check(options) = cli::parse(args).map_err(|err| err.to_string())? else {
        panic!("expected check command");
    };
    let result = frontend::invoke_comptime(&options, entry, 10_000_000, None, None)
        .map_err(|err| err.to_string());
    let _ = std::fs::remove_file(&path);
    result
}

#[test]
fn meta_patterns_exports_and_type_pattern_view_work_with_sysroot() {
    let source = r#"
        use meta.patterns.{PatternShape, PatternViewErr, TypePatternView};

        pure comptime fn Wrap<A, B> -> Type { A }
        pure comptime fn Single<T> -> Type { T }

        struct WrapView {}
        comptime impl TypePatternView for WrapView {
            assoc Args: Type = Tuple<Type, Type>;

            pure comptime fn type_pattern_view_destructure(target: Type) -> ?Args {
                target match {
                    Wrap<a, b> => (a, b),
                    _ => null,
                }
            }
        }

        struct SingleView {}
        comptime impl TypePatternView for SingleView {
            assoc Args: Type = Type;

            pure comptime fn type_pattern_view_destructure(target: Type) -> ?Args {
                target match {
                    Single<inner> => inner,
                    _ => null,
                }
            }
        }

        struct TaggedView where tag: Type {}
        comptime impl TypePatternView for TaggedView<tag> where tag: Type {
            assoc Args: Type = Type;

            pure comptime fn type_pattern_view_destructure(target: Type) -> ?Args {
                target match {
                    Wrap<first, second> => if first == tag { second } else { null },
                    _ => null,
                }
            }
        }

        struct PairRecord {
            left: Type,
            right: Type,
        }

        struct RecordView {}
        comptime impl TypePatternView for RecordView {
            assoc Args: Type = PairRecord;

            pure comptime fn type_pattern_view_destructure(target: Type) -> ?Args {
                target match {
                    Wrap<a, b> => PairRecord { left: a, right: b },
                    _ => null,
                }
            }
        }

        comptime fn classify_err(e: PatternViewErr) -> u8 {
            e match {
                .InvalidArity => 1,
                .ExpectedConstantSlot => 2,
                .InvalidPatternInput => 3,
                .TypeMismatch => 4,
                .UnsupportedSlotShape => 5,
                .DisallowedContextEscape => 6,
            }
        }

        comptime fn classify_shape(s: PatternShape) -> u8 {
            s match {
                .wildcard => 1,
                .binding => 2,
                .constant => 3,
                .tuple => 4,
                .record => 5,
                .list => 6,
                .typed_binding => 7,
                .complex => 8,
            }
        }

        comptime fn test_types_and_views() -> bool {
            let err_ok = classify_err(.InvalidArity) == 1
                and classify_err(.DisallowedContextEscape) == 6;
            let shape_ok = classify_shape(.wildcard) == 1
                and classify_shape(.complex) == 8;
            let positional_hit = Wrap<u8, bool> match {
                WrapView<a, b> => a == u8 and b == bool,
                _ => false,
            };
            let fallthrough_ok = Single<u16> match {
                WrapView<_, _> => false,
                SingleView<inner> => inner == u16,
                _ => false,
            };
            let applied_hit = Wrap<u32, str> match {
                TaggedView<u16><_> => false,
                TaggedView<u32><payload> => payload == str,
                _ => false,
            };
            let named_hit = Wrap<i32, bool> match {
                RecordView<right: r, left: l> => l == i32 and r == bool,
                _ => false,
            };
            err_ok and shape_ok and positional_hit and fallthrough_ok and applied_hit and named_hit
        }
    "#;
    assert_eq!(
        support::invoke(source, "test_types_and_views").unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn type_pattern_view_preserves_normal_form_opacity_arity_and_exhaustiveness() {
    let prefix = r#"
        use meta.patterns.TypePatternView;

        pure comptime fn Identity<T> -> Type { T }
        pure comptime fn Wrap<A, B> -> Type { A }

        struct WrapView {}
        comptime impl TypePatternView for WrapView {
            assoc Args: Type = Tuple<Type, Type>;

            pure comptime fn type_pattern_view_destructure(target: Type) -> ?Args {
                target match {
                    Wrap<a, b> => (a, b),
                    _ => null,
                }
            }
        }
    "#;

    let opaque = format!(
        r#"{prefix}
        comptime fn run(v: Type) -> bool {{
            v match {{
                Identity<WrapView><a, b> => true,
                _ => false,
            }}
        }}"#
    );
    let err = support::compile(&opaque).unwrap_err();
    assert!(
        err.contains("pattern head is not a normal-form definition"),
        "{err}"
    );

    let wrong_arity = format!(
        r#"{prefix}
        comptime fn run(v: Type) -> bool {{
            v match {{
                WrapView<a> => true,
                _ => false,
            }}
        }}"#
    );
    let err = support::compile(&wrong_arity).unwrap_err();
    assert!(
        err.contains("type pattern view argument count mismatch"),
        "{err}"
    );

    let non_exhaustive = format!(
        r#"{prefix}
        comptime fn run(v: Type) -> bool {{
            v match {{
                WrapView<_, _> => true,
            }}
        }}"#
    );
    let err = support::compile(&non_exhaustive).unwrap_err();
    assert!(err.contains("non-exhaustive match"), "{err}");
}

#[test]
fn cli_type_pattern_protocol_selector_overrides_default() {
    let source = r#"
        trait CustomTypeView {
            assoc Args: Type;
            pure comptime fn type_pattern_view_destructure(target: Type) -> ?Args;
        }

        pure comptime fn Boxed<T> -> Type { T }

        struct MyView {}
        impl CustomTypeView for MyView {
            assoc Args: Type = Type;

            pure comptime fn type_pattern_view_destructure(target: Type) -> ?Args {
                target match {
                    Boxed<inner> => inner,
                    _ => null,
                }
            }
        }

        comptime fn run() -> bool {
            Boxed<u64> match {
                MyView<x> => x == u64,
                _ => false,
            }
        }
    "#;
    assert_eq!(
        invoke_with_args(
            source,
            "run",
            &["--type-pattern-protocol", "input.CustomTypeView"],
        )
        .unwrap(),
        Value::Bool(true)
    );

    let bad_trait = r#"
        trait BadTypeView {
            assoc Args: Type;
            fn type_pattern_view_destructure(target: Type) -> ?Args;
        }
        comptime fn run() -> bool { true }
    "#;
    let err = invoke_with_args(
        bad_trait,
        "run",
        &["--type-pattern-protocol", "input.BadTypeView"],
    )
    .unwrap_err();
    assert!(
        err.contains(
            "TypePatternView type_pattern_view_destructure must be a pure comptime function"
        ),
        "{err}"
    );
}

#[test]
fn dependent_normal_form_value_patterns_contextual_elaboration() {
    let source = r#"
        struct Payload where len: usize {
            tag: u8,
        }

        pure comptime fn Buffer<len: usize, body: Payload<len>> -> Type {
            Type
        }

        comptime fn match_clause_len(input: Type, expected_tag: u8) -> bool where Len: usize {
            let bound = input match {
                Buffer<'Len, payload> => payload.tag == expected_tag,
                _ => false,
            };
            let struct_match = input match {
                Buffer<'Len, { tag: t }> => t == expected_tag,
                _ => false,
            };
            bound and struct_match
        }

        comptime fn match_guarded_len(input: Type) -> u8 where Len: usize {
            input match {
                Buffer<(n if n == Len), payload> => payload.tag,
                _ => 0,
            }
        }

        comptime fn run() -> bool {
            let p4 = Payload<4> { tag: 9 };
            let b4 = Buffer<4, p4>;
            let b5 = Buffer<5, Payload<5> { tag: 9 }>;
            match_clause_len<4>(b4, 9)
                and not match_clause_len<4>(b5, 9)
                and match_guarded_len<4>(b4) == 9
                and match_guarded_len<4>(b5) == 0
        }
    "#;
    assert_eq!(support::invoke(source, "run").unwrap(), Value::Bool(true));

    let eq_source = r#"
        use meta.ComptimeEq;

        struct Payload where len: usize {
            tag: u8,
        }

        comptime impl ComptimeEq for Payload<len> where len: usize {
            comptime fn eq(self, other: Self) -> bool {
                self.tag == other.tag
            }
        }

        pure comptime fn Buffer<len: usize, body: Payload<len>> -> Type {
            Type
        }

        comptime fn match_exact(input: Type, expected: Payload<Len>) -> bool where Len: usize {
            input match {
                Buffer<'Len, 'expected> => true,
                _ => false,
            }
        }

        comptime fn run() -> bool {
            let p4 = Payload<4> { tag: 9 };
            let other4 = Payload<4> { tag: 3 };
            let b4 = Buffer<4, p4>;
            let b5 = Buffer<5, Payload<5> { tag: 9 }>;
            match_exact<4>(b4, p4)
                and not match_exact<4>(b4, other4)
                and not match_exact<4>(b5, p4)
        }
    "#;
    assert_eq!(
        invoke_with_args(
            eq_source,
            "run",
            &["--eq-protocol", "meta.numeric.equality.ComptimeEq"]
        )
        .unwrap(),
        Value::Bool(true)
    );

    let invalid = r#"
        pure comptime fn Box<T, value: T> -> Type { T }
        comptime fn bad(v: Type) -> bool {
            v match {
                Box<T, item> => item,
                _ => false,
            }
        }
    "#;
    let err = support::compile(invalid).unwrap_err();
    assert!(
        err.contains("dependent normal-form value patterns require exact candidate evidence"),
        "{err}"
    );
    assert!(!err.contains("not yet implemented"), "{err}");
}
