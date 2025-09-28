mod ast_scanner;
mod error;
mod import_resolution;
mod vfs_scanner;

pub use error::*;
pub use import_resolution::*;
pub use vfs_scanner::*;

use std::collections::HashMap;

use crate::{
    context::{CompilerContext, scope::ScopeId},
    hir::Hir,
    vfs::Vfs,
};

/// Orchestrates the scan pass over the virtual file system, building scopes and collecting imports.
pub struct ScanOrchestrator;

impl ScanOrchestrator {
    pub fn new() -> Self {
        Self
    }

    /// Execute the complete scan pass on a VFS
    pub fn run<'hir, 'vfs>(
        &mut self,
        vfs: &'vfs Vfs,
        ctx: &mut CompilerContext<'hir>,
        hir: &'hir Hir,
    ) -> ScanResult<HashMap<ScopeId, Vec<PendingImport<'vfs>>>> {
        let mut vfs_scan_ctx = VfsScopeScanner::new(ctx, hir, vfs);
        vfs_scan_ctx.scan_vfs()
    }
}
