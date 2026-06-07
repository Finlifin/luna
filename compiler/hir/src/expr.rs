//! HIR expressions, blocks, and statements.
//!
//! In Flurry, **types are first-class citizens**. A type expression such as
//! `Int`, `&T`, or `fn(A) -> B` is simply an [`Expr`] whose value is a
//! type. The [`ExprKind`] enum therefore contains both "value" variants
//! (literals, calls, operators, …) and "type constructor" variants
//! (`TyFn`, `TyPtr`, `TyOptional`, …).

use rustc_span::Span;
use symbol::Symbol;

use crate::body::BodyId;
use crate::common::{Arg, BinOp, Ident, Lit, Path, TyParam, UnOp};
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

    Ident(Symbol),
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
    Object(&'hir [Expr<'hir>], &'hir [FieldExpr<'hir>]),

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
    Unit,

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
