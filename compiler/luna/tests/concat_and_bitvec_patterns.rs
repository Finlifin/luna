//! End-to-end tests for `++` concat patterns (`ConcatPatternView`, `str`, `String`,
//! `Slice<T>`, `meta.List<T>`), orthogonal `BitVec<N>` splitting, borrow lifetimes,
//! and ambiguous split rejection.
#[path = "support/sysroot_execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::Value;
use std::fs;
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct NativeCase(PathBuf);

impl NativeCase {
    fn new(source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "luna-concat-bitvec-{}-{}",
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

#[test]
fn str_prefix_suffix_and_chained_concat_patterns_run_natively_and_at_comptime() {
    let source = r#"
use std.prelude.*;

fn route_id(url: str) -> u64 {
    url match {
        "/api/" ++ path ++ ".json" => path.byte_length(),
        "/raw/" ++ rest => {
            let n = rest.byte_length();
            if n <= 1000000 { 100 + n } else { 0 }
        },
        name ++ ".txt" => {
            let n = name.byte_length();
            if n <= 1000000 { 200 + n } else { 0 }
        },
        _ => 0,
    }
}

comptime fn ct_route_id(url: str) -> u64 {
    url match {
        "/api/" ++ path ++ ".json" => path.byte_length(),
        "/raw/" ++ rest => {
            let n = rest.byte_length();
            if n <= 1000000 { 100 + n } else { 0 }
        },
        name ++ ".txt" => {
            let n = name.byte_length();
            if n <= 1000000 { 200 + n } else { 0 }
        },
        _ => 0,
    }
}

const CT_OK: bool = ct_route_id("/api/users.json") == 5
    and ct_route_id("/raw/ab") == 102
    and ct_route_id("notes.txt") == 205
    and ct_route_id("none") == 0;

fn run() -> bool {
    let r1 = route_id("/api/users.json");
    let r2 = route_id("/raw/ab");
    let r3 = route_id("notes.txt");
    let r4 = route_id("other");
    r1 == 5 and r2 == 102 and r3 == 205 and r4 == 0 and CT_OK
}
"#;
    let case = NativeCase::new(source);
    let build = case.build();
    assert!(
        build.status.success(),
        "build failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    let out = case.run();
    assert!(out.status.success());
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn string_concat_pattern_borrows_zero_copy_without_consuming_and_rejects_escapes() {
    let source = r#"
use std.prelude.*;

fn run() -> bool {
    let s = String.from_str("/api/items.json") match {
        v! => v,
        error _ => return false,
    };
    let extracted_len: u64 = s.ref match {
        "/v2/" ++ r => 1000,
        "/api/" ++ path ++ ".json" => path.byte_length(),
        _ => 999,
    };
    -- `s` was borrowed via `s.ref`, not consumed, so it remains usable after `match`.
    let still_len = s.len();
    let local_borrow = s.ref match {
        "/api/" ++ rest => rest,
        _ => "",
    };
    let same = local_borrow == "items.json";
    extracted_len == 5 and still_len == 15 and same
}
"#;
    let case = NativeCase::new(source);
    let build = case.build();
    assert!(
        build.status.success(),
        "build failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    let out = case.run();
    assert!(out.status.success());
    assert_eq!(out.stdout, b"(bool true)\n");

    // Escaping borrowed str from local String out of function must be rejected.
    let escape_fn = r#"
use std.prelude.*;

fn bad_return(s: String) -> str {
    s.ref match {
        "/api/" ++ rest => rest,
        _ => "",
    }
}

fn run() -> u64 {
    let s = String.from_str("/api/secret.json") match {
        v! => v,
        error _ => return 0,
    };
    bad_return(s).byte_length()
}
"#;
    let out_fn = NativeCase::new(escape_fn).build();
    assert!(!out_fn.status.success(), "expected escape build failure");
    let err_fn = String::from_utf8_lossy(&out_fn.stderr);
    assert!(
        err_fn.contains("backing-region")
            || err_fn.contains("local")
            || err_fn.contains("descriptor")
            || err_fn.contains("escapes")
            || err_fn.contains("text"),
        "unexpected error for function return escape: {err_fn}"
    );

    // Escaping borrowed str out of block-scoped String must be rejected.
    let escape_block = r#"
use std.prelude.*;

fn run() -> u64 {
    let r = do {
        let s = String.from_str("/api/x") match {
            v! => v,
            error _ => return 0,
        };
        s.ref match {
            "/api/" ++ rest => rest,
            _ => "",
        }
    };
    r.byte_length()
}
"#;
    let out_block = NativeCase::new(escape_block).build();
    assert!(!out_block.status.success(), "expected block escape failure");
    let err_block = String::from_utf8_lossy(&out_block.stderr);
    assert!(
        err_block.contains("escapes")
            || err_block.contains("local")
            || err_block.contains("live")
            || err_block.contains("descriptor")
            || err_block.contains("text"),
        "unexpected error for block escape: {err_block}"
    );

    // Escaping borrowed str from temporary rvalue String out of match must be rejected.
    let escape_temp = r#"
use std.prelude.*;

fn make_s() -> String {
    String.from_str("/api/x") match {
        v! => v,
        error _ => String.new(),
    }
}

fn run() -> u64 {
    let r = make_s() match {
        "/api/" ++ rest => rest,
        _ => "",
    };
    r.byte_length()
}
"#;
    let out_temp = NativeCase::new(escape_temp).build();
    assert!(!out_temp.status.success(), "expected temp escape failure");
    let err_temp = String::from_utf8_lossy(&out_temp.stderr);
    assert!(
        err_temp.contains("escapes")
            || err_temp.contains("local")
            || err_temp.contains("live")
            || err_temp.contains("descriptor")
            || err_temp.contains("backing-region")
            || err_temp.contains("text"),
        "unexpected error for temp escape: {err_temp}"
    );
}

#[test]
fn slice_concat_patterns_run_natively() {
    let source = r#"
use std.prelude.*;

fn sum_rest(s: Slice<i64>, pfx: Slice<i64>, sfx: Slice<i64>) -> i64 {
    s match {
        < pfx > ++ rest => {
            var acc: i64 = 0;
            var i: usize = 0;
            while i < rest.len() and i < 1000000 {
                let elem = rest.get(i) match { v? => v, null => 0 };
                if acc > -100000 and acc < 100000 and elem > -100000 and elem < 100000 {
                    acc = acc + elem;
                }
                i = i + 1;
            }
            acc
        },
        head ++ < sfx > => if head.len() == 2 { 2 } else { 0 },
        _ => -1,
    }
}

fn run() -> bool {
    var data: Vec<i64> = Vec<i64>.new();
    let _ = data.ref.push(1);
    let _ = data.ref.push(2);
    let _ = data.ref.push(30);
    let _ = data.ref.push(40);

    var pfx: Vec<i64> = Vec<i64>.new();
    let _ = pfx.ref.push(1);
    let _ = pfx.ref.push(2);

    var sfx: Vec<i64> = Vec<i64>.new();
    let _ = sfx.ref.push(30);
    let _ = sfx.ref.push(40);

    var miss: Vec<i64> = Vec<i64>.new();
    let _ = miss.ref.push(99);

    let a = sum_rest(data.ref.as_slice(), pfx.ref.as_slice(), sfx.ref.as_slice());
    let b = sum_rest(data.ref.as_slice(), miss.ref.as_slice(), sfx.ref.as_slice());
    a == 70 and b == 2
}
"#;
    let case = NativeCase::new(source);
    let build = case.build();
    assert!(
        build.status.success(),
        "build failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    let out = case.run();
    assert!(out.status.success());
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn meta_list_comptime_concat_patterns_split_prefix_suffix_and_chained() {
    let source = r#"
use meta.List;

comptime fn inspect(xs: List<u8>) -> Integer {
    xs match {
        [1, 2] ++ mid ++ [5] => mid.length() + 100,
        [1, 2] ++ rest => rest.length() + 200,
        head ++ [9] => head.length() + 300,
        _ => 0,
    }
}

comptime fn entry() -> bool {
    inspect([1, 2, 3, 4, 5]) == 102
        and inspect([1, 2, 7]) == 201
        and inspect([8, 9]) == 301
        and inspect([4, 5]) == 0
}
"#;
    assert_eq!(support::invoke(source, "entry"), Ok(Value::Bool(true)));
}

#[test]
fn bitvec_concat_patterns_split_msb_first_and_infer_complement_width() {
    let source = r#"
fn split32(word: BitVec<32>) -> i64 {
    word match {
        (hi: BitVec<8>) ++ lo => {
            let h: u8 = hi.as(u8);
            let l: u32 = lo.as(u32);
            if h == 0x12 and l == 0x345678 { 1 } else { 0 }
        },
    }
}

fn split_right_typed(word: BitVec<32>) -> i64 {
    word match {
        hi ++ (lo: BitVec<8>) => {
            let h: u32 = hi.as(u32);
            let l: u8 = lo.as(u8);
            if h == 0x123456 and l == 0x78 { 1 } else { 0 }
        },
    }
}

fn split_magic64(packet: BitVec<64>) -> i64 {
    packet match {
        0xcafebabe ++ payload => {
            let p: u32 = payload.as(u32);
            if p == 0xdeadbeef { 1 } else { 0 }
        },
        _ => 0,
    }
}

fn run() -> bool {
    let w_val: u32 = 0x12345678;
    let p_val: u64 = 0xcafebabedeadbeef;
    let w: BitVec<32> = w_val'bitvec;
    let p: BitVec<64> = p_val'bitvec;
    split32(w) == 1 and split_right_typed(w) == 1 and split_magic64(p) == 1
}
"#;
    let case = NativeCase::new(source);
    let build = case.build();
    assert!(
        build.status.success(),
        "build failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    let out = case.run();
    assert!(out.status.success());
    assert_eq!(out.stdout, b"(bool true)\n");
}

#[test]
fn ambiguous_concat_patterns_are_rejected_for_sequences_and_unannotated_bitvecs() {
    let seq_ambig = r#"
fn bad(s: str) -> str {
    s match {
        left ++ right => left,
        _ => "",
    }
}
"#;
    let err_seq = support::compile(seq_ambig).unwrap_err();
    assert!(
        err_seq.contains(
            "ambiguous concat pattern: cannot split sequence between two variable-length patterns without a fixed-width BitVec annotation"
        ),
        "unexpected sequence ambiguity error: {err_seq}"
    );

    let bv_ambig = r#"
fn bad_bits(b: BitVec<32>) -> i64 {
    b match {
        a ++ b => 1,
    }
}
"#;
    let err_bv = support::compile(bv_ambig).unwrap_err();
    assert!(
        err_bv.contains("ambiguous concat pattern"),
        "unexpected BitVec ambiguity error: {err_bv}"
    );
}
