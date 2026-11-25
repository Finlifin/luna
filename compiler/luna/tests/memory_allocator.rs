//! End-to-end Milestone C tests for checked integer conversions (TryFrom/TryInto &
//! ComptimeTryFrom/ComptimeTryInto), builtin.mem raw intrinsics, std.os.unix raw I/O,
//! SystemAllocator/RawBlock/Layout, and Box<T> affine lifecycle.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::Value;
use luna::cli::{self, Command};
use luna::frontend::{self, Artifact};
use std::fs;
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct NativeCase(PathBuf);

impl NativeCase {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "luna-mem-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("main.fl"), source).unwrap();
        Self(path)
    }

    fn build(&self) -> Output {
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
                "0",
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

fn run_with_target(source: &str, arch: &str, os: &str, bits: &str) -> Result<Value, String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "luna-mem-target-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&path, source).unwrap();
    let library = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let Command::Check(options) = cli::parse([
        "check".into(),
        path.clone().into_os_string(),
        "--sysroot".into(),
        library.into_os_string(),
        "--build-target".into(),
        arch.into(),
        os.into(),
        bits.into(),
    ])
    .unwrap() else {
        panic!("expected check command");
    };
    let compiled = frontend::compile(&options);
    let _ = fs::remove_file(&path);
    match compiled.map_err(|err| err.to_string())? {
        Artifact::Nir(package) => {
            let graph = luna::instantiation::instantiate(&package, "run", 1_000_000)
                .map_err(|err| err.to_string())?;
            let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default())
                .map_err(|err| err.to_string())?;
            vm.invoke_instance(&graph.roots[0], vec![])
                .map_err(|err| err.to_string())
        }
        _ => panic!("expected NIR artifact"),
    }
}

