//! Construction macro token-stream expansion (`^macro3` / Mode 3) interleaved with template macros.
use crate::imports::MacroScope;
use crate::provenance::{ExpansionId, ExpansionKind, ExpansionMap, MacroToken, TokenSpacing};
use crate::registry::{ExpansionContext, MacroInputMode};
use crate::template::expand_tokens_with_host;
use crate::{MacroBudget, MacroError, MacroErrorKind};
use lex::TokenKind;
use rustc_span::Span;

/// Mutable expansion context passed into token-stream macro expansion for `@path { ... }` calls.
pub struct ConstructionHost<'a, 'b> {
    pub scope: &'a MacroScope,
    pub ctx: &'a mut ExpansionContext<'b>,
}

/// Expand all file-local template macros (`define` / `$name`) and Mode 3 (`^macro3`) construction
/// macro calls in `input`. Returns `(tokens, expanded_anything)`.
pub fn expand_all_tokens(
    input: Vec<MacroToken>,
    scope: &MacroScope,
    ctx: &mut ExpansionContext<'_>,
    provenance: &mut ExpansionMap,
    budget: &mut MacroBudget,
) -> Result<(Vec<MacroToken>, bool), MacroError> {
    let mut host = ConstructionHost { scope, ctx };
    expand_tokens_with_host(&input, Some(&mut host), provenance, budget)
}

fn find_delimited_span(
    tokens: &[MacroToken],
    open_idx: usize,
    path_span: Span,
    open_kind: TokenKind,
    close_kind: TokenKind,
    macro_label: &str,
    macro_name: &str,
    delim_desc: &str,
    close_desc: &str,
    provenance: &ExpansionMap,
    parent_exp: Option<ExpansionId>,
) -> Result<(usize, usize), MacroError> {
    let Some(open_tok) = tokens.get(open_idx) else {
        return Err(MacroError::new(
            MacroErrorKind::DelimiterMismatch,
            format!("{macro_label} `@{macro_name}` requires `{delim_desc}` delimiters"),
            path_span,
        )
        .with_chain(provenance, parent_exp));
    };
    if open_tok.kind != open_kind {
        return Err(MacroError::new(
            MacroErrorKind::DelimiterMismatch,
            format!(
                "{macro_label} `@{macro_name}` requires `{delim_desc}` delimiters, found `{}`",
                open_tok.text
            ),
            open_tok.span,
        )
        .with_label(path_span, "macro called here")
        .with_chain(provenance, parent_exp));
    }

    let mut depth = 1usize;
    let mut scan = open_idx + 1;
    while scan < tokens.len() {
        if tokens[scan].kind == open_kind {
            depth += 1;
        } else if tokens[scan].kind == close_kind {
            depth -= 1;
            if depth == 0 {
                return Ok((open_idx, scan));
            }
        }
        scan += 1;
    }

    Err(MacroError::new(
        MacroErrorKind::UnterminatedMacroInput,
        format!(
            "unterminated {macro_label} input `{delim_desc}` for `@{macro_name}`; expected closing `{close_desc}`"
        ),
        open_tok.span,
    )
    .with_label(path_span, "macro called here")
    .with_chain(provenance, parent_exp))
}

fn finish_macro_call_output(
    mut produced: Vec<MacroToken>,
    exp_id: ExpansionId,
    call_span: Span,
    close_spacing: TokenSpacing,
    next_cursor: usize,
    provenance: &ExpansionMap,
    budget: &mut MacroBudget,
) -> Result<(Vec<MacroToken>, ExpansionId, TokenSpacing, usize), MacroError> {
    budget
        .consume_tokens(produced.len())
        .map_err(|e| e.with_chain_if_empty(provenance, Some(exp_id), call_span))?;

    for tok in &mut produced {
        if tok.expansion_id.is_none() {
            tok.expansion_id = Some(exp_id);
        }
        if tok.span == rustc_span::DUMMY_SP {
            tok.span = call_span;
        }
    }

    Ok((produced, exp_id, close_spacing, next_cursor))
}

