//! Source compilation sessions and mid-end query coordination for luna.
//!
//! [`CompilerInstance`] owns source inputs, diagnostics and the query database.
//! [`Compiler`] borrows that instance to expose provider-backed HIR, package
//! interface, per-item signature, trait/impl inventory, and elaboration queries.

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
    /// VFS instances for the sysroot packages loaded by [`Session::sysroot`]
    /// (ordered by `PackageId`: `builtin` and `std` via `Sysroot::packages()`;
    /// multi-package driver builds load all six builtin packages: `builtin`,
    /// `std`, `meta`, `build`, `attrs`, and `verification`).
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

    /// Store configuration inputs for the `elaborated_package` query.
    pub fn set_elaboration_input(&self, input: Arc<middle::ElaborationQueryInput>) {
        self.db.set_elaboration_input(input);
    }
}

// ── Compiler handle ──────────────────────────────────────────────────────────

/// Borrowed access to the compilation's source state and mid-end queries.
///
/// References obtained from the helper arena remain bounded by the instance
/// borrow; owned HIR and mid-end query results retain their own allocation
/// owner.
///
/// ```
/// use std::path::PathBuf;
/// use interface::{CompilerConfig, CompilerInstance, Session};
///
/// let sess = Session::new(CompilerConfig::new("demo", PathBuf::from(".")));
/// let instance = CompilerInstance::new(&sess);
/// instance.set_elaboration_input(Default::default());
/// assert!(instance.db.elaboration_input().is_some());
/// ```
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

    // ── Mid-end interface, signature & elaboration queries ─────────────────────

    /// Compute the lowered package's interface summary (memoised via `middle::Db`).
    pub fn package_interface(self) -> Arc<middle::PackageInterface> {
        self.db.package_interface(())
    }

    /// Look up the [`middle::ItemInterfaceSummary`] for `owner`.
    pub fn item_signature(self, owner: hir::OwnerId) -> Option<Arc<middle::ItemInterfaceSummary>> {
        self.db.item_signature(owner)
    }

    /// Cycle-safe variant of [`Compiler::item_signature`].
    pub fn try_item_signature(
        self,
        owner: hir::OwnerId,
    ) -> query::QueryResult<Option<Arc<middle::ItemInterfaceSummary>>> {
        self.db.try_item_signature(owner)
    }

    /// Compute the package's trait and implementation inventory.
    pub fn package_trait_impls(self) -> Arc<middle::PackageTraitImpls> {
        self.db.package_trait_impls(())
    }

    /// Evaluate and memoise mid-end package elaboration via the registered provider.
    pub fn elaborated_package(self) -> Arc<middle::ElaboratedPackage> {
        self.db.elaborated_package(())
    }

    /// Cycle-safe variant of [`Compiler::elaborated_package`].
    pub fn try_elaborated_package(self) -> query::QueryResult<Arc<middle::ElaboratedPackage>> {
        self.db.try_elaborated_package(())
    }
}

impl<'c, 'sess> Deref for Compiler<'c, 'sess> {
    type Target = CompilerInstance<'sess>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.instance
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn test_hir_box(name: &str) -> HirPackageBox {
        HirPackageBox::new(|arena| {
            let mut package = hir::Package::new();
            let root = package.alloc_owner_id();
            package.root_mod = root;
            let fn_owner = package.alloc_owner_id();
            let fn_item = arena.alloc_item(hir::Item {
                owner_id: fn_owner,
                ident: hir::common::Ident::new(
                    hir::common::Symbol::intern(name),
                    rustc_span::DUMMY_SP,
                ),
                span: rustc_span::DUMMY_SP,
                kind: hir::ItemKind::Fn(
                    hir::item::FnSig {
                        params: &[],
                        return_ty: None,
                        return_bind: None,
                        return_binding: None,
                        modifiers: Default::default(),
                        clause_params: &[],
                        clause_constraints: &[],
                        span: rustc_span::DUMMY_SP,
                    },
                    hir::BodyId::new(hir::HirId::make_owner(fn_owner)),
                ),
            });
            package.insert_owner(
                fn_owner,
                hir::owner::OwnerInfo {
                    node: hir::owner::OwnerNode::Item(fn_item),
                    nodes: hir::owner::OwnerNodes::new(),
                },
            );
            let root_item = arena.alloc_item(hir::Item {
                owner_id: root,
                ident: hir::common::Ident::new(
                    hir::common::Symbol::intern("root"),
                    rustc_span::DUMMY_SP,
                ),
                span: rustc_span::DUMMY_SP,
                kind: hir::ItemKind::Mod(hir::item::ModDef {
                    items: vec![fn_owner],
                }),
            });
            package.insert_owner(
                root,
                hir::owner::OwnerInfo {
                    node: hir::owner::OwnerNode::Item(root_item),
                    nodes: hir::owner::OwnerNodes::new(),
                },
            );
            package
        })
    }

