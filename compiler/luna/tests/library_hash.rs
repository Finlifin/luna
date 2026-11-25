#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;

use comptime_interpretor::Value;

fn fnv1a_bytes(bytes: &[u8]) -> u64 {
    bytes.iter().fold(14695981039346656037u64, |state, byte| {
        (state ^ u64::from(*byte)).wrapping_mul(1099511628211)
    })
}

fn hash_host_intrinsics() -> Vec<(nir::intrinsics::Operation, String)> {
    use nir::intrinsics::Operation as Op;
    let mut intrinsics: Vec<(Op, String)> = [
        (Op::MemRawRead, "input.mem_impl.raw.target.Memory.raw_read"),
        (
            Op::MemRawWrite,
            "input.mem_impl.raw.target.Memory.raw_write",
        ),
        (Op::MemCopy, "input.mem_impl.raw.target.Memory.copy"),
        (
            Op::MemCopyNonOverlapping,
            "input.mem_impl.raw.target.Memory.copy_nonoverlapping",
        ),
        (
            Op::MemSetBytes,
            "input.mem_impl.raw.target.Memory.set_bytes",
        ),
        (
            Op::MemCompareBytes,
            "input.mem_impl.raw.target.Memory.compare_bytes",
        ),
        (
            Op::MemBorrowStr,
            "input.mem_impl.raw.target.Memory.borrow_str",
        ),
        (Op::MemPtrAddr, "input.mem_impl.raw.target.Memory.ptr_addr"),
    ]
    .into_iter()
    .map(|(op, sel)| (op, sel.to_owned()))
    .collect();
    for scalar in [
        "fma",
        "sqrt",
        "copysign",
        "floor",
        "ceil",
        "trunc",
        "roundeven",
        "round",
        "fabs",
        "sin",
        "cos",
        "exp",
        "exp2",
        "log",
        "log2",
        "log10",
        "pow",
        "powi",
        "ldexp",
        "minimum",
        "maximum",
        "bitreverse",
        "ctpop",
        "ctlz",
        "cttz",
        "bswap",
        "fshl",
        "fshr",
        "minimumnum",
        "maximumnum",
        "isnan",
        "isinf",
        "isfinite",
        "isnormal",
        "issubnormal",
        "signbit",
        "add_sat",
        "sub_sat",
        "min",
        "max",
        "abs",
        "bit_and",
        "bit_or",
        "bit_xor",
    ] {
        intrinsics.push((
            Op::from_name(scalar).unwrap(),
            format!("input.numeric.math_intrinsics.target.Scalar.{scalar}"),
        ));
    }
    intrinsics
}

fn hash_selectors<'a>(
    intrinsics: &'a [(nir::intrinsics::Operation, String)],
) -> luna::protocols::type_constructors::Selectors<'a> {
    luna::protocols::type_constructors::Selectors {
        tuple: Some("input.types.tuple.Tuple"),
        meta_list: Some("input.meta.List"),
        copy: Some("input.protocols.resources.Copy"),
        drop: Some("input.protocols.resources.Drop"),
        splice: Some("input.protocols.staging.Splice"),
        literal_into: Some("input.protocols.conversion.LiteralInto"),
        host_intrinsics: intrinsics,
        ..Default::default()
    }
}

fn compile_hash_result(
    src: &str,
    protocols: &elaboration::program::Protocols,
) -> Result<nir::program::Package, String> {
    let intrinsics = hash_host_intrinsics();
    support::compile_with_source_bindings_for_target(
        src,
        protocols,
        hash_selectors(&intrinsics),
        Some(nir::target::TargetData::new(64).unwrap()),
    )
}

fn compile_hash(src: &str) -> nir::program::Package {
    compile_hash_result(src, &support::meta_fixture_protocols()).unwrap()
}

fn compile_comptime_hash(src: &str) -> nir::program::Package {
    support::compile_with_source_bindings_for_target(
        src,
        &support::meta_fixture_protocols(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.meta.List"),
            copy: Some("input.protocols.resources.Copy"),
            literal_into: Some("input.protocols.conversion.LiteralInto"),
            ..Default::default()
        },
        None,
    )
    .unwrap()
}

fn compile_hash_for_target(
    src: &str,
    target: Option<nir::target::TargetData>,
) -> Result<nir::program::Package, String> {
    let intrinsics = hash_host_intrinsics();
    support::compile_with_source_bindings_for_target(
        src,
        &support::meta_fixture_protocols(),
        hash_selectors(&intrinsics),
        target,
    )
}

