//! Standalone `flurryfmt` CLI binary entry point.

use std::process::ExitCode;

fn main() -> ExitCode {
    fmt::run_cli(std::env::args_os().skip(1), "flurryfmt", false)
}
