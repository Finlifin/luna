//! Flurry AST fragment argument parsing and `SyntaxTree` snapshot construction for Mode 1 macros.
use crate::materialize::render_macro_tokens;
use crate::provenance::{MacroToken, escape_sexp_str};
use crate::registry::AstNodeKind;
use crate::{MacroBudget, MacroError, MacroErrorKind};
use ast::{Ast, NodeIndex, node_kind_symbol_name};
use lex::{Symbol, Token, TokenKind};
use parse::fragment::FragmentKind;
use parse::parser::Parser;
use rustc_span::source_map::FilePathMapping;
use rustc_span::{BytePos, FileName, SourceMap, Span};
use std::collections::HashMap;
use std::fmt;

/// Parameter shape derived from a `@define_macro1` handler signature.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AstParamShape {
    Single(AstNodeKind),
    Optional(AstNodeKind),
    Repetition(AstNodeKind),
}

impl AstParamShape {
    pub fn node_kind(self) -> AstNodeKind {
        match self {
            Self::Single(kind) | Self::Optional(kind) | Self::Repetition(kind) => kind,
        }
    }
}

impl fmt::Display for AstParamShape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Single(kind) => write!(f, "(single {kind})"),
            Self::Optional(kind) => write!(f, "(optional {kind})"),
            Self::Repetition(kind) => write!(f, "(repetition {kind})"),
        }
    }
}

/// Compact 0-indexed node inside a parsed `ParsedSyntaxTree` snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedSyntaxNode {
    pub kind: String,
    pub span: Span,
    pub children: Vec<usize>,
    pub text: String,
}

impl fmt::Display for ParsedSyntaxNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(syntax-node (kind \"{}\") (span {} {}) (children",
            escape_sexp_str(&self.kind),
            self.span.lo().0,
            self.span.hi().0,
        )?;
        for child in &self.children {
            write!(f, " {child}")?;
        }
        write!(f, ") (text \"{}\"))", escape_sexp_str(&self.text))
    }
}

/// Parsed Flurry AST subtree with arena nodes and exact source tokens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedSyntaxTree {
    pub nodes: Vec<ParsedSyntaxNode>,
    pub root: usize,
    pub tokens: Vec<MacroToken>,
}

impl fmt::Display for ParsedSyntaxTree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(syntax-tree (root {}) (nodes", self.root)?;
        for node in &self.nodes {
            write!(f, " {node}")?;
        }
        write!(f, ") (tokens {}))", self.tokens.len())
    }
}

/// Parsed argument value bound to one Mode 1 handler parameter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AstArgValue {
    Single(ParsedSyntaxTree),
    Optional(Option<ParsedSyntaxTree>),
    Repetition(Vec<ParsedSyntaxTree>),
}

impl fmt::Display for AstArgValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Single(tree) => write!(f, "(arg-single {tree})"),
            Self::Optional(Some(tree)) => write!(f, "(arg-optional (some {tree}))"),
            Self::Optional(None) => f.write_str("(arg-optional null)"),
            Self::Repetition(trees) => {
                f.write_str("(arg-repetition")?;
                for tree in trees {
                    write!(f, " {tree}")?;
                }
                f.write_str(")")
            }
        }
    }
}

/// Convert `AstNodeKind` to `parse::fragment::FragmentKind`.
pub fn fragment_kind_for(kind: AstNodeKind) -> FragmentKind {
    match kind {
        AstNodeKind::FlurryPattern => FragmentKind::Pattern,
        AstNodeKind::FlurryExpr => FragmentKind::Expr,
        AstNodeKind::FlurryStatement => FragmentKind::Statement,
        AstNodeKind::FlurryExtendedStatement => FragmentKind::ExtendedStatement,
        AstNodeKind::FlurryItem => FragmentKind::Item,
        AstNodeKind::FlurryType => FragmentKind::Type,
        AstNodeKind::FlurryIdent => FragmentKind::Ident,
        AstNodeKind::FlurryLiteral => FragmentKind::Literal,
        AstNodeKind::FlurryBlock => FragmentKind::Block,
        AstNodeKind::FlurryPath => FragmentKind::Path,
    }
}

