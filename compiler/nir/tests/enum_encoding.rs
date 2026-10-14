use comptime_runtime::{Budget, bitvec::BitVec};
use nir::enum_encoding::{Error, Field, Layout, Variant};

#[test]
fn symbol_encodings_share_checked_construction_and_discrimination() {
    use nir::enum_encoding::{SymbolRepresentation,SymbolVariant};
    let entries=vec![
        SymbolVariant {name:"client.ready".into(),value:"wire_ready".into()},
        SymbolVariant {name:"closed".into(),value:"wire_\"λ".into()},
    ];
    let layout=SymbolRepresentation::new(entries.clone(),&mut Budget::default()).unwrap();
    for index in 0..entries.len() {
        let raw=layout.encode(index).unwrap();
        assert_eq!(layout.discriminate(raw,&mut Budget::default()).unwrap(),index);
    }
    assert_eq!(layout.encode(2),None);
    assert_eq!(layout.discriminate("unknown",&mut Budget::default()),Err(Error::InvalidValue));
    assert!(layout.to_string().contains("(symbol \"wire_\\\"λ\")"));
    assert_eq!(SymbolRepresentation::new(vec![entries[0].clone(),
        SymbolVariant {name:"other".into(),value:"wire_ready".into()}],&mut Budget::default()),
        Err(Error::SymbolOverlap {first:0,second:1,witness:"wire_ready".into()}));
    for duplicate in [
        SymbolVariant {name:"other".into(),value:"wire_ready".into()},
        SymbolVariant {name:"client.ready".into(),value:"other".into()},
    ] {
        assert!(SymbolRepresentation::new(vec![entries[0].clone(),duplicate],&mut Budget::default()).is_err());
    }
    assert!(SymbolRepresentation::new(entries.clone(),&mut Budget {steps:1,..Budget::default()}).is_err());
    assert!(layout.discriminate("wire_ready",&mut Budget {steps:1,..Budget::default()}).is_err());
    let recovered=SymbolRepresentation::new(entries,&mut Budget::default()).unwrap();
    assert_eq!(recovered.discriminate("wire_ready",&mut Budget::default()).unwrap(),0);
    assert_eq!(SymbolRepresentation::new(vec![],&mut Budget::default()).unwrap()
        .discriminate("anything",&mut Budget::default()),Err(Error::InvalidValue));
}

fn bits(width: usize, value: u32) -> BitVec {
    BitVec::from_words(width, &[value], &mut Budget::default()).unwrap()
}
fn instruction(name: &str, value: u32) -> Variant {
    Variant::new(name.into(), bits(32, 0xfe00707f), bits(32, value),
        [("rs2", 20), ("rs1", 15), ("rd", 7)].into_iter()
            .map(|(name, start)| Field { name: name.into(), start, width: 5 }).collect(),
        &mut Budget::default()).unwrap()
}

#[test]
fn risc_v_instructions_share_construction_discrimination_and_payload_positions() {
    let mut budget = Budget::default();
    let layout = Layout::new(32, vec![instruction("add", 0x33), instruction("sub", 0x40000033)], &mut budget).unwrap();
    for (index, raw) in [(0, 0x003100b3), (1, 0x403100b3)] {
        let payloads = vec![bits(5, 3), bits(5, 2), bits(5, 1)];
        let encoded = layout.variants()[index].construct(&payloads, &mut budget).unwrap();
        assert_eq!(encoded, bits(32, raw));
        assert_eq!(layout.discriminate(&encoded, &mut budget).unwrap(), index);
        assert_eq!(layout.variants()[index].extract(&encoded, &mut budget).unwrap(), payloads);
    }
    let missing = layout.uncovered(&mut budget).unwrap().unwrap();
    assert!(matches!(layout.discriminate(&missing, &mut budget), Err(Error::InvalidValue)));
    assert!(matches!(layout.discriminate(&bits(32, 0), &mut budget), Err(Error::InvalidValue)));
    assert!(matches!(layout.variants()[0].extract(&bits(32, 0x403100b3), &mut budget), Err(Error::InvalidValue)));
    let dump = layout.to_string();
    assert!(dump.starts_with("(enum-encoding (width 32)"));
    assert!(dump.contains("(field (name \"rs2\") (start 20) (width 5))"));
}

#[test]
fn overlap_reports_an_actual_shared_encoding() {
    let first = instruction("add", 0x33);
    let second = instruction("alias", 0x33);
    let mut budget = Budget::default();
    let error = Layout::new(32, vec![first.clone(), second.clone()], &mut budget).unwrap_err();
    let Error::Overlap { first: 0, second: 1, witness } = error else { panic!("{error}"); };
    assert!(first.accepts(&witness, &mut budget).unwrap());
    assert!(second.accepts(&witness, &mut budget).unwrap());
}

#[test]
fn payload_validation_never_truncates_or_invents_bits() {
    let mut budget = Budget::default();
    for fields in [
        vec![Field {name:"x".into(), start:0, width:5}],
        vec![Field {name:"x".into(), start:4, width:5}],
        vec![Field {name:"x".into(), start:4, width:3}],
        vec![Field {name:"x".into(), start:4, width:4},Field {name:"y".into(), start:4, width:4}],
        vec![Field {name:"x".into(), start:4, width:2},Field {name:"x".into(), start:6, width:2}],
        vec![Field {name:"x".into(), start:usize::MAX, width:2}],
    ] {
        assert!(Variant::new("bad".into(), bits(8,15), bits(8,3), fields, &mut budget).is_err());
    }
    assert!(Variant::new("bad".into(), bits(8,15), bits(8,16), vec![], &mut budget).is_err());
    let variant = instruction("add", 0x33);
    assert!(variant.construct(&[], &mut budget).is_err());
    assert!(variant.construct(&[bits(6,32),bits(5,0),bits(5,0)], &mut budget).is_err());
}

