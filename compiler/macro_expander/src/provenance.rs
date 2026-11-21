//! Provenance tracking, token buffers, and Lisp S-expression formatting.
use lex::TokenKind;
use rustc_span::Span;
use std::fmt;

/// Spacing relationship between a token and its successor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenSpacing {
    /// Immediately adjacent to the following token (e.g. `23u32`, `foo(`, `{{`).
    Joint,
    /// Separated from the following token by horizontal whitespace or inline comment.
    Alone,
    /// Separated from the following token by at least one newline.
    Newline,
    /// Whitespace-separated infix operator (` + `, ` - `, ` * `, ` / `, ` % `, ` < `, ` > `).
    Separated,
}

impl TokenSpacing {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Joint => "joint",
            Self::Alone => "alone",
            Self::Newline => "newline",
            Self::Separated => "separated",
        }
    }
}

impl fmt::Display for TokenSpacing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Identifier of one macro expansion event within a file expansion session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ExpansionId(pub u32);

impl fmt::Display for ExpansionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Which macro mechanism produced an expansion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExpansionKind {
    Template,
    ConstructionAst,
    ConstructionPeg,
    ConstructionTokens,
    ConstructionSource,
}

impl ExpansionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Template => "template",
            Self::ConstructionAst => "construction-ast",
            Self::ConstructionPeg => "construction-peg",
            Self::ConstructionTokens => "construction-tokens",
            Self::ConstructionSource => "construction-source",
        }
    }
}

impl fmt::Display for ExpansionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Provenance entry for a single macro expansion step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpansionRecord {
    pub id: ExpansionId,
    pub kind: ExpansionKind,
    pub macro_name: String,
    pub call_site: Span,
    pub def_site: Option<Span>,
    pub parent: Option<ExpansionId>,
    /// Byte range `[start, end)` in the materialized output source file, if any tokens remained.
    pub output_range: Option<(usize, usize)>,
}

impl fmt::Display for ExpansionRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(expansion (id {}) (kind {}) (macro \"{}\") (call-site {} {})",
            self.id,
            self.kind,
            escape_sexp_str(&self.macro_name),
            self.call_site.lo().0,
            self.call_site.hi().0,
        )?;
        if let Some(def_site) = self.def_site {
            write!(f, " (def-site {} {})", def_site.lo().0, def_site.hi().0)?;
        }
        if let Some(parent) = self.parent {
            write!(f, " (parent {parent})")?;
        }
        if let Some((start, end)) = self.output_range {
            write!(f, " (output {start} {end})")?;
        }
        f.write_str(")")
    }
}

/// Provenance-annotated token used during macro expansion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MacroToken {
    pub kind: TokenKind,
    pub text: String,
    pub span: Span,
    pub spacing: TokenSpacing,
    pub expansion_id: Option<ExpansionId>,
}

impl MacroToken {
    pub fn new(
        kind: TokenKind,
        text: impl Into<String>,
        span: Span,
        spacing: TokenSpacing,
        expansion_id: Option<ExpansionId>,
    ) -> Self {
        Self {
            kind,
            text: text.into(),
            span,
            spacing,
            expansion_id,
        }
    }
}

impl fmt::Display for MacroToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(macro-token (kind {:?}) (text \"{}\") (spacing {}) (span {} {})",
            self.kind,
            escape_sexp_str(&self.text),
            self.spacing,
            self.span.lo().0,
            self.span.hi().0,
        )?;
        if let Some(id) = self.expansion_id {
            write!(f, " (expansion {id})")?;
        }
        f.write_str(")")
    }
}

/// Ordered table of expansion provenance records for one expanded source file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExpansionMap {
    records: Vec<ExpansionRecord>,
}

impl ExpansionMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(
        &mut self,
        kind: ExpansionKind,
        macro_name: impl Into<String>,
        call_site: Span,
        def_site: Option<Span>,
        parent: Option<ExpansionId>,
    ) -> ExpansionId {
        let id = ExpansionId(self.records.len() as u32);
        self.records.push(ExpansionRecord {
            id,
            kind,
            macro_name: macro_name.into(),
            call_site,
            def_site,
            parent,
            output_range: None,
        });
        id
    }

    pub fn extend_output_range(&mut self, id: ExpansionId, start: usize, end: usize) {
        if let Some(entry) = self.records.get_mut(id.0 as usize) {
            entry.output_range = Some(match entry.output_range {
                Some((prev_start, prev_end)) => (prev_start.min(start), prev_end.max(end)),
                None => (start, end),
            });
        }
    }

    pub fn get(&self, id: ExpansionId) -> Option<&ExpansionRecord> {
        self.records.get(id.0 as usize)
    }

    /// Return the expansion ancestry from innermost (`id`) to outermost root.
    pub fn chain(&self, id: ExpansionId) -> Vec<ExpansionRecord> {
        let mut chain = Vec::new();
        let mut cursor = Some(id);
        let mut guard = 0usize;
        while let Some(current) = cursor {
            if guard > self.records.len() {
                break;
            }
            guard += 1;
            let Some(record) = self.get(current) else {
                break;
            };
            chain.push(record.clone());
            cursor = record.parent;
        }
        chain
    }

    /// Find the innermost expansion covering a byte offset in the materialized source.
    pub fn find_at_offset(&self, offset: usize) -> Option<&ExpansionRecord> {
        self.records.iter().rev().find(|record| {
            record
                .output_range
                .is_some_and(|(start, end)| offset >= start && offset < end)
        })
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn records(&self) -> &[ExpansionRecord] {
        &self.records
    }
}

impl fmt::Display for ExpansionMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(expansion-map")?;
        for record in &self.records {
            write!(f, " {record}")?;
        }
        f.write_str(")")
    }
}

/// Escape a string for Lisp S-expression output.
pub fn escape_sexp_str(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\0"),
            c if c.is_control() => {
                use std::fmt::Write;
                let _ = write!(out, "\\u{{{:x}}}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

pub fn is_separated_kind(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::SeparatedPlus
            | TokenKind::SeparatedMinus
            | TokenKind::SeparatedStar
            | TokenKind::SeparatedSlash
            | TokenKind::SeparatedPercent
            | TokenKind::SeparatedLt
            | TokenKind::SeparatedGt
    )
}
