use std::fs;

use ast::NodeKind;
use fmt::bridge::{BridgedConstruct, DefineMacroKind, parse_and_bridge};
use fmt::config::{FormatConfig, NewlineStyle, SmallHeuristics};
use fmt::directives::{DirectiveAction, ScopedConfigStack};

#[test]
fn parses_flurryfmt_fl_and_roundtrips_print_config() {
    let sample = r#"
-- Formatter config comment
.max_width 88,
.tab_spaces 2,
.newline_style .crlf,
.use_small_heuristics .max,
.reorder_imports false,
.normalize_comments false,
{- block comment -}
.ignore ["generated/**", "vendor/"]
"#;
    let mut cfg = FormatConfig::default();
    cfg.parse_flurry_config(sample, "flurryfmt.fl").unwrap();
    assert_eq!(cfg.max_width, 88);
    assert_eq!(cfg.tab_spaces, 2);
    assert_eq!(cfg.newline_style, NewlineStyle::CrLf);
    assert_eq!(cfg.use_small_heuristics, SmallHeuristics::Max);
    assert!(!cfg.reorder_imports);
    assert!(!cfg.normalize_comments);
    assert_eq!(cfg.ignore, vec!["generated/**", "vendor/"]);

    let printed = cfg.to_flurry_config_string(false);
    let mut reparsed = FormatConfig::default();
    reparsed
        .parse_flurry_config(&printed, "printed.fl")
        .unwrap();
    assert_eq!(cfg, reparsed);

    // Minimal config on defaults emits empty string.
    assert_eq!(FormatConfig::default().to_flurry_config_string(true), "");
}

