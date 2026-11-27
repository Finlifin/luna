use ariadne::{Color, ColorGenerator, Label, Report, ReportKind};
use rustc_span::{FileNameDisplayPreference, SourceMap, Span};
use std::{
    cell::{Cell, Ref, RefCell},
    fmt,
    io::{self, Write},
};

// 罢了, warning也用这个trait吧
pub trait FlurryError {
    fn error_code(&self) -> u32;

    fn error_name(&self) -> &'static str;

    fn emit(&self, diag_ctx: &DiagnosticContext, base_pos: rustc_span::BytePos);
}

/// Diagnostic severity level
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Error,
    Warning,
    Note,
    Help,
}

impl Level {
    pub fn to_ariadne_kind(&self) -> ReportKind {
        match self {
            Level::Error => ReportKind::Error,
            Level::Warning => ReportKind::Warning,
            Level::Note | Level::Help => ReportKind::Advice,
        }
    }

    pub fn color(&self) -> Color {
        match self {
            Level::Error => Color::Red,
            Level::Warning => Color::Yellow,
            Level::Note => Color::Blue,
            Level::Help => Color::Cyan,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Level::Error => "error",
            Level::Warning => "warning",
            Level::Note => "note",
            Level::Help => "help",
        }
    }
}

/// A diagnostic message with location information
#[derive(Debug, Clone)]
pub struct DiagnosticMessage {
    pub span: Span,
    pub message: String,
    pub level: Level,
}

impl DiagnosticMessage {
    pub fn new(span: Span, message: String, level: Level) -> Self {
        Self {
            span,
            message,
            level,
        }
    }
}

/// A complete diagnostic with primary message and optional sub-diagnostics
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub level: Level,
    pub code: Option<u32>,
    pub message: String,
    pub primary_span: Option<Span>,
    pub labels: Vec<DiagnosticMessage>,
    pub notes: Vec<String>,
    pub helps: Vec<String>,
}

impl Diagnostic {
    pub fn new(level: Level, message: String) -> Self {
        Self {
            level,
            code: None,
            message,
            primary_span: None,
            labels: Vec::new(),
            notes: Vec::new(),
            helps: Vec::new(),
        }
    }

    pub fn error(message: String) -> Self {
        Self::new(Level::Error, message)
    }

    pub fn warning(message: String) -> Self {
        Self::new(Level::Warning, message)
    }

    pub fn note(message: String) -> Self {
        Self::new(Level::Note, message)
    }

    pub fn help(message: String) -> Self {
        Self::new(Level::Help, message)
    }
}

/// Builder for constructing diagnostics
pub struct DiagnosticBuilder {
    diagnostic: Diagnostic,
}

impl DiagnosticBuilder {
    pub fn new(level: Level, message: String) -> Self {
        Self {
            diagnostic: Diagnostic::new(level, message),
        }
    }

    pub fn error(message: String) -> Self {
        Self::new(Level::Error, message)
    }

    pub fn warning(message: String) -> Self {
        Self::new(Level::Warning, message)
    }

    pub fn note(message: String) -> Self {
        Self::new(Level::Note, message)
    }

    pub fn help(message: String) -> Self {
        Self::new(Level::Help, message)
    }

    pub fn with_code(mut self, code: u32) -> Self {
        self.diagnostic.code = Some(code);
        self
    }

    pub fn with_primary_span(mut self, span: Span) -> Self {
        self.diagnostic.primary_span = Some(span);
        self
    }

    pub fn with_label(mut self, span: Span, message: String, level: Level) -> Self {
        self.diagnostic
            .labels
            .push(DiagnosticMessage::new(span, message, level));
        self
    }

    pub fn with_error_label(self, span: Span, message: String) -> Self {
        self.with_label(span, message, Level::Error)
    }

    pub fn with_warning_label(self, span: Span, message: String) -> Self {
        self.with_label(span, message, Level::Warning)
    }

    pub fn with_note_label(self, span: Span, message: String) -> Self {
        self.with_label(span, message, Level::Note)
    }

    pub fn with_help_label(self, span: Span, message: String) -> Self {
        self.with_label(span, message, Level::Help)
    }

    pub fn with_note(mut self, note: String) -> Self {
        self.diagnostic.notes.push(note);
        self
    }

