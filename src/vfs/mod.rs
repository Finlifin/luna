mod vfs_visitor;
pub use vfs_visitor::*;

mod vfs_scope_scanner;
use std::{
    cell::RefCell,
    fs, mem,
    ops::{Deref, DerefMut},
    path::PathBuf,
    sync::Arc,
};
pub use vfs_scope_scanner::*;

use rustc_data_structures::fx::FxIndexMap;
use rustc_span::{FileNameDisplayPreference, SourceFile, SourceMap};

use crate::parse::ast::Ast;

pub struct Vfs {
    pub root: NodeId,
    pub nodes: FxIndexMap<NodeId, Node>,
    pub asts: RefCell<FxIndexMap<NodeId, Ast>>,
    pub project_path: PathBuf,
}

#[derive(Debug, Clone)]
pub enum Node {
    File(NodeId, Arc<SourceFile>),
    Directory(NodeId, String, Vec<NodeId>),
    SpecialFile(NodeId, Arc<SourceFile>, SpecialFileType),
    SpecialDirectory(NodeId, String, Vec<NodeId>, SpecialDirectoryType),
}

// 0 is reserved for a invalid node
pub type NodeId = usize;

pub trait NodeIdExt {
    fn is_valid(self) -> bool;
}

impl NodeIdExt for NodeId {
    fn is_valid(self) -> bool {
        self != 0
    }
}

impl Node {
    pub fn is_file(&self) -> bool {
        matches!(self, Node::File(_, _) | Node::SpecialFile(_, _, _))
    }

    pub fn is_directory(&self) -> bool {
        matches!(
            self,
            Node::Directory(_, _, _) | Node::SpecialDirectory(_, _, _, _)
        )
    }

    pub fn parent(&self) -> NodeId {
        match self {
            Node::File(id, _) => *id,
            Node::Directory(id, _, _) => *id,
            Node::SpecialFile(id, _, _) => *id,
            Node::SpecialDirectory(id, _, _, _) => *id,
        }
    }
}

