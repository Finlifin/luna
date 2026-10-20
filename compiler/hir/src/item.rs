//! HIR items – top-level definition descriptors.
//!
//! An [`Item`] is the HIR representation of any top-level definition:
//! functions, structs, enums, modules, impls, traits, type aliases, and
//! use-imports.  Each item is identified by an [`OwnerId`] and stored in the
//! [`Package`](crate::Package) owner table.

use rustc_span::Span;

use crate::body::BodyId;
use crate::clause::ClauseConstraint;
use crate::common::{FnParam, Ident, Path};
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
    /// Required interface member or bodyless declaration. No synthetic runtime
    /// body may be invented for a signature-only function.
    FnDeclaration(FnSig<'hir>),
    /// An operation signature, never an ordinary function or enum payload.
    Effect(FnSig<'hir>, EffectModifiers),
    EffectGroup(EffectGroupDef<'hir>),
    Associated(AssociatedDecl<'hir>),
    Struct(StructDef<'hir>),
    Enum(EnumDef<'hir>),
    Mod(ModDef),
    Impl(ImplDef<'hir>),
    Trait(TraitDef<'hir>),
    TypeAlias(
        &'hir Expr<'hir>,
        &'hir [ClauseParam<'hir>],
        &'hir [ClauseConstraint<'hir>],
    ),
    /// A nominal identity with an explicit representation edge, never an alias.
    Newtype(
        &'hir Expr<'hir>,
        &'hir [ClauseParam<'hir>],
        &'hir [ClauseConstraint<'hir>],
    ),
    Use(UsePath<'hir>),
    Const(&'hir Expr<'hir>, &'hir Expr<'hir>),
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EffectModifiers {
    pub is_async: bool,
    pub is_inline: bool,
    pub is_direct_passing: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EffectGroupDef<'hir> {
    pub operations: Vec<OwnerId>,
    pub clause_params: &'hir [ClauseParam<'hir>],
    pub clause_constraints: &'hir [ClauseConstraint<'hir>],
}

/// Full function signature: declaration + modifiers + clause parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct FnSig<'hir> {
    pub params: &'hir [FnParam<'hir>],
    pub return_ty: Option<&'hir Expr<'hir>>,
    pub return_bind: Option<Ident>,
    pub return_binding: Option<crate::HirId>,
    pub modifiers: FnModifiers,
    pub clause_params: &'hir [ClauseParam<'hir>],
    pub clause_constraints: &'hir [ClauseConstraint<'hir>],
    pub span: Span,
}

/// Function phase and modifier metadata; normal-form heads remain opaque.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FnModifiers {
    pub is_inline: bool,
    pub is_quote: bool,
    pub is_normal_form_head: bool,
    pub is_pure: bool,
    pub is_comptime: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NFSig<'hir> {
    // 偷懒就直接用FnParam了
    pub params: &'hir [FnParam<'hir>],
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
    pub configurations: Vec<Configuration<'hir>>,
    pub clause_params: &'hir [ClauseParam<'hir>],
    pub clause_constraints: &'hir [ClauseConstraint<'hir>],
    pub nested_items: Vec<OwnerId>,
}

/// Source-ordered metadata of an enclosing definition. These are not fields.
#[derive(Debug, Clone, PartialEq)]
pub struct Configuration<'hir> {
    pub ident: Ident,
    pub value: &'hir Expr<'hir>,
    pub span: Span,
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
    pub super_traits: Option<&'hir Expr<'hir>>,
    pub clause_params: &'hir [ClauseParam<'hir>],
    pub clause_constraints: &'hir [ClauseConstraint<'hir>],
    pub items: Vec<OwnerId>,
}

/// An associated value, including (but not restricted to) a value of Type.
#[derive(Debug, Clone, PartialEq)]
pub struct AssociatedDecl<'hir> {
    pub parameters: &'hir [FnParam<'hir>],
    pub ty: &'hir Expr<'hir>,
    pub default: Option<&'hir Expr<'hir>>,
    pub clause_params: &'hir [ClauseParam<'hir>],
    pub clause_constraints: &'hir [ClauseConstraint<'hir>],
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
    Group(&'hir [UsePath<'hir>]),
    Alias(Ident),
}

/// The kind of a definition — a coarser classification than [`ItemKind`]
/// that is useful when all you need to know is *what sort* of item a
/// [`LocalDefId`](crate::hir_id::LocalDefId) refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DefKind {
    Fn,
    Effect,
    EffectGroup,
    Struct,
    Enum,
    Mod,
    Impl,
    Trait,
    Associated,
    TypeAlias,
    Newtype,
    Use,
    Const,
    Invalid,
}

impl DefKind {
    pub fn from_item_kind(kind: &ItemKind<'_>) -> Self {
        match kind {
            ItemKind::Fn(..) | ItemKind::FnDeclaration(..) => DefKind::Fn,
            ItemKind::Effect(..) => DefKind::Effect,
            ItemKind::EffectGroup(..) => DefKind::EffectGroup,
            ItemKind::Struct(..) => DefKind::Struct,
            ItemKind::Enum(..) => DefKind::Enum,
            ItemKind::Mod(..) => DefKind::Mod,
            ItemKind::Impl(..) => DefKind::Impl,
            ItemKind::Trait(..) => DefKind::Trait,
            ItemKind::Associated(..) => DefKind::Associated,
            ItemKind::TypeAlias(..) => DefKind::TypeAlias,
            ItemKind::Newtype(..) => DefKind::Newtype,
            ItemKind::Use(..) => DefKind::Use,
            ItemKind::Const(..) => DefKind::Const,
            ItemKind::Invalid => DefKind::Invalid,
        }
    }
}
