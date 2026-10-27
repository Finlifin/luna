//! Exercise driver-selected built-in preludes independently of unfinished APIs.
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct Inputs(PathBuf);
impl Inputs {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let serial = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "luna-six-preludes-{}-{nonce}-{serial}",
            std::process::id()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn write(&self, path: &str, source: &str) {
        let path = self.0.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, source).unwrap();
    }
    fn run(&self, source: &str) -> std::process::Output {
        self.write("input.fl", source);
        std::process::Command::new(env!("CARGO_BIN_EXE_luna"))
            .arg(self.0.join("input.fl"))
            .arg("--sysroot")
            .arg(self.0.join("library"))
            .args(["--interpret", "run"])
            .output()
            .unwrap()
    }
}
impl Drop for Inputs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn assert_true(output: std::process::Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"(bool true)\n");
}

#[test]
fn shipped_text_module_exposes_checked_utf8_byte_reads() {
    let input = Inputs::new();
    input.write(
        "library/builtin/lib.fl",
        "pub use primitives.*; pub mod text {pub use text_bytes.*;} pub mod prelude {pub use primitives.*;}",
    );
    input.write(
        "library/builtin/primitives.fl",
        include_str!("../../../library/builtin/primitives.fl"),
    );
    input.write(
        "library/builtin/text_bytes.fl",
        include_str!("../../../library/builtin/text_bytes.fl"),
    );
    for package in ["meta", "build", "attrs", "verification", "std"] {
        input.write(&format!("library/{package}/lib.fl"), "pub mod prelude {}");
    }
    assert_true(input.run("use builtin.text; fn run()->bool {let length=text.byte_length(\"é\");let byte=text.byte(\"é\",1);let valid=byte match {value?=>value == 169,null=>false};length == 2 and valid}"));
}

#[test]
fn five_default_preludes_deduplicate_identity_and_std_stays_explicit() {
    let input = Inputs::new();
    input.write(
        "library/builtin/lib.fl",
        "
        pub mod math {pub fn answer()->bool {true}}
        pub mod prelude {pub fn base()->bool {true} pub use .math;}
    ",
    );
    input.write(
        "library/meta/lib.fl",
        "
        pub mod prelude {pub use builtin.prelude.base; pub fn metadata()->bool {true}}
    ",
    );
    for (package, marker) in [
        ("build", "configuration"),
        ("attrs", "attribute"),
        ("verification", "specification"),
        ("std", "standard"),
    ] {
        input.write(
            &format!("library/{package}/lib.fl"),
            &format!("pub mod prelude {{pub fn {marker}()->bool {{true}}}}"),
        );
    }
    // math is a builtin module. A stale standalone directory must not be loaded.
    input.write(
        "library/math/lib.fl",
        "this is intentionally not a package;",
    );
    assert_true(input.run("fn run()->bool {base() and metadata() and configuration() and attribute() and specification() and math.answer()}"));
    let missing = input.run("fn run()->bool {standard()}");
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("standard"));
    assert_true(
        input.run("use std.prelude.*; fn run()->bool {standard() and base() and metadata()}"),
    );

    // Automatic globs supply local lookup, not a new public export route.
    let private = input.run("use build.base; fn run()->bool {base()}");
    assert!(!private.status.success());
    assert!(String::from_utf8_lossy(&private.stderr).contains("base"));
    assert_true(input.run("fn run()->bool {base() and specification()}"));
}

#[test]
fn optional_provider_absence_is_distinct_from_a_broken_present_provider() {
    let input = Inputs::new();
    input.write(
        "library/builtin/lib.fl",
        "pub mod prelude {pub fn base()->bool {true}}",
    );
    input.write("library/std/lib.fl", "pub mod prelude {}");
    assert_true(input.run("fn run()->bool {base()}"));
    std::fs::create_dir(input.0.join("library/meta")).unwrap();
    let missing_entry = input.run("fn run()->bool {base()}");
    assert!(!missing_entry.status.success());
    assert!(
        String::from_utf8_lossy(&missing_entry.stderr).contains("sysroot package 'meta' requires")
    );
    input.write(
        "library/meta/lib.fl",
        "pub mod prelude {pub fn metadata()->bool {true}}",
    );
    assert_true(input.run("fn run()->bool {base() and metadata()}"));
}
