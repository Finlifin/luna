use diagnostic::DiagnosticContext;
use lex::TokenKind;
use macro_expander::peg::{
    ParsedPegNode, ParsedPegTree, PegArgValue, PegExpr, PegParamShape, PegRule,
    parse_and_validate_peg_grammar_str,
};
use macro_expander::provenance::{ExpansionKind, MacroToken, TokenSpacing};
use macro_expander::registry::{
    AstNodeKind, ExpansionContext, MacroDescriptor, MacroExecutor, MacroId, MacroInputMode,
    MacroRegistry, MacroVisibility,
};
use macro_expander::{
    AstArgValue, ExpandedFile, MacroBudget, MacroError, MacroErrorKind, expand_source_file,
    expand_source_file_with_context, scan_referenced_macro_roots,
};
use parse::parser::Parser;
use rustc_span::source_map::FilePathMapping;
use rustc_span::{DUMMY_SP, FileName, SourceMap, Span};
use std::collections::BTreeMap;

fn expand_builtins(src: &str) -> Result<ExpandedFile, MacroError> {
    let sm = SourceMap::new(FilePathMapping::empty());
    let sf = sm.new_source_file(FileName::Custom("peg_macro_def.fl".into()), src.to_string());
    let mut budget = MacroBudget::default();
    let (expanded, lex_errors) = expand_source_file(&sm, &sf, &mut budget)?;
    assert!(
        lex_errors.is_empty(),
        "unexpected lex errors: {lex_errors:?}"
    );
    Ok(expanded)
}

fn parse_expanded_clean(expanded: &ExpandedFile) {
    let src = expanded
        .source_text
        .as_deref()
        .expect("expected materialized source text");
    let sm = SourceMap::new(FilePathMapping::empty());
    let sf = sm.new_source_file(FileName::Custom("expanded_peg.fl".into()), src.to_string());
    let (tokens, symbols, lex_errs) = lex::lex(src, sf.start_pos);
    assert!(
        lex_errs.is_empty(),
        "lex errors on lowered output `{src}`: {lex_errs:?}"
    );
    let diag_ctx = DiagnosticContext::new(&sm);
    let mut parser = Parser::new(&sm, tokens, symbols, sf.start_pos);
    assert!(
        parser.parse(&diag_ctx),
        "parse failed on lowered output:\n{src}"
    );
    let ast = parser.finalize();
    ast.validate_structure().expect("valid lowered AST");
}

fn expand_with_ctx(
    src: &str,
    ctx: &mut ExpansionContext<'_>,
    budget: &mut MacroBudget,
) -> Result<ExpandedFile, MacroError> {
    let sm = SourceMap::new(FilePathMapping::empty());
    let sf = sm.new_source_file(
        FileName::Custom("peg_macro_call.fl".into()),
        src.to_string(),
    );
    let (expanded, lex_errors) = expand_source_file_with_context(&sm, &sf, ctx, budget)?;
    assert!(
        lex_errors.is_empty(),
        "unexpected lex errors: {lex_errors:?}"
    );
    Ok(expanded)
}

#[derive(Default)]
struct RecordingPegExecutor {
    calls: Vec<(String, PegArgValue)>,
    return_map: BTreeMap<String, Vec<MacroToken>>,
}

impl MacroExecutor for RecordingPegExecutor {
    fn execute_raw_source(
        &mut self,
        descriptor: &MacroDescriptor,
        _raw_source: &str,
        call_site: Span,
        _budget: &mut MacroBudget,
    ) -> Result<Vec<MacroToken>, MacroError> {
        Err(MacroError::new(
            MacroErrorKind::HandlerExecutionError,
            format!("unexpected raw macro `{}`", descriptor.full_path()),
            call_site,
        ))
    }

    fn execute_tokens(
        &mut self,
        descriptor: &MacroDescriptor,
        _input_tokens: &[MacroToken],
        call_site: Span,
        _budget: &mut MacroBudget,
    ) -> Result<Vec<MacroToken>, MacroError> {
        Err(MacroError::new(
            MacroErrorKind::HandlerExecutionError,
            format!("unexpected token macro `{}`", descriptor.full_path()),
            call_site,
        ))
    }

