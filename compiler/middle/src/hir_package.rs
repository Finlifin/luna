//! An owned HIR package whose arena is constructed in a generative scope.
//! The package can only borrow that arena or static data. Its references are
//! exposed with the owner's borrow lifetime, never the erased storage lifetime.
use hir::{HirArena, Package};

pub struct HirPackageBox {
    // Drop the package before its allocation owner.
    package: Package<'static>,
    arena: Box<HirArena>,
}

impl HirPackageBox {
    /// Build a package inside its allocation owner's scope.
    ///
    /// The higher-ranked callback prevents a package borrowing an unrelated
    /// arena from being smuggled into the owned result.
    ///
    /// ```compile_fail
    /// fn detach<'hir>(foreign: hir::Package<'hir>) -> middle::HirPackageBox {
    ///     middle::HirPackageBox::new(|_| foreign)
    /// }
    /// ```
    ///
    /// Shared allocation requires a thread-safe arena; moving an unaliased
    /// owner is governed separately by its natural auto traits.
    ///
    /// ```compile_fail
    /// fn share(package: std::sync::Arc<middle::HirPackageBox>) {
    ///     std::thread::spawn(move || drop(package));
    /// }
    /// ```
    pub fn new(build: impl for<'hir> FnOnce(&'hir HirArena) -> Package<'hir>) -> Self {
        let arena = Box::new(HirArena::new());
        let package = build(&arena);
        // SAFETY: the callback must work for any arena borrow lifetime. The
        // returned package can therefore borrow only this arena or static data.
        // The boxed arena stays at its address and outlives the package. The
        // accessor shortens all exposed references to the owner's borrow.
        let package = unsafe { std::mem::transmute::<Package<'_>, Package<'static>>(package) };
        Self { package, arena }
    }

    pub fn package(&self) -> &Package<'_> {
        &self.package
    }

    pub fn arena(&self) -> &HirArena {
        &self.arena
    }
}