    pub fn with_help(mut self, help: String) -> Self {
        self.diagnostic.helps.push(help);
        self
    }

    pub fn build(self) -> Diagnostic {
        self.diagnostic
    }

    pub fn emit(self, context: &DiagnosticContext) {
        context.emit(self.diagnostic);
    }
}

/// Context for managing and emitting diagnostics
pub struct DiagnosticContext<'a> {
    source_map: &'a SourceMap,
    emitted_diagnostics: RefCell<Vec<Diagnostic>>,
    error_count: Cell<usize>,
    warning_count: Cell<usize>,
}

impl<'a> DiagnosticContext<'a> {
    pub fn new(source_map: &'a SourceMap) -> Self {
        Self {
            source_map,
            emitted_diagnostics: RefCell::new(Vec::new()),
            error_count: Cell::new(0),
            warning_count: Cell::new(0),
        }
    }

    pub fn source_map(&self) -> &SourceMap {
        self.source_map
    }

    pub fn emit(&self, diagnostic: Diagnostic) {
        match diagnostic.level {
            Level::Error => self.error_count.set(self.error_count.get() + 1),
            Level::Warning => self.warning_count.set(self.warning_count.get() + 1),
            _ => {}
        }

        // Emit to ariadne
        self.emit_to_ariadne(&diagnostic);

        // Store for later analysis
        self.emitted_diagnostics.borrow_mut().push(diagnostic);
    }

    pub fn error_count(&self) -> usize {
        self.error_count.get()
    }

    pub fn warning_count(&self) -> usize {
        self.warning_count.get()
    }

    pub fn has_errors(&self) -> bool {
        self.error_count.get() > 0
    }

    pub fn has_warnings(&self) -> bool {
        self.warning_count.get() > 0
    }

