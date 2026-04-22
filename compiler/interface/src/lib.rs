//! Compiler interface – the "middle" crate of the Luna compiler.
//!
//! This crate defines the central types that **every compiler pass** needs
//! access to. It sits between the low-level infrastructure crates (`ast`,
//! `vfs`, `query`, `diagnostic`) and the high-level passes (type checking,
//! lowering, codegen, …).
//!
//! # Crate Dependency Graph
//!
//! ```text
//!   ast, lex, diagnostic, vfs, query        ← foundational crates
//!                  ↑
//!             interface                      ← THIS CRATE (Session, Compiler, …)
//!                  ↑
//!        typeck, lowering, …                 ← analysis / transform passes
//!                  ↑
//!               luna                         ← binary driver (main)
//! ```
//!
//! Any crate that needs `Compiler<'c>` as a parameter depends on
//! `interface`, **not** on `luna`.
//!
//! # Architecture
//!
//! ```text
//!   Session              (owns: config, source_map, sysroot)
//!       │
//!       ▼
//!   CompilerInstance<'s>  (owns: diag_ctx, vfs, sysroot_vfs[], query_engine;
//!                          borrows Session)
//!       │
//!       ▼
//!   Compiler<'c>          (thin Copy handle, Deref → CompilerInstance)
//! ```

mod session;

pub use session::{CompilerConfig, Session};

// Re-export key types from dependency crates so downstream passes only
// need to depend on `interface`.
pub use diagnostic;
pub use hir;
pub use intrinsic;
pub use query;
pub use ty;
pub use vfs;

use std::ops::Deref;

use diagnostic::DiagnosticContext;
use hir::HirArena;
use hir::hir_id::LocalDefId;
use intrinsic::IntrinsicContext;
use intrinsic::sysroot::PackageId;
use query::{QueryCache, QueryEngine};
use ty::{AdtDef, AdtId, TyCtxt};
use vfs::Vfs;

// ── Queries ──────────────────────────────────────────────────────────────────

/// Per-query-kind caches — one field per registered query.
pub struct Queries {
    /// `adt_def(AdtId) -> AdtDef` — look up an ADT definition.
    pub adt_def: QueryCache<AdtId, Option<AdtDef>>,
    /// `def_ty(LocalDefId) -> Option<Ty<'static>>` — look up a definition's
    /// semantic type. Wrapped in a serialisable form because `Ty` is a
    /// thin pointer. For now this is unused (types live in TyCtxt tables).
    pub def_ty: QueryCache<LocalDefId, ()>,
}

impl Queries {
    pub fn new() -> Self {
        Queries {
            adt_def: QueryCache::new(),
            def_ty: QueryCache::new(),
        }
    }
}

impl Default for Queries {
    fn default() -> Self {
        Self::new()
    }
}

// ── CompilerInstance ─────────────────────────────────────────────────────────

/// Global compiler context – owns all compilation-wide data.
///
/// Analogous to rustc's `GlobalCtxt`. Created once per compilation and lives
/// as long as the [`Session`] it borrows from.
///
/// The [`HirArena`] is owned here, giving it the same lifetime as the
/// instance. Every `&'hir` reference in HIR types points into this arena.
/// The `'hir` lifetime is obtained by borrowing the `CompilerInstance`
/// (i.e. `'hir` = the borrow lifetime of `&CompilerInstance`).
pub struct CompilerInstance<'sess> {
    /// Reference to the long-lived session.
    pub sess: &'sess Session,
    /// Diagnostic context (error / warning reporting).
    pub diag_ctx: DiagnosticContext<'sess>,
    /// Virtual file system for the **user** package.
    pub vfs: Vfs,
    /// VFS for sysroot packages, keyed by [`PackageId`].
    /// Index 0 = builtin, index 1 = std.
    pub sysroot_vfs: Vec<Vfs>,
    /// The demand-driven query engine (memoization + cycle detection).
    pub query_engine: QueryEngine,
    /// Per-query-kind caches.
    pub queries: Queries,
    /// The HIR arena – backing memory for all `&'hir` HIR nodes.
    pub hir_arena: HirArena,
    /// The type context – interning arena and type tables for semantic types.
    pub ty_ctxt: TyCtxt,
    /// Intrinsic / built-in context (lang items, built-in functions, etc.).
    pub intrinsic_ctx: IntrinsicContext,
}

impl<'sess> CompilerInstance<'sess> {
    /// Create a new compiler instance tied to the given session.
    ///
    /// An empty [`Vfs`] is created using the project name and root from
    /// the session's config. If a sysroot is available, its packages are
    /// scanned into separate VFS instances.
    pub fn new(sess: &'sess Session) -> Self {
        let ty_ctxt = TyCtxt::new();
        let intrinsic_ctx = intrinsic::initialize(&ty_ctxt);

        // ── Load sysroot packages ────────────────────────────────────────
        let sysroot_vfs = Self::load_sysroot(sess);

        CompilerInstance {
            diag_ctx: DiagnosticContext::new(&sess.source_map),
            vfs: Vfs::new(&sess.config.name, sess.config.root.clone()),
            sysroot_vfs,
            query_engine: QueryEngine::new(),
            queries: Queries::new(),
            hir_arena: HirArena::new(),
            ty_ctxt,
            intrinsic_ctx,
            sess,
        }
    }

    /// Scan sysroot packages into VFS instances (in dependency order).
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
    ///
    /// Returns `None` if it is not a sysroot package or the sysroot
    /// was not loaded.
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
    ///
    /// This is the primary way to hand the compiler context to analysis
    /// passes. The closure receives a cheap, `Copy` handle that provides
    /// read access to everything.
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

    // ── Mutable accessors (used during construction / mutation phases) ────

    /// Mutable access to the VFS (for adding files, storing ASTs, etc.).
    pub fn vfs_mut(&mut self) -> &mut Vfs {
        &mut self.vfs
    }
}

// ── Compiler ─────────────────────────────────────────────────────────────────

/// Thin, `Copy` handle into the compiler – passed to every compiler pass.
///
/// Analogous to rustc's `TyCtxt<'tcx>`. Provides **read-only** access to
/// all compilation-wide data through the underlying [`CompilerInstance`].
///
/// The lifetime `'c` also serves as the `'hir` lifetime for all HIR
/// references. Since `CompilerInstance` owns the [`HirArena`], any
/// `&'c` borrow of the instance can be used to allocate into the arena
/// and obtain `&'c T` (= `&'hir T`) references.
///
/// Implements [`Deref`] to [`CompilerInstance`], so all public fields
/// (`sess`, `diag_ctx`, `vfs`, `query_engine`, `hir_arena`, …) are
/// accessible directly:
///
/// ```ignore
/// fn some_pass(cx: Compiler<'_>) {
///     let sm = &cx.sess.source_map;  // via Deref
///     cx.diag_ctx.emit(...);         // via Deref
///     let e = cx.hir_arena.alloc_expr(...);  // arena allocation
/// }
/// ```
#[derive(Clone, Copy)]
pub struct Compiler<'c> {
    instance: &'c CompilerInstance<'c>,
}

impl<'c> Compiler<'c> {
    /// Query: look up an ADT definition by id (memoised).
    pub fn adt_def(self, adt_id: AdtId) -> Option<AdtDef> {
        self.query_engine.execute(
            &self.queries.adt_def,
            "adt_def",
            &adt_id,
            |k| format!("{:?}", k),
            |k| self.ty_ctxt.adt_def(*k),
        )
    }
}

impl<'c> Deref for Compiler<'c> {
    type Target = CompilerInstance<'c>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.instance
    }
}
