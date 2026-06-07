//! Compiler interface – the coordination layer of the Luna compiler.
//!
//! This crate defines the central types that **every compiler pass** needs:
//! [`Session`], [`CompilerInstance`], and the lightweight [`Compiler`]
//! handle.  It sits between the foundational crates and the analysis passes.
//!
//! # Crate Dependency Graph
//!
//! ```text
//!   symbol, ast, lex, diagnostic, vfs, query, middle   ← foundational
//!                          ↑
//!                      interface                        ← THIS CRATE
//!                          ↑
//!              typeck, mir_build, codegen, …            ← analysis passes
//!                          ↑
//!                        luna                           ← binary driver
//! ```
//!
//! # Key types
//!
//! ```text
//!   Session                   (config, source_map, sysroot)
//!       │
//!       ▼
//!   CompilerInstance<'sess>   (db, hir_arena, vfs, diag_ctx, …)
//!       │  db: middle::Db     ← salsa database + TyCtxt arena
//!       │
//!       ▼
//!   Compiler<'c>              (thin Copy handle, Deref → CompilerInstance)
//! ```
//!
//! The [`middle::Db`] owns both the salsa storage (query memoisation,
//! change tracking) and the [`middle::TyCtxt`] arena (type interning).
//! All previously separate [`QueryEngine`] / [`Queries`] / [`TyCtxt`]
//! fields are now unified in `db`.

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
use hir::hir_id::LocalDefId;
use intrinsic::IntrinsicContext;
use intrinsic::sysroot::PackageId;
use middle::queries::LunaDatabase as _;   // bring query methods into scope
use middle::{AdtDef, Db, HirPackageBox, NFId};
use vfs::Vfs;

// ── CompilerInstance ─────────────────────────────────────────────────────────

/// Global compiler context – owns all compilation-wide data.
///
/// Analogous to rustc's `GlobalCtxt`.  Created once per compilation and
/// lives for the entire duration of the session it borrows.
///
/// # Central store: `db`
///
/// [`middle::Db`] unifies:
/// - the salsa query engine (memoisation, cycle detection, dep-graph)
/// - the [`TyCtxt`](middle::TyCtxt) type-interning arena
///
/// Both are accessed through `self.db` or `self.db.ty_ctxt`.
///
/// # HIR arena
///
/// The [`HirArena`] is owned here so that `'hir = 'c` (the borrow
/// lifetime of [`Compiler`]).  Every `&'hir` HIR reference can be
/// obtained by borrowing `CompilerInstance`.
pub struct CompilerInstance<'sess> {
    /// The long-lived compiler session.
    pub sess: &'sess Session,
    /// Diagnostic context (errors, warnings, suggestions).
    pub diag_ctx: DiagnosticContext<'sess>,
    /// Virtual file system for the **user** package.
    pub vfs: Vfs,
    /// VFS instances for sysroot packages (index 0 = builtin, 1 = std).
    pub sysroot_vfs: Vec<Vfs>,
    /// The central salsa database.
    ///
    /// Provides:
    /// - memoised query results via salsa (call `self.db.adt_def(id)`, …)
    /// - the type-interning arena (`self.db.ty_ctxt`)
    pub db: Db,
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
        let intrinsic_ctx = intrinsic::initialize(&db.ty_ctxt);
        let sysroot_vfs = Self::load_sysroot(sess);

        CompilerInstance {
            diag_ctx: DiagnosticContext::new(&sess.source_map),
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
        if id.is_sysroot() { self.sysroot_vfs.get(id.index()) } else { None }
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
        F: FnOnce(Compiler<'_>) -> R,
    {
        f(Compiler { instance: self })
    }

    /// Obtain a [`Compiler`] handle without the closure indirection.
    pub fn compiler(&self) -> Compiler<'_> {
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
    pub fn set_hir_input(&self, input: Arc<middle::HirQueryInput>) {
        self.db.set_hir_input(input);
    }
}

// ── Compiler handle ──────────────────────────────────────────────────────────

/// Thin, `Copy` handle into the compiler – passed to every compiler pass.
///
/// Analogous to rustc's `TyCtxt<'tcx>`.  Provides **read-only** access to
/// all compilation-wide data through the underlying [`CompilerInstance`].
///
/// The lifetime `'c` also serves as `'hir`: since `CompilerInstance` owns
/// the [`HirArena`], any `&'c CompilerInstance` borrow can be used to
/// obtain `&'c T` (= `&'hir T`) HIR references.
///
/// ```ignore
/// fn some_pass(cx: Compiler<'_>) {
///     let adt = cx.adt_def(id);        // salsa query
///     let ty  = cx.db.ty_ctxt.def_ty(def_id);  // TyCtxt arena lookup
///     let e   = cx.hir_arena.alloc_expr(…);    // HIR allocation
/// }
/// ```
#[derive(Clone, Copy)]
pub struct Compiler<'c> {
    instance: &'c CompilerInstance<'c>,
}

impl<'c> Compiler<'c> {
    // ── Salsa-backed query methods ────────────────────────────────────────

    /// Look up an ADT definition by id (memoised by salsa).
    ///
    /// Returns `None` if no ADT has been registered for `id`.
    pub fn adt_def(self, id: NFId) -> Option<Arc<AdtDef>> {
        self.db.adt_def(id)
    }

    /// Check whether a semantic type has been recorded for `id`.
    pub fn def_ty_exists(self, id: LocalDefId) -> bool {
        self.db.def_ty_exists(id)
    }
    // ── HIR queries ────────────────────────────────────────────────────────────

    /// Lower the package's AST to HIR (memoised).
    ///
    /// Dispatches to the provider registered via
    /// [`CompilerInstance::set_hir_provider`].  Calling this before
    /// registering a provider will panic.
    pub fn hir_package(self) -> Arc<HirPackageBox> {
        self.db.hir_package(())
    }}

impl<'c> Deref for Compiler<'c> {
    type Target = CompilerInstance<'c>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.instance
    }
}