    /// The borrow guard keeps stored diagnostics alive until the reader is done.
    pub fn diagnostics(&self) -> Ref<'_, [Diagnostic]> {
        Ref::map(self.emitted_diagnostics.borrow(), Vec::as_slice)
    }

    /// Plain, stable diagnostics for library callers, independent of terminal
    /// rendering. Retain stage, source locations, codes and explanatory labels.
    pub fn failure_summary(&self, stage: &str) -> String {
        use std::fmt::Write as _;
        let mut output = format!("{stage} failed");
        let location = |output: &mut String, span: Span| {
            if span != rustc_span::DUMMY_SP {
                let loc = self.source_map.lookup_char_pos(span.lo());
                let _ = write!(
                    output,
                    "{}:{}:{}: ",
                    loc.file.name.display(FileNameDisplayPreference::Local),
                    loc.line,
                    loc.col.0 + 1
                );
            }
        };
        for diagnostic in self
            .diagnostics()
            .iter()
            .filter(|value| value.level == Level::Error)
        {
            output.push('\n');
            if let Some(span) = diagnostic
                .primary_span
                .or_else(|| diagnostic.labels.first().map(|label| label.span))
            {
                location(&mut output, span);
            }
            output.push_str("error");
            if let Some(code) = diagnostic.code {
                let _ = write!(output, "[E{code:04}]");
            }
            let _ = write!(output, ": {}", diagnostic.message);
            for label in &diagnostic.labels {
                output.push_str("\n  ");
                location(&mut output, label.span);
                let _ = write!(output, "{}: {}", label.level.name(), label.message);
            }
            for note in &diagnostic.notes {
                let _ = write!(output, "\n  note: {note}");
            }
            for help in &diagnostic.helps {
                let _ = write!(output, "\n  help: {help}");
            }
        }
        output
    }

    /// Create a new diagnostic builder
    pub fn error(&self, message: String) -> DiagnosticBuilder {
        DiagnosticBuilder::error(message)
    }

    pub fn warning(&self, message: String) -> DiagnosticBuilder {
        DiagnosticBuilder::warning(message)
    }

    pub fn note(&self, message: String) -> DiagnosticBuilder {
        DiagnosticBuilder::note(message)
    }

    pub fn help(&self, message: String) -> DiagnosticBuilder {
        DiagnosticBuilder::help(message)
    }

    /// Render a diagnostic to a UTF-8 string (useful for testing and plain callers).
    pub fn render_to_string(&self, diagnostic: &Diagnostic) -> String {
        let mut buf = Vec::new();
        self.render_diagnostic(diagnostic, &mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    }

    /// Emit diagnostic using ariadne (with plain-text fallback for `DUMMY_SP` or
    /// missing source text).
    fn emit_to_ariadne(&self, diagnostic: &Diagnostic) {
        let mut stderr = io::stderr().lock();
        self.render_diagnostic(diagnostic, &mut stderr);
    }

    fn write_plain_diagnostic(&self, diagnostic: &Diagnostic, writer: &mut dyn Write) {
        let _ = write!(writer, "{}", diagnostic.level.name());
        if let Some(code) = diagnostic.code {
            let _ = write!(writer, "[E{code:04}]");
        }
        let _ = writeln!(writer, ": {}", diagnostic.message);
        for label in &diagnostic.labels {
            if label.span != rustc_span::DUMMY_SP && !self.source_map.files().is_empty() {
                let loc = self.source_map.lookup_char_pos(label.span.lo());
                let _ = writeln!(
                    writer,
                    "  {}:{}:{}: {}: {}",
                    loc.file.name.display(FileNameDisplayPreference::Local),
                    loc.line,
                    loc.col.0 + 1,
                    label.level.name(),
                    label.message
                );
            } else {
                let _ = writeln!(writer, "  {}: {}", label.level.name(), label.message);
            }
        }
        for note in &diagnostic.notes {
            let _ = writeln!(writer, "  note: {note}");
        }
        for help in &diagnostic.helps {
            let _ = writeln!(writer, "  help: {help}");
        }
    }

    fn render_diagnostic(&self, diagnostic: &Diagnostic, writer: &mut dyn Write) {
        let primary_span = diagnostic
            .primary_span
            .filter(|&span| span != rustc_span::DUMMY_SP)
            .or_else(|| {
                diagnostic
                    .labels
                    .iter()
                    .map(|label| label.span)
                    .find(|&span| span != rustc_span::DUMMY_SP)
            })
            .unwrap_or(rustc_span::DUMMY_SP);

        if primary_span == rustc_span::DUMMY_SP || self.source_map.files().is_empty() {
            self.write_plain_diagnostic(diagnostic, writer);
            return;
        }

        let source_file = self.source_map.lookup_source_file(primary_span.lo());
        let primary_content = match &source_file.src {
            Some(content) => content.as_str(),
            None => {
                self.write_plain_diagnostic(diagnostic, writer);
                return;
            }
        };

        let byte_to_char =
            |content: &str, base: rustc_span::BytePos, pos: rustc_span::BytePos| -> usize {
                let raw_offset = pos.0.saturating_sub(base.0) as usize;
                let clamped = raw_offset.min(content.len());
                if let Some(prefix) = content.get(..clamped) {
                    prefix.chars().count()
                } else {
                    let mut idx = clamped;
                    while idx > 0 && !content.is_char_boundary(idx) {
                        idx -= 1;
                    }
                    content[..idx].chars().count()
                }
            };

        let primary_id = format!(
            "{}",
            source_file
                .name
                .display(FileNameDisplayPreference::Local)
                .to_string_lossy()
        );

        let char_start = byte_to_char(primary_content, source_file.start_pos, primary_span.lo());
        let char_end =
            byte_to_char(primary_content, source_file.start_pos, primary_span.hi()).max(char_start);

        let mut report = Report::build(
            diagnostic.level.to_ariadne_kind(),
            (primary_id.clone(), char_start..char_end),
        );

        if let Some(code) = diagnostic.code {
            report = report.with_code(code);
        }

        report = report.with_message(&diagnostic.message);

        let mut colors = ColorGenerator::new();
        let mut owned_sources: Vec<(String, std::sync::Arc<rustc_span::SourceFile>)> =
            vec![(primary_id.clone(), source_file.clone())];
        let mut seen_files: Vec<(usize, String)> = vec![(
            source_file.as_ref() as *const _ as usize,
            primary_id.clone(),
        )];

        let mut extra_notes: Vec<String> = Vec::new();

        for label in &diagnostic.labels {
            if label.span == rustc_span::DUMMY_SP {
                extra_notes.push(format!("{}: {}", label.level.name(), label.message));
                continue;
            }
            let label_file = self.source_map.lookup_source_file(label.span.lo());
            let Some(label_content) = label_file.src.as_deref() else {
                let loc = self.source_map.lookup_char_pos(label.span.lo());
                extra_notes.push(format!(
                    "{}:{}:{}: {}: {}",
                    loc.file.name.display(FileNameDisplayPreference::Local),
                    loc.line,
                    loc.col.0 + 1,
                    label.level.name(),
                    label.message
                ));
                continue;
            };

            let label_ptr = label_file.as_ref() as *const _ as usize;
            let label_id = if let Some((_, existing_id)) =
                seen_files.iter().find(|(ptr, _)| *ptr == label_ptr)
            {
                existing_id.clone()
            } else {
                let base_name = format!(
                    "{}",
                    label_file
                        .name
                        .display(FileNameDisplayPreference::Local)
                        .to_string_lossy()
                );
                let mut candidate = base_name.clone();
                let mut suffix = 1usize;
                while owned_sources.iter().any(|(id, _)| id == &candidate) {
                    candidate = format!("{base_name}#{suffix}");
                    suffix += 1;
                }
                seen_files.push((label_ptr, candidate.clone()));
                owned_sources.push((candidate.clone(), label_file.clone()));
                candidate
            };

            let label_char_start =
                byte_to_char(label_content, label_file.start_pos, label.span.lo());
            let label_char_end = byte_to_char(label_content, label_file.start_pos, label.span.hi())
                .max(label_char_start);

            let color = colors.next();
            report = report.with_label(
                Label::new((label_id, label_char_start..label_char_end))
                    .with_message(&label.message)
                    .with_color(color),
            );
        }

        for note in extra_notes.iter().chain(diagnostic.notes.iter()) {
            report = report.with_note(note);
        }

        for help in &diagnostic.helps {
            report = report.with_help(help);
        }

        let file_sources: Vec<(String, &str)> = owned_sources
            .iter()
            .filter_map(|(id, sf)| sf.src.as_deref().map(|src| (id.clone(), src.as_str())))
            .collect();

        if let Err(e) = report
            .finish()
            .write(ariadne::sources(file_sources), &mut *writer)
        {
            let _ = writeln!(writer, "Error printing diagnostic: {}", e);
        }
    }
}

