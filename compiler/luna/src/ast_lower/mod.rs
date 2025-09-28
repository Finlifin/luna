mod definition;
mod error;
mod expr;
mod pattern;
use crate::{
    context::{CompilerContext, scope::Item},
    hir::{Definition, Hir, HirId, HirMapping, Module},
};
use ast::Ast;
use core::panic;
pub use error::*;
use vfs::{NodeIdExt, Vfs};

pub struct LoweringContext<'hir, 'ctx, 'vfs> {
    pub ctx: &'ctx CompilerContext<'hir>,
    pub vfs: &'vfs Vfs,
    pub hir: &'hir Hir,
}

pub type LoweringResult<T> = Result<T, LowerError>;

impl<'hir, 'ctx, 'vfs> LoweringContext<'hir, 'ctx, 'vfs> {
    pub fn new(
        ctx: &'ctx CompilerContext<'hir>,
        hir: &'hir Hir,
        vfs: &'vfs Vfs,
    ) -> LoweringContext<'hir, 'ctx, 'vfs> {
        LoweringContext { ctx, hir, vfs }
    }

    pub fn lower(&self) -> LoweringResult<()> {
        let root_scope = self.ctx.scope_manager.root;
        if let Some(packages) = self.ctx.scope_manager.items(root_scope) {
            for package in packages {
                self.lower_package(&package)?;
            }
            Ok(())
        } else {
            Err(LowerError::InternalError("No items in root scope".into()))
        }
    }

    pub fn lower_package(&self, package: &Item<'hir>) -> LoweringResult<Definition<'hir>> {
        let scope_id = match package.scope_id {
            Some(id) => id,
            None => {
                return Err(LowerError::InternalError("Package has no scope ID".into()));
            }
        };

        if let Some(items) = self.ctx.scope_manager.items(scope_id) {
            self.lower_package_scope_items(package, &items)?;
        } else {
            return Err(LowerError::InternalError("Invalid scope ID".into()));
        };

        let result = Definition::Package {
            name: package.symbol,
            scope_id,
        };
        self.hir.update(
            package.hir_id,
            HirMapping::Definition(self.hir.intern_definition(result), 0),
        );
        Ok(result)
    }

    pub fn lower_package_scope_items(
        &self,
        package: &Item<'hir>,
        items: &[Item<'hir>],
    ) -> LoweringResult<()> {
        for item in items {
            self.lower_unresolved_item(item, package)?;
        }
        Ok(())
    }

    pub fn lower_unresolved_item(
        &self,
        item: &Item<'hir>,
        owner: &Item<'hir>,
    ) -> LoweringResult<()> {
        if let Some(definition) = self.hir.get(item.hir_id) {
            use HirMapping::*;
            let result = match definition {
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
                        FunctionDef => self.lower_function_def(ast, node_index, item, owner),
                        _ => Err(LowerError::InternalError(format!(
                            "Unexpected AST node kind for unresolved item: {:?}",
                            item_kind
                        ))),
                    }
                }
                UnresolvedDirectoryModule(dir_id, owner_id) => {
                    let entry_file = self.vfs.entry_file(dir_id);
                    use crate::hir::Definition;
                    if !entry_file.is_valid() {
                        Ok(Definition::Module(Module {
                            name: item.symbol,
                            clauses: self.hir.intern_clauses(vec![]),
                            scope_id: item.scope_id.expect("Invalid item scope ID"),
                        }))
                    } else {
                        let ast = self
                            .vfs
                            .get_ast(entry_file)
                            .expect("Invalid entry file for AST");
                        self.lower_module_or_file_scope(ast, ast.root, item, owner)
                    }
                }
                UnresolvedFileScope(file_id, owner_id) => {
                    let ast = self
                        .vfs
                        .get_ast(file_id)
                        .expect("Invalid file node id for AST");
                    self.lower_module_or_file_scope(ast, ast.root, item, owner)
                }
                Definition(def, _) => Ok(*def),
                _ => {
                    panic!("Unexpected lowering repetition: {:?}", definition);
                }
            };

            let lowered_item = result?;

            self.hir.update(
                item.hir_id,
                Definition(self.hir.intern_definition(lowered_item), owner.hir_id),
            );
            Ok(())
        } else {
            Err(LowerError::InternalError("Invalid HIR mapping".into()))
        }
    }

    pub fn assert_kind(
        &self,
        ast: &Ast,
        ast_node_index: ast::NodeIndex,
        expected_kind: ast::NodeKind,
    ) {
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

    pub fn get_item_clauses(&self, item: &Item<'hir>) -> Vec<HirId> {
        self.ctx
            .scope_manager
            .scope_clauses(item.scope_id.unwrap())
            .expect("Failed to get clause IDs")
            .into_iter()
            .map(|clause| clause.self_id().expect("Unexpected clause without self ID"))
            .collect()
    }
}
