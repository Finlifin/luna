//! Lexer contracts from docs/basics/literals.md and docs/grammar/syntax.

use lex::{LexError, Lexer, TokenKind};
use rustc_span::BytePos;

fn tokens(source: &str) -> Vec<(TokenKind, &str)> {
    let (tokens, _, errors) = lex::lex(source, BytePos(100));
    assert!(errors.is_empty(), "{source:?}: {errors:?}");
    tokens
        .into_iter()
        .filter(|token| !matches!(token.kind, TokenKind::Sof | TokenKind::Eof))
        .map(|token| (token.kind, &source[token.from..token.to]))
        .collect()
}

fn assert_literal(source: &str, kind: TokenKind) {
    assert_eq!(tokens(source), [(kind, source)], "{source:?}");
}

fn first_error(source: &str) -> LexError {
    Lexer::new(source, BytePos(100)).next().expect_err(source)
}

#[test]
fn documented_numeric_spellings_keep_every_source_byte() {
    for source in [
        "0",
        "00012",
        "42",
        "1_000_000",
        "1__2___3",
        "99999999999999999999999999999999999999999999999999999999",
    ] {
        assert_literal(source, TokenKind::Int);
    }
    for source in ["0b0", "0b1010_1010", "0b1__0___1"] {
        assert_literal(source, TokenKind::IntBin);
    }
    for source in ["0o0", "0o777", "0o1__7___0"] {
        assert_literal(source, TokenKind::IntOct);
    }
    for source in ["0x0", "0xFF_23", "0xabcd", "0xA__B___0", "0xa__b___0"] {
        assert_literal(source, TokenKind::IntHex);
    }
    for source in ["0.0", "3.14159", "1_234.567_89", "1__2.3___4"] {
        assert_literal(source, TokenKind::Real);
    }
    for source in [
        "1.0e-6",
        "1.0E+6",
        "1.0e6",
        "1__2.3___4E-5__6",
        "1.23456789123456789123456789e999999999999999999999999999",
    ] {
        assert_literal(source, TokenKind::RealSci);
    }
}

#[test]
fn malformed_numbers_are_not_split_into_apparently_valid_literals() {
    for source in [
        "0B1", "0O7", "0XFF", "0b2", "0b102", "0o8", "0o778", "0xAa", "0xaA", "0xA__b", "0b_1",
        "0o_7", "0x_F", "0xg", "0b_", "0x_", "1_", "1__", "0b1_", "0o7__", "0xF__", "1_.0", "1.0_",
        "1.0e_2", "1.0e+", "1.0e-", "1.0e+_1", "1.0e2_", "1e6", "0e6", "1E+6", "1e-6",
    ] {
        assert!(
            matches!(first_error(source), LexError::InvalidNumber { .. }),
            "{source:?}"
        );
    }
}

#[test]
fn suffixes_remain_separate_identifiers_including_e_prefixed_names() {
    for (source, numeral, kind, suffix) in [
        ("23u32", "23", TokenKind::Int, "u32"),
        ("100u32px", "100", TokenKind::Int, "u32px"),
        ("0b10u8", "0b10", TokenKind::IntBin, "u8"),
        ("0o77u32", "0o77", TokenKind::IntOct, "u32"),
        ("0xFFu32", "0xFF", TokenKind::IntHex, "u32"),
        ("2.718f32", "2.718", TokenKind::Real, "f32"),
        ("1.0e6f64", "1.0e6", TokenKind::RealSci, "f64"),
        ("1egg", "1", TokenKind::Int, "egg"),
        ("0egg", "0", TokenKind::Int, "egg"),
        ("1e", "1", TokenKind::Int, "e"),
        ("1E", "1", TokenKind::Int, "E"),
        ("1.0e", "1.0", TokenKind::Real, "e"),
        ("1.0E", "1.0", TokenKind::Real, "E"),
        ("1.0egg", "1.0", TokenKind::Real, "egg"),
        ("'a'letter", "'a'", TokenKind::Char, "letter"),
        ("\"msg\"c", "\"msg\"", TokenKind::Str, "c"),
    ] {
        assert_eq!(
            tokens(source),
            [(kind, numeral), (TokenKind::Id, suffix)],
            "{source:?}"
        );
    }
}

