//! Flurry macro expansion engine (template macros, construction macros, provenance, and budgets).
pub mod ast_input;
pub mod construction;
pub mod define_macros;
pub mod imports;
pub mod materialize;
pub mod peg;
pub mod provenance;
pub mod raw_source;
pub mod registry;
pub mod template;

pub use ast_input::{AstArgValue, AstParamShape, ParsedSyntaxNode, ParsedSyntaxTree};
pub use peg::{
    ParsedPegNode, ParsedPegTree, PegArgValue, PegExpr, PegGrammar, PegParamShape, PegRule,
};

use diagnostic::{DiagnosticBuilder, DiagnosticContext, FlurryError};
use lex::{LexError, Symbol, Token, TokenKind};
pub use materialize::{ExpandedFile, materialize_expanded, tokens_to_macro_tokens};
pub use provenance::{
    ExpansionId, ExpansionKind, ExpansionMap, ExpansionRecord, MacroToken, TokenSpacing,
    escape_sexp_str, is_separated_kind,
};
pub use raw_source::{ReferencedMacros, scan_referenced_macro_roots};
pub use registry::{
    AstNodeKind, ExpansionContext, MacroDescriptor, MacroExecutor, MacroId, MacroInputMode,
    MacroRegistry, MacroVisibility,
};
use rustc_span::{BytePos, SourceFile, SourceMap, Span};
use std::collections::HashMap;
use std::fmt;

/// Bounded execution/materialization limits for macro expansion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MacroBudget {
    pub max_depth: usize,
    pub max_tokens: usize,
    pub max_steps: usize,
    pub max_memo_entries: usize,
    pub used_tokens: usize,
    pub used_steps: usize,
}

impl Default for MacroBudget {
    fn default() -> Self {
        Self {
            max_depth: 64,
            max_tokens: 1_000_000,
            max_steps: 5_000_000,
            max_memo_entries: 2_000_000,
            used_tokens: 0,
            used_steps: 0,
        }
    }
}

impl MacroBudget {
    pub fn consume_step(&mut self) -> Result<(), MacroError> {
        self.used_steps = self.used_steps.saturating_add(1);
        if self.used_steps > self.max_steps {
            return Err(MacroError::new(
                MacroErrorKind::BudgetExhausted,
                format!("macro expansion step budget ({}) exhausted", self.max_steps),
                rustc_span::DUMMY_SP,
            ));
        }
        Ok(())
    }

    pub fn consume_tokens(&mut self, count: usize) -> Result<(), MacroError> {
        self.used_tokens = self.used_tokens.saturating_add(count);
        if self.used_tokens > self.max_tokens {
            return Err(MacroError::new(
                MacroErrorKind::BudgetExhausted,
                format!(
                    "macro expansion output token budget ({}) exhausted",
                    self.max_tokens
                ),
                rustc_span::DUMMY_SP,
            ));
        }
        Ok(())
    }
}

/// Classification of macro expansion failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MacroErrorKind {
    InvalidTemplateDefinition,
    InvalidTemplateBody,
    InvalidTemplateCall,
    UndefinedTemplateMacro,
    DepthExceeded,
    BudgetExhausted,
    InvalidMacroImport,
    UndefinedMacro,
    AmbiguousMacro,
    MacroVisibilityError,
    DelimiterMismatch,
    AstInputParseError,
    PegGrammarError,
    PegInputParseError,
    HandlerExecutionError,
    InvalidMacroOutput,
    PreparationCycle,
    UnterminatedMacroInput,
}

impl MacroErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidTemplateDefinition => "invalid-template-definition",
            Self::InvalidTemplateBody => "invalid-template-body",
            Self::InvalidTemplateCall => "invalid-template-call",
            Self::UndefinedTemplateMacro => "undefined-template-macro",
            Self::DepthExceeded => "depth-exceeded",
            Self::BudgetExhausted => "budget-exhausted",
            Self::InvalidMacroImport => "invalid-macro-import",
            Self::UndefinedMacro => "undefined-macro",
            Self::AmbiguousMacro => "ambiguous-macro",
            Self::MacroVisibilityError => "macro-visibility-error",
            Self::DelimiterMismatch => "delimiter-mismatch",
            Self::AstInputParseError => "ast-input-parse-error",
            Self::PegGrammarError => "peg-grammar-error",
            Self::PegInputParseError => "peg-input-parse-error",
            Self::HandlerExecutionError => "handler-execution-error",
            Self::InvalidMacroOutput => "invalid-macro-output",
            Self::PreparationCycle => "preparation-cycle",
            Self::UnterminatedMacroInput => "unterminated-macro-input",
        }
    }

    pub fn error_code(self) -> u32 {
        match self {
            Self::InvalidTemplateDefinition => 4001,
            Self::InvalidTemplateBody => 4002,
            Self::InvalidTemplateCall => 4003,
            Self::UndefinedTemplateMacro => 4004,
            Self::DepthExceeded => 4005,
            Self::BudgetExhausted => 4006,
            Self::InvalidMacroImport => 4007,
            Self::UndefinedMacro => 4008,
            Self::AmbiguousMacro => 4009,
            Self::MacroVisibilityError => 4010,
            Self::DelimiterMismatch => 4011,
            Self::AstInputParseError => 4012,
            Self::PegGrammarError => 4013,
            Self::PegInputParseError => 4014,
            Self::HandlerExecutionError => 4015,
            Self::InvalidMacroOutput => 4016,
            Self::PreparationCycle => 4017,
            Self::UnterminatedMacroInput => 4018,
        }
    }
}

