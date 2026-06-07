//! Common types shared across HIR nodes.

use std::fmt;

use crate::expr::Expr;
use crate::hir_id::HirId;
use rustc_span::Span;
pub use symbol::DefId;
pub use symbol::PathAnchor;
pub use symbol::Symbol;

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

/// A qualified path: `A.B.C<T>`.
#[derive(Debug, Clone, PartialEq)]
pub struct Path<'hir> {
    /// Where the path starts resolving from.
    pub anchor: PathAnchor,
    pub segments: &'hir [PathSegment<'hir>],
    pub span: Span,
    /// The definition this path was resolved to during early name resolution,
    /// if resolution succeeded.  `None` for paths that are not yet resolved or
    /// that could not be resolved (unresolved names are reported separately).
    pub res: Option<symbol::DefId>,
}

impl<'hir> fmt::Display for Path<'hir> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.anchor {
            PathAnchor::Local => {}
            PathAnchor::Super(n) => {
                for _ in 0..n {
                    f.write_str(".")?;
                }
            }
            PathAnchor::Package => f.write_str("@")?,
        }
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
    pub args: &'hir [Arg<'hir>],
}

#[derive(Debug, Clone, PartialEq)]
pub enum Arg<'hir> {
    Positional(&'hir Expr<'hir>),
    Named(Ident, &'hir Expr<'hir>),
    Expand(&'hir Expr<'hir>),
    Implicit(&'hir Expr<'hir>),
}
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
            BinOp::And => "and",
            BinOp::Or => "or",
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
            UnOp::Not => "not",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BindingMode {
    ByValue,
    ByRef,
}

pub const TPARAM_IMPLICIT: u32 = 1 << 0;
pub const TPARAM_COMPTIME: u32 = 1 << 1;
pub const TPARAM_QUOTE: u32 = 1 << 2;
pub const TPARAM_ERROR: u32 = 1 << 3;
pub const TPARAM_LAMBDA: u32 = 1 << 4;
pub const TPARAM_ASSOC: u32 = 1 << 5;

#[derive(Debug, Clone, PartialEq)]
pub struct TyParam<'hir> {
    pub hir_id: HirId,
    pub kind: TyParamKind<'hir>,
    pub flags: u32,
    pub span: Span,
}

impl TyParam<'_> {
    pub fn is_implicit(&self) -> bool {
        self.flags & TPARAM_IMPLICIT != 0
    }
    pub fn is_comptime(&self) -> bool {
        self.flags & TPARAM_COMPTIME != 0
    }
    pub fn is_quote(&self) -> bool {
        self.flags & TPARAM_QUOTE != 0
    }
    pub fn is_error(&self) -> bool {
        self.flags & TPARAM_ERROR != 0
    }
    pub fn is_lambda(&self) -> bool {
        self.flags & TPARAM_LAMBDA != 0
    }
    pub fn is_assoc(&self) -> bool {
        self.flags & TPARAM_ASSOC != 0
    }
}

impl<'hir> TyParam<'hir> {
    pub fn new(hir_id: HirId, kind: TyParamKind<'hir>, span: Span) -> Self {
        Self {
            hir_id,
            kind,
            flags: 0,
            span,
        }
    }

    pub fn with_implicit(mut self) -> Self {
        self.flags |= TPARAM_IMPLICIT;
        self
    }
    pub fn with_comptime(mut self) -> Self {
        self.flags |= TPARAM_COMPTIME;
        self
    }
    pub fn with_quote(mut self) -> Self {
        self.flags |= TPARAM_QUOTE;
        self
    }
    pub fn with_error(mut self) -> Self {
        self.flags |= TPARAM_ERROR;
        self
    }
    pub fn with_lambda(mut self) -> Self {
        self.flags |= TPARAM_LAMBDA;
        self
    }
    pub fn with_assoc(mut self) -> Self {
        self.flags |= TPARAM_ASSOC;
        self
    }
    pub fn with_flags(mut self, flags: u32) -> Self {
        self.flags |= flags;
        self
    }
}

/// Type alias for one entry in `FnSig::params` — lets the arena macro work
/// with this tuple type via a plain identifier.
pub type FnSigParam<'hir> = (Ident, TyParam<'hir>);

#[derive(Debug, Clone, PartialEq)]
pub enum TyParamKind<'hir> {
    // 比如`fn<T, a: T>`则`T`的`is_dependently_catch`为`true`
    PositionalDependencyCatched(Ident, &'hir Expr<'hir>),
    Positional(&'hir Expr<'hir>),
    Optional(Ident, &'hir Expr<'hir>, &'hir Expr<'hir>),
    Varadic(Ident, &'hir Expr<'hir>),
    Itself { is_ref: bool },
}
