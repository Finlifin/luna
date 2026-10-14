use comptime_runtime::{Budget, Error, Integer};
use nir::{Term, program::IntegerOp as Op};

#[test]
fn exact_scalar_operations_share_canonical_values_and_bounded_work() {
    let left = Integer::from_i128(-5);
    let right = Integer::from_u128(3);
    for (op, expected) in [(Op::Add, -2), (Op::Subtract, -8), (Op::Multiply, -15)] {
        assert_eq!(
            op.evaluate(&left, &right, &mut Budget::default()).unwrap(),
            Term::Integer(Integer::from_i128(expected))
        );
    }
    for (op, expected) in [
        (Op::Equal, false),
        (Op::NotEqual, true),
        (Op::Less, true),
        (Op::LessEqual, true),
        (Op::Greater, false),
        (Op::GreaterEqual, false),
    ] {
        assert_eq!(
            op.evaluate(&left, &right, &mut Budget::default()).unwrap(),
            Term::Bool(expected)
        );
    }
    for op in [Op::Equal, Op::LessEqual, Op::GreaterEqual] {
        assert_eq!(
            op.evaluate(&left, &left, &mut Budget::default()).unwrap(),
            Term::Bool(true)
        );
    }
    for op in [
        Op::Add,
        Op::Subtract,
        Op::Multiply,
        Op::Equal,
        Op::NotEqual,
        Op::Less,
        Op::LessEqual,
        Op::Greater,
        Op::GreaterEqual,
    ] {
        assert_eq!(
            op.evaluate(
                &left,
                &right,
                &mut Budget {
                    steps: 0,
                    max_limbs: 4096
                }
            ),
            Err(Error::BudgetExhausted)
        );
        assert_eq!(
            op.evaluate(
                &left,
                &right,
                &mut Budget {
                    steps: 100,
                    max_limbs: 0
                }
            ),
            Err(Error::SizeLimit)
        );
    }
}

#[test]
fn checked_negation_preserves_zero_and_rejects_excess_work() {
    let zero = Integer::from_u128(0);
    assert_eq!(zero.negate(&mut Budget::default()).unwrap(), zero);
    let value = Integer::from_i128(-19);
    assert_eq!(
        value.negate(&mut Budget::default()).unwrap(),
        Integer::from_u128(19)
    );
    assert_eq!(
        value.negate(&mut Budget {
            steps: 0,
            max_limbs: 4096
        }),
        Err(Error::BudgetExhausted)
    );
    assert_eq!(
        value.negate(&mut Budget {
            steps: 100,
            max_limbs: 0
        }),
        Err(Error::SizeLimit)
    );
    assert_eq!(value.to_signed(128), Ok(-19));
}

#[test]
fn explicit_truncating_division_has_signed_results_and_rejects_zero() {
    for a in -24i128..=24 {
        for b in -9i128..=9 {
            for op in [Op::DivideTrunc, Op::RemainderTrunc] {
                let result = op.evaluate(
                    &Integer::from_i128(a),
                    &Integer::from_i128(b),
                    &mut Budget::default(),
                );
                if b == 0 {
                    assert_eq!(result, Err(Error::DivisionByZero));
                } else {
                    let expected = if op == Op::DivideTrunc { a / b } else { a % b };
                    assert_eq!(result, Ok(Term::Integer(Integer::from_i128(expected))));
                }
                assert!(!op.is_comparison());
            }
        }
    }
    for op in [Op::DivideTrunc, Op::RemainderTrunc] {
        assert_eq!(
            op.evaluate(
                &Integer::from_u128(50),
                &Integer::from_u128(3),
                &mut Budget {
                    steps: 1,
                    max_limbs: 4
                }
            ),
            Err(Error::BudgetExhausted)
        );
    }
    assert_eq!(Op::DivideTrunc.to_string(), "divide-trunc");
    assert_eq!(Op::RemainderTrunc.to_string(), "remainder-trunc");
}