impl Vfs {
    pub fn new(project_path: PathBuf) -> Self {
        let project_name = project_path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| project_path.to_string_lossy().to_string());
        let root_dir = Node::Directory(0, project_name, Vec::new());
        let mut result = Vfs {
            root: 0,
            nodes: FxIndexMap::default(),
            project_path,
            asts: RefCell::new(FxIndexMap::default()),
        };
        result.root = result.add_node(root_dir);
        result
    }

    pub fn put_ast(&self, node_id: NodeId, ast: Ast) {
        self.asts.borrow_mut().insert(node_id, ast);
    }

    pub fn get_ast<'vfs>(&'vfs self, node_id: NodeId) -> Option<&'vfs Ast> {
        unsafe { mem::transmute(self.asts.borrow().get(&node_id)) }
    }

    pub fn new_in_current_dir() -> Self {
        let project_path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Vfs::new(project_path)
    }

    // Recursively build a Vfs from a given project path
    pub fn build_from_path(
        project_path: PathBuf,
        ignores: &[&str],
        source_map: &SourceMap,
    ) -> Self {
        if !project_path.exists() {
            panic!("Project path does not exist: {:?}", project_path);
        }

        let mut vfs = Vfs::new(project_path.clone());

        // Recursively walk through the directory structure and add nodes
        if let Err(e) = vfs.build_directory_recursive(&source_map, vfs.root, &project_path, ignores)
        {
            eprintln!(
                "Warning: Failed to build VFS from path {:?}: {}",
                project_path, e
            );
        }

        vfs
    }

    fn build_directory_recursive(
        &mut self,
        source_map: &rustc_span::SourceMap,
        parent_node: NodeId,
        dir_path: &std::path::Path,
        ignores: &[&str],
    ) -> Result<(), Box<dyn std::error::Error>> {
        let entries = fs::read_dir(dir_path)?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let file_name = entry.file_name().to_string_lossy().to_string();
            
            if path.is_dir() {
                // Check if this directory should be ignored
                if ignores.contains(&file_name.as_str()) {
                    continue;
                }
                
                // Create directory node
                let dir_node = if path.file_name().unwrap().to_string_lossy().eq("src") {
                    Node::SpecialDirectory(
                        parent_node,
                        file_name,
                        Vec::new(),
                        SpecialDirectoryType::Src,
                    )
                } else {
                    Node::Directory(parent_node, file_name, Vec::new())
                };
                let dir_id = self.add_node(dir_node);
                
                // Recursively process subdirectory
                self.build_directory_recursive(source_map, dir_id, &path, ignores)?;
            } else if path.is_file() {
                // Only add source files (you can customize the extensions as needed)
                if let Some(ext) = path.extension() {
                    if ext == "fl" {
                        // Load file as SourceFile
                        if let Ok(source_file) = source_map.load_file(&path) {
                            self.add_file(parent_node, source_file);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    pub fn add_node(&mut self, node: Node) -> NodeId {
        let node_id = self.nodes.len() + 1; // Ensure node_id starts from 1
        let parent = node.parent();
        self.nodes.insert(node_id, node);
        match self.nodes.get_mut(&parent) {
            Some(Node::Directory(_, _, children))
            | Some(Node::SpecialDirectory(_, _, children, _)) => {
                children.push(node_id);
            }
            _ => {}
        }
        node_id
    }

    pub fn add_file(&mut self, parent: NodeId, source_file: Arc<SourceFile>) -> NodeId {
        let node_id = self.nodes.len() + 1; // Ensure node_id starts from 1
        self.nodes.insert(node_id, Node::File(parent, source_file));
        match self.nodes.get_mut(&parent) {
            Some(Node::Directory(_, _, children))
            | Some(Node::SpecialDirectory(_, _, children, _)) => {
                children.push(node_id);
            }
            _ => {}
        }
        node_id
    }

    pub fn resolve(&self, path: &[&str]) -> Option<NodeId> {
        let mut current_node = self.root;
        for segment in path {
            if let Some(Node::Directory(_, _, children))
            | Some(Node::SpecialDirectory(_, _, children, _)) = self.nodes.get(&current_node)
            {
                if let Some(&child_id) = children.iter().find(|&&id| {
                    match self.nodes.get(&id) {
                        // 匹配文件
                        Some(Node::File(_, source_file) | Node::SpecialFile(_, source_file, _)) => {
                            let file_name = source_file.name.prefer_local().to_string();
                            let module_name = if let Some(name) = file_name.rfind('/') {
                                &file_name[name + 1..]
                            } else {
                                &file_name
                            };

                            module_name == *segment
                        }
                        // 匹配目录
                        Some(
                            Node::Directory(_, name, _) | Node::SpecialDirectory(_, name, _, _),
                        ) => name == segment,
                        None => false,
                    }
                }) {
                    current_node = child_id;
                } else {
                    println!("[DEBUG] Segment not found: {}", segment);
                    return None; // Segment not found
                }
            } else {
                println!(
                    "[DEBUG] Current node is not a directory: {:?}",
                    self.nodes.get(&current_node)
                );
                return None; // Not a directory
            }
        }
        Some(current_node)
    }

    pub fn node_name(&self, node: NodeId) -> String {
        match self.nodes.get(&node) {
            Some(Node::File(_, source_file) | Node::SpecialFile(_, source_file, _)) => source_file
                .name
                .display(FileNameDisplayPreference::Local)
                .to_string(),
            Some(Node::Directory(_, name, _)) => name.clone(),
            Some(Node::SpecialDirectory(_, name, _, _)) => name.clone(),
            None => "<invalid node>".to_string(),
        }
    }

    pub fn node_path(&self, node: NodeId) -> PathBuf {
        let mut path_components = Vec::new();
        let mut current_node = node;
        let mut iteration: usize = 100; // Prevent infinite loops

        while current_node != 0 && iteration > 0 {
            if let Some(Node::Directory(_, name, _)) = self.nodes.get(&current_node) {
                path_components.push(name.clone());
            } else if let Some(Node::File(_, source_file) | Node::SpecialFile(_, source_file, _)) =
                self.nodes.get(&current_node)
            {
                path_components.push(
                    source_file
                        .name
                        .display(FileNameDisplayPreference::Local)
                        .to_string(),
                );
            }
            current_node = self
                .nodes
                .get(&current_node)
                .and_then(|n| Some(n.parent()))
                .unwrap_or(0);
            iteration -= 1;
        }

        path_components.reverse();
        path_components.into_iter().collect()
    }

    pub fn absolute_path(&self, node: NodeId) -> PathBuf {
        let mut path = self.project_path.clone();
        path.push(self.node_path(node));
        path
    }

    pub fn entry_file(&self, node: NodeId) -> NodeId {
        match self.nodes.get(&node) {
            Some(Node::Directory(_, _, children)) => self.find_file_in_children(children, "mod.fl"),
            Some(Node::SpecialDirectory(_, _, children, SpecialDirectoryType::Src)) => {
                self.find_file_in_children(children, "main.fl")
            }
            _ => 0,
        }
    }

    fn find_file_in_children(&self, children: &[NodeId], filename: &str) -> NodeId {
        children
            .iter()
            .find(|&&child| self.is_file_with_name(child, filename))
            .copied()
            .unwrap_or(0)
    }

    fn is_file_with_name(&self, node_id: NodeId, filename: &str) -> bool {
        match self.nodes.get(&node_id) {
            Some(Node::File(_, source_file) | Node::SpecialFile(_, source_file, _)) => {
                source_file
                    .name
                    .display(FileNameDisplayPreference::Local)
                    .to_string_lossy()
                    == filename
            }
            _ => false,
        }
    }
}

impl Deref for Vfs {
    type Target = FxIndexMap<NodeId, Node>;

    fn deref(&self) -> &Self::Target {
        &self.nodes
    }
}

impl DerefMut for Vfs {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.nodes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecialFileType {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecialDirectoryType {
    Src,
    Scripts,
    Resources,
}
