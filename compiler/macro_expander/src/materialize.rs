//! Materialize expanded `MacroToken` streams into synthetic `SourceFile`s and parser tokens.
use crate::provenance::{ExpansionMap, MacroToken, TokenSpacing};
use lex::{Token, TokenKind};
use rustc_span::{BytePos, FileName, FileNameDisplayPreference, SourceFile, SourceMap, Span};
use std::collections::HashMap;
use symbol::Symbol;

/// Result of preparing a single source file through macro expansion.
#[derive(Clone, Debug)]
pub struct ExpandedFile {
    pub tokens: Vec<Token>,
    pub symbols: HashMap<usize, Symbol>,
    pub start_pos: BytePos,
    pub expanded: bool,
    pub provenance: ExpansionMap,
    pub source_text: Option<String>,
}

/// Convert raw lexer output into comment-stripped `MacroToken`s preserving spacing & newlines.
pub fn tokens_to_macro_tokens(tokens: &[Token], src: &str, base_pos: BytePos) -> Vec<MacroToken> {
    let significant: Vec<Token> = tokens
        .iter()
        .copied()
        .filter(|tok| {
            !matches!(
                tok.kind,
                TokenKind::Sof | TokenKind::Eof | TokenKind::Comment
            )
        })
        .collect();

    let mut out = Vec::with_capacity(significant.len());
    for (idx, tok) in significant.iter().enumerate() {
        let text = src.get(tok.from..tok.to).unwrap_or("").to_string();
        let span = Span::new(
            base_pos + BytePos(tok.from as u32),
            base_pos + BytePos(tok.to as u32),
        );
        let spacing = if crate::provenance::is_separated_kind(tok.kind) {
            TokenSpacing::Separated
        } else if let Some(next) = significant.get(idx + 1) {
            if tok.to == next.from {
                TokenSpacing::Joint
            } else {
                let gap = src.get(tok.to..next.from).unwrap_or("");
                if gap.contains('\n') {
                    TokenSpacing::Newline
                } else {
                    TokenSpacing::Alone
                }
            }
        } else {
            TokenSpacing::Newline
        };
        out.push(MacroToken::new(tok.kind, text, span, spacing, None));
    }
    out
}

fn is_word_or_number(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Id
            | TokenKind::Int
            | TokenKind::IntBin
            | TokenKind::IntOct
            | TokenKind::IntHex
            | TokenKind::Real
            | TokenKind::RealSci
            | TokenKind::Underscore
    ) || is_keyword(kind)
}

fn is_literal_with_suffix_base(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Int
            | TokenKind::IntBin
            | TokenKind::IntOct
            | TokenKind::IntHex
            | TokenKind::Real
            | TokenKind::RealSci
            | TokenKind::Str
            | TokenKind::Char
    )
}

fn is_keyword(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::And
            | TokenKind::As
            | TokenKind::Assoc
            | TokenKind::Asserts
            | TokenKind::Assumes
            | TokenKind::Async
            | TokenKind::Atomic
            | TokenKind::Axiom
            | TokenKind::Await
            | TokenKind::Break
            | TokenKind::Case
            | TokenKind::Catch
            | TokenKind::Comptime
            | TokenKind::Defer
            | TokenKind::Const
            | TokenKind::Continue
            | TokenKind::Decreases
            | TokenKind::Define
            | TokenKind::Derive
            | TokenKind::Do
            | TokenKind::Dyn
            | TokenKind::Effect
            | TokenKind::Else
            | TokenKind::Ensures
            | TokenKind::Enum
            | TokenKind::Error
            | TokenKind::Exists
            | TokenKind::Extend
            | TokenKind::False
            | TokenKind::Fn
            | TokenKind::FnCap
            | TokenKind::For
            | TokenKind::Forall
            | TokenKind::Ghost
            | TokenKind::Handles
            | TokenKind::If
            | TokenKind::Implicit
            | TokenKind::Impl
            | TokenKind::In
            | TokenKind::Inline
            | TokenKind::Invariant
            | TokenKind::Is
            | TokenKind::Itself
            | TokenKind::Lambda
            | TokenKind::Lemma
            | TokenKind::Var
            | TokenKind::Let
            | TokenKind::Lift
            | TokenKind::Match
            | TokenKind::Matches
            | TokenKind::Move
            | TokenKind::Mod
            | TokenKind::Newtype
            | TokenKind::Not
            | TokenKind::Null
            | TokenKind::Opaque
            | TokenKind::Opens
            | TokenKind::Or
            | TokenKind::Outcomes
            | TokenKind::Predicate
            | TokenKind::Private
            | TokenKind::Pub
            | TokenKind::Pure
            | TokenKind::KwQuote
            | TokenKind::Ref
            | TokenKind::Refines
            | TokenKind::Requires
            | TokenKind::Resume
            | TokenKind::Return
            | TokenKind::SelfLower
            | TokenKind::SelfCap
            | TokenKind::Spec
            | TokenKind::Static
            | TokenKind::Struct
            | TokenKind::Template
            | TokenKind::Test
            | TokenKind::Trait
            | TokenKind::True
            | TokenKind::Typealias
            | TokenKind::Union
            | TokenKind::Unsafe
            | TokenKind::Undefined
            | TokenKind::Use
            | TokenKind::Verified
            | TokenKind::When
            | TokenKind::While
            | TokenKind::Where
    )
}

