#[path = "support/library.rs"]
mod library;
use luna::frontend::{self, Artifact};
use nir::target::TargetData;
use std::sync::atomic::{AtomicU64, Ordering};

fn compile(source: &str, width: Option<u32>) -> Result<nir::program::Package, String> {
    compile_with_intrinsics(source, width, &[])
}

fn compile_with_intrinsics(
    source: &str,
    width: Option<u32>,
    bindings: &[&str],
) -> Result<nir::program::Package, String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "luna-target-{}-{}.fl",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, source).unwrap();
    let luna::cli::Command::Compile(options) =
        luna::cli::parse([path.clone().into_os_string(), "--emit".into(), "nir".into()]).unwrap()
    else {
        panic!("expected compile command")
    };
    let bindings: Vec<_> = bindings
        .iter()
        .map(|binding| {
            let (operation, selector) = binding.split_once('=').unwrap();
            (
                nir::intrinsics::Operation::from_name(operation).unwrap(),
                selector.to_owned(),
            )
        })
        .collect();
    let result = frontend::compile_bound_for_target(
        &options,
        0,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            host_intrinsics: &bindings,
            ..Default::default()
        },
        width.map(|width| TargetData::new(width).unwrap()),
    )
    .map(|(artifact, _)| artifact);
    std::fs::remove_file(path).unwrap();
    match result? {
        Artifact::Nir(package) => Ok(package),
        _ => panic!("expected NIR"),
    }
}

#[test]
fn source_platform_literals_use_explicit_target_and_retain_identity() {
    for (source, expected) in [
        (
            "typealias U=.usize'builtin; fn run()->U {4294967296}",
            comptime_interpretor::Value::UnsignedInteger(1u128 << 32),
        ),
        (
            "typealias I=.isize'builtin; fn run()->I { -2147483649 }",
            comptime_interpretor::Value::SignedInteger(-2147483649),
        ),
    ] {
        let error = compile(source, None).unwrap_err();
        assert!(error.contains("pointer-width"), "{error}");
        assert!(compile(source, Some(32)).is_err());
        let package = compile(source, Some(64)).unwrap();
        assert_eq!(package.target, Some(TargetData::new(64).unwrap()));
        let entry = &package.functions[0];
        assert!(matches!(
            entry.body.ty,
            nir::Term::Primitive(nir::Primitive::Usize | nir::Primitive::Isize)
        ));
        let mut vm = comptime_interpretor::Interpreter::new(&package, Default::default()).unwrap();
        assert_eq!(vm.invoke(entry.declaration, vec![]).unwrap(), expected);
        let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
        assert_eq!(graph.target, package.target);
        let mut vm =
            comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
        assert_eq!(
            vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
            expected
        );
    }
}

#[test]
fn source_platform_literal_bounds_reject_without_truncation() {
    for (name, width, spelling, valid) in [
        ("usize", 32, "4294967295", true),
        ("usize", 32, "4294967296", false),
        ("isize", 32, "-2147483648", true),
        ("isize", 32, "2147483648", false),
        ("usize", 64, "18446744073709551615", true),
        ("usize", 64, "18446744073709551616", false),
        ("isize", 64, "-9223372036854775808", true),
        ("isize", 64, "-9223372036854775809", false),
    ] {
        let source = format!("typealias Word=.{name}'builtin; fn run()->Word {{ {spelling} }}");
        assert_eq!(
            compile(&source, Some(width)).is_ok(),
            valid,
            "{name}/{width}: {spelling}"
        );
    }
}

#[test]
fn source_platform_comparison_and_arithmetic_execute_in_iir() {
    for (body, expected) in [
        (
            "fn run()->bool { let a:Word=7; a < 9 }",
            comptime_interpretor::Value::Bool(true),
        ),
        (
            "fn run()->Word { let a:Word=20; a + 22 }",
            comptime_interpretor::Value::UnsignedInteger(42),
        ),
    ] {
        let source = format!("typealias Word=.usize'builtin; {body}");
        for width in [32, 64] {
            let package = compile(&source, Some(width)).unwrap();
            let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
            let mut vm =
                comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
            assert_eq!(
                vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
                expected
            );
        }
    }
}

#[test]
fn source_platform_overflow_requires_target_specific_proof() {
    let source = "typealias Word=.usize'builtin; fn run()->Word { let a:Word=4294967295; a + 1 }";
    assert!(compile(source, Some(32)).is_err());
    let package = compile(source, Some(64)).unwrap();
    let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        comptime_interpretor::Value::UnsignedInteger(1u128 << 32)
    );
}

#[test]
fn platform_parameter_domains_supply_bounds_but_do_not_prove_overflow_safe() {
    for width in [32, 64] {
        let safe = "typealias Word=.usize'builtin; fn next(a:Word)->Word where requires a < 100 { a + 1 } fn run()->Word { next(41) }";
        let package = compile(safe, Some(width)).unwrap();
        let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
        let mut vm =
            comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
        assert_eq!(
            vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
            comptime_interpretor::Value::UnsignedInteger(42)
        );
        // A type's full domain includes its maximum; it cannot justify adding one.
        assert!(
            compile(
                "typealias Word=.usize'builtin; fn next(a:Word)->Word { a + 1 }",
                Some(width)
            )
            .is_err()
        );
        let facts = "typealias Word=.usize'builtin; fn identity(a:Word)->r:Word where ensures r >= 0 { a } fn run()->Word { identity(42) }";
        let package = compile(facts, Some(width)).unwrap();
        let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
        let mut vm =
            comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
        assert_eq!(
            vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
            comptime_interpretor::Value::UnsignedInteger(42)
        );
    }
}

