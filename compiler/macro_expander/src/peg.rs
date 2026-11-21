//! Mode 2 PEG grammar AST, static left-recursion validator, and packrat memoizing interpreter.
use crate::ast_input::{
    ParsedSyntaxTree, build_parser_input, fragment_kind_for, snapshot_ast_subtree,
};
use crate::materialize::tokens_to_macro_tokens;
use crate::provenance::{MacroToken, escape_sexp_str};
use crate::registry::AstNodeKind;
use crate::{MacroBudget, MacroError, MacroErrorKind};
use lex::TokenKind;
use parse::parser::Parser;
use rustc_span::{BytePos, Span};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;

/// Handler input shape permitted for a `@define_macro2` construction macro.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PegParamShape {
    PegNode,
    Single(AstNodeKind),
    Optional(AstNodeKind),
    Repetition(AstNodeKind),
}

impl fmt::Display for PegParamShape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PegNode => f.write_str("peg-node"),
            Self::Single(kind) => write!(f, "(single {kind})"),
            Self::Optional(kind) => write!(f, "(optional {kind})"),
            Self::Repetition(kind) => write!(f, "(repetition {kind})"),
        }
    }
}

/// PEG grammar expression node with deterministic Lisp S-expression `Display`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PegExpr {
    Literal(String),
    TokenKind(TokenKind),
    FlurryNode(AstNodeKind),
    RuleRef(String),
    Seq(Vec<PegExpr>),
    Choice(Vec<PegExpr>),
    ZeroOrMore(Box<PegExpr>),
    OneOrMore(Box<PegExpr>),
    Optional(Box<PegExpr>),
    AndPred(Box<PegExpr>),
    NotPred(Box<PegExpr>),
}

impl PegExpr {
    fn precedence(&self) -> u8 {
        match self {
            Self::Choice(_) => 1,
            Self::Seq(_) => 2,
            Self::AndPred(_) | Self::NotPred(_) => 3,
            Self::ZeroOrMore(_) | Self::OneOrMore(_) | Self::Optional(_) => 4,
            Self::Literal(_) | Self::TokenKind(_) | Self::FlurryNode(_) | Self::RuleRef(_) => 5,
        }
    }

    fn write_peg_syntax(&self, out: &mut String, min_prec: u8) {
        let needs_parens = self.precedence() < min_prec;
        if needs_parens {
            out.push('(');
        }
        match self {
            Self::Literal(text) => {
                out.push_str(&crate::raw_source::encode_flurry_string_literal(text));
            }
            Self::TokenKind(kind) => {
                out.push('.');
                out.push_str(kind.symbol_name());
            }
            Self::FlurryNode(kind) => {
                out.push_str(kind.as_str());
            }
            Self::RuleRef(name) => {
                out.push_str(name);
            }
            Self::Seq(items) => {
                for (idx, item) in items.iter().enumerate() {
                    if idx > 0 {
                        out.push_str(" ~ ");
                    }
                    item.write_peg_syntax(out, 3);
                }
            }
            Self::Choice(alts) => {
                for (idx, alt) in alts.iter().enumerate() {
                    if idx > 0 {
                        out.push_str(" | ");
                    }
                    alt.write_peg_syntax(out, 2);
                }
            }
            Self::ZeroOrMore(inner) => {
                inner.write_peg_syntax(out, 5);
                out.push('*');
            }
            Self::OneOrMore(inner) => {
                inner.write_peg_syntax(out, 5);
                out.push('+');
            }
            Self::Optional(inner) => {
                inner.write_peg_syntax(out, 5);
                out.push('?');
            }
            Self::AndPred(inner) => {
                out.push('&');
                inner.write_peg_syntax(out, 4);
            }
            Self::NotPred(inner) => {
                out.push('!');
                inner.write_peg_syntax(out, 4);
            }
        }
        if needs_parens {
            out.push(')');
        }
    }

    fn expected_summary(&self) -> String {
        match self {
            Self::Literal(text) => format!("{text:?}"),
            Self::TokenKind(kind) => format!(".{}", kind.symbol_name()),
            Self::FlurryNode(kind) => kind.as_str().to_string(),
            Self::RuleRef(name) => name.clone(),
            Self::Seq(items) => items
                .first()
                .map_or_else(|| "sequence".into(), Self::expected_summary),
            Self::Choice(alts) => {
                let parts: Vec<String> = alts.iter().take(4).map(Self::expected_summary).collect();
                parts.join(" or ")
            }
            Self::ZeroOrMore(inner) | Self::OneOrMore(inner) | Self::Optional(inner) => {
                inner.expected_summary()
            }
            Self::AndPred(inner) => format!("&{}", inner.expected_summary()),
            Self::NotPred(inner) => format!("!{}", inner.expected_summary()),
        }
    }
}

impl fmt::Display for PegExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Literal(text) => write!(f, "(lit \"{}\")", escape_sexp_str(text)),
            Self::TokenKind(kind) => write!(f, "(token .{})", escape_sexp_str(kind.symbol_name())),
            Self::FlurryNode(kind) => write!(f, "(flurry {kind})"),
            Self::RuleRef(name) => write!(f, "(ref \"{}\")", escape_sexp_str(name)),
            Self::Seq(items) => {
                f.write_str("(seq")?;
                for item in items {
                    write!(f, " {item}")?;
                }
                f.write_str(")")
            }
            Self::Choice(alts) => {
                f.write_str("(choice")?;
                for alt in alts {
                    write!(f, " {alt}")?;
                }
                f.write_str(")")
            }
            Self::ZeroOrMore(inner) => write!(f, "(star {inner})"),
            Self::OneOrMore(inner) => write!(f, "(plus {inner})"),
            Self::Optional(inner) => write!(f, "(opt {inner})"),
            Self::AndPred(inner) => write!(f, "(and-pred {inner})"),
            Self::NotPred(inner) => write!(f, "(not-pred {inner})"),
        }
    }
}

