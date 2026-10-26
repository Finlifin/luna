use nir::{
    Primitive, Term,
    program::{Expr, ExprKind as K, Literal, SourceRange},
};
fn expr(ty: Term, kind: K) -> Expr {
    Expr {
        ty,
        kind,
        source: SourceRange { start: 1, end: 2 },
    }
}
#[test]
fn staging_folds_optional_payload_and_retains_absence_layers() {
    let boolean = Term::Primitive(Primitive::Bool);
    let optional = Term::Optional(Box::new(boolean.clone()));
    let value = expr(
        optional.clone(),
        K::OptionalPresent(Box::new(expr(
            boolean.clone(),
            K::Not(Box::new(expr(boolean, K::Literal(Literal::Bool(true))))),
        ))),
    );
    let folded = iir::staging::fold_value(&value, &mut 1000).unwrap();
    let K::OptionalPresent(inner) = &folded.kind else {
        panic!()
    };
    assert_eq!(inner.kind, K::Literal(Literal::Bool(false)));
    let layouts = iir::staging::Layouts::new(&[], &[]);
    iir::staging::canonical_input_with_layouts(layouts, &folded, &mut 1000).unwrap();
    let absent = expr(optional.clone(), K::OptionalAbsent);
    let nested = expr(
        Term::Optional(Box::new(optional.clone())),
        K::OptionalPresent(Box::new(absent)),
    );
    assert_eq!(
        iir::staging::fold_value(&nested, &mut 1000).unwrap(),
        nested
    );
    let wrong = Expr {
        ty: optional,
        ..nested
    };
    assert!(iir::staging::canonical_input_with_layouts(layouts, &wrong, &mut 1000).is_err());
    assert!(iir::staging::fold_value(&value, &mut 0).is_err());
    assert!(iir::staging::fold_value(&value, &mut 1000).is_ok());
}

#[test]
fn staged_optional_match_binds_one_layer_and_recovers_after_a_guard_miss() {
    use nir::program::{LocalId, MatchArm, Pattern};
    let boolean = Term::Primitive(Primitive::Bool);
    let optional = Term::Optional(Box::new(boolean.clone()));
    let local = LocalId { owner: 1, index: 0 };
    let bind = Pattern::OptionalPresent(Box::new(Pattern::Binding {
        binding: local,
        inner: Box::new(Pattern::Wild),
    }));
    let input = expr(
        optional,
        K::OptionalPresent(Box::new(expr(
            boolean.clone(),
            K::Literal(Literal::Bool(true)),
        ))),
    );
    let matched = expr(
        boolean.clone(),
        K::Match {
            value: Box::new(input),
            arms: vec![
                MatchArm {
                    pattern: bind.clone(),
                    guard: Some(expr(boolean.clone(), K::Literal(Literal::Bool(false)))),
                    body: expr(boolean.clone(), K::Literal(Literal::Bool(false))),
                },
                MatchArm {
                    pattern: bind,
                    guard: None,
                    body: expr(boolean, K::Local(local)),
                },
            ],
        },
    );
    let folded = iir::staging::fold_value(&matched, &mut 10000).unwrap();
    assert_eq!(folded.kind, K::Literal(Literal::Bool(true)));
}
