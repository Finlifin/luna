//! Stage 2E-2a native execution tests for batched `ByteWriter.write_raw_bytes`
//! and bulk string/formatting sinks across `builtin.fmt` and `std` (`Vec<u8>`,
//! `File`, `BufWriter`, `Writer`, and custom `ByteWriter` implementations).
use std::fs;
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct NativeCase(PathBuf);

impl NativeCase {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "luna-stage2e-bulk-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("main.fl"), source).unwrap();
        Self(path)
    }

    fn build(&self, opt_level: &str) -> Output {
        ProcessCommand::new(env!("CARGO_BIN_EXE_luna"))
            .current_dir(&self.0)
            .args([
                "build",
                "main.fl",
                "--entry",
                "run",
                "--output",
                "program",
                "--opt-level",
                opt_level,
                "--fuel",
                "10000000",
                "--sysroot",
            ])
            .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library"))
            .output()
            .unwrap()
    }

    fn run(&self) -> Output {
        ProcessCommand::new(self.0.join("program"))
            .current_dir(&self.0)
            .output()
            .unwrap()
    }
}

impl Drop for NativeCase {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn native_batched_byte_writer_bulk_writes_and_buffered_io() {
    let source = r#"
        use std.prelude.*;
        use builtin.fmt.ByteWriter;
        use std.fmt.write_fmt;
        use std.fs.{File, read_to_string, remove_file};
        use std.io.{BufWriter, IoErr, Write, stdout};
        use std.mem.{AllocErr, Allocator, FixedBufferAllocator, Layout, SystemAllocator};
        use std.string.Utf8Err;

        struct CountingBulkWriter {
            buf: Vec<u8>,
            byte_calls: usize,
            raw_calls: usize,
            raw_bytes_total: usize,
            str_calls: usize,
        }

        impl CountingBulkWriter {
            fn new() -> Self {
                CountingBulkWriter {
                    buf: Vec<u8>.new(),
                    byte_calls: 0,
                    raw_calls: 0,
                    raw_bytes_total: 0,
                    str_calls: 0,
                }
            }
        }

        impl ByteWriter for CountingBulkWriter {
            assoc Err: Type = AllocErr;

            fn write_byte(*self, byte: u8) -> !AllocErr void {
                if self.byte_calls.* < 1000000 {
                    self.byte_calls.* = self.byte_calls.* + 1;
                }
                self.buf.push(byte)
            }

            unsafe fn write_raw_bytes(*self, src: usize, len: usize) -> !AllocErr void {
                if self.raw_calls.* < 1000000 {
                    self.raw_calls.* = self.raw_calls.* + 1;
                }
                if len <= 1000000 and self.raw_bytes_total.* <= 1000000 {
                    self.raw_bytes_total.* = self.raw_bytes_total.* + len;
                }
                self.buf.as(ByteWriter).write_raw_bytes(src, len)
            }

            fn write_str(*self, text: str) -> !AllocErr void {
                if self.str_calls.* < 1000000 {
                    self.str_calls.* = self.str_calls.* + 1;
                }
                self.buf.as(ByteWriter).write_str(text)
            }
        }

        struct FallbackOnlyWriter {
            buf: Vec<u8>,
            byte_calls: usize,
            limit: usize,
        }

        impl FallbackOnlyWriter {
            fn with_limit(limit: usize) -> Self {
                FallbackOnlyWriter {
                    buf: Vec<u8>.new(),
                    byte_calls: 0,
                    limit: limit,
                }
            }
        }

        impl ByteWriter for FallbackOnlyWriter {
            assoc Err: Type = AllocErr;

            fn write_byte(*self, byte: u8) -> !AllocErr void {
                if self.byte_calls.* >= self.limit.* {
                    return error AllocErr.OutOfMemory;
                }
                if self.byte_calls.* < 1000000 {
                    self.byte_calls.* = self.byte_calls.* + 1;
                }
                self.buf.push(byte)
            }
        }

        fn build_repeated_chunk() -> !AllocErr String {
            var s = String.new();
            var i: usize = 0;
            while i < 12 {
                s.ref.push_str("abcdefghij")!;
                i = i + 1;
            }
            s
        }

