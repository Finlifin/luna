//! End-to-end parsing, elaboration, verification, and LLVM host execution for `static` globals.
#[path = "support/primitive_bindings.rs"]
mod primitive_bindings;

use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

struct Work(PathBuf);

impl Work {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "luna-static-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        primitive_bindings::write_fixture(path.join("source.fl"), source).unwrap();
        Self(path)
    }

    fn build(&self) -> Output {
        let mut c = Command::new(env!("CARGO_BIN_EXE_luna"));
        c.current_dir(&self.0).arg("source.fl").args([
            "--build",
            "run",
            "--copy-protocol",
            "Duplicate",
            "--drop-protocol",
            "Retire",
            "--output",
            "native program",
        ]);
        c.output().unwrap()
    }

    fn execute(&self) -> Output {
        let mut command = Command::new(self.0.join("native program"));
        command.current_dir(&self.0);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        loop {
            match command.output() {
                Err(error)
                    if error.raw_os_error() == Some(26) && std::time::Instant::now() < deadline =>
                {
                    std::thread::sleep(
                        std::time::Duration::from_millis(2)
                            .min(deadline.saturating_duration_since(std::time::Instant::now())),
                    );
                }
                result => return result.unwrap(),
            }
        }
    }
}

impl Drop for Work {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn native(source: &str, expected: &str) {
    let work = Work::new(source);
    let build = work.build();
    assert!(
        build.status.success(),
        "build failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr),
    );
    let run = work.execute();
    assert!(
        run.status.success(),
        "execution failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr),
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout), expected);
}

#[test]
fn static_global_read_write_across_functions_executes_natively() {
    native(
        r#"
        trait Duplicate {}
        trait Retire { fn drop(*self); }
        typealias I = .i64'builtin;
        impl Duplicate for I {}
        static counter: I = 10;

        fn bump(delta: I) -> I
        where requires delta >= 0 and delta <= 100
        {
            let c = counter;
            if c >= 0 and c <= 1000 {
                counter = c + delta;
            }
            counter
        }

        fn run() -> I {
            let a = bump(5);
            let b = bump(7);
            let c = counter;
            if a >= 0 and a <= 1000 and b >= 0 and b <= 1000 and c >= 0 and c <= 1000 {
                a + b + c
            } else {
                0
            }
        }
        "#,
        "(signed-integer 59)\n",
    );
}

#[test]
fn static_uninitialized_declaration_and_struct_field_access_work() {
    native(
        r#"
        trait Duplicate {}
        trait Retire { fn drop(*self); }
        typealias I = .i64'builtin;
        impl Duplicate for I {}

        struct Pair {
            left: I,
            right: I,
        }
        impl Duplicate for Pair {}

        static slot: I;
        static pair: Pair = Pair { left: 1 + 2, right: 4 };

        mod inner {
            pub static offset: I = 10;
        }

        fn run() -> I {
            slot = 12;
            let l = pair.left;
            if l >= 0 and l <= 1000 {
                pair.left = l + 5;
            }
            let rp = pair.right.ref;
            rp.* = 20;
            let sp = slot.ref;
            let sv = sp.*;
            let pl = pair.left;
            let pr = pair.right;
            let off = inner.offset;
            if sv >= 0 and sv <= 1000 and pl >= 0 and pl <= 1000 and pr >= 0 and pr <= 1000 and off >= 0 and off <= 1000 {
                sv + pl + pr + off
            } else {
                0
            }
        }
        "#,
        "(signed-integer 50)\n",
    );
}

#[test]
fn static_rejects_comptime_access_pure_access_and_comptime_only_type() {
    let comptime_read = Work::new(
        r#"
        trait Duplicate {}
        trait Retire { fn drop(*self); }
        typealias I = .i64'builtin;
        impl Duplicate for I {}
        static g: I = 7;
        comptime fn read_g() -> I { g }
        const x: I = read_g();
        fn run() -> I { x }
        "#,
    );
    let comptime_out = comptime_read.build();
    assert!(!comptime_out.status.success());
    let comptime_err = String::from_utf8_lossy(&comptime_out.stderr);
    assert!(
        comptime_err.contains("comptime") || comptime_err.contains("static"),
        "unexpected comptime error: {comptime_err}"
    );

    let pure_access = Work::new(
        r#"
        trait Duplicate {}
        trait Retire { fn drop(*self); }
        typealias I = .i64'builtin;
        impl Duplicate for I {}
        static g: I = 7;
        pure fn read_g() -> I { g }
        fn run() -> I { read_g() }
        "#,
    );
    let pure_out = pure_access.build();
    assert!(!pure_out.status.success());
    let pure_err = String::from_utf8_lossy(&pure_out.stderr);
    assert!(
        pure_err.contains("pure") || pure_err.contains("purity"),
        "unexpected pure error: {pure_err}"
    );

    let comptime_type = Work::new(
        r#"
        trait Duplicate {}
        trait Retire { fn drop(*self); }
        typealias I = .i64'builtin;
        typealias T = .Type'builtin;
        static bad: T = I;
        fn run() -> I { 0 }
        "#,
    );
    let type_out = comptime_type.build();
    assert!(!type_out.status.success());
    let type_err = String::from_utf8_lossy(&type_out.stderr);
    assert!(
        type_err.contains("runtime value type"),
        "unexpected type error: {type_err}"
    );
}
