//! File-local sequential template macro definition and expansion (`docs/macro/template-macro.md`).
use crate::provenance::{ExpansionId, ExpansionKind, ExpansionMap, MacroToken, TokenSpacing};
use crate::{MacroBudget, MacroError, MacroErrorKind};
use lex::TokenKind;
use rustc_span::Span;
use std::collections::HashMap;

#[derive(Clone, Debug)]
enum TemplateKind {
    Fixed { count: usize },
    Repeated { count: usize },
    Named { params: Vec<String> },
}

#[derive(Clone, Debug)]
enum TemplatePart {
    Literal(MacroToken),
    Param {
        index: usize,
        spacing: TokenSpacing,
    },
    /// Escaped `$$` inside template body, materialized as a single `$` token.
    EscapedDollar {
        span: Span,
        spacing: TokenSpacing,
    },
}

#[derive(Clone, Debug)]
struct TemplateMacroDef {
    name: String,
    def_site: Span,
    kind: TemplateKind,
    body: Vec<TemplatePart>,
}

/// Expand all file-local `define` template macro definitions and `$name` calls sequentially.
/// Returns `(tokens, expanded_anything)`.
pub fn expand_template_macros(
    input: Vec<MacroToken>,
    provenance: &mut ExpansionMap,
    budget: &mut MacroBudget,
) -> Result<(Vec<MacroToken>, bool), MacroError> {
    expand_tokens_with_host(&input, None, provenance, budget)
}

pub(crate) fn expand_tokens_with_host(
    input: &[MacroToken],
    construction: Option<&mut crate::construction::ConstructionHost<'_, '_>>,
    provenance: &mut ExpansionMap,
    budget: &mut MacroBudget,
) -> Result<(Vec<MacroToken>, bool), MacroError> {
    let mut env = HashMap::new();
    let mut expanded_any = false;
    let output = expand_slice_with_construction(
        input,
        &mut env,
        construction,
        provenance,
        budget,
        0,
        None,
        &mut expanded_any,
    )?;
    Ok((output, expanded_any))
}

