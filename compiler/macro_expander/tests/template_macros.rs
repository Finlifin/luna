use lex::TokenKind;
use macro_expander::{
    ExpansionKind, MacroBudget, MacroErrorKind, MacroToken, TokenSpacing, expand_source,
};
use rustc_span::source_map::FilePathMapping;
use rustc_span::{FileName, SourceMap};

fn expand(
    source: &str,
    budget: &mut MacroBudget,
) -> Result<macro_expander::ExpandedFile, macro_expander::MacroError> {
    let map = SourceMap::new(FilePathMapping::empty());
    let file = map.new_source_file(FileName::Custom("template_test.fl".into()), source.into());
    let (tokens, symbols, lex_errors) = lex::lex(source, file.start_pos);
    assert!(
        lex_errors.is_empty(),
        "unexpected lex errors: {lex_errors:?}"
    );
    expand_source(&map, &file, tokens, symbols, budget)
}

#[test]
fn fixed_count_template_macro_zero_one_and_two_tokens() {
    let src = r#"
define answer 0 { 42 }
define id 1 { $0 }
define constant_function 2 {
    fn $0() -> i32 {
        $1
    }
}

$constant_function one 1
$constant_function two 2
fn three() -> i32 { $id($answer) }
"#;
    let mut budget = MacroBudget::default();
    let expanded = expand(src, &mut budget).unwrap();
    assert!(expanded.expanded);
    let text = expanded.source_text.as_deref().unwrap();
    assert!(text.contains("fn one() -> i32"), "{text}");
    assert!(text.contains("fn two() -> i32"), "{text}");
    assert!(text.contains("fn three() -> i32"), "{text}");
    assert!(text.contains("42"), "{text}");
    assert!(!text.contains("define"), "{text}");
}

#[test]
fn repeated_template_macro_expands_groups_and_empty_call() {
    let src = r#"
define constants 2 ... {
    const $0 = $1;
}

$constants FIRST 1 SECOND 2 THIRD 3$
$constants $
"#;
    let mut budget = MacroBudget::default();
    let expanded = expand(src, &mut budget).unwrap();
    let text = expanded.source_text.as_deref().unwrap();
    assert!(text.contains("const FIRST = 1;"), "{text}");
    assert!(text.contains("const SECOND = 2;"), "{text}");
    assert!(text.contains("const THIRD = 3;"), "{text}");
}

#[test]
fn named_template_macro_handles_nested_delimiters_zero_args_and_trailing_comma() {
    let src = r#"
define zero() { true }
define pair(left, right) { ($left, $right) }
define call_twice(f, x) { $f($x) + $f($x) }

let a = $zero();
let b = $pair(foo(1, 2), [3, 4],);
let c = $call_twice(compute, { let z = 1; z });
"#;
    let mut budget = MacroBudget::default();
    let expanded = expand(src, &mut budget).unwrap();
    let text = expanded.source_text.as_deref().unwrap();
    assert!(text.contains("let a = true;"), "{text}");
    assert!(text.contains("(foo(1, 2), [3, 4])"), "{text}");
    assert!(
        text.contains("compute({ let z = 1; z }) + compute({ let z = 1; z })"),
        "{text}"
    );
}

#[test]
fn preserves_spacing_suffixes_and_separated_operators() {
    let src = r#"
define add_expr(lhs, rhs) { $lhs + $rhs }
define with_suffix 1 { $0 }

let x = $add_expr(23u32, 10);
let y = $with_suffix 42u64;
"#;
    let mut budget = MacroBudget::default();
    let expanded = expand(src, &mut budget).unwrap();
    let text = expanded.source_text.as_deref().unwrap();
    assert!(text.contains("23u32 + 10"), "{text}");
    assert!(text.contains("42u64"), "{text}");
    assert!(
        expanded
            .tokens
            .iter()
            .any(|t| t.kind == TokenKind::SeparatedPlus)
    );
}

#[test]
fn double_dollar_escape_and_token_buffer_passthrough() {
    let src = r#"
define make_buf(x) {
    {{ $x + $local_var }}
}
let buf1 = {{ $acc or $pattern_ast }};
let buf2 = $make_buf(42);
"#;
    let mut budget = MacroBudget::default();
    let expanded = expand(src, &mut budget).unwrap();
    let text = expanded.source_text.as_deref().unwrap();
    assert!(text.contains("{{ $acc or $pattern_ast }}"), "{text}");
    assert!(text.contains("{{ 42 + $local_var }}"), "{text}");

    // Unexpanded file with only `{{ ... }}` stays on zero-copy fast path.
    let no_macro_src = "let buf = {{ $acc or $pattern_ast }};";
    let fast = expand(no_macro_src, &mut MacroBudget::default()).unwrap();
    assert!(!fast.expanded);
    assert!(fast.provenance.is_empty());
}

