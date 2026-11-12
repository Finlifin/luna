//! Real sysroot, resource verification, LLVM linking and observable host I/O.
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};
struct Case(PathBuf);
impl Case {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "luna-std-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("main.fl"), source).unwrap();
        Self(path)
    }
    fn build(&self, level: &str) -> Output {
        self.build_with_options(level, &[])
    }
    fn build_with_options(&self, level: &str, options: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_luna"))
            .current_dir(&self.0)
            .args([
                "build",
                "main.fl",
                "--entry",
                "run",
                "--output",
                "program",
                "--opt-level",
                level,
                "--fuel",
                "10000000",
                "--sysroot",
            ])
            .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library"))
            .args(options)
            .output()
            .unwrap()
    }
    fn run(&self) -> Output {
        Command::new(self.0.join("program"))
            .current_dir(&self.0)
            .output()
            .unwrap()
    }
}
impl Drop for Case {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn success(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn println_and_display_preserve_bytes_and_external_effects() {
    let case = Case::new(
        r#"
        use std.prelude.*;
        struct Label { value: u32 }
        impl Display for Label {
            fn write(self, output: *W) -> !W.as(std.io.ByteWriter).Error void
            where W :- std.io.ByteWriter
            {
                output.write_byte(99)!;
                output.write_byte(117)!;
                output.write_byte(115)!;
                output.write_byte(116)!;
                output.write_byte(111)!;
                output.write_byte(109)!;
                output.write_byte(61)!;
                self.value.write(output)
            }
        }
        fn output() -> !std.io.Error void {
            println("Hello, Flurry! 世界")!;
            print("embedded\0NUL")!;
            println("")!;
            let small: i8 = -128;
            let large: u128 = 340282366920938463463374607431768211455;
            let least: i128 = -170141183460469231731687303715884105728;
            println_value(small)!;
            println_value(large)!;
            println_value(least)!;
            println_value(true)!;
            println_value(Label { value: 42 })!;
            eprintln("stderr")
        }
        fn run() -> bool { output() match { _! => true, error _ => false } }
    "#,
    );
    for level in ["0", "3"] {
        success(&case.build(level));
        let output = case.run();
        success(&output);
        assert_eq!(output.stdout, "Hello, Flurry! 世界\nembedded\0NUL\n-128\n340282366920938463463374607431768211455\n-170141183460469231731687303715884105728\ntrue\ncustom=42\n(bool true)\n".as_bytes());
        assert_eq!(output.stderr, b"stderr\n");
    }
}

#[test]
fn files_round_trip_and_close_without_retiring_a_reused_descriptor() {
    let case = Case::new(
        r#"
        use std.fs.File;
        use std.os.linux;
        fn exercise() -> !std.io.Error bool {
            var file = File.create<"bytes.dat">()!;
            file.write_all<"A\0Z">()!;
            file.close()!;
            let descriptor = linux.open<"bytes.dat">(0, 0);
            assumes descriptor >= 0 and descriptor <= 2147483647;
            let fd = descriptor'bitvec[0..32].as(i32);
            do { let retired = file; };
            let first = linux.read_byte(fd);
            let second = linux.read_byte(fd);
            let third = linux.read_byte(fd);
            let end = linux.read_byte(fd);
            let closed = linux.close(fd);
            var reader = File.open<"bytes.dat">()!;
            let from_file = reader.read_byte()!;
            reader.close()!;
            std.fs.remove_file("bytes.dat")!;
            first == 65 and second == 0 and third == 90 and end == 256
                and closed == 0 and (from_file matches 65?)
        }
        fn run() -> bool { exercise() match { value! => value, error _ => false } }
    "#,
    );
    success(&case.build("0"));
    let result = case.run();
    success(&result);
    assert_eq!(result.stdout, b"(bool true)\n");
    assert!(!case.0.join("bytes.dat").exists());
}

#[test]
fn descriptor_errors_and_path_rejection_remain_observable() {
    let case = Case::new(
        r#"
        use std.os.linux;
        fn run() -> bool {
            let bad_write = linux.write_byte(-1, 65);
            let bad_read = linux.read_byte(-1);
            let bad_close = linux.close(-1);
            let bad_offset = linux.write<"unchanged">(1, 99);
            let missing = linux.open<"not-present">(0, 0);
            let nul = linux.open<"not-present\0suffix">(0, 0);
            bad_write == -9 and bad_read == -9 and bad_close == -9
                and bad_offset == -22 and missing == -2 and nul == -22
                and std.os.unix.getpid() > 0
        }
    "#,
    );
    success(&case.build("3"));
    let output = case.run();
    success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");
}

#[test]
fn allocation_layout_checks_alignment_padding_and_overflow() {
    let case = Case::new(
        r#"
        use std.mem.Layout;
        fn run() -> bool {
            let invalid = Layout.new(8, 3).is_none();
            let zero = Layout.new(8, 0).is_none();
            let overflow = Layout.new(18446744073709551615, 8).is_none();
            let layout = Layout.new(3, 4) match { value? => value, null => return false };
            let repeated = layout.repeat(3) match { value? => value, null => return false };
            invalid and zero and overflow and repeated.byte_size() == 12
                and repeated.byte_alignment() == 4
                and layout.repeat(18446744073709551615).is_none()
        }
    "#,
    );
    success(&case.build("0"));
    let output = case.run();
    success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");
}

#[test]
fn platform_operations_cannot_be_declared_pure() {
    let case = Case::new("pure fn run() -> i64 { std.os.linux.getpid() }");
    let output = case.build("0");
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("P4") || error.contains("pure"), "{error}");
    assert!(!case.0.join("program").exists());
}

#[test]
fn writes_retry_interruption_and_short_transfers_but_propagate_failure() {
    let case = Case::new(
        r#"
        use std.prelude.*;
        fn run() -> bool {
            println("hello") match {
                _! => true,
                error std.io.Error.write_zero => false,
                error std.io.Error.os(code) => code != 5,
            }
        }
    "#,
    );
    success(&case.build("3"));
    let library = case.0.join("faults.so");
    let source = case.0.join("faults.c");
    fs::write(
        &source,
        r#"
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <stdlib.h>
#include <unistd.h>
ssize_t write(int fd, const void *bytes, size_t size) {
    static ssize_t (*actual)(int, const void *, size_t);
    static unsigned calls;
    if (!actual) actual = dlsym(RTLD_NEXT, "write");
    const char *mode = getenv("FLURRY_TEST_WRITE_MODE");
    if (!mode || fd != 1) return actual(fd, bytes, size);
    unsigned call = calls++;
    if (mode[0] == 's') {
        if (call == 0) { errno = EINTR; return -1; }
        return actual(fd, bytes, size > 2 ? 2 : size);
    }
    if (mode[0] == 'z') return 0;
    if (mode[0] == 'p' && call == 0) return actual(fd, bytes, size > 2 ? 2 : size);
    errno = EIO;
    return -1;
}
"#,
    )
    .unwrap();
    success(
        &Command::new("cc")
            .args(["-shared", "-fPIC", "-o"])
            .arg(&library)
            .arg(source)
            .arg("-ldl")
            .output()
            .unwrap(),
    );
    for (mode, expected) in [
        ("short", "hello\n(bool true)\n"),
        ("zero", "(bool false)\n"),
        ("error", "(bool false)\n"),
        ("partial", "he(bool false)\n"),
    ] {
        let output = Command::new(case.0.join("program"))
            .current_dir(&case.0)
            .env("LD_PRELOAD", &library)
            .env("FLURRY_TEST_WRITE_MODE", mode)
            .output()
            .unwrap();
        success(&output);
        assert_eq!(output.stdout, expected.as_bytes(), "mode {mode}");
    }
}

#[test]
fn file_scope_exit_closes_owned_descriptor() {
    let case = Case::new(
        r#"
        use std.fs.File;
        use std.os.linux;
        fn exercise() -> !std.io.Error bool {
            let raw = linux.open<"drop.dat">(65, 438);
            assumes raw >= 0 and raw <= 2147483647;
            let fd = raw'bitvec[0..32].as(i32);
            let first_close = linux.close(fd);
            do { let file = File.open<"drop.dat">()!; };
            let after_drop = linux.close(fd);
            std.fs.remove_file("drop.dat")!;
            first_close == 0 and after_drop == -9
        }
        fn run() -> bool { exercise() match { value! => value, error _ => false } }
    "#,
    );
    success(&case.build("0"));
    let output = case.run();
    success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");
}

#[test]
fn discard_result_keeps_only_program_output() {
    let case = Case::new(
        r#"
        fn run() {
            let succeeded = std.fmt.println("hello") match {
                _! => true,
                error _ => false,
            };
            assumes succeeded;
        }
    "#,
    );
    success(&case.build_with_options("3", &["--discard-result"]));
    let output = case.run();
    success(&output);
    assert_eq!(output.stdout, b"hello\n");
}

#[test]
fn builtin_display_borrows_stateful_writers_and_preserves_their_errors() {
    let case = Case::new(
        r#"
        use builtin.fmt.ByteWriter;
        enum SinkError { full(u8) }
        struct Sink { count: u64, limit: u64, bytes: u64, attempts: u64 }
        impl ByteWriter for Sink {
            assoc Error: Type = SinkError;
            fn write_byte(*self, byte: u8) -> !SinkError void {
                assumes self.attempts.* < 16;
                self.attempts.* = self.attempts.* + 1;
                if self.count.* >= self.limit.* { return error SinkError.full(byte); }
                assumes self.count.* < 8;
                assumes self.bytes.* <= 72057594037927935;
                let next_count = self.count.* + 1;
                let next_bytes = self.bytes.* * 256 + byte'bitvec.as(u64);
                self.bytes.* = next_bytes;
                self.count.* = next_count;
            }
        }
        fn render(value: T, sink: *W) -> !W.as(ByteWriter).Error void
        where T :- Display, W :- ByteWriter
        {
            value.write(sink)
        }
        fn run() -> bool {
            var sink = Sink { count: 0, limit: 2, bytes: 0, attempts: 0 };
            let value: u32 = 12345;
            let failed = render(value, sink.ref) match {
                _! => false,
                error SinkError.full(byte) => byte == 51,
            };
            let prefix = sink.count == 2 and sink.attempts == 3 and sink.bytes == 12594;
            sink.limit = 8;
            let negative: i8 = -7;
            let resumed = render(negative, sink.ref) match { _! => true, error _ => false };
            let text = render(true, sink.ref) match { _! => true, error _ => false };
            let full = render(false, sink.ref) match {
                _! => false,
                error SinkError.full(byte) => byte == 102,
            };
            failed and prefix and resumed and text and full and sink.count == 8
                and sink.attempts == 10 and sink.bytes == 3544945572894111077
        }
    "#,
    );
    for level in ["0", "3"] {
        success(&case.build(level));
        let output = case.run();
        success(&output);
        assert_eq!(output.stdout, b"(bool true)\n");
    }
}

#[test]
fn display_writes_through_a_borrowed_file_without_transferring_ownership() {
    let case = Case::new(
        r#"
        use std.fs.File;
        fn exercise() -> !std.io.Error bool {
            var file = File.create<"formatted.txt">()!;
            let value: u32 = 42;
            value.write(file.ref)!;
            true.write(file.ref)!;
            file.close()!;
            var reader = File.open<"formatted.txt">()!;
            let a = reader.read_byte()!;
            let b = reader.read_byte()!;
            let c = reader.read_byte()!;
            let d = reader.read_byte()!;
            let e = reader.read_byte()!;
            let f = reader.read_byte()!;
            let end = reader.read_byte()!;
            reader.close()!;
            std.fs.remove_file("formatted.txt")!;
            (a matches 52?) and (b matches 50?) and (c matches 116?)
                and (d matches 114?) and (e matches 117?) and (f matches 101?)
                and (end matches null)
        }
        fn run() -> bool { exercise() match { value! => value, error _ => false } }
    "#,
    );
    success(&case.build("3"));
    let output = case.run();
    success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");
    assert!(!case.0.join("formatted.txt").exists());
}