/// Single named production rule in a Mode 2 `@define_macro2` PEG grammar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PegRule {
    pub name: String,
    pub expr: PegExpr,
    pub span: Span,
}

impl fmt::Display for PegRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(peg-rule (name \"{}\") {})",
            escape_sexp_str(&self.name),
            self.expr
        )
    }
}

/// Validated collection of PEG rules containing an `Entry` production.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PegGrammar {
    pub rules: Vec<PegRule>,
}

impl PegGrammar {
    pub fn rule(&self, name: &str) -> Option<&PegRule> {
        self.rules.iter().find(|r| r.name == name)
    }

    /// Render canonical PEG rule source text suitable for storage in definition metadata.
    pub fn to_source(&self) -> String {
        let mut out = String::new();
        for (idx, rule) in self.rules.iter().enumerate() {
            if idx > 0 {
                out.push('\n');
            }
            out.push_str(&rule.name);
            out.push_str(" -> ");
            rule.expr.write_peg_syntax(&mut out, 0);
            out.push(';');
        }
        out
    }

    fn unwrap_alias<'a>(&'a self, mut expr: &'a PegExpr) -> &'a PegExpr {
        let mut seen = HashSet::new();
        while let PegExpr::RuleRef(name) = expr {
            if !seen.insert(name.as_str()) {
                break;
            }
            let Some(rule) = self.rule(name) else {
                break;
            };
            expr = &rule.expr;
        }
        expr
    }

