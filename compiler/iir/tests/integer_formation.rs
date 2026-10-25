use nir::{
    Primitive as P, Term,
    program::{Expr, ExprKind as K, Literal, SourceRange},
    target::TargetData,
};

fn pending(domain: P) -> Expr {
    Expr {
        source: SourceRange { start: 10, end: 20 },
        ty: Term::Primitive(domain),
        kind: K::Literal(Literal::PendingInteger {
            spelling: "4294967296".into(),
            negative: false,
        }),
    }
}

#[test]
fn staged_formation_uses_explicit_target_and_preserves_canonical_boundary() {
    let source = pending(P::Usize);
    for width in [32, 64] {
        let target = Some(TargetData::new(width).unwrap());
        let layouts = iir::staging::Layouts::new(&[], &[]).with_target(target);
        assert!(iir::staging::canonical_input_with_layouts(layouts, &source, &mut 10000).is_err());
        let folded = iir::staging::fold_value_with_layouts(layouts, &source, &mut 10000);
        if width == 32 {
            let staged = folded.unwrap_err();
            let normal = nir::integer_formation::complete(&mut source.clone(), target, &mut 10000)
                .unwrap_err();
            assert_eq!(staged, normal);
            assert!(
                staged.contains("(source 10 20)") && staged.contains("(pointer-bits 32)"),
                "{staged}"
            );
        } else {
            let formed = folded.unwrap();
            assert_eq!(
                formed.kind,
                K::Literal(Literal::UnsignedInteger(4294967296))
            );
            assert_eq!(formed.source, source.source);
            iir::staging::canonical_input_with_layouts(layouts, &formed, &mut 10000).unwrap();
        }
    }
    assert!(matches!(
        source.kind,
        K::Literal(Literal::PendingInteger { .. })
    ));
}

#[test]
fn missing_target_is_distinct_from_overflow_and_fixed_width_needs_no_target() {
    let source = pending(P::Usize);
    let error = iir::staging::fold_value(&source, &mut 10000).unwrap_err();
    assert_eq!(
        error,
        nir::integer_formation::complete(&mut source.clone(), None, &mut 10000).unwrap_err()
    );
    assert!(
        error.contains("requires explicit target pointer-width"),
        "{error}"
    );
    let source = pending(P::U64);
    assert!(iir::staging::fold_value(&source, &mut 0).is_err());
    assert_eq!(
        iir::staging::fold_value(&source, &mut 10000).unwrap().kind,
        K::Literal(Literal::UnsignedInteger(4294967296))
    );
}
