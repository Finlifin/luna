use hir::definitions::DefinitionBatch;
use hir::*;
use rustc_span::DUMMY_SP;
fn field<'a>(arena: &'a HirArena, owner: OwnerId, local: u32, name: &str) -> FieldDef<'a> {
    FieldDef {
        hir_id: HirId::new(owner, ItemLocalId::new(local)),
        ident: Ident::new(Symbol::intern(name), DUMMY_SP),
        ty: arena.alloc_expr(Expr {
            hir_id: HirId::new(owner, ItemLocalId::new(10 + local)),
            kind: ExprKind::TyVoid,
            span: DUMMY_SP,
        }),
        default: None,
        span: DUMMY_SP,
    }
}
fn definition<'a>(arena: &'a HirArena, owner: OwnerId, kind: ItemKind<'a>) -> OwnerInfo<'a> {
    OwnerInfo {
        node: OwnerNode::Item(arena.alloc_item(Item {
            owner_id: owner,
            ident: Ident::new(Symbol::intern("Generated"), DUMMY_SP),
            kind,
            span: DUMMY_SP,
        })),
        nodes: OwnerNodes::new(),
    }
}
#[test]
fn generated_fields_reject_foreign_root_and_duplicate_identities_without_publication() {
    for case in 0..3 {
        let arena = HirArena::new();
        let mut package = Package::new();
        let mut batch = DefinitionBatch::new(&mut package);
        let owner = batch.reserve().unwrap();
        let foreign = OwnerId::new(LocalDefId::new(900));
        let fields = arena.alloc_field_def_slice(match case {
            0 => vec![field(&arena, foreign, 1, "a")],
            1 => vec![field(&arena, owner, 0, "a")],
            _ => vec![field(&arena, owner, 1, "a"), field(&arena, owner, 1, "b")],
        });
        let kind = |fields| {
            ItemKind::Struct(StructDef {
                fields,
                clause_params: &[],
                clause_constraints: &[],
                nested_items: vec![],
            })
        };
        batch
            .define(definition(&arena, owner, kind(fields)), None, None)
            .unwrap();
        let before = batch.to_string();
        assert!(
            batch.publish(&mut 100).is_err(),
            "invalid field case {case} published"
        );
        assert_eq!(batch.to_string(), before);
        assert_eq!(batch.published().num_defs(), 0);
        let repaired = arena.alloc_field_def_slice(vec![
            field(&arena, owner, 1, "a"),
            field(&arena, owner, 2, "b"),
        ]);
        batch
            .define(definition(&arena, owner, kind(repaired)), None, None)
            .unwrap();
        let repaired_snapshot = batch.to_string();
        assert_eq!(batch.publish(&mut 2).unwrap_err().kind, "budget exhausted");
        assert_eq!(batch.to_string(), repaired_snapshot);
        assert_eq!(batch.published().num_defs(), 0);
        batch.publish(&mut 100).unwrap();
        assert_eq!(batch.published().num_defs(), 1);
    }
}