    /// Infer whether `Entry` directly projects into a single `Flurry*`, `?Flurry*`, or `List<Flurry*>`.
    pub fn entry_projection(&self) -> Option<PegParamShape> {
        let entry = self.rule("Entry")?;
        let unwrapped = self.unwrap_alias(&entry.expr);
        match unwrapped {
            PegExpr::FlurryNode(kind) => Some(PegParamShape::Single(*kind)),
            PegExpr::Optional(inner) => {
                if let PegExpr::FlurryNode(kind) = self.unwrap_alias(inner) {
                    Some(PegParamShape::Optional(*kind))
                } else {
                    None
                }
            }
            PegExpr::ZeroOrMore(inner) | PegExpr::OneOrMore(inner) => {
                if let PegExpr::FlurryNode(kind) = self.unwrap_alias(inner) {
                    Some(PegParamShape::Repetition(*kind))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub fn supports_param_shape(&self, shape: PegParamShape) -> bool {
        match shape {
            PegParamShape::PegNode => true,
            _ => self.entry_projection() == Some(shape),
        }
    }

    /// Validate entry rule presence, uniqueness, name resolution, and left-recursion freedom.
    pub fn validate(&self, anchor_span: Span) -> Result<(), MacroError> {
        let mut by_name: HashMap<&str, &PegRule> = HashMap::with_capacity(self.rules.len());
        for rule in &self.rules {
            if AstNodeKind::from_type_name(&rule.name).is_some() {
                return Err(MacroError::new(
                    MacroErrorKind::PegGrammarError,
                    format!(
                        "PEG rule name `{}` collides with built-in Flurry AST nonterminal",
                        rule.name
                    ),
                    rule.span,
                ));
            }
            if by_name.insert(rule.name.as_str(), rule).is_some() {
                return Err(MacroError::new(
                    MacroErrorKind::PegGrammarError,
                    format!("duplicate PEG rule `{}`", rule.name),
                    rule.span,
                ));
            }
        }

        if !by_name.contains_key("Entry") {
            return Err(MacroError::new(
                MacroErrorKind::PegGrammarError,
                "PEG macro definition must declare an `Entry` rule (e.g. `Entry -> ...;`)",
                anchor_span,
            ));
        }

        // Verify all RuleRefs resolve before computing fixpoints/cycles.
        fn check_refs(
            expr: &PegExpr,
            by_name: &HashMap<&str, &PegRule>,
            rule_span: Span,
        ) -> Result<(), MacroError> {
            match expr {
                PegExpr::Literal(_) | PegExpr::TokenKind(_) | PegExpr::FlurryNode(_) => Ok(()),
                PegExpr::RuleRef(target) => {
                    if !by_name.contains_key(target.as_str()) {
                        return Err(MacroError::new(
                            MacroErrorKind::PegGrammarError,
                            format!("undefined PEG rule `{target}`"),
                            rule_span,
                        ));
                    }
                    Ok(())
                }
                PegExpr::Seq(items) | PegExpr::Choice(items) => {
                    for item in items {
                        check_refs(item, by_name, rule_span)?;
                    }
                    Ok(())
                }
                PegExpr::ZeroOrMore(inner)
                | PegExpr::OneOrMore(inner)
                | PegExpr::Optional(inner)
                | PegExpr::AndPred(inner)
                | PegExpr::NotPred(inner) => check_refs(inner, by_name, rule_span),
            }
        }

        for rule in &self.rules {
            check_refs(&rule.expr, &by_name, rule.span)?;
        }

        // Compute least fixed-point nullability across all rules.
        let mut nullable: HashMap<&str, bool> = self
            .rules
            .iter()
            .map(|r| (r.name.as_str(), false))
            .collect();
        fn is_nullable(expr: &PegExpr, nullable: &HashMap<&str, bool>) -> bool {
            match expr {
                PegExpr::Literal(_) | PegExpr::TokenKind(_) | PegExpr::FlurryNode(_) => false,
                PegExpr::ZeroOrMore(_)
                | PegExpr::Optional(_)
                | PegExpr::AndPred(_)
                | PegExpr::NotPred(_) => true,
                PegExpr::OneOrMore(inner) => is_nullable(inner, nullable),
                PegExpr::Seq(items) => items.iter().all(|item| is_nullable(item, nullable)),
                PegExpr::Choice(alts) => alts.iter().any(|alt| is_nullable(alt, nullable)),
                PegExpr::RuleRef(target) => *nullable.get(target.as_str()).unwrap_or(&false),
            }
        }

        let mut changed = true;
        while changed {
            changed = false;
            for rule in &self.rules {
                let next = is_nullable(&rule.expr, &nullable);
                if next && !nullable[rule.name.as_str()] {
                    nullable.insert(rule.name.as_str(), true);
                    changed = true;
                }
            }
        }

        // Collect all rules reachable without guaranteed consumption (unguarded leading positions).
        fn collect_leading_refs<'a>(
            expr: &'a PegExpr,
            nullable: &HashMap<&str, bool>,
            out: &mut BTreeSet<&'a str>,
        ) {
            match expr {
                PegExpr::Literal(_) | PegExpr::TokenKind(_) | PegExpr::FlurryNode(_) => {}
                PegExpr::RuleRef(target) => {
                    out.insert(target.as_str());
                }
                PegExpr::Seq(items) => {
                    for item in items {
                        collect_leading_refs(item, nullable, out);
                        if !is_nullable(item, nullable) {
                            break;
                        }
                    }
                }
                PegExpr::Choice(alts) => {
                    for alt in alts {
                        collect_leading_refs(alt, nullable, out);
                    }
                }
                PegExpr::ZeroOrMore(inner)
                | PegExpr::OneOrMore(inner)
                | PegExpr::Optional(inner)
                | PegExpr::AndPred(inner)
                | PegExpr::NotPred(inner) => {
                    collect_leading_refs(inner, nullable, out);
                }
            }
        }

        let mut edges: HashMap<&str, BTreeSet<&str>> = HashMap::with_capacity(self.rules.len());
        for rule in &self.rules {
            let mut set = BTreeSet::new();
            collect_leading_refs(&rule.expr, &nullable, &mut set);
            edges.insert(rule.name.as_str(), set);
        }

        #[derive(Clone, Copy, PartialEq, Eq)]
        enum VisitState {
            Visiting,
            Done,
        }

        fn dfs_cycle<'a>(
            node: &'a str,
            edges: &HashMap<&'a str, BTreeSet<&'a str>>,
            by_name: &HashMap<&'a str, &'a PegRule>,
            state: &mut HashMap<&'a str, VisitState>,
            stack: &mut Vec<&'a str>,
        ) -> Result<(), MacroError> {
            state.insert(node, VisitState::Visiting);
            stack.push(node);
            if let Some(nexts) = edges.get(node) {
                for &next in nexts {
                    match state.get(next) {
                        Some(VisitState::Visiting) => {
                            let start_idx = stack.iter().position(|&n| n == next).unwrap_or(0);
                            let mut cycle: Vec<&str> = stack[start_idx..].to_vec();
                            cycle.push(next);
                            let span = by_name[next].span;
                            return Err(MacroError::new(
                                MacroErrorKind::PegGrammarError,
                                format!(
                                    "unguarded left recursion in PEG grammar: {}",
                                    cycle.join(" -> ")
                                ),
                                span,
                            ));
                        }
                        None => dfs_cycle(next, edges, by_name, state, stack)?,
                        Some(VisitState::Done) => {}
                    }
                }
            }
            stack.pop();
            state.insert(node, VisitState::Done);
            Ok(())
        }

        let mut state = HashMap::new();
        let mut stack = Vec::new();
        for rule in &self.rules {
            let name = rule.name.as_str();
            if !state.contains_key(name) {
                dfs_cycle(name, &edges, &by_name, &mut state, &mut stack)?;
            }
        }

        Ok(())
    }
}

impl fmt::Display for PegGrammar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(peg-grammar")?;
        for rule in &self.rules {
            write!(f, " {rule}")?;
        }
        f.write_str(")")
    }
}

/// Compact 0-indexed node inside a `ParsedPegTree` arena.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedPegNode {
    pub rule: String,
    pub span: Span,
    pub text: String,
    pub tokens: Vec<MacroToken>,
    pub children: Vec<usize>,
}

impl fmt::Display for ParsedPegNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(peg-node (rule \"{}\") (span {} {}) (tokens {}) (children",
            escape_sexp_str(&self.rule),
            self.span.lo().0,
            self.span.hi().0,
            self.tokens.len(),
        )?;
        for c in &self.children {
            write!(f, " {c}")?;
        }
        write!(f, ") (text \"{}\"))", escape_sexp_str(&self.text))
    }
}

/// Parsed PEG syntax tree handed to a Mode 2 macro taking `PegNode`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedPegTree {
    pub nodes: Vec<ParsedPegNode>,
    pub root: usize,
}

impl fmt::Display for ParsedPegTree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(peg-tree (root {}) (nodes", self.root)?;
        for node in &self.nodes {
            write!(f, " {node}")?;
        }
        f.write_str("))")
    }
}

/// Marshalled PEG parse result aligned to the macro handler's declared parameter shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PegArgValue {
    PegNode(ParsedPegTree),
    Single(ParsedSyntaxTree),
    Optional(Option<ParsedSyntaxTree>),
    Repetition(Vec<ParsedSyntaxTree>),
}

impl fmt::Display for PegArgValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PegNode(tree) => write!(f, "(peg-arg-node {tree})"),
            Self::Single(tree) => write!(f, "(peg-arg-single {tree})"),
            Self::Optional(Some(tree)) => write!(f, "(peg-arg-optional (some {tree}))"),
            Self::Optional(None) => f.write_str("(peg-arg-optional null)"),
            Self::Repetition(items) => {
                f.write_str("(peg-arg-repetition")?;
                for item in items {
                    write!(f, " {item}")?;
                }
                f.write_str(")")
            }
        }
    }
}

