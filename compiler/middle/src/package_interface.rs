//! Mid-end package interface, item signature, and elaboration query summaries.
//!
//! These owned, session-independent query results allow incremental queries
//! beyond `hir_package` to memoise package interfaces, per-definition
//! signatures, trait/impl inventories, and elaboration outcomes without holding
//! borrowed HIR arena references across query boundaries.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use hir::common::{DefId, Symbol};
use hir::item::ImplementationPhase;
use hir::{ItemKind, OwnerId, Package};
use rustc_span::Span;

fn write_escaped_str(f: &mut fmt::Formatter<'_>, s: &str) -> fmt::Result {
    f.write_str("\"")?;
    for ch in s.chars() {
        match ch {
            '"' => f.write_str("\\\"")?,
            '\\' => f.write_str("\\\\")?,
            '\n' => f.write_str("\\n")?,
            '\r' => f.write_str("\\r")?,
            '\t' => f.write_str("\\t")?,
            c if c.is_control() => write!(f, "\\u{{{:x}}}", c as u32)?,
            c => write!(f, "{c}")?,
        }
    }
    f.write_str("\"")
}

/// Compact structural summary of an [`hir::ItemKind`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemKindSummary {
    Mod {
        item_count: usize,
    },
    Fn {
        param_count: usize,
        clause_param_count: usize,
        is_comptime: bool,
        is_pure: bool,
        is_unsafe: bool,
        is_spec: bool,
        is_verified: bool,
        is_normal_form_head: bool,
        has_return_ty: bool,
    },
    Case {
        param_count: usize,
        clause_param_count: usize,
        has_return_ty: bool,
    },
    FnDeclaration {
        param_count: usize,
        clause_param_count: usize,
        is_function_template: bool,
        has_return_ty: bool,
    },
    FunctionTemplateInstance {
        parameter_count: usize,
        clause_count: usize,
        is_comptime: bool,
    },
    TestTemplateInstance {
        port_count: usize,
    },
    Effect {
        param_count: usize,
        clause_param_count: usize,
        is_async: bool,
        is_inline: bool,
        is_direct_passing: bool,
    },
    EffectGroup {
        operation_count: usize,
        clause_param_count: usize,
    },
    Associated {
        param_count: usize,
        clause_param_count: usize,
        has_default: bool,
    },
    Struct {
        field_count: usize,
        clause_param_count: usize,
        nested_count: usize,
    },
    Enum {
        variant_count: usize,
        clause_param_count: usize,
        is_intrinsic: bool,
        nested_count: usize,
    },
    Impl {
        item_count: usize,
        clause_param_count: usize,
        is_comptime: bool,
        has_trait_ref: bool,
    },
    Trait {
        item_count: usize,
        clause_param_count: usize,
        comptime_only: bool,
        has_super_traits: bool,
    },
    Deriver {
        param_count: usize,
        clause_param_count: usize,
    },
    Derive {
        is_comptime: bool,
        clause_param_count: usize,
        has_strategy: bool,
    },
    TypeAlias {
        clause_param_count: usize,
    },
    Newtype {
        clause_param_count: usize,
    },
    Use,
    Const,
    Static {
        has_init: bool,
    },
    Invalid,
}

