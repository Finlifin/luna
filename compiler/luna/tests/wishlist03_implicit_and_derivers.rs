//! Wave 2A integration tests (`OQ-03-04`, `OQ-03-05`, `WL-03-06`, `WL-03-08..10`,
//! `WL-03-15..20`, `WL-03-23`, `ROB-08-03`).
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::Value;
use std::fs;
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct NativeCase(PathBuf);

impl NativeCase {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "luna-w03-w2a-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("main.fl"), source).unwrap();
        Self(path)
    }

    fn build(&self, opt_level: &str) -> Output {
        ProcessCommand::new(env!("CARGO_BIN_EXE_luna"))
            .current_dir(&self.0)
            .args([
                "build",
                "main.fl",
                "--entry",
                "run",
                "--output",
                "program",
                "--opt-level",
                opt_level,
                "--fuel",
                "10000000",
                "--sysroot",
            ])
            .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library"))
            .output()
            .unwrap()
    }

    fn run(&self) -> Output {
        ProcessCommand::new(self.0.join("program"))
            .current_dir(&self.0)
            .output()
            .unwrap()
    }
}

impl Drop for NativeCase {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn implicit_passing_forbids_arbitrary_probing_and_supports_spreads_defaults_and_overrides() {
    // 1. Explicit `.name = expr` overrides `ImplicitPassing`, dependent defaults receive implicit
    //    caller/src values, and `...spread` works alongside implicit passing.
    let positive = r#"
        fn log_with(
            tag: str,
            implicit __caller__: str,
            implicit __line__: u32,
            implicit __src__: meta.SourceSpan,
            .line_hint: u32 = __line__,
        ) -> bool {
            tag == "ok"
                and __line__ > 0
                and line_hint == __line__
                and __caller__ == "input.outer"
                and __src__.start() <= __src__.end()
        }

        fn spread_implicit(
            ...items: Tuple<u32, u32>,
            implicit __package__: str,
        ) -> bool {
            let (a, b) = items;
            a == 10 and b == 20 and __package__ == "input"
        }

        fn override_caller(implicit __caller__: str) -> bool {
            __caller__ == "explicit_override"
        }

        fn outer() -> bool {
            let args = (10u32, 20u32);
            log_with("ok")
                and spread_implicit(...args)
                and override_caller(.__caller__ = "explicit_override")
        }

        fn run() -> bool {
            outer()
        }
    "#;
    let package = support::compile(positive).unwrap();
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 10_000_000)
            .unwrap(),
        Value::Bool(true)
    );

    // 2. CallDescription stays empty `{}` (probing fields or Context internals must fail).
    let probe_call_desc = r#"
        fn bad(calling: builtin.CallDescription) -> u32 {
            calling.line
        }
    "#;
    assert!(support::compile(probe_call_desc).is_err());

    let probe_context = r#"
        fn bad(ctx: builtin.Context) -> u32 {
            ctx.line
        }
    "#;
    assert!(support::compile(probe_context).is_err());

    // 3. Missing `ImplicitPassing` impl never captures a same-named local variable.
    let local_capture = r#"
        fn need_secret(implicit secret_token: u32) -> u32 {
            secret_token
        }

        fn run() -> u32 {
            let secret_token: u32 = 42;
            let _ = secret_token;
            need_secret()
        }
    "#;
    assert!(support::compile(local_capture).is_err());
}