#[test]
fn checked_integer_conversions_work_at_runtime_and_comptime_across_targets() {
    let runtime_source = r#"
        use std.prelude.*;
        use builtin.{TryFrom, TryInto};

        fn check_conversions() -> bool {
            let x64: u64 = 42;
            let via_from = usize.as(TryFrom<u64>).try_from(x64) match {
                v! => v == 42,
                error _ => false,
            };
            let via_into = x64.as(TryInto<usize>).try_into() match {
                v! => v == 42,
                error _ => false,
            };
            let too_big_u128: u128 = 18446744073709551616;
            let u128_overflow = too_big_u128.as(TryInto<usize>).try_into() match {
                _! => false,
                error err => err matches .Overflow,
            };
            let u128_fit: u128 = 9001;
            let u128_ok = u128_fit.as(TryInto<usize>).try_into() match {
                v! => v == 9001,
                error _ => false,
            };

            let neg: i64 = -1;
            let neg_to_u32 = neg.as(TryInto<u32>).try_into() match {
                _! => false,
                error err => err matches .Underflow,
            };
            let neg_i8_min: i32 = -128;
            let i8_min_ok = neg_i8_min.as(TryInto<i8>).try_into() match {
                v! => v == -128,
                error _ => false,
            };
            let neg_i8_under: i32 = -129;
            let i8_under = neg_i8_under.as(TryInto<i8>).try_into() match {
                _! => false,
                error err => err matches .Underflow,
            };

            let u16_255: u16 = 255;
            let u8_max_ok = u16_255.as(TryInto<u8>).try_into() match {
                v! => v == 255,
                error _ => false,
            };
            let u16_256: u16 = 256;
            let u8_over = u16_256.as(TryInto<u8>).try_into() match {
                _! => false,
                error err => err matches .Overflow,
            };

            let u8_127: u8 = 127;
            let u8_to_i8_ok = u8_127.as(TryInto<i8>).try_into() match {
                v! => v == 127,
                error _ => false,
            };
            let u8_128: u8 = 128;
            let u8_to_i8_over = u8_128.as(TryInto<i8>).try_into() match {
                _! => false,
                error err => err matches .Overflow,
            };

            let max_u = builtin.mem.usize_max();
            let usize_to_isize_over = max_u.as(TryInto<isize>).try_into() match {
                _! => false,
                error err => err matches .Overflow,
            };
            let small_u: usize = 77;
            let usize_to_isize_ok = small_u.as(TryInto<isize>).try_into() match {
                v! => v == 77,
                error _ => false,
            };
            let same_u32: u32 = 99;
            let same_ok = same_u32.as(TryInto<u32>).try_into() match {
                v! => v == 99,
                error _ => false,
            };

            via_from and via_into and u128_overflow and u128_ok
                and neg_to_u32 and i8_min_ok and i8_under
                and u8_max_ok and u8_over and u8_to_i8_ok and u8_to_i8_over
                and usize_to_isize_over and usize_to_isize_ok and same_ok
        }

        fn run() -> bool {
            check_conversions()
        }
    "#;
    let case = NativeCase::new(runtime_source);
    let build = case.build();
    assert_success(&build);
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");

    let target_probe = r#"
        use meta.{ComptimeTryFrom, ComptimeTryInto};

        comptime fn check_fit_32_ok() -> bool {
            let v: u64 = 4294967295;
            v.as(ComptimeTryInto<usize>).try_into() match {
                n! => n == 4294967295,
                error _ => false,
            }
        }
        comptime fn make_over_32() -> !builtin.ConversionErr usize {
            let v: u64 = 4294967296;
            v.as(ComptimeTryInto<usize>).try_into()
        }
        comptime fn check_neg_rejected() -> bool {
            let v: i16 = -1;
            u8.as(ComptimeTryFrom<i16>).try_from(v) match {
                _! => false,
                error err => err matches .Underflow,
            }
        }
        comptime fn check_narrowed_signed() -> bool {
            let v: i32 = -128;
            v.as(ComptimeTryInto<i8>).try_into() match {
                n! => n == -128,
                error _ => false,
            }
        }

        comptime fn check_over_32_is_none() -> bool {
            make_over_32() match {
                _! => false,
                error err => err matches .Overflow,
            }
        }

        const fit_32_ok: bool = check_fit_32_ok();
        const over_32_is_none: bool = check_over_32_is_none();
        const neg_rejected: bool = check_neg_rejected();
        const narrowed_signed: bool = check_narrowed_signed();

        fn run() -> bool {
            fit_32_ok and over_32_is_none and neg_rejected and narrowed_signed
        }
    "#;
    assert_eq!(
        run_with_target(target_probe, "x86", "linux", "32").unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        run_with_target(target_probe, "x86_64", "linux", "64").unwrap(),
        Value::Bool(false)
    );
}

