use super::*;
use comptime_runtime::{Budget, Integer, decimal::DecimalValue};

fn source() -> SourceRange {
    SourceRange { start: 12, end: 19 }
}
fn build(value: &Term, domain: Primitive) -> Expr {
    expression(
        value,
        &Term::Primitive(domain),
        None,
        source(),
        &mut 100_000,
    )
    .unwrap()
}

#[test]
fn scalar_construction_preserves_bits_exact_values_and_source_domains() {
    let cases = [
        (Term::Bool(true), Primitive::Bool, Literal::Bool(true)),
        (Term::Char('界'), Primitive::Char, Literal::Char('界')),
        (
            Term::String("a\0界".into()),
            Primitive::Str,
            Literal::String("a\0界".into()),
        ),
        (
            Term::Symbol("\"name\"".into()),
            Primitive::Symbol,
            Literal::Symbol("\"name\"".into()),
        ),
        (
            Term::Float16(0x7e35),
            Primitive::F16,
            Literal::Float16(0x7e35),
        ),
        (
            Term::BFloat16(0x8000),
            Primitive::Bf16,
            Literal::BFloat16(0x8000),
        ),
        (
            Term::Float32(0x7fc000ab),
            Primitive::F32,
            Literal::Float32(0x7fc000ab),
        ),
        (
            Term::Float64(0x8000000000000000),
            Primitive::F64,
            Literal::Float64(0x8000000000000000),
        ),
        (
            Term::Integer(Integer::from_i128(-128)),
            Primitive::I8,
            Literal::SignedInteger(-128),
        ),
        (
            Term::Integer(Integer::from_u128(u128::MAX)),
            Primitive::U128,
            Literal::UnsignedInteger(u128::MAX),
        ),
    ];
    for (value, primitive, literal) in cases {
        let result = build(&value, primitive);
        assert_eq!(result.source, source());
        assert_eq!(result.ty, Term::Primitive(primitive));
        assert_eq!(result.kind, ExprKind::Literal(literal));
    }
    assert_eq!(build(&Term::Unit, Primitive::Void).kind, ExprKind::Unit);
    let mut budget = Budget::default();
    let integer =
        Integer::parse_literal("0x100000000000000000000000000000000", &mut budget).unwrap();
    let ExprKind::Literal(Literal::Integer(spelling)) =
        build(&Term::Integer(integer.clone()), Primitive::Integer).kind
    else {
        panic!("exact integer")
    };
    assert_eq!(
        Integer::parse_literal(&spelling, &mut budget).unwrap(),
        integer
    );
    let float = DecimalValue::parse_literal(
        "-123456789.012345678901234567890123456789e-1000",
        &mut budget,
    )
    .unwrap();
    let ExprKind::Literal(Literal::Float(spelling)) =
        build(&Term::Float(float.clone()), Primitive::Float).kind
    else {
        panic!("exact float")
    };
    assert_eq!(
        DecimalValue::parse_literal(&spelling, &mut budget).unwrap(),
        float
    );
    let bits =
        comptime_runtime::bitvec::BitVec::from_words(33, &[u32::MAX, 1], &mut budget).unwrap();
    assert_eq!(
        build(&Term::BitVec(bits), Primitive::BitVec(33)).kind,
        ExprKind::Literal(Literal::BitVec {
            width: 33,
            words: vec![u32::MAX, 1]
        })
    );
}

#[test]
fn scalar_construction_requires_explicit_target_and_never_unfolds_domains() {
    let value = Term::Integer(Integer::from_u128(256));
    let domain = Term::Primitive(Primitive::Usize);
    let target = |width| Some(TargetData::new(width).unwrap());
    assert!(expression(&value, &domain, None, source(), &mut 1000).is_err());
    assert!(expression(&value, &domain, target(8), source(), &mut 1000).is_err());
    let result = expression(&value, &domain, target(16), source(), &mut 1000).unwrap();
    assert_eq!(result.ty, domain);
    assert_eq!(
        result.kind,
        ExprKind::Literal(Literal::UnsignedInteger(256))
    );
    let opaque = Term::NormalForm {
        head: crate::DeclarationId {
            package: 5,
            local: 1,
        },
        sort: crate::Sort::Type,
        arguments: vec![Term::Primitive(Primitive::Bool)],
    };
    for (value, domain) in [
        (Term::Bool(true), opaque),
        (Term::Bool(true), Term::Primitive(Primitive::U8)),
        (value, Term::Primitive(Primitive::U8)),
        (
            Term::Integer(Integer::from_i128(-1)),
            Term::Primitive(Primitive::U8),
        ),
        (
            Term::Integer(Integer::from_i128(128)),
            Term::Primitive(Primitive::I8),
        ),
        (Term::TupleValue(vec![]), Term::Primitive(Primitive::Void)),
        (
            Term::Primitive(Primitive::U8),
            Term::Primitive(Primitive::Type),
        ),
        (
            Term::Bound {
                depth: 0,
                index: 0,
                sort: crate::Sort::ComptimeValue,
            },
            Term::Primitive(Primitive::Bool),
        ),
    ] {
        assert!(matches!(
            expression(&value, &domain, target(64), source(), &mut 1000),
            Err(Error::Invalid(_))
        ));
    }
}

#[test]
fn scalar_construction_charges_payload_and_spelling_and_recovers_after_failure() {
    let float = DecimalValue::parse_literal("-123456789.0e-300", &mut Budget::default()).unwrap();
    for value in [Term::String("界".repeat(100).into()), Term::Float(float)] {
        let domain = Term::Primitive(value.scalar_index_domain().unwrap());
        let mut fuel = 100_000;
        let expected = expression(&value, &domain, None, source(), &mut fuel).unwrap();
        let cost = 100_000 - fuel;
        for steps in 0..cost {
            let mut remaining = steps;
            assert_eq!(
                expression(&value, &domain, None, source(), &mut remaining).unwrap_err(),
                Error::Budget
            );
            assert!(remaining <= steps);
        }
        let mut exact = cost;
        assert_eq!(
            expression(&value, &domain, None, source(), &mut exact).unwrap(),
            expected
        );
        assert_eq!(exact, 0);
    }
}
