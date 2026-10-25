use elaboration::definitions::publish;
use elaboration::program::Protocols;
use hir::definitions::{DefinitionBatch, PublicationError};
use hir::*;
use nir::target::TargetData;
use rustc_span::DUMMY_SP;
use std::collections::BTreeMap;

#[test]
fn semantic_rejection_preserves_draft_then_publishes_checked_function() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let owner = batch.reserve().unwrap();
    let id = |local| HirId::new(owner, ItemLocalId::new(local));
    let invalid_type = arena.alloc_expr(Expr {
        hir_id: id(1),
        kind: ExprKind::Lit(Lit {
            kind: LitKind::Bool(true),
            span: DUMMY_SP,
        }),
        span: DUMMY_SP,
    });
    let valid_type = arena.alloc_expr(Expr {
        hir_id: id(1),
        kind: ExprKind::TyVoid,
        span: DUMMY_SP,
    });
    let block = Block {
        hir_id: id(3),
        stmts: &[],
        expr: None,
        span: DUMMY_SP,
    };
    let value = Expr {
        hir_id: id(2),
        kind: ExprKind::Block(&block),
        span: DUMMY_SP,
    };
    let body_id = BodyId::new(id(2));
    batch
        .body(
            body_id,
            Body {
                params: &[],
                value: &value,
            },
        )
        .unwrap();
    let define = |return_ty| OwnerInfo {
        node: OwnerNode::Item(arena.alloc_item(Item {
            owner_id: owner,
            ident: Ident::new(Symbol::intern("generated"), DUMMY_SP),
            kind: ItemKind::Fn(
                FnSig {
                    params: &[],
                    return_ty: Some(return_ty),
                    return_bind: None,
                    return_binding: None,
                    modifiers: hir::item::FnModifiers::default(),
                    clause_params: &[],
                    clause_constraints: &[],
                    span: DUMMY_SP,
                },
                body_id,
            ),
            span: DUMMY_SP,
        })),
        nodes: OwnerNodes::new(),
    };
    batch.define(define(invalid_type), None, None).unwrap();
    let before = batch.to_string();
    let identities = BTreeMap::from([(0, 73)]);
    let target = Some(TargetData::new(32).unwrap());
    let error = publish(
        &mut batch,
        &mut 100,
        &identities,
        &Protocols::default(),
        target,
    )
    .unwrap_err();
    assert!(matches!(error, PublicationError::Check(_)), "{error}");
    assert_eq!(batch.to_string(), before);
    assert_eq!(batch.published().num_defs(), 0);
    assert_eq!(batch.published().num_bodies(), 0);
    batch.define(define(valid_type), None, None).unwrap();
    let (owners, checked) = publish(
        &mut batch,
        &mut 100,
        &identities,
        &Protocols::default(),
        target,
    )
    .unwrap();
    assert_eq!(owners, vec![owner]);
    assert_eq!(checked.functions.len(), 1);
    assert_eq!(checked.functions[0].declaration.package, 73);
    assert_eq!(checked.target, target);
    assert_eq!(batch.published().num_defs(), 1);
    assert_eq!(batch.published().num_bodies(), 1);
    // A second rejected candidate must preserve the previously checked package.
    let unchanged = batch.published().to_string();
    let incomplete = batch.reserve().unwrap();
    assert_ne!(incomplete, owner);
    assert!(matches!(
        publish(
            &mut batch,
            &mut 100,
            &identities,
            &Protocols::default(),
            target
        ),
        Err(PublicationError::Structure(_))
    ));
    assert_eq!(batch.published().to_string(), unchanged);
    assert_eq!(checked.functions.len(), 1);
}

#[test]
fn generated_definition_attributes_cannot_bypass_semantic_admission() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let owner = batch.reserve().unwrap();
    let value = Expr {
        hir_id: HirId::new(owner, ItemLocalId::new(1)),
        kind: ExprKind::Lit(Lit {
            kind: LitKind::Bool(true),
            span: DUMMY_SP,
        }),
        span: DUMMY_SP,
    };
    batch
        .define(
            OwnerInfo {
                node: OwnerNode::Item(arena.alloc_item(Item {
                    owner_id: owner,
                    ident: Ident::new(Symbol::intern("generated"), DUMMY_SP),
                    kind: ItemKind::Mod(ModDef { items: vec![] }),
                    span: DUMMY_SP,
                })),
                nodes: OwnerNodes::new(),
            },
            None,
            None,
        )
        .unwrap();
    for configuration_key in [None, Some("representation".into())] {
        batch.set_attributes(vec![DeferredDefinitionAttribute {
            target: AttributeTarget::Definition(owner),
            lexical_owner: owner,
            expression: "true".into(),
            value: &value,
            configuration_key,
            span: DUMMY_SP,
        }]);
        let before = batch.to_string();
        let error = publish(
            &mut batch,
            &mut 100,
            &BTreeMap::new(),
            &Protocols::default(),
            None,
        )
        .unwrap_err();
        assert!(matches!(error, PublicationError::Check(_)), "{error}");
        assert!(
            error
                .to_string()
                .contains("elaboration is not implemented for definition")
        );
        assert_eq!(batch.to_string(), before);
        assert_eq!(batch.published().num_defs(), 0);
        assert!(batch.published().definition_attributes.is_empty());
        assert!(batch.published().pending_semantics.is_empty());
    }
    batch.set_attributes(vec![]);
    publish(
        &mut batch,
        &mut 100,
        &BTreeMap::new(),
        &Protocols::default(),
        None,
    )
    .unwrap();
    assert_eq!(batch.published().num_defs(), 1);
}

