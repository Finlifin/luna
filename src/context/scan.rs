use super::scope::ScopeId;
use crate::{
    context::{CompilerContext, scope::Item},
    hir::*,
    parse::ast::{Ast, NodeIndex, NodeKind},
    vfs,
};

// 扫描ast中的作用域和符号
pub fn scan<'hir>(
    ctx: &mut CompilerContext<'hir>,
    hir: &'hir Hir,
    file_id: vfs::NodeId,
    ast: &Ast,
    parent_scope: ScopeId,
    owner_hir_id: HirId,
) {
    let file_scope_items_index = ast.get_children(ast.root)[0];
    let items = ast
        .get_multi_child_slice(file_scope_items_index)
        .expect("Invalid items slice");
    scan_inner(ctx, &hir, file_id, ast, parent_scope, items, owner_hir_id);
}

fn scan_inner<'hir>(
    ctx: &mut CompilerContext<'hir>,
    hir: &'hir Hir,
    file_id: vfs::NodeId,
    ast: &Ast,
    parent_scope: ScopeId,
    items: &[NodeIndex],
    owner_hir_id: HirId,
) {
    for item in items {
        let item_kind = ast.get_node_kind(*item).expect("Invalid node index");

        use NodeKind::*;
        match item_kind {
            ModuleDef => {
                let id = ast.get_children(*item)[0];
                let name = hir
                    .str_arena
                    .intern_string(ast.source_content(id, &hir.source_map).unwrap());
                let hir_id = hir.put(HirMapping::Unresolved(file_id, *item, owner_hir_id));
                let scope = ctx
                    .scope_manager
                    .add_scope(Some(name), Some(parent_scope), false, hir_id)
                    .unwrap();

                let block_index = ast.get_children(*item)[2];
                let block_items_index = ast.get_children(block_index)[0];
                let block_items = ast
                    .get_multi_child_slice(block_items_index)
                    .expect("Invalid block items slice");
                scan_inner(ctx, hir, file_id, ast, scope, block_items, hir_id);
            }

            StructDef => {
                let id = ast.get_children(*item)[0];
                let name = hir
                    .str_arena
                    .intern_string(ast.source_content(id, &hir.source_map).unwrap());
                let hir_id = hir.put(HirMapping::Unresolved(file_id, *item, owner_hir_id));
                let scope = ctx
                    .scope_manager
                    .add_scope(Some(name), Some(parent_scope), false, hir_id)
                    .unwrap();

                let body_index = ast.get_children(*item)[2];
                let body_items_index = ast.get_children(body_index)[0];
                let body_items = ast
                    .get_multi_child_slice(body_items_index)
                    .expect("Invalid body items slice");
                scan_inner(ctx, hir, file_id, ast, scope, body_items, hir_id);
            }

            EnumDef => {
                let id = ast.get_children(*item)[0];
                let name = hir
                    .str_arena
                    .intern_string(ast.source_content(id, &hir.source_map).unwrap());
                let hir_id = hir.put(HirMapping::Unresolved(file_id, *item, owner_hir_id));
                let scope = ctx
                    .scope_manager
                    .add_scope(Some(name), Some(parent_scope), false, hir_id)
                    .unwrap();

                let body_index = ast.get_children(*item)[2];
                let body_items_index = ast.get_children(body_index)[0];
                let body_items = ast
                    .get_multi_child_slice(body_items_index)
                    .expect("Invalid body items slice");
                scan_inner(ctx, hir, file_id, ast, scope, body_items, hir_id);
            }

            UnionDef => {
                let id = ast.get_children(*item)[0];
                let name = hir
                    .str_arena
                    .intern_string(ast.source_content(id, &hir.source_map).unwrap());
                let hir_id = hir.put(HirMapping::Unresolved(file_id, *item, owner_hir_id));
                let scope = ctx
                    .scope_manager
                    .add_scope(Some(name), Some(parent_scope), false, hir_id)
                    .unwrap();

                let body_index = ast.get_children(*item)[2];
                let body_items_index = ast.get_children(body_index)[0];
                let body_items = ast
                    .get_multi_child_slice(body_items_index)
                    .expect("Invalid body items slice");
                scan_inner(ctx, hir, file_id, ast, scope, body_items, hir_id);
            }

            FunctionDef => {
                let id = ast.get_children(*item)[0];
                let name = hir
                    .str_arena
                    .intern_string(ast.source_content(id, &hir.source_map).unwrap());
                let hir_id = hir.put(HirMapping::Unresolved(file_id, *item, owner_hir_id));
                let item = Item::new(name, hir_id, None); // 函数是纯符号，没有子scope
                ctx.scope_manager.add_item(item, parent_scope).unwrap();
            }

            _ => {}
        }
    }
}