fn decode_flurry_str_literal(raw: &str, span: Span) -> Result<String, MacroError> {
    let body = raw
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .ok_or_else(|| {
            MacroError::new(
                MacroErrorKind::PegGrammarError,
                format!("invalid string literal `{raw}` in PEG grammar"),
                span,
            )
        })?;
    let mut chars = body.chars().peekable();
    let mut out = String::with_capacity(body.len());
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                let esc = match chars.next() {
                    Some('\\') => '\\',
                    Some('\'') => '\'',
                    Some('"') => '"',
                    Some('n') => '\n',
                    Some('r') => '\r',
                    Some('t') => '\t',
                    Some('0') => '\0',
                    Some('$') => '$',
                    Some('x') => {
                        if chars.next() != Some('{') {
                            return Err(MacroError::new(
                                MacroErrorKind::PegGrammarError,
                                "unicode escape in PEG string literal must use `\\x{...}`",
                                span,
                            ));
                        }
                        let mut scalar = 0u32;
                        let mut digits = 0usize;
                        loop {
                            match chars.next() {
                                Some('}') => break,
                                Some(d) if d.is_ascii_hexdigit() => {
                                    digits += 1;
                                    if digits > 6 {
                                        return Err(MacroError::new(
                                            MacroErrorKind::PegGrammarError,
                                            "unicode escape exceeds six hex digits",
                                            span,
                                        ));
                                    }
                                    scalar = scalar * 16 + d.to_digit(16).unwrap_or(0);
                                }
                                _ => {
                                    return Err(MacroError::new(
                                        MacroErrorKind::PegGrammarError,
                                        "invalid unicode escape in PEG string literal",
                                        span,
                                    ));
                                }
                            }
                        }
                        if digits == 0 {
                            return Err(MacroError::new(
                                MacroErrorKind::PegGrammarError,
                                "unicode escape requires at least one hex digit",
                                span,
                            ));
                        }
                        char::from_u32(scalar).ok_or_else(|| {
                            MacroError::new(
                                MacroErrorKind::PegGrammarError,
                                "unicode escape is not a valid scalar value",
                                span,
                            )
                        })?
                    }
                    _ => {
                        return Err(MacroError::new(
                            MacroErrorKind::PegGrammarError,
                            "unsupported escape sequence in PEG string literal",
                            span,
                        ));
                    }
                };
                out.push(esc);
            }
            ch => out.push(ch),
        }
    }
    if out.is_empty() {
        return Err(MacroError::new(
            MacroErrorKind::PegGrammarError,
            "PEG string literal token matchers cannot be empty",
            span,
        ));
    }
    Ok(out)
}

struct RuleTokenParser<'a> {
    tokens: &'a [MacroToken],
    pos: usize,
    anchor_span: Span,
}

