//! Built-in function catalogue.
//!
//! Each [`BuiltinFn`] describes a function that the compiler provides
//! without any source definition. During intrinsic initialisation every
//! built-in is registered into the type context so that the type checker
//! can resolve calls to them.

use std::fmt;

use hir::common::Symbol;

// ── BuiltinFnId ──────────────────────────────────────────────────────────────

/// Unique identifier for a built-in function.
///
/// Uses a dense index so it can double as an array index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BuiltinFnId(pub u16);

impl BuiltinFnId {
    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

// ── BuiltinParam ─────────────────────────────────────────────────────────────

/// Description of a parameter in a built-in function signature.
#[derive(Debug, Clone)]
pub struct BuiltinParam {
    /// The parameter name (for diagnostics).
    pub name: &'static str,
    /// The type of this parameter, expressed as a [`BuiltinTy`].
    pub ty: BuiltinTy,
}

// ── BuiltinTy ────────────────────────────────────────────────────────────────

/// Lightweight type descriptor used *only* for declaring built-in
/// signatures. Resolved to real [`ty::Ty`] during initialisation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuiltinTy {
    Int,
    Float,
    Bool,
    Char,
    Str,
    Unit,
    Never,
    /// A generic / any-type parameter (for polymorphic builtins like `print`).
    Any,
}

// ── BuiltinFn ────────────────────────────────────────────────────────────────

/// Static description of a compiler-provided function.
#[derive(Debug, Clone)]
pub struct BuiltinFn {
    /// Dense index (matches position in [`ALL_BUILTINS`]).
    pub id: BuiltinFnId,
    /// The function name as it appears in source code.
    pub name: &'static str,
    /// Parameter list.
    pub params: &'static [BuiltinParam],
    /// Return type.
    pub ret: BuiltinTy,
    /// Short description (shown in hover / documentation).
    pub doc: &'static str,
}

impl BuiltinFn {
    /// The name as an interned [`Symbol`].
    pub fn symbol(&self) -> Symbol {
        Symbol::intern(self.name)
    }
}

impl fmt::Display for BuiltinFn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "fn {}(", self.name)?;
        for (i, p) in self.params.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}: {:?}", p.name, p.ty)?;
        }
        write!(f, ") -> {:?}", self.ret)
    }
}

// ── Catalogue ────────────────────────────────────────────────────────────────

macro_rules! builtin_fn {
    ($id:expr, $name:expr, [ $( ($pname:expr, $pty:expr) ),* $(,)? ], $ret:expr, $doc:expr) => {
        BuiltinFn {
            id: BuiltinFnId($id),
            name: $name,
            params: &[ $( BuiltinParam { name: $pname, ty: $pty } ),* ],
            ret: $ret,
            doc: $doc,
        }
    };
}

/// The complete catalogue of built-in functions.
///
/// The index in this array **is** the [`BuiltinFnId`].
pub static ALL_BUILTINS: &[BuiltinFn] = &[
    // ── I/O ──────────────────────────────────────────────────────────────
    builtin_fn!(
        0,
        "print",
        [("value", BuiltinTy::Any)],
        BuiltinTy::Unit,
        "Print a value to stdout (no trailing newline)."
    ),
    builtin_fn!(
        1,
        "println",
        [("value", BuiltinTy::Any)],
        BuiltinTy::Unit,
        "Print a value to stdout followed by a newline."
    ),
    // ── Assertions / panics ──────────────────────────────────────────────
    builtin_fn!(
        2,
        "assert",
        [("condition", BuiltinTy::Bool)],
        BuiltinTy::Unit,
        "Assert that a condition is true; panic otherwise."
    ),
    builtin_fn!(
        3,
        "panic",
        [("message", BuiltinTy::Str)],
        BuiltinTy::Never,
        "Abort execution with an error message."
    ),
    builtin_fn!(
        4,
        "todo",
        [],
        BuiltinTy::Never,
        "Mark unfinished code; panics at runtime."
    ),
    builtin_fn!(
        5,
        "unreachable",
        [],
        BuiltinTy::Never,
        "Indicate unreachable code; panics at runtime if reached."
    ),
    // ── Introspection ────────────────────────────────────────────────────
    builtin_fn!(
        6,
        "sizeof",
        [("value", BuiltinTy::Any)],
        BuiltinTy::Int,
        "Return the size in bytes of a value's type."
    ),
    builtin_fn!(
        7,
        "typeof",
        [("value", BuiltinTy::Any)],
        BuiltinTy::Str,
        "Return a string representation of a value's type."
    ),
];

/// Look up a built-in function by name.
pub fn lookup_builtin(name: &str) -> Option<&'static BuiltinFn> {
    ALL_BUILTINS.iter().find(|b| b.name == name)
}

/// Look up a built-in function by its [`BuiltinFnId`].
pub fn get_builtin(id: BuiltinFnId) -> Option<&'static BuiltinFn> {
    ALL_BUILTINS.get(id.index())
}