#[test]
fn silver_record_reflection_narrowing_meta_structs_and_object_collector() {
    // 1. `Type.predicates.is_anonymous_record`, `has_field`, `FieldType`, `inline if` narrowing,
    //    `reflect_record(Ps)` + `FieldInfo.span()`, and `meta.structs.{field_ptr, field_value}`.
    let positive = r#"
        use std.mem.{Allocator, DefaultAllocator};

        struct Point {
            x: u32,
            y: u32,
        }

        pure comptime fn read_optional_u32(
            props: Ps,
            comptime key: Symbol,
            fallback: u32,
        ) -> u32
        where
            Ps: Type,
            requires Type.predicates.is_anonymous_record(Ps),
        {
            inline if Type.predicates.has_field(Ps, key) {
                inline if Type.predicates.eq(meta.structs.FieldType(Ps, key), u32) {
                    meta.structs.field_value(props, key)
                } else {
                    fallback
                }
            } else {
                fallback
            }
        }

        comptime fn diagnose_unknown_props(...props: Ps) -> str
        where
            Ps: Type,
            requires Type.predicates.is_anonymous_record(Ps),
        {
            let info = meta.reflection.reflect_record(Ps) match {
                r! => r,
                error _ => return "not-record",
            };
            let fields = info.fields();
            var i: Integer = 0;
            while i < fields.length() {
                let f = fields[i];
                f.name() match {
                    sym? => {
                        if sym != .color and sym != .width {
                            let s = f.span();
                            let prefix = "unknown property ".as(meta.ComptimeTextBytes)
                                .concat(sym'tag_name)
                                .as(meta.ComptimeTextBytes)
                                .concat(" at ")
                                .as(meta.ComptimeTextBytes)
                                .concat(s.start().as(meta.ComptimeDisplay).fmt())
                                .as(meta.ComptimeTextBytes)
                                .concat("..")
                                .as(meta.ComptimeTextBytes)
                                .concat(s.end().as(meta.ComptimeDisplay).fmt());
                            return prefix;
                        }
                    },
                    null => {},
                }
                i = i + 1;
            }
            "ok"
        }

        comptime fn inspect_record_props(...props: Ps) -> bool
        where
            Ps: Type,
            requires Type.predicates.is_anonymous_record(Ps),
        {
            let info = meta.reflection.reflect_record(Ps) match {
                r! => r,
                error _ => return false,
            };
            let w = read_optional_u32(props, .width, 0);
            let h = read_optional_u32(props, .height, 99);
            info.fields().length() == 1
                and info.fields()[0].name() match { s? => s == .width, null => false }
                and info.fields()[0].span().start() <= info.fields()[0].span().end()
                and w == 42
                and h == 99
        }

        comptime fn collect_object(...props: meta.Object) -> bool {
            let count = props.length();
            let got_w = props.get<u32>(.width) match { v? => v == 12, null => false };
            let got_tag = props.get<Symbol>(.tag) match { s? => s == .panel, null => false };
            count == 2 and got_w and got_tag
        }

        fn sum_point_fields(p: *Point) -> u32 {
            let x = meta.structs.field_ptr(p, .x).*;
            let y = meta.structs.field_ptr(p, .y).*;
            assumes x <= 1000000 and y <= 1000000;
            x + y
        }

        fn run() -> bool {
            const ct_ok = inspect_record_props { width: 42u32 }
                and collect_object { width: 12u32, tag: .panel }
                and diagnose_unknown_props { color: "red", width: 10u32 } == "ok"
                and diagnose_unknown_props { colour: "red" }.as(meta.ComptimeTextBytes).starts_with("unknown property colour at ");
            let pt = Point { x: 11, y: 31 };
            ct_ok and sum_point_fields(pt.ref) == 42
        }
    "#;
    let package = support::compile(positive).unwrap();
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 10_000_000)
            .unwrap(),
        Value::Bool(true)
    );

    // 2. Calling `meta.structs.field_ptr` or `field_value` without `has_field` proof fails.
    let unguarded_ptr = r#"
        struct Point { x: u32 }
        fn bad(p: *Point) -> u32 {
            meta.structs.field_ptr(p, .missing).*
        }
    "#;
    assert!(support::compile(unguarded_ptr).is_err());

    let unguarded_val = r#"
        struct Point { x: u32 }
        comptime fn bad(p: Point) -> u32 {
            meta.structs.field_value(p, .missing)
        }
    "#;
    assert!(support::compile(unguarded_val).is_err());
}

