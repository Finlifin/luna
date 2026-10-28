//! Public comptime math, with real source aliases and independently bound VM tags.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Budget, Interpreter, Value};
use nir::intrinsics::Operation;

fn compile(body: &str, width: u32) -> Result<nir::program::Package, String> {
    let bindings: Vec<_> = Operation::ALL
        .iter()
        .map(|op| (*op, format!("input.math.target.Scalar.{}", op.name())))
        .collect();
    support::compile_with_source_bindings_for_target(
        &format!(
            "mod builtin {{pub mod primitives {{ {} }} pub use primitives.*; }} use builtin.primitives.*; mod math {{ {} }} {body}",
            include_str!("../../../library/builtin/primitives.fl"),
            include_str!("../../../library/meta/scalar_math.fl")
        ),
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            comptime_intrinsics: &bindings,
            ..Default::default()
        },
        Some(nir::target::TargetData::new(width).unwrap()),
    )
}

#[test]
fn scalar_float_operations_execute_at_both_ieee_widths() {
    let cases = [
        ("fma(2.0, 3.0, 1.0)", 7.0),
        ("sqrt(4.0)", 2.0),
        ("copysign(2.0, -1.0)", -2.0),
        ("floor(-1.5)", -2.0),
        ("ceil(-1.5)", -1.0),
        ("trunc(-1.5)", -1.0),
        ("roundeven(2.5)", 2.0),
        ("round(-2.5)", -3.0),
        ("abs(-2.0)", 2.0),
        ("sin(0.0)", 0.0),
        ("cos(0.0)", 1.0),
        ("exp(0.0)", 1.0),
        ("exp2(3.0)", 8.0),
        ("log(1.0)", 0.0),
        ("log2(8.0)", 3.0),
        ("log10(100.0)", 2.0),
        ("pow(2.0, 3.0)", 8.0),
        ("powi(2.0, -3)", 0.125),
        ("ldexp(1.5, 3)", 12.0),
        ("minimum(2.0, -1.0)", -1.0),
        ("maximum(2.0, -1.0)", 2.0),
        ("minimumnum(2.0, -1.0)", -1.0),
        ("maximumnum(2.0, -1.0)", 2.0),
    ];
    for ty in ["f32", "f64"] {
        let source: String = cases
            .iter()
            .enumerate()
            .map(|(i, (call, _))| {
                let (name, args) = call.split_once('(').unwrap();
                format!("comptime fn case_{i}()->{ty} {{ math.{name}<{ty}>({args} }}\n")
            })
            .collect();
        let package = compile(&source, 64).unwrap();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        for (i, (_, expected)) in cases.iter().enumerate() {
            let expected = if ty == "f32" {
                Value::Float32((*expected as f32).to_bits())
            } else {
                Value::Float64((*expected as f64).to_bits())
            };
            assert_eq!(
                vm.invoke(support::id(&package, &format!("case_{i}")), vec![])
                    .unwrap(),
                expected,
                "{ty} {}",
                cases[i].0
            );
        }
        assert!(package.to_string().contains("comptime-intrinsic"));
    }
}

