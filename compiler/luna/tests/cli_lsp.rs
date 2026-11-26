//! The LSP service runs in the single luna executable, without stdout banners.
use anar::json::JsonValue;
use anar::rpc::{RpcMessage, read_rpc_message, write_rpc_payload};
use std::ffi::OsString;
use std::io::{BufReader, Cursor, Read, Write};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

fn run(args: Vec<OsString>, input: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_luna"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let output = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let errors = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).unwrap();
        bytes
    });
    child.stdin.take().unwrap().write_all(input).unwrap();
    let deadline = Instant::now() + Duration::from_secs(180);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("luna lsp exceeded deadline");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    Output {
        status,
        stdout: output.join().unwrap(),
        stderr: errors.join().unwrap(),
    }
}

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

#[test]
fn help_version_and_invalid_options_do_not_enter_the_protocol() {
    for flags in [&["help", "lsp"][..], &["lsp", "--help"]] {
        let output = run(args(flags), &[]);
        assert!(output.status.success(), "{output:?}");
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains("Usage: luna lsp"));
        assert!(text.contains("For full type checking, use luna check"));
    }
    let version = run(args(&["lsp", "--version"]), &[]);
    assert!(version.status.success());
    assert!(
        String::from_utf8(version.stdout)
            .unwrap()
            .starts_with("luna lsp ")
    );
    for flags in [
        &["lsp", "--unknown"][..],
        &["lsp", "--sysroot"],
        &["lsp", "--check"],
        &["lsp", "--stdio", "--check", "file.fl"],
        &["packages", "lsp"],
        &["help", "packages", "lsp"],
    ] {
        let output = run(args(flags), &[]);
        assert_eq!(output.status.code(), Some(2), "{flags:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
    }
}

#[test]
fn stdio_preserves_framing_shutdown_and_abnormal_exit_status() {
    for shutdown in [false, true] {
        let mut input = Vec::new();
        let initialize = JsonValue::parse(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":null}}"#,
        )
        .unwrap();
        write_rpc_payload(&mut input, &initialize).unwrap();
        if shutdown {
            let message =
                JsonValue::parse(r#"{"jsonrpc":"2.0","id":2,"method":"shutdown","params":null}"#)
                    .unwrap();
            write_rpc_payload(&mut input, &message).unwrap();
        }
        let exit = JsonValue::parse(r#"{"jsonrpc":"2.0","method":"exit","params":null}"#).unwrap();
        write_rpc_payload(&mut input, &exit).unwrap();
        let output = run(args(&["lsp", "--stdio"]), &input);
        assert_eq!(
            output.status.code(),
            Some(if shutdown { 0 } else { 1 }),
            "{output:?}"
        );
        assert!(output.stderr.is_empty(), "{output:?}");
        let mut reader = BufReader::new(Cursor::new(output.stdout));
        let message = read_rpc_message(&mut reader).unwrap().unwrap();
        let RpcMessage::Response {
            result: Some(result),
            error: None,
            ..
        } = message
        else {
            panic!("initialize did not return a response: {message:?}");
        };
        assert_eq!(
            result
                .get("serverInfo")
                .and_then(|info| info.get_str("name")),
            Some("luna")
        );
        if shutdown {
            assert!(matches!(
                read_rpc_message(&mut reader).unwrap(),
                Some(RpcMessage::Response { error: None, .. })
            ));
        }
        assert!(read_rpc_message(&mut reader).unwrap().is_none());
    }
}

#[test]
fn batch_hir_check_reports_errors_and_keeps_sources_unchanged() {
    let root = std::env::temp_dir().join(format!("luna-lsp-check-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let file = root.join("main.fl");
    let sysroot = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    for (source, expected) in [("fn run() {}", 0), ("fn broken( {", 1)] {
        std::fs::write(&file, source).unwrap();
        let output = run(
            vec![
                "lsp".into(),
                "--check".into(),
                file.clone().into(),
                "--sysroot".into(),
                sysroot.clone().into(),
            ],
            &[],
        );
        assert_eq!(output.status.code(), Some(expected), "{output:?}");
        assert!(output.stdout.is_empty());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), source);
        if expected != 0 {
            assert!(String::from_utf8_lossy(&output.stderr).contains("error"));
        }
    }
    let missing = run(
        vec![
            "lsp".into(),
            "--check".into(),
            root.join("missing.fl").into(),
        ],
        &[],
    );
    assert_eq!(missing.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("cannot read"));
    std::fs::remove_dir_all(root).unwrap();
}
