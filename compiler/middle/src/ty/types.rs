//! Semantic type definitions.
//!
//! These types represent the **compiler's internal understanding** of types
//! after name resolution, unlike the syntactic type expressions in
//! [`hir::expr::ExprKind`] which mirror what the programmer wrote.

use std::fmt;

use hir::hir_id::LocalDefId;

/// Pointer/reference mutability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mutability {
    Immutable,
    Mutable,
}

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

    /// Returns `true` if this is the void type.
    pub fn is_void(self) -> bool {
        matches!(self.kind(), TyKind::Void)
    }

    /// Returns `true` if this is the never type `!`.
    pub fn is_never(self) -> bool {
        matches!(self.kind(), TyKind::NoReturn)
    }

    /// Returns `true` if this is an error type.
    pub fn is_error(self) -> bool {
        matches!(self.kind(), TyKind::ErrorQualified(_))
    }

    /// Returns `true` if this is an inference variable (not yet resolved).
    pub fn is_infer(self) -> bool {
        matches!(self.kind(), TyKind::Infer(_))
    }

    /// Returns `true` if this type contains any inference variables.
    pub fn has_infer_vars(self) -> bool {
        match self.kind() {
            TyKind::Infer(_) => true,
            TyKind::Ptr(inner, _) | TyKind::Optional(inner) => inner.has_infer_vars(),
            TyKind::Fn(tys) | TyKind::NornmalForm(tys) | TyKind::ErrorQualified(tys) => {
                tys.iter().any(|t| t.has_infer_vars())
            }
            TyKind::FnArrow(a, b) => a.has_infer_vars() || b.has_infer_vars(),
            TyKind::NFApplication(_, args) => args.iter().any(|t| t.has_infer_vars()),
            TyKind::Param
            | TyKind::Primitive(_)
            | TyKind::Void
            | TyKind::NoReturn
            | TyKind::EffectQualified => false,
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

/// The payload of a semantic type.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TyKind<'tcx> {
    Primitive(PrimTy),

    Void,

    /// A raw pointer `*const T` or `*T`.
    Ptr(Ty<'tcx>, Mutability),

    /// An optional type `?T`.
    Optional(Ty<'tcx>),

    /// A function type `fn(...)`.
    Fn(&'tcx [Ty<'tcx>]),
    /// A function type `fn<...>`.
    NornmalForm(&'tcx [Ty<'tcx>]),
    FnArrow(Ty<'tcx>, Ty<'tcx>),
    /// TODO: 给不同参数类型添加不同变体
    Param,

    NFApplication(NFId, &'tcx [Ty<'tcx>]),

    /// A not-yet-resolved inference variable. Created during type
    /// inference and later unified to a concrete type.
    Infer(InferTy),

    /// The bottom type `!` (never returns).
    NoReturn,

    /// 错误集必须按内存升序排列
    ErrorQualified(&'tcx [Ty<'tcx>]),
    /// TODO
    EffectQualified,
}

impl fmt::Display for TyKind<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TyKind::Primitive(p) => write!(f, "{p}"),
            TyKind::Void => write!(f, "()"),
            TyKind::Ptr(inner, Mutability::Immutable) => write!(f, "*const {inner}"),
            TyKind::Ptr(inner, Mutability::Mutable) => write!(f, "*mut {inner}"),
            TyKind::Optional(inner) => write!(f, "{inner}?"),
            TyKind::Fn(params) => {
                write!(f, "fn(")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{p}")?;
                }
                write!(f, ")")
            }
            TyKind::NornmalForm(tys) => {
                write!(f, "nf[")?;
                for (i, t) in tys.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{t}")?;
                }
                write!(f, "]")
            }
            TyKind::FnArrow(from, to) => write!(f, "({from} -> {to})"),
            TyKind::Param => write!(f, "_"),
            TyKind::NFApplication(id, args) => {
                write!(f, "NF({:?}", id.0)?;
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
            TyKind::Infer(iv) => write!(f, "?{}", iv.0),
            TyKind::NoReturn => write!(f, "!"),
            TyKind::ErrorQualified(tys) => {
                write!(f, "Error[")?;
                for (i, t) in tys.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{t}")?;
                }
                write!(f, "]")
            }
            TyKind::EffectQualified => write!(f, "<effect>"),
        }
    }
}

/// Built-in primitive types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrimTy {
    I8,
    I16,
    I32,
    I64,
    Isize,
    U8,
    U16,
    U32,
    U64,
    Usize,
    F32,
    F64,
    Bool,
    Char,
    Str,
    Symbol,

    Integer,
    Float,
}

impl PrimTy {
    /// Returns `true` if this is a signed integer type.
    pub fn is_signed_int(self) -> bool {
        matches!(
            self,
            PrimTy::I8 | PrimTy::I16 | PrimTy::I32 | PrimTy::I64 | PrimTy::Isize
        )
    }

    /// Returns `true` if this is an unsigned integer type.
    pub fn is_unsigned_int(self) -> bool {
        matches!(
            self,
            PrimTy::U8 | PrimTy::U16 | PrimTy::U32 | PrimTy::U64 | PrimTy::Usize
        )
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
            PrimTy::Symbol => "Symbol",
            PrimTy::Integer => "Integer",
            PrimTy::Float => "Float",
        })
    }
}

/// Identity of an algebraic data type (struct or enum).
///
/// Wraps a [`LocalDefId`] pointing at the definition in the package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NFId(pub LocalDefId);

/// An unresolved inference variable.
///
/// Created by the type inference engine. The `u32` is a unique id within
/// the current inference context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InferTy(pub u32);

/// Describes a user-defined algebraic data type (struct / enum).
///
/// Stored in [`TyCtxt`] keyed by [`AdtId`]. Contains the information
/// needed for type checking field access and struct literal construction.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AdtKind {
    Struct,
    Enum,
}

/// A field within a struct / enum variant.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