impl<'a> RuleTokenParser<'a> {
    fn peek(&self) -> Option<&'a MacroToken> {
        self.tokens.get(self.pos)
    }

    fn err_here(&self, msg: impl Into<String>) -> MacroError {
        let span = self.peek().map_or(self.anchor_span, |t| t.span);
        MacroError::new(MacroErrorKind::PegGrammarError, msg, span)
    }

    fn parse_grammar(&mut self) -> Result<PegGrammar, MacroError> {
        let mut rules = Vec::new();
        while self.pos < self.tokens.len() {
            rules.push(self.parse_rule()?);
        }
        Ok(PegGrammar { rules })
    }

    fn parse_rule(&mut self) -> Result<PegRule, MacroError> {
        let name_tok = self
            .peek()
            .ok_or_else(|| self.err_here("expected PEG rule name"))?;
        if name_tok.kind != TokenKind::Id {
            return Err(MacroError::new(
                MacroErrorKind::PegGrammarError,
                format!(
                    "expected PEG rule name identifier, found `{}`",
                    name_tok.text
                ),
                name_tok.span,
            ));
        }
        self.pos += 1;

        let arrow_tok = self.peek().ok_or_else(|| {
            MacroError::new(
                MacroErrorKind::PegGrammarError,
                format!("expected `->` after PEG rule name `{}`", name_tok.text),
                name_tok.span,
            )
        })?;
        if arrow_tok.kind != TokenKind::Arrow {
            return Err(MacroError::new(
                MacroErrorKind::PegGrammarError,
                format!(
                    "expected `->` after PEG rule name `{}`, found `{}`",
                    name_tok.text, arrow_tok.text
                ),
                arrow_tok.span,
            ));
        }
        self.pos += 1;

        let expr = self.parse_choice()?;
        let semi_tok = self.peek().ok_or_else(|| {
            MacroError::new(
                MacroErrorKind::PegGrammarError,
                format!("expected `;` terminating PEG rule `{}`", name_tok.text),
                name_tok.span,
            )
        })?;
        if semi_tok.kind != TokenKind::Semi {
            return Err(MacroError::new(
                MacroErrorKind::PegGrammarError,
                format!(
                    "expected `;` after PEG rule `{}`, found `{}`",
                    name_tok.text, semi_tok.text
                ),
                semi_tok.span,
            ));
        }
        self.pos += 1;

        Ok(PegRule {
            name: name_tok.text.clone(),
            expr,
            span: Span::new(name_tok.span.lo(), semi_tok.span.hi()),
        })
    }

    fn parse_choice(&mut self) -> Result<PegExpr, MacroError> {
        let mut alts = vec![self.parse_seq()?];
        while self.peek().is_some_and(|t| t.kind == TokenKind::Pipe) {
            self.pos += 1;
            alts.push(self.parse_seq()?);
        }
        if alts.len() == 1 {
            Ok(alts.pop().unwrap())
        } else {
            Ok(PegExpr::Choice(alts))
        }
    }

    fn parse_seq(&mut self) -> Result<PegExpr, MacroError> {
        let mut items = vec![self.parse_prefix()?];
        while self.peek().is_some_and(|t| t.kind == TokenKind::Tilde) {
            self.pos += 1;
            items.push(self.parse_prefix()?);
        }
        if items.len() == 1 {
            Ok(items.pop().unwrap())
        } else {
            Ok(PegExpr::Seq(items))
        }
    }

    fn parse_prefix(&mut self) -> Result<PegExpr, MacroError> {
        match self.peek().map(|t| t.kind) {
            Some(TokenKind::Ampersand) => {
                self.pos += 1;
                Ok(PegExpr::AndPred(Box::new(self.parse_prefix()?)))
            }
            Some(TokenKind::Bang) => {
                self.pos += 1;
                Ok(PegExpr::NotPred(Box::new(self.parse_prefix()?)))
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> Result<PegExpr, MacroError> {
        let mut expr = self.parse_primary()?;
        loop {
            match self.peek().map(|t| t.kind) {
                Some(TokenKind::Star | TokenKind::SeparatedStar) => {
                    self.pos += 1;
                    expr = PegExpr::ZeroOrMore(Box::new(expr));
                }
                Some(TokenKind::Plus | TokenKind::SeparatedPlus) => {
                    self.pos += 1;
                    expr = PegExpr::OneOrMore(Box::new(expr));
                }
                Some(TokenKind::Question) => {
                    self.pos += 1;
                    expr = PegExpr::Optional(Box::new(expr));
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    fn parse_primary(&mut self) -> Result<PegExpr, MacroError> {
        let tok = self
            .peek()
            .ok_or_else(|| self.err_here("expected PEG pattern element"))?;
        match tok.kind {
            TokenKind::LParen => {
                let open_span = tok.span;
                self.pos += 1;
                let inner = self.parse_choice()?;
                let close = self.peek().ok_or_else(|| {
                    MacroError::new(
                        MacroErrorKind::PegGrammarError,
                        "unclosed `(` in PEG rule expression",
                        open_span,
                    )
                })?;
                if close.kind != TokenKind::RParen {
                    return Err(MacroError::new(
                        MacroErrorKind::PegGrammarError,
                        format!(
                            "expected `)` in PEG rule expression, found `{}`",
                            close.text
                        ),
                        close.span,
                    ));
                }
                self.pos += 1;
                Ok(inner)
            }
            TokenKind::Str => {
                self.pos += 1;
                let decoded = decode_flurry_str_literal(&tok.text, tok.span)?;
                Ok(PegExpr::Literal(decoded))
            }
            TokenKind::Dot => {
                self.pos += 1;
                let sym = self.peek().ok_or_else(|| {
                    MacroError::new(
                        MacroErrorKind::PegGrammarError,
                        "expected token kind symbol name after `.` in PEG rule",
                        tok.span,
                    )
                })?;
                if sym.kind != TokenKind::Id {
                    return Err(MacroError::new(
                        MacroErrorKind::PegGrammarError,
                        format!(
                            "expected token kind identifier after `.` in PEG rule, found `{}`",
                            sym.text
                        ),
                        sym.span,
                    ));
                }
                self.pos += 1;
                let kind = TokenKind::from_symbol_name(&sym.text)
                    .filter(|k| {
                        !matches!(
                            k,
                            TokenKind::Sof
                                | TokenKind::Eof
                                | TokenKind::Invalid
                                | TokenKind::Comment
                        )
                    })
                    .ok_or_else(|| {
                        MacroError::new(
                            MacroErrorKind::PegGrammarError,
                            format!("unknown token kind symbol `.{}` in PEG rule", sym.text),
                            sym.span,
                        )
                    })?;
                Ok(PegExpr::TokenKind(kind))
            }
            TokenKind::Id => {
                self.pos += 1;
                if let Some(node_kind) = AstNodeKind::from_type_name(&tok.text) {
                    Ok(PegExpr::FlurryNode(node_kind))
                } else {
                    Ok(PegExpr::RuleRef(tok.text.clone()))
                }
            }
            _ => Err(MacroError::new(
                MacroErrorKind::PegGrammarError,
                format!("unexpected token `{}` inside PEG rule", tok.text),
                tok.span,
            )),
        }
    }
}

/// Parse PEG rules from a `MacroToken` slice inside `@define_macro2 { ... }`.
pub fn parse_peg_rules_from_tokens(
    tokens: &[MacroToken],
    anchor_span: Span,
) -> Result<PegGrammar, MacroError> {
    let mut parser = RuleTokenParser {
        tokens,
        pos: 0,
        anchor_span,
    };
    parser.parse_grammar()
}

/// Lex, parse, and statically validate serialized PEG grammar source text (e.g. from NIR metadata).
pub fn parse_and_validate_peg_grammar_str(src: &str) -> Result<PegGrammar, MacroError> {
    let (lexed, _, errs) = lex::lex(src, BytePos(0));
    if let Some(first) = errs.into_iter().next() {
        return Err(MacroError::new(
            MacroErrorKind::PegGrammarError,
            format!("invalid stored PEG grammar metadata: {}", first.message()),
            rustc_span::DUMMY_SP,
        ));
    }
    let macro_tokens = tokens_to_macro_tokens(&lexed, src, BytePos(0));
    let grammar = parse_peg_rules_from_tokens(&macro_tokens, rustc_span::DUMMY_SP)?;
    grammar.validate(rustc_span::DUMMY_SP)?;
    Ok(grammar)
}

#[derive(Clone, Debug)]
enum PegMatchNode {
    Leaf {
        rule: String,
        start: usize,
        end: usize,
        ast: Option<ParsedSyntaxTree>,
    },
    Rule {
        name: String,
        start: usize,
        end: usize,
        children: Vec<PegMatchNode>,
    },
    Group {
        start: usize,
        end: usize,
        items: Vec<PegMatchNode>,
    },
}

impl PegMatchNode {
    fn append_flattened(self, out: &mut Vec<PegMatchNode>) {
        match self {
            Self::Group { items, .. } => {
                for item in items {
                    item.append_flattened(out);
                }
            }
            other => out.push(other),
        }
    }

    fn collect_ast_trees(&self, out: &mut Vec<ParsedSyntaxTree>) {
        match self {
            Self::Leaf {
                ast: Some(tree), ..
            } => out.push(tree.clone()),
            Self::Leaf { ast: None, .. } => {}
            Self::Rule { children, .. } => {
                for child in children {
                    child.collect_ast_trees(out);
                }
            }
            Self::Group { items, .. } => {
                for item in items {
                    item.collect_ast_trees(out);
                }
            }
        }
    }
}

struct PackratEngine<'a, 'b> {
    input_tokens: &'a [MacroToken],
    grammar: &'a PegGrammar,
    parser: &'a mut Parser<'b>,
    src_text: &'a str,
    base_pos: BytePos,
    token_byte_ranges: &'a [(usize, usize)],
    call_site: Span,
    budget: &'a mut MacroBudget,
    rule_memo: HashMap<(String, usize), Option<(usize, PegMatchNode)>>,
    flurry_memo: HashMap<(AstNodeKind, usize), Option<(usize, ParsedSyntaxTree)>>,
    furthest_pos: usize,
    furthest_expected: Option<String>,
    suppress_error_tracking: bool,
}

impl<'a, 'b> PackratEngine<'a, 'b> {
    fn total_memo_entries(&self) -> usize {
        self.rule_memo.len().saturating_add(self.flurry_memo.len())
    }

    fn check_memo_budget(&self) -> Result<(), MacroError> {
        if self.total_memo_entries() >= self.budget.max_memo_entries {
            return Err(MacroError::new(
                MacroErrorKind::BudgetExhausted,
                format!(
                    "PEG packrat memoization budget ({}) exhausted",
                    self.budget.max_memo_entries
                ),
                self.call_site,
            ));
        }
        Ok(())
    }

    fn note_failure(&mut self, pos: usize, expected: &PegExpr) {
        if self.suppress_error_tracking {
            return;
        }
        if pos > self.furthest_pos || (pos == self.furthest_pos && self.furthest_expected.is_none())
        {
            self.furthest_pos = pos;
            self.furthest_expected = Some(expected.expected_summary());
        }
    }

    fn eval(
        &mut self,
        expr: &PegExpr,
        pos: usize,
    ) -> Result<Option<(usize, PegMatchNode)>, MacroError> {
        self.budget
            .consume_step()
            .map_err(|e| e.with_span(self.call_site))?;

        match expr {
            PegExpr::Literal(expected) => {
                if let Some(tok) = self.input_tokens.get(pos) {
                    if tok.text == *expected {
                        return Ok(Some((
                            pos + 1,
                            PegMatchNode::Leaf {
                                rule: "literal".to_string(),
                                start: pos,
                                end: pos + 1,
                                ast: None,
                            },
                        )));
                    }
                }
                self.note_failure(pos, expr);
                Ok(None)
            }
            PegExpr::TokenKind(kind) => {
                if let Some(tok) = self.input_tokens.get(pos) {
                    if tok.kind == *kind {
                        return Ok(Some((
                            pos + 1,
                            PegMatchNode::Leaf {
                                rule: kind.symbol_name().to_string(),
                                start: pos,
                                end: pos + 1,
                                ast: None,
                            },
                        )));
                    }
                }
                self.note_failure(pos, expr);
                Ok(None)
            }
            PegExpr::FlurryNode(kind) => {
                let key = (*kind, pos);
                if let Some(cached) = self.flurry_memo.get(&key).cloned() {
                    if cached.is_none() {
                        self.note_failure(pos, expr);
                    }
                    return Ok(cached.map(|(end, tree)| {
                        (
                            end,
                            PegMatchNode::Leaf {
                                rule: kind.as_str().to_string(),
                                start: pos,
                                end,
                                ast: Some(tree),
                            },
                        )
                    }));
                }

                let matched = match self.parser.parse_fragment_at(pos, fragment_kind_for(*kind)) {
                    Ok(Some(m)) => {
                        let tree = snapshot_ast_subtree(
                            &self.parser.ast,
                            m.root,
                            self.input_tokens,
                            m.start_cursor,
                            m.end_cursor,
                            self.src_text,
                            self.base_pos,
                            self.token_byte_ranges,
                            self.call_site,
                        );
                        Some((m.end_cursor, tree))
                    }
                    Ok(None) | Err(_) => {
                        self.note_failure(pos, expr);
                        None
                    }
                };

                self.check_memo_budget()?;
                self.flurry_memo.insert(key, matched.clone());
                Ok(matched.map(|(end, tree)| {
                    (
                        end,
                        PegMatchNode::Leaf {
                            rule: kind.as_str().to_string(),
                            start: pos,
                            end,
                            ast: Some(tree),
                        },
                    )
                }))
            }
            PegExpr::RuleRef(name) => {
                let key = (name.clone(), pos);
                if let Some(cached) = self.rule_memo.get(&key).cloned() {
                    if cached.is_none() {
                        self.note_failure(pos, expr);
                    }
                    return Ok(cached);
                }

                let rule_expr = &self
                    .grammar
                    .rule(name)
                    .ok_or_else(|| {
                        MacroError::new(
                            MacroErrorKind::PegGrammarError,
                            format!("undefined PEG rule `{name}`"),
                            self.call_site,
                        )
                    })?
                    .expr;

                let res = self.eval(rule_expr, pos)?;
                let wrapped = res.map(|(end, body)| {
                    let mut children = Vec::new();
                    body.append_flattened(&mut children);
                    (
                        end,
                        PegMatchNode::Rule {
                            name: name.clone(),
                            start: pos,
                            end,
                            children,
                        },
                    )
                });
                self.check_memo_budget()?;
                self.rule_memo.insert(key, wrapped.clone());
                Ok(wrapped)
            }
            PegExpr::Choice(alts) => {
                for alt in alts {
                    if let Some(matched) = self.eval(alt, pos)? {
                        return Ok(Some(matched));
                    }
                }
                Ok(None)
            }
            PegExpr::Seq(parts) => {
                let mut cur = pos;
                let mut items = Vec::with_capacity(parts.len());
                for part in parts {
                    let Some((next, node)) = self.eval(part, cur)? else {
                        return Ok(None);
                    };
                    cur = next;
                    items.push(node);
                }
                Ok(Some((
                    cur,
                    PegMatchNode::Group {
                        start: pos,
                        end: cur,
                        items,
                    },
                )))
            }
            PegExpr::ZeroOrMore(inner) => {
                let mut cur = pos;
                let mut items = Vec::new();
                while let Some((next, node)) = self.eval(inner, cur)? {
                    items.push(node);
                    if next == cur {
                        break;
                    }
                    cur = next;
                }
                Ok(Some((
                    cur,
                    PegMatchNode::Group {
                        start: pos,
                        end: cur,
                        items,
                    },
                )))
            }
            PegExpr::OneOrMore(inner) => {
                let Some((first_end, first_node)) = self.eval(inner, pos)? else {
                    return Ok(None);
                };
                let mut cur = first_end;
                let mut items = vec![first_node];
                if cur > pos {
                    while let Some((next, node)) = self.eval(inner, cur)? {
                        items.push(node);
                        if next == cur {
                            break;
                        }
                        cur = next;
                    }
                }
                Ok(Some((
                    cur,
                    PegMatchNode::Group {
                        start: pos,
                        end: cur,
                        items,
                    },
                )))
            }
            PegExpr::Optional(inner) => {
                if let Some((next, node)) = self.eval(inner, pos)? {
                    Ok(Some((next, node)))
                } else {
                    Ok(Some((
                        pos,
                        PegMatchNode::Group {
                            start: pos,
                            end: pos,
                            items: Vec::new(),
                        },
                    )))
                }
            }
            PegExpr::AndPred(inner) => {
                let prev = self.suppress_error_tracking;
                self.suppress_error_tracking = true;
                let ok = self.eval(inner, pos)?.is_some();
                self.suppress_error_tracking = prev;
                if ok {
                    Ok(Some((
                        pos,
                        PegMatchNode::Group {
                            start: pos,
                            end: pos,
                            items: Vec::new(),
                        },
                    )))
                } else {
                    self.note_failure(pos, expr);
                    Ok(None)
                }
            }
            PegExpr::NotPred(inner) => {
                let prev = self.suppress_error_tracking;
                self.suppress_error_tracking = true;
                let matched = self.eval(inner, pos)?.is_some();
                self.suppress_error_tracking = prev;
                if matched {
                    self.note_failure(pos, expr);
                    Ok(None)
                } else {
                    Ok(Some((
                        pos,
                        PegMatchNode::Group {
                            start: pos,
                            end: pos,
                            items: Vec::new(),
                        },
                    )))
                }
            }
        }
    }
}

fn flatten_peg_tree(
    root: PegMatchNode,
    input_tokens: &[MacroToken],
    src_text: &str,
    token_byte_ranges: &[(usize, usize)],
    call_site: Span,
) -> ParsedPegTree {
    fn slice_meta(
        start: usize,
        end: usize,
        input_tokens: &[MacroToken],
        src_text: &str,
        token_byte_ranges: &[(usize, usize)],
        fallback: Span,
    ) -> (Span, String, Vec<MacroToken>) {
        let clamped_end = end.min(input_tokens.len());
        let clamped_start = start.min(clamped_end);
        if clamped_start >= clamped_end {
            return (fallback, String::new(), Vec::new());
        }
        let span = Span::new(
            input_tokens[clamped_start].span.lo(),
            input_tokens[clamped_end - 1].span.hi(),
        );
        let tokens = input_tokens[clamped_start..clamped_end].to_vec();
        let text = match (
            token_byte_ranges.get(clamped_start),
            token_byte_ranges.get(clamped_end - 1),
        ) {
            (Some(&(lo, _)), Some(&(_, hi))) if lo <= hi => {
                src_text.get(lo..hi).unwrap_or("").to_string()
            }
            _ => String::new(),
        };
        (span, text, tokens)
    }

    fn push_node(
        node: PegMatchNode,
        input_tokens: &[MacroToken],
        src_text: &str,
        token_byte_ranges: &[(usize, usize)],
        fallback: Span,
        out: &mut Vec<ParsedPegNode>,
    ) -> usize {
        match node {
            PegMatchNode::Leaf {
                rule, start, end, ..
            } => {
                let my_idx = out.len();
                let (span, text, tokens) = slice_meta(
                    start,
                    end,
                    input_tokens,
                    src_text,
                    token_byte_ranges,
                    fallback,
                );
                out.push(ParsedPegNode {
                    rule,
                    span,
                    text,
                    tokens,
                    children: Vec::new(),
                });
                my_idx
            }
            PegMatchNode::Rule {
                name,
                start,
                end,
                children,
            } => {
                let my_idx = out.len();
                let (span, text, tokens) = slice_meta(
                    start,
                    end,
                    input_tokens,
                    src_text,
                    token_byte_ranges,
                    fallback,
                );
                out.push(ParsedPegNode {
                    rule: name,
                    span,
                    text,
                    tokens,
                    children: Vec::new(),
                });
                let mut flattened_children = Vec::new();
                for ch in children {
                    ch.append_flattened(&mut flattened_children);
                }
                let mut child_indices = Vec::with_capacity(flattened_children.len());
                for ch in flattened_children {
                    child_indices.push(push_node(
                        ch,
                        input_tokens,
                        src_text,
                        token_byte_ranges,
                        fallback,
                        out,
                    ));
                }
                out[my_idx].children = child_indices;
                my_idx
            }
            PegMatchNode::Group { start, end, items } => {
                let my_idx = out.len();
                let (span, text, tokens) = slice_meta(
                    start,
                    end,
                    input_tokens,
                    src_text,
                    token_byte_ranges,
                    fallback,
                );
                out.push(ParsedPegNode {
                    rule: "group".to_string(),
                    span,
                    text,
                    tokens,
                    children: Vec::new(),
                });
                let mut child_indices = Vec::with_capacity(items.len());
                for item in items {
                    child_indices.push(push_node(
                        item,
                        input_tokens,
                        src_text,
                        token_byte_ranges,
                        fallback,
                        out,
                    ));
                }
                out[my_idx].children = child_indices;
                my_idx
            }
        }
    }

    let mut nodes = Vec::new();
    let root_idx = push_node(
        root,
        input_tokens,
        src_text,
        token_byte_ranges,
        call_site,
        &mut nodes,
    );
    ParsedPegTree {
        nodes,
        root: root_idx,
    }
}

/// Run the packrat PEG interpreter for `@path { ... }` against `input_tokens`.
pub fn parse_peg_input(
    input_tokens: &[MacroToken],
    grammar: &PegGrammar,
    param_shape: PegParamShape,
    macro_name: &str,
    call_site: Span,
    budget: &mut MacroBudget,
) -> Result<PegArgValue, MacroError> {
    let (source_map, src_text, base_pos, parser_tokens, parser_symbols, token_byte_ranges) =
        build_parser_input(input_tokens, &format!("<macro-peg-input:{macro_name}>"));
    let mut parser = Parser::new(&source_map, parser_tokens, parser_symbols, base_pos);

    let mut engine = PackratEngine {
        input_tokens,
        grammar,
        parser: &mut parser,
        src_text: &src_text,
        base_pos,
        token_byte_ranges: &token_byte_ranges,
        call_site,
        budget,
        rule_memo: HashMap::new(),
        flurry_memo: HashMap::new(),
        furthest_pos: 0,
        furthest_expected: None,
        suppress_error_tracking: false,
    };

    let entry_expr = PegExpr::RuleRef("Entry".to_string());
    let matched = engine.eval(&entry_expr, 0)?;

    let Some((end_pos, root_match)) = matched else {
        let err_pos = engine.furthest_pos;
        let err_span = input_tokens
            .get(err_pos)
            .or_else(|| input_tokens.last())
            .map_or(call_site, |t| t.span);
        let expected_note = engine
            .furthest_expected
            .map(|e| format!(" (expected {e})"))
            .unwrap_or_default();
        let found_note = input_tokens
            .get(err_pos)
            .map(|t| format!(", found `{}`", t.text))
            .unwrap_or_else(|| ", reached end of macro input".to_string());
        return Err(MacroError::new(
            MacroErrorKind::PegInputParseError,
            format!(
                "PEG input failed to match `@{macro_name}` `Entry` rule{expected_note}{found_note}"
            ),
            err_span,
        ));
    };

    if end_pos < input_tokens.len() {
        let report_pos =
            if engine.furthest_pos >= end_pos && engine.furthest_pos < input_tokens.len() {
                engine.furthest_pos
            } else {
                end_pos
            };
        let err_span = input_tokens.get(report_pos).map_or(call_site, |t| t.span);
        let expected_note = if engine.furthest_pos >= end_pos {
            engine
                .furthest_expected
                .map(|e| format!(" (expected {e})"))
                .unwrap_or_default()
        } else {
            String::new()
        };
        return Err(MacroError::new(
            MacroErrorKind::PegInputParseError,
            format!(
                "unexpected trailing token `{}` in call to `@{macro_name}`{expected_note}",
                input_tokens[report_pos].text
            ),
            err_span,
        ));
    }

    match param_shape {
        PegParamShape::PegNode => {
            let tree = flatten_peg_tree(
                root_match,
                input_tokens,
                &src_text,
                &token_byte_ranges,
                call_site,
            );
            Ok(PegArgValue::PegNode(tree))
        }
        PegParamShape::Single(_) => {
            let mut items = Vec::new();
            root_match.collect_ast_trees(&mut items);
            let single = items.into_iter().next().ok_or_else(|| {
                MacroError::new(
                    MacroErrorKind::PegInputParseError,
                    format!(
                        "internal error: PEG macro `@{macro_name}` produced no AST node for single parameter"
                    ),
                    call_site,
                )
            })?;
            Ok(PegArgValue::Single(single))
        }
        PegParamShape::Optional(_) => {
            let mut items = Vec::new();
            root_match.collect_ast_trees(&mut items);
            Ok(PegArgValue::Optional(items.into_iter().next()))
        }
        PegParamShape::Repetition(_) => {
            let mut items = Vec::new();
            root_match.collect_ast_trees(&mut items);
            Ok(PegArgValue::Repetition(items))
        }
    }
}
