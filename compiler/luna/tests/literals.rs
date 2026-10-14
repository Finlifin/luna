//! Black-box acceptance for development/runs/002-literal-foundations/contract.md.
//! Source semantics come from docs/basics/literals.md and docs/grammar/{syntax,index.md}.
//! These fixtures require neither a standard library nor literal-conversion elaboration.

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);
const OLD_OUTPUT: &[u8] = b"previous literal dump\n\0preserve all bytes";

struct Sandbox(PathBuf);

impl Sandbox {
    fn new(source: &str) -> Self {
        loop {
            let serial = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("luna-literals-{}-{serial}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => {
                    fs::write(path.join("input.fl"), source).unwrap();
                    return Self(path);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("cannot create {}: {error}", path.display()),
            }
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_luna"));
        command
            .current_dir(&self.0)
            .env_remove("FLURRY_SYSROOT")
            .env("NO_COLOR", "1")
            .env("RUST_BACKTRACE", "0")
            .arg("input.fl")
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().unwrap();
        // Drain concurrently so error recovery cannot fill a pipe and hang a test.
        fn drain(mut pipe: impl Read) -> Vec<u8> {
            const LIMIT: usize = 1024 * 1024;
            let mut result = Vec::new();
            let mut buffer = [0; 8192];
            loop {
                match pipe.read(&mut buffer) {
                    Ok(0) => return result,
                    Ok(count) => {
                        let keep = count.min(LIMIT.saturating_sub(result.len()));
                        result.extend_from_slice(&buffer[..keep]);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) => panic!("cannot read CLI output: {error}"),
                }
            }
        }
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let stdout_reader = thread::spawn(move || drain(stdout));
        let stderr_reader = thread::spawn(move || drain(stderr));
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut timed_out = false;
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                timed_out = true;
                let _ = child.kill();
                break child.wait().unwrap();
            }
            thread::sleep(Duration::from_millis(10));
        };
        let output = Output {
            status,
            stdout: stdout_reader.join().unwrap(),
            stderr: stderr_reader.join().unwrap(),
        };
        assert!(!timed_out, "CLI timeout: {}", details(&output));
        output
    }

    fn hir(&self) -> Tree {
        let output = self.run(&["--emit", "hir"]);
        assert_eq!(output.status.code(), Some(0), "{}", details(&output));
        assert!(output.stderr.is_empty(), "{}", details(&output));
        let tree = Tree::parse(std::str::from_utf8(&output.stdout).unwrap());
        assert_eq!(tree.tag(), Some("hir-package"), "{tree:?}");
        tree
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn details(output: &Output) -> String {
    format!(
        "status: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

// A small dump reader lets assertions inspect a conversion or constant-pattern
// subtree, rather than accidentally finding the expected value in its sibling.
// It also checks that output is one complete, escaped S-expression.
#[derive(Debug, PartialEq)]
enum Tree {
    Atom(String),
    Text(String),
    List(Vec<Tree>),
}

impl Tree {
    fn parse(source: &str) -> Self {
        fn parse_one(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Tree {
            while chars.peek().is_some_and(|c| c.is_whitespace()) {
                chars.next();
            }
            match chars.next().expect("incomplete dump") {
                '(' => {
                    let mut children = Vec::new();
                    loop {
                        while chars.peek().is_some_and(|c| c.is_whitespace()) {
                            chars.next();
                        }
                        if chars.peek() == Some(&')') {
                            chars.next();
                            return Tree::List(children);
                        }
                        children.push(parse_one(chars));
                    }
                }
                '"' => {
                    let mut text = String::new();
                    loop {
                        match chars.next().expect("unterminated dump string") {
                            '"' => return Tree::Text(text),
                            '\\' => {
                                text.push(match chars.next().expect("incomplete dump escape") {
                                    '\\' => '\\',
                                    '"' => '"',
                                    'n' => '\n',
                                    'r' => '\r',
                                    't' => '\t',
                                    'u' => {
                                        let hex: String = chars.take(4).collect();
                                        assert_eq!(hex.len(), 4, "short dump Unicode escape");
                                        char::from_u32(u32::from_str_radix(&hex, 16).unwrap())
                                            .expect("invalid scalar in dump")
                                    }
                                    other => panic!("unexpected dump escape: {other}"),
                                });
                            }
                            c => text.push(c),
                        }
                    }
                }
                ')' => panic!("unexpected closing parenthesis"),
                first => {
                    let mut atom = String::from(first);
                    while chars
                        .peek()
                        .is_some_and(|c| !c.is_whitespace() && *c != ')')
                    {
                        atom.push(chars.next().unwrap());
                    }
                    Tree::Atom(atom)
                }
            }
        }
        let mut chars = source.chars().peekable();
        let tree = parse_one(&mut chars);
        assert!(chars.all(char::is_whitespace), "text after dump root");
        tree
    }

    fn tag(&self) -> Option<&str> {
        match self {
            Self::List(children) => match children.first() {
                Some(Self::Atom(tag)) => Some(tag),
                _ => None,
            },
            _ => None,
        }
    }

    fn forms<'a>(&'a self, tag: &str) -> Vec<&'a Tree> {
        let mut found = Vec::new();
        if self.tag() == Some(tag) {
            found.push(self);
        }
        if let Self::List(children) = self {
            for child in children {
                found.extend(child.forms(tag));
            }
        }
        found
    }

    fn field(&self, tag: &str) -> &Tree {
        let Self::List(children) = self else {
            panic!("expected list: {self:?}");
        };
        let fields: Vec<_> = children
            .iter()
            .filter(|child| child.tag() == Some(tag))
            .collect();
        assert_eq!(fields.len(), 1, "expected one {tag} field: {self:?}");
        let Self::List(values) = fields[0] else {
            unreachable!()
        };
        assert_eq!(values.len(), 2, "expected one {tag} value: {self:?}");
        &values[1]
    }
}

fn assert_text(tree: &Tree, kind: &str, field: &str, expected: &str) {
    let forms = tree.forms(kind);
    assert_eq!(forms.len(), 1, "expected one {kind}: {tree:?}");
    assert_eq!(forms[0].field(field), &Tree::Text(expected.to_owned()));
}

fn expression(source: &str) -> String {
    format!("fn literal_case() {{ {source}; }}\n")
}

fn assert_conversion(tree: &Tree, kind: &str, field: &str, value: &str, suffix: &str) {
    let forms = tree.forms("literal-conversion");
    assert_eq!(forms.len(), 1, "expected exactly one conversion: {tree:?}");
    assert_text(forms[0].field("literal"), kind, field, value);
    assert_text(forms[0].field("suffix"), "ident", "name", suffix);
    assert!(forms[0].forms("type-cast").is_empty(), "{tree:?}");
}

fn assert_rejected(source: &str, emissions: &[&str], diagnostic_words: &[&str]) {
    let sandbox = Sandbox::new(source);
    for emit in emissions {
        for target in [None, Some("existing.lisp"), Some("new.lisp")] {
            fs::write(sandbox.0.join("existing.lisp"), OLD_OUTPUT).unwrap();
            let mut args = vec!["--emit", *emit];
            if let Some(target) = target {
                args.extend(["--output", target]);
            }
            let output = sandbox.run(&args);
            assert_eq!(
                output.status.code(),
                Some(1),
                "source: {source}\n{}",
                details(&output)
            );
            assert!(
                output.stdout.is_empty(),
                "source: {source}\n{}",
                details(&output)
            );
            let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
            assert!(
                !stderr.is_empty() && !stderr.contains("panicked at"),
                "{stderr}"
            );
            for word in diagnostic_words {
                assert!(stderr.contains(word), "missing diagnostic {word}: {stderr}");
            }
            assert_eq!(
                fs::read(sandbox.0.join("existing.lisp")).unwrap(),
                OLD_OUTPUT
            );
            assert!(
                !sandbox.0.join("new.lisp").exists(),
                "published failed dump"
            );
            assert_eq!(
                fs::read_to_string(sandbox.0.join("input.fl")).unwrap(),
                source
            );
        }
    }
}

#[test]
fn integer_radices_and_arbitrary_precision_preserve_exact_spelling() {
    for spelling in [
        "0",
        "00",
        "1__234_567",
        "9223372036854775808",
        "18446744073709551616",
        "1234567890123456789012345678901234567890123456789012345678901234567890",
        "0b0",
        "0b1010_1010",
        "0b1_00000000_00000000_00000000_00000000_00000000_00000000_00000000_00000000",
        "0o0",
        "0o777_123",
        "0o177777777777777777777777777777777777777777777777777",
        "0x0",
        "0xFF_23",
        "0xabcd",
        "0x1234_ABCD_EFFF_0011_2233_4455_6677_8899",
        "0xffff_ffff_ffff_ffff_ffff_ffff_ffff_ffff_ffff_ffff",
    ] {
        let tree = Sandbox::new(&expression(spelling)).hir();
        assert_text(&tree, "integer", "spelling", spelling);
    }
}

#[test]
fn float_fraction_and_exponent_preserve_precision_and_spelling() {
    for spelling in [
        "0.0",
        "0001.0000",
        "1_234.567_89",
        "1.0e6",
        "1.0E+6",
        "1.0e-6",
        "1.2345678901234567890123456789012345678901234567890123456789",
        "9007199254740993.0",
        "1.0000000000000001",
        "1.0e10000",
        "1.0e-10000",
        "123_456.789_012E+000123",
    ] {
        let tree = Sandbox::new(&expression(spelling)).hir();
        assert_text(&tree, "float", "spelling", spelling);
    }
}

#[test]
fn negative_sign_remains_an_operator_outside_the_literal() {
    let tree = Sandbox::new(&expression("-18446744073709551616")).hir();
    assert_text(&tree, "integer", "spelling", "18446744073709551616");
    // The exact dump tag of the prefix operator is established by HIR Display.
    assert_eq!(
        tree.forms("unary").len(),
        1,
        "negative operand lost: {tree:?}"
    );
}

#[test]
fn documented_character_escapes_and_unicode_scalar_boundaries_decode() {
    for (source, value) in [
        ("'a'", "a"),
        ("'Z'", "Z"),
        ("' '", " "),
        ("'Σ'", "Σ"),
        ("'😀'", "😀"),
        (r"'\\'", "\\"),
        (r"'\''", "'"),
        (r#"'\"'"#, "\""),
        (r"'\n'", "\n"),
        (r"'\r'", "\r"),
        (r"'\t'", "\t"),
        (r"'\0'", "\0"),
        (r"'\$'", "$"),
        (r"'\x{0}'", "\0"),
        (r"'\x{000041}'", "A"),
        (r"'\x{3A3}'", "Σ"),
        (r"'\x{D7FF}'", "\u{d7ff}"),
        (r"'\x{E000}'", "\u{e000}"),
        (r"'\x{10FFFF}'", "\u{10ffff}"),
    ] {
        let tree = Sandbox::new(&expression(source)).hir();
        assert_text(&tree, "char", "value", value);
    }
}

#[test]
fn strings_decode_documented_escapes_without_losing_control_characters() {
    for (source, value) in [
        (r#""""#, ""),
        (r#""plain Σ😀""#, "plain Σ😀"),
        (r#""\\\'\"\n\r\t\0\$""#, "\\'\"\n\r\t\0$"),
        (r#""\x{0}\x{41}\x{3A3}\x{01F600}""#, "\0AΣ😀"),
        (
            r#""\x{D7FF}\x{E000}\x{10FFFF}""#,
            "\u{d7ff}\u{e000}\u{10ffff}",
        ),
        (
            r#""(nested) \"quoted\" \\ path""#,
            "(nested) \"quoted\" \\ path",
        ),
    ] {
        assert_text(
            &Sandbox::new(&expression(source)).hir(),
            "string",
            "value",
            value,
        );
    }
}

#[test]
fn adjacent_strings_concatenate_after_decoding_each_leaf() {
    for (source, value) in [
        (r#""a""b""#, "ab"),
        (r#""say you, " "say me""#, "say you, say me"),
        ("\"a\"\t\"b\"\n\"c\"\r\n\"d\"", "abcd"),
        (r#""" "" """#, ""),
        (r#""a\n" "\x{3A3}" "\tend""#, "a\nΣ\tend"),
        (r#""\\" "n""#, "\\n"),
        (r#""\$" "name""#, "$name"),
    ] {
        assert_text(
            &Sandbox::new(&expression(source)).hir(),
            "string",
            "value",
            value,
        );
    }
}

#[test]
fn literal_suffix_is_one_unresolved_conversion_request_for_all_four_kinds() {
    for (source, kind, field, value, suffix) in [
        ("100u32px", "integer", "spelling", "100", "u32px"),
        (
            "18446744073709551616huge",
            "integer",
            "spelling",
            "18446744073709551616",
            "huge",
        ),
        ("0b1010bits", "integer", "spelling", "0b1010", "bits"),
        ("0o777mode", "integer", "spelling", "0o777", "mode"),
        ("0xFFFFword", "integer", "spelling", "0xFFFF", "word"),
        (
            "1.234567890123456789precise",
            "float",
            "spelling",
            "1.234567890123456789",
            "precise",
        ),
        ("1.0e10000wide", "float", "spelling", "1.0e10000", "wide"),
        ("'a'letter", "char", "value", "a", "letter"),
        (r"'\x{3A3}'letter", "char", "value", "Σ", "letter"),
        (r#""message"c"#, "string", "value", "message", "c"),
        (r#""a" "b"joined"#, "string", "value", "ab", "joined"),
    ] {
        assert_conversion(
            &Sandbox::new(&expression(source)).hir(),
            kind,
            field,
            value,
            suffix,
        );
    }
}

#[test]
fn suffix_identifiers_do_not_need_a_local_declaration_to_reach_hir() {
    for source in ["23 custom_unit", "23custom_unit"] {
        assert_conversion(
            &Sandbox::new(&expression(source)).hir(),
            "integer",
            "spelling",
            "23",
            "custom_unit",
        );
    }
}

#[test]
fn exponent_like_identifiers_remain_suffixes_when_they_are_not_exponents() {
    // A separated e6 cannot form an exponent across whitespace, but it is a
    // valid custom suffix identifier. Bare e has no exponent digits either.
    for (source, kind, spelling, suffix) in [
        ("1.0 e6", "float", "1.0", "e6"),
        ("1.0e", "float", "1.0", "e"),
        ("1e", "integer", "1", "e"),
    ] {
        assert_conversion(
            &Sandbox::new(&expression(source)).hir(),
            kind,
            "spelling",
            spelling,
            suffix,
        );
    }
}

#[test]
fn constant_patterns_preserve_decimal_float_character_and_concatenated_string() {
    for (pattern, kind, field, value) in [
        (
            "18446744073709551616",
            "integer",
            "spelling",
            "18446744073709551616",
        ),
        ("1.0e10000", "float", "spelling", "1.0e10000"),
        (r"'\x{3A3}'", "char", "value", "Σ"),
        (r#""one" "two""#, "string", "value", "onetwo"),
    ] {
        // `matches` avoids constructing a standard-library type or match arm body.
        let source = format!("fn literal_case() {{ let value = 0; value matches {pattern}; }}\n");
        let tree = Sandbox::new(&source).hir();
        let patterns = tree.forms("pattern");
        let constant = patterns
            .iter()
            .find_map(|pattern| pattern.forms("const").first().copied())
            .unwrap_or_else(|| panic!("no constant pattern: {tree:?}"));
        assert_text(constant, kind, field, value);
    }
}

#[test]
fn suffix_in_a_pattern_remains_a_constant_conversion_expression() {
    for (pattern, kind, field, value, suffix) in [
        ("100u32px", "integer", "spelling", "100", "u32px"),
        ("1.0e10000wide", "float", "spelling", "1.0e10000", "wide"),
        ("'a'letter", "char", "value", "a", "letter"),
        (r#""a" "b"joined"#, "string", "value", "ab", "joined"),
    ] {
        let source = format!("fn literal_case() {{ let value = 0; value matches {pattern}; }}\n");
        let tree = Sandbox::new(&source).hir();
        let constants = tree.forms("const");
        assert_eq!(constants.len(), 1, "constant pattern missing: {tree:?}");
        assert_conversion(constants[0], kind, field, value, suffix);
    }
}

#[test]
fn character_literals_and_take_views_keep_distinct_boundaries() {
    let tree =
        Sandbox::new("fn literal_case() { let value = 'a'; value'type; ' 'letter; }\n").hir();
    assert_eq!(tree.forms("char").len(), 2, "{tree:?}");
    let views = tree.forms("type-of");
    assert_eq!(views.len(), 1, "{tree:?}");
    assert_text(views[0], "resolved-local", "name", "value");
    assert_conversion(&tree, "char", "value", " ", "letter");
}

#[test]
fn character_literals_after_statement_blocks_start_new_expressions() {
    // grammar/index.md omits the statement separator after a closing `}`.
    // That brace must not force the following apostrophe to be a take view.
    for preceding_statement in [
        "if true {}",
        "if true {} else {}",
        "if true { if false {} }",
        "while false {}",
    ] {
        for (literal, value) in [("'a'", "a"), ("' '", " "), ("'Σ'", "Σ")] {
            let source = format!("fn f() {{ {preceding_statement} {literal}; }}\n");
            assert_text(&Sandbox::new(&source).hir(), "char", "value", value);
        }
    }
}

#[test]
fn suffixed_characters_after_statement_blocks_remain_literal_conversions() {
    for (gap, literal, value) in [
        ("", "'a'letter", "a"),
        (" ", "'a'letter", "a"),
        ("\n", "' 'letter", " "),
        (" {- between statements -} ", r"'\x{3A3}'letter", "Σ"),
        (" -- between statements\n", "'a'letter", "a"),
    ] {
        let source = format!("fn f() {{ if true {{}}{gap}{literal}; }}\n");
        assert_conversion(
            &Sandbox::new(&source).hir(),
            "char",
            "value",
            value,
            "letter",
        );
    }
}

#[test]
fn object_expression_closing_braces_still_allow_postfix_take_views() {
    // The initializer establishes expression context: these braces close an
    // object, so the following apostrophe still starts `expr ' id`.
    for (value, view_count) in [
        ("{ field: 1 }'type", 1),
        ("{ field: 1 } 'type", 1),
        ("{}'type", 1),
        ("{}'a'type", 2),
        ("{ field: { nested: 1 } }'type", 1),
        ("{ field: 1 }'type'type", 2),
    ] {
        let source = format!("fn f() {{ let value = {value}; }}\n");
        let output = Sandbox::new(&source).run(&["--emit", "ast"]);
        assert_eq!(output.status.code(), Some(0), "{}", details(&output));
        assert!(output.stderr.is_empty(), "{}", details(&output));
        let dump = String::from_utf8(output.stdout).unwrap();
        assert!(dump.contains("(TakeView (Object "), "{dump}");
        assert_eq!(dump.matches("(TakeView ").count(), view_count, "{dump}");
        assert!(
            !dump.contains("(Char "),
            "view misread as character: {dump}"
        );
    }
}

#[test]
fn malformed_radix_digits_prefixes_and_mixed_hex_case_are_rejected() {
    for source in [
        "0b",
        "0o",
        "0x",
        "0b___",
        "0o___",
        "0x___",
        "0b2",
        "0b102",
        "0o8",
        "0o778",
        "0xG",
        "0xaB",
        "0xA_b",
        "0xdeadBEEF",
    ] {
        assert_rejected(&expression(source), &["ast", "hir"], &[]);
    }
}

#[test]
fn malformed_scientific_and_incomplete_decimal_forms_are_rejected() {
    for source in [
        "1.", ".5", "1e6", "1E+6", "1.0e+", "1.0e-", "1.0e 6", "1.0e+ 6", "1.0 e+6",
    ] {
        assert_rejected(&expression(source), &["ast", "hir"], &[]);
    }
}

#[test]
fn unknown_escapes_are_rejected_in_strings_and_characters() {
    for escape in [r"\q", r"\a", r"\b", r"\v", r"\f", r"\u{41}", r"\x41"] {
        for source in [format!("\"{escape}\""), format!("'{escape}'")] {
            assert_rejected(&expression(&source), &["hir"], &[]);
        }
    }
}

#[test]
fn malformed_unicode_escape_lengths_digits_and_non_scalars_are_rejected() {
    for escape in [
        r"\x{}",
        r"\x{0000000}",
        r"\x{G}",
        r"\x{12G}",
        r"\x{41",
        r"\x{D800}",
        r"\x{DFFF}",
        r"\x{110000}",
        r"\x{FFFFFF}",
    ] {
        for source in [format!("\"{escape}\""), format!("'{escape}'")] {
            assert_rejected(&expression(&source), &["hir"], &[]);
        }
    }
}

#[test]
fn characters_require_exactly_one_unicode_scalar() {
    for source in ["''", "'ab'", "'a b'", "'é'", "'😀😀'", r"'\n\t'"] {
        assert_rejected(&expression(source), &["hir"], &[]);
    }
}

#[test]
fn suffixes_cannot_stack_or_attach_to_non_literals() {
    for source in [
        "23first second",
        "1.0first second",
        "'a'first second",
        r#""a"first second"#,
        "true unit",
        "false unit",
        "()unit",
        "(.ok)unit",
    ] {
        assert_rejected(&expression(source), &["ast", "hir"], &[]);
    }
}

#[test]
fn concatenation_rejects_intervening_comments_and_ends_at_a_suffix() {
    for source in [
        r#""a" {- comment -} "b""#,
        "\"a\" -- comment\n\"b\"",
        r#""a"first "b""#,
    ] {
        assert_rejected(&expression(source), &["ast", "hir"], &[]);
    }
}

#[test]
fn unsupported_interpolation_diagnoses_without_publishing_hir() {
    for source in [
        r#""$name""#,
        r#""${1}""#,
        r#""before $name after""#,
        r#""plain" "${1}""#,
        r#""\\$name""#,
    ] {
        let source = expression(source);
        // Interpolation is syntactically a string; this milestone rejects it
        // during lowering, before any fixed string can be published.
        let ast = Sandbox::new(&source).run(&["--emit", "ast"]);
        assert_eq!(ast.status.code(), Some(0), "{}", details(&ast));
        assert!(!ast.stdout.is_empty());
        assert_rejected(&source, &["hir"], &["unsupported", "interpolation"]);
    }
}

#[test]
fn interpolation_inner_quotes_and_nested_delimiters_reach_explicit_lowering_error() {
    // `${expr}` admits the ordinary expression grammar. Quotes within that
    // expression belong to its strings/chars, not to the enclosing string.
    // This only tests lexical boundaries and the milestone's unsupported
    // diagnostic; it does not claim interpolation elaboration or evaluation.
    for literal in [
        r#""${"x"}""#,
        r#""before ${"x"} after""#,
        r#""${"}"}""#,
        r#""${"{"}""#,
        r#""${"\""}""#,
        r#""${"\\"}""#,
        r#""${'}'}""#,
        r#""${'{'}""#,
        r#""${'\"'}""#,
        r#""${'\''}""#,
        r#""${{ key: "}" }}""#,
        r#""${["}", "{"]}""#,
        r#""${("}", "{")}""#,
        r#""${"nested ${"x"}"}""#,
        r#""${"x"} ${"y"}""#,
        r#""${"x"}" "tail""#,
        r#""${"x"}"custom"#,
        r#""${"cost \$5"}""#,
        r#""${1 {- " } ' -}}""#,
        r#""${1 {- outer {- " } ' -} tail -}}""#,
        r#""${1 -- " } '
}""#,
    ] {
        let source = expression(literal);
        let ast = Sandbox::new(&source).run(&["--emit", "ast"]);
        assert_eq!(
            ast.status.code(),
            Some(0),
            "source: {source}\n{}",
            details(&ast)
        );
        assert!(ast.stderr.is_empty(), "{}", details(&ast));
        assert!(
            String::from_utf8_lossy(&ast.stdout).contains("(Str "),
            "interpolation string token lost: {}",
            details(&ast)
        );
        assert_rejected(&source, &["hir"], &["unsupported", "interpolation"]);
    }
}

#[test]
fn escaped_dollars_bare_braces_and_noninterpolating_dollars_are_plain_text() {
    for (source, value) in [
        (r#""\$name""#, "$name"),
        (r#""\${1}""#, "${1}"),
        (r#""\x{24}name""#, "$name"),
        (r#""cost \$5""#, "cost $5"),
        (r#""{plain} } {""#, "{plain} } {"),
        (r#""$""#, "$"),
        (r#""$5 $! $ ""#, "$5 $! $ "),
    ] {
        assert_text(
            &Sandbox::new(&expression(source)).hir(),
            "string",
            "value",
            value,
        );
    }
}

#[test]
fn hir_literal_dump_is_deterministic_across_runs_and_explicit_output() {
    let sandbox = Sandbox::new("fn literal_case() { 0xABCD; 1.0e10000; \"(Σ)\\n\"; 'a'letter; }\n");
    let first = sandbox.run(&["--emit", "hir"]);
    assert_eq!(first.status.code(), Some(0), "{}", details(&first));
    Tree::parse(std::str::from_utf8(&first.stdout).unwrap());
    let second = sandbox.run(&["--emit", "hir"]);
    assert_eq!(second.status.code(), Some(0), "{}", details(&second));
    assert_eq!(first.stdout, second.stdout);
    fs::write(sandbox.0.join("dump.lisp"), OLD_OUTPUT).unwrap();
    let saved = sandbox.run(&["--emit", "hir", "--output", "dump.lisp"]);
    assert_eq!(saved.status.code(), Some(0), "{}", details(&saved));
    assert!(saved.stdout.is_empty());
    assert_eq!(fs::read(sandbox.0.join("dump.lisp")).unwrap(), first.stdout);
}

#[test]
fn radix_patterns_retain_bitvec_identity_instead_of_becoming_integer_constants() {
    // docs/control-flow/pattern-matching/bitvec.md reserves radix-prefixed
    // patterns for BitVec, including a pattern with just one fixed segment.
    for (pattern, ast_kind) in [
        ("0b1010", "BitVecBinPattern"),
        ("0o777", "BitVecOctPattern"),
        ("0xABCD", "BitVecHexPattern"),
    ] {
        let source = format!("fn literal_case() {{ let value = 0; value matches {pattern}; }}\n");
        let output = Sandbox::new(&source).run(&["--emit", "ast"]);
        assert_eq!(output.status.code(), Some(0), "{}", details(&output));
        let dump = String::from_utf8(output.stdout).unwrap();
        assert!(dump.contains(ast_kind), "expected {ast_kind}: {dump}");
        let output = Sandbox::new(&source).run(&["--emit", "hir"]);
        assert!(output.status.success(), "{}", details(&output));
        assert!(
            String::from_utf8(output.stdout)
                .unwrap()
                .contains("(bit-vec (radix")
        );
        assert_rejected(
            &source,
            &["nir"],
            &["bit-vector patterns require a bitvec subject"],
        );
    }
}