#[test]
fn numeral_boundaries_preserve_prefix_minus_projection_and_range() {
    assert_eq!(
        tokens("-42; 1.foo; 1..2; _23; _name"),
        [
            (TokenKind::Minus, "-"),
            (TokenKind::Int, "42"),
            (TokenKind::Semi, ";"),
            (TokenKind::Int, "1"),
            (TokenKind::Dot, "."),
            (TokenKind::Id, "foo"),
            (TokenKind::Semi, ";"),
            (TokenKind::Int, "1"),
            (TokenKind::Dot, "."),
            (TokenKind::Dot, "."),
            (TokenKind::Int, "2"),
            (TokenKind::Semi, ";"),
            (TokenKind::Id, "_23"),
            (TokenKind::Semi, ";"),
            (TokenKind::Id, "_name"),
        ]
    );
}

#[test]
fn characters_include_letters_space_delimiters_and_unicode_scalars() {
    for source in [
        "'a'", "'中'", "'🦀'", "' '", "'{'", "'('", "'['", "'<'", "'0'",
    ] {
        assert_literal(source, TokenKind::Char);
    }
}

#[test]
fn all_documented_escapes_are_valid_in_both_literal_kinds() {
    for body in [
        r"\\",
        r"\'",
        r#"\""#,
        r"\n",
        r"\r",
        r"\t",
        r"\0",
        r"\$",
        r"\x{0}",
        r"\x{000000}",
        r"\x{3A3}",
        r"\x{d7ff}",
        r"\x{e000}",
        r"\x{10FFFF}",
    ] {
        assert_literal(&format!("'{body}'"), TokenKind::Char);
        assert_literal(&format!("\"{body}\""), TokenKind::Str);
    }
}

#[test]
fn undocumented_and_non_scalar_escapes_are_rejected_in_both_literal_kinds() {
    for body in [
        r"\a",
        r"\b",
        r"\f",
        r"\v",
        r"\u{41}",
        r"\q",
        r"\x41",
        r"\x{}",
        r"\x{1234567}",
        r"\x{D800}",
        r"\x{DFFF}",
        r"\x{110000}",
        r"\x{FFFFFF}",
        r"\x{1_0}",
        r"\x{xyz}",
        r"\x{41",
    ] {
        for delimiter in ['\'', '"'] {
            let source = format!("{delimiter}{body}{delimiter}");
            assert!(
                matches!(first_error(&source), LexError::InvalidEscape { .. }),
                "{source:?}"
            );
        }
    }
}

#[test]
fn empty_multiple_scalar_and_unterminated_characters_are_rejected() {
    assert!(matches!(first_error("''"), LexError::EmptyChar { .. }));
    assert!(matches!(first_error("'''"), LexError::EmptyChar { .. }));
    for source in ["'ab'", "'中a'", "'  '", "'\\nq'"] {
        assert!(
            matches!(first_error(source), LexError::InvalidChar { .. }),
            "{source:?}"
        );
    }
    // A grapheme containing a combining mark is still multiple scalars.
    assert!(!lex::lex("'e\u{301}'", BytePos(100)).2.is_empty());
    assert!(matches!(
        first_error("'\\"),
        LexError::UnterminatedChar { .. }
    ));
}

#[test]
fn char_starts_after_expression_boundaries() {
    let source = "let x = 'a'; return '中'; f(' ', 'x'); ['a', 'b']";
    let actual = tokens(source);
    let characters: Vec<_> = actual
        .into_iter()
        .filter(|(kind, _)| *kind == TokenKind::Char)
        .collect();
    assert_eq!(
        characters,
        [
            (TokenKind::Char, "'a'"),
            (TokenKind::Char, "'中'"),
            (TokenKind::Char, "' '"),
            (TokenKind::Char, "'x'"),
            (TokenKind::Char, "'a'"),
            (TokenKind::Char, "'b'"),
        ]
    );
}

