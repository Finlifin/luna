use super::error::{ScanError, ScanResult};
use crate::{
    context::{
        CompilerContext,
        scope::{Item, ScopeId},
    },
    hir::*,
    parse::ast::{Ast, NodeIndex, NodeKind},
    vfs,
};

/// AST scan context that holds all necessary state for scanning AST into scopes.
// Better name suggestion: AstScopeScanner
pub struct AstScopeScanner<'hir, 'ctx> {
    pub ctx: &'ctx mut CompilerContext<'hir>,
    pub hir: &'hir Hir,
    pub file_id: vfs::NodeId,
    pub ast: &'ctx Ast,
}

impl<'hir, 'ctx> AstScopeScanner<'hir, 'ctx> {
    pub fn new(
        ctx: &'ctx mut CompilerContext<'hir>,
        hir: &'hir Hir,
        file_id: vfs::NodeId,
        ast: &'ctx Ast,
    ) -> Self {
        Self {
            ctx,
            hir,
            file_id,
            ast,
        }
    }

    /// Scans AST nodes to register scopes and items, returning unresolved import indices.
    pub fn scan_ast(
        &mut self,
        parent_scope: ScopeId,
        owner_hir_id: HirId,
    ) -> ScanResult<Vec<(ScopeId, NodeIndex)>> {
        let file_scope_items_index = self.ast.get_children(self.ast.root)[0];
        let items = self
            .ast
            .get_multi_child_slice(file_scope_items_index)
            .ok_or_else(|| ScanError::InternalError("Invalid items slice".into()))?;

        self.scan_items(parent_scope, items, owner_hir_id)
    }

    /// Scans a list of AST item nodes, collecting unresolved imports.
    fn scan_items(
        &mut self,
        parent_scope: ScopeId,
        items: &[NodeIndex],
        owner_hir_id: HirId,
    ) -> ScanResult<Vec<(ScopeId, NodeIndex)>> {
        let mut imports_index = vec![];

        for &item in items {
            let item_kind = self
                .ast
                .get_node_kind(item)
                .ok_or_else(|| ScanError::InternalError("Invalid node index".into()))?;

            use NodeKind::*;
            match item_kind {
                ModuleDef => {
                    self.scan_module_def(parent_scope, item, owner_hir_id, &mut imports_index)?;
                }
                StructDef => {
                    self.scan_struct_def(parent_scope, item, owner_hir_id, &mut imports_index)?;
                }
                EnumDef => {
                    self.scan_enum_def(parent_scope, item, owner_hir_id, &mut imports_index)?;
                }
                UnionDef => {
                    self.scan_union_def(parent_scope, item, owner_hir_id, &mut imports_index)?;
                }
                FunctionDef => {
                    self.scan_function_def(parent_scope, item, owner_hir_id)?;
                }
                UseStatement => {
                    imports_index.push((parent_scope, item));
                }
                _ => {
                    // 忽略其他节点类型
                }
            }
        }

        Ok(imports_index)
    }

    /// Scans a module definition node, creates a new scope, and scans nested items.
    fn scan_module_def(
        &mut self,
        parent_scope: ScopeId,
        item: NodeIndex,
        owner_hir_id: HirId,
        imports_index: &mut Vec<(ScopeId, NodeIndex)>,
    ) -> ScanResult<()> {
        let id = self.ast.get_children(item)[0];
        let name = self.hir.str_arena.intern_string(
            self.ast
                .source_content(id, &self.hir.source_map)
                .ok_or_else(|| ScanError::InternalError("Failed to get source content".into()))?,
        );

        let hir_id = self
            .hir
            .put(HirMapping::Unresolved(self.file_id, item, owner_hir_id));
        let scope = self
            .ctx
            .scope_manager
            .add_scope(Some(name), Some(parent_scope), false, hir_id)
            .map_err(|e| ScanError::ScopeCreationFailed {
                message: format!("Failed to create module scope: {:?}", e),
                span: rustc_span::DUMMY_SP,
            })?;

        let block_index = self.ast.get_children(item)[2];
        let block_items_index = self.ast.get_children(block_index)[0];
        let block_items = self
            .ast
            .get_multi_child_slice(block_items_index)
            .ok_or_else(|| ScanError::InternalError("Invalid block items slice".into()))?;

        let nested_imports = self.scan_items(scope, block_items, hir_id)?;
        imports_index.extend(nested_imports);

        Ok(())
    }