/// Build an isolated `SourceMap` and `Parser` token buffer from a `MacroToken` slice.
pub fn build_parser_input(
    tokens: &[MacroToken],
    label: &str,
) -> (
    SourceMap,
    String,
    BytePos,
    Vec<Token>,
    HashMap<usize, Symbol>,
    Vec<(usize, usize)>,
) {
    let rendered = render_macro_tokens(tokens);
    let source_map = SourceMap::new(FilePathMapping::empty());
    let source_file =
        source_map.new_source_file(FileName::Custom(label.to_string()), rendered.text.clone());
    (
        source_map,
        rendered.text,
        source_file.start_pos,
        rendered.tokens,
        rendered.symbols,
        rendered.macro_token_ranges,
    )
}

fn map_byte_range_to_caller_span(
    rel_lo: usize,
    rel_hi: usize,
    arg_tokens: &[MacroToken],
    tok_start: usize,
    tok_end: usize,
    token_byte_ranges: &[(usize, usize)],
    fallback_span: Span,
) -> Span {
    let mut first_span: Option<Span> = None;
    let mut last_span: Option<Span> = None;

    for idx in tok_start..tok_end.min(arg_tokens.len()) {
        if let Some(&(t_lo, t_hi)) = token_byte_ranges.get(idx) {
            if t_hi > rel_lo && t_lo < rel_hi {
                let s = arg_tokens[idx].span;
                if first_span.is_none() {
                    first_span = Some(s);
                }
                last_span = Some(s);
            }
        }
    }

    match (first_span, last_span) {
        (Some(lo), Some(hi)) => Span::new(lo.lo(), hi.hi()),
        _ => fallback_span,
    }
}

/// Snapshot a parsed AST subtree reachable from `root` into compact 0-indexed `ParsedSyntaxTree` data.
#[allow(clippy::too_many_arguments)]
pub fn snapshot_ast_subtree(
    ast: &Ast,
    root: NodeIndex,
    arg_tokens: &[MacroToken],
    tok_start: usize,
    tok_end: usize,
    src_text: &str,
    base_pos: BytePos,
    token_byte_ranges: &[(usize, usize)],
    fallback_span: Span,
) -> ParsedSyntaxTree {
    let slice_end = tok_end.min(arg_tokens.len());
    let slice_start = tok_start.min(slice_end);
    let tokens = arg_tokens[slice_start..slice_end].to_vec();

    let call_fallback = if !tokens.is_empty() {
        Span::new(tokens[0].span.lo(), tokens[tokens.len() - 1].span.hi())
    } else {
        fallback_span
    };

    let mut nodes: Vec<ParsedSyntaxNode> = Vec::new();
    let mut visited: HashMap<NodeIndex, usize> = HashMap::new();

    #[allow(clippy::too_many_arguments)]
    fn visit(
        ast: &Ast,
        node: NodeIndex,
        arg_tokens: &[MacroToken],
        slice_start: usize,
        slice_end: usize,
        src_text: &str,
        base_pos: BytePos,
        token_byte_ranges: &[(usize, usize)],
        fallback: Span,
        visited: &mut HashMap<NodeIndex, usize>,
        out: &mut Vec<ParsedSyntaxNode>,
    ) -> usize {
        if let Some(&existing) = visited.get(&node) {
            return existing;
        }
        let my_idx = out.len();
        visited.insert(node, my_idx);

        let kind = ast
            .get_node_kind(node)
            .map(node_kind_symbol_name)
            .unwrap_or_else(|| "invalid".to_string());
        let raw_span = ast.get_span(node).unwrap_or(rustc_span::DUMMY_SP);
        let rel_lo = raw_span
            .lo()
            .0
            .saturating_sub(base_pos.0)
            .min(src_text.len() as u32) as usize;
        let rel_hi = raw_span
            .hi()
            .0
            .saturating_sub(base_pos.0)
            .min(src_text.len() as u32) as usize;
        let text = if rel_lo < rel_hi {
            src_text.get(rel_lo..rel_hi).unwrap_or("").to_string()
        } else {
            String::new()
        };
        let mapped_span = map_byte_range_to_caller_span(
            rel_lo,
            rel_hi,
            arg_tokens,
            slice_start,
            slice_end,
            token_byte_ranges,
            fallback,
        );

        out.push(ParsedSyntaxNode {
            kind,
            span: mapped_span,
            children: Vec::new(),
            text,
        });

        let raw_children = ast.direct_child_nodes(node);
        let mut child_indices = Vec::with_capacity(raw_children.len());
        for child in raw_children {
            let c_idx = visit(
                ast,
                child,
                arg_tokens,
                slice_start,
                slice_end,
                src_text,
                base_pos,
                token_byte_ranges,
                fallback,
                visited,
                out,
            );
            child_indices.push(c_idx);
        }
        out[my_idx].children = child_indices;
        my_idx
    }

    let root_idx = visit(
        ast,
        root,
        arg_tokens,
        slice_start,
        slice_end,
        src_text,
        base_pos,
        token_byte_ranges,
        call_fallback,
        &mut visited,
        &mut nodes,
    );

    ParsedSyntaxTree {
        nodes,
        root: root_idx,
        tokens,
    }
}