impl ItemKindSummary {
    pub fn from_hir(kind: &ItemKind<'_>) -> Self {
        match kind {
            ItemKind::Mod(def) => Self::Mod {
                item_count: def.items.len(),
            },
            ItemKind::Fn(sig, _) => Self::Fn {
                param_count: sig.params.len(),
                clause_param_count: sig.clause_params.len(),
                is_comptime: sig.modifiers.is_comptime,
                is_pure: sig.modifiers.is_pure,
                is_unsafe: sig.modifiers.is_unsafe,
                is_spec: sig.modifiers.is_spec,
                is_verified: sig.modifiers.is_verified,
                is_normal_form_head: sig.modifiers.is_normal_form_head,
                has_return_ty: sig.return_ty.is_some(),
            },
            ItemKind::Case(sig, _) => Self::Case {
                param_count: sig.params.len(),
                clause_param_count: sig.clause_params.len(),
                has_return_ty: sig.return_ty.is_some(),
            },
            ItemKind::FnDeclaration(sig) => Self::FnDeclaration {
                param_count: sig.params.len(),
                clause_param_count: sig.clause_params.len(),
                is_function_template: sig.modifiers.is_function_template,
                has_return_ty: sig.return_ty.is_some(),
            },
            ItemKind::FunctionTemplateInstance {
                parameters,
                clause_bindings,
                modifiers,
                ..
            } => Self::FunctionTemplateInstance {
                parameter_count: parameters.len(),
                clause_count: clause_bindings.len(),
                is_comptime: modifiers.is_comptime,
            },
            ItemKind::TestTemplateInstance { ports, .. } => Self::TestTemplateInstance {
                port_count: ports.len(),
            },
            ItemKind::Effect(sig, modifiers) => Self::Effect {
                param_count: sig.params.len(),
                clause_param_count: sig.clause_params.len(),
                is_async: modifiers.is_async,
                is_inline: modifiers.is_inline,
                is_direct_passing: modifiers.is_direct_passing,
            },
            ItemKind::EffectGroup(def) => Self::EffectGroup {
                operation_count: def.operations.len(),
                clause_param_count: def.clause_params.len(),
            },
            ItemKind::Associated(decl) => Self::Associated {
                param_count: decl.parameters.len(),
                clause_param_count: decl.clause_params.len(),
                has_default: decl.default.is_some(),
            },
            ItemKind::Struct(def) => Self::Struct {
                field_count: def.fields.len(),
                clause_param_count: def.clause_params.len(),
                nested_count: def.nested_items.len(),
            },
            ItemKind::Enum(def) => Self::Enum {
                variant_count: def.variants.len(),
                clause_param_count: def.clause_params.len(),
                is_intrinsic: def.is_intrinsic,
                nested_count: def.nested_items.len(),
            },
            ItemKind::Impl(def) => Self::Impl {
                item_count: def.items.len(),
                clause_param_count: def.clause_params.len(),
                is_comptime: matches!(def.phase, ImplementationPhase::Comptime),
                has_trait_ref: def.trait_ref.is_some(),
            },
            ItemKind::Trait(def) => Self::Trait {
                item_count: def.items.len(),
                clause_param_count: def.clause_params.len(),
                comptime_only: def.comptime_only,
                has_super_traits: def.super_traits.is_some(),
            },
            ItemKind::Deriver(def) => Self::Deriver {
                param_count: def.parameters.len(),
                clause_param_count: def.clause_params.len(),
            },
            ItemKind::Derive(req) => Self::Derive {
                is_comptime: matches!(req.phase, ImplementationPhase::Comptime),
                clause_param_count: req.clause_params.len(),
                has_strategy: req.strategy.is_some(),
            },
            ItemKind::TypeAlias(_, clause_params, _) => Self::TypeAlias {
                clause_param_count: clause_params.len(),
            },
            ItemKind::Newtype(_, clause_params, _) => Self::Newtype {
                clause_param_count: clause_params.len(),
            },
            ItemKind::Use(_) => Self::Use,
            ItemKind::Const(..) => Self::Const,
            ItemKind::Static(_, init) => Self::Static {
                has_init: init.is_some(),
            },
            ItemKind::Invalid => Self::Invalid,
        }
    }
}