fn source(body: &str) -> String {
    format!(
        r#"
mod primitives {{ {primitives} }}
use primitives.*;
mod bindings {{ {bindings} }}
mod numeric {{
    pub mod comparison {{ {comparison} }}
    pub mod literals {{ {literals} }}
    pub mod math_intrinsics {{ {math_intrinsics} }}
}}
mod comparison {{ pub use numeric.comparison.*; }}
mod text_impl {{
    pub mod bytes {{ {text_bytes} }}
}}
mod mem_impl {{
    pub mod raw {{ {mem_raw} }}
    pub mod slice {{ {mem_slice} }}
}}
mod protocols {{
    pub mod resources {{ {resources} }}
    pub mod native_resources {{ {native_resources} }}
    pub mod staging {{ {staging} }}
    pub mod formatting {{
        pub mod writer {{ {writer} }}
        pub mod debug {{
            use protocols.formatting.writer.ByteWriter;
            pub trait Debug {{
                fn write_debug(*self, output: *W) -> !W.as(ByteWriter).Err void where W :- ByteWriter;
            }}
        }}
    }}
    pub mod patterns {{ {patterns} }}
    pub mod hash {{ {hash} }}
    pub mod conversion {{ {conversion} }}
}}
mod resources {{ pub use protocols.resources.*; }}
mod hash {{ pub use protocols.hash.*; }}
mod conversion {{ pub use protocols.conversion.*; }}
mod traits {{ {traits} }}
mod types {{
    pub mod tuple {{ {tuple} }}
    pub mod hashing {{ {types_hashing} }}
}}
pub use traits.*;
pub use mem_impl.slice.Slice;
pub mod builtin {{
    pub use primitives.*;
    pub use comparison.{{Order, TotalFloat, TotalOrd}};
    pub use traits.{{ConversionErr, InfallibleErr}};
    pub mod compiler {{ pub use bindings.*; }}
}}
mod meta {{
    use builtin;
    pub const List: pure comptime fn<T: builtin.Type> -> builtin.Type = .List'builtin;
    mod types {{
        pub mod bindings {{ {meta_types_bindings} }}
    }}
    mod bindings {{ {meta_bindings} }}
    mod text_impl {{ pub mod bindings {{ {meta_text_bindings} }} }}
    mod numeric {{
        pub mod bindings {{ {meta_num_bindings} }}
        pub mod equality {{ {meta_eq} }}
        pub mod ordering {{ {meta_ord} }}
        pub mod conversions {{ {meta_conv} }}
    }}
    mod data {{
        pub mod integer {{ {meta_integer} }}
        pub mod list {{ {meta_list} }}
        pub mod hashing {{ {meta_hash} }}
    }}
    mod integer {{ pub use data.integer.*; }}
    pub use data.hashing.{{ComptimeBuildHasher, ComptimeHash, ComptimeHasher, DefaultComptimeHasher}};
    pub use numeric.equality.ComptimeEq;
}}
pub use meta.{{ComptimeBuildHasher, ComptimeEq, ComptimeHash, ComptimeHasher, DefaultComptimeHasher}};

struct TestFnvHasher {{
    state: u64,
}}
impl Clone for TestFnvHasher {{
    fn clone(*self) -> Self {{ TestFnvHasher {{ state: self.state.* }} }}
}}
impl Copy for TestFnvHasher {{}}
impl Hasher for TestFnvHasher {{
    fn finish(*self) -> u64 {{ self.state.* }}
    fn write_u8(*self, value: u8) -> void {{
        let bits = self.state.*'bitvec;
        let input = value'bitvec.as(u64)'bitvec;
        let mixed = ((bits and not input) or (not bits and input)).as(u128);
        assumes mixed <= 18446744073709551615;
        let product = mixed * 1099511628211;
        self.state.* = product'bitvec[0..64].as(u64);
    }}
    fn write(*self, bytes: Slice<u8>) -> void {{
        let len = bytes.len();
        var i: usize = 0;
        while i < len {{
            let b = bytes.get(i) match {{
                v? => v,
                null => break,
            }};
            self.write_u8(b);
            i = i + 1;
        }}
    }}
}}
struct TestFnvBuild {{}}
impl TestFnvBuild {{
    fn hash_val(value: *T) -> u64 where T: Type, requires T :- Hash {{
        var h = TestFnvHasher {{ state: 14695981039346656037 }};
        value.as(Hash).hash(h.ref);
        h.ref.finish()
    }}
}}

{body}
"#,
        primitives = include_str!("../../../library/builtin/primitives.fl"),
        bindings = include_str!("../../../library/builtin/bindings.fl"),
        comparison = support::with_attrs(include_str!(
            "../../../library/builtin/numeric/comparison.fl"
        )),
        literals = include_str!("../../../library/builtin/numeric/literals.fl"),
        math_intrinsics = include_str!("../../../library/builtin/numeric/math_intrinsics.fl"),
        resources = include!("support/resources_source.rs"),
        native_resources = include_str!("../../../library/builtin/protocols/native_resources.fl"),
        staging = include_str!("../../../library/builtin/protocols/staging.fl"),
        text_bytes = include_str!("../../../library/builtin/text_impl/bytes.fl"),
        tuple = include_str!("../../../library/builtin/types/tuple.fl"),
        types_hashing =
            support::with_attrs(include_str!("../../../library/builtin/types/hashing.fl")),
        mem_raw = include_str!("../../../library/builtin/mem_impl/raw.fl"),
        mem_slice = support::with_attrs(include_str!("../../../library/builtin/mem_impl/slice.fl")),
        writer = include_str!("../../../library/builtin/protocols/formatting/writer.fl"),
        patterns = support::with_attrs(include_str!(
            "../../../library/builtin/protocols/patterns.fl"
        )),
        hash = support::with_attrs(include_str!("../../../library/builtin/protocols/hash.fl")),
        conversion = support::with_attrs(include_str!(
            "../../../library/builtin/protocols/conversion.fl"
        )),
        traits = support::with_attrs(include_str!("../../../library/builtin/protocols/traits.fl")),
        meta_types_bindings = include_str!("../../../library/meta/types/bindings.fl"),
        meta_bindings = include_str!("../../../library/meta/bindings.fl"),
        meta_text_bindings = include_str!("../../../library/meta/text_impl/bindings.fl"),
        meta_num_bindings = include_str!("../../../library/meta/numeric/bindings.fl"),
        meta_eq = include_str!("../../../library/meta/numeric/equality.fl"),
        meta_ord = include_str!("../../../library/meta/numeric/ordering.fl"),
        meta_conv = include_str!("../../../library/meta/numeric/conversions.fl"),
        meta_integer = include_str!("../../../library/meta/data/integer.fl"),
        meta_list = include_str!("../../../library/meta/data/list.fl"),
        meta_hash = include_str!("../../../library/meta/data/hashing.fl"),
    )
}

