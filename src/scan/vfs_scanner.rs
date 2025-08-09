use rustc_span::SourceFile;
use std::{collections::HashMap, sync::Arc};

use super::{
    ast_scanner::AstScopeScanner,
    error::{ScanError, ScanResult},
};
use crate::{
    context::{CompilerContext, scope::ScopeId},
    diagnostic::FlurryError,
    hir::{Hir, HirId, HirMapping},
    lex::lex,
    parse::parser::Parser,
    scan::UnresolvedImport,
    vfs::{Node, NodeId, SpecialDirectoryType, Vfs},
};

/// Virtual file system scan context: orchestrates scanning of directories and files into scopes.
pub struct VfsScopeScanner<'hir, 'ctx, 'vfs> {
    /// Compiler context for the current compilation.
    pub ctx: &'ctx mut CompilerContext<'hir>,
    /// Virtual file system reference.
    pub vfs: &'vfs Vfs,
    /// High-level intermediate representation (HIR) mapping.
    pub hir: &'hir Hir,
    /// Unresolved imports collected per scope.
    pub unresolved_imports: HashMap<ScopeId, Vec<UnresolvedImport<'vfs>>>,
}

impl<'hir, 'ctx, 'vfs> VfsScopeScanner<'hir, 'ctx, 'vfs> {
    /// Creates a new virtual file system scan context.
    pub fn new(
        ctx: &'ctx mut CompilerContext<'hir>,
        hir: &'hir Hir,
        vfs: &'vfs Vfs,
    ) -> VfsScopeScanner<'hir, 'ctx, 'vfs> {
        VfsScopeScanner {
            ctx,
            hir,
            vfs,
            unresolved_imports: HashMap::new(),
        }
    }

    /// Scans the entire virtual file system, building scopes and collecting unresolved imports.
    pub fn scan_vfs(&mut self) -> ScanResult<HashMap<ScopeId, Vec<UnresolvedImport<'vfs>>>> {
        let root_scope = self.ctx.scope_manager.root;

        // 扫描项目根目录
        self.scan_package_root(self.vfs.root, root_scope)?;

        // 返回未解析的导入map
        Ok(std::mem::take(&mut self.unresolved_imports))
    }

    /// Adds an unresolved import to the specified scope.
    fn add_import(&mut self, scope_id: ScopeId, import: UnresolvedImport<'vfs>) {
        self.unresolved_imports
            .entry(scope_id)
            .or_insert_with(Vec::new)
            .push(import);
    }

    /// Processes the project root node and creates the initial project scope.
    fn scan_package_root(&mut self, node_id: NodeId, root_scope: ScopeId) -> ScanResult<()> {
        let node = self
            .vfs
            .nodes
            .get(&node_id)
            .ok_or_else(|| ScanError::InternalError("Project root node not found".into()))?;

        match node {
            Node::Directory(_, project_name, children) => {
                let project_name_interned =
                    self.hir.str_arena.intern_string(project_name.to_string());
                let hir_id = self.hir.put(HirMapping::UnresolvedPackage(node_id));
                let project_scope = self
                    .ctx
                    .scope_manager
                    .add_scope(Some(project_name_interned), Some(root_scope), false, hir_id)
                    .map_err(|e| ScanError::ScopeCreationFailed {
                        message: format!(
                            "Failed to create project scope '{}': {:?}",
                            project_name, e
                        ),
                        span: rustc_span::DUMMY_SP,
                    })?;

                // 查找 src 目录
                for &child_id in children {
                    if let Some(child_node) = self.vfs.nodes.get(&child_id) {
                        if let Node::SpecialDirectory(
                            _,
                            _name,
                            src_children,
                            SpecialDirectoryType::Src,
                        ) = child_node
                        {
                            self.process_package_source_directory(
                                src_children,
                                project_scope,
                                hir_id,
                            )?;
                        }
                    }
                }

                Ok(())
            }
            _ => Err(ScanError::InvalidNodeType {
                message: "Project root is not a directory".into(),
                span: rustc_span::DUMMY_SP,
            }),
        }
    }

    /// 处理项目源代码目录的内容
    /// Processes the source directory of a package, scanning each file and subdirectory.
    fn process_package_source_directory(
        &mut self,
        children: &[NodeId],
        scope_id: ScopeId,
        owner_hir_id: HirId,
    ) -> ScanResult<()> {
        for &child_id in children {
            if let Some(child_node) = self.vfs.nodes.get(&child_id) {
                match child_node {
                    Node::File(_, source_file) => {
                        let file_name = source_file.name.prefer_local().to_string();
                        // Check if this is an entry file (main.fl or lib.fl)
                        if file_name.ends_with("main.fl") || file_name.ends_with("lib.fl") {
                            // 包入口文件：内容直接扫描到包 scope 中
                            self.process_entry_file(child_id, source_file, scope_id, owner_hir_id)?;
                        } else {
                            // Regular file - create as submodule
                            self.process_child_as_module(child_id, scope_id, owner_hir_id)?;
                        }
                    }
                    Node::Directory(_, _, _) => {
                        // Directory - create as submodule
                        self.process_child_as_module(child_id, scope_id, owner_hir_id)?;
                    }
                    _ => {
                        return Err(ScanError::InvalidNodeType {
                            message: format!(
                                "Unsupported node type in source directory: {:?}",
                                child_node
                            ),
                            span: rustc_span::DUMMY_SP,
                        });
                    }
                }
            }
        }

        Ok(())
    }

    /// 处理入口文件（main.fl, lib.fl, mod.fl）
    /// Processes an entry file (main.fl, lib.fl, mod.fl), parsing and scanning its AST content.
    fn process_entry_file(
        &mut self,
        node_id: NodeId,
        source_file: &Arc<SourceFile>,
        parent_scope: ScopeId,
        owner_hir_id: HirId,
    ) -> ScanResult<()> {
        // 解析文件并扫描其内容
        let content = source_file
            .src
            .as_ref()
            .ok_or_else(|| ScanError::FileParsingFailed {
                message: "Source file content is None".into(),
                span: rustc_span::DUMMY_SP,
            })?;

        let (tokens, lex_errors) = lex(content, source_file.start_pos);

        // 报告词法错误
        for err in lex_errors {
            err.emit(self.ctx.diag_ctx(), source_file.start_pos);
        }

        // 解析AST
        let mut parser = Parser::new(&self.hir.source_map, tokens, source_file.start_pos);
        parser.parse(self.ctx.diag_ctx());
        self.vfs.put_ast(node_id, parser.ast);
        let ast = self.vfs.get_ast(node_id).ok_or_else(|| {
            ScanError::InternalError("AST not found while we just added it".into())
        })?;

        let mut ast_scan_ctx = AstScopeScanner::new(self.ctx, self.hir, node_id, ast);
        let import_indices = ast_scan_ctx.scan_ast(parent_scope, owner_hir_id)?;

        // 将导入添加到map中
        for (parent_scope_id, import_index) in import_indices {
            let import = UnresolvedImport(ast, import_index, parent_scope_id);
            self.add_import(parent_scope_id, import);
        }

        Ok(())
    }

    /// 将子节点作为模块处理（可能是目录或文件）
    /// Processes a child node as a module, handling files or directories appropriately.
    fn process_child_as_module(
        &mut self,
        node_id: NodeId,
        parent_scope: ScopeId,
        owner_hir_id: HirId,
    ) -> ScanResult<()> {
        match self.vfs.nodes.get(&node_id) {
            Some(Node::File(_, source_file)) => {
                // 普通文件作为子模块
                self.process_file_as_submodule(node_id, source_file, parent_scope, owner_hir_id)
            }
            Some(Node::Directory(_, name, children)) => {
                // 普通目录作为子模块
                self.process_directory_as_module(
                    node_id,
                    name,
                    children,
                    parent_scope,
                    owner_hir_id,
                )
            }
            _ => Err(ScanError::InvalidNodeType {
                message: format!("Unsupported node type for child {}", node_id),
                span: rustc_span::DUMMY_SP,
            }),
        }
    }

    /// 将目录作为模块处理
    /// Processes a directory node as a module, creating its scope and scanning its contents.
    fn process_directory_as_module(
        &mut self,
        node_id: NodeId,
        name: &str,
        children: &[NodeId],
        parent_scope: ScopeId,
        owner_hir_id: HirId,
    ) -> ScanResult<()> {
        // 为目录创建模块 scope
        let module_name = self.hir.str_arena.intern_string(name.to_string());
        let hir_id = self
            .hir
            .put(HirMapping::UnresolvedDirectoryModule(node_id, owner_hir_id));
        let module_scope = self
            .ctx
            .scope_manager
            .add_scope(Some(module_name), Some(parent_scope), false, hir_id)
            .map_err(|e| ScanError::ScopeCreationFailed {
                message: format!(
                    "Failed to create scope for directory module '{}': {:?}",
                    name, e
                ),
                span: rustc_span::DUMMY_SP,
            })?;

        // 查找并处理 mod.fl 文件（目录入口文件）
        let mod_file = children.iter().find_map(|&child_id| {
            if let Some(Node::File(_, source_file)) = self.vfs.nodes.get(&child_id) {
                let file_name = source_file.name.prefer_local().to_string();
                if file_name.ends_with("mod.fl") {
                    Some(child_id)
                } else {
                    None
                }
            } else {
                None
            }
        });

        if let Some(mod_file_id) = mod_file {
            if let Some(Node::File(_, source_file)) = self.vfs.nodes.get(&mod_file_id) {
                self.process_entry_file(mod_file_id, source_file, module_scope, hir_id)?;
            }
        }

        // 处理其他子模块（排除 mod.fl 文件）
        for &child_id in children {
            if Some(child_id) != mod_file {
                self.process_child_as_module(child_id, module_scope, hir_id)?;
            }
        }
        Ok(())
    }

    /// 将文件作为子模块处理
    /// Processes a file node as a submodule, creating a scope and scanning its AST.
    fn process_file_as_submodule(
        &mut self,
        node_id: NodeId,
        source_file: &Arc<SourceFile>,
        parent_scope: ScopeId,
        owner_hir_id: HirId,
    ) -> ScanResult<()> {
        // 从文件名提取模块名（去掉扩展名）
        let file_name = source_file.name.prefer_local().to_string();
        let module_name = if let Some(name) = file_name.rfind('/') {
            &file_name[name + 1..]
        } else {
            &file_name
        };

        // 去掉 .fl 扩展名
        let module_name = if module_name.ends_with(".fl") {
            &module_name[..module_name.len() - 3]
        } else {
            module_name
        };

        // 解析文件并扫描其内容到文件模块的 scope 中
        let content = source_file
            .src
            .as_ref()
            .ok_or_else(|| ScanError::FileParsingFailed {
                message: "Source file content is None".into(),
                span: rustc_span::DUMMY_SP,
            })?;

        let (tokens, lex_errors) = lex(content, source_file.start_pos);

        // 报告词法错误
        for err in lex_errors {
            err.emit(self.ctx.diag_ctx(), source_file.start_pos);
        }

        // 解析AST
        let mut parser = Parser::new(&self.hir.source_map, tokens, source_file.start_pos);
        parser.parse(self.ctx.diag_ctx());
        self.vfs.put_ast(node_id, parser.ast);
        let ast = self.vfs.get_ast(node_id).ok_or_else(|| {
            ScanError::InternalError("AST not found while we just added it".into())
        })?;

        // 为文件创建一个子模块 scope
        let module_name_interned = self.hir.str_arena.intern_string(module_name.to_string());
        let hir_id = self
            .hir
            .put(HirMapping::UnresolvedFileScope(node_id, owner_hir_id));
        let file_module_scope = self
            .ctx
            .scope_manager
            .add_scope(
                Some(module_name_interned),
                Some(parent_scope),
                false,
                hir_id,
            )
            .map_err(|e| ScanError::ScopeCreationFailed {
                message: format!(
                    "Failed to create scope for file module '{}': {:?}",
                    module_name, e
                ),
                span: rustc_span::DUMMY_SP,
            })?;

        let mut ast_scan_ctx = AstScopeScanner::new(self.ctx, self.hir, node_id, ast);
        let import_indices = ast_scan_ctx.scan_ast(file_module_scope, hir_id)?;

        // 将导入添加到map中
        for (parent_scope_id, import_index) in import_indices {
            let import = UnresolvedImport(ast, import_index, parent_scope_id);
            self.add_import(parent_scope_id, import);
        }

        Ok(())
    }
}
