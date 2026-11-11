//! Official source code formatter for the Flurry programming language.

pub mod bridge;
pub mod config;
pub mod directives;

pub use config::{ConfigError, FormatConfig, NewlineStyle, SmallHeuristics, WidthHeuristics};
pub use directives::{DirectiveAction, DirectiveLine, DirectiveWarning, ScopedConfigStack};
