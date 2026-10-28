#[path = "support/execution.rs"]
#[allow(dead_code)]
mod support;
use luna::host::intrinsic::{Error, Operation, Registry, Tag};
use nir::program::{Expr, ExprKind as K, SourceRange};
use nir::{Primitive as P, Term};

fn bind_fma(package: nir::program::Package) -> nir::program::Package {
    let tag = tag(&package, "Ops", 0);
    let registry = Registry::new(&package, &[(tag, Operation::FusedMultiplyAdd)]).unwrap();
    luna::host::intrinsic::bind(package, &registry, &mut 100000).unwrap()
}

#[test]
fn repository_math_generic_wrapper_specializes_both_float_widths() {
    let source=format!("mod math {{
{}
}} fn min_single()->bool {{math.minimumnum<.f32'builtin>(3.0,-2.0)==-2.0}} fn min_double()->bool {{math.minimumnum<.f64'builtin>(3.0,-2.0)==-2.0}} fn max_single()->bool {{math.maximumnum<.f32'builtin>(3.0,-2.0)==3.0}} fn max_double()->bool {{math.maximumnum<.f64'builtin>(3.0,-2.0)==3.0}} fn abs_single()->bool {{math.abs<.f32'builtin>(-3.5)==3.5}} fn abs_double()->bool {{math.abs<.f64'builtin>(-3.5)==3.5}} fn single()->.f32'builtin {{math.fma<.f32'builtin>(3.0,4.0,5.0)}} fn double()->.f64'builtin {{math.fma<.f64'builtin>(3.0,4.0,5.0)}}",include!("support/math_source.rs"));
    let package = support::compile_with_source_bindings(
        &source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            host_fma: Some("input.math.target.Scalar.fma"),
            host_sqrt: Some("input.math.target.Scalar.sqrt"),
            host_copysign: Some("input.math.target.Scalar.copysign"),
            host_intrinsics: &[
                (
                    Operation::AbsoluteValue,
                    "input.math.target.Scalar.fabs".into(),
                ),
                (Operation::Floor, "input.math.target.Scalar.floor".into()),
                (Operation::Ceiling, "input.math.target.Scalar.ceil".into()),
                (Operation::Truncate, "input.math.target.Scalar.trunc".into()),
                (
                    Operation::RoundEven,
                    "input.math.target.Scalar.roundeven".into(),
                ),
                (
                    Operation::RoundAway,
                    "input.math.target.Scalar.round".into(),
                ),
                (Operation::Sine, "input.math.target.Scalar.sin".into()),
                (Operation::Cosine, "input.math.target.Scalar.cos".into()),
                (
                    Operation::Exponential,
                    "input.math.target.Scalar.exp".into(),
                ),
                (
                    Operation::Exponential2,
                    "input.math.target.Scalar.exp2".into(),
                ),
                (Operation::NaturalLog, "input.math.target.Scalar.log".into()),
                (Operation::Log2, "input.math.target.Scalar.log2".into()),
                (Operation::Log10, "input.math.target.Scalar.log10".into()),
                (Operation::Power, "input.math.target.Scalar.pow".into()),
                (
                    Operation::ScaleByPowerOfTwo,
                    "input.math.target.Scalar.ldexp".into(),
                ),
                (
                    Operation::IntegerPower,
                    "input.math.target.Scalar.powi".into(),
                ),
                (
                    Operation::Minimum,
                    "input.math.target.Scalar.minimum".into(),
                ),
                (
                    Operation::Maximum,
                    "input.math.target.Scalar.maximum".into(),
                ),
                (
                    Operation::MinimumNumber,
                    "input.math.target.Scalar.minimumnum".into(),
                ),
                (
                    Operation::MaximumNumber,
                    "input.math.target.Scalar.maximumnum".into(),
                ),
                (Operation::IsNan, "input.math.target.Scalar.isnan".into()),
                (
                    Operation::IsInfinite,
                    "input.math.target.Scalar.isinf".into(),
                ),
                (
                    Operation::IsFinite,
                    "input.math.target.Scalar.isfinite".into(),
                ),
                (
                    Operation::IsNormal,
                    "input.math.target.Scalar.isnormal".into(),
                ),
                (
                    Operation::IsSubnormal,
                    "input.math.target.Scalar.issubnormal".into(),
                ),
                (
                    Operation::SignBit,
                    "input.math.target.Scalar.signbit".into(),
                ),
                (
                    Operation::SaturatingAdd,
                    "input.math.target.Scalar.add_sat".into(),
                ),
                (
                    Operation::SaturatingSubtract,
                    "input.math.target.Scalar.sub_sat".into(),
                ),
                (
                    Operation::IntegerMinimum,
                    "input.math.target.Scalar.min".into(),
                ),
                (
                    Operation::IntegerMaximum,
                    "input.math.target.Scalar.max".into(),
                ),
                (Operation::BitAnd, "input.math.target.Scalar.bit_and".into()),
                (Operation::BitOr, "input.math.target.Scalar.bit_or".into()),
                (Operation::BitXor, "input.math.target.Scalar.bit_xor".into()),
                (
                    Operation::WrappingAbsoluteValue,
                    "input.math.target.Scalar.abs".into(),
                ),
                (
                    Operation::FunnelShiftLeft,
                    "input.math.target.Scalar.fshl".into(),
                ),
                (
                    Operation::FunnelShiftRight,
                    "input.math.target.Scalar.fshr".into(),
                ),
                (Operation::ByteSwap, "input.math.target.Scalar.bswap".into()),
                (
                    Operation::BitReverse,
                    "input.math.target.Scalar.bitreverse".into(),
                ),
                (
                    Operation::PopulationCount,
                    "input.math.target.Scalar.ctpop".into(),
                ),
                (
                    Operation::LeadingZeros,
                    "input.math.target.Scalar.ctlz".into(),
                ),
                (
                    Operation::TrailingZeros,
                    "input.math.target.Scalar.cttz".into(),
                ),
            ],
            ..Default::default()
        },
    )
    .unwrap();
    for (entry, symbol) in [
        ("single", "@llvm.fma.f32"),
        ("double", "@llvm.fma.f64"),
        ("abs_single", "@llvm.fabs.f32"),
        ("abs_double", "@llvm.fabs.f64"),
        ("min_single", "@llvm.minimum.f32"),
        ("min_double", "@llvm.minimum.f64"),
        ("max_single", "@llvm.maximum.f32"),
        ("max_double", "@llvm.maximum.f64"),
    ] {
        let ir = luna::host::llvm_ir(&package, entry, 100000).unwrap();
        assert!(ir.contains(symbol), "{ir}");
        luna::host::Toolchain::default()
            .execute_test(&ir, std::time::Duration::from_secs(5))
            .unwrap();
    }
}

