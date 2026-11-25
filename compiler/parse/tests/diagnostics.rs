use diagnostic::{Diagnostic, DiagnosticContext, FlurryError, Level};
use parse::error::{
    PARSE_BRACE_CONFLICT, PARSE_FOREIGN_HABIT, PARSE_SEPARATED_OPERATOR, PARSE_SEPARATOR,
    PARSE_UNCLOSED_DELIMITER, PARSE_WARN_DEREF_ASSIGN, PARSE_WARN_PARENTHESES,
};
use parse::parser::Parser;
use rustc_span::source_map::FilePathMapping;
use rustc_span::{DUMMY_SP, FileName, SourceMap};
use std::fs;
use std::path::{Path, PathBuf};

fn parse_with_diagnostics(source: &str) -> (bool, Vec<Diagnostic>) {
    let map = SourceMap::new(FilePathMapping::empty());
    let file = map.new_source_file(FileName::Custom("diag_test.fl".into()), source.into());
    let diag_ctx = DiagnosticContext::new(&map);

    let (tokens, symbols, lex_errors) = lex::lex(source, file.start_pos);
    for lerr in &lex_errors {
        lerr.emit(&diag_ctx, file.start_pos);
    }

    let mut parser = Parser::new(&map, tokens, symbols, file.start_pos);
    let parsed_ok = parser.parse(&diag_ctx) && lex_errors.is_empty();
    let diags = diag_ctx.diagnostics().to_vec();

    for d in &diags {
        assert_style_and_spans(source, d);
    }

    (parsed_ok, diags)
}

fn assert_style_and_spans(source: &str, d: &Diagnostic) {
    assert!(
        d.code.is_some(),
        "missing diagnostic code for: {d:?}\nin:\n{source}"
    );
    assert!(
        !d.message.trim().is_empty(),
        "empty diagnostic headline for: {d:?}\nin:\n{source}"
    );
    let primary = d.primary_span.unwrap_or(DUMMY_SP);
    assert_ne!(
        primary, DUMMY_SP,
        "diagnostic has DUMMY_SP primary_span: {d:?}\nin:\n{source}"
    );
    assert!(
        !d.labels.is_empty(),
        "diagnostic has no span labels: {d:?}\nin:\n{source}"
    );
    for lbl in &d.labels {
        assert_ne!(
            lbl.span, DUMMY_SP,
            "diagnostic label has DUMMY_SP: {d:?}\nin:\n{source}"
        );
    }

    // Check style constraints: NO first-person pronouns ("I", "we", "my", "our")
    // and NO emojis anywhere in headline, labels, notes, or helps!
    let mut all_text = vec![d.message.as_str()];
    for lbl in &d.labels {
        all_text.push(lbl.message.as_str());
    }
    for note in &d.notes {
        all_text.push(note.as_str());
    }
    for help in &d.helps {
        all_text.push(help.as_str());
    }

    for text in all_text {
        // Strip backticked code snippets before checking English pronouns so code like `i` or `my_var` never triggers.
        let prose = strip_backticks(text);
        for word in prose.split(|c: char| !c.is_ascii_alphabetic() && c != '\'') {
            let lower = word.to_ascii_lowercase();
            assert!(
                !matches!(
                    lower.as_str(),
                    "i" | "i'm"
                        | "i've"
                        | "i'd"
                        | "i'll"
                        | "me"
                        | "my"
                        | "we"
                        | "we're"
                        | "we've"
                        | "our"
                        | "us"
                ),
                "first-person pronoun `{word}` found in diagnostic prose: {text:?}\nfor source:\n{source}"
            );
        }
        assert!(
            !text.chars().any(is_emoji_like),
            "emoji character found in diagnostic text: {text:?}\nfor source:\n{source}"
        );
    }
}

fn strip_backticks(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tick = false;
    for ch in s.chars() {
        if ch == '`' {
            in_tick = !in_tick;
            out.push(' ');
        } else if !in_tick {
            out.push(ch);
        }
    }
    out
}