    fn execute_ast(
        &mut self,
        descriptor: &MacroDescriptor,
        _args: &[AstArgValue],
        call_site: Span,
        _budget: &mut MacroBudget,
    ) -> Result<Vec<MacroToken>, MacroError> {
        Err(MacroError::new(
            MacroErrorKind::HandlerExecutionError,
            format!("unexpected AST macro `{}`", descriptor.full_path()),
            call_site,
        ))
    }

    fn execute_peg(
        &mut self,
        descriptor: &MacroDescriptor,
        arg: &PegArgValue,
        call_site: Span,
        _budget: &mut MacroBudget,
    ) -> Result<Vec<MacroToken>, MacroError> {
        let name = descriptor
            .canonical_path
            .last()
            .cloned()
            .unwrap_or_default();
        self.calls.push((name.clone(), arg.clone()));

        if name == "infinite" {
            return Ok(vec![
                MacroToken::new(TokenKind::At, "@", call_site, TokenSpacing::Joint, None),
                MacroToken::new(
                    TokenKind::Id,
                    "scripts",
                    call_site,
                    TokenSpacing::Joint,
                    None,
                ),
                MacroToken::new(TokenKind::Dot, ".", call_site, TokenSpacing::Joint, None),
                MacroToken::new(
                    TokenKind::Id,
                    "infinite",
                    call_site,
                    TokenSpacing::Alone,
                    None,
                ),
                MacroToken::new(TokenKind::LBrace, "{", call_site, TokenSpacing::Alone, None),
                MacroToken::new(TokenKind::Id, "x", call_site, TokenSpacing::Alone, None),
                MacroToken::new(TokenKind::RBrace, "}", call_site, TokenSpacing::Alone, None),
            ]);
        }

        if let Some(tokens) = self.return_map.get(&name) {
            return Ok(tokens.clone());
        }

        Ok(vec![MacroToken::new(
            TokenKind::Int,
            "0",
            call_site,
            TokenSpacing::Alone,
            None,
        )])
    }
}

fn register_peg_macro(
    reg: &mut MacroRegistry,
    local: u32,
    name: &str,
    grammar_src: &str,
    param_shape: PegParamShape,
) {
    let grammar = parse_and_validate_peg_grammar_str(grammar_src).expect("valid test grammar");
    assert!(
        grammar.supports_param_shape(param_shape),
        "grammar `{grammar_src}` does not support {param_shape}"
    );
    reg.register(MacroDescriptor {
        id: MacroId { package: 10, local },
        canonical_path: vec!["scripts".into(), name.into()],
        package: 10,
        module_path: vec!["scripts".into()],
        vis: MacroVisibility::Public,
        def_site: None,
        mode: MacroInputMode::Peg {
            grammar,
            param_shape,
        },
    });
}

