use nir::{
    Primitive as P, Term,
    program::{Expr, ExprKind as K, Literal, SourceRange},
};

fn pending(ty: Term) -> Expr {
    Expr {
        source: SourceRange { start: 10, end: 14 },
        ty,
        kind: K::Literal(Literal::PendingFloat {
            spelling: "0.0".into(),
            negative: true,
        }),
    }
}

#[test]
fn folding_forms_float_before_canonical_input_admission() {
    for (domain, expected) in [
        (P::F16, Literal::Float16(1 << 15)),
        (P::Bf16, Literal::BFloat16(1 << 15)),
        (P::F32, Literal::Float32(1 << 31)),
        (P::F64, Literal::Float64(1 << 63)),
    ] {
        let source = pending(Term::Primitive(domain));
        let layouts = iir::staging::Layouts::new(&[], &[]);
        assert!(iir::staging::canonical_input_with_layouts(layouts, &source, &mut 10000).is_err());
        let formed = iir::staging::fold_value(&source, &mut 10000).unwrap();
        assert_eq!(formed.kind, K::Literal(expected));
        assert_eq!(formed.source, source.source);
        iir::staging::canonical_input_with_layouts(layouts, &formed, &mut 10000).unwrap();
        assert!(matches!(
            source.kind,
            K::Literal(Literal::PendingFloat { .. })
        ));
    }
}

#[test]
fn folding_rejects_unresolved_or_invalid_domains_and_recovers_after_budget_failure() {
    for ty in [
        Term::Primitive(P::Bool),
        Term::Rigid {
            declaration: nir::DeclarationId {
                package: 0,
                local: 1,
            },
            sort: nir::Sort::Type,
        },
    ] {
        assert!(iir::staging::fold_value(&pending(ty), &mut 10000).is_err());
    }
    let source = pending(Term::Primitive(P::F64));
    assert!(iir::staging::fold_value(&source, &mut 0).is_err());
    assert!(iir::staging::fold_value(&source, &mut 10000).is_ok());
}

#[test]
fn staged_overflow_has_the_same_source_aware_diagnostic_as_normal_completion() {
    let mut source = pending(Term::Primitive(P::F32));
    source.kind = K::Literal(Literal::PendingFloat {
        spelling: "1.0e40".into(),
        negative: false,
    });
    let staged = iir::staging::fold_value(&source, &mut 10000).unwrap_err();
    let normal = nir::float_formation::complete(&mut source.clone(), &mut 10000).unwrap_err();
    assert_eq!(staged, normal);
    assert!(staged.contains("finite float literal exceeds"), "{staged}");
    source.ty = Term::Primitive(P::F64);
    assert!(iir::staging::fold_value(&source, &mut 10000).is_ok());
}

#[test]
fn half_comparison_folding_preserves_zero_and_unordered_nan() {
    for (ty, literal, nan) in [
        (P::F16, Literal::Float16 as fn(u16) -> Literal, 0x7e01),
        (P::Bf16, Literal::BFloat16 as fn(u16) -> Literal, 0x7fc1),
    ] {
        for (left, right, equal) in [(0, 0x8000, true), (nan, nan, false), (nan, 0, false)] {
            for (kind, expected) in [
                (K::ScalarEqual as fn(Box<Expr>, Box<Expr>) -> K, equal),
                (K::ScalarLessEqual as fn(Box<Expr>, Box<Expr>) -> K, equal),
                (
                    K::ScalarGreaterEqual as fn(Box<Expr>, Box<Expr>) -> K,
                    equal,
                ),
            ] {
                let value = |bits| {
                    Box::new(Expr {
                        source: SourceRange { start: 1, end: 2 },
                        ty: Term::Primitive(ty),
                        kind: K::Literal(literal(bits)),
                    })
                };
                let expression = Expr {
                    source: SourceRange { start: 1, end: 2 },
                    ty: Term::Primitive(P::Bool),
                    kind: kind(value(left), value(right)),
                };
                assert_eq!(
                    iir::staging::fold_value(&expression, &mut 10000)
                        .unwrap()
                        .kind,
                    K::Literal(Literal::Bool(expected))
                );
            }
        }
    }
}
