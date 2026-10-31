use comptime_runtime::{Budget, Error};
use nir::{Primitive as P, float_formation::FloatFormation, program::Literal};

#[test]
fn one_formation_supports_independent_width_choices_without_double_rounding() {
    let formation = FloatFormation::parse(
        "1.0000000596046447753906250000000001",
        false,
        &mut Budget::default(),
    )
    .unwrap();
    let dump = formation.to_string();
    assert!(dump.starts_with("(float-formation (decimal-literal "));
    assert_eq!(
        formation.form(P::F64, &mut Budget::default()).unwrap(),
        Literal::Float64(0x3ff0000010000000)
    );
    assert_eq!(
        formation.form(P::F32, &mut Budget::default()).unwrap(),
        Literal::Float32(0x3f800001)
    );
    assert_eq!(formation.to_string(), dump);
    let copy = formation.clone();
    assert_eq!(
        copy.form(P::F32, &mut Budget::default()).unwrap(),
        Literal::Float32(0x3f800001)
    );
}

#[test]
fn exact_formation_retains_negative_zero_in_both_widths() {
    let formation = FloatFormation::parse("0.0", true, &mut Budget::default()).unwrap();
    assert_eq!(
        formation.form(P::F32, &mut Budget::default()).unwrap(),
        Literal::Float32(1 << 31)
    );
    assert_eq!(
        formation.form(P::F64, &mut Budget::default()).unwrap(),
        Literal::Float64(1 << 63)
    );
    assert!(matches!(
        formation.form(P::Float, &mut Budget::default()).unwrap(),
        Literal::Float(_)
    ));
    assert_eq!(
        formation.form(P::F32, &mut Budget::default()).unwrap(),
        Literal::Float32(1 << 31)
    );
}

#[test]
fn rejected_domain_overflow_and_exhaustion_do_not_poison_later_formation() {
    let formation = FloatFormation::parse("1.0e40", false, &mut Budget::default()).unwrap();
    assert_eq!(
        formation.form(P::Bool, &mut Budget::default()),
        Err(Error::InvalidLiteral)
    );
    assert_eq!(
        formation.form(P::F32, &mut Budget::default()),
        Err(Error::OutOfRange)
    );
    let mut exhausted = Budget {
        steps: 0,
        ..Budget::default()
    };
    assert!(formation.form(P::F64, &mut exhausted).is_err());
    assert_eq!(
        formation.form(P::F64, &mut Budget::default()).unwrap(),
        Literal::Float64(1e40f64.to_bits())
    );
    assert!(FloatFormation::parse("not-a-number", false, &mut Budget::default()).is_err());
}

#[test]
fn failed_completion_reports_float_domain_and_keeps_the_pending_node() {
    use nir::{
        Term,
        program::{Expr, ExprKind, SourceRange},
    };
    let mut expression = Expr {
        source: SourceRange { start: 42, end: 48 },
        ty: Term::Primitive(P::F32),
        kind: ExprKind::Literal(Literal::PendingFloat {
            spelling: "1.0e40".into(),
            negative: false,
        }),
    };
    let original = expression.clone();
    let error = nir::float_formation::complete(&mut expression, &mut 10000).unwrap_err();
    assert!(error.contains("(source 42 48)"), "{error}");
    assert!(error.contains("f32"), "{error}");
    assert!(error.contains("finite float literal exceeds"), "{error}");
    assert!(!error.contains("integer"), "{error}");
    assert_eq!(expression, original);
    expression.ty = Term::Primitive(P::F64);
    nir::float_formation::complete(&mut expression, &mut 10000).unwrap();
    assert_eq!(
        expression.kind,
        ExprKind::Literal(Literal::Float64(1e40f64.to_bits()))
    );
}

#[test]
fn half_literals_keep_distinct_domains_and_exact_rounding() {
    let formation = FloatFormation::parse(
        "1.00390625000000000000000000000001",
        false,
        &mut Budget::default(),
    )
    .unwrap();
    for domain in [P::F16, P::Bf16] {
        let ty = nir::Term::Primitive(domain);
        assert!(nir::normal_forms::supported_input_type(&ty, &[]));
        assert!(nir::program::intrinsic_pointer_value(&ty));
    }
    let half = formation.form(P::F16, &mut Budget::default()).unwrap();
    let brain = formation.form(P::Bf16, &mut Budget::default()).unwrap();
    assert_eq!(half, Literal::Float16(0x3c04));
    assert_eq!(brain, Literal::BFloat16(0x3f81));
    assert!(half.has_primitive_type(&nir::Term::Primitive(P::F16)));
    assert!(!half.has_primitive_type(&nir::Term::Primitive(P::Bf16)));
    assert!(brain.has_primitive_type(&nir::Term::Primitive(P::Bf16)));
    assert!(!brain.has_primitive_type(&nir::Term::Primitive(P::F16)));
    assert_eq!(half.to_string(), "(float16 (bits \"3c04\"))");
    assert_eq!(brain.to_string(), "(bfloat16 (bits \"3f81\"))");
}
