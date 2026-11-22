//! End-to-end tests for `pattern_automata` Decision-DAG lowering in `host/llvm`:
//! single-observation lazy list caching, loop reset safety, multi-way switches
//! (`Enum`/`Optional`/`Error`/`Int`/`Char`/`BitSlice`/`ListLen`), `GuardCheck`/`Bind`/`Or`,
//! affine `Drop` resource transfer via IIR `cleanup::Schedule` + LLVM IR, and sound fallbacks.

use luna::cli;
use luna::frontend::{self, Artifact};
use std::fs;
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct NativeDagCase(PathBuf);

impl NativeDagCase {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "luna-pa-dag-llvm-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("main.fl"), source).unwrap();
        Self(path)
    }

    fn sysroot() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../library")
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
            .arg(Self::sysroot())
            .output()
            .unwrap()
    }

    fn interpret(&self) -> Output {
        self.interpret_entry("run")
    }

    fn interpret_entry(&self, entry: &str) -> Output {
        ProcessCommand::new(env!("CARGO_BIN_EXE_luna"))
            .current_dir(&self.0)
            .args([
                "main.fl",
                "--interpret",
                entry,
                "--fuel",
                "10000000",
                "--build-target",
                "x86_64",
                "linux",
                "64",
                "--sysroot",
            ])
            .arg(Self::sysroot())
            .output()
            .unwrap()
    }

    fn run(&self) -> Output {
        let mut command = ProcessCommand::new(self.0.join("program"));
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

impl Drop for NativeDagCase {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn assert_ok(output: &Output, context: &str) {
    assert!(
        output.status.success(),
        "{context}\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn verify_interpret_and_native(source: &str) {
    let dir = NativeDagCase::new(source);
    let interp = dir.interpret();
    assert_ok(&interp, "interpret");
    assert_eq!(
        String::from_utf8_lossy(&interp.stdout).trim(),
        "(bool true)"
    );
    for opt in ["0", "3"] {
        let build = dir.build(opt);
        assert_ok(&build, opt);
        let run = dir.run();
        assert_ok(&run, opt);
        assert_eq!(
            String::from_utf8_lossy(&run.stdout).trim(),
            "(bool true)",
            "opt={opt}"
        );
    }
}

fn prepare_host(source: &str) -> (nir::program::Package, luna::host::Prepared, String) {
    let dir = NativeDagCase::new(source);
    let command = cli::parse([
        dir.0.join("main.fl").into_os_string(),
        "--build".into(),
        "run".into(),
        "--output".into(),
        "program".into(),
        "--fuel".into(),
        "10000000".into(),
        "--sysroot".into(),
        NativeDagCase::sysroot().into_os_string(),
    ])
    .unwrap();
    let cli::Command::Host { options, .. } = command else {
        panic!("expected Host command");
    };
    let (artifact, bindings) =
        frontend::compile_bound_for_native_host(&options, None, None, None, None, None, &[])
            .unwrap();
    let Artifact::Nir(package) = artifact else {
        panic!("expected NIR artifact");
    };
    let prepared = luna::host::prepare_with_protocols(
        &package,
        "run",
        10_000_000,
        bindings.copy.unwrap(),
        bindings.drop.unwrap(),
    )
    .unwrap();
    let ir = luna::host::llvm_ir_prepared(&prepared, 10_000_000).unwrap();
    (package, prepared, ir)
}

fn llvm_function_body<'a>(ir: &'a str, fn_index: usize) -> &'a str {
    let needle = format!("@flurry_{fn_index}(");
    let def_start = ir
        .match_indices("define ")
        .find_map(|(pos, _)| {
            let line_end = ir[pos..].find('\n').map_or(ir.len(), |n| pos + n);
            ir[pos..line_end].contains(&needle).then_some(pos)
        })
        .unwrap_or_else(|| panic!("missing LLVM function @flurry_{fn_index} in:\n{ir}"));
    let rest = &ir[def_start..];
    let body_end = rest.find("\n}\n").map_or(rest.len(), |n| def_start + n + 3);
    &ir[def_start..body_end]
}

fn iir_fn_indices(prepared: &luna::host::Prepared, package: u64, name: &str) -> Vec<usize> {
    prepared
        .graph()
        .functions
        .iter()
        .enumerate()
        .filter(|(_, f)| f.key.declaration.package == package && f.function.name == name)
        .map(|(idx, _)| idx)
        .collect()
}

fn count_flurry_calls(body: &str, fn_index: usize) -> usize {
    let direct = format!("@flurry_{fn_index}(");
    let scalar = format!("@flurry_{fn_index}_scalar(");
    body.matches(&direct).count() + body.matches(&scalar).count()
}

#[test]
fn dag_single_observation_list_len_and_llvm_switch() {
    let source = r#"
        use std.prelude.*;
        use std.mem.{AllocErr, ArenaAllocator};

        struct Probe {
            len: usize,
            a: i64,
            b: i64,
        }

        impl ListPatternView for Probe {
            assoc Element: Type = i64;
            fn list_pattern_view_get_len(*self) -> usize {
                self.len.*
            }
            fn list_pattern_view_get_element(*self, comptime index: usize) -> i64 {
                if index == 0 {
                    self.a.*
                } else {
                    self.b.*
                }
            }
        }

        impl StableListPatternView for Probe {}

        fn inspect_probe(p: Probe) -> i64 {
            p match {
                [] => 10,
                [x] => x,
                [1, y] => if y > -1000 and y < 1000 { y + 100 } else { -1 },
                [x, y, ..._] => if x > -1000 and x < 1000 and y > -1000 and y < 1000 {
                    x + y + 1000
                } else {
                    -2
                },
            }
        }

        fn classify_slice(s: Slice<i64>) -> i64 {
            s match {
                [] => 0,
                [x] => x,
                [7, y] => if y > -1000 and y < 1000 { y * 10 } else { -1 },
                [x, y, ...rest] => {
                    let extra: i64 = if rest.ref.len() == 0 {
                        0
                    } else if rest.ref.len() == 1 {
                        1
                    } else {
                        2
                    };
                    if x > -1000 and x < 1000 and y > -1000 and y < 1000 {
                        x + y + extra
                    } else {
                        -2
                    }
                },
            }
        }

        fn classify_pair_and_nested(tag: bool, nested: Slice<Slice<i64>>) -> i64 {
            (tag, nested) match {
                (true, [[x], ..._]) => if x > -1000 and x < 1000 { x + 1 } else { -1 },
                (true, [_, [y]]) => if y > -1000 and y < 1000 { y + 2 } else { -2 },
                (false, [first, ..._]) => if first.ref.len() == 1 { 101 } else { 100 },
                _ => -1,
            }
        }

        fn test_slices_and_vecs() -> !AllocErr bool {
            var v = Vec<i64>.new();
            let s0 = v.ref.as_slice();
            if classify_slice(s0) != 0 {
                return false;
            }
            v.ref.push(9)!;
            let s1 = v.ref.as_slice();
            if classify_slice(s1) != 9 {
                return false;
            }
            v.ref.push(4)!;
            let s3: i64 = v match {
                [] => -1,
                [x] => x,
                [9, y] => if y > -1000 and y < 1000 { y * 100 } else { -2 },
                [x, y, ...rest] => {
                    let extra: i64 = if rest.ref.len() == 1 { 1 } else { 0 };
                    if x > -1000 and x < 1000 and y > -1000 and y < 1000 {
                        x + y + extra
                    } else {
                        -3
                    }
                },
            };
            if s3 != 400 {
                return false;
            }

            var arena = ArenaAllocator.new();
            var av = Vec<i64, .A = ArenaAllocator, .allocator = arena.ref>.with_capacity(4)!;
            av.ref.push(7)!;
            av.ref.push(3)!;
            av.ref.push(8)!;
            let a_res: i64 = av match {
                [] => -1,
                [x] => x,
                [7, y] => y,
                [x, y, ...rest] => {
                    let extra: i64 = if rest.ref.len() == 1 { 1 } else { 0 };
                    if x > -1000 and x < 1000 and y > -1000 and y < 1000 {
                        x * 10 + y + extra
                    } else {
                        -3
                    }
                },
            };
            if a_res != 74 {
                return false;
            }

            var inner1 = Vec<i64>.new();
            inner1.ref.push(41)!;
            var inner2 = Vec<i64>.new();
            inner2.ref.push(19)!;
            inner2.ref.push(20)!;
            var outer = Vec<Slice<i64>>.new();
            outer.ref.push(inner1.ref.as_slice())!;
            outer.ref.push(inner2.ref.as_slice())!;
            let nested_ok = classify_pair_and_nested(true, outer.ref.as_slice()) == 42
                and classify_pair_and_nested(false, outer.ref.as_slice()) == 101;
            nested_ok
        }

        fn check_probe_arms() -> bool {
            if inspect_probe(Probe { len: 0, a: 0, b: 0 }) != 10 {
                return false;
            }
            if inspect_probe(Probe { len: 1, a: 42, b: 0 }) != 42 {
                return false;
            }
            if inspect_probe(Probe { len: 2, a: 1, b: 7 }) != 107 {
                return false;
            }
            if inspect_probe(Probe { len: 2, a: 5, b: 6 }) != 1011 {
                return false;
            }
            if inspect_probe(Probe { len: 3, a: 5, b: 6 }) != 1011 {
                return false;
            }
            true
        }

        fn run() -> bool {
            if not check_probe_arms() {
                return false;
            }
            test_slices_and_vecs() match {
                ok! => ok,
                error _ => false,
            }
        }

        fn run_parity() -> bool {
            check_probe_arms()
        }
    "#;

    let (_, prepared, ir) = prepare_host(source);
    let inspect_indices = iir_fn_indices(&prepared, 0, "inspect_probe");
    assert_eq!(inspect_indices.len(), 1);
    let inspect_ir = llvm_function_body(&ir, inspect_indices[0]);
    let len_fn_indices = iir_fn_indices(&prepared, 0, "list_pattern_view_get_len");
    assert_eq!(len_fn_indices.len(), 1);
    let len_calls_in_ir = count_flurry_calls(inspect_ir, len_fn_indices[0]);
    assert_eq!(
        len_calls_in_ir, 1,
        "expected single get_len call in Decision-DAG lowered `inspect_probe`, found {len_calls_in_ir}:\n{inspect_ir}"
    );
    assert!(
        inspect_ir.contains("switch i64 "),
        "expected LLVM switch on list length in `inspect_probe`:\n{inspect_ir}"
    );

    let slice_indices = iir_fn_indices(&prepared, 0, "classify_slice");
    assert_eq!(slice_indices.len(), 1);
    let slice_ir = llvm_function_body(&ir, slice_indices[0]);
    assert!(
        slice_ir.contains("switch i64 "),
        "expected LLVM switch on Slice<i64> length in `classify_slice`:\n{slice_ir}"
    );

    let nested_indices = iir_fn_indices(&prepared, 0, "classify_pair_and_nested");
    assert_eq!(nested_indices.len(), 1);
    let nested_ir = llvm_function_body(&ir, nested_indices[0]);
    assert!(
        nested_ir.contains("switch i64 "),
        "expected LLVM switch on nested Slice<Slice<i64>> length in `classify_pair_and_nested`:\n{nested_ir}"
    );

    let dir = NativeDagCase::new(source);
    let interp = dir.interpret_entry("run_parity");
    assert_ok(&interp, "interpret");
    assert_eq!(
        String::from_utf8_lossy(&interp.stdout).trim(),
        "(bool true)"
    );
    for opt in ["0", "3"] {
        let build = dir.build(opt);
        assert_ok(&build, opt);
        let run = dir.run();
        assert_ok(&run, opt);
        assert_eq!(
            String::from_utf8_lossy(&run.stdout).trim(),
            "(bool true)",
            "opt={opt}"
        );
    }
}

#[test]
fn dag_list_loop_resets_cached_observation_every_iteration() {
    let source = r#"
        use std.prelude.*;
        use std.mem.AllocErr;

        fn step_slice(s: Slice<i64>) -> i64 {
            s match {
                [] => 1,
                [x] => if x > -1000 and x < 1000 { x * 2 } else { -1 },
                [1, y] => if y > -1000 and y < 1000 { y + 100 } else { -2 },
                [x, y, ...rest] => {
                    let extra: i64 = if rest.ref.len() == 0 {
                        0
                    } else if rest.ref.len() == 1 {
                        10
                    } else {
                        20
                    };
                    if x > -1000 and x < 1000 and y > -1000 and y < 1000 {
                        x + y + extra
                    } else {
                        -3
                    }
                },
            }
        }

        fn loop_test() -> !AllocErr bool {
            var v = Vec<i64>.new();
            var acc: i64 = 0;
            var i: usize = 0;
            while i < 4 {
                -- Every iteration matches a fresh slice/vec inside the loop body directly.
                let s = v.ref.as_slice();
                let res: i64 = s match {
                    [] => 1,
                    [x] => if x > -1000 and x < 1000 { x * 2 } else { -1 },
                    [10, y] => if y > -1000 and y < 1000 { y + 100 } else { -2 },
                    [x, y, ...rest] => {
                        let extra: i64 = if rest.ref.len() == 0 {
                            0
                        } else if rest.ref.len() == 1 {
                            10
                        } else {
                            20
                        };
                        if x > -1000 and x < 1000 and y > -1000 and y < 1000 {
                            x + y + extra
                        } else {
                            -3
                        }
                    },
                };
                let step = step_slice(v.ref.as_slice());
                if acc > -10000 and acc < 10000 and res > -1000 and res < 1000 and step > -1000 and step < 1000 {
                    acc = acc + res + step;
                }
                if i == 0 {
                    v.ref.push(10)!;
                } else if i == 1 {
                    v.ref.push(20)!;
                } else if i == 2 {
                    v.ref.push(30)!;
                }
                i = i + 1;
            }
            -- iter0 (`[]`): res=1 + step=1 => 2
            -- iter1 (`[10]`): res=20 + step=20 => 40
            -- iter2 (`[10, 20]`): res=120 + step=(10+20+0)=30 => 150
            -- iter3 (`[10, 20, 30]`): res=(10+20+10)=40 + step=40 => 80
            -- total = 2 + 40 + 150 + 80 = 272
            acc == 272
        }

        struct LoopProbe {
            len: usize,
            a: i64,
            b: i64,
        }

        impl ListPatternView for LoopProbe {
            assoc Element: Type = i64;
            fn list_pattern_view_get_len(*self) -> usize {
                self.len.*
            }
            fn list_pattern_view_get_element(*self, comptime index: usize) -> i64 {
                if index == 0 {
                    self.a.*
                } else {
                    self.b.*
                }
            }
        }

        impl StableListPatternView for LoopProbe {}

        fn loop_probe_check() -> bool {
            var acc: i64 = 0;
            var i: usize = 0;
            while i < 4 {
                let p = LoopProbe { len: i, a: 10, b: 20 };
                let res: i64 = p match {
                    [] => 1,
                    [x] => if x > -1000 and x < 1000 { x * 2 } else { -1 },
                    [10, y] => if y > -1000 and y < 1000 { y + 100 } else { -2 },
                    [x, y, ..._] => if x > -1000 and x < 1000 and y > -1000 and y < 1000 {
                        x + y + 10
                    } else {
                        -3
                    },
                };
                if acc > -10000 and acc < 10000 and res > -1000 and res < 1000 {
                    acc = acc + res;
                }
                i = i + 1;
            }
            -- total = 1 + 20 + 120 + 40 = 181
            acc == 181
        }

        fn run() -> bool {
            let probe_ok = loop_probe_check();
            let vec_ok = loop_test() match {
                ok! => ok,
                error _ => false,
            };
            probe_ok and vec_ok
        }

        fn run_parity() -> bool {
            loop_probe_check()
        }
    "#;

    let (_, prepared, ir) = prepare_host(source);
    let loop_indices = iir_fn_indices(&prepared, 0, "loop_probe_check");
    assert_eq!(loop_indices.len(), 1);
    let loop_ir = llvm_function_body(&ir, loop_indices[0]);
    let len_fn_indices = iir_fn_indices(&prepared, 0, "list_pattern_view_get_len");
    assert_eq!(len_fn_indices.len(), 1);
    let len_calls_in_ir = count_flurry_calls(loop_ir, len_fn_indices[0]);
    assert_eq!(
        len_calls_in_ir, 1,
        "expected single get_len call in Decision-DAG lowered `loop_probe_check`, found {len_calls_in_ir}:\n{loop_ir}"
    );
    assert!(
        loop_ir.contains("switch i64 "),
        "expected LLVM switch on list length in `loop_probe_check`:\n{loop_ir}"
    );
    assert!(
        loop_ir.contains("store i1 false, ptr "),
        "expected per-entry list-cache reset store in `loop_probe_check`:\n{loop_ir}"
    );

    let dir = NativeDagCase::new(source);
    let interp = dir.interpret_entry("run_parity");
    assert_ok(&interp, "interpret");
    assert_eq!(
        String::from_utf8_lossy(&interp.stdout).trim(),
        "(bool true)"
    );
    for opt in ["0", "3"] {
        let build = dir.build(opt);
        assert_ok(&build, opt);
        let run = dir.run();
        assert_ok(&run, opt);
        assert_eq!(
            String::from_utf8_lossy(&run.stdout).trim(),
            "(bool true)",
            "opt={opt}"
        );
    }
}

#[test]
fn dag_all_switch_kinds_and_native_parity() {
    let source = r#"
        typealias Bits16 = BitVec<16>;
        typealias Bits128 = BitVec<128>;
        typealias Nib = BitVec<4>;

        enum Tri { alpha, beta, gamma }

        struct Point { x: i64, y: i64 }

        enum Shape {
            none,
            dot(i64),
            pair(i64, i64),
            named { tag: bool, point: Point },
        }

        enum OptSmall { empty, one, two, carry(bool) }

        enum Instruction {
            .width 16,
            .discrete_tag true,
            nop: 0b0000000000000000,
            halt: 0b1111111111111111,
            mov: 0b0001 (dst: Nib) (src: u8),
        }

        struct AErr { code: i64 }
        struct BErr { flag: bool }
        struct CErr {}

        fn switch_unit(t: Tri) -> i64 {
            t match {
                .alpha => 1,
                .beta => 2,
                .gamma => 3,
            }
        }

        fn switch_tagged(s: Shape) -> i64 {
            s match {
                .none => 0,
                .dot(v) => v,
                .pair(a, b) => if a > -1000 and a < 1000 and b > -1000 and b < 1000 {
                    a + b
                } else {
                    -1
                },
                .named { tag: true, point: { x, y } } => if x > -1000 and x < 1000 and y > -1000 and y < 1000 {
                    x + y + 100
                } else {
                    -2
                },
                .named { tag: false, point: { x, y } } => if x > -1000 and x < 1000 and y > -1000 and y < 1000 {
                    x + y
                } else {
                    -3
                },
            }
        }

        fn switch_niche(n: OptSmall) -> i64 {
            n match {
                .empty => 10,
                .one => 20,
                .two => 30,
                .carry(b) => if b { 41 } else { 40 },
            }
        }

        fn switch_fixed_bits(ins: Instruction) -> i64 {
            ins match {
                .nop => 1,
                .halt => 2,
                .mov { dst, src } => if dst == 3 and src == 25 { 325 } else { 0 },
            }
        }

        fn switch_symbol(s: Symbol) -> i64 {
            s match {
                .read => 10,
                .write => 20,
                .sync => 30,
                _ => 40,
            }
        }

        fn make_error_val(flag: i32) -> ![AErr, BErr, CErr] i64 {
            if flag == 0 {
                42
            } else if flag == 1 {
                error AErr { code: 7 }
            } else if flag == 2 {
                error BErr { flag: true }
            } else {
                error CErr {}
            }
        }

        fn switch_error(flag: i32) -> i64 {
            make_error_val(flag) match {
                v! => v,
                error AErr { code } => if code > -1000 and code < 1000 { code + 100 } else { -1 },
                error BErr { flag: true } => 201,
                error BErr { flag: false } => 200,
                error CErr {} => 300,
            }
        }

        fn switch_signed_int_tree(v: i8) -> i64 {
            v match {
                -128..=-100 => 1,
                -99..=-1 => 2,
                0 => 3,
                1..=10 => 4,
                11..=20 => 5,
                21..=50 => 6,
                51..=100 => 7,
                101..=127 => 8,
            }
        }

        fn switch_char_ranges(c: char) -> i64 {
            c match {
                'a'..='z' => 1,
                'A'..='Z' => 2,
                '0'..='9' => 3,
                '_' => 4,
                _ => 5,
            }
        }

        fn switch_bitvec_16(bits: Bits16) -> i64 {
            bits match {
                0x0A (mid: Nib) 5 => if mid == 7 { 107 } else { 100 },
                0x1 (mid: i8) F => if mid == -128 { 872 } else { 0 },
                _ => -1,
            }
        }

        fn switch_bitvec_128(bits: Bits128) -> i64 {
            let tail: Bits128 = bits[0..128];
            tail match {
                0x00000000000000000000000000000001 => 1,
                0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF => 2,
                _ => 3,
            }
        }

        fn switch_nested_tuple(v: Tuple<Tri, ?Tuple<i64, char>>) -> i64 {
            v match {
                (.alpha, (x, 'a')?) => x,
                (.alpha, (_, _)?) => 999,
                (.alpha, null) => -1,
                (.beta, _) => 200,
                (.gamma, _) => 300,
            }
        }

        fn run() -> bool {
            switch_unit(.alpha) == 1
                and switch_unit(.gamma) == 3
                and switch_tagged(.none) == 0
                and switch_tagged(.dot(12)) == 12
                and switch_tagged(.pair(3, 4)) == 7
                and switch_tagged(.named { tag: true, point: Point { x: 1, y: 2 } }) == 103
                and switch_niche(.two) == 30
                and switch_niche(.carry(true)) == 41
                and switch_fixed_bits(.nop) == 1
                and switch_fixed_bits(.halt) == 2
                and switch_fixed_bits(Instruction.mov { dst: 3, src: 25 }) == 325
                and switch_symbol(.sync) == 30
                and switch_symbol(.other) == 40
                and switch_error(0) == 42
                and switch_error(1) == 107
                and switch_error(2) == 201
                and switch_error(3) == 300
                and switch_signed_int_tree(-128) == 1
                and switch_signed_int_tree(-5) == 2
                and switch_signed_int_tree(0) == 3
                and switch_signed_int_tree(15) == 5
                and switch_signed_int_tree(127) == 8
                and switch_char_ranges('m') == 1
                and switch_char_ranges('Q') == 2
                and switch_char_ranges('7') == 3
                and switch_char_ranges('_') == 4
                and switch_char_ranges('!') == 5
                and switch_bitvec_16(0x0A75) == 107
                and switch_bitvec_16(0x0A35) == 100
                and switch_bitvec_16(0x18FF) == 0
                and switch_bitvec_16(0x1FF5) == -1
                and switch_bitvec_16(0x180F) == 872
                and switch_bitvec_128(1) == 1
                and switch_bitvec_128(0x55) == 3
                and switch_nested_tuple((.alpha, (55, 'a'))) == 55
                and switch_nested_tuple((.alpha, (55, 'z'))) == 999
                and switch_nested_tuple((.alpha, null)) == -1
                and switch_nested_tuple((.gamma, null)) == 300
        }
    "#;

    let (_, prepared, ir) = prepare_host(source);
    for (fn_name, needle) in [
        ("switch_unit", "switch i8 "),
        ("switch_niche", "switch i8 "),
        ("switch_error", "switch i8 "),
    ] {
        let idxs = iir_fn_indices(&prepared, 0, fn_name);
        assert_eq!(idxs.len(), 1);
        let body = llvm_function_body(&ir, idxs[0]);
        assert!(
            body.contains(needle),
            "expected `{needle}` in DAG-lowered `{fn_name}`:\n{body}"
        );
    }

    verify_interpret_and_native(source);
}

#[test]
fn dag_guarded_or_remap_and_fallback_parity() {
    let source = r#"
        enum Channel {
            left(i64, i64),
            right(i64, i64),
            idle,
        }

        impl Clone for Channel {
            fn clone(*self) -> Self {
                self.*
            }
        }

        impl Copy for Channel {}

        impl Channel {
            case positive_sum(*self) -> ?Tuple<i64, bool> {
                self.* match {
                    .left(a, b) if a > -1000 and a < 1000 and b > -1000 and b < 1000 and a + b > 0 => {
                        if a > -1000 and a < 1000 and b > -1000 and b < 1000 {
                            (a + b, true)
                        } else {
                            null
                        }
                    },
                    .right(a, b) if a > -1000 and a < 1000 and b > -1000 and b < 1000 and a + b > 0 => {
                        if a > -1000 and a < 1000 and b > -1000 and b < 1000 {
                            (a + b, true)
                        } else {
                            null
                        }
                    },
                    _ => null,
                }
            }
        }

        fn route_with_guard_and_remap(c: Channel) -> i64 {
            c match {
                (.left(x, y) or .right(y, x)) if x > 10 and y > 0 => {
                    if x > 10 and x < 1000 and y > 0 and y < 1000 {
                        x * 100 + y
                    } else {
                        -2
                    }
                },
                .left(x, y) => if x > -1000 and x < 1000 and y > -1000 and y < 1000 {
                    x + y
                } else {
                    -3
                },
                .right(x, y) => if x > -1000 and x < 1000 and y > -1000 and y < 1000 {
                    y - x
                } else {
                    -4
                },
                .idle => -1,
            }
        }

        fn inner_or_guard_fallback(c: Channel) -> i64 {
            c match {
                (.left(a, b) if a > b) or (.right(a, b) if b > a) => {
                    if a > -1000 and a < 1000 and b > -1000 and b < 1000 {
                        a + b
                    } else {
                        -98
                    }
                },
                _ => -99,
            }
        }

        fn float_and_str_fallback(f: f64, s: str) -> bool {
            let f_ok = f match {
                1.5 => true,
                _ => false,
            };
            let s_ok = s match {
                "flurry" => true,
                _ => false,
            };
            f_ok and s_ok
        }

        fn run() -> bool {
            route_with_guard_and_remap(.left(20, 3)) == 2003
                and route_with_guard_and_remap(.right(3, 20)) == 2003
                and route_with_guard_and_remap(.left(5, 3)) == 8
                and route_with_guard_and_remap(.right(2, 9)) == 7
                and route_with_guard_and_remap(.idle) == -1
                and inner_or_guard_fallback(.left(8, 3)) == 11
                and inner_or_guard_fallback(.left(2, 9)) == -99
                and float_and_str_fallback(1.5, "flurry")
                and (Channel.left(4, 5) match {
                    .positive_sum(sum, true) => sum == 9,
                    _ => false,
                })
        }
    "#;

    verify_interpret_and_native(source);
}

#[test]
fn dag_affine_drop_partial_moves_as_whole_and_or_cleanup_events() {
    let source = r#"
        fn mark_guard_hit() {}
        fn mark_or_move() {}
        fn mark_err_move() {}
        fn mark_as_whole() {}

        fn drop_1() {}
        fn drop_2() {}
        fn drop_3() {}
        fn drop_4() {}
        fn drop_5() {}
        fn drop_6() {}
        fn drop_7() {}
        fn drop_8() {}
        fn drop_9() {}
        fn drop_10() {}

        struct Token { id: u32 }

        impl Drop for Token {
            fn drop(*self) {
                let id = self.id.*;
                if id == 1 {
                    drop_1();
                } else if id == 2 {
                    drop_2();
                } else if id == 3 {
                    drop_3();
                } else if id == 4 {
                    drop_4();
                } else if id == 5 {
                    drop_5();
                } else if id == 6 {
                    drop_6();
                } else if id == 7 {
                    drop_7();
                } else if id == 8 {
                    drop_8();
                } else if id == 9 {
                    drop_9();
                } else if id == 10 {
                    drop_10();
                }
            }
        }

        fn consume(t: Token) -> u32 {
            t.id
        }

        enum Boxed {
            left(bool, Token, Token),
            right(bool, Token, Token),
        }

        struct Wrapper {
            ready: bool,
            tok: Token,
        }

        struct BadErr {
            first: Token,
            second: Token,
        }

        fn test_guard_fallback_and_or_move() -> bool {
            let b = Boxed.left(true, Token { id: 1 }, Token { id: 2 });
            let moved_id = b match {
                (.left(true, t, _) or .right(true, _, t)) if t.id > 500 => {
                    mark_guard_hit();
                    consume(t)
                },
                (.left(true, t, _) or .right(true, _, t)) => {
                    mark_or_move();
                    consume(t)
                },
                _ => 0,
            };
            moved_id == 1
        }

        fn test_as_whole_copy_overlap(flag: bool) -> bool {
            let w = Wrapper { ready: flag, tok: Token { id: 3 } };
            let chosen = w match {
                { ready: true } as whole => {
                    mark_as_whole();
                    whole
                },
                whole => whole,
            };
            chosen.tok.id == 3
        }

        fn test_error_payload_move(fail: bool) -> bool {
            let res: !BadErr Token = if fail {
                error BadErr { first: Token { id: 4 }, second: Token { id: 5 } }
            } else {
                Token { id: 6 }
            };
            res match {
                ok! => ok.id == 6,
                error BadErr { first: _, second: kept } => {
                    mark_err_move();
                    consume(kept) == 5
                },
            }
        }

        fn run() -> bool {
            test_guard_fallback_and_or_move()
                and test_as_whole_copy_overlap(true)
                and test_error_payload_move(true)
        }
    "#;

    let expected = [
        // test_guard_fallback_and_or_move: guard fails on (1,2), second arm moves 1 (drops in consume), drops residual 2
        "mark_or_move",
        "drop_1",
        "drop_2",
        // test_as_whole_copy_overlap: returns `whole` with Token { id: 3 }, then dropped at test_as_whole_copy_overlap end
        "mark_as_whole",
        "drop_3",
        // test_error_payload_move: moves `kept` (5 -> drops in consume), drops residual `first` (4)
        "mark_err_move",
        "drop_5",
        "drop_4",
    ];

    let (package, prepared, ir) = prepare_host(source);
    let mut vm = comptime_interpretor::Interpreter::from_iir(
        prepared.graph(),
        comptime_interpretor::Budget {
            steps: 10_000_000,
            depth: 128,
        },
    )
    .unwrap()
    .with_cleanup_schedule(prepared.schedule())
    .unwrap();
    assert_eq!(
        vm.invoke_instance(&prepared.graph().roots[0], vec![])
            .unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let observed: Vec<&str> = vm
        .events()
        .iter()
        .filter_map(|event| match event {
            comptime_interpretor::Event::EnterFunction(id) => {
                let name = package
                    .functions
                    .iter()
                    .find(|f| f.declaration == *id)
                    .map(|f| f.name.as_str())?;
                if name.starts_with("drop_") || name.starts_with("mark_") {
                    Some(name)
                } else {
                    None
                }
            }
            _ => None,
        })
        .collect();
    assert_eq!(observed, expected, "unexpected IIR cleanup trace");

    let drop_indices = iir_fn_indices(&prepared, 0, "drop");
    assert_eq!(drop_indices.len(), 1);
    let drop_idx = drop_indices[0];
    let drop_calls = ir
        .matches(&format!("call {{ {{}}, ptr }} @flurry_{drop_idx}(ptr "))
        .count()
        + ir.matches(&format!("call void @flurry_{drop_idx}_scalar(ptr "))
            .count();
    assert!(
        drop_calls >= 3,
        "expected Token::drop (@flurry_{drop_idx}) calls in LLVM IR, found {drop_calls}"
    );

    verify_interpret_and_native(source);
}
