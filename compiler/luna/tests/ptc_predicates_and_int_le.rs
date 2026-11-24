//! End-to-end tests for unified `Type.predicates.*` short/legacy names,
//! `TypeKind::{Char, Record, Union, Module}`, A11 `Type.predicates.is_not_sized`,
//! and A12 `IntLe` in `requires` / `assumes`.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::Value;
use luna::cli::{self, Command};
use luna::frontend::{self, Artifact};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

fn check_source(source: &str) -> Result<nir::program::Package, String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "luna-ptc-predicates-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&path, source).unwrap();
    let library = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let Command::Check(options) = cli::parse([
        "check".into(),
        path.clone().into_os_string(),
        "--sysroot".into(),
        library.into_os_string(),
        "--build-target".into(),
        "x86_64".into(),
        "linux".into(),
        "64".into(),
    ])
    .unwrap() else {
        panic!("expected check command");
    };
    let compiled = frontend::compile(&options);
    let _ = fs::remove_file(&path);
    match compiled? {
        Artifact::Nir(package) => Ok(package),
        _ => Err("expected NIR artifact".into()),
    }
}

fn run_entry(source: &str) -> Result<Value, String> {
    let package = check_source(source)?;
    let graph = luna::instantiation::instantiate(&package, "run", 1_000_000)?;
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default())
        .map_err(|e| format!("{e:?}"))?;
    vm.invoke_instance(&graph.roots[0], vec![])
        .map_err(|e| format!("{e:?}"))
}

#[test]
fn short_and_legacy_type_predicates_and_new_kinds_work_in_comptime_and_requires() {
    let source = r#"
        struct Plain { value: u32 }
        enum Choice { left, right }
        newtype RegisterId = u32;
        comptime fn schema(...properties: T) -> T where T { properties }
        const SAMPLE_RECORD = schema { a: true, b: false };
        typealias SampleRecord = SAMPLE_RECORD'type;

        comptime fn check_short_and_new_kinds() -> bool {
            Type.predicates.is_bool(bool)
                and Type.predicates.is_boolean(bool)
                and not Type.predicates.is_bool(char)
                and Type.predicates.is_char(char)
                and not Type.predicates.is_char(u32)
                and Type.predicates.is_int(i32)
                and Type.predicates.is_int(u64)
                and Type.predicates.is_int(Integer)
                and Type.predicates.is_signed_int(i16)
                and Type.predicates.is_unsigned_int(u16)
                and Type.predicates.is_float(f32)
                and Type.predicates.is_float(Float)
                and Type.predicates.is_fn(fn(u32) -> bool)
                and Type.predicates.is_record(SampleRecord)
                and not Type.predicates.is_record(Tuple<bool, u32>)
                and not Type.predicates.is_record(Plain)
                and not Type.predicates.is_union(Plain)
                and not Type.predicates.is_union(Choice)
                and not Type.predicates.is_module(u32)
                and not Type.predicates.is_module(SampleRecord)
        }

        const SHORT_OK = check_short_and_new_kinds();

        fn require_char(x: T) -> bool
        where T, requires Type.predicates.is_char(T)
        {
            true
        }

        fn require_record_type() -> bool
        where T, requires Type.predicates.is_record(T)
        {
            assumes Type.predicates.is_record(T);
            true
        }

        fn require_short_int(x: T) -> bool
        where T, requires Type.predicates.is_int(T) and not Type.predicates.is_bool(T)
        {
            true
        }

        fn classify(value: T) -> u8 where T {
            inline when {
                Type.predicates.is_bool(T) => 1,
                Type.predicates.is_char(T) => 2,
                Type.predicates.is_signed_int(T) => 3,
                Type.predicates.is_unsigned_int(T) => 4,
                Type.predicates.is_float(T) => 5,
                Type.predicates.is_fn(T) => 6,
                Type.predicates.is_record(T) => 7,
                else => 0,
            }
        }

        fn helper(x: u32) -> bool { true }

        fn run() -> bool {
            let signed_v: i32 = -1;
            let unsigned_v: u32 = 2;
            let float_v: f64 = 1.0;
            SHORT_OK
                and require_char('a')
                and require_record_type<SampleRecord>()
                and require_short_int(unsigned_v)
                and classify(true) == 1
                and classify('z') == 2
                and classify(signed_v) == 3
                and classify(unsigned_v) == 4
                and classify(float_v) == 5
                and classify(helper) == 6
                and classify(Plain { value: 1 }) == 0
        }
    "#;
    assert_eq!(run_entry(source), Ok(Value::Bool(true)));
}

