use lex::TokenKind;
use macro_expander::provenance::{ExpansionKind, MacroToken, TokenSpacing};
use macro_expander::registry::{
    ExpansionContext, MacroDescriptor, MacroExecutor, MacroId, MacroInputMode, MacroRegistry,
    MacroVisibility,
};
use macro_expander::{
    ExpandedFile, MacroBudget, MacroError, MacroErrorKind, expand_source_file,
    expand_source_file_with_context, scan_referenced_macro_roots,
};
use rustc_span::source_map::FilePathMapping;
use rustc_span::{FileName, SourceMap, Span};

fn run_expand_builtins(src: &str) -> Result<ExpandedFile, MacroError> {
    let sm = SourceMap::new(FilePathMapping::empty());
    let sf = sm.new_source_file(FileName::Custom("raw_test.fl".into()), src.to_string());
    let mut budget = MacroBudget::default();
    let (expanded, lex_errors) = expand_source_file(&sm, &sf, &mut budget)?;
    assert!(
        lex_errors.is_empty(),
        "unexpected lex errors: {lex_errors:?}"
    );
    Ok(expanded)
}

fn run_expand_with_context(
    src: &str,
    ctx: &mut ExpansionContext<'_>,
) -> Result<ExpandedFile, MacroError> {
    let sm = SourceMap::new(FilePathMapping::empty());
    let sf = sm.new_source_file(FileName::Custom("raw_ctx_test.fl".into()), src.to_string());
    let mut budget = MacroBudget::default();
    let (expanded, lex_errors) = expand_source_file_with_context(&sm, &sf, ctx, &mut budget)?;
    assert!(
        lex_errors.is_empty(),
        "unexpected lex errors: {lex_errors:?}"
    );
    Ok(expanded)
}

#[test]
fn builtin_str_raw_preserves_verbatim_bytes_and_bypasses_normal_lexer() {
    let src = r#"
const SAMPLE = @builtin.str.raw {
    name = sample
    punctuation: $ @ ^
    unterminated: " \q {- not comment } { ok }
    utf8: 你好 🦀
}
const EMPTY = @builtin.str.raw {};
"#;
    let expanded = run_expand_builtins(src).unwrap();
    assert!(expanded.expanded);
    let expanded_src = expanded.source_text.as_deref().unwrap();
    assert!(!expanded_src.contains("@builtin.str.raw"), "{expanded_src}");
    assert!(expanded.provenance.len() == 2);
    assert_eq!(
        expanded.provenance.records()[0].kind,
        ExpansionKind::ConstructionSource
    );

    // Ensure the emitted source contains valid lexable tokens including two Str literals.
    let str_tokens: Vec<_> = expanded
        .tokens
        .iter()
        .filter(|t| t.kind == TokenKind::Str)
        .collect();
    assert_eq!(str_tokens.len(), 2, "{expanded_src}");
}

