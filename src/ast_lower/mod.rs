use core::panic;
use std::result;

use crate::{
    context::{
        CompilerContext,
        scope::{Item, ScopeId},
    },
    hir::{Definition, Expr, Hir, HirMapping, Module, SDefinition, Struct},
    parse::ast::{self, Ast},
    vfs::{self, NodeIdExt, Vfs},
};

pub struct LoweringContext<'hir, 'ctx, 'vfs> {
    pub ctx: &'ctx mut CompilerContext<'hir>,
    pub vfs: &'vfs Vfs,
    pub hir: &'hir Hir,
}

#[derive(Debug, Clone)]
pub enum LoweringError<'hir> {
    UnsupportedItem(ast::NodeKind),
    InvalidNodeIndex(ast::NodeIndex),
    InternalError(String),
    PlaceholderError(&'hir str),
}

pub type LoweringResult<'hir, T> = Result<T, LoweringError<'hir>>;

impl<'hir, 'ctx, 'vfs> LoweringContext<'hir, 'ctx, 'vfs> {
    pub fn new(
        ctx: &'ctx mut CompilerContext<'hir>,
        hir: &'hir Hir,
        vfs: &'vfs Vfs,
    ) -> LoweringContext<'hir, 'ctx, 'vfs> {
        LoweringContext { ctx, hir, vfs }
    }

    pub fn lower(&self) -> LoweringResult<'_, ()> {
        let root_scope = self.ctx.scope_manager.root;
        if let Some(packages) = self.ctx.scope_manager.items(root_scope) {
            for package in packages {
                self.lower_package(package)?;
            }
            Ok(())
        } else {
            Err(LoweringError::InternalError(
                "No items in root scope".into(),
            ))
        }
    }

    pub fn lower_package(&self, package: &Item<'hir>) -> LoweringResult<'hir, Definition<'hir>> {
        let scope_id = match package.scope_id {
            Some(id) => id,
            None => {
                return Err(LoweringError::InternalError(
                    "Package has no scope ID".into(),
                ));
            }
        };

        let items = if let Some(items) = self.ctx.scope_manager.items(scope_id) {
            self.lower_package_scope_items(package, items)?
        } else {
            return Err(LoweringError::InternalError("Invalid scope ID".into()));
        };

        let result = Definition::Package {
            name: package.symbol,
            items: self.hir.intern_definitions(items),
            scope_id,
        };
        self.hir.update(package.hir_id, HirMapping::Definition(self.hir.intern_definition(result), 0));
        Ok(result)
    }

    pub fn lower_package_scope_items(
        &self,
        package: &Item<'hir>,
        items: &[Item<'hir>],
    ) -> LoweringResult<'hir, Vec<Definition<'hir>>> {
        let mut definitions = vec![];
        for item in items {
            definitions.push(self.lower_unresolved_item(item, package)?);
        }
        Ok(definitions)
    }

    pub fn lower_unresolved_item(
        &self,
        item: &Item<'hir>,
        owner: &Item<'hir>,
    ) -> LoweringResult<'hir, Definition<'hir>> {
        if let Some(definition) = self.hir.get(item.hir_id) {
            use HirMapping::*;
            let lowered_item = match definition {
                // 通常是main.fl中的item
                Unresolved(file_id, node_index, owner_id) => {
                    let ast = self
                        .vfs
                        .get_ast(file_id)
                        .expect("Invalid file node id for AST");
                    let item_kind = ast.get_node_kind(node_index).expect("Invalid node index");
                    use ast::NodeKind::*;
                    match item_kind {
                        StructDef => self.lower_struct_def(ast, node_index, item, owner),
                        ModuleDef => self.lower_module_or_file_scope(ast, node_index, item, owner),
                        _ => Err(LoweringError::UnsupportedItem(item_kind)),
                    }?
                }
                // src下的普通目录
                UnresolvedDirectoryModule(dir_id, owner_id) => {
                    let entry_file = self.vfs.entry_file(dir_id);
                    use crate::hir::Definition;
                    if !entry_file.is_valid() {
                        Definition::Module(Module {
                            name: item.symbol,
                            clauses: self.hir.intern_clauses(vec![]),
                            scope_id: owner.scope_id.expect("Invalid owner scope ID"),
                        })
                    } else {
                        let ast = self
                            .vfs
                            .get_ast(entry_file)
                            .expect("Invalid entry file for AST");
                        self.lower_module_or_file_scope(ast, ast.root, item, owner)?
                    }
                }
                // src下的普通文件
                UnresolvedFileScope(file_id, owner_id) => {
                    let ast = self
                        .vfs
                        .get_ast(file_id)
                        .expect("Invalid file node id for AST");
                    self.lower_module_or_file_scope(ast, ast.root, item, owner)?
                }
                _ => {
                    panic!("Unexpected lowering repetition: {:?}", definition);
                }
            };

            self.hir.update(
                item.hir_id,
                Definition(self.hir.intern_definition(lowered_item), owner.hir_id),
            );
            Ok(lowered_item)
        } else {
            Err(LoweringError::InternalError("Invalid HIR mapping".into()))
        }
    }

    pub fn lower_module_or_file_scope(
        &self,
        ast: &Ast,
        node_index: ast::NodeIndex,
        item: &Item<'hir>,
        owner: &Item<'hir>,
    ) -> LoweringResult<'hir, Definition<'hir>> {
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

    fn lower_struct_def(
        &self,
        ast: &Ast,
        item_index: ast::NodeIndex,
        item: &Item<'hir>,
        owner: &Item<'hir>,
    ) -> LoweringResult<'hir, Definition<'hir>> {
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
            println!("Lowering struct body item: {:?}", tag);
            use ast::NodeKind::*;
            match tag {
                StructField => {
                    let child_children = ast.get_children(child_item);
                    let field_name = ast
                        .source_content(child_children[0], &self.hir.source_map)
                        .expect("Failed to get field name");
                    println!("[DEBUG] Lowering struct field: {:?}", &field_name);
                    let field = Definition::StructField(
                        self.hir.intern_str(&field_name),
                        self.hir.intern_expr(Expr::TyAny),
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

    fn assert_kind(&self, ast: &Ast, ast_node_index: ast::NodeIndex, expected_kind: ast::NodeKind) {
        if let Some(actual_kind) = ast.get_node_kind(ast_node_index) {
            assert_eq!(
                actual_kind, expected_kind,
                "Expected node index {:?} to have kind {:?}, but found {:?}",
                ast_node_index, expected_kind, actual_kind
            );
        } else {
            panic!("Node index {:?} does not have a valid kind", ast_node_index);
        }
    }
}
