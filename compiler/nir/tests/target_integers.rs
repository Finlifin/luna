use nir::{Primitive as P, Term, program::Literal, target::TargetData};

#[test]
fn captured_comptime_integer_checks_target_domain_and_recovers() {
    use comptime_runtime::Integer;
    for (primitive, value, fits32, fits64) in [
        (P::Usize, 1i128 << 32, false, true),
        (P::Usize, -1, false, false),
        (P::Isize, i32::MIN as i128, true, true),
        (P::Isize, i32::MIN as i128 - 1, false, true),
        (P::Isize, i64::MAX as i128 + 1, false, false),
    ] {
        let value = Term::Integer(Integer::from_i128(value));
        let domain = Term::Primitive(primitive);
        for (bits, fits) in [(32, fits32), (64, fits64)] {
            let target = Some(TargetData::new(bits).unwrap());
            let mut fuel = 1000;
            assert_eq!(
                nir::comptime_values::validate_in(&value, &domain, target, &mut fuel).is_ok(),
                fits
            );
            assert!(fuel < 1000);
            assert!(nir::comptime_values::validate_in(&value, &domain, target, &mut 0).is_err());
            assert_eq!(
                nir::comptime_values::validate_in(&value, &domain, target, &mut 1000).is_ok(),
                fits
            );
        }
        assert!(nir::comptime_values::validate(&value, &domain, &mut 1000).is_err());
    }
}

#[test]
fn platform_literal_ranges_follow_explicit_target() {
    let t32 = Some(TargetData::new(32).unwrap());
    let t64 = Some(TargetData::new(64).unwrap());
    let unsigned = Term::Primitive(P::Usize);
    let signed = Term::Primitive(P::Isize);
    for (value, fits32, fits64) in [
        (0, true, true),
        (u32::MAX as u128, true, true),
        (u32::MAX as u128 + 1, false, true),
        (u64::MAX as u128, false, true),
        (u64::MAX as u128 + 1, false, false),
    ] {
        let literal = Literal::UnsignedInteger(value);
        assert_eq!(literal.has_primitive_type_in(&unsigned, t32), fits32);
        assert_eq!(literal.has_primitive_type_in(&unsigned, t64), fits64);
        assert!(!literal.has_primitive_type(&unsigned));
        assert!(!literal.has_primitive_type_in(&signed, t64));
    }
    for (value, fits32, fits64) in [
        (i32::MIN as i128, true, true),
        (i32::MIN as i128 - 1, false, true),
        (i32::MAX as i128, true, true),
        (i32::MAX as i128 + 1, false, true),
        (i64::MIN as i128, false, true),
        (i64::MIN as i128 - 1, false, false),
        (i64::MAX as i128 + 1, false, false),
    ] {
        let literal = Literal::SignedInteger(value);
        assert_eq!(literal.has_primitive_type_in(&signed, t32), fits32);
        assert_eq!(literal.has_primitive_type_in(&signed, t64), fits64);
        assert!(!literal.has_primitive_type(&signed));
        assert!(!literal.has_primitive_type_in(&unsigned, t64));
    }
}

#[test]
fn target_data_does_not_redefine_fixed_types_or_accept_invalid_widths() {
    for bits in [0, 7, 31, 129, 256, u32::MAX] {
        assert!(TargetData::new(bits).is_err());
    }
    for bits in [16, 32, 64, 128] {
        let target = TargetData::new(bits).unwrap();
        assert_eq!(target.pointer_bits(), bits);
        assert_eq!(target.integer_layout(P::Usize), Some((false, bits)));
        assert_eq!(target.integer_layout(P::I64), Some((true, 64)));
        assert_eq!(target.integer_layout(P::Bool), None);
        assert_eq!(
            target.to_string(),
            format!("(target-data (pointer-bits {bits}))")
        );
        assert!(
            Literal::UnsignedInteger(u64::MAX as u128)
                .has_primitive_type_in(&Term::Primitive(P::U64), Some(target))
        );
    }
    assert_ne!(Term::Primitive(P::Usize), Term::Primitive(P::U64));
    assert_eq!(P::Usize.fixed_integer_layout(), None);
    let target = Some(TargetData::new(128).unwrap());
    assert!(
        Literal::UnsignedInteger(u128::MAX)
            .has_primitive_type_in(&Term::Primitive(P::Usize), target)
    );
    assert!(
        Literal::SignedInteger(i128::MIN).has_primitive_type_in(&Term::Primitive(P::Isize), target)
    );
}
