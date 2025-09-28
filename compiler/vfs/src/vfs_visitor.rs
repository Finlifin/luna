use std::sync::Arc;

use rustc_span::{FileNameDisplayPreference, SourceFile};

use crate::{Node, NodeId, SpecialDirectoryType, SpecialFileType, Vfs};

/// VFS visitor trait，用于遍历 VFS 树并对每个节点执行操作
pub trait VfsVisitor {
    /// 访问者的返回类型
    type Output;

    /// 访问一个节点
    fn visit_node(&mut self, vfs: &Vfs, node_id: super::NodeId) -> Self::Output;

    /// 访问文件节点
    fn visit_file(
        &mut self,
        vfs: &Vfs,
        node_id: NodeId,
        source_file: &Arc<SourceFile>,
    ) -> Self::Output;

    /// 访问目录节点
    fn visit_directory(
        &mut self,
        vfs: &Vfs,
        node_id: NodeId,
        name: &str,
        children: &[NodeId],
    ) -> Self::Output;

    /// 访问特殊文件节点
    fn visit_special_file(
        &mut self,
        vfs: &Vfs,
        node_id: NodeId,
        source_file: &Arc<SourceFile>,
        file_type: SpecialFileType,
    ) -> Self::Output;

    /// 访问特殊目录节点
    fn visit_special_directory(
        &mut self,
        vfs: &Vfs,
        node_id: NodeId,
        name: &str,
        children: &[NodeId],
        dir_type: SpecialDirectoryType,
    ) -> Self::Output;

    /// 默认的 visitor 实现，根据节点类型分发到对应的方法
    fn default_visit_node(&mut self, vfs: &Vfs, node_id: NodeId) -> Self::Output {
        if node_id == 0 {
            panic!("Cannot visit invalid node");
        }

        if let Some(node) = vfs.nodes.get(&node_id) {
            match node {
                Node::File(_, source_file) => self.visit_file(vfs, node_id, source_file),
                Node::Directory(_, name, children) => {
                    self.visit_directory(vfs, node_id, name, children)
                }
                Node::SpecialFile(_, source_file, file_type) => {
                    self.visit_special_file(vfs, node_id, source_file, *file_type)
                }
                Node::SpecialDirectory(_, name, children, dir_type) => {
                    self.visit_special_directory(vfs, node_id, name, children, *dir_type)
                }
            }
        } else {
            panic!("Node {} not found in VFS", node_id);
        }
    }
}

/// 通用的访问函数，遍历 VFS 节点
pub fn visit_vfs<V: VfsVisitor>(visitor: &mut V, vfs: &Vfs, node_id: NodeId) -> V::Output {
    visitor.visit_node(vfs, node_id)
}

/// S-表达式转储 visitor，用于将 VFS 树转换为 S-表达式格式
pub struct VfsSExpressionVisitor {
    indent_level: usize,
}

impl VfsSExpressionVisitor {
    pub fn new() -> Self {
        VfsSExpressionVisitor { indent_level: 0 }
    }

    fn indent(&self) -> String {
        "  ".repeat(self.indent_level)
    }

    fn visit_children(&mut self, vfs: &Vfs, children: &[NodeId]) -> String {
        if children.is_empty() {
            return String::new();
        }

        self.indent_level += 1;
        let children_str = children
            .iter()
            .map(|&child_id| format!("\n{}{}", self.indent(), self.visit_node(vfs, child_id)))
            .collect::<Vec<_>>()
            .join("");
        self.indent_level -= 1;

        children_str
    }
}

impl VfsVisitor for VfsSExpressionVisitor {
    type Output = String;

    fn visit_node(&mut self, vfs: &Vfs, node_id: NodeId) -> Self::Output {
        if node_id == 0 {
            return "(<invalid node>)".to_string();
        }
        self.default_visit_node(vfs, node_id)
    }

    fn visit_file(
        &mut self,
        _vfs: &Vfs,
        _node_id: NodeId,
        source_file: &Arc<SourceFile>,
    ) -> Self::Output {
        let file_name = source_file.name.display(FileNameDisplayPreference::Local);
        format!("(file \"{}\")", file_name)
    }

    fn visit_directory(
        &mut self,
        vfs: &Vfs,
        _node_id: NodeId,
        name: &str,
        children: &[NodeId],
    ) -> Self::Output {
        let children_str = self.visit_children(vfs, children);
        if children.is_empty() {
            format!("(directory \"{}\")", name)
        } else {
            format!("(directory \"{}\"{})", name, children_str)
        }
    }

    fn visit_special_file(
        &mut self,
        _vfs: &Vfs,
        _node_id: NodeId,
        source_file: &Arc<SourceFile>,
        _file_type: SpecialFileType,
    ) -> Self::Output {
        let file_name = source_file.name.display(FileNameDisplayPreference::Local);
        format!("(special-file \"{}\")", file_name)
    }

    fn visit_special_directory(
        &mut self,
        vfs: &Vfs,
        _node_id: NodeId,
        name: &str,
        children: &[NodeId],
        dir_type: SpecialDirectoryType,
    ) -> Self::Output {
        let children_str = self.visit_children(vfs, children);
        let type_str = match dir_type {
            SpecialDirectoryType::Src => "src",
            SpecialDirectoryType::Scripts => "scripts",
            SpecialDirectoryType::Resources => "resources",
        };
        if children.is_empty() {
            format!("(special-directory \"{}\" :type {})", name, type_str)
        } else {
            format!(
                "(special-directory \"{}\" :type {}{})",
                name, type_str, children_str
            )
        }
    }
}

/// 便利函数：使用 VfsSExpressionVisitor 转储 VFS 为 S-表达式
pub fn dump_vfs_to_s_expression(vfs: &Vfs, node_id: NodeId) -> String {
    let mut visitor = VfsSExpressionVisitor::new();
    visit_vfs(&mut visitor, vfs, node_id)
}