    #[test]
    fn compiler_exposes_memoised_midend_queries_with_invalidation() {
        static HIR_CALLS: AtomicUsize = AtomicUsize::new(0);
        static ELAB_CALLS: AtomicUsize = AtomicUsize::new(0);
        fn hir_provider(db: &Db) -> Arc<HirPackageBox> {
            HIR_CALLS.fetch_add(1, Ordering::SeqCst);
            let raw = db.hir_input().unwrap().file_id().raw();
            Arc::new(test_hir_box(&format!("fn_{raw}")))
        }
        fn elab_provider(db: &Db) -> Arc<middle::ElaboratedPackage> {
            ELAB_CALLS.fetch_add(1, Ordering::SeqCst);
            let iface = db.package_interface(());
            let config = db.elaboration_input().unwrap_or_default();
            Arc::new(middle::ElaboratedPackage {
                succeeded: !config.package_identities.is_empty(),
                struct_count: 0,
                enum_count: 0,
                function_count: iface.len().saturating_sub(1),
                trait_count: 0,
                impl_count: 0,
                warnings: vec![],
                errors: vec![],
                nir_lisp: iface.to_string(),
            })
        }

        let sess = Session::new(CompilerConfig::new("pkg", PathBuf::from(".")));
        let mut instance = CompilerInstance::new(&sess);
        instance.db.providers.hir_package = hir_provider;
        instance.db.providers.elaborated_package = elab_provider;

        let make_input = |raw| {
            Arc::new(middle::HirQueryInput::new(
                Arc::new(middle::ast::Ast::new(rustc_span::BytePos(0))),
                vfs::FileId::from_raw(raw),
                Arc::new(middle::resolve::ModuleTree {
                    scope_tree: middle::resolve::scope::ScopeTree::new(),
                    def_names: Default::default(),
                    def_count: 0,
                    impls: vec![],
                    errors: vec![],
                    file_scopes: Default::default(),
                }),
                middle::resolve::ScopeId::ROOT,
                &sess.source_map,
                Arc::clone(&instance.diag_ctx),
            ))
        };

        instance.set_hir_input(make_input(1));
        instance.set_elaboration_input(Arc::new(middle::ElaborationQueryInput::new(
            BTreeMap::from([(0, 10)]),
        )));

        let (iface1, elab1) = instance.enter(|compiler| {
            let iface = compiler.package_interface();
            let sig = compiler
                .item_signature(hir::OwnerId::new(hir::LocalDefId::new(1)))
                .expect("fn signature");
            assert_eq!(sig.name.as_str(), "fn_1");
            let impls = compiler.package_trait_impls();
            assert!(impls.traits.is_empty());
            let elab = compiler.elaborated_package();
            assert!(elab.succeeded);
            assert!(Arc::ptr_eq(&iface, &compiler.package_interface()));
            assert!(Arc::ptr_eq(&elab, &compiler.elaborated_package()));
            (iface, elab)
        });
        assert_eq!(HIR_CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(ELAB_CALLS.load(Ordering::SeqCst), 1);

        // Updating elaboration input invalidates elaborated_package without re-running HIR.
        instance.set_elaboration_input(Arc::new(middle::ElaborationQueryInput::new(
            BTreeMap::new(),
        )));
        let elab2 = instance.enter(|compiler| {
            assert!(Arc::ptr_eq(&iface1, &compiler.package_interface()));
            compiler.elaborated_package()
        });
        assert!(!Arc::ptr_eq(&elab1, &elab2));
        assert!(!elab2.succeeded);
        assert_eq!(HIR_CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(ELAB_CALLS.load(Ordering::SeqCst), 2);

        // Updating HIR input transitively invalidates package_interface, item_signature, and elaborated_package.
        instance.set_hir_input(make_input(2));
        instance.enter(|compiler| {
            let iface2 = compiler.package_interface();
            assert!(!Arc::ptr_eq(&iface1, &iface2));
            let sig2 = compiler
                .item_signature(hir::OwnerId::new(hir::LocalDefId::new(1)))
                .expect("fn signature");
            assert_eq!(sig2.name.as_str(), "fn_2");
            let elab3 = compiler.elaborated_package();
            assert!(!Arc::ptr_eq(&elab2, &elab3));
        });
        assert_eq!(HIR_CALLS.load(Ordering::SeqCst), 2);
        assert_eq!(ELAB_CALLS.load(Ordering::SeqCst), 3);
    }
}