/// Structured macro diagnostic with provenance chain and Lisp S-expression `Display`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MacroError {
    pub kind: MacroErrorKind,
    pub message: String,
    pub span: Span,
    pub secondary_labels: Vec<(Span, String)>,
    pub expansion_chain: Vec<ExpansionRecord>,
    pub notes: Vec<String>,
}

impl MacroError {
    pub fn new(kind: MacroErrorKind, message: impl Into<String>, span: Span) -> Self {
        Self {
            kind,
            message: message.into(),
            span,
            secondary_labels: Vec::new(),
            expansion_chain: Vec::new(),
            notes: Vec::new(),
        }
    }

    pub fn with_span(mut self, span: Span) -> Self {
        if self.span == rustc_span::DUMMY_SP {
            self.span = span;
        }
        self
    }

    pub fn with_label(mut self, span: Span, message: impl Into<String>) -> Self {
        self.secondary_labels.push((span, message.into()));
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    pub fn with_chain(mut self, provenance: &ExpansionMap, exp_id: Option<ExpansionId>) -> Self {
        if let Some(id) = exp_id {
            self.expansion_chain = provenance.chain(id);
        }
        self
    }

    pub fn with_chain_if_empty(
        mut self,
        provenance: &ExpansionMap,
        exp_id: Option<ExpansionId>,
        fallback_span: Span,
    ) -> Self {
        if self.span == rustc_span::DUMMY_SP {
            self.span = fallback_span;
        }
        if self.expansion_chain.is_empty() {
            if let Some(id) = exp_id {
                self.expansion_chain = provenance.chain(id);
            }
        }
        self
    }
}

impl fmt::Display for MacroError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(macro-error (kind {}) (message \"{}\") (span {} {})",
            self.kind.as_str(),
            escape_sexp_str(&self.message),
            self.span.lo().0,
            self.span.hi().0,
        )?;
        if !self.expansion_chain.is_empty() {
            f.write_str(" (chain")?;
            for rec in &self.expansion_chain {
                write!(f, " {rec}")?;
            }
            f.write_str(")")?;
        }
        f.write_str(")")
    }
}

impl std::error::Error for MacroError {}

impl FlurryError for MacroError {
    fn error_code(&self) -> u32 {
        self.kind.error_code()
    }

    fn error_name(&self) -> &'static str {
        self.kind.as_str()
    }

    fn emit(&self, diag_ctx: &DiagnosticContext, base_pos: BytePos) {
        let primary = if self.span == rustc_span::DUMMY_SP {
            Span::new(base_pos, base_pos)
        } else {
            self.span
        };
        let mut builder = DiagnosticBuilder::error(self.message.clone())
            .with_code(self.error_code())
            .with_primary_span(primary);

        for (span, label) in &self.secondary_labels {
            if *span != rustc_span::DUMMY_SP {
                builder = builder.with_note_label(*span, label.clone());
            }
        }
        for rec in &self.expansion_chain {
            if rec.call_site != rustc_span::DUMMY_SP {
                builder = builder.with_note_label(
                    rec.call_site,
                    format!("expanded from {} macro `{}` here", rec.kind, rec.macro_name),
                );
            }
            builder = builder.with_note(format!(
                "in expansion of {} macro `{}`",
                rec.kind, rec.macro_name
            ));
        }
        for note in &self.notes {
            builder = builder.with_note(note.clone());
        }
        builder.emit(diag_ctx);
    }
}