/// Parse and execute a Mode 1 / Mode 2 / Mode 3 construction macro call starting at `cursor` (`TokenKind::At`).
/// Returns `(produced_tokens, exp_id, trailing_spacing, next_cursor)`.
pub(crate) fn expand_at_call(
    tokens: &[MacroToken],
    cursor: usize,
    host: &mut ConstructionHost<'_, '_>,
    provenance: &mut ExpansionMap,
    budget: &mut MacroBudget,
    depth: usize,
    parent_exp: Option<ExpansionId>,
) -> Result<(Vec<MacroToken>, ExpansionId, TokenSpacing, usize), MacroError> {
    let at_tok = &tokens[cursor];
    debug_assert_eq!(at_tok.kind, TokenKind::At);

    let Some(first_tok) = tokens.get(cursor + 1) else {
        return Err(MacroError::new(
            MacroErrorKind::UndefinedMacro,
            "unexpected `@` at end of input; expected macro path",
            at_tok.span,
        )
        .with_chain(provenance, parent_exp));
    };
    if first_tok.kind != TokenKind::Id {
        return Err(MacroError::new(
            MacroErrorKind::UndefinedMacro,
            format!(
                "expected macro identifier after `@`, found `{}`",
                first_tok.text
            ),
            first_tok.span,
        )
        .with_chain(provenance, parent_exp));
    }

    let mut segments = vec![first_tok.text.clone()];
    let mut idx = cursor + 2;
    let mut path_hi = first_tok.span.hi();

    while tokens.get(idx).is_some_and(|t| t.kind == TokenKind::Dot)
        && tokens.get(idx + 1).is_some_and(|t| t.kind == TokenKind::Id)
    {
        let seg = &tokens[idx + 1];
        segments.push(seg.text.clone());
        path_hi = seg.span.hi();
        idx += 2;
    }

    let path_span = Span::new(at_tok.span.lo(), path_hi);
    let desc = host
        .scope
        .resolve_call_path(&segments, path_span, host.ctx)
        .map_err(|err| err.with_chain(provenance, parent_exp))?
        .clone();

    match desc.mode {
        MacroInputMode::RawSource | MacroInputMode::BuiltinRawStr => Err(MacroError::new(
            MacroErrorKind::HandlerExecutionError,
            format!(
                "raw-source macro `@{}` requires raw pre-lexing source text and cannot be invoked from generated token streams",
                desc.full_path()
            ),
            path_span,
        )
        .with_chain(provenance, parent_exp)),
        MacroInputMode::Tokens => {
            let full_name = desc.full_path();
            let (open_idx, close_idx) = find_delimited_span(
                tokens,
                idx,
                path_span,
                TokenKind::LBrace,
                TokenKind::RBrace,
                "token macro",
                &full_name,
                "{ ... }",
                "}",
                provenance,
                parent_exp,
            )?;
            let close_tok = &tokens[close_idx];
            let call_span = Span::new(at_tok.span.lo(), close_tok.span.hi());
            if depth + 1 > budget.max_depth {
                return Err(MacroError::new(
                    MacroErrorKind::DepthExceeded,
                    format!(
                        "macro expansion depth limit ({}) exceeded while expanding `@{full_name}`",
                        budget.max_depth
                    ),
                    call_span,
                )
                .with_chain(provenance, parent_exp));
            }
            budget
                .consume_step()
                .map_err(|e| e.with_chain_if_empty(provenance, parent_exp, call_span))?;
            let exp_id = provenance.record(
                ExpansionKind::ConstructionTokens,
                full_name.clone(),
                call_span,
                desc.def_site,
                parent_exp,
            );
            let Some(executor) = host.ctx.executor.as_deref_mut() else {
                return Err(MacroError::new(
                    MacroErrorKind::HandlerExecutionError,
                    format!(
                        "cannot evaluate `^macro3` construction macro `@{full_name}` without a comptime macro executor"
                    ),
                    call_span,
                )
                .with_chain(provenance, Some(exp_id)));
            };
            let inner_tokens = &tokens[open_idx + 1..close_idx];
            let produced = executor
                .execute_tokens(&desc, inner_tokens, call_span, budget)
                .map_err(|e| e.with_chain_if_empty(provenance, Some(exp_id), call_span))?;
            finish_macro_call_output(
                produced,
                exp_id,
                call_span,
                close_tok.spacing,
                close_idx + 1,
                provenance,
                budget,
            )
        }
        MacroInputMode::BuiltinDefineMacro1 => Err(MacroError::new(
            MacroErrorKind::AstInputParseError,
            format!(
                "`@{}` must be bound to a `const` item: `[pub] const name = @define_macro1 {{ fn(...) -> TokenBuffer {{ ... }} }}`",
                desc.full_path()
            ),
            path_span,
        )
        .with_chain(provenance, parent_exp)),
        MacroInputMode::BuiltinDefineMacro2 => Err(MacroError::new(
            MacroErrorKind::PegGrammarError,
            format!(
                "`@{}` must be bound to a `const` item: `[pub] const name = @define_macro2 {{ Entry -> ...; fn(...) -> TokenBuffer {{ ... }} }}`",
                desc.full_path()
            ),
            path_span,
        )
        .with_chain(provenance, parent_exp)),
        MacroInputMode::Ast { ref params } => {
            let full_name = desc.full_path();
            let (open_idx, close_idx) = find_delimited_span(
                tokens,
                idx,
                path_span,
                TokenKind::LParen,
                TokenKind::RParen,
                "AST macro",
                &full_name,
                "(...)",
                ")",
                provenance,
                parent_exp,
            )?;
            let close_tok = &tokens[close_idx];
            let call_span = Span::new(at_tok.span.lo(), close_tok.span.hi());
            if depth + 1 > budget.max_depth {
                return Err(MacroError::new(
                    MacroErrorKind::DepthExceeded,
                    format!(
                        "macro expansion depth limit ({}) exceeded while expanding `@{full_name}`",
                        budget.max_depth
                    ),
                    call_span,
                )
                .with_chain(provenance, parent_exp));
            }
            budget
                .consume_step()
                .map_err(|e| e.with_chain_if_empty(provenance, parent_exp, call_span))?;
            let exp_id = provenance.record(
                ExpansionKind::ConstructionAst,
                full_name.clone(),
                call_span,
                desc.def_site,
                parent_exp,
            );
            let arg_tokens = &tokens[open_idx + 1..close_idx];
            let parsed_args = crate::ast_input::parse_ast_arguments(
                arg_tokens,
                params,
                &full_name,
                call_span,
                budget,
            )
            .map_err(|e| e.with_chain_if_empty(provenance, Some(exp_id), call_span))?;
            let Some(executor) = host.ctx.executor.as_deref_mut() else {
                return Err(MacroError::new(
                    MacroErrorKind::HandlerExecutionError,
                    format!(
                        "cannot evaluate AST macro `@{full_name}` without a comptime macro executor"
                    ),
                    call_span,
                )
                .with_chain(provenance, Some(exp_id)));
            };
            let produced = executor
                .execute_ast(&desc, &parsed_args, call_span, budget)
                .map_err(|e| e.with_chain_if_empty(provenance, Some(exp_id), call_span))?;
            finish_macro_call_output(
                produced,
                exp_id,
                call_span,
                close_tok.spacing,
                close_idx + 1,
                provenance,
                budget,
            )
        }
        MacroInputMode::Peg {
            ref grammar,
            param_shape,
        } => {
            let full_name = desc.full_path();
            let (open_idx, close_idx) = find_delimited_span(
                tokens,
                idx,
                path_span,
                TokenKind::LBrace,
                TokenKind::RBrace,
                "PEG macro",
                &full_name,
                "{ ... }",
                "}",
                provenance,
                parent_exp,
            )?;
            let close_tok = &tokens[close_idx];
            let call_span = Span::new(at_tok.span.lo(), close_tok.span.hi());
            if depth + 1 > budget.max_depth {
                return Err(MacroError::new(
                    MacroErrorKind::DepthExceeded,
                    format!(
                        "macro expansion depth limit ({}) exceeded while expanding `@{full_name}`",
                        budget.max_depth
                    ),
                    call_span,
                )
                .with_chain(provenance, parent_exp));
            }
            budget
                .consume_step()
                .map_err(|e| e.with_chain_if_empty(provenance, parent_exp, call_span))?;
            let exp_id = provenance.record(
                ExpansionKind::ConstructionPeg,
                full_name.clone(),
                call_span,
                desc.def_site,
                parent_exp,
            );
            let inner_tokens = &tokens[open_idx + 1..close_idx];
            let parsed_arg = crate::peg::parse_peg_input(
                inner_tokens,
                grammar,
                param_shape,
                &full_name,
                call_span,
                budget,
            )
            .map_err(|e| e.with_chain_if_empty(provenance, Some(exp_id), call_span))?;
            let Some(executor) = host.ctx.executor.as_deref_mut() else {
                return Err(MacroError::new(
                    MacroErrorKind::HandlerExecutionError,
                    format!(
                        "cannot evaluate PEG macro `@{full_name}` without a comptime macro executor"
                    ),
                    call_span,
                )
                .with_chain(provenance, Some(exp_id)));
            };
            let produced = executor
                .execute_peg(&desc, &parsed_arg, call_span, budget)
                .map_err(|e| e.with_chain_if_empty(provenance, Some(exp_id), call_span))?;
            finish_macro_call_output(
                produced,
                exp_id,
                call_span,
                close_tok.spacing,
                close_idx + 1,
                provenance,
                budget,
            )
        }
    }
}