    /// Scans a struct definition node, creates a new scope, and scans nested items.
    fn scan_struct_def(
        &mut self,
        parent_scope: ScopeId,
        item: NodeIndex,
        owner_hir_id: HirId,
        imports_index: &mut Vec<(ScopeId, NodeIndex)>,
    ) -> ScanResult<()> {
        let id = self.ast.get_children(item)[0];
        let name = self.hir.str_arena.intern_string(
            self.ast
                .source_content(id, &self.hir.source_map)
                .ok_or_else(|| ScanError::InternalError("Failed to get source content".into()))?,
        );

        let hir_id = self
            .hir
            .put(HirMapping::Unresolved(self.file_id, item, owner_hir_id));
        let scope = self
            .ctx
            .scope_manager
            .add_scope(Some(name), Some(parent_scope), false, hir_id)
            .map_err(|e| ScanError::ScopeCreationFailed {
                message: format!("Failed to create struct scope: {:?}", e),
                span: rustc_span::DUMMY_SP,
            })?;

        let body_index = self.ast.get_children(item)[2];
        let body_items_index = self.ast.get_children(body_index)[0];
        let body_items = self
            .ast
            .get_multi_child_slice(body_items_index)
            .ok_or_else(|| ScanError::InternalError("Invalid body items slice".into()))?;

        let nested_imports = self.scan_items(scope, body_items, hir_id)?;
        imports_index.extend(nested_imports);

        Ok(())
    }

    /// Scans an enum definition node, creates a new scope, and scans nested items.
    fn scan_enum_def(
        &mut self,
        parent_scope: ScopeId,
        item: NodeIndex,
        owner_hir_id: HirId,
        imports_index: &mut Vec<(ScopeId, NodeIndex)>,
    ) -> ScanResult<()> {
        let id = self.ast.get_children(item)[0];
        let name = self.hir.str_arena.intern_string(
            self.ast
                .source_content(id, &self.hir.source_map)
                .ok_or_else(|| ScanError::InternalError("Failed to get source content".into()))?,
        );

        let hir_id = self
            .hir
            .put(HirMapping::Unresolved(self.file_id, item, owner_hir_id));
        let scope = self
            .ctx
            .scope_manager
            .add_scope(Some(name), Some(parent_scope), false, hir_id)
            .map_err(|e| ScanError::ScopeCreationFailed {
                message: format!("Failed to create enum scope: {:?}", e),
                span: rustc_span::DUMMY_SP,
            })?;

        let body_index = self.ast.get_children(item)[2];
        let body_items_index = self.ast.get_children(body_index)[0];
        let body_items = self
            .ast
            .get_multi_child_slice(body_items_index)
            .ok_or_else(|| ScanError::InternalError("Invalid body items slice".into()))?;

        let nested_imports = self.scan_items(scope, body_items, hir_id)?;
        imports_index.extend(nested_imports);

        Ok(())
    }

    /// Scans a union definition node, creates a new scope, and scans nested items.
    fn scan_union_def(
        &mut self,
        parent_scope: ScopeId,
        item: NodeIndex,
        owner_hir_id: HirId,
        imports_index: &mut Vec<(ScopeId, NodeIndex)>,
    ) -> ScanResult<()> {
        let id = self.ast.get_children(item)[0];
        let name = self.hir.str_arena.intern_string(
            self.ast
                .source_content(id, &self.hir.source_map)
                .ok_or_else(|| ScanError::InternalError("Failed to get source content".into()))?,
        );

        let hir_id = self
            .hir
            .put(HirMapping::Unresolved(self.file_id, item, owner_hir_id));
        let scope = self
            .ctx
            .scope_manager
            .add_scope(Some(name), Some(parent_scope), false, hir_id)
            .map_err(|e| ScanError::ScopeCreationFailed {
                message: format!("Failed to create union scope: {:?}", e),
                span: rustc_span::DUMMY_SP,
            })?;

        let body_index = self.ast.get_children(item)[2];
        let body_items_index = self.ast.get_children(body_index)[0];
        let body_items = self
            .ast
            .get_multi_child_slice(body_items_index)
            .ok_or_else(|| ScanError::InternalError("Invalid body items slice".into()))?;

        let nested_imports = self.scan_items(scope, body_items, hir_id)?;
        imports_index.extend(nested_imports);

        Ok(())
    }

    /// Scans a function definition node and registers it as an item in the parent scope.
    fn scan_function_def(
        &mut self,
        parent_scope: ScopeId,
        item: NodeIndex,
        owner_hir_id: HirId,
    ) -> ScanResult<()> {
        let id = self.ast.get_children(item)[0];
        let name = self.hir.str_arena.intern_string(
            self.ast
                .source_content(id, &self.hir.source_map)
                .ok_or_else(|| ScanError::InternalError("Failed to get source content".into()))?,
        );

        let hir_id = self
            .hir
            .put(HirMapping::Unresolved(self.file_id, item, owner_hir_id));
        let item = Item::new(name, hir_id, None); // 函数是纯符号，没有子scope
        self.ctx
            .scope_manager
            .add_item(item, parent_scope)
            .map_err(|e| ScanError::ScopeCreationFailed {
                message: format!("Failed to add function item: {:?}", e),
                span: rustc_span::DUMMY_SP,
            })?;

        Ok(())
    }
}