#[test]
fn define_macro2_lowers_official_and_multi_rule_forms_and_roundtrips() {
    let refs = scan_referenced_macro_roots(
        r#"
        @use meta.macros.*;
        const statements = @define_macro2 {
            Entry -> FlurryExtendedStatement*;
            fn(body: Repetition<FlurryExtendedStatement>) -> TokenBuffer { {{ $body }} }
        };
        "#,
    );
    assert!(!refs.has_non_builtin_macros);

    let refs_qual = scan_referenced_macro_roots(
        r#"
        const s = @meta.macros.define_macro2 {
            Entry -> FlurryExpr;
            fn(e: FlurryExpr) -> TokenBuffer { {{ $e }} }
        }
        "#,
    );
    assert!(!refs_qual.has_non_builtin_macros);

    let src = r#"
@use meta.macros.*;

const statements = @define_macro2 {
    Entry -> FlurryExtendedStatement*;

    fn(body: Repetition<meta.ast.FlurryExtendedStatement>) -> TokenBuffer {
        {{ $body }}
    }
}

pub const dsl = @meta.macros.define_macro2 {
    Entry -> Step+;
    Step -> ("inc" ~ FlurryIdent ~ ";") | ("set" ~ FlurryIdent ~ "=" ~ FlurryExpr ~ .semi);

    fn(tree: PegNode) -> TokenBuffer {
        tree.to_tokens()
    }
};

private const opt_expr = @define_macro2 {
    Entry -> (&"val" ~ "val" ~ FlurryExpr) | (!"val" ~ FlurryIdent);

    fn(node: meta.ast.PegNode) -> meta.ast.TokenBuffer {
        node.to_tokens()
    }
}
"#;
    let expanded = expand_builtins(src).expect("valid @define_macro2 forms");
    assert!(expanded.expanded);
    let out = expanded.source_text.as_deref().unwrap();
    assert!(out.contains("attrs.macros.peg("), "{out}");
    assert!(out.contains("comptime fn statements"), "{out}");
    assert!(out.contains("pub comptime fn dsl"), "{out}");
    assert!(out.contains("private comptime fn opt_expr"), "{out}");
    assert!(
        out.contains("meta.ast.Repetition<meta.ast.FlurryExtendedStatement>"),
        "{out}"
    );
    assert!(out.contains("tree: meta.ast.PegNode"), "{out}");
    assert!(!out.contains("meta.ast.meta"), "{out}");
    assert!(
        expanded
            .provenance
            .records()
            .iter()
            .all(|r| r.kind == ExpansionKind::ConstructionPeg)
    );
    parse_expanded_clean(&expanded);

    // Canonical grammar round-trips cleanly through `to_source`.
    let g = parse_and_validate_peg_grammar_str(
        r#"
        Entry -> "(" ~ Entry ~ ")" | Step*;
        Step -> ("inc" ~ FlurryIdent ~ .semi) | (!"end" ~ .id ~ FlurryExpr);
        "#,
    )
    .unwrap();
    let canonical = g.to_source();
    let reparsed = parse_and_validate_peg_grammar_str(&canonical).unwrap();
    assert_eq!(g.rules.len(), reparsed.rules.len());
    for (a, b) in g.rules.iter().zip(reparsed.rules.iter()) {
        assert_eq!(a.name, b.name);
        assert_eq!(a.expr, b.expr);
    }
}

#[test]
fn define_macro2_static_validator_rejects_invalid_and_left_recursive_grammars() {
    let bad_defs = [
        // Missing Entry
        r#"@use meta.macros.*; const m = @define_macro2 { Step -> FlurryIdent; fn(n: PegNode) -> TokenBuffer { {{ 1 }} } };"#,
        // Duplicate rule
        r#"@use meta.macros.*; const m = @define_macro2 { Entry -> "a"; Entry -> "b"; fn(n: PegNode) -> TokenBuffer { {{ 1 }} } };"#,
        // Collides with built-in Flurry AST nonterminal
        r#"@use meta.macros.*; const m = @define_macro2 { Entry -> FlurryExpr; FlurryExpr -> "x"; fn(n: PegNode) -> TokenBuffer { {{ 1 }} } };"#,
        // Undefined rule ref
        r#"@use meta.macros.*; const m = @define_macro2 { Entry -> Missing; fn(n: PegNode) -> TokenBuffer { {{ 1 }} } };"#,
        // Direct left recursion
        r#"@use meta.macros.*; const m = @define_macro2 { Entry -> Entry | "x"; fn(n: PegNode) -> TokenBuffer { {{ 1 }} } };"#,
        // Indirect left recursion
        r#"@use meta.macros.*; const m = @define_macro2 { Entry -> A; A -> B; B -> Entry | "z"; fn(n: PegNode) -> TokenBuffer { {{ 1 }} } };"#,
        // Nullable-prefixed left recursion (`Opt` is optional)
        r#"@use meta.macros.*; const m = @define_macro2 { Entry -> Opt ~ Entry | "z"; Opt -> "a"?; fn(n: PegNode) -> TokenBuffer { {{ 1 }} } };"#,
        // ZeroOrMore-prefixed left recursion
        r#"@use meta.macros.*; const m = @define_macro2 { Entry -> "a"* ~ Entry | "z"; fn(n: PegNode) -> TokenBuffer { {{ 1 }} } };"#,
        // Empty string literal matcher
        r#"@use meta.macros.*; const m = @define_macro2 { Entry -> ""; fn(n: PegNode) -> TokenBuffer { {{ 1 }} } };"#,
        // Unknown token kind symbol
        r#"@use meta.macros.*; const m = @define_macro2 { Entry -> .nonexistent_tok; fn(n: PegNode) -> TokenBuffer { {{ 1 }} } };"#,
        // Zero parameters in handler
        r#"@use meta.macros.*; const m = @define_macro2 { Entry -> "x"; fn() -> TokenBuffer { {{ 1 }} } };"#,
        // Two parameters in handler
        r#"@use meta.macros.*; const m = @define_macro2 { Entry -> "x"; fn(a: PegNode, b: PegNode) -> TokenBuffer { {{ 1 }} } };"#,
        // Variadic `...` in @define_macro2
        r#"@use meta.macros.*; const m = @define_macro2 { Entry -> FlurryExpr*; fn(...a: Repetition<FlurryExpr>) -> TokenBuffer { {{ 1 }} } };"#,
        // Bare `@define_macro2` outside `const`
        r#"@use meta.macros.*; const X = 1; @define_macro2 { Entry -> "x"; fn(n: PegNode) -> TokenBuffer { {{ 1 }} } }"#,
    ];
    for (idx, src) in bad_defs.iter().enumerate() {
        let err = expand_builtins(src).unwrap_err();
        assert_eq!(
            err.kind,
            MacroErrorKind::PegGrammarError,
            "case #{idx} wrong kind: {err}"
        );
        assert_eq!(err.kind.error_code(), 4013);
    }

    // Without `@use meta.macros.*`
    let err_no_import = expand_builtins(
        r#"const m = @define_macro2 { Entry -> "x"; fn(n: PegNode) -> TokenBuffer { {{ 1 }} } };"#,
    )
    .unwrap_err();
    assert_eq!(err_no_import.kind, MacroErrorKind::UndefinedMacro);

    // Wrong delimiter on `@define_macro2`
    let err_delim = expand_builtins(
        r#"@use meta.macros.*; const m = @define_macro2 ( Entry -> "x"; fn(n: PegNode) -> TokenBuffer { {{ 1 }} } );"#,
    )
    .unwrap_err();
    assert_eq!(err_delim.kind, MacroErrorKind::DelimiterMismatch);
    assert_eq!(err_delim.kind.error_code(), 4011);

    // Guarded recursion (`"(" ~ Entry ~ ")"` and `"+"+ ~ Entry`) MUST be allowed!
    assert!(
        parse_and_validate_peg_grammar_str(
            r#"
            Entry -> "(" ~ Entry ~ ")" | "a"+ ~ Entry | "x";
            "#,
        )
        .is_ok()
    );
}