#[test]
fn tier1_runtime_by_fields_derivers_work_for_structs_enums_generics_and_native() {
    let source = r#"
        use std.collections.Vec;
        use std.string.String;

        struct VecBuf {
            bytes: Vec<u8>,
        }

        impl builtin.fmt.ByteWriter for VecBuf {
            assoc Err: Type = std.mem.AllocErr;

            fn write_byte(*self, byte: u8) -> !std.mem.AllocErr void {
                self.bytes.push(byte)
            }
        }

        struct Pair
        where
            T: Type,
        {
            first: T,
            second: T,
        }

        derive Eq for Pair<T> where T :- Eq;
        derive Ord for Pair<T> where T :- Ord;
        derive Hash for Pair<T> where T :- Hash;
        derive Default for Pair<T> where T :- Default;
        derive Clone for Pair<T> where T :- Clone;
        derive Copy for Pair<T> where T :- Copy;
        derive Debug for Pair<T> where T :- Debug;

        enum Shape {
            empty,
            circle(u32),
            rect(u32, u32),
        }

        derive Eq for Shape;
        derive Ord for Shape;
        derive Hash for Shape;
        derive Clone for Shape;
        derive Copy for Shape;
        derive Debug for Shape;

        struct FnvHasher {
            state: u64,
        }

        impl builtin.hash.Hasher for FnvHasher {
            fn finish(*self) -> u64 {
                self.state.*
            }

            fn write_u8(*self, value: u8) -> void {
                let bits = self.state.*'bitvec;
                let input = value'bitvec.as(u64)'bitvec;
                let mixed = ((bits and not input) or (not bits and input)).as(u128);
                assumes mixed <= 18446744073709551615;
                let product = mixed * 1099511628211;
                self.state.* = product'bitvec[0..64].as(u64);
            }

            fn write(*self, bytes: builtin.Slice<u8>) -> void {
                let len = bytes.len();
                var i: usize = 0;
                while i < len {
                    let b = bytes.get(i) match { v? => v, null => break };
                    self.write_u8(b);
                    i = i + 1;
                }
            }
        }

        fn hash_of(v: *T) -> u64
        where
            T: Type,
            requires T :- Hash,
        {
            var h = FnvHasher { state: 14695981039346656037 };
            v.as(Hash).hash(h.ref);
            h.as(builtin.hash.Hasher).finish()
        }

        fn run() -> u8 {
            let p0 = Pair<u32>.as(Default).default();
            let p1 = Pair<u32> { first: 1, second: 2 };
            let p1_copy = p1;
            let p1_clone = p1_copy.as(Clone).clone();
            let p2 = Pair<u32> { first: 1, second: 3 };

            let p0_expected = Pair<u32> { first: 0, second: 0 };
            let eq_ok = p0.as(Eq).eq(p0_expected.ref)
                and p1.as(Eq).eq(p1_clone.ref)
                and not p1.as(Eq).eq(p2.ref);
            let ord_ok = (p1.as(Ord).cmp(p2.ref) matches .less)
                and (p2.as(Ord).cmp(p1.ref) matches .greater)
                and (p1.as(Ord).cmp(p1_clone.ref) matches .equal);
            let hash_ok = hash_of(p1.ref) == hash_of(p1_clone.ref)
                and hash_of(p1.ref) != hash_of(p2.ref);

            let s0 = Shape.empty;
            let s1 = Shape.circle(5);
            let s1c = s1.as(Clone).clone();
            let s2 = Shape.rect(3, 4);
            let enum_ok = s1.as(Eq).eq(s1c.ref)
                and not s0.as(Eq).eq(s1.ref)
                and (s0.as(Ord).cmp(s1.ref) matches .less)
                and (s1.as(Ord).cmp(s2.ref) matches .less)
                and hash_of(s0.ref) != hash_of(s1.ref)
                and hash_of(s1.ref) == hash_of(s1c.ref);

            var buf = VecBuf { bytes: Vec<u8>.new() };
            let _ = p1.as(Debug).write_debug(buf.ref);
            let _ = s2.as(Debug).write_debug(buf.ref);

            if eq_ok and ord_ok and hash_ok and enum_ok and buf.bytes.len() > 0 {
                0
            } else {
                1
            }
        }
    "#;
    for level in ["0", "3"] {
        let case = NativeCase::new(source);
        let build = case.build(level);
        assert_success(&build);
        let output = case.run();
        assert_success(&output);
        assert_eq!(output.status.code(), Some(0));
    }

    // Negative: `derive Copy` must reject a struct that implements `Drop` or holds a non-`Copy` field.
    let copy_with_drop = r#"
        struct Guard { id: u32 }
        impl Clone for Guard {
            fn clone(*self) -> Self { Guard { id: self.id.* } }
        }
        impl Drop for Guard {
            fn drop(*self) {}
        }
        derive Copy for Guard;
    "#;
    assert!(support::compile(copy_with_drop).is_err());

    let copy_with_non_copy_field = r#"
        struct NonCopy { id: u32 }
        impl Clone for NonCopy {
            fn clone(*self) -> Self { NonCopy { id: self.id.* } }
        }
        struct Outer { inner: NonCopy }
        derive Clone for Outer;
        derive Copy for Outer;
    "#;
    assert!(support::compile(copy_with_non_copy_field).is_err());

    // Negative: `derive Default` rejects enums.
    let default_enum = r#"
        enum Choice { a, b(u32) }
        derive Default for Choice;
    "#;
    assert!(support::compile(default_enum).is_err());
}

