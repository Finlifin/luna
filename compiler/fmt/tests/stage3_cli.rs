use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "flurryfmt_stage3_{prefix}_{}_{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        Self { path: dir }
    }

    fn write(&self, rel: &str, content: &str) -> PathBuf {
        let full = self.path.join(rel);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).expect("create parent dir");
        }
        fs::write(&full, content).expect("write file");
        full
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn flurryfmt_bin() -> &'static str {
    env!("CARGO_BIN_EXE_flurryfmt")
}

fn run_flurryfmt(
    args: &[&str],
    stdin_data: Option<&str>,
    cwd: Option<&Path>,
) -> (i32, String, String) {
    let mut cmd = Command::new(flurryfmt_bin());
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    if stdin_data.is_some() {
        cmd.stdin(Stdio::piped());
    } else {
        cmd.stdin(Stdio::null());
    }
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().expect("spawn flurryfmt");
    if let Some(data) = stdin_data {
        child
            .stdin
            .as_mut()
            .expect("stdin pipe")
            .write_all(data.as_bytes())
            .expect("write stdin");
    }
    let out = child.wait_with_output().expect("wait flurryfmt");
    let code = out.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    (code, stdout, stderr)
}

#[test]
fn stdin_default_and_dash_format_to_stdout() {
    let input = "pub inline pure fn add(a:i32,b:i32)->i32{a + b}\n";
    let expected = "pub pure inline fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n";

    let (code1, out1, err1) = run_flurryfmt(&[], Some(input), None);
    assert_eq!(code1, 0, "stderr: {err1}");
    assert_eq!(out1, expected);

    let (code2, out2, err2) = run_flurryfmt(&["-"], Some(input), None);
    assert_eq!(code2, 0, "stderr: {err2}");
    assert_eq!(out2, expected);
}

#[test]
fn check_diff_color_and_files_with_diff_modes() {
    let tmp = TempDir::new("check_diff");
    let file_a = tmp.write(
        "a.fl",
        "use zeta.item;\nuse alpha.item;\n\nfn calc(x:i32)->i32{x + 1}\n",
    );
    let file_b = tmp.write("b.fl", "fn clean(x: i32) -> i32 {\n    x\n}\n");

    // `--check --color never` exits 1 and prints unified diff header & +/- lines.
    let (code, out, err) = run_flurryfmt(
        &[
            "--check",
            "--color",
            "never",
            file_a.to_str().unwrap(),
            file_b.to_str().unwrap(),
        ],
        None,
        None,
    );
    assert_eq!(code, 1, "stderr: {err}");
    assert!(out.contains("Diff in "), "stdout:\n{out}");
    assert!(
        out.contains("-use zeta.item;\n use alpha.item;\n+use zeta.item;"),
        "stdout:\n{out}"
    );
    assert!(
        out.contains("-fn calc(x:i32)->i32{x + 1}\n+fn calc(x: i32) -> i32 {"),
        "stdout:\n{out}"
    );
    assert!(
        !out.contains("\x1b["),
        "unexpected ANSI in never mode:\n{out}"
    );
    assert!(
        !out.contains("b.fl"),
        "clean file should not appear:\n{out}"
    );

    // `--check --color always` emits ANSI color escapes.
    let (code_c, out_c, _) = run_flurryfmt(
        &["--check", "--color=always", file_a.to_str().unwrap()],
        None,
        None,
    );
    assert_eq!(code_c, 1);
    assert!(out_c.contains("\x1b[1mDiff in "), "stdout:\n{out_c}");
    assert!(out_c.contains("\x1b[31m-"), "stdout:\n{out_c}");
    assert!(out_c.contains("\x1b[32m+"), "stdout:\n{out_c}");

    // `--check -l` prints only the path with diffs.
    let (code_l, out_l, _) = run_flurryfmt(
        &[
            "--check",
            "-l",
            file_a.to_str().unwrap(),
            file_b.to_str().unwrap(),
        ],
        None,
        None,
    );
    assert_eq!(code_l, 1);
    assert_eq!(out_l.trim(), file_a.display().to_string());
}

