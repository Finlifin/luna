//! Exact public Integer algorithms use ordinary comptime methods and loops.
#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Budget, ErrorKind, Interpreter, Value};
use comptime_runtime::Integer;

fn compile(source: &str) -> Result<nir::program::Package, String> {
    compile_for_target(source, 64)
}

fn compile_for_target(source: &str, width: u32) -> Result<nir::program::Package, String> {
    support::compile_with_source_bindings_for_target(
        &format!(
            "mod meta {{ {} }} use meta.builtin.Integer; {source}",
            include!("support/meta_source.rs")
        ),
        &support::meta_fixture_protocols(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.meta.List"),
            ..Default::default()
        },
        Some(nir::target::TargetData::new(width).unwrap()),
    )
}

#[test]
fn euclidean_identities_and_nonnegative_gcd_lcm_cover_all_signs() {
    let package = compile(
        "comptime fn run(a: Integer, b: Integer) -> bool {
        let g = a.gcd(b);
        let l = a.lcm(b);
        if a == 0 and b == 0 { return g == 0 and l == 0; };
        if g <= 0 or l < 0 { return false; };
        if a.rem_trunc(g) != 0 or b.rem_trunc(g) != 0 { return false; };
        if g * l != (a * b).abs() { return false; };
        if b == 0 { return g == a.abs() and l == 0; };
        let q = a.div_euclid(b);
        let r = a.rem_euclid(b);
        a == q * b + r and r >= 0 and r < b.abs()
    }",
    )
    .unwrap();
    let mut vm = Interpreter::for_comptime(
        &package,
        Budget {
            steps: 10_000_000,
            ..Default::default()
        },
    )
    .unwrap();
    let id = support::id(&package, "run");
    for a in -8..=8 {
        for b in -8..=8 {
            assert_eq!(
                vm.invoke(
                    id,
                    vec![
                        Value::Integer(Integer::from_i128(a)),
                        Value::Integer(Integer::from_i128(b))
                    ]
                )
                .unwrap(),
                Value::Bool(true),
                "{a}, {b}"
            );
        }
    }
}

#[test]
fn methods_preserve_arbitrary_precision_and_public_conversion() {
    let package = compile(include_str!(
        "../../../development/examples/sysroot-meta-integers.fl"
    ))
    .unwrap();
    assert_eq!(
        luna::invocation::invoke(&package, "integer_algorithms", 1_000_000).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn zero_division_restores_invocation_and_arguments_run_once_in_order() {
    let package = compile(
        "comptime fn fail(a: Integer, b: Integer) -> Integer { a.div_euclid(b) }
        comptime fn fail_remainder(a: Integer, b: Integer) -> Integer { a.rem_euclid(b) }
        comptime fn run() -> bool {
            var trace: Integer = 0;
            let value = (do {trace = trace * 10 + 1; -7}).div_euclid(
                do {trace = trace * 10 + 2; 3});
            value == -3 and trace == 12
        }
        comptime fn early() -> Integer {
            let value: Integer = -7;
            value.div_euclid(do {return 99; 0})
        }",
    )
    .unwrap();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for entry in ["fail", "fail_remainder"] {
        let error = vm
            .invoke(
                support::id(&package, entry),
                vec![
                    Value::Integer(Integer::from_i128(7)),
                    Value::Integer(Integer::from_i128(0)),
                ],
            )
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::DivisionByZero);
        assert!(error.source.is_some());
        assert_eq!(
            vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
            Value::Bool(true)
        );
    }
    assert_eq!(
        vm.invoke(support::id(&package, "early"), vec![]).unwrap(),
        Value::Integer(Integer::from_i128(99))
    );
}

#[test]
fn exact_integer_methods_reject_runtime_and_wrong_operand_domains() {
    for source in [
        "fn bad(a: Integer, b: Integer) -> Integer { a.gcd(b) }",
        "comptime fn bad(a: Integer) -> Integer { a.div_euclid(true) }",
        "comptime fn bad(a: Integer) -> Integer { a.lcm(false) }",
    ] {
        assert!(compile(source).is_err(), "accepted {source}");
    }
}

#[test]
fn integer_size_methods_obey_the_selected_target_and_preserve_all_bits() {
    for width in [32, 64] {
        let package = compile_for_target(
            "use meta.builtin.usize;
             comptime fn narrow(value: Integer) -> usize { value.to_usize() }
             comptime fn widen(value: usize) -> Integer { Integer.from_usize(value) }",
            width,
        )
        .unwrap();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let max = (1u128 << width) - 1;
        for bad in [Integer::from_i128(-1), Integer::from_u128(max + 1)] {
            assert_eq!(
                vm.invoke(support::id(&package, "narrow"), vec![Value::Integer(bad)])
                    .unwrap_err()
                    .kind,
                ErrorKind::MachineOverflow
            );
        }
        for value in [0, 1, max] {
            let before = vm.bytecode_invocations();
            let integer = Value::Integer(Integer::from_u128(value));
            let narrowed = vm
                .invoke(support::id(&package, "narrow"), vec![integer.clone()])
                .unwrap();
            assert_eq!(narrowed, Value::UnsignedInteger(value));
            assert_eq!(
                vm.invoke(support::id(&package, "widen"), vec![narrowed])
                    .unwrap(),
                integer
            );
            // User entry, method and named compiler-operation wrapper for each call.
            assert_eq!(vm.bytecode_invocations() - before, 6);
        }
    }
}