impl<'a> fmt::Debug for DiagnosticContext<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DiagnosticContext")
            .field("error_count", &self.error_count)
            .field("warning_count", &self.warning_count)
            .field(
                "diagnostics_count",
                &self.emitted_diagnostics.borrow().len(),
            )
            .finish()
    }
}

#[cfg(test)]
mod context_tests {
    use super::*;
    use rustc_span::source_map::FilePathMapping;

    #[test]
    fn plain_failure_retains_explanations_without_terminal_output() {
        let map = SourceMap::new(FilePathMapping::empty());
        let context = DiagnosticContext::new(&map);
        let diagnostic = DiagnosticBuilder::error("cannot resolve value".into())
            .with_code(42)
            .with_note("lookup searched the current scope".into())
            .with_help("declare the value before this use".into())
            .build();
        context
            .emitted_diagnostics
            .borrow_mut()
            .extend([Diagnostic::warning("unrelated warning".into()), diagnostic]);
        assert_eq!(
            context.failure_summary("resolution"),
            "resolution failed\nerror[E0042]: cannot resolve value\n  note: lookup searched the current scope\n  help: declare the value before this use"
        );
        assert_eq!(
            context.diagnostics().len(),
            2,
            "formatting must not consume diagnostics"
        );
    }

    #[test]
    fn diagnostic_view_keeps_its_storage_borrowed() {
        let map = SourceMap::new(FilePathMapping::empty());
        let context = DiagnosticContext::new(&map);
        context
            .emitted_diagnostics
            .borrow_mut()
            .push(Diagnostic::error("first".into()));
        let view = context.diagnostics();
        assert_eq!(view[0].message, "first");
        assert!(context.emitted_diagnostics.try_borrow_mut().is_err());
        drop(view);
        context
            .emitted_diagnostics
            .borrow_mut()
            .push(Diagnostic::error("second".into()));
        assert_eq!(context.diagnostics().len(), 2);
    }

