use fmt::{FormatConfig, format_source};

fn fmt_ok(src: &str) -> String {
    let cfg = FormatConfig::default();
    fmt_with_cfg(src, &cfg)
}

fn fmt_with_cfg(src: &str, cfg: &FormatConfig) -> String {
    let res = format_source(src, "test.fl", cfg, false)
        .unwrap_or_else(|e| panic!("format_source failed: {e}\n--- input ---\n{src}"));
    // Every formatted output MUST be strictly idempotent!
    let second = format_source(&res.formatted, "test.fl", cfg, false).unwrap_or_else(|e| {
        panic!(
            "second format_source failed: {e}\n--- first ---\n{}",
            res.formatted
        )
    });
    assert_eq!(
        res.formatted, second.formatted,
        "formatter is not idempotent!\n--- pass 1 ---\n{}\n--- pass 2 ---\n{}",
        res.formatted, second.formatted
    );
    res.formatted
}

#[test]
fn canonical_modifiers_and_attributes() {
    let src = "\
^inline_hint
^align(16)
inline pure pub unsafe fn compute(x:i32)->i32{x + 1}

pub pure comptime trait MetaTrait {
    assoc Out: Type;
}

pure comptime fn Pair<A: Type, B: Type> -> Type = Tuple<A, B>;
";
    let got = fmt_ok(src);
    let expected = "\
^inline_hint
^align(16)
pub pure inline unsafe fn compute(x: i32) -> i32 {
    x + 1
}

pub pure comptime trait MetaTrait {
    assoc Out: Type;
}

pure comptime fn Pair<A: Type, B: Type> -> Type = Tuple<A, B>;
";
    assert_eq!(got, expected);
}

#[test]
fn use_sorting_and_compact_multi_path_braces() {
    let src = "\
use std.mem;
use std.collections.{ HashMap, Vec, BTreeMap };
use builtin.str;

pub use zeta.beta;
pub use alpha.omega;

@use pkg.z;
@use pkg.a;
";
    let got = fmt_ok(src);
    let expected = "\
use builtin.str;
use std.collections.{BTreeMap, HashMap, Vec}
use std.mem;

pub use alpha.omega;
pub use zeta.beta;

@use pkg.a;
@use pkg.z;
";
    assert_eq!(got, expected);
}

#[test]
fn semicolon_rules_value_blocks_and_greedy_continuations() {
    let src = "\
fn void_block() {
    do {
        side_effect();
    };
}

fn value_block() -> i32 {
    do {
        42
    }
}

fn greedy_continuation(y: Wrapper) {
    let a = Foo { x: 1 };
    let b = Bar { y: 2 };
    (y).run();
}
";
    let got = fmt_ok(src);
    let expected = "\
fn void_block() {
    do {
        side_effect();
    };
}

fn value_block() -> i32 {
    do { 42 }
}

fn greedy_continuation(y: Wrapper) {
    let a = Foo { x: 1 }
    let b = Bar { y: 2 };
    (y).run();
}
";
    assert_eq!(got, expected);
}

#[test]
fn mixed_adt_and_enum_variants_preserve_order_and_blank_lines() {
    let src = "\
struct Buffer where T {
    .repr .c,
    ptr: *T,
    len: usize = 0,
    pub fn is_empty(self) -> bool {
        self.len == 0
    }
}

enum Message {
    Quit,
    Move { x: i32, y: i32 },
    Write(str),
    Code = 1,
    Sub.{
        Ping,
        Pong(u32),
    },
    pub fn tag(self) -> i32 {
        0
    }
}
";
    let got = fmt_ok(src);
    let expected = "\
struct Buffer where T {
    .repr .c,

    ptr: *T,
    len: usize = 0,

    pub fn is_empty(self) -> bool {
        self.len == 0
    }
}

enum Message {
    Quit,
    Move { x: i32, y: i32 },
    Write(str),
    Code = 1,
    Sub.{
        Ping,
        Pong(u32),
    },

    pub fn tag(self) -> i32 {
        0
    }
}
";
    assert_eq!(got, expected);
}

