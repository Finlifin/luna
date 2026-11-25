//! Integration tests for Milestone 1 of Wishlist 01:
//! - `default` identifier freedom & `Default.default()` / `ComptimeDefault.default()`
//! - removal of orphan bitwise operator traits from `builtin.ops` / `builtin.prelude`
//! - `|>` reserved diagnostic & precedence/associativity
//! - `==>` lexing, right-associativity, short-circuit, occurrence typing, PTC, and `Implies`/`ComptimeImplies`
//! - sysroot `LiteralInto`, 12 machine integer suffixes, atomic `-128i8`, `-32px` via `Neg`, and string suffixes
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::Value;

#[test]
fn default_is_ordinary_identifier_and_default_traits_use_default_method() {
    let source = r#"
mod default {
    pub fn default(default: i32) -> i32 {
        if default < 1000 { default + 1 } else { default }
    }
}

struct Config {
    default: i32,
}

impl Config {
    pub fn get(self) -> i32 {
        self.default
    }
}

impl Default for Config {
    fn default() -> Self {
        Config { default: 7 }
    }
}

comptime impl ComptimeDefault for Config {
    pure comptime fn default() -> Self {
        Config { default: 11 }
    }
}

comptime fn ct_default() -> i32 {
    Config.as(ComptimeDefault).default().default
}

fn run() -> bool {
    let x = Config.as(Default).default().get();
    let y = default.default(x);
    let z = ct_default();
    x == 7 and y == 8 and z == 11
}
"#;
    assert_eq!(
        support::invoke(source, "ct_default").unwrap(),
        Value::SignedInteger(11)
    );
    assert_eq!(support::run(source), Value::Bool(true));

    // The old `default_value` method name must no longer exist on `Default`.
    let old_name = r#"
struct S { v: i32 }
impl Default for S {
    fn default_value() -> Self { S { v: 1 } }
}
fn run() -> i32 { S.as(Default).default_value().v }
"#;
    assert!(support::compile(old_name).is_err());
}

#[test]
fn orphan_bitwise_traits_are_absent_from_builtin_ops_and_prelude() {
    for trait_name in ["BitAnd", "BitOr", "BitXor", "Shl", "Shr"] {
        let prelude_use = format!(
            "struct W {{ x: u32 }}\nimpl {trait_name} for W {{ assoc Output: Type = W; fn f(self, rhs: W) -> W {{ self }} }}\nfn run() {{}}"
        );
        assert!(
            support::compile(&prelude_use).is_err(),
            "{trait_name} should not exist in builtin.prelude"
        );
        let ops_use = format!("use builtin.ops.{trait_name};\nfn run() {{}}");
        assert!(
            support::compile(&ops_use).is_err(),
            "{trait_name} should not exist in builtin.ops"
        );
    }
}

#[test]
fn pipeline_operator_precedence_and_reserved_diagnostic() {
    // Parse + fmt must accept `|>` with precedence 35 (tighter than `==` (25), looser than `+` (60))
    // and left-associativity.
    if let Err(err) =
        fmt::bridge::parse_and_bridge("fn f() { a + 1 |> g |> h; }\n", "test.fl", false)
    {
        panic!("{}", err.rendered_diagnostics);
    }
    let formatted = fmt::format_source(
        "fn f() {\n    (a + 1 |> g) |> h;\n}\n",
        "test.fl",
        &fmt::FormatConfig::default(),
        false,
    )
    .unwrap()
    .formatted;
    assert!(
        formatted.contains("a + 1 |> g |> h"),
        "expected redundant parens stripped under prec 35 + left-assoc: {formatted}"
    );
    let err = support::compile("fn f(x: i32) -> i32 { x + 1 |> x }").unwrap_err();
    assert!(
        err.contains("pipeline operator `|>` (reserved)"),
        "unexpected error: {err}"
    );
}