#[test]
fn builtin_mem_raw_ops_and_checked_usize_work_and_enforce_safety() {
    let source = r#"
        use std.prelude.*;
        use std.os.linux;

        struct Pair {
            tag: u8,
            value: i64,
        }

        fn addr_offset(base: usize, delta: usize) -> usize {
            builtin.mem.checked_add_usize(base, delta) match {
                next? => next,
                null => 0,
            }
        }

        fn test_raw_ops() -> bool {
            let max = builtin.mem.usize_max();
            let add_over = builtin.mem.checked_add_usize(max, 1).is_none();
            let add_ok = builtin.mem.checked_add_usize(10, 20) match {
                v? => v == 30,
                null => false,
            };
            let sub_under = builtin.mem.checked_sub_usize(5, 6).is_none();
            let sub_ok = builtin.mem.checked_sub_usize(10, 3) match {
                v? => v == 7,
                null => false,
            };
            let mul_over = builtin.mem.checked_mul_usize(max, 2).is_none();
            let mul_ok = builtin.mem.checked_mul_usize(6, 7) match {
                v? => v == 42,
                null => false,
            };
            let align_zero = builtin.mem.align_up_usize(0, 8) match {
                v? => v == 0,
                null => false,
            };
            let align_ok = builtin.mem.align_up_usize(9, 8) match {
                v? => v == 16,
                null => false,
            };
            let align_bad = builtin.mem.align_up_usize(9, 3).is_none();
            let align_over = builtin.mem.align_up_usize(max, 8).is_none();

            if not (add_over and add_ok and sub_under and sub_ok and mul_over and mul_ok
                and align_zero and align_ok and align_bad and align_over)
            {
                return false;
            }

            let buf_a = unsafe { linux.alloc(32, 8) };
            let buf_b = unsafe { linux.alloc(32, 8) };
            if buf_a == 0 or buf_b == 0 {
                return false;
            }

            let ok = unsafe {
                builtin.mem.raw_write<u64>(buf_a, 123456789);
                let got_u64 = builtin.mem.raw_read<u64>(buf_a);
                builtin.mem.raw_write<void>(1, ());
                builtin.mem.raw_read<void>(1);

                let pair_addr = addr_offset(buf_a, 16);
                builtin.mem.raw_write<Pair>(pair_addr, Pair { tag: 7, value: -555 });
                let got_pair = builtin.mem.raw_read<Pair>(pair_addr);

                builtin.mem.set_bytes(buf_a, 10, 8);
                builtin.mem.set_bytes(buf_b, 10, 8);
                let eq_zero = builtin.mem.compare_bytes(buf_a, buf_b, 8) == 0;
                builtin.mem.raw_write<u8>(addr_offset(buf_b, 7), 11);
                let lt_neg = builtin.mem.compare_bytes(buf_a, buf_b, 8) < 0;
                let gt_pos = builtin.mem.compare_bytes(buf_b, buf_a, 8) > 0;

                builtin.mem.copy_bytes_nonoverlapping(buf_b, buf_a, 8);
                let after_memcpy = builtin.mem.compare_bytes(buf_a, buf_b, 8) == 0;

                -- Overlapping forward and backward copy inside buf_a: [1, 2, 3, 4, 0, 0, ...]
                builtin.mem.set_bytes(buf_a, 0, 16);
                builtin.mem.raw_write<u8>(addr_offset(buf_a, 0), 1);
                builtin.mem.raw_write<u8>(addr_offset(buf_a, 1), 2);
                builtin.mem.raw_write<u8>(addr_offset(buf_a, 2), 3);
                builtin.mem.raw_write<u8>(addr_offset(buf_a, 3), 4);
                -- Right shift overlap: copy 4 bytes from +0 to +2 -> [1, 2, 1, 2, 3, 4]
                builtin.mem.copy_bytes(addr_offset(buf_a, 2), buf_a, 4);
                let right_ok = builtin.mem.raw_read<u8>(addr_offset(buf_a, 0)) == 1
                    and builtin.mem.raw_read<u8>(addr_offset(buf_a, 2)) == 1
                    and builtin.mem.raw_read<u8>(addr_offset(buf_a, 3)) == 2
                    and builtin.mem.raw_read<u8>(addr_offset(buf_a, 4)) == 3
                    and builtin.mem.raw_read<u8>(addr_offset(buf_a, 5)) == 4;
                -- Left shift overlap: copy 4 bytes from +2 to +1 -> [1, 1, 2, 3, 4, 4]
                builtin.mem.copy_bytes(addr_offset(buf_a, 1), addr_offset(buf_a, 2), 4);
                let left_ok = builtin.mem.raw_read<u8>(addr_offset(buf_a, 1)) == 1
                    and builtin.mem.raw_read<u8>(addr_offset(buf_a, 2)) == 2
                    and builtin.mem.raw_read<u8>(addr_offset(buf_a, 3)) == 3
                    and builtin.mem.raw_read<u8>(addr_offset(buf_a, 4)) == 4;

                -- Zero-length guards never dereference null addresses.
                builtin.mem.set_bytes(0, 255, 0);
                builtin.mem.copy_bytes(0, 0, 0);
                builtin.mem.copy_bytes_nonoverlapping(0, 0, 0);
                let cmp_empty = builtin.mem.compare_bytes(0, 0, 0) == 0;

                got_u64 == 123456789
                    and got_pair.tag == 7
                    and got_pair.value == -555
                    and eq_zero and lt_neg and gt_pos and after_memcpy
                    and right_ok and left_ok and cmp_empty
            };

            unsafe {
                linux.dealloc(buf_a, 32, 8);
                linux.dealloc(buf_b, 32, 8);
            }
            ok
        }

        fn run() -> bool {
            test_raw_ops()
        }
    "#;
    let case = NativeCase::new(source);
    assert_success(&case.build());
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");

    let safe_read = r#"
        fn run() -> u64 {
            builtin.mem.raw_read<u64>(1)
        }
    "#;
    assert!(support::compile(safe_read).is_err());

    let safe_set = r#"
        fn run() -> bool {
            builtin.mem.set_bytes(0, 0, 0);
            true
        }
    "#;
    assert!(support::compile(safe_set).is_err());

    let comptime_mem = r#"
        const BAD: u64 = unsafe { builtin.mem.raw_read<u64>(1) };
        fn run() -> u64 { BAD }
    "#;
    assert!(support::compile(comptime_mem).is_err());

    let comptime_set = r#"
        comptime fn bad() -> bool {
            unsafe {
                builtin.mem.set_bytes(0, 0, 0);
            }
            true
        }
        fn run() -> bool { bad() }
    "#;
    assert!(support::compile(comptime_set).is_err());
}

