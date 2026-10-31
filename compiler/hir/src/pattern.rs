//! HIR pattern types.

use rustc_span::Span;

use crate::Expr;
use crate::common::{BindingMode, Ident, Path};
use crate::hir_id::HirId;

#[derive(Debug, Clone, PartialEq)]
pub struct Pattern<'hir> {
    pub hir_id: HirId,
    pub kind: PatternKind<'hir>,
    pub span: Span,
}

/// All pattern kinds in the Flurry HIR.
///
/// Arm guards are retained on `PatternArm`; unimplemented pattern protocols
/// must produce diagnostics rather than dropping their matching conditions.
#[derive(Debug, Clone, PartialEq)]
pub enum PatternKind<'hir> {
    CodeSplice(&'hir Expr<'hir>, Vec<CodePatternOutput>),
    Then(
        &'hir Pattern<'hir>,
        &'hir super::expr::Expr<'hir>,
        &'hir Pattern<'hir>,
    ),
    Guard(&'hir Pattern<'hir>, &'hir super::expr::Expr<'hir>),
    Wild,

    Projection(&'hir Pattern<'hir>, Ident),

    Binding(BindingMode, Ident, Option<&'hir Pattern<'hir>>),

    Const(&'hir Expr<'hir>),
    /// A comptime expression `<comptime_expr>` that is evaluated at compile
    /// time to produce a constant value used as a match discriminant.
    Comptime(&'hir Expr<'hir>),

    /// Tuple pattern: `(pat1, pat2, …)`.
    Tuple(&'hir [Pattern<'hir>]),
    NormalForm(&'hir super::expr::Expr<'hir>, &'hir [TypePatternArg<'hir>]),
    /// Struct pattern without a leading path; the concrete struct type is
    /// inferred.  Syntax: `{ field1, field2, field3: pat3, .. }`.
    /// Fields without an explicit sub-pattern bind their name (`field: field`).
    Struct(&'hir Pattern<'hir>, &'hir [FieldPat<'hir>], bool),
    /// List pattern: `[pat1, pat2, ...rest]` or `[pat1, pat2, ..._]`.
    List(&'hir [Pattern<'hir>], Option<&'hir Pattern<'hir>>),

    /// Tuple-like enum variant pattern: `.NetErr.Timeout(pat1, pat2)`.
    AppTuple(&'hir Pattern<'hir>, &'hir [Pattern<'hir>]),
    /// Struct-like enum variant pattern: `.NetErr.Timeout { field: pat }`.
    AppStruct(&'hir Pattern<'hir>, &'hir [FieldPat<'hir>]),

    /// Matches the `Some` side of an optional value: `some_value?`.
    OptionSome(&'hir Pattern<'hir>),
    /// Matches the `None`/null side of an optional value: `null`.
    OptionNull,

    /// Matches the `Ok` side of a result/error value: `ok_result!`.
    ErrorOk(&'hir Pattern<'hir>),
    /// Matches the `Err` side of a result/error value: `error err_pattern`.
    ErrorErr(&'hir Pattern<'hir>),

    Or(&'hir [Pattern<'hir>]),
    Not(&'hir Pattern<'hir>),
    Ref(&'hir Pattern<'hir>),
    Range(
        Option<&'hir super::expr::Expr<'hir>>,
        Option<&'hir super::expr::Expr<'hir>>,
        BoundType,
    ),

    // TODO
    Async,
    BitVec {
        radix: u32,
        segments: Vec<BitSegment<'hir>>,
    },

    Invalid,
}

/// Explicit success outputs introduce fresh bindings; they do not read
/// same-named bindings from the surrounding runtime scope.
#[derive(Debug, Clone, PartialEq)]
pub struct CodePatternOutput {
    pub port: Ident,
    pub name: Ident,
    pub hir_id: HirId,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PatternArm<'hir> {
    pub hir_id: HirId,
    pub pat: Pattern<'hir>,
    pub guard: Option<&'hir Expr<'hir>>,
    pub body: &'hir Expr<'hir>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BoundType {
    Inclusive,
    Exclusive,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldPat<'hir> {
    pub ident: Ident,
    pub pat: Pattern<'hir>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BitSegment<'hir> {
    Literal {
        spelling: String,
        span: Span,
    },
    Typed {
        pattern: &'hir Pattern<'hir>,
        ty: &'hir Expr<'hir>,
    },
    Computed(&'hir Expr<'hir>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypePatternArg<'hir> {
    pub name: Option<Ident>,
    pub pattern: Pattern<'hir>,
}