#[test]
fn bool_implies_right_assoc_short_circuit_occurrence_typing_and_ptc() {
    let source = r#"
comptime fn ct_assoc() -> bool {
    false ==> false ==> false
}

fn check_short_circuit(flag: bool) -> bool {
    var touched = false;
    let res = flag ==> do {
        touched = true;
        true
    };
    res and (not touched)
}

fn check_occ(x: ?i32) -> bool {
    let a = x != null ==> (x match { v? => v > 0, null => false });
    let b = (x == null) ==> (x match { _? => false, null => true });
    a and b
}

fn run() -> bool {
    let assoc_rt = false ==> false ==> false;
    let assoc_ct = ct_assoc();
    let sc = check_short_circuit(false);
    let occ1 = check_occ(5);
    let occ2 = check_occ(null);
    assoc_rt and assoc_ct and sc and occ1 and occ2
}
"#;
    assert_eq!(
        support::invoke(source, "ct_assoc").unwrap(),
        Value::Bool(true)
    );
    assert_eq!(support::run(source), Value::Bool(true));

    // BitVec must not accept `==>`.
    let bv_err =
        support::compile("fn f(a: BitVec<4>, b: BitVec<4>) -> BitVec<4> { a ==> b }").unwrap_err();
    assert!(!bv_err.is_empty());

    // `==>` in PTC `requires` must validate implication statically.
    let ptc_ok = r#"
fn guard_ok(a: bool, b: bool) -> bool where requires a ==> b {
    if a { b } else { true }
}
fn caller_ok(x: bool) -> bool {
    guard_ok(false, x)
}
fn run() -> bool { caller_ok(false) }
"#;
    assert_eq!(support::run(ptc_ok), Value::Bool(true));
}

#[test]
fn implies_operator_supports_runtime_and_comptime_overloads() {
    let source = r#"
struct Edge {
    weight: i32,
}

impl Implies for Edge {
    assoc Output: Type = bool;

    fn implies(self, rhs: Edge) -> bool {
        self.weight <= rhs.weight
    }
}

comptime impl ComptimeImplies for Edge {
    assoc Output: Type = bool;

    pure comptime fn implies(self, rhs: Edge) -> bool {
        self.weight == rhs.weight
    }
}

comptime fn ct_eval() -> bool {
    Edge { weight: 3 } ==> Edge { weight: 3 }
}

fn run() -> bool {
    let rt = Edge { weight: 4 } ==> Edge { weight: 5 };
    let ct = ct_eval();
    rt and ct
}
"#;
    assert_eq!(
        support::invoke(source, "ct_eval").unwrap(),
        Value::Bool(true)
    );
    assert_eq!(support::run(source), Value::Bool(true));
}

#[test]
fn literal_into_sysroot_machine_integers_and_atomic_negative_boundaries() {
    let source = r#"
const CT_U8: u8 = 255u8;
const CT_I8_MIN: i8 = -128i8;

comptime fn ct_bounds() -> i128 {
    let a = CT_U8;
    let b = CT_I8_MIN;
    let c = -32768i16;
    let d = -2147483648i32;
    let e = -9223372036854775808i64;
    let f = -170141183460469231731687303715884105728i128;
    if a == 255u8 and b == -128i8 and c == -32768i16 and d == -2147483648i32 and e == -9223372036854775808i64 {
        f
    } else {
        0i128
    }
}

fn rt_bounds() -> bool {
    let a = 42u8;
    let b = 42u16;
    let c = 42u32;
    let d = 42u64;
    let e = 42u128;
    let f = 42usize;
    let g = -128i8;
    let h = -32768i16;
    let i = -2147483648i32;
    let j = -9223372036854775808i64;
    let k = -1isize;
    a == 42u8
        and b == 42u16
        and c == 42u32
        and d == 42u64
        and e == 42u128
        and f == 42usize
        and g == -128i8
        and h == -32768i16
        and i == -2147483648i32
        and j == -9223372036854775808i64
        and k == -1isize
}

fn run() -> bool {
    let ct = ct_bounds();
    rt_bounds() and ct == -170141183460469231731687303715884105728i128
}
"#;
    assert_eq!(
        support::invoke(source, "ct_bounds").unwrap(),
        Value::SignedInteger(-170141183460469231731687303715884105728)
    );
    assert_eq!(support::run(source), Value::Bool(true));

    for bad_literal in [
        "256u8",
        "128i8",
        "-129i8",
        "-10u32",
        "18446744073709551616u64",
    ] {
        let rt_bad = format!("fn run() {{ let _x = {bad_literal}; }}");
        let rt_failed = match support::compile(&rt_bad) {
            Err(_) => true,
            Ok(package) => luna::instantiation::instantiate(&package, "run", 1_000_000).is_err(),
        };
        assert!(rt_failed, "{bad_literal} should fail at runtime");
        let ct_bad = format!("comptime fn run() {{ let _x = {bad_literal}; }}");
        assert!(
            support::invoke(&ct_bad, "run").is_err(),
            "{bad_literal} should fail at comptime"
        );
    }
    // Atomic negative underflow must already reject `-129i8` at elaboration time.
    assert!(support::compile("fn run() { let _x = -129i8; }").is_err());
}

