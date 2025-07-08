//! Ariadne wrapper for pretty-printing diagnostics

use super::{Diagnostic, DiagnosticContext, Level};
use ariadne::{Color, Label, Report, Source, ColorGenerator};

/// Configuration for diagnostic emission
#[derive(Debug, Clone)]
pub struct EmitterConfig {
    /// Whether to use colors in output
    pub use_colors: bool,
    /// Maximum number of lines to show in context
    pub context_lines: usize,
    /// Whether to show line numbers
    pub show_line_numbers: bool,
}

impl Default for EmitterConfig {
    fn default() -> Self {
        Self {
            use_colors: true,
            context_lines: 3,
            show_line_numbers: true,
        }
    }
}

/// Ariadne-based diagnostic emitter
pub struct AriadneEmitter {
    config: EmitterConfig,
}

impl AriadneEmitter {
    pub fn new(config: EmitterConfig) -> Self {
        Self { config }
    }

    pub fn new_default() -> Self {
        Self::new(EmitterConfig::default())
    }

    /// Emit a single diagnostic
    pub fn emit_diagnostic(&self, diagnostic: &Diagnostic, context: &DiagnosticContext) {
        let primary_span = diagnostic.primary_span.unwrap_or_else(|| {
            diagnostic.labels.first()
                .map(|label| label.span)
                .unwrap_or(rustc_span::DUMMY_SP)
        });

        let source_file = context.source_map().lookup_source_file(primary_span.lo());
        let _colors = ColorGenerator::new();
        let file_name = format!("{:?}", source_file.name);
        
        // Convert byte positions to character positions for ariadne
        let source_content = match &source_file.src {
            Some(content) => content.as_str(),
            None => {
                eprintln!("Error: Source file content not available");
                return;
            }
        };
        
        let byte_start = (primary_span.lo().0 - source_file.start_pos.0) as usize;
        let byte_end = (primary_span.hi().0 - source_file.start_pos.0) as usize;
        
        // Debug: 输出字节和字符位置信息
        // eprintln!("Debug: byte_start={}, byte_end={}, file_start_pos={}", 
        //     byte_start, byte_end, source_file.start_pos.0);
        // eprintln!("Debug: source content length: {} bytes, {} chars", 
        //     source_content.len(), source_content.chars().count());
        
        // Convert byte indices to character indices by counting UTF-8 chars
        let char_start = source_content.get(..byte_start.min(source_content.len()))
            .map(|s| s.chars().count())
            .unwrap_or(0);
        let char_end = source_content.get(..byte_end.min(source_content.len()))
            .map(|s| s.chars().count())
            .unwrap_or(char_start);
        
        // Debug: 输出转换后的字符位置
        eprintln!("Debug: char_start={}, char_end={}", char_start, char_end);

        let mut report = Report::build(
            diagnostic.level.to_ariadne_kind(),
            (&file_name, char_start..char_end)
        );

        if let Some(code) = diagnostic.code {
            report = report.with_code(code);
        }

        report = report.with_message(&diagnostic.message);

        // Add labels with different colors
        for label in &diagnostic.labels {
            let label_file = context.source_map().lookup_source_file(label.span.lo());
            if std::ptr::eq(label_file.as_ref(), source_file.as_ref()) {
                let color = match label.level {
                    Level::Error => Color::Red,
                    Level::Warning => Color::Yellow,
                    Level::Note => Color::Blue,
                    Level::Help => Color::Cyan,
                };

                let label_byte_start = (label.span.lo().0 - source_file.start_pos.0) as usize;
                let label_byte_end = (label.span.hi().0 - source_file.start_pos.0) as usize;
                
                // Convert byte indices to character indices for label
                let label_char_start = source_content.get(..label_byte_start.min(source_content.len()))
                    .map(|s| s.chars().count())
                    .unwrap_or(0);
                let label_char_end = source_content.get(..label_byte_end.min(source_content.len()))
                    .map(|s| s.chars().count())
                    .unwrap_or(label_char_start);
                
                report = report.with_label(
                    Label::new((&file_name, label_char_start..label_char_end))
                        .with_message(&label.message)
                        .with_color(color)
                );
            }
        }

        // Add notes
        for note in &diagnostic.notes {
            report = report.with_note(note);
        }

        // Add helps
        for help in &diagnostic.helps {
            report = report.with_help(help);
        }

        // Print the report
        let source_content = match &source_file.src {
            Some(content) => content.as_str(),
            None => {
                eprintln!("Error: Source file content not available");
                return;
            }
        };
        
        if let Err(e) = report.finish().print((&file_name, Source::from(source_content))) {
            eprintln!("Error printing diagnostic: {}", e);
        }
    }

    /// Emit all diagnostics from a context
    pub fn emit_all(&self, context: &DiagnosticContext) {
        for diagnostic in context.diagnostics() {
            self.emit_diagnostic(diagnostic, context);
        }
    }
}

impl Level {
    pub fn name(&self) -> &'static str {
        match self {
            Level::Error => "error",
            Level::Warning => "warning", 
            Level::Note => "note",
            Level::Help => "help",
        }
    }
}
