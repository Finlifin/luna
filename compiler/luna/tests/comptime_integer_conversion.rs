#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use comptime_interpretor::{Budget, ErrorKind, Interpreter, Value};
use comptime_runtime::Integer;

fn compile(source: &str, width: Option<u32>) -> Result<nir::program::Package, String> {
    support::compile_with_source_bindings_for_target(
        &format!(
            "mod meta {{ {} }}
            typealias Integer = .Integer'builtin;
            typealias usize = .usize'builtin;
            typealias isize = .isize'builtin;
            typealias u128 = .u128'builtin;
            typealias i8 = .i8'builtin;
            typealias u8 = .u8'builtin;
            const convert = .comptime_integer_convert'builtin;
            const representation = .integer_representation'builtin;
            {source}",
            include!("support/meta_source.rs")
        ),
        &support::meta_fixture_protocols(),
        luna::protocols::type_constructors::Selectors {
            meta_list: Some("input.meta.List"),
            ..Default::default()
        },
        width.map(|width| nir::target::TargetData::new(width).unwrap()),
    )
}

#[test]
fn public_size_conversions_use_target_bounds_and_recover_after_overflow() {
    for width in [32, 64] {
        let package = compile(
            "comptime fn narrow(value:Integer)->usize {meta.to_usize(value)}
             comptime fn widen(value:usize)->Integer {meta.from_usize(value)}",
            Some(width),
        )
        .unwrap();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let max = (1u128 << width) - 1;
        for bad in [Integer::from_i128(-1), Integer::from_u128(max + 1)] {
            let failure = vm
                .invoke(support::id(&package, "narrow"), vec![Value::Integer(bad)])
                .unwrap_err();
            assert_eq!(failure.kind, ErrorKind::MachineOverflow);
            assert!(failure.source.is_some());
        }
        for value in [0, 1, max] {
            let before = vm.bytecode_invocations();
            assert_eq!(
                vm.invoke(
                    support::id(&package, "narrow"),
                    vec![Value::Integer(Integer::from_u128(value))]
                )
                .unwrap(),
                Value::UnsignedInteger(value)
            );
            assert_eq!(
                vm.invoke(
                    support::id(&package, "widen"),
                    vec![Value::UnsignedInteger(value)]
                )
                .unwrap(),
                Value::Integer(Integer::from_u128(value))
            );
            assert_eq!(vm.bytecode_invocations() - before, 4);
        }
        assert!(package.to_string().contains("(comptime-integer-convert "));
    }
}

#[test]
fn size_conversion_preserves_single_evaluation_and_early_return() {
    let package = compile(
        "comptime fn run()->bool {
            var count:Integer=0;
            let size=meta.to_usize(do {count += 1; 17});
            let original=meta.from_usize(size);
            count == 1 and original == 17
        }
        comptime fn early()->usize {
            meta.to_usize(do {return 9; -1})
        }",
        Some(32),
    )
    .unwrap();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    assert_eq!(
        vm.invoke(support::id(&package, "run"), vec![]).unwrap(),
        Value::Bool(true)
    );
    assert_eq!(vm.bytecode_invocations(), 3);
    let before = vm.bytecode_invocations();
    assert_eq!(
        vm.invoke(support::id(&package, "early"), vec![]).unwrap(),
        Value::UnsignedInteger(9)
    );
    assert_eq!(vm.bytecode_invocations() - before, 1);
    assert!(luna::invocation::invoke(&package, "run", 1).is_err());
}

#[test]
fn privileged_integer_conversion_preserves_signed_values_without_bit_reinterpretation() {
    let package = compile(
        "comptime fn narrow(value:Integer)->i8 {convert(i8,value)}
         comptime fn widen(value:i8)->Integer {convert(Integer,value)}",
        Some(64),
    )
    .unwrap();
    let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
    for value in [-128, 0, 127] {
        assert_eq!(
            vm.invoke(
                support::id(&package, "narrow"),
                vec![Value::Integer(Integer::from_i128(value))]
            )
            .unwrap(),
            Value::SignedInteger(value)
        );
        assert_eq!(
            vm.invoke(
                support::id(&package, "widen"),
                vec![Value::SignedInteger(value)]
            )
            .unwrap(),
            Value::Integer(Integer::from_i128(value))
        );
    }
    for value in [-129, 128] {
        assert_eq!(
            vm.invoke(
                support::id(&package, "narrow"),
                vec![Value::Integer(Integer::from_i128(value))]
            )
            .unwrap_err()
            .kind,
            ErrorKind::MachineOverflow
        );
    }
}

#[test]
fn size_conversion_rejects_missing_target_wrong_domains_and_runtime_intrinsics() {
    let package = compile("comptime fn run()->usize {meta.to_usize(1)}", None).unwrap();
    assert!(luna::invocation::invoke(&package, "run", 100_000).is_err());
    for source in [
        "fn bad(value:usize)->usize {convert(usize,value)}",
        "comptime fn bad()->usize {meta.to_usize(true)}",
        "comptime fn bad()->bool {convert(bool,1)}",
    ] {
        assert!(compile(source, Some(32)).is_err(), "{source}");
    }
}

#[test]
fn runtime_iir_rejects_a_residual_conversion_even_with_machine_only_types() {
    let package = compile("fn run()->u8 {1}", Some(32)).unwrap();
    let mut graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let body = &mut graph.functions[0].function.body;
    body.kind = nir::program::ExprKind::ComptimeIntegerConvert(Box::new(body.clone()));
    let error = graph.validate(&mut 100_000).unwrap_err();
    assert!(
        error.contains("comptime integer conversion completion"),
        "{error}"
    );
}

#[test]
fn target_integer_representation_uses_compiled_operands_and_target_widths() {
    for width in [8, 32, 64, 128] {
        let package = compile(
            "comptime fn select(value:isize)->isize {value}
            comptime fn bits(value:isize)->u128 {representation(select(value))}",
            Some(width),
        )
        .unwrap();
        let mut vm = Interpreter::for_comptime(&package, Budget::default()).unwrap();
        let mut tree = Interpreter::new(&package, Budget::default()).unwrap();
        for value in [-1, 0, 1] {
            let before = vm.bytecode_invocations();
            let expected = Value::UnsignedInteger((value as u128) & (u128::MAX >> (128 - width)));
            let id = support::id(&package, "bits");
            assert_eq!(
                vm.invoke(id, vec![Value::SignedInteger(value)]).unwrap(),
                expected
            );
            assert_eq!(
                tree.invoke(id, vec![Value::SignedInteger(value)]).unwrap(),
                expected
            );
            assert_eq!(vm.bytecode_invocations() - before, 2);
        }
    }
}