fn is_emoji_like(c: char) -> bool {
    let u = c as u32;
    (0x1F300..=0x1FAFF).contains(&u) || (0x2600..=0x27BF).contains(&u)
}

fn assert_error_with_code(source: &str, expected_code: u32, expected_substr: &str) {
    let (ok, diags) = parse_with_diagnostics(source);
    assert!(
        !ok,
        "expected parse failure for:\n{source}\ngot diags: {diags:?}"
    );
    let matching = diags.iter().find(|d| {
        d.level == Level::Error
            && d.code == Some(expected_code)
            && (d.message.contains(expected_substr)
                || d.helps.iter().any(|h| h.contains(expected_substr))
                || d.notes.iter().any(|n| n.contains(expected_substr))
                || d.labels.iter().any(|l| l.message.contains(expected_substr)))
    });
    assert!(
        matching.is_some(),
        "expected error code {expected_code} containing {expected_substr:?} for:\n{source}\nactual diagnostics:\n{diags:#?}"
    );
}

fn assert_clean(source: &str) {
    let (ok, diags) = parse_with_diagnostics(source);
    assert!(
        ok && diags.is_empty(),
        "expected clean parse (0 errors, 0 warnings) for:\n{source}\ngot:\n{diags:#?}"
    );
}

#[test]
fn chained_attribute_boundaries_precede_infix_habit_diagnostics() {
    for source in [
        "^attrs.first ^attrs.second fn f() {}",
        "^attrs.first(1) ^attrs.second(2) fn f() {}",
        "^attrs.first + attrs.second ^attrs.third fn f() {}",
        "fn f(^attrs.first ^attrs.second value: bool) {}",
        "fn f(comptime ^attrs.first ^attrs.second value: bool) {}",
        "typealias Callback = fn(^attrs.first ^attrs.second comptime value: bool);",
        "fn f(^attrs.first.enabled ^attrs.second .value: bool = true) {}",
        "fn f() { ^attrs.first ^attrs.second let value = true; }",
        "fn f() { consume(^attrs.first ^attrs.second true, false); }",
        "fn f() { consume(^attrs.first(^attrs.inner ^attrs.last true) ^attrs.second false); }",
        "fn f() { consume(^{ mark: true } ^attrs.second true); }",
        "struct S { ^attrs.first ^attrs.second value: bool, other: bool }",
        "effect enum E { ^attrs.first ^attrs.second read() -> bool, }",
        "fn f() { true match { ^attrs.first ^attrs.second true => false, false => true } }",
        "^attrs.first\n^attrs.second fn f() {}",
    ] {
        assert_clean(source);
    }
    // Attribute boundaries apply only at the prefix's own expression level.
    // Nested operands and the attributed target keep the ordinary diagnostics.
    for source in [
        "^attrs.first(mask ^ 1) ^attrs.second fn f() {}",
        "^(mask ^ 1) ^attrs.second fn f() {}",
        "^{ mark: mask ^ 1 } ^attrs.second fn f() {}",
        "fn f() { ^attrs.first ^attrs.second let value = mask ^ 1; }",
        "fn f() { consume(^attrs.first ^attrs.second mask ^ 1); }",
    ] {
        assert_error_with_code(source, PARSE_FOREIGN_HABIT, "bit_xor");
    }
}

// =============================================================================
// 1. Whitespace-Separated Operators & Sensitive Syntax Forms
// =============================================================================