#[test]
fn mode2_packrat_evaluates_choice_lookahead_nullable_repetition_and_projections() {
    let mut reg = MacroRegistry::with_builtins();
    register_peg_macro(
        &mut reg,
        1,
        "statements",
        "Entry -> FlurryExtendedStatement*;",
        PegParamShape::Repetition(AstNodeKind::FlurryExtendedStatement),
    );
    register_peg_macro(
        &mut reg,
        2,
        "dsl",
        r#"
        Entry -> Step*;
        Step -> ("inc" ~ FlurryIdent ~ ";") | ("set" ~ FlurryIdent ~ "=" ~ FlurryExpr ~ ";");
        "#,
        PegParamShape::PegNode,
    );
    register_peg_macro(
        &mut reg,
        3,
        "ordered",
        r#"
        Entry -> (&"special" ~ "special" ~ .id) | ("special" ~ FlurryLiteral) | (!"special" ~ FlurryIdent ~ FlurryLiteral);
        "#,
        PegParamShape::PegNode,
    );
    register_peg_macro(
        &mut reg,
        4,
        "opt_expr",
        "Entry -> FlurryExpr?;",
        PegParamShape::Optional(AstNodeKind::FlurryExpr),
    );
    register_peg_macro(
        &mut reg,
        5,
        "one_expr",
        "Entry -> FlurryExpr;",
        PegParamShape::Single(AstNodeKind::FlurryExpr),
    );
    register_peg_macro(
        &mut reg,
        6,
        "nullable_star",
        r#"
        Entry -> ("opt"?)* ~ FlurryIdent;
        "#,
        PegParamShape::PegNode,
    );
    // Exponential backtracking without packrat memoization: A -> "a" ~ A ~ "b" | "a" ~ A ~ "c" | "x"
    register_peg_macro(
        &mut reg,
        7,
        "packrat",
        r#"
        Entry -> ("a" ~ Entry ~ "b") | ("a" ~ Entry ~ "c") | "x";
        "#,
        PegParamShape::PegNode,
    );

    let mut exec = RecordingPegExecutor::default();
    let mut ctx = ExpansionContext {
        current_package: 0,
        current_module_path: vec![],
        registry: &reg,
        executor: Some(&mut exec),
    };
    let mut budget = MacroBudget::default();

    let src = r#"
@use scripts.*;
const S0 = @statements {};
const S2 = @statements { let a = 1; fn f() -> i32 { 2 } };
const D = @dsl { inc x; set y = 1 + 2; };
const P1 = @ordered { special alpha };
const P2 = @ordered { special 42 };
const P3 = @ordered { normal 99 };
const O0 = @opt_expr {};
const O1 = @opt_expr { 10 + 20 };
const ONE = @one_expr { 1 + 2 };
const N = @nullable_star { opt foo };
const PK = @packrat { a a a a a a a a a a a a x c c c c c c c c c c c c };
"#;
    let expanded = expand_with_ctx(src, &mut ctx, &mut budget).expect("all Mode 2 calls pass");
    assert!(expanded.expanded);
    assert_eq!(exec.calls.len(), 11);

    // 1. Repetition<FlurryExtendedStatement> empty vs 2 statements
    assert!(matches!(&exec.calls[0].1, PegArgValue::Repetition(v) if v.is_empty()));
    let PegArgValue::Repetition(s2) = &exec.calls[1].1 else {
        panic!("expected Repetition");
    };
    assert_eq!(s2.len(), 2);
    assert_eq!(s2[0].nodes[s2[0].root].kind, "let_decl");
    assert_eq!(s2[1].nodes[s2[1].root].kind, "function");

    // 2. PegNode tree for `@dsl { inc x; set y = 1 + 2; }`
    let PegArgValue::PegNode(dsl_tree) = &exec.calls[2].1 else {
        panic!("expected PegNode");
    };
    let root = &dsl_tree.nodes[dsl_tree.root];
    assert_eq!(root.rule, "Entry");
    assert_eq!(root.children.len(), 2);
    let step0 = &dsl_tree.nodes[root.children[0]];
    assert_eq!(step0.rule, "Step");
    assert_eq!(step0.text, "inc x;");
    assert_eq!(step0.tokens.len(), 3);
    assert_eq!(step0.children.len(), 3);
    assert_eq!(dsl_tree.nodes[step0.children[1]].rule, "FlurryIdent");
    assert_eq!(dsl_tree.nodes[step0.children[1]].text, "x");
    let step1 = &dsl_tree.nodes[root.children[1]];
    assert_eq!(step1.rule, "Step");
    assert_eq!(dsl_tree.nodes[step1.children[1]].text, "y");
    assert_eq!(dsl_tree.nodes[step1.children[3]].rule, "FlurryExpr");
    assert_eq!(dsl_tree.nodes[step1.children[3]].text, "1 + 2");

    // 3. Ordered choice + lookahead predicates (& / !)
    assert!(matches!(&exec.calls[3].1, PegArgValue::PegNode(_)));
    assert!(matches!(&exec.calls[4].1, PegArgValue::PegNode(_)));
    assert!(matches!(&exec.calls[5].1, PegArgValue::PegNode(_)));

    // 4. Optional & Single projections
    assert!(matches!(&exec.calls[6].1, PegArgValue::Optional(None)));
    assert!(matches!(&exec.calls[7].1, PegArgValue::Optional(Some(_))));
    assert!(matches!(&exec.calls[8].1, PegArgValue::Single(_)));

    // 5. Nullable repetition terminated cleanly
    assert!(matches!(&exec.calls[9].1, PegArgValue::PegNode(_)));

    // 6. Deep packrat memoization succeeded quickly
    assert!(matches!(&exec.calls[10].1, PegArgValue::PegNode(_)));

    // 7. S-expression Display checks
    let grammar_sexp = parse_and_validate_peg_grammar_str("Entry -> \"inc\" ~ .id?;")
        .unwrap()
        .to_string();
    assert!(
        grammar_sexp.contains("(peg-rule (name \"Entry\") (seq (lit \"inc\") (opt (token .id))))"),
        "{grammar_sexp}"
    );
    let peg_node_sexp = ParsedPegNode {
        rule: "Entry".into(),
        span: DUMMY_SP,
        text: "x".into(),
        tokens: Vec::new(),
        children: vec![1],
    }
    .to_string();
    assert_eq!(
        peg_node_sexp,
        "(peg-node (rule \"Entry\") (span 0 0) (tokens 0) (children 1) (text \"x\"))"
    );
    let peg_tree_sexp = ParsedPegTree {
        nodes: vec![],
        root: 0,
    }
    .to_string();
    assert_eq!(peg_tree_sexp, "(peg-tree (root 0) (nodes))");
    let shape_sexp = PegParamShape::Optional(AstNodeKind::FlurryExpr).to_string();
    assert_eq!(shape_sexp, "(optional FlurryExpr)");
    let _expr_ref = PegExpr::OneOrMore(Box::new(PegExpr::RuleRef("Step".into()))).to_string();
    let _rule_ref = PegRule {
        name: "Step".into(),
        expr: PegExpr::FlurryNode(AstNodeKind::FlurryIdent),
        span: DUMMY_SP,
    }
    .to_string();
}

