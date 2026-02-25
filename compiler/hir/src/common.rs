//! Common types shared across HIR nodes.

use std::fmt;
use std::ops::Deref;

use internment::Intern;
use rustc_span::Span;

// ── Symbol / Ident ───────────────────────────────────────────────────────────

/// An interned string backed by [`internment::Intern`].
///
/// * **`Copy`** – zero-cost to duplicate.
/// * **O(1) `Eq` / `Hash`** – pointer comparison.
/// * **`Deref<Target = str>`** – use anywhere a `&str` is expected.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Symbol(Intern<String>);

impl Symbol {
    /// Intern a string slice, returning the canonical [`Symbol`].
    #[inline]
    pub fn intern(s: &str) -> Self {
        Symbol(Intern::new(s.to_owned()))
    }

    /// View the underlying string slice.
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Deref for Symbol {
    type Target = str;
    #[inline]
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for Symbol {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq<str> for Symbol {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for Symbol {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<Symbol> for str {
    fn eq(&self, other: &Symbol) -> bool {
        self == other.as_str()
    }
}

impl From<&str> for Symbol {
    #[inline]
    fn from(s: &str) -> Self {
        Symbol::intern(s)
    }
}

impl From<String> for Symbol {
    #[inline]
    fn from(s: String) -> Self {
        Symbol(Intern::new(s))
    }
}

/// Convenience macro – intern a string expression as a [`Symbol`].
///
/// ```ignore
/// let s = sym!("hello");
/// ```
#[macro_export]
macro_rules! sym {
    ($s:expr) => {
        $crate::common::Symbol::intern($s)
    };
}

/// Define pre-interned [`Symbol`] constants (lazily initialised on first
/// access).
///
/// ```ignore
/// hir::define_symbols! {
///     pub kw_if   => "if",
///     pub kw_let  => "let",
///     pub main,             // interned as "main"
/// }
/// ```
#[macro_export]
macro_rules! define_symbols {
    ($( $vis:vis $name:ident $(=> $s:expr)? ),* $(,)?) => {
        $(
            $crate::define_symbols!(@one $vis $name $(=> $s)?);
        )*
    };
    (@one $vis:vis $name:ident => $s:expr) => {
        #[allow(non_upper_case_globals)]
        $vis static $name: std::sync::LazyLock<$crate::common::Symbol> =
            std::sync::LazyLock::new(|| $crate::common::Symbol::intern($s));
    };
    (@one $vis:vis $name:ident) => {
        #[allow(non_upper_case_globals)]
        $vis static $name: std::sync::LazyLock<$crate::common::Symbol> =
            std::sync::LazyLock::new(|| $crate::common::Symbol::intern(stringify!($name)));
    };
}

define_symbols! {
    pub kw_if   => "if",
    pub kw_let  => "let",
    pub kw_fn   => "fn",
    pub main,              // 等价于 main => "main"
}

/// An identifier: a name plus its source location.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Ident {
    pub name: Symbol,
    pub span: Span,
}

impl Ident {
    pub fn new(name: impl Into<Symbol>, span: Span) -> Self {
        Ident {
            name: name.into(),
            span,
        }
    }
}

impl fmt::Display for Ident {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)
    }
}

// ── Path ─────────────────────────────────────────────────────────────────────

/// A qualified path: `A.B.C<T>`.
#[derive(Debug, Clone, PartialEq)]
pub struct Path<'hir> {
    pub segments: &'hir [PathSegment<'hir>],
    pub span: Span,
}

impl<'hir> fmt::Display for Path<'hir> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, seg) in self.segments.iter().enumerate() {
            if i > 0 {
                write!(f, ".")?;
            }
            write!(f, "{}", seg.ident)?;
        }
        Ok(())
    }
}

/// One segment of a [`Path`], e.g. `HashMap<K, V>`.
#[derive(Debug, Clone, PartialEq)]
pub struct PathSegment<'hir> {
    pub ident: Ident,
    pub args: &'hir [GenericArg<'hir>],
}

/// A generic argument in a path segment.
#[derive(Debug, Clone, PartialEq)]
pub enum GenericArg<'hir> {
    /// A common argument, e.g. `T` in `Vec<T>`.
    Expr(&'hir super::expr::Expr<'hir>),
    Optional(Ident, &'hir super::expr::Expr<'hir>),
}

// ── Lit ──────────────────────────────────────────────────────────────────────

/// A literal value.
#[derive(Debug, Clone, PartialEq)]
pub struct Lit {
    pub kind: LitKind,
    pub span: Span,
}

/// The kind of a literal.
#[derive(Debug, Clone, PartialEq)]
pub enum LitKind {
    Integer(i64),
    Float(f64),
    String(String),
    Bool(bool),
    Char(char),
    Symbol(Symbol),
}

// ── Operators ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

impl fmt::Display for BinOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Gt => ">",
            BinOp::Le => "<=",
            BinOp::Ge => ">=",
            BinOp::And => "&&",
            BinOp::Or => "||",
            BinOp::BitAnd => "&",
            BinOp::BitOr => "|",
            BinOp::BitXor => "^",
            BinOp::Shl => "<<",
            BinOp::Shr => ">>",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnOp {
    Neg,
    Not,
}

impl fmt::Display for UnOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            UnOp::Neg => "-",
            UnOp::Not => "!",
        })
    }
}

// ── Mutability & Binding ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mutability {
    Mutable,
    Immutable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BindingMode {
    ByValue,
    ByRef,
}