#[test]
fn separated_binary_operators_report_missing_whitespace_on_either_or_both_sides() {
    for (op, sample_fix) in [
        ("+", "a + b"),
        ("-", "a - b"),
        ("*", "a * b"),
        ("/", "a / b"),
        ("%", "a % b"),
        ("<", "a < b"),
        (">", "a > b"),
    ] {
        // Missing space on both sides (`a+b`)
        assert_error_with_code(
            &format!("fn f() {{ let x = a{op}b; }}"),
            PARSE_SEPARATED_OPERATOR,
            sample_fix,
        );
        // Missing space on right side (`a +b`)
        assert_error_with_code(
            &format!("fn f() {{ let x = a {op}b; }}"),
            PARSE_SEPARATED_OPERATOR,
            sample_fix,
        );
        // Missing space on left side (`a+ b`)
        assert_error_with_code(
            &format!("fn f() {{ let x = a{op} b; }}"),
            PARSE_SEPARATED_OPERATOR,
            sample_fix,
        );
    }

    // Unspaced `<` inside `if` / `while` headers rewinds cleanly before `{` without breaking outer blocks!
    assert_error_with_code(
        "fn f(a: i32, b: i32) -> i32 { if a<b { return 1; }; return 0; }",
        PARSE_SEPARATED_OPERATOR,
        "a < b",
    );
    assert_error_with_code(
        "fn f(n: i32) { var i = 0; while i<n { i += 1; }; }",
        PARSE_SEPARATED_OPERATOR,
        "i < n",
    );
}

#[test]
fn valid_prefix_postfix_generic_and_statement_boundaries_stay_clean() {
    for valid in [
        "fn f(x: *T, y: i32) -> List<List<T>> where T { let z = -y; let v = x.*; list }",
        "fn f() { if ready {} -value; if ready {} *Ptr; }",
        "typealias Handler<Ctx, T> = %Ctx fn(T) -> !Failure ?T;",
    ] {
        assert_clean(valid);
    }
}

#[test]
fn sensitive_separators_and_loop_labels_are_diagnosed_clearly() {
    // Missing `;` between statements
    assert_error_with_code("fn f() { let a = 1\n let b = 2; }", PARSE_SEPARATOR, "`;`");

    // `,` written instead of `;` between statements in block
    assert_error_with_code("fn f() { let a = 1, let b = 2; }", PARSE_SEPARATOR, "`;`");

    // `;` written instead of `,` in struct fields and function parameters
    assert_error_with_code("struct Point { x: i32; y: i32 }", PARSE_SEPARATOR, "`,`");
    assert_error_with_code(
        "fn add(a: i32; b: i32) -> i32 { a + b }",
        PARSE_SEPARATOR,
        "`,`",
    );

    // Stray empty `;` in block vs valid `if ok {};`
    assert_error_with_code("fn f() { let a = 1;; }", PARSE_SEPARATOR, "empty `;`");
    assert_clean("fn f(ok: bool) { if ok {}; };");

    // `.field = value` inside `{ ... }`
    assert_error_with_code(
        "fn f() { let cfg = Config { .width = 80 }; }",
        PARSE_SEPARATOR,
        "width: 80",
    );

    // Loop labels (`'outer: while`, `while 'outer`, `break :outer`, `break 'outer`, `continue :outer`)
    assert_error_with_code(
        "fn f() { 'outer: while ready { break; } }",
        PARSE_FOREIGN_HABIT,
        "while :outer",
    );
    assert_error_with_code(
        "fn f() { while 'outer ready { break; } }",
        PARSE_FOREIGN_HABIT,
        ":outer",
    );
    assert_error_with_code(
        "fn f() { while :outer ready { break :outer; } }",
        PARSE_SEPARATOR,
        "break outer",
    );
    assert_error_with_code(
        "fn f() { while :outer ready { continue 'outer; } }",
        PARSE_FOREIGN_HABIT,
        "continue outer",
    );
    assert_clean("fn f() { while :outer ready { if done { break outer; }; continue outer; } }");
}

// =============================================================================
// 2. Cross-Language Habit Heuristics
// =============================================================================