#[test]
fn inspect_attributes_evaluates_typed_data_without_authorizing_publication() {
    let arena = HirArena::new();
    let mut package = Package::new();
    let mut batch = DefinitionBatch::new(&mut package);
    let owner = batch.reserve().unwrap();
    batch
        .define(
            OwnerInfo {
                node: OwnerNode::Item(arena.alloc_item(Item {
                    owner_id: owner,
                    ident: Ident::new(Symbol::intern("metadata"), DUMMY_SP),
                    kind: ItemKind::Mod(ModDef { items: vec![] }),
                    span: DUMMY_SP,
                })),
                nodes: OwnerNodes::new(),
            },
            None,
            None,
        )
        .unwrap();
    batch.publish(&mut 100).unwrap();
    drop(batch);
    let flag = Expr {
        hir_id: HirId::new(owner, ItemLocalId::new(1)),
        kind: ExprKind::Lit(Lit {
            kind: LitKind::Bool(true),
            span: DUMMY_SP,
        }),
        span: DUMMY_SP,
    };
    let entries = [
        hir::expr::ObjectEntry::Property(hir::expr::FieldExpr {
            ident: Ident::new(Symbol::intern("enabled"), DUMMY_SP),
            expr: &flag,
            span: DUMMY_SP,
        }),
        hir::expr::ObjectEntry::Value(&flag),
    ];
    let object = Expr {
        hir_id: HirId::new(owner, ItemLocalId::new(2)),
        kind: ExprKind::Object(entries.to_vec()),
        span: DUMMY_SP,
    };
    let attribute = |value, key| DeferredDefinitionAttribute {
        target: AttributeTarget::Definition(owner),
        lexical_owner: owner,
        expression: "ignored text: evaluate the tree".into(),
        value,
        configuration_key: key,
        span: DUMMY_SP,
    };
    package.definition_attributes = vec![
        attribute(&flag, Some("quote\"\\\n".into())),
        attribute(&object, None),
    ];
    package
        .pending_semantics
        .push((DUMMY_SP, "definition configuration".into()));
    package
        .pending_semantics
        .push((DUMMY_SP, "definition attribute".into()));
    let inspect = |p: &Package<'_>| {
        elaboration::program::attributes::inspect(p, &BTreeMap::new(), &Protocols::default(), None)
    };
    let result = inspect(&package).unwrap();
    assert_eq!(result.len(), 2);
    assert_eq!(
        result[0].value.ty,
        nir::Term::Primitive(nir::Primitive::Bool)
    );
    assert_eq!(
        result[1].value.ty,
        nir::Term::Primitive(nir::Primitive::Object)
    );
    assert!(
        result[0]
            .to_string()
            .contains("(configuration \"quote\\\"\\\\\\n\")")
    );
    assert!(result[1].to_string().contains("(configuration)"));
    let nir::program::ExprKind::Object { names, fields } = &result[1].value.kind else {
        panic!("prefix did not retain its Object constructor");
    };
    assert_eq!(names, &[Some("enabled".into()), None]);
    assert_eq!(fields.len(), 2);
    assert!(elaboration::program::elaborate(&package).is_err());
    package.definition_attributes[1].value = &flag;
    assert!(inspect(&package).is_err());
    package.definition_attributes[1].value = &object;
    assert_eq!(inspect(&package).unwrap().len(), 2);
    package.definition_attributes[1].target =
        AttributeTarget::Member(HirId::new(owner, ItemLocalId::new(99)));
    assert!(
        inspect(&package)
            .unwrap_err()
            .to_string()
            .contains("missing attribute member")
    );
    package.definition_attributes[1].target = AttributeTarget::Definition(owner);
    package
        .pending_semantics
        .push((DUMMY_SP, "definition attribute or modifier".into()));
    assert!(
        inspect(&package).is_err(),
        "inspection must not suppress unsupported modifiers"
    );
    assert_eq!(package.definition_attributes.len(), 2);
}