#[test]
fn is_not_sized_works_in_requires_and_rejects_comptime_only_prerequisites() {
    let valid = r#"
        newtype RegisterId = u32;
        struct InstructionOperands {
            destination: RegisterId,
            sources: Tuple<u32, u32>,
        }

        fn accept_sized(x: T) -> bool
        where T, requires not Type.predicates.is_not_sized(T)
        {
            true
        }

        fn require_concrete_operands() -> bool
        where requires not Type.predicates.is_not_sized(InstructionOperands)
        {
            assumes not Type.predicates.is_not_sized(InstructionOperands);
            true
        }

        fn forward_sized(x: T) -> bool
        where T, requires not Type.predicates.is_not_sized(T)
        {
            accept_sized(x)
        }

        fn run() -> bool {
            let v: u32 = 7;
            let one: u32 = 1;
            let two: u32 = 2;
            let ops = InstructionOperands {
                destination: v.as(RegisterId),
                sources: (one, two),
            };
            accept_sized(v)
                and accept_sized(v.ref)
                and accept_sized((one, true))
                and accept_sized(ops)
                and forward_sized(v)
                and require_concrete_operands()
        }
    "#;
    assert_eq!(run_entry(valid), Ok(Value::Bool(true)));

    // Comptime-only types must fail prerequisite checking (even negated or inside a tautology).
    for bad_ty in ["Type", "Integer", "Float"] {
        let bad_negated = format!(
            r#"
            fn bad() -> bool
            where requires not Type.predicates.is_not_sized({bad_ty})
            {{
                true
            }}
            fn run() -> bool {{ bad() }}
            "#
        );
        let err = check_source(&bad_negated).expect_err("comptime-only type must be rejected");
        assert!(
            err.contains("comptime-only") || err.contains("is_not_sized"),
            "unexpected error for {bad_ty}: {err}"
        );
    }

    // Comptime-only user nominals (including nested nominals and compounds) must be rejected
    // even inside tautologies like `P or not P`, both directly and after generic instantiation.
    let comptime_nominal_cases = [
        r#"
        struct Holder { x: Type }
        fn bad() -> bool
        where requires Type.predicates.is_not_sized(Holder) or not Type.predicates.is_not_sized(Holder)
        {
            true
        }
        fn run() -> bool { bad() }
        "#,
        r#"
        struct Holder { x: Type }
        fn poly() -> bool
        where T, requires Type.predicates.is_not_sized(T) or not Type.predicates.is_not_sized(T)
        {
            true
        }
        fn run() -> bool { poly<Holder>() }
        "#,
        r#"
        struct Holder { x: Type }
        fn bad_opt() -> bool
        where requires not Type.predicates.is_not_sized(?Holder)
        {
            true
        }
        fn run() -> bool { bad_opt() }
        "#,
        r#"
        struct Holder { x: Type }
        fn bad_tuple() -> bool
        where requires not Type.predicates.is_not_sized(Tuple<Holder, u32>)
        {
            true
        }
        fn run() -> bool { bad_tuple() }
        "#,
        r#"
        struct Outer { h: Holder }
        struct Holder { x: Type }
        fn bad_outer() -> bool
        where requires Type.predicates.is_not_sized(Outer) or not Type.predicates.is_not_sized(Outer)
        {
            true
        }
        fn run() -> bool { bad_outer() }
        "#,
    ];
    for (idx, case) in comptime_nominal_cases.iter().enumerate() {
        let err = check_source(case).expect_err(&format!(
            "comptime-only nominal case {idx} must be rejected"
        ));
        assert!(
            err.contains("comptime-only") || err.contains("is_not_sized"),
            "unexpected error for comptime-only nominal case {idx}: {err}"
        );
    }

    // Unconstrained generic T cannot satisfy `not is_not_sized(T)`.
    let unresolved = r#"
        fn need_sized(x: T) -> bool
        where T, requires not Type.predicates.is_not_sized(T)
        {
            true
        }
        fn caller(x: T) -> bool where T {
            need_sized(x)
        }
        fn run() -> bool {
            let v: u32 = 1;
            caller(v)
        }
    "#;
    assert!(check_source(unresolved).is_err());

    // Expression-level `is_not_sized` works at comptime, in runtime bodies, and with `requires`.
    let expr_level = r#"
        struct Pair { a: u8, b: u64 }
        const C_U32: bool = Type.predicates.is_not_sized(u32);
        const C_PAIR: bool = Type.predicates.is_not_sized(Pair);
        fn check_req() -> bool
        where T, requires not Type.predicates.is_not_sized(T)
        {
            not Type.predicates.is_not_sized(T)
        }
        fn run() -> bool {
            not C_U32
                and not C_PAIR
                and not Type.predicates.is_not_sized(u32)
                and not Type.predicates.is_not_sized(Pair)
                and not Type.predicates.is_not_sized(*u8)
                and check_req<u32>()
        }
    "#;
    assert_eq!(run_entry(expr_level), Ok(Value::Bool(true)));

    // Polymorphic expression-level `is_not_sized(T)` resolves at instantiation
    // and rejects comptime-only instantiations cleanly.
    let poly_sized = r#"
        fn check_open() -> bool where T {
            Type.predicates.is_not_sized(T)
        }
        fn run() -> bool { check_open<u32>() }
    "#;
    assert_eq!(run_entry(poly_sized), Ok(Value::Bool(false)));

    let poly_comptime_only = r#"
        fn check_open() -> bool where T {
            Type.predicates.is_not_sized(T)
        }
        fn run() -> bool { check_open<Type>() }
    "#;
    assert!(run_entry(poly_comptime_only).is_err());

    // Expression-level `is_not_sized` rejects comptime-only types (IllFormed / Type error).
    for bad in [
        "Type",
        "Integer",
        "Float",
        "meta.List<u32>",
        "Type.vars.Closure",
        "Type.vars.EffectRow",
        "Type.vars.ErrorRow",
        "Type.vars.Reachability",
        "Type.vars.Any",
        "comptime fn() -> Type",
    ] {
        let src = format!("fn run() -> bool {{ Type.predicates.is_not_sized({bad}) }}");
        assert!(
            check_source(&src).is_err(),
            "expression-level is_not_sized({bad}) must fail"
        );
    }
}

