use diagnostic::DiagnosticContext;
use lex::TokenKind;
use macro_expander::ast_input::{AstArgValue, AstParamShape, ParsedSyntaxNode, ParsedSyntaxTree};
use macro_expander::provenance::{ExpansionKind, MacroToken, TokenSpacing};
use macro_expander::registry::{
    AstNodeKind, ExpansionContext, MacroDescriptor, MacroExecutor, MacroId, MacroInputMode,
    MacroRegistry, MacroVisibility,
};
use macro_expander::{
    ExpandedFile, MacroBudget, MacroError, MacroErrorKind, expand_source_file,
    expand_source_file_with_context, scan_referenced_macro_roots,
};
use parse::parser::Parser;
use rustc_span::source_map::FilePathMapping;
use rustc_span::{FileName, SourceMap, Span};
use std::collections::BTreeMap;

fn expand_builtins(src: &str) -> Result<ExpandedFile, MacroError> {
    let sm = SourceMap::new(FilePathMapping::empty());
    let sf = sm.new_source_file(FileName::Custom("ast_macro_def.fl".into()), src.to_string());
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
    let sf = sm.new_source_file(FileName::Custom("expanded.fl".into()), src.to_string());
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
        FileName::Custom("ast_macro_call.fl".into()),
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
struct RecordingAstExecutor {
    calls: Vec<(String, Vec<AstArgValue>)>,
    return_map: BTreeMap<String, Vec<MacroToken>>,
    recurse_forever: bool,
}

impl MacroExecutor for RecordingAstExecutor {
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
        args: &[AstArgValue],
        call_site: Span,
        _budget: &mut MacroBudget,
    ) -> Result<Vec<MacroToken>, MacroError> {
        let name = descriptor
            .canonical_path
            .last()
            .cloned()
            .unwrap_or_default();
        self.calls.push((name.clone(), args.to_vec()));

        if self.recurse_forever || name == "infinite" {
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
                    TokenSpacing::Joint,
                    None,
                ),
                MacroToken::new(TokenKind::LParen, "(", call_site, TokenSpacing::Joint, None),
                MacroToken::new(TokenKind::Int, "1", call_site, TokenSpacing::Joint, None),
                MacroToken::new(TokenKind::RParen, ")", call_site, TokenSpacing::Alone, None),
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

    fn execute_peg(
        &mut self,
        descriptor: &MacroDescriptor,
        _arg: &macro_expander::PegArgValue,
        call_site: Span,
        _budget: &mut MacroBudget,
    ) -> Result<Vec<MacroToken>, MacroError> {
        Err(MacroError::new(
            MacroErrorKind::HandlerExecutionError,
            format!("unexpected PEG macro `{}`", descriptor.full_path()),
            call_site,
        ))
    }
}

fn register_ast_macro(reg: &mut MacroRegistry, local: u32, name: &str, params: Vec<AstParamShape>) {
    reg.register(MacroDescriptor {
        id: MacroId { package: 10, local },
        canonical_path: vec!["scripts".into(), name.into()],
        package: 10,
        module_path: vec!["scripts".into()],
        vis: MacroVisibility::Public,
        def_site: None,
        mode: MacroInputMode::Ast { params },
    });
}

#[test]
fn define_macro1_lowers_variants_and_parses_cleanly() {
    let refs = scan_referenced_macro_roots(
        r#"
        @use meta.macros.*;
        const m = @define_macro1 {
            fn(e: FlurryExpr) -> TokenBuffer { {{ $e }} }
        };
        "#,
    );
    assert!(!refs.has_non_builtin_macros);

    let refs_qual = scan_referenced_macro_roots(
        r#"
        const m = @meta.macros.define_macro1 {
            fn() -> meta.ast.TokenBuffer { {{ 1 }} }
        }
        "#,
    );
    assert!(!refs_qual.has_non_builtin_macros);

    let src = r#"
@use meta.macros.*;

const zero = @define_macro1 {
    fn() -> TokenBuffer {
        {{ 42 }}
    }
}

pub const single = @meta.macros.define_macro1 {
    fn(expr_ast: FlurryExpr) -> meta.ast.TokenBuffer {
        {{ $expr_ast }}
    }
};

private const with_opt = @define_macro1 {
    fn(base: FlurryExpr, extra: Optional<FlurryExpr>) -> TokenBuffer {
        extra match {
            e? => {{ $base + $e }},
            null => {{ $base }},
        }
    }
}

pub const alternatives = @define_macro1 {
    fn(...patterns: Repetition<meta.ast.FlurryPattern>) -> TokenBuffer {
        patterns.fold({{ not _ }}, |acc, p| {
            {{ $acc or $p }}
        })
    }
};
"#;
    let expanded = expand_builtins(src).expect("valid @define_macro1 forms");
    assert!(expanded.expanded);
    let out = expanded.source_text.as_deref().unwrap();
    assert!(out.contains("flurry_macro_kind: .ast"), "{out}");
    assert!(out.contains("flurry_macro_variadic: false"), "{out}");
    assert!(out.contains("flurry_macro_variadic: true"), "{out}");
    assert!(out.contains("comptime fn zero"), "{out}");
    assert!(out.contains("pub comptime fn single"), "{out}");
    assert!(out.contains("private comptime fn with_opt"), "{out}");
    assert!(out.contains("pub comptime fn alternatives"), "{out}");
    assert!(
        out.contains("meta.ast.Optional<meta.ast.FlurryExpr>"),
        "{out}"
    );
    assert!(
        out.contains("meta.ast.Repetition<meta.ast.FlurryPattern>"),
        "{out}"
    );
    // Ensure `meta.ast.TokenBuffer` and `meta.ast.FlurryPattern` were NOT double-qualified.
    assert!(!out.contains("meta.ast.meta"), "{out}");
    assert!(!out.contains("ast.meta"), "{out}");
    assert!(
        expanded
            .provenance
            .records()
            .iter()
            .all(|r| r.kind == ExpansionKind::ConstructionAst)
    );
    parse_expanded_clean(&expanded);
}

#[test]
fn define_macro1_rejects_malformed_definitions_and_misuse() {
    let err_no_import =
        expand_builtins("const m = @define_macro1 { fn() -> TokenBuffer { {{ 1 }} } };")
            .unwrap_err();
    assert_eq!(err_no_import.kind, MacroErrorKind::UndefinedMacro);
    assert_eq!(err_no_import.kind.error_code(), 4008);

    let err_bare = expand_builtins(
        "@use meta.macros.*; const X = 1; @define_macro1 { fn() -> TokenBuffer { {{ 1 }} } }",
    )
    .unwrap_err();
    assert_eq!(err_bare.kind, MacroErrorKind::AstInputParseError);
    assert_eq!(err_bare.kind.error_code(), 4012);

    let err_delim = expand_builtins(
        "@use meta.macros.*; const m = @define_macro1 ( fn() -> TokenBuffer { {{ 1 }} } );",
    )
    .unwrap_err();
    assert_eq!(err_delim.kind, MacroErrorKind::DelimiterMismatch);
    assert_eq!(err_delim.kind.error_code(), 4011);

    let err_non_trailing = expand_builtins(
        "@use meta.macros.*; const m = @define_macro1 { fn(...ps: Repetition<FlurryPattern>, e: FlurryExpr) -> TokenBuffer { {{ 1 }} } };",
    )
    .unwrap_err();
    assert_eq!(err_non_trailing.kind, MacroErrorKind::AstInputParseError);

    let err_missing_fn =
        expand_builtins("@use meta.macros.*; const m = @define_macro1 { let x = 1; };")
            .unwrap_err();
    assert_eq!(err_missing_fn.kind, MacroErrorKind::AstInputParseError);

    let err_named_fn = expand_builtins(
        "@use meta.macros.*; const m = @define_macro1 { fn named() -> TokenBuffer { {{ 1 }} } };",
    )
    .unwrap_err();
    assert_eq!(err_named_fn.kind, MacroErrorKind::AstInputParseError);

    let err_missing_arrow =
        expand_builtins("@use meta.macros.*; const m = @define_macro1 { fn() { {{ 1 }} } };")
            .unwrap_err();
    assert_eq!(err_missing_arrow.kind, MacroErrorKind::AstInputParseError);

    let err_unclosed_body = expand_builtins(
        "@use meta.macros.*; const m = @define_macro1 { fn() -> TokenBuffer { {{ 1 }} };",
    )
    .unwrap_err();
    assert_eq!(
        err_unclosed_body.kind,
        MacroErrorKind::UnterminatedMacroInput
    );

    let err_trailing = expand_builtins(
        "@use meta.macros.*; const m = @define_macro1 { fn() -> TokenBuffer { {{ 1 }} } extra };",
    )
    .unwrap_err();
    assert_eq!(err_trailing.kind, MacroErrorKind::AstInputParseError);
}

#[test]
fn mode1_parses_all_ten_ast_node_kinds_nested_args_optionals_and_variadics() {
    let mut reg = MacroRegistry::with_builtins();
    let all_kinds = [
        (1, "p_pat", AstNodeKind::FlurryPattern),
        (2, "p_expr", AstNodeKind::FlurryExpr),
        (3, "p_stmt", AstNodeKind::FlurryStatement),
        (4, "p_ext", AstNodeKind::FlurryExtendedStatement),
        (5, "p_item", AstNodeKind::FlurryItem),
        (6, "p_ty", AstNodeKind::FlurryType),
        (7, "p_id", AstNodeKind::FlurryIdent),
        (8, "p_lit", AstNodeKind::FlurryLiteral),
        (9, "p_blk", AstNodeKind::FlurryBlock),
        (10, "p_path", AstNodeKind::FlurryPath),
    ];
    for &(local, name, kind) in &all_kinds {
        register_ast_macro(&mut reg, local, name, vec![AstParamShape::Single(kind)]);
    }
    register_ast_macro(
        &mut reg,
        11,
        "alt",
        vec![AstParamShape::Repetition(AstNodeKind::FlurryPattern)],
    );
    register_ast_macro(
        &mut reg,
        12,
        "with_opt",
        vec![
            AstParamShape::Single(AstNodeKind::FlurryExpr),
            AstParamShape::Optional(AstNodeKind::FlurryExpr),
        ],
    );

    let mut exec = RecordingAstExecutor::default();
    let mut ctx = ExpansionContext {
        current_package: 0,
        current_module_path: vec![],
        registry: &reg,
        executor: Some(&mut exec),
    };
    let mut budget = MacroBudget::default();

    let src = r#"
@use scripts.*;
const A = @p_pat((1, 2) or "ok");
const B = @p_expr((1, 2) + foo<List<i32>>(3, 4));
const C = @p_stmt(let v = 10;);
const D = @p_ext(fn inner(x: i32) -> i32 { x + 1 });
const E = @p_item(pub struct Pair { x: i32, y: i32 });
const F = @p_ty(?meta.List<i32>);
const G = @p_id(my_ident);
const H = @p_lit(.ready);
const I = @p_blk({ let z = 1; z });
const J = @p_path(std.io.Reader);
const K0 = @alt();
const K2 = @alt("yes", "Y",);
const O1 = @with_opt(10);
const O2 = @with_opt(10, 20);
"#;
    let expanded = expand_with_ctx(src, &mut ctx, &mut budget).expect("all mode 1 calls succeed");
    assert!(expanded.expanded);
    assert_eq!(exec.calls.len(), 14);

    // Check root node kinds, texts, child indices, and captured tokens across the 10 categories.
    let assert_single_tree = |idx: usize, expected_kind: &str, text_substr: &str| {
        let (_, args) = &exec.calls[idx];
        let AstArgValue::Single(tree) = &args[0] else {
            panic!("expected single AST arg at index {idx}");
        };
        let root_node = &tree.nodes[tree.root];
        assert_eq!(root_node.kind, expected_kind, "call #{idx}");
        assert!(
            root_node.text.contains(text_substr),
            "expected `{}` to contain `{text_substr}` (call #{idx})",
            root_node.text
        );
        assert!(!tree.tokens.is_empty(), "empty tokens on call #{idx}");
        for &child in &root_node.children {
            assert!(child < tree.nodes.len());
        }
        tree.clone()
    };

    let pat_tree = assert_single_tree(0, "or_pattern", "\"ok\"");
    assert!(pat_tree.nodes.len() >= 3);
    assert!(!pat_tree.nodes[pat_tree.root].children.is_empty());

    assert_single_tree(1, "add", "foo");
    assert_single_tree(2, "let_decl", "let v");
    assert_single_tree(3, "function", "inner");
    assert_single_tree(4, "pub", "Pair");
    assert_single_tree(5, "optional_type", "List");
    assert_single_tree(6, "id", "my_ident");
    assert_single_tree(7, "symbol", ".ready");
    assert_single_tree(8, "block", "let z");
    assert_single_tree(9, "projection_path", "std");

    // Empty variadic and trailing comma variadic
    let AstArgValue::Repetition(k0_items) = &exec.calls[10].1[0] else {
        panic!("expected repetition arg");
    };
    assert!(k0_items.is_empty());

    let AstArgValue::Repetition(k2_items) = &exec.calls[11].1[0] else {
        panic!("expected repetition arg");
    };
    assert_eq!(k2_items.len(), 2);
    assert_eq!(k2_items[0].nodes[k2_items[0].root].text, "\"yes\"");
    assert_eq!(k2_items[1].nodes[k2_items[1].root].text, "\"Y\"");

    // Optional omitted vs present
    assert!(matches!(exec.calls[12].1[1], AstArgValue::Optional(None)));
    assert!(matches!(
        exec.calls[13].1[1],
        AstArgValue::Optional(Some(_))
    ));

    // Verify S-expression Display implementations
    let param_sexp = AstParamShape::Repetition(AstNodeKind::FlurryPattern).to_string();
    assert_eq!(param_sexp, "(repetition FlurryPattern)");
    let mode_sexp = MacroInputMode::Ast {
        params: vec![
            AstParamShape::Single(AstNodeKind::FlurryExpr),
            AstParamShape::Optional(AstNodeKind::FlurryType),
        ],
    }
    .to_string();
    assert_eq!(mode_sexp, "(ast (single FlurryExpr) (optional FlurryType))");
    let tree_sexp = pat_tree.to_string();
    assert!(tree_sexp.starts_with("(syntax-tree (root 0) (nodes (syntax-node"));
    let node_sexp = ParsedSyntaxNode {
        kind: "id".into(),
        span: rustc_span::DUMMY_SP,
        children: vec![1, 2],
        text: "hi".into(),
    }
    .to_string();
    assert_eq!(
        node_sexp,
        "(syntax-node (kind \"id\") (span 0 0) (children 1 2) (text \"hi\"))"
    );
    let arg_sexp = AstArgValue::Optional(None).to_string();
    assert_eq!(arg_sexp, "(arg-optional null)");
    let _tree_type: ParsedSyntaxTree = pat_tree;
}

#[test]
fn mode1_reports_delimiters_arity_parse_errors_and_budgets() {
    let mut reg = MacroRegistry::with_builtins();
    register_ast_macro(
        &mut reg,
        1,
        "pair",
        vec![
            AstParamShape::Single(AstNodeKind::FlurryExpr),
            AstParamShape::Single(AstNodeKind::FlurryExpr),
        ],
    );
    register_ast_macro(
        &mut reg,
        2,
        "only_id",
        vec![AstParamShape::Single(AstNodeKind::FlurryIdent)],
    );
    register_ast_macro(
        &mut reg,
        3,
        "infinite",
        vec![AstParamShape::Single(AstNodeKind::FlurryExpr)],
    );

    let mut exec = RecordingAstExecutor::default();
    let mut ctx = ExpansionContext {
        current_package: 0,
        current_module_path: vec![],
        registry: &reg,
        executor: Some(&mut exec),
    };

    let mut b = MacroBudget::default();
    let err_delim = expand_with_ctx("@scripts.pair { 1, 2 }", &mut ctx, &mut b).unwrap_err();
    assert_eq!(err_delim.kind, MacroErrorKind::DelimiterMismatch);
    assert_eq!(err_delim.kind.error_code(), 4011);

    let err_unterm = expand_with_ctx("@scripts.pair(1, 2", &mut ctx, &mut b).unwrap_err();
    assert_eq!(err_unterm.kind, MacroErrorKind::UnterminatedMacroInput);
    assert_eq!(err_unterm.kind.error_code(), 4018);

    let err_missing = expand_with_ctx("@scripts.pair(1)", &mut ctx, &mut b).unwrap_err();
    assert_eq!(err_missing.kind, MacroErrorKind::AstInputParseError);
    assert_eq!(err_missing.kind.error_code(), 4012);

    let err_extra = expand_with_ctx("@scripts.pair(1, 2, 3)", &mut ctx, &mut b).unwrap_err();
    assert_eq!(err_extra.kind, MacroErrorKind::AstInputParseError);
    assert_eq!(err_extra.kind.error_code(), 4012);

    let err_bad_kind = expand_with_ctx("@scripts.only_id(1 + 2)", &mut ctx, &mut b).unwrap_err();
    assert_eq!(err_bad_kind.kind, MacroErrorKind::AstInputParseError);
    assert_eq!(err_bad_kind.kind.error_code(), 4012);

    let mut depth_budget = MacroBudget {
        max_depth: 3,
        ..Default::default()
    };
    let err_depth =
        expand_with_ctx("@scripts.infinite(1)", &mut ctx, &mut depth_budget).unwrap_err();
    assert_eq!(err_depth.kind, MacroErrorKind::DepthExceeded);
    assert_eq!(err_depth.kind.error_code(), 4005);
    assert!(!err_depth.expansion_chain.is_empty());

    let mut step_budget = MacroBudget {
        max_steps: 2,
        ..Default::default()
    };
    let err_steps =
        expand_with_ctx("@scripts.only_id(foo)", &mut ctx, &mut step_budget).unwrap_err();
    assert_eq!(err_steps.kind, MacroErrorKind::BudgetExhausted);
    assert_eq!(err_steps.kind.error_code(), 4006);
}
