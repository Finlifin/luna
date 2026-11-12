//! Official source code formatter for the Flurry programming language.

pub mod bridge;
pub mod cli;
pub mod comments;
pub mod config;
pub mod diff;
pub mod directives;
pub mod doc;
pub mod exprs;
pub mod formatter;
pub mod items;
pub mod patterns;
pub mod verify;

pub use cli::{
    ColorChoice, EXIT_CLI_OR_CONFIG_ERR, EXIT_DIFF_OR_FORMAT_ERR, EXIT_OK, EmitMode,
    PrintConfigMode, render_help, run_cli, run_cli_with_io,
};
pub use config::{ConfigError, FormatConfig, NewlineStyle, SmallHeuristics, WidthHeuristics};
pub use diff::{DiffLine, Mismatch, make_diff, render_check_diff};
pub use directives::{DirectiveAction, DirectiveLine, DirectiveWarning, ScopedConfigStack};
pub use formatter::{FormatError, FormatResult, Formatter, format_source};
