//! Public command routing, rejection boundaries and executable behavior.
use luna::cli::{self, Command, Emit, HostOutput};
use std::{ffi::OsString, process::Command as Process};
#[path = "support/primitive_bindings.rs"]
mod primitive_bindings;
fn parse(args: &[&str]) -> Result<Command, String> {
    cli::parse(args.iter().map(OsString::from))
}
#[test]
fn command_defaults_select_distinct_pipelines() {
    assert!(matches!(
        parse(&["check", "input.fl"]).unwrap(),
        Command::Check(_)
    ));
    assert!(
        matches!(parse(&["dump", "input.fl"]).unwrap(), Command::Compile(o) if o.emit == Emit::Nir)
    );
    assert!(
        matches!(parse(&["build", "input.fl", "-o", "app"]).unwrap(), Command::Host {entry, output_kind:HostOutput::Executable,..} if entry == "main")
    );
    assert!(matches!(
        parse(&["test", "input.fl"]).unwrap(),
        Command::Test {
            native: true,
            list_only: false,
            ..
        }
    ));
    assert!(matches!(
        parse(&["test", "input.fl", "--engine", "reference"]).unwrap(),
        Command::Test { native: false, .. }
    ));
    assert!(
        matches!(parse(&["test", "input.fl", "--list", "--filter", "hello"]).unwrap(), Command::Test {list_only:true,filter:Some(f),..} if f == "hello")
    );
    assert!(
        matches!(parse(&["interpret", "input.fl"]).unwrap(), Command::Interpret {entry,..} if entry == "main")
    );
    assert!(
        matches!(parse(&["comptime", "input.fl", "--entry", "inspect"]).unwrap(), Command::Invoke {entry,..} if entry == "inspect")
    );
}
#[test]
fn formats_and_analysis_routes_preserve_options() {
    for (format, expected) in [
        ("ast", Emit::Ast),
        ("hir", Emit::Hir),
        ("nir", Emit::Nir),
        ("attributes", Emit::Attributes),
        ("merged-attributes", Emit::MergedAttributes),
    ] {
        assert!(
            matches!(parse(&["dump","input.fl","--emit",format]).unwrap(), Command::Compile(o) if o.emit == expected)
        );
    }
    assert!(
        matches!(parse(&["dump","input.fl","--emit","iir","--entry","run"]).unwrap(), Command::Instantiate{entry,..} if entry=="run")
    );
    for (format, expected) in [("llvm", HostOutput::Llvm), ("asm", HostOutput::Assembly)] {
        assert!(
            matches!(parse(&["dump","input.fl","--emit",format]).unwrap(), Command::Host{output_kind,..} if output_kind==expected)
        );
    }
    for (format, expected) in [
        ("object", HostOutput::Object),
        ("bitcode", HostOutput::Bitcode),
    ] {
        assert!(
            matches!(parse(&["build","input.fl","--emit",format,"-o","artifact"]).unwrap(), Command::Host{output_kind,..} if output_kind==expected)
        );
    }
    assert!(matches!(
        parse(&["analyze", "text-regions", "input.fl"]).unwrap(),
        Command::AnalyzeTextRegions { .. }
    ));
    assert!(matches!(
        parse(&[
            "analyze",
            "resources",
            "input.fl",
            "--copy-protocol",
            "Copy"
        ])
        .unwrap(),
        Command::AnalyzeResources { .. }
    ));
}
#[test]
fn misplaced_duplicate_and_conflicting_actions_are_rejected() {
    for args in [
        vec!["buid", "input.fl"],
        vec!["check"],
        vec!["build", "input.fl"],
        vec!["check", "input.fl", "--output", "out"],
        vec!["check", "input.fl", "--emit", "ast"],
        vec!["test", "input.fl", "--build", "main"],
        vec!["test", "input.fl", "--engine", "bad"],
        vec!["test", "input.fl", "--list", "--engine", "native"],
        vec![
            "test",
            "input.fl",
            "--engine",
            "reference",
            "--timeout-ms",
            "1",
        ],
        vec!["dump", "input.fl", "--entry", "main"],
        vec!["comptime", "input.fl"],
        vec!["interpret", "input.fl", "--entry", "one", "--entry", "two"],
        vec!["dump", "one.fl", "two.fl"],
        vec!["dump", "--sysroot", "--test"],
        vec!["analyze", "unknown", "input.fl"],
        vec!["serve", "unknown"],
        vec!["packages", "check"],
        vec!["packages", "dump", "input.fl"],
    ] {
        assert!(parse(&args).is_err(), "accepted {args:?}");
    }
}
#[test]
fn package_and_stream_commands_keep_versioned_input_contracts() {
    assert!(matches!(
        parse(&[
            "packages",
            "dump",
            "--package",
            "1",
            "101",
            "app",
            "input.fl",
            "--primary",
            "1",
            "--emit",
            "hir"
        ])
        .unwrap(),
        Command::Packages(_)
    ));
    assert!(matches!(
        parse(&["serve", "packages-v1", "--cache-entries", "0"]).unwrap(),
        Command::PackageStream { capacity: 0 }
    ));
    for command in [
        "check",
        "build",
        "test",
        "dump",
        "comptime",
        "interpret",
        "analyze",
        "packages",
        "serve",
    ] {
        let Command::HelpTopic(text) = parse(&[command, "--help"]).unwrap() else {
            panic!("missing help")
        };
        assert!(text.contains(&format!("Usage: luna {command}")));
        assert_eq!(parse(&["help", command]).unwrap(), Command::HelpTopic(text));
    }
    assert_eq!(parse(&[]).unwrap(), Command::Help);
    assert!(
        matches!(parse(&["--version"]).unwrap(), Command::HelpTopic(text)
        if text == format!("luna {}\n", env!("CARGO_PKG_VERSION")))
    );
}
#[cfg(unix)]
#[test]
fn non_utf8_paths_and_dash_prefixed_inputs_are_preserved() {
    use std::os::unix::ffi::OsStringExt;
    let path = OsString::from_vec(vec![b'-', 0xff]);
    let Command::Compile(options) = cli::parse(["dump".into(), "--".into(), path.clone()]).unwrap()
    else {
        panic!()
    };
    assert_eq!(options.input.as_os_str(), path);
}
#[test]
fn executable_subcommands_check_dump_invoke_interpret_and_test() {
    let directory = std::env::temp_dir().join(format!("luna-subcommands-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let input = directory.join("input.fl");
    primitive_bindings::write_fixture(
        &input,
        "comptime fn inspect()->bool {true} fn main()->bool {true} test {assumes true;}",
    )
    .unwrap();
    for args in [
        vec!["check"],
        vec!["dump", "--emit", "ast"],
        vec!["comptime", "--entry", "inspect"],
        vec!["interpret"],
        vec!["test", "--list"],
        vec!["test", "--engine", "reference"],
        vec!["test"],
    ] {
        let output = Process::new(env!("CARGO_BIN_EXE_luna"))
            .args(&args)
            .arg(&input)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        if args[0] == "check" {
            assert!(output.stdout.is_empty());
        } else {
            assert!(!output.stdout.is_empty(), "{args:?}");
        }
    }
    let output = Process::new(env!("CARGO_BIN_EXE_luna"))
        .args(["check", "--emit", "ast"])
        .arg(&input)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn executable_native_build_and_package_check_use_subcommands() {
    let directory =
        std::env::temp_dir().join(format!("luna-subcommands-native-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let input = directory.join("input.fl");
    let executable = directory.join("program");
    primitive_bindings::write_fixture(&input, "fn main()->bool {true}").unwrap();
    let output = Process::new(env!("CARGO_BIN_EXE_luna"))
        .arg("build")
        .arg(&input)
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(Process::new(&executable).output().unwrap().status.success());
    let output = Process::new(env!("CARGO_BIN_EXE_luna"))
        .args(["packages", "check", "--package", "1", "101", "app"])
        .arg(&input)
        .args(["--primary", "1"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    std::fs::remove_dir_all(directory).unwrap();
}
