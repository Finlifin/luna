//! End-to-end tests for Stage 2C across `build` target/metadata helpers,
//! `std.time.Duration`, `std.process`, `std.fs` read/write helpers, and
//! `Debug` implementations for `VecDeque`, `HashMap`, and `HashSet`.
use std::fs;
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct TempCase(PathBuf);

impl TempCase {
    fn new(prefix: &str, files: &[(&str, &str)]) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "luna-{prefix}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        for (name, contents) in files {
            fs::write(path.join(name), contents).unwrap();
        }
        Self(path)
    }

    fn library_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library")
    }

    fn build_native(&self, main_file: &str, entry: &str) -> Output {
        ProcessCommand::new(env!("CARGO_BIN_EXE_luna"))
            .current_dir(&self.0)
            .args([
                "build",
                main_file,
                "--entry",
                entry,
                "--output",
                "program",
                "--opt-level",
                "0",
                "--sysroot",
            ])
            .arg(Self::library_dir())
            .output()
            .unwrap()
    }

    fn run_native(&self) -> Output {
        ProcessCommand::new(self.0.join("program"))
            .current_dir(&self.0)
            .output()
            .unwrap()
    }
}

impl Drop for TempCase {
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
fn build_target_and_target_features_helpers_work_via_sysroot() {
    let x86_source = r#"
        comptime fn run() -> bool {
            let layout_ok = build.target.is_64_bit()
                and (not build.target.is_32_bit())
                and build.target.pointer_bytes() == 8
                and build.target.is_arch(.x86_64)
                and (not build.target.is_arch(.aarch64))
                and build.target.is_os(.linux)
                and (not build.target.is_os(.windows));
            if not layout_ok {
                return false;
            }

            let features_ok = build.target.has_feature(.avx2)
                and (not build.target.has_feature(.neon))
                and (not build.target.has_feature(.missing))
                and build.target.features.is_enabled(.avx2)
                and (not build.target.features.is_disabled(.avx2))
                and build.target.features.is_disabled(.neon)
                and (not build.target.features.is_enabled(.neon))
                and (not build.target.features.is_enabled(.missing))
                and (not build.target.features.is_disabled(.missing));
            if not features_ok {
                return false;
            }

            let keys = build.target.features.keys();
            if keys.length() != 2 {
                return false;
            }
            let k0 = keys.get(0) match {
                s? => s,
                null => return false,
            };
            let k1 = keys.get(1) match {
                s? => s,
                null => return false,
            };
            (k0 == .avx2 and k1 == .neon) or (k0 == .neon and k1 == .avx2)
        }
    "#;
    let x86_case = TempCase::new("stage2c-target-x86", &[("main.fl", x86_source)]);
    let x86_out = ProcessCommand::new(env!("CARGO_BIN_EXE_luna"))
        .current_dir(&x86_case.0)
        .args(["comptime", "main.fl", "--entry", "run", "--sysroot"])
        .arg(TempCase::library_dir())
        .args([
            "--build-target",
            "x86_64",
            "linux",
            "64",
            "--build-feature",
            "avx2=true",
            "--build-feature",
            "neon=false",
        ])
        .output()
        .unwrap();
    assert_success(&x86_out);
    let x86_stdout = String::from_utf8_lossy(&x86_out.stdout);
    assert!(
        x86_stdout.contains("true"),
        "unexpected stdout: {x86_stdout}"
    );

    let rv32_source = r#"
        comptime fn run() -> bool {
            build.target.is_32_bit()
                and (not build.target.is_64_bit())
                and build.target.pointer_bytes() == 4
                and build.target.is_arch(.riscv32)
                and build.target.is_os(.none)
                and build.target.features.keys().length() == 0
        }
    "#;
    let rv32_case = TempCase::new("stage2c-target-rv32", &[("main.fl", rv32_source)]);
    let rv32_out = ProcessCommand::new(env!("CARGO_BIN_EXE_luna"))
        .current_dir(&rv32_case.0)
        .args(["comptime", "main.fl", "--entry", "run", "--sysroot"])
        .arg(TempCase::library_dir())
        .args(["--build-target", "riscv32", "none", "32"])
        .output()
        .unwrap();
    assert_success(&rv32_out);
    let rv32_stdout = String::from_utf8_lossy(&rv32_out.stdout);
    assert!(
        rv32_stdout.contains("true"),
        "unexpected stdout: {rv32_stdout}"
    );
}

#[test]
fn build_metadata_features_has_and_keys_work_via_real_build_package() {
    let app_source = r#"
        comptime fn run() -> bool {
            let f = build.package_metadata("build").features;
            let presence_ok = f.has(.enabled) and f.has(.disabled) and (not f.has(.absent));
            if not presence_ok {
                return false;
            }
            let values_ok = (f.get(.enabled, bool) matches true?)
                and (f.get(.disabled, bool) matches false?)
                and (f.get(.absent, bool) matches null);
            if not values_ok {
                return false;
            }
            let ks = f.keys();
            if ks.length() != 2 {
                return false;
            }
            let k0 = ks.get(0) match {
                s? => s,
                null => return false,
            };
            let k1 = ks.get(1) match {
                s? => s,
                null => return false,
            };
            (k0 == .enabled and k1 == .disabled) or (k0 == .disabled and k1 == .enabled)
        }
    "#;
    let case = TempCase::new("stage2c-metadata", &[("app.fl", app_source)]);
    let luna::cli::Command::Compile(options) = luna::cli::parse([
        case.0.join("app.fl").into_os_string(),
        "--sysroot".into(),
        TempCase::library_dir().into_os_string(),
        "--emit".into(),
        "nir".into(),
    ])
    .unwrap() else {
        panic!("expected compilation");
    };
    let mut protocols = elaboration::program::Protocols::default();
    let target = luna::frontend::native_test_target().expect("native target available");
    protocols.build_metadata = Some(std::sync::Arc::new(
        elaboration::program::build_context::Metadata {
            aliases: [(100, [("build".into(), 4)].into())].into(),
            inputs: [(
                4,
                [
                    (
                        "enabled".into(),
                        (
                            nir::Term::Primitive(nir::Primitive::Bool),
                            nir::Term::Bool(true),
                        ),
                    ),
                    (
                        "disabled".into(),
                        (
                            nir::Term::Primitive(nir::Primitive::Bool),
                            nir::Term::Bool(false),
                        ),
                    ),
                ]
                .into(),
            )]
            .into(),
        },
    ));
    let (luna::frontend::Artifact::Nir(package), _bindings) =
        luna::frontend::compile_bound_for_target(
            &options,
            100,
            &protocols,
            Default::default(),
            Some(target),
        )
        .unwrap()
    else {
        panic!("expected NIR");
    };
    assert_eq!(
        luna::invocation::invoke(&package, "run", 10_000_000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
}

#[test]
fn std_time_duration_and_std_process_run_natively() {
    let source = r#"
        use std.prelude.*;
        use std.mem.AllocErr;
        use std.process.{ExitCode, FAILURE, SUCCESS, id};
        use std.time.{
            Duration,
            MICROS_PER_SEC,
            MILLIS_PER_SEC,
            NANOS_PER_MICRO,
            NANOS_PER_MILLI,
            NANOS_PER_SEC,
            ZERO,
        };

        fn expect_duration(opt: ?Duration, secs: u64, nanos: u32) -> bool {
            opt match {
                d? => d.as_secs() == secs and d.subsec_nanos() == nanos,
                null => false,
            }
        }

        fn exercise() -> !AllocErr u32 {
            if NANOS_PER_SEC != 1000000000
                or NANOS_PER_MILLI != 1000000
                or NANOS_PER_MICRO != 1000
                or MILLIS_PER_SEC != 1000
                or MICROS_PER_SEC != 1000000
            {
                return 1;
            }
            if not ZERO.is_zero() or ZERO.as_secs() != 0 or ZERO.subsec_nanos() != 0 {
                return 2;
            }

            let d = Duration.new(1, 1500000000) match {
                v? => v,
                null => return 3,
            };
            if d.is_zero()
                or d.as_secs() != 2
                or d.subsec_nanos() != 500000000
                or d.subsec_millis() != 500
                or d.subsec_micros() != 500000
                or d.as_millis() != 2500
                or d.as_micros() != 2500000
                or d.as_nanos() != 2500000000
            {
                return 4;
            }
            if Duration.new(18446744073709551615, 1000000000).is_some() {
                return 5;
            }

            let d_secs = Duration.from_secs(3);
            let d_ms = Duration.from_millis(2345);
            let d_us = Duration.from_micros(1234567);
            let d_ns = Duration.from_nanos(2500000001);
            if d_secs.as_secs() != 3
                or d_secs.subsec_nanos() != 0
                or d_ms.as_secs() != 2
                or d_ms.subsec_nanos() != 345000000
                or d_us.as_secs() != 1
                or d_us.subsec_nanos() != 234567000
                or d_ns.as_secs() != 2
                or d_ns.subsec_nanos() != 500000001
            {
                return 6;
            }

            let add_a = Duration.from_nanos(1700000000);
            let add_b = Duration.from_nanos(2500000000);
            if not expect_duration(add_a.checked_add(add_b), 4, 200000000) {
                return 7;
            }
            let max_half = Duration.new(18446744073709551615, 500000000) match {
                v? => v,
                null => return 8,
            };
            if max_half.checked_add(Duration.from_millis(600)).is_some() {
                return 9;
            }

            let sub_a = Duration.from_nanos(3200000000);
            let sub_b = Duration.from_nanos(1700000000);
            if not expect_duration(sub_a.checked_sub(sub_b), 1, 500000000) {
                return 10;
            }
            if Duration.from_secs(1).checked_sub(Duration.from_nanos(1000000001)).is_some() {
                return 11;
            }

            let mul_base = Duration.from_millis(1500);
            if not expect_duration(mul_base.checked_mul(3), 4, 500000000)
                or not expect_duration(mul_base.checked_mul(0), 0, 0)
                or Duration.from_secs(18446744073709551615).checked_mul(2).is_some()
            {
                return 12;
            }

            if not expect_duration(Duration.from_secs(3).checked_div(2), 1, 500000000)
                or d.checked_div(0).is_some()
            {
                return 13;
            }

            let sat_add = max_half.saturating_add(Duration.from_millis(600));
            let sat_sub = Duration.from_secs(1).saturating_sub(Duration.from_secs(2));
            if sat_add.as_secs() != 18446744073709551615
                or sat_add.subsec_nanos() != 999999999
                or not sat_sub.is_zero()
            {
                return 14;
            }

            -- Exercise Copy (using `d` after copies), Clone, Eq, Ord, Hash, and Debug.
            let d_copy = d;
            let d_cloned = d_copy.as(Clone).clone();
            if d != d_cloned or d == ZERO {
                return 15;
            }
            let cmp_eq = d.as(Ord).cmp(d_cloned) matches .equal;
            let cmp_sec_lt = Duration.from_secs(1).as(Ord).cmp(Duration.from_secs(2)) matches .less;
            let cmp_sec_gt = Duration.from_secs(3).as(Ord).cmp(Duration.from_secs(2)) matches .greater;
            let cmp_ns_lt = Duration.from_nanos(100).as(Ord).cmp(Duration.from_nanos(200)) matches .less;
            let cmp_ns_gt = Duration.from_nanos(200).as(Ord).cmp(Duration.from_nanos(100)) matches .greater;
            if not cmp_eq or not cmp_sec_lt or not cmp_sec_gt or not cmp_ns_lt or not cmp_ns_gt {
                return 16;
            }

            if d.as(Hash).hash() != d_cloned.as(Hash).hash()
                or d.as(Hash).hash() == ZERO.as(Hash).hash()
            {
                return 17;
            }

            let d_dbg = format_debug(d.ref)!;
            if not d_dbg.ref.equals_str("Duration { secs: 2, nanos: 500000000 }") {
                return 18;
            }

            let pid1 = id();
            let pid2 = id();
            if pid1 == 0 or pid1 != pid2 {
                return 19;
            }
            if not SUCCESS.is_success()
                or SUCCESS.as_u8() != 0
                or FAILURE.is_success()
                or FAILURE.as_u8() != 1
            {
                return 20;
            }

            let custom = ExitCode.from_u8(42);
            let custom_cloned = custom.as(Clone).clone();
            if custom.as_u8() != 42
                or custom.is_success()
                or custom != custom_cloned
                or custom == SUCCESS
            {
                return 21;
            }
            let code_dbg = format_debug(custom.ref)!;
            if not code_dbg.ref.equals_str("ExitCode(42)") {
                return 22;
            }
            0
        }

        fn run() -> u32 {
            exercise() match {
                code! => code,
                error _ => 99,
            }
        }
    "#;

    let case = TempCase::new("stage2c-time-proc", &[("main.fl", source)]);
    assert_success(&case.build_native("main.fl", "run"));
    let output = case.run_native();
    assert_success(&output);
    assert_eq!(output.stdout, b"(unsigned-integer 0)\n");
}

#[test]
fn std_fs_helpers_with_default_and_custom_allocators_run_natively() {
    let source = r#"
        use std.prelude.*;
        use std.fs.{read, read_to_string, remove_file, write_bytes, write_str};
        use std.io.IoErr;
        use std.mem.{AllocErr, Allocator, FixedBufferAllocator, Layout, SystemAllocator};

        fn expect_byte(opt: ?u8, expected: u8) -> bool {
            opt match {
                v? => v == expected,
                null => false,
            }
        }

        fn exercise(fba: *FixedBufferAllocator) -> ![IoErr, AllocErr] u32 {
            write_str("stage2c_text.txt", "hello flurry\n")!;
            let default_text = read_to_string("stage2c_text.txt") match {
                s! => s,
                error _ => return 1,
            };
            if not default_text.ref.equals_str("hello flurry\n") {
                return 2;
            }
            let fba_text = read_to_string<.A = FixedBufferAllocator, .allocator = fba>("stage2c_text.txt") match {
                s! => s,
                error _ => return 3,
            };
            if not fba_text.ref.equals_str("hello flurry\n") {
                return 4;
            }

            var raw_in = Vec<u8, .A = FixedBufferAllocator, .allocator = fba>.new();
            raw_in.ref.push(255)!;
            raw_in.ref.push(254)!;
            raw_in.ref.push(10)!;
            raw_in.ref.push(0)!;
            raw_in.ref.push(65)!;
            write_bytes("stage2c_bin.dat", raw_in.ref)!;

            let def_bytes = read("stage2c_bin.dat")!;
            let fba_bytes = read<.A = FixedBufferAllocator, .allocator = fba>("stage2c_bin.dat")!;
            if def_bytes.ref.len() != 5
                or fba_bytes.ref.len() != 5
                or not expect_byte(def_bytes.ref.get(0), 255)
                or not expect_byte(def_bytes.ref.get(1), 254)
                or not expect_byte(def_bytes.ref.get(2), 10)
                or not expect_byte(def_bytes.ref.get(3), 0)
                or not expect_byte(def_bytes.ref.get(4), 65)
                or not expect_byte(fba_bytes.ref.get(0), 255)
                or not expect_byte(fba_bytes.ref.get(4), 65)
            {
                return 5;
            }

            let invalid_utf8_caught = read_to_string("stage2c_bin.dat") match {
                _! => false,
                error IoErr.Os(_) => false,
                error IoErr.WriteZero => false,
                error IoErr.UnexpectedEof => false,
                error AllocErr.OutOfMemory => false,
                error AllocErr.InvalidLayout => false,
                error _ => true,
            };
            if not invalid_utf8_caught {
                return 6;
            }

            let missing_caught = read("stage2c_missing.txt") match {
                _! => false,
                error IoErr.Os(code) => code > 0,
                error IoErr.WriteZero => false,
                error IoErr.UnexpectedEof => false,
                error AllocErr.OutOfMemory => false,
                error AllocErr.InvalidLayout => false,
            };
            if not missing_caught {
                return 7;
            }

            remove_file("stage2c_text.txt")!;
            remove_file("stage2c_bin.dat")!;
            0
        }

        fn run() -> u32 {
            let layout = Layout.new(32768, 16) match {
                found? => found,
                null => return 97,
            };
            let backing = SystemAllocator.GLOBAL.ref.as(Allocator).allocate(layout) match {
                block! => block,
                error _ => return 98,
            };
            var fba = FixedBufferAllocator.from_raw_block(backing);
            let code = exercise(fba.ref) match {
                passed! => passed,
                error _ => 99,
            };
            let clean = fba.ref.live_allocations() == 0;
            unsafe {
                SystemAllocator.GLOBAL.ref.as(Allocator).deallocate(backing);
            }
            if clean { code } else { 96 }
        }
    "#;

    let case = TempCase::new("stage2c-fs", &[("main.fl", source)]);
    assert_success(&case.build_native("main.fl", "run"));
    let output = case.run_native();
    assert_success(&output);
    assert_eq!(output.stdout, b"(unsigned-integer 0)\n");
}

#[test]
fn std_collections_debug_impls_preserve_once_elements_and_custom_allocators() {
    let source = r#"
        use std.prelude.*;
        use builtin.fmt.ByteWriter;
        use std.collections.{HashMap, HashSet, VecDeque};
        use std.mem.{AllocErr, Allocator, FixedBufferAllocator, Layout, SystemAllocator};

        struct KeyTag {
            id: u64,
        }

        impl Clone for KeyTag {
            fn clone(*self) -> Self {
                KeyTag { id: self.id.* }
            }
        }

        impl Eq for KeyTag {
            fn eq(self, other: Self) -> bool {
                self.id == other.id
            }
        }

        impl Hash for KeyTag {
            fn hash(self) -> u64 {
                self.id.as(Hash).hash()
            }
        }

        impl Debug for KeyTag {
            fn write_debug(*self, output: *W) -> !W.as(ByteWriter).Err void where W :- ByteWriter {
                output.write_str("K(")!;
                self.id.*.as(Display).write(output)!;
                output.write_byte(41)
            }
        }

        fn make_fba_str(text: str) -> !AllocErr String<.A = FixedBufferAllocator, .allocator = fba>
        where
            quote fba: lift *FixedBufferAllocator,
        {
            String<.A = FixedBufferAllocator, .allocator = fba>.from_str(text)
        }

        fn check_fba_debug(
            fba: *FixedBufferAllocator,
            expected: str,
            value: *T,
        ) -> !AllocErr bool
        where
            T :- Debug,
        {
            let formatted = format_debug<.A = FixedBufferAllocator, .allocator = fba>(value)!;
            formatted.ref.equals_str(expected)
        }

        fn exercise(fba: *FixedBufferAllocator) -> !AllocErr u32 {
            -- 1. Empty collection Debug impls.
            var empty_dq = VecDeque<String<.A = FixedBufferAllocator, .allocator = fba>, .A = FixedBufferAllocator, .allocator = fba>.new();
            var empty_map = HashMap<KeyTag, String<.A = FixedBufferAllocator, .allocator = fba>, .A = FixedBufferAllocator, .allocator = fba>.new();
            var empty_set = HashSet<KeyTag, .A = FixedBufferAllocator, .allocator = fba>.new();
            let empty_dq_ok = check_fba_debug(fba, "[]", empty_dq.ref)!;
            let empty_map_ok = check_fba_debug(fba, "{}", empty_map.ref)!;
            let empty_set_ok = check_fba_debug(fba, "{}", empty_set.ref)!;
            if not empty_dq_ok or not empty_map_ok or not empty_set_ok {
                return 8;
            }

            -- 2. Wraparound VecDeque<String, ...> Debug must read ring order without consuming items.
            var dq = VecDeque<String<.A = FixedBufferAllocator, .allocator = fba>, .A = FixedBufferAllocator, .allocator = fba>.with_capacity(4)!;
            dq.ref.push_back(make_fba_str<fba>("drop0")!)!;
            dq.ref.push_back(make_fba_str<fba>("drop1")!)!;
            let _ = dq.ref.pop_front();
            let _ = dq.ref.pop_front();
            dq.ref.push_back(make_fba_str<fba>("alpha")!)!;
            dq.ref.push_back(make_fba_str<fba>("beta")!)!;
            dq.ref.push_back(make_fba_str<fba>("gamma")!)!;
            let dq_dbg_ok = check_fba_debug(fba, "[\"alpha\", \"beta\", \"gamma\"]", dq.ref)!;
            if not dq_dbg_ok or dq.ref.len() != 3 {
                return 9;
            }

            -- 3. HashMap<KeyTag, String, ...> Debug with non-Copy Clone key and affine Once String value.
            var map = HashMap<KeyTag, String<.A = FixedBufferAllocator, .allocator = fba>, .A = FixedBufferAllocator, .allocator = fba>.new();
            let _ = map.ref.insert(KeyTag { id: 1 }, make_fba_str<fba>("k")!)!;
            let map_one_ok = check_fba_debug(fba, "{K(1): \"k\"}", map.ref)!;
            if not map_one_ok {
                return 10;
            }
            let _ = map.ref.insert(KeyTag { id: 2 }, make_fba_str<fba>("m")!)!;
            let map_two_dbg = format_debug<.A = FixedBufferAllocator, .allocator = fba>(map.ref)!;
            if (not map_two_dbg.ref.equals_str("{K(1): \"k\", K(2): \"m\"}"))
                and (not map_two_dbg.ref.equals_str("{K(2): \"m\", K(1): \"k\"}"))
            {
                return 11;
            }
            if map.ref.len() != 2 or not map.ref.contains_key(KeyTag { id: 1 }) {
                return 12;
            }
            let removed_k_ok = map.ref.remove(KeyTag { id: 1 }) match {
                v? => v.ref.equals_str("k"),
                null => false,
            };
            if not removed_k_ok {
                return 13;
            }

            -- 4. HashSet<KeyTag, ...> Debug with non-Copy Clone element and intact lookup afterwards.
            var set = HashSet<KeyTag, .A = FixedBufferAllocator, .allocator = fba>.new();
            let _ = set.ref.insert(KeyTag { id: 42 })!;
            let set_dbg_ok = check_fba_debug(fba, "{K(42)}", set.ref)!;
            if not set_dbg_ok
                or set.ref.len() != 1
                or not set.ref.contains(KeyTag { id: 42 })
            {
                return 14;
            }

            0
        }

        fn run() -> u32 {
            let layout = Layout.new(65536, 16) match {
                found? => found,
                null => return 97,
            };
            let backing = SystemAllocator.GLOBAL.ref.as(Allocator).allocate(layout) match {
                block! => block,
                error _ => return 98,
            };
            var fba = FixedBufferAllocator.from_raw_block(backing);
            let code = exercise(fba.ref) match {
                passed! => passed,
                error _ => 99,
            };
            let clean = fba.ref.live_allocations() == 0;
            unsafe {
                SystemAllocator.GLOBAL.ref.as(Allocator).deallocate(backing);
            }
            if clean { code } else { 96 }
        }
    "#;

    let case = TempCase::new("stage2c-collections-debug", &[("main.fl", source)]);
    assert_success(&case.build_native("main.fl", "run"));
    let output = case.run_native();
    assert_success(&output);
    assert_eq!(output.stdout, b"(unsigned-integer 0)\n");
}
