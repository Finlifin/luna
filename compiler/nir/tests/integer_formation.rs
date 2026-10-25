use comptime_runtime::{Budget, Error};
use nir::{Primitive as P, integer_formation::form, program::Literal, target::TargetData};

#[test]
fn target_word_size_is_explicit_and_range_checked() {
    let spelling = "4294967296";
    assert_eq!(
        form(spelling, false, P::Usize, None, &mut Budget::default()),
        Err(Error::InvalidLiteral)
    );
    assert_eq!(
        form(
            spelling,
            false,
            P::Usize,
            Some(TargetData::new(32).unwrap()),
            &mut Budget::default()
        ),
        Err(Error::OutOfRange)
    );
    assert_eq!(
        form(
            spelling,
            false,
            P::Usize,
            Some(TargetData::new(64).unwrap()),
            &mut Budget::default()
        )
        .unwrap(),
        Literal::UnsignedInteger(4294967296)
    );
}

#[test]
fn full_width_signed_minimum_and_unsigned_maximum_are_exact() {
    assert_eq!(
        form(
            "170141183460469231731687303715884105728",
            true,
            P::I128,
            None,
            &mut Budget::default()
        )
        .unwrap(),
        Literal::SignedInteger(i128::MIN)
    );
    assert_eq!(
        form(
            "340282366920938463463374607431768211455",
            false,
            P::U128,
            None,
            &mut Budget::default()
        )
        .unwrap(),
        Literal::UnsignedInteger(u128::MAX)
    );
    assert_eq!(
        form(
            "340282366920938463463374607431768211456",
            false,
            P::U128,
            None,
            &mut Budget::default()
        ),
        Err(Error::OutOfRange)
    );
    assert!(matches!(
        form(
            "340282366920938463463374607431768211456",
            false,
            P::Integer,
            None,
            &mut Budget::default()
        ),
        Ok(Literal::Integer(_))
    ));
}

#[test]
fn domain_and_budget_failures_allow_independent_retry() {
    assert_eq!(
        form("1", false, P::Bool, None, &mut Budget::default()),
        Err(Error::InvalidLiteral)
    );
    assert!(
        form(
            "255",
            false,
            P::U8,
            None,
            &mut Budget {
                steps: 0,
                ..Budget::default()
            }
        )
        .is_err()
    );
    assert_eq!(
        form("255", false, P::U8, None, &mut Budget::default()).unwrap(),
        Literal::UnsignedInteger(255)
    );
    assert!(form("invalid", false, P::U8, None, &mut Budget::default()).is_err());
}

#[test]
fn failed_completion_keeps_source_and_pending_node_for_retry() {
    use nir::{
        Term,
        program::{Expr, ExprKind, SourceRange},
    };
    let mut expression = Expr {
        source: SourceRange { start: 42, end: 45 },
        ty: Term::Primitive(P::U8),
        kind: ExprKind::Literal(Literal::PendingInteger {
            spelling: "256".into(),
            negative: false,
        }),
    };
    let original = expression.clone();
    let error = nir::integer_formation::complete(&mut expression, None, &mut 10000).unwrap_err();
    assert!(
        error.contains("(source 42 45)") && error.contains("u8"),
        "{error}"
    );
    assert_eq!(expression, original);
    expression.ty = Term::Primitive(P::U16);
    nir::integer_formation::complete(&mut expression, None, &mut 10000).unwrap();
    assert_eq!(
        expression.kind,
        ExprKind::Literal(Literal::UnsignedInteger(256))
    );
}
