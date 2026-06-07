//! HIR items – top-level definition descriptors.
//!
//! An [`Item`] is the HIR representation of any top-level definition:
//! functions, structs, enums, modules, impls, traits, type aliases, and
//! use-imports.  Each item is identified by an [`OwnerId`] and stored in the
//! [`Package`](crate::Package) owner table.

use rustc_span::Span;

use crate::body::BodyId;
use crate::clause::ClauseConstraint;
use crate::common::{FnSigParam, Ident, Path};
use crate::expr::Expr;
use crate::hir_id::{HirId, OwnerId};
use crate::{ClauseParam, Pattern};

/// A single top-level HIR item (function, struct, enum, …).
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

/// Discriminates the concrete form of a top-level [`Item`].
#[derive(Debug, Clone, PartialEq)]
pub enum ItemKind<'hir> {
    Fn(FnSig<'hir>, BodyId),
    Struct(StructDef<'hir>),
    Enum(EnumDef<'hir>),
    Mod(ModDef),
    Impl(ImplDef<'hir>),
    Trait(TraitDef<'hir>),
    TypeAlias(&'hir Expr<'hir>),
    Use(UsePath<'hir>),
    Const(&'hir Expr<'hir>, &'hir Expr<'hir>),
    Invalid,
}

/// Full function signature: declaration + modifiers + clause parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct FnSig<'hir> {
    pub params: &'hir [FnSigParam<'hir>],
    pub return_ty: Option<&'hir Expr<'hir>>,
    pub return_bind: Option<Ident>,
    pub modifiers: FnModifiers,
    pub clause_params: &'hir [ClauseParam<'hir>],
    pub clause_constraints: &'hir [ClauseConstraint<'hir>],
    pub span: Span,
}

/// Modifier flags on a function definition (`pure`, `comptime`, `extern`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FnModifiers {
    pub is_pure: bool,
    pub is_comptime: bool,
    pub is_extern: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NFSig<'hir> {
    pub params: &'hir [FnSigParam<'hir>],
    pub return_ty: Option<&'hir Expr<'hir>>,
    pub return_bind: Option<Ident>,
    pub clause_params: &'hir [ClauseParam<'hir>],
    pub clause_constraints: &'hir [ClauseConstraint<'hir>],
    pub span: Span,
}

/// Definition body of a `struct` item.
#[derive(Debug, Clone, PartialEq)]
pub struct StructDef<'hir> {
    pub fields: &'hir [FieldDef<'hir>],
    pub clause_params: &'hir [ClauseParam<'hir>],
    pub clause_constraints: &'hir [ClauseConstraint<'hir>],
    pub nested_items: Vec<OwnerId>,
}

/// A single field inside a struct or enum variant.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldDef<'hir> {
    pub hir_id: HirId,
    pub ident: Ident,
    pub ty: &'hir Expr<'hir>,
    pub default: Option<&'hir Expr<'hir>>,
    pub span: Span,
}

/// Definition body of an `enum` item.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumDef<'hir> {
    pub variants: &'hir [Variant<'hir>],
    pub clause_params: &'hir [ClauseParam<'hir>],
    pub clause_constraints: &'hir [ClauseConstraint<'hir>],
    pub nested_items: Vec<OwnerId>,
}

/// A single variant of an enum.
#[derive(Debug, Clone, PartialEq)]
pub struct Variant<'hir> {
    pub hir_id: HirId,
    pub ident: Ident,
    pub kind: VariantKind<'hir>,
    pub span: Span,
}

/// Shape of an enum variant's payload.
#[derive(Debug, Clone, PartialEq)]
pub enum VariantKind<'hir> {
    Unit,
    Pattern(&'hir Pattern<'hir>),
    Const(&'hir Expr<'hir>),
    Tuple(&'hir [Expr<'hir>]),
    Struct(&'hir [FieldDef<'hir>]),
    SubEnum(&'hir [Variant<'hir>]),
}

/// Definition body of a `mod` item — simply a list of child owners.
#[derive(Debug, Clone, PartialEq)]
pub struct ModDef {
    pub items: Vec<OwnerId>,
}

/// Definition body of an `impl` block (inherent or trait implementation).
#[derive(Debug, Clone, PartialEq)]
pub struct ImplDef<'hir> {
    pub self_ty: &'hir Expr<'hir>,
    pub trait_ref: Option<&'hir Expr<'hir>>,
    pub clause_params: &'hir [ClauseParam<'hir>],
    pub clause_constraints: &'hir [ClauseConstraint<'hir>],
    pub items: Vec<OwnerId>,
}

/// Definition body of a `trait` item.
#[derive(Debug, Clone, PartialEq)]
pub struct TraitDef<'hir> {
    pub clause_params: &'hir [ClauseParam<'hir>],
    pub clause_constraints: &'hir [ClauseConstraint<'hir>],
    pub items: Vec<OwnerId>,
}

/// The path and import style of a `use` item.
#[derive(Debug, Clone, PartialEq)]
pub struct UsePath<'hir> {
    pub path: Path<'hir>,
    pub kind: UseKind<'hir>,
    pub span: Span,
}

/// Import style of a `use` item.
#[derive(Debug, Clone, PartialEq)]
pub enum UseKind<'hir> {
    Simple,
    Glob,
    Multi(&'hir [Ident]),
    Alias(Ident),
}

/// The kind of a definition — a coarser classification than [`ItemKind`]
/// that is useful when all you need to know is *what sort* of item a
/// [`LocalDefId`](crate::hir_id::LocalDefId) refers to.
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
    Const,
    Invalid,
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
            ItemKind::Const(..) => DefKind::Const,
            ItemKind::Invalid => DefKind::Invalid,
        }
    }
}
