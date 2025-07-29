use crate::{
    context::{CompilerContext, scan::scan, scope::ScopeId},
    hir::Hir,
    lex::lex,
    parse::parser::Parser,
    vfs::{Node, NodeId, SpecialDirectoryType, Vfs, VfsVisitor},
};

/// VFS扫描器，负责遍历VFS树并建立scope结构
pub struct VfsScopeScanner {
    // 移除了不必要的生命周期参数和字段
}

#[derive(Debug)]
pub struct ScanResult {
    pub scope_id: ScopeId,
    pub processed_files: usize,
    pub processed_modules: usize,
}

impl VfsScopeScanner {
    pub fn new() -> Self {
        Self {}
    }

    /// 扫描整个VFS，从根节点开始
    pub fn scan_vfs<'hir>(
        &mut self,
        vfs: &Vfs,
        ctx: &mut CompilerContext<'hir>,
        hir: &'hir Hir,
    ) -> ScanResult {
        let root_scope = ctx.scope_manager.root;
        self.scan_node(vfs, vfs.root, root_scope, ctx, hir);

        // 计算统计信息
        let processed_files = self.count_processed_files(vfs, vfs.root);
        let processed_modules = self.count_processed_modules(vfs, vfs.root);

        ScanResult {
            scope_id: root_scope,
            processed_files,
            processed_modules,
        }
    }

    /// 扫描指定的VFS节点
    fn scan_node<'hir>(
        &mut self,
        vfs: &Vfs,
        node_id: NodeId,
        parent_scope: ScopeId,
        ctx: &mut CompilerContext<'hir>,
        hir: &'hir Hir,
    ) -> ScopeId {
        match vfs.nodes.get(&node_id) {
            Some(Node::File(_, source_file)) => {
                self.process_file_node(vfs, node_id, source_file, parent_scope, ctx, hir)
            }
            Some(Node::Directory(_, name, children)) => {
                self.process_directory_node(vfs, node_id, name, children, parent_scope, ctx, hir)
            }
            Some(Node::SpecialDirectory(_, name, children, dir_type)) => self
                .process_special_directory_node(
                    vfs,
                    node_id,
                    name,
                    children,
                    *dir_type,
                    parent_scope,
                    ctx,
                    hir,
                ),
            Some(Node::SpecialFile(_, source_file, _)) => {
                self.process_file_node(vfs, node_id, source_file, parent_scope, ctx, hir)
            }
            None => {
                eprintln!("Warning: Node {} not found in VFS", node_id);
                parent_scope
            }
        }
    }

    fn process_file_node<'hir>(
        &mut self,
        _vfs: &Vfs,
        _node_id: NodeId,
        source_file: &std::sync::Arc<rustc_span::SourceFile>,
        parent_scope: ScopeId,
        ctx: &mut CompilerContext<'hir>,
        hir: &'hir Hir,
    ) -> ScopeId {
        // 解析文件并扫描其内容
        let (tokens, lex_errors) = lex(&source_file.src.as_ref().unwrap(), source_file.start_pos);

        // 报告词法错误
        for err in lex_errors {
            // 使用正确的错误报告方式
            eprintln!("Lex error: {:?}", err);
        }

        // 解析AST
        let mut parser = Parser::new(&hir.source_map, tokens);
        parser.parse(ctx.diag_ctx());
        if !parser.ast.nodes.is_empty() {
            // 使用现有的scan函数处理文件内容
            scan(ctx, hir, &parser.ast, parent_scope);
        }

        parent_scope
    }

    fn process_directory_node<'hir>(
        &mut self,
        vfs: &Vfs,
        _node_id: NodeId,
        name: &str,
        children: &[NodeId],
        parent_scope: ScopeId,
        ctx: &mut CompilerContext<'hir>,
        hir: &'hir Hir,
    ) -> ScopeId {
        // 为普通目录创建模块scope
        let module_name = hir.str_arena.intern_string(name.to_string());
        let module_scope =
            match ctx
                .scope_manager
                .add_scope(Some(module_name), Some(parent_scope), false)
            {
                Ok(scope) => scope,
                Err(e) => {
                    eprintln!(
                        "Warning: Failed to create scope for directory '{}': {:?}",
                        name, e
                    );
                    return parent_scope;
                }
            };

        // 查找并处理mod.fl文件（如果存在）
        self.process_module_definition_file(vfs, children, module_scope, ctx, hir);

        // 处理子节点：目录直接递归，文件（除了mod.fl）创建子模块
        for &child_id in children {
            match vfs.nodes.get(&child_id) {
                Some(Node::File(_, source_file)) => {
                    let file_name = source_file.name.prefer_local().to_string();
                    if file_name.ends_with("mod.fl") {
                        // mod.fl 已经在 process_module_definition_file 中处理了，跳过
                        continue;
                    } else {
                        // 其他文件创建子模块
                        self.process_file_as_submodule(vfs, child_id, source_file, module_scope, ctx, hir);
                    }
                }
                Some(Node::Directory(_, _, _)) | Some(Node::SpecialDirectory(_, _, _, _)) => {
                    // 目录直接递归处理
                    self.scan_node(vfs, child_id, module_scope, ctx, hir);
                }
                Some(Node::SpecialFile(_, source_file, _)) => {
                    // 特殊文件也创建子模块
                    self.process_file_as_submodule(vfs, child_id, source_file, module_scope, ctx, hir);
                }
                None => {
                    eprintln!("Warning: Child node {} not found", child_id);
                }
            }
        }

        module_scope
    }

    fn process_special_directory_node<'hir>(
        &mut self,
        vfs: &Vfs,
        _node_id: NodeId,
        _name: &str,
        children: &[NodeId],
        dir_type: SpecialDirectoryType,
        parent_scope: ScopeId,
        ctx: &mut CompilerContext<'hir>,
        hir: &'hir Hir,
    ) -> ScopeId {
        match dir_type {
            SpecialDirectoryType::Src => {
                // src目录：查找main.fl或lib.fl作为包的入口
                self.process_src_directory(vfs, children, parent_scope, ctx, hir)
            }
            SpecialDirectoryType::Scripts | SpecialDirectoryType::Resources => {
                // 对于scripts和resources目录，直接处理子节点，不创建额外的scope
                for &child_id in children {
                    self.scan_node(vfs, child_id, parent_scope, ctx, hir);
                }
                parent_scope
            }
        }
    }

    fn process_src_directory<'hir>(
        &mut self,
        vfs: &Vfs,
        children: &[NodeId],
        parent_scope: ScopeId,
        ctx: &mut CompilerContext<'hir>,
        hir: &'hir Hir,
    ) -> ScopeId {
        // 查找包级别的定义文件（main.fl 或 lib.fl）
        let package_file = children.iter().find_map(|&child_id| {
            if let Some(Node::File(_, source_file)) = vfs.nodes.get(&child_id) {
                let file_name = source_file.name.prefer_local().to_string();

                if file_name.ends_with("main.fl") || file_name.ends_with("lib.fl") {
                    Some(child_id)
                } else {
                    None
                }
            } else {
                None
            }
        });

        // 处理包级别定义文件
        if let Some(package_file_id) = package_file {
            if let Some(Node::File(_, source_file)) = vfs.nodes.get(&package_file_id) {
                self.process_file_node(vfs, package_file_id, source_file, parent_scope, ctx, hir);
            }
        }

        // 处理其他子模块（排除包级别文件）
        for &child_id in children {
            if Some(child_id) != package_file {
                match vfs.nodes.get(&child_id) {
                    Some(Node::File(_, source_file)) => {
                        // 文件（除了main.fl/lib.fl）创建子模块
                        self.process_file_as_submodule(vfs, child_id, source_file, parent_scope, ctx, hir);
                    }
                    Some(Node::Directory(_, _, _)) | Some(Node::SpecialDirectory(_, _, _, _)) => {
                        // 目录直接递归处理
                        self.scan_node(vfs, child_id, parent_scope, ctx, hir);
                    }
                    Some(Node::SpecialFile(_, source_file, _)) => {
                        // 特殊文件也创建子模块
                        self.process_file_as_submodule(vfs, child_id, source_file, parent_scope, ctx, hir);
                    }
                    None => {
                        eprintln!("Warning: Child node {} not found", child_id);
                    }
                }
            }
        }

        parent_scope
    }

    /// 将文件作为子模块处理
    fn process_file_as_submodule<'hir>(
        &mut self,
        _vfs: &Vfs,
        _node_id: NodeId,
        source_file: &std::sync::Arc<rustc_span::SourceFile>,
        parent_scope: ScopeId,
        ctx: &mut CompilerContext<'hir>,
        hir: &'hir Hir,
    ) -> ScopeId {
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

        // 为文件创建一个子模块 scope
        let module_name_interned = hir.str_arena.intern_string(module_name.to_string());
        let file_module_scope = match ctx
            .scope_manager
            .add_scope(Some(module_name_interned), Some(parent_scope), false)
        {
            Ok(scope) => scope,
            Err(e) => {
                eprintln!(
                    "Warning: Failed to create scope for file module '{}': {:?}",
                    module_name, e
                );
                return parent_scope;
            }
        };

        // 解析文件并扫描其内容到文件模块的 scope 中
        let (tokens, lex_errors) = lex(&source_file.src.as_ref().unwrap(), source_file.start_pos);

        // 报告词法错误
        for err in lex_errors {
            eprintln!("Lex error in file '{}': {:?}", file_name, err);
        }

        // 解析AST
        let mut parser = Parser::new(&hir.source_map, tokens);
        parser.parse(ctx.diag_ctx());
        if !parser.ast.nodes.is_empty() {
            // 使用现有的scan函数处理文件内容，放入文件模块的scope中
            scan(ctx, hir, &parser.ast, file_module_scope);
        }

        file_module_scope
    }

    fn process_module_definition_file<'hir>(
        &mut self,
        vfs: &Vfs,
        children: &[NodeId],
        module_scope: ScopeId,
        ctx: &mut CompilerContext<'hir>,
        hir: &'hir Hir,
    ) {
        // 查找mod.fl文件
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

        // 如果找到mod.fl，处理其内容
        if let Some(mod_file_id) = mod_file {
            if let Some(Node::File(_, source_file)) = vfs.nodes.get(&mod_file_id) {
                self.process_file_node(vfs, mod_file_id, source_file, module_scope, ctx, hir);
            }
        }
    }

    fn count_processed_files(&self, vfs: &Vfs, node_id: NodeId) -> usize {
        match vfs.nodes.get(&node_id) {
            Some(Node::File(_, _)) | Some(Node::SpecialFile(_, _, _)) => 1,
            Some(Node::Directory(_, _, children))
            | Some(Node::SpecialDirectory(_, _, children, _)) => children
                .iter()
                .map(|&child| self.count_processed_files(vfs, child))
                .sum(),
            None => 0,
        }
    }

    fn count_processed_modules(&self, vfs: &Vfs, node_id: NodeId) -> usize {
        match vfs.nodes.get(&node_id) {
            Some(Node::File(_, _)) | Some(Node::SpecialFile(_, _, _)) => 0,
            Some(Node::Directory(_, _, children)) => {
                1 + children
                    .iter()
                    .map(|&child| self.count_processed_modules(vfs, child))
                    .sum::<usize>()
            }
            Some(Node::SpecialDirectory(_, _, children, SpecialDirectoryType::Src)) => {
                // src目录本身不算模块，但其子目录算
                children
                    .iter()
                    .map(|&child| self.count_processed_modules(vfs, child))
                    .sum()
            }
            Some(Node::SpecialDirectory(_, _, children, _)) => {
                // 其他特殊目录不算模块
                children
                    .iter()
                    .map(|&child| self.count_processed_modules(vfs, child))
                    .sum()
            }
            None => 0,
        }
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
        _source_file: &std::sync::Arc<rustc_span::SourceFile>,
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
        _source_file: &std::sync::Arc<rustc_span::SourceFile>,
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