#[test]
fn platform_integer_llvm_emission_checks_toolchain_target() {
    let toolchain = luna::host::Toolchain::default();
    let layout = toolchain.target_layout().unwrap();
    let source = "typealias Word=.usize'builtin; fn next(a:Word)->Word where requires a < 100 { a + 1 } fn run()->Word { next(41) }";
    let package = compile(source, Some(layout.pointer_bits)).unwrap();
    let ir = luna::host::llvm_ir(&package, "run", 100_000).unwrap();
    assert!(ir.contains("!flurry.target"));
    for optimization in [
        luna::host::OptimizationLevel::O0,
        luna::host::OptimizationLevel::O3,
    ] {
        let toolchain = luna::host::Toolchain {
            optimization,
            ..toolchain.clone()
        };
        let optimized = toolchain.optimized_ir(&ir).unwrap();
        assert!(optimized.contains("!flurry.target"));
        let bytes = luna::host::executable(&optimized, &toolchain).unwrap();
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!(
            "luna-platform-native-{}-{optimization:?}",
            std::process::id()
        ));
        std::fs::write(&path, bytes).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let output = std::process::Command::new(&path).output().unwrap();
        std::fs::remove_file(path).unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"(unsigned-integer 42)\n");
    }
    let other = if layout.pointer_bits == 32 { 64 } else { 32 };
    let package = compile(source, Some(other)).unwrap();
    let ir = luna::host::llvm_ir(&package, "run", 100_000).unwrap();
    assert!(
        toolchain
            .optimized_ir(&ir)
            .unwrap_err()
            .contains("pointer width mismatch")
    );
    assert!(
        luna::host::executable(&ir, &toolchain)
            .unwrap_err()
            .contains("pointer width mismatch")
    );
}

