//! HIR expressions, blocks, and statements.
//!
//! In Flurry, **types are first-class citizens**. A type expression such as
//! `Int`, `&T`, or `fn(A) -> B` is simply an [`Expr`] whose value is a
//! type. The [`ExprKind`] enum therefore contains both "value" variants
//! (literals, calls, operators, …) and "type constructor" variants
//! (`TyFn`, `TyPtr`, `TyOptional`, …).

use rustc_span::Span;
use symbol::{DefId, Symbol};

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
    /// A comptime code value; its body is checked as runtime code.
    CodeQuote(&'hir Expr<'hir>, Vec<CodeSubstitution<'hir>>),
    /// Obtain a comptime code value and connect its runtime interface here.
    CodeSplice(&'hir Expr<'hir>, Vec<CodeSubstitution<'hir>>),
    ContextualExprType(Vec<CodeContextBinding<'hir>>, &'hir Expr<'hir>),
    /// A request for the literal-conversion protocol, selected in elaboration.
    /// The suffix identifies the protocol request, not a resolved value name.
    LiteralConversion {
        literal: &'hir Expr<'hir>,
        suffix: Ident,
    },
    Path(Path<'hir>),

    UnresolvedIdent(Symbol),
    /// A contextual tactic-lambda parameter, bound during elaboration.
    TacticParameter(usize),
    /// Declaration-routed argument with its own tactic-lambda boundary.
    TacticArgument(&'hir Expr<'hir>),
    /// A trailing-do body whose closure arity is determined at its input slot.
    PostLambdaBody(BodyId),
    ResolvedLocal(Symbol, HirId),
    ResolvedFnParam(Symbol, HirId),
    ResolvedClauseParam(Symbol, DefId),
    ResolvedDef {
        name: Symbol,
        has_clause_param: bool,
        def_id: DefId,
        // scope_id: ..
    },
    ResolvedLoopLabel(Symbol, HirId),

    SelfValue,

    Index(&'hir Expr<'hir>, &'hir Expr<'hir>),
    Range {
        start: Option<&'hir Expr<'hir>>,
        end: Option<&'hir Expr<'hir>>,
        inclusive: bool,
    },
    Application(&'hir Expr<'hir>, &'hir [Arg<'hir>]),
    ExtendedApplication(&'hir Expr<'hir>, &'hir [Arg<'hir>]),
    /// Elaboration-generated packet for a selected property collection entry.
    /// Retains source fields so ordinary argument inference can defer/retry it.
    CollectedProperties(&'hir [FieldExpr<'hir>]),
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
    /// Request/propagate the effects of a computation (`value#`).
    EffectRequest(&'hir Expr<'hir>),
    /// Install branches around a computation. Branches execute in the outer
    /// effect environment; the computation retains this handler on resumption.
    EffectHandle(&'hir Expr<'hir>, &'hir [PatternArm<'hir>]),
    Assign(&'hir Expr<'hir>, &'hir Expr<'hir>),
    AssignOp(BinOp, &'hir Expr<'hir>, &'hir Expr<'hir>),
    Assumes(&'hir Expr<'hir>),
    /// Explicit permission to retain the computation for instantiation.
    /// Does not relax ordinary definition-side operation checks.
    Defer(&'hir Expr<'hir>),
    RefinedType { requirement: &'hir Expr<'hir>, base: &'hir Expr<'hir> },
    RefinementSubject,
    Return(Option<&'hir Expr<'hir>>),
    OptionPropagate(&'hir Expr<'hir>),
    Resume(Option<&'hir Expr<'hir>>),
    Break(Ident),
    BreakTo(HirId),
    Continue(Ident),
    ContinueTo(HirId),

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

    /// Protocol-directed iteration. Keep the source construct until elaboration
    /// selects iteration evidence; AST lowering must not invent method names.
    For {
        label: Option<Ident>,
        pat: &'hir Pattern<'hir>,
        guard: Option<&'hir Expr<'hir>>,
        iter: &'hir Expr<'hir>,
        body: &'hir super::Block<'hir>,
    },

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
    TyClosure(&'hir Expr<'hir>, &'hir Expr<'hir>),
    TyClosureEnvironment(&'hir [ClosureEnvironmentBinding<'hir>]),
    /// Optional type `??`.
    TyOptional(&'hir Expr<'hir>),
    /// Function types are constructed using `TyFn` and `TyFnArrow`.
    TyFn(&'hir [TyParam<'hir>]),
    TyComptimeFn(&'hir [TyParam<'hir>]),
    TyNFFn(&'hir [TyParam<'hir>]),
    TyFnArrow(&'hir Expr<'hir>, &'hir Expr<'hir>),

    /// TODO
    ReachabilityType,
    ErrorQualifiedType,
    EffectQualifiedType(&'hir Expr<'hir>, &'hir Expr<'hir>),

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
    TraitBound(&'hir Expr<'hir>, &'hir Expr<'hir>),
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

    Builtin(Symbol),
    TakeOld(&'hir Expr<'hir>),
    TakeBitVec(&'hir Expr<'hir>),
    TakeTagName(&'hir Expr<'hir>),
    TypeOf(&'hir Expr<'hir>),
    MultiDeref(&'hir Expr<'hir>),

    Invalid,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CodeSubstitution<'hir> {
    pub port: Ident,
    pub binding: &'hir Expr<'hir>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CodeContextBinding<'hir> {
    pub hir_id: HirId,
    pub name: Ident,
    pub ty: &'hir Expr<'hir>,
    /// Original lexical association, never resolved again at the splice site.
    pub default: Option<&'hir Expr<'hir>>,
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
    /// None denotes the explicit fallback arm, not a synthetic true expression.
    pub cond: Option<&'hir Expr<'hir>>,
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

#[derive(Debug, Clone, PartialEq)]
pub struct ClosureEnvironmentBinding<'hir> {
    pub name: Ident,
    pub ty: &'hir Expr<'hir>,
    pub value: Option<&'hir Expr<'hir>>,
}