#[test]
fn integer_operations_respect_width_signedness_and_selected_target() {
    let cases = [
        ("u8", "bit_reverse(1)", Value::UnsignedInteger(128)),
        ("u128", "bit_reverse(1)", Value::UnsignedInteger(1 << 127)),
        (
            "u128",
            "count_ones(340282366920938463463374607431768211455)",
            Value::UnsignedInteger(128),
        ),
        (
            "u128",
            "saturating_add(340282366920938463463374607431768211455, 1)",
            Value::UnsignedInteger(u128::MAX),
        ),
        (
            "i128",
            "saturating_sub(-170141183460469231731687303715884105728, 1)",
            Value::SignedInteger(i128::MIN),
        ),
        (
            "i128",
            "wrapping_abs(-170141183460469231731687303715884105728)",
            Value::SignedInteger(i128::MIN),
        ),
        (
            "u8",
            "funnel_shift_left(129, 12, 8)",
            Value::UnsignedInteger(129),
        ),
        (
            "u8",
            "funnel_shift_right(129, 12, 8)",
            Value::UnsignedInteger(12),
        ),
        ("u16", "byte_swap(258)", Value::UnsignedInteger(513)),
        ("u8", "count_ones(129)", Value::UnsignedInteger(2)),
        ("u8", "leading_zeros(0)", Value::UnsignedInteger(8)),
        ("u8", "trailing_zeros(0)", Value::UnsignedInteger(8)),
        (
            "u8",
            "funnel_shift_left(128, 1, 1)",
            Value::UnsignedInteger(0),
        ),
        (
            "u8",
            "funnel_shift_right(128, 1, 1)",
            Value::UnsignedInteger(0),
        ),
        ("u8", "rotate_left(129, 1)", Value::UnsignedInteger(3)),
        ("u8", "rotate_right(129, 1)", Value::UnsignedInteger(192)),
        ("u8", "saturating_add(250, 10)", Value::UnsignedInteger(255)),
        ("i8", "saturating_sub(-120, 20)", Value::SignedInteger(-128)),
        ("i8", "min(-1, 1)", Value::SignedInteger(-1)),
        ("i8", "max(-1, 1)", Value::SignedInteger(1)),
        ("i8", "wrapping_abs(-128)", Value::SignedInteger(-128)),
        ("u8", "bit_and(10, 3)", Value::UnsignedInteger(2)),
        ("u8", "bit_or(10, 3)", Value::UnsignedInteger(11)),
        ("u8", "bit_xor(10, 3)", Value::UnsignedInteger(9)),
    ];
    let mut source: String = cases
        .iter()
        .enumerate()
        .map(|(i, (ty, call, _))| {
            let (name, args) = call.split_once('(').unwrap();
            format!("comptime fn case_{i}()->{ty} {{math.{name}<{ty}>({args}}}\n")
        })
        .collect();
    source.push_str("comptime fn width()->usize {let zero:usize=0; math.leading_zeros(zero)}");
    for width in [32, 64] {
        let package = compile(&source, width).unwrap();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        for (i, (_, call, expected)) in cases.iter().enumerate() {
            assert_eq!(
                vm.invoke(support::id(&package, &format!("case_{i}")), vec![])
                    .unwrap(),
                *expected,
                "{call}"
            );
        }
        assert_eq!(
            vm.invoke(support::id(&package, "width"), vec![]).unwrap(),
            Value::UnsignedInteger(width as u128)
        );
    }
}

#[test]
fn phase_and_domain_boundaries_are_not_erased() {
    for source in [
        "fn bad(x:f64)->f64 {math.sqrt(x)}",
        "comptime fn bad(x:Integer)->Integer {math.leading_zeros(x)}",
        "comptime fn bad(x:u8)->u8 {math.byte_swap(x)}",
        "comptime fn bad(x:bool)->bool {math.sqrt(x)}",
    ] {
        assert!(compile(source, 64).is_err(), "{source}");
    }
}

