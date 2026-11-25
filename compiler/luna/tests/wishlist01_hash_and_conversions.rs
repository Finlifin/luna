//! Integration tests for Milestone 2B & 2C of Wishlist 01:
//! - Streaming runtime `Hash` / `Hasher` / `BuildHasher` (`builtin.hash` & `builtin.prelude`)
//! - `std.hash.{DefaultHasher, RandomState}` and `HashMap` / `HashSet` with custom `BuildHasher`
//! - Non-`Copy` / non-`Clone` hashing in `Vec`, `String`, `HashMap`, `Slice`
//! - Comptime value-passing `meta.hash.{ComptimeHasher, ComptimeHash, ComptimeBuildHasher, DefaultComptimeHasher}`
//! - Runtime & comptime conversions: `From`, `Into`, `TryFrom`, `TryInto`, `AsRef`, `AsMut`,
//!   `InfallibleErr`, `ConversionErr`, and `ComptimeFrom`..`ComptimeTryInto`
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
            "luna-wl01-hash-conv-{}-{}",
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

#[test]
fn runtime_streaming_hash_std_hash_and_custom_build_hasher_work() {
    let source = r#"
        use std.prelude.*;
        use std.collections.{HashMap, HashSet};
        use std.hash.{DefaultHasher, RandomState};
        use std.time.Duration;

        -- Custom streaming FNV-1a hasher implementing only `write` + `finish`
        -- (exercising Hasher.write_u8 and all write_u*/write_str default methods).
        struct CustomFnvHasher {
            state: u64,
        }

        impl Clone for CustomFnvHasher {
            fn clone(*self) -> Self {
                CustomFnvHasher { state: self.state.* }
            }
        }

        impl Copy for CustomFnvHasher {}

        impl Hasher for CustomFnvHasher {
            fn finish(*self) -> u64 {
                self.state.*
            }

            fn write(*self, bytes: Slice<u8>) -> void {
                let len = bytes.len();
                var i: usize = 0;
                while i < len {
                    let b = bytes.get(i) match {
                        v? => v,
                        null => break,
                    };
                    let x = builtin.math.bit_xor(self.state.*, b'bitvec.as(u64));
                    let w = x'bitvec.as(u128);
                    assumes w <= 18446744073709551615;
                    self.state.* = (w * 1099511628211)'bitvec[0..64].as(u64);
                    i = i + 1;
                }
            }
        }

        struct CustomFnvBuild {
            seed: u64,
        }

        impl Clone for CustomFnvBuild {
            fn clone(*self) -> Self {
                CustomFnvBuild { seed: self.seed.* }
            }
        }

        impl Copy for CustomFnvBuild {}

        impl BuildHasher for CustomFnvBuild {
            assoc HasherTy: Type = CustomFnvHasher;

            fn build_hasher(*self) -> CustomFnvHasher {
                CustomFnvHasher { state: self.seed.* }
            }
        }

        use std.mem.{Allocator, Layout, SystemAllocator};

        -- Affine non-Copy, non-Clone key proving HashMap & Hash borrow without Clone.
        struct AffineKey {
            tag: u32,
            drops_addr: usize,
        }

        impl Drop for AffineKey {
            fn drop(*self) {
                unsafe {
                    let count = builtin.mem.raw_read<u32>(self.drops_addr.*);
                    assumes count < 1000000;
                    builtin.mem.raw_write<u32>(self.drops_addr.*, count + 1);
                }
            }
        }

        impl Eq for AffineKey {
            fn eq(*self, other: *Self) -> bool {
                self.tag.* == other.tag.*
            }
        }

        impl Hash for AffineKey {
            fn hash(*self, state: *H) where H: Type, requires H :- Hasher {
                self.tag.as(Hash).hash(state);
            }
        }

        fn sip_str_pair(a: str, b: str) -> u64 {
            var h = DefaultHasher.new();
            a.ref.as(Hash).hash(h.ref);
            b.ref.as(Hash).hash(h.ref);
            h.ref.finish()
        }

        fn run() -> bool {
            -- 1. SipHash-1-3 DefaultHasher & RandomState prefix-freedom and String/str parity.
            let pair1 = sip_str_pair("ab", "c");
            let pair2 = sip_str_pair("a", "bc");
            if pair1 == pair2 {
                return false;
            }

            let rs = RandomState.new();
            let owned = String.from_str("flurry-streaming-hash") match {
                s! => s,
                error _ => return false,
            };
            let lit: str = "flurry-streaming-hash";
            if rs.ref.hash_one(owned.ref) != rs.ref.hash_one(lit.ref) {
                return false;
            }

            -- 2. Custom BuildHasher exercising default Hasher.write_u8 -> write(Slice<u8>).
            let custom_builder = CustomFnvBuild { seed: 14695981039346656037 };
            let custom_h1 = custom_builder.ref.hash_one(owned.ref);
            let custom_h2 = custom_builder.ref.hash_one(lit.ref);
            if custom_h1 != custom_h2 {
                return false;
            }

            -- 3. Vec<String> and Duration streaming hashes work on non-Copy elements.
            var vec = Vec<String>.new();
            vec.ref.push(String.from_str("one") match { s! => s, error _ => return false }) match {
                _! => {},
                error _ => return false,
            };
            vec.ref.push(String.from_str("two") match { s! => s, error _ => return false }) match {
                _! => {},
                error _ => return false,
            };
            let vec_hash = rs.ref.hash_one(vec.ref);
            let dur = Duration.from_millis(1250);
            let zero_dur = std.time.ZERO;
            if vec_hash == 0 or rs.ref.hash_one(dur.ref) == rs.ref.hash_one(zero_dur.ref) {
                return false;
            }

            -- 4. HashMap & HashSet with custom BuildHasher and non-Copy, non-Clone AffineKey.
            let sys_alloc = SystemAllocator {};
            let counter_layout = Layout.of<u32>() match {
                v? => v,
                null => return false,
            };
            let counter_block = sys_alloc.ref.as(Allocator).allocate(counter_layout) match {
                b! => b,
                error _ => return false,
            };
            let drops_addr = counter_block.address();
            unsafe {
                builtin.mem.raw_write<u32>(drops_addr, 0);
            }
            let map_ok = do {
                var map = HashMap<AffineKey, u64, .S = CustomFnvBuild>.with_hasher(custom_builder);
                let _ = map.ref.insert(AffineKey { tag: 11, drops_addr: drops_addr }, 110) match {
                    v! => v,
                    error _ => return false,
                };
                let _ = map.ref.insert(AffineKey { tag: 22, drops_addr: drops_addr }, 220) match {
                    v! => v,
                    error _ => return false,
                };
                let probe = AffineKey { tag: 11, drops_addr: drops_addr };
                let got_ref = map.ref.get_ref(probe.ref) match {
                    v? => v == 110,
                    null => false,
                };
                let has_ref = map.ref.contains_key_ref(probe.ref);
                let builder_seed_ok = map.ref.hasher().seed.* == 14695981039346656037;
                got_ref and has_ref and builder_seed_ok
            };
            let drops = unsafe { builtin.mem.raw_read<u32>(drops_addr) };
            unsafe {
                sys_alloc.ref.as(Allocator).deallocate(counter_block);
            }
            if not map_ok or drops != 3 {
                return false;
            }

            var set = HashSet<u32, .S = CustomFnvBuild>.with_hasher(custom_builder);
            let ins_ok = set.ref.insert(42) match {
                b! => b,
                error _ => return false,
            };
            ins_ok and set.ref.contains(42)
        }
    "#;
    let case = NativeCase::new(source);
    let build = case.build();
    assert_success(&build);
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");
}

#[test]
fn comptime_streaming_hash_and_pointer_rejection_work() {
    let source = r#"
        use meta.hash.{ComptimeBuildHasher, ComptimeHash, ComptimeHasher, DefaultComptimeHasher};
        use meta.{List, Map, Set};

        struct CustomComptimeBuilder {
            salt: u64,
        }

        comptime impl ComptimeBuildHasher for CustomComptimeBuilder {
            assoc HasherTy: Type = DefaultComptimeHasher;

            pure comptime fn build_hasher(self) -> DefaultComptimeHasher {
                var h = DefaultComptimeHasher.new();
                -- Exercise ComptimeHasher.write_u64 via value-ref mutation without pointers.
                var seeded = h;
                seeded.write_u64(self.salt);
                seeded
            }
        }

        struct Token {
            id: Integer,
            name: str,
        }

        comptime impl ComptimeHash for Token {
            comptime fn hash(self, state: H) -> H
            where
                H: Type,
                requires H :- ComptimeHasher,
            {
                var acc = self.id.as(ComptimeHash).hash(state);
                self.name.as(ComptimeHash).hash(acc)
            }
        }

        comptime fn check_comptime_hash() -> bool {
            -- 1. Prefix-freedom with streaming state threading.
            var h1 = DefaultComptimeHasher.new();
            h1 = "ab".as(ComptimeHash).hash(h1);
            h1 = "c".as(ComptimeHash).hash(h1);
            var h2 = DefaultComptimeHasher.new();
            h2 = "a".as(ComptimeHash).hash(h2);
            h2 = "bc".as(ComptimeHash).hash(h2);
            if h1.finish() == h2.finish() {
                return false;
            }

            -- 2. Hash slice & collections & custom ComptimeBuildHasher.
            let t1 = Token { id: 7, name: "alpha" };
            let t2 = Token { id: 7, name: "beta" };
            let b1 = CustomComptimeBuilder { salt: 1 };
            let b2 = CustomComptimeBuilder { salt: 2 };
            if b1.hash_one(t1) == b1.hash_one(t2) or b1.hash_one(t1) == b2.hash_one(t1) {
                return false;
            }

            var xs: List<u32> = [1, 2, 3];
            var ys: List<u32> = [3, 2, 1];
            let list_diff = DefaultComptimeHasher.new().hash_one(xs)
                != DefaultComptimeHasher.new().hash_one(ys);

            var m1 = Map<str, Integer>.new();
            m1.insert("k1", 10);
            m1.insert("k2", 20);
            var m2 = Map<str, Integer>.new();
            m2.insert("k2", 20);
            m2.insert("k1", 10);
            let map_eq = DefaultComptimeHasher.new().hash_one(m1)
                == DefaultComptimeHasher.new().hash_one(m2);

            let s1 = Set<Integer>.from_list([1, 2, 3]);
            let s2 = Set<Integer>.from_list([3, 1, 2]);
            let set_eq = DefaultComptimeHasher.new().hash_one(s1)
                == DefaultComptimeHasher.new().hash_one(s2);

            list_diff and map_eq and set_eq
        }

        const CT_HASH_OK: bool = check_comptime_hash();

        fn run() -> bool {
            CT_HASH_OK
        }
    "#;
    assert_eq!(
        support::invoke(source, "check_comptime_hash").unwrap(),
        Value::Bool(true)
    );
    assert_eq!(support::run(source), Value::Bool(true));

    let bad_comptime_ptr = r#"
        use meta.hash.DefaultComptimeHasher;
        comptime fn bad() -> u64 {
            var h = DefaultComptimeHasher.new();
            h.ref.write_u64(1);
            h.finish()
        }
    "#;
    assert!(
        support::compile(bad_comptime_ptr).is_err(),
        "comptime `.ref` on DefaultComptimeHasher must be rejected"
    );
}

#[test]
fn runtime_and_comptime_conversions_with_error_lines_and_as_ref_mut_work() {
    let source = r#"
        use std.prelude.*;
        use builtin.convert.{AsMut, AsRef, ConversionErr, From, InfallibleErr, Into, TryFrom, TryInto};
        use meta.convert.{ComptimeFrom, ComptimeInto, ComptimeTryFrom, ComptimeTryInto};

        struct Port {
            raw: u16,
        }

        impl Eq for Port {
            fn eq(*self, other: *Self) -> bool {
                self.raw.* == other.raw.*
            }
        }

        impl From<u16> for Port {
            fn from(source: u16) -> Self {
                Port { raw: source }
            }
        }

        impl AsRef<u16> for Port {
            fn as_ref(*self) -> *u16 {
                self.raw
            }
        }

        impl AsMut<u16> for Port {
            fn as_mut(*self) -> *u16 {
                self.raw
            }
        }

        enum ParseErr {
            Empty,
            NonZeroByte,
        }

        struct ZeroMarker {
            byte: u8,
        }

        impl TryFrom<str> for ZeroMarker {
            assoc Err: Type = ParseErr;

            fn try_from(source: str) -> !ParseErr Self {
                let len = source.byte_length();
                if len == 0 {
                    return error ParseErr.Empty;
                }
                let first = source.byte(0) match {
                    b? => b,
                    null => return error ParseErr.Empty,
                };
                if first != 48 {
                    return error ParseErr.NonZeroByte;
                }
                ZeroMarker { byte: first }
            }
        }

        fn narrow_u16_to_u8(x: u16) -> !ConversionErr u8 {
            x.as(TryInto<u8>).try_into()
        }

        comptime fn check_comptime_conversions() -> bool {
            let x200: u8 = 200;
            let widened: u32 = x200.as(ComptimeInto<u32>).into();
            let reflexive: u16 = u16.as(ComptimeFrom<u16>).from(512);
            let x12: u8 = 12;
            let infallible_ok = x12.as(ComptimeTryInto<u32>).try_into() match {
                v! => v == 12,
                error (_: InfallibleErr) => false,
            };
            let neg_one: i16 = -1;
            let overflow_ok = neg_one.as(ComptimeTryInto<u8>).try_into() match {
                _! => false,
                error err => err matches .Underflow,
            };
            let over_u16: u16 = 300;
            let underflow_ok = over_u16.as(ComptimeTryInto<u8>).try_into() match {
                _! => false,
                error err => err matches .Overflow,
            };
            let char_oob = char.as(ComptimeTryFrom<u32>).try_from(55296) match {
                _! => false,
                error err => err matches .OutOfRange,
            };
            widened == 200 and reflexive == 512 and infallible_ok
                and overflow_ok and underflow_ok and char_oob
        }

        const CT_CONV_OK: bool = check_comptime_conversions();

        fn bump_port(target: *T) where T: Type, requires T :- AsMut<u16> {
            let slot = target.as(AsMut<u16>).as_mut();
            if slot.* < 65535 {
                slot.* = slot.* + 1;
            }
        }

        fn read_port(target: *T) -> u16 where T: Type, requires T :- AsRef<u16> {
            target.as(AsRef<u16>).as_ref().*
        }

        fn run() -> bool {
            -- 1. Blanket From -> Into -> TryFrom (!InfallibleErr) -> TryInto (!InfallibleErr).
            let p1 = Port.as(From<u16>).from(8080);
            let port_num: u16 = 8080;
            let p2: Port = port_num.as(Into<Port>).into();
            if p1.ref.as(Eq).ne(p2.ref) {
                return false;
            }
            let https: u16 = 443;
            let p3_ok = https.as(TryInto<Port>).try_into() match {
                port! => port.raw == 443,
                error (_: InfallibleErr) => false,
            };
            if not p3_ok {
                return false;
            }

            -- 2. Lossless widening & ConversionErr variants with `!` propagation.
            let u8_100: u8 = 100;
            let w64: u64 = u8_100.as(Into<u64>).into();
            let b_byte: u8 = true.as(Into<u8>).into();
            let c_code: u32 = 'A'.as(Into<u32>).into();
            if w64 != 100 or b_byte != 1 or c_code != 65 {
                return false;
            }
            let ok_narrow = narrow_u16_to_u8(250) match {
                v! => v == 250,
                error _ => false,
            };
            let over_narrow = narrow_u16_to_u8(300) match {
                _! => false,
                error err => err matches .Overflow,
            };
            let neg_five: i16 = -5;
            let under_narrow = neg_five.as(TryInto<u8>).try_into() match {
                _! => false,
                error err => err matches .Underflow,
            };
            let bad_code: u32 = 56000;
            let bad_char = bad_code.as(TryInto<char>).try_into() match {
                _! => false,
                error err => err matches .OutOfRange,
            };
            if not ok_narrow or not over_narrow or not under_narrow or not bad_char {
                return false;
            }

            -- 3. Custom TryFrom with custom PascalCaseErr.
            let zm_ok = "0x".as(TryInto<ZeroMarker>).try_into() match {
                z! => z.byte == 48,
                error _ => false,
            };
            let zm_empty = "".as(TryInto<ZeroMarker>).try_into() match {
                _! => false,
                error err => err matches .Empty,
            };
            let zm_bad = "1x".as(TryInto<ZeroMarker>).try_into() match {
                _! => false,
                error err => err matches .NonZeroByte,
            };
            if not zm_ok or not zm_empty or not zm_bad {
                return false;
            }

            -- 4. AsRef & AsMut in-place mutation + String AsRef<Vec<u8>>.
            var port = Port { raw: 80 };
            bump_port(port.ref);
            if read_port(port.ref) != 81 {
                return false;
            }
            let x: u16 = 7;
            if x.ref.as(AsRef<u16>).as_ref().* != 7 {
                return false;
            }
            let s = String.from_str("hi") match {
                v! => v,
                error _ => return false,
            };
            let bytes_len = s.ref.as(AsRef<Vec<u8>>).as_ref().len();

            CT_CONV_OK and bytes_len == 2
        }
    "#;
    let case = NativeCase::new(source);
    let build = case.build();
    assert_success(&build);
    let output = case.run();
    assert_success(&output);
    assert_eq!(output.stdout, b"(bool true)\n");
}
