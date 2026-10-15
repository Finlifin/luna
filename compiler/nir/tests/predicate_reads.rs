use nir::{
    Primitive as P, Term,
    program::{Expr, ExprKind as K, Literal, LocalId, SourceRange, checkable_predicate},
};

fn expr(ty: Term, kind: K) -> Expr {
    Expr {
        source: SourceRange { start: 0, end: 1 },
        ty,
        kind,
    }
}
fn boolean() -> Term {
    Term::Primitive(P::Bool)
}
fn pointer() -> Expr {
    expr(
        Term::Pointer(Box::new(boolean())),
        K::Local(LocalId { owner: 0, index: 0 }),
    )
}

#[test]
fn predicate_shape_admits_typed_reads_without_proving_pointer_validity() {
    let read = expr(boolean(), K::ReadPointer(Box::new(pointer())));
    assert!(checkable_predicate(&read));
    let local = expr(boolean(), K::Local(LocalId { owner: 0, index: 1 }));
    let address = expr(
        Term::Pointer(Box::new(boolean())),
        K::AddressOf(Box::new(local)),
    );
    assert!(checkable_predicate(&expr(
        boolean(),
        K::ReadPointer(Box::new(address))
    )));
    let mut malformed = read;
    let K::ReadPointer(pointer) = &mut malformed.kind else {
        unreachable!()
    };
    pointer.ty = Term::Pointer(Box::new(Term::Primitive(P::U8)));
    assert!(!checkable_predicate(&malformed));
}

#[test]
fn predicate_shape_rejects_writes_and_owned_pointer_delivery() {
    let yes = expr(boolean(), K::Literal(Literal::Bool(true)));
    let write = expr(
        Term::Primitive(P::Void),
        K::WritePointer {
            pointer: Box::new(pointer()),
            value: Box::new(yes.clone()),
        },
    );
    assert!(!checkable_predicate(&expr(
        boolean(),
        K::Block {
            statements: vec![write],
            result: Some(Box::new(yes))
        }
    )));
    let owned = Term::Record(vec![("flag".into(), boolean())]);
    let pointer = expr(
        Term::Pointer(Box::new(owned.clone())),
        K::Local(LocalId { owner: 0, index: 0 }),
    );
    let value = expr(owned, K::ReadPointer(Box::new(pointer)));
    assert!(!checkable_predicate(&expr(
        boolean(),
        K::AnonymousField {
            value: Box::new(value),
            name: "flag".into(),
            index: 0
        }
    )));
}

#[test]
fn predicate_substitution_rebinds_read_addresses_and_field_pointers() {
    let original = LocalId { owner: 0, index: 0 };
    let target = LocalId { owner: 9, index: 3 };
    let bindings = std::collections::BTreeMap::from([(original, target)]);
    let record = Term::Record(vec![("flag".into(), boolean())]);
    let record_pointer = expr(Term::Pointer(Box::new(record)), K::Local(original));
    let field = expr(
        Term::Pointer(Box::new(boolean())),
        K::PointerAnonymousField {
            pointer: Box::new(record_pointer),
            name: "flag".into(),
            index: 0,
        },
    );
    let address = expr(
        Term::Pointer(Box::new(boolean())),
        K::AddressOf(Box::new(expr(boolean(), K::Local(original)))),
    );
    for pointer in [pointer(), address, field] {
        let mut predicate = expr(boolean(), K::ReadPointer(Box::new(pointer)));
        assert!(checkable_predicate(&predicate));
        nir::contracts::remap_predicate(&mut predicate, &bindings).unwrap();
        let mut locals = Vec::new();
        nir::visit::expression(&predicate, &mut 100, |expr, _| {
            if let K::Local(id) = expr.kind {
                locals.push(id);
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(locals, vec![target]);
        assert!(checkable_predicate(&predicate));
    }
    let mut write = expr(
        Term::Primitive(P::Void),
        K::WritePointer {
            pointer: Box::new(pointer()),
            value: Box::new(expr(boolean(), K::Literal(Literal::Bool(true)))),
        },
    );
    assert!(nir::contracts::remap_predicate(&mut write, &bindings).is_err());
}