#[test]
fn cross_language_operator_and_syntax_habits_give_targeted_flurry_guidance() {
    // Logical & predicate operators
    assert_error_with_code("fn f() -> bool { a && b }", PARSE_FOREIGN_HABIT, "and");
    assert_error_with_code("fn f() -> bool { a || b }", PARSE_FOREIGN_HABIT, "or");
    assert_clean("fn f() { let thunk = || 42; }");

    assert_error_with_code("fn f() -> bool { !ready }", PARSE_FOREIGN_HABIT, "not");
    assert_error_with_code(
        "fn f(x: bool) -> i32 { x match { !true => 0, _ => 1 } }",
        PARSE_FOREIGN_HABIT,
        "not",
    );
    assert_error_with_code("fn f() -> bool { a !== b }", PARSE_FOREIGN_HABIT, "!=");
    // Note: `==>` is Flurry's `BoolImplies` operator (`TokenKind::EqEqEq`).
    assert_clean("fn f(p: bool, q: bool) -> bool { p ==> q }");

    // Pattern alternatives & conjunctions
    assert_error_with_code(
        "fn f(x: i32) -> bool { x match { 1 | 2 => true, _ => false } }",
        PARSE_FOREIGN_HABIT,
        "or",
    );
    assert_error_with_code(
        "fn f(x: i32) -> bool { x match { 1 || 2 => true, _ => false } }",
        PARSE_FOREIGN_HABIT,
        "or",
    );

    // Bitwise operators (`&`, `^`, `~`, `<<`, `>>`)
    assert_error_with_code("fn f() -> u32 { mask & 1 }", PARSE_FOREIGN_HABIT, "bit_and");
    assert_error_with_code("fn f() -> u32 { mask ^ 1 }", PARSE_FOREIGN_HABIT, "bit_xor");
    assert_error_with_code("fn f() -> u32 { ~mask }", PARSE_FOREIGN_HABIT, "not");
    assert_error_with_code("fn f() -> u32 { x << 2 }", PARSE_FOREIGN_HABIT, "<<");
    assert_error_with_code("fn f() -> u32 { x >> 2 }", PARSE_FOREIGN_HABIT, ">>");

    // Taking references, pointers, and mutability
    assert_error_with_code("fn f() { let p = &value; }", PARSE_FOREIGN_HABIT, ".ref");
    assert_error_with_code(
        "fn f() { let p = &mut value; }",
        PARSE_FOREIGN_HABIT,
        ".ref",
    );
    assert_error_with_code("fn f(&self) {}", PARSE_FOREIGN_HABIT, "self");
    assert_error_with_code("fn f(&mut self) {}", PARSE_FOREIGN_HABIT, "*self");
    assert_error_with_code("fn f() { let mut count = 0; }", PARSE_FOREIGN_HABIT, "var");
    assert_error_with_code("fn f(mut count: i32) {}", PARSE_FOREIGN_HABIT, "var");
    assert_error_with_code(
        "fn f(p: *i32) { p match { *x => {} } }",
        PARSE_FOREIGN_HABIT,
        ".*",
    );

    // `*ptr = val;` emits a non-fatal warning recommending `ptr.* = val;`
    let (ok, diags) = parse_with_diagnostics("fn f(ptr: *i32) { *ptr = 1; }");
    assert!(
        ok,
        "expected `*ptr = 1;` to parse with warning, got: {diags:#?}"
    );
    assert!(
        diags
            .iter()
            .any(|d| d.level == Level::Warning && d.code == Some(PARSE_WARN_DEREF_ASSIGN)),
        "expected PARSE_WARN_DEREF_ASSIGN warning, got: {diags:#?}"
    );

    // Other foreign habits: `::`, `//`, `/*`, prefix `match`, infix `as`, `fn(): T`, `extern fn`, `<'a>`
    assert_error_with_code("fn f() { let x = std::mem; }", PARSE_FOREIGN_HABIT, ".");
    assert_error_with_code("use std::mem;", PARSE_FOREIGN_HABIT, ".");
    assert_error_with_code(
        "fn f() { // comment\n let x = 1; }",
        PARSE_FOREIGN_HABIT,
        "--",
    );
    assert_error_with_code(
        "fn f() { /* block */ let x = 1; }",
        PARSE_FOREIGN_HABIT,
        "{-",
    );
    assert_error_with_code(
        "fn f(x: i32) -> i32 { match x { _ => 0 } }",
        PARSE_FOREIGN_HABIT,
        "x match",
    );
    assert_error_with_code(
        "fn f(x: i32) -> i64 { x as i64 }",
        PARSE_FOREIGN_HABIT,
        ".as(",
    );
    assert_error_with_code("fn f(x: i32): i32 { x }", PARSE_FOREIGN_HABIT, "->");
    assert_error_with_code("extern fn puts(s: *u8);", PARSE_FOREIGN_HABIT, "attrs.abi");
    assert_error_with_code("fn id<T>(x: T) -> T { x }", PARSE_FOREIGN_HABIT, "where");
    assert_error_with_code(
        "pure comptime fn Borrow<'a>(T: Type) -> Type { T }",
        PARSE_FOREIGN_HABIT,
        "lifetime",
    );
}