#[test]
fn inplace_backup_and_clean_check_roundtrip() {
    let tmp = TempDir::new("backup");
    let orig_a = "fn messy(x:i32)->i32{x + 1}\n";
    let orig_b = "fn already_clean() {}\n";
    let file_a = tmp.write("a.fl", orig_a);
    let file_b = tmp.write("b.fl", orig_b);

    let (code, out, err) = run_flurryfmt(
        &[
            "--backup",
            "-l",
            file_a.to_str().unwrap(),
            file_b.to_str().unwrap(),
        ],
        None,
        None,
    );
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(out.trim(), file_a.display().to_string());

    // `a.fl.bk` created with original contents; `b.fl.bk` NOT created since `b.fl` was unchanged!
    let bk_a = tmp.path.join("a.fl.bk");
    let bk_b = tmp.path.join("b.fl.bk");
    assert!(bk_a.is_file());
    assert!(!bk_b.exists());
    assert_eq!(fs::read_to_string(&bk_a).unwrap(), orig_a);
    assert_eq!(
        fs::read_to_string(&file_a).unwrap(),
        "fn messy(x: i32) -> i32 {\n    x + 1\n}\n"
    );

    // Re-running `--check` now exits 0 with empty stdout!
    let (code2, out2, err2) = run_flurryfmt(&["--check", file_a.to_str().unwrap()], None, None);
    assert_eq!(code2, 0, "stderr: {err2}");
    assert!(out2.is_empty());
}

#[test]
fn directory_walk_respects_hidden_target_config_and_ignore() {
    let tmp = TempDir::new("dir_walk");
    tmp.write(
        "flurryfmt.fl",
        ".tab_spaces 2,\n.ignore [\"generated/**\", \"*.gen.fl\"],\n",
    );
    let src_file = tmp.write("src/lib.fl", "fn greet(){hello()}\n");
    let ignored_dir_file = tmp.write("generated/out.fl", "fn untouched(){1}\n");
    let ignored_glob_file = tmp.write("src/schema.gen.fl", "fn untouched2(){1}\n");
    let hidden_file = tmp.write(".git/hooks.fl", "fn untouched3(){1}\n");
    let target_file = tmp.write("target/build.fl", "fn untouched4(){1}\n");

    let (code, _, err) = run_flurryfmt(&[tmp.path.to_str().unwrap()], None, None);
    assert_eq!(code, 0, "stderr: {err}");

    // `src/lib.fl` formatted using `tab_spaces = 2` from `flurryfmt.fl`!
    assert_eq!(
        fs::read_to_string(&src_file).unwrap(),
        "fn greet() {\n  hello()\n}\n"
    );
    // All excluded paths remain untouched!
    assert_eq!(
        fs::read_to_string(&ignored_dir_file).unwrap(),
        "fn untouched(){1}\n"
    );
    assert_eq!(
        fs::read_to_string(&ignored_glob_file).unwrap(),
        "fn untouched2(){1}\n"
    );
    assert_eq!(
        fs::read_to_string(&hidden_file).unwrap(),
        "fn untouched3(){1}\n"
    );
    assert_eq!(
        fs::read_to_string(&target_file).unwrap(),
        "fn untouched4(){1}\n"
    );
}

#[test]
fn stdin_filepath_discovers_config_and_honors_ignore() {
    let tmp = TempDir::new("stdin_fp");
    tmp.write("flurryfmt.fl", ".tab_spaces 2,\n.ignore [\"vendor/**\"],\n");
    let virtual_active = tmp.path.join("src/virtual.fl");
    let virtual_ignored = tmp.path.join("vendor/third_party.fl");

    let (c1, out1, err1) = run_flurryfmt(
        &["--stdin-filepath", virtual_active.to_str().unwrap()],
        Some("fn demo(){call()}\n"),
        None,
    );
    assert_eq!(c1, 0, "stderr: {err1}");
    assert_eq!(out1, "fn demo() {\n  call()\n}\n");

    // Ignored `--stdin-filepath` passes input through unchanged and exits 0 even under `--check`!
    let unformatted = "fn demo(){call()}\n";
    let (c2, out2, _) = run_flurryfmt(
        &["--stdin-filepath", virtual_ignored.to_str().unwrap()],
        Some(unformatted),
        None,
    );
    assert_eq!(c2, 0);
    assert_eq!(out2, unformatted);

    let (c3, out3, _) = run_flurryfmt(
        &[
            "--check",
            "--stdin-filepath",
            virtual_ignored.to_str().unwrap(),
        ],
        Some(unformatted),
        None,
    );
    assert_eq!(c3, 0);
    assert!(out3.is_empty());
}

