use super::*;
use crate::{
    context::{
        CompilerContext,
        scope::{Item, ScopeId},
    },
    hir::{Definition, Expr, Hir, HirMapping, Module, SDefinition, Struct},
    parse::ast::{self, Ast},
    vfs::{self, NodeIdExt, Vfs},
};

impl<'hir, 'ctx, 'vfs> LoweringContext<'hir, 'ctx, 'vfs> {
    pub fn lower_module_or_file_scope(
        &self,
        ast: &Ast,
        node_index: ast::NodeIndex,
        item: &Item<'hir>,
        owner: &Item<'hir>,
    ) -> LoweringResult<Definition<'hir>> {
        assert!(
            ast.get_node_kind(node_index).expect("Invalid node index") == ast::NodeKind::ModuleDef
                || ast.get_node_kind(node_index).expect("Invalid node index")
                    == ast::NodeKind::FileScope,
            "Expected node to be a ModuleDef",
        );
        assert!(item.scope_id.is_some(), "Item must have a scope ID");

        if let Some(scope) = self.ctx.scope_manager.items(item.scope_id.unwrap()) {
            for child in scope {
                self.lower_unresolved_item(child, item)?;
            }
        }

        Ok(Definition::Module(Module {
            name: item.symbol,
            clauses: self.hir.intern_clauses(vec![]),
            scope_id: owner.scope_id.expect("Invalid owner scope ID"),
        }))
    }

    pub fn lower_struct_def(
        &self,
        ast: &Ast,
        item_index: ast::NodeIndex,
        item: &Item<'hir>,
        owner: &Item<'hir>,
    ) -> LoweringResult<Definition<'hir>> {
        self.assert_kind(ast, item_index, ast::NodeKind::StructDef);
        let children = ast.get_children(item_index);
        let id = ast
            .source_content(children[0], &self.hir.source_map)
            .expect("Failed to get struct id");
        let body = ast
            .get_multi_child_slice(ast.get_children(children[2])[0])
            .expect("Invalid struct body index");

        let mut fields = vec![];
        for &child_item in body {
            let tag = ast
                .get_node_kind(child_item)
                .expect("Invalid struct body item index");
            use ast::NodeKind::*;
            match tag {
                StructField => {
                    let child_children = ast.get_children(child_item);
                    let field_name = ast
                        .source_content(child_children[0], &self.hir.source_map)
                        .expect("Failed to get field name");
                    let field_type_index = child_children[1];
                    let field_type = self.lower_expr(ast, field_type_index, item)?;
                    let field = Definition::StructField(
                        self.hir.intern_str(&field_name),
                        self.hir.intern_expr(field_type),
                        None,
                    );
                    fields.push(field);
                }
                _ => {
                    println!("Skipping unsupported struct body item: {:?}", tag);
                }
            }
        }

        if let Some(scope) = self.ctx.scope_manager.items(item.scope_id.unwrap()) {
            for child in scope {
                self.lower_unresolved_item(child, item)?;
            }
        }

        Ok(Definition::Struct(Struct {
            name: self.hir.intern_str(&id),
            fields: self.hir.intern_definitions(fields),
            clauses: self.hir.intern_clauses(vec![]),
            scope_id: item.scope_id.expect("Struct scope must not be None"),
        }))
    }
}