#[test]
fn mode2_enforces_delimiters_trailing_tokens_depth_steps_and_memo_budgets() {
    let mut reg = MacroRegistry::with_builtins();
    register_peg_macro(
        &mut reg,
        1,
        "dsl",
        r#"
        Entry -> "set" ~ FlurryIdent ~ "=" ~ FlurryExpr ~ ";";
        "#,
        PegParamShape::PegNode,
    );
    register_peg_macro(
        &mut reg,
        2,
        "infinite",
        "Entry -> FlurryIdent;",
        PegParamShape::PegNode,
    );

    let mut exec = RecordingPegExecutor::default();
    let mut ctx = ExpansionContext {
        current_package: 0,
        current_module_path: vec![],
        registry: &reg,
        executor: Some(&mut exec),
    };

    let mut b = MacroBudget::default();
    let err_delim = expand_with_ctx("@scripts.dsl(set x = 1;)", &mut ctx, &mut b).unwrap_err();
    assert_eq!(err_delim.kind, MacroErrorKind::DelimiterMismatch);
    assert_eq!(err_delim.kind.error_code(), 4011);

    let err_unterm = expand_with_ctx("@scripts.dsl { set x = 1;", &mut ctx, &mut b).unwrap_err();
    assert_eq!(err_unterm.kind, MacroErrorKind::UnterminatedMacroInput);

    let err_nomatch = expand_with_ctx("@scripts.dsl { inc x; }", &mut ctx, &mut b).unwrap_err();
    assert_eq!(err_nomatch.kind, MacroErrorKind::PegInputParseError);
    assert_eq!(err_nomatch.kind.error_code(), 4014);

    let err_trailing =
        expand_with_ctx("@scripts.dsl { set x = 1; extra }", &mut ctx, &mut b).unwrap_err();
    assert_eq!(err_trailing.kind, MacroErrorKind::PegInputParseError);
    assert_eq!(err_trailing.kind.error_code(), 4014);

    let mut memo_budget = MacroBudget {
        max_memo_entries: 1,
        ..Default::default()
    };
    let err_memo =
        expand_with_ctx("@scripts.dsl { set x = 1; }", &mut ctx, &mut memo_budget).unwrap_err();
    assert_eq!(err_memo.kind, MacroErrorKind::BudgetExhausted);
    assert_eq!(err_memo.kind.error_code(), 4006);

    let mut step_budget = MacroBudget {
        max_steps: 2,
        ..Default::default()
    };
    let err_steps =
        expand_with_ctx("@scripts.dsl { set x = 1; }", &mut ctx, &mut step_budget).unwrap_err();
    assert_eq!(err_steps.kind, MacroErrorKind::BudgetExhausted);

    let mut depth_budget = MacroBudget {
        max_depth: 3,
        ..Default::default()
    };
    let err_depth =
        expand_with_ctx("@scripts.infinite { foo }", &mut ctx, &mut depth_budget).unwrap_err();
    assert_eq!(err_depth.kind, MacroErrorKind::DepthExceeded);
    assert!(!err_depth.expansion_chain.is_empty());
}
