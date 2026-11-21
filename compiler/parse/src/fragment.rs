//! Thin single-node parser entrypoints for macro AST construction and PEG delegation.
use crate::error::ParseError;
use crate::parser::{Cursor, ParseResult, Parser};
use ast::{NodeBuilder, NodeIndex, NodeKind};
use lex::TokenKind;
use std::fmt;

/// Syntactic category selector for single-node Flurry parsing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FragmentKind {
    Pattern,
    Expr,
    Statement,
    ExtendedStatement,
    Item,
    Type,
    Ident,
    Literal,
    Block,
    Path,
}

impl FragmentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pattern => "pattern",
            Self::Expr => "expr",
            Self::Statement => "statement",
            Self::ExtendedStatement => "extended-statement",
            Self::Item => "item",
            Self::Type => "type",
            Self::Ident => "ident",
            Self::Literal => "literal",
            Self::Block => "block",
            Self::Path => "path",
        }
    }
}

impl fmt::Display for FragmentKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Successful single-node parse spanning token cursors `[start_cursor, end_cursor)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FragmentMatch {
    pub root: NodeIndex,
    pub start_cursor: Cursor,
    pub end_cursor: Cursor,
}

impl fmt::Display for FragmentMatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(fragment-match (root {}) (tokens {} {}))",
            self.root, self.start_cursor, self.end_cursor
        )
    }
}

fn is_definition_only_node_kind(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Function
            | NodeKind::NormalFormHeadDef
            | NodeKind::AlgebraicEffect
            | NodeKind::EffectEnumDef
            | NodeKind::StructDef
            | NodeKind::EnumDef
            | NodeKind::UnionDef
            | NodeKind::TraitDef
            | NodeKind::ImplDef
            | NodeKind::ImplTraitDef
            | NodeKind::ExtendDef
            | NodeKind::ExtendTraitDef
            | NodeKind::DeriveDef
            | NodeKind::DeriverDef
            | NodeKind::CaseDef
            | NodeKind::TypealiasDef
            | NodeKind::NewtypeDef
            | NodeKind::ModuleDef
            | NodeKind::ModStatement
            | NodeKind::TestDef
            | NodeKind::TestTemplateDef
            | NodeKind::FunctionTemplateDef
            | NodeKind::FunctionTemplateInstance
            | NodeKind::FunctionTemplateInstanceWithParams
            | NodeKind::TestTemplateInstance
    )
}

fn is_item_node_kind(kind: NodeKind) -> bool {
    is_definition_only_node_kind(kind)
        || matches!(
            kind,
            NodeKind::ConstDef | NodeKind::ConstDecl | NodeKind::UseStatement
        )
}

fn unwrap_modifiers(ast: &ast::Ast, mut node: NodeIndex) -> NodeIndex {
    while let Some(kind) = ast.get_node_kind(node) {
        let children = ast.get_children(node);
        match kind {
            NodeKind::Attribute | NodeKind::AttributeSetTrue if children.len() == 2 => {
                node = children[1];
            }
            NodeKind::Pub | NodeKind::Private if !children.is_empty() => {
                node = children[0];
            }
            _ => break,
        }
    }
    node
}

impl Parser<'_> {
    /// Parse a single Flurry syntactic unit starting at `self.cursor`.
    ///
    /// Returns `Ok(Some(match))` advancing `self.cursor` and retaining the parsed
    /// AST nodes on success. On `Ok(None)` or `Err(err)`, all parser state and AST
    /// allocations are rolled back cleanly so callers (such as PEG ordered choice)
    /// can backtrack without side effects.
    pub fn parse_fragment(
        &mut self,
        kind: FragmentKind,
    ) -> Result<Option<FragmentMatch>, ParseError> {
        let start_cursor = self.cursor;
        let saved_stack_len = self.cursor_stack.len();
        let saved_quote_holes = self.quote_holes.clone();
        let saved_errors_len = self.errors.len();
        let saved_warnings_len = self.warnings.len();
        let saved_parens = self.parenthesized_nodes.clone();
        let saved_warned = self.warned_nodes.clone();
        let saved_recovery = self.recovery_enabled;
        let saved_angle = self.angle_bracket_depth;
        let saved_nodes_len = self.ast.nodes.len();
        let saved_spans_len = self.ast.spans.len();
        let saved_cstart_len = self.ast.children_start.len();
        let saved_clen_len = self.ast.children_len.len();
        let saved_children_len = self.ast.children.len();
        let saved_root = self.ast.root;

        let rollback = |p: &mut Self| {
            p.cursor = start_cursor;
            p.cursor_stack.truncate(saved_stack_len);
            p.quote_holes = saved_quote_holes;
            p.errors.truncate(saved_errors_len);
            p.warnings.truncate(saved_warnings_len);
            p.parenthesized_nodes = saved_parens;
            p.warned_nodes = saved_warned;
            p.recovery_enabled = saved_recovery;
            p.angle_bracket_depth = saved_angle;
            p.ast.nodes.truncate(saved_nodes_len);
            p.ast.spans.truncate(saved_spans_len);
            p.ast.children_start.truncate(saved_cstart_len);
            p.ast.children_len.truncate(saved_clen_len);
            p.ast.children.truncate(saved_children_len);
            p.ast.root = saved_root;
        };

        self.recovery_enabled = false;
        let outcome = self.parse_fragment_inner(kind);
        self.recovery_enabled = saved_recovery;

        match outcome {
            Ok(root)
                if root != 0
                    && self.cursor > start_cursor
                    && self.errors.len() == saved_errors_len =>
            {
                Ok(Some(FragmentMatch {
                    root,
                    start_cursor,
                    end_cursor: self.cursor,
                }))
            }
            Ok(_) => {
                let recorded_err = if self.errors.len() > saved_errors_len {
                    self.errors.get(saved_errors_len).cloned()
                } else {
                    None
                };
                rollback(self);
                if let Some(err) = recorded_err {
                    Err(err)
                } else {
                    Ok(None)
                }
            }
            Err(err) if err.is_internal_sentinel() => {
                rollback(self);
                Ok(None)
            }
            Err(err) => {
                rollback(self);
                Err(err)
            }
        }
    }

    /// Parse a single Flurry fragment and verify it consumes the entire token stream
    /// up to `TokenKind::Eof`.
    pub fn parse_single_fragment(
        &mut self,
        kind: FragmentKind,
    ) -> Result<FragmentMatch, ParseError> {
        let matched = self.parse_fragment(kind)?.ok_or_else(|| {
            ParseError::invalid_syntax(
                format!("expected Flurry `{kind}`"),
                self.peek_next_token().kind,
                self.next_token_span(),
            )
        })?;
        let next = self.peek_next_token();
        if next.kind != TokenKind::Eof {
            return Err(ParseError::unexpected_token(
                TokenKind::Eof,
                next.kind,
                self.next_token_span(),
            )
            .with_primary_label(format!(
                "unexpected trailing token after `{kind}`: found {}",
                crate::error::describe_token(next.kind)
            )));
        }
        Ok(matched)
    }

    fn parse_fragment_inner(&mut self, kind: FragmentKind) -> ParseResult {
        match kind {
            FragmentKind::Pattern => self.try_pattern(),
            FragmentKind::Expr | FragmentKind::Type => self.try_expr(),
            FragmentKind::Ident => self.try_id(),
            FragmentKind::Literal => self.try_fragment_literal(),
            FragmentKind::Block => self.try_block(),
            FragmentKind::Path => self.try_path(),
            FragmentKind::Statement => {
                let (inner, attrs) = self.try_attributed_statement_or_definition()?;
                if inner == 0 {
                    return Ok(0);
                }
                let core = unwrap_modifiers(&self.ast, inner);
                if let Some(core_kind) = self.ast.get_node_kind(core) {
                    if is_definition_only_node_kind(core_kind) {
                        return Err(ParseError::invalid_syntax(
                            "expected a statement, found a definition".to_string(),
                            self.current_token().kind,
                            self.ast
                                .get_span(core)
                                .unwrap_or_else(|| self.current_span()),
                        ));
                    }
                }
                let wrapped = if attrs.is_empty() {
                    inner
                } else {
                    self.wrap_with_attributes(inner, &attrs)
                };
                self.eat_token(TokenKind::Semi);
                Ok(wrapped)
            }
            FragmentKind::ExtendedStatement => {
                let (inner, attrs) = self.try_attributed_statement_or_definition()?;
                if inner == 0 {
                    return Ok(0);
                }
                let wrapped = if attrs.is_empty() {
                    inner
                } else {
                    self.wrap_with_attributes(inner, &attrs)
                };
                self.eat_token(TokenKind::Semi);
                Ok(wrapped)
            }
            FragmentKind::Item => {
                let (inner, attrs) = self.try_attributed_statement_or_definition()?;
                if inner == 0 {
                    return Ok(0);
                }
                let core = unwrap_modifiers(&self.ast, inner);
                let is_item = self.ast.get_node_kind(core).is_some_and(is_item_node_kind);
                if !is_item {
                    return Err(ParseError::invalid_syntax(
                        "expected an item/definition".to_string(),
                        self.current_token().kind,
                        self.ast
                            .get_span(core)
                            .unwrap_or_else(|| self.current_span()),
                    ));
                }
                let wrapped = if attrs.is_empty() {
                    inner
                } else {
                    self.wrap_with_attributes(inner, &attrs)
                };
                self.eat_token(TokenKind::Semi);
                Ok(wrapped)
            }
        }
    }

    fn try_attributed_statement_or_definition(
        &mut self,
    ) -> Result<(NodeIndex, Vec<NodeIndex>), ParseError> {
        let attrs = self.try_attribute_prefix_chain()?;
        let inner = self.try_statement_or_definition()?;
        if inner == 0 && !attrs.is_empty() {
            return Err(ParseError::invalid_syntax(
                "expected a statement or definition after `^attribute`".to_string(),
                self.peek_next_token().kind,
                self.next_token_span(),
            ));
        }
        Ok((inner, attrs))
    }

    fn try_fragment_literal(&mut self) -> ParseResult {
        if self.peek(&[TokenKind::Dot, TokenKind::Id]) {
            return self.try_symbol();
        }
        let next = self.peek_next_token();
        if !matches!(
            next.kind,
            TokenKind::Int
                | TokenKind::IntBin
                | TokenKind::IntOct
                | TokenKind::IntHex
                | TokenKind::Real
                | TokenKind::RealSci
                | TokenKind::Str
                | TokenKind::Char
                | TokenKind::True
                | TokenKind::False
                | TokenKind::Null
        ) {
            return Ok(0);
        }

        let base = self.try_atomic()?;
        if base == 0 {
            return Ok(0);
        }
        if matches!(
            self.ast.get_node_kind(base),
            Some(
                NodeKind::Int
                    | NodeKind::Real
                    | NodeKind::Str
                    | NodeKind::StringConcat
                    | NodeKind::Char
            )
        ) && self.peek_next_token().kind == TokenKind::Id
            && self.current_token().to == self.peek_next_token().from
        {
            let suffix = self.try_id()?;
            if suffix != 0 {
                let span = rustc_span::Span::new(
                    self.ast.get_span(base).unwrap().lo(),
                    self.ast.get_span(suffix).unwrap().hi(),
                );
                return Ok(NodeBuilder::new(NodeKind::LiteralExtension, span)
                    .add_single_child(base)
                    .add_single_child(suffix)
                    .build(&mut self.ast));
            }
        }
        Ok(base)
    }
}
