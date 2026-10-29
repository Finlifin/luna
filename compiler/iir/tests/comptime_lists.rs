//! Stable list folding uses trusted collection metadata and complete elements.
use iir::staging::{Layouts, fold_value_with_layouts};
use nir::normal_forms::{Head, Kind, Parameter, Role};
use nir::program::{Expr, ExprKind as K, Literal, LocalId, SourceRange};
use nir::{DeclarationId, Primitive, Sort, Term};

fn id(local: u32) -> DeclarationId {
    DeclarationId { package: 51, local }
}
fn head() -> Head {
    Head {
        collector: None,
        declaration: id(1),
        role: Role::TypeList,
        requirements: vec![],
        parameters: vec![Parameter {
            declaration: id(2),
            binding: LocalId { owner: 1, index: 0 },
            kind: Kind::Type,
            ty: Term::Primitive(Primitive::Type),
        }],
    }
}
fn expr(ty: Term, kind: K) -> Expr {
    Expr {
        ty,
        kind,
        source: SourceRange { start: 2, end: 7 },
    }
}
fn boolean(value: bool) -> Expr {
    expr(
        Term::Primitive(Primitive::Bool),
        K::Literal(Literal::Bool(value)),
    )
}
fn list(element: Term, fields: Vec<Expr>) -> Expr {
    expr(
        Term::NormalForm {
            head: id(1),
            sort: Sort::Type,
            arguments: vec![element],
        },
        K::TypeList(fields),
    )
}
fn index(values: Expr, element: Term, position: &str) -> Expr {
    expr(
        element,
        K::TypeListIndex(
            Box::new(values),
            Box::new(expr(
                Term::Primitive(Primitive::Integer),
                K::Literal(Literal::Integer(position.into())),
            )),
        ),
    )
}

#[test]
fn nested_list_folding_keeps_identity_and_selects_the_exact_element() {
    let heads = [head()];
    let layouts = Layouts::new(&heads, &[]);
    let bool_ty = Term::Primitive(Primitive::Bool);
    let values = list(bool_ty.clone(), vec![boolean(true), boolean(false)]);
    let folded = fold_value_with_layouts(layouts, &values, &mut 100_000).unwrap();
    assert_eq!(folded.ty, values.ty);
    let nested = list(values.ty.clone(), vec![values.clone()]);
    let selected = index(index(nested, values.ty.clone(), "0"), bool_ty.clone(), "1");
    let folded = fold_value_with_layouts(layouts, &selected, &mut 100_000).unwrap();
    assert!(matches!(folded.kind, K::Literal(Literal::Bool(false))));
    assert_eq!(folded.ty, bool_ty);
    assert_eq!(folded.source, selected.source);
    for position in ["-1", "2", "999999999999999999999999999999999999999999"] {
        assert!(
            fold_value_with_layouts(
                layouts,
                &index(values.clone(), bool_ty.clone(), position),
                &mut 100_000
            )
            .is_err()
        );
    }
    assert!(fold_value_with_layouts(layouts, &values, &mut 0).is_err());
    assert!(fold_value_with_layouts(layouts, &values, &mut 100_000).is_ok());
}

#[test]
fn folding_rejects_unbound_heads_and_does_not_skip_unfinished_elements() {
    let bool_ty = Term::Primitive(Primitive::Bool);
    let values = list(bool_ty.clone(), vec![boolean(true)]);
    let mut unbound = head();
    unbound.role = Role::Opaque;
    assert!(fold_value_with_layouts(Layouts::new(&[unbound], &[]), &values, &mut 100_000).is_err());
    let heads = [head()];
    let layouts = Layouts::new(&heads, &[]);
    let unfinished = list(
        bool_ty.clone(),
        vec![
            boolean(true),
            expr(bool_ty.clone(), K::Local(LocalId { owner: 9, index: 0 })),
        ],
    );
    assert!(
        fold_value_with_layouts(
            layouts,
            &index(unfinished, bool_ty.clone(), "0"),
            &mut 100_000
        )
        .is_err()
    );
    let wrong = list(
        bool_ty.clone(),
        vec![expr(
            Term::Primitive(Primitive::Type),
            K::TypeValue(bool_ty.clone()),
        )],
    );
    assert!(fold_value_with_layouts(layouts, &wrong, &mut 100_000).is_err());
    let empty = list(bool_ty.clone(), vec![]);
    assert!(fold_value_with_layouts(layouts, &empty, &mut 100_000).is_ok());
    assert!(fold_value_with_layouts(layouts, &index(empty, bool_ty, "0"), &mut 100_000).is_err());
}

#[test]
fn list_length_and_concat_fold_with_the_original_nominal_identity() {
    let heads = [head()];
    let layouts = Layouts::new(&heads, &[]);
    for element in [Primitive::Bool, Primitive::Type] {
        let field = if element == Primitive::Type {
            expr(
                Term::Primitive(element),
                K::TypeValue(Term::Primitive(Primitive::Bool)),
            )
        } else {
            boolean(true)
        };
        let values = list(Term::Primitive(element), vec![field]);
        let joined = expr(
            values.ty.clone(),
            K::ListConcat(Box::new(values.clone()), Box::new(values.clone())),
        );
        let folded = fold_value_with_layouts(layouts, &joined, &mut 100_000).unwrap();
        assert_eq!(folded.ty, values.ty);
        let length = expr(
            Term::Primitive(Primitive::Integer),
            K::ListLength(Box::new(joined.clone())),
        );
        let folded = fold_value_with_layouts(layouts, &length, &mut 100_000).unwrap();
        assert!(matches!(folded.kind,K::Literal(Literal::Integer(ref n)) if n=="2"));
        assert!(joined.to_string().contains("(list-concat"));
        assert!(length.to_string().contains("(list-length"));
        assert!(fold_value_with_layouts(layouts, &joined, &mut 1).is_err());
        let mut bad = length;
        bad.ty = Term::Primitive(Primitive::Bool);
        assert!(fold_value_with_layouts(layouts, &bad, &mut 100_000).is_err());
    }
}