fn comptime_source(body: &str) -> String {
    format!(
        r#"
mod primitives {{ {primitives} }}
use primitives.*;
mod bindings {{ {bindings} }}
mod numeric {{
    pub mod comparison {{ {comparison} }}
}}
mod comparison {{ pub use numeric.comparison.*; }}
mod protocols {{
    pub mod resources {{ {resources} }}
    pub mod formatting {{
        pub mod writer {{
            use primitives.{{str, u8, void}};
            pub trait ByteWriter {{
                assoc Err: Type;
                fn write_byte(*self, byte: u8) -> !Err void;
                fn write_str(*self, text: str) -> !Err void;
            }}
        }}
        pub mod debug {{
            use protocols.formatting.writer.ByteWriter;
            pub trait Debug {{
                fn write_debug(*self, output: *W) -> !W.as(ByteWriter).Err void where W :- ByteWriter;
            }}
        }}
    }}
    pub mod conversion {{ {conversion} }}
}}
pub mod builtin {{
    pub use primitives.*;
    pub use comparison.{{Order, TotalFloat, TotalOrd}};
    pub use protocols.conversion.{{ConversionErr, InfallibleErr}};
    pub mod compiler {{ pub use bindings.*; }}
}}
mod meta {{
    use builtin;
    pub const List: pure comptime fn<T: builtin.Type> -> builtin.Type = .List'builtin;
    mod types {{
        pub mod bindings {{ {meta_types_bindings} }}
    }}
    mod bindings {{ {meta_bindings} }}
    mod text_impl {{ pub mod bindings {{ {meta_text_bindings} }} }}
    mod numeric {{
        pub mod bindings {{ {meta_num_bindings} }}
        pub mod equality {{ {meta_eq} }}
        pub mod ordering {{ {meta_ord} }}
        pub mod conversions {{ {meta_conv} }}
    }}
    mod data {{
        pub mod integer {{ {meta_integer} }}
        pub mod list {{ {meta_list} }}
        pub mod hashing {{ {meta_hash} }}
    }}
    mod integer {{ pub use data.integer.*; }}
    pub use data.hashing.{{ComptimeBuildHasher, ComptimeHash, ComptimeHasher, DefaultComptimeHasher}};
    pub use numeric.equality.ComptimeEq;
}}
pub use meta.{{ComptimeBuildHasher, ComptimeEq, ComptimeHash, ComptimeHasher, DefaultComptimeHasher}};

{body}
"#,
        primitives = include_str!("../../../library/builtin/primitives.fl"),
        bindings = include_str!("../../../library/builtin/bindings.fl"),
        comparison = support::with_attrs(include_str!(
            "../../../library/builtin/numeric/comparison.fl"
        )),
        resources = include!("support/resources_source.rs"),
        conversion = support::with_attrs(include_str!(
            "../../../library/builtin/protocols/conversion.fl"
        )),
        meta_types_bindings = include_str!("../../../library/meta/types/bindings.fl"),
        meta_bindings = include_str!("../../../library/meta/bindings.fl"),
        meta_text_bindings = include_str!("../../../library/meta/text_impl/bindings.fl"),
        meta_num_bindings = include_str!("../../../library/meta/numeric/bindings.fl"),
        meta_eq = include_str!("../../../library/meta/numeric/equality.fl"),
        meta_ord = include_str!("../../../library/meta/numeric/ordering.fl"),
        meta_conv = include_str!("../../../library/meta/numeric/conversions.fl"),
        meta_integer = include_str!("../../../library/meta/data/integer.fl"),
        meta_list = include_str!("../../../library/meta/data/list.fl"),
        meta_hash = include_str!("../../../library/meta/data/hashing.fl"),
    )
}

#[test]
fn scalar_hashes_agree_across_execution_phases_and_integer_boundaries() {
    let cases: [(&str, &str, Vec<u8>); 15] = [
        ("bool", "false", vec![0]),
        ("bool", "true", vec![1]),
        ("char", "'A'", 65u32.to_le_bytes().to_vec()),
        ("char", "'\u{d7ff}'", 0xd7ffu32.to_le_bytes().to_vec()),
        ("char", "'\u{e000}'", 0xe000u32.to_le_bytes().to_vec()),
        ("char", "'🦀'", 0x1f980u32.to_le_bytes().to_vec()),
        ("char", "'\u{10ffff}'", 0x10ffffu32.to_le_bytes().to_vec()),
        ("u8", "255", vec![255]),
        ("u16", "65535", 65535u16.to_le_bytes().to_vec()),
        ("u32", "4294967295", u32::MAX.to_le_bytes().to_vec()),
        (
            "u64",
            "18446744073709551615",
            u64::MAX.to_le_bytes().to_vec(),
        ),
        ("i8", "-128", i8::MIN.to_le_bytes().to_vec()),
        ("i16", "-32768", i16::MIN.to_le_bytes().to_vec()),
        ("i32", "-2147483648", i32::MIN.to_le_bytes().to_vec()),
        (
            "i64",
            "-9223372036854775808",
            i64::MIN.to_le_bytes().to_vec(),
        ),
    ];
    for (ty, literal, bytes) in cases.into_iter().chain([
        (
            "u128",
            "18446744073709551616",
            (1u128 << 64).to_le_bytes().to_vec(),
        ),
        (
            "u128",
            "340282366920938463463374607431768211455",
            u128::MAX.to_le_bytes().to_vec(),
        ),
        (
            "i128",
            "-170141183460469231731687303715884105728",
            i128::MIN.to_le_bytes().to_vec(),
        ),
        ("i128", "-1", (-1i128).to_le_bytes().to_vec()),
    ]) {
        let expected = Value::UnsignedInteger(fnv1a_bytes(&bytes).into());
        let package = compile_hash(&source(&format!(
            "fn run()->u64 {{let value:{ty}={literal};TestFnvBuild.hash_val(value.ref)}}\ncomptime fn staged()->u64 {{let value:{ty}={literal};DefaultComptimeHasher.new().hash_one(value)}}"
        )));
        assert_eq!(
            luna::invocation::invoke(&package, "staged", 1_000_000).unwrap(),
            expected,
            "{ty} {literal}"
        );
        assert_eq!(
            luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
            expected,
            "{ty} {literal}"
        );
    }
}