        fn exercise_bulk_io(fba: *FixedBufferAllocator) -> ![AllocErr, IoErr, Utf8Err] u32 {
            -- 1. Empty String must short-circuit without calling write_byte or write_raw_bytes.
            var counting = CountingBulkWriter.new();
            var empty = String.new();
            empty.ref.write_to(counting.ref)!;
            if counting.ref.byte_calls.* != 0 or counting.ref.raw_calls.* != 0 {
                return 1;
            }

            -- 2. String.write_to uses write_raw_bytes in a single bulk call.
            var payload = String.from_str("hello-bulk-string-payload")!;
            payload.ref.write_to(counting.ref)!;
            if counting.ref.byte_calls.* != 0
                or counting.ref.raw_calls.* != 1
                or counting.ref.raw_bytes_total.* != 25
                or counting.ref.str_calls.* != 0
            {
                return 2;
            }

            -- 3. Display for bool, Debug for void, Debug for str escapes, and Display for String
            --    all route multi-byte constants via write_str and String via write_raw_bytes.
            let payload_for_fmt = String.from_str("hello-bulk-string-payload")!;
            let unit_val: void = ();
            write_fmt(
                counting.ref,
                "|{}:{:?}:{:?}:{:?}|{}",
                true,
                false,
                unit_val,
                "a\nb\\c\"d",
                payload_for_fmt,
            )!;
            if counting.ref.raw_calls.* != 2 {
                return 3;
            }
            if counting.ref.raw_bytes_total.* != 50 {
                return 4;
            }
            -- 6 literal segments (including trailing empty segment) + true, false, (), \n, \\, \"
            if counting.ref.str_calls.* != 12 {
                return 5;
            }
            -- Bool ("true"/"false") and void ("()") must not add write_byte calls;
            -- only the 2 enclosing quotes and printable chars of Debug(str) use write_byte.
            -- In `"a\nb\\c\"d"`, '"'x2 + 'a','b','c','d' = 6 single bytes.
            if counting.ref.byte_calls.* != 6 {
                return 6;
            }
            let counting_text = String.from_utf8(counting.buf)!;
            if not counting_text.ref.equals_str(
                "hello-bulk-string-payload|true:false:():\"a\\nb\\\\c\\\"d\"|hello-bulk-string-payload",
            ) {
                return 7;
            }

            -- 4. Default ByteWriter.write_raw_bytes fallback + fail-fast without rollback.
            var fallback = FallbackOnlyWriter.with_limit(100);
            empty.ref.write_to(fallback.ref)!;
            if fallback.ref.byte_calls.* != 0 {
                return 8;
            }
            payload.ref.write_to(fallback.ref)!;
            if fallback.ref.byte_calls.* != 25 {
                return 9;
            }
            let fallback_text = String.from_utf8(fallback.buf)!;
            if not fallback_text.ref.equals_str("hello-bulk-string-payload") {
                return 10;
            }

            var failing = FallbackOnlyWriter.with_limit(3);
            let stopped = payload.ref.write_to(failing.ref) match {
                _! => false,
                error AllocErr.OutOfMemory => true,
                error _ => false,
            };
            if not stopped or failing.ref.byte_calls.* != 3 or failing.ref.buf.len() != 3 {
                return 11;
            }
            let partial_text = String.from_utf8(failing.buf)!;
            if not partial_text.ref.equals_str("hel") {
                return 12;
            }

            -- 5. Direct Vec<u8> ByteWriter bulk write with custom FixedBufferAllocator + zero leaks.
            var long_chunk = build_repeated_chunk()!;
            var vec_sink = Vec<u8, .A = FixedBufferAllocator, .allocator = fba>.new();
            long_chunk.ref.write_to(vec_sink.ref)!;
            empty.ref.write_to(vec_sink.ref)!;
            write_fmt(vec_sink.ref, ":{}", true)!;
            let vec_str = String<
                .A = FixedBufferAllocator,
                .allocator = fba,
            >.from_utf8(vec_sink)!;
            if vec_str.ref.len() != 125 {
                return 13;
            }

            -- 6. Zero-length write_raw_bytes on stdout Writer exercises Writer.write_raw_bytes safely.
            var out = stdout();
            unsafe {
                out.ref.as(ByteWriter).write_raw_bytes(0, 0)!;
            }

            -- 7. File and BufWriter<File> bulk ByteWriter paths (> capacity bypass + buffered path).
            var direct_file = File.create<"stage2e_bulk_direct.txt">()!;
            long_chunk.ref.write_to(direct_file.ref)!;
            empty.ref.write_to(direct_file.ref)!;
            direct_file.ref.close()!;

            var direct_back = read_to_string<
                .A = FixedBufferAllocator,
                .allocator = fba,
            >("stage2e_bulk_direct.txt")!;
            if not direct_back.ref.equals_str("abcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghij") {
                return 14;
            }

            do {
                var buf_file = File.create<"stage2e_bulk_buffered.txt">()!;
                var bw = BufWriter<
                    File,
                    .A = FixedBufferAllocator,
                    .allocator = fba,
                >.with_capacity(32, buf_file)!;
                let short_tag = String.from_str("hdr:")!;
                short_tag.ref.write_to(bw.ref)!;
                if bw.ref.buffered_len() != 4 {
                    return 15;
                }
                long_chunk.ref.write_to(bw.ref)!;
                if bw.ref.buffered_len() != 0 {
                    return 16;
                }
                write_fmt(bw.ref, ":{}", false)!;
                bw.ref.flush()!;
            }

            var buf_back = read_to_string<
                .A = FixedBufferAllocator,
                .allocator = fba,
            >("stage2e_bulk_buffered.txt")!;
            if buf_back.ref.len() != 130 or not buf_back.ref.equals_str(
                "hdr:abcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghij:false",
            ) {
                return 17;
            }
            0
        }

        fn run() -> u32 {
            let layout = Layout.new(32768, 16) match {
                found? => found,
                null => return 90,
            };
            let backing = SystemAllocator.GLOBAL.ref.as(Allocator).allocate(layout) match {
                block! => block,
                error _ => return 91,
            };
            var fba = FixedBufferAllocator.from_raw_block(backing);
            let code = exercise_bulk_io(fba.ref) match {
                result! => result,
                error _ => 92,
            };
            let _ = remove_file("stage2e_bulk_direct.txt");
            let _ = remove_file("stage2e_bulk_buffered.txt");
            let clean = fba.ref.live_allocations() == 0;
            unsafe {
                SystemAllocator.GLOBAL.ref.as(Allocator).deallocate(backing);
            }
            if code != 0 {
                return code;
            }
            if not clean {
                return 93;
            }
            0
        }
    "#;

    for opt_level in ["0", "3"] {
        let case = NativeCase::new(source);
        assert_success(&case.build(opt_level));
        let output = case.run();
        assert_success(&output);
        assert_eq!(
            output.stdout, b"(unsigned-integer 0)\n",
            "unexpected stdout at -O{opt_level}"
        );
    }
}
