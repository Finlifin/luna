//! Type context – the primary API for creating and querying semantic types.
//!
//! [`TyCtxt`] is owned by [`CompilerInstance`](interface) and provides
//! methods for creating interned types, caching common types, and mapping
//! from HIR definitions to their semantic types.

use std::cell::RefCell;

use hir::hir_id::LocalDefId;
use rustc_data_structures::fx::FxHashMap;

use crate::interner::TyInterner;
use crate::types::*;

// ── CommonTypes ──────────────────────────────────────────────────────────────

/// Pre-interned types that are used so frequently that caching them avoids
/// repeated hash-set lookups.
pub struct CommonTypes<'tcx> {
    pub int: Ty<'tcx>,
    pub float: Ty<'tcx>,
    pub bool_: Ty<'tcx>,
    pub char_: Ty<'tcx>,
    pub str_: Ty<'tcx>,
    pub unit: Ty<'tcx>,
    pub never: Ty<'tcx>,
    pub error: Ty<'tcx>,
}

// ── TyCtxt ───────────────────────────────────────────────────────────────────

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
}

impl TyCtxt {
    /// Create a new, empty type context.
    pub fn new() -> Self {
        TyCtxt {
            interner: TyInterner::new(),
            def_types: RefCell::new(FxHashMap::default()),
            node_types: RefCell::new(FxHashMap::default()),
            next_infer_var: RefCell::new(0),
        }
    }

    // ── Common types ─────────────────────────────────────────────────────

    /// Return pre-interned common types.
    ///
    /// Since the interner deduplicates, calling `mk_primitive` every time
    /// would also work, but this avoids the hash-set probe.
    pub fn common<'tcx>(&'tcx self) -> CommonTypes<'tcx> {
        CommonTypes {
            int: self.mk_primitive(PrimTy::Int),
            float: self.mk_primitive(PrimTy::Float),
            bool_: self.mk_primitive(PrimTy::Bool),
            char_: self.mk_primitive(PrimTy::Char),
            str_: self.mk_primitive(PrimTy::Str),
            unit: self.mk_unit(),
            never: self.mk_never(),
            error: self.mk_error(),
        }
    }

    // ── Type constructors ────────────────────────────────────────────────

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
        self.intern(TyKind::Unit)
    }

    /// Never type `!`.
    pub fn mk_never<'tcx>(&'tcx self) -> Ty<'tcx> {
        self.intern(TyKind::Never)
    }

    /// Error type (sentinel after a type error).
    pub fn mk_error<'tcx>(&'tcx self) -> Ty<'tcx> {
        self.intern(TyKind::Error)
    }

    /// Tuple type, e.g. `(Int, Bool)`.
    pub fn mk_tuple<'tcx>(&'tcx self, elems: &[Ty<'tcx>]) -> Ty<'tcx> {
        let elems = self.intern_ty_slice(elems);
        self.intern(TyKind::Tuple(elems))
    }

    /// Reference type `&T` or `&mut T`.
    pub fn mk_ref<'tcx>(
        &'tcx self,
        inner: Ty<'tcx>,
        mutability: hir::Mutability,
    ) -> Ty<'tcx> {
        self.intern(TyKind::Ref(inner, mutability))
    }

    /// Raw pointer type.
    pub fn mk_ptr<'tcx>(
        &'tcx self,
        inner: Ty<'tcx>,
        mutability: hir::Mutability,
    ) -> Ty<'tcx> {
        self.intern(TyKind::Ptr(inner, mutability))
    }

    /// Optional type `T?`.
    pub fn mk_optional<'tcx>(&'tcx self, inner: Ty<'tcx>) -> Ty<'tcx> {
        self.intern(TyKind::Optional(inner))
    }

    /// Function type `(A, B) -> C`.
    pub fn mk_fn<'tcx>(&'tcx self, params: &[Ty<'tcx>], ret: Ty<'tcx>) -> Ty<'tcx> {
        let params = self.intern_ty_slice(params);
        self.intern(TyKind::Fn(params, ret))
    }

    /// Fixed-size array `[T; N]`.
    pub fn mk_array<'tcx>(&'tcx self, elem: Ty<'tcx>, len: u64) -> Ty<'tcx> {
        self.intern(TyKind::Array(elem, len))
    }

    /// Slice `[T]`.
    pub fn mk_slice<'tcx>(&'tcx self, elem: Ty<'tcx>) -> Ty<'tcx> {
        self.intern(TyKind::Slice(elem))
    }

    /// Algebraic data type (struct / enum) with generic substitutions.
    pub fn mk_adt<'tcx>(
        &'tcx self,
        adt_id: AdtId,
        args: &[Ty<'tcx>],
    ) -> Ty<'tcx> {
        let args = self.intern_ty_slice(args);
        self.intern(TyKind::Adt(adt_id, args))
    }

    /// Type parameter.
    pub fn mk_param<'tcx>(&'tcx self, index: u32, name: impl Into<String>) -> Ty<'tcx> {
        self.intern(TyKind::Param(ParamTy { index, name: name.into() }))
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

    // ── Slice interning ──────────────────────────────────────────────────

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

    // ── Definition type table ────────────────────────────────────────────

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
        self.def_types.borrow().get(&def_id).map(|&ty| unsafe {
            std::mem::transmute::<Ty<'static>, Ty<'tcx>>(ty)
        })
    }

    // ── Node type table ──────────────────────────────────────────────────

    /// Record the inferred type of a HIR node (expression, pattern, etc.).
    pub fn register_node_ty<'tcx>(&'tcx self, hir_id: hir::HirId, ty: Ty<'tcx>) {
        let ty_static = unsafe { std::mem::transmute::<Ty<'tcx>, Ty<'static>>(ty) };
        self.node_types.borrow_mut().insert(hir_id, ty_static);
    }

    /// Look up the inferred type of a HIR node.
    pub fn node_ty<'tcx>(&'tcx self, hir_id: hir::HirId) -> Option<Ty<'tcx>> {
        self.node_types.borrow().get(&hir_id).map(|&ty| unsafe {
            std::mem::transmute::<Ty<'static>, Ty<'tcx>>(ty)
        })
    }
}

impl Default for TyCtxt {
    fn default() -> Self {
        Self::new()
    }
}
