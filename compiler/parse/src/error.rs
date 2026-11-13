use diagnostic::{DiagnosticContext, FlurryError, Level};
use lex::TokenKind;
use rustc_span::{BytePos, DUMMY_SP, Span};
use std::fmt;

// Parse error codes (2001..2099)
pub const PARSE_UNEXPECTED_TOKEN: u32 = 2001;
pub const PARSE_INVALID_SYNTAX: u32 = 2002;
pub const PARSE_UNCLOSED_DELIMITER: u32 = 2003;
pub const PARSE_SEPARATOR: u32 = 2004;
pub const PARSE_SEPARATED_OPERATOR: u32 = 2005;
pub const PARSE_FOREIGN_HABIT: u32 = 2006;
pub const PARSE_BRACE_CONFLICT: u32 = 2007;

// Parse warning codes (2901..2999)
pub const PARSE_WARN_PARENTHESES: u32 = 2901;
pub const PARSE_WARN_DEREF_ASSIGN: u32 = 2902;
pub const PARSE_WARN_BRACE_AMBIGUITY: u32 = 2903;

/// Human-friendly description of a token kind for relaxed diagnostic prose.
pub fn describe_token(kind: TokenKind) -> String {
    match kind {
        TokenKind::Eof => "the end of the file".to_string(),
        TokenKind::Id => "an identifier".to_string(),
        TokenKind::Int | TokenKind::IntBin | TokenKind::IntOct | TokenKind::IntHex => {
            "an integer literal".to_string()
        }
        TokenKind::Real | TokenKind::RealSci => "a floating-point literal".to_string(),
        TokenKind::Str => "a string literal".to_string(),
        TokenKind::Char => "a character literal".to_string(),
        TokenKind::MacroContent => "`'{ ... }` macro content".to_string(),
        TokenKind::Comment => "a comment".to_string(),
        _ => format!("`{}`", kind.lexme().trim()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseDiag {
    pub code: u32,
    pub name: &'static str,
    pub message: String,
    pub span: Span,
    pub found: Option<TokenKind>,
    pub primary_label: Option<String>,
    pub secondary_labels: Vec<(Span, String)>,
    pub notes: Vec<String>,
    pub helps: Vec<String>,
}

/// Parser error or internal Pratt / extended-call control sentinel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    Diagnostic(Box<ParseDiag>),
    /// Internal sentinel when a trailing identifier is not a literal suffix.
    MeetPostId,
    /// Internal sentinel when `{` is owned by an outer construct (`no_extended_call`).
    MeetPostExtendedCallStart,
}

impl ParseError {
    pub fn custom(code: u32, name: &'static str, message: impl Into<String>, span: Span) -> Self {
        Self::Diagnostic(Box::new(ParseDiag {
            code,
            name,
            message: message.into(),
            span,
            found: None,
            primary_label: None,
            secondary_labels: Vec::new(),
            notes: Vec::new(),
            helps: Vec::new(),
        }))
    }

    pub fn unexpected_token(expected: TokenKind, found: TokenKind, span: Span) -> Self {
        let exp = describe_token(expected);
        let got = describe_token(found);
        let message = if found == TokenKind::Eof {
            format!("reached the end of the file while looking for {exp}")
        } else {
            format!("expected {exp} here, but ran into {got}")
        };
        let label = format!("expected {exp}, found {got}");
        Self::custom(PARSE_UNEXPECTED_TOKEN, "unexpected_token", message, span)
            .with_found(found)
            .with_primary_label(label)
    }

    pub fn invalid_syntax(message: String, found: TokenKind, span: Span) -> Self {
        let got = describe_token(found);
        let primary_label = if found == TokenKind::Eof {
            "reached the end of the file here".to_string()
        } else {
            format!("unexpected {got} here")
        };
        Self::custom(PARSE_INVALID_SYNTAX, "invalid_syntax", message, span)
            .with_found(found)
            .with_primary_label(primary_label)
    }

    pub fn is_internal_sentinel(&self) -> bool {
        matches!(
            self,
            ParseError::MeetPostId | ParseError::MeetPostExtendedCallStart
        )
    }

    pub fn with_code(mut self, code: u32, name: &'static str) -> Self {
        if let Self::Diagnostic(ref mut d) = self {
            d.code = code;
            d.name = name;
        }
        self
    }

    pub fn with_found(mut self, found: TokenKind) -> Self {
        if let Self::Diagnostic(ref mut d) = self {
            d.found = Some(found);
        }
        self
    }

    pub fn with_span(mut self, span: Span) -> Self {
        if let Self::Diagnostic(ref mut d) = self {
            d.span = span;
        }
        self
    }

    pub fn with_fallback_span(mut self, fallback: Span) -> Self {
        if let Self::Diagnostic(ref mut d) = self
            && d.span == DUMMY_SP
        {
            d.span = fallback;
        }
        self
    }

    pub fn with_primary_label(mut self, label: impl Into<String>) -> Self {
        if let Self::Diagnostic(ref mut d) = self {
            d.primary_label = Some(label.into());
        }
        self
    }

    pub fn with_secondary_label(mut self, span: Span, label: impl Into<String>) -> Self {
        if let Self::Diagnostic(ref mut d) = self
            && span != DUMMY_SP
        {
            d.secondary_labels.push((span, label.into()));
        }
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        if let Self::Diagnostic(ref mut d) = self {
            d.notes.push(note.into());
        }
        self
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        if let Self::Diagnostic(ref mut d) = self {
            d.helps.push(help.into());
        }
        self
    }

    pub fn message(&self) -> &str {
        match self {
            ParseError::Diagnostic(d) => &d.message,
            ParseError::MeetPostId => "internal literal-extension boundary",
            ParseError::MeetPostExtendedCallStart => "internal extended-call boundary",
        }
    }

    pub fn to_span(&self) -> Span {
        match self {
            ParseError::Diagnostic(d) => d.span,
            ParseError::MeetPostId | ParseError::MeetPostExtendedCallStart => DUMMY_SP,
        }
    }

    pub fn details(&self) -> Option<&ParseDiag> {
        match self {
            ParseError::Diagnostic(d) => Some(d),
            _ => None,
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message())
    }
}

impl std::error::Error for ParseError {}

impl FlurryError for ParseError {
    fn error_code(&self) -> u32 {
        match self {
            ParseError::Diagnostic(d) => d.code,
            ParseError::MeetPostId | ParseError::MeetPostExtendedCallStart => PARSE_INVALID_SYNTAX,
        }
    }

    fn error_name(&self) -> &'static str {
        match self {
            ParseError::Diagnostic(d) => d.name,
            ParseError::MeetPostId => "meet_post_id",
            ParseError::MeetPostExtendedCallStart => "meet_post_extended_call_start",
        }
    }

    fn emit(&self, diag_ctx: &DiagnosticContext, base_pos: BytePos) {
        let fallback_span = Span::new(base_pos, base_pos);
        match self {
            ParseError::Diagnostic(d) => {
                let primary_span = if d.span == DUMMY_SP {
                    fallback_span
                } else {
                    d.span
                };
                let label = d
                    .primary_label
                    .clone()
                    .unwrap_or_else(|| "syntax error here".to_string());
                let mut builder = diag_ctx
                    .error(d.message.clone())
                    .with_code(d.code)
                    .with_primary_span(primary_span)
                    .with_error_label(primary_span, label);
                for (sec_span, sec_msg) in &d.secondary_labels {
                    if *sec_span != DUMMY_SP {
                        builder = builder.with_label(*sec_span, sec_msg.clone(), Level::Note);
                    }
                }
                for note in &d.notes {
                    builder = builder.with_note(note.clone());
                }
                for help in &d.helps {
                    builder = builder.with_help(help.clone());
                }
                builder.emit(diag_ctx);
            }
            ParseError::MeetPostId | ParseError::MeetPostExtendedCallStart => {}
        }
    }
}

/// Non-fatal parser warning (such as recommending explicit parentheses on
/// multi-layer mixed prefix / infix / postfix expressions or patterns).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseWarning {
    pub code: u32,
    pub message: String,
    pub span: Span,
    pub primary_label: Option<String>,
    pub secondary_labels: Vec<(Span, String)>,
    pub notes: Vec<String>,
    pub helps: Vec<String>,
}