#[test]
fn wide_hash_collisions_still_require_equality() {
    use comptime_executor::{Context, KeyProtocol};
    let package = compile_comptime_hash(&comptime_source(
        "struct Collide { key: u8, tag: u8 }
         comptime impl ComptimeEq for Collide {
             comptime fn eq(self, other: Self) -> bool { self.key == other.key and self.tag == other.tag }
         }
         comptime impl ComptimeHash for Collide {
             comptime fn hash(self, state: H) -> H where H: Type, requires H :- ComptimeHasher {
                 self.key.as(ComptimeHash).hash(state)
             }
         }
         comptime fn make_low() -> Collide { Collide { key: 1, tag: 1 } }
         comptime fn make_high() -> Collide { Collide { key: 1, tag: 2 } }",
    ));
    let ty = package
        .functions
        .iter()
        .find(|f| f.name == "make_low")
        .unwrap()
        .signature
        .callable_signature();
    let nir::Term::Function { result, .. } = ty else {
        panic!("signature")
    };
    let mut items = intrinsic::LangItems::new();
    let bindings = [
        (intrinsic::LangItem::Eq, "ComptimeEq"),
        (intrinsic::LangItem::Hash, "ComptimeHash"),
    ]
    .map(|(item, name)| {
        (
            item,
            package
                .traits
                .iter()
                .find(|t| t.name == name)
                .unwrap()
                .declaration,
        )
    });
    items.bind_source_traits(&package, &bindings).unwrap();
    let mut protocol = luna::comptime_keys::Protocol::prepare(
        &package,
        &items,
        result,
        elaboration::ptc::Budget {
            steps: 1_000_000,
            ..Default::default()
        },
    )
    .unwrap();
    let low = luna::invocation::invoke(&package, "make_low", 1_000_000).unwrap();
    let high = luna::invocation::invoke(&package, "make_high", 1_000_000).unwrap();
    let mut context = Context::new(1_000_000);
    assert_eq!(
        protocol.hash(&low, &mut context).unwrap(),
        protocol.hash(&high, &mut context).unwrap()
    );
    assert!(!protocol.equivalent(&low, &high, &mut context).unwrap());
    assert!(protocol.equivalent(&high, &high, &mut context).unwrap());
}

#[test]
fn scalar_hashes_lower_to_host_code_without_comptime_dispatch() {
    let wide_expected = fnv1a_bytes(&(1u128 << 64).to_le_bytes());
    let signed_expected = fnv1a_bytes(&i128::MIN.to_le_bytes());
    let bool_expected = fnv1a_bytes(&[1]);
    let package = compile_hash(&source(&format!(
        "fn verify() {{
            let wide:u128=18446744073709551616;
            let signed:i128=-170141183460469231731687303715884105728;
            let wide_hash=TestFnvBuild.hash_val(wide.ref); assumes wide_hash == {wide_expected};
            let signed_hash=TestFnvBuild.hash_val(signed.ref); assumes signed_hash == {signed_expected};
            let t=true; let bool_hash=TestFnvBuild.hash_val(t.ref); assumes bool_hash == {bool_expected};
        }}"
    )));
    let protocol = |name: &str| {
        package
            .traits
            .iter()
            .find(|item| item.name == name)
            .unwrap()
            .declaration
    };
    let ir = luna::host::llvm_ir_with_protocols(
        &package,
        "verify",
        1_000_000,
        protocol("Copy"),
        protocol("Drop"),
    )
    .unwrap();
    for optimization in [
        luna::host::OptimizationLevel::O0,
        luna::host::OptimizationLevel::O3,
    ] {
        luna::host::Toolchain {
            optimization,
            ..Default::default()
        }
        .execute_test(&ir, std::time::Duration::from_secs(5))
        .unwrap();
    }
}

#[test]
fn character_bit_reads_do_not_authorize_reconstruction_or_writeback() {
    for body in [
        "let value:u32=65;value'bitvec.as(char)",
        "var value:char='A';value'bitvec[0..32]=55296;value",
    ] {
        let error = compile_hash_result(
            &source(&format!("fn invalid()->char {{{body}}}")),
            &support::meta_fixture_protocols(),
        )
        .unwrap_err();
        assert!(
            error.contains("representation") || error.contains("unrestricted"),
            "{error}"
        );
    }
    let package = compile_hash(&source(
        "fn read(value:char)->u64 {value'bitvec.as(u64)} fn verify() {let value=read('🦀');assumes value == 129408;}",
    ));
    let protocol = |name: &str| {
        package
            .traits
            .iter()
            .find(|item| item.name == name)
            .unwrap()
            .declaration
    };
    let ir = luna::host::llvm_ir_with_protocols(
        &package,
        "verify",
        1_000_000,
        protocol("Copy"),
        protocol("Drop"),
    )
    .unwrap();
    luna::host::Toolchain::default()
        .execute_test(&ir, std::time::Duration::from_secs(5))
        .unwrap();
}