#[test]
fn quote_expression_pattern_and_macro_starts_remain_distinct() {
    for source in ["'(value)", "' (value)", "'[a](value)", "'<value>", "'T"] {
        assert_eq!(tokens(source)[0], (TokenKind::Quote, "'"), "{source:?}");
    }
    assert_literal("'{ nested { body } }", TokenKind::MacroContent);
    assert_eq!(
        tokens("'a"),
        [(TokenKind::Quote, "'"), (TokenKind::Id, "a")]
    );
    assert_eq!(
        tokens("F<'T>"),
        [
            (TokenKind::Id, "F"),
            (TokenKind::Lt, "<"),
            (TokenKind::Quote, "'"),
            (TokenKind::Id, "T"),
            (TokenKind::Gt, ">"),
        ]
    );
}

#[test]
fn take_view_chains_do_not_become_character_literals() {
    for source in [
        "value'a'b",
        "value 'a'b",
        "value {- comment -}'a'b",
        "value -- comment\n'a'b",
    ] {
        let actual: Vec<_> = tokens(source)
            .into_iter()
            .filter(|(kind, _)| *kind != TokenKind::Comment)
            .collect();
        assert_eq!(
            actual,
            [
                (TokenKind::Id, "value"),
                (TokenKind::Quote, "'"),
                (TokenKind::Id, "a"),
                (TokenKind::Quote, "'"),
                (TokenKind::Id, "b"),
            ],
            "{source:?}"
        );
    }
    for source in [
        "'a''type",
        "(value)'a'b",
        "f(value)'a'b",
        "items[0]'a'b",
        "value.*'a'b",
        "value.ref'a'b",
        "value.await'a'b",
        "value?'a'b",
        "value!'a'b",
        "value#'a'b",
        "Value<T>'a'b",
    ] {
        let actual = tokens(source);
        assert_eq!(
            actual
                .iter()
                .filter(|(kind, _)| *kind == TokenKind::Quote)
                .count(),
            if source == "'a''type" { 1 } else { 2 },
            "{source:?}"
        );
    }
}

#[test]
fn error_recovery_preserves_following_tokens_and_byte_positions() {
    for invalid in [r#""\q""#, r"'\x{d800}'", "'ab'"] {
        let source = format!("{invalid}; '中'; 42");
        let (all, _, errors) = lex::lex(&source, BytePos(100));
        assert_eq!(errors.len(), 1, "{source:?}: {errors:?}");
        let actual: Vec<_> = all
            .into_iter()
            .filter(|token| !matches!(token.kind, TokenKind::Sof | TokenKind::Eof))
            .map(|token| (token.kind, &source[token.from..token.to]))
            .collect();
        assert_eq!(
            actual,
            [
                (TokenKind::Semi, ";"),
                (TokenKind::Char, "'中'"),
                (TokenKind::Semi, ";"),
                (TokenKind::Int, "42")
            ]
        );
    }
    let source = "\"中\"; '\\x{D800}'";
    let (_, _, errors) = lex::lex(source, BytePos(100));
    assert_eq!(
        errors[0].start_position() as usize,
        source.find('\\').unwrap()
    );
    assert_eq!(
        errors[0].to_span(BytePos(100)).lo().0,
        100 + source.find('\\').unwrap() as u32
    );
}

#[test]
fn interpolation_is_kept_in_the_string_for_later_lowering() {
    for source in [
        r#""$name""#,
        r#""${value}""#,
        r#""{ordinary}""#,
        r#""\$name""#,
    ] {
        assert_literal(source, TokenKind::Str);
    }
}

#[test]
fn invalid_unicode_escape_character_has_a_complete_byte_span() {
    let error = first_error("\"\\中\"");
    let span = error.to_span(BytePos(100));
    assert_eq!((span.lo().0, span.hi().0), (101, 105));
}