#[test]
fn ieee_edges_and_fused_rounding_survive_vm_invocation() {
    for ty in ["f32", "f64"] {
        let source = format!(
            "
            comptime fn low(a:{ty},b:{ty})->{ty} {{math.minimum(a,b)}}
            comptime fn high(a:{ty},b:{ty})->{ty} {{math.maximum(a,b)}}
            comptime fn numeric(a:{ty},b:{ty})->{ty} {{math.minimumnum(a,b)}}
            comptime fn scale(a:{ty},n:i32)->{ty} {{math.ldexp(a,n)}}
            comptime fn fused(a:{ty},b:{ty},c:{ty})->{ty} {{math.fma(a,b,c)}}
            comptime fn nan(a:{ty})->bool {{math.isnan(a)}}
            comptime fn inf(a:{ty})->bool {{math.isinf(a)}}
            comptime fn finite(a:{ty})->bool {{math.isfinite(a)}}
            comptime fn normal(a:{ty})->bool {{math.isnormal(a)}}
            comptime fn subnormal(a:{ty})->bool {{math.issubnormal(a)}}
            comptime fn sign(a:{ty})->bool {{math.signbit(a)}}"
        );
        let package = compile(&source, 64).unwrap();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let encode = |x: f64| {
            if ty == "f32" {
                Value::Float32((x as f32).to_bits())
            } else {
                Value::Float64(x.to_bits())
            }
        };
        let mut invoke = |name: &str, args| vm.invoke(support::id(&package, name), args).unwrap();
        assert_eq!(invoke("low", vec![encode(0.0), encode(-0.0)]), encode(-0.0));
        assert_eq!(invoke("high", vec![encode(-0.0), encode(0.0)]), encode(0.0));
        assert_eq!(
            invoke("numeric", vec![encode(f64::NAN), encode(2.0)]),
            encode(2.0)
        );
        let nan = invoke("low", vec![encode(2.0), encode(f64::NAN)]);
        assert_eq!(invoke("nan", vec![nan]), Value::Bool(true));
        assert_eq!(
            invoke("inf", vec![encode(f64::INFINITY)]),
            Value::Bool(true)
        );
        assert_eq!(
            invoke("finite", vec![encode(f64::INFINITY)]),
            Value::Bool(false)
        );
        assert_eq!(invoke("normal", vec![encode(1.0)]), Value::Bool(true));
        assert_eq!(invoke("sign", vec![encode(-0.0)]), Value::Bool(true));
        let (tiny, min_exp, huge_exp, small_exp) = if ty == "f32" {
            (Value::Float32(1), -149, 127, -126)
        } else {
            (Value::Float64(1), -1074, 1023, -1022)
        };
        assert_eq!(invoke("subnormal", vec![tiny.clone()]), Value::Bool(true));
        assert_eq!(
            invoke("scale", vec![encode(1.0), Value::SignedInteger(min_exp)]),
            tiny
        );
        assert_eq!(
            invoke(
                "scale",
                vec![
                    encode(2.0f64.powi(huge_exp)),
                    Value::SignedInteger(small_exp as i128)
                ]
            ),
            encode(2.0)
        );
        assert_eq!(
            invoke(
                "scale",
                vec![encode(-0.0), Value::SignedInteger(i32::MAX as i128)]
            ),
            encode(-0.0)
        );
        let eps = if ty == "f32" {
            2.0f64.powi(-23)
        } else {
            2.0f64.powi(-52)
        };
        assert_eq!(
            invoke(
                "fused",
                vec![encode(1.0 + eps), encode(1.0 - eps), encode(-1.0)]
            ),
            encode(-eps * eps)
        );
    }
}

#[test]
fn operands_are_evaluated_once_in_source_order_and_early_return_survives() {
    let package = compile(
        "comptime fn run()->bool {
        var visits:u8=0;
        let result=math.rotate_left<u8>(
            do {visits += 1; let value:u8=129; value},
            do {assumes visits == 1; visits += 1; let amount:u8=1; amount});
        visits == 2 and result == 3
    }
    comptime fn early()->f64 {
        math.fma<f64>(do {return 9.0; 0.0}, 2.0, 3.0)
    }",
        64,
    )
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::invocation::invoke(&package, "early", 100_000).unwrap(),
        Value::Float64(9.0f64.to_bits())
    );
    assert!(luna::invocation::invoke(&package, "run", 1).is_err());
}

#[test]
fn vm_world_retains_both_trait_phases_and_rejects_duplicate_relationships() {
    let source = support::with_attrs(
        "trait Shared {
        fn get(self)->bool;
        ^attrs.comptime_phase comptime fn get_comptime(self)->bool;
    }
    impl Shared for bool {fn get(self)->bool {false}}
    comptime impl Shared for bool {comptime fn get_comptime(self)->bool {true}}
    comptime fn run()->bool {true.as(Shared).get_comptime()}
    fn runtime()->bool {true.as(Shared).get()}",
    );
    let package = compile(&source, 64).unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        luna::execution::interpret(&package, "runtime", 100_000).unwrap(),
        Value::Bool(false)
    );
    let relationship = package
        .implementations
        .iter()
        .find(|i| i.phases.comptime)
        .unwrap();
    assert!(relationship.phases.runtime && relationship.phases.comptime);
    let mut invalid = package.clone();
    let mut duplicate = invalid
        .implementations
        .iter()
        .find(|i| i.phases.comptime)
        .unwrap()
        .clone();
    duplicate.declaration.local = 999999;
    invalid.implementations.push(duplicate);
    assert!(Interpreter::new(&invalid, Budget::default()).is_err());
}

#[test]
fn invocation_retains_generic_parent_witnesses_for_the_loaded_world() {
    let package = compile(
        "trait Parent {} impl Parent for T where T {}
        trait Child :- Parent {} impl Child for bool {}
        comptime fn run()->bool {true}",
        64,
    )
    .unwrap();
    assert!(!package.dispatch_instances.is_empty());
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100_000).unwrap(),
        Value::Bool(true)
    );
}