#[test]
fn symbol_hashes_follow_utf8_content_in_both_phases_and_native_code() {
    for name in ["a", "foobar", "é"] {
        let mut bytes = name.as_bytes().to_vec();
        bytes.push(255);
        let expected = fnv1a_bytes(&bytes);
        let package = compile_hash(&source(&format!(
            "fn hash_sym(value:Symbol)->u64 {{TestFnvBuild.hash_val(value.ref)}}
             fn run()->u64 {{hash_sym(.{name})}}
             comptime fn staged()->u64 {{let value:Symbol=.{name};DefaultComptimeHasher.new().hash_one(value)}}
             fn verify() {{let value=run();assumes value == {expected};}}"
        )));
        assert_eq!(
            luna::invocation::invoke(&package, "staged", 1_000_000).unwrap(),
            Value::UnsignedInteger(expected.into()),
            "{name}"
        );
        assert_eq!(
            luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
            Value::UnsignedInteger(expected.into()),
            "{name}"
        );
        let protocol = |name: &str| {
            package
                .traits
                .iter()
                .find(|item| item.name == name)
                .unwrap()
                .declaration
        };
        let ir = luna::host::llvm_ir_with_protocols(
            &package,
            "verify",
            1_000_000,
            protocol("Copy"),
            protocol("Drop"),
        )
        .unwrap();
        for optimization in [
            luna::host::OptimizationLevel::O0,
            luna::host::OptimizationLevel::O3,
        ] {
            luna::host::Toolchain {
                optimization,
                ..Default::default()
            }
            .execute_test(&ir, std::time::Duration::from_secs(5))
            .unwrap();
        }
    }
}

#[test]
fn text_and_symbol_cache_keys_compare_content_and_recover_after_budget_exhaustion() {
    use comptime_executor::{Context, KeyProtocol};
    let package = compile_comptime_hash(&comptime_source(""));
    let mut items = intrinsic::LangItems::new();
    let bindings = [
        (intrinsic::LangItem::Eq, "ComptimeEq"),
        (intrinsic::LangItem::Hash, "ComptimeHash"),
    ]
    .map(|(item, name)| {
        (
            item,
            package
                .traits
                .iter()
                .find(|t| t.name == name)
                .unwrap()
                .declaration,
        )
    });
    items.bind_source_traits(&package, &bindings).unwrap();
    for primitive in [nir::Primitive::Symbol, nir::Primitive::Str] {
        let value = |text: String| {
            if primitive == nir::Primitive::Symbol {
                Value::Symbol(text.into())
            } else {
                Value::String(text.into())
            }
        };
        let mut protocol = luna::comptime_keys::Protocol::prepare(
            &package,
            &items,
            &nir::Term::Primitive(primitive),
            elaboration::ptc::Budget {
                steps: 1_000_000,
                ..Default::default()
            },
        )
        .unwrap();
        let first = value("é".repeat(128));
        let same = value("é".repeat(128));
        let different = value("e".repeat(128));
        assert!(protocol.hash(&first, &mut Context::new(1)).is_err());
        let mut context = Context::new(4_000_000);
        let hash = protocol.hash(&first, &mut context).unwrap();
        assert_eq!(hash, protocol.hash(&same, &mut context).unwrap());
        assert!(protocol.equivalent(&first, &same, &mut context).unwrap());
        assert!(
            !protocol
                .equivalent(&first, &different, &mut context)
                .unwrap()
        );
        assert_eq!(
            protocol.hash(&value("".to_owned()), &mut context).unwrap(),
            protocol.hash(&value(String::new()), &mut context).unwrap()
        );
    }
}

#[test]
fn target_sized_hashes_use_the_selected_target_width() {
    for width in (8u32..=128).step_by(8) {
        let mask = u128::MAX >> (128 - width);
        let sign = 1u128 << (width - 1);
        for (ty, literal, representation) in [
            ("usize", mask.to_string(), mask),
            ("isize", "-1".to_owned(), mask),
            ("isize", format!("-{sign}"), sign),
            ("isize", "0".to_owned(), 0),
        ] {
            let text = source(&format!(
                "fn run()->u64 {{let value:{ty}={literal};TestFnvBuild.hash_val(value.ref)}}
                 comptime fn staged()->u64 {{let value:{ty}={literal};DefaultComptimeHasher.new().hash_one(value)}}"
            ));
            let package =
                compile_hash_for_target(&text, Some(nir::target::TargetData::new(width).unwrap()))
                    .unwrap();
            let runtime_expected = if width <= 32 {
                fnv1a_bytes(&(representation as u32).to_le_bytes())
            } else if width <= 64 {
                fnv1a_bytes(&(representation as u64).to_le_bytes())
            } else {
                fnv1a_bytes(&representation.to_le_bytes())
            };
            let comptime_expected = fnv1a_bytes(&representation.to_le_bytes());
            assert_eq!(
                luna::invocation::invoke(&package, "staged", 1_000_000).unwrap(),
                Value::UnsignedInteger(comptime_expected.into()),
                "{ty} {width}"
            );
            assert_eq!(
                luna::execution::interpret(&package, "run", 1_000_000).unwrap(),
                Value::UnsignedInteger(runtime_expected.into()),
                "{ty} {width}"
            );
        }
    }
}