#[test]
fn at_use_import_forms_shadowing_diamond_dedup_ambiguity_and_visibility() {
    let mut reg = MacroRegistry::with_builtins();
    let macro_a = MacroDescriptor {
        id: MacroId {
            package: 1,
            local: 10,
        },
        canonical_path: vec!["alpha".into(), "emit".into()],
        package: 1,
        module_path: vec!["alpha".into()],
        vis: MacroVisibility::Public,
        def_site: None,
        mode: MacroInputMode::RawSource,
    };
    let macro_b = MacroDescriptor {
        id: MacroId {
            package: 2,
            local: 20,
        },
        canonical_path: vec!["beta".into(), "emit".into()],
        package: 2,
        module_path: vec!["beta".into()],
        vis: MacroVisibility::Public,
        def_site: None,
        mode: MacroInputMode::RawSource,
    };
    let hidden = MacroDescriptor {
        id: MacroId {
            package: 3,
            local: 30,
        },
        canonical_path: vec!["gamma".into(), "secret".into()],
        package: 3,
        module_path: vec!["gamma".into()],
        vis: MacroVisibility::Private,
        def_site: None,
        mode: MacroInputMode::RawSource,
    };
    let pkg_only = MacroDescriptor {
        id: MacroId {
            package: 3,
            local: 31,
        },
        canonical_path: vec!["gamma".into(), "pkg_only".into()],
        package: 3,
        module_path: vec!["gamma".into()],
        vis: MacroVisibility::Package(3),
        def_site: None,
        mode: MacroInputMode::RawSource,
    };
    reg.register(macro_a.clone());
    reg.register(macro_b.clone());
    reg.register(hidden);
    reg.register(pkg_only);
    // Diamond re-export: `reexport.emit` points to the exact same `MacroId` as `alpha.emit`.
    reg.register_path_alias(vec!["reexport".into(), "emit".into()], macro_a.id);

    struct MockRawExecutor;
    impl MacroExecutor for MockRawExecutor {
        fn execute_raw_source(
            &mut self,
            descriptor: &MacroDescriptor,
            raw_source: &str,
            call_site: Span,
            _budget: &mut MacroBudget,
        ) -> Result<Vec<MacroToken>, MacroError> {
            let formatted = format!("\"{}:{}\"", descriptor.full_path(), raw_source.trim());
            Ok(vec![MacroToken::new(
                TokenKind::Str,
                formatted,
                call_site,
                TokenSpacing::Alone,
                None,
            )])
        }

        fn execute_tokens(
            &mut self,
            _descriptor: &MacroDescriptor,
            _input_tokens: &[MacroToken],
            _call_site: Span,
            _budget: &mut MacroBudget,
        ) -> Result<Vec<MacroToken>, MacroError> {
            unreachable!()
        }
    }

    // 1. Diamond dedup + nested multi-group + optional semicolon after brace + alias
    let mut exec = MockRawExecutor;
    let mut ctx = ExpansionContext {
        current_package: 0,
        current_module_path: vec![],
        registry: &reg,
        executor: Some(&mut exec),
    };
    let expanded = run_expand_with_context(
        r#"
        @use alpha.emit;
        @use reexport.{emit, *}
        @use builtin.str.raw as raw_text;
        const A = @emit { one };
        const B = @raw_text { two $ };
        "#,
        &mut ctx,
    )
    .unwrap();
    let text = expanded.source_text.as_deref().unwrap();
    assert!(text.contains("\"alpha.emit:one\""), "{text}");
    assert!(text.contains("\" two \\$ \""), "{text}");

    // 2. Glob ambiguity does not error on import, errors on call unless explicitly shadowed
    let mut exec = MockRawExecutor;
    let mut ctx = ExpansionContext {
        current_package: 0,
        current_module_path: vec![],
        registry: &reg,
        executor: Some(&mut exec),
    };
    let err_ambig = run_expand_with_context(
        r#"
        @use alpha.*;
        @use beta.*;
        const X = @emit { fail };
        "#,
        &mut ctx,
    )
    .unwrap_err();
    assert_eq!(err_ambig.kind, MacroErrorKind::AmbiguousMacro);

    let mut exec = MockRawExecutor;
    let mut ctx = ExpansionContext {
        current_package: 0,
        current_module_path: vec![],
        registry: &reg,
        executor: Some(&mut exec),
    };
    let shadowed = run_expand_with_context(
        r#"
        @use alpha.*;
        @use beta.*;
        @use beta.emit;
        const X = @emit { win };
        "#,
        &mut ctx,
    )
    .unwrap();
    let s = shadowed.source_text.as_deref().unwrap();
    assert!(s.contains("\"beta.emit:win\""), "{s}");

    // 3. Explicit conflict errors immediately
    let mut ctx = ExpansionContext::with_registry(&reg);
    let err_conflict = run_expand_with_context(
        r#"
        @use alpha.emit;
        @use beta.emit;
        "#,
        &mut ctx,
    )
    .unwrap_err();
    assert_eq!(err_conflict.kind, MacroErrorKind::AmbiguousMacro);

    // 4. Private / package visibility enforcement
    let mut ctx = ExpansionContext::with_registry(&reg);
    let err_vis = run_expand_with_context("@use gamma.secret;", &mut ctx).unwrap_err();
    assert_eq!(err_vis.kind, MacroErrorKind::MacroVisibilityError);

    let err_pkg_vis = run_expand_with_context("@gamma.pkg_only { x }", &mut ctx).unwrap_err();
    assert_eq!(err_pkg_vis.kind, MacroErrorKind::MacroVisibilityError);

    let mut exec = MockRawExecutor;
    let mut same_pkg_ctx = ExpansionContext {
        current_package: 3,
        current_module_path: vec!["gamma".into()],
        registry: &reg,
        executor: Some(&mut exec),
    };
    assert!(run_expand_with_context("@gamma.secret { ok }", &mut same_pkg_ctx).is_ok());
}

#[test]
fn skips_comments_strings_take_views_token_buffers_and_ordinary_use_at_pkg() {
    let src = r#"
        use @attrs.derivation.default as automatic;
        use @other.{item1, item2}
        fn f(value: str) {
            -- @unknown { ignored in comment }
            {- nested {- @unknown { also ignored } -} -}
            let a = "string @unknown { ignored } ${ { x } }";
            let b = value'a'b;
            let c = .u8'builtin;
            let t = {{ @not_expanded_here { 1 } }};
        }
    "#;
    let expanded = run_expand_builtins(src).unwrap();
    assert!(!expanded.expanded);
}