impl fmt::Display for ItemKindSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mod { item_count } => write!(f, "(mod (items {item_count}))"),
            Self::Fn {
                param_count,
                clause_param_count,
                is_comptime,
                is_pure,
                is_unsafe,
                is_spec,
                is_verified,
                is_normal_form_head,
                has_return_ty,
            } => write!(
                f,
                "(fn (params {param_count}) (clauses {clause_param_count}) (comptime {is_comptime}) (pure {is_pure}) (unsafe {is_unsafe}) (spec {is_spec}) (verified {is_verified}) (nf-head {is_normal_form_head}) (has-return {has_return_ty}))"
            ),
            Self::Case {
                param_count,
                clause_param_count,
                has_return_ty,
            } => write!(
                f,
                "(case (params {param_count}) (clauses {clause_param_count}) (has-return {has_return_ty}))"
            ),
            Self::FnDeclaration {
                param_count,
                clause_param_count,
                is_function_template,
                has_return_ty,
            } => write!(
                f,
                "(fn-decl (params {param_count}) (clauses {clause_param_count}) (template {is_function_template}) (has-return {has_return_ty}))"
            ),
            Self::FunctionTemplateInstance {
                parameter_count,
                clause_count,
                is_comptime,
            } => write!(
                f,
                "(fn-template-instance (params {parameter_count}) (clauses {clause_count}) (comptime {is_comptime}))"
            ),
            Self::TestTemplateInstance { port_count } => {
                write!(f, "(test-template-instance (ports {port_count}))")
            }
            Self::Effect {
                param_count,
                clause_param_count,
                is_async,
                is_inline,
                is_direct_passing,
            } => write!(
                f,
                "(effect (params {param_count}) (clauses {clause_param_count}) (async {is_async}) (inline {is_inline}) (direct-passing {is_direct_passing}))"
            ),
            Self::EffectGroup {
                operation_count,
                clause_param_count,
            } => write!(
                f,
                "(effect-group (ops {operation_count}) (clauses {clause_param_count}))"
            ),
            Self::Associated {
                param_count,
                clause_param_count,
                has_default,
            } => write!(
                f,
                "(associated (params {param_count}) (clauses {clause_param_count}) (has-default {has_default}))"
            ),
            Self::Struct {
                field_count,
                clause_param_count,
                nested_count,
            } => write!(
                f,
                "(struct (fields {field_count}) (clauses {clause_param_count}) (nested {nested_count}))"
            ),
            Self::Enum {
                variant_count,
                clause_param_count,
                is_intrinsic,
                nested_count,
            } => write!(
                f,
                "(enum (variants {variant_count}) (clauses {clause_param_count}) (intrinsic {is_intrinsic}) (nested {nested_count}))"
            ),
            Self::Impl {
                item_count,
                clause_param_count,
                is_comptime,
                has_trait_ref,
            } => write!(
                f,
                "(impl (items {item_count}) (clauses {clause_param_count}) (comptime {is_comptime}) (has-trait {has_trait_ref}))"
            ),
            Self::Trait {
                item_count,
                clause_param_count,
                comptime_only,
                has_super_traits,
            } => write!(
                f,
                "(trait (items {item_count}) (clauses {clause_param_count}) (comptime-only {comptime_only}) (has-supers {has_super_traits}))"
            ),
            Self::Deriver {
                param_count,
                clause_param_count,
            } => write!(
                f,
                "(deriver (params {param_count}) (clauses {clause_param_count}))"
            ),
            Self::Derive {
                is_comptime,
                clause_param_count,
                has_strategy,
            } => write!(
                f,
                "(derive (comptime {is_comptime}) (clauses {clause_param_count}) (has-strategy {has_strategy}))"
            ),
            Self::TypeAlias { clause_param_count } => {
                write!(f, "(typealias (clauses {clause_param_count}))")
            }
            Self::Newtype { clause_param_count } => {
                write!(f, "(newtype (clauses {clause_param_count}))")
            }
            Self::Use => f.write_str("(use)"),
            Self::Const => f.write_str("(const)"),
            Self::Static { has_init } => write!(f, "(static (has-init {has_init}))"),
            Self::Invalid => f.write_str("(invalid)"),
        }
    }
}

/// Owned interface/signature summary for a single HIR owner definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemInterfaceSummary {
    pub owner: OwnerId,
    pub name: Symbol,
    pub span: Span,
    pub def_id: Option<DefId>,
    pub kind: ItemKindSummary,
}

impl fmt::Display for ItemInterfaceSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(item (owner {}) (name ", self.owner)?;
        write_escaped_str(f, self.name.as_str())?;
        write!(f, ") ")?;
        if let Some(def_id) = self.def_id {
            write!(f, "(def-id ({} {})) ", def_id.pkg, def_id.index)?;
        }
        write!(
            f,
            "(span {}..{}) {})",
            self.span.lo().0,
            self.span.hi().0,
            self.kind
        )
    }
}

