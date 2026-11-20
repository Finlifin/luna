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
            fn write(self, output: *W) -> !W.as(std.io.ByteWriter).Err void
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
        fn mark(counter: *u8) -> u8 {
            let previous = counter.*;
            assumes previous < 255;
            counter.* = previous + 1;
            previous
        }
        fn output() -> !std.io.IoErr void {
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
            println("界{{{}}}\0{}", .varargs = (Label { value: 42 }, false))!;
            var calls: u8 = 0;
            println("{}{}", mark(calls.ref), mark(calls.ref))!;
            assumes calls == 2;
            eprintln("stderr")
        }
        fn run() -> bool { output() match { _! => true, error _ => false } }
    "#,
    );
    for level in ["0", "3"] {
        success(&case.build(level));
        let output = case.run();
        success(&output);
        assert_eq!(output.stdout, "Hello, Flurry! 世界\nembedded\0NUL\n-128\n340282366920938463463374607431768211455\n-170141183460469231731687303715884105728\ntrue\n界{custom=42}\0false\n01\n(bool true)\n".as_bytes());
        assert_eq!(output.stderr, b"stderr\n");
    }
}

#[test]
fn files_round_trip_and_close_without_retiring_a_reused_descriptor() {
    let case = Case::new(
        r#"
        use std.fs.File;
        use std.os.linux;
        fn exercise() -> !std.io.IoErr bool {
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
                error std.io.IoErr.WriteZero => false,
                error std.io.IoErr.Os(code) => code != 5,
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
        fn exercise() -> !std.io.IoErr bool {
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
        enum SinkErr { Full(u8) }
        struct Sink { count: u64, limit: u64, bytes: u64, attempts: u64 }
        impl ByteWriter for Sink {
            assoc Err: Type = SinkErr;
            fn write_byte(*self, byte: u8) -> !SinkErr void {
                assumes self.attempts.* < 16;
                self.attempts.* = self.attempts.* + 1;
                if self.count.* >= self.limit.* { return error SinkErr.Full(byte); }
                assumes self.count.* < 8;
                assumes self.bytes.* <= 72057594037927935;
                self.bytes.* = self.bytes.* * 256 + byte'bitvec.as(u64);
                self.count.* = self.count.* + 1;
            }
        }
        fn render(value: T, sink: *W) -> !W.as(ByteWriter).Err void
        where T :- Display, W :- ByteWriter
        {
            value.write(sink)
        }
        fn run() -> bool {
            var sink = Sink { count: 0, limit: 2, bytes: 0, attempts: 0 };
            let value: u32 = 12345;
            let failed = render(value, sink.ref) match {
                _! => false,
                error SinkErr.Full(byte) => byte == 51,
            };
            let prefix = sink.count == 2 and sink.attempts == 3 and sink.bytes == 12594;
            sink.limit = 8;
            let negative: i8 = -7;
            let resumed = render(negative, sink.ref) match { _! => true, error _ => false };
            let text = render(true, sink.ref) match { _! => true, error _ => false };
            let full = render(false, sink.ref) match {
                _! => false,
                error SinkErr.Full(byte) => byte == 102,
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
        fn exercise() -> !std.io.IoErr bool {
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

#[test]
fn splice_construction_runs_per_use_after_staged_inputs_are_ready() {
    let case = Case::new(
        r#"
        struct Seed { byte: u8 }
        impl Splice for Seed {
            assoc RuntimeTarget: Type = u8;
            fn splice(comptime data: Seed) -> u8 {
                let written = std.fmt.print("convert;") match {
                    _! => true, error _ => false,
                };
                assumes written;
                data.byte
            }
        }
        const CONFIG: Seed = Seed { byte: 42 };
        fn explicit(comptime data: Seed) -> u8 { Seed.as(Splice).splice(data) }
        fn run() -> bool {
            let first: u8 = CONFIG;
            let second = explicit(Seed { byte: 42 });
            let flag = std.os.unix.getpid() > 0;
            let third: u8 = if flag { CONFIG } else { CONFIG };
            first == 42 and second == 42 and third == 42
        }
    "#,
    );
    for level in ["0", "3"] {
        success(&case.build(level));
        let output = case.run();
        success(&output);
        assert_eq!(output.stdout, b"convert;convert;convert;(bool true)\n");
    }
}

#[test]
fn quoted_constants_construct_runtime_values_at_each_executed_use() {
    let case = Case::new(
        r#"
        struct Seed { byte: u8 }
        impl Splice for Seed {
            assoc RuntimeTarget: Type = u8;
            fn splice(comptime data: Seed) -> u8 {
                let written = std.fmt.print("convert;") match {
                    _! => true, error _ => false,
                };
                assumes written;
                data.byte
            }
        }
        const CONFIG: Seed = Seed { byte: 42 };
        fn read(comptime data: Seed) -> u8 { data.byte }
        pure comptime fn quoted() -> <|- u8> { '(CONFIG) }
        pure comptime fn callback() -> <|- fn() -> u8> { '(|| CONFIG) }
        pure comptime fn input() -> <|- u8> { '(read(CONFIG)) }
        fn run() -> bool {
            const code = quoted();
            let direct: u8 = CONFIG;
            let first: u8 = ~<code>;
            let second: u8 = ~<code>;
            let selected: u8 = if std.os.unix.getpid() > 0 { ~<code> } else { ~<code> };
            let action: fn() -> u8 = ~<callback()>;
            let left = action();
            let right = action();
            let source = ~<input()>;
            direct == 42 and first == 42 and second == 42 and selected == 42
                and left == 42 and right == 42 and source == 42
        }
        "#,
    );
    for level in ["0", "3"] {
        success(&case.build(level));
        let output = case.run();
        success(&output);
        assert_eq!(
            output.stdout,
            b"convert;convert;convert;convert;convert;convert;(bool true)\n"
        );
    }
}

#[test]
fn source_field_observation_precedes_runtime_construction() {
    let case = Case::new(
        r#"
        struct Seed { byte: u8 }
        struct Packet { item: Seed }
        impl Splice for Seed {
            assoc RuntimeTarget: Type = u8;
            fn splice(comptime data: Seed) -> u8 {
                let written = std.fmt.print("convert;") match {
                    _! => true, error _ => false,
                };
                assumes written;
                data.byte
            }
        }
        const CONFIG: Packet = Packet { item: Seed { byte: 42 } };
        pure comptime fn produce() -> Packet { CONFIG }
        pure comptime fn generic() -> Packet
        where T
        {
            CONFIG
        }
        const FACTORY: comptime fn() -> Packet = produce;
        fn use_callback(comptime callback: comptime fn() -> Packet) -> u8 {
            callback().item
        }
        fn run() -> bool {
            let direct: u8 = CONFIG.item;
            let nested: u8 = CONFIG.item.byte;
            let produced: u8 = produce().item;
            let produced_nested: u8 = produce().item.byte;
            let generic_value: u8 = generic<bool>().item;
            let stored: u8 = FACTORY().item;
            let passed: u8 = use_callback(produce);
            const local: Packet = CONFIG;
            let local_field: u8 = local.item;
            let selected: u8 = if std.os.unix.getpid() > 0 {
                CONFIG.item
            } else {
                produce().item
            };
            let runtime = Packet { item: Seed { byte: 42 } };
            direct == 42 and nested == 42 and produced == 42
                and produced_nested == 42 and generic_value == 42
                and stored == 42 and passed == 42 and local_field == 42
                and selected == 42 and runtime.item.byte == 42
        }
        "#,
    );
    for level in ["0", "3"] {
        success(&case.build(level));
        let output = case.run();
        success(&output);
        assert_eq!(
            output.stdout,
            b"convert;convert;convert;convert;convert;convert;convert;(bool true)\n"
        );
    }
}

#[test]
fn source_field_observation_does_not_grant_whole_value_or_runtime_conversions() {
    let definitions = r#"
        struct Packet { byte: u8 }
        const CONFIG: Packet = Packet { byte: 42 };
        pure comptime fn produce() -> Packet { CONFIG }
        fn runtime() -> Packet { Packet { byte: 42 } }
    "#;
    for body in [
        "let value = CONFIG; value.byte == 42",
        "let value = produce(); value.byte == 42",
        "let value: u8 = runtime(); value == 42",
    ] {
        let case = Case::new(&format!("{definitions} fn run() -> bool {{ {body} }}"));
        let output = case.build("0");
        assert!(
            !output.status.success(),
            "source observation must not authorize this runtime delivery: {body}"
        );
    }
}