#[test]
fn literal_into_custom_suffix_neg_routing_and_string_suffixes() {
    let source = r#"
struct Pixel {
    value: i32,
}

impl LiteralInto<.px, Pixel> for Integer {
    fn literal_into(comptime value: Integer) -> Pixel {
        Pixel { value: value.to_i32() }
    }
}

comptime impl LiteralInto<.px, Pixel> for Integer {
    comptime fn comptime_literal_into(comptime value: Integer) -> Pixel {
        Pixel { value: value.to_i32() }
    }
}

impl Neg for Pixel {
    assoc Output: Type = Pixel;

    fn neg(self) -> Pixel {
        if self.value > -1000000 and self.value < 1000000 {
            Pixel { value: -self.value }
        } else {
            Pixel { value: 0 }
        }
    }
}

struct Meters { n: i32 }
struct Seconds { n: i32 }

impl LiteralInto<.m, Meters> for Integer {
    fn literal_into(comptime value: Integer) -> Meters {
        Meters { n: value.to_i32() }
    }
}

impl LiteralInto<.m, Seconds> for Integer {
    fn literal_into(comptime value: Integer) -> Seconds {
        let secs = value.to_i32();
        Seconds { n: if secs >= 0 and secs < 1000 { secs * 60 } else { 0 } }
    }
}

struct CharCode { code: u32 }
struct StrLen { len: u64 }

impl LiteralInto<.tag, CharCode> for char {
    fn literal_into(comptime value: char) -> CharCode {
        CharCode { code: value'bitvec.as(u32) }
    }
}

impl LiteralInto<.tag, StrLen> for str {
    fn literal_into(comptime value: str) -> StrLen {
        StrLen { len: value.byte_length() }
    }
}

comptime fn ct_pixel() -> i32 {
    (17px).value
}

fn run() -> bool {
    let p1 = 32px;
    let p2 = -32px;
    let p3 = ct_pixel();
    let d: Meters = 5m;
    let t: Seconds = 5m;
    let c = 'A'tag;
    let s1 = "hello"tag;
    let s2 = "ab" "cd"tag;
    p1.value == 32
        and p2.value == -32
        and p3 == 17
        and d.n == 5
        and t.n == 300
        and c.code == 65u32
        and s1.len == 5u64
        and s2.len == 4u64
}
"#;
    assert_eq!(
        support::invoke(source, "ct_pixel").unwrap(),
        Value::SignedInteger(17)
    );
    assert_eq!(support::run(source), Value::Bool(true));

    // Runtime-only `LiteralInto` must still be rejected in comptime phase.
    let rt_only_in_ct = r#"
struct Meters { n: i32 }
impl LiteralInto<.m, Meters> for Integer {
    fn literal_into(comptime value: Integer) -> Meters { Meters { n: value.to_i32() } }
}
comptime fn bad() -> Meters { 5m }
"#;
    assert!(support::compile(rt_only_in_ct).is_err());

    // `-32px` without `impl Neg` for `Pixel` must fail.
    let no_neg = r#"
struct Pixel { value: i32 }
impl LiteralInto<.px, Pixel> for Integer {
    fn literal_into(comptime value: Integer) -> Pixel { Pixel { value: value.to_i32() } }
}
fn run() -> Pixel { -32px }
"#;
    assert!(support::compile(no_neg).is_err());

    // Ambiguous suffix without contextual type must fail.
    let ambig = r#"
struct Meters { n: i32 }
struct Seconds { n: i32 }
impl LiteralInto<.m, Meters> for Integer { fn literal_into(comptime v: Integer) -> Meters { Meters { n: v.to_i32() } } }
impl LiteralInto<.m, Seconds> for Integer { fn literal_into(comptime v: Integer) -> Seconds { Seconds { n: v.to_i32() } } }
fn run() { let _x = 5m; }
"#;
    assert!(support::compile(ambig).is_err());

    // Interpolated strings with suffixes and double suffixes must be rejected.
    for bad in [
        r#"struct StrLen { len: u64 } impl LiteralInto<.tag, StrLen> for str { fn literal_into(comptime v: str) -> StrLen { StrLen { len: v.byte_length() } } } fn run(x: i32) { let _s = "hi ${x}"tag; }"#,
        r#"struct StrLen { len: u64 } impl LiteralInto<.tag, StrLen> for str { fn literal_into(comptime v: str) -> StrLen { StrLen { len: v.byte_length() } } } fn run(x: i32) { let _s = "hi $x"tag; }"#,
        r#"fn run() { let _x = 12pxrem; }"#,
    ] {
        assert!(support::compile(bad).is_err(), "{bad}");
    }
}