#[test]
fn complete_characters_after_braces_remain_available_to_statement_parsing() {
    for source in [
        "fn f() { if true {} 'a'; }",
        "fn f() { {} 'a'tag; }",
        "fn f() { if true {} ' '; }",
        "fn f() { if true {} '\\n'; }",
        "fn f() { if true {} '\\x{4E2D}'tag; }",
        "fn f() { if true {} {- gap -} '中'tag; }",
    ] {
        let actual = tokens(source);
        assert_eq!(
            actual
                .iter()
                .filter(|(kind, _)| *kind == TokenKind::Char)
                .count(),
            1,
            "{source:?}"
        );
    }
    // In expression position the parser reinterprets Char("'a'") + Id("b")
    // as two views, using the raw spelling and each identifier's byte span.
    assert_eq!(
        tokens("{}'a'b"),
        [
            (TokenKind::LBrace, "{"),
            (TokenKind::RBrace, "}"),
            (TokenKind::Char, "'a'"),
            (TokenKind::Id, "b"),
        ]
    );
    assert_eq!(
        tokens("{}'foo'bar"),
        [
            (TokenKind::LBrace, "{"),
            (TokenKind::RBrace, "}"),
            (TokenKind::Quote, "'"),
            (TokenKind::Id, "foo"),
            (TokenKind::Quote, "'"),
            (TokenKind::Id, "bar"),
        ]
    );
    assert_eq!(
        tokens("{}'type")[2..],
        [(TokenKind::Quote, "'"), (TokenKind::Id, "type")]
    );
}

#[test]
fn braced_interpolation_preserves_nested_lexical_structures_in_one_string() {
    for source in [
        r#""${"x"}""#,
        r#""before ${ { key: "}" } } after""#,
        r#""${ "${"nested"}" }""#,
        r#""${'}'}""#,
        r#""${'\x{7D}'}""#,
        r#""${value'type}""#,
        r#""${value {- } " ' { -} }""#,
        r#""${value {- outer {- " } ' -} " } ' tail -} }""#,
        "\"${value -- } \" ' {\n}\"",
        r#""${"escaped \" }"}""#,
        r#""${'\''}""#,
        r#""${} and ${1}""#,
    ] {
        assert_literal(source, TokenKind::Str);
        let suffixed = format!("{source}text");
        assert_eq!(
            tokens(&suffixed),
            [(TokenKind::Str, source), (TokenKind::Id, "text")]
        );
    }
}

#[test]
fn nested_comments_preserve_the_previous_significant_token() {
    let comment = "{- outer {- } ' inner -} \" tail -}";
    assert_literal(comment, TokenKind::Comment);
    let source = format!("value {comment}'a'b");
    let actual: Vec<_> = tokens(&source)
        .into_iter()
        .filter(|(kind, _)| *kind != TokenKind::Comment)
        .collect();
    assert_eq!(
        actual,
        [
            (TokenKind::Id, "value"),
            (TokenKind::Quote, "'"),
            (TokenKind::Id, "a"),
            (TokenKind::Quote, "'"),
            (TokenKind::Id, "b"),
        ]
    );
    assert!(matches!(
        first_error("{- outer {- inner -}"),
        LexError::UnterminatedComment { .. }
    ));
}

#[test]
fn interpolation_does_not_consume_following_tokens_or_activate_escaped_dollars() {
    let source = r#""${"中"}"; 'a'; "\${literal}""#;
    assert_eq!(
        tokens(source),
        [
            (TokenKind::Str, r#""${"中"}""#),
            (TokenKind::Semi, ";"),
            (TokenKind::Char, "'a'"),
            (TokenKind::Semi, ";"),
            (TokenKind::Str, r#""\${literal}""#),
        ]
    );
    for source in [r#""${value""#, r#""${ { value }""#] {
        assert!(
            matches!(first_error(source), LexError::UnterminatedString { .. }),
            "{source:?}"
        );
    }
}

#[test]
fn bit_pattern_radix_prefixes_are_distinct_from_integer_values() {
    for (source, kind) in [
        ("0b", TokenKind::BitBinPrefix),
        ("0o", TokenKind::BitOctPrefix),
        ("0x", TokenKind::BitHexPrefix),
    ] {
        assert_literal(source, kind);
    }
}

#[test]
fn extern_is_an_ordinary_identifier() {
    assert_eq!(tokens("extern"), [(TokenKind::Id, "extern")]);
    assert_eq!(tokens("extern_name"), [(TokenKind::Id, "extern_name")]);
}
