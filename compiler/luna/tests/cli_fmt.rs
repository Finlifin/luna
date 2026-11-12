//! End-to-end integration tests for `luna fmt`.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "luna_fmt_integration_{tag}_{}_{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn write(&self, rel: &str, content: &str) -> PathBuf {
        let full = self.path.join(rel);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&full, content).unwrap();
        full
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn run_luna(args: &[&str], stdin_data: Option<&str>, cwd: Option<&Path>) -> (i32, String, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_luna"));
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.stdin(if stdin_data.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().expect("failed to spawn luna");
    if let Some(input) = stdin_data {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
    }
    let out = child.wait_with_output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn luna_fmt_defaults_to_current_directory_and_supports_check() {
    let tmp = TempDir::new("cwd_and_check");
    let cfg_raw = ".tab_spaces 2,\n.ignore [\"vendor/**\"],\n";
    let cfg_file = tmp.write("flurryfmt.fl", cfg_raw);
    let src_file = tmp.write(
        "src/lib.fl",
        "use z.item;\nuse a.item;\n\nfn calc(x:i32)->i32{x + 1}\n",
    );
    let ignored_file = tmp.write("vendor/ext.fl", "fn keep(){1}\n");
    let target_file = tmp.write("target/gen.fl", "fn keep2(){1}\n");

    // `luna fmt --check` in `tmp.path` detects diffs and exits 1 without modifying files.
    let (c_check1, out_check1, err_check1) = run_luna(
        &["fmt", "--check", "--color", "never"],
        None,
        Some(&tmp.path),
    );
    assert_eq!(c_check1, 1, "stderr: {err_check1}");
    assert!(out_check1.contains("Diff in "), "stdout:\n{out_check1}");
    assert!(
        fs::read_to_string(&src_file)
            .unwrap()
            .starts_with("use z.item;"),
        "expected `--check` not to modify file"
    );

    // `luna fmt` with no paths recursively formats `.` using `flurryfmt.fl`.
    let (c_fmt, out_fmt, err_fmt) = run_luna(&["fmt"], None, Some(&tmp.path));
    assert_eq!(c_fmt, 0, "stderr: {err_fmt}");
    assert!(out_fmt.is_empty());
    assert_eq!(
        fs::read_to_string(&src_file).unwrap(),
        "use a.item;\nuse z.item;\n\nfn calc(x: i32) -> i32 {\n  x + 1\n}\n"
    );
    // `flurryfmt.fl`, `.ignore`, and `target/` remain untouched.
    assert_eq!(fs::read_to_string(&cfg_file).unwrap(), cfg_raw);
    assert_eq!(fs::read_to_string(&ignored_file).unwrap(), "fn keep(){1}\n");
    assert_eq!(fs::read_to_string(&target_file).unwrap(), "fn keep2(){1}\n");

    // Subsequent `luna fmt --check` exits 0 cleanly.
    let (c_check2, out_check2, err_check2) = run_luna(&["fmt", "--check"], None, Some(&tmp.path));
    assert_eq!(c_check2, 0, "stderr: {err_check2}");
    assert!(out_check2.is_empty());
}

#[test]
fn luna_fmt_dash_streams_stdin_and_preserves_exit_codes() {
    let tmp = TempDir::new("stdin_and_errors");

    // `luna fmt -` streams stdin -> stdout.
    let (c_stdin, out_stdin, err_stdin) = run_luna(
        &["fmt", "-", "--config", "tab_spaces=2"],
        Some("fn id(x:i32)->i32{x}\n"),
        Some(&tmp.path),
    );
    assert_eq!(c_stdin, 0, "stderr: {err_stdin}");
    assert_eq!(out_stdin, "fn id(x: i32) -> i32 {\n  x\n}\n");

    // Syntax error exits 1 and never overwrites the broken file.
    let broken = tmp.write("broken.fl", "fn broken( {\n");
    let (c_syn, _, err_syn) = run_luna(&["fmt", broken.to_str().unwrap()], None, Some(&tmp.path));
    assert_eq!(c_syn, 1);
    assert!(err_syn.contains("syntax error"), "stderr: {err_syn}");
    assert_eq!(fs::read_to_string(&broken).unwrap(), "fn broken( {\n");

    // Unknown CLI flag and invalid `--config` exit 2.
    let (c_bad_flag, _, _) = run_luna(&["fmt", "--no-such-flag"], None, Some(&tmp.path));
    assert_eq!(c_bad_flag, 2);

    let (c_bad_cfg, _, _) = run_luna(
        &["fmt", "--config", "max_width=invalid"],
        None,
        Some(&tmp.path),
    );
    assert_eq!(c_bad_cfg, 2);

    // Top-level `luna --help`, `luna fmt --help`, and `luna help fmt` all document `fmt` and exit 0.
    for help_args in [
        &["--help"][..],
        &["fmt", "--help"][..],
        &["help", "fmt"][..],
    ] {
        let (code, stdout, stderr) = run_luna(help_args, None, Some(&tmp.path));
        assert_eq!(code, 0, "{help_args:?} stderr: {stderr}");
        assert!(stdout.contains("fmt"), "{help_args:?} stdout:\n{stdout}");
    }
}