#[test]
fn platform_bit_representation_uses_target_width_and_signedness() {
    for width in [32, 64] {
        let source = format!(
            r#"
typealias U = .usize'builtin;
typealias I = .isize'builtin;
typealias Bits = .BitVec'builtin<{width}>;
pure fn signed(value: I) -> I {{ value'bitvec.as(I) }}
pure fn unsigned(value: U) -> U {{ value'bitvec.as(U) }}
pure fn update(value: U) -> U {{
    var word: U = value;
    word'bitvec[0..4] = 0b1010;
    word
}}
fn run() -> bool {{
    signed(-1) == -1 and unsigned(42) == 42 and update(16) == 26
        and (do {{ let bits: Bits = 0b1010; bits }}).as(U) == 10
}}
"#
        );
        let package = compile(&source, Some(width)).unwrap();
        let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
        let mut vm =
            comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
        assert_eq!(
            vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
            comptime_interpretor::Value::Bool(true)
        );
        assert!(compile(&source, None).is_err());
        let mismatched = format!(
            "typealias I=.isize'builtin; typealias B=.BitVec'builtin<{}>; fn run()->I {{ let bits:B=0; bits.as(I) }}",
            width / 2
        );
        assert!(
            compile(&mismatched, Some(width))
                .unwrap_err()
                .contains("exactly the target width")
        );
    }
}

#[test]
fn platform_enum_encoding_checks_base_payload_and_target() {
    let source = r#"
typealias U = .usize'builtin;
typealias I = .isize'builtin;
enum Packet { .base_type U, .discrete_tag true, data: 0b(value: I), }
fn run() -> U { let packet = Packet.data { value: -1 }; packet'bitvec.as(U) }
"#;
    for width in [32, 64] {
        let package = compile(source, Some(width)).unwrap();
        let expected = comptime_interpretor::Value::UnsignedInteger((1u128 << width) - 1);
        let mut vm = comptime_interpretor::Interpreter::new(&package, Default::default()).unwrap();
        assert_eq!(
            vm.invoke(package.functions[0].declaration, vec![]).unwrap(),
            expected
        );
        let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
        let mut vm =
            comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
        assert_eq!(
            vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
            expected
        );
        let mut wrong = graph.clone();
        wrong.target = Some(TargetData::new(if width == 32 { 64 } else { 32 }).unwrap());
        assert!(wrong.validate_index().is_err());
        let mut payload_only = package.enums[0].clone();
        payload_only.encoding.as_mut().unwrap().base_type = None;
        assert!(payload_only.validate_encoding_in(package.target).is_ok());
        assert!(payload_only.validate_encoding_in(wrong.target).is_err());
        let mut wrong = package.clone();
        wrong.target = None;
        assert!(comptime_interpretor::Interpreter::new(&wrong, Default::default()).is_err());
        let conflict = source.replace(
            ".base_type U,",
            &format!(".base_type U, .width {},", width / 2),
        );
        assert!(
            compile(&conflict, Some(width))
                .unwrap_err()
                .contains("conflicts")
        );
    }
    assert!(compile(source, None).is_err());
}

#[test]
fn platform_bit_pattern_guards_prove_only_complete_partitions() {
    for width in [32, 64] {
        for (primitive, special) in [("usize", "0"), ("isize", "-1")] {
            let source = format!(
                r#"
typealias Word=.{primitive}'builtin;
typealias Bits=.BitVec'builtin<{width}>;
fn classify(bits: Bits)->bool {{ bits match {{
    0b(value: Word) if value == {special} => true,
    0b(value: Word) if value != {special} => false,
}} }}
fn run()->bool {{
    let word: Word = {special};
    classify(word'bitvec) and not classify(42)
}}
"#
            );
            let package = compile(&source, Some(width)).unwrap();
            let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
            let mut vm =
                comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
            assert_eq!(
                vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
                comptime_interpretor::Value::Bool(true)
            );
            let incomplete = source.replace(
                &format!("value != {special}"),
                &format!("value == {special}"),
            );
            assert!(
                compile(&incomplete, Some(width))
                    .unwrap_err()
                    .contains("exhaustive")
            );
        }
    }
}

#[test]
fn platform_range_patterns_use_finite_target_domains() {
    for width in [32, 64] {
        for (primitive, low, high) in [
            ("usize", "0".to_string(), ((1u128 << width) - 1).to_string()),
            (
                "isize",
                (-(1i128 << (width - 1))).to_string(),
                ((1i128 << (width - 1)) - 1).to_string(),
            ),
        ] {
            let source = format!(
                r#"
typealias Word=.{primitive}'builtin;
fn classify(word: Word)->bool {{ word match {{
    {low}..10 => true,
    10..={high} => false,
}} }}
fn run()->bool {{ classify({low}) and classify(9) and not classify(10) and not classify({high}) }}
"#
            );
            if width == 32 {
                assert!(
                    compile(&source, Some(64))
                        .unwrap_err()
                        .contains("exhaustive")
                );
            }
            assert!(compile(&source, None).is_err());
            let package = compile(&source, Some(width)).unwrap();
            let graph = luna::instantiation::instantiate(&package, "run", 100_000).unwrap();
            let mut vm =
                comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
            assert_eq!(
                vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
                comptime_interpretor::Value::Bool(true)
            );
            for incomplete in [
                source.replace("10..=", "11..="),
                source.replace(&format!("..={high}"), &format!("..{high}")),
            ] {
                assert!(
                    compile(&incomplete, Some(width))
                        .unwrap_err()
                        .contains("exhaustive")
                );
            }
            let complement = source.replace(&format!("10..={high}"), &format!("not ({low}..10)"));
            assert!(compile(&complement, Some(width)).is_ok());
        }
    }
}

#[test]
fn platform_normal_form_inputs_preserve_opaque_heads() {
    let source = "typealias U=.usize'builtin; typealias Type=.Type'builtin; pure comptime fn Tagged<n:U>->Type {bool} typealias Selected=Tagged<4294967296>; comptime fn Get()->Type {Selected} comptime fn run()->bool {Get()==Tagged<4294967296>} fn retained(value:Selected)->Selected {value}";
    assert!(compile(source, None).is_err());
    assert!(compile(source, Some(32)).is_err());
    let package = compile(source, Some(64)).unwrap();
    assert!(package.normal_forms.iter().any(|head| {
        head.parameters
            .iter()
            .any(|parameter| parameter.ty == nir::Term::Primitive(nir::Primitive::Usize))
    }));
    assert_eq!(
        luna::invocation::invoke(&package, "run", 100000).unwrap(),
        comptime_interpretor::Value::Bool(true)
    );
    let graph = luna::instantiation::instantiate(&package, "retained", 100000).unwrap();
    comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert!(graph.to_string().contains("normal-form"));
    let nir::Term::Function { parameters, result } = &graph.functions[0].function.signature else {
        panic!("not a function")
    };
    assert_eq!(&parameters[0].ty, result.as_ref());
    assert!(matches!(result.as_ref(), nir::Term::NormalForm { .. }));
}

#[test]
fn platform_nominal_indices_construct_and_retain_identity() {
    let source = "typealias U=.usize'builtin; struct Tagged where n:U {value:U} fn run()->U {let item=Tagged<4294967296> {value:42};item.value}";
    assert!(compile(source, None).is_err());
    assert!(compile(source, Some(32)).is_err());
    let package = compile(source, Some(64)).unwrap();
    let graph = luna::instantiation::instantiate(&package, "run", 100000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        comptime_interpretor::Value::UnsignedInteger(42)
    );
}

#[test]
fn platform_newtype_indices_preserve_representation() {
    let source = "typealias U=.usize'builtin; newtype Key<n:U> = U; fn run()->U {let value:U=42;value.as(Key<4294967296>).as(U)}";
    assert!(compile(source, Some(32)).is_err());
    let package = compile(source, Some(64)).unwrap();
    let graph = luna::instantiation::instantiate(&package, "run", 100000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        comptime_interpretor::Value::UnsignedInteger(42)
    );
}

#[test]
fn platform_trait_value_indices_select_methods() {
    let source = "typealias U=.usize'builtin; trait Read where tag:U {fn read()->U;} impl Read<tag> for bool where tag:U {fn read()->U {tag}} fn run()->U {bool.as(Read<4294967296>).read()}";
    assert!(compile(source, Some(32)).is_err());
    let package = compile(source, Some(64)).unwrap();
    let graph = luna::instantiation::instantiate(&package, "run", 100000).unwrap();
    let mut vm = comptime_interpretor::Interpreter::from_iir(&graph, Default::default()).unwrap();
    assert_eq!(
        vm.invoke_instance(&graph.roots[0], vec![]).unwrap(),
        comptime_interpretor::Value::UnsignedInteger(1u128 << 32)
    );
}

#[test]
fn platform_comparison_declarations_do_not_choose_a_target_width() {
    for primitive in ["usize", "isize"] {
        for operator in ["<", "<=", ">", ">=", "==", "!="] {
            let source = format!(
                "typealias Word=.{primitive}'builtin; fn compare(a:Word,b:Word)->bool {{a {operator} b}}"
            );
            let package = compile(&source, None).unwrap();
            assert_eq!(package.target, None);
            let nir::Term::Function { parameters, .. } = &package.functions[0].signature else {
                panic!("function signature")
            };
            assert_eq!(
                parameters[0].ty,
                nir::Term::Primitive(if primitive == "usize" {
                    nir::Primitive::Usize
                } else {
                    nir::Primitive::Isize
                })
            );
            // Declaration acceptance must not silently use this machine's width.
            if let Ok(mut vm) = comptime_interpretor::Interpreter::new(&package, Default::default())
            {
                let value = if primitive == "usize" {
                    comptime_interpretor::Value::UnsignedInteger(1)
                } else {
                    comptime_interpretor::Value::SignedInteger(1)
                };
                assert!(
                    vm.invoke(package.functions[0].declaration, vec![value.clone(), value])
                        .is_err()
                );
            }
        }
    }
}

#[test]
fn repository_platform_comparison_protocols_follow_the_selected_target() {
    for (width, unsigned_max, signed_min, signed_max) in [
        (32, "4294967295", "-2147483648", "2147483647"),
        (
            64,
            "18446744073709551615",
            "-9223372036854775808",
            "9223372036854775807",
        ),
    ] {
        let source = format!("{}\n{}\n
            fn run() {{
                let low:.isize'builtin={signed_min};
                let high:.isize'builtin={signed_max};
                let zero:.usize'builtin=0;
                let maximum:.usize'builtin={unsigned_max};
                let signed = low.as(Ord).lt(high) and high.as(Ord).gt(low); assumes signed;
                let equal = high.as(Eq).eq(high) and low.as(Ord).le(low); assumes equal;
                let unsigned = zero.as(Ord).lt(maximum) and maximum.as(Ord).ge(zero); assumes unsigned;
                let different = maximum.as(Eq).ne(zero); assumes different;
                let less = low.as(Ord).cmp(high) matches .less; assumes less;
                let greater = maximum.as(Ord).cmp(zero) matches .greater; assumes greater;
                let same = maximum.as(Ord).cmp(maximum) matches .equal; assumes same;
            }}",
            library::with_attrs(include_str!("../../../library/builtin/comparison.fl")),
            include_str!("../../../library/builtin/resources.fl"));
        let package = compile(&source, Some(width)).unwrap();
        assert_eq!(
            luna::execution::interpret_with_protocols(&package, "run", "Copy", "Drop", 100_000)
                .unwrap(),
            comptime_interpretor::Value::Unit
        );
        let tools = luna::host::Toolchain::default();
        if tools
            .target_layout()
            .unwrap()
            .semantic_target()
            .unwrap()
            .pointer_bits()
            == width
        {
            let copy = package
                .traits
                .iter()
                .find(|t| t.name == "Copy")
                .unwrap()
                .declaration;
            let drop = package
                .traits
                .iter()
                .find(|t| t.name == "Drop")
                .unwrap()
                .declaration;
            let prepared =
                luna::host::prepare_with_protocols(&package, "run", 100_000, copy, drop).unwrap();
            let ir = luna::host::llvm_ir_prepared(&prepared, 100_000).unwrap();
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
        if width == 64 {
            assert!(compile(&source, Some(32)).is_err());
        }
    }
}

#[test]
fn generic_platform_literal_formation_uses_each_package_target() {
    let source =
        "fn large()->T where T, requires T == .usize'builtin or T == .u64'builtin {4294967296}
        fn run()->.usize'builtin {large<.usize'builtin>()}
        fn fixed()->.u64'builtin {large<.u64'builtin>()}";
    for width in [32, 64] {
        let package = compile(source, Some(width)).unwrap();
        assert_eq!(
            luna::execution::interpret(&package, "fixed", 100000).unwrap(),
            comptime_interpretor::Value::UnsignedInteger(4294967296)
        );
        if width == 32 {
            let error = luna::instantiation::instantiate(&package, "run", 100000).unwrap_err();
            assert!(error.contains("integer formation"), "{error}");
        } else {
            assert_eq!(
                luna::execution::interpret(&package, "run", 100000).unwrap(),
                comptime_interpretor::Value::UnsignedInteger(4294967296)
            );
        }
    }
}

fn generic_platform_comparisons() -> String {
    [("lt","<"),("le","<="),("gt",">"),("ge",">="),("eq","=="),("ne","!=")]
        .into_iter().map(|(name,op)|format!("pure fn {name}(a:T,b:T)->bool where T,
            requires T == .usize'builtin or T == .isize'builtin or T == .u64'builtin or T == .i64'builtin {{a {op} b}}\n"))
        .collect()
}

#[test]
fn generic_platform_comparison_declarations_keep_target_unspecified() {
    let source = format!(
        "{} fn compare(a:.usize'builtin,b:.usize'builtin)->bool {{lt<.usize'builtin>(a,b)}}",
        generic_platform_comparisons()
    );
    let package = compile(&source, None).unwrap();
    assert_eq!(package.target, None);
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "compare")
        .unwrap();
    let nir::Term::Function { parameters, .. } = &function.signature else {
        panic!("signature")
    };
    assert_eq!(
        parameters[0].ty,
        nir::Term::Primitive(nir::Primitive::Usize)
    );
    if let Ok(mut vm) = comptime_interpretor::Interpreter::new(&package, Default::default()) {
        assert!(
            vm.invoke(
                function.declaration,
                vec![
                    comptime_interpretor::Value::UnsignedInteger(1),
                    comptime_interpretor::Value::UnsignedInteger(2)
                ]
            )
            .is_err()
        );
    }
    for width in [32, 64] {
        let package = compile(&source, Some(width)).unwrap();
        let function = package
            .functions
            .iter()
            .find(|f| f.name == "compare")
            .unwrap();
        let mut vm = comptime_interpretor::Interpreter::new(&package, Default::default()).unwrap();
        assert_eq!(
            vm.invoke(
                function.declaration,
                vec![
                    comptime_interpretor::Value::UnsignedInteger(1),
                    comptime_interpretor::Value::UnsignedInteger(2)
                ]
            )
            .unwrap(),
            comptime_interpretor::Value::Bool(true),
        );
    }
}

#[test]
fn generic_platform_comparisons_follow_the_selected_target() {
    for (width, maximum, minimum, high) in [
        (32, "4294967295", "-2147483648", "2147483647"),
        (
            64,
            "18446744073709551615",
            "-9223372036854775808",
            "9223372036854775807",
        ),
    ] {
        let source=format!("{}
            fn run() {{
                let low:.isize'builtin={minimum};let high:.isize'builtin={high};
                let zero:.usize'builtin=0;let maximum:.usize'builtin={maximum};
                let signed=lt<.isize'builtin>(low,high) and le<.isize'builtin>(low,low) and gt<.isize'builtin>(high,low) and ge<.isize'builtin>(high,high);assumes signed;
                let unsigned=lt<.usize'builtin>(zero,maximum) and le<.usize'builtin>(maximum,maximum) and gt<.usize'builtin>(maximum,zero) and ge<.usize'builtin>(zero,zero);assumes unsigned;
                let identities=eq<.isize'builtin>(low,low) and ne<.isize'builtin>(low,high) and eq<.usize'builtin>(maximum,maximum) and ne<.usize'builtin>(zero,maximum);assumes identities;
                let fixed=lt<.i64'builtin>(-1,0) and gt<.u64'builtin>(18446744073709551615,0);assumes fixed;
            }}",generic_platform_comparisons());
        let package = compile(&source, Some(width)).unwrap();
        assert_eq!(
            luna::execution::interpret(&package, "run", 100000).unwrap(),
            comptime_interpretor::Value::Unit
        );
        let tools = luna::host::Toolchain::default();
        if tools
            .target_layout()
            .unwrap()
            .semantic_target()
            .unwrap()
            .pointer_bits()
            == width
        {
            let ir = luna::host::llvm_ir(&package, "run", 100000).unwrap();
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
        if width == 64 {
            assert!(compile(&source, Some(32)).is_err());
        }
    }
}

#[test]
fn generic_platform_arithmetic_uses_explicit_target_bounds() {
    let source = "
        pure fn calculate(value:T)->T where T,
            requires T == .usize'builtin or T == .isize'builtin,
            requires value >= 1 and value <= 10 {
            (value + 2) * 3 - 1
        }
        pure fn quotient(value:T)->T where T,
            requires T == .usize'builtin or T == .u64'builtin {
            value / 3 + value % 3
        }
        pure fn negate(value:T)->T where T,
            requires T == .isize'builtin or T == .i64'builtin,
            requires value >= -100 and value <= 100 { -value }
        fn run() {
            let valid = calculate<.usize'builtin>(10) == 35
            and calculate<.isize'builtin>(1) == 8
            and quotient<.usize'builtin>(20) == 8
            and quotient<.u64'builtin>(20) == 8
            and negate<.isize'builtin>(-100) == 100
            and negate<.i64'builtin>(100) == -100;
            assumes valid;
        }";
    assert!(compile(source, None).is_err());
    for width in [32, 64] {
        let package = compile(source, Some(width)).unwrap();
        assert_eq!(
            luna::execution::interpret(&package, "run", 100000).unwrap(),
            comptime_interpretor::Value::Unit
        );
        let tools = luna::host::Toolchain::default();
        if tools.target_layout().unwrap().pointer_bits == width {
            let ir = luna::host::llvm_ir(&package, "run", 100000).unwrap();
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
}

#[test]
fn generic_platform_arithmetic_checks_unused_definitions_for_each_target() {
    let source = "pure fn next(value:T)->T where T,
        requires T == .usize'builtin or T == .u64'builtin,
        requires value <= 4294967295 { value + 1 }
        fn run()->bool { true }";
    assert!(compile(source, Some(64)).is_ok());
    assert!(compile(source, Some(32)).is_err());
    assert!(compile(source, None).is_err());
    for width in [32, 64] {
        for body in [
            "pure fn bad(value:T)->T where T, requires T == .usize'builtin or T == .u64'builtin {value + 1}",
            "pure fn bad(value:T)->T where T, requires T == .isize'builtin or T == .i64'builtin {- value}",
            "pure fn bad(value:T)->T where T, requires T == .isize'builtin or T == .i64'builtin {value / 3}",
            "pure fn bad(value:T)->T where T, requires T == .usize'builtin or T == .u64'builtin {value / 0}",
        ] {
            assert!(
                compile(&format!("{body} fn run()->bool {{true}}"), Some(width)).is_err(),
                "accepted {body} at {width}"
            );
        }
    }
}

#[test]
fn platform_bitwise_intrinsics_keep_unspecified_layout_until_consumed() {
    let source = "^intrinsic_enum enum Ops {bits}
        fn combine(a:.usize'builtin,b:.usize'builtin)->.usize'builtin {
            unsafe {Ops.bits'intrinsic(a,b).as(.usize'builtin)}
        }";
    let package = compile_with_intrinsics(source, None, &["bit_and=input.Ops.bits"]).unwrap();
    assert_eq!(package.target, None);
    let mut found = false;
    for function in &package.functions {
        nir::visit::expressions(function, &mut 100000, |expr, _| {
            if matches!(expr.kind, nir::program::ExprKind::Intrinsic { .. }) {
                let checked = nir::intrinsics::checked(expr).unwrap();
                assert_eq!(checked.width(), None);
                assert_eq!(expr.ty, nir::Term::Primitive(nir::Primitive::Usize));
                found = true;
            }
            Ok(())
        })
        .unwrap();
    }
    assert!(found);
}

#[test]
fn platform_bitwise_intrinsics_use_package_target_for_proofs_and_native_layout() {
    let bindings = [
        "bit_and=input.Ops.band",
        "bit_or=input.Ops.bor",
        "bit_xor=input.Ops.bxor",
    ];
    for (width, maximum, minimum, signed_max) in [
        (32, "4294967295", "-2147483648", "2147483647"),
        (
            64,
            "18446744073709551615",
            "-9223372036854775808",
            "9223372036854775807",
        ),
    ] {
        let source=format!("^intrinsic_enum enum Ops {{band,bor,bxor}}
            typealias Word=.usize'builtin;typealias Signed=.isize'builtin;
            pure fn mix(a:T,b:T)->T where T,requires T==Word or T==Signed {{unsafe {{Ops.bxor'intrinsic(a,b).as(T)}}}}
            fn low(a:Word)->r:Word where ensures r<=15 {{let mask:Word=15;unsafe {{Ops.band'intrinsic(a,mask).as(Word)}}}}
            fn fill(a:Signed)->r:Signed where ensures r>=-16 and r<=-1 {{let mask:Signed=-16;unsafe {{Ops.bor'intrinsic(a,mask).as(Signed)}}}}
            fn invert()->r:Signed where ensures r=={signed_max} {{let a:Signed={minimum};let b:Signed=-1;unsafe {{Ops.bxor'intrinsic(a,b).as(Signed)}}}}
            fn run() {{
                let unsigned=mix<Word>({maximum},0)=={maximum};assumes unsigned;
                let signed=mix<Signed>({minimum},-1)=={signed_max};assumes signed;
                let masked=low({maximum})==15;assumes masked;
                let filled=fill({minimum})==-16;assumes filled;
                let inverted=invert()=={signed_max};assumes inverted;
            }}");
        assert!(compile_with_intrinsics(&source, None, &bindings).is_err());
        let package = compile_with_intrinsics(&source, Some(width), &bindings).unwrap();
        let mut without_target = package.clone();
        without_target.target = None;
        assert!(luna::host::llvm_ir(&without_target, "run", 100000).is_err());
        let graph = luna::instantiation::instantiate(&package, "run", 100000).unwrap();
        let mut count = 0;
        for instance in &graph.functions {
            nir::visit::expressions(&instance.function, &mut 100000, |expr, _| {
                if matches!(expr.kind, nir::program::ExprKind::Intrinsic { .. }) {
                    let checked = nir::intrinsics::checked(expr).unwrap();
                    assert_eq!(checked.width(), None);
                    assert_eq!(checked.width_for_target(graph.target), Some(width));
                    assert_eq!(
                        checked.width_for_target(Some(
                            TargetData::new(if width == 32 { 64 } else { 32 }).unwrap()
                        )),
                        Some(if width == 32 { 64 } else { 32 })
                    );
                    assert!(matches!(
                        expr.ty,
                        nir::Term::Primitive(nir::Primitive::Usize | nir::Primitive::Isize)
                    ));
                    count += 1;
                }
                Ok(())
            })
            .unwrap();
        }
        assert!(count >= 4);
        let ir = luna::host::llvm_ir(&package, "run", 100000).unwrap();
        assert!(ir.contains(&format!("xor i{width} ")));
        let tools = luna::host::Toolchain::default();
        if tools.target_layout().unwrap().pointer_bits == width {
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
        if width == 64 {
            assert!(compile_with_intrinsics(&source, Some(32), &bindings).is_err());
            let mut wrong_target = package.clone();
            wrong_target.target = Some(TargetData::new(32).unwrap());
            assert!(luna::host::llvm_ir(&wrong_target, "run", 100000).is_err());
        }
    }
}

#[test]
fn platform_bit_counts_reversal_and_funnels_use_target_layout() {
    let bindings: Vec<String> = nir::intrinsics::Operation::ALL
        .iter()
        .map(|operation| {
            format!(
                "{}=input.math.target.Scalar.{}",
                operation.name(),
                operation.name()
            )
        })
        .collect();
    let bindings: Vec<&str> = bindings.iter().map(String::as_str).collect();
    for width in [32, 64] {
        let high = 1u128 << (width - 1);
        let maximum = (1u128 << width) - 1;
        let source = format!(
            "mod math {{\n{}\n}}
            typealias Word=.usize'builtin;
            fn count(value:Word)->r:Word where ensures r<={width} {{
                unsafe {{math.target.Scalar.ctpop'intrinsic(value).as(Word)}}
            }}
            fn leading(value:Word)->r:Word where ensures r<={width} {{
                unsafe {{math.target.Scalar.ctlz'intrinsic(value).as(Word)}}
            }}
            fn trailing(value:Word)->r:Word where ensures r<={width} {{
                unsafe {{math.target.Scalar.cttz'intrinsic(value).as(Word)}}
            }}
            fn run() {{
                let a=count({maximum})=={width};assumes a;
                let b=leading(0)=={width};assumes b;
                let c=trailing(0)=={width};assumes c;
                let d=math.bit_reverse<Word>(1)=={high};assumes d;
                let e=math.count_ones<Word>({high})==1;assumes e;
                let f=math.leading_zeros<Word>(1)=={width} - 1;assumes f;
                let g=math.trailing_zeros<Word>({high})=={width} - 1;assumes g;
                let h=math.rotate_left<Word>({high},1)==1;assumes h;
                let i=math.rotate_right<Word>(1,1)=={high};assumes i;
                let j=math.funnel_shift_left<Word>(1,2,{width})==1;assumes j;
                let k=math.funnel_shift_right<Word>(1,2,{width})==2;assumes k;
                let l=math.funnel_shift_left<Word>(1,{high},{width} + 1)==3;assumes l;
                let m=math.funnel_shift_right<Word>(1,2,{width} + 1)=={high} + 1;assumes m;
            }}",
            include_str!("../../../library/builtin/math_intrinsics.fl")
        );
        assert!(compile_with_intrinsics(&source, None, &bindings).is_err());
        let package = compile_with_intrinsics(&source, Some(width), &bindings).unwrap();
        let ir = luna::host::llvm_ir(&package, "run", 100000).unwrap();
        for operation in ["ctpop", "ctlz", "cttz", "bitreverse", "fshl", "fshr"] {
            assert!(
                ir.contains(&format!("@llvm.{operation}.i{width}")),
                "{operation}"
            );
        }
        if luna::host::Toolchain::default()
            .target_layout()
            .unwrap()
            .pointer_bits
            == width
        {
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
        let invalid = format!("^intrinsic_enum enum Ops {{count}} typealias Word=.usize'builtin;
            fn wrong(a:Word)->r:Word where ensures r < {width} {{unsafe {{Ops.count'intrinsic(a).as(Word)}}}}");
        let error =
            compile_with_intrinsics(&invalid, Some(width), &["ctpop=input.Ops.count"]).unwrap_err();
        assert!(error.contains("postcondition is not proven"), "{error}");
    }
}

#[test]
fn platform_saturation_extrema_and_absolute_value_follow_selected_bounds() {
    let bindings: Vec<String> = nir::intrinsics::Operation::ALL
        .iter()
        .map(|op| format!("{}=input.math.target.Scalar.{}", op.name(), op.name()))
        .collect();
    let bindings: Vec<&str> = bindings.iter().map(String::as_str).collect();
    for width in [32, 64] {
        let maximum = (1u128 << width) - 1;
        let high = (1i128 << (width - 1)) - 1;
        let low = -(1i128 << (width - 1));
        let source = format!(
            "mod math {{\n{}\n}}
            typealias Word=.usize'builtin;typealias Signed=.isize'builtin;
            fn cap(a:Word,b:Word)->r:Word where ensures r<=a'old and r<=b'old {{
                unsafe {{math.target.Scalar.min'intrinsic(a,b).as(Word)}}
            }}
            fn floor(a:Signed,b:Signed)->r:Signed where ensures r>=a'old and r>=b'old {{
                unsafe {{math.target.Scalar.max'intrinsic(a,b).as(Signed)}}
            }}
            fn subtract(a:Word)->r:Word where ensures r==0 {{let b:Word={maximum};
                unsafe {{math.target.Scalar.sub_sat'intrinsic(a,b).as(Word)}}
            }}
            fn run() {{
                let a=math.saturating_add<Word>({maximum},1)=={maximum};assumes a;
                let b=subtract(42)==0;assumes b;
                let c=math.saturating_add<Signed>({low},-1)=={low};assumes c;
                let d=math.saturating_add<Signed>({high},1)=={high};assumes d;
                let e=math.saturating_sub<Signed>({high},-1)=={high};assumes e;
                let f=math.saturating_sub<Signed>({low},1)=={low};assumes f;
                let g=math.min<Word>({maximum},1)==1;assumes g;
                let h=math.max<Word>({maximum},1)=={maximum};assumes h;
                let i=math.min<Signed>({low},{high})=={low};assumes i;
                let j=math.max<Signed>({low},{high})=={high};assumes j;
                let k=math.wrapping_abs<Signed>({low})=={low};assumes k;
                let l=math.wrapping_abs<Signed>(-42)==42;assumes l;
                let m=cap({maximum},1)==1;assumes m;
                let n=floor({low},{high})=={high};assumes n;
            }}",
            include_str!("../../../library/builtin/math_intrinsics.fl")
        );
        assert!(compile_with_intrinsics(&source, None, &bindings).is_err());
        let package = compile_with_intrinsics(&source, Some(width), &bindings).unwrap();
        let ir = luna::host::llvm_ir(&package, "run", 1000000).unwrap();
        for operation in [
            "uadd.sat", "usub.sat", "sadd.sat", "ssub.sat", "umin", "umax", "smin", "smax", "abs",
        ] {
            assert!(
                ir.contains(&format!("@llvm.{operation}.i{width}")),
                "{operation}"
            );
        }
        if luna::host::Toolchain::default()
            .target_layout()
            .unwrap()
            .pointer_bits
            == width
        {
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
}

#[test]
fn generic_platform_program_proofs_cover_all_layouts_without_selecting_one() {
    let portable = "pure fn next(value:T)->r:T where T,
        requires T == .isize'builtin or T == .i32'builtin,
        requires value >= 0 and value <= 100,
        ensures r == value'old + 1 {value + 1}";
    let package = compile(portable, None).unwrap();
    assert_eq!(package.target, None);
    // A range safe on common 32/64-bit hosts still overflows the allowed
    // 8-bit layout. An unused definition must not evade universal checking.
    let narrow_overflow = portable.replace("<= 100", "<= 127");
    assert!(compile(&narrow_overflow, None).is_err());
    assert!(compile(&narrow_overflow, Some(32)).is_ok());
    assert!(compile(&narrow_overflow, Some(8)).is_err());
    // Ordinary non-generic value formation still requires an explicit layout.
    assert!(compile("fn run()->.usize'builtin {1}", None).is_err());
}

#[test]
fn platform_byte_swap_checks_layout_at_admission_and_after_specialization() {
    let declaration = "^intrinsic_enum enum Ops {swap}
        typealias Word=.usize'builtin;
        fn swap(a:Word)->Word {unsafe {Ops.swap'intrinsic(a).as(Word)}}";
    let bindings = ["bswap=input.Ops.swap"];
    let deferred = compile_with_intrinsics(declaration, None, &bindings).unwrap();
    assert_eq!(deferred.target, None);
    for width in (8..=128).step_by(8) {
        if width % 16 != 0 {
            let error = compile_with_intrinsics(declaration, Some(width), &bindings).unwrap_err();
            assert!(
                error.contains("bswap requires a positive multiple of 16 bits"),
                "{width}: {error}"
            );
            continue;
        }
        let reversed = 258u128.swap_bytes() >> (128 - width);
        let source = format!(
            "{declaration}
            fn run() {{let correct=swap(258)=={reversed};assumes correct;}}"
        );
        let package = compile_with_intrinsics(&source, Some(width), &bindings).unwrap();
        let ir = luna::host::llvm_ir(&package, "run", 100000).unwrap();
        assert!(ir.contains(&format!("@llvm.bswap.i{width}")));
        if luna::host::Toolchain::default()
            .target_layout()
            .unwrap()
            .pointer_bits
            == width
        {
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
        let mut changed = package.clone();
        changed.target = Some(TargetData::new(width - 8).unwrap());
        assert!(
            nir::phases::validate(&changed)
                .unwrap_err()
                .message
                .contains("bswap requires")
        );
        assert!(luna::host::llvm_ir(&changed, "run", 100000).is_err());
    }
    // Generic scalar admission must not invent layout evidence. Even if the
    // template passes source checking, its executable instance must reject i8.
    let generic = "^intrinsic_enum enum Ops {swap}
        pure fn swap(a:T)->T where T, requires T == .usize'builtin or T == .u16'builtin {
            unsafe {Ops.swap'intrinsic(a).as(T)}
        }
        fn run()->.usize'builtin {swap<.usize'builtin>(1)}";
    match compile_with_intrinsics(generic, Some(8), &bindings) {
        Ok(package) => {
            let error = luna::host::llvm_ir(&package, "run", 100000).unwrap_err();
            assert!(error.contains("bswap requires"), "{error}");
        }
        Err(error) => assert!(error.contains("bswap requires"), "{error}"),
    }
}