#[test]
fn int_le_in_requires_and_assumes_proves_difference_bounds_and_rejects_violations() {
    let valid = r#"
        fn bounded() -> bool
        where n: Integer, requires 0 <= n and n < 10
        {
            assumes -1 <= n;
            assumes n <= 9;
            true
        }

        fn transitive() -> bool
        where a: Integer, b: Integer, c: Integer,
              requires a + 1 <= b and b + 2 <= c
        {
            assumes a + 3 <= c;
            assumes a < c;
            true
        }

        fn run() -> bool {
            bounded<0>()
                and bounded<9>()
                and transitive<1, 2, 4>()
        }
    "#;
    assert_eq!(run_entry(valid), Ok(Value::Bool(true)));

    // Out-of-bounds call site must be rejected.
    let oob = r#"
        fn bounded() -> bool
        where n: Integer, requires 0 <= n and n < 10
        {
            true
        }
        fn run() -> bool { bounded<10>() }
    "#;
    assert!(check_source(oob).is_err());

    // Stronger assumption not entailed by premises must be rejected.
    let unentailed = r#"
        fn bad() -> bool
        where a: Integer, b: Integer, c: Integer,
              requires a + 1 <= b and b + 2 <= c
        {
            assumes a + 4 <= c;
            true
        }
        fn run() -> bool { bad<1, 2, 4>() }
    "#;
    assert!(check_source(unentailed).is_err());

    // Substituting an integer outside i128 range into a recursive `IntLe` requirement
    // must fail cleanly instead of leaking the original rigid parameter `n`.
    let huge_recursive = r#"
        fn bad() -> bool
        where n: Integer, requires n <= 10
        {
            bad<100000000000000000000000000000000000000000>()
        }
        fn run() -> bool { bad<0>() }
    "#;
    assert!(check_source(huge_recursive).is_err());
}
