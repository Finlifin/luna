//! CLI entry point for `anar` — the Flurry Language Server.

use std::env;
use std::io::{self, BufReader, BufWriter};
use std::path::PathBuf;
use std::process;

use anar::{ANAR_VERSION, LanguageServer, WorkspaceAnalyzer};

fn print_help() {
    println!(
        "\
anar {ANAR_VERSION}
Official HIR-stage Language Server for the Flurry programming language

USAGE:
    anar [OPTIONS]

OPTIONS:
    --stdio             Communicate over standard input/output (default)
    --sysroot <DIR>     Explicit path to the Flurry `library/` sysroot directory
    --check <PATH>      Run one-shot syntax/resolve/HIR check on a file or package directory
    -h, --help          Print help information
    -V, --version       Print version information"
    );
}

fn main() {
    let mut sysroot: Option<PathBuf> = None;
    let mut check_target: Option<PathBuf> = None;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--stdio" => {}
            "--sysroot" => {
                let Some(dir) = args.next() else {
                    eprintln!("anar: `--sysroot` requires a directory argument");
                    process::exit(2);
                };
                sysroot = Some(PathBuf::from(dir));
            }
            s if s.starts_with("--sysroot=") => {
                sysroot = Some(PathBuf::from(&s["--sysroot=".len()..]));
            }
            "--check" => {
                let Some(target) = args.next() else {
                    eprintln!("anar: `--check` requires a file or directory path");
                    process::exit(2);
                };
                check_target = Some(PathBuf::from(target));
            }
            "-h" | "--help" => {
                print_help();
                return;
            }
            "-V" | "--version" => {
                println!("anar {ANAR_VERSION}");
                return;
            }
            unknown => {
                eprintln!("anar: unrecognized option `{unknown}` (try `anar --help`)");
                process::exit(2);
            }
        }
    }

    if let Some(target) = check_target {
        let mut analyzer = WorkspaceAnalyzer::new(sysroot);
        let snap = analyzer.ensure_snapshot(&target).clone();
        let mut error_count = 0usize;
        let mut files: Vec<_> = snap.diagnostics.keys().cloned().collect();
        files.sort();
        for f in files {
            if let Some(diags) = snap.diagnostics.get(&f) {
                for d in diags {
                    if d.severity == anar::analysis::lsp_severity::ERROR {
                        error_count += 1;
                    }
                    eprintln!(
                        "{}:{}:{}: {}{}: {}",
                        f.display(),
                        d.range.start.line + 1,
                        d.range.start.character + 1,
                        if d.severity == 1 { "error" } else { "warning" },
                        d.code
                            .as_ref()
                            .map(|c| format!("[{c}]"))
                            .unwrap_or_default(),
                        d.message
                    );
                }
            }
        }
        if error_count > 0 {
            process::exit(1);
        }
        return;
    }

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = BufWriter::new(stdout.lock());

    let mut server = LanguageServer::new(sysroot);
    match server.run(&mut reader, &mut writer) {
        Ok(code) => process::exit(code),
        Err(err) => {
            eprintln!("anar: fatal I/O error: {err}");
            process::exit(1);
        }
    }
}
