//! Virtual File System for a single package.
//!
//! The VFS manages source files and their parsed ASTs. It is a **storage and
//! lookup layer only** – parsing is the caller's responsibility.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use rustc_span::{SourceFile, SourceMap};

use ast::{Ast, NodeIndex};

// ── Identifiers ──────────────────────────────────────────────────────────────

/// Identifies a source file within a package's [`Vfs`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileId(u32);

impl FileId {
    pub const INVALID: FileId = FileId(u32::MAX);

    #[inline]
    pub fn from_raw(raw: u32) -> Self {
        FileId(raw)
    }

    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }

    #[inline]
    pub fn raw(self) -> u32 {
        self.0
    }

    #[inline]
    pub fn is_valid(self) -> bool {
        self != Self::INVALID
    }
}

/// Globally unique identifier for an AST node within a package.
///
/// Encodes **two** `u32` values:
/// - `file` – indexes into the VFS file list ([`FileId`]).
/// - `node` – indexes into the file's AST node array ([`NodeIndex`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AstNodeId {
    pub file: u32,
    pub node: u32,
}

impl AstNodeId {
    pub const INVALID: AstNodeId = AstNodeId {
        file: u32::MAX,
        node: 0,
    };

    #[inline]
    pub fn new(file: FileId, node: NodeIndex) -> Self {
        AstNodeId {
            file: file.0,
            node,
        }
    }

    #[inline]
    pub fn file_id(self) -> FileId {
        FileId(self.file)
    }

    #[inline]
    pub fn node_index(self) -> NodeIndex {
        self.node
    }

    #[inline]
    pub fn is_valid(self) -> bool {
        self.file != u32::MAX && self.node != 0
    }
}

// ── Source entry ─────────────────────────────────────────────────────────────

/// A source file entry stored in the VFS.
pub struct SourceEntry {
    /// Relative path from the package root (e.g. `src/main.fl`).
    pub rel_path: PathBuf,
    /// The `rustc_span` source file handle (source text + byte positions).
    pub source_file: Arc<SourceFile>,
}

// ── VFS ──────────────────────────────────────────────────────────────────────

/// Virtual File System for a single package.
///
/// Stores source files and their parsed ASTs. Parsing is performed externally
/// and fed into the VFS via [`Vfs::set_ast`].
pub struct Vfs {
    /// Package / project name.
    pub name: String,
    /// Absolute path to the package root directory.
    pub root: PathBuf,
    /// Source files, indexed by [`FileId`].
    files: Vec<SourceEntry>,
    /// Parsed ASTs, indexed by [`FileId`]. `None` until parsing is complete.
    asts: Vec<Option<Ast>>,
}

impl Vfs {
    /// Create an empty VFS for a package.
    pub fn new(name: impl Into<String>, root: PathBuf) -> Self {
        Vfs {
            name: name.into(),
            root,
            files: Vec::new(),
            asts: Vec::new(),
        }
    }

    // ── File management ──────────────────────────────────────────────────

    /// Add a source file and return its [`FileId`].
    pub fn add_file(&mut self, rel_path: PathBuf, source_file: Arc<SourceFile>) -> FileId {
        let id = FileId(self.files.len() as u32);
        self.files.push(SourceEntry {
            rel_path,
            source_file,
        });
        self.asts.push(None);
        id
    }

    /// Get the source entry for a file.
    #[inline]
    pub fn file(&self, id: FileId) -> &SourceEntry {
        &self.files[id.index()]
    }

    /// Look up a file by its relative path.
    pub fn find_file(&self, rel_path: &Path) -> Option<FileId> {
        self.files
            .iter()
            .position(|e| e.rel_path == rel_path)
            .map(|i| FileId(i as u32))
    }

    /// Number of source files in this VFS.
    #[inline]
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Iterate over all files with their [`FileId`]s.
    pub fn files(&self) -> impl Iterator<Item = (FileId, &SourceEntry)> {
        self.files
            .iter()
            .enumerate()
            .map(|(i, entry)| (FileId(i as u32), entry))
    }

    // ── AST management ───────────────────────────────────────────────────

    /// Store a parsed AST for a file. Replaces any previous AST.
    pub fn set_ast(&mut self, id: FileId, ast: Ast) {
        self.asts[id.index()] = Some(ast);
    }

    /// Get a file's AST (returns `None` if not yet parsed).
    #[inline]
    pub fn get_ast(&self, id: FileId) -> Option<&Ast> {
        self.asts.get(id.index())?.as_ref()
    }

    /// Get a mutable reference to a file's AST.
    #[inline]
    pub fn get_ast_mut(&mut self, id: FileId) -> Option<&mut Ast> {
        self.asts.get_mut(id.index())?.as_mut()
    }

    // ── AstNodeId helpers ────────────────────────────────────────────────

    /// Build an [`AstNodeId`] from a file and node index.
    #[inline]
    pub fn node_id(&self, file: FileId, node: NodeIndex) -> AstNodeId {
        AstNodeId::new(file, node)
    }

    /// Resolve an [`AstNodeId`] to `(Ast, NodeIndex)`.
    pub fn resolve_node(&self, id: AstNodeId) -> Option<(&Ast, NodeIndex)> {
        let ast = self.get_ast(id.file_id())?;
        Some((ast, id.node_index()))
    }

    // ── Directory scanning ───────────────────────────────────────────────

    /// Scan a package directory and populate the VFS with all `.fl` source
    /// files found recursively.
    ///
    /// Directories whose names appear in `ignores` are skipped.
    pub fn scan(root: PathBuf, source_map: &SourceMap, ignores: &[&str]) -> Self {
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unnamed".into());

        let mut vfs = Vfs::new(name, root.clone());
        vfs.scan_dir(source_map, &root, &root, ignores);
        vfs
    }

    fn scan_dir(
        &mut self,
        source_map: &SourceMap,
        base: &Path,
        dir: &Path,
        ignores: &[&str],
    ) {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("warning: cannot read directory {:?}: {}", dir, e);
                return;
            }
        };

        // Collect and sort for deterministic ordering.
        let mut entries: Vec<_> = entries.filter_map(|e| e.ok()).collect();
        entries.sort_by_key(|e| e.file_name());

        for entry in entries {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();

            if ignores.contains(&name.as_str()) {
                continue;
            }

            if path.is_dir() {
                self.scan_dir(source_map, base, &path, ignores);
            } else if path.extension().is_some_and(|ext| ext == "fl") {
                if let Ok(source_file) = source_map.load_file(&path) {
                    let rel_path = path.strip_prefix(base).unwrap_or(&path).to_path_buf();
                    self.add_file(rel_path, source_file);
                }
            }
        }
    }
}