#[test]
fn target_sized_hashes_execute_on_the_native_target() {
    let target = luna::host::Toolchain::default().target_layout().unwrap();
    let mask = u128::MAX >> (128 - target.pointer_bits);
    let expected = if target.pointer_bits <= 32 {
        fnv1a_bytes(&(mask as u32).to_le_bytes())
    } else if target.pointer_bits <= 64 {
        fnv1a_bytes(&(mask as u64).to_le_bytes())
    } else {
        fnv1a_bytes(&mask.to_le_bytes())
    };
    let text = source(&format!(
        "fn verify() {{
            let signed:isize=-1;
            let unsigned:usize={mask};
            let first=TestFnvBuild.hash_val(signed.ref);
            let second=TestFnvBuild.hash_val(unsigned.ref);
            assumes first == {expected} and second == {expected};
        }}"
    ));
    let package = compile_hash_for_target(
        &text,
        Some(nir::target::TargetData::new(target.pointer_bits).unwrap()),
    )
    .unwrap();
    let protocol = |name: &str| {
        package
            .traits
            .iter()
            .find(|item| item.name == name)
            .unwrap()
            .declaration
    };
    let ir = luna::host::llvm_ir_with_protocols(
        &package,
        "verify",
        1_000_000,
        protocol("Copy"),
        protocol("Drop"),
    )
    .unwrap();
    for optimization in [
        luna::host::OptimizationLevel::O0,
        luna::host::OptimizationLevel::O3,
    ] {
        luna::host::Toolchain {
            optimization,
            ..Default::default()
        }
        .execute_test(&ir, std::time::Duration::from_secs(5))
        .unwrap();
    }
}

#[test]
fn text_hashes_and_equality_observe_complete_utf8_contents() {
    for (literal, raw_bytes) in [
        ("\"\"", b"".as_slice()),
        ("\"a\"", b"a".as_slice()),
        ("\"é\"", "é".as_bytes()),
        ("\"a\\0b\"", b"a\0b".as_slice()),
    ] {
        let mut bytes = raw_bytes.to_vec();
        bytes.push(255);
        let expected = fnv1a_bytes(&bytes);
        let package = compile_hash(&source(&format!(
            "fn run()->u64 {{let s={literal}; TestFnvBuild.hash_val(s.ref)}}
             comptime fn staged()->u64 {{DefaultComptimeHasher.new().hash_one({literal})}}
             fn verify() {{let value=run();assumes value == {expected};}}"
        )));
        assert_eq!(
            luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 1_000_000)
                .unwrap(),
            Value::UnsignedInteger(expected.into())
        );
        assert_eq!(
            luna::invocation::invoke(&package, "staged", 1_000_000).unwrap(),
            Value::UnsignedInteger(expected.into())
        );
        let protocol = |name: &str| {
            package
                .traits
                .iter()
                .find(|item| item.name == name)
                .unwrap()
                .declaration
        };
        let ir = luna::host::llvm_ir_with_protocols(
            &package,
            "verify",
            1_000_000,
            protocol("Copy"),
            protocol("Drop"),
        )
        .unwrap();
        for optimization in [
            luna::host::OptimizationLevel::O0,
            luna::host::OptimizationLevel::O3,
        ] {
            luna::host::Toolchain {
                optimization,
                ..Default::default()
            }
            .execute_test(&ir, std::time::Duration::from_secs(5))
            .unwrap();
        }
    }
    let body = r#"fn run()->bool {
        let pre = "é";
        let decomp = "é";
        let a0b = "a\0b";
        let a0c = "a\0c";
        let empty = "";
        let a = "a";
        pre.as(Eq).eq(pre.ref) and pre.as(Eq).ne(decomp.ref)
        and a0b.as(Eq).ne(a0c.ref) and empty.as(Eq).ne(a.ref)
    }"#;
    let staged = r#"comptime fn staged()->bool {
        "é".as(ComptimeEq).eq("é") and "é".as(ComptimeEq).ne("é")
        and "a\0b".as(ComptimeEq).ne("a\0c") and "".as(ComptimeEq).ne("a")
    }"#;
    let package = compile_hash(&source(&format!(
        "{body}{staged} fn verify() {{let valid=run();assumes valid;}}"
    )));
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 1_000_000)
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::invocation::invoke(&package, "staged", 1_000_000).unwrap(),
        Value::Bool(true)
    );
    let protocol = |name: &str| {
        package
            .traits
            .iter()
            .find(|item| item.name == name)
            .unwrap()
            .declaration
    };
    let ir = luna::host::llvm_ir_with_protocols(
        &package,
        "verify",
        1_000_000,
        protocol("Copy"),
        protocol("Drop"),
    )
    .unwrap();
    luna::host::Toolchain::default()
        .execute_test(&ir, std::time::Duration::from_secs(5))
        .unwrap();
}

#[test]
fn string_hash_does_not_bypass_resource_provenance_checks() {
    let package = compile_hash(&source(
        "fn verify() {let value:str=\"foobar\";let lit=\"foobar\";let expected=TestFnvBuild.hash_val(lit.ref);let actual=TestFnvBuild.hash_val(value.ref);assumes actual == expected;}",
    ));
    let protocol = |name: &str| {
        package
            .traits
            .iter()
            .find(|item| item.name == name)
            .unwrap()
            .declaration
    };
    let ir = luna::host::llvm_ir_with_protocols(
        &package,
        "verify",
        1_000_000,
        protocol("Copy"),
        protocol("Drop"),
    )
    .unwrap();
    luna::host::Toolchain::default()
        .execute_test(&ir, std::time::Duration::from_secs(5))
        .unwrap();

    let err = elaboration::verification::memory::pointer_input_layout(
        None,
        &nir::Term::Primitive(nir::Primitive::Str),
        &mut 10_000,
    )
    .unwrap_err();
    assert!(err.contains("string-view memory provenance"), "{err}");

    let escaping = compile_hash(&source(
        "fn leak()->*str {let local=\"foobar\";local.ref}
         fn run()->u64 {let _=leak();0}",
    ));
    assert!(
        luna::host::llvm_ir_with_protocols(
            &escaping,
            "run",
            1_000_000,
            protocol("Copy"),
            protocol("Drop"),
        )
        .is_err()
    );
}