    #[test]
    fn dummy_span_with_bytepos_one_source_does_not_underflow() {
        use rustc_span::{BytePos, DUMMY_SP, FileName};

        let map = SourceMap::new(FilePathMapping::empty());
        // Simulate a dummy 1-byte reservation at BytePos(0) so real files start at BytePos(1),
        // or directly register a file whose start_pos > DUMMY_SP.lo() (0) via offset padding.
        let pad = map.new_source_file(FileName::Custom("dummy".into()), " ".into());
        assert_eq!(pad.start_pos, BytePos(0));
        let real = map.new_source_file(FileName::Custom("main.fl".into()), "fn main() {}".into());
        assert!(real.start_pos >= BytePos(1));

        let context = DiagnosticContext::new(&map);
        let diag = DiagnosticBuilder::error("missing entry point".into())
            .with_code(7)
            .with_primary_span(DUMMY_SP)
            .with_note("checked package root".into())
            .with_help("add fn main()".into())
            .build();

        let rendered = context.render_to_string(&diag);
        assert!(
            rendered.contains("error[E0007]: missing entry point"),
            "{rendered}"
        );
        assert!(
            rendered.contains("note: checked package root"),
            "{rendered}"
        );
        assert!(rendered.contains("help: add fn main()"), "{rendered}");

        context.emit(diag);
        assert_eq!(context.error_count(), 1);

        // Also test None primary_span + empty labels on non-empty SourceMap starting at BytePos(1).
        let bare = DiagnosticBuilder::warning("standalone warning".into()).build();
        let bare_rendered = context.render_to_string(&bare);
        assert!(
            bare_rendered.contains("warning: standalone warning"),
            "{bare_rendered}"
        );
        context.emit(bare);
        assert_eq!(context.warning_count(), 1);
    }

    #[test]
    fn dummy_secondary_label_alongside_valid_primary_span_does_not_underflow() {
        use rustc_span::{BytePos, DUMMY_SP, FileName};

        let map = SourceMap::new(FilePathMapping::empty());
        let _ = map.new_source_file(FileName::Custom("pad".into()), " ".into());
        let sf = map.new_source_file(FileName::Custom("a.fl".into()), "let x = 1;".into());
        let primary = Span::new(sf.start_pos + BytePos(4), sf.start_pos + BytePos(5));

        let context = DiagnosticContext::new(&map);
        let diag = DiagnosticBuilder::error("invalid binding".into())
            .with_primary_span(primary)
            .with_error_label(primary, "bound here".into())
            .with_note_label(DUMMY_SP, "synthetic context".into())
            .build();

        let rendered = context.render_to_string(&diag);
        assert!(rendered.contains("a.fl"), "{rendered}");
        assert!(rendered.contains("bound here"), "{rendered}");
        assert!(rendered.contains("synthetic context"), "{rendered}");
    }

    #[test]
    fn cross_file_secondary_labels_are_rendered() {
        use rustc_span::{BytePos, FileName};

        let map = SourceMap::new(FilePathMapping::empty());
        let def_file = map.new_source_file(
            FileName::Custom("defs.fl".into()),
            "pub fn helper(a: bool) -> bool { a }".into(),
        );
        let use_file = map.new_source_file(
            FileName::Custom("caller.fl".into()),
            "fn main() { helper(1); }".into(),
        );

        let call_span = Span::new(
            use_file.start_pos + BytePos(12),
            use_file.start_pos + BytePos(21),
        );
        let def_span = Span::new(
            def_file.start_pos + BytePos(7),
            def_file.start_pos + BytePos(13),
        );

        let context = DiagnosticContext::new(&map);
        let diag = DiagnosticBuilder::error("mismatched argument type".into())
            .with_code(101)
            .with_primary_span(call_span)
            .with_error_label(call_span, "expected bool here".into())
            .with_note_label(def_span, "helper defined in defs.fl".into())
            .build();

        let rendered = context.render_to_string(&diag);
        assert!(rendered.contains("caller.fl"), "{rendered}");
        assert!(rendered.contains("expected bool here"), "{rendered}");
        assert!(rendered.contains("defs.fl"), "{rendered}");
        assert!(rendered.contains("helper defined in defs.fl"), "{rendered}");
    }
}