#[allow(clippy::too_many_arguments)]
fn finish_expanded_replacement(
    mut substituted: Vec<MacroToken>,
    exp_id: ExpansionId,
    trailing_spacing: TokenSpacing,
    env: &mut HashMap<String, TemplateMacroDef>,
    construction: Option<&mut crate::construction::ConstructionHost<'_, '_>>,
    provenance: &mut ExpansionMap,
    budget: &mut MacroBudget,
    depth: usize,
    expanded_any: &mut bool,
    out: &mut Vec<MacroToken>,
) -> Result<(), MacroError> {
    for tok in &mut substituted {
        if tok.expansion_id.is_none() {
            tok.expansion_id = Some(exp_id);
        }
    }

    let mut recursively_expanded = expand_slice_with_construction(
        &substituted,
        env,
        construction,
        provenance,
        budget,
        depth + 1,
        Some(exp_id),
        expanded_any,
    )?;
    let propagated_spacing = if trailing_spacing == TokenSpacing::Separated {
        TokenSpacing::Alone
    } else {
        trailing_spacing
    };
    if let Some(last) = recursively_expanded.last_mut() {
        if last.spacing != TokenSpacing::Separated {
            last.spacing = propagated_spacing;
        }
    } else if let Some(prev_out) = out.last_mut() {
        if prev_out.spacing == TokenSpacing::Joint && propagated_spacing != TokenSpacing::Joint {
            prev_out.spacing = propagated_spacing;
        }
    }
    out.extend(recursively_expanded);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn expand_slice_with_construction(
    tokens: &[MacroToken],
    env: &mut HashMap<String, TemplateMacroDef>,
    mut construction: Option<&mut crate::construction::ConstructionHost<'_, '_>>,
    provenance: &mut ExpansionMap,
    budget: &mut MacroBudget,
    depth: usize,
    parent_exp: Option<ExpansionId>,
    expanded_any: &mut bool,
) -> Result<Vec<MacroToken>, MacroError> {
    let mut out = Vec::with_capacity(tokens.len());
    let mut cursor = 0usize;
    let mut in_use_stmt = false;
    let mut use_brace_depth = 0usize;
    let mut use_can_end = false;

    while cursor < tokens.len() {
        budget
            .consume_step()
            .map_err(|err| err.with_chain_if_empty(provenance, parent_exp, tokens[cursor].span))?;

        let tok = &tokens[cursor];

        // Preserve `{{ ... }}` token buffers verbatim without interpreting `$` or `@` inside them.
        if is_token_buffer_open(tokens, cursor, out.last()) {
            if let Some(end) = find_token_buffer_end(tokens, cursor) {
                budget
                    .consume_tokens(end - cursor)
                    .map_err(|err| err.with_chain_if_empty(provenance, parent_exp, tok.span))?;
                out.extend_from_slice(&tokens[cursor..end]);
                cursor = end;
                continue;
            }
        }

        if tok.kind == TokenKind::Define {
            let (def, next_cursor) = parse_template_definition(tokens, cursor, env, budget)?;
            env.insert(def.name.clone(), def);
            *expanded_any = true;
            cursor = next_cursor;
            continue;
        }

        if !in_use_stmt
            && matches!(
                tok.kind,
                TokenKind::Pub | TokenKind::Private | TokenKind::Const
            )
        {
            if let Some(host) = construction.as_deref_mut() {
                if let Some((produced, exp_id, trailing_spacing, next_cursor)) =
                    crate::define_macros::try_expand_define_macro(
                        tokens, cursor, host, provenance, budget, parent_exp,
                    )?
                {
                    *expanded_any = true;
                    finish_expanded_replacement(
                        produced,
                        exp_id,
                        trailing_spacing,
                        env,
                        Some(host),
                        provenance,
                        budget,
                        depth,
                        expanded_any,
                        &mut out,
                    )?;
                    cursor = next_cursor;
                    continue;
                }
            }
        }

        if tok.kind == TokenKind::At && !in_use_stmt {
            if let Some(host) = construction.as_deref_mut() {
                *expanded_any = true;
                let (produced, exp_id, trailing_spacing, next_cursor) =
                    crate::construction::expand_at_call(
                        tokens, cursor, host, provenance, budget, depth, parent_exp,
                    )?;
                finish_expanded_replacement(
                    produced,
                    exp_id,
                    trailing_spacing,
                    env,
                    Some(host),
                    provenance,
                    budget,
                    depth,
                    expanded_any,
                    &mut out,
                )?;
                cursor = next_cursor;
                continue;
            }
        }

        if tok.kind == TokenKind::Dollar {
            let Some(name_tok) = tokens.get(cursor + 1) else {
                return Err(MacroError::new(
                    MacroErrorKind::InvalidTemplateCall,
                    "unexpected `$` at end of input; expected template macro name",
                    tok.span,
                )
                .with_chain(provenance, parent_exp));
            };
            if name_tok.kind != TokenKind::Id {
                return Err(MacroError::new(
                    MacroErrorKind::InvalidTemplateCall,
                    format!(
                        "expected template macro identifier after `$`, found `{}`",
                        name_tok.text
                    ),
                    name_tok.span,
                )
                .with_chain(provenance, parent_exp));
            }

            let Some(def) = env.get(&name_tok.text).cloned() else {
                return Err(MacroError::new(
                    MacroErrorKind::UndefinedTemplateMacro,
                    format!(
                        "undefined template macro `${}` (template macros must be defined before use in the same file)",
                        name_tok.text
                    ),
                    Span::new(tok.span.lo(), name_tok.span.hi()),
                )
                .with_chain(provenance, parent_exp));
            };

            if depth + 1 > budget.max_depth {
                return Err(MacroError::new(
                    MacroErrorKind::DepthExceeded,
                    format!(
                        "macro expansion depth limit ({}) exceeded while expanding `${}`",
                        budget.max_depth, def.name
                    ),
                    Span::new(tok.span.lo(), name_tok.span.hi()),
                )
                .with_chain(provenance, parent_exp));
            }

            *expanded_any = true;
            let (substituted, exp_id, trailing_spacing, next_cursor) =
                instantiate_call(&def, tokens, cursor, provenance, parent_exp, budget)?;

            finish_expanded_replacement(
                substituted,
                exp_id,
                trailing_spacing,
                env,
                construction.as_deref_mut(),
                provenance,
                budget,
                depth,
                expanded_any,
                &mut out,
            )?;
            cursor = next_cursor;
            continue;
        }

        if !in_use_stmt && tok.kind == TokenKind::Use {
            let after_dot = out.last().is_some_and(|t| t.kind == TokenKind::Dot);
            if !after_dot {
                in_use_stmt = true;
                use_brace_depth = 0;
                use_can_end = false;
            }
        } else if in_use_stmt {
            match tok.kind {
                TokenKind::LBrace => {
                    use_brace_depth += 1;
                    use_can_end = false;
                }
                TokenKind::RBrace => {
                    if use_brace_depth > 0 {
                        use_brace_depth -= 1;
                        if use_brace_depth == 0 {
                            in_use_stmt = false;
                            use_can_end = false;
                        }
                    }
                }
                TokenKind::Semi if use_brace_depth == 0 => {
                    in_use_stmt = false;
                    use_can_end = false;
                }
                TokenKind::Dot | TokenKind::Comma | TokenKind::At | TokenKind::As => {
                    use_can_end = false;
                }
                TokenKind::Id | TokenKind::Star | TokenKind::SeparatedStar => {
                    use_can_end = true;
                }
                _ => {}
            }
        }

        budget
            .consume_tokens(1)
            .map_err(|err| err.with_chain_if_empty(provenance, parent_exp, tok.span))?;
        out.push(tok.clone());
        if in_use_stmt
            && use_brace_depth == 0
            && use_can_end
            && tok.spacing == TokenSpacing::Newline
        {
            in_use_stmt = false;
            use_can_end = false;
        }
        cursor += 1;
    }

    Ok(out)
}

fn is_token_buffer_open(
    tokens: &[MacroToken],
    cursor: usize,
    prev_out: Option<&MacroToken>,
) -> bool {
    let Some(t1) = tokens.get(cursor) else {
        return false;
    };
    let Some(t2) = tokens.get(cursor + 1) else {
        return false;
    };
    if t1.kind != TokenKind::LBrace
        || t2.kind != TokenKind::LBrace
        || t1.spacing != TokenSpacing::Joint
    {
        return false;
    }
    let prev = if cursor > 0 {
        tokens.get(cursor - 1)
    } else {
        prev_out
    };
    !matches!(
        prev.map(|t| t.kind),
        Some(
            TokenKind::Match
                | TokenKind::Do
                | TokenKind::When
                | TokenKind::Else
                | TokenKind::Hash
                | TokenKind::Bang
                | TokenKind::Struct
                | TokenKind::Enum
                | TokenKind::Union
                | TokenKind::Trait
                | TokenKind::Impl
                | TokenKind::Extend
                | TokenKind::Mod
                | TokenKind::Unsafe
                | TokenKind::Async
                | TokenKind::Comptime
        )
    )
}

fn find_token_buffer_end(tokens: &[MacroToken], start: usize) -> Option<usize> {
    let mut cursor = start + 2;
    let mut inner_brace_depth = 0usize;
    while cursor < tokens.len() {
        let tok = &tokens[cursor];
        if is_token_buffer_open(tokens, cursor, tokens.get(cursor.saturating_sub(1))) {
            if let Some(nested_end) = find_token_buffer_end(tokens, cursor) {
                cursor = nested_end;
                continue;
            }
            return None;
        }
        match tok.kind {
            TokenKind::LBrace => {
                inner_brace_depth += 1;
                cursor += 1;
            }
            TokenKind::RBrace => {
                if inner_brace_depth > 0 {
                    inner_brace_depth -= 1;
                    cursor += 1;
                } else if tok.spacing == TokenSpacing::Joint
                    && tokens
                        .get(cursor + 1)
                        .is_some_and(|next| next.kind == TokenKind::RBrace)
                {
                    return Some(cursor + 2);
                } else {
                    // Unmatched single `}` before `}}`.
                    return None;
                }
            }
            _ => cursor += 1,
        }
    }
    None
}

fn parse_template_definition(
    tokens: &[MacroToken],
    start: usize,
    env: &HashMap<String, TemplateMacroDef>,
    budget: &mut MacroBudget,
) -> Result<(TemplateMacroDef, usize), MacroError> {
    let define_tok = &tokens[start];
    let mut cursor = start + 1;

    let Some(name_tok) = tokens.get(cursor) else {
        return Err(MacroError::new(
            MacroErrorKind::InvalidTemplateDefinition,
            "expected template macro name after `define`",
            define_tok.span,
        ));
    };
    if name_tok.kind != TokenKind::Id {
        return Err(MacroError::new(
            MacroErrorKind::InvalidTemplateDefinition,
            format!(
                "expected template macro identifier after `define`, found `{}`",
                name_tok.text
            ),
            name_tok.span,
        ));
    }
    let name = name_tok.text.clone();
    cursor += 1;

    let Some(after_name) = tokens.get(cursor) else {
        return Err(MacroError::new(
            MacroErrorKind::InvalidTemplateDefinition,
            format!("expected parameter count or `(param*)` after `define {name}`"),
            name_tok.span,
        ));
    };

    let kind = if after_name.kind == TokenKind::Int {
        let count_str: String = after_name.text.chars().filter(|&c| c != '_').collect();
        let count = count_str.parse::<usize>().map_err(|_| {
            MacroError::new(
                MacroErrorKind::InvalidTemplateDefinition,
                format!("invalid template parameter count `{}`", after_name.text),
                after_name.span,
            )
        })?;
        cursor += 1;

        let is_repeated = tokens.get(cursor).is_some_and(|d1| {
            d1.kind == TokenKind::Dot
                && d1.spacing == TokenSpacing::Joint
                && tokens.get(cursor + 1).is_some_and(|d2| {
                    d2.kind == TokenKind::Dot
                        && d2.spacing == TokenSpacing::Joint
                        && tokens
                            .get(cursor + 2)
                            .is_some_and(|d3| d3.kind == TokenKind::Dot)
                })
        });
        if is_repeated {
            if count == 0 {
                return Err(MacroError::new(
                    MacroErrorKind::InvalidTemplateDefinition,
                    format!("repeated template macro `{name}` requires group size >= 1, found 0"),
                    after_name.span,
                ));
            }
            cursor += 3;
            TemplateKind::Repeated { count }
        } else {
            TemplateKind::Fixed { count }
        }
    } else if after_name.kind == TokenKind::LParen {
        cursor += 1;
        let mut params = Vec::new();
        let mut expect_comma = false;
        loop {
            let Some(t) = tokens.get(cursor) else {
                return Err(MacroError::new(
                    MacroErrorKind::InvalidTemplateDefinition,
                    format!("unterminated parameter list in `define {name}(`"),
                    after_name.span,
                ));
            };
            if t.kind == TokenKind::RParen {
                cursor += 1;
                break;
            }
            if expect_comma {
                if t.kind != TokenKind::Comma {
                    return Err(MacroError::new(
                        MacroErrorKind::InvalidTemplateDefinition,
                        format!(
                            "expected `,` or `)` in `define {name}` parameter list, found `{}`",
                            t.text
                        ),
                        t.span,
                    ));
                }
                cursor += 1;
                expect_comma = false;
                continue;
            }
            if t.kind != TokenKind::Id {
                return Err(MacroError::new(
                    MacroErrorKind::InvalidTemplateDefinition,
                    format!(
                        "expected parameter identifier in `define {name}`, found `{}`",
                        t.text
                    ),
                    t.span,
                ));
            }
            if params.contains(&t.text) {
                return Err(MacroError::new(
                    MacroErrorKind::InvalidTemplateDefinition,
                    format!("duplicate parameter `{}` in `define {name}`", t.text),
                    t.span,
                ));
            }
            params.push(t.text.clone());
            cursor += 1;
            expect_comma = true;
        }
        TemplateKind::Named { params }
    } else {
        return Err(MacroError::new(
            MacroErrorKind::InvalidTemplateDefinition,
            format!(
                "expected integer count or `(params)` after `define {name}`, found `{}`",
                after_name.text
            ),
            after_name.span,
        ));
    };

    let Some(lbrace) = tokens.get(cursor) else {
        return Err(MacroError::new(
            MacroErrorKind::InvalidTemplateDefinition,
            format!("expected `{{` body after `define {name}` header"),
            name_tok.span,
        ));
    };
    if lbrace.kind != TokenKind::LBrace {
        return Err(MacroError::new(
            MacroErrorKind::InvalidTemplateDefinition,
            format!(
                "expected `{{` body for `define {name}`, found `{}`",
                lbrace.text
            ),
            lbrace.span,
        ));
    }
    cursor += 1;

    // Collect body tokens up to the matching `}`, respecting nested `{ ... }` and `{{ ... }}`.
    let body_start = cursor;
    let mut brace_depth = 1usize;
    while cursor < tokens.len() {
        budget
            .consume_step()
            .map_err(|err| err.with_span(tokens[cursor].span))?;
        let t = &tokens[cursor];
        match t.kind {
            TokenKind::LBrace => {
                brace_depth += 1;
                cursor += 1;
            }
            TokenKind::RBrace => {
                brace_depth -= 1;
                if brace_depth == 0 {
                    break;
                }
                cursor += 1;
            }
            _ => cursor += 1,
        }
    }
    if brace_depth != 0 {
        return Err(MacroError::new(
            MacroErrorKind::InvalidTemplateDefinition,
            format!("unterminated body `{{` for template macro `{name}`"),
            lbrace.span,
        ));
    }

    let body_end = cursor;
    let rbrace_span = tokens[cursor].span;
    cursor += 1;

    let body_tokens = &tokens[body_start..body_end];
    let body = compile_template_body(&name, &kind, body_tokens, env)?;
    let def_site = Span::new(define_tok.span.lo(), rbrace_span.hi());

    Ok((
        TemplateMacroDef {
            name,
            def_site,
            kind,
            body,
        },
        cursor,
    ))
}

fn compile_template_body(
    macro_name: &str,
    kind: &TemplateKind,
    tokens: &[MacroToken],
    env: &HashMap<String, TemplateMacroDef>,
) -> Result<Vec<TemplatePart>, MacroError> {
    let mut parts = Vec::with_capacity(tokens.len());
    let mut i = 0usize;
    let mut tb_end_stack: Vec<usize> = Vec::new();

    while i < tokens.len() {
        while tb_end_stack.last().is_some_and(|&end| i >= end) {
            tb_end_stack.pop();
        }
        let prev = (i > 0).then(|| &tokens[i - 1]);
        if is_token_buffer_open(tokens, i, prev) {
            if let Some(end) = find_token_buffer_end(tokens, i) {
                tb_end_stack.push(end);
            }
        }
        let in_token_buffer = !tb_end_stack.is_empty();

        let tok = &tokens[i];
        if tok.kind == TokenKind::Dollar {
            let Some(next) = tokens.get(i + 1) else {
                return Err(MacroError::new(
                    MacroErrorKind::InvalidTemplateBody,
                    format!("dangling `$` at end of template macro `{macro_name}` body"),
                    tok.span,
                ));
            };
            if next.kind == TokenKind::Dollar {
                parts.push(TemplatePart::EscapedDollar {
                    span: Span::new(tok.span.lo(), next.span.hi()),
                    spacing: next.spacing,
                });
                i += 2;
                continue;
            }

            match kind {
                TemplateKind::Fixed { count } | TemplateKind::Repeated { count } => {
                    if next.kind == TokenKind::Int {
                        let idx = next.text.parse::<usize>().map_err(|_| {
                            MacroError::new(
                                MacroErrorKind::InvalidTemplateBody,
                                format!(
                                    "invalid parameter index `${}` in `{macro_name}`",
                                    next.text
                                ),
                                next.span,
                            )
                        })?;
                        if idx >= *count {
                            return Err(MacroError::new(
                                MacroErrorKind::InvalidTemplateBody,
                                format!(
                                    "template parameter index `${idx}` is out of range for `{macro_name}` (declared {count} parameter(s))"
                                ),
                                Span::new(tok.span.lo(), next.span.hi()),
                            ));
                        }
                        parts.push(TemplatePart::Param {
                            index: idx,
                            spacing: next.spacing,
                        });
                        i += 2;
                        continue;
                    }
                    if in_token_buffer
                        || (next.kind == TokenKind::Id && env.contains_key(&next.text))
                    {
                        parts.push(TemplatePart::Literal(tok.clone()));
                        i += 1;
                        continue;
                    }
                    return Err(MacroError::new(
                        MacroErrorKind::InvalidTemplateBody,
                        format!(
                            "positional template macro `{macro_name}` requires `$0`..`${}` parameters, found `${}`",
                            count.saturating_sub(1),
                            next.text
                        ),
                        Span::new(tok.span.lo(), next.span.hi()),
                    ));
                }
                TemplateKind::Named { params } => {
                    if next.kind == TokenKind::Id {
                        if let Some(idx) = params.iter().position(|p| p == &next.text) {
                            parts.push(TemplatePart::Param {
                                index: idx,
                                spacing: next.spacing,
                            });
                            i += 2;
                            continue;
                        }
                        if in_token_buffer || env.contains_key(&next.text) {
                            parts.push(TemplatePart::Literal(tok.clone()));
                            i += 1;
                            continue;
                        }
                        return Err(MacroError::new(
                            MacroErrorKind::InvalidTemplateBody,
                            format!(
                                "unknown template parameter `${}` in `{macro_name}`",
                                next.text
                            ),
                            Span::new(tok.span.lo(), next.span.hi()),
                        ));
                    }
                    if in_token_buffer {
                        parts.push(TemplatePart::Literal(tok.clone()));
                        i += 1;
                        continue;
                    }
                    return Err(MacroError::new(
                        MacroErrorKind::InvalidTemplateBody,
                        format!(
                            "named template macro `{macro_name}` requires `$name` parameters, found `${}`",
                            next.text
                        ),
                        Span::new(tok.span.lo(), next.span.hi()),
                    ));
                }
            }
        }

        parts.push(TemplatePart::Literal(tok.clone()));
        i += 1;
    }

    Ok(parts)
}

fn instantiate_call(
    def: &TemplateMacroDef,
    tokens: &[MacroToken],
    start: usize,
    provenance: &mut ExpansionMap,
    parent_exp: Option<ExpansionId>,
    budget: &mut MacroBudget,
) -> Result<(Vec<MacroToken>, ExpansionId, TokenSpacing, usize), MacroError> {
    let dollar_tok = &tokens[start];
    let name_tok = &tokens[start + 1];
    let mut cursor = start + 2;

    match &def.kind {
        TemplateKind::Fixed { count } => {
            if tokens.len() - cursor < *count {
                let found = tokens.len() - cursor;
                return Err(MacroError::new(
                    MacroErrorKind::InvalidTemplateCall,
                    format!(
                        "template macro `${}` expects {} token(s), found only {} before end of input",
                        def.name, count, found
                    ),
                    Span::new(dollar_tok.span.lo(), name_tok.span.hi()),
                )
                .with_chain(provenance, parent_exp));
            }
            let mut captured = Vec::with_capacity(*count);
            for i in 0..*count {
                captured.push(std::slice::from_ref(&tokens[cursor + i]));
            }
            let (call_end, trailing_spacing) = if *count > 0 {
                let last = &tokens[cursor + count - 1];
                (last.span.hi(), last.spacing)
            } else {
                (name_tok.span.hi(), name_tok.spacing)
            };
            cursor += *count;
            let call_span = Span::new(dollar_tok.span.lo(), call_end);
            let exp_id = provenance.record(
                ExpansionKind::Template,
                def.name.clone(),
                call_span,
                Some(def.def_site),
                parent_exp,
            );
            let expanded = substitute_group(&def.body, &captured, budget, dollar_tok.span)
                .map_err(|err| err.with_chain(provenance, Some(exp_id)))?;
            Ok((expanded, exp_id, trailing_spacing, cursor))
        }
        TemplateKind::Repeated { count } => {
            let group_start = cursor;
            let mut delim_depth = 0usize;
            let mut tb_end: Option<usize> = None;
            let mut found_close = None;
            while cursor < tokens.len() {
                if tb_end.is_some_and(|end| cursor >= end) {
                    tb_end = None;
                }
                let prev = (cursor > 0).then(|| &tokens[cursor - 1]);
                if tb_end.is_none() && is_token_buffer_open(tokens, cursor, prev) {
                    if let Some(end) = find_token_buffer_end(tokens, cursor) {
                        tb_end = Some(end);
                    }
                }
                let t = &tokens[cursor];
                if tb_end.is_none() {
                    match t.kind {
                        TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => {
                            delim_depth += 1;
                        }
                        TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                            delim_depth = delim_depth.saturating_sub(1);
                        }
                        TokenKind::Dollar if delim_depth == 0 => {
                            found_close = Some(cursor);
                            break;
                        }
                        _ => {}
                    }
                }
                cursor += 1;
            }
            let Some(close_idx) = found_close else {
                return Err(MacroError::new(
                    MacroErrorKind::InvalidTemplateCall,
                    format!(
                        "unterminated repeated template macro call `${}`: missing closing `$`",
                        def.name
                    ),
                    Span::new(dollar_tok.span.lo(), name_tok.span.hi()),
                )
                .with_chain(provenance, parent_exp));
            };
            let captured_all = &tokens[group_start..close_idx];
            if captured_all.len() % *count != 0 {
                return Err(MacroError::new(
                    MacroErrorKind::InvalidTemplateCall,
                    format!(
                        "repeated template macro `${}` requires groups of {} token(s), got {} token(s) before closing `$`",
                        def.name,
                        count,
                        captured_all.len()
                    ),
                    Span::new(dollar_tok.span.lo(), tokens[close_idx].span.hi()),
                )
                .with_chain(provenance, parent_exp));
            }
            let call_span = Span::new(dollar_tok.span.lo(), tokens[close_idx].span.hi());
            let trailing_spacing = tokens[close_idx].spacing;
            let exp_id = provenance.record(
                ExpansionKind::Template,
                def.name.clone(),
                call_span,
                Some(def.def_site),
                parent_exp,
            );
            let mut output = Vec::new();
            for chunk in captured_all.chunks_exact(*count) {
                let group_args: Vec<&[MacroToken]> =
                    chunk.iter().map(std::slice::from_ref).collect();
                let substituted = substitute_group(&def.body, &group_args, budget, dollar_tok.span)
                    .map_err(|err| err.with_chain(provenance, Some(exp_id)))?;
                output.extend(substituted);
            }
            Ok((output, exp_id, trailing_spacing, close_idx + 1))
        }
        TemplateKind::Named { params } => {
            let Some(lparen) = tokens.get(cursor) else {
                return Err(MacroError::new(
                    MacroErrorKind::InvalidTemplateCall,
                    format!(
                        "named template macro `${}` requires `(...)` argument list",
                        def.name
                    ),
                    Span::new(dollar_tok.span.lo(), name_tok.span.hi()),
                )
                .with_chain(provenance, parent_exp));
            };
            if lparen.kind != TokenKind::LParen {
                return Err(MacroError::new(
                    MacroErrorKind::InvalidTemplateCall,
                    format!(
                        "named template macro `${}` requires `(...)` argument list, found `{}`",
                        def.name, lparen.text
                    ),
                    lparen.span,
                )
                .with_chain(provenance, parent_exp));
            }
            cursor += 1;

            let mut args: Vec<&[MacroToken]> = Vec::new();
            let mut current_start = cursor;
            let mut paren_depth = 0usize;
            let mut bracket_depth = 0usize;
            let mut brace_depth = 0usize;
            let mut close_idx = None;

            while cursor < tokens.len() {
                let t = &tokens[cursor];
                let at_top = paren_depth == 0 && bracket_depth == 0 && brace_depth == 0;
                if at_top && t.kind == TokenKind::RParen {
                    if cursor > current_start {
                        args.push(&tokens[current_start..cursor]);
                    } else if !args.is_empty() {
                        // Trailing comma after previous arg is allowed; empty arg list handled below.
                    }
                    close_idx = Some(cursor);
                    break;
                }
                if at_top && t.kind == TokenKind::Comma {
                    if cursor == current_start {
                        return Err(MacroError::new(
                            MacroErrorKind::InvalidTemplateCall,
                            format!("empty argument before `,` in call to `${}`", def.name),
                            t.span,
                        )
                        .with_chain(provenance, parent_exp));
                    }
                    args.push(&tokens[current_start..cursor]);
                    cursor += 1;
                    current_start = cursor;
                    continue;
                }
                match t.kind {
                    TokenKind::LParen => paren_depth += 1,
                    TokenKind::RParen => paren_depth = paren_depth.saturating_sub(1),
                    TokenKind::LBracket => bracket_depth += 1,
                    TokenKind::RBracket => bracket_depth = bracket_depth.saturating_sub(1),
                    TokenKind::LBrace => brace_depth += 1,
                    TokenKind::RBrace => brace_depth = brace_depth.saturating_sub(1),
                    _ => {}
                }
                cursor += 1;
            }

            let Some(rparen_idx) = close_idx else {
                return Err(MacroError::new(
                    MacroErrorKind::InvalidTemplateCall,
                    format!(
                        "unterminated argument list in call to `${}`: missing `)`",
                        def.name
                    ),
                    Span::new(dollar_tok.span.lo(), lparen.span.hi()),
                )
                .with_chain(provenance, parent_exp));
            };

            if args.len() != params.len() {
                return Err(MacroError::new(
                    MacroErrorKind::InvalidTemplateCall,
                    format!(
                        "template macro `${}` expects {} argument(s), got {}",
                        def.name,
                        params.len(),
                        args.len()
                    ),
                    Span::new(dollar_tok.span.lo(), tokens[rparen_idx].span.hi()),
                )
                .with_chain(provenance, parent_exp));
            }

            let call_span = Span::new(dollar_tok.span.lo(), tokens[rparen_idx].span.hi());
            let trailing_spacing = tokens[rparen_idx].spacing;
            let exp_id = provenance.record(
                ExpansionKind::Template,
                def.name.clone(),
                call_span,
                Some(def.def_site),
                parent_exp,
            );
            let expanded = substitute_group(&def.body, &args, budget, dollar_tok.span)
                .map_err(|err| err.with_chain(provenance, Some(exp_id)))?;
            Ok((expanded, exp_id, trailing_spacing, rparen_idx + 1))
        }
    }
}