#[test]
fn where_clauses_contracts_predicates_and_expr_bodied_fn() {
    let src = "\
predicate Positive(x: i32) = x > 0;

fn clamp(x: i32, lo: i32, hi: i32) -> res: i32 where T :- Ord, requires lo <= hi and Positive(hi), ensures res >= lo and res <= hi, outcomes [.ok] {
    if x < lo { lo } else if x > hi { hi } else { x }
}
";
    let got = fmt_ok(src);
    let expected = "\
predicate Positive(x: i32) = x > 0;

fn clamp(x: i32, lo: i32, hi: i32) -> res: i32
where
    T :- Ord,
    requires lo <= hi and Positive(hi),
    ensures res >= lo and res <= hi,
    outcomes [.ok],
{
    if x < lo {
        lo
    } else if x > hi {
        hi
    } else {
        x
    }
}
";
    assert_eq!(got, expected);
}

#[test]
fn operators_ranges_parens_and_front_wrapping() {
    let mut cfg = FormatConfig::default();
    cfg.max_width = 36;

    let src = "\
fn ops(a: i32, b: i32, c: i32) -> bool {
    let r1 = 0..10;
    let r2 = 0.. -1;
    let r3 = -5..= -1;
    let p = (a + b) * c + - -a;
    let t: Foo<A -> B> -> (C -> D) = f;
    alpha_value + beta_value + gamma_value == delta_value ==> epsilon_flag
}
";
    let got = fmt_with_cfg(src, &cfg);
    assert!(got.contains("let r1 = 0..10;"), "got:\n{got}");
    assert!(got.contains("let r2 = 0.. -1;"), "got:\n{got}");
    assert!(got.contains("let r3 = -5..= -1;"), "got:\n{got}");
    assert!(got.contains("let p = (a + b) * c + - -a;"), "got:\n{got}");
    assert!(got.contains("Foo<A -> B> -> (C -> D)"), "got:\n{got}");
    // Front-style wrapping puts `+` / `==>` at start of continuation lines.
    assert!(
        got.contains("\n        + "),
        "expected Front `+` wrap in:\n{got}"
    );
}

#[test]
fn postfix_chains_views_and_trailing_blocks_no_pyramid() {
    let src = "\
fn pipeline(input: Source) -> i32 {
    let v = input'header'payload;
    let p = v.ref.*.as(i32);
    input.step_one_with_long_name().step_two_with_long_name().step_three_with_long_name() match {
        0 => 1,
        n => {
            n + 1
        },
    }
}
";
    let got = fmt_ok(src);
    let expected = "\
fn pipeline(input: Source) -> i32 {
    let v = input'header'payload;
    let p = v.ref.*.as(i32);
    input
        .step_one_with_long_name()
        .step_two_with_long_name()
        .step_three_with_long_name() match {
        0 => 1,
        n => { n + 1 },
    }
}
";
    assert_eq!(got, expected);
}

#[test]
fn loops_labels_type_qualifiers_cmtt_and_patterns() {
    let src = "\
fn advanced(xs: List<i32>, raw: BitVec) -> < x: i32 |- i32 > {
    for:outer item in xs {
        while:inner item > 0 {
            break outer while item == 99;
            continue inner;
        }
    }
    let single = (1,);
    let ( < 1 + 2 > , y) = (3, 4);
    let bits_ok = raw match {
        0b1010(rest: u4)<y> => true,
        0xdead beef => true,
        1 or 2 or 3 => false,
        _ => false,
    }
    let pat_ty: < Option<i32> -| v: i32 > = '<v?>;
    let empty_pat_ty: < i32 -|> = '<0>;
    let closed_ty: < |- i32 > = '(42);
    let q: ?*!Io #Console %{ cap: Cap = c } dyn Reader = null;
    '[x = y](x + ~[a = y]<closed_ty>)
}
";
    let got = fmt_ok(src);
    assert!(got.contains("for:outer item in xs {"), "got:\n{got}");
    assert!(got.contains("while:inner item > 0 {"), "got:\n{got}");
    assert!(got.contains("break outer while item == 99;"), "got:\n{got}");
    assert!(got.contains("let single = (1,);"), "got:\n{got}");
    assert!(
        got.contains("let ( < 1 + 2 > , y) = (3, 4);"),
        "got:\n{got}"
    );
    assert!(got.contains("0b1010(rest: u4)<y> => true,"), "got:\n{got}");
    assert!(got.contains("0xdead beef => true,"), "got:\n{got}");
    assert!(got.contains("< Option<i32> -| v: i32 >"), "got:\n{got}");
    assert!(got.contains("< i32 -|>"), "got:\n{got}");
    assert!(got.contains("< |- i32 >"), "got:\n{got}");
    assert!(
        got.contains("'[x = y](x + ~[a = y]<closed_ty>)"),
        "got:\n{got}"
    );
}

