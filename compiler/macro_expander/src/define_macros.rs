//! Built-in `@define_macro1` definition-macro recognizer and lowering to metadata-attributed `comptime fn`.
use crate::construction::ConstructionHost;
use crate::provenance::{ExpansionId, ExpansionKind, ExpansionMap, MacroToken, TokenSpacing};
use crate::registry::MacroInputMode;
use crate::{MacroBudget, MacroError, MacroErrorKind};
use lex::TokenKind;
use rustc_span::Span;

fn make_syn(
    kind: TokenKind,
    text: impl Into<String>,
    span: Span,
    spacing: TokenSpacing,
) -> MacroToken {
    MacroToken::new(kind, text, span, spacing, None)
}

fn find_matching_brace(tokens: &[MacroToken], open_idx: usize) -> Option<usize> {
    let mut depth = 1usize;
    let mut idx = open_idx + 1;
    while idx < tokens.len() {
        match tokens[idx].kind {
            TokenKind::LBrace => depth += 1,
            TokenKind::RBrace => {
                depth -= 1;
                if depth == 0 {
                    return Some(idx);
                }
            }
            _ => {}
        }
        idx += 1;
    }
    None
}

fn find_matching_paren(tokens: &[MacroToken], open_idx: usize) -> Option<usize> {
    let mut depth = 1usize;
    let mut idx = open_idx + 1;
    while idx < tokens.len() {
        match tokens[idx].kind {
            TokenKind::LParen => depth += 1,
            TokenKind::RParen => {
                depth -= 1;
                if depth == 0 {
                    return Some(idx);
                }
            }
            _ => {}
        }
        idx += 1;
    }
    None
}

fn find_top_level_arrow_and_brace(inner: &[MacroToken], start: usize) -> Option<(usize, usize)> {
    let mut idx = start;
    let mut paren = 0usize;
    let mut bracket = 0usize;
    let mut brace = 0usize;
    let mut angle = 0usize;
    let mut arrow_idx = None;
    while idx < inner.len() {
        match inner[idx].kind {
            TokenKind::LParen => paren += 1,
            TokenKind::RParen => paren = paren.saturating_sub(1),
            TokenKind::LBracket => bracket += 1,
            TokenKind::RBracket => bracket = bracket.saturating_sub(1),
            TokenKind::LBrace => {
                if paren == 0 && bracket == 0 && brace == 0 && angle == 0 {
                    return arrow_idx.map(|a| (a, idx));
                }
                brace += 1;
            }
            TokenKind::RBrace => brace = brace.saturating_sub(1),
            TokenKind::Lt => angle += 1,
            TokenKind::Gt if angle > 0 => angle -= 1,
            TokenKind::Arrow if paren == 0 && bracket == 0 && brace == 0 && angle == 0 => {
                if arrow_idx.is_none() {
                    arrow_idx = Some(idx);
                }
            }
            _ => {}
        }
        idx += 1;
    }
    None
}

fn split_top_level_commas<'a>(
    tokens: &'a [MacroToken],
    macro_span: Span,
) -> Result<Vec<&'a [MacroToken]>, MacroError> {
    if tokens.is_empty() {
        return Ok(Vec::new());
    }
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut paren = 0usize;
    let mut bracket = 0usize;
    let mut brace = 0usize;
    let mut angle = 0usize;

    for (idx, tok) in tokens.iter().enumerate() {
        match tok.kind {
            TokenKind::LParen => paren += 1,
            TokenKind::RParen => paren = paren.saturating_sub(1),
            TokenKind::LBracket => bracket += 1,
            TokenKind::RBracket => bracket = bracket.saturating_sub(1),
            TokenKind::LBrace => brace += 1,
            TokenKind::RBrace => brace = brace.saturating_sub(1),
            TokenKind::Lt => angle += 1,
            TokenKind::Gt if angle > 0 => angle -= 1,
            TokenKind::Comma if paren == 0 && bracket == 0 && brace == 0 && angle == 0 => {
                let slice = &tokens[start..idx];
                if slice.is_empty() {
                    return Err(MacroError::new(
                        MacroErrorKind::AstInputParseError,
                        "empty parameter entry before `,` in `@define_macro1` signature",
                        tok.span,
                    ));
                }
                parts.push(slice);
                start = idx + 1;
            }
            _ => {}
        }
    }

    if start < tokens.len() {
        parts.push(&tokens[start..]);
    } else if parts.is_empty() {
        return Err(MacroError::new(
            MacroErrorKind::AstInputParseError,
            "unexpected `,` in empty parameter list of `@define_macro1`",
            macro_span,
        ));
    }
    Ok(parts)
}