#[test]
fn print_config_emit_stdout_quiet_and_exit_codes() {
    let tmp = TempDir::new("print_and_errors");
    tmp.write("flurryfmt.fl", ".max_width 88,\n");

    let (c_min, out_min, _) = run_flurryfmt(&["--print-config", "minimal"], None, Some(&tmp.path));
    assert_eq!(c_min, 0);
    assert_eq!(out_min, ".max_width 88,\n");

    let (c_def, out_def, _) = run_flurryfmt(&["--print-config=default"], None, Some(&tmp.path));
    assert_eq!(c_def, 0);
    assert!(out_def.contains(".max_width 100,"));
    assert!(out_def.contains(".tab_spaces 4,"));

    // Syntax error exits 1 without modifying the broken file, while still formatting valid siblings!
    let broken = tmp.write("broken.fl", "fn oops( {\n");
    let valid = tmp.write("valid.fl", "fn ok(x:i32)->i32{x}\n");
    let (c_err, _, stderr_err) = run_flurryfmt(
        &[broken.to_str().unwrap(), valid.to_str().unwrap()],
        None,
        None,
    );
    assert_eq!(c_err, 1);
    assert!(stderr_err.contains("syntax error"), "stderr:\n{stderr_err}");
    assert_eq!(fs::read_to_string(&broken).unwrap(), "fn oops( {\n");
    assert_eq!(
        fs::read_to_string(&valid).unwrap(),
        "fn ok(x: i32) -> i32 {\n    x\n}\n"
    );

    // Unknown `--%` directive warns on stderr normally, suppressed with `-q`.
    let warn_src = "fn w() {\n    --% .not_a_real_directive\n    let x = 1;\n}\n";
    let (cw1, _, ew1) = run_flurryfmt(&[], Some(warn_src), None);
    assert_eq!(cw1, 0);
    assert!(
        ew1.contains("unknown `--%` formatter directive `.not_a_real_directive`"),
        "stderr: {ew1}"
    );

    let (cw2, _, ew2) = run_flurryfmt(&["-q"], Some(warn_src), None);
    assert_eq!(cw2, 0);
    assert!(ew2.is_empty(), "expected quiet stderr, got: {ew2}");

    // Invalid CLI flags and invalid `--config` return exit code 2!
    let (bad1, _, _) = run_flurryfmt(&["--definitely-unknown"], None, None);
    assert_eq!(bad1, 2);

    let (bad2, _, _) = run_flurryfmt(&["--config", "max_width=not_a_num"], None, None);
    assert_eq!(bad2, 2);

    let (bad3, _, _) = run_flurryfmt(&["-", valid.to_str().unwrap()], None, None);
    assert_eq!(bad3, 2);

    let (bad4, _, _) = run_flurryfmt(&["--check", "--emit", "files"], None, None);
    assert_eq!(bad4, 2);

    // `--help` and `--version` exit 0.
    let (ch, out_h, _) = run_flurryfmt(&["--help"], None, None);
    assert_eq!(ch, 0);
    assert!(out_h.contains("Usage: flurryfmt"));

    let (cv, out_v, _) = run_flurryfmt(&["--version"], None, None);
    assert_eq!(cv, 0);
    assert!(out_v.starts_with("flurryfmt "));
}

#[test]
fn shared_driver_supports_luna_fmt_default_cwd_and_dash_stdin() {
    let tmp = TempDir::new("luna_mode");
    let f = tmp.write("sub/mod.fl", "fn f(a:i32)->i32{a}\n");

    // Explicit `-` in `default_to_cwd = true` reads stdin -> stdout.
    let mut stdin_buf: &[u8] = b"fn s(x:i32)->i32{x}\n";
    let mut stdout_buf = Vec::new();
    let mut stderr_buf = Vec::new();
    let code = fmt::run_cli_with_io(
        ["-"],
        "luna fmt",
        true,
        &mut stdin_buf,
        &mut stdout_buf,
        &mut stderr_buf,
        false,
        false,
    );
    assert_eq!(code, 0);
    assert_eq!(
        String::from_utf8(stdout_buf).unwrap(),
        "fn s(x: i32) -> i32 {\n    x\n}\n"
    );

    // Passing directory path formats `sub/mod.fl`.
    let mut empty_in: &[u8] = b"";
    let mut out2 = Vec::new();
    let mut err2 = Vec::new();
    let code2 = fmt::run_cli_with_io(
        [tmp.path.to_str().unwrap()],
        "luna fmt",
        true,
        &mut empty_in,
        &mut out2,
        &mut err2,
        false,
        false,
    );
    assert_eq!(code2, 0);
    assert_eq!(
        fs::read_to_string(&f).unwrap(),
        "fn f(a: i32) -> i32 {\n    a\n}\n"
    );
}
