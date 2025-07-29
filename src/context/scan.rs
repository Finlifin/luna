use internment::Arena;
use rustc_span::{SourceMap, source_map};

use super::scope::{ScopeId, ScopeManager, Symbol};
use crate::{
    context::{
        CompilerContext,
        scope::{self, Item},
    },
    hir::*,
    parse::ast::{Ast, NodeIndex, NodeKind},
};

// 扫描ast中的作用域和符号
pub fn scan<'hir>(
    ctx: &mut CompilerContext<'hir>,
    hir: &'hir Hir,
    ast: &Ast,
    parent_scope: ScopeId,
) {
    let file_scope_items_index = ast.get_children(ast.root)[0];
    let items = ast
        .get_multi_child_slice(file_scope_items_index)
        .expect("Invalid items slice");
    scan_inner(ctx, &hir, ast, parent_scope, items);
}

fn scan_inner<'hir>(
    ctx: &mut CompilerContext<'hir>,
    hir: &'hir Hir,
    ast: &Ast,
    parent_scope: ScopeId,
    items: &[NodeIndex],
) {
    for item in items {
        let item_kind = ast.get_node_kind(*item).expect("Invalid node index");
        println!("Scanning item: {:?}", item_kind);

        use NodeKind::*;
        match item_kind {
            ModuleDef => {
                let id = ast.get_children(*item)[0];
                let name = hir
                    .str_arena
                    .intern_string(ast.source_content(id, &hir.source_map).unwrap());
                let scope = ctx
                    .scope_manager
                    .add_scope(Some(name), Some(parent_scope), false)
                    .unwrap();

                let block_index = ast.get_children(*item)[2];
                let block_items_index = ast.get_children(block_index)[0];
                let block_items = ast
                    .get_multi_child_slice(block_items_index)
                    .expect("Invalid block items slice");
                scan_inner(ctx, hir, ast, scope, block_items);
            }

            StructDef => {
                let id = ast.get_children(*item)[0];
                let name = hir
                    .str_arena
                    .intern_string(ast.source_content(id, &hir.source_map).unwrap());
                let scope = ctx
                    .scope_manager
                    .add_scope(Some(name), Some(parent_scope), false)
                    .unwrap();

                let body_index = ast.get_children(*item)[2];
                let body_items_index = ast.get_children(body_index)[0];
                let body_items = ast
                    .get_multi_child_slice(body_items_index)
                    .expect("Invalid body items slice");
                scan_inner(ctx, hir, ast, scope, body_items);
            }

            EnumDef => {
                let id = ast.get_children(*item)[0];
                let name = hir
                    .str_arena
                    .intern_string(ast.source_content(id, &hir.source_map).unwrap());
                let scope = ctx
                    .scope_manager
                    .add_scope(Some(name), Some(parent_scope), false)
                    .unwrap();

                let body_index = ast.get_children(*item)[2];
                let body_items_index = ast.get_children(body_index)[0];
                let body_items = ast
                    .get_multi_child_slice(body_items_index)
                    .expect("Invalid body items slice");
                scan_inner(ctx, hir, ast, scope, body_items);
            }

            UnionDef => {
                let id = ast.get_children(*item)[0];
                let name = hir
                    .str_arena
                    .intern_string(ast.source_content(id, &hir.source_map).unwrap());
                let scope = ctx
                    .scope_manager
                    .add_scope(Some(name), Some(parent_scope), false)
                    .unwrap();

                let body_index = ast.get_children(*item)[2];
                let body_items_index = ast.get_children(body_index)[0];
                let body_items = ast
                    .get_multi_child_slice(body_items_index)
                    .expect("Invalid body items slice");
                scan_inner(ctx, hir, ast, scope, body_items);
            }

            FunctionDef => {
                let id = ast.get_children(*item)[0];
                let name = hir
                    .str_arena
                    .intern_string(ast.source_content(id, &hir.source_map).unwrap());
                let item = Item::new(name, 0, Some(parent_scope));
                ctx.scope_manager.add_item(item, parent_scope).unwrap();
            }

            _ => {}
        }
    }
}
