//! HIR items – top-level definition descriptors.

use rustc_span::Span;

use crate::body::BodyId;
use crate::clause::ClauseConstraint;
use crate::common::{Ident, Path};
use crate::expr::Expr;
use crate::hir_id::{HirId, OwnerId};
use crate::{ClauseParam, Pattern};

// ── Item ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct Item<'hir> {
    pub owner_id: OwnerId,
    pub ident: Ident,
    pub kind: ItemKind<'hir>,
    pub span: Span,
}

impl Item<'_> {
    pub fn hir_id(&self) -> HirId {
        HirId::make_owner(self.owner_id)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ItemKind<'hir> {
    Fn(FnSig<'hir>, BodyId),
    Struct(StructDef<'hir>, &'hir [ClauseConstraint<'hir>]),
    Enum(EnumDef<'hir>, &'hir [ClauseConstraint<'hir>]),
    Mod(ModDef),
    Impl(ImplDef<'hir>),
    Trait(TraitDef<'hir>),
    TypeAlias(&'hir Expr<'hir>, &'hir [ClauseConstraint<'hir>]),
    Use(UsePath<'hir>),
    Err,
}

// ── Function ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct FnSig<'hir> {
    pub decl: FnDecl<'hir>,
    pub modifiers: FnModifiers,
    pub clause_params: &'hir [ClauseParam<'hir>],
    pub clause_constraints: &'hir [ClauseConstraint<'hir>],
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FnDecl<'hir> {
    pub inputs: &'hir [FnParamTy<'hir>],
    pub output: Option<&'hir Expr<'hir>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FnParamTy<'hir> {
    Typed(Ident, &'hir Expr<'hir>, Span, FnParamKind),
    Optional {
        ident: Ident,
        ty: &'hir Expr<'hir>,
        default: &'hir Expr<'hir>,
        span: Span,
        kind: FnParamKind,
    },
    Variadic(Ident, &'hir Expr<'hir>, Span, FnParamKind),
}

#[derive(Debug, Clone, PartialEq)]
pub enum FnParamKind {
    Common,
    Comptime,
    Implicit,
    Lambda,
    Quote,
    Error,
    Catch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FnModifiers {
    pub is_pure: bool,
    pub is_comptime: bool,
    pub is_extern: bool,
}

// ── Struct ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct StructDef<'hir> {
    pub fields: &'hir [FieldDef<'hir>],
    pub clause_params: &'hir [ClauseParam<'hir>],
    pub clause_constraints: &'hir [ClauseConstraint<'hir>],
    pub nested_items: &'hir [OwnerId],
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldDef<'hir> {
    pub hir_id: HirId,
    pub ident: Ident,
    pub ty: &'hir Expr<'hir>,
    pub default: Option<&'hir Expr<'hir>>,
    pub span: Span,
}

// ── Enum ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct EnumDef<'hir> {
    pub variants: &'hir [Variant<'hir>],
    pub clause_params: &'hir [ClauseParam<'hir>],
    pub clause_constraints: &'hir [ClauseConstraint<'hir>],
    pub nested_items: &'hir [OwnerId],
}

#[derive(Debug, Clone, PartialEq)]
pub struct Variant<'hir> {
    pub hir_id: HirId,
    pub ident: Ident,
    pub kind: VariantKind<'hir>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VariantKind<'hir> {
    Unit,
    Pattern(&'hir Pattern<'hir>),
    Const(&'hir Expr<'hir>),
    Tuple(&'hir [Expr<'hir>]),
    Struct(&'hir [FieldDef<'hir>]),
    SubEnum(&'hir [Variant<'hir>]),
}

// ── Module ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct ModDef {
    pub items: Vec<OwnerId>,
}

// ── Impl / Trait ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct ImplDef<'hir> {
    pub self_ty: &'hir Expr<'hir>,
    pub trait_ref: Option<Path<'hir>>,
    pub clause_params: &'hir [ClauseParam<'hir>],
    pub clause_constraints: &'hir [ClauseConstraint<'hir>],
    pub items: Vec<OwnerId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TraitDef<'hir> {
    pub clause_params: &'hir [ClauseParam<'hir>],
    pub clause_constraints: &'hir [ClauseConstraint<'hir>],
    pub items: Vec<OwnerId>,
}

// ── Use ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct UsePath<'hir> {
    pub path: Path<'hir>,
    pub kind: UseKind<'hir>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum UseKind<'hir> {
    Simple,
    Glob,
    Multi(&'hir [Ident]),
    Alias(Ident),
}

// ── DefKind ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DefKind {
    Fn,
    Struct,
    Enum,
    Mod,
    Impl,
    Trait,
    TypeAlias,
    Use,
}

impl DefKind {
    pub fn from_item_kind(kind: &ItemKind<'_>) -> Self {
        match kind {
            ItemKind::Fn(..) => DefKind::Fn,
            ItemKind::Struct(..) => DefKind::Struct,
            ItemKind::Enum(..) => DefKind::Enum,
            ItemKind::Mod(..) => DefKind::Mod,
            ItemKind::Impl(..) => DefKind::Impl,
            ItemKind::Trait(..) => DefKind::Trait,
            ItemKind::TypeAlias(..) => DefKind::TypeAlias,
            ItemKind::Use(..) => DefKind::Use,
            ItemKind::Err => DefKind::Fn,
        }
    }
}