fn is_meta_ast_exported_type(name: &str) -> bool {
    matches!(
        name,
        "Repetition"
            | "Optional"
            | "TokenBuffer"
            | "FlurryPattern"
            | "FlurryExpr"
            | "FlurryStatement"
            | "FlurryExtendedStatement"
            | "FlurryItem"
            | "FlurryType"
            | "FlurryIdent"
            | "FlurryLiteral"
            | "FlurryBlock"
            | "FlurryPath"
    )
}

fn qualify_meta_ast_type(tokens: &[MacroToken]) -> Vec<MacroToken> {
    let mut out = Vec::with_capacity(tokens.len() + 8);
    for (idx, tok) in tokens.iter().enumerate() {
        let prev_is_dot = idx > 0 && tokens[idx - 1].kind == TokenKind::Dot;
        if tok.kind == TokenKind::Id && !prev_is_dot && is_meta_ast_exported_type(&tok.text) {
            let s = tok.span;
            out.push(make_syn(TokenKind::Id, "meta", s, TokenSpacing::Joint));
            out.push(make_syn(TokenKind::Dot, ".", s, TokenSpacing::Joint));
            out.push(make_syn(TokenKind::Id, "ast", s, TokenSpacing::Joint));
            out.push(make_syn(TokenKind::Dot, ".", s, TokenSpacing::Joint));
        }
        out.push(tok.clone());
    }
    out
}

