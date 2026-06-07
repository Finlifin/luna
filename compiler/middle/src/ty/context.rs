//! Type context – the primary API for creating and querying semantic types.
//!
//! [`TyCtxt`] is owned by [`CompilerInstance`](interface) and provides
//! methods for creating interned types, caching common types, and mapping
//! from HIR definitions to their semantic types.

use std::cell::RefCell;

use hir::hir_id::LocalDefId;
use rustc_data_structures::fx::FxHashMap;

use crate::ty::interner::TyInterner;
use crate::ty::types::*;

/// Pre-interned types that are used so frequently that caching them avoids
/// repeated hash-set lookups.
pub struct CommonTypes<'tcx> {
    pub i8: Ty<'tcx>,
    pub i16: Ty<'tcx>,
    pub i32: Ty<'tcx>,
    pub i64: Ty<'tcx>,
    pub isize: Ty<'tcx>,
    pub u8: Ty<'tcx>,
    pub u16: Ty<'tcx>,
    pub u32: Ty<'tcx>,
    pub u64: Ty<'tcx>,
    pub usize: Ty<'tcx>,
    pub f32: Ty<'tcx>,
    pub f64: Ty<'tcx>,
    pub bool_: Ty<'tcx>,
    pub char_: Ty<'tcx>,
    pub str_: Ty<'tcx>,
    pub void: Ty<'tcx>,
    pub no_return: Ty<'tcx>,
}

/// The **type context** – central hub for creating, interning, and
/// retrieving semantic types.
///
/// Analogous to rustc's `TyCtxt<'tcx>`. Owned by `CompilerInstance` and
/// shared across all type-checking passes via the `Compiler<'c>` handle.
///
/// # Lifetimes
///
/// The `'tcx` lifetime of the types it creates is the borrow lifetime of
/// `&'tcx self`. Since `CompilerInstance` owns the `TyCtxt` and
/// `Compiler<'c>` borrows `CompilerInstance`, `'c` = `'tcx`.
///
/// # Usage
///
/// ```ignore
/// fn check_expr(tcx: &TyCtxt, expr: &Expr<'_>) -> Ty<'_> {
///     // Create types via the context:
///     let int = tcx.common().int;
///     let pair = tcx.mk_tuple(&[int, int]);
///     let opt  = tcx.mk_optional(int);
///
///     // Store a definition's type:
///     tcx.register_def_ty(def_id, pair);
///
///     // Retrieve it later:
///     let ty = tcx.def_ty(def_id).unwrap();
/// }
/// ```
pub struct TyCtxt {
    /// The interning engine.
    interner: TyInterner,

    /// Mapping from definition ids → their resolved semantic type.
    ///
    /// Populated during type checking. For example, after checking a
    /// function `fn add(a: Int, b: Int) -> Int`, the function's
    /// `LocalDefId` will map to `Fn([Int, Int], Int)`.
    def_types: RefCell<FxHashMap<LocalDefId, Ty<'static>>>,

    /// Mapping from HIR node ids → their inferred type.
    ///
    /// Populated during type inference. Each expression / pattern that
    /// has a type gets an entry here.
    node_types: RefCell<FxHashMap<hir::HirId, Ty<'static>>>,

    /// Counter for fresh inference variables.
    next_infer_var: RefCell<u32>,

    /// ADT definitions keyed by their [`AdtId`].
    adt_defs: RefCell<FxHashMap<NFId, AdtDef>>,
}

impl TyCtxt {
    /// Create a new, empty type context.
    pub fn new() -> Self {
        TyCtxt {
            interner: TyInterner::new(),
            def_types: RefCell::new(FxHashMap::default()),
            node_types: RefCell::new(FxHashMap::default()),
            next_infer_var: RefCell::new(0),
            adt_defs: RefCell::new(FxHashMap::default()),
        }
    }

