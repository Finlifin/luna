//! Source compilation sessions and HIR query coordination for luna.
//!
//! [`CompilerInstance`] owns source inputs, diagnostics and the query database.
//! [`Compiler`] borrows that instance to expose provider-backed HIR queries.
//! Semantic types and their checks belong to the NIR/elaboration pipeline.

mod session;

pub use session::{CompilerConfig, Session};

// Re-export dependency crates so downstream passes only need `interface`.
pub use diagnostic;
pub use hir;
pub use intrinsic;
pub use middle;
pub use query;
pub use vfs;

use std::ops::Deref;
use std::sync::Arc;

use diagnostic::DiagnosticContext;
use hir::HirArena;
use intrinsic::IntrinsicContext;
use intrinsic::sysroot::PackageId;
use middle::queries::LunaDatabase as _; // bring query methods into scope
use middle::{Db, HirPackageBox};
use vfs::Vfs;

// ── CompilerInstance ─────────────────────────────────────────────────────────

/// Source state for one compilation, bounded by its session lifetime.
///
/// The database owns HIR query inputs and caches. Query results own their HIR
/// arenas; the separate helper arena is borrowed through this instance.
pub struct CompilerInstance<'sess> {
    /// The long-lived compiler session.
    pub sess: &'sess Session,
    /// Diagnostic context (errors, warnings, suggestions).
    pub diag_ctx: Arc<DiagnosticContext<'sess>>,
    /// Virtual file system for the **user** package.
    pub vfs: Vfs,
    /// VFS instances for sysroot packages (index 0 = builtin, 1 = std).
    pub sysroot_vfs: Vec<Vfs>,
    /// The central query database.
    ///
    /// Stores inputs and memoised HIR results with dependency invalidation.
    pub db: Db<'sess>,
    /// HIR arena – backing memory for all `&'hir` HIR nodes.
    ///
    /// Owned here so the `'hir` lifetime equals the borrow lifetime `'c`
    /// of the [`Compiler`] handle.
    pub hir_arena: HirArena,
    /// Intrinsic / lang-item context (built-ins, primitive ops, etc.).
    pub intrinsic_ctx: IntrinsicContext,
}

impl<'sess> CompilerInstance<'sess> {
    /// Create a new compiler instance tied to the given session.
    pub fn new(sess: &'sess Session) -> Self {
        let db = Db::new();
        let intrinsic_ctx = intrinsic::initialize();
        let sysroot_vfs = Self::load_sysroot(sess);

        CompilerInstance {
            diag_ctx: Arc::new(DiagnosticContext::new(&sess.source_map)),
            vfs: Vfs::new(&sess.config.name, sess.config.root.clone()),
            sysroot_vfs,
            db,
            hir_arena: HirArena::new(),
            intrinsic_ctx,
            sess,
        }
    }

    /// Scan sysroot packages into VFS instances (dependency order).
    fn load_sysroot(sess: &Session) -> Vec<Vfs> {
        let Some(ref sysroot) = sess.sysroot else {
            return Vec::new();
        };
        let ignores: Vec<&str> = sess.config.ignores.iter().map(|s| s.as_str()).collect();
        sysroot
            .packages()
            .map(|pkg| Vfs::scan(pkg.source_root.clone(), &sess.source_map, &ignores))
            .collect()
    }

    /// Get the VFS for a sysroot package by [`PackageId`].
    pub fn sysroot_package(&self, id: PackageId) -> Option<&Vfs> {
        if id.is_sysroot() {
            self.sysroot_vfs.get(id.index())
        } else {
            None
        }
    }

    /// Get the `builtin` sysroot VFS (if loaded).
    pub fn builtin_vfs(&self) -> Option<&Vfs> {
        self.sysroot_package(PackageId::BUILTIN)
    }

    /// Get the `std` sysroot VFS (if loaded).
    pub fn std_vfs(&self) -> Option<&Vfs> {
        self.sysroot_package(PackageId::STD)
    }

    /// Enter a read-only scope via a [`Compiler`] handle.
    pub fn enter<F, R>(&self, f: F) -> R
    where
        F: FnOnce(Compiler<'_, 'sess>) -> R,
    {
        f(Compiler { instance: self })
    }

    /// Obtain a [`Compiler`] handle without the closure indirection.
    pub fn compiler(&self) -> Compiler<'_, 'sess> {
        Compiler { instance: self }
    }

    /// Mutable access to the VFS (for adding files, storing ASTs, etc.).
    pub fn vfs_mut(&mut self) -> &mut Vfs {
        &mut self.vfs
    }

    /// Store the query inputs for the `hir_package` query.
    ///
    /// Must be called after name resolution and before `Compiler::hir_package()`.
    /// Also ensure providers are registered via
    /// `ast_lowering::set_providers(&mut instance.db.providers)`.
    pub fn set_hir_input(&self, input: Arc<middle::HirQueryInput<'sess>>) {
        self.db.set_hir_input(input);
    }
}

// ── Compiler handle ──────────────────────────────────────────────────────────

/// Borrowed access to the compilation's source state and HIR queries.
///
/// References obtained from the helper arena remain bounded by the instance
/// borrow; owned HIR query results retain their own allocation owner.
#[derive(Clone, Copy)]
pub struct Compiler<'c, 'sess> {
    instance: &'c CompilerInstance<'sess>,
}

impl<'c, 'sess> Compiler<'c, 'sess> {
    // ── HIR queries ────────────────────────────────────────────────────────────

    /// Lower the package's AST to HIR (memoised via the query engine).
    ///
    /// Dispatches to the provider registered via
    /// `ast_lowering::set_providers`.  Calling this before registering a
    /// provider will panic.
    pub fn hir_package(self) -> Arc<HirPackageBox> {
        self.db.hir_package(())
    }
}

impl<'c, 'sess> Deref for Compiler<'c, 'sess> {
    type Target = CompilerInstance<'sess>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.instance
    }
}
