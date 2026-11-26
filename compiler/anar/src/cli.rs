//! The in-process `luna lsp` entrypoint. Stdout belongs to JSON-RPC in server mode.
use crate::{ANAR_VERSION, LanguageServer, WorkspaceAnalyzer};
use std::ffi::OsString;
use std::io::{self, BufReader, BufWriter};
use std::path::PathBuf;
use std::process::ExitCode;

pub fn help() -> String {
    format!(
        "\
Usage: luna lsp [options]
HIR-stage Language Server for Flurry (luna {ANAR_VERSION})

  --stdio             Communicate over standard input/output (default)
  --sysroot <DIR>      Explicit Flurry library directory
  --check <PATH>       One-shot syntax, early resolution and HIR diagnostics
  -h, --help          Print help
  -V, --version       Print version

--check and --stdio are separate modes. For full type checking, use luna check.
Path arguments need not be UTF-8.\n"
    )
}

#[derive(Debug, PartialEq, Eq)]
enum Request {
    Help,
    Version,
    Serve {
        sysroot: Option<PathBuf>,
    },
    Check {
        sysroot: Option<PathBuf>,
        target: PathBuf,
    },
}

fn parse(args: Vec<OsString>) -> Result<Request, String> {
    let mut sysroot = None;
    let mut target = None;
    let mut stdio = false;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--stdio") => {
                if stdio {
                    return Err("--stdio may only be specified once".into());
                }
                stdio = true;
            }
            Some(flag @ ("--sysroot" | "--check")) => {
                let value = args
                    .next()
                    .filter(|v| !v.is_empty())
                    .ok_or_else(|| format!("{flag} requires a path"))?;
                let slot = if flag == "--sysroot" {
                    &mut sysroot
                } else {
                    &mut target
                };
                if slot.replace(PathBuf::from(value)).is_some() {
                    return Err(format!("{flag} may only be specified once"));
                }
            }
            Some(value) if value.starts_with("--sysroot=") => {
                let value = &value["--sysroot=".len()..];
                if value.is_empty() {
                    return Err("--sysroot requires a path".into());
                }
                if sysroot.replace(PathBuf::from(value)).is_some() {
                    return Err("--sysroot may only be specified once".into());
                }
            }
            Some("-h" | "--help") => return Ok(Request::Help),
            Some("-V" | "--version") => return Ok(Request::Version),
            _ => return Err(format!("unrecognized option {arg:?} (try luna lsp --help)")),
        }
    }
    match target {
        Some(_) if stdio => Err("--check cannot be combined with --stdio".into()),
        Some(target) => Ok(Request::Check { sysroot, target }),
        None => Ok(Request::Serve { sysroot }),
    }
}

pub fn run(args: Vec<OsString>) -> ExitCode {
    let request = match parse(args) {
        Ok(request) => request,
        Err(error) => {
            eprintln!("luna lsp: {error}");
            return ExitCode::from(2);
        }
    };
    match request {
        Request::Help => {
            print!("{}", help());
            ExitCode::SUCCESS
        }
        Request::Version => {
            println!("luna lsp {ANAR_VERSION}");
            ExitCode::SUCCESS
        }
        Request::Check { sysroot, target } => check(sysroot, target),
        Request::Serve { sysroot } => {
            let stdin = io::stdin();
            let stdout = io::stdout();
            let mut reader = BufReader::new(stdin.lock());
            let mut writer = BufWriter::new(stdout.lock());
            match LanguageServer::new(sysroot).run(&mut reader, &mut writer) {
                Ok(0) => ExitCode::SUCCESS,
                Ok(_) => ExitCode::FAILURE,
                Err(error) => {
                    eprintln!("luna lsp: fatal I/O error: {error}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}

fn check(sysroot: Option<PathBuf>, target: PathBuf) -> ExitCode {
    if let Err(error) = target.metadata() {
        eprintln!("luna lsp: cannot read {}: {error}", target.display());
        return ExitCode::FAILURE;
    }
    let mut analyzer = WorkspaceAnalyzer::new(sysroot);
    let snapshot = analyzer.ensure_snapshot(&target);
    let mut errors = false;
    let mut files: Vec<_> = snapshot.diagnostics.keys().collect();
    files.sort();
    for file in files {
        for diagnostic in &snapshot.diagnostics[file] {
            let is_error = diagnostic.severity == crate::analysis::lsp_severity::ERROR;
            errors |= is_error;
            eprintln!(
                "{}:{}:{}: {}{}: {}",
                file.display(),
                diagnostic.range.start.line + 1,
                diagnostic.range.start.character + 1,
                if is_error { "error" } else { "warning" },
                diagnostic
                    .code
                    .as_ref()
                    .map(|code| format!("[{code}]"))
                    .unwrap_or_default(),
                diagnostic.message
            );
        }
    }
    if errors {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_and_options_do_not_mix_protocol_output_with_batch_diagnostics() {
        let parse = |args: &[&str]| super::parse(args.iter().map(OsString::from).collect());
        assert_eq!(parse(&[]).unwrap(), Request::Serve { sysroot: None });
        assert_eq!(
            parse(&["--stdio", "--sysroot", "library"]).unwrap(),
            Request::Serve {
                sysroot: Some("library".into())
            }
        );
        assert_eq!(
            parse(&["--check", "file.fl", "--sysroot=library"]).unwrap(),
            Request::Check {
                sysroot: Some("library".into()),
                target: "file.fl".into()
            }
        );
        for args in [
            vec!["--sysroot"],
            vec!["--check"],
            vec!["--sysroot="],
            vec!["--stdio", "--check", "file.fl"],
            vec!["--stdio", "--stdio"],
            vec!["--sysroot=a", "--sysroot", "b"],
            vec!["--unknown"],
        ] {
            assert!(parse(&args).is_err(), "{args:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn path_arguments_preserve_non_utf8_bytes() {
        use std::os::unix::ffi::OsStringExt;
        let path = OsString::from_vec(b"library-\xff".to_vec());
        assert_eq!(
            parse(vec!["--sysroot".into(), path.clone()]).unwrap(),
            Request::Serve {
                sysroot: Some(path.into())
            }
        );
    }
}