#[test]
fn system_allocator_raw_block_and_unix_raw_io_work_end_to_end() {
    let tmp_path = std::env::temp_dir().join(format!("luna-raw-io-{}.bin", std::process::id()));
    let _ = fs::remove_file(&tmp_path);
    let tmp_str = tmp_path
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");

    let source = format!(
        r#"
        use std.prelude.*;
        use std.mem.{{AllocErr, Allocator, Layout, RawBlock, ReallocErr, SystemAllocator}};
        use std.os.{{linux, unix}};

        fn addr_offset(base: usize, delta: usize) -> usize {{
            builtin.mem.checked_add_usize(base, delta) match {{
                next? => next,
                null => 0,
            }}
        }}

        fn test_allocator_and_io() -> bool {{
            let alloc = SystemAllocator {{}};
            let layout8 = Layout.new(8, 8) match {{ value? => value, null => return false }};
            let layout16 = Layout.new(16, 8) match {{ value? => value, null => return false }};
            let layout4 = Layout.new(4, 8) match {{ value? => value, null => return false }};

            let zero_block = alloc.as(Allocator).allocate_zeroed(layout8) match {{
                value! => value,
                error _ => return false,
            }};
            let zeroed_ok = unsafe {{ builtin.mem.raw_read<u64>(zero_block.address()) == 0 }};
            unsafe {{
                builtin.mem.raw_write<u32>(zero_block.address(), 112233);
                builtin.mem.raw_write<u32>(addr_offset(zero_block.address(), 4), 445566);
            }}

            let grown = unsafe {{
                alloc.as(Allocator).reallocate(zero_block, layout16)
            }} match {{
                value! => value,
                error _ => {{
                    unsafe {{ alloc.as(Allocator).deallocate(zero_block); }}
                    return false;
                }},
            }};
            let grow_ok = grown.byte_size() == 16 and unsafe {{
                builtin.mem.raw_read<u32>(grown.address()) == 112233
                    and builtin.mem.raw_read<u32>(addr_offset(grown.address(), 4)) == 445566
            }};

            -- Huge layout request returns error while keeping `grown` untouched and intact.
            let huge = Layout.new(1152921504606846976, 8) match {{
                value? => value,
                null => {{
                    unsafe {{ alloc.as(Allocator).deallocate(grown); }}
                    return false;
                }},
            }};
            let failed_keeps_old = unsafe {{
                alloc.as(Allocator).reallocate(grown, huge)
            }} match {{
                value! => {{
                    unsafe {{ alloc.as(Allocator).deallocate(value); }}
                    return false;
                }},
                error err => err match {{
                    .OutOfMemory => true,
                    .InvalidLayout => false,
                }},
            }};

            let shrunk = unsafe {{
                alloc.as(Allocator).reallocate(grown, layout4)
            }} match {{
                value! => value,
                error _ => {{
                    unsafe {{ alloc.as(Allocator).deallocate(grown); }}
                    return false;
                }},
            }};
            let shrink_ok = shrunk.byte_size() == 4
                and unsafe {{ builtin.mem.raw_read<u32>(shrunk.address()) == 112233 }};
            unsafe {{
                alloc.as(Allocator).deallocate(shrunk);
            }}

            -- Zero-sized layout works without OS allocation.
            let zst_layout = Layout.of<void>() match {{ value? => value, null => return false }};
            let zst_block = alloc.as(Allocator).allocate(zst_layout) match {{
                value! => value,
                error _ => return false,
            }};
            let zst_ok = zst_block.is_empty() and zst_block.address() == 1;
            let zst_grown = unsafe {{
                alloc.as(Allocator).reallocate(zst_block, layout4)
            }} match {{
                value! => value,
                error _ => return false,
            }};
            let zst_back = unsafe {{
                alloc.as(Allocator).reallocate(zst_grown, zst_layout)
            }} match {{
                value! => value,
                error _ => return false,
            }};
            let zst_round_ok = zst_back.is_empty();
            unsafe {{
                alloc.as(Allocator).deallocate(zst_back);
            }}

            -- std.os.unix raw fd I/O round-trip over raw buffers.
            let write_buf = alloc.as(Allocator).allocate(layout8) match {{
                value! => value,
                error _ => return false,
            }};
            let read_buf = alloc.as(Allocator).allocate_zeroed(layout8) match {{
                value! => value,
                error _ => {{
                    unsafe {{ alloc.as(Allocator).deallocate(write_buf); }}
                    return false;
                }},
            }};
            unsafe {{
                builtin.mem.raw_write<u64>(write_buf.address(), 9876543210123456);
            }}
            const io_path = "{tmp_str}";
            const create_flags: i32 = linux.read_write + linux.create + linux.truncate + linux.close_on_exec;
            const read_flags: i32 = linux.read_only + linux.close_on_exec;
            let raw_fd = linux.open<io_path>(create_flags, 384);
            if raw_fd < 0 {{
                unsafe {{
                    alloc.as(Allocator).deallocate(write_buf);
                    alloc.as(Allocator).deallocate(read_buf);
                }}
                return false;
            }}
            let fd = raw_fd'bitvec[0..32].as(i32);
            let wrote = unsafe {{ unix.write_raw(fd, write_buf.address(), 8) }} match {{
                value! => value,
                error _ => 0,
            }};
            let _closed1 = unix.close(fd);
            let open_read = linux.open<io_path>(read_flags, 0);
            var read_count: u64 = 0;
            if open_read >= 0 {{
                let rfd = open_read'bitvec[0..32].as(i32);
                read_count = unsafe {{ unix.read_raw(rfd, read_buf.address(), 8) }} match {{
                    value! => value,
                    error _ => 0,
                }};
                let _closed2 = unix.close(rfd);
            }}
            let _unlinked = linux.unlink<io_path>();
            let io_match = wrote == 8 and read_count == 8 and unsafe {{
                builtin.mem.compare_bytes(write_buf.address(), read_buf.address(), 8) == 0
                    and builtin.mem.raw_read<u64>(read_buf.address()) == 9876543210123456
            }};
            unsafe {{
                alloc.as(Allocator).deallocate(write_buf);
                alloc.as(Allocator).deallocate(read_buf);
            }}

            zeroed_ok and grow_ok and failed_keeps_old and shrink_ok
                and zst_ok and zst_round_ok and io_match
        }}

        fn run() -> bool {{
            test_allocator_and_io()
        }}
        "#
    );
    let case = NativeCase::new(&source);
    assert_success(&case.build());
    let output = case.run();
    assert_success(&output);
    let _ = fs::remove_file(&tmp_path);
    assert_eq!(output.stdout, b"(bool true)\n");
}

