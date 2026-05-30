//! HIR expressions, blocks, and statements.
//!
//! In Flurry, **types are first-class citizens**. A type expression such as
//! `Int`, `&T`, or `fn(A) -> B` is simply an [`Expr`] whose value is a
//! type. The [`ExprKind`] enum therefore contains both "value" variants
//! (literals, calls, operators, …) and "type constructor" variants
//! (`TyFn`, `TyPtr`, `TyOptional`, …).

use rustc_span::Span;

use crate::body::BodyId;
use crate::common::{BinOp, Ident, Lit, Path, UnOp};
use crate::decl::LetDecl;
use crate::hir_id::{HirId, OwnerId};
use crate::pattern::{Pattern, PatternArm};

#[derive(Debug, Clone, PartialEq)]
pub struct Expr<'hir> {
    pub hir_id: HirId,
    pub kind: ExprKind<'hir>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind<'hir> {
    Lit(Lit),
    Path(Path<'hir>),

    SelfValue,

    Index(&'hir Expr<'hir>, &'hir Expr<'hir>),
    Application(&'hir Expr<'hir>, &'hir [Arg<'hir>]),
    ExtendedApplication(&'hir Expr<'hir>, &'hir [Arg<'hir>]),
    NFApplication(&'hir Expr<'hir>, &'hir [Arg<'hir>]),

    Binary(BinOp, &'hir Expr<'hir>, &'hir Expr<'hir>),
    Unary(UnOp, &'hir Expr<'hir>),

    If(
        &'hir Expr<'hir>,
        &'hir Block<'hir>,
        Option<&'hir Expr<'hir>>,
    ),
    When(&'hir [CondictionArm<'hir>]),
    Block(&'hir Block<'hir>),
    Loop(&'hir Block<'hir>),
    Match(&'hir Expr<'hir>, &'hir [PatternArm<'hir>]),
    Assign(&'hir Expr<'hir>, &'hir Expr<'hir>),
    AssignOp(BinOp, &'hir Expr<'hir>, &'hir Expr<'hir>),
    Return(Option<&'hir Expr<'hir>>),
    Resume(Option<&'hir Expr<'hir>>),
    Break(Ident),
    Continue(Ident),

    Projection(&'hir Expr<'hir>, Ident),

    Tuple(&'hir [Expr<'hir>]),
    List(&'hir [Expr<'hir>]),
    Object(&'hir [FieldExpr<'hir>]),

    Ref(&'hir Expr<'hir>),
    Deref(&'hir Expr<'hir>),
    ErrorNew(&'hir Expr<'hir>),
    Closure(&'hir [ClosureParam<'hir>], Option<&'hir Expr<'hir>>, BodyId),
    Cast(&'hir Expr<'hir>, &'hir Expr<'hir>),

    /// Statement-as-expression: `let pat = init`
    Let(&'hir LetDecl<'hir>),
    /// Statement-as-expression: `expr;` (value discarded)
    Semi(&'hir Expr<'hir>),
    /// Statement-as-expression: inline item definition
    Item(OwnerId),

    Undefined,
    Null,

    /// TODO: inline control flow expressions
    InlineIf {
        cond: &'hir Expr<'hir>,
        then_expr: &'hir Expr<'hir>,
        else_expr: Option<&'hir Expr<'hir>>,
    },
    InlineMatch(&'hir [PatternArm<'hir>]),
    InlineFor {
        label: Option<Ident>,
        pat: &'hir Pattern<'hir>,
        iter: &'hir Expr<'hir>,
        body: &'hir Expr<'hir>,
    },

    /// Pointer type `*T`.
    TyPtr(&'hir Expr<'hir>),
    /// Optional type `??`.
    TyOptional(&'hir Expr<'hir>),
    /// Function types are constructed using `TyFn` and `TyFnArrow`.
    TyFn(&'hir [TyParam<'hir>]),
    TyNFFn(&'hir [TyParam<'hir>]),
    TyFnArrow(&'hir Expr<'hir>, &'hir Expr<'hir>),

    /// TODO
    ReachabilityType,
    ErrorQualifiedType,
    EffectQualifiedType,

    /// Type inference placeholder `_`.
    TyPlaceholder,
    TyNoReturn,
    TyVoid,
    TyAny,
    TyType,
    TySelf,

    /// propositions
    /// `t: T`
    TermTypedWith,
    /// `T:- U`
    TraitBound,
    /// `F:+ G`
    LambdaBound,
    /// `t:- U`
    TermTraitBound,
    /// `expr ==> expr`
    Implication,
    /// `T1 <: T2`
    Subtype,

    /// TODO
    Forall,
    Exist,

    Invalid,
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

#[derive(Debug, Clone, PartialEq)]
pub enum TyParamKind<'hir> {
    // 比如`fn<T, a: T>`则`T`的`is_dependently_catch`为`true`
    PositionalDependencyCatched(Ident, &'hir Expr<'hir>),
    Positional(&'hir Expr<'hir>),
    Optional(Ident, &'hir Expr<'hir>),
    Varadic(Ident, &'hir Expr<'hir>),
    Itself { is_ref: bool },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Arg<'hir> {
    Positional(&'hir Expr<'hir>),
    Named(Ident, &'hir Expr<'hir>),
    Expand(&'hir Expr<'hir>),
    Implicit(&'hir Expr<'hir>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block<'hir> {
    pub hir_id: HirId,
    pub stmts: &'hir [Expr<'hir>],
    pub expr: Option<&'hir Expr<'hir>>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CondictionArm<'hir> {
    pub hir_id: HirId,
    pub cond: &'hir Expr<'hir>,
    pub body: &'hir Expr<'hir>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldExpr<'hir> {
    pub ident: Ident,
    pub expr: &'hir Expr<'hir>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClosureParam<'hir> {
    pub hir_id: HirId,
    pub pat: Pattern<'hir>,
    pub ty: Option<&'hir Expr<'hir>>,
    pub span: Span,
}