#[test]
fn effect_enum_and_effect_handling_arms() {
    let src = "\
effect enum Console {
    read_line() -> str,
    write_line(msg: str) -> (),
}

fn handle_it() {
    run_app()# {
        Console.read_line() => resume \"hello\",
        Console.write_line(s) => {
            resume ()
        },
    }
    fallible()! {
        catch err => log(err),
        ok_val => use_val(ok_val),
    }
}
";
    let got = fmt_ok(src);
    let expected = "\
effect enum Console {
    read_line() -> str,
    write_line(msg: str) -> (),
}

fn handle_it() {
    run_app()# {
        Console.read_line() => resume \"hello\",
        Console.write_line(s) => {
            resume ();
        },
    }
    fallible()! {
        catch err => log(err),
        ok_val => use_val(ok_val),
    }
}
";
    assert_eq!(got, expected);
}

#[test]
fn macros_comments_alignment_and_directives() {
    let src = "\
@define_macro1
define make_adder(name) {
    fn   $name(x:i32)->i32 { x + 1 }
}

fn demo() {
    --unspaced comment
    ----
    let a = 1;      --aligned one
    let bb = 22;    --aligned two
    let raw = @builtin.str.raw {
        unformatted   {  raw  }
    };
    let v = @vec( 1 ,  2 , 3 );
    --%.skip
    let matrix = [
        1,   0,   0,
        0,  10,   0,
    ];
    --% .preserve
    let lut = [
        1,   20,
        300,  4,
    ];
    --% .off
    let   untouched  =   1   +   2 ;
    @@@ completely unparseable dsl inside off @@@
    --% .on
    --% .max_width 35
    {
        let wrapped = alpha_one + beta_two + gamma_three;
    }
}
";
    let got = fmt_ok(src);
    assert!(
        got.contains(
            "@define_macro1\ndefine make_adder(name) {\n    fn   $name(x:i32)->i32 { x + 1 }\n}"
        ),
        "got:\n{got}"
    );
    assert!(
        got.contains("    -- unspaced comment\n    ----\n"),
        "got:\n{got}"
    );
    assert!(
        got.contains("    let a = 1;      -- aligned one\n    let bb = 22;    -- aligned two\n"),
        "got:\n{got}"
    );
    assert!(got.contains("let v = @vec(1, 2, 3);"), "got:\n{got}");
    assert!(
        got.contains(
            "    --% .skip\n    let matrix = [\n        1,   0,   0,\n        0,  10,   0,\n    ];"
        ),
        "got:\n{got}"
    );
    assert!(
        got.contains(
            "    --% .preserve\n    let lut = [\n        1,   20,\n        300,  4,\n    ];"
        ),
        "got:\n{got}"
    );
    assert!(got.contains("    --% .off\n    let   untouched  =   1   +   2 ;\n    @@@ completely unparseable dsl inside off @@@\n    --% .on"), "got:\n{got}");
    assert!(
        got.contains("+ beta_two\n                + gamma_three;"),
        "got:\n{got}"
    );
}

#[test]
fn control_flow_is_matches_when_and_let_else() {
    let src = "\
fn control(opt: ?i32, n: i32) -> i32 {
    let v = opt else {
        return 0;
    };
    if opt is y? do {
        let z = y + 1;
        return z;
    }
    if n is do {
        0 => return 10,
        k => return k,
    }
    while opt is w? do {
        break;
    }
    when {
        v < 0 => -1,
        v == 0 => 0,
        else => 1,
    }
}
";
    let got = fmt_ok(src);
    let expected = "\
fn control(opt: ?i32, n: i32) -> i32 {
    let v = opt else { return 0; }
    if opt is y? do {
        let z = y + 1;
        return z;
    }
    if n is do {
        0 => return 10,
        k => return k,
    }
    while opt is w? do {
        break;
    }
    when {
        v < 0 => -1,
        v == 0 => 0,
        else => 1,
    }
}
";
    assert_eq!(got, expected);
}