// =============================================================================
// 3. Extended Application & Extended Application Pattern Brace Conflicts
// =============================================================================

#[test]
fn forward_and_reverse_extended_application_brace_conflicts_are_detected() {
    // Forward conflicts in control-flow headers
    for forward in [
        "fn f() { if Config { debug: true } { run(); } }",
        "fn f() { if value matches .some { item: x } { use_val(x); } }",
        "fn f() { if pt matches Point { x } { use_val(x); } }",
        "fn f() { while State { active: true } { step(); } }",
        "fn f() { for x in Range { stop: 10 } { step(x); } }",
        "fn f() -> Output { ok: bool } { make_output() }",
        "impl Proto { Item: i32 } for Widget {}",
        "typealias MyRec = Builder { width: i32 };",
    ] {
        assert_error_with_code(forward, PARSE_BRACE_CONFLICT, "(");
    }

    // Parenthesizing resolves every forward conflict cleanly!
    for resolved in [
        "fn f() { if (Config { debug: true }) { run(); } }",
        "fn f() { if (value matches .some { item: x }) { use_val(x); } }",
        "fn f() { if (pt matches Point { x }) { use_val(x); } }",
        "fn f() { while (State { active: true }) { step(); } }",
        "fn f() { for x in (Range { stop: 10 }) { step(x); } }",
        "fn f() -> (Output { ok: bool }) { make_output() }",
        "impl (Proto { Item: i32 }) for Widget {}",
        "typealias MyRec = (Builder { width: i32 });",
    ] {
        assert_clean(resolved);
    }

    // Reverse conflicts: omitted `do` in `if ... is` / `while ... is`, or omitted `=>` in `match` / `when`
    assert_error_with_code(
        "fn f(x: Opt) -> i32 { if x is .some { return 1; } }",
        PARSE_BRACE_CONFLICT,
        "do",
    );
    assert_error_with_code(
        "fn f(x: Opt) -> i32 { if x is .some { 1 } }",
        PARSE_BRACE_CONFLICT,
        "do",
    );
    assert_error_with_code(
        "fn f(x: Opt) { while x is .some { step(); } }",
        PARSE_BRACE_CONFLICT,
        "do",
    );
    assert_error_with_code(
        "fn f(x: Opt) -> i32 { x match { .ok { return 1; } } }",
        PARSE_BRACE_CONFLICT,
        "=>",
    );
    assert_error_with_code(
        "fn f(x: Opt) -> i32 { x match { .ok { value } } }",
        PARSE_BRACE_CONFLICT,
        "=>",
    );
    assert_error_with_code(
        "fn f(ready: bool) -> i32 { when { ready { return 1; } } }",
        PARSE_BRACE_CONFLICT,
        "=>",
    );
}

// =============================================================================
// 4. Multi-Error Recovery & Unclosed Delimiters
// =============================================================================