fn substitute_group(
    body: &[TemplatePart],
    captured: &[&[MacroToken]],
    budget: &mut MacroBudget,
    call_span: Span,
) -> Result<Vec<MacroToken>, MacroError> {
    let mut output = Vec::with_capacity(body.len());
    for part in body {
        budget
            .consume_step()
            .map_err(|err| err.with_span(call_span))?;
        match part {
            TemplatePart::Literal(tok) => {
                budget
                    .consume_tokens(1)
                    .map_err(|err| err.with_span(call_span))?;
                output.push(tok.clone());
            }
            TemplatePart::EscapedDollar { span, spacing } => {
                budget
                    .consume_tokens(1)
                    .map_err(|err| err.with_span(call_span))?;
                output.push(MacroToken::new(
                    TokenKind::Dollar,
                    "$",
                    *span,
                    *spacing,
                    None,
                ));
            }
            TemplatePart::Param { index, spacing } => {
                let slice = captured[*index];
                budget
                    .consume_tokens(slice.len())
                    .map_err(|err| err.with_span(call_span))?;
                for (i, tok) in slice.iter().enumerate() {
                    let mut cloned = tok.clone();
                    cloned.expansion_id = None;
                    if i + 1 == slice.len() && cloned.spacing != TokenSpacing::Separated {
                        cloned.spacing = *spacing;
                    }
                    output.push(cloned);
                }
            }
        }
    }
    Ok(output)
}
