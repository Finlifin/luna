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

/// Parse and execute a `@path { ... }` Mode 3 construction macro call starting at `cursor` (`TokenKind::At`).
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
            let Some(open_tok) = tokens.get(idx) else {
                return Err(MacroError::new(
                    MacroErrorKind::DelimiterMismatch,
                    format!(
                        "token macro `@{}` requires `{{ ... }}` brace delimiters",
                        desc.full_path()
                    ),
                    path_span,
                )
                .with_chain(provenance, parent_exp));
            };
            if open_tok.kind != TokenKind::LBrace {
                return Err(MacroError::new(
                    MacroErrorKind::DelimiterMismatch,
                    format!(
                        "token macro `@{}` requires `{{ ... }}` brace delimiters, found `{}`",
                        desc.full_path(),
                        open_tok.text
                    ),
                    open_tok.span,
                )
                .with_label(path_span, "macro called here")
                .with_chain(provenance, parent_exp));
            }

            let open_idx = idx;
            let mut brace_depth = 1usize;
            let mut scan = open_idx + 1;
            let mut close_idx = None;
            while scan < tokens.len() {
                match tokens[scan].kind {
                    TokenKind::LBrace => brace_depth += 1,
                    TokenKind::RBrace => {
                        brace_depth -= 1;
                        if brace_depth == 0 {
                            close_idx = Some(scan);
                            break;
                        }
                    }
                    _ => {}
                }
                scan += 1;
            }
            let Some(close_idx) = close_idx else {
                return Err(MacroError::new(
                    MacroErrorKind::UnterminatedMacroInput,
                    format!(
                        "unterminated token macro input `{{ ... }}` for `@{}`; expected closing `}}`",
                        desc.full_path()
                    ),
                    open_tok.span,
                )
                .with_label(path_span, "macro called here")
                .with_chain(provenance, parent_exp));
            };

            let close_tok = &tokens[close_idx];
            let call_span = Span::new(at_tok.span.lo(), close_tok.span.hi());
            if depth + 1 > budget.max_depth {
                return Err(MacroError::new(
                    MacroErrorKind::DepthExceeded,
                    format!(
                        "macro expansion depth limit ({}) exceeded while expanding `@{}`",
                        budget.max_depth,
                        desc.full_path()
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
                desc.full_path(),
                call_span,
                desc.def_site,
                parent_exp,
            );

            let Some(executor) = host.ctx.executor.as_deref_mut() else {
                return Err(MacroError::new(
                    MacroErrorKind::HandlerExecutionError,
                    format!(
                        "cannot evaluate `^macro3` construction macro `@{}` without a comptime macro executor",
                        desc.full_path()
                    ),
                    call_span,
                )
                .with_chain(provenance, Some(exp_id)));
            };

            let inner_tokens = &tokens[open_idx + 1..close_idx];
            let mut produced = executor
                .execute_tokens(&desc, inner_tokens, call_span, budget)
                .map_err(|e| e.with_chain_if_empty(provenance, Some(exp_id), call_span))?;

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

            Ok((produced, exp_id, close_tok.spacing, close_idx + 1))
        }
    }
}
