use super::*;
use crate::{
    context::scope::Item,
    hir::{Definition, FnKind, Function, HirId, HirMapping, Param},
};
use ast::Ast;

use crate::{hir_clauses, hir_module, hir_struct, hir_struct_field}; // Explicit macro imports

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
                self.lower_unresolved_item(&child, item)?;
            }
        }

        // Use HIR module macro instead of manual construction
        Ok(hir_module!(
            self.hir,
            &*item.symbol,
            item.scope_id.expect("Invalid item scope ID")
        ))
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

                    // Use HIR struct field macro instead of manual construction
                    let field = hir_struct_field!(self.hir, &field_name, field_type);
                    fields.push(field);
                }
                _ => {
                    println!("Skipping unsupported struct body item: {:?}", tag);
                }
            }
        }

        if let Some(scope) = self.ctx.scope_manager.items(item.scope_id.unwrap()) {
            for child in scope {
                self.lower_unresolved_item(&child, item)?;
            }
        }

        // Use HIR struct macro instead of manual construction
        Ok(hir_struct!(
            self.hir,
            &id,
            self.hir.intern_definitions(fields),
            item.scope_id.expect("Struct scope must not be None"),
            clauses: hir_clauses!(self.hir)  // Empty clauses using macro
        ))
    }

    pub fn lower_function_def(
        &self,
        ast: &Ast,
        item_index: ast::NodeIndex,
        item: &Item<'hir>,
        owner: &Item<'hir>,
    ) -> LoweringResult<Definition<'hir>> {
        self.assert_kind(ast, item_index, ast::NodeKind::FunctionDef);

        let clause_ids: Vec<_> = self.get_item_clauses(item);

        let params_index = ast.get_children(item_index)[1];
        let params = ast
            .get_multi_child_slice(params_index)
            .ok_or_else(|| LowerError::InternalError("Invalid params slice".into()))?;
        let param_ids: Vec<_> = params
            .iter()
            .map(|&param| self.lower_param(ast, param, item))
            .collect::<LoweringResult<Vec<_>>>()?;

        let return_type = self.lower_expr(ast, ast.get_children(item_index)[2], item)?;
        let body = self.lower_expr(ast, ast.get_children(item_index)[5], item)?;

        Ok(Definition::Function(Function {
            kind: FnKind::Normal,
            name: item.symbol,
            params: self.hir.intern_ids(param_ids),
            return_type: self.hir.intern_expr(return_type),
            body: self.hir.intern_expr(body),
            body_scope: item.scope_id.expect("Function scope must not be None"),
            clauses: self.hir.intern_ids(clause_ids),
        }))
    }

    pub fn lower_param(
        &self,
        ast: &Ast,
        param_index: ast::NodeIndex,
        owner: &Item<'hir>,
    ) -> LoweringResult<HirId> {
        let Some((kind, span, children)) = ast.get_node(param_index) else {
            return Err(LowerError::InternalError("Invalid param index".into()));
        };
        let fn_body_scope = owner.scope_id.expect("Owner fn must have a body scope ID");

        use ast::NodeKind::*;
        let hir_id = match kind {
            // id : type_expr
            ParamTyped => {
                let param_name = ast
                    .source_content(children[0], &self.hir.source_map)
                    .expect("Failed to get param name");
                let interned_name = self.hir.intern_str(&param_name);
                let param_type = self.lower_expr(ast, children[1], owner)?;
                let param = Param::Typed(interned_name, self.hir.intern_expr(param_type), None);
                let hir_id = self.hir.put(HirMapping::Param(
                    self.hir.intern_param(param),
                    owner.hir_id,
                ));
                self.ctx
                    .scope_manager
                    .add_item(Item::new(interned_name, hir_id, None), fn_body_scope);
                hir_id
            }
            // .id : type_expr = default_expr
            ParamOptional => {
                let param_name = ast
                    .source_content(children[0], &self.hir.source_map)
                    .expect("Failed to get param name");
                let interned_name = self.hir.intern_str(&param_name);
                let param_type = self.lower_expr(ast, children[1], owner)?;
                let default_expr = self.lower_expr(ast, children[2], owner)?;
                let param = Param::Typed(
                    interned_name,
                    self.hir.intern_expr(param_type),
                    Some(self.hir.intern_expr(default_expr)),
                );
                let hir_id = self.hir.put(HirMapping::Param(
                    self.hir.intern_param(param),
                    owner.hir_id,
                ));
                self.ctx
                    .scope_manager
                    .add_item(Item::new(interned_name, hir_id, None), fn_body_scope);
                hir_id
            }
            _ => unreachable!("Unexpected param kind: {:?}", kind),
        };

        Ok(hir_id)
    }
}
