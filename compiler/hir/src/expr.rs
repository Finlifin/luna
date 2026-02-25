//! HIR expressions, blocks, and statements.
//!
//! In Flurry, **types are first-class citizens**. A type expression such as
//! `Int`, `&T`, or `fn(A) -> B` is simply an [`Expr`] whose value is a
//! type. The [`ExprKind`] enum therefore contains both "value" variants
//! (literals, calls, operators, …) and "type constructor" variants
//! (`TyFn`, `TyPtr`, `TyOptional`, …).

use rustc_span::Span;

use crate::ClauseParam;
use crate::body::BodyId;
use crate::common::{BinOp, Ident, Lit, Path, UnOp};
use crate::hir_id::{HirId, OwnerId};
use crate::pattern::Pattern;

#[derive(Debug, Clone, PartialEq)]
pub struct Expr<'hir> {
    pub hir_id: HirId,
    pub kind: ExprKind<'hir>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind<'hir> {
    // ── Value expressions ────────────────────────────────────────────────
    Lit(Lit),
    Path(Path<'hir>),
    Call(&'hir Expr<'hir>, &'hir [Arg<'hir>]),
    ExtendedCall(&'hir Expr<'hir>, &'hir [Arg<'hir>]),
    MethodCall(Ident, &'hir Expr<'hir>, &'hir [Expr<'hir>]),
    ExtendedMethodCall(Ident, &'hir Expr<'hir>, &'hir [Arg<'hir>]),
    Binary(BinOp, &'hir Expr<'hir>, &'hir Expr<'hir>),
    Unary(UnOp, &'hir Expr<'hir>),
    If(
        &'hir Expr<'hir>,
        &'hir Block<'hir>,
        Option<&'hir Expr<'hir>>,
    ),
    Block(&'hir Block<'hir>),
    Field(&'hir Expr<'hir>, Ident),
    Tuple(&'hir [Expr<'hir>]),
    Index(&'hir Expr<'hir>, &'hir Expr<'hir>),
    Assign(&'hir Expr<'hir>, &'hir Expr<'hir>),
    AssignOp(BinOp, &'hir Expr<'hir>, &'hir Expr<'hir>),
    Match(&'hir Expr<'hir>, &'hir [Arm<'hir>]),
    Loop(&'hir Block<'hir>),
    Return(Option<&'hir Expr<'hir>>),
    Resume(Option<&'hir Expr<'hir>>),
    Break(Ident),
    Continue(Ident),
    Ref(&'hir Expr<'hir>),
    Deref(&'hir Expr<'hir>),
    ErrorNew(&'hir Expr<'hir>),
    StructLit(Path<'hir>, &'hir [FieldExpr<'hir>]),
    Array(&'hir [Expr<'hir>]),
    Closure(&'hir [ClosureParam<'hir>], Option<&'hir Expr<'hir>>, BodyId),
    Cast(&'hir Expr<'hir>, &'hir Expr<'hir>),

    Undefined,
    Null,

    // ── Type expressions (types are first-class) ─────────────────────────
    /// Raw pointer type `*T`.
    TyPtr(&'hir Expr<'hir>),
    /// Optional type `T?`.
    TyOptional(&'hir Expr<'hir>),
    /// Function type `fn(A, B) -> C`.
    TyFn(&'hir [Expr<'hir>], &'hir Expr<'hir>),
    /// Type inference placeholder `_`.
    TyPlaceholder,
    TyNoReturn,
    TyVoid,
    TyAny,

    Invalid,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Arg<'hir> {
    Positional(&'hir Expr<'hir>),
    Named(Ident, &'hir Expr<'hir>),
    Expand(&'hir Expr<'hir>),
    Implicit(&'hir Expr<'hir>),
    DependencyCatch(&'hir ClauseParam<'hir>, &'hir Expr<'hir>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block<'hir> {
    pub hir_id: HirId,
    pub stmts: &'hir [Stmt<'hir>],
    pub expr: Option<&'hir Expr<'hir>>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stmt<'hir> {
    pub hir_id: HirId,
    pub kind: StmtKind<'hir>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StmtKind<'hir> {
    Let(&'hir LetStmt<'hir>),
    Semi(&'hir Expr<'hir>),
    Expr(&'hir Expr<'hir>),
    Item(OwnerId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct LetStmt<'hir> {
    pub hir_id: HirId,
    pub pat: Pattern<'hir>,
    pub ty: Option<&'hir Expr<'hir>>,
    pub init: Option<&'hir Expr<'hir>>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Arm<'hir> {
    pub hir_id: HirId,
    pub pat: Pattern<'hir>,
    pub guard: Option<&'hir Expr<'hir>>,
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