#[test]
fn coverage_agrees_with_exhaustive_small_domains() {
    let mut budget = Budget::default();
    // Every subset of eight singleton encodings: coverage and membership are
    // checked against a separate enumeration of the full input domain.
    for selected in 0u32..256 {
        let variants = (0..8).filter(|value| selected & (1 << value) != 0).map(|value|
            Variant::new(value.to_string(), bits(3,7), bits(3,value), vec![], &mut budget).unwrap()).collect();
        let layout = Layout::new(3, variants, &mut budget).unwrap();
        let missing = layout.uncovered(&mut budget).unwrap();
        assert_eq!(missing.is_none(), selected == 255);
        if let Some(missing) = missing { assert_eq!(selected & (1 << missing.words()[0]), 0); }
        for raw in 0..8 {
            assert_eq!(layout.discriminate(&bits(3,raw), &mut budget).is_ok(), selected & (1 << raw) != 0);
        }
    }
}

#[test]
fn masked_payload_domains_match_exhaustive_intersection_and_coverage() {
    // Each bit independently takes one of {fixed zero, fixed one, payload}.
    let mut cubes = Vec::new();
    for mut code in 0..27 {
        let (mut mask, mut value) = (0, 0);
        let mut fields = Vec::new();
        for bit in 0..3 {
            match code % 3 {
                0 => mask |= 1 << bit,
                1 => { mask |= 1 << bit; value |= 1 << bit; },
                _ => fields.push(Field {name:format!("b{bit}"),start:bit,width:1}),
            }
            code /= 3;
        }
        cubes.push((mask, value, fields));
    }
    for (mask_a, value_a, fields_a) in &cubes {
        for (mask_b, value_b, fields_b) in &cubes {
            let mut budget = Budget::default();
            let a = Variant::new("a".into(), bits(3,*mask_a), bits(3,*value_a), fields_a.clone(), &mut budget).unwrap();
            let b = Variant::new("b".into(), bits(3,*mask_b), bits(3,*value_b), fields_b.clone(), &mut budget).unwrap();
            let overlap = (0..8).any(|raw| raw & mask_a == *value_a && raw & mask_b == *value_b);
            match Layout::new(3, vec![a,b], &mut budget) {
                Err(Error::Overlap {witness,..}) => {
                    assert!(overlap);
                    assert_eq!(witness.words()[0] & mask_a, *value_a);
                    assert_eq!(witness.words()[0] & mask_b, *value_b);
                },
                Ok(layout) => {
                    assert!(!overlap);
                    let covered = (0..8).all(|raw| raw & mask_a == *value_a || raw & mask_b == *value_b);
                    let missing = layout.uncovered(&mut budget).unwrap();
                    assert_eq!(missing.is_none(), covered);
                    if let Some(missing) = missing {
                        assert_ne!(missing.words()[0] & mask_a, *value_a);
                        assert_ne!(missing.words()[0] & mask_b, *value_b);
                    }
                },
                Err(error) => panic!("unexpected error: {error}"),
            }
        }
    }
}

#[test]
fn whole_width_payloads_and_non_word_aligned_encodings_round_trip() {
    let mut budget = Budget::default();
    let width = 257;
    let zero = BitVec::from_words(width, &[0;9], &mut budget).unwrap();
    let payload = BitVec::from_words(width, &[u32::MAX,u32::MAX,2,3,4,5,6,7,1], &mut budget).unwrap();
    let variant = Variant::new("x\"\n".into(), zero.clone(), zero,
        vec![Field {name:"payload".into(),start:0,width}], &mut budget).unwrap();
    let layout = Layout::new(width, vec![variant], &mut budget).unwrap();
    assert_eq!(layout.uncovered(&mut budget).unwrap(), None);
    let result = layout.variants()[0].construct(&[payload.clone()], &mut budget).unwrap();
    assert_eq!(layout.variants()[0].extract(&result, &mut budget).unwrap(), vec![payload]);
    assert!(layout.to_string().contains("(name \"x\\\"\\n\")"));
    let empty = Layout::new(width, vec![], &mut budget).unwrap();
    assert!(empty.uncovered(&mut budget).unwrap().is_some());
}

#[test]
fn exhausted_checks_never_publish_coverage_or_overlap_results() {
    let layout = Layout::new(32, vec![instruction("add",0x33)], &mut Budget::default()).unwrap();
    let mut exhausted = Budget {steps:0, max_limbs:100};
    assert!(layout.uncovered(&mut exhausted).is_err());
    assert!(layout.discriminate(&bits(32,0x33), &mut exhausted).is_err());
    assert!(layout.variants()[0].construct(&[bits(5,0),bits(5,0),bits(5,0)], &mut exhausted).is_err());
    assert!(Layout::new(32, vec![], &mut exhausted).is_err());
    assert!(Layout::new(32, vec![instruction("add",0x33)], &mut Budget {steps:100,max_limbs:0}).is_err());
    assert!(layout.uncovered(&mut Budget::default()).unwrap().is_some());
    let constant = Variant::new("constant".into(), bits(32,u32::MAX),bits(32,1),vec![], &mut Budget::default()).unwrap();
    assert!(constant.construct(&[], &mut Budget {steps:100,max_limbs:0}).is_err());
}
