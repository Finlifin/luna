use luna::build_context::metadata::{
    Entry, Input,
    wire::{MAX_BYTES, decode_hex, encode_hex},
};
use nir::{DeclarationId, Primitive as P, Sort, Term as T};
fn entry(ty: T, value: T) -> Entry {
    Entry::new(ty, value, &mut 100_000).unwrap()
}
fn packet(fields: Vec<(String, Entry)>) -> Vec<Input> {
    vec![Input {
        identity: 200,
        fields,
    }]
}
#[test]
fn wire_preserves_nominal_normal_form_optional_and_numeric_identity() {
    let id = DeclarationId {
        package: 200,
        local: 7,
    };
    let nominal = T::Nominal {
        declaration: id,
        arguments: vec![],
    };
    let normal = T::NormalForm {
        head: id,
        sort: Sort::Type,
        arguments: vec![T::TypeListValue(vec![T::Primitive(P::U32)])],
    };
    let optional = T::Optional(Box::new(nominal.clone()));
    let mut budget = comptime_runtime::Budget::default();
    let number =
        comptime_runtime::Integer::parse_literal("0x123456789abcdef0123456789", &mut budget)
            .unwrap()
            .negated();
    let bits = comptime_runtime::Integer::from_u128(31)
        .to_bitvec(5, &mut budget)
        .unwrap();
    let decimal =
        comptime_runtime::decimal::DecimalValue::parse_literal("-0.0e12", &mut budget).unwrap();
    let values = packet(vec![
        ("opaque".into(), entry(T::Primitive(P::Type), normal)),
        (
            "enum".into(),
            entry(
                nominal.clone(),
                T::NominalValue {
                    ty: Box::new(nominal),
                    variant: Some(1),
                    fields: vec![],
                },
            ),
        ),
        (
            "missing".into(),
            entry(
                optional.clone(),
                T::OptionalValue {
                    ty: Box::new(optional),
                    value: None,
                },
            ),
        ),
        (
            "integer".into(),
            entry(T::Primitive(P::Integer), T::Integer(number)),
        ),
        (
            "bits".into(),
            entry(T::Primitive(P::BitVec(5)), T::BitVec(bits)),
        ),
        (
            "negative-zero".into(),
            entry(T::Primitive(P::Float), T::Float(decimal)),
        ),
        (
            "nan".into(),
            entry(T::Primitive(P::F64), T::Float64(0x7ff8000000000042)),
        ),
        (
            "escaped\n\"λ".into(),
            entry(T::Primitive(P::Str), T::String("λ\n\0\\\"".into())),
        ),
    ]);
    let encoded = encode_hex(&values).unwrap();
    assert_eq!(decode_hex(&encoded).unwrap(), values);
    assert_eq!(encode_hex(&decode_hex(&encoded).unwrap()).unwrap(), encoded);
    assert_eq!(decode_hex(&encoded.to_uppercase()).unwrap(), values);
}
#[test]
fn wire_rejects_truncation_trailing_data_unknown_versions_and_bad_lengths() {
    let good = encode_hex(&packet(vec![(
        "enabled".into(),
        entry(T::Primitive(P::Bool), T::Bool(true)),
    )]))
    .unwrap();
    for cut in 0..good.len() {
        assert!(decode_hex(&good[..cut]).is_err(), "cut {cut}");
    }
    for bad in [
        format!("{good}00"),
        good.replacen("464d4401", "464d4402", 1),
        "zz".into(),
        "00".repeat(MAX_BYTES + 1),
        "464d4401ffffffffffffffff".into(),
    ] {
        assert!(decode_hex(&bad).is_err());
    }
    assert_eq!(decode_hex(&encode_hex(&[]).unwrap()).unwrap(), vec![]);
    // A syntactically intact packet must still reject invalid boolean tags.
    let mut bad = good.clone();
    bad.replace_range(bad.len() - 2.., "02");
    assert!(decode_hex(&bad).is_err());
    assert!(decode_hex(&good).is_ok());
}
#[test]
fn wire_refuses_executable_descriptors_and_bounds_nesting_and_bytes() {
    let callable = T::Function {
        parameters: vec![],
        result: Box::new(T::Primitive(P::Bool)),
    }
    .with_comptime_phase(true);
    let value = T::ClosureSnapshot {
        factory: DeclarationId {
            package: 200,
            local: 1,
        },
        arguments: vec![],
    };
    assert!(encode_hex(&packet(vec![("factory".into(), entry(callable, value))])).is_err());
    let mut ty = T::Primitive(P::Bool);
    for _ in 0..65 {
        ty = T::Optional(Box::new(ty));
    }
    assert!(
        encode_hex(&packet(vec![(
            "deep".into(),
            entry(T::Primitive(P::Type), ty)
        )]))
        .is_err()
    );
    assert!(
        encode_hex(&packet(vec![(
            "x".repeat(MAX_BYTES),
            entry(T::Primitive(P::Bool), T::Bool(true))
        )]))
        .is_err()
    );
}

#[test]
fn wire_v1_matches_an_independent_boolean_packet() {
    let expected = concat!(
        "464d4401",
        "0100000000000000",
        "c800000000000000",
        "0100000000000000",
        "0100000000000000",
        "78",
        "0002",
        "0201"
    );
    let values = packet(vec![(
        "x".into(),
        entry(T::Primitive(P::Bool), T::Bool(true)),
    )]);
    assert_eq!(encode_hex(&values).unwrap(), expected);
    assert_eq!(decode_hex(expected).unwrap(), values);
    let mut invalid_utf8 = expected.to_owned();
    invalid_utf8.replace_range(72..74, "ff");
    assert!(decode_hex(&invalid_utf8).unwrap_err().contains("UTF-8"));
    let mut unknown = expected.to_owned();
    unknown.replace_range(78..80, "fe");
    assert!(
        decode_hex(&unknown)
            .unwrap_err()
            .contains("unknown descriptor tag")
    );
}