#[test]
fn root_dispatch_preserves_generic_parents_of_concrete_implementations() {
    let package = support::compile(
        "trait Parent {} impl Parent for T where T {}\n\
         trait Child :- Parent {} impl Child for bool {}\n\
         comptime fn run()->bool {true}",
    );
    let root = iir::InstanceKey {
        declaration: package
            .functions
            .iter()
            .find(|f| f.name == "run")
            .unwrap()
            .declaration,
        arguments: vec![].into(),
    };
    for invocation in [false, true] {
        let materialize = if invocation {
            elaboration::materialization::dispatch_instances_for_invocation
        } else {
            elaboration::materialization::dispatch_instances_for_roots
        };
        assert!(
            materialize(
                &package,
                &[root.clone()],
                elaboration::ptc::Budget {
                    steps: 0,
                    ..Default::default()
                }
            )
            .is_err()
        );
        let mut prepared = package.clone();
        prepared.dispatch_instances =
            materialize(&package, &[root.clone()], Default::default()).unwrap();
        let parent = package
            .traits
            .iter()
            .find(|t| t.name == "Parent")
            .unwrap()
            .declaration;
        assert!(
            prepared
                .dispatch_instances
                .iter()
                .any(|i| i.interface == parent
                    && i.subject == nir::Term::Primitive(nir::Primitive::Bool))
        );
        let mut vm =
            comptime_interpretor::Interpreter::for_comptime(&prepared, Default::default()).unwrap();
        assert_eq!(
            vm.invoke_instantiated(root.declaration, vec![], vec![])
                .unwrap(),
            Value::Bool(true)
        );
        let mut missing = package.clone();
        missing
            .implementations
            .retain(|i| i.interface != Some(parent));
        assert!(materialize(&missing, &[root.clone()], Default::default()).is_err());
    }
}

#[test]
fn shipped_text_hash_runs_its_loop_in_bytecode() {
    use comptime_interpretor::{Budget, Interpreter};
    let package = compile_hash(&source(
        "comptime fn staged()->u64 {DefaultComptimeHasher.new().hash_one(\"é\")}",
    ));
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    let result = vm.invoke(support::id(&package, "staged"), vec![]).unwrap();
    let mut bytes = "é".as_bytes().to_vec();
    bytes.push(255);
    assert_eq!(result, Value::UnsignedInteger(fnv1a_bytes(&bytes).into()));
    assert!(
        vm.bytecode_invocations() >= 7,
        "hash_one, hasher new/finish, str hash, write_str, length, byte reads and mix calls must compile in bytecode"
    );
}

#[test]
fn same_named_methods_select_the_execution_phase_without_source_prefixes() {
    let package = compile_hash(&source(
        "fn run()->bool {let value:u64=9;let eight:u64=8;TestFnvBuild.hash_val(value.ref) != TestFnvBuild.hash_val(eight.ref) and value.eq(value.ref) and value.ne(eight.ref)}
         comptime fn staged()->bool {let value:u64=9;DefaultComptimeHasher.new().hash_one(value) != DefaultComptimeHasher.new().hash_one(8u64) and value.eq(value) and value.ne(8)}",
    ));
    assert_eq!(
        luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 1_000_000)
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::invocation::invoke(&package, "staged", 1_000_000).unwrap(),
        Value::Bool(true)
    );
    for (eq, hash) in [("Eq", "ComptimeHash"), ("ComptimeEq", "Hash")] {
        let mut items = intrinsic::LangItems::new();
        let bindings = [
            (intrinsic::LangItem::Eq, eq),
            (intrinsic::LangItem::Hash, hash),
        ]
        .map(|(item, name)| {
            (
                item,
                package
                    .traits
                    .iter()
                    .find(|t| t.name == name)
                    .unwrap()
                    .declaration,
            )
        });
        items.bind_source_traits(&package, &bindings).unwrap();
        let failure = luna::comptime_keys::Protocol::prepare(
            &package,
            &items,
            &nir::Term::Primitive(nir::Primitive::U64),
            elaboration::ptc::Budget {
                steps: 1_000_000,
                ..Default::default()
            },
        )
        .err()
        .unwrap();
        assert!(failure.contains("comptime trait membership"), "{failure}");
    }
}