#[test]
fn tier1_comptime_by_fields_derivers_are_pointer_free_and_cover_to_tokens() {
    let source = r#"
        use meta.{
            ComptimeClone,
            ComptimeDebug,
            ComptimeDefault,
            ComptimeDisplay,
            ComptimeEq,
            ComptimeHash,
            ComptimeOrd,
            DefaultComptimeHasher,
            ToTokens,
        };

        struct Config {
            port: u16,
            retries: u8,
        }

        comptime derive ComptimeEq for Config;
        comptime derive ComptimeOrd for Config;
        comptime derive ComptimeHash for Config;
        comptime derive ComptimeDefault for Config;
        comptime derive ComptimeClone for Config;
        comptime derive ComptimeDebug for Config;
        comptime derive ComptimeDisplay for Config;
        comptime derive ToTokens for Config;

        enum Mode {
            off,
            port(u16),
        }

        comptime derive ComptimeEq for Mode;
        comptime derive ComptimeOrd for Mode;
        comptime derive ComptimeHash for Mode;
        comptime derive ComptimeClone for Mode;
        comptime derive ComptimeDebug for Mode;
        comptime derive ComptimeDisplay for Mode;
        comptime derive ToTokens for Mode;

        comptime fn run() -> bool {
            let c0 = Config.as(ComptimeDefault).default();
            let c1 = Config { port: 8080, retries: 3 };
            let c1_copy = c1.as(ComptimeClone).clone();
            let c2 = Config { port: 9090, retries: 1 };

            let h0 = DefaultComptimeHasher.new().as(meta.ComptimeBuildHasher).hash_one(c1);
            let h1 = DefaultComptimeHasher.new().as(meta.ComptimeBuildHasher).hash_one(c1_copy);
            let h2 = DefaultComptimeHasher.new().as(meta.ComptimeBuildHasher).hash_one(c2);

            let m0 = Mode.off;
            let m1 = Mode.port(8080);
            let m1c = m1.as(ComptimeClone).clone();

            let tok_ok = not c1.as(ToTokens).to_tokens().is_empty()
                and not m1.as(ToTokens).to_tokens().is_empty();

            c0.as(ComptimeEq).eq(Config { port: 0, retries: 0 })
                and c1.as(ComptimeEq).eq(c1_copy)
                and not c1.as(ComptimeEq).eq(c2)
                and (c1.as(ComptimeOrd).cmp(c2) matches .less)
                and h0 == h1
                and h0 != h2
                and m1.as(ComptimeEq).eq(m1c)
                and not m0.as(ComptimeEq).eq(m1)
                and (m0.as(ComptimeOrd).cmp(m1) matches .less)
                and c1.as(ComptimeDebug).fmt() == "{port: 8080, retries: 3}"
                and c1.as(ComptimeDisplay).fmt() == "{port: 8080, retries: 3}"
                and m1.as(ComptimeDebug).fmt() == ".port(8080)"
                and m1.as(ComptimeDisplay).fmt() == ".port(8080)"
                and tok_ok
        }
    "#;
    assert_eq!(support::invoke(source, "run").unwrap(), Value::Bool(true));
}

#[test]
fn tactic_lambda_rejects_non_copy_duplicate_consumption_and_routes_post_do_with_collectors() {
    let post_do_with_collector = r#"
        use std.collections.Vec;

        fn fold_sum(
            init: i32,
            ...items: Vec<i32>,
            lambda combine: fn(i32, i32) -> i32,
        ) -> !std.mem.AllocErr i32 {
            var acc = init;
            let len = items.len();
            var i: usize = 0;
            while i < len {
                let x = items.get(i) match { v? => v, null => break };
                acc = combine(acc, x);
                i = i + 1;
            }
            acc
        }

        fn compute_fold() -> !std.mem.AllocErr i32 {
            (fold_sum(10, 1, 2, 3, 4) do {
                assumes _0 >= 0 and _0 < 1000 and _1 >= 0 and _1 < 1000;
                _0 + _1
            })!
        }

        fn run() -> bool {
            compute_fold() match {
                v! => v == 20,
                error _ => false,
            }
        }
    "#;
    let case = NativeCase::new(post_do_with_collector);
    let build = case.build("0");
    assert_success(&build);
    let out = case.run();
    assert_success(&out);
    assert_eq!(out.stdout, b"(bool true)\n");

    // Duplicate consumption of a non-Copy `_0` placeholder must be rejected.
    let dup_non_copy = r#"
        struct Token { id: u32 }
        fn take_two(a: Token, b: Token) -> u32 { a.id + b.id }
        fn apply(t: Token, lambda f: fn(Token) -> u32) -> u32 { f(t) }
        fn bad() -> u32 {
            apply(Token { id: 7 }) do { take_two(_0, _0) }
        }
    "#;
    assert!(support::compile(dup_non_copy).is_err());
}