#[test]
fn escaped_dollar_and_chained_template_expansion_keep_provenance() {
    let src = r#"
define leaf(x) { $x + 1 }
define mid(y) { $leaf($y) }
define outer(z) { $mid($z) }
define emit_dollar() { $$kept }

let val = $outer(9);
"#;
    let mut budget = MacroBudget::default();
    let expanded = expand(src, &mut budget).unwrap();
    let text = expanded.source_text.as_deref().unwrap();
    assert!(text.contains("let val = 9 + 1;"), "{text}");

    let recs = expanded.provenance.records();
    assert_eq!(recs.len(), 3);
    assert_eq!(recs[0].macro_name, "outer");
    assert_eq!(recs[0].kind, ExpansionKind::Template);
    assert_eq!(recs[1].macro_name, "mid");
    assert_eq!(recs[1].parent, Some(recs[0].id));
    assert_eq!(recs[2].macro_name, "leaf");
    assert_eq!(recs[2].parent, Some(recs[1].id));

    let chain = expanded.provenance.chain(recs[2].id);
    assert_eq!(chain.len(), 3);
    let sexpr = expanded.provenance.to_string();
    assert!(sexpr.starts_with("(expansion-map "), "{sexpr}");
    assert!(sexpr.contains("(macro \"leaf\")"), "{sexpr}");

    let nine_offset = text.find('9').unwrap();
    let hit = expanded.provenance.find_at_offset(nine_offset).unwrap();
    assert_eq!(hit.macro_name, "leaf");
}

#[test]
fn repeated_call_does_not_terminate_on_dollar_inside_token_buffer() {
    let src = r#"
define emit_each 6 ... { $0$1 $2$3 $4$5; }
$emit_each {{ $x }} {{ $y }} $
"#;
    let mut budget = MacroBudget::default();
    let expanded = expand(src, &mut budget).unwrap();
    let text = expanded.source_text.as_deref().unwrap();
    assert!(text.contains("{{ $x }};"), "{text}");
    assert!(text.contains("{{ $y }};"), "{text}");
}

#[test]
fn template_macro_errors_and_sexpr_formatting() {
    let cases = [
        ("$missing(1)", MacroErrorKind::UndefinedTemplateMacro),
        (
            "$later(1)\ndefine later(x) { $x }",
            MacroErrorKind::UndefinedTemplateMacro,
        ),
        ("define bad 1 { $1 }", MacroErrorKind::InvalidTemplateBody),
        ("define bad(x) { $y }", MacroErrorKind::InvalidTemplateBody),
        (
            "define bad(x, x) { $x }",
            MacroErrorKind::InvalidTemplateDefinition,
        ),
        (
            "define bad 0 ... { 1 }",
            MacroErrorKind::InvalidTemplateDefinition,
        ),
        (
            "define bad 2 { $0 + $1 }\n$bad only_one",
            MacroErrorKind::InvalidTemplateCall,
        ),
        (
            "define bad 2 ... { $0 = $1; }\n$bad A 1 B",
            MacroErrorKind::InvalidTemplateCall,
        ),
        (
            "define bad 2 ... { $0 = $1; }\n$bad A 1 B $",
            MacroErrorKind::InvalidTemplateCall,
        ),
        (
            "define bad(a, b) { $a }\n$bad(1)",
            MacroErrorKind::InvalidTemplateCall,
        ),
        (
            "define bad(a, b) { $a }\n$bad(1, , 2)",
            MacroErrorKind::InvalidTemplateCall,
        ),
        (
            "define bad(a) { $a }\n$bad 1",
            MacroErrorKind::InvalidTemplateCall,
        ),
        (
            "define bad(a) { $a }\n$bad(1",
            MacroErrorKind::InvalidTemplateCall,
        ),
    ];

    for (source, expected_kind) in cases {
        let err = expand(source, &mut MacroBudget::default()).unwrap_err();
        assert_eq!(err.kind, expected_kind, "{source} => {err}");
        let sexpr = err.to_string();
        assert!(sexpr.starts_with("(macro-error "), "{sexpr}");
    }
}

#[test]
fn depth_and_budget_limits_report_expansion_chain_without_truncation() {
    let src = r#"
define rec(x) { $$rec($x) }
let a = $rec(1);
"#;
    let mut budget = MacroBudget {
        max_depth: 5,
        ..Default::default()
    };
    let err = expand(src, &mut budget).unwrap_err();
    assert_eq!(err.kind, MacroErrorKind::DepthExceeded);
    assert_eq!(err.expansion_chain.len(), 5, "{err}");

    let huge = r#"
define dup(x) { $x + $x + $x + $x + $x }
let b = $dup(1);
"#;
    let mut tiny_tokens = MacroBudget {
        max_tokens: 4,
        ..Default::default()
    };
    let err2 = expand(huge, &mut tiny_tokens).unwrap_err();
    assert_eq!(err2.kind, MacroErrorKind::BudgetExhausted);
    assert!(!err2.expansion_chain.is_empty(), "{err2}");

    let mt_sexpr = MacroToken::new(
        TokenKind::Id,
        "foo",
        rustc_span::DUMMY_SP,
        TokenSpacing::Alone,
        None,
    )
    .to_string();
    assert!(mt_sexpr.starts_with("(macro-token "), "{mt_sexpr}");
}