#[test]
fn lambdas_quantifiers_atomic_and_defer() {
    let src = "\
fn closures_and_specs(lock_a: Mutex, lock_b: Mutex) -> bool {
    defer cleanup();
    let zero = || 0;
    let add = |a: i32, b: i32| -> i32 { a + b };
    let _ = atomic(lock_a, lock_b) {
        step_one();
    };
    let ok = forall x: i32 => exists y: i32 => y > x;
    ok
}
";
    let got = fmt_ok(src);
    let expected = "\
fn closures_and_specs(lock_a: Mutex, lock_b: Mutex) -> bool {
    defer cleanup();
    let zero = || 0;
    let add = |a: i32, b: i32| -> i32 { a + b }
    let _ = atomic(lock_a, lock_b) {
        step_one();
    }
    let ok = forall x: i32 => exists y: i32 => y > x;
    ok
}
";
    assert_eq!(got, expected);
}

#[test]
fn traits_impls_derivers_aliases_unions_and_tests() {
    let src = "\
pub typealias Byte = u8;
pub newtype UserId = u64;

union RawSlot {
    int_val: i64,
    float_val: f64,
}

impl Clone for UserId {
    pub fn clone(self) -> Self {
        self
    }
}

derive Eq for UserId;

derive fn Debug for T where T {
    fn fmt_it(x: T) {}
}

test basic_addition {
    asserts 1 + 1 == 2;
}
";
    let got = fmt_ok(src);
    let expected = "\
pub typealias Byte = u8;
pub newtype UserId = u64;

union RawSlot {
    int_val: i64,
    float_val: f64,
}

impl Clone for UserId {
    pub fn clone(self) -> Self {
        self
    }
}

derive Eq for UserId;

derive fn Debug for T where T {
    fn fmt_it(x: T) {}
}

test basic_addition {
    asserts 1 + 1 == 2;
}
";
    assert_eq!(got, expected);
}

#[test]
fn block_comments_shift_indent_and_unknown_directive_warns() {
    let src = "\
fn documented() {
  {-
    Block comment body
      indented detail
  -}
  --% .unknown_rule 123
  let x = 1;
}
";
    let cfg = FormatConfig::default();
    let res = format_source(src, "doc.fl", &cfg, false).unwrap();
    assert_eq!(res.warnings.len(), 1);
    assert!(
        res.formatted
            .contains("    {-\n      Block comment body\n        indented detail\n    -}")
    );
    assert!(
        res.formatted
            .contains("    --% .unknown_rule 123\n    let x = 1;")
    );
}

#[test]
fn symmetric_if_else_let_else_multiline_cond_or_pat_and_multi_bound_where() {
    let src = "\
enum Choice where T, enabled: bool {
    On(T),
    Off,
}

fn single_bound(r: R) where R :- Reading {
    read_one(r);
}

fn multi_bounds(r: R, w: W) where R :- Reading, W :- Writing {
    pipe(r, w);
}

fn control_heuristics(flag: bool, opt: ?i32) -> i32 {
    let short = if flag { 1 } else { 2 };
    let medium = if flag_with_a_longer_condition_name_exceeding_fifty { 1 } else { 2 };
    let v = opt else { return 0; };
    let very_long_extracted_binding = compute_some_optional_candidate(opt) else { return 0; };
    if flag_alpha_condition_one_two_three_four and flag_beta_condition_five_six_seven_eight and flag_gamma_condition_nine_ten {
        return 9;
    }
    v match {
        100_000_001 or 100_000_002 or 100_000_003 or 100_000_004 or 100_000_005 or 100_000_006 or 100_000_007 or 100_000_008 => 1,
        _ => short + medium + very_long_extracted_binding,
    }
}
";
    let got = fmt_ok(src);
    let expected = "\
enum Choice where T, enabled: bool {
    On(T),
    Off,
}

