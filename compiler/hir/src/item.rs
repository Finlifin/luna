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
    FunctionTemplateInstance {
        template: &'hir Expr<'hir>,
        /// The declaration-side signature, linked after all files are lowered.
        signature: Option<FnSig<'hir>>,
        explicit_parameters: Option<&'hir [crate::common::FnParam<'hir>]>,
        parameters: Vec<FunctionTemplateInput>,
        clause_bindings: Vec<FunctionTemplateClause>,
        modifiers: FnModifiers,
        body: BodyId,
    },
    /// Body ports have fresh instance-local identities. Their types and actual
    /// substitutions are checked when the selected template is expanded.
    TestTemplateInstance {
        template: &'hir Expr<'hir>,
        /// Filled after all source files have been lowered. This is the actual
        /// declaration interface, not a copy resolved in the instance scope.
        interface: Option<TestTemplateInterface<'hir>>,
        ports: Vec<TestTemplatePort>,
        body: BodyId,
    },
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
    Deriver(DeriverDef<'hir>),
    Derive(DeriveRequest<'hir>),
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

#[derive(Debug, Clone, PartialEq)]
pub struct TestTemplatePort {
    pub name: Ident,
    pub binding: HirId,
    pub formal: Option<HirId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionTemplateClause {
    pub formal: crate::common::DefId,
    pub binding: crate::common::DefId,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionTemplateInput {
    pub name: Ident,
    pub binding: HirId,
    pub formal: Option<HirId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TestTemplateInterface<'hir> {
    pub parameter: HirId,
    pub ty: &'hir Expr<'hir>,
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

impl<'hir> FnSig<'hir> {
    /// The body is supplied to the unique anchored clause input, independently
    /// of its spelling. Optional and collection inputs are separate roles.
    /// Type and trait-bound shorthand still denote anchored inputs; `quote`
    /// changes the parameter's interpretation, not its routing category.
    pub fn test_template_body(&self) -> Result<Option<&ClauseParam<'hir>>, &'static str> {
        if !self.modifiers.is_test_template {
            return Ok(None);
        }
        fn anchored(kind: &crate::ClauseParamKind<'_>) -> bool {
            use crate::ClauseParamKind as K;
            match kind {
                K::Type(_) | K::Positional(..) | K::TraitBound(..) => true,
                K::Optional(..) | K::Varadic(..) => false,
                K::Quote(inner) => anchored(inner),
            }
        }
        let mut inputs = self.clause_params.iter().filter(|p| anchored(&p.kind));
        let body = inputs
            .next()
            .ok_or("test template requires exactly one anchored body parameter")?;
        if inputs.next().is_some() {
            return Err("test template has more than one anchored parameter");
        }
        Ok(Some(body))
    }
}

/// Function phase and modifier metadata; normal-form heads remain opaque.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FnModifiers {
    pub is_test: bool,
    /// Contextual code template, not a runnable test or ordinary function.
    pub is_test_template: bool,
    /// Reusable signature/code definition, never an ordinary runtime entry.
    pub is_function_template: bool,
    pub is_inline: bool,
    pub is_quote: bool,
    pub is_normal_form_head: bool,
    pub is_pure: bool,
    pub is_comptime: bool,
    /// Declared unsafe call boundary, separate from purity and phase.
    pub is_unsafe: bool,
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

/// A named compile-time implementation generator, not a callable function.
#[derive(Debug, Clone, PartialEq)]
pub struct DeriverDef<'hir> {
    pub parameters: &'hir [FnParam<'hir>],
    pub trait_ref: &'hir Expr<'hir>,
    pub clause_params: &'hir [ClauseParam<'hir>],
    pub clause_constraints: &'hir [ClauseConstraint<'hir>],
    pub body: BodyId,
}

/// An unevaluated generation request; it supplies no implementation evidence.
#[derive(Debug, Clone, PartialEq)]
pub struct DeriveRequest<'hir> {
    pub phase: ImplementationPhase,
    pub trait_ref: &'hir Expr<'hir>,
    pub self_ty: &'hir Expr<'hir>,
    pub strategy: Option<&'hir Expr<'hir>>,
    pub clause_params: &'hir [ClauseParam<'hir>],
    pub clause_constraints: &'hir [ClauseConstraint<'hir>],
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
    /// Declares a family of pipeline operation labels, not an implementation.
    pub is_intrinsic: bool,
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

/// The phase of an implementation contribution, independent of member keywords.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImplementationPhase {
    #[default]
    Runtime,
    Comptime,
}

/// Definition body of an `impl` block (inherent or trait implementation).
#[derive(Debug, Clone, PartialEq)]
pub struct ImplDef<'hir> {
    /// Strategy applicability is checked against the full requested interface.
    pub generation_trait: Option<&'hir Expr<'hir>>,
    pub phase: ImplementationPhase,
    pub self_ty: &'hir Expr<'hir>,
    pub trait_ref: Option<&'hir Expr<'hir>>,
    pub clause_params: &'hir [ClauseParam<'hir>],
    pub clause_constraints: &'hir [ClauseConstraint<'hir>],
    pub items: Vec<OwnerId>,
}

/// Definition body of a `trait` item.
#[derive(Debug, Clone, PartialEq)]
pub struct TraitDef<'hir> {
    /// Interface membership is exclusively comptime; not function purity.
    pub comptime_only: bool,
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
    FunctionTemplateInstance,
    TestTemplateInstance,
    Effect,
    EffectGroup,
    Struct,
    Enum,
    Mod,
    Impl,
    Trait,
    Deriver,
    Derive,
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
            ItemKind::FunctionTemplateInstance { .. } => DefKind::FunctionTemplateInstance,
            ItemKind::TestTemplateInstance { .. } => DefKind::TestTemplateInstance,
            ItemKind::Effect(..) => DefKind::Effect,
            ItemKind::EffectGroup(..) => DefKind::EffectGroup,
            ItemKind::Struct(..) => DefKind::Struct,
            ItemKind::Enum(..) => DefKind::Enum,
            ItemKind::Mod(..) => DefKind::Mod,
            ItemKind::Impl(..) => DefKind::Impl,
            ItemKind::Trait(..) => DefKind::Trait,
            ItemKind::Deriver(..) => DefKind::Deriver,
            ItemKind::Derive(..) => DefKind::Derive,
            ItemKind::Associated(..) => DefKind::Associated,
            ItemKind::TypeAlias(..) => DefKind::TypeAlias,
            ItemKind::Newtype(..) => DefKind::Newtype,
            ItemKind::Use(..) => DefKind::Use,
            ItemKind::Const(..) => DefKind::Const,
            ItemKind::Invalid => DefKind::Invalid,
        }
    }
}
