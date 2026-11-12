//! Official source code formatter for the Flurry programming language.

pub mod bridge;
pub mod comments;
pub mod config;
pub mod directives;
pub mod doc;
pub mod exprs;
pub mod formatter;
pub mod items;
pub mod patterns;
pub mod verify;

pub use config::{ConfigError, FormatConfig, NewlineStyle, SmallHeuristics, WidthHeuristics};
pub use directives::{DirectiveAction, DirectiveLine, DirectiveWarning, ScopedConfigStack};
pub use formatter::{FormatError, FormatResult, Formatter, format_source};