#[test]
fn math_float_domain_rejects_nonfloat_instantiation() {
    let source = format!(
        "mod math {{\n{}\n}} fn run()->bool {{math.fma<bool>(true,true,true)}}",
        include!("support/math_source.rs")
    );
    let error = support::compile_with_source_bindings(
        &source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            host_fma: Some("input.math.target.Scalar.fma"),
            host_sqrt: Some("input.math.target.Scalar.sqrt"),
            host_copysign: Some("input.math.target.Scalar.copysign"),
            host_intrinsics: &[
                (
                    Operation::AbsoluteValue,
                    "input.math.target.Scalar.fabs".into(),
                ),
                (Operation::Floor, "input.math.target.Scalar.floor".into()),
                (Operation::Ceiling, "input.math.target.Scalar.ceil".into()),
                (Operation::Truncate, "input.math.target.Scalar.trunc".into()),
                (
                    Operation::RoundEven,
                    "input.math.target.Scalar.roundeven".into(),
                ),
                (
                    Operation::RoundAway,
                    "input.math.target.Scalar.round".into(),
                ),
                (Operation::Sine, "input.math.target.Scalar.sin".into()),
                (Operation::Cosine, "input.math.target.Scalar.cos".into()),
                (
                    Operation::Exponential,
                    "input.math.target.Scalar.exp".into(),
                ),
                (
                    Operation::Exponential2,
                    "input.math.target.Scalar.exp2".into(),
                ),
                (Operation::NaturalLog, "input.math.target.Scalar.log".into()),
                (Operation::Log2, "input.math.target.Scalar.log2".into()),
                (Operation::Log10, "input.math.target.Scalar.log10".into()),
                (Operation::Power, "input.math.target.Scalar.pow".into()),
                (
                    Operation::ScaleByPowerOfTwo,
                    "input.math.target.Scalar.ldexp".into(),
                ),
                (
                    Operation::IntegerPower,
                    "input.math.target.Scalar.powi".into(),
                ),
                (
                    Operation::Minimum,
                    "input.math.target.Scalar.minimum".into(),
                ),
                (
                    Operation::Maximum,
                    "input.math.target.Scalar.maximum".into(),
                ),
                (
                    Operation::MinimumNumber,
                    "input.math.target.Scalar.minimumnum".into(),
                ),
                (
                    Operation::MaximumNumber,
                    "input.math.target.Scalar.maximumnum".into(),
                ),
                (Operation::IsNan, "input.math.target.Scalar.isnan".into()),
                (
                    Operation::IsInfinite,
                    "input.math.target.Scalar.isinf".into(),
                ),
                (
                    Operation::IsFinite,
                    "input.math.target.Scalar.isfinite".into(),
                ),
                (
                    Operation::IsNormal,
                    "input.math.target.Scalar.isnormal".into(),
                ),
                (
                    Operation::IsSubnormal,
                    "input.math.target.Scalar.issubnormal".into(),
                ),
                (
                    Operation::SignBit,
                    "input.math.target.Scalar.signbit".into(),
                ),
                (
                    Operation::SaturatingAdd,
                    "input.math.target.Scalar.add_sat".into(),
                ),
                (
                    Operation::SaturatingSubtract,
                    "input.math.target.Scalar.sub_sat".into(),
                ),
                (
                    Operation::IntegerMinimum,
                    "input.math.target.Scalar.min".into(),
                ),
                (
                    Operation::IntegerMaximum,
                    "input.math.target.Scalar.max".into(),
                ),
                (Operation::BitAnd, "input.math.target.Scalar.bit_and".into()),
                (Operation::BitOr, "input.math.target.Scalar.bit_or".into()),
                (Operation::BitXor, "input.math.target.Scalar.bit_xor".into()),
                (
                    Operation::WrappingAbsoluteValue,
                    "input.math.target.Scalar.abs".into(),
                ),
                (
                    Operation::FunnelShiftLeft,
                    "input.math.target.Scalar.fshl".into(),
                ),
                (
                    Operation::FunnelShiftRight,
                    "input.math.target.Scalar.fshr".into(),
                ),
                (Operation::ByteSwap, "input.math.target.Scalar.bswap".into()),
                (
                    Operation::BitReverse,
                    "input.math.target.Scalar.bitreverse".into(),
                ),
                (
                    Operation::PopulationCount,
                    "input.math.target.Scalar.ctpop".into(),
                ),
                (
                    Operation::LeadingZeros,
                    "input.math.target.Scalar.ctlz".into(),
                ),
                (
                    Operation::TrailingZeros,
                    "input.math.target.Scalar.cttz".into(),
                ),
            ],
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(error.contains("type requirement"), "{error}");
}

#[test]
fn unsupported_unselected_operations_stay_unbound_and_cannot_reuse_old_evidence() {
    let source = "^intrinsic_enum enum Ops {fma,unknown} typealias F=.f32'builtin;
        fn run()->F {let x:F=2.0;unsafe {Ops.fma'intrinsic(x,x,x).as(F)}}
        fn unused() {unsafe {Ops.unknown'intrinsic();}}";
    let bound = bind_fma(support::compile(source));
    luna::host::llvm_ir(&bound, "run", 100000).unwrap();
    assert!(
        luna::host::llvm_ir(&bound, "unused", 100000)
            .unwrap_err()
            .contains("intrinsic target behavior")
    );
    let empty = Registry::new(&bound, &[]).unwrap();
    let unbound = luna::host::intrinsic::bind(bound, &empty, &mut 100000).unwrap();
    assert!(
        luna::host::llvm_ir(&unbound, "run", 100000)
            .unwrap_err()
            .contains("intrinsic target behavior")
    );
}

#[test]
fn bound_intrinsic_evaluates_inputs_once_in_order_with_resource_protocols() {
    let source = "trait Duplicate {} trait Retire {fn drop(*self);} impl Duplicate for bool {} impl Duplicate for .f32'builtin {}
        ^intrinsic_enum enum Ops {fma} typealias F=.f32'builtin;
        fn run() {let a:F=3.0;let b:F=4.0;let c:F=5.0;var seen=false;
            let result=unsafe {Ops.fma'intrinsic(
                do {assumes not seen;seen=true;a},
                do {assumes seen;seen=false;b},
                do {assumes not seen;seen=true;c}).as(F)};
            assumes seen;assumes result == 17.0;
        }";
    let package = bind_fma(support::compile(source));
    let copy = package
        .traits
        .iter()
        .find(|t| t.name == "Duplicate")
        .unwrap()
        .declaration;
    let drop = package
        .traits
        .iter()
        .find(|t| t.name == "Retire")
        .unwrap()
        .declaration;
    let prepared = luna::host::prepare_with_protocols(&package, "run", 100000, copy, drop).unwrap();
    let ir = luna::host::llvm_ir_prepared(&prepared, 100000).unwrap();
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
    // Artifact validation independently checks the declaration behind evidence.
    let mut forged = prepared.graph().clone();
    for nominal in &mut forged.nominals {
        if let iir::NominalDefinition::Enum(family) = &mut nominal.definition {
            family.is_intrinsic = false;
        }
    }
    assert!(
        forged
            .validate(&mut 100000)
            .unwrap_err()
            .contains("intrinsic_enum")
    );
}

#[test]
fn intrinsic_binding_evidence_cannot_survive_signature_or_identity_forgery() {
    let package = support::compile(
        "^intrinsic_enum enum Ops {fma} typealias F=.f32'builtin;fn run()->F {let a:F=1.0;unsafe {Ops.fma'intrinsic(a,a,a).as(F)}}",
    );
    let tag = tag(&package, "Ops", 0);
    let registry = Registry::new(&package, &[(tag, Operation::FusedMultiplyAdd)]).unwrap();
    assert!(luna::host::intrinsic::bind(package.clone(), &registry, &mut 0).is_err());
    let bound = luna::host::intrinsic::bind(package, &registry, &mut 100000).unwrap();
    for change_tag in [false, true] {
        let mut bad = bound.clone();
        nir::visit::expression_mut(&mut bad.functions[0].body, &mut 10000, |expr, _| {
            if let K::Intrinsic { tag, inputs, .. } = &mut expr.kind {
                if change_tag {
                    tag.variant = 1;
                } else {
                    inputs[0].ty = Term::Primitive(P::F64);
                }
                assert!(nir::intrinsics::checked(expr).is_err());
            }
            Ok(())
        })
        .unwrap();
        assert!(nir::phases::validate(&bad).is_err());
        assert!(luna::host::llvm_ir(&bad, "run", 100000).is_err());
    }
    assert!(luna::host::llvm_ir(&bound, "run", 100000).is_ok());
}

#[test]
fn bound_source_fma_executes_natively_with_single_rounding() {
    use std::os::unix::fs::PermissionsExt;
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    for (ty, a, b, c, expected) in [
        (
            "f32",
            "3.0",
            "4.0",
            "5.0",
            format!("(float32 (bits {:08x}))\n", 17f32.to_bits()),
        ),
        (
            "f64",
            "3.0",
            "4.0",
            "5.0",
            format!("(float64 (bits {:016x}))\n", 17f64.to_bits()),
        ),
        (
            "f32",
            "1.00000011920928955078125",
            "0.99999988079071044921875",
            "-1.0",
            format!(
                "(float32 (bits {:08x}))\n",
                (1f32 + f32::EPSILON)
                    .mul_add(1f32 - f32::EPSILON, -1.)
                    .to_bits()
            ),
        ),
        (
            "f64",
            "1.0000000000000002220446049250313080847263336181640625",
            "0.9999999999999997779553950749686919152736663818359375",
            "-1.0",
            format!(
                "(float64 (bits {:016x}))\n",
                (1f64 + f64::EPSILON)
                    .mul_add(1f64 - f64::EPSILON, -1.)
                    .to_bits()
            ),
        ),
    ] {
        let source = format!(
            "^intrinsic_enum enum Ops {{any_name}} typealias F=.{ty}'builtin; fn fused(a:F,b:F,c:F)->F {{unsafe {{Ops.any_name'intrinsic(a,b,c).as(F)}}}} fn run()->F {{fused({a},{b},{c})}}"
        );
        let package = bind_fma(support::compile(&source));
        let ir = luna::host::llvm_ir(&package, "run", 100000).unwrap();
        assert!(
            ir.contains(&format!(
                "call {} @llvm.fma.{ty}",
                if ty == "f32" { "float" } else { "double" }
            )),
            "{ir}"
        );
        for optimization in [
            luna::host::OptimizationLevel::O0,
            luna::host::OptimizationLevel::O3,
        ] {
            let tools = luna::host::Toolchain {
                optimization,
                ..Default::default()
            };
            let bytes = luna::host::executable(&ir, &tools).unwrap();
            // PIDs can be reused across sandbox sessions. Reserve a fresh
            // directory atomically without touching another run's artifacts.
            let dir = (0..1024)
                .find_map(|_| {
                    let dir = std::env::temp_dir().join(format!(
                        "flurry-fma-{}-{}",
                        std::process::id(),
                        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                    ));
                    match std::fs::create_dir(&dir) {
                        Ok(()) => Some(dir),
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                        Err(error) => panic!("cannot reserve FMA test directory: {error}"),
                    }
                })
                .expect("FMA test directory attempts exhausted");
            let path = dir.join("program");
            std::fs::write(&path, bytes).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
            let output = loop {
                match std::process::Command::new(&path)
                    .stdin(std::process::Stdio::null())
                    .output()
                {
                    // ETXTBSY means no child started. Parallel test forks may
                    // briefly inherit a writer; never retry a completed run.
                    Err(error)
                        if error.raw_os_error() == Some(26)
                            && std::time::Instant::now() < deadline =>
                    {
                        std::thread::sleep(std::time::Duration::from_millis(2));
                    }
                    result => break result.unwrap(),
                }
            };
            std::fs::remove_dir_all(dir).unwrap();
            assert!(output.status.success(), "{output:?}");
            assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
        }
    }
}

fn operation(tag: Tag) -> Expr {
    Expr {
        source: SourceRange { start: 40, end: 70 },
        ty: Term::Primitive(P::Bool),
        kind: K::Intrinsic {
            tag,
            binding: None,
            inputs: [true, false, true]
                .into_iter()
                .enumerate()
                .map(|(i, value)| Expr {
                    source: SourceRange {
                        start: 41 + i as u32,
                        end: 42 + i as u32,
                    },
                    ty: Term::Primitive(P::Bool),
                    kind: K::Literal(nir::program::Literal::Bool(value)),
                })
                .collect(),
        },
    }
}

#[test]
fn operation_ir_preserves_order_identity_types_sources_and_budgets() {
    let package = support::compile("^intrinsic_enum enum Ops {fma}");
    let tag = tag(&package, "Ops", 0);
    let mut expr = operation(tag);
    let original = expr.clone();
    let text = expr.to_string();
    assert!(text.contains("(intrinsic (intrinsic-tag"), "{text}");
    assert!(text.contains("(inputs"), "{text}");
    let mut visited = Vec::new();
    nir::visit::expression(&expr, &mut 1000, |e, _| {
        visited.push(e.source.start);
        Ok(())
    })
    .unwrap();
    visited.sort();
    assert_eq!(visited, vec![40, 41, 42, 43]);
    assert!(nir::visit::expression(&expr, &mut 1, |_, _| Ok(())).is_err());
    assert!(nir::copy_budget::charge_expression(&expr, &mut 1).is_err());
    nir::copy_budget::charge_expression(&expr, &mut 1000).unwrap();
    expr.rewrite_types(&mut |t| {
        Ok::<_, nir::TypeError>(if *t == Term::Primitive(P::Bool) {
            Term::Primitive(P::F32)
        } else {
            t.clone()
        })
    })
    .unwrap();
    let K::Intrinsic {
        tag: actual,
        inputs,
        ..
    } = &expr.kind
    else {
        panic!()
    };
    assert_eq!(*actual, tag);
    assert_eq!(
        inputs.iter().map(|x| x.source.start).collect::<Vec<_>>(),
        vec![41, 42, 43]
    );
    assert!(inputs.iter().all(|x| x.ty == Term::Primitive(P::F32)));
    assert_eq!(expr.source, original.source);
    assert_eq!(expr.ty, Term::Primitive(P::F32));
    let mut count = 0;
    nir::visit::expression_mut(&mut expr, &mut 1000, |_, _| {
        count += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(count, 4);
}

#[test]
fn operation_ir_requires_lexical_permission_and_runtime_phase() {
    let mut package = support::compile("^intrinsic_enum enum Ops {fma} fn run()->bool {true}");
    let tag = tag(&package, "Ops", 0);
    package.functions[0].body = operation(tag);
    assert!(
        nir::call_safety::validate(&package.functions[0], &mut 1000)
            .unwrap_err()
            .contains("unsafe boundary at 40..70")
    );
    assert!(nir::phases::validate(&package).is_err());
    package.functions[0].is_unsafe = true;
    nir::phases::validate(&package).unwrap();
    package.functions[0].is_unsafe = false;
    let inner = package.functions[0].body.clone();
    package.functions[0].body.kind = K::UnsafeScope(Box::new(inner));
    nir::phases::validate(&package).unwrap();
    package.functions[0].phase = nir::program::FunctionPhase::Comptime;
    assert!(
        nir::phases::validate(&package)
            .unwrap_err()
            .message
            .contains("cannot execute at comptime")
    );
    package.functions[0].phase = nir::program::FunctionPhase::Runtime;
    package.enums[0].is_intrinsic = false;
    assert!(
        nir::phases::validate(&package)
            .unwrap_err()
            .message
            .contains("intrinsic_enum")
    );
    package.enums[0].is_intrinsic = true;
    nir::phases::validate(&package).unwrap();
}

#[test]
fn unknown_target_behavior_cannot_be_published_or_proved_from_its_output() {
    let mut package = support::compile("^intrinsic_enum enum Ops {fma} fn run()->bool {true}");
    let tag = tag(&package, "Ops", 0);
    package.functions[0].body = operation(tag);
    package.functions[0].is_unsafe = true;
    assert!(
        elaboration::verification::flow::analyze(&package.functions[0], &mut Default::default())
            .is_err()
    );
    let reason =
        elaboration::verification::memory::infer(&package.functions[0], &mut 10000).unwrap_err();
    assert!(reason.contains("intrinsic memory behavior"), "{reason}");
    let reason = elaboration::verification::moves::analyze(
        &package.functions[0],
        |_| Ok(elaboration::verification::moves::Delivery::Copy),
        10000,
    )
    .unwrap_err();
    assert!(
        reason.to_string().contains("intrinsic ownership behavior"),
        "{reason}"
    );
    let mut interpreter =
        comptime_interpretor::Interpreter::new(&package, Default::default()).unwrap();
    let failure = interpreter
        .invoke(package.functions[0].declaration, vec![])
        .unwrap_err();
    assert_eq!(failure.kind, comptime_interpretor::ErrorKind::Unsupported);
    package.functions[0].is_unsafe = false;
    let inner = package.functions[0].body.clone();
    package.functions[0].body.kind = K::UnsafeScope(Box::new(inner));
    let key = iir::InstanceKey {
        declaration: package.functions[0].declaration,
        arguments: vec![],
    };
    let graph = iir::Package {
        roots: vec![key.clone()],
        functions: vec![iir::FunctionInstance {
            key,
            function: package.functions[0].clone(),
        }],
        ..Default::default()
    };
    let reason = graph.validate(&mut 10000).unwrap_err();
    assert!(reason.contains("intrinsic target behavior"), "{reason}");
}

fn tag(package: &nir::program::Package, name: &str, variant: usize) -> Tag {
    Tag {
        family: package
            .enums
            .iter()
            .find(|e| e.name == name)
            .unwrap()
            .declaration,
        variant,
    }
}

#[test]
fn source_intrinsic_forms_ordered_operation_with_immediate_output_interface() {
    let package = support::compile(
        "^intrinsic_enum enum Ops {fma,barrier} typealias F=.f32'builtin; unsafe fn fused(a:F,b:F,c:F)->F {(Ops.fma'intrinsic(a,b,c)).as(F)} unsafe fn wait() {Ops.barrier'intrinsic();}",
    );
    let mut found = vec![];
    for function in &package.functions {
        nir::visit::expressions(function, &mut 10000, |expr, _| {
            if let K::Intrinsic { tag, inputs, .. } = &expr.kind {
                found.push((*tag, expr.ty.clone(), inputs.clone()));
            }
            Ok(())
        })
        .unwrap();
    }
    let fused = found.iter().find(|(tag, _, _)| tag.variant == 0).unwrap();
    let wait = found.iter().find(|(tag, _, _)| tag.variant == 1).unwrap();
    assert_eq!(fused.1, Term::Primitive(P::F32));
    assert_eq!(wait.1, Term::Primitive(P::Void));
    assert!(wait.2.is_empty());
    let parameters = &package
        .functions
        .iter()
        .find(|f| f.name == "fused")
        .unwrap()
        .parameters;
    assert_eq!(
        fused
            .2
            .iter()
            .map(|e| match e.kind {
                K::Local(id) => id,
                _ => panic!(),
            })
            .collect::<Vec<_>>(),
        *parameters
    );
    let registry = Registry::new(&package, &[(fused.0, Operation::FusedMultiplyAdd)]).unwrap();
    registry
        .check(
            fused.0,
            &fused.2.iter().map(|e| e.ty.clone()).collect::<Vec<_>>(),
            &fused.1,
        )
        .unwrap();
}

#[test]
fn source_intrinsic_keeps_generic_operand_and_result_types_without_head_expansion() {
    let package = support::compile(
        "^intrinsic_enum enum Ops {op} fn project(value:T)->T where T {unsafe {Ops.op'intrinsic(value).as(T)}}",
    );
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "project")
        .unwrap();
    let mut count = 0;
    nir::visit::expressions(function, &mut 10000, |expr, _| {
        if let K::Intrinsic { inputs, .. } = &expr.kind {
            count += 1;
            assert_eq!(inputs.len(), 1);
            assert_eq!(inputs[0].ty, expr.ty);
            assert!(!matches!(expr.ty, Term::Primitive(_)));
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn binding_uses_declaration_identity_not_operation_spelling() {
    let package = support::compile(
        "^intrinsic_enum enum First {arbitrary} ^intrinsic_enum enum Second {arbitrary} enum Ordinary {arbitrary}",
    );
    let first = tag(&package, "First", 0);
    let second = tag(&package, "Second", 0);
    let ordinary = tag(&package, "Ordinary", 0);
    let registry = Registry::new(&package, &[(first, Operation::FusedMultiplyAdd)]).unwrap();
    let ty = Term::Primitive(P::F32);
    let checked = registry
        .check(first, &[ty.clone(), ty.clone(), ty.clone()], &ty)
        .unwrap();
    assert_eq!(checked.tag(), first);
    assert_eq!(checked.width(), Some(32));
    assert!(
        checked
            .to_string()
            .contains("(operation fused-multiply-add)")
    );
    assert_eq!(
        registry.check(second, &[], &ty),
        Err(Error::UnsupportedTag(second))
    );
    assert!(
        matches!(Registry::new(&package, &[(ordinary, Operation::FusedMultiplyAdd)]), Err(Error::NotIntrinsic(t)) if t == ordinary)
    );
}

#[test]
fn fma_requires_three_same_precision_inputs_and_explicit_float_output() {
    let package = support::compile("^intrinsic_enum enum Ops {fma}");
    let tag = tag(&package, "Ops", 0);
    let registry = Registry::new(&package, &[(tag, Operation::FusedMultiplyAdd)]).unwrap();
    for (primitive, width) in [(P::F32, 32), (P::F64, 64)] {
        let ty = Term::Primitive(primitive);
        let inputs = vec![ty.clone(); 3];
        for output in [P::Void, P::U32, P::Bool] {
            assert_eq!(
                registry.check(tag, &inputs, &Term::Primitive(output)),
                Err(Error::OutputType(tag))
            );
        }
        for n in [0, 1, 2, 4] {
            assert_eq!(
                registry.check(tag, &vec![ty.clone(); n], &ty),
                Err(Error::OperandCount {
                    tag,
                    actual: n,
                    expected: 3
                })
            );
        }
        for index in 0..3 {
            let mut mixed = inputs.clone();
            mixed[index] = Term::Primitive(if width == 32 { P::F64 } else { P::F32 });
            assert_eq!(
                registry.check(tag, &mixed, &ty),
                Err(Error::OperandType { tag, index })
            );
        }
        // Failed checks cannot poison the next valid use.
        assert_eq!(
            registry.check(tag, &inputs, &ty).unwrap().width(),
            Some(width)
        );
    }
}

#[test]
fn invalid_bindings_are_rejected_without_publishing_partial_registry() {
    let mut package = support::compile("^intrinsic_enum enum Ops {fma}");
    let tag = tag(&package, "Ops", 0);
    let binding = (tag, Operation::FusedMultiplyAdd);
    assert!(matches!(
        Registry::new(&package, &[binding, binding]),
        Err(Error::DuplicateBinding(_))
    ));
    let absent = Tag { variant: 1, ..tag };
    assert!(matches!(
        Registry::new(&package, &[(absent, binding.1)]),
        Err(Error::UnknownVariant(_))
    ));
    let unknown = Tag {
        family: nir::DeclarationId {
            package: u64::MAX,
            local: 0,
        },
        ..tag
    };
    assert!(matches!(
        Registry::new(&package, &[(unknown, binding.1)]),
        Err(Error::UnknownFamily(_))
    ));
    package.enums[0].variants[0]
        .fields
        .push(Term::Primitive(P::Bool));
    assert!(matches!(
        Registry::new(&package, &[binding]),
        Err(Error::PayloadTag(_))
    ));
    package.enums[0].variants[0].fields.clear();
    package.enums[0]
        .parameter_types
        .push(Term::Primitive(P::Type));
    assert!(matches!(
        Registry::new(&package, &[binding]),
        Err(Error::UnsupportedFamily(_))
    ));
    package.enums[0].parameter_types.clear();
    package.enums.push(package.enums[0].clone());
    assert!(matches!(
        Registry::new(&package, &[binding]),
        Err(Error::AmbiguousFamily(_))
    ));
    package.enums.pop();
    assert!(Registry::new(&package, &[binding]).is_ok());
}

#[test]
fn generic_fma_specializes_pending_signature_before_host_admission() {
    let source = "^intrinsic_enum enum Ops {fma}
        pure fn fused(a:T,b:T,c:T)->T where T, requires T == .f32'builtin or T == .f64'builtin {
            unsafe {Ops.fma'intrinsic(a,b,c).as(T)}
        }
        fn run() {let a=fused<.f32'builtin>(3.0,4.0,5.0);let b=fused<.f64'builtin>(3.0,4.0,5.0);assumes a == 17.0 and b == 17.0;}";
    let compile = |source: &str| {
        support::compile_with_source_bindings(
            source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                host_fma: Some("input.Ops.fma"),
                ..Default::default()
            },
        )
    };
    let package = compile(source).unwrap();
    let function = package
        .functions
        .iter()
        .find(|f| f.name == "fused")
        .unwrap();
    let mut pending = None;
    nir::visit::expressions(function, &mut 10000, |expr, _| {
        if matches!(expr.kind, K::Intrinsic { .. }) {
            pending = Some(expr.clone());
        }
        Ok(())
    })
    .unwrap();
    let pending = pending.unwrap();
    assert!(pending.to_string().contains("pending-type"));
    assert!(
        nir::intrinsics::checked(&pending)
            .unwrap_err()
            .contains("concrete specialization")
    );
    for (primitive, width) in [(P::F32, Some(32)), (P::F64, Some(64)), (P::Bool, None)] {
        let mut instantiated = pending.clone();
        let parameter = pending.ty.clone();
        instantiated
            .rewrite_types(&mut |ty| {
                Ok::<_, String>(if *ty == parameter {
                    Term::Primitive(primitive)
                } else {
                    ty.clone()
                })
            })
            .unwrap();
        let result = nir::intrinsics::checked(&instantiated);
        if let Some(width) = width {
            assert_eq!(result.unwrap().width(), Some(width));
        } else {
            assert!(result.is_err());
        }
    }
    let ir = luna::host::llvm_ir(&package, "run", 100000).unwrap();
    assert!(ir.contains("@llvm.fma.f32") && ir.contains("@llvm.fma.f64"));
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
    let unconstrained = source.replace(", requires T == .f32'builtin or T == .f64'builtin", "");
    assert!(
        compile(&unconstrained)
            .unwrap_err()
            .contains("type requirement")
    );
    let too_wide = source.replace("T == .f64'builtin", "T == bool");
    assert!(compile(&too_wide).unwrap_err().contains("type requirement"));
    let opaque = format!(
        "pure comptime fn Identity<T> -> .Type'builtin {{T}}\n{}",
        source
            .split("fn run()")
            .next()
            .unwrap()
            .replace("T == .f32'builtin", "T == Identity<.f32'builtin>")
    );
    let error = compile(&opaque).unwrap_err();
    assert!(error.contains("type requirement"), "{error}");
    let mixed = source.replace("a,b,c).as(T)", "a,b,true).as(T)");
    assert!(compile(&mixed).unwrap_err().contains("operand 2"));
    let unsafe_missing = source.replace("unsafe {Ops", "do {Ops");
    assert!(compile(&unsafe_missing).unwrap_err().contains("unsafe"));
    compile(source).unwrap();
}

#[test]
fn sqrt_has_unary_same_precision_protocol() {
    let package = support::compile("^intrinsic_enum enum Ops {sqrt}");
    let tag = tag(&package, "Ops", 0);
    let registry = Registry::new(&package, &[(tag, Operation::SquareRoot)]).unwrap();
    for primitive in [P::F32, P::F64] {
        let ty = Term::Primitive(primitive);
        for count in [0, 2, 3] {
            assert_eq!(
                registry.check(tag, &vec![ty.clone(); count], &ty),
                Err(Error::OperandCount {
                    tag,
                    actual: count,
                    expected: 1
                })
            );
        }
        assert_eq!(
            registry.check(tag, &[Term::Primitive(P::Bool)], &ty),
            Err(Error::OperandType { tag, index: 0 })
        );
        assert_eq!(
            registry.check(tag, std::slice::from_ref(&ty), &Term::Primitive(P::U32)),
            Err(Error::OutputType(tag))
        );
        let checked = registry.check(tag, std::slice::from_ref(&ty), &ty).unwrap();
        assert_eq!(checked.operation(), Operation::SquareRoot);
        assert!(checked.to_string().contains("(operation square-root)"));
    }
}

#[test]
fn sqrt_executes_both_precisions_at_o0_and_o3() {
    for width in [32, 64] {
        let source = format!(
            "^intrinsic_enum enum Ops {{sqrt}}
            typealias F=.f{width}'builtin;
            fn root(x:F)->F {{unsafe {{Ops.sqrt'intrinsic(x).as(F)}}}}
            fn run() {{
                let four = root(4.0);
                let zero = root(0.0);
                let negative_zero = root(-0.0);
                let infinity = root(1.0 / 0.0);
                let nan = root(-1.0);
                assumes four == 2.0;
                assumes zero == 0.0;
                assumes 1.0 / negative_zero == -1.0 / 0.0;
                assumes infinity == 1.0 / 0.0;
                assumes nan != nan;
            }}"
        );
        let package = support::compile(&source);
        let tag = tag(&package, "Ops", 0);
        let registry = Registry::new(&package, &[(tag, Operation::SquareRoot)]).unwrap();
        let bound = luna::host::intrinsic::bind(package, &registry, &mut 100000).unwrap();
        let ir = luna::host::llvm_ir(&bound, "run", 100000).unwrap();
        assert!(ir.contains(&format!("@llvm.sqrt.f{width}")), "{ir}");
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
fn generic_intrinsic_rebinding_preserves_only_existing_source_admission() {
    for (name, operation, parameters, arguments, call, value) in [
        (
            "fma",
            Operation::FusedMultiplyAdd,
            "a:T,b:T,c:T",
            "a,b,c",
            "3.0,4.0,5.0",
            "17.0",
        ),
        (
            "minimum",
            Operation::Minimum,
            "a:T,b:T",
            "a,b",
            "3.0,-2.0",
            "-2.0",
        ),
        (
            "maximum",
            Operation::Maximum,
            "a:T,b:T",
            "a,b",
            "3.0,-2.0",
            "3.0",
        ),
        (
            "minimumnum",
            Operation::MinimumNumber,
            "a:T,b:T",
            "a,b",
            "3.0,-2.0",
            "-2.0",
        ),
        (
            "maximumnum",
            Operation::MaximumNumber,
            "a:T,b:T",
            "a,b",
            "3.0,-2.0",
            "3.0",
        ),
        ("sqrt", Operation::SquareRoot, "a:T", "a", "81.0", "9.0"),
        ("sin", Operation::Sine, "a:T", "a", "0.0", "0.0"),
        ("cos", Operation::Cosine, "a:T", "a", "0.0", "1.0"),
        ("exp", Operation::Exponential, "a:T", "a", "0.0", "1.0"),
        ("exp2", Operation::Exponential2, "a:T", "a", "3.0", "8.0"),
        ("log", Operation::NaturalLog, "a:T", "a", "1.0", "0.0"),
        ("log2", Operation::Log2, "a:T", "a", "8.0", "3.0"),
        ("log10", Operation::Log10, "a:T", "a", "1.0", "0.0"),
        (
            "pow",
            Operation::Power,
            "a:T,b:T",
            "a,b",
            "2.0,-3.0",
            "0.125",
        ),
        ("fabs", Operation::AbsoluteValue, "a:T", "a", "-2.5", "2.5"),
        ("round", Operation::RoundAway, "a:T", "a", "-2.5", "-3.0"),
        ("floor", Operation::Floor, "a:T", "a", "-2.5", "-3.0"),
        ("ceil", Operation::Ceiling, "a:T", "a", "-2.5", "-2.0"),
        ("trunc", Operation::Truncate, "a:T", "a", "-2.5", "-2.0"),
        (
            "roundeven",
            Operation::RoundEven,
            "a:T",
            "a",
            "-2.5",
            "-2.0",
        ),
        (
            "copysign",
            Operation::CopySign,
            "a:T,b:T",
            "a,b",
            "3.5,-0.0",
            "-3.5",
        ),
    ] {
        let source = format!("^intrinsic_enum enum Ops {{{name}}}
            pure fn calculate({parameters})->T where T, requires T == .f32'builtin or T == .f64'builtin {{
                unsafe {{Ops.{name}'intrinsic({arguments}).as(T)}}
            }}
            fn run() {{let a=calculate<.f32'builtin>({call});let b=calculate<.f64'builtin>({call});assumes a == {value} and b == {value};}}");
        let selector = format!("input.Ops.{name}");
        let package = support::compile_with_source_bindings(
            &source,
            &Default::default(),
            luna::protocols::type_constructors::Selectors {
                host_fma: (name == "fma").then_some(selector.as_str()),
                host_sqrt: (name == "sqrt").then_some(selector.as_str()),
                host_copysign: (name == "copysign").then_some(selector.as_str()),
                host_intrinsics: &[(operation, selector.clone())],
                ..Default::default()
            },
        )
        .unwrap();
        let identity = tag(&package, "Ops", 0);
        let registry = Registry::new(&package, &[(identity, operation)]).unwrap();
        assert!(luna::host::intrinsic::bind(package.clone(), &registry, &mut 0).is_err());
        let rebound = luna::host::intrinsic::bind(package.clone(), &registry, &mut 100000).unwrap();
        let ir = luna::host::llvm_ir(&rebound, "run", 100000).unwrap();
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
        // Clearing a registry clears pending evidence too; rebinding alone
        // cannot recreate the source check that established its float domain.
        let empty = Registry::new(&package, &[]).unwrap();
        let unbound = luna::host::intrinsic::bind(package.clone(), &empty, &mut 100000).unwrap();
        let error = luna::host::intrinsic::bind(unbound, &registry, &mut 100000).unwrap_err();
        assert!(error.contains("matching source admission"), "{error}");
        let other = if name == "fma" {
            Operation::SquareRoot
        } else {
            Operation::FusedMultiplyAdd
        };
        let wrong = Registry::new(&package, &[(identity, other)]).unwrap();
        assert!(luna::host::intrinsic::bind(package.clone(), &wrong, &mut 100000).is_err());
        let mut forged = package.clone();
        for function in &mut forged.functions {
            nir::visit::expression_mut(&mut function.body, &mut 10000, |expr, _| {
                if let K::Intrinsic { inputs, .. } = &mut expr.kind {
                    inputs[0].ty = Term::Primitive(P::Bool);
                }
                Ok(())
            })
            .unwrap();
        }
        assert!(luna::host::intrinsic::bind(forged, &registry, &mut 100000).is_err());
        luna::host::intrinsic::bind(package, &registry, &mut 100000).unwrap();
    }
}

#[test]
fn copysign_requires_two_same_precision_operands() {
    let package = support::compile("^intrinsic_enum enum Ops {sign}");
    let tag = tag(&package, "Ops", 0);
    let registry = Registry::new(&package, &[(tag, Operation::CopySign)]).unwrap();
    for primitive in [P::F32, P::F64] {
        let ty = Term::Primitive(primitive);
        for count in [0, 1, 3] {
            assert_eq!(
                registry.check(tag, &vec![ty.clone(); count], &ty),
                Err(Error::OperandCount {
                    tag,
                    actual: count,
                    expected: 2
                })
            );
        }
        for index in [0, 1] {
            let mut inputs = vec![ty.clone(); 2];
            inputs[index] = Term::Primitive(if primitive == P::F32 { P::F64 } else { P::F32 });
            assert_eq!(
                registry.check(tag, &inputs, &ty),
                Err(Error::OperandType { tag, index })
            );
        }
        assert_eq!(
            registry.check(tag, &[ty.clone(), ty.clone()], &Term::Primitive(P::U32)),
            Err(Error::OutputType(tag))
        );
        let checked = registry.check(tag, &[ty.clone(), ty.clone()], &ty).unwrap();
        assert!(checked.to_string().contains("(operation copy-sign)"));
    }
}

#[test]
fn iir_intrinsic_family_requires_the_exact_unparameterized_instance() {
    let package = bind_fma(support::compile(
        "^intrinsic_enum enum Ops {fma} typealias F=.f32'builtin;
        fn run()->F {let x:F=2.0;unsafe {Ops.fma'intrinsic(x,x,x).as(F)}}",
    ));
    let graph = luna::instantiation::instantiate(&package, "run", 100000).unwrap();
    graph.validate(&mut 100000).unwrap();
    let mut forged = graph.clone();
    let family = forged.nominals.iter_mut().find(|nominal| {
        matches!(&nominal.definition, iir::NominalDefinition::Enum(e) if e.is_intrinsic)
    }).unwrap();
    let Term::Nominal { arguments, .. } = &mut family.ty else {
        panic!()
    };
    assert!(arguments.is_empty());
    arguments.push(Term::Primitive(P::Bool));
    let error = forged.validate(&mut 100000).unwrap_err();
    assert!(error.contains("intrinsic tag family"), "{error}");
    assert!(comptime_interpretor::Interpreter::from_iir(&forged, Default::default()).is_err());
    assert!(graph.validate(&mut 0).is_err());
    graph.validate(&mut 100000).unwrap();
}

#[test]
fn unary_math_intrinsics_require_one_matching_float_operand() {
    let package = support::compile("^intrinsic_enum enum Ops {round}");
    let tag = tag(&package, "Ops", 0);
    for operation in [
        Operation::AbsoluteValue,
        Operation::Floor,
        Operation::Ceiling,
        Operation::Truncate,
        Operation::RoundEven,
        Operation::RoundAway,
        Operation::Sine,
        Operation::Cosine,
        Operation::Exponential,
        Operation::Exponential2,
        Operation::NaturalLog,
        Operation::Log2,
        Operation::Log10,
    ] {
        let registry = Registry::new(&package, &[(tag, operation)]).unwrap();
        for primitive in [P::F32, P::F64] {
            let ty = Term::Primitive(primitive);
            for count in [0, 2, 3] {
                assert_eq!(
                    registry.check(tag, &vec![ty.clone(); count], &ty),
                    Err(Error::OperandCount {
                        tag,
                        actual: count,
                        expected: 1
                    })
                );
            }
            let other = Term::Primitive(if primitive == P::F32 { P::F64 } else { P::F32 });
            assert_eq!(
                registry.check(tag, &[other], &ty),
                Err(Error::OperandType { tag, index: 0 })
            );
            assert_eq!(
                registry.check(tag, std::slice::from_ref(&ty), &Term::Primitive(P::Bool)),
                Err(Error::OutputType(tag))
            );
            assert_eq!(
                registry
                    .check(tag, std::slice::from_ref(&ty), &ty)
                    .unwrap()
                    .operation(),
                operation
            );
        }
    }
}

#[test]
fn binary_math_requires_two_same_precision_operands() {
    let package = support::compile("^intrinsic_enum enum Ops {sign}");
    let tag = tag(&package, "Ops", 0);
    for (name, operation) in [
        ("pow", Operation::Power),
        ("minimum", Operation::Minimum),
        ("maximum", Operation::Maximum),
        ("minimumnum", Operation::MinimumNumber),
        ("maximumnum", Operation::MaximumNumber),
    ] {
        let registry = Registry::new(&package, &[(tag, operation)]).unwrap();
        for primitive in [P::F32, P::F64] {
            let ty = Term::Primitive(primitive);
            for count in [0, 1, 3] {
                assert_eq!(
                    registry.check(tag, &vec![ty.clone(); count], &ty),
                    Err(Error::OperandCount {
                        tag,
                        actual: count,
                        expected: 2
                    })
                );
            }
            for index in [0, 1] {
                let mut inputs = vec![ty.clone(); 2];
                inputs[index] = Term::Primitive(if primitive == P::F32 { P::F64 } else { P::F32 });
                assert_eq!(
                    registry.check(tag, &inputs, &ty),
                    Err(Error::OperandType { tag, index })
                );
            }
            assert_eq!(
                registry.check(tag, &[ty.clone(), ty.clone()], &Term::Primitive(P::U32)),
                Err(Error::OutputType(tag))
            );
            let checked = registry.check(tag, &[ty.clone(), ty.clone()], &ty).unwrap();
            assert!(checked.to_string().contains(&format!("(operation {name})")));
        }
    }
}

#[test]
fn native_transcendental_intrinsics_preserve_special_value_classes() {
    use Operation::*;
    for (name, operation) in [
        ("sin", Sine),
        ("cos", Cosine),
        ("exp", Exponential),
        ("exp2", Exponential2),
        ("log", NaturalLog),
        ("log2", Log2),
        ("log10", Log10),
        ("pow", Power),
    ] {
        for width in [32, 64] {
            let encode = |value: f64| {
                if width == 32 {
                    (value as f32).to_bits() as u64
                } else {
                    value.to_bits()
                }
            };
            let mut cases = match operation {
                Sine => vec![
                    (0.0, 0.0, 0.0),
                    (-0.0, 0.0, -0.0),
                    (f64::INFINITY, 0.0, f64::NAN),
                ],
                Cosine => vec![
                    (0.0, 0.0, 1.0),
                    (-0.0, 0.0, 1.0),
                    (f64::INFINITY, 0.0, f64::NAN),
                ],
                Exponential | Exponential2 => vec![
                    (0.0, 0.0, 1.0),
                    (f64::NEG_INFINITY, 0.0, 0.0),
                    (f64::INFINITY, 0.0, f64::INFINITY),
                ],
                NaturalLog | Log2 | Log10 => vec![
                    (1.0, 0.0, 0.0),
                    (0.0, 0.0, f64::NEG_INFINITY),
                    (-0.0, 0.0, f64::NEG_INFINITY),
                    (-1.0, 0.0, f64::NAN),
                    (f64::INFINITY, 0.0, f64::INFINITY),
                ],
                Power => vec![
                    (2.0, -3.0, 0.125),
                    (-2.0, 3.0, -8.0),
                    (-2.0, 0.5, f64::NAN),
                    (-0.0, 3.0, -0.0),
                    (-0.0, -3.0, f64::NEG_INFINITY),
                    (f64::NAN, 0.0, 1.0),
                    (1.0, f64::NAN, 1.0),
                ],
                _ => unreachable!(),
            };
            cases.push((f64::NAN, 1.0, f64::NAN));
            let mut checks = String::new();
            for (a, b, expected) in cases {
                let arguments = if operation == Power { "a,b" } else { "a" };
                let comparison = if expected.is_nan() {
                    "assumes output != output;".into()
                } else {
                    format!(
                        "let bits=output'bitvec;assumes bits == {};",
                        encode(expected)
                    )
                };
                checks.push_str(&format!("do {{var a:F=0.0;a'bitvec[0..{width}]={};var b:F=0.0;b'bitvec[0..{width}]={};let output=apply({arguments});{comparison}}};", encode(a), encode(b)));
            }
            let parameters = if operation == Power { "a:F,b:F" } else { "a:F" };
            let arguments = if operation == Power { "a,b" } else { "a" };
            let source = format!(
                "typealias F=.f{width}'builtin; ^intrinsic_enum enum Ops {{calculate}}
                fn apply({parameters})->F {{unsafe {{Ops.calculate'intrinsic({arguments}).as(F)}}}}
                fn run() {{{checks}}}"
            );
            let package = support::compile(&source);
            let registry =
                Registry::new(&package, &[(tag(&package, "Ops", 0), operation)]).unwrap();
            let bound = luna::host::intrinsic::bind(package, &registry, &mut 100_000).unwrap();
            let ir = luna::host::llvm_ir(&bound, "run", 100_000).unwrap();
            assert!(ir.contains(&format!("@llvm.{name}.f{width}")));
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
fn native_fabs_clears_sign_without_changing_nan_payloads() {
    for (width, patterns) in [
        (
            32,
            vec![
                0u64, 0x80000000, 0xbf800000, 0x80000001, 0xff800000, 0xffc12345, 0xff812345,
            ],
        ),
        (
            64,
            vec![
                0u64,
                0x8000000000000000,
                0xbff0000000000000,
                0x8000000000000001,
                0xfff0000000000000,
                0xfff8123456789abc,
                0xfff0123456789abc,
            ],
        ),
    ] {
        let mut checks = String::new();
        let mask = (1u64 << (width - 1)) - 1;
        for bits in patterns {
            let expected = bits & mask;
            checks.push_str(&format!("do {{var input:F=0.0;input'bitvec[0..{width}]={bits};var seen=false;let result=absolute(do {{assumes not seen;seen=true;input}});assumes seen;assumes result'bitvec == {expected};}};"));
        }
        let source = format!(
            "typealias F=.f{width}'builtin;^intrinsic_enum enum Ops {{absolute}}
            fn absolute(input:F)->F {{unsafe {{Ops.absolute'intrinsic(input).as(F)}}}}
            fn run() {{{checks}}}"
        );
        let package = support::compile(&source);
        let registry = Registry::new(
            &package,
            &[(tag(&package, "Ops", 0), Operation::AbsoluteValue)],
        )
        .unwrap();
        let package = luna::host::intrinsic::bind(package, &registry, &mut 100000).unwrap();
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

#[test]
fn native_round_away_preserves_special_values_and_halfway_direction() {
    for width in [32, 64] {
        let encode = |value: f64| {
            if width == 32 {
                (value as f32).to_bits() as u64
            } else {
                value.to_bits()
            }
        };
        let cases = [
            (2.5, 3.0),
            (-2.5, -3.0),
            (0.5, 1.0),
            (-0.5, -1.0),
            (0.25, 0.0),
            (-0.25, -0.0),
            (0.0, 0.0),
            (-0.0, -0.0),
            (f64::INFINITY, f64::INFINITY),
            (f64::NEG_INFINITY, f64::NEG_INFINITY),
            (f64::NAN, f64::NAN),
        ];
        let mut checks = String::new();
        for (input, expected) in cases {
            let comparison = if expected.is_nan() {
                "output != output".into()
            } else {
                format!("output'bitvec == {}", encode(expected))
            };
            checks.push_str(&format!("do {{var input:F=0.0;input'bitvec[0..{width}]={};let output=apply(input);assumes {comparison};}};",encode(input)));
        }
        let source=format!("typealias F=.f{width}'builtin;^intrinsic_enum enum Ops {{round}}
            fn apply(input:F)->F {{unsafe {{Ops.round'intrinsic(input).as(F)}}}} fn run() {{{checks}}}");
        let package = support::compile(&source);
        let registry =
            Registry::new(&package, &[(tag(&package, "Ops", 0), Operation::RoundAway)]).unwrap();
        let package = luna::host::intrinsic::bind(package, &registry, &mut 100000).unwrap();
        let ir = luna::host::llvm_ir(&package, "run", 100000).unwrap();
        assert!(ir.contains(&format!("@llvm.round.f{width}")));
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
fn integer_intrinsics_enforce_domain_and_revalidate_exact_scalar_types() {
    let package = support::compile("^intrinsic_enum enum Ops {bits}");
    let tag = tag(&package, "Ops", 0);
    for operation in [
        Operation::FunnelShiftLeft,
        Operation::FunnelShiftRight,
        Operation::ByteSwap,
        Operation::BitReverse,
        Operation::PopulationCount,
        Operation::LeadingZeros,
        Operation::TrailingZeros,
    ] {
        let arity = operation.operand_count();
        let registry = Registry::new(&package, &[(tag, operation)]).unwrap();
        for (primitive, width) in [
            (P::U8, 8),
            (P::U16, 16),
            (P::U32, 32),
            (P::U64, 64),
            (P::U128, 128),
        ] {
            let ty = Term::Primitive(primitive);
            if operation == Operation::ByteSwap && primitive == P::U8 {
                assert_eq!(
                    registry.check(tag, &vec![ty.clone(); arity], &ty),
                    Err(Error::OutputType(tag))
                );
                continue;
            }
            let checked = registry.check(tag, &vec![ty.clone(); arity], &ty).unwrap();
            assert_eq!(checked.width(), Some(width));
            assert!(checked.to_string().contains("(scalar-type "));
            let input = Expr {
                source: SourceRange { start: 0, end: 1 },
                ty: ty.clone(),
                kind: K::Unit,
            };
            checked
                .validate(tag, &vec![input.clone(); arity], &ty)
                .unwrap();
            for wrong in [
                P::F32,
                P::F64,
                P::Bool,
                P::I16,
                P::I32,
                P::Isize,
                P::Usize,
                P::BitVec(16),
                P::Void,
            ] {
                if wrong == P::Usize {
                    continue;
                }
                let wrong = Term::Primitive(wrong);
                assert_eq!(
                    registry.check(tag, &vec![wrong.clone(); arity], &wrong),
                    Err(Error::OutputType(tag))
                );
                let forged = Expr {
                    ty: wrong.clone(),
                    ..input.clone()
                };
                assert!(checked.validate(tag, &vec![forged; arity], &wrong).is_err());
            }
            let other = Term::Primitive(if primitive == P::U8 { P::U16 } else { P::U8 });
            for index in 0..arity {
                let mut inputs = vec![ty.clone(); arity];
                inputs[index] = other.clone();
                assert!(matches!(registry.check(tag, &inputs, &ty),
                    Err(Error::OperandType { index: rejected, .. }) if rejected == index));
                let mut forged = vec![input.clone(); arity];
                forged[index].ty = other.clone();
                assert!(checked.validate(tag, &forged, &ty).is_err());
            }
            for count in [0, arity - 1, arity + 1] {
                assert!(matches!(
                    registry.check(tag, &vec![ty.clone(); count], &ty),
                    Err(Error::OperandCount { .. })
                ));
            }
            // Rejected rewrites do not alter the original evidence.
            checked.validate(tag, &vec![input; arity], &ty).unwrap();
        }
    }
}

#[test]
fn integer_intrinsic_generic_domain_requires_source_proof() {
    for operation in [
        Operation::FunnelShiftLeft,
        Operation::FunnelShiftRight,
        Operation::ByteSwap,
        Operation::BitReverse,
        Operation::PopulationCount,
        Operation::LeadingZeros,
        Operation::TrailingZeros,
    ] {
        for constraint in [
            "",
            "requires T == .f32'builtin or T == .f64'builtin,",
            "requires T == .u8'builtin or T == bool,",
        ] {
            let parameters = if operation.operand_count() == 3 {
                "a:T,b:T,amount:T"
            } else {
                "value:T"
            };
            let arguments = if operation.operand_count() == 3 {
                "a,b,amount"
            } else {
                "value"
            };
            let source = format!(
                "^intrinsic_enum enum Ops {{bits}} pure fn bits({parameters})->T where T, {constraint} {{unsafe {{Ops.bits'intrinsic({arguments}).as(T)}}}}"
            );
            let bindings = [(operation, "input.Ops.bits".into())];
            let result = support::compile_with_source_bindings(
                &source,
                &Default::default(),
                luna::protocols::type_constructors::Selectors {
                    host_intrinsics: &bindings,
                    ..Default::default()
                },
            );
            assert!(result.is_err(), "unproved domain accepted: {source}");
            let error = result.unwrap_err();
            assert!(
                error.contains("type requirement"),
                "wrong rejection for {source}: {error}"
            );
        }
    }
}

#[test]
fn integer_generic_rebinding_cannot_exchange_same_arity_domains_or_operations() {
    let source = "^intrinsic_enum enum Ops {bits}
        pure fn bits(value:T)->T where T, requires T == .u32'builtin {
            unsafe {Ops.bits'intrinsic(value).as(T)}
        }
        fn run()->.u32'builtin {bits<.u32'builtin>(1)}";
    let package = support::compile_with_source_bindings(
        source,
        &Default::default(),
        luna::protocols::type_constructors::Selectors {
            host_intrinsics: &[(Operation::BitReverse, "input.Ops.bits".into())],
            ..Default::default()
        },
    )
    .unwrap();
    let identity = tag(&package, "Ops", 0);
    let valid = Registry::new(&package, &[(identity, Operation::BitReverse)]).unwrap();
    for other in [
        Operation::SquareRoot,
        Operation::PopulationCount,
        Operation::ByteSwap,
    ] {
        let wrong = Registry::new(&package, &[(identity, other)]).unwrap();
        let error = luna::host::intrinsic::bind(package.clone(), &wrong, &mut 100000).unwrap_err();
        assert!(error.contains("matching source admission"), "{error}");
    }
    let empty = Registry::new(&package, &[]).unwrap();
    let unbound = luna::host::intrinsic::bind(package.clone(), &empty, &mut 100000).unwrap();
    assert!(
        luna::host::intrinsic::bind(unbound, &valid, &mut 100000)
            .unwrap_err()
            .contains("matching source admission")
    );
    assert!(luna::host::intrinsic::bind(package.clone(), &valid, &mut 0).is_err());
    let bound = luna::host::intrinsic::bind(package, &valid, &mut 100000).unwrap();
    let ir = luna::host::llvm_ir(&bound, "run", 100000).unwrap();
    assert!(ir.contains("@llvm.bitreverse.i32"), "{ir}");
    assert!(!ir.contains("@llvm.bitreverse.f32"));
}

#[test]
fn integer_zero_counts_lower_with_a_defined_zero_flag() {
    for operation in [Operation::LeadingZeros, Operation::TrailingZeros] {
        for width in [8, 16, 32, 64, 128] {
            let source = format!(
                "^intrinsic_enum enum Ops {{bits}}
                pure fn bits(value:.u{width}'builtin)->.u{width}'builtin {{
                    unsafe {{Ops.bits'intrinsic(value).as(.u{width}'builtin)}}
                }}
                fn run()->bool {{bits(0)=={width}}}"
            );
            let package = support::compile_with_source_bindings(
                &source,
                &Default::default(),
                luna::protocols::type_constructors::Selectors {
                    host_intrinsics: &[(operation, "input.Ops.bits".into())],
                    ..Default::default()
                },
            )
            .unwrap();
            let registry =
                Registry::new(&package, &[(tag(&package, "Ops", 0), operation)]).unwrap();
            let bound = luna::host::intrinsic::bind(package, &registry, &mut 100000).unwrap();
            let ir = luna::host::llvm_ir(&bound, "run", 100000).unwrap();
            let symbol = format!("@llvm.{}.i{width}", operation.name());
            assert!(
                ir.contains(&format!("declare i{width} {symbol}(i{width}, i1 immarg)")),
                "{ir}"
            );
            let calls: Vec<_> = ir
                .lines()
                .filter(|line| line.contains("call ") && line.contains(&symbol))
                .collect();
            assert!(!calls.is_empty(), "{ir}");
            assert!(
                calls.iter().all(|line| line.contains(", i1 false)")),
                "{ir}"
            );
        }
    }
}

#[test]
fn scalar_intrinsic_predicates_revalidate_evidence_and_all_operands() {
    use nir::program::{LocalId, checkable_predicate};
    let package = support::compile("^intrinsic_enum enum Ops {bound}");
    let tag = tag(&package, "Ops", 0);
    let make = |ty, kind| Expr {
        source: SourceRange { start: 0, end: 1 },
        ty,
        kind,
    };
    let predicate = |value: Expr| {
        make(
            Term::Primitive(P::Bool),
            K::ScalarEqual(Box::new(value.clone()), Box::new(value)),
        )
    };
    for &operation in Operation::ALL {
        let ty = Term::Primitive(operation.scalar_types()[0]);
        let result = operation.result_type(&ty);
        let registry = Registry::new(&package, &[(tag, operation)]).unwrap();
        let input_types = (0..operation.operand_count())
            .map(|index| operation.operand_type(&ty, index))
            .collect::<Vec<_>>();
        let binding = registry.check(tag, &input_types, &result).unwrap();
        let inputs = input_types
            .into_iter()
            .enumerate()
            .map(|(index, ty)| {
                make(
                    ty,
                    K::Local(LocalId {
                        owner: 0,
                        index: index as u32,
                    }),
                )
            })
            .collect();
        let original = make(
            result.clone(),
            K::Intrinsic {
                tag,
                inputs,
                binding: Some(binding),
            },
        );
        assert!(
            checkable_predicate(&predicate(original.clone())),
            "{operation:?}"
        );
        for mutation in 0..5 {
            let mut forged = original.clone();
            let K::Intrinsic {
                tag,
                inputs,
                binding,
            } = &mut forged.kind
            else {
                unreachable!()
            };
            match mutation {
                0 => *binding = None,
                1 => tag.variant += 1,
                2 => inputs[0].ty = Term::Primitive(P::Bool),
                3 => {
                    inputs.pop();
                }
                _ => {
                    forged.ty = if operation.is_float_predicate() {
                        ty.clone()
                    } else {
                        Term::Primitive(P::Bool)
                    }
                }
            }
            assert!(
                !checkable_predicate(&predicate(forged)),
                "{operation:?}: {mutation}"
            );
        }
        for index in 0..operation.operand_count() {
            let mut written = original.clone();
            let K::Intrinsic { inputs, .. } = &mut written.kind else {
                unreachable!()
            };
            let value = inputs[index].clone();
            inputs[index] = make(
                value.ty.clone(),
                K::Block {
                    statements: vec![make(
                        Term::Primitive(P::Void),
                        K::Assign {
                            place: nir::program::Place {
                                root: LocalId { owner: 0, index: 0 },
                                fields: vec![],
                            },
                            value: Box::new(value.clone()),
                        },
                    )],
                    result: Some(Box::new(value)),
                },
            );
            assert!(
                !checkable_predicate(&predicate(written)),
                "{operation:?}: input {index}"
            );
        }
        assert!(checkable_predicate(&predicate(original)));
        let abstract_ty = Term::Rigid {
            declaration: nir::DeclarationId {
                package: 9,
                local: 7,
            },
            sort: nir::Sort::Type,
        };
        let input_types = (0..operation.operand_count())
            .map(|index| operation.operand_type(&abstract_ty, index))
            .collect::<Vec<_>>();
        let pending = registry
            .defer(tag, &input_types, &operation.result_type(&abstract_ty))
            .unwrap();
        let inputs = input_types
            .into_iter()
            .enumerate()
            .map(|(index, ty)| {
                make(
                    ty,
                    K::Local(LocalId {
                        owner: 9,
                        index: index as u32,
                    }),
                )
            })
            .collect();
        let template = make(
            operation.result_type(&abstract_ty),
            K::Intrinsic {
                tag,
                inputs,
                binding: Some(pending),
            },
        );
        assert!(checkable_predicate(&predicate(template.clone())));
        assert!(
            nir::intrinsics::checked(&template).is_err(),
            "template shape must not authorize execution"
        );
    }
}

#[test]
fn float_classification_requires_float_input_and_bool_result() {
    let package = support::compile("^intrinsic_enum enum Ops {classify}");
    let tag = tag(&package, "Ops", 0);
    for op in [
        Operation::IsNan,
        Operation::IsInfinite,
        Operation::IsFinite,
        Operation::IsNormal,
        Operation::IsSubnormal,
        Operation::SignBit,
    ] {
        let registry = Registry::new(&package, &[(tag, op)]).unwrap();
        for primitive in [P::F32, P::F64] {
            let input = Term::Primitive(primitive);
            let output = Term::Primitive(P::Bool);
            assert!(registry.check(tag, &[input.clone()], &output).is_ok());
            assert!(registry.check(tag, &[input.clone()], &input).is_err());
            assert!(registry.check(tag, &[output.clone()], &output).is_err());
            assert!(registry.check(tag, &[], &output).is_err());
            assert!(
                registry
                    .check(tag, &[input.clone(), input], &output)
                    .is_err()
            );
        }
    }
}

#[test]
fn integer_math_intrinsics_reject_mixed_signs_widths_and_nonintegers() {
    let package = support::compile("^intrinsic_enum enum Ops {sat}");
    let tag = tag(&package, "Ops", 0);
    for op in [
        Operation::SaturatingAdd,
        Operation::SaturatingSubtract,
        Operation::IntegerMinimum,
        Operation::IntegerMaximum,
    ] {
        let registry = Registry::new(&package, &[(tag, op)]).unwrap();
        for &p in op.scalar_types() {
            let ty = Term::Primitive(p);
            assert!(registry.check(tag, &[ty.clone(), ty.clone()], &ty).is_ok());
            for q in [P::U8, P::I8, P::U128, P::I128, P::F32, P::F64, P::Bool] {
                let other = Term::Primitive(q);
                if other != ty {
                    assert!(
                        registry
                            .check(tag, &[ty.clone(), other.clone()], &ty)
                            .is_err()
                    );
                    assert!(
                        registry
                            .check(tag, &[ty.clone(), ty.clone()], &other)
                            .is_err()
                    );
                }
            }
            for count in [0, 1, 3] {
                assert!(registry.check(tag, &vec![ty.clone(); count], &ty).is_err());
            }
        }
        for p in [P::F32, P::F64, P::Bool] {
            let ty = Term::Primitive(p);
            assert!(registry.check(tag, &[ty.clone(), ty.clone()], &ty).is_err());
        }
    }
}

#[test]
fn wrapping_absolute_value_requires_one_signed_input_and_same_result() {
    let package = support::compile("^intrinsic_enum enum Ops {absolute}");
    let tag = tag(&package, "Ops", 0);
    let registry = Registry::new(&package, &[(tag, Operation::WrappingAbsoluteValue)]).unwrap();
    for p in [P::I8, P::I16, P::I32, P::I64, P::I128, P::Isize] {
        let ty = Term::Primitive(p);
        assert!(registry.check(tag, &[ty.clone()], &ty).is_ok());
        assert!(registry.check(tag, &[], &ty).is_err());
        assert!(registry.check(tag, &[ty.clone(), ty.clone()], &ty).is_err());
        assert!(
            registry
                .check(tag, &[ty.clone()], &Term::Primitive(P::U128))
                .is_err()
        );
    }
    for p in [
        P::U8,
        P::U16,
        P::U32,
        P::U64,
        P::U128,
        P::F32,
        P::F64,
        P::Bool,
        P::Usize,
    ] {
        let ty = Term::Primitive(p);
        assert!(registry.check(tag, &[ty.clone()], &ty).is_err());
    }
}

#[test]
fn bitwise_intrinsics_keep_signed_unsigned_identity_and_reject_unproved_domains() {
    let package = support::compile("^intrinsic_enum enum Ops {bits}");
    let tag = tag(&package, "Ops", 0);
    for operation in [Operation::BitAnd, Operation::BitOr, Operation::BitXor] {
        let registry = Registry::new(&package, &[(tag, operation)]).unwrap();
        for primitive in [
            P::U8,
            P::U16,
            P::U32,
            P::U64,
            P::U128,
            P::I8,
            P::I16,
            P::I32,
            P::I64,
            P::I128,
        ] {
            let ty = Term::Primitive(primitive);
            let checked = registry.check(tag, &[ty.clone(), ty.clone()], &ty).unwrap();
            assert_eq!(
                checked.width(),
                Some(primitive.fixed_integer_layout().unwrap().1)
            );
            let other = Term::Primitive(if primitive == P::U8 { P::I8 } else { P::U8 });
            assert!(registry.check(tag, &[ty.clone(), other], &ty).is_err());
            assert!(registry.check(tag, &[ty.clone()], &ty).is_err());
            assert!(checked.to_string().contains(&ty.to_string()));
        }
        for primitive in [P::Bool, P::F32, P::F64, P::BitVec(8), P::Void] {
            let ty = Term::Primitive(primitive);
            assert!(registry.check(tag, &[ty.clone(), ty.clone()], &ty).is_err());
        }
    }
}

#[test]
fn platform_bitwise_binding_reads_width_without_erasing_scalar_identity() {
    let package = support::compile("^intrinsic_enum enum Ops {bits}");
    let tag = tag(&package, "Ops", 0);
    for operation in [Operation::BitAnd, Operation::BitOr, Operation::BitXor] {
        let registry = Registry::new(&package, &[(tag, operation)]).unwrap();
        for primitive in [P::Usize, P::Isize] {
            let ty = Term::Primitive(primitive);
            let checked = registry.check(tag, &[ty.clone(), ty.clone()], &ty).unwrap();
            assert_eq!(checked.width(), None);
            for width in [8, 32, 64, 128] {
                assert_eq!(
                    checked.width_for_target(Some(nir::target::TargetData::new(width).unwrap())),
                    Some(width)
                );
            }
            for other in [
                P::U64,
                P::I64,
                if primitive == P::Usize {
                    P::Isize
                } else {
                    P::Usize
                },
            ] {
                assert!(
                    registry
                        .check(tag, &[ty.clone(), Term::Primitive(other)], &ty)
                        .is_err()
                );
            }
            assert!(checked.to_string().contains(&ty.to_string()));
        }
    }
}

fn check_integer_exponent_signature(operation: Operation) {
    let package = support::compile("^intrinsic_enum enum Ops {power}");
    let tag = tag(&package, "Ops", 0);
    let registry = Registry::new(&package, &[(tag, operation)]).unwrap();
    let exponent = Term::Primitive(P::I32);
    for scalar in [Term::Primitive(P::F32), Term::Primitive(P::F64)] {
        let binding = registry
            .check(tag, &[scalar.clone(), exponent.clone()], &scalar)
            .unwrap();
        assert_eq!(binding.operation(), operation);
        let mut operands = vec![scalar.clone(), exponent.clone()]
            .into_iter()
            .enumerate()
            .map(|(index, ty)| Expr {
                source: SourceRange { start: 0, end: 1 },
                ty,
                kind: K::Local(nir::program::LocalId {
                    owner: 0,
                    index: index as u32,
                }),
            })
            .collect::<Vec<_>>();
        assert!(binding.validate(tag, &operands, &scalar).is_ok());
        operands[1].ty = scalar.clone();
        assert!(binding.validate(tag, &operands, &scalar).is_err());

        for wrong in [P::F32, P::F64, P::U32, P::I64, P::Integer] {
            assert_eq!(
                registry.check(tag, &[scalar.clone(), Term::Primitive(wrong)], &scalar),
                Err(Error::OperandType { tag, index: 1 })
            );
        }
        assert!(
            registry
                .check(tag, std::slice::from_ref(&scalar), &scalar)
                .is_err()
        );
        assert!(
            registry
                .check(tag, &[exponent.clone(), scalar.clone()], &scalar)
                .is_err()
        );
    }
    let generic = Term::Rigid {
        declaration: nir::DeclarationId {
            package: 1,
            local: 42,
        },
        sort: nir::Sort::Type,
    };
    assert!(
        registry
            .defer(tag, &[generic.clone(), exponent.clone()], &generic)
            .is_ok()
    );
    assert!(
        registry
            .defer(tag, &[generic.clone(), generic.clone()], &generic)
            .is_err()
    );
}

#[test]
fn powi_requires_float_base_and_signed_i32_exponent_including_pending_templates() {
    check_integer_exponent_signature(Operation::IntegerPower);
}

#[test]
fn ldexp_requires_float_base_and_signed_i32_exponent_including_pending_templates() {
    check_integer_exponent_signature(Operation::ScaleByPowerOfTwo);
}
