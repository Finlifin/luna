use nir::program::{Expr, ExprKind as K, Literal, SourceRange};
use nir::{Primitive as P, Term};
fn object() -> Expr {
    Expr {
        ty: Term::Primitive(P::Object),
        source: SourceRange { start: 0, end: 1 },
        kind: K::Object {
            names: vec![None, Some("quoted\"\\\n".into())],
            fields: vec![
                Expr {
                    ty: Term::Primitive(P::Bool),
                    source: SourceRange { start: 1, end: 2 },
                    kind: K::Literal(Literal::Bool(true)),
                },
                Expr {
                    ty: Term::Primitive(P::Object),
                    source: SourceRange { start: 2, end: 3 },
                    kind: K::Object {
                        names: vec![],
                        fields: vec![],
                    },
                },
            ],
        },
    }
}
#[test]
fn object_children_participate_in_visitation_and_type_rewriting() {
    let mut value = object();
    let mut seen = vec![];
    nir::visit::expression(&value, &mut 100, |expr, _| {
        seen.push(expr.source.start);
        Ok(())
    })
    .unwrap();
    assert_eq!(seen.len(), 3);
    assert!(seen.contains(&1) && seen.contains(&2));
    value
        .rewrite_types(&mut |ty| {
            Ok::<_, ()>(if *ty == Term::Primitive(P::Bool) {
                Term::Primitive(P::Char)
            } else {
                ty.clone()
            })
        })
        .unwrap();
    let K::Object { names, fields } = &value.kind else {
        panic!()
    };
    assert_eq!(names, &vec![None, Some("quoted\"\\\n".into())]);
    assert_eq!(fields[0].ty, Term::Primitive(P::Char));
    assert!(
        value
            .to_string()
            .contains("(property \"quoted\\\"\\\\\\n\")")
    );
    assert!(value.to_string().contains("(positional)"));
}
#[test]
fn object_copy_budget_includes_property_names_and_recovers() {
    let mut value = object();
    let K::Object { names, .. } = &mut value.kind else {
        panic!()
    };
    names[1] = Some("x".repeat(4096));
    assert!(nir::copy_budget::charge_expression(&value, &mut 100).is_err());
    let mut enough = 10000;
    nir::copy_budget::charge_expression(&value, &mut enough).unwrap();
    assert!(enough < 5904);
}

#[test]
fn frozen_objects_have_typed_ordered_identity_and_escaped_display() {
    use nir::{DeclarationId, Sort};
    let parameter = DeclarationId {
        package: 7,
        local: 1,
    };
    let domain = Term::Rigid {
        declaration: parameter,
        sort: Sort::Type,
    };
    let frozen = nir::objects::value(vec![
        nir::objects::entry(Some("key\"\\\n"), Term::Primitive(P::Type), domain),
        nir::objects::entry(None, Term::Primitive(P::Bool), Term::Bool(true)),
    ]);
    let replaced = frozen
        .instantiate_rigids(&[parameter], &[Term::Primitive(P::Char)])
        .unwrap();
    assert!(replaced.check_scope(&[]).is_ok());
    let dump = replaced.to_string();
    assert!(dump.starts_with("(object-value"), "{dump}");
    assert!(dump.contains("key\\\"\\\\\\n"), "{dump}");
    assert!(dump.contains("(key none)"), "{dump}");
    let mut low = 2;
    assert!(nir::type_computations::charge_term(&replaced, &mut low).is_err());
    let positional = nir::objects::value(vec![nir::objects::entry(
        None,
        Term::Primitive(P::Bool),
        Term::Bool(true),
    )]);
    let empty_key = nir::objects::value(vec![nir::objects::entry(
        Some(""),
        Term::Primitive(P::Bool),
        Term::Bool(true),
    )]);
    assert_ne!(positional, empty_key);
}
