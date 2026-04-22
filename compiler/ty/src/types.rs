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
///
/// Phase 1 focuses on concrete runtime types. Comptime types like
/// `Integer`, `Object`, `Float` are resolved to their default runtime
/// representations (e.g. `Integer` → `I64`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrimTy {
    // ── Signed integers ──────────────────────────────────────────────
    I8,
    I16,
    I32,
    I64,
    Isize,
    // ── Unsigned integers ────────────────────────────────────────────
    U8,
    U16,
    U32,
    U64,
    Usize,
    // ── Floating point ───────────────────────────────────────────────
    F32,
    F64,
    // ── Other scalars ────────────────────────────────────────────────
    Bool,
    Char,
    Str,
}

impl PrimTy {
    /// Returns `true` if this is a signed integer type.
    pub fn is_signed_int(self) -> bool {
        matches!(self, PrimTy::I8 | PrimTy::I16 | PrimTy::I32 | PrimTy::I64 | PrimTy::Isize)
    }

    /// Returns `true` if this is an unsigned integer type.
    pub fn is_unsigned_int(self) -> bool {
        matches!(self, PrimTy::U8 | PrimTy::U16 | PrimTy::U32 | PrimTy::U64 | PrimTy::Usize)
    }

    /// Returns `true` if this is any integer type.
    pub fn is_int(self) -> bool {
        self.is_signed_int() || self.is_unsigned_int()
    }

    /// Returns `true` if this is a floating point type.
    pub fn is_float(self) -> bool {
        matches!(self, PrimTy::F32 | PrimTy::F64)
    }

    /// Returns `true` if this is a numeric type.
    pub fn is_numeric(self) -> bool {
        self.is_int() || self.is_float()
    }
}

impl fmt::Display for PrimTy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PrimTy::I8 => "i8",
            PrimTy::I16 => "i16",
            PrimTy::I32 => "i32",
            PrimTy::I64 => "i64",
            PrimTy::Isize => "isize",
            PrimTy::U8 => "u8",
            PrimTy::U16 => "u16",
            PrimTy::U32 => "u32",
            PrimTy::U64 => "u64",
            PrimTy::Usize => "usize",
            PrimTy::F32 => "f32",
            PrimTy::F64 => "f64",
            PrimTy::Bool => "bool",
            PrimTy::Char => "char",
            PrimTy::Str => "str",
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

// ── ADT definition ──────────────────────────────────────────────────────────

/// Describes a user-defined algebraic data type (struct / enum).
///
/// Stored in [`TyCtxt`] keyed by [`AdtId`]. Contains the information
/// needed for type checking field access and struct literal construction.
#[derive(Debug, Clone)]
pub struct AdtDef {
    /// The name of the type (for diagnostics / codegen).
    pub name: String,
    /// The kind of ADT (struct vs enum).
    pub kind: AdtKind,
    /// The fields, in declaration order.
    pub fields: Vec<FieldDef>,
    /// Type parameter names, in declaration order (e.g. `["T", "U"]`).
    pub type_params: Vec<String>,
}

/// Whether an ADT is a struct or an enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdtKind {
    Struct,
    Enum,
}

/// A field within a struct / enum variant.
#[derive(Debug, Clone)]
pub struct FieldDef {
    /// The field name.
    pub name: String,
    /// The index of this field in declaration order.
    pub index: u32,
    /// The type of this field (stored as `'static`; use with `TyCtxt`).
    pub ty: Ty<'static>,
}

impl AdtDef {
    /// Look up a field by name. Returns its index if found.
    pub fn field_index(&self, name: &str) -> Option<u32> {
        self.fields.iter().find(|f| f.name == name).map(|f| f.index)
    }
}
