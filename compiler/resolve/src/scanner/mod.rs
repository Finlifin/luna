//! Scanners – build the scope tree and collect imports from AST and VFS.
//!
//! This module is the migration target for `luna/src/scan`. It contains:
//!
//! - [`AstScanner`] – walks one file's AST to register items and collect use-statement indices.
//! - [`VfsScanner`] – walks the VFS directory tree, parses files, and delegates to `AstScanner`.

mod ast_scanner;
mod vfs_scanner;

pub use ast_scanner::AstScanner;
pub use vfs_scanner::VfsScanner;