#[test]
fn arbitrary_precision_integer_hashes_use_canonical_target_independent_bytes() {
    use comptime_interpretor::{Budget, Interpreter};
    use comptime_runtime::Integer;
    let package = compile_comptime_hash(&comptime_source(
        "comptime fn digest(value:Integer)->u64 {DefaultComptimeHasher.new().hash_one(value)}
         comptime fn same(left:Integer,right:Integer)->bool {left.eq(right) and not left.ne(right)}",
    ));
    assert!(package.target.is_none());
    let mut vm = Interpreter::for_comptime(
        &package,
        Budget {
            steps: 5_000_000,
            ..Default::default()
        },
    )
    .unwrap();
    for (literal, expected) in [
        ("0", 590684067820433389u128),
        ("1", 590682968308805178),
        ("-1", 589728592215707255),
        ("255", 590474061099445088),
        ("-255", 589911111145990281),
        ("256", 15658190276026904068),
        ("-256", 15034027211670106815),
        (
            "1606938044258990275541962092341162602522202993782792835301376",
            8424690812682328228,
        ),
        (
            "-1606938044258990275541962092341162602522202993782792835301376",
            18410723617023889215,
        ),
        (
            "1606938044258990275541962092341162602522202993782792835301377",
            6053771721358495617,
        ),
    ] {
        let mut value =
            Integer::parse_literal(literal.trim_start_matches('-'), &mut Default::default())
                .unwrap();
        if literal.starts_with('-') {
            value = value.negated();
        }
        let value = Value::Integer(value);
        assert_eq!(
            vm.invoke(support::id(&package, "digest"), vec![value.clone()])
                .unwrap(),
            Value::UnsignedInteger(expected),
            "{literal}"
        );
        assert_eq!(
            vm.invoke(support::id(&package, "same"), vec![value.clone(), value])
                .unwrap(),
            Value::Bool(true),
            "{literal}"
        );
    }
}

#[test]
fn integer_key_protocol_recovers_from_budget_exhaustion_without_target_data() {
    use comptime_executor::{Context, KeyProtocol};
    use comptime_runtime::Integer;
    let package = compile_comptime_hash(&comptime_source(""));
    let mut items = intrinsic::LangItems::new();
    let bindings = [
        (intrinsic::LangItem::Eq, "ComptimeEq"),
        (intrinsic::LangItem::Hash, "ComptimeHash"),
    ]
    .map(|(item, name)| {
        (
            item,
            package
                .traits
                .iter()
                .find(|t| t.name == name)
                .unwrap()
                .declaration,
        )
    });
    items.bind_source_traits(&package, &bindings).unwrap();
    let mut protocol = luna::comptime_keys::Protocol::prepare(
        &package,
        &items,
        &nir::Term::Primitive(nir::Primitive::Integer),
        elaboration::ptc::Budget {
            steps: 1_000_000,
            ..Default::default()
        },
    )
    .unwrap();
    let magnitude = Integer::parse_literal(
        "1606938044258990275541962092341162602522202993782792835301376",
        &mut Default::default(),
    )
    .unwrap();
    let negative = Value::Integer(magnitude.negated());
    let positive = Value::Integer(magnitude);
    assert!(protocol.hash(&positive, &mut Context::new(1)).is_err());
    let mut context = Context::new(5_000_000);
    let first = protocol.hash(&positive, &mut context).unwrap();
    assert!(
        protocol
            .equivalent(&positive, &positive, &mut context)
            .unwrap()
    );
    assert!(
        !protocol
            .equivalent(&positive, &negative, &mut context)
            .unwrap()
    );
    let other = protocol.hash(&negative, &mut context).unwrap();
    assert_eq!(protocol.hash(&negative, &mut context).unwrap(), other);
    assert_eq!(protocol.hash(&positive, &mut context).unwrap(), first);
}

#[test]
fn runtime_qualified_equality_requires_payload_evidence() {
    for domain in ["?Key", "!Key bool", "!bool Key"] {
        let input = source(&format!(
            "mod qualified_equality {{ {} }}
             struct Key {{value:bool}}
             fn invalid(left:{domain},right:{domain})->bool {{left.as(Eq).eq(right.ref)}}",
            include_str!("../../../library/builtin/types/equality.fl")
        ));
        let intrinsics = hash_host_intrinsics();
        let failure = support::compile_with_source_bindings_for_target(
            &input,
            &support::meta_fixture_protocols(),
            hash_selectors(&intrinsics),
            Some(nir::target::TargetData::new(64).unwrap()),
        )
        .err()
        .expect("missing runtime payload Eq must reject the call");
        assert!(
            failure.contains("trait") || failure.contains("method"),
            "{failure}"
        );
    }
}

#[test]
fn runtime_qualified_hashes_encode_branches_and_require_payload_protocols() {
    let package = compile_hash(&source(
        "fn absent()->u64 {let value:?u64=null; TestFnvBuild.hash_val(value.ref)}
        fn present()->u64 {let value:?u64=7; TestFnvBuild.hash_val(value.ref)}
        fn success()->u64 {let value:!u64 u64=7; TestFnvBuild.hash_val(value.ref)}
        fn failure()->u64 {let value:!u64 u64=error 7; TestFnvBuild.hash_val(value.ref)}",
    ));
    for (name, tag, payload) in [
        ("absent", 0u8, None),
        ("present", 1, Some(7u64)),
        ("success", 0, Some(7)),
        ("failure", 1, Some(7)),
    ] {
        let bytes: Vec<u8> = std::iter::once(tag)
            .chain(payload.into_iter().flat_map(u64::to_le_bytes))
            .collect();
        assert_eq!(
            luna::execution::interpret_with_protocols(&package, name, "Copy", "Drop", 1_000_000)
                .unwrap(),
            Value::UnsignedInteger(fnv1a_bytes(&bytes).into())
        );
    }
    for domain in ["?Key", "!Key bool", "!bool Key"] {
        let input = source(&format!(
            "struct Key {{value:bool}}
            fn invalid(value:{domain})->u64 {{TestFnvBuild.hash_val(value.ref)}}"
        ));
        let failure = compile_hash_result(&input, &support::meta_fixture_protocols())
            .err()
            .expect("missing runtime payload Hash must reject the call");
        assert!(
            failure.contains("trait") || failure.contains("method"),
            "{failure}"
        );
    }
}