/// Owned summary of a lowered HIR package's definitions and resolver bindings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageInterface {
    pub root_mod: OwnerId,
    pub items: BTreeMap<OwnerId, Arc<ItemInterfaceSummary>>,
    pub by_def_id: BTreeMap<DefId, OwnerId>,
    pub pending_semantics_count: usize,
}

impl PackageInterface {
    /// Extract an owned interface summary from a lowered [`hir::Package`].
    pub fn from_hir(package: &Package<'_>) -> Self {
        let mut items = BTreeMap::new();
        let mut by_def_id = BTreeMap::new();
        for (owner, info) in package.owners() {
            let item = info.node.expect_item();
            let def_id = package.owner_definition(owner);
            if let Some(def_id) = def_id {
                by_def_id.insert(def_id, owner);
            }
            items.insert(
                owner,
                Arc::new(ItemInterfaceSummary {
                    owner,
                    name: item.ident.name,
                    span: item.span,
                    def_id,
                    kind: ItemKindSummary::from_hir(&item.kind),
                }),
            );
        }
        Self {
            root_mod: package.root_mod,
            items,
            by_def_id,
            pending_semantics_count: package.pending_semantics.len(),
        }
    }

    pub fn get(&self, owner: OwnerId) -> Option<&Arc<ItemInterfaceSummary>> {
        self.items.get(&owner)
    }

    pub fn owner_of(&self, def_id: DefId) -> Option<OwnerId> {
        self.by_def_id.get(&def_id).copied()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl fmt::Display for PackageInterface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(package-interface (root {}) (pending-semantics {}) (items",
            self.root_mod, self.pending_semantics_count
        )?;
        for item in self.items.values() {
            write!(f, " {item}")?;
        }
        f.write_str("))")
    }
}

/// Trait and implementation index extracted from HIR and name resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageTraitImpls {
    pub traits: Vec<OwnerId>,
    pub impls: Vec<OwnerId>,
    pub resolved_impls: usize,
}

impl PackageTraitImpls {
    pub fn from_interface(interface: &PackageInterface, resolved_impls: usize) -> Self {
        let mut traits = Vec::new();
        let mut impls = Vec::new();
        for (owner, summary) in &interface.items {
            match summary.kind {
                ItemKindSummary::Trait { .. } => traits.push(*owner),
                ItemKindSummary::Impl { .. } => impls.push(*owner),
                _ => {}
            }
        }
        Self {
            traits,
            impls,
            resolved_impls,
        }
    }
}

impl fmt::Display for PackageTraitImpls {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(package-trait-impls (traits")?;
        for id in &self.traits {
            write!(f, " {id}")?;
        }
        write!(f, ") (impls")?;
        for id in &self.impls {
            write!(f, " {id}")?;
        }
        write!(f, ") (resolved-impls {}))", self.resolved_impls)
    }
}

/// Configuration inputs for the `elaborated_package` mid-end query.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ElaborationQueryInput {
    pub package_identities: BTreeMap<u32, u64>,
    pub target_triple: Option<String>,
    pub pointer_width_bits: Option<u8>,
    pub build_modes: BTreeMap<u64, String>,
    pub emit_warnings: bool,
}

impl ElaborationQueryInput {
    pub fn new(package_identities: BTreeMap<u32, u64>) -> Self {
        Self {
            package_identities,
            ..Default::default()
        }
    }
}

/// Owned mid-end elaboration query result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElaboratedPackage {
    pub succeeded: bool,
    pub struct_count: usize,
    pub enum_count: usize,
    pub function_count: usize,
    pub trait_count: usize,
    pub impl_count: usize,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
    pub nir_lisp: String,
}

impl fmt::Display for ElaboratedPackage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(elaborated-package (succeeded {}) (structs {}) (enums {}) (functions {}) (traits {}) (impls {}) (warnings {}) (errors {}))",
            self.succeeded,
            self.struct_count,
            self.enum_count,
            self.function_count,
            self.trait_count,
            self.impl_count,
            self.warnings.len(),
            self.errors.len()
        )
    }
}
