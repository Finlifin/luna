//! Semantic type definitions.
//!
//! These types represent the **compiler's internal understanding** of types
//! after name resolution, unlike the syntactic type expressions in
//! [`hir::expr::ExprKind`] which mirror what the programmer wrote.

use std::fmt;

use hir::common::Mutability;
use hir::hir_id::LocalDefId;

// ── Ty ───────────────────────────────────────────────────────────────────────

/// A semantic type – a thin, interned pointer into the [`TyCtxt`] arena.
///
/// Because types are interned, two `Ty` values are equal if and only if
/// they point to the same allocation. This makes equality checking O(1)
/// and allows `Ty` to be `Copy`.
///
/// The lifetime `'tcx` ties the pointer to the [`TyCtxt`](super::TyCtxt)
/// that created it.
#[derive(Clone, Copy, Eq)]
pub struct Ty<'tcx>(pub(crate) &'tcx TyKind<'tcx>);

impl<'tcx> Ty<'tcx> {
    /// The kind (payload) of this type.
    #[inline]
    pub fn kind(self) -> &'tcx TyKind<'tcx> {
        self.0
    }

    /// Returns `true` if this is one of the primitive scalar types.
    pub fn is_primitive(self) -> bool {
        matches!(self.kind(), TyKind::Primitive(_))
    }

    /// Returns `true` if this is the unit type `()`.
    pub fn is_unit(self) -> bool {
        matches!(self.kind(), TyKind::Unit)
    }

    /// Returns `true` if this is the never type `!`.
    pub fn is_never(self) -> bool {
        matches!(self.kind(), TyKind::Never)
    }

    /// Returns `true` if this is the error sentinel type.
    pub fn is_error(self) -> bool {
        matches!(self.kind(), TyKind::Error)
    }

    /// Returns `true` if this is an inference variable (not yet resolved).
    pub fn is_infer(self) -> bool {
        matches!(self.kind(), TyKind::Infer(_))
    }

    /// Returns `true` if this type contains any inference variables.
    pub fn has_infer_vars(self) -> bool {
        match self.kind() {
            TyKind::Infer(_) => true,
            TyKind::Ref(inner, _) | TyKind::Ptr(inner, _) | TyKind::Optional(inner) => {
                inner.has_infer_vars()
            }
            TyKind::Tuple(elems) => elems.iter().any(|t| t.has_infer_vars()),
            TyKind::Fn(params, ret) => {
                params.iter().any(|t| t.has_infer_vars()) || ret.has_infer_vars()
            }
            TyKind::Array(elem, _) | TyKind::Slice(elem) => elem.has_infer_vars(),
            TyKind::Adt(_, args) => args.iter().any(|t| t.has_infer_vars()),
            TyKind::Param(_)
            | TyKind::Primitive(_)
            | TyKind::Unit
            | TyKind::Never
            | TyKind::Error => false,
        }
    }
}

/// Pointer-based equality (interned types are structurally unique).
impl PartialEq for Ty<'_> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.0, other.0)
    }
}

impl std::hash::Hash for Ty<'_> {
    #[inline]
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (self.0 as *const TyKind<'_>).hash(state);
    }
}

impl fmt::Debug for Ty<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

impl fmt::Display for Ty<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ── TyKind ───────────────────────────────────────────────────────────────────

/// The payload of a semantic type.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TyKind<'tcx> {
    /// A primitive scalar type (`Int`, `Float`, `Bool`, `Char`, `String`).
    Primitive(PrimTy),

    /// The unit type `()`.
    Unit,

    /// A tuple type, e.g. `(Int, Bool)`.
    Tuple(&'tcx [Ty<'tcx>]),

    /// A reference `&T` or `&mut T`.
    Ref(Ty<'tcx>, Mutability),

    /// A raw pointer `*const T` or `*mut T`.
    Ptr(Ty<'tcx>, Mutability),

    /// An optional type `T?`.
    Optional(Ty<'tcx>),

    /// A function type `(A, B) -> C`.
    Fn(&'tcx [Ty<'tcx>], Ty<'tcx>),

    /// A fixed-size array `[T; N]`.
    Array(Ty<'tcx>, u64),

    /// A slice `[T]`.
    Slice(Ty<'tcx>),

    /// An algebraic data type (struct / enum), with its definition id and
    /// substituted generic arguments.
    Adt(AdtId, &'tcx [Ty<'tcx>]),

    /// A type parameter, e.g. `T` in `fn foo<T>(x: T)`.
    Param(ParamTy),

    /// A not-yet-resolved inference variable. Created during type
    /// inference and later unified to a concrete type.
    Infer(InferTy),

    /// The bottom type `!` (never returns).
    Never,

    /// Placeholder for type errors – allows compilation to continue after
    /// a type error is reported.
    Error,
}

impl fmt::Display for TyKind<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TyKind::Primitive(p) => write!(f, "{p}"),
            TyKind::Unit => write!(f, "()"),
            TyKind::Tuple(elems) => {
                write!(f, "(")?;
                for (i, e) in elems.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{e}")?;
                }
                write!(f, ")")
            }
            TyKind::Ref(inner, Mutability::Immutable) => write!(f, "&{inner}"),
            TyKind::Ref(inner, Mutability::Mutable) => write!(f, "&mut {inner}"),
            TyKind::Ptr(inner, Mutability::Immutable) => write!(f, "*const {inner}"),
            TyKind::Ptr(inner, Mutability::Mutable) => write!(f, "*mut {inner}"),
            TyKind::Optional(inner) => write!(f, "{inner}?"),
            TyKind::Fn(params, ret) => {
                write!(f, "fn(")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{p}")?;
                }
                write!(f, ") -> {ret}")
            }
            TyKind::Array(elem, len) => write!(f, "[{elem}; {len}]"),
            TyKind::Slice(elem) => write!(f, "[{elem}]"),
            TyKind::Adt(id, args) => {
                write!(f, "Adt({:?}", id.0)?;
                if !args.is_empty() {
                    write!(f, "<")?;
                    for (i, a) in args.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{a}")?;
                    }
                    write!(f, ">")?;
                }
                write!(f, ")")
            }
            TyKind::Param(p) => write!(f, "{}", p.name),
            TyKind::Infer(iv) => write!(f, "?{}", iv.0),
            TyKind::Never => write!(f, "!"),
            TyKind::Error => write!(f, "<error>"),
        }
    }
}

// ── Primitive types ──────────────────────────────────────────────────────────

/// Built-in primitive types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrimTy {
    Int,
    Float,
    Bool,
    Char,
    Str,
}

impl fmt::Display for PrimTy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PrimTy::Int => "Int",
            PrimTy::Float => "Float",
            PrimTy::Bool => "Bool",
            PrimTy::Char => "Char",
            PrimTy::Str => "String",
        })
    }
}

// ── ADT id ───────────────────────────────────────────────────────────────────

/// Identity of an algebraic data type (struct or enum).
///
/// Wraps a [`LocalDefId`] pointing at the definition in the package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AdtId(pub LocalDefId);

// ── Type parameters ──────────────────────────────────────────────────────────

/// A type parameter, e.g. `T` in `fn id<T>(x: T) -> T`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ParamTy {
    /// De Bruijn-style index identifying this parameter among the
    /// enclosing generic binders.
    pub index: u32,
    /// The user-facing name (for diagnostics / pretty-printing).
    pub name: String,
}

// ── Inference variables ──────────────────────────────────────────────────────

/// An unresolved inference variable.
///
/// Created by the type inference engine. The `u32` is a unique id within
/// the current inference context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InferTy(pub u32);
