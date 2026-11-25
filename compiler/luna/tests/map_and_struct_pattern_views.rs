//! End-to-end integration tests for `MapPatternView` / `StableMapPatternView`
//! and `StructPatternView` / `StableStructPatternView` (both runtime and comptime phases),
//! including `std.collections.HashMap`, `meta.Map`, custom heterogeneous views,
//! OQ-3 (`...`), shorthand fields, nested `?V` null vs absent keys, dual-impl
//! ambiguity, `NoReturn` field rejection, and CLI protocol selectors.
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
            "luna-map-struct-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("main.fl"), source).unwrap();
        Self(path)
    }

    fn build(&self, extra_args: &[&str]) -> Output {
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
                "0",
                "--fuel",
                "10000000",
                "--sysroot",
            ])
            .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library"))
            .args(extra_args)
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
fn map_pattern_view_hashmap_symbol_str_string_and_nested_optional_natively() {
    let source = r#"
        use std.prelude.*;
        use std.collections.HashMap;
        use std.mem.AllocErr;
        use std.string.String;

        fn match_symbol_map() -> !AllocErr i64 {
            var m = HashMap<Symbol, i64>.new();
            let _ = m.ref.insert(.code, 40)!;
            let _ = m.ref.insert(.port, 2)!;
            m match {
                { code, port: 2, ... } => if code == 40 { 42 } else { 0 },
                { code: c, ... } => c,
                _ => 0,
            }
        }

        fn match_string_map() -> i64 {
            let m = HashMap<String, i64>.new();
            m match {
                { hello: h, world, ... } => if h == 7 and world == 35 { 42 } else { 0 },
                _ => 42,
            }
        }

        fn match_nested_optional_present_null() -> !AllocErr i64 {
            var m = HashMap<Symbol, ?i64>.new();
            let _ = m.ref.insert(.present_null, null)!;
            let _ = m.ref.insert(.present_val, 55)!;
            m match {
                { present_null: null, present_val: v?, ... } => if v == 55 { 56 } else { 0 },
                { present_null: _?, ... } => 2,
                _ => 0,
            }
        }

        fn match_nested_optional_absent_key_falls_through() -> !AllocErr i64 {
            var m = HashMap<Symbol, ?i64>.new();
            let _ = m.ref.insert(.other, 10)!;
            m match {
                { absent_key: null, ... } => 100,
                { absent_key: _?, ... } => 200,
                _ => 77,
            }
        }

        fn run_all() -> !AllocErr u8 {
            let s1 = match_symbol_map()! match {
                42 => true,
                _ => false,
            };
            let s3 = match_string_map() match {
                42 => true,
                _ => false,
            };
            let s4 = match_nested_optional_present_null()! match {
                56 => true,
                _ => false,
            };
            let s5 = match_nested_optional_absent_key_falls_through()! match {
                77 => true,
                _ => false,
            };
            if s1 and s3 and s4 and s5 { 0 } else { 1 }
        }

        pub fn run() -> u8 {
            run_all() match {
                code! => code,
                error _ => 2,
            }
        }
    "#;
    let case = NativeCase::new(source);
    assert_success(&case.build(&[]));
    let out = case.run();
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn map_pattern_view_comptime_meta_map_and_heterogeneous_mini_obj() {
    let source = r#"
        use std.prelude.*;
        use meta.Map;

        pure comptime fn mini_field(property: Symbol) -> Type {
            inline if property == .code {
                u64
            } else {
                inline if property == .flag {
                    bool
                } else {
                    NoReturn
                }
            }
        }

        struct MiniObj {
            code: u64,
            flag: bool,
        }

        impl MapPatternView for MiniObj {
            assoc FieldType: pure comptime fn(Symbol) -> Type = mini_field;

            fn map_pattern_view_get(*self, comptime property: Symbol) -> ?FieldType(property) {
                inline if property == .code {
                    self.code.*
                } else {
                    inline if property == .flag {
                        self.flag.*
                    } else {
                        null
                    }
                }
            }
        }

        impl StableMapPatternView for MiniObj {}

        comptime impl MapPatternView for MiniObj {
            assoc FieldTypeComptime: pure comptime fn(Symbol) -> Type = mini_field;

            comptime fn map_pattern_view_get_comptime(
                self,
                property: Symbol,
            ) -> ?FieldTypeComptime(property) {
                inline if property == .code {
                    self.code
                } else {
                    inline if property == .flag {
                        self.flag
                    } else {
                        null
                    }
                }
            }
        }

        comptime impl StableMapPatternView for MiniObj {}

        comptime fn check_comptime() -> bool {
            var m = Map<Symbol, Integer>.new();
            let _ = m.insert(.host, 100);
            let _ = m.insert(.port, 8080);
            let a = m match {
                { host, port: 8080, ... } => host + 1,
                _ => 0,
            };

            var opt_m = Map<Symbol, ?Integer>.new();
            let _ = opt_m.insert(.explicit_null, null);
            let _ = opt_m.insert(.explicit_val, 25);
            let b = opt_m match {
                { missing: null, ... } => 999,
                { explicit_null: null, explicit_val: v?, ... } => v,
                _ => 0,
            };

            let obj = MiniObj { code: 41, flag: true };
            let c = obj match {
                { code, flag: true, ... } => if code == 41 { 42 } else { 0 },
                _ => 0,
            };
            a == 101 and b == 25 and c == 42
        }

        const CT_OK: bool = check_comptime();

        pub fn run() -> u8 {
            let obj = MiniObj { code: 40, flag: true };
            let rt = obj match {
                { code, flag: true, ... } => if code == 40 { 42 } else { 0 },
                _ => 0,
            };
            if CT_OK and rt == 42 { 0 } else { 1 }
        }
    "#;
    let val = support::invoke(source, "check_comptime").unwrap();
    assert_eq!(val, Value::Bool(true));

    let case = NativeCase::new(source);
    assert_success(&case.build(&[]));
    let out = case.run();
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn struct_pattern_view_runtime_and_comptime_guaranteed_and_rest() {
    let source = r#"
        use std.prelude.*;

        pure comptime fn pair_field(field: Symbol) -> Type {
            inline if field == .left {
                i64
            } else if field == .tag {
                Symbol
            } else {
                NoReturn
            }
        }

        struct HiddenPair {
            raw_left: i64,
            raw_tag: Symbol,
        }

        impl StructPatternView for HiddenPair {
            assoc FieldType: pure comptime fn(Symbol) -> Type = pair_field;

            fn struct_pattern_view_get(*self, comptime field: Symbol) -> FieldType(field) {
                inline if field == .left {
                    self.raw_left.*
                } else if field == .tag {
                    self.raw_tag.*
                } else {
                    while true {}
                }
            }
        }

        impl StableStructPatternView for HiddenPair {}

        comptime impl StructPatternView for HiddenPair {
            assoc FieldTypeComptime: pure comptime fn(Symbol) -> Type = pair_field;

            comptime fn struct_pattern_view_get_comptime(
                self,
                field: Symbol,
            ) -> FieldTypeComptime(field) {
                inline if field == .left {
                    self.raw_left
                } else if field == .tag {
                    self.raw_tag
                } else {
                    while true {}
                }
            }
        }

        comptime impl StableStructPatternView for HiddenPair {}

        comptime fn check_comptime() -> i64 {
            let p = HiddenPair { raw_left: 20, raw_tag: .ok };
            let a = p match {
                HiddenPair { left, tag: .ok } => if left == 20 { 21 } else { 0 },
                _ => 0,
            };
            let b = p match {
                { left: v, ... } => v,
                _ => 0,
            };
            if a == 21 and b == 20 { 41 } else { 0 }
        }

        const CT_TOTAL: i64 = check_comptime();

        pub fn run() -> u8 {
            let p1 = HiddenPair { raw_left: 30, raw_tag: .ok };
            let rt_exact = p1 match {
                HiddenPair { left: x, tag: .ok } => x,
                _ => 0,
            };
            let p2 = HiddenPair { raw_left: 30, raw_tag: .ok };
            let rt_rest = p2 match {
                { left, ... } => left,
                _ => 0,
            };
            if CT_TOTAL == 41 and rt_exact == 30 and rt_rest == 30 { 0 } else { 1 }
        }
    "#;
    let val = support::invoke(source, "check_comptime").unwrap();
    assert_eq!(val, Value::SignedInteger(41));

    let case = NativeCase::new(source);
    assert_success(&case.build(&[]));
    let out = case.run();
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn map_and_struct_pattern_view_diagnostics_and_exhaustiveness() {
    // 1. Dual-impl ambiguity error (implements both StructPatternView and MapPatternView).
    let dual = r#"
        pure comptime fn ft(_prop: Symbol) -> Type { i64 }
        struct Both {}
        impl MapPatternView for Both {
            assoc FieldType: pure comptime fn(Symbol) -> Type = ft;
            fn map_pattern_view_get(*self, comptime prop: Symbol) -> ?FieldType(prop) { null }
        }
        impl StructPatternView for Both {
            assoc FieldType: pure comptime fn(Symbol) -> Type = ft;
            fn struct_pattern_view_get(*self, comptime field: Symbol) -> FieldType(field) { 0 }
        }
        fn bad(b: Both) -> i64 {
            b match {
                { a: x, ... } => x,
                _ => 0,
            }
        }
    "#;
    let err = support::compile(dual).unwrap_err();
    assert!(
        err.contains("ambiguous record pattern view"),
        "expected dual-impl ambiguity error, got: {err}"
    );

    // 2. Omitting `...` on open MapPatternView is rejected clearly.
    let no_rest = r#"
        use std.collections.HashMap;
        fn bad(m: HashMap<Symbol, i64>) -> i64 {
            m match {
                { code: x } => x,
                _ => 0,
            }
        }
    "#;
    let err = support::compile(no_rest).unwrap_err();
    assert!(
        err.contains("open MapPatternView pattern requires trailing `...`"),
        "expected missing `...` error, got: {err}"
    );

    // 3. NoReturn unknown field rejection in both MapPatternView and StructPatternView.
    let noreturn_struct = r#"
        pure comptime fn ft(field: Symbol) -> Type {
            inline if field == .ok { i64 } else { NoReturn }
        }
        struct View {}
        impl StructPatternView for View {
            assoc FieldType: pure comptime fn(Symbol) -> Type = ft;
            fn struct_pattern_view_get(*self, comptime field: Symbol) -> FieldType(field) {
                inline if field == .ok { 1 } else { while true {} }
            }
        }
        fn bad(v: View) -> i64 {
            v match {
                { bogus: x } => x,
                _ => 0,
            }
        }
    "#;
    let err = support::compile(noreturn_struct).unwrap_err();
    assert!(
        err.contains("unknown StructPatternView field `bogus` has NoReturn type"),
        "expected NoReturn field error, got: {err}"
    );

    let noreturn_map = r#"
        pure comptime fn ft(prop: Symbol) -> Type {
            inline if prop == .ok { i64 } else { NoReturn }
        }
        struct MView {}
        comptime impl MapPatternView for MView {
            assoc FieldTypeComptime: pure comptime fn(Symbol) -> Type = ft;
            comptime fn map_pattern_view_get_comptime(
                self,
                prop: Symbol,
            ) -> ?FieldTypeComptime(prop) {
                inline if prop == .ok { 1 } else { null }
            }
        }
        comptime fn bad(v: MView) -> i64 {
            v match {
                { nope: x, ... } => x,
                _ => 0,
            }
        }
    "#;
    let err = support::compile(noreturn_map).unwrap_err();
    assert!(
        err.contains("unknown MapPatternView field `nope` has NoReturn type"),
        "expected NoReturn comptime MapPatternView error, got: {err}"
    );

    // 4. Non-exhaustive without `_` fallback even when StableMapPatternView / StableStructPatternView is implemented.
    let non_exhaustive_map = r#"
        use std.collections.HashMap;
        fn bad(m: HashMap<Symbol, i64>) -> i64 {
            m match {
                { code: x, ... } => x,
            }
        }
    "#;
    let err = support::compile(non_exhaustive_map).unwrap_err();
    assert!(
        err.contains("non-exhaustive match") || err.contains("missing"),
        "expected non-exhaustive error for open StableMapPatternView without _, got: {err}"
    );

    // 5. Phase mismatch: meta.Map<Symbol, Integer> only implements comptime MapPatternView.
    let phase_mismatch = r#"
        use meta.Map;
        fn bad(m: Map<Symbol, Integer>) -> Integer {
            m match {
                { code: x, ... } => x,
                _ => 0,
            }
        }
    "#;
    let err = support::compile(phase_mismatch).unwrap_err();
    assert!(
        err.contains("runtime MapPatternView phase"),
        "expected runtime phase error for comptime-only Map, got: {err}"
    );
}

#[test]
fn cli_protocol_selectors_override_map_and_struct_pattern_views() {
    let source = r#"
        mod custom {
            pub pure comptime fn ft(_prop: Symbol) -> Type { i64 }

            pub trait CustomMap {
                assoc FieldType: pure comptime fn(Symbol) -> Type;
                fn map_pattern_view_get(*self, comptime property: Symbol) -> ?FieldType(property);
            }

            pub trait CustomStableMap :- CustomMap {}

            pub trait CustomStruct {
                assoc FieldType: pure comptime fn(Symbol) -> Type;
                fn struct_pattern_view_get(*self, comptime field: Symbol) -> FieldType(field);
            }

            pub trait CustomStableStruct :- CustomStruct {}
        }

        struct Bag { v: i64 }

        impl custom.CustomMap for Bag {
            assoc FieldType: pure comptime fn(Symbol) -> Type = custom.ft;
            fn map_pattern_view_get(*self, comptime property: Symbol) -> ?FieldType(property) {
                self.v.*
            }
        }

        impl custom.CustomStableMap for Bag {}

        struct Boxed { v: i64 }

        impl custom.CustomStruct for Boxed {
            assoc FieldType: pure comptime fn(Symbol) -> Type = custom.ft;
            fn struct_pattern_view_get(*self, comptime field: Symbol) -> FieldType(field) {
                self.v.*
            }
        }

        impl custom.CustomStableStruct for Boxed {}

        pub fn run() -> u8 {
            let b = Bag { v: 21 };
            let x = b match {
                { anything: n, ... } => n,
                _ => 0,
            };
            let bx = Boxed { v: 21 };
            let y = bx match {
                { anything: n } => n,
                _ => 0,
            };
            if x == 21 and y == 21 { 0 } else { 1 }
        }
    "#;
    let case = NativeCase::new(source);
    let output = case.build(&[
        "--map-pattern-protocol",
        "input.custom.CustomMap",
        "--stable-map-pattern-protocol",
        "input.custom.CustomStableMap",
        "--struct-pattern-protocol",
        "input.custom.CustomStruct",
        "--stable-struct-pattern-protocol",
        "input.custom.CustomStableStruct",
    ]);
    assert_success(&output);
    let run_out = case.run();
    assert_eq!(run_out.status.code(), Some(0));
}