#[test]
fn cli_overrides_support_both_rustfmt_and_flurry_syntax() {
    let mut cfg = FormatConfig::default();
    cfg.apply_cli_overrides(r#"max_width=120,newline_style=auto,ignore=["gen/**","tmp.fl"]"#)
        .unwrap();
    assert_eq!(cfg.max_width, 120);
    assert_eq!(cfg.newline_style, NewlineStyle::Auto);
    assert_eq!(cfg.ignore, vec!["gen/**", "tmp.fl"]);

    cfg.apply_cli_overrides(".tab_spaces 2, .use_small_heuristics .off, .reorder_imports false")
        .unwrap();
    assert_eq!(cfg.tab_spaces, 2);
    assert_eq!(cfg.use_small_heuristics, SmallHeuristics::Off);
    assert!(!cfg.reorder_imports);

    assert!(cfg.apply_cli_overrides(".unknown_key 1").is_err());
    assert!(cfg.apply_cli_overrides("max_width=0").is_err());
}

#[test]
fn discovers_config_upward_and_matches_ignore_globs() {
    let unique = format!(
        "flurryfmt_cfg_test_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let root = std::env::temp_dir().join(unique);
    let nested = root.join("src").join("sub");
    fs::create_dir_all(&nested).unwrap();
    fs::write(
        root.join("flurryfmt.fl"),
        ".max_width 92,\n.ignore [\"generated/**\", \"vendor\", \"*.gen.fl\"],\n",
    )
    .unwrap();

    let discovered = FormatConfig::discover(&nested, None).unwrap();
    assert_eq!(discovered.max_width, 92);
    assert!(discovered.is_ignored(&root.join("generated/a/b.fl"), &root));
    assert!(discovered.is_ignored(&root.join("src/vendor/lib.fl"), &root));
    assert!(discovered.is_ignored(&root.join("src/foo.gen.fl"), &root));
    assert!(!discovered.is_ignored(&root.join("src/sub/main.fl"), &root));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn directives_distinguish_standalone_from_inline_strings_and_comments() {
    let src = r#"
--%.skip
fn a() {
    let s = "hello ${
--% .off
    1 + 2}";
    let r = @builtin.str.raw {
--% .off
    };
    {- block
--% .off
    -}
    let x = 1; --% .skip trailing comment, not a directive!
    --% .max_width 120, .preserve
    let y = 2;
    --% .bogus_flag 42
    let z = 3;
}
"#;
    let parsed = parse_and_bridge(src, "directives.fl", false).unwrap();
    // Only 3 standalone `--%` lines: `--%.skip`, `--% .max_width 120, .preserve`, `--% .bogus_flag 42`.
    assert_eq!(parsed.directives.len(), 3);
    assert_eq!(parsed.directives[0].normalized_text, "--% .skip");
    assert_eq!(parsed.directives[0].skip_flag(), Some(true));

    assert_eq!(
        parsed.directives[1].normalized_text,
        "--% .max_width 120, .preserve"
    );
    assert_eq!(parsed.directives[1].preserve_flag(), Some(true));

    assert!(!parsed.directives[2].is_valid);
    assert_eq!(parsed.directives[2].normalized_text, "--% .bogus_flag 42");
    assert_eq!(parsed.warnings.len(), 1);
    assert!(parsed.off_regions.is_empty());

    // Verify scoped config stack behavior.
    let mut stack = ScopedConfigStack::new(FormatConfig::default());
    stack.apply_at_depth(1, &parsed.directives[1].actions);
    assert_eq!(stack.current().max_width, 120);
    stack.pop_above(0);
    assert_eq!(stack.current().max_width, 100);
    assert!(matches!(
        parsed.directives[1].actions[0],
        DirectiveAction::MaxWidth(120)
    ));
}

#[test]
fn unclosed_off_directive_stops_at_enclosing_block_rbrace() {
    let src = r#"fn outer() {
    --% .off
    let   weird  =   1 +   2;
}

fn after() {
    let clean = 3;
}
"#;
    let parsed = parse_and_bridge(src, "off_scope.fl", false).unwrap();
    assert_eq!(parsed.off_regions.len(), 1);
    let region = &parsed.off_regions[0];
    let disabled_slice = &src[region.body_start..region.body_end];
    assert_eq!(disabled_slice, "    let   weird  =   1 +   2;\n");
}

#[test]
fn bridges_macros_predicates_outcomes_and_raw_strings_with_exact_offsets() {
    let src = r#"@use std.{io, mem}

@define_macro1
pub comptime fn make_pair(x: Code) -> Code {
    x
}

define repeat_twice(body) {
    $body;
    $body;
}

comptime predicate NonEmpty(n: usize) -> bool = n > 0;

fn compute(a: i32) -> i32
where
    T <: Copy,
    requires a > 0 ==> a >= 1,
    outcomes: ok [a > 0, a == 1],
{
    let raw = @builtin.str.raw { C:\unparsed\escape "quote" { inner } };
    let buf = {{ arbitrary + tokens }};
    let m = @my.macro(1, 2);
    $repeat_twice {
        a
    }
    a
}
"#;
    let parsed = parse_and_bridge(src, "bridged.fl", false).unwrap();
    assert_eq!(parsed.original_src.len(), parsed.shadow_src.len());
    // Every newline byte offset in shadow_src matches original_src!
    let orig_nl: Vec<usize> = src
        .bytes()
        .enumerate()
        .filter_map(|(i, b)| (b == b'\n').then_some(i))
        .collect();
    let shadow_nl: Vec<usize> = parsed
        .shadow_src
        .bytes()
        .enumerate()
        .filter_map(|(i, b)| (b == b'\n').then_some(i))
        .collect();
    assert_eq!(orig_nl, shadow_nl);

    assert_eq!(parsed.bridge.at_use_offsets.len(), 1);
    assert_eq!(
        parsed
            .bridge
            .define_macro_offsets
            .values()
            .copied()
            .collect::<Vec<_>>(),
        vec![DefineMacroKind::Macro1]
    );
    assert_eq!(parsed.bridge.predicate_kw_offsets.len(), 1);
    assert_eq!(parsed.bridge.outcomes_kw_offsets.len(), 1);
    assert_eq!(parsed.bridge.implies_offsets.len(), 0);
    assert_eq!(parsed.bridge.subtype_offsets.len(), 1);

    let mut has_raw_brace = false;
    let mut has_token_buf = false;
    let mut has_template_def = false;
    for c in parsed.bridge.constructs_by_start.values() {
        match c {
            BridgedConstruct::BraceMacroCall { path, .. } if path == "builtin.str.raw" => {
                has_raw_brace = true;
            }
            BridgedConstruct::TokenBuffer { .. } => has_token_buf = true,
            BridgedConstruct::TemplateMacroDef { .. } => has_template_def = true,
            _ => {}
        }
    }
    assert!(has_raw_brace);
    assert!(has_token_buf);
    assert!(has_template_def);
}

#[test]
fn exact_token_bounds_and_outer_parens_recover_transparent_syntax() {
    let src = "fn first() {}\n\nstruct Second {\n    x: i32,\n}\n\nfn calc() {\n    let v = (1 + 2) * ((3));\n    let t = (4,);\n}\n";
    let parsed = parse_and_bridge(src, "spans.fl", false).unwrap();
    let root_children = parsed.ast.get_children(parsed.ast.root);
    let items = parsed.ast.get_multi_child_slice(root_children[0]).unwrap();
    assert_eq!(items.len(), 3);

    let (lo0, hi0) = parsed.exact_byte_range(items[0], 1);
    assert_eq!(&src[lo0..hi0], "fn first() {}");
    let (_, end_tok0) = parsed.exact_token_bounds(items[0], 1);

    let (lo1, hi1) = parsed.exact_byte_range(items[1], end_tok0 + 1);
    assert_eq!(&src[lo1..hi1], "struct Second {\n    x: i32,\n}");
    let (_, end_tok1) = parsed.exact_token_bounds(items[1], end_tok0 + 1);

    let (lo2, hi2) = parsed.exact_byte_range(items[2], end_tok1 + 1);
    assert!(src[lo2..hi2].starts_with("fn calc()"));

    // Inspect `(1 + 2) * ((3))` inside `let v = ...;`
    let fn_children = parsed.ast.get_children(items[2]);
    let fn_block = fn_children[4];
    let stmts = parsed
        .ast
        .get_multi_child_slice(parsed.ast.get_children(fn_block)[0])
        .unwrap();
    let let_v = stmts[0];
    assert_eq!(parsed.ast.get_node_kind(let_v), Some(NodeKind::LetDecl));
    let (let_f, let_l) = parsed.exact_token_bounds(let_v, 1);
    let eq_tok = parsed
        .find_depth_zero_token(let_f, let_l, |k| k == lex::TokenKind::Eq, false)
        .unwrap();
    let mul = parsed.ast.get_children(let_v)[2];
    assert_eq!(parsed.ast.get_node_kind(mul), Some(NodeKind::Mul));
    let (mul_parens, mul_f, mul_l) = parsed.count_outer_parens(mul, eq_tok + 1, let_l);
    assert_eq!(mul_parens, 0);
    let mul_kids = parsed.ast.get_children(mul);
    let lhs_add = mul_kids[0];
    let rhs_three = mul_kids[1];

    // Find `*` operator token at depth 0 between lhs and rhs.
    let star_tok = parsed
        .find_depth_zero_token(mul_f, mul_l, |k| k == lex::TokenKind::SeparatedStar, true)
        .unwrap();
    let (lhs_parens, _, _) = parsed.count_outer_parens(lhs_add, mul_f, star_tok - 1);
    let (rhs_parens, _, _) = parsed.count_outer_parens(rhs_three, star_tok + 1, mul_l);
    assert_eq!(lhs_parens, 1);
    assert_eq!(rhs_parens, 2);

    // Inspect `(4,)` — single-element Tuple should have 0 extra outer parens!
    let let_t = stmts[1];
    let tup = parsed.ast.get_children(let_t)[2];
    assert_eq!(parsed.ast.get_node_kind(tup), Some(NodeKind::Tuple));
    let (tup_f, tup_l) = parsed.exact_token_bounds(tup, 1);
    let (tup_parens, _, _) = parsed.count_outer_parens(tup, tup_f, tup_l);
    assert_eq!(tup_parens, 0);
}

#[test]
fn off_and_skip_protect_unparseable_code_while_real_syntax_errors_fail() {
    let off_src = "fn valid() {}\n--% .off\n??? completely unparseable wip syntax !!!\n--% .on\nfn still_valid() {}\n";
    assert!(parse_and_bridge(off_src, "off_wip.fl", false).is_ok());

    let skip_src = "fn valid() {\n    --% .skip\n    let ??? = !!!;\n    let ok = 1;\n}\n";
    assert!(parse_and_bridge(skip_src, "skip_wip.fl", false).is_ok());

    let bad_src = "fn broken( {\n    let x = 1;\n}\n";
    assert!(parse_and_bridge(bad_src, "broken.fl", false).is_err());
}