pub(crate) fn needs_safety_space(prev: &MacroToken, curr: &MacroToken) -> bool {
    if is_literal_with_suffix_base(prev.kind) && curr.kind == TokenKind::Id {
        return false;
    }
    if is_word_or_number(prev.kind) && is_word_or_number(curr.kind) {
        return true;
    }
    if (prev.kind == TokenKind::Minus && curr.kind == TokenKind::Minus)
        || (prev.kind == TokenKind::LBrace && curr.kind == TokenKind::Minus)
        || (prev.kind == TokenKind::Minus && curr.kind == TokenKind::RBrace)
    {
        return true;
    }
    // Avoid forming `{{`, `}}`, or `$$` across two unrelated substitution boundaries.
    if prev.expansion_id != curr.expansion_id
        && matches!(
            (prev.kind, curr.kind),
            (TokenKind::LBrace, TokenKind::LBrace)
                | (TokenKind::RBrace, TokenKind::RBrace)
                | (TokenKind::Dollar, TokenKind::Dollar)
        )
    {
        return true;
    }
    // Avoid fusing two joint punctuation tokens into a different compound token.
    matches!(
        (prev.kind, curr.kind),
        (
            TokenKind::Eq,
            TokenKind::Eq | TokenKind::EqEq | TokenKind::Gt
        ) | (TokenKind::EqEq, TokenKind::Eq)
            | (TokenKind::Plus, TokenKind::Plus | TokenKind::Eq)
            | (TokenKind::Minus, TokenKind::Eq | TokenKind::Gt)
            | (
                TokenKind::Star
                    | TokenKind::Slash
                    | TokenKind::Percent
                    | TokenKind::Bang
                    | TokenKind::Lt
                    | TokenKind::Gt,
                TokenKind::Eq
            )
            | (
                TokenKind::Colon,
                TokenKind::Colon | TokenKind::Tilde | TokenKind::Minus
            )
            | (TokenKind::Tilde | TokenKind::Pipe, TokenKind::Gt)
    )
}

/// Rendered token stream image together with per-token byte offsets inside `text`.
pub(crate) struct RenderedTokenStream {
    pub(crate) text: String,
    pub(crate) tokens: Vec<Token>,
    pub(crate) symbols: HashMap<usize, Symbol>,
    pub(crate) macro_token_ranges: Vec<(usize, usize)>,
}

/// Render `macro_tokens` into contiguous UTF-8 source text while preserving lexeme
/// boundaries, safety spaces, and separated operators.
pub(crate) fn render_macro_tokens(macro_tokens: &[MacroToken]) -> RenderedTokenStream {
    let mut expanded_src = String::new();
    let mut out_tokens = Vec::with_capacity(macro_tokens.len() + 2);
    let mut out_symbols = HashMap::new();
    let mut macro_token_ranges = Vec::with_capacity(macro_tokens.len());

    out_tokens.push(Token::new(TokenKind::Sof, 0, 0));

    for (idx, mt) in macro_tokens.iter().enumerate() {
        if idx > 0 {
            let prev = &macro_tokens[idx - 1];
            match prev.spacing {
                TokenSpacing::Newline => expanded_src.push('\n'),
                TokenSpacing::Alone | TokenSpacing::Separated => expanded_src.push(' '),
                TokenSpacing::Joint => {
                    if mt.spacing == TokenSpacing::Separated
                        || crate::provenance::is_separated_kind(prev.kind)
                        || crate::provenance::is_separated_kind(mt.kind)
                        || needs_safety_space(prev, mt)
                    {
                        expanded_src.push(' ');
                    }
                }
            }
        } else if mt.spacing == TokenSpacing::Separated
            || crate::provenance::is_separated_kind(mt.kind)
        {
            expanded_src.push(' ');
        }

        let from = expanded_src.len();
        expanded_src.push_str(&mt.text);
        let to = expanded_src.len();
        macro_token_ranges.push((from, to));

        let tok_idx = out_tokens.len();
        out_tokens.push(Token::new(mt.kind, from, to));
        if mt.kind == TokenKind::Id {
            out_symbols.insert(tok_idx, Symbol::intern(&mt.text));
        }
    }

    if macro_tokens
        .last()
        .is_some_and(|mt| mt.spacing == TokenSpacing::Separated)
    {
        expanded_src.push(' ');
    }

    let eof_pos = expanded_src.len();
    out_tokens.push(Token::new(TokenKind::Eof, eof_pos, eof_pos));

    RenderedTokenStream {
        text: expanded_src,
        tokens: out_tokens,
        symbols: out_symbols,
        macro_token_ranges,
    }
}

/// Materialize `macro_tokens` into a synthetic `SourceFile` registered in `source_map`.
pub fn materialize_expanded(
    source_map: &SourceMap,
    orig_file: &SourceFile,
    macro_tokens: &[MacroToken],
    mut provenance: ExpansionMap,
) -> ExpandedFile {
    let rendered = render_macro_tokens(macro_tokens);

    for (idx, mt) in macro_tokens.iter().enumerate() {
        if let Some(exp_id) = mt.expansion_id {
            let (from, to) = rendered.macro_token_ranges[idx];
            let mut cursor = Some(exp_id);
            let mut guard = 0usize;
            while let Some(id) = cursor {
                if guard > provenance.len() {
                    break;
                }
                guard += 1;
                provenance.extend_output_range(id, from, to);
                cursor = provenance.get(id).and_then(|rec| rec.parent);
            }
        }
    }

    let label = orig_file
        .name
        .display(FileNameDisplayPreference::Local)
        .to_string_lossy()
        .into_owned();
    let synthetic_name = FileName::Custom(format!(
        "<macro-expanded:{label}:{}>",
        orig_file.start_pos.0
    ));
    let synthetic_file = source_map.new_source_file(synthetic_name, rendered.text.clone());

    ExpandedFile {
        tokens: rendered.tokens,
        symbols: rendered.symbols,
        start_pos: synthetic_file.start_pos,
        expanded: true,
        provenance,
        source_text: Some(rendered.text),
    }
}