/// Fast-path / full expansion entrypoint for a single lexed source file.
pub fn expand_source(
    source_map: &SourceMap,
    source_file: &SourceFile,
    tokens: Vec<Token>,
    symbols: HashMap<usize, Symbol>,
    budget: &mut MacroBudget,
) -> Result<ExpandedFile, MacroError> {
    let src = source_file.src.as_deref().map(String::as_str).unwrap_or("");
    let has_template_syntax = tokens
        .iter()
        .any(|tok| matches!(tok.kind, TokenKind::Define | TokenKind::Dollar));

    if !has_template_syntax {
        return Ok(ExpandedFile {
            tokens,
            symbols,
            start_pos: source_file.start_pos,
            expanded: false,
            provenance: ExpansionMap::new(),
            source_text: None,
        });
    }

    let macro_tokens = tokens_to_macro_tokens(&tokens, src, source_file.start_pos);
    let mut provenance = ExpansionMap::new();
    let (expanded_tokens, expanded_any) =
        template::expand_template_macros(macro_tokens, &mut provenance, budget)?;

    if !expanded_any {
        return Ok(ExpandedFile {
            tokens,
            symbols,
            start_pos: source_file.start_pos,
            expanded: false,
            provenance,
            source_text: None,
        });
    }

    Ok(materialize_expanded(
        source_map,
        source_file,
        &expanded_tokens,
        provenance,
    ))
}

/// Pre-lexing full macro expansion entrypoint for one source file using built-in macros only.
pub fn expand_source_file(
    source_map: &SourceMap,
    source_file: &SourceFile,
    budget: &mut MacroBudget,
) -> Result<(ExpandedFile, Vec<LexError>), MacroError> {
    let registry = MacroRegistry::with_builtins();
    let mut ctx = ExpansionContext::with_registry(&registry);
    expand_source_file_with_context(source_map, source_file, &mut ctx, budget)
}

/// Pre-lexing full macro expansion entrypoint for one source file.
///
/// Runs the Mode 4 / `@use` raw-source scanner before normal Flurry lexing so raw macro
/// bodies never fail ordinary lexing, then runs unified template + Mode 3 (`^macro3`) expansion.
pub fn expand_source_file_with_context(
    source_map: &SourceMap,
    source_file: &SourceFile,
    ctx: &mut ExpansionContext<'_>,
    budget: &mut MacroBudget,
) -> Result<(ExpandedFile, Vec<LexError>), MacroError> {
    let src = source_file.src.as_deref().map(String::as_str).unwrap_or("");
    let mut provenance = ExpansionMap::new();

    let (raw_tokens, macro_scope, raw_modified) = if src.contains('@') {
        let raw_pass = raw_source::process_raw_source_and_imports(
            src,
            source_file.start_pos,
            ctx,
            &mut provenance,
            budget,
        )?;
        if raw_pass.modified {
            if !raw_pass.lex_errors.is_empty() {
                return Ok((
                    ExpandedFile {
                        tokens: Vec::new(),
                        symbols: HashMap::new(),
                        start_pos: source_file.start_pos,
                        expanded: false,
                        provenance,
                        source_text: None,
                    },
                    raw_pass.lex_errors,
                ));
            }
            (Some(raw_pass.tokens), raw_pass.scope, true)
        } else {
            (None, raw_pass.scope, false)
        }
    } else {
        (None, imports::MacroScope::new(), false)
    };

    if let Some(macro_tokens) = raw_tokens {
        let needs_token_pass = src.contains('@')
            || macro_tokens.iter().any(|tok| {
                matches!(
                    tok.kind,
                    TokenKind::Define | TokenKind::Dollar | TokenKind::At
                )
            });
        let final_tokens = if needs_token_pass {
            let (after_tokens, _) = construction::expand_all_tokens(
                macro_tokens,
                &macro_scope,
                ctx,
                &mut provenance,
                budget,
            )?;
            after_tokens
        } else {
            macro_tokens
        };
        return Ok((
            materialize_expanded(source_map, source_file, &final_tokens, provenance),
            Vec::new(),
        ));
    }

    let (tokens, symbols, lex_errors) = lex::lex(src, source_file.start_pos);
    if !lex_errors.is_empty() {
        return Ok((
            ExpandedFile {
                tokens,
                symbols,
                start_pos: source_file.start_pos,
                expanded: false,
                provenance,
                source_text: None,
            },
            lex_errors,
        ));
    }

    let needs_token_pass = src.contains('@')
        || tokens.iter().any(|tok| {
            matches!(
                tok.kind,
                TokenKind::Define | TokenKind::Dollar | TokenKind::At
            )
        });
    if !needs_token_pass {
        return Ok((
            ExpandedFile {
                tokens,
                symbols,
                start_pos: source_file.start_pos,
                expanded: false,
                provenance,
                source_text: None,
            },
            Vec::new(),
        ));
    }

    let macro_tokens = tokens_to_macro_tokens(&tokens, src, source_file.start_pos);
    let (final_tokens, token_expanded) =
        construction::expand_all_tokens(macro_tokens, &macro_scope, ctx, &mut provenance, budget)?;
    if !raw_modified && !token_expanded {
        return Ok((
            ExpandedFile {
                tokens,
                symbols,
                start_pos: source_file.start_pos,
                expanded: false,
                provenance,
                source_text: None,
            },
            Vec::new(),
        ));
    }

    Ok((
        materialize_expanded(source_map, source_file, &final_tokens, provenance),
        Vec::new(),
    ))
}
