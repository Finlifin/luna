use std::sync::Arc;

use rustc_span::SourceFile;

use crate::{
    context::{CompilerContext, scan::scan, scope::ScopeId},
    diagnostic::FlurryError,
    hir::{Hir, HirId, HirMapping},
    lex::lex,
    parse::parser::Parser,
    vfs::{Node, NodeId, SpecialDirectoryType, Vfs, vfs_visitor::VfsVisitor},
};

#[derive(Debug)]
pub struct ScanResult {
    pub scope_id: ScopeId,
}

pub struct VfsScopeScanner;

impl VfsScopeScanner {
    pub fn new() -> Self {
        Self
    }

    pub fn scan_vfs<'hir>(
        &mut self,
        vfs: &Vfs,
        ctx: &mut CompilerContext<'hir>,
        hir: &'hir Hir,
    ) -> ScanResult {
        // 获取根scope
        let root_scope = ctx.scope_manager.root;

        // 扫描项目根目录
        self.scan_package_root(vfs, vfs.root, root_scope, ctx, hir);

        ScanResult {
            scope_id: root_scope,
        }
    }

    fn scan_package_root<'hir>(
        &mut self,
        vfs: &Vfs,
        node_id: NodeId,
        root_scope: ScopeId,
        ctx: &mut CompilerContext<'hir>,
        hir: &'hir Hir,
    ) {
        if let Some(node) = vfs.nodes.get(&node_id) {
            match node {
                Node::Directory(_, project_name, children) => {
                    let project_name_interned =
                        hir.str_arena.intern_string(project_name.to_string());
                    let hir_id = hir.put(HirMapping::UnresolvedPackage(node_id));
                    let project_scope = match ctx.scope_manager.add_scope(
                        Some(project_name_interned),
                        Some(root_scope),
                        false,
                        hir_id,
                    ) {
                        Ok(scope) => scope,
                        Err(e) => {
                            eprintln!(
                                "Warning: Failed to create project scope '{}': {:?}",
                                project_name, e
                            );
                            root_scope
                        }
                    };

                    // 查找 src 目录
                    for &child_id in children {
                        if let Some(child_node) = vfs.nodes.get(&child_id) {
                            match child_node {
                                Node::SpecialDirectory(
                                    _,
                                    _name,
                                    src_children,
                                    SpecialDirectoryType::Src,
                                ) => {
                                    self.process_package_source_directory(
                                        vfs,
                                        src_children,
                                        project_scope,
                                        ctx,
                                        hir,
                                        hir_id,
                                    );
                                    return;
                                }
                                _ => {}
                            }
                        }
                    }
                }
                _ => {
                    eprintln!("Warning: Project root is not a directory");
                }
            }
        }
    }

    /// 处理项目源代码目录的内容
    fn process_package_source_directory<'hir>(
        &mut self,
        vfs: &Vfs,
        children: &[NodeId],
        scope_id: ScopeId,
        ctx: &mut CompilerContext<'hir>,
        hir: &'hir Hir,
        owner_hir_id: HirId,
    ) {
        for &child_id in children {
            if let Some(child_node) = vfs.nodes.get(&child_id) {
                match child_node {
                    Node::File(_, source_file) => {
                        let file_name = source_file.name.prefer_local().to_string();
                        // Check if this is an entry file (main.fl or lib.fl)
                        if file_name.ends_with("main.fl") || file_name.ends_with("lib.fl") {
                            // 包入口文件：内容直接扫描到包 scope 中
                            self.process_entry_file(
                                vfs,
                                hir,
                                ctx,
                                child_id,
                                source_file,
                                scope_id,
                                owner_hir_id,
                            );
                        } else {
                            // Regular file - create as submodule
                            self.process_child_as_module(
                                vfs,
                                child_id,
                                scope_id,
                                ctx,
                                hir,
                                owner_hir_id,
                            );
                        }
                    }
                    Node::Directory(_, _, _) => {
                        // Directory - create as submodule
                        self.process_child_as_module(
                            vfs,
                            child_id,
                            scope_id,
                            ctx,
                            hir,
                            owner_hir_id,
                        );
                    }
                    _ => {
                        eprintln!(
                            "Warning: Unsupported node type in source directory: {:?}",
                            child_node
                        );
                    } // Node::SpecialDirectory(_, _, _, _) => {
                      //     // Special directory - create as submodule
                      //     self.process_child_as_module(vfs, child_id, scope_id, ctx, hir);
                      // }
                      // Node::SpecialFile(_, _, _) => {
                      //     // Special file - create as submodule
                      //     self.process_child_as_module(vfs, child_id, scope_id, ctx, hir);
                      // }
                }
            }
        }
    }

    /// 处理入口文件（main.fl, lib.fl, mod.fl）
    fn process_entry_file<'hir>(
        &mut self,
        vfs: &Vfs,
        hir: &'hir Hir,
        ctx: &mut CompilerContext<'hir>,
        node_id: NodeId,
        source_file: &Arc<SourceFile>,
        parent_scope: ScopeId,
        owner_hir_id: HirId,
    ) {
        // 解析文件并扫描其内容
        let content = source_file.src.as_ref().unwrap();
        let (tokens, lex_errors) = lex(content, source_file.start_pos);

        // 报告词法错误
        for err in lex_errors {
            err.emit(ctx.diag_ctx(), source_file.start_pos);
        }

        // 解析AST
        let mut parser = Parser::new(&hir.source_map, tokens, source_file.start_pos);
        parser.parse(ctx.diag_ctx());
        vfs.put_ast(node_id, parser.ast);
        let ast = vfs
            .get_ast(node_id)
            .expect("AST not found while we just added it");

        scan(ctx, hir, node_id, ast, parent_scope, owner_hir_id);
    }

    /// 将子节点作为模块处理（可能是目录或文件）
    fn process_child_as_module<'hir>(
        &mut self,
        vfs: &Vfs,
        node_id: NodeId,
        parent_scope: ScopeId,
        ctx: &mut CompilerContext<'hir>,
        hir: &'hir Hir,
        owner_hir_id: HirId,
    ) {
        match vfs.nodes.get(&node_id) {
            Some(Node::File(_, source_file)) => {
                // 普通文件作为子模块
                self.process_file_as_submodule(
                    vfs,
                    node_id,
                    source_file,
                    parent_scope,
                    ctx,
                    hir,
                    owner_hir_id,
                );
            }
            Some(Node::Directory(_, name, children)) => {
                // 普通目录作为子模块
                self.process_directory_as_module(
                    vfs,
                    node_id,
                    name,
                    children,
                    parent_scope,
                    ctx,
                    hir,
                    owner_hir_id,
                );
            }
            _ => {
                eprintln!("Unsupported node type for child {}", node_id);
            }
        }
    }

    /// 将目录作为模块处理
    fn process_directory_as_module<'hir>(
        &mut self,
        vfs: &Vfs,
        node_id: NodeId,
        name: &str,
        children: &[NodeId],
        parent_scope: ScopeId,
        ctx: &mut CompilerContext<'hir>,
        hir: &'hir Hir,
        owner_hir_id: HirId,
    ) {
        // 为目录创建模块 scope
        let module_name = hir.str_arena.intern_string(name.to_string());
        let hir_id = hir.put(HirMapping::UnresolvedDirectoryModule(node_id, owner_hir_id));
        let module_scope =
            match ctx
                .scope_manager
                .add_scope(Some(module_name), Some(parent_scope), false, hir_id)
            {
                Ok(scope) => scope,
                Err(e) => {
                    eprintln!(
                        "Warning: Failed to create scope for directory module '{}': {:?}",
                        name, e
                    );
                    return;
                }
            };

        // 查找并处理 mod.fl 文件（目录入口文件）
        let mod_file = children.iter().find_map(|&child_id| {
            if let Some(Node::File(_, source_file)) = vfs.nodes.get(&child_id) {
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
            if let Some(Node::File(_, source_file)) = vfs.nodes.get(&mod_file_id) {
                self.process_entry_file(
                    vfs,
                    hir,
                    ctx,
                    mod_file_id,
                    source_file,
                    module_scope,
                    hir_id,
                );
            }
        }

        // 处理其他子模块（排除 mod.fl 文件）
        for &child_id in children {
            if Some(child_id) != mod_file {
                self.process_child_as_module(vfs, child_id, module_scope, ctx, hir, hir_id);
            }
        }
    }

    /// 将文件作为子模块处理
    fn process_file_as_submodule<'hir>(
        &mut self,
        vfs: &Vfs,
        node_id: NodeId,
        source_file: &Arc<SourceFile>,
        parent_scope: ScopeId,
        ctx: &mut CompilerContext<'hir>,
        hir: &'hir Hir,
        owner_hir_id: HirId,
    ) {
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
        let content = source_file.src.as_ref().unwrap();
        let (tokens, lex_errors) = lex(content, source_file.start_pos);

        // 报告词法错误
        for err in lex_errors {
            err.emit(ctx.diag_ctx(), source_file.start_pos);
        }

        // 解析AST
        let mut parser = Parser::new(&hir.source_map, tokens, source_file.start_pos);
        parser.parse(ctx.diag_ctx());
        vfs.put_ast(node_id, parser.ast);
        let ast = vfs
            .get_ast(node_id)
            .expect("AST not found while we just added it");

        // 为文件创建一个子模块 scope
        let module_name_interned = hir.str_arena.intern_string(module_name.to_string());
        let hir_id = hir.put(HirMapping::UnresolvedFileScope(node_id, owner_hir_id));
        let file_module_scope = match ctx.scope_manager.add_scope(
            Some(module_name_interned),
            Some(parent_scope),
            false,
            hir_id,
        ) {
            Ok(scope) => scope,
            Err(e) => {
                eprintln!(
                    "Warning: Failed to create scope for file module '{}': {:?}",
                    module_name, e
                );
                return;
            }
        };

        scan(ctx, hir, node_id, ast, file_module_scope, hir_id);
    }
}