#[test]
fn delimiter_mismatch_unterminated_raw_and_combined_template_raw_work() {
    let err_delim = run_expand_builtins("const X = @builtin.str.raw(hello);").unwrap_err();
    assert_eq!(err_delim.kind, MacroErrorKind::DelimiterMismatch);

    let err_unclosed = run_expand_builtins("const X = @builtin.str.raw { { inner } ").unwrap_err();
    assert_eq!(err_unclosed.kind, MacroErrorKind::UnterminatedMacroInput);

    let err_import = run_expand_builtins("@use builtin.str.raw").unwrap_err();
    assert_eq!(err_import.kind, MacroErrorKind::InvalidMacroImport);

    // Combined Mode 4 raw source + template macro in one file
    let combined = r#"
        @use builtin.str.raw;
        define wrap_fn(name, value) {
            fn $name() -> str { $value }
        }
        $wrap_fn(banner, @raw { $ @ ^ })
    "#;
    let expanded = run_expand_builtins(combined).unwrap();
    assert!(expanded.expanded);
    let out = expanded.source_text.as_deref().unwrap();
    assert!(out.contains("fn banner() -> str"), "{out}");
    assert!(out.contains("\" \\$ @ ^ \""), "{out}");
    assert_eq!(expanded.provenance.len(), 2);
}