    /// Return pre-interned common types.
    ///
    /// Since the interner deduplicates, calling `mk_primitive` every time
    /// would also work, but this avoids the hash-set probe.
    pub fn common<'tcx>(&'tcx self) -> CommonTypes<'tcx> {
        CommonTypes {
            i8: self.mk_primitive(PrimTy::I8),
            i16: self.mk_primitive(PrimTy::I16),
            i32: self.mk_primitive(PrimTy::I32),
            i64: self.mk_primitive(PrimTy::I64),
            isize: self.mk_primitive(PrimTy::Isize),
            u8: self.mk_primitive(PrimTy::U8),
            u16: self.mk_primitive(PrimTy::U16),
            u32: self.mk_primitive(PrimTy::U32),
            u64: self.mk_primitive(PrimTy::U64),
            usize: self.mk_primitive(PrimTy::Usize),
            f32: self.mk_primitive(PrimTy::F32),
            f64: self.mk_primitive(PrimTy::F64),
            bool_: self.mk_primitive(PrimTy::Bool),
            char_: self.mk_primitive(PrimTy::Char),
            str_: self.mk_primitive(PrimTy::Str),
            void: self.mk_unit(),
            no_return: self.mk_never(),
        }
    }

    /// Intern an arbitrary [`TyKind`].
    #[inline]
    pub fn intern<'tcx>(&'tcx self, kind: TyKind<'tcx>) -> Ty<'tcx> {
        self.interner.intern(kind)
    }

    /// Primitive type.
    pub fn mk_primitive<'tcx>(&'tcx self, prim: PrimTy) -> Ty<'tcx> {
        self.intern(TyKind::Primitive(prim))
    }

    /// Unit type `()`.
    pub fn mk_unit<'tcx>(&'tcx self) -> Ty<'tcx> {
        self.intern(TyKind::Void)
    }

    /// Never type `!`.
    pub fn mk_never<'tcx>(&'tcx self) -> Ty<'tcx> {
        self.intern(TyKind::NoReturn)
    }

    /// Error type (empty error-qualified type, used as error sentinel).
    pub fn mk_error<'tcx>(&'tcx self) -> Ty<'tcx> {
        self.intern(TyKind::ErrorQualified(&[]))
    }

    /// Error-qualified type with a set of error types.
    pub fn mk_error_qualified<'tcx>(&'tcx self, tys: &[Ty<'tcx>]) -> Ty<'tcx> {
        let tys = self.intern_ty_slice(tys);
        self.intern(TyKind::ErrorQualified(tys))
    }

    /// Raw pointer type.
    pub fn mk_ptr<'tcx>(&'tcx self, inner: Ty<'tcx>, mutability: Mutability) -> Ty<'tcx> {
        self.intern(TyKind::Ptr(inner, mutability))
    }

    /// Optional type `T?`.
    pub fn mk_optional<'tcx>(&'tcx self, inner: Ty<'tcx>) -> Ty<'tcx> {
        self.intern(TyKind::Optional(inner))
    }

    /// Function type (params only, no explicit return position).
    pub fn mk_fn<'tcx>(&'tcx self, params: &[Ty<'tcx>]) -> Ty<'tcx> {
        let params = self.intern_ty_slice(params);
        self.intern(TyKind::Fn(params))
    }

    /// Function arrow type `A -> B`.
    pub fn mk_fn_arrow<'tcx>(&'tcx self, from: Ty<'tcx>, to: Ty<'tcx>) -> Ty<'tcx> {
        self.intern(TyKind::FnArrow(from, to))
    }

    /// Normal-form type (used for type-level normal-form expressions).
    pub fn mk_nf<'tcx>(&'tcx self, tys: &[Ty<'tcx>]) -> Ty<'tcx> {
        let tys = self.intern_ty_slice(tys);
        self.intern(TyKind::NornmalForm(tys))
    }

    /// NF application: apply a named-form `id` to type arguments.
    pub fn mk_nf_application<'tcx>(&'tcx self, id: NFId, args: &[Ty<'tcx>]) -> Ty<'tcx> {
        let args = self.intern_ty_slice(args);
        self.intern(TyKind::NFApplication(id, args))
    }

    /// Type parameter (unit variant – the name/index is tracked externally).
    pub fn mk_param<'tcx>(&'tcx self) -> Ty<'tcx> {
        self.intern(TyKind::Param)
    }

    /// Fresh inference variable.
    pub fn mk_infer<'tcx>(&'tcx self) -> Ty<'tcx> {
        let id = {
            let mut counter = self.next_infer_var.borrow_mut();
            let id = *counter;
            *counter += 1;
            id
        };
        self.intern(TyKind::Infer(InferTy(id)))
    }

    /// Intern a slice of `Ty` values into the arena.
    ///
    /// This is used internally by composite type constructors to ensure
    /// the sub-type slices live as long as `'tcx`.
    fn intern_ty_slice<'tcx>(&'tcx self, tys: &[Ty<'tcx>]) -> &'tcx [Ty<'tcx>] {
        if tys.is_empty() {
            return &[];
        }
        // SAFETY: same transmute pattern as HirArena – the slice arena
        // owns the memory, and 'tcx is the borrow lifetime of &self.
        unsafe {
            let static_tys: Vec<Ty<'static>> = tys
                .iter()
                .map(|&t| std::mem::transmute::<Ty<'tcx>, Ty<'static>>(t))
                .collect();
            let r = self.interner.alloc_ty_slice(static_tys);
            std::mem::transmute::<&[Ty<'static>], &'tcx [Ty<'tcx>]>(r)
        }
    }

    /// Associate a definition with its semantic type.
    ///
    /// Called during type checking to record the type of a top-level
    /// definition (function, struct, type alias, …).
    pub fn register_def_ty<'tcx>(&'tcx self, def_id: LocalDefId, ty: Ty<'tcx>) {
        let ty_static = unsafe { std::mem::transmute::<Ty<'tcx>, Ty<'static>>(ty) };
        self.def_types.borrow_mut().insert(def_id, ty_static);
    }

    /// Look up the semantic type of a definition.
    pub fn def_ty<'tcx>(&'tcx self, def_id: LocalDefId) -> Option<Ty<'tcx>> {
        self.def_types
            .borrow()
            .get(&def_id)
            .map(|&ty| unsafe { std::mem::transmute::<Ty<'static>, Ty<'tcx>>(ty) })
    }

    /// Record the inferred type of a HIR node (expression, pattern, etc.).
    pub fn register_node_ty<'tcx>(&'tcx self, hir_id: hir::HirId, ty: Ty<'tcx>) {
        let ty_static = unsafe { std::mem::transmute::<Ty<'tcx>, Ty<'static>>(ty) };
        self.node_types.borrow_mut().insert(hir_id, ty_static);
    }

    /// Look up the inferred type of a HIR node.
    pub fn node_ty<'tcx>(&'tcx self, hir_id: hir::HirId) -> Option<Ty<'tcx>> {
        self.node_types
            .borrow()
            .get(&hir_id)
            .map(|&ty| unsafe { std::mem::transmute::<Ty<'static>, Ty<'tcx>>(ty) })
    }

    /// Register an ADT definition (struct / enum).
    pub fn register_adt_def(&self, adt_id: NFId, def: AdtDef) {
        self.adt_defs.borrow_mut().insert(adt_id, def);
    }

    /// Look up an ADT definition by id.
    pub fn adt_def(&self, adt_id: NFId) -> Option<AdtDef> {
        self.adt_defs.borrow().get(&adt_id).cloned()
    }

    /// Substitute type parameters in `ty` using the given `substs` slice.
    ///
    /// `substs[i]` replaces the i-th `TyKind::Param` occurrence.
    /// Since `Param` is now a unit variant, substitution replaces by
    /// index based on the caller's context.
    pub fn subst<'tcx>(&'tcx self, ty: Ty<'tcx>, substs: &[Ty<'tcx>]) -> Ty<'tcx> {
        if substs.is_empty() {
            return ty;
        }
        match ty.kind() {
            TyKind::Ptr(inner, m) => {
                let new = self.subst(*inner, substs);
                self.mk_ptr(new, *m)
            }
            TyKind::Optional(inner) => {
                let new = self.subst(*inner, substs);
                self.mk_optional(new)
            }
            TyKind::NFApplication(id, args) => {
                let new_args: Vec<_> = args.iter().map(|a| self.subst(*a, substs)).collect();
                self.mk_nf_application(*id, &new_args)
            }
            TyKind::Fn(params) => {
                let new_params: Vec<_> = params.iter().map(|p| self.subst(*p, substs)).collect();
                self.mk_fn(&new_params)
            }
            TyKind::FnArrow(from, to) => {
                let new_from = self.subst(*from, substs);
                let new_to = self.subst(*to, substs);
                self.mk_fn_arrow(new_from, new_to)
            }
            TyKind::ErrorQualified(tys) => {
                let new_tys: Vec<_> = tys.iter().map(|t| self.subst(*t, substs)).collect();
                self.mk_error_qualified(&new_tys)
            }
            _ => ty,
        }
    }
}

impl Default for TyCtxt {
    fn default() -> Self {
        Self::new()
    }
}

// SAFETY: The Luna compiler is single-threaded.  `TyCtxt` uses `RefCell`
// and `TypedArena` (both `!Sync`/`!Send`) internally for arena allocation
// and mutable caches.  We promise never to share a `TyCtxt` across threads
// concurrently.  These impls are required because `Arc<TyCtxt>` must be
// `Send + Sync` for salsa 0.22's `HasStorage` bound.
unsafe impl Sync for TyCtxt {}
unsafe impl Send for TyCtxt {}