#[test]
fn box_lifecycle_handles_values_drops_zsts_and_nesting() {
    let source = r#"
        use std.prelude.*;
        use std.mem.{Allocator, Box, Layout, SystemAllocator};

        struct Pair {
            a: u8,
            b: i64,
            c: bool,
        }

        struct TrackDrop {
            id: u64,
            counter_addr: usize,
        }

        impl Drop for TrackDrop {
            fn drop(*self) {
                unsafe {
                    let current = builtin.mem.raw_read<u64>(self.counter_addr.*);
                    assumes current < 18446744073709551615;
                    builtin.mem.raw_write<u64>(self.counter_addr.*, current + 1);
                }
            }
        }

        struct ZeroDrop {}

        impl Drop for ZeroDrop {
            fn drop(*self) {
                let printed = std.fmt.print("Z") match {
                    _! => true,
                    error _ => false,
                };
            }
        }

        fn read_count(addr: usize) -> u64 {
            unsafe { builtin.mem.raw_read<u64>(addr) }
        }

        fn reset_count(addr: usize) {
            unsafe {
                builtin.mem.raw_write<u64>(addr, 0);
            }
        }

        fn test_box() -> bool {
            let alloc = SystemAllocator {};
            let counter_layout = Layout.of<u64>() match {
                value? => value,
                null => return false,
            };
            let counter_block = alloc.as(Allocator).allocate_zeroed(counter_layout) match {
                value! => value,
                error _ => return false,
            };
            let counter_addr = counter_block.address();

            -- 1. Primitive Box<u64> with set, replace, get, into_inner.
            let initial_u64: u64 = 99;
            var b = Box<u64>.new(initial_u64) match {
                value! => value,
                error _ => {
                    unsafe { alloc.as(Allocator).deallocate(counter_block); }
                    return false;
                },
            };
            if b.ref.get() != 99 {
                unsafe { alloc.as(Allocator).deallocate(counter_block); }
                return false;
            }
            b.ref.set(123);
            if b.ref.get() != 123 {
                unsafe { alloc.as(Allocator).deallocate(counter_block); }
                return false;
            }
            let old_val = b.ref.replace(456);
            let prim_ok = old_val == 123 and b.into_inner() == 456;

            -- 2. Multi-field struct Box<Pair>.
            let pair_box = Box<Pair>.new(Pair { a: 7, b: -88, c: true }) match {
                value! => value,
                error _ => {
                    unsafe { alloc.as(Allocator).deallocate(counter_block); }
                    return false;
                },
            };
            let pair = pair_box.into_inner();
            let pair_ok = pair.a == 7 and pair.b == -88 and pair.c;

            -- 3. Drop on scope exit drops inner TrackDrop once.
            reset_count(counter_addr);
            {
                let held = Box<TrackDrop>.new(TrackDrop { id: 1, counter_addr: counter_addr }) match {
                    value! => value,
                    error _ => {
                        unsafe { alloc.as(Allocator).deallocate(counter_block); }
                        return false;
                    },
                };
            }
            let scope_drop_ok = read_count(counter_addr) == 1;

            -- 4. into_inner disarms Box drop; inner drops only once when caller retires it.
            reset_count(counter_addr);
            var moved_id: u64 = 0;
            {
                let held = Box<TrackDrop>.new(TrackDrop { id: 42, counter_addr: counter_addr }) match {
                    value! => value,
                    error _ => {
                        unsafe { alloc.as(Allocator).deallocate(counter_block); }
                        return false;
                    },
                };
                let inner = held.into_inner();
                if read_count(counter_addr) != 0 {
                    unsafe { alloc.as(Allocator).deallocate(counter_block); }
                    return false;
                }
                moved_id = inner.id;
            }
            let into_inner_drop_ok = moved_id == 42 and read_count(counter_addr) == 1;

            -- 5. set drops old inner immediately; replace returns old without dropping it.
            reset_count(counter_addr);
            {
                var held = Box<TrackDrop>.new(TrackDrop { id: 10, counter_addr: counter_addr }) match {
                    value! => value,
                    error _ => {
                        unsafe { alloc.as(Allocator).deallocate(counter_block); }
                        return false;
                    },
                };
                held.ref.set(TrackDrop { id: 20, counter_addr: counter_addr });
                if read_count(counter_addr) != 1 {
                    unsafe { alloc.as(Allocator).deallocate(counter_block); }
                    return false;
                }
                let prev = held.ref.replace(TrackDrop { id: 30, counter_addr: counter_addr });
                if read_count(counter_addr) != 1 or prev.id != 20 {
                    unsafe { alloc.as(Allocator).deallocate(counter_block); }
                    return false;
                }
            }
            let replace_set_drop_ok = read_count(counter_addr) == 3;

            -- 6. Nested Box<Box<TrackDrop>> and Box<Box<u64>>.
            reset_count(counter_addr);
            {
                let inner = Box<TrackDrop>.new(TrackDrop { id: 77, counter_addr: counter_addr }) match {
                    value! => value,
                    error _ => {
                        unsafe { alloc.as(Allocator).deallocate(counter_block); }
                        return false;
                    },
                };
                let outer = Box<Box<TrackDrop>>.new(inner) match {
                    value! => value,
                    error _ => {
                        unsafe { alloc.as(Allocator).deallocate(counter_block); }
                        return false;
                    },
                };
            }
            let nested_drop_ok = read_count(counter_addr) == 1;

            let inner_val: u64 = 808;
            let inner_u = Box<u64>.new(inner_val) match {
                value! => value,
                error _ => {
                    unsafe { alloc.as(Allocator).deallocate(counter_block); }
                    return false;
                },
            };
            let outer_u = Box<Box<u64>>.new(inner_u) match {
                value! => value,
                error _ => {
                    unsafe { alloc.as(Allocator).deallocate(counter_block); }
                    return false;
                },
            };
            let nested_u_ok = outer_u.into_inner().into_inner() == 808;

            -- 7. Box<void> and zero-sized Drop struct.
            var unit_box = Box<void>.new(()) match {
                value! => value,
                error _ => {
                    unsafe { alloc.as(Allocator).deallocate(counter_block); }
                    return false;
                },
            };
            let unit_ok = unit_box.ref.layout().byte_size() == 0;
            {
                let zbox = Box<ZeroDrop>.new(ZeroDrop {}) match {
                    value! => value,
                    error _ => {
                        unsafe { alloc.as(Allocator).deallocate(counter_block); }
                        return false;
                    },
                };
            }
            let z_into = Box<ZeroDrop>.new(ZeroDrop {}) match {
                value! => value,
                error _ => {
                    unsafe { alloc.as(Allocator).deallocate(counter_block); }
                    return false;
                },
            };
            let z_raw = z_into.into_inner();

            unsafe {
                alloc.as(Allocator).deallocate(counter_block);
            }

            prim_ok and pair_ok and scope_drop_ok and into_inner_drop_ok
                and replace_set_drop_ok and nested_drop_ok and nested_u_ok and unit_ok
        }

        fn run() -> bool {
            test_box()
        }
    "#;
    let case = NativeCase::new(source);
    assert_success(&case.build());
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"ZZ(bool true)\n");
}

#[test]
fn drop_supports_staged_println_and_formatting_end_to_end() {
    let source = r#"
        use std.prelude.*;
        use std.mem.Box;

        struct TaggedDrop {
            tag: u64,
        }

        impl Drop for TaggedDrop {
            fn drop(*self) {
                let _ok = std.fmt.println("drop {}", self.tag.*) match {
                    _! => true,
                    error _ => false,
                };
            }
        }

        fn run() -> bool {
            let direct = TaggedDrop { tag: 11 };
            let boxed = Box<TaggedDrop>.new(TaggedDrop { tag: 22 }) match {
                value! => value,
                error _ => return false,
            };
            true
        }
    "#;
    let case = NativeCase::new(source);
    assert_success(&case.build());
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"drop 22\ndrop 11\n(bool true)\n");
}