#[test]
fn parser_recovers_across_statements_comma_lists_and_items_in_one_pass() {
    let source = r#"
struct BrokenFields {
    a: i32;
    b: i32,
}

fn first(mut x: i32) -> i32 {
    let mut local = x+1;
    if local && true {
        return local;
    }
    return 0;
}

fn second(flag: bool) -> bool {
    !flag
}
"#;

    let (ok, diags) = parse_with_diagnostics(source);
    assert!(!ok);
    let error_codes: Vec<u32> = diags
        .iter()
        .filter(|d| d.level == Level::Error)
        .filter_map(|d| d.code)
        .collect();
    assert!(
        error_codes.len() >= 5,
        "expected recovery to report errors across all 3 top-level items, got {} errors:\n{diags:#?}",
        error_codes.len()
    );
    assert!(error_codes.contains(&PARSE_SEPARATOR));
    assert!(error_codes.contains(&PARSE_SEPARATED_OPERATOR));
    assert!(error_codes.contains(&PARSE_FOREIGN_HABIT));
}

#[test]
fn unclosed_delimiters_carry_opening_span_secondary_labels() {
    let source = "fn broken() {\n    let items = [1, 2, 3;\n}";
    let (ok, diags) = parse_with_diagnostics(source);
    assert!(!ok);
    let has_unclosed = diags
        .iter()
        .any(|d| d.code == Some(PARSE_UNCLOSED_DELIMITER) && d.labels.len() >= 2);
    assert!(
        has_unclosed,
        "expected unclosed delimiter diagnostic with secondary opening label, got:\n{diags:#?}"
    );
}

// =============================================================================
// 5. Complex Expr & Pattern Parenthesization Recommendations (Warnings)
// =============================================================================

#[test]
fn mixed_prefix_infix_postfix_expressions_and_patterns_recommend_parentheses() {
    for expr_src in [
        "fn f(a: i32, b: i32) -> bool { not a == b }",
        "fn f(a: bool, b: bool, c: bool) -> bool { not a and b or c }",
        "fn f(x: i32, y: i32) -> !i32 void { error x + y }",
        "typealias FnPtr = *fn(i32) -> bool;",
        "typealias OptUnion = ?Alpha | Beta;",
        "typealias ErrUnion = !Err Alpha | Beta;",
        "fn f() -> i8 { -128.as(i8) }",
        "fn f(x: bool) -> bool { not x match { _ => true } }",
        "fn f(a: bool, b: bool) { let r = ..a and b; }",
    ] {
        let (ok, diags) = parse_with_diagnostics(expr_src);
        assert!(
            ok,
            "expected valid parse with warning for:\n{expr_src}\ngot:\n{diags:#?}"
        );
        assert!(
            diags
                .iter()
                .any(|d| d.level == Level::Warning && d.code == Some(PARSE_WARN_PARENTHESES)),
            "expected PARSE_WARN_PARENTHESES (2901) for:\n{expr_src}\ngot:\n{diags:#?}"
        );
    }

    // Explicit parentheses silence every expression warning!
    for clean_expr_src in [
        "fn f(a: i32, b: i32) -> bool { not (a == b) }",
        "fn f(a: i32, b: i32) -> bool { (not a) == b }",
        "fn f(a: bool, b: bool, c: bool) -> bool { ((not a) and b) or c }",
        "fn f(x: i32, y: i32) -> !i32 void { error (x + y) }",
        "typealias FnPtr = *(fn(i32) -> bool);",
        "typealias OptUnion = (?Alpha) | Beta;",
        "typealias ErrUnion = (!Err Alpha) | Beta;",
        "fn f() -> i8 { (-128).as(i8) }",
        "fn f(x: bool) -> bool { not (x match { _ => true }) }",
        "fn f(a: bool, b: bool) { let r = ..(a and b); }",
    ] {
        assert_clean(clean_expr_src);
    }

    // Complex mixed patterns emit PARSE_WARN_PARENTHESES (2901)
    for pat_src in [
        "fn f(x: bool) -> bool { x match { not true or false => true, _ => false } }",
        "fn f(x: Pair) { x match { ref inner as whole => {} } }",
        "fn f(x: Res) { x match { error code if code > 0 => {}, _ => {} } }",
        "fn f(x: ?bool) { x match { not flag? => {}, _ => {} } }",
    ] {
        let (ok, diags) = parse_with_diagnostics(pat_src);
        assert!(
            ok,
            "expected valid parse with pattern warning for:\n{pat_src}\ngot:\n{diags:#?}"
        );
        assert!(
            diags
                .iter()
                .any(|d| { d.level == Level::Warning && d.code == Some(PARSE_WARN_PARENTHESES) }),
            "expected PARSE_WARN_PARENTHESES (2901) for:\n{pat_src}\ngot:\n{diags:#?}"
        );
    }

    // Explicit parentheses silence every pattern warning!
    for clean_pat_src in [
        "fn f(x: bool) -> bool { x match { not (true or false) => true, _ => false } }",
        "fn f(x: bool) -> bool { x match { (not true) or false => true, _ => false } }",
        "fn f(x: Pair) { x match { (ref inner) as whole => {} } }",
        "fn f(x: Pair) { x match { ref (inner as whole) => {} } }",
        "fn f(x: Res) { x match { (error code) if code > 0 => {}, _ => {} } }",
        "fn f(x: ?bool) { x match { not (flag?) => {}, _ => {} } }",
    ] {
        assert_clean(clean_pat_src);
    }
}

