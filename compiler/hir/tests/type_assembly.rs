use hir::{
    definitions::{DefinitionNodes, Error, TypeDeclaration, assemble_newtype, assemble_type_alias},
    *,
};
use rustc_span::DUMMY_SP;

#[test]
fn type_declaration_assembly_requires_budget_and_complete_nodes() {
    let arena = HirArena::new();
    check(&arena);
}

fn check<'a>(arena: &'a HirArena) {
    let owner = OwnerId::new(LocalDefId::new(0));
    let name = Ident::new(Symbol::intern("Generated"), DUMMY_SP);
    for nominal in [false, true] {
        let mut called = false;
        let make =
            |_: &mut DefinitionNodes<'a>, _: &mut usize| -> Result<TypeDeclaration<'a>, Error> {
                called = true;
                unreachable!()
            };
        let result = if nominal {
            assemble_newtype(&arena, owner, name, DUMMY_SP, &mut 0, make)
        } else {
            assemble_type_alias(&arena, owner, name, DUMMY_SP, &mut 0, make)
        };
        assert_eq!(result.unwrap_err().kind, "budget exhausted");
        assert!(!called);
        for complete in [false, true] {
            let make = |nodes: &mut DefinitionNodes<'a>,
                        fuel: &mut usize|
             -> Result<TypeDeclaration<'a>, Error> {
                let root = nodes.root();
                let value = arena.alloc_expr(Expr {
                    hir_id: nodes.reserve(root, fuel)?,
                    kind: ExprKind::Builtin(Symbol::intern("bool")),
                    span: DUMMY_SP,
                });
                if complete {
                    nodes.register(Node::Expr(value), fuel)?;
                }
                Ok((value, &[][..], &[][..]))
            };
            let result = if nominal {
                assemble_newtype(&arena, owner, name, DUMMY_SP, &mut 100, make)
            } else {
                assemble_type_alias(&arena, owner, name, DUMMY_SP, &mut 100, make)
            };
            if complete {
                let info = result.unwrap();
                assert_eq!(
                    matches!(info.node.expect_item().kind, ItemKind::Newtype(..)),
                    nominal
                );
                let dump = info.node.expect_item().to_string();
                assert!(dump.starts_with('('));
                assert!(dump.contains("Generated"));
            } else {
                assert!(result.is_err());
            }
        }
    }
}