impl ParseWarning {
    pub fn new(code: u32, message: impl Into<String>, span: Span) -> Self {
        Self {
            code,
            message: message.into(),
            span,
            primary_label: None,
            secondary_labels: Vec::new(),
            notes: Vec::new(),
            helps: Vec::new(),
        }
    }

    pub fn with_primary_label(mut self, label: impl Into<String>) -> Self {
        self.primary_label = Some(label.into());
        self
    }

    pub fn with_secondary_label(mut self, span: Span, label: impl Into<String>) -> Self {
        if span != DUMMY_SP {
            self.secondary_labels.push((span, label.into()));
        }
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.helps.push(help.into());
        self
    }

    pub fn emit(&self, diag_ctx: &DiagnosticContext, base_pos: BytePos) {
        let primary_span = if self.span == DUMMY_SP {
            Span::new(base_pos, base_pos)
        } else {
            self.span
        };
        let mut builder = diag_ctx
            .warning(self.message.clone())
            .with_code(self.code)
            .with_primary_span(primary_span);
        if let Some(ref label) = self.primary_label {
            builder = builder.with_label(primary_span, label.clone(), Level::Warning);
        }
        for (sec_span, sec_msg) in &self.secondary_labels {
            if *sec_span != DUMMY_SP {
                builder = builder.with_label(*sec_span, sec_msg.clone(), Level::Note);
            }
        }
        for note in &self.notes {
            builder = builder.with_note(note.clone());
        }
        for help in &self.helps {
            builder = builder.with_help(help.clone());
        }
        builder.emit(diag_ctx);
    }
}