// =============================================================================
// 6. Full `library/**/*.fl` Corpus Regression Check
// =============================================================================

fn collect_fl_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect_fl_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "fl") {
            out.push(path);
        }
    }
}

#[test]
fn entire_builtin_library_corpus_parses_with_zero_errors() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let library_dir = manifest_dir.join("../../library");
    let mut files = Vec::new();
    collect_fl_files(&library_dir, &mut files);
    assert!(
        !files.is_empty(),
        "expected `.fl` files under {}",
        library_dir.display()
    );

    let mut error_files = Vec::new();
    for path in &files {
        let src = fs::read_to_string(path).expect("read library .fl file");
        let (ok, diags) = parse_with_diagnostics(&src);
        let errs: Vec<_> = diags
            .into_iter()
            .filter(|d| d.level == Level::Error)
            .collect();
        if !ok || !errs.is_empty() {
            error_files.push((path.clone(), errs));
        }
    }

    assert!(
        error_files.is_empty(),
        "library `.fl` files failed to parse:\n{error_files:#?}"
    );
}

#[test]
fn struct_rest_pattern_three_dots_accepted_and_invalid_forms_rejected() {
    for clean_src in [
        "fn f(p: Point) { let Point { x, ... } = p; }",
        "fn f(p: Point) { let { x, ... } = p; }",
        "fn f(p: Point) { let { ... } = p; }",
        "fn f(m: Msg) { m match { .data { a, ... } => a, _ => false } }",
    ] {
        let (ok, diags) = parse_with_diagnostics(clean_src);
        let errs: Vec<_> = diags
            .into_iter()
            .filter(|d| d.level == Level::Error)
            .collect();
        assert!(
            ok && errs.is_empty(),
            "expected clean parse for {clean_src}: {errs:#?}"
        );
    }

    for (bad_src, needle) in [
        (
            "fn f(p: Point) { let Point { x, .. } = p; }",
            "use `...` (three dots) instead of `..`",
        ),
        (
            "fn f(p: Point) { let { x, .. } = p; }",
            "use `...` (three dots) instead of `..`",
        ),
        (
            "fn f(p: Point) { let Point { ..., x } = p; }",
            "must be the last entry",
        ),
        (
            "fn f(p: Point) { let { x, ..., ... } = p; }",
            "duplicate `...`",
        ),
    ] {
        let (_ok, diags) = parse_with_diagnostics(bad_src);
        assert!(
            diags
                .iter()
                .any(|d| d.level == Level::Error && d.message.contains(needle)),
            "expected diagnostic `{needle}` for `{bad_src}`, got: {diags:#?}"
        );
    }
}