fn single_bound(r: R) where R :- Reading {
    read_one(r);
}

fn multi_bounds(r: R, w: W)
where
    R :- Reading,
    W :- Writing,
{
    pipe(r, w);
}

fn control_heuristics(flag: bool, opt: ?i32) -> i32 {
    let short = if flag { 1 } else { 2 }
    let medium = if flag_with_a_longer_condition_name_exceeding_fifty {
        1
    } else {
        2
    }
    let v = opt else { return 0; }
    let very_long_extracted_binding = compute_some_optional_candidate(opt) else {
        return 0;
    }
    if
        flag_alpha_condition_one_two_three_four
            and flag_beta_condition_five_six_seven_eight
            and flag_gamma_condition_nine_ten
    {
        return 9;
    }
    v match {
        100_000_001
        or 100_000_002
        or 100_000_003
        or 100_000_004
        or 100_000_005
        or 100_000_006
        or 100_000_007
        or 100_000_008 => 1,
        _ => short + medium + very_long_extracted_binding,
    }
}
";
    assert_eq!(got, expected);
}

#[test]
fn formats_sysroot_library_files_idempotently_and_verifies_ast() {
    use std::fs;
    use std::path::{Path, PathBuf};

    fn collect_fl_files(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|x| x.path())).collect();
        paths.sort();
        for p in paths {
            if p.is_dir() {
                collect_fl_files(&p, out);
            } else if p.extension().and_then(|s| s.to_str()) == Some("fl") {
                out.push(p);
            }
        }
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let lib_root = manifest_dir.join("../../library");
    let mut files = Vec::new();
    collect_fl_files(&lib_root, &mut files);
    assert!(!files.is_empty(), "expected .fl files under library/");

    let cfg = FormatConfig::default();
    let mut formatted_count = 0usize;
    let mut failures = Vec::new();

    for path in &files {
        let src = fs::read_to_string(path).expect("read sysroot .fl file");
        let name = path.display().to_string();
        match format_source(&src, &name, &cfg, false) {
            Ok(first) => match format_source(&first.formatted, &name, &cfg, false) {
                Ok(second) => {
                    if first.formatted != second.formatted {
                        failures.push(format!(
                            "{name}: non-idempotent on pass 2:\n--- pass 1 ---\n{}\n--- pass 2 ---\n{}",
                            first.formatted, second.formatted
                        ));
                    } else {
                        formatted_count += 1;
                    }
                }
                Err(e) => failures.push(format!("{name}: pass 2 error: {e}")),
            },
            Err(e) => failures.push(format!("{name}: pass 1 error: {e}")),
        }
    }

    assert!(
        failures.is_empty(),
        "failed on {}/{} library .fl files:\n{}",
        failures.len(),
        formatted_count + failures.len(),
        failures.join("\n---\n")
    );
    assert!(
        formatted_count >= 10,
        "expected at least 10 parseable sysroot files, got {formatted_count}"
    );
}

#[test]
fn formats_lambda_param_angles_guarded_effect_arms_and_final_case_arm() {
    let src = "\
fn apply(lambda f: fn(bool) -> bool, value: Tuple<Tuple<bool, Tuple<bool, bool>>, Tuple<bool, Tuple<bool, bool>>, Tuple<bool, Tuple<bool, bool>>>) -> bool {
    let x = subject()# {
        select(v) if value == (true, false) => do { v },
        other(v) => v,
    };
    value match {
        (true, _) => true,
        _ => false
    }
}

fn make() -> Tuple<Tuple<Tuple<bool, bool>, bool>, Tuple<bool, Tuple<Tuple<bool, bool>, Tuple<bool, bool>>>> {
    Pair<do { var count = 0; count = count + 1; count }, do { var flag = true; flag = false; flag }>
}
";
    let got = fmt_ok(src);
    assert!(got.contains("lambda f: fn(bool) -> bool"), "{got}");
    assert!(!got.contains(",\n>"), "{got}");
    assert!(!got.contains("\n>"), "{got}");
    assert!(got.contains("select(v) if"), "{got}");
    assert!(got.contains("_ => false,"), "{got}");
}