/// Attempt to expand `[pub | private] const name = @define_macro1 { fn(...) -> TokenBuffer { ... } };?`.
///
/// Returns `Ok(None)` immediately when `tokens[cursor..]` is not a `const` binding targeting a
/// `MacroInputMode::BuiltinDefineMacro1` descriptor, leaving normal `const` and `@call` handling intact.
pub(crate) fn try_expand_define_macro1(
    tokens: &[MacroToken],
    cursor: usize,
    host: &mut ConstructionHost<'_, '_>,
    provenance: &mut ExpansionMap,
    budget: &mut MacroBudget,
    parent_exp: Option<ExpansionId>,
) -> Result<Option<(Vec<MacroToken>, ExpansionId, TokenSpacing, usize)>, MacroError> {
    let mut p = cursor;
    let vis_tok = match tokens.get(p).map(|t| t.kind) {
        Some(TokenKind::Pub | TokenKind::Private) => {
            let tok = tokens[p].clone();
            p += 1;
            Some(tok)
        }
        _ => None,
    };

    let Some(const_tok) = tokens.get(p) else {
        return Ok(None);
    };
    if const_tok.kind != TokenKind::Const {
        return Ok(None);
    }
    let const_start_span = vis_tok.as_ref().map_or(const_tok.span, |t| t.span);
    p += 1;

    let Some(macro_name_tok) = tokens.get(p) else {
        return Ok(None);
    };
    if macro_name_tok.kind != TokenKind::Id {
        return Ok(None);
    }
    p += 1;

    if !tokens.get(p).is_some_and(|t| t.kind == TokenKind::Eq) {
        return Ok(None);
    }
    p += 1;

    let Some(at_tok) = tokens.get(p) else {
        return Ok(None);
    };
    if at_tok.kind != TokenKind::At {
        return Ok(None);
    }
    p += 1;

    let Some(first_seg_tok) = tokens.get(p) else {
        return Ok(None);
    };
    if first_seg_tok.kind != TokenKind::Id {
        return Ok(None);
    }
    let mut segments = vec![first_seg_tok.text.clone()];
    let mut path_hi = first_seg_tok.span.hi();
    p += 1;

    while tokens.get(p).is_some_and(|t| t.kind == TokenKind::Dot)
        && tokens.get(p + 1).is_some_and(|t| t.kind == TokenKind::Id)
    {
        let seg = &tokens[p + 1];
        segments.push(seg.text.clone());
        path_hi = seg.span.hi();
        p += 2;
    }

    let path_span = Span::new(at_tok.span.lo(), path_hi);
    let Ok(desc) = host.scope.resolve_call_path(&segments, path_span, host.ctx) else {
        return Ok(None);
    };
    if !matches!(desc.mode, MacroInputMode::BuiltinDefineMacro1) {
        return Ok(None);
    }
    let macro_def_name = desc.full_path();
    let macro_def_site = desc.def_site;

    let Some(open_tok) = tokens.get(p) else {
        return Err(MacroError::new(
            MacroErrorKind::DelimiterMismatch,
            format!("`@{macro_def_name}` requires `{{ ... }}` brace delimiters"),
            path_span,
        )
        .with_chain(provenance, parent_exp));
    };
    if open_tok.kind != TokenKind::LBrace {
        return Err(MacroError::new(
            MacroErrorKind::DelimiterMismatch,
            format!(
                "`@{macro_def_name}` requires `{{ ... }}` brace delimiters, found `{}`",
                open_tok.text
            ),
            open_tok.span,
        )
        .with_label(path_span, "definition macro invoked here")
        .with_chain(provenance, parent_exp));
    }
    let open_idx = p;
    let Some(close_idx) = find_matching_brace(tokens, open_idx) else {
        return Err(MacroError::new(
            MacroErrorKind::UnterminatedMacroInput,
            format!("unterminated `@{macro_def_name} {{ ... }}`; expected closing `}}`"),
            open_tok.span,
        )
        .with_label(path_span, "definition macro invoked here")
        .with_chain(provenance, parent_exp));
    };

    let mut next_cursor = close_idx + 1;
    let mut trailing_spacing = tokens[close_idx].spacing;
    if tokens
        .get(next_cursor)
        .is_some_and(|t| t.kind == TokenKind::Semi)
    {
        trailing_spacing = tokens[next_cursor].spacing;
        next_cursor += 1;
    }
    let call_span = Span::new(const_start_span.lo(), tokens[close_idx].span.hi());

    budget
        .consume_step()
        .map_err(|e| e.with_chain_if_empty(provenance, parent_exp, call_span))?;

    let exp_id = provenance.record(
        ExpansionKind::ConstructionAst,
        macro_def_name,
        call_span,
        macro_def_site,
        parent_exp,
    );

    let inner = &tokens[open_idx + 1..close_idx];
    let mut k = 0usize;
    while inner
        .get(k)
        .is_some_and(|t| matches!(t.kind, TokenKind::Pure | TokenKind::Comptime))
    {
        k += 1;
    }

    let Some(fn_tok) = inner.get(k) else {
        return Err(MacroError::new(
            MacroErrorKind::AstInputParseError,
            "expected `fn(...) -> TokenBuffer { ... }` inside `@define_macro1`",
            open_tok.span,
        )
        .with_chain(provenance, Some(exp_id)));
    };
    if fn_tok.kind != TokenKind::Fn {
        return Err(MacroError::new(
            MacroErrorKind::AstInputParseError,
            format!(
                "expected `fn` handler definition inside `@define_macro1`, found `{}`",
                fn_tok.text
            ),
            fn_tok.span,
        )
        .with_chain(provenance, Some(exp_id)));
    }
    k += 1;

    let Some(lparen_tok) = inner.get(k) else {
        return Err(MacroError::new(
            MacroErrorKind::AstInputParseError,
            "expected `(` parameter list after `fn` inside `@define_macro1`",
            fn_tok.span,
        )
        .with_chain(provenance, Some(exp_id)));
    };
    if lparen_tok.kind != TokenKind::LParen {
        return Err(MacroError::new(
            MacroErrorKind::AstInputParseError,
            format!(
                "expected `(` after `fn` inside `@define_macro1` (handlers are unnamed), found `{}`",
                lparen_tok.text
            ),
            lparen_tok.span,
        )
        .with_chain(provenance, Some(exp_id)));
    }
    let params_open = k;
    let Some(params_close) = find_matching_paren(inner, params_open) else {
        return Err(MacroError::new(
            MacroErrorKind::AstInputParseError,
            "unclosed `(` parameter list inside `@define_macro1`",
            lparen_tok.span,
        )
        .with_chain(provenance, Some(exp_id)));
    };

    let raw_params = split_top_level_commas(&inner[params_open + 1..params_close], lparen_tok.span)
        .map_err(|e| e.with_chain(provenance, Some(exp_id)))?;
    let mut is_variadic = false;
    let mut lowered_param_tokens = Vec::new();

    for (idx, param_slice) in raw_params.iter().enumerate() {
        let is_last = idx + 1 == raw_params.len();
        let mut start_idx = 0usize;
        if param_slice.len() >= 3
            && param_slice[0].kind == TokenKind::Dot
            && param_slice[1].kind == TokenKind::Dot
            && param_slice[2].kind == TokenKind::Dot
        {
            if !is_last {
                return Err(MacroError::new(
                    MacroErrorKind::AstInputParseError,
                    "variadic `...` parameter in `@define_macro1` must be the last parameter",
                    param_slice[0].span,
                )
                .with_chain(provenance, Some(exp_id)));
            }
            is_variadic = true;
            start_idx = 3;
        }

        let Some(name_tok) = param_slice.get(start_idx) else {
            return Err(MacroError::new(
                MacroErrorKind::AstInputParseError,
                "expected parameter name inside `@define_macro1`",
                param_slice[0].span,
            )
            .with_chain(provenance, Some(exp_id)));
        };
        if !matches!(name_tok.kind, TokenKind::Id | TokenKind::Underscore) {
            return Err(MacroError::new(
                MacroErrorKind::AstInputParseError,
                format!(
                    "expected parameter identifier in `@define_macro1`, found `{}`",
                    name_tok.text
                ),
                name_tok.span,
            )
            .with_chain(provenance, Some(exp_id)));
        }

        let Some(colon_tok) = param_slice.get(start_idx + 1) else {
            return Err(MacroError::new(
                MacroErrorKind::AstInputParseError,
                format!(
                    "expected `:` and AST node type after parameter `{}` in `@define_macro1`",
                    name_tok.text
                ),
                name_tok.span,
            )
            .with_chain(provenance, Some(exp_id)));
        };
        if colon_tok.kind != TokenKind::Colon {
            return Err(MacroError::new(
                MacroErrorKind::AstInputParseError,
                format!(
                    "expected `:` after parameter `{}`, found `{}`",
                    name_tok.text, colon_tok.text
                ),
                colon_tok.span,
            )
            .with_chain(provenance, Some(exp_id)));
        }

        let ty_tokens = &param_slice[start_idx + 2..];
        if ty_tokens.is_empty() {
            return Err(MacroError::new(
                MacroErrorKind::AstInputParseError,
                format!(
                    "expected type after `{}:` in `@define_macro1`",
                    name_tok.text
                ),
                colon_tok.span,
            )
            .with_chain(provenance, Some(exp_id)));
        }

        if idx > 0 {
            lowered_param_tokens.push(make_syn(
                TokenKind::Comma,
                ",",
                name_tok.span,
                TokenSpacing::Alone,
            ));
        }
        lowered_param_tokens.push(name_tok.clone());
        lowered_param_tokens.push(colon_tok.clone());
        lowered_param_tokens.extend(qualify_meta_ast_type(ty_tokens));
    }

    let Some((arrow_idx, body_open)) = find_top_level_arrow_and_brace(inner, params_close + 1)
    else {
        return Err(MacroError::new(
            MacroErrorKind::AstInputParseError,
            "expected `-> TokenBuffer { ... }` return type and body in `@define_macro1`",
            inner[params_close].span,
        )
        .with_chain(provenance, Some(exp_id)));
    };

    if arrow_idx != params_close + 1 {
        return Err(MacroError::new(
            MacroErrorKind::AstInputParseError,
            "unexpected tokens before `->` in `@define_macro1` signature",
            inner[params_close + 1].span,
        )
        .with_chain(provenance, Some(exp_id)));
    }

    let ret_tokens = &inner[arrow_idx + 1..body_open];
    if ret_tokens.is_empty() {
        return Err(MacroError::new(
            MacroErrorKind::AstInputParseError,
            "expected return type after `->` in `@define_macro1`",
            inner[arrow_idx].span,
        )
        .with_chain(provenance, Some(exp_id)));
    }
    let qualified_ret = qualify_meta_ast_type(ret_tokens);

    let Some(body_close) = find_matching_brace(inner, body_open) else {
        return Err(MacroError::new(
            MacroErrorKind::AstInputParseError,
            "unclosed function body `{ ... }` inside `@define_macro1`",
            inner[body_open].span,
        )
        .with_chain(provenance, Some(exp_id)));
    };

    if body_close + 1 < inner.len() {
        return Err(MacroError::new(
            MacroErrorKind::AstInputParseError,
            format!(
                "unexpected trailing token `{}` after body in `@define_macro1`",
                inner[body_close + 1].text
            ),
            inner[body_close + 1].span,
        )
        .with_chain(provenance, Some(exp_id)));
    }

    let head_span = macro_name_tok.span;
    let variadic_tok = if is_variadic {
        make_syn(TokenKind::True, "true", head_span, TokenSpacing::Alone)
    } else {
        make_syn(TokenKind::False, "false", head_span, TokenSpacing::Alone)
    };

    let mut out = vec![
        make_syn(TokenKind::Caret, "^", head_span, TokenSpacing::Joint),
        make_syn(TokenKind::LBrace, "{", head_span, TokenSpacing::Alone),
        make_syn(TokenKind::Id, "metadata", head_span, TokenSpacing::Joint),
        make_syn(TokenKind::Colon, ":", head_span, TokenSpacing::Alone),
        make_syn(TokenKind::LBrace, "{", head_span, TokenSpacing::Alone),
        make_syn(
            TokenKind::Id,
            "flurry_macro_kind",
            head_span,
            TokenSpacing::Joint,
        ),
        make_syn(TokenKind::Colon, ":", head_span, TokenSpacing::Alone),
        make_syn(TokenKind::Dot, ".", head_span, TokenSpacing::Joint),
        make_syn(TokenKind::Id, "ast", head_span, TokenSpacing::Joint),
        make_syn(TokenKind::Comma, ",", head_span, TokenSpacing::Alone),
        make_syn(
            TokenKind::Id,
            "flurry_macro_variadic",
            head_span,
            TokenSpacing::Joint,
        ),
        make_syn(TokenKind::Colon, ":", head_span, TokenSpacing::Alone),
        variadic_tok,
        make_syn(TokenKind::RBrace, "}", head_span, TokenSpacing::Alone),
        make_syn(TokenKind::RBrace, "}", head_span, TokenSpacing::Newline),
    ];

    if let Some(vis) = vis_tok {
        let mut vis = vis;
        vis.spacing = TokenSpacing::Alone;
        out.push(vis);
    }

    out.push(make_syn(
        TokenKind::Comptime,
        "comptime",
        fn_tok.span,
        TokenSpacing::Alone,
    ));
    out.push(make_syn(
        TokenKind::Fn,
        "fn",
        fn_tok.span,
        TokenSpacing::Alone,
    ));
    let mut name_out = macro_name_tok.clone();
    name_out.spacing = TokenSpacing::Joint;
    out.push(name_out);
    out.push(make_syn(
        TokenKind::LParen,
        "(",
        lparen_tok.span,
        TokenSpacing::Alone,
    ));
    out.extend(lowered_param_tokens);
    out.push(make_syn(
        TokenKind::RParen,
        ")",
        inner[params_close].span,
        TokenSpacing::Alone,
    ));
    out.push(make_syn(
        TokenKind::Arrow,
        "->",
        inner[arrow_idx].span,
        TokenSpacing::Alone,
    ));
    out.extend(qualified_ret);
    if let Some(last) = out.last_mut() {
        last.spacing = TokenSpacing::Alone;
    }
    out.extend_from_slice(&inner[body_open..=body_close]);

    for tok in &mut out {
        if tok.expansion_id.is_none() {
            tok.expansion_id = Some(exp_id);
        }
    }

    Ok(Some((out, exp_id, trailing_spacing, next_cursor)))
}