fn parser_token_caller_span(
    tok_cursor_idx: usize,
    arg_tokens: &[MacroToken],
    call_site: Span,
) -> Span {
    arg_tokens.get(tok_cursor_idx).map_or(call_site, |t| t.span)
}

/// Parse Mode 1 `@path(...)` invocation arguments according to the handler's `params` shapes.
pub fn parse_ast_arguments(
    arg_tokens: &[MacroToken],
    params: &[AstParamShape],
    macro_name: &str,
    call_site: Span,
    budget: &mut MacroBudget,
) -> Result<Vec<AstArgValue>, MacroError> {
    let (source_map, src_text, base_pos, parser_tokens, parser_symbols, token_byte_ranges) =
        build_parser_input(arg_tokens, &format!("<macro-ast-args:{macro_name}>"));

    let mut parser = Parser::new(&source_map, parser_tokens, parser_symbols, base_pos);
    let mut parsed_args = Vec::with_capacity(params.len());
    let mut cursor_pos = 0usize;

    let map_parse_err =
        |err: parse::error::ParseError, fallback: Span, expected_desc: &str| -> MacroError {
            let raw_span = err.to_span();
            let rel_lo = raw_span
                .lo()
                .0
                .saturating_sub(base_pos.0)
                .min(src_text.len() as u32) as usize;
            let rel_hi = raw_span
                .hi()
                .0
                .saturating_sub(base_pos.0)
                .min(src_text.len() as u32) as usize;
            let caller_span = map_byte_range_to_caller_span(
                rel_lo,
                rel_hi,
                arg_tokens,
                0,
                arg_tokens.len(),
                &token_byte_ranges,
                fallback,
            );
            MacroError::new(
                MacroErrorKind::AstInputParseError,
                format!(
                    "invalid `{expected_desc}` argument in call to `@{macro_name}`: {}",
                    err.message()
                ),
                caller_span,
            )
        };

    for (param_idx, shape) in params.iter().enumerate() {
        budget.consume_step().map_err(|e| e.with_span(call_site))?;
        let node_kind = shape.node_kind();
        let frag_kind = fragment_kind_for(node_kind);

        match shape {
            AstParamShape::Single(_) => {
                if parser.peek_next_token().kind == TokenKind::Eof {
                    return Err(MacroError::new(
                        MacroErrorKind::AstInputParseError,
                        format!(
                            "missing `{node_kind}` argument #{} in call to `@{macro_name}`",
                            param_idx + 1
                        ),
                        call_site,
                    ));
                }
                let matched = parser
                    .parse_fragment(frag_kind)
                    .map_err(|e| {
                        map_parse_err(
                            e,
                            parser_token_caller_span(cursor_pos, arg_tokens, call_site),
                            node_kind.as_str(),
                        )
                    })?
                    .ok_or_else(|| {
                        MacroError::new(
                            MacroErrorKind::AstInputParseError,
                            format!(
                                "expected `{node_kind}` for argument #{} in call to `@{macro_name}`",
                                param_idx + 1
                            ),
                            parser_token_caller_span(cursor_pos, arg_tokens, call_site),
                        )
                    })?;
                cursor_pos = matched.end_cursor;
                let tree = snapshot_ast_subtree(
                    &parser.ast,
                    matched.root,
                    arg_tokens,
                    matched.start_cursor,
                    matched.end_cursor,
                    &src_text,
                    base_pos,
                    &token_byte_ranges,
                    call_site,
                );
                parsed_args.push(AstArgValue::Single(tree));

                if parser.peek_next_token().kind != TokenKind::Eof {
                    if !parser.eat_token(TokenKind::Comma) {
                        let span = parser_token_caller_span(cursor_pos, arg_tokens, call_site);
                        return Err(MacroError::new(
                            MacroErrorKind::AstInputParseError,
                            format!(
                                "expected `,` after `{node_kind}` argument #{} in call to `@{macro_name}`",
                                param_idx + 1
                            ),
                            span,
                        ));
                    }
                    cursor_pos += 1;
                }
            }
            AstParamShape::Optional(_) => {
                if parser.peek_next_token().kind == TokenKind::Eof {
                    parsed_args.push(AstArgValue::Optional(None));
                } else {
                    let matched = parser
                        .parse_fragment(frag_kind)
                        .map_err(|e| {
                            map_parse_err(
                                e,
                                parser_token_caller_span(cursor_pos, arg_tokens, call_site),
                                node_kind.as_str(),
                            )
                        })?
                        .ok_or_else(|| {
                            MacroError::new(
                                MacroErrorKind::AstInputParseError,
                                format!(
                                    "expected optional `{node_kind}` for argument #{} in call to `@{macro_name}`",
                                    param_idx + 1
                                ),
                                parser_token_caller_span(cursor_pos, arg_tokens, call_site),
                            )
                        })?;
                    cursor_pos = matched.end_cursor;
                    let tree = snapshot_ast_subtree(
                        &parser.ast,
                        matched.root,
                        arg_tokens,
                        matched.start_cursor,
                        matched.end_cursor,
                        &src_text,
                        base_pos,
                        &token_byte_ranges,
                        call_site,
                    );
                    parsed_args.push(AstArgValue::Optional(Some(tree)));

                    if parser.peek_next_token().kind != TokenKind::Eof {
                        if !parser.eat_token(TokenKind::Comma) {
                            let span = parser_token_caller_span(cursor_pos, arg_tokens, call_site);
                            return Err(MacroError::new(
                                MacroErrorKind::AstInputParseError,
                                format!(
                                    "expected `,` after `{node_kind}` argument #{} in call to `@{macro_name}`",
                                    param_idx + 1
                                ),
                                span,
                            ));
                        }
                        cursor_pos += 1;
                    }
                }
            }
            AstParamShape::Repetition(_) => {
                let mut items = Vec::new();
                while parser.peek_next_token().kind != TokenKind::Eof {
                    budget.consume_step().map_err(|e| e.with_span(call_site))?;
                    let matched = parser
                        .parse_fragment(frag_kind)
                        .map_err(|e| {
                            map_parse_err(
                                e,
                                parser_token_caller_span(cursor_pos, arg_tokens, call_site),
                                node_kind.as_str(),
                            )
                        })?
                        .ok_or_else(|| {
                            MacroError::new(
                                MacroErrorKind::AstInputParseError,
                                format!(
                                    "expected `{node_kind}` in variadic argument of `@{macro_name}`"
                                ),
                                parser_token_caller_span(cursor_pos, arg_tokens, call_site),
                            )
                        })?;
                    cursor_pos = matched.end_cursor;
                    items.push(snapshot_ast_subtree(
                        &parser.ast,
                        matched.root,
                        arg_tokens,
                        matched.start_cursor,
                        matched.end_cursor,
                        &src_text,
                        base_pos,
                        &token_byte_ranges,
                        call_site,
                    ));
                    if parser.peek_next_token().kind == TokenKind::Eof {
                        break;
                    }
                    if !parser.eat_token(TokenKind::Comma) {
                        let span = parser_token_caller_span(cursor_pos, arg_tokens, call_site);
                        return Err(MacroError::new(
                            MacroErrorKind::AstInputParseError,
                            format!(
                                "expected `,` between `{node_kind}` arguments in call to `@{macro_name}`"
                            ),
                            span,
                        ));
                    }
                    cursor_pos += 1;
                }
                parsed_args.push(AstArgValue::Repetition(items));
            }
        }
    }

    if parser.peek_next_token().kind != TokenKind::Eof {
        let span = parser_token_caller_span(cursor_pos, arg_tokens, call_site);
        return Err(MacroError::new(
            MacroErrorKind::AstInputParseError,
            format!("unexpected extra argument in call to `@{macro_name}`"),
            span,
        ));
    }

    Ok(parsed_args)
}