impl VfsVisitor for VfsScopeScanner {
    type Output = ScopeId;

    fn visit_node(&mut self, _vfs: &Vfs, _node_id: NodeId) -> Self::Output {
        unimplemented!("Use scan_vfs instead")
    }

    fn visit_file(
        &mut self,
        _vfs: &Vfs,
        _node_id: NodeId,
        _source_file: &Arc<SourceFile>,
    ) -> Self::Output {
        unimplemented!("Use scan_vfs instead")
    }

    fn visit_directory(
        &mut self,
        _vfs: &Vfs,
        _node_id: NodeId,
        _name: &str,
        _children: &[NodeId],
    ) -> Self::Output {
        unimplemented!("Use scan_vfs instead")
    }

    fn visit_special_file(
        &mut self,
        _vfs: &Vfs,
        _node_id: NodeId,
        _source_file: &Arc<SourceFile>,
        _file_type: crate::vfs::SpecialFileType,
    ) -> Self::Output {
        unimplemented!("Use scan_vfs instead")
    }

    fn visit_special_directory(
        &mut self,
        _vfs: &Vfs,
        _node_id: NodeId,
        _name: &str,
        _children: &[NodeId],
        _dir_type: SpecialDirectoryType,
    ) -> Self::Output {
        unimplemented!("Use scan_vfs instead")
    }
}