#[test]
fn exact_integer_transport_spelling_is_bounded_and_distinct_from_dump() {
    for value in [0i128, 1, -1, i128::MIN, i128::MAX] {
        let integer = Integer::from_i128(value);
        let spelling = integer.to_hex_literal(&mut Budget::default()).unwrap();
        let (negative, magnitude) = spelling
            .strip_prefix('-')
            .map_or((false, spelling.as_str()), |s| (true, s));
        let mut parsed = Integer::parse_literal(magnitude, &mut Budget::default()).unwrap();
        if negative {
            parsed = parsed.negate(&mut Budget::default()).unwrap();
        }
        assert_eq!(parsed, integer);
        assert_ne!(spelling, integer.to_string());
        assert_eq!(
            integer.to_hex_literal(&mut Budget {
                steps: 0,
                max_limbs: 4096
            }),
            Err(Error::BudgetExhausted)
        );
    }
}

#[test]
fn completed_float_values_have_bit_identity_and_stable_dumps() {
    use nir::{Primitive,Sort};
    let values=[Term::Float32(0),Term::Float32(1<<31),Term::Float32(0x7fc00042),Term::Float32(0x7fc00043),
        Term::Float64(0),Term::Float64(1<<63),Term::Float64(0x7ff8000000000042)];
    for (i,value) in values.iter().enumerate() {
        assert_eq!(value.sort(),Sort::ComptimeValue);
        value.check_scope(&[]).unwrap();
        assert_eq!(&value.abstract_parameters(&[]).unwrap(),value);
        for (j,other) in values.iter().enumerate() {assert_eq!(value==other,i==j);}
    }
    assert_eq!(Term::Float32(0x7fc00042).to_string(),"(float32-bits 0x7fc00042)");
    assert_eq!(Term::Float64(1<<63).to_string(),"(float64-bits 0x8000000000000000)");
    assert_ne!(Term::Float32(0),Term::Primitive(Primitive::F32));
}

#[test]
fn completed_records_keep_labels_scope_and_escaped_dumps() {
    use nir::Sort;
    let name = "x\"\n".to_string();
    let value = Term::RecordValue(vec![(name.clone(), Term::Bound { depth: 0, index: 0, sort: Sort::Type })]);
    assert_eq!(value.sort(), Sort::ComptimeValue);
    assert!(value.check_scope(&[vec![Sort::Type]]).is_ok());
    assert!(value.check_scope(&[]).is_err());
    let closed = value.substitute(&[Term::Primitive(nir::Primitive::Bool)]).unwrap();
    assert_eq!(closed, Term::RecordValue(vec![(name, Term::Primitive(nir::Primitive::Bool))]));
    assert_eq!(closed.to_string(), "(record-value (fields (field \"x\\\"\\n\" (primitive bool))))");
    assert_ne!(closed, Term::RecordValue(vec![("different".into(), Term::Primitive(nir::Primitive::Bool))]));
    assert!(Term::RecordValue(vec![("x".into(),Term::Bool(true)),("x".into(),Term::Bool(false))]).check_scope(&[]).is_err());
}

#[test]
fn nominal_values_rewrite_scope_without_erasing_identity_or_variant() {
    use nir::{DeclarationId,Sort,Primitive};
    let variable=Term::Bound {depth:0,index:0,sort:Sort::Type};
    let value=Term::NominalValue {
        ty:Box::new(Term::Nominal {declaration:DeclarationId {package:3,local:7},arguments:vec![variable.clone()]}),
        variant:Some(2),fields:vec![variable],
    };
    value.check_scope(&[vec![Sort::Type]]).unwrap();
    assert!(value.check_scope(&[]).is_err());
    let closed=value.substitute(&[Term::Primitive(Primitive::Bool)]).unwrap();
    closed.check_scope(&[]).unwrap();
    let Term::NominalValue {ty,variant,fields}=&closed else {panic!()};
    assert_eq!(*variant,Some(2));
    assert_eq!(fields,&vec![Term::Primitive(Primitive::Bool)]);
    assert!(matches!(ty.as_ref(),Term::Nominal {arguments,..} if arguments==fields));
    assert_ne!(closed,Term::NominalValue {ty:ty.clone(),variant:None,fields:fields.clone()});
    let mut fuel=2;
    assert!(nir::type_computations::charge_term(&closed,&mut fuel).is_err());
    nir::type_computations::charge_term(&closed,&mut 100).unwrap();
    assert!(Term::NominalValue {ty:Box::new(Term::Primitive(Primitive::Bool)),variant:None,fields:vec![]}.check_scope(&[]).is_err());
}