#[test]
fn mode3_token_macros_compose_and_enforce_delimiters_depth_and_budgets() {
    let mut reg = MacroRegistry::with_builtins();
    let passthrough = MacroDescriptor {
        id: MacroId {
            package: 10,
            local: 1,
        },
        canonical_path: vec!["scripts".into(), "passthrough".into()],
        package: 10,
        module_path: vec!["scripts".into()],
        vis: MacroVisibility::Public,
        def_site: None,
        mode: MacroInputMode::Tokens,
    };
    let double_tok = MacroDescriptor {
        id: MacroId {
            package: 10,
            local: 2,
        },
        canonical_path: vec!["scripts".into(), "emit_raw_after_lex".into()],
        package: 10,
        module_path: vec!["scripts".into()],
        vis: MacroVisibility::Public,
        def_site: None,
        mode: MacroInputMode::Tokens,
    };
    let recurse = MacroDescriptor {
        id: MacroId {
            package: 10,
            local: 3,
        },
        canonical_path: vec!["scripts".into(), "recurse".into()],
        package: 10,
        module_path: vec!["scripts".into()],
        vis: MacroVisibility::Public,
        def_site: None,
        mode: MacroInputMode::Tokens,
    };
    let raw_m = MacroDescriptor {
        id: MacroId {
            package: 10,
            local: 4,
        },
        canonical_path: vec!["scripts".into(), "raw_m".into()],
        package: 10,
        module_path: vec!["scripts".into()],
        vis: MacroVisibility::Public,
        def_site: None,
        mode: MacroInputMode::RawSource,
    };
    reg.register(passthrough);
    reg.register(double_tok);
    reg.register(recurse);
    reg.register(raw_m);

    struct MockMode3Executor;
    impl MacroExecutor for MockMode3Executor {
        fn execute_raw_source(
            &mut self,
            _descriptor: &MacroDescriptor,
            raw_source: &str,
            call_site: Span,
            _budget: &mut MacroBudget,
        ) -> Result<Vec<MacroToken>, MacroError> {
            Ok(vec![MacroToken::new(
                TokenKind::Str,
                format!("{raw_source:?}"),
                call_site,
                TokenSpacing::Alone,
                None,
            )])
        }

        fn execute_tokens(
            &mut self,
            descriptor: &MacroDescriptor,
            input_tokens: &[MacroToken],
            call_site: Span,
            _budget: &mut MacroBudget,
        ) -> Result<Vec<MacroToken>, MacroError> {
            match descriptor.canonical_path.last().map(String::as_str) {
                Some("passthrough") => Ok(input_tokens.to_vec()),
                Some("emit_raw_after_lex") => Ok(vec![
                    MacroToken::new(TokenKind::At, "@", call_site, TokenSpacing::Joint, None),
                    MacroToken::new(TokenKind::Id, "raw_m", call_site, TokenSpacing::Alone, None),
                    MacroToken::new(TokenKind::LBrace, "{", call_site, TokenSpacing::Alone, None),
                    MacroToken::new(TokenKind::Int, "1", call_site, TokenSpacing::Alone, None),
                    MacroToken::new(TokenKind::RBrace, "}", call_site, TokenSpacing::Alone, None),
                ]),
                Some("recurse") => Ok(vec![
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
                        "recurse",
                        call_site,
                        TokenSpacing::Alone,
                        None,
                    ),
                    MacroToken::new(TokenKind::LBrace, "{", call_site, TokenSpacing::Alone, None),
                    MacroToken::new(TokenKind::RBrace, "}", call_site, TokenSpacing::Alone, None),
                ]),
                _ => unreachable!(),
            }
        }
    }

    // 1. Chained Mode 3 + Mode 4 inside Mode 3 + Template macros + ordinary `use @local.helper;`
    let mut exec = MockMode3Executor;
    let mut ctx = ExpansionContext {
        current_package: 0,
        current_module_path: vec![],
        registry: &reg,
        executor: Some(&mut exec),
    };
    let expanded = run_expand_with_context(
        r#"
        @use scripts.passthrough;
        use @local.helper;

        define mk_fn(name, val) {
            fn $name() -> i32 { $val }
        }

        @passthrough {
            const RAW = @scripts.raw_m { hello $ };
            $mk_fn(get_one, 1)
        }
        "#,
        &mut ctx,
    )
    .unwrap();
    let out = expanded.source_text.as_deref().unwrap();
    assert!(
        out.contains("use @local .helper ;") || out.contains("use @local.helper;"),
        "{out}"
    );
    assert!(out.contains("fn get_one() -> i32"), "{out}");
    assert!(out.contains("\" hello $ \""), "{out}");
    assert!(
        expanded
            .provenance
            .records()
            .iter()
            .any(|r| r.kind == ExpansionKind::ConstructionTokens)
    );

    // 2. Delimiter mismatch and unterminated `{ ... }` for Mode 3
    let err_delim = run_expand_with_context("@scripts.passthrough(1)", &mut ctx).unwrap_err();
    assert_eq!(err_delim.kind, MacroErrorKind::DelimiterMismatch);

    let err_eof = run_expand_with_context("@scripts.passthrough", &mut ctx).unwrap_err();
    assert_eq!(err_eof.kind, MacroErrorKind::DelimiterMismatch);

    let err_unterm =
        run_expand_with_context("@scripts.passthrough { let x = { 1 };", &mut ctx).unwrap_err();
    assert_eq!(err_unterm.kind, MacroErrorKind::UnterminatedMacroInput);

    // 3. Post-lexing synthesized Mode 4 call is rejected cleanly
    let err_post_lex_m4 = run_expand_with_context(
        "@use scripts.raw_m;\n@scripts.emit_raw_after_lex {}",
        &mut ctx,
    )
    .unwrap_err();
    assert_eq!(err_post_lex_m4.kind, MacroErrorKind::HandlerExecutionError);

    // 4. Infinite Mode 3 recursion triggers DepthExceeded
    let sm = SourceMap::new(FilePathMapping::empty());
    let sf = sm.new_source_file(
        FileName::Custom("recurse.fl".into()),
        "@scripts.recurse {}".to_string(),
    );
    let mut small_budget = MacroBudget {
        max_depth: 4,
        ..Default::default()
    };
    let err_depth =
        expand_source_file_with_context(&sm, &sf, &mut ctx, &mut small_budget).unwrap_err();
    assert_eq!(err_depth.kind, MacroErrorKind::DepthExceeded);
    assert!(!err_depth.expansion_chain.is_empty());

    // 5. Output token budget enforcement on Mode 3
    let sm2 = SourceMap::new(FilePathMapping::empty());
    let sf2 = sm2.new_source_file(
        FileName::Custom("budget.fl".into()),
        "@scripts.passthrough { let a = 1 + 2 + 3; }".to_string(),
    );
    let mut tiny_tok_budget = MacroBudget {
        max_tokens: 3,
        ..Default::default()
    };
    let err_tok =
        expand_source_file_with_context(&sm2, &sf2, &mut ctx, &mut tiny_tok_budget).unwrap_err();
    assert_eq!(err_tok.kind, MacroErrorKind::BudgetExhausted);
}

#[test]
fn scan_referenced_macro_roots_filters_builtins_comments_strings_and_buffers() {
    let only_builtin = scan_referenced_macro_roots(
        r#"
        @use builtin.str.raw;
        use @local.helper;
        fn f(s: str) {
            -- @ghost.m { 1 }
            let x = "@fake.m { 2 }";
            let y = {{ @tb.m { 3 } }};
            let z = s'a'b;
            let raw = @raw { ok };
        }
        "#,
    );
    assert!(!only_builtin.has_non_builtin_macros);
    assert!(only_builtin.roots.is_empty());

    let real_macros = scan_referenced_macro_roots(
        r#"
        @use scripts.passthrough
        @use helper.macros.*;
        fn f() {
            @passthrough { const X = 1; }
            @other.emit { hi }
        }
        "#,
    );
    assert!(real_macros.has_non_builtin_macros);
    assert!(real_macros.import_roots.contains("scripts"));
    assert!(real_macros.import_roots.contains("helper"));
    assert!(real_macros.roots.contains("other"));
}
