//! HIR to typed NIR with a first-order function-phase boundary.
//! Unsupported semantic protocols fail explicitly. Bound Splice calls preserve
//! the boundary between comptime producers and runtime conversion instances.
//! Resource/effect verification remains a separate obligation;
//! this is a type-elaboration output, not permission to generate executable code.
mod arguments;
pub mod attributes;
mod computed_patterns;
mod control;
mod delivered_types;
mod effects;
mod error_handlers;
mod expression_values;
mod extended;
mod function_templates;
mod function_types;
mod inline_control;
mod inline_loops;
pub mod inlining;
mod intrinsics;
mod spreads;
mod tactics;
use arguments::route_arguments;
mod bit_indices;
pub mod build_context;
mod builtin_bindings;
mod cmtt;
mod constants;
mod defaults;
mod dependent_inputs;
mod derivation;
mod dynamic_instances;
mod enum_groups;
mod interfaces;
mod iteration;
mod lexical_inputs;
mod list_operations;
mod list_patterns;
mod literal_conversion;
mod nominal;
mod nominal_encoding;
mod normal;
mod open_functions;
mod operators;
mod places;
mod projections;
mod purity;
mod refinements;
mod required;
mod splicing;
mod text_queries;
mod trait_inputs;
mod type_defaults;
use crate::verification::purity::{copy_scalar, stable_default};
pub(crate) use cmtt::{expand_bound_quote, expand_materialized_quotes};
use hir::common::{Arg, BinOp, TyParamKind, UnOp};
use hir::{Expr, ExprKind as H, ItemKind, OwnerId};
use nir::program::{self as n, ExprKind as K};
use nir::{DeclarationId, Parameter, Primitive as P, Sort, Term};
use rustc_span::Span;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug)]
pub struct DiagnosticNote {
    pub source: n::SourceRange,
    pub message: String,
}

#[derive(Debug)]
pub struct Error {
    pub source: n::SourceRange,
    pub message: String,
    pub notes: Vec<DiagnosticNote>,
    needs_context: bool,
    needs_tactic: bool,
    needs_capability: bool,
    incompatible_context: bool,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}..{}: {}",
            self.source.start, self.source.end, self.message
        )?;
        for note in &self.notes {
            write!(
                f,
                "\n  note: {}..{}: {}",
                note.source.start, note.source.end, note.message
            )?;
        }
        Ok(())
    }
}
impl std::error::Error for Error {}
fn source(span: Span) -> n::SourceRange {
    n::SourceRange {
        start: span.lo().0,
        end: span.hi().0,
    }
}
fn error(span: Span, message: impl Into<String>) -> Error {
    Error {
        source: source(span),
        message: message.into(),
        notes: Vec::new(),
        needs_context: false,
        needs_tactic: false,
        needs_capability: false,
        incompatible_context: false,
    }
}
// Only missing inference inputs are resumable. Semantic errors and solver
// exhaustion remain errors, never alternative evidence for another candidate.
fn context_error(span: Span, message: impl Into<String>) -> Error {
    Error {
        needs_context: true,
        ..error(span, message)
    }
}
// A known expression shape cannot inhabit the supplied context. This is
// distinct from missing inference inputs and from semantic/budget failures.
fn incompatible_context(span: Span, message: impl Into<String>) -> Error {
    Error {
        incompatible_context: true,
        ..error(span, message)
    }
}
fn call_inference_error(span: Span, cause: crate::calls::Error) -> Error {
    let pending = matches!(
        cause,
        crate::calls::Error::NeedsContext
            | crate::calls::Error::Obligations(crate::obligations::Error::Inference(
                crate::InferenceError::Unresolved(_)
            ))
    );
    Error {
        needs_context: pending,
        ..error(span, format!("generic call inference failed: {cause}"))
    }
}
fn p(value: P) -> Term {
    Term::Primitive(value)
}
fn local(id: hir::HirId) -> n::LocalId {
    n::LocalId {
        owner: id.owner.def_id.raw(),
        index: id.local_id.raw(),
    }
}
fn semantic_package(identities: &BTreeMap<u32, u64>, source: u32) -> u64 {
    identities
        .get(&source)
        .copied()
        .unwrap_or(u64::from(source))
}
fn declaration(
    package: &hir::Package<'_>,
    identities: &BTreeMap<u32, u64>,
    owner: OwnerId,
) -> DeclarationId {
    let definition = package.owner_definition(owner);
    DeclarationId {
        package: semantic_package(identities, definition.map_or(0, |id| id.pkg)),
        // Clause parameters already use resolver identities. Keep declarations
        // in that same namespace; dense HIR owners can collide with parameters.
        local: definition.map_or(owner.def_id.raw(), |id| id.index),
    }
}
fn is_never(ty: &Term) -> bool {
    *ty == p(P::Never)
}
/// Imports own HIR syntax, but never introduce a semantic declaration.
/// Do not interpret their dense owner index as a resolver declaration index.
fn matches_declaration(
    package: &hir::Package<'_>,
    identities: &BTreeMap<u32, u64>,
    owner: OwnerId,
    id: DeclarationId,
) -> bool {
    !matches!(
        package.item(owner).map(|item| &item.kind),
        Some(ItemKind::Use(_))
    ) && declaration(package, identities, owner) == id
}
fn discard_result(value: n::Expr) -> n::Expr {
    if is_never(&value.ty) || value.ty == p(P::Void) {
        return value;
    }
    n::Expr {
        ty: p(P::Void),
        source: value.source,
        kind: K::Discard(Box::new(value)),
    }
}

pub fn elaborate(package: &hir::Package<'_>) -> Result<n::Package, Error> {
    elaborate_in_package(package, 0)
}

/// The driver supplies a stable build/package identity. HIR owner IDs remain
/// local; semantic declarations and rigids carry this identity from creation.
pub fn elaborate_in_package(
    package: &hir::Package<'_>,
    package_identity: u64,
) -> Result<n::Package, Error> {
    elaborate_packages(package, &BTreeMap::from([(0, package_identity)]))
}

/// Elaborate a combined HIR collection while preserving each owner's package.
pub fn elaborate_packages(
    package: &hir::Package<'_>,
    package_identities: &BTreeMap<u32, u64>,
) -> Result<n::Package, Error> {
    elaborate_packages_with_protocols(package, package_identities, &Protocols::default())
}

/// Trusted, package-qualified language-item identities supplied by the driver.
/// Source names never designate these protocols implicitly.
#[derive(Clone, Debug, Default)]
pub struct Protocols {
    /// Independent limit for final package dispatch preparation.
    pub dispatch_fuel: Option<usize>,
    /// Explicit authority to implement compiler builtin types, by semantic package ID.
    pub builtin_implementation_packages: BTreeSet<u64>,
    /// Explicit build modes by source semantic package identity. Empty means
    /// unavailable, never an implicit host/debug configuration.
    pub build_modes: BTreeMap<u64, String>,
    /// Host pipeline targets selected by the originating source package.
    pub build_targets: BTreeMap<u64, build_context::Target>,
    /// Immutable source-qualified dependency metadata supplied by the driver.
    pub build_metadata: Option<std::sync::Arc<build_context::Metadata>>,
    /// Explicit target operation declaration and variant, resolved by the driver.
    pub host_fma: Option<(DeclarationId, String)>,
    pub host_sqrt: Option<(DeclarationId, String)>,
    pub host_copysign: Option<(DeclarationId, String)>,
    pub comptime_intrinsics: Vec<((DeclarationId, String), nir::intrinsics::Operation)>,
    pub host_intrinsics: Vec<((DeclarationId, String), nir::intrinsics::Operation)>,
    pub copy: Option<DeclarationId>,
    pub drop: Option<DeclarationId>,
    pub index: Option<DeclarationId>,
    pub iterator: Option<DeclarationId>,
    pub into_iterator: Option<DeclarationId>,
    pub ordering: Option<DeclarationId>,
    pub equality: Option<DeclarationId>,
    pub list_pattern: Option<DeclarationId>,
    pub literal_into: Option<DeclarationId>,
    pub arithmetic: [Option<DeclarationId>; 7],
    pub splice: Option<DeclarationId>,
    pub meta_list: Option<DeclarationId>,
    pub tuple: Option<DeclarationId>,
}
impl Protocols {
    pub fn host_bindings(
        &self,
    ) -> impl Iterator<Item = (&(DeclarationId, String), nir::intrinsics::Operation)> {
        use nir::intrinsics::Operation;
        [
            (&self.host_fma, Operation::FusedMultiplyAdd),
            (&self.host_sqrt, Operation::SquareRoot),
            (&self.host_copysign, Operation::CopySign),
        ]
        .into_iter()
        .filter_map(|(binding, operation)| binding.as_ref().map(|binding| (binding, operation)))
        .chain(
            self.host_intrinsics
                .iter()
                .map(|(binding, operation)| (binding, *operation)),
        )
    }
}

pub fn elaborate_packages_with_protocols(
    package: &hir::Package<'_>,
    package_identities: &BTreeMap<u32, u64>,
    protocols: &Protocols,
) -> Result<n::Package, Error> {
    elaborate_packages_for_target(package, package_identities, protocols, None)
}

/// Target facts are invocation input and remain attached to the resulting NIR.
pub fn elaborate_packages_for_target(
    package: &hir::Package<'_>,
    package_identities: &BTreeMap<u32, u64>,
    protocols: &Protocols,
    target: Option<nir::target::TargetData>,
) -> Result<n::Package, Error> {
    elaborate_packages_with_invocation_budget(
        package,
        package_identities,
        protocols,
        target,
        &mut 1_000_000,
    )
}

/// Share required comptime invocation/preparation fuel across elaborations.
/// This is not a budget for PTC, template expansion or every semantic pass.
/// Errors preserve consumed work; no fresh invocation allowance is created.
pub fn elaborate_packages_with_invocation_budget(
    package: &hir::Package<'_>,
    package_identities: &BTreeMap<u32, u64>,
    protocols: &Protocols,
    target: Option<nir::target::TargetData>,
    invocation_steps: &mut u64,
) -> Result<n::Package, Error> {
    elaborate_with_attributes(
        package,
        package_identities,
        protocols,
        target,
        false,
        invocation_steps,
    )
    .map(|(program, _)| program)
}

fn elaborate_with_attributes(
    package: &hir::Package<'_>,
    package_identities: &BTreeMap<u32, u64>,
    protocols: &Protocols,
    target: Option<nir::target::TargetData>,
    inspect_attributes: bool,
    invocation_steps: &mut u64,
) -> Result<(n::Package, Vec<attributes::EvaluatedAttribute>), Error> {
    if package
        .pending_semantics
        .iter()
        .any(|(_, feature)| feature == "function template expansion")
    {
        let arena = hir::HirArena::new();
        let expanded = function_templates::expand(package, &arena)?;
        return elaborate_with_attributes(
            &expanded,
            package_identities,
            protocols,
            target,
            inspect_attributes,
            invocation_steps,
        );
    }
    if !inspect_attributes
        && package
            .pending_semantics
            .iter()
            .any(|(_, feature)| feature == "derivation expansion")
    {
        let arena = hir::HirArena::new();
        let expanded = derivation::expand(package, &arena, &mut 100_000)
            .map_err(|failure| derivation::with_context(package, failure))?;
        return elaborate_with_attributes(
            &expanded,
            package_identities,
            protocols,
            target,
            inspect_attributes,
            invocation_steps,
        )
        .map_err(|failure| derivation::with_context(package, failure));
    }
    if let Some((span, feature)) = package.pending_semantics.iter().find(|(_, feature)| {
        feature != "test template CMTT expansion"
            && !(inspect_attributes && feature == "derivation expansion")
            && !matches!(
                feature.as_str(),
                "definition attribute" | "definition configuration"
            )
    }) {
        return Err(error(
            *span,
            format!(
                "elaboration is not implemented for {}",
                if feature == "definition attribute" {
                    "definition attribute or modifier"
                } else {
                    feature
                }
            ),
        ));
    }
    if !inspect_attributes && package.definition_attributes.is_empty() {
        if let Some((span, feature)) = package.pending_semantics.iter().find(|(_, feature)| {
            matches!(
                feature.as_str(),
                "definition attribute" | "definition configuration"
            )
        }) {
            return Err(error(
                *span,
                format!("elaboration is not implemented for {feature} without structured metadata"),
            ));
        }
    }
    let mut checker = Checker {
        next_definition_identity: package
            .next_definition_index()
            .max(u32::try_from(package.num_defs()).unwrap_or(u32::MAX)),
        delivered_types: BTreeMap::new(),
        target,
        protocols: protocols.clone(),
        comptime_callee_result: None,
        structural_result_context: None,
        package_identities: package_identities.clone(),
        package,
        effect_parents: package
            .owners()
            .flat_map(|(owner, info)| match &info.node.expect_item().kind {
                ItemKind::EffectGroup(group) => group
                    .operations
                    .iter()
                    .map(|operation| (*operation, owner))
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            })
            .collect(),
        signatures: BTreeMap::new(),
        open_functions: BTreeMap::new(),
        active_open_definitions: BTreeSet::new(),
        open_instances: std::collections::HashMap::new(),
        next_open_instance: 0,
        open_instantiation_depth: 0,
        generated_functions: Vec::new(),
        generated_function_parameters: BTreeMap::new(),
        ordinary_behavior_functions: Vec::new(),
        checking_open_behavior: false,
        checking_default_behavior: false,
        default_behavior_functions: BTreeSet::new(),
        describing_open_spread: false,
        describing_open_call: false,
        ordinary_behavior_inputs: BTreeMap::new(),
        open_loop_transfers: BTreeMap::new(),
        pending_default_stability: Vec::new(),
        specialization_inputs: Vec::new(),
        active_signatures: BTreeSet::new(),
        refinement_subjects: Vec::new(),
        pending_refinements: Vec::new(),
        executable_refinement_owner: None,
        interface_signatures: BTreeMap::new(),
        self_types: BTreeMap::new(),
        self_containers: BTreeMap::new(),
        interfaces_ready: false,
        member_phases: BTreeMap::new(),
        active_early_inherent: BTreeSet::new(),
        checking_method_contract: false,
        pending_contract_projections: Vec::new(),
        checked_traits: Vec::new(),
        checked_implementations: Vec::new(),
        associated_initializers: BTreeMap::new(),
        implementation_relationships: BTreeMap::new(),
        complementary_associated: Vec::new(),
        active_associated_values: BTreeSet::new(),
        selected_associated_implementation: None,
        active_associated_signatures: BTreeSet::new(),
        declaration_projection_depth: 0,
        associated_definition_equations: BTreeMap::new(),
        associated_signature_values: BTreeMap::new(),
        implementation_index: crate::implementation::Index::for_builtin_packages(
            target,
            protocols.builtin_implementation_packages.iter().copied(),
        ),
        trait_parameter_types: BTreeMap::new(),
        nominal_input_prefixes: BTreeMap::new(),
        inherent_index: crate::implementation::InherentIndex::for_builtin_packages(
            target,
            protocols.builtin_implementation_packages.iter().copied(),
        ),
        type_parameters: BTreeMap::new(),
        normal_parameters: BTreeMap::new(),
        normal_input_prefixes: BTreeMap::new(),
        type_input_bindings: BTreeMap::new(),
        partial_staged_bindings: BTreeMap::new(),
        lexical_input_domains: BTreeMap::new(),
        callable_type_inputs: BTreeMap::new(),
        callable_value_inputs: BTreeMap::new(),
        value_input_bindings: BTreeMap::new(),
        derived_staged_bindings: BTreeSet::new(),
        open_staged_locals: BTreeSet::new(),
        open_member_locals: BTreeSet::new(),
        signature_prefixes: BTreeMap::new(),
        signature_domains: BTreeMap::new(),
        default_input_scope: None,
        default_input_owner: None,
        requirements: BTreeMap::new(),
        program_requirements: BTreeMap::new(),
        program_postconditions: BTreeMap::new(),
        old_scope: None,
        tactic_frame: None,
        capabilities: BTreeMap::new(),
        pending_type_uses: Some(Vec::new()),
        signature_type_uses: Vec::new(),
        checking_contract: false,
        numeric_defaults: true,
        signature_owner: None,
        context: crate::ptc::Context::default(),
        defaults: BTreeMap::new(),
        locals: BTreeMap::new(),
        code_values: BTreeMap::new(),
        template_code_values: BTreeMap::new(),
        pattern_codes: BTreeMap::new(),
        pattern_quote_pending: None,
        pattern_generators: BTreeMap::new(),
        code_generators: BTreeMap::new(),
        data_consumers: BTreeMap::new(),
        data_consumer_depth: 0,
        code_method_captures: BTreeMap::new(),
        code_method_targets: BTreeMap::new(),
        code_packet_nominals: BTreeSet::new(),
        computed_scalar_packets: BTreeSet::new(),
        generator_data: BTreeMap::new(),
        generator_cells: BTreeMap::new(),
        checking_code_generator: false,
        abstract_code_dependencies: BTreeMap::new(),
        in_code_body: false,
        inline_selection: None,
        inline_match_input: None,
        in_unselected_inline: false,
        quote_holes: BTreeSet::new(),
        code_construction_depth: 0,
        expansion_sources: Vec::new(),
        aliases: BTreeSet::new(),
        completed_aliases: std::collections::HashMap::new(),
        checked_alias_definitions: BTreeSet::new(),
        alias_substitution: BTreeMap::new(),
        alias_value_bindings: BTreeMap::new(),
        normal_default_bindings: BTreeMap::new(),
        forbidden_default_inputs: BTreeSet::new(),
        next_temporary_binding: 0,
        alias_expression_budget: 16_384,
        alias_scopes: BTreeMap::new(),
        constants: BTreeMap::new(),
        active_constants: BTreeSet::new(),
        active_nominals: BTreeSet::new(),
        active_required: BTreeSet::new(),
        active_contracts: BTreeSet::new(),
        active_function_defaults: BTreeSet::new(),
        active_type_defaults: BTreeSet::new(),
        default_uses: BTreeMap::new(),
        contract_uses: BTreeMap::new(),
        required_steps: *invocation_steps,
        in_required_comptime: false,
        in_staged_input: false,
        preparing_spread: None,
        pending_spread_inputs: BTreeMap::new(),
        observing_type: false,
        result: None,
        partial_return_context: None,
        pending_optional_returns: BTreeMap::new(),
        pending_error_returns: BTreeMap::new(),
        inferred_return_sites: BTreeSet::new(),
        optional_branch_probe_budget: 100_000,
        probing_optional_branch: false,
        optional_target_probe: None,
        effect_environment: None,
        effect_resume_result: None,
        preserve_effect_value: false,
        captures: Vec::new(),
        return_observations: None,
        loops: Vec::new(),
        structs: BTreeMap::new(),
        newtypes: BTreeMap::new(),
        alias_contracts: BTreeMap::new(),
        enums: BTreeMap::new(),
        enum_sources: BTreeMap::new(),
        field_defaults: BTreeMap::new(),
        active_defaults: BTreeSet::new(),
        immutable: BTreeSet::new(),
    };
    // Account even on unwinding, then preserve the original panic. Candidate
    // state remains private; this does not recover or reuse a panicking checker.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        elaborate_candidate(
            &mut checker,
            package,
            package_identities,
            target,
            inspect_attributes,
        )
    }));
    *invocation_steps = checker.required_steps;
    match result {
        Ok(result) => result,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

fn elaborate_candidate<'a, 'h>(
    checker: &mut Checker<'a, 'h>,
    package: &'a hir::Package<'h>,
    package_identities: &BTreeMap<u32, u64>,
    target: Option<nir::target::TargetData>,
    inspect_attributes: bool,
) -> Result<(n::Package, Vec<attributes::EvaluatedAttribute>), Error> {
    // Register all declaration telescopes before interpreting interface arguments.
    for (owner, info) in package.owners() {
        if matches!(&info.node.expect_item().kind, ItemKind::Fn(sig, _) if sig.modifiers.is_test_template)
            || matches!(
                info.node.expect_item().kind,
                ItemKind::TestTemplateInstance { .. }
            )
        {
            checker.type_parameters.insert(owner, Vec::new());
            continue;
        }
        let clauses = match &info.node.expect_item().kind {
            ItemKind::TypeAlias(_, parameters, _) | ItemKind::Newtype(_, parameters, _) => {
                *parameters
            }
            ItemKind::Struct(def) => def.clause_params,
            ItemKind::Enum(def) => def.clause_params,
            ItemKind::Impl(def) => def.clause_params,
            ItemKind::Trait(def) => def.clause_params,
            ItemKind::Associated(def) => def.clause_params,
            ItemKind::Effect(sig, _) => sig.clause_params,
            ItemKind::EffectGroup(group) => group.clause_params,
            ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig) => sig.clause_params,
            _ => continue,
        };
        let parameters = clauses
            .iter()
            .filter_map(|p| match p.kind {
                hir::clause::ClauseParamKind::Type(_)
                | hir::clause::ClauseParamKind::TraitBound(..)
                | hir::clause::ClauseParamKind::Optional(..) => {
                    p.definition.map(|id| DeclarationId {
                        package: semantic_package(package_identities, id.pkg),
                        local: id.index,
                    })
                }
                hir::clause::ClauseParamKind::Positional(..)
                    if matches!(
                        info.node.expect_item().kind,
                        ItemKind::TypeAlias(..)
                            | ItemKind::Newtype(..)
                            | ItemKind::Struct(_)
                            | ItemKind::Enum(_)
                            | ItemKind::Impl(_)
                            | ItemKind::Trait(_)
                            | ItemKind::EffectGroup(_)
                    ) || matches!(
                        &info.node.expect_item().kind,
                        ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)
                            if sig.return_ty.is_some_and(|ty| matches!(ty.kind, H::ContextualExprType(..) | H::ContextualPatternType(..)))
                    ) || checker.is_data_consumer(owner) =>
                {
                    p.definition.map(|id| DeclarationId {
                        package: semantic_package(package_identities, id.pkg),
                        local: id.index,
                    })
                }
                _ => None,
            })
            .collect();
        checker.type_parameters.insert(owner, parameters);
    }
    checker.prepare_self_types()?;
    // Collect every signature before bodies: ordinary recursion does not wait
    // for a body result and declaration order does not change lookup.
    // Register generic nominal telescopes before reading any signature or field,
    // so forward references use the same rigid declaration identities.
    for (owner, info) in package.owners() {
        checker.signature_owner = Some(owner);
        let clauses = match &info.node.expect_item().kind {
            ItemKind::TypeAlias(_, parameters, _) | ItemKind::Newtype(_, parameters, _) => {
                Some(*parameters)
            }
            ItemKind::Struct(def) => Some(def.clause_params),
            ItemKind::Enum(def) => Some(def.clause_params),
            ItemKind::Impl(def) => Some(def.clause_params),
            ItemKind::Trait(def) => Some(def.clause_params),
            ItemKind::EffectGroup(group) => Some(group.clause_params),
            _ => None,
        };
        if let Some(clauses) = clauses {
            let mut parameters = Vec::new();
            let mut trait_domains = Vec::new();
            for parameter in clauses {
                let trait_domain = if matches!(
                    info.node.expect_item().kind,
                    ItemKind::Trait(_)
                        | ItemKind::Struct(_)
                        | ItemKind::Enum(_)
                        | ItemKind::Newtype(..)
                        | ItemKind::TypeAlias(..)
                        | ItemKind::Impl(_)
                        | ItemKind::EffectGroup(_)
                ) {
                    match parameter.kind {
                        hir::clause::ClauseParamKind::Positional(_, ty)
                        | hir::clause::ClauseParamKind::Optional(_, ty, _) => {
                            Some(checker.type_expr(ty)?)
                        }
                        _ => Some(p(P::Type)),
                    }
                } else {
                    None
                };
                if let Some(domain) = &trait_domain {
                    trait_domains.push(domain.clone());
                }
                let optional_type_parameter =
                    if let hir::clause::ClauseParamKind::Optional(_, ty, _) = parameter.kind {
                        matches!(
                            info.node.expect_item().kind,
                            ItemKind::Trait(_)
                                | ItemKind::Struct(_)
                                | ItemKind::Enum(_)
                                | ItemKind::TypeAlias(..)
                                | ItemKind::Newtype(..)
                        ) && checker.type_expr(ty)? == p(P::Type)
                    } else {
                        false
                    };
                let explicit_type_parameter =
                    if let hir::clause::ClauseParamKind::Positional(_, ty) = parameter.kind {
                        checker.type_expr(ty)? == p(P::Type)
                    } else {
                        false
                    };
                if !explicit_type_parameter
                    && !optional_type_parameter
                    && !(trait_domain.is_some()
                        && matches!(
                            parameter.kind,
                            hir::clause::ClauseParamKind::Positional(..)
                                | hir::clause::ClauseParamKind::Optional(..)
                        ))
                    && !matches!(
                        parameter.kind,
                        hir::clause::ClauseParamKind::Type(_)
                            | hir::clause::ClauseParamKind::TraitBound(..)
                    )
                {
                    return Err(error(
                        parameter.span,
                        "nominal type clause kind is not implemented",
                    ));
                }
                let id = parameter
                    .definition
                    .ok_or_else(|| error(parameter.span, "missing clause parameter identity"))?;
                let id = DeclarationId {
                    package: semantic_package(package_identities, id.pkg),
                    local: id.index,
                };
                parameters.push(id);
                if let Some(domain) = trait_domain {
                    checker
                        .implementation_index
                        .register_parameter_domain(id, domain.clone())
                        .map_err(|message| error(parameter.span, message))?;
                    checker
                        .inherent_index
                        .register_parameter_domain(id, domain.clone())
                        .map_err(|message| error(parameter.span, message))?;
                    checker.nominal_input_prefixes.insert(id, domain);
                }
                checker.prepare_parameter_capabilities_prefix(owner, clauses, false)?;
            }
            checker.prepare_parameter_capabilities_prefix(owner, clauses, true)?;
            let capabilities = checker
                .capabilities
                .get(&owner)
                .cloned()
                .unwrap_or_default();
            for parameter in &parameters {
                checker.nominal_input_prefixes.remove(parameter);
            }
            checker.type_parameters.insert(owner, parameters);
            if !trait_domains.is_empty() {
                checker.trait_parameter_types.insert(owner, trait_domains);
            }
            checker.capabilities.insert(owner, capabilities);
            // An earlier required computation may already have requested the
            // layout while clause registration was still in progress.
            if let Some(definition) =
                checker
                    .structs
                    .get_mut(&declaration(package, package_identities, owner))
            {
                definition.capabilities = checker.capabilities[&owner].clone();
                definition.parameter_types = checker
                    .trait_parameter_types
                    .get(&owner)
                    .cloned()
                    .unwrap_or_default();
            }
            if let Some(definition) =
                checker
                    .enums
                    .get_mut(&declaration(package, package_identities, owner))
            {
                definition.capabilities = checker.capabilities[&owner].clone();
                definition.parameter_types = checker
                    .trait_parameter_types
                    .get(&owner)
                    .cloned()
                    .unwrap_or_default();
            }
            let nominal_id = declaration(package, package_identities, owner);
            if checker.newtypes.contains_key(&nominal_id) {
                let parameter_types =
                    checker.nominal_parameter_types(owner, info.node.expect_item().span)?;
                let definition = checker.newtypes.get_mut(&nominal_id).unwrap();
                definition.capabilities = checker.capabilities[&owner].clone();
                definition.parameter_types = parameter_types;
            }
            if matches!(
                info.node.expect_item().kind,
                ItemKind::Struct(_) | ItemKind::Enum(_) | ItemKind::Newtype(..)
            ) {
                checker.context.admit_nominal_constructor(declaration(
                    package,
                    package_identities,
                    owner,
                ));
            }
        }
    }
    // Interface selection must not wait for unrelated aliases or ordinary
    // signatures that themselves invoke an interface. Prepare just the world
    // declarations and their method contracts; their nominal/head dependencies
    // are demanded through the existing ensure_* queries.
    for (owner, info) in package.owners() {
        if checker.self_types.contains_key(&owner) || checker.self_containers.contains_key(&owner) {
            if matches!(
                info.node.expect_item().kind,
                ItemKind::Fn(..) | ItemKind::FnDeclaration(_)
            ) {
                if checker.is_data_consumer(owner) {
                    checker.ensure_data_consumer(owner, info.node.expect_item().span)?;
                } else {
                    checker.ensure_signature(owner, info.node.expect_item().span)?;
                }
            }
        }
    }
    for (owner, info) in package.owners() {
        if matches!(
            info.node.expect_item().kind,
            ItemKind::Trait(_) | ItemKind::Impl(_)
        ) || checker.self_types.contains_key(&owner)
            || checker.self_containers.contains_key(&owner)
        {
            checker.ensure_contract(owner, info.node.expect_item().span)?;
        }
    }
    checker.signature_owner = None;
    checker.context = checker.nominal_context();
    let prepared_attributes = checker.prepare_phase_attributes(inspect_attributes)?;
    if inspect_attributes
        && package
            .pending_semantics
            .iter()
            .any(|(_, feature)| feature == "derivation expansion")
    {
        // Registration must observe the original strategy targets before any
        // strategy is selected. This inspection path publishes no executable
        // package or evidence that generated implementations are valid.
        return Ok((
            n::Package {
                target,
                ..Default::default()
            },
            prepared_attributes,
        ));
    }
    let (traits, implementations) = checker.check_interfaces()?;
    checker.checked_traits = traits.clone();
    checker.checked_implementations = implementations.clone();
    for (&member, &parent) in &checker.self_containers {
        let inherited = checker.requirements[&parent].clone();
        checker
            .requirements
            .entry(member)
            .or_default()
            .extend(inherited);
        let inherited = checker
            .capabilities
            .get(&parent)
            .cloned()
            .unwrap_or_default();
        let capabilities = checker.capabilities.entry(member).or_default();
        for capability in inherited {
            if !capabilities.contains(&capability) {
                capabilities.push(capability);
            }
        }
    }
    for interface in &checker.checked_traits {
        for method in &interface.methods {
            if let Some((member, _)) = package.owners().find(|(owner, _)| {
                matches_declaration(package, package_identities, *owner, method.declaration)
            }) {
                checker
                    .requirements
                    .entry(member)
                    .or_default()
                    .extend(interface.requirements.clone());
            }
        }
    }
    if let Some(id) = checker.protocols.splice {
        let span = package
            .owners()
            .find_map(|(owner, info)| {
                (matches_declaration(package, package_identities, owner, id))
                    .then_some(info.node.expect_item().span)
            })
            .unwrap_or(rustc_span::DUMMY_SP);
        crate::splice::protocol(&checker.checked_traits, id)
            .map_err(|reason| error(span, reason))?;
    }
    checker.interfaces_ready = true;
    for operator in crate::arithmetic::Operator::ALL {
        if let Some(id) = checker.protocols.arithmetic[operator as usize] {
            crate::arithmetic::protocol(&checker.checked_traits, id, operator)
                .map_err(|reason| error(rustc_span::DUMMY_SP, reason))?;
        }
    }
    if let Some(id) = checker.protocols.index {
        crate::indexing::protocol(&checker.checked_traits, id)
            .map_err(|reason| error(rustc_span::DUMMY_SP, reason))?;
    }
    if let Some(id) = checker.protocols.list_pattern {
        crate::list_patterns::protocol(&checker.checked_traits, id)
            .map_err(|reason| error(rustc_span::DUMMY_SP, reason))?;
    }
    if let Some(id) = checker.protocols.literal_into {
        crate::literal_conversion::protocol(&checker.checked_traits, id)
            .map_err(|reason| error(rustc_span::DUMMY_SP, reason))?;
    }
    if let Some(id) = checker.protocols.iterator {
        checker.validate_iterator_protocol(id)?;
    }
    if let Some(id) = checker.protocols.into_iterator {
        checker.validate_into_iterator_protocol(id)?;
    }
    checker.finish_signature_refinements()?;
    checker.associated_signature_values.clear();
    checker.complete_associated_values()?;
    for (owner, info) in package.owners() {
        if matches!(
            info.node.expect_item().kind,
            ItemKind::Trait(_)
                | ItemKind::Struct(_)
                | ItemKind::Enum(_)
                | ItemKind::Newtype(..)
                | ItemKind::TypeAlias(..)
        ) {
            checker.check_type_defaults(owner)?;
        }
    }
    let traits = checker.checked_traits.clone();
    let implementations = checker.checked_implementations.clone();
    let mut functions = Vec::new();
    for (owner, info) in package.owners() {
        let item = info.node.expect_item();
        checker.signature_owner = Some(owner);
        match &item.kind {
            ItemKind::TestTemplateInstance { .. } => continue,
            ItemKind::Fn(sig, _) if sig.modifiers.is_test_template => continue,
            ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig) => {
                if checker.is_data_consumer(owner) {
                    checker.ensure_data_consumer(owner, item.span)?;
                    continue;
                }
                if checker.is_pattern_generator(owner) {
                    checker.ensure_pattern_generator(owner, item.span)?;
                    continue;
                }
                if checker.is_code_generator(owner) {
                    checker.ensure_code_generator(owner, item.span)?;
                    continue;
                }
                if sig.modifiers.is_pure
                    && !sig.modifiers.is_comptime
                    && !sig.modifiers.is_function_template
                    && matches!(item.kind, ItemKind::FnDeclaration(_))
                    && !checker
                        .checked_traits
                        .iter()
                        .flat_map(|interface| &interface.methods)
                        .any(|method| {
                            method.declaration == declaration(package, package_identities, owner)
                        })
                {
                    return Err(error(
                        item.span,
                        "runtime pure declaration outside a trait requires an external behavior contract",
                    ));
                }
                checker.ensure_signature(owner, item.span)?;
                checker.check_type_defaults(owner)?;
                if let ItemKind::Fn(_, body) = item.kind {
                    functions.push((owner, item, sig, body));
                }
            }
            ItemKind::Effect(_, _) => {
                checker.ensure_signature(owner, item.span)?;
                checker.check_type_defaults(owner)?;
            }
            ItemKind::EffectGroup(_) => checker.check_type_defaults(owner)?,
            ItemKind::Struct(_) | ItemKind::Enum(_) | ItemKind::Newtype(..) => {
                checker.ensure_nominal(owner, item.span)?
            }
            ItemKind::Mod(_) | ItemKind::Use(_) | ItemKind::Trait(_) | ItemKind::Impl(_) => {}
            ItemKind::TypeAlias(value, ..) => {
                checker.check_type_defaults(owner)?;
                if checker.type_parameters[&owner].is_empty() {
                    checker.alias_type(owner, value, &[], item.span)?;
                } else {
                    checker.check_alias_definition(owner, value, item.span)?;
                }
            }
            ItemKind::Const(..) => {
                if checker
                    .compiler_operation_binding(owner, item.span)?
                    .is_some()
                {
                    // Calls use the binding's declaration identity; there is no
                    // runtime function value to materialize for this operation.
                } else if checker.is_build_context(owner) {
                    checker.validate_build_context(owner, item.span)?;
                } else {
                    checker.constant_value(owner, item.span)?;
                }
            }
            ItemKind::Associated(_) => {}
            _ => {
                return Err(error(
                    item.span,
                    "this declaration requires elaboration not yet implemented",
                ));
            }
        }
    }
    checker.prepare_staged_forwarders(functions.iter().map(|(owner, _, _, _)| *owner))?;
    // Finish contracts not already requested by required comptime computations.
    // Formation obligations are delayed, not assumed: declarations may refer
    // forward to other constrained types. Contract operands themselves must be
    // well formed without assuming the contract they are defining.
    checker.signature_owner = None;
    checker.checking_contract = true;
    for (owner, info) in package.owners() {
        checker.signature_owner = Some(owner);
        if matches!(&info.node.expect_item().kind, ItemKind::Fn(sig, _) if sig.modifiers.is_test_template)
            || checker.is_code_generator(owner)
            || checker.is_pattern_generator(owner)
            || checker.is_data_consumer(owner)
        {
            continue;
        }
        if !matches!(
            info.node.expect_item().kind,
            ItemKind::Fn(..)
                | ItemKind::TypeAlias(..)
                | ItemKind::Newtype(..)
                | ItemKind::FnDeclaration(_)
                | ItemKind::Effect(..)
                | ItemKind::EffectGroup(_)
                | ItemKind::Struct(_)
                | ItemKind::Enum(_)
                | ItemKind::Impl(_)
                | ItemKind::Trait(_)
        ) {
            continue;
        }
        checker.ensure_contract(owner, info.node.expect_item().span)?;
        let requirements = checker.requirements[&owner].clone();
        if let Some(def) = checker
            .structs
            .get_mut(&declaration(package, package_identities, owner))
        {
            def.requirements = requirements.clone();
        }
        if let Some(def) = checker
            .enums
            .get_mut(&declaration(package, package_identities, owner))
        {
            def.requirements = requirements.clone();
        }
        if let Some(def) =
            checker
                .newtypes
                .get_mut(&declaration(package, package_identities, owner))
        {
            def.requirements = requirements.clone();
            def.capabilities = checker
                .capabilities
                .get(&owner)
                .cloned()
                .unwrap_or_default();
        }
        checker.requirements.insert(owner, requirements);
    }
    checker.checking_contract = false;
    checker.signature_owner = None;
    checker.context = checker.nominal_context();
    checker.check_struct_cycles()?;
    // Resolve formation only after the complete concrete implementation world
    // exists. Contract operands may use binder capabilities, not their own
    // unproved requires predicates.
    for (scope, assume_requirements, target, arguments, span) in checker
        .pending_type_uses
        .take()
        .unwrap()
        .into_iter()
        .chain(std::mem::take(&mut checker.signature_type_uses))
    {
        checker.signature_owner = scope;
        checker.context = match scope {
            Some(owner) if assume_requirements => checker.definition_context(owner),
            Some(owner) => checker.binder_context(owner),
            None => checker.nominal_context(),
        };
        checker.check_formation_requirements(target, &arguments, span)?;
    }
    checker.context = checker.nominal_context();
    checker.signature_owner = None;
    // Definition-scope default checking precedes body checking. Only earlier
    // parameters are available; no caller-local lookup or evaluation occurs.
    for (owner, info) in package.owners() {
        if matches!(&info.node.expect_item().kind, ItemKind::Fn(sig, _) if sig.modifiers.is_test_template)
            || checker.is_code_generator(owner)
            || checker.is_pattern_generator(owner)
            || checker.is_data_consumer(owner)
        {
            continue;
        }
        if !matches!(
            info.node.expect_item().kind,
            ItemKind::Fn(..) | ItemKind::FnDeclaration(_)
        ) {
            continue;
        }
        checker.ensure_defaults(owner, info.node.expect_item().span)?;
    }
    for (owner, info) in package.owners() {
        if let ItemKind::Struct(def) = &info.node.expect_item().kind {
            for (index, field) in def.fields.iter().enumerate() {
                if field.default.is_some() {
                    checker.field_default(owner, None, index)?;
                }
            }
        }
    }
    for (owner, info) in package.owners() {
        if let ItemKind::Enum(def) = &info.node.expect_item().kind {
            let (leaves, _) = nominal::enum_leaves(def.variants)?;
            for (v, (variant, _)) in leaves.iter().enumerate() {
                if let hir::item::VariantKind::Struct(fields) = &variant.kind {
                    for (index, field) in fields.iter().enumerate() {
                        if field.default.is_some() {
                            checker.field_default(owner, Some(v), index)?;
                        }
                    }
                }
            }
        }
    }
    let mut output = n::Package::default();
    output.target = target;
    output.builtin_implementation_packages =
        checker.protocols.builtin_implementation_packages.clone();
    for (owner, info) in package.owners() {
        if matches!(&info.node.expect_item().kind, ItemKind::Fn(sig, _) if sig.modifiers.is_test) {
            output
                .tests
                .push(declaration(package, package_identities, owner));
        }
    }

    output.normal_forms = checker.normal_form_metadata();
    output.function_parameters = checker.function_parameter_metadata();
    for (owner, info) in package.owners() {
        let item = info.node.expect_item();
        if let ItemKind::Effect(_, modifiers) = &item.kind {
            output.effects.push(nir::effects::Declaration {
                declaration: declaration(package, package_identities, owner),
                group: checker
                    .effect_parents
                    .get(&owner)
                    .map(|group| declaration(package, package_identities, *group)),
                name: item.ident.name.as_str().into(),
                type_parameters: checker.type_parameters[&owner].clone(),
                parameter_types: checker.type_parameters[&owner]
                    .iter()
                    .map(|id| {
                        checker
                            .trait_input_domain(*id)
                            .unwrap_or_else(|| p(P::Type))
                    })
                    .collect(),
                signature: checker.signatures[&owner].clone(),
                abi: if modifiers.is_direct_passing {
                    nir::effects::Abi::DirectPassing
                } else if modifiers.is_async {
                    nir::effects::Abi::Async
                } else {
                    nir::effects::Abi::Direct
                },
                inline: modifiers.is_inline,
                requirements: checker.definition_requirements(owner),
                capabilities: checker
                    .capabilities
                    .get(&owner)
                    .cloned()
                    .unwrap_or_default(),
            });
        }
    }
    output.traits = traits;
    output.implementations = implementations;
    // Code methods belong to the elaboration world. Ordinary execution keeps
    // the same trait/implementation identities but no code-only member slots.
    let code_methods: BTreeSet<_> = checker
        .code_generators
        .keys()
        .chain(checker.pattern_generators.keys())
        .chain(checker.data_consumers.keys())
        .map(|owner| declaration(package, package_identities, *owner))
        .collect();
    for interface in &mut output.traits {
        interface.methods.retain(|method| {
            !matches!(
                method.signature,
                nir::code_interfaces::MethodSignature::Generator(_)
                    | nir::code_interfaces::MethodSignature::PatternGenerator(_)
                    | nir::code_interfaces::MethodSignature::DataConsumer(_)
            )
        });
    }
    for implementation in &mut output.implementations {
        implementation
            .methods
            .retain(|(_, body)| !code_methods.contains(body));
    }
    output.newtypes = checker.newtypes.values().cloned().collect();
    output.structs = checker.structs.values().cloned().collect();
    output.enums = checker.enums.values().cloned().collect();
    for owner in checker.open_functions.keys().copied().collect::<Vec<_>>() {
        checker.check_open_definition(owner)?;
    }
    for owner in checker
        .pattern_generators
        .keys()
        .copied()
        .collect::<Vec<_>>()
    {
        checker.check_pattern_generator(owner)?;
    }
    for owner in checker.code_generators.keys().copied().collect::<Vec<_>>() {
        checker.check_code_generator(owner)?;
    }
    for owner in checker.data_consumers.keys().copied().collect::<Vec<_>>() {
        checker.check_data_consumer(owner)?;
    }
    for (owner, item, sig, body_id) in functions {
        if checker.open_functions.contains_key(&owner) {
            continue;
        }
        checker.signature_owner = Some(owner);
        checker.context = checker.definition_context(owner);
        checker.locals.clear();
        let signature = checker.signatures[&owner].clone();
        let Term::Function { parameters, result } = &signature else {
            unreachable!()
        };
        for (param, ty) in sig.params.iter().zip(parameters) {
            checker.locals.insert(local(param.hir_id), ty.ty.clone());
        }
        if let Some(parameters) = checker.normal_parameters.get(&owner) {
            for (_, binding, ty) in parameters.iter().filter(|(_, _, ty)| *ty != p(P::Type)) {
                checker.locals.insert(*binding, ty.clone());
            }
        }
        checker.result = Some((**result).clone());
        let body = package
            .body(body_id)
            .ok_or_else(|| error(item.span, "missing function body"))?;
        let preconditions = checker.program_preconditions(owner)?;
        let result_binding = sig.return_binding.map(local);
        let (postconditions, entry_snapshots) =
            checker.program_postconditions(owner, result_binding, result)?;
        let previous_code = checker.in_code_body;
        let previous_holes = std::mem::take(&mut checker.quote_holes);
        checker.in_code_body = sig.modifiers.is_quote;
        let value = checker.computation_body(body.value, Some(result));
        checker.in_code_body = previous_code;
        checker.quote_holes = previous_holes;
        let value = value?;
        output.functions.push(n::Function {
            phase: if sig.modifiers.is_normal_form_head {
                n::FunctionPhase::NormalForm
            } else if sig.modifiers.is_comptime {
                n::FunctionPhase::Comptime
            } else {
                n::FunctionPhase::Runtime
            },
            inline: sig.modifiers.is_inline,
            pure: sig.modifiers.is_pure,
            is_unsafe: sig.modifiers.is_unsafe,
            preconditions,
            postconditions,
            entry_snapshots,
            result_binding,
            capabilities: checker
                .capabilities
                .get(&owner)
                .cloned()
                .unwrap_or_default(),
            requirements: checker.definition_requirements(owner),
            type_parameters: checker.type_parameters[&owner].clone(),
            declaration: declaration(package, package_identities, owner),
            name: item.ident.name.as_str().into(),
            signature,
            parameters: if sig.modifiers.is_normal_form_head {
                checker.normal_parameters[&owner]
                    .iter()
                    .filter(|(_, _, ty)| *ty != p(P::Type))
                    .map(|(_, binding, _)| *binding)
                    .collect()
            } else {
                sig.params.iter().map(|x| local(x.hir_id)).collect()
            },
            body: value,
            defaults: checker.defaults[&owner]
                .iter()
                .enumerate()
                .filter_map(|(i, value)| value.clone().map(|value| (i, value)))
                .collect(),
        });
    }
    let attributes = prepared_attributes;
    checker.expand_test_templates(&mut output)?;
    // Required invocations in bodies, defaults and templates can publish types
    // after the initial layout snapshot. Consumers below need their complete
    // descriptions for contract, purity and quote analysis, not just emission.
    let mut emitted_structs: BTreeSet<_> = output
        .structs
        .iter()
        .map(|definition| definition.declaration)
        .collect();
    for (id, definition) in &checker.structs {
        if emitted_structs.insert(*id) {
            output.structs.push(definition.clone());
        }
    }
    // Body lowering can create private scalar observation packets after the
    // source declaration layouts were collected above.
    for declaration in &checker.computed_scalar_packets {
        if !output
            .enums
            .iter()
            .any(|item| item.declaration == *declaration)
        {
            output.enums.push(checker.enums[declaration].clone());
        }
        output.internal_return_packets.insert(*declaration);
    }
    // Staged declaration values can remain in checked default descriptions and
    // invocation inputs even though only concrete residual bodies are emitted.
    // Identity metadata admits their phase; it never authorizes execution.
    for owner in checker.open_functions.keys().copied() {
        if !matches!(&package.item(owner).unwrap().kind, ItemKind::Fn(sig, _)
            if !sig.modifiers.is_comptime && !sig.modifiers.is_normal_form_head)
        {
            continue;
        }
        if let Some(signature) = checker.known_callable_signature(owner) {
            output
                .runtime_function_identities
                .push(n::RuntimeFunctionIdentity {
                    declaration: declaration(package, package_identities, owner),
                    type_parameters: checker.type_parameters[&owner].clone(),
                    signature: checker.callable_type(owner, signature),
                });
        }
    }
    output.function_parameters = checker.function_parameter_metadata();
    output.functions.append(&mut checker.generated_functions);
    output.alias_contracts = checker.alias_contracts.values().cloned().collect();
    output.expansion_sources = std::mem::take(&mut checker.expansion_sources);
    crate::verification::contracts::inherit(&mut output, &mut checker.next_temporary_binding)
        .map_err(|mut failure| {
            failure.source = failure
                .source
                .original(&output.expansion_sources)
                .unwrap_or(failure.source);
            Error {
                source: failure.source,
                message: failure.to_string(),
                notes: Vec::new(),
                needs_context: false,
                needs_tactic: false,
                needs_capability: false,
                incompatible_context: false,
            }
        })?;
    // Definition-side witnesses are analysis-only, including suppliers whose
    // inline alternatives must be checked before their inputs are supplied.
    let runtime_count = output.functions.len();
    output
        .functions
        .append(&mut checker.ordinary_behavior_functions);
    let defaults_result = purity::validate_defaults(
        &output,
        &checker.pending_default_stability,
        &checker.protocols,
    );
    checker.ordinary_behavior_functions = output.functions.split_off(runtime_count);
    defaults_result?;
    cmtt::expand_quote_functions(
        &mut output,
        package,
        package_identities,
        &checker.protocols,
        &mut checker.next_temporary_binding,
        &mut checker.required_steps,
    )?;
    output.quote_identity_cursor = checker.next_temporary_binding;
    output.definition_identity_cursor = checker.next_definition_identity;
    if output
        .implementations
        .iter()
        .any(|i| !i.type_parameters.is_empty() || !i.requirements.is_empty())
    {
        output.dispatch_instances = crate::materialization::dispatch_instances(
            &output,
            crate::ptc::Budget {
                steps: checker
                    .protocols
                    .dispatch_fuel
                    .unwrap_or(crate::ptc::Budget::default().steps),
                ..crate::ptc::Budget::default()
            },
        )
        .map_err(|message| error(Span::default(), message))?;
    }
    // Ordinary behavior witnesses cover all inline branches but are not
    // executable artifacts: their comptime inputs remain abstract.
    let runtime_count = output.functions.len();
    output
        .functions
        .append(&mut checker.ordinary_behavior_functions);
    let purity_result = purity::validate_runtime_promises(&output, &checker.protocols);
    output.functions.truncate(runtime_count);
    purity_result?;
    crate::verification::flow::validate_machine_operations(&output).map_err(|mut failure| {
        failure.source = failure
            .source
            .original(&output.expansion_sources)
            .unwrap_or(failure.source);
        Error {
            source: failure.source,
            message: failure.to_string(),
            notes: Vec::new(),
            needs_context: false,
            needs_tactic: false,
            needs_capability: false,
            incompatible_context: false,
        }
    })?;
    delivered_types::admit(
        &checker.delivered_types,
        &output,
        &mut checker.required_steps,
    )?;
    Ok((output, attributes))
}
#[derive(Clone)]
struct CaptureFrame {
    comptime: bool,
    outer: BTreeSet<n::LocalId>,
    values: Vec<n::Capture>,
}
#[derive(Clone, Debug)]
struct FormationInputs {
    terms: Vec<Term>,
    // Executable type constructors retain deferred contracts for invocation.
    deferred: BTreeSet<DeclarationId>,
    environment: BTreeMap<DeclarationId, std::sync::Arc<Term>>,
}
impl FormationInputs {
    fn ready(terms: &[Term]) -> Self {
        Self {
            terms: terms.to_vec(),
            deferred: BTreeSet::new(),
            environment: BTreeMap::new(),
        }
    }
}

struct OldScope {
    owner: OwnerId,
    active: bool,
    bindings: BTreeMap<n::LocalId, n::LocalId>,
}
/// A return scope can infer a closure's result, or merely check an open
/// definition whose result domain is still suspended. The latter must not
/// unify distinct source exits before the invocation inputs are available.
#[derive(Clone)]
enum ReturnObservations {
    Infer(Vec<Term>),
    Deferred,
}
impl ReturnObservations {
    fn iter(&self) -> std::slice::Iter<'_, Term> {
        match self {
            Self::Infer(types) => types.iter(),
            Self::Deferred => [].iter(),
        }
    }
    fn push(&mut self, ty: Term) {
        if let Self::Infer(types) = self {
            types.push(ty);
        }
    }
}

struct Checker<'a, 'h> {
    next_definition_identity: u32,
    delivered_types: BTreeMap<n::LocalId, (std::sync::Arc<Term>, Span)>,
    target: Option<nir::target::TargetData>,
    protocols: Protocols,
    /// Only this expression's result is being used as a callee. Argument and
    /// callback expressions keep their own phase and conversion boundaries.
    comptime_callee_result: Option<hir::HirId>,
    /// Partial result information belongs only to this exact call expression.
    structural_result_context: Option<(hir::HirId, crate::calls::InputContext)>,
    package_identities: BTreeMap<u32, u64>,
    package: &'a hir::Package<'h>,
    effect_parents: BTreeMap<OwnerId, OwnerId>,
    signatures: BTreeMap<OwnerId, Term>,
    open_functions: BTreeMap<OwnerId, open_functions::Definition>,
    active_open_definitions: BTreeSet<OwnerId>,
    open_instances:
        std::collections::HashMap<(OwnerId, Vec<Term>, Vec<Option<Term>>, Term), DeclarationId>,
    next_open_instance: u32,
    open_instantiation_depth: usize,
    generated_functions: Vec<n::Function>,
    generated_function_parameters: BTreeMap<DeclarationId, nir::function_parameters::Declaration>,
    ordinary_behavior_functions: Vec<n::Function>,
    checking_open_behavior: bool,
    checking_default_behavior: bool,
    default_behavior_functions: BTreeSet<OwnerId>,
    describing_open_spread: bool,
    describing_open_call: bool,
    ordinary_behavior_inputs: BTreeMap<n::LocalId, Term>,
    open_loop_transfers: BTreeMap<n::LocalId, open_functions::LoopTransfers>,
    pending_default_stability: Vec<(n::Expr, Span, BTreeSet<n::LocalId>)>,
    specialization_inputs: Vec<(n::LocalId, Term, Term)>,
    active_signatures: BTreeSet<OwnerId>,
    refinement_subjects: Vec<Term>,
    pending_refinements: Vec<refinements::Pending>,
    executable_refinement_owner: Option<OwnerId>,
    interface_signatures: BTreeMap<OwnerId, Term>,
    self_types: BTreeMap<OwnerId, Term>,
    self_containers: BTreeMap<OwnerId, OwnerId>,
    interfaces_ready: bool,
    member_phases: BTreeMap<OwnerId, n::TraitPhase>,
    active_early_inherent: BTreeSet<String>,
    checking_method_contract: bool,
    pending_contract_projections: Vec<interfaces::PendingProjection>,
    checked_traits: Vec<n::Trait>,
    checked_implementations: Vec<n::Implementation>,
    associated_initializers:
        BTreeMap<(DeclarationId, DeclarationId), (OwnerId, Vec<DeclarationId>, Vec<Term>)>,
    implementation_relationships: BTreeMap<DeclarationId, DeclarationId>,
    complementary_associated: Vec<(
        (DeclarationId, DeclarationId),
        (OwnerId, Vec<DeclarationId>, Vec<Term>),
    )>,
    active_associated_values: BTreeSet<(DeclarationId, DeclarationId)>,
    selected_associated_implementation: Option<DeclarationId>,
    active_associated_signatures: BTreeSet<OwnerId>,
    declaration_projection_depth: usize,
    associated_definition_equations: BTreeMap<DeclarationId, Vec<(Term, Term)>>,
    associated_signature_values: BTreeMap<OwnerId, n::Expr>,
    implementation_index: crate::implementation::Index,
    trait_parameter_types: BTreeMap<OwnerId, Vec<Term>>,
    nominal_input_prefixes: BTreeMap<DeclarationId, Term>,
    inherent_index: crate::implementation::InherentIndex,
    type_parameters: BTreeMap<OwnerId, Vec<DeclarationId>>,
    normal_parameters: BTreeMap<OwnerId, Vec<(DeclarationId, n::LocalId, Term)>>,
    // Active signature prefixes are not completed head metadata and must never
    // be copied into required-invocation packages.
    normal_input_prefixes: BTreeMap<OwnerId, Vec<(DeclarationId, n::LocalId, Term)>>,
    // Ordinary Type inputs live in the function telescope, independently of
    // declaration-level generic parameters and their nominal identities.
    type_input_bindings: BTreeMap<n::LocalId, Term>,
    partial_staged_bindings: BTreeMap<n::LocalId, std::sync::Arc<constants::PartialStagedValue>>,
    lexical_input_domains: BTreeMap<DeclarationId, Term>,
    callable_type_inputs: BTreeMap<n::LocalId, Option<Term>>,
    callable_value_inputs: BTreeMap<n::LocalId, (Term, Term)>,
    // Ordinary comptime values retain both their bound identity and domain.
    // Runtime parameters are never registered as static normal-form inputs.
    value_input_bindings: BTreeMap<n::LocalId, (Term, Term)>,
    derived_staged_bindings: BTreeSet<n::LocalId>,
    // Description-only provenance; never authorizes executable code capture.
    open_staged_locals: BTreeSet<n::LocalId>,
    // Pending member results, not Type identities or permission evidence.
    open_member_locals: BTreeSet<n::LocalId>,
    signature_prefixes: BTreeMap<OwnerId, Vec<Parameter>>,
    signature_domains: BTreeMap<OwnerId, Vec<(Sort, Option<Term>)>>,
    default_input_scope: Option<Vec<(Sort, Option<Term>)>>,
    default_input_owner: Option<OwnerId>,
    requirements: BTreeMap<OwnerId, Vec<n::TypePredicate>>,
    program_requirements: BTreeMap<OwnerId, Vec<&'h Expr<'h>>>,
    program_postconditions: BTreeMap<OwnerId, Vec<&'h Expr<'h>>>,
    old_scope: Option<OldScope>,
    tactic_frame: Option<(n::SourceRange, Vec<hir::HirId>, bool)>,
    capabilities: BTreeMap<OwnerId, Vec<n::TraitRequirement>>,
    pending_type_uses: Option<Vec<(Option<OwnerId>, bool, OwnerId, FormationInputs, Span)>>,
    signature_type_uses: Vec<(Option<OwnerId>, bool, OwnerId, FormationInputs, Span)>,
    checking_contract: bool,
    // Final literal defaulting also settles an otherwise unconstrained Symbol;
    // speculative attempts must leave room for a later enum target.
    numeric_defaults: bool,
    signature_owner: Option<OwnerId>,
    context: crate::ptc::Context,
    defaults: BTreeMap<OwnerId, Vec<Option<n::Expr>>>,
    locals: BTreeMap<n::LocalId, Term>,
    code_values: BTreeMap<n::LocalId, std::sync::Arc<cmtt::ExpressionCode>>,
    template_code_values: BTreeMap<(u32, u32), std::sync::Arc<cmtt::ExpressionCode>>,
    pattern_codes: BTreeMap<n::LocalId, std::sync::Arc<cmtt::patterns::PatternCode>>,
    pattern_quote_pending: Option<bool>,
    pattern_generators: BTreeMap<OwnerId, cmtt::patterns::PatternGenerator>,
    code_generators: BTreeMap<OwnerId, cmtt::Generator>,
    data_consumers: BTreeMap<OwnerId, cmtt::Generator<Term>>,
    data_consumer_depth: usize,
    // Invocation-local environments; recursive calls restore the previous entry.
    // The shared generator declaration cache always remains unspecialized.
    code_method_captures: BTreeMap<OwnerId, Vec<(DeclarationId, Term)>>,
    code_method_targets: BTreeMap<OwnerId, OwnerId>,
    code_packet_nominals: BTreeSet<DeclarationId>,
    computed_scalar_packets: BTreeSet<DeclarationId>,
    generator_data: BTreeMap<n::LocalId, Option<n::Expr>>,
    generator_cells: BTreeMap<n::LocalId, (n::Expr, comptime_interpretor::Storage)>,
    checking_code_generator: bool,
    abstract_code_dependencies: BTreeMap<n::LocalId, BTreeSet<n::LocalId>>,
    in_code_body: bool,
    inline_selection: Option<(hir::HirId, usize)>,
    inline_match_input: Option<(hir::HirId, n::Expr, Term)>,
    in_unselected_inline: bool,
    quote_holes: BTreeSet<(u32, u32)>,
    code_construction_depth: usize,
    expansion_sources: Vec<n::ExpansionSource>,
    aliases: BTreeSet<OwnerId>,
    completed_aliases: std::collections::HashMap<
        (
            OwnerId,
            Vec<Term>,
            Vec<(DeclarationId, std::sync::Arc<Term>)>,
        ),
        std::sync::Arc<Term>,
    >,
    checked_alias_definitions: BTreeSet<OwnerId>,
    alias_scopes: BTreeMap<OwnerId, Option<OwnerId>>,
    alias_substitution: BTreeMap<DeclarationId, std::sync::Arc<Term>>,
    alias_value_bindings: BTreeMap<DeclarationId, n::LocalId>,
    normal_default_bindings: BTreeMap<DeclarationId, (n::LocalId, Term)>,
    forbidden_default_inputs: BTreeSet<DeclarationId>,
    next_temporary_binding: u32,
    alias_expression_budget: usize,
    constants: BTreeMap<OwnerId, n::Expr>,
    active_constants: BTreeSet<OwnerId>,
    active_nominals: BTreeSet<OwnerId>,
    active_required: BTreeSet<n::LocalId>,
    active_contracts: BTreeSet<OwnerId>,
    active_function_defaults: BTreeSet<(OwnerId, usize)>,
    active_type_defaults: BTreeSet<(OwnerId, usize)>,
    default_uses:
        BTreeMap<(OwnerId, usize), Vec<(Option<OwnerId>, bool, OwnerId, FormationInputs, Span)>>,
    contract_uses: BTreeMap<OwnerId, Vec<(Option<OwnerId>, bool, OwnerId, FormationInputs, Span)>>,
    required_steps: u64,
    in_required_comptime: bool,
    in_staged_input: bool,
    preparing_spread: Option<n::SourceRange>,
    pending_spread_inputs: BTreeMap<n::LocalId, spreads::PendingPacket>,
    observing_type: bool,
    result: Option<Term>,
    partial_return_context: Option<crate::calls::InputContext>,
    // Unresolved missing returns (explicit null or propagation), completed
    // before exporting the enclosing callback. Values are absence-node spans.
    pending_optional_returns: BTreeMap<(u32, u32), n::SourceRange>,
    // Private propagation sites, completed after the callback result is inferred.
    pending_error_returns: BTreeMap<(u32, u32), Term>,
    inferred_return_sites: BTreeSet<(u32, u32)>,
    optional_branch_probe_budget: usize,
    probing_optional_branch: bool,
    optional_target_probe: Option<hir::HirId>,
    effect_environment: Option<Vec<nir::effects::Operation>>,
    effect_resume_result: Option<Term>,
    preserve_effect_value: bool,
    captures: Vec<CaptureFrame>,
    return_observations: Option<ReturnObservations>,
    loops: Vec<n::LocalId>,
    structs: BTreeMap<DeclarationId, n::Struct>,
    newtypes: BTreeMap<DeclarationId, nir::newtypes::Newtype>,
    alias_contracts: BTreeMap<DeclarationId, nir::type_aliases::Contract>,
    enums: BTreeMap<DeclarationId, n::Enum>,
    enum_sources: BTreeMap<DeclarationId, Vec<&'h hir::item::Variant<'h>>>,
    field_defaults: BTreeMap<(OwnerId, Option<usize>, usize), n::Expr>,
    active_defaults: BTreeSet<(OwnerId, Option<usize>, usize)>,
    immutable: BTreeSet<n::LocalId>,
}
impl Checker<'_, '_> {
    fn integer_layout(&self, primitive: P) -> Option<(bool, u32)> {
        self.target.map_or_else(
            || primitive.fixed_integer_layout(),
            |target| target.integer_layout(primitive),
        )
    }

    /// Resolve signature dependencies on demand, retaining only completed
    /// signatures. Recursive value bodies do not require recursive signatures;
    /// cycles through type observations are diagnosed explicitly.
    fn ensure_signature(&mut self, owner: OwnerId, span: Span) -> Result<(), Error> {
        if self.builtin_normal_form_constant(owner, span)?.is_some() {
            return Ok(());
        }
        if self.package.item(owner).is_some_and(
            |item| matches!(&item.kind, ItemKind::Fn(sig, _) if sig.modifiers.is_test_template),
        ) {
            return Err(error(
                span,
                "test templates are instantiated with `test ... use`, not called as functions",
            ));
        }
        self.validate_quote_function(owner, span)?;
        if self.is_data_consumer(owner) {
            return Err(error(
                span,
                "contextual data consumers are invoked at comptime construction boundaries, not as ordinary function values",
            ));
        }
        if self.is_pattern_generator(owner) {
            return self.ensure_pattern_generator(owner, span);
        }
        if self.is_code_generator(owner) {
            return self.ensure_code_generator(owner, span);
        }
        if self.signatures.contains_key(&owner)
            || self.interface_signatures.contains_key(&owner)
            || self.open_functions.contains_key(&owner)
        {
            return Ok(());
        }
        if self.active_signatures.len() >= 64 {
            return Err(error(
                span,
                "function signature dependency depth limit exceeded",
            ));
        }
        if !self.active_signatures.insert(owner) {
            return Err(error(span, "cyclic function signature dependency"));
        }
        let saved_owner = self.signature_owner.replace(owner);
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_captures = std::mem::take(&mut self.captures);
        let saved_result = self.result.take();
        let saved_partial_return = self.partial_return_context.take();
        let saved_optional_returns = std::mem::take(&mut self.pending_optional_returns);
        let saved_error_returns = std::mem::take(&mut self.pending_error_returns);
        let saved_return_sites = std::mem::take(&mut self.inferred_return_sites);
        let saved_returns = self.return_observations.take();
        let saved_loops = std::mem::take(&mut self.loops);
        let saved_immutable = std::mem::take(&mut self.immutable);
        let saved_context = self.context.clone();
        let saved_contract = std::mem::replace(&mut self.checking_contract, false);
        let saved_defaults = std::mem::replace(&mut self.numeric_defaults, true);
        self.context = self.nominal_context();
        // Declaration obligations survive a failed caller-side inference probe;
        // keep them separate from the caller's speculative obligations.
        let saved_pending = self.pending_type_uses.replace(Vec::new());
        let package = self.package;
        let saved_capabilities = self.capabilities.get(&owner).cloned();
        let had_contract = self.requirements.contains_key(&owner);
        let checked = (|| {
            let item = package
                .item(owner)
                .ok_or_else(|| error(span, "missing function declaration"))?;
            let (ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig) | ItemKind::Effect(sig, _)) =
                &item.kind
            else {
                return Err(error(span, "declaration is not a supported callable"));
            };
            if let Some(parent) = self.effect_parents.get(&owner).copied() {
                self.ensure_contract(parent, span)?;
            }
            if let Some(parent) = package.lexical_parents.get(&owner).copied() {
                self.ensure_signature(parent, span)?;
            }
            let mut type_parameters = if let Some(parent) = self
                .effect_parents
                .get(&owner)
                .or_else(|| self.self_containers.get(&owner))
                .or_else(|| package.lexical_parents.get(&owner))
            {
                self.type_parameters[parent]
                    .iter()
                    .copied()
                    .chain(self.nominal_capture_ids(*parent))
                    .collect()
            } else {
                match self.self_types.get(&owner) {
                    Some(Term::Rigid {
                        declaration: interface,
                        ..
                    }) => {
                        let parent = package
                            .owners()
                            .find_map(|(owner, _)| {
                                (matches_declaration(
                                    self.package,
                                    &self.package_identities,
                                    owner,
                                    *interface,
                                ))
                                .then_some(owner)
                            })
                            .unwrap();
                        std::iter::once(*interface)
                            .chain(self.type_parameters[&parent].iter().copied())
                            .collect()
                    }
                    _ => Vec::new(),
                }
            };
            if let Some(parent) = package.lexical_parents.get(&owner).copied() {
                for (_, input, _, domain) in self.lexical_staged_inputs(parent, span)? {
                    if !type_parameters.contains(&input) {
                        type_parameters.push(input);
                    }
                    self.lexical_input_domains.insert(input, domain);
                }
            }
            let parent = self
                .effect_parents
                .get(&owner)
                .or_else(|| self.self_containers.get(&owner))
                .or_else(|| package.lexical_parents.get(&owner))
                .copied()
                .or_else(|| {
                    let Some(Term::Rigid {
                        declaration: interface,
                        ..
                    }) = self.self_types.get(&owner)
                    else {
                        return None;
                    };
                    package.owners().find_map(|(parent, _)| {
                        (matches_declaration(
                            self.package,
                            &self.package_identities,
                            parent,
                            *interface,
                        ))
                        .then_some(parent)
                    })
                });
            let mut capabilities = parent
                .and_then(|parent| self.capabilities.get(&parent))
                .cloned()
                .unwrap_or_default();
            if let Some(parent) = package.lexical_parents.get(&owner).copied() {
                for capability in &mut capabilities {
                    capability.subject =
                        self.lift_lexical_type(parent, &capability.subject, span)?;
                    for argument in &mut capability.interface_arguments {
                        *argument = self.lift_lexical_type(parent, argument, span)?;
                    }
                }
            }
            self.capabilities.insert(owner, capabilities.clone());
            let parameter_subjects = required::parameter_capability_subjects(sig.clause_params)?;
            let mut pending_capabilities = Vec::new();
            for (subject, bound) in &parameter_subjects {
                let atom = Expr {
                    kind: H::TraitBound(subject, bound),
                    ..subject.clone()
                };
                required::signature_capability_atoms(&atom, &mut pending_capabilities, 0)?;
            }
            let mut normal_parameters = Vec::new();
            for parameter in sig.clause_params {
                let prefix = self.normal_input_prefixes.entry(owner).or_default();
                let prepared = prefix.len();
                prefix.extend(normal_parameters[prepared..].iter().cloned());
                self.ready_signature_capabilities(owner, &mut pending_capabilities, false)?;
                if let hir::clause::ClauseParamKind::Varadic(_, declared) = parameter.kind {
                    let ty = self.type_expr(declared)?;
                    if !sig.modifiers.is_normal_form_head || self.list_element_type(&ty).is_none() {
                        return Err(error(
                            parameter.span,
                            "variadic normal-form input requires a bound meta.List<T>",
                        ));
                    }
                    if !std::ptr::eq(parameter, sig.clause_params.last().unwrap()) {
                        return Err(error(
                            parameter.span,
                            "variadic normal-form input must be the final parameter",
                        ));
                    }
                    let id = parameter.definition.ok_or_else(|| {
                        error(parameter.span, "missing normal-form parameter identity")
                    })?;
                    normal_parameters.push((
                        DeclarationId {
                            package: semantic_package(&self.package_identities, id.pkg),
                            local: id.index,
                        },
                        local(parameter.hir_id),
                        ty,
                    ));
                    continue;
                }
                if let hir::clause::ClauseParamKind::Positional(_, ty) = parameter.kind {
                    let ty = self.type_expr(ty)?;
                    if !nir::normal_forms::supported_input_type_with_collections_in(
                        &ty,
                        &type_parameters
                            .iter()
                            .map(|declaration| nir::DeclarationParameter {
                                declaration: *declaration,
                                sort: Sort::Type,
                            })
                            .chain(
                                normal_parameters
                                    .iter()
                                    .filter(|(_, _, ty)| *ty != p(P::Type))
                                    .map(|(declaration, _, _)| nir::DeclarationParameter {
                                        declaration: *declaration,
                                        sort: Sort::ComptimeValue,
                                    }),
                            )
                            .collect::<Vec<_>>(),
                        |head| Some(head) == self.protocols.tuple,
                        |head| Some(head) == self.protocols.meta_list,
                        |id| {
                            self.structs.contains_key(&id)
                                || self.enums.contains_key(&id)
                                || self.newtypes.contains_key(&id)
                                || self.context.is_nominal_constructor(id)
                        },
                        self.target,
                    ) {
                        return Err(error(
                            parameter.span,
                            "normal-form parameter type is not yet supported",
                        ));
                    }
                    if ty != p(P::Type) {
                        let id = parameter.definition.ok_or_else(|| {
                            error(parameter.span, "missing normal-form parameter identity")
                        })?;
                        if !sig.modifiers.is_normal_form_head {
                            type_parameters.push(DeclarationId {
                                package: semantic_package(&self.package_identities, id.pkg),
                                local: id.index,
                            });
                        }
                        normal_parameters.push((
                            DeclarationId {
                                package: semantic_package(&self.package_identities, id.pkg),
                                local: id.index,
                            },
                            local(parameter.hir_id),
                            ty,
                        ));
                        continue;
                    }
                } else if let hir::clause::ClauseParamKind::Optional(_, ty, _) = parameter.kind {
                    let ty = self.type_expr(ty)?;
                    if ty != p(P::Type) {
                        if !nir::normal_forms::supported_input_type_with_collections_in(
                            &ty,
                            &type_parameters
                                .iter()
                                .map(|declaration| nir::DeclarationParameter {
                                    declaration: *declaration,
                                    sort: Sort::Type,
                                })
                                .chain(
                                    normal_parameters
                                        .iter()
                                        .filter(|(_, _, ty)| *ty != p(P::Type))
                                        .map(|(declaration, _, _)| nir::DeclarationParameter {
                                            declaration: *declaration,
                                            sort: Sort::ComptimeValue,
                                        }),
                                )
                                .collect::<Vec<_>>(),
                            |head| Some(head) == self.protocols.tuple,
                            |head| Some(head) == self.protocols.meta_list,
                            |id| {
                                self.structs.contains_key(&id)
                                    || self.enums.contains_key(&id)
                                    || self.newtypes.contains_key(&id)
                                    || self.context.is_nominal_constructor(id)
                            },
                            self.target,
                        ) {
                            return Err(error(
                                parameter.span,
                                "normal-form parameter type is not yet supported",
                            ));
                        }
                        let id = parameter.definition.ok_or_else(|| {
                            error(parameter.span, "missing normal-form parameter identity")
                        })?;
                        if !sig.modifiers.is_normal_form_head {
                            type_parameters.push(DeclarationId {
                                package: semantic_package(&self.package_identities, id.pkg),
                                local: id.index,
                            });
                        }
                        normal_parameters.push((
                            DeclarationId {
                                package: semantic_package(&self.package_identities, id.pkg),
                                local: id.index,
                            },
                            local(parameter.hir_id),
                            ty,
                        ));
                        continue;
                    }
                } else if !matches!(
                    parameter.kind,
                    hir::clause::ClauseParamKind::Type(_)
                        | hir::clause::ClauseParamKind::TraitBound(..)
                ) {
                    return Err(error(
                        parameter.span,
                        "comptime/generic/contract clause kind is not implemented",
                    ));
                }
                let id = parameter
                    .definition
                    .ok_or_else(|| error(parameter.span, "missing clause parameter identity"))?;
                type_parameters.push(DeclarationId {
                    package: semantic_package(&self.package_identities, id.pkg),
                    local: id.index,
                });
                if sig.modifiers.is_normal_form_head {
                    // Later input types may refer only to Type inputs already
                    // encountered in this declaration telescope.
                    self.type_parameters.insert(owner, type_parameters.clone());
                }
                normal_parameters.push((
                    DeclarationId {
                        package: semantic_package(&self.package_identities, id.pkg),
                        local: id.index,
                    },
                    local(parameter.hir_id),
                    p(P::Type),
                ));
            }
            let prefix = self.normal_input_prefixes.entry(owner).or_default();
            let prepared = prefix.len();
            prefix.extend(normal_parameters[prepared..].iter().cloned());
            capabilities = self.capabilities.get(&owner).cloned().unwrap_or_default();
            if had_contract {
                for requirement in saved_capabilities.iter().flatten() {
                    if !capabilities.contains(requirement) {
                        capabilities.push(requirement.clone());
                    }
                }
            }
            self.capabilities.insert(owner, capabilities);
            if !sig.modifiers.is_normal_form_head {
                let domains: Vec<_> = type_parameters
                    .iter()
                    .map(|id| {
                        normal_parameters
                            .iter()
                            .find(|(input, _, _)| input == id)
                            .map(|(_, _, ty)| ty.clone())
                            .or_else(|| self.trait_input_domain(*id))
                            .unwrap_or_else(|| p(P::Type))
                    })
                    .collect();
                for (id, domain) in type_parameters.iter().zip(&domains) {
                    self.implementation_index
                        .register_parameter_domain(*id, domain.clone())
                        .map_err(|message| error(span, message))?;
                    self.inherent_index
                        .register_parameter_domain(*id, domain.clone())
                        .map_err(|message| error(span, message))?;
                }
                self.trait_parameter_types.insert(owner, domains);
            }
            self.type_parameters.insert(owner, type_parameters);
            let mut parameters = normal_parameters
                .iter()
                .filter(|(_, _, ty)| sig.modifiers.is_normal_form_head && *ty != p(P::Type))
                .map(|(_, _, ty)| Parameter {
                    sort: Sort::ComptimeValue,
                    ty: ty.clone(),
                })
                .collect::<Vec<_>>();
            if sig.modifiers.is_normal_form_head {
                self.normal_parameters.insert(owner, normal_parameters);
            }
            for clause in sig.clause_constraints {
                if let hir::clause::ClauseConstraintKind::Requires(value) = clause.kind {
                    required::signature_capability_atoms(value, &mut pending_capabilities, 0)?;
                }
            }
            self.ready_signature_capabilities(owner, &mut pending_capabilities, false)?;
            if let ItemKind::Effect(_, modifiers) = &item.kind {
                if modifiers.is_direct_passing && (modifiers.is_async || !sig.params.is_empty()) {
                    return Err(error(
                        item.span,
                        "direct passing effects must be synchronous and have no parameters",
                    ));
                }
            }
            let mut annotations = Vec::new();
            for (parameter_index, parameter) in sig.params.iter().enumerate() {
                self.ready_signature_capabilities(owner, &mut pending_capabilities, false)?;
                if parameter.ty.is_catch() {
                    return Err(error(
                        parameter.span,
                        "catch parameters require continuation binding, which is not yet implemented for function signatures",
                    ));
                }
                if matches!(item.kind, ItemKind::Effect(..))
                    && !matches!(parameter.ty.kind, TyParamKind::Positional(_))
                {
                    return Err(error(
                        parameter.span,
                        "effect parameter protocol is not yet implemented",
                    ));
                }
                if parameter.ty.flags
                    & !(hir::common::TPARAM_IMPLICIT
                        | hir::common::TPARAM_LAMBDA
                        | hir::common::TPARAM_COMPTIME)
                    != 0
                {
                    return Err(error(
                        parameter.span,
                        "parameter protocols are not implemented",
                    ));
                }
                if (parameter.ty.is_implicit() || parameter.ty.is_lambda())
                    && (matches!(item.kind, ItemKind::Effect(..))
                        || !matches!(
                            parameter.ty.kind,
                            TyParamKind::Positional(_)
                                | TyParamKind::PositionalDependencyCatched(..)
                        ))
                {
                    return Err(error(
                        parameter.span,
                        "this parameter marker combination is not yet implemented",
                    ));
                }
                let annotation = match parameter.ty.kind {
                    TyParamKind::Itself { is_ref } => {
                        if parameter_index != 0 {
                            return Err(error(parameter.span, "self must be the first parameter"));
                        }
                        let subject = self.self_type(owner, parameter.span)?;
                        if is_ref {
                            open_functions::Annotation::Known(Term::Pointer(Box::new(subject)))
                        } else {
                            open_functions::Annotation::Known(subject)
                        }
                    }
                    TyParamKind::Optional(_, ty, _)
                    | TyParamKind::Positional(ty)
                    | TyParamKind::PositionalDependencyCatched(_, ty)
                    | TyParamKind::Varadic(_, ty) => self.signature_annotation(ty)?,
                };
                if annotation.is_pending() {
                    // This records staging only. A pending comptime domain is
                    // not yet known to be Type; invocation determines that sort
                    // before publishing a concrete function signature.
                    let phase = if parameter.ty.is_comptime() {
                        Sort::ComptimeValue
                    } else {
                        Sort::Runtime
                    };
                    annotations.push((phase, annotation));
                    self.signature_domains
                        .entry(owner)
                        .or_default()
                        .push((phase, None));
                    continue;
                }
                let open_functions::Annotation::Known(ty) = annotation else {
                    unreachable!()
                };
                let ty = if parameter.ty.is_comptime() {
                    self.signature_parameter_domain(ty, sig.clause_constraints, parameter.span)?
                } else {
                    ty
                };
                let parameter_type = Parameter {
                    sort: if parameter.ty.is_comptime() {
                        if ty == p(P::Type) {
                            Sort::Type
                        } else {
                            Sort::ComptimeValue
                        }
                    } else {
                        Sort::Runtime
                    },
                    ty,
                };
                // Ordinary inputs of a comptime function are available to
                // deferred signature computations without changing their sort.
                if sig.modifiers.is_comptime {
                    self.locals
                        .insert(local(parameter.hir_id), parameter_type.ty.clone());
                }
                annotations.push((
                    parameter_type.sort,
                    open_functions::Annotation::Known(parameter_type.ty.clone()),
                ));
                self.signature_domains
                    .entry(owner)
                    .or_default()
                    .push((parameter_type.sort, Some(parameter_type.ty.clone())));
                let pending = annotations
                    .iter()
                    .any(|(_, annotation)| annotation.is_pending());
                let index = if pending {
                    parameter_index
                } else {
                    parameters.len()
                };
                if parameter_type.sort == Sort::Type {
                    self.type_input_bindings.insert(
                        local(parameter.hir_id),
                        Term::Bound {
                            depth: 0,
                            index,
                            sort: Sort::Type,
                        },
                    );
                }
                if parameter_type.sort == Sort::ComptimeValue {
                    self.value_input_bindings.insert(
                        local(parameter.hir_id),
                        (
                            Term::Bound {
                                depth: 0,
                                index,
                                sort: Sort::ComptimeValue,
                            },
                            parameter_type.ty.clone(),
                        ),
                    );
                }
                if !pending {
                    parameters.push(parameter_type);
                    self.signature_prefixes
                        .entry(owner)
                        .or_default()
                        .push(parameters.last().unwrap().clone());
                }
            }
            self.ready_signature_capabilities(owner, &mut pending_capabilities, true)?;
            let result_annotation = if let Some(ty) = sig.return_ty {
                self.signature_annotation(ty)?
            } else {
                open_functions::Annotation::Known(p(P::Void))
            };
            // Staged body work waits for inputs even when the callable itself
            // is comptime; a closed signature does not complete those inputs.
            if result_annotation.is_pending()
                || annotations
                    .iter()
                    .any(|(_, annotation)| annotation.is_pending())
                || (!sig.modifiers.is_normal_form_head
                    && parameters
                        .iter()
                        .any(|parameter| parameter.sort != Sort::Runtime)
                    && self.has_staged_body_work(owner)?)
            {
                self.register_open_function(owner, annotations, result_annotation, item.span)?;
                return Ok(());
            }
            let open_functions::Annotation::Known(result) = result_annotation else {
                unreachable!()
            };
            if sig.modifiers.is_normal_form_head && result != p(P::Type) {
                return Err(error(
                    item.span,
                    "only Type-result normal-form heads are currently supported",
                ));
            }
            let signatures = if matches!(item.kind, ItemKind::FnDeclaration(_)) {
                &mut self.interface_signatures
            } else {
                &mut self.signatures
            };
            signatures.insert(
                owner,
                Term::Function {
                    parameters,
                    result: Box::new(result),
                },
            );
            Ok(())
        })();
        let signature_pending =
            std::mem::replace(&mut self.pending_type_uses, saved_pending).unwrap();
        if checked.is_ok() {
            self.signature_type_uses.extend(signature_pending);
        }
        self.signature_owner = saved_owner;
        self.locals = saved_locals;
        self.captures = saved_captures;
        self.result = saved_result;
        self.partial_return_context = saved_partial_return;
        self.pending_optional_returns = saved_optional_returns;
        self.pending_error_returns = saved_error_returns;
        self.inferred_return_sites = saved_return_sites;
        self.return_observations = saved_returns;
        self.loops = saved_loops;
        self.immutable = saved_immutable;
        self.context = saved_context;
        self.checking_contract = saved_contract;
        self.numeric_defaults = saved_defaults;
        self.active_signatures.remove(&owner);
        self.normal_input_prefixes.remove(&owner);
        self.signature_prefixes.remove(&owner);
        self.signature_domains.remove(&owner);
        if checked.is_err() {
            self.pending_refinements
                .retain(|pending| pending.owner != owner);
            self.type_input_bindings
                .retain(|binding, _| binding.owner != owner.def_id.raw());
            self.partial_staged_bindings
                .retain(|binding, _| binding.owner != owner.def_id.raw());
            self.value_input_bindings
                .retain(|binding, _| binding.owner != owner.def_id.raw());
            match saved_capabilities {
                Some(capabilities) => {
                    self.capabilities.insert(owner, capabilities);
                }
                None => {
                    self.capabilities.remove(&owner);
                }
            }
        }
        checked
    }

    fn optional_payload(&self, ty: &Term, span: Span) -> Result<Term, Error> {
        match self.type_identity_view(ty, span)? {
            Term::Optional(payload) => Ok(*payload),
            _ => Err(error(span, "optional operation requires an optional type")),
        }
    }
    fn instantiated_variants(&self, ty: &Term, span: Span) -> Result<Vec<n::Variant>, Error> {
        let Term::Nominal {
            declaration,
            arguments,
        } = ty
        else {
            return Err(error(span, "variant requires an enum type"));
        };
        self.enums
            .get(declaration)
            .ok_or_else(|| error(span, "missing enum descriptor"))?
            .instantiate_variants_in_scope(arguments, &self.call_input_frames())
            .map_err(|e| error(span, e.to_string()))
    }
    fn variant_target(
        &mut self,
        callee: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<Option<(Term, usize)>, Error> {
        let Some((ty, name)) = self.enum_member_reference(callee, expected)? else {
            return Ok(None);
        };
        let Term::Nominal { declaration, .. } = &ty else {
            return Ok(None);
        };
        let Some(definition) = self.enums.get(declaration) else {
            return Ok(None);
        };
        let index = definition
            .variants
            .iter()
            .position(|variant| variant.name == name.as_str());
        match index {
            Some(index) => Ok(Some((ty, index))),
            None if definition.groups.contains(&name) => Err(error(
                callee.span,
                "enum group is a pattern set, not a constructible value",
            )),
            // A qualified enum member may be an inherent method. Keep symbol
            // construction diagnostics, but let ordinary member lookup decide.
            None if matches!(callee.kind, H::Projection(..)) => Ok(None),
            None => Err(error(callee.span, "unknown enum variant")),
        }
    }
    fn construct_named_variant(
        &mut self,
        expr: &Expr<'_>,
        ty: Term,
        index: usize,
        args: &[Arg<'_>],
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let Term::Nominal {
            declaration: id,
            arguments,
        } = &ty
        else {
            unreachable!()
        };
        let id = *id;
        let variant = self.instantiated_variants(&ty, expr.span)?[index].clone();
        let n::VariantShape::Named(names) = &variant.shape else {
            return Err(error(expr.span, "enum variant does not have named fields"));
        };
        let mut supplied = BTreeMap::new();
        for arg in args {
            let Arg::Named(name, value) = arg else {
                return Err(error(expr.span, "enum payload fields must be fully named"));
            };
            let slot = names
                .iter()
                .position(|n| n == name.name.as_str())
                .ok_or_else(|| error(name.span, "unknown enum payload field"))?;
            if supplied.insert(slot, *value).is_some() {
                return Err(error(
                    name.span,
                    "enum payload field supplied more than once",
                ));
            }
        }
        let owner = self.package.owners().find_map(|(owner, _)| {
            (matches_declaration(self.package, &self.package_identities, owner, id))
                .then_some(owner)
        });
        let mut fields = Vec::new();
        for (slot, ty) in variant.fields.iter().enumerate() {
            fields.push(if let Some(value) = supplied.get(&slot) {
                self.expr(value, Some(ty))?
            } else {
                let owner = owner.ok_or_else(|| {
                    error(
                        expr.span,
                        format!("missing required enum payload field {}", names[slot]),
                    )
                })?;
                self.field_default(owner, Some(index), slot)?
                    .instantiate_declaration_parameters_in_scope(
                        &self.enums[&id].type_parameters,
                        &self.enums[&id].parameter_types,
                        arguments,
                        &self.call_input_frames(),
                    )
                    .map_err(|e| error(expr.span, e.to_string()))?
            });
        }
        let ty = if fields.iter().any(|f| is_never(&f.ty)) {
            p(P::Never)
        } else {
            ty
        };
        if let Some(expected) = expected {
            self.require(&ty, expected, expr.span)?;
        }
        Ok(n::Expr {
            ty,
            source: source(expr.span),
            kind: K::Variant {
                declaration: id,
                index,
                fields,
            },
        })
    }
    /// Infer a contextual enum payload without publishing candidate-local holes.
    fn infer_contextual_variant(
        &mut self,
        expr: &Expr<'_>,
        id: nir::DeclarationId,
        name: &str,
        args: &[Arg<'_>],
        context: &crate::calls::InputContext,
    ) -> Result<n::Expr, Error> {
        let definition = self
            .enums
            .get(&id)
            .ok_or_else(|| error(expr.span, "variant requires an enum type"))?
            .clone();
        let index = definition
            .variants
            .iter()
            .position(|variant| variant.name == name)
            .ok_or_else(|| error(expr.span, format!("unknown enum variant {name}")))?;
        let variant = &definition.variants[index];
        let owner = self
            .package
            .owners()
            .find_map(|(owner, _)| {
                (matches_declaration(self.package, &self.package_identities, owner, id))
                    .then_some(owner)
            })
            .ok_or_else(|| error(expr.span, "enum declaration owner is unavailable"))?;
        let binders = definition
            .parameter_binders()
            .map_err(|e| error(expr.span, e.to_string()))?;
        let mut open_binders = Vec::new();
        let mut supplied = Vec::new();
        for (slot, binder) in binders.iter().enumerate() {
            let domain = definition
                .parameter_types
                .get(slot)
                .cloned()
                .unwrap_or(p(P::Type));
            let domain = self.instantiate_input_types(
                &domain,
                &definition.type_parameters[..slot],
                &supplied,
                expr.span,
            )?;
            let sort = if domain == p(P::Type) {
                Sort::Type
            } else {
                Sort::ComptimeValue
            };
            let value = match context {
                crate::calls::InputContext::Nominal {
                    declaration,
                    arguments,
                } if *declaration == id => match arguments.get(slot) {
                    Some(crate::calls::InputContext::Known(value)) => Some(value),
                    _ => None,
                },
                crate::calls::InputContext::Known(Term::Nominal {
                    declaration,
                    arguments,
                }) if *declaration == id => arguments.get(slot),
                _ => None,
            }
            .filter(|value| value.sort() == sort);
            if let Some(value) = value {
                supplied.push(self.copy_alias_type(value, expr.span)?);
            } else {
                open_binders.push(nir::DeclarationParameter {
                    declaration: binder.declaration,
                    sort,
                });
                supplied.push(Term::Rigid {
                    declaration: binder.declaration,
                    sort,
                });
            }
        }
        let result = Term::Nominal {
            declaration: id,
            arguments: supplied.clone(),
        };
        let signature = Term::Function {
            parameters: variant
                .fields
                .iter()
                .map(|ty| {
                    Ok(Parameter {
                        sort: Sort::Runtime,
                        ty: self.instantiate_input_types(
                            ty,
                            &definition.type_parameters,
                            &supplied,
                            expr.span,
                        )?,
                    })
                })
                .collect::<Result<Vec<_>, Error>>()?,
            result: Box::new(
                result
                    .abstract_parameters(&[])
                    .map_err(|e| error(expr.span, e.to_string()))?,
            ),
        }
        .abstract_parameters(&open_binders)
        .map_err(|e| error(expr.span, e.to_string()))?;
        let scheme = crate::calls::Scheme {
            parameter_sorts: open_binders.iter().map(|binder| binder.sort).collect(),
            type_parameters: open_binders.len(),
            signature,
        };
        let mut inputs = vec![None; variant.fields.len()];
        let mut defaults = vec![None; variant.fields.len()];
        match (&variant.shape, &expr.kind) {
            (n::VariantShape::Tuple, H::Application(..)) => {
                if args.iter().any(|arg| matches!(arg, Arg::Expand(_))) {
                    let mut checked = BTreeMap::new();
                    let mut prepared_packets = BTreeMap::new();
                    let candidate = self.infer_enum_spread_inputs(
                        &scheme,
                        variant.fields.len(),
                        args,
                        context,
                        expr.span,
                        &mut checked,
                        &mut prepared_packets,
                    )?;
                    let fields = (0..variant.fields.len())
                        .map(|slot| {
                            candidate
                                .parameter_input_context(slot)
                                .map_err(|e| call_inference_error(expr.span, e))
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    let mut widths = BTreeMap::new();
                    for (index, packet) in &prepared_packets {
                        if let Term::Tuple(fields) = self.structural_type(&packet.ty, expr.span)? {
                            widths.insert(*index, fields.len());
                        }
                    }
                    let packets = self
                        .spread_packet_layout_with_widths(args, &fields, expr.span, &widths)?
                        .into_iter()
                        .map(|fields| {
                            fields.map(|fields| match self.protocols.tuple {
                                Some(head) => {
                                    crate::calls::InputContext::NormalTuple { head, fields }
                                }
                                None => crate::calls::InputContext::Tuple(fields),
                            })
                        })
                        .collect::<Vec<_>>();
                    return self.prepare_spreads_with_input(
                        expr,
                        args,
                        &packets,
                        checked,
                        prepared_packets,
                        |this, inputs| {
                            this.infer_contextual_variant(expr, id, name, inputs, context)
                        },
                    );
                }
                if args.len() != inputs.len() {
                    return Err(error(expr.span, "enum payload argument count mismatch"));
                }
                for (slot, arg) in args.iter().enumerate() {
                    let Arg::Positional(value) = arg else {
                        return Err(error(
                            expr.span,
                            "positional enum payload requires positional arguments",
                        ));
                    };
                    inputs[slot] = Some(*value);
                }
            }
            (n::VariantShape::Named(names), H::ExtendedApplication(..)) => {
                for arg in args {
                    let Arg::Named(name, value) = arg else {
                        return Err(error(expr.span, "enum payload fields must be fully named"));
                    };
                    let slot = names
                        .iter()
                        .position(|field| field == name.name.as_str())
                        .ok_or_else(|| error(name.span, "unknown enum payload field"))?;
                    if inputs[slot].replace(*value).is_some() {
                        return Err(error(
                            name.span,
                            "enum payload field supplied more than once",
                        ));
                    }
                }
                for (slot, input) in inputs.iter().enumerate() {
                    if input.is_none() {
                        defaults[slot] = Some(self.field_default(owner, Some(index), slot)?);
                    }
                }
            }
            _ => {
                return Err(error(
                    expr.span,
                    "enum construction syntax does not match the variant payload",
                ));
            }
        }
        let scope = self.call_inference_domains();
        let mut checked: Vec<Option<n::Expr>> = vec![None; inputs.len()];
        let instance = loop {
            let actual = checked
                .iter()
                .map(|value| value.as_ref().and_then(arguments::inference_type).cloned())
                .collect::<Vec<_>>();
            let mut candidate = scheme
                .infer_sparse_scoped_types(
                    &actual,
                    None,
                    &self.context,
                    crate::ptc::Budget::default(),
                    &scope,
                )
                .map_err(|e| call_inference_error(expr.span, e))?;
            candidate
                .constrain_result_context(context, crate::ptc::Budget::default())
                .map_err(|e| call_inference_error(expr.span, e))?;
            candidate
                .refine_type_views_metered(
                    |ty, budget| {
                        self.callable_input_view_metered(ty, budget, expr.span)
                            .map_err(|e| e.message)
                    },
                    crate::ptc::Budget::default(),
                )
                .map_err(|e| call_inference_error(expr.span, e))?;
            if checked
                .iter()
                .zip(&inputs)
                .all(|(value, input)| value.is_some() || input.is_none())
            {
                break candidate
                    .finish()
                    .map_err(|e| call_inference_error(expr.span, e))?;
            }
            let mut progress = false;
            for defaults in [false, true] {
                if defaults && (!self.numeric_defaults || progress) {
                    break;
                }
                for (slot, input) in inputs.iter().enumerate() {
                    let Some(input) = input else {
                        continue;
                    };
                    if checked[slot].is_some() {
                        continue;
                    }
                    let input_context = candidate
                        .parameter_input_context(slot)
                        .map_err(|e| call_inference_error(input.span, e))?;
                    let expected = candidate
                        .parameter_type(slot)
                        .map_err(|e| call_inference_error(input.span, e))?;
                    checked[slot] = self.argument_attempt_with_input(
                        input,
                        expected.as_ref(),
                        None,
                        defaults,
                        Some(&input_context),
                    )?;
                    if checked[slot].is_some() {
                        progress = true;
                        // Rebuild constraints before another payload can default.
                        break;
                    }
                }
                if progress {
                    break;
                }
            }
            if !progress {
                return Err(context_error(
                    expr.span,
                    "enum payload inference needs more type information",
                ));
            }
        };
        let complete_arguments = supplied
            .iter()
            .map(|value| {
                value
                    .instantiate_parameters(&open_binders, &instance.type_arguments)
                    .map_err(|e| error(expr.span, e.to_string()))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        self.check_type_application(owner, &complete_arguments, expr.span)?;
        let variants = self.instantiated_variants(&instance.result, expr.span)?;
        let mut fields = Vec::with_capacity(checked.len());
        for (slot, (value, default)) in checked.into_iter().zip(defaults).enumerate() {
            let value = match value {
                Some(value) => value,
                None => default
                    .expect("omitted payload has a checked default")
                    .instantiate_declaration_parameters_in_scope(
                        &definition.type_parameters,
                        &definition.parameter_types,
                        &complete_arguments,
                        &self.call_input_frames(),
                    )
                    .map_err(|e| error(expr.span, e.to_string()))?,
            };
            self.require(&value.ty, &variants[index].fields[slot], expr.span)?;
            fields.push(value);
        }
        Ok(n::Expr {
            source: source(expr.span),
            ty: if fields.iter().any(|value| is_never(&value.ty)) {
                p(P::Never)
            } else {
                instance.result
            },
            kind: K::Variant {
                declaration: id,
                index,
                fields,
            },
        })
    }
    fn construct_variant(
        &mut self,
        expr: &Expr<'_>,
        ty: Term,
        index: usize,
        args: &[Arg<'_>],
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let Term::Nominal { declaration, .. } = &ty else {
            unreachable!()
        };
        let declaration = *declaration;
        let variant = self.instantiated_variants(&ty, expr.span)?[index].clone();
        if variant.shape != n::VariantShape::Tuple {
            return Err(error(
                expr.span,
                "unit enum variant does not take arguments",
            ));
        }
        if args.iter().any(|arg| matches!(arg, Arg::Expand(_))) {
            let contexts = self.spread_packet_contexts(args, &variant.fields, expr.span)?;
            return self.prepare_spreads(expr, args, &contexts, |this, inputs, diverges| {
                this.construct_variant(expr, ty, index, inputs, expected.filter(|_| !diverges))
            });
        }
        if args.len() != variant.fields.len() {
            return Err(error(expr.span, "enum payload argument count mismatch"));
        }
        let mut fields = Vec::new();
        for (arg, ty) in args.iter().zip(&variant.fields) {
            let Arg::Positional(value) = arg else {
                return Err(error(
                    expr.span,
                    "positional enum payload requires positional arguments",
                ));
            };
            fields.push(self.expr(value, Some(ty))?);
        }
        let ty = if fields.iter().any(|field| is_never(&field.ty)) {
            p(P::Never)
        } else {
            ty
        };
        if let Some(expected) = expected {
            self.require(&ty, expected, expr.span)?;
        }
        Ok(n::Expr {
            ty,
            source: source(expr.span),
            kind: K::Variant {
                declaration,
                index,
                fields,
            },
        })
    }
    fn enum_variant(&self, ty: &Term, name: &str, span: Span) -> Result<n::Expr, Error> {
        let shape = self.structural_type(ty, span)?;
        let ty = &shape;
        let Term::Nominal {
            declaration,
            arguments,
        } = ty
        else {
            return Err(incompatible_context(
                span,
                "variant requires a nominal enum context",
            ));
        };
        let definition = self
            .enums
            .get(declaration)
            .ok_or_else(|| error(span, "variant requires an enum type"))?;
        definition
            .instantiate_variants_in_scope(arguments, &self.call_input_frames())
            .map_err(|e| error(span, e.to_string()))?;
        let index = definition
            .variants
            .iter()
            .position(|variant| variant.name == name)
            .ok_or_else(|| error(span, format!("unknown enum variant {name}")))?;
        if definition.variants[index].shape != n::VariantShape::Unit {
            return Err(error(span, "enum payload variant requires arguments"));
        }
        Ok(n::Expr {
            ty: ty.clone(),
            source: source(span),
            kind: K::Variant {
                declaration: *declaration,
                index,
                fields: Vec::new(),
            },
        })
    }
    fn bound_tuple_elements<'a>(&self, ty: &'a Term) -> Option<&'a [Term]> {
        if let Term::NormalForm {
            head,
            sort: Sort::Type,
            arguments,
        } = ty
        {
            if Some(*head) == self.protocols.tuple {
                if let [Term::TypeListValue(elements)] = arguments.as_slice() {
                    return Some(elements);
                }
                let owner = self.normal_parameters.keys().find(|owner| {
                    matches_declaration(self.package, &self.package_identities, **owner, *head)
                })?;
                return self.normal_form_head_metadata(*owner)?.tuple_elements(ty);
            }
        }
        None
    }
    fn structural_type(&self, ty: &Term, span: Span) -> Result<Term, Error> {
        let ty = self.type_identity_view(ty, span)?;
        if let Some(elements) = self.bound_tuple_elements(&ty) {
            return Ok(Term::Tuple(elements.to_vec()));
        }
        Ok(ty)
    }
    fn type_identity_view(&self, ty: &Term, span: Span) -> Result<Term, Error> {
        if self.bound_tuple_elements(ty).is_some() || self.context.is_list_type(ty) {
            return Ok(ty.clone());
        }
        if !matches!(
            ty,
            Term::Bound { .. }
                | Term::AssociatedType { .. }
                | Term::Rigid { .. }
                | Term::NormalForm { .. }
                | Term::Hole { .. }
        ) {
            return Ok(ty.clone());
        }
        self.context
            .structural_view_with_heads_metered(ty, crate::ptc::Budget::default(), |head| {
                Some(head) == self.protocols.tuple
            })
            .0
            .map(|view| {
                let view = view.map(|(ty, _)| ty).unwrap_or_else(|| ty.clone());
                view
            })
            .map_err(|reason| {
                error(
                    span,
                    format!("structural type view remains unresolved: {reason:?}; type {ty}"),
                )
            })
    }
    fn negated_pattern_domain(
        &self,
        ty: &Term,
        fuel: &mut usize,
        depth: usize,
        cache: &mut std::collections::HashMap<Term, bool>,
    ) -> Result<bool, String> {
        if depth >= 128 {
            return Err("negated pattern type depth exceeded".into());
        }
        *fuel = fuel
            .checked_sub(1)
            .ok_or("negated pattern type budget exhausted")?;
        let symbolic = matches!(
            ty,
            Term::Bound { .. }
                | Term::AssociatedType { .. }
                | Term::Rigid { .. }
                | Term::NormalForm { .. }
                | Term::Hole { .. }
        );
        if symbolic {
            crate::associated::charge_projection_copy(ty, fuel)?;
            if let Some(answer) = cache.get(ty) {
                return Ok(*answer);
            }
        }
        let original = ty;
        let exposed;
        let ty = if symbolic {
            let (view, used) = self.context.structural_view_with_heads_metered(
                ty,
                // The prover reserves its allowance. Leave budget for sibling
                // fields instead of letting the first generic field take all.
                crate::ptc::Budget {
                    steps: (*fuel / 2).min(10_000),
                    max_depth: 128,
                },
                |head| Some(head) == self.protocols.tuple,
            );
            *fuel = fuel.saturating_sub(used);
            let Some((view, _)) = view.map_err(|reason| {
                format!("negated pattern type view remains unresolved: {reason:?}")
            })?
            else {
                return Ok(false);
            };
            exposed = if let Some(elements) = self.bound_tuple_elements(&view) {
                crate::associated::charge_projection_copy(&view, fuel)?;
                Term::Tuple(elements.to_vec())
            } else {
                view
            };
            &exposed
        } else {
            ty
        };
        let answer = match ty {
            Term::Primitive(primitive) => {
                matches!(
                    primitive,
                    P::Void
                        | P::Bool
                        | P::Char
                        | P::Str
                        | P::Symbol
                        | P::Integer
                        | P::Float
                        | P::F32
                        | P::F64
                        | P::BitVec(_)
                ) || self.integer_layout(*primitive).is_some()
            }
            Term::Tuple(fields) => {
                for field in fields {
                    if !self.negated_pattern_domain(field, fuel, depth + 1, cache)? {
                        return Ok(false);
                    }
                }
                true
            }
            Term::Record(fields) => {
                for (_, field) in fields {
                    if !self.negated_pattern_domain(field, fuel, depth + 1, cache)? {
                        return Ok(false);
                    }
                }
                true
            }
            Term::ErrorResult { errors, result } => {
                for field in std::iter::once(result.as_ref()).chain(errors) {
                    if !self.negated_pattern_domain(field, fuel, depth + 1, cache)? {
                        return Ok(false);
                    }
                }
                true
            }
            Term::Nominal { declaration, .. } => {
                let variants = self.enums.get(declaration).map(|item| item.variants.len());
                for index in 0..variants.unwrap_or(1) {
                    let fields = nir::normal_forms::nominal_value_fields_with_lookup_in(
                        ty,
                        variants.map(|_| index as u32),
                        fuel,
                        &|id, fuel| {
                            *fuel = fuel.checked_sub(1)?;
                            self.structs
                                .get(&id)
                                .map(nir::normal_forms::NominalValueDeclaration::Struct)
                                .or_else(|| {
                                    self.enums
                                        .get(&id)
                                        .map(nir::normal_forms::NominalValueDeclaration::Enum)
                                })
                        },
                        self.target,
                    )
                    .ok_or("negated nominal pattern layout is unresolved or budget exhausted")?;
                    for field in fields {
                        if !self.negated_pattern_domain(&field, fuel, depth + 1, cache)? {
                            return Ok(false);
                        }
                    }
                }
                // This admits observation syntax, not duplication or cleanup.
                // Resource-checked execution still acquires Copy evidence for
                // the candidate-local subject and every represented component.
                true
            }
            _ => false,
        };
        if symbolic {
            crate::associated::charge_projection_copy(original, fuel)?;
            cache.insert(original.clone(), answer);
        }
        Ok(answer)
    }

    fn patterns_exhaustive(
        &self,
        patterns: &[&n::Pattern],
        ty: &Term,
    ) -> crate::coverage::Coverage {
        crate::coverage::check_metered_in(
            &self.structs,
            &self.enums,
            ty,
            patterns,
            100_000,
            &self.context,
            self.protocols.tuple,
            self.target,
        )
        .0
    }
    fn pattern_exhaustive(&self, pattern: &n::Pattern, ty: &Term) -> bool {
        self.patterns_exhaustive(&[pattern], ty) == crate::coverage::Coverage::Exhaustive
    }
    fn field_default(
        &mut self,
        owner: OwnerId,
        variant: Option<usize>,
        index: usize,
    ) -> Result<n::Expr, Error> {
        if let Some(value) = self.field_defaults.get(&(owner, variant, index)) {
            return Ok(value.clone());
        }
        let item = self.package.item(owner).unwrap();
        let (field, ty) = match (&item.kind, variant) {
            (ItemKind::Struct(def), None) => (
                &def.fields[index],
                self.structs[&declaration(self.package, &self.package_identities, owner)].fields
                    [index]
                    .1
                    .clone(),
            ),
            (ItemKind::Enum(_), Some(v)) => {
                let id = declaration(self.package, &self.package_identities, owner);
                let hir::item::VariantKind::Struct(fields) = &self.enum_sources[&id][v].kind else {
                    return Err(error(item.span, "missing required enum payload field"));
                };
                (
                    &fields[index],
                    self.enums[&declaration(self.package, &self.package_identities, owner)]
                        .variants[v]
                        .fields[index]
                        .clone(),
                )
            }
            _ => unreachable!(),
        };
        let value = field.default.ok_or_else(|| {
            error(
                field.span,
                format!(
                    "missing required {} field {}",
                    if variant.is_some() {
                        "enum payload"
                    } else {
                        "struct"
                    },
                    field.ident.name
                ),
            )
        })?;
        if !self.active_defaults.insert((owner, variant, index)) {
            return Err(error(field.span, "cyclic field default elaboration"));
        }
        let locals = std::mem::take(&mut self.locals);
        let captures = std::mem::take(&mut self.captures);
        let result = self.result.take();
        let partial_return_context = self.partial_return_context.take();
        let optional_returns = std::mem::take(&mut self.pending_optional_returns);
        let error_returns = std::mem::take(&mut self.pending_error_returns);
        let return_sites = std::mem::take(&mut self.inferred_return_sites);
        let returns = self.return_observations.take();
        let loops = std::mem::take(&mut self.loops);
        let base_context = self.definition_context(owner);
        let context = std::mem::replace(&mut self.context, base_context);
        let signature_owner = self.signature_owner.replace(owner);
        let checked = self.expr(value, Some(&ty));
        self.signature_owner = signature_owner;
        self.context = context;
        self.locals = locals;
        self.captures = captures;
        self.result = result;
        self.partial_return_context = partial_return_context;
        self.pending_optional_returns = optional_returns;
        self.pending_error_returns = error_returns;
        self.inferred_return_sites = return_sites;
        self.return_observations = returns;
        self.loops = loops;
        self.active_defaults.remove(&(owner, variant, index));
        let checked = checked?;
        self.field_defaults
            .insert((owner, variant, index), checked.clone());
        Ok(checked)
    }
    fn construct(
        &mut self,
        expr: &Expr<'_>,
        callee: &Expr<'_>,
        args: &[Arg<'_>],
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let ty = self.type_expr(callee)?;
        self.construct_typed(expr, ty, args, expected)
    }
    fn construct_typed(
        &mut self,
        expr: &Expr<'_>,
        ty: Term,
        args: &[Arg<'_>],
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        self.ensure_nominal_type(&ty, expr.span)?;
        let Term::Nominal {
            declaration: id,
            arguments,
        } = &ty
        else {
            return Err(error(
                expr.span,
                "construction requires a supported struct type",
            ));
        };
        let def = self
            .structs
            .get(id)
            .ok_or_else(|| error(expr.span, "missing struct descriptor"))?
            .clone();
        let instantiated_fields = def
            .instantiate_fields_in_scope(arguments, &self.call_input_frames())
            .map_err(|e| error(expr.span, e.to_string()))?;
        let owner = self.package.owners().find_map(|(owner, _)| {
            (matches_declaration(self.package, &self.package_identities, owner, *id))
                .then_some(owner)
        });
        let mut supplied = BTreeMap::new();
        for arg in args {
            let Arg::Named(name, value) = arg else {
                return Err(error(expr.span, "struct fields must be fully named"));
            };
            let index = def
                .fields
                .iter()
                .position(|(field, _)| field == name.name.as_str())
                .ok_or_else(|| error(name.span, format!("unknown struct field {}", name.name)))?;
            if supplied.insert(index, *value).is_some() {
                return Err(error(name.span, "struct field supplied more than once"));
            }
        }
        let mut fields = Vec::new();
        for (index, (_, field_type)) in instantiated_fields.iter().enumerate() {
            fields.push(if let Some(value) = supplied.get(&index) {
                let field_type = self.callable_input_view(field_type, value.span)?;
                self.expr(value, Some(&field_type))?
            } else {
                self.field_default(
                    owner.ok_or_else(|| {
                        error(
                            expr.span,
                            format!("missing struct field {}", def.fields[index].0),
                        )
                    })?,
                    None,
                    index,
                )?
                .instantiate_declaration_parameters_in_scope(
                    &def.type_parameters,
                    &def.parameter_types,
                    arguments,
                    &self.call_input_frames(),
                )
                .map_err(|e| error(expr.span, e.to_string()))?
            });
        }
        let actual = if fields.iter().any(|f| is_never(&f.ty)) {
            p(P::Never)
        } else {
            ty
        };
        if let Some(expected) = expected {
            self.require(&actual, expected, expr.span)?;
        }
        Ok(n::Expr {
            ty: actual,
            source: source(expr.span),
            kind: K::Record {
                declaration: def.declaration,
                fields,
            },
        })
    }
    fn check_struct_cycles(&self) -> Result<(), Error> {
        let mut representations = crate::representations::Checker::new(
            self.protocols.tuple,
            &self.structs,
            &self.enums,
            &self.newtypes,
        );
        let mut fuel = 100_000;
        for (owner, info) in self.package.owners() {
            if matches!(
                &info.node.expect_item().kind,
                ItemKind::Struct(_) | ItemKind::Enum(_) | ItemKind::Newtype(..)
            ) {
                let mut arguments: Vec<_> = self
                    .declaration_binders(owner)
                    .into_iter()
                    .map(|parameter| Term::Rigid {
                        declaration: parameter.declaration,
                        sort: parameter.sort,
                    })
                    .collect();
                arguments.extend(
                    self.nominal_captures(owner, info.node.expect_item().span)?
                        .into_iter()
                        .map(|declaration| Term::Rigid {
                            declaration,
                            sort: self.input_binders(&[declaration])[0].sort,
                        }),
                );
                representations
                    .check(
                        &Term::Nominal {
                            declaration: declaration(self.package, &self.package_identities, owner),
                            arguments,
                        },
                        &mut fuel,
                    )
                    .map_err(|failure| error(info.node.expect_item().span, failure.to_string()))?;
            }
        }
        Ok(())
    }
    fn when_expr(
        &mut self,
        expr: &Expr<'_>,
        arms: &[hir::expr::CondictionArm<'_>],
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        self.when_expr_with_input(expr, arms, expected, partial, None)
    }
    fn when_expr_with_input(
        &mut self,
        expr: &Expr<'_>,
        arms: &[hir::expr::CondictionArm<'_>],
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
        input_context: Option<&crate::calls::InputContext>,
    ) -> Result<n::Expr, Error> {
        let parent = self.context.clone();
        let result = self.when_expr_in_context(expr, arms, expected, partial, input_context);
        self.context = parent;
        result
    }
    fn when_expr_in_context(
        &mut self,
        expr: &Expr<'_>,
        arms: &[hir::expr::CondictionArm<'_>],
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
        input_context: Option<&crate::calls::InputContext>,
    ) -> Result<n::Expr, Error> {
        let fallback = arms.iter().position(|arm| arm.cond.is_none());
        if fallback.is_some_and(|index| index + 1 != arms.len()) {
            return Err(error(expr.span, "when fallback must be the final arm"));
        }
        let inferred = if expected.is_none() && fallback.is_some() {
            match input_context {
                Some(context) => self.optional_branch_context(expr.span, context, arms.len(), |checker, index| {
                    // The probe must see the same ordered path as the final
                    // check. checking_attempt rolls all these facts back.
                    for (prior, arm) in arms[..=index].iter().enumerate() {
                        checker.optional_branch_probe_budget=checker.optional_branch_probe_budget.checked_sub(1)
                            .ok_or_else(||error(expr.span,"optional branch inference budget exhausted; add a type annotation"))?;
                        if let Some(condition)=arm.cond {
                            let condition=checker.expr(condition,Some(&p(P::Bool)))?;
                            checker.assume_type_condition(&condition,prior==index,arm.body.span)?;
                        }
                    }
                    checker.input_context_expression(arms[index].body, context)
                })?,
                None => None,
            }
        } else {
            None
        };
        let expected = expected.or(inferred.as_ref());
        let mut branches = Vec::new();
        let join_context = self.context.clone();
        let mut branch_type = if fallback.is_some() {
            expected.cloned()
        } else {
            None
        };
        let selection = self
            .inline_selection
            .filter(|(id, _)| *id == expr.hir_id)
            .map(|(_, index)| index);
        for (index, arm) in arms.iter().enumerate() {
            let suppressed = self.in_unselected_inline;
            self.in_unselected_inline =
                suppressed || selection.is_some_and(|selected| selected != index);
            let checked = (|| {
                let condition = arm
                    .cond
                    .map(|cond| self.expr(cond, Some(&p(P::Bool))))
                    .transpose()?;
                if selection.is_some() && !self.checking_open_behavior {
                    if let Some(condition) = &condition {
                        self.check_inline_input_availability(
                            condition,
                            &BTreeSet::new(),
                            arm.body.span,
                        )?;
                    }
                }
                let check = |checker: &mut Self| {
                    checker.with_callee_delivery(expr.hir_id, Some(arm.body.hir_id), |checker| {
                        match input_context {
                            Some(input_context) if fallback.is_some() && branch_type.is_none() => {
                                checker.input_context_expression(arm.body, input_context)
                            }
                            _ => checker.expr_context(
                                arm.body,
                                branch_type.as_ref(),
                                if fallback.is_some() { partial } else { None },
                            ),
                        }
                    })
                };
                let mut body = match &condition {
                    Some(condition) => {
                        self.under_type_condition(condition, true, arm.body.span, check)?
                    }
                    None => check(self)?,
                };
                if branch_type.is_none() && !is_never(&body.ty) {
                    branch_type = Some(body.ty.clone());
                }
                if fallback.is_none() {
                    body = discard_result(body);
                }
                if let Some(condition) = &condition {
                    self.assume_type_condition(condition, false, arm.body.span)?;
                }
                Ok((condition, body))
            })();
            self.in_unselected_inline = suppressed;
            branches.push(checked?);
        }
        self.context = join_context;
        let selected_body = selection.map(|index| {
            branches
                .get(index)
                .map(|(_, body)| body.clone())
                .unwrap_or(n::Expr {
                    ty: p(P::Void),
                    source: source(expr.span),
                    kind: K::Unit,
                })
        });
        let mut tail = n::Expr {
            ty: p(P::Void),
            source: source(expr.span),
            kind: K::Unit,
        };
        for (condition, body) in branches.into_iter().rev() {
            let Some(condition) = condition else {
                tail = body;
                continue;
            };
            let ty = if control::conditional_never(&condition, &body.ty, &tail.ty) {
                p(P::Never)
            } else if fallback.is_some() && branch_type.is_some() {
                // Each completing arm was checked against this context under
                // its own ordered type facts, including a sole surviving arm.
                branch_type.clone().unwrap()
            } else if is_never(&body.ty) {
                tail.ty.clone()
            } else if is_never(&tail.ty) {
                body.ty.clone()
            } else {
                self.require(&tail.ty, &body.ty, expr.span)?;
                body.ty.clone()
            };
            tail = n::Expr {
                ty,
                source: source(expr.span),
                kind: K::If {
                    condition: Box::new(condition),
                    then_branch: Box::new(body),
                    else_branch: Some(Box::new(tail)),
                },
            };
        }
        if let Some(expected) = expected {
            self.require(&tail.ty, expected, expr.span)?;
        }
        Ok(selected_body.unwrap_or(tail))
    }
    fn is_type_head(&self, expr: &Expr<'_>) -> bool {
        if self.associated_reference_is_type(expr) {
            return true;
        }
        match &expr.kind {
            H::RefinedType { .. }
            | H::DeliveredType(_)
            | H::RefinementSubject
            | H::EffectQualifiedType(..)
            | H::ErrorQualifiedType(..)
            | H::TypeOf(_)
            | H::Builtin(_)
            | H::TyType
            | H::TyVoid
            | H::TyNoReturn
            | H::TyPtr(_)
            | H::TyOptional(_)
            | H::TyClosure(..)
            | H::TyFn(_)
            | H::TyComptimeFn(_)
            | H::TyNFFn(_)
            | H::TyModifiedFn(..)
            | H::TyFnArrow(..) => true,
            H::TySelf => {
                self.self_types.contains_key(&expr.hir_id.owner)
                    || self.self_containers.contains_key(&expr.hir_id.owner)
            }
            H::ResolvedClauseParam(_, id) => {
                let declaration = DeclarationId {
                    package: semantic_package(&self.package_identities, id.pkg),
                    local: id.index,
                };
                self.normal_default_bindings
                    .get(&declaration)
                    .map(|(_, ty)| ty.clone())
                    .or_else(|| self.trait_input_domain(declaration))
                    .or_else(|| {
                        self.normal_parameters
                            .values()
                            .flatten()
                            .find(|(id, _, _)| *id == declaration)
                            .map(|(_, _, ty)| ty.clone())
                    })
                    .is_none_or(|ty| ty == p(P::Type))
            }
            H::NFApplication(head, _) => {
                self.is_type_head(head)
                    || matches!(head.kind,
                H::ResolvedDef { def_id, .. } if self.package.definition_owner(def_id)
                    .and_then(|o| self.package.item(o)).is_some_and(|i| matches!(&i.kind,
                        ItemKind::Fn(sig, _) if sig.modifiers.is_normal_form_head)))
            }
            H::Projection(base, _) if self.is_type_head(base) => {
                let is_type = |id| {
                    self.package
                        .definition_owner(id)
                        .and_then(|owner| self.package.item(owner))
                        .is_some_and(|item| {
                            matches!(
                                item.kind,
                                ItemKind::Struct(_)
                                    | ItemKind::Enum(_)
                                    | ItemKind::Newtype(..)
                                    | ItemKind::TypeAlias(..)
                            )
                        })
                };
                if let Some(member) = self.package.member_projections.get(&expr.hir_id) {
                    is_type(*member)
                } else {
                    self.package
                        .deferred_member_projections
                        .get(&expr.hir_id)
                        .is_some_and(|members| {
                            !members.is_empty()
                                && members.iter().all(|(_, member)| is_type(*member))
                        })
                }
            }
            H::ResolvedDef { def_id, .. } => self
                .package
                .definition_owner(*def_id)
                .and_then(|owner| self.package.item(owner))
                .is_some_and(|item| {
                    matches!(
                        item.kind,
                        ItemKind::Struct(_)
                            | ItemKind::Enum(_)
                            | ItemKind::Newtype(..)
                            | ItemKind::TypeAlias(..)
                    )
                }),
            _ => false,
        }
    }
    fn type_expr(&mut self, expr: &Expr<'_>) -> Result<Term, Error> {
        if let H::DeliveredType(ty) = &expr.kind {
            return self.delivered_type(expr.hir_id, ty, expr.span);
        }
        if let H::ResolvedLocal(_, id) = expr.kind
            && !self.callable_type_inputs.contains_key(&local(id))
            && !self.generator_data.contains_key(&local(id))
            && let Some(ty) = self.observed_local_type(local(id), expr.span)?
        {
            return Ok(ty);
        }
        if let H::Projection(value, field) = &expr.kind {
            if field.name.as_str() == "Closure"
                && let H::Projection(base, namespace) = &value.kind
                && namespace.name.as_str() == "vars"
                && self.metatype_namespace_receiver(base)?
            {
                return Ok(Term::ClosureContextType);
            }
        }
        // This is a type position: computed member receivers are required
        // inputs, not runtime values to deliver through Splice. Ordinary
        // expression projections retain the caller's phase in their probe.
        let saved_mode = std::mem::replace(&mut self.in_required_comptime, true);
        let projected = self.projected_member_type(expr);
        self.in_required_comptime = saved_mode;
        if let Some(ty) = projected? {
            return Ok(ty);
        }
        if let Some(ty) = self.observed_field_type(expr)? {
            return Ok(ty);
        }
        if self.projected_constant_owner(expr)?.is_some() {
            return match self.required_expression(expr, Some(&p(P::Type)))? {
                comptime_interpretor::Value::Type(ty) => Ok(ty),
                _ => Err(error(
                    expr.span,
                    "constant used as a type does not contain a Type value",
                )),
            };
        }
        if let H::RefinementSubject = &expr.kind {
            return self
                .refinement_subjects
                .last()
                .cloned()
                .ok_or_else(|| error(expr.span, "Itself has no active refinement subject"));
        }
        if let H::RefinedType { requirement, base } = &expr.kind {
            let base = self.type_expr(base)?;
            self.refinement_subjects.push(base.clone());
            let requirement_result = self.check_refinement_expression(requirement);
            self.refinement_subjects.pop();
            requirement_result?;
            return Ok(base);
        }
        if let H::Defer(value) = &expr.kind {
            // Signature annotations prepare open computations separately. Other
            // type positions still demand a completed computation here; never
            // manufacture an opaque normal-form head for unavailable inputs.
            return self.type_expr(value);
        }
        if let H::Projection(value, field) = &expr.kind {
            if let Some(value) =
                self.associated_projection(value, field.name.as_str(), expr.span, expr.hir_id)?
            {
                let K::TypeValue(ty) = value.kind else {
                    return Err(error(
                        expr.span,
                        "associated value used as a type does not contain a completed Type",
                    ));
                };
                return Ok(ty);
            }
        }
        match &expr.kind {
            H::ResolvedLocal(_, id) if self.callable_type_inputs.contains_key(&local(*id)) => {
                self.callable_type_inputs[&local(*id)].clone().ok_or_else(|| error(expr.span,
                    "function type dependency requires an earlier explicit comptime Type parameter"))
            }
            H::ResolvedLocal(_, id) if self.generator_data.contains_key(&local(*id)) => {
                match self.generator_data.get(&local(*id)) {
                    Some(Some(value)) if matches!(value.kind, K::TypeValue(_)) => {
                        constants::charge_copy(value, &mut self.required_steps, expr.span)?;
                        let K::TypeValue(ty) = &value.kind else {
                            unreachable!()
                        };
                        Ok(ty.clone())
                    }
                    _ => Err(error(
                        expr.span,
                        "local constant used as a type must contain a completed Type value",
                    )),
                }
            }
            H::ResolvedLocal(_, id)
                if self.derived_staged_bindings.contains(&local(*id))
                    && self.locals.get(&local(*id)) == Some(&p(P::Type))
                    && self.type_input_bindings.contains_key(&local(*id)) =>
            {
                // A checked const field can have a complete Type identity even
                // when another field still depends on invocation-time inputs.
                let ty = &self.type_input_bindings[&local(*id)];
                let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let before = fuel;
                let charged = nir::type_computations::charge_term(ty, &mut fuel);
                self.required_steps -= (before - fuel) as u64;
                charged.map_err(|e| error(expr.span, e.to_string()))?;
                Ok(ty.clone())
            }
            H::ResolvedFnParam(_, id) => {
                if let Some((symbol, domain)) = self.captured_staged_input(*id, expr.span)? {
                    if domain != p(P::Type) {
                        return Err(error(expr.span, "captured comptime value is not a Type"));
                    }
                    return Ok(symbol);
                }
                let binding = local(*id);
                let in_signature = self.signature_owner.is_some_and(|owner| {
                    owner == id.owner && self.active_signatures.contains(&owner)
                });
                self.type_input_bindings
                    .get(&binding)
                    .filter(|_| {
                        in_signature
                            || self.locals.contains_key(&binding)
                            || (self.checking_contract && self.signature_owner == Some(id.owner))
                    })
                    .cloned()
                    .ok_or_else(|| {
                        error(
                            expr.span,
                            "type annotation requires an earlier comptime Type parameter",
                        )
                    })
            }
            H::ErrorQualifiedType(errors, result) => {
                let labels = match &errors.kind {
                    H::List(items) => items
                        .iter()
                        .map(|item| self.type_expr(item))
                        .collect::<Result<Vec<_>, _>>()?,
                    _ => vec![self.type_expr(errors)?],
                };
                Ok(Term::error_result(labels, self.type_expr(result)?))
            }
            H::EffectQualifiedType(effects, result) => {
                let operations = self.effect_row(effects, 0)?;
                Ok(Term::computation(operations, self.type_expr(result)?))
            }
            H::Application(..)
            | H::ExtendedApplication(..)
            | H::Projection(..)
            | H::Index(..)
            | H::If(..)
            | H::When(_)
            | H::Match(..)
            | H::Block(_) => {
                // After member/associated-type and observed-field selection,
                // remaining projections are ordinary computations. Evaluate
                // the complete receiver and projection at the required stage;
                // a Type field's declared domain alone is not its identity.
                if !self.generator_data.is_empty() && self.data_consumer_call(expr).is_none() {
                    return self.generator_required_type(expr);
                }
                let value = self.required_value(expr, P::Type)?;
                let comptime_interpretor::Value::Type(ty) = value else {
                    unreachable!()
                };
                Ok(ty)
            }
            H::TySelf => {
                let ty = self.self_type(expr.hir_id.owner, expr.span)?;
                let identity = match &ty {
                    Term::Nominal { declaration, .. } | Term::Rigid { declaration, .. } => {
                        Some(*declaration)
                    }
                    _ => None,
                };
                if identity.is_some_and(|id| self.alias_value_bindings.contains_key(&id)) {
                    return Err(error(
                        expr.span,
                        "captured Self input is not yet available in this definition",
                    ));
                }
                if let Term::Rigid { declaration, .. } = &ty {
                    if let Some(value) = self.alias_substitution.get(declaration).cloned() {
                        return self.copy_alias_type(&value, expr.span);
                    }
                }
                if !self.alias_substitution.is_empty() {
                    let parameters = self.input_binders(
                        &self.alias_substitution.keys().copied().collect::<Vec<_>>(),
                    );
                    let arguments: Vec<_> = self
                        .alias_substitution
                        .values()
                        .map(|ty| ty.as_ref().clone())
                        .collect();
                    return ty
                        .instantiate_parameters(&parameters, &arguments)
                        .map_err(|e| error(expr.span, e.to_string()));
                }
                Ok(ty)
            }
            H::NFApplication(head, args) => {
                if let H::Builtin(name) = &head.kind {
                    if name.as_str() == "BitVec" {
                        let [Arg::Positional(width)] = *args else {
                            return Err(error(
                                expr.span,
                                "builtin BitVec requires exactly one positional comptime width",
                            ));
                        };
                        if matches!(
                            &width.kind,
                            H::Lit(hir::Lit {
                                kind: hir::LitKind::Float(_),
                                ..
                            })
                        ) {
                            return Err(error(
                                width.span,
                                "BitVec width requires a nonnegative comptime integer",
                            ));
                        }
                        let exact = if let H::Lit(hir::Lit {
                            kind: hir::LitKind::Integer(spelling),
                            ..
                        }) = &width.kind
                        {
                            comptime_runtime::Integer::parse_literal(
                                spelling,
                                &mut comptime_runtime::Budget::default(),
                            )
                            .map_err(|e| error(width.span, e.to_string()))?
                        } else {
                            let comptime_interpretor::Value::Integer(value) =
                                self.required_value(width, P::Integer)?
                            else {
                                return Err(error(
                                    width.span,
                                    "BitVec width is still an abstract comptime value; symbolic widths are not supported at this type formation boundary",
                                ));
                            };
                            value
                        };
                        if exact.is_negative() {
                            return Err(error(
                                width.span,
                                "BitVec width must be a nonnegative comptime integer",
                            ));
                        }
                        let width = exact
                            .to_u128()
                            .ok()
                            .and_then(|width| usize::try_from(width).ok())
                            .ok_or_else(|| {
                                error(
                                    width.span,
                                    "BitVec width exceeds the implementation size limit",
                                )
                            })?;
                        return Ok(p(P::BitVec(width)));
                    }
                }
                if let Some(owner) = self.normal_form_constant_target(head)?
                    && !matches!(head.kind, H::ResolvedDef { def_id, .. } if self.package.definition_owner(def_id) == Some(owner))
                {
                    let definition = self.package.owner_definition(owner).ok_or_else(|| error(expr.span, "normal-form callable has no declaration identity"))?;
                    let target = Expr {
                        kind: H::ResolvedDef { name: self.package.item(owner).unwrap().ident.name, has_clause_param: true, def_id: definition },
                        ..(*head).clone()
                    };
                    let application = Expr { kind: H::NFApplication(&target, args), ..expr.clone() };
                    return self.type_expr(&application);
                }
                let H::ResolvedDef { def_id, .. } = head.kind else {
                    return match self.required_value(expr, P::Type)? {
                        comptime_interpretor::Value::Type(ty) => Ok(ty),
                        _ => Err(error(expr.span, "type application did not produce a completed Type")),
                    };
                };
                let owner = self
                    .package
                    .definition_owner(def_id)
                    .ok_or_else(|| error(expr.span, "type definition has no HIR owner"))?;
                if self.normal_parameters.contains_key(&owner) || matches!(&self.package.item(owner).unwrap().kind,
                    ItemKind::Fn(sig, _) if sig.modifiers.is_normal_form_head)
                {
                    self.ensure_signature(owner, expr.span)?;
                    if !self.normal_type_list_tail(owner) {
                        if let Some(arguments) = normal::literal_spreads(args) {
                            let expanded = Expr {
                                hir_id: expr.hir_id,
                                span: expr.span,
                                kind: H::NFApplication(head, &arguments),
                            };
                            return self.type_expr(&expanded);
                        }
                    }
                    if (self.has_optional_type_clauses(owner)
                        || self.normal_type_inputs_need_plan(owner, args)
                        || (!self.normal_type_list_tail(owner)
                            && args.iter().any(|a| matches!(a, Arg::Expand(_)))))
                        && self.normal_parameters[&owner]
                            .iter()
                            .any(|(_, _, ty)| *ty != p(P::Type))
                    {
                        // Type annotations consume these inputs at comptime;
                        // a bound runtime Splice protocol must not intercept a
                        // packet producer while preparing the head application.
                        let checked = self.with_staged_input(true, |this| {
                            this.normal_type_expression(expr, owner, args, Some(&p(P::Type)))
                        })?;
                        let mut budget = comptime_runtime::Budget {
                            steps: self.required_steps as usize,
                            max_limbs: 4096,
                        };
                        let before = budget.steps;
                        let known = self.known_normal_input(&checked, 0, &mut budget);
                        self.required_steps -= (before - budget.steps) as u64;
                        let known = match known {
                            Some(known) => Some(known),
                            None => self.required_normal_type_plan(expr, &checked)?,
                        };
                        if let Some(ty @ Term::NormalForm { .. }) = known {
                            let Term::NormalForm { arguments, .. } = &ty else {
                                unreachable!()
                            };
                            self.check_type_application(owner, arguments, expr.span)?;
                            return Ok(ty);
                        }
                        let (comptime_interpretor::Value::Type(ty), _) =
                            self.required_normal_input(expr, checked, Vec::new())?
                        else {
                            return Err(error(
                                expr.span,
                                "normal-form application did not produce Type",
                            ));
                        };
                        return Ok(ty);
                    }
                    if self.has_optional_type_clauses(owner)
                        || (args.iter().any(|arg| matches!(arg, Arg::Expand(_)))
                            && self.normal_parameters[&owner]
                                .iter()
                                .all(|(_, _, ty)| *ty == p(P::Type)))
                    {
                        let arguments = self.type_clause_arguments(owner, args, None, expr.span)?;
                        self.check_type_application(owner, &arguments, expr.span)?;
                        return Ok(Term::NormalForm {
                            head: declaration(self.package, &self.package_identities, owner),
                            sort: Sort::Type,
                            arguments,
                        });
                    }
                    let parameters = self.normal_parameters[&owner].clone();
                    let variadic = self.normal_type_list_tail(owner);
                    let (routed, order) = self.normal_type_list_route(owner, args, expr.span)?;
                    let args = routed.as_slice();
                    if if variadic {
                        args.len() < parameters.len() - 1
                    } else {
                        parameters.len() != args.len()
                    } {
                        return Err(error(expr.span, "normal-form argument count mismatch"));
                    }
                    let mut arguments = vec![None; parameters.len()];
                    let mut type_arguments = Vec::new();
                    for slot in order {
                        let (_, _, ty) = &parameters[slot];
                        if variadic && slot + 1 == parameters.len() {
                            let ty = self.instantiate_input_types(
                                ty,
                                &self.type_parameters[&owner][..type_arguments.len()],
                                &type_arguments,
                                expr.span,
                            )?;
                            if let Some(input) =
                                self.normal_type_list_supply(owner, &args[slot..], expr.span)?
                            {
                                arguments[slot] = Some(self.normal_value_argument(input, &ty)?);
                            } else {
                                arguments[slot] = Some(self.normal_collection_value_arguments(
                                    &args[slot..], &ty, expr.span,
                                )?);
                            }
                            continue;
                        }
                        let argument = &args[slot];
                        let Arg::Positional(value) = argument else {
                            return Err(error(
                                expr.span,
                                "normal-form arguments must be positional",
                            ));
                        };
                        let argument = if *ty == p(P::Type) {
                            let value = self.type_expr(value)?;
                            type_arguments.push(value.clone());
                            value
                        } else {
                            let ty = self.instantiate_input_types(
                                ty,
                                &self.type_parameters[&owner][..type_arguments.len()],
                                &type_arguments,
                                value.span,
                            )?;
                            self.normal_value_argument(value, &ty)?
                        };
                        arguments[slot] = Some(argument);
                    }
                    let arguments = arguments
                        .into_iter()
                        .collect::<Option<Vec<_>>>()
                        .ok_or_else(|| error(expr.span, "incomplete normal-form input routing"))?;
                    self.check_type_application(owner, &arguments, expr.span)?;
                    return Ok(Term::NormalForm {
                        head: declaration(self.package, &self.package_identities, owner),
                        sort: Sort::Type,
                        arguments,
                    });
                }
                if let Some(ItemKind::TypeAlias(value, ..)) =
                    self.package.item(owner).map(|item| &item.kind)
                {
                    let count = self.type_parameters[&owner].len();
                    if count == 0 {
                        return Err(error(
                            expr.span,
                            "generic type alias argument count mismatch",
                        ));
                    }
                    let arguments = self
                        .type_clause_arguments(owner, args, None, expr.span)
                        .map_err(|mut failure| {
                            if failure.message == "type argument count mismatch" {
                                failure.message =
                                    "generic type alias argument count mismatch".into();
                            }
                            failure
                        })?;
                    self.check_type_application(owner, &arguments, expr.span)?;
                    return self.alias_type(owner, value, &arguments, expr.span);
                }
                if !matches!(
                    self.package.item(owner).map(|item| &item.kind),
                    Some(ItemKind::Struct(_) | ItemKind::Enum(_) | ItemKind::Newtype(..))
                ) {
                    return Err(error(
                        expr.span,
                        "this type application requires comptime elaboration not yet implemented",
                    ));
                }
                if self.type_parameters[&owner].is_empty() {
                    return Err(error(
                        expr.span,
                        "generic type type argument count mismatch",
                    ));
                }
                let mut arguments = self.type_clause_arguments(owner, args, None, expr.span)?;
                self.check_type_application(owner, &arguments, expr.span)?;
                arguments.extend(self.nominal_capture_arguments(owner, expr.span)?);
                Ok(Term::Nominal {
                    declaration: declaration(self.package, &self.package_identities, owner),
                    arguments,
                })
            }

            H::TypeOf(value) => {
                if matches!(
                    &value.kind,
                    H::Lit(hir::Lit {
                        kind: hir::LitKind::Integer(_),
                        ..
                    })
                ) {
                    return Ok(p(P::Integer));
                }
                if let H::Unary(UnOp::Neg, inner) = &value.kind {
                    if matches!(
                        &inner.kind,
                        H::Lit(hir::Lit {
                            kind: hir::LitKind::Integer(_),
                            ..
                        })
                    ) {
                        return Ok(p(P::Integer));
                    }
                }
                if matches!(
                    &value.kind,
                    H::Lit(hir::Lit {
                        kind: hir::LitKind::Float(_),
                        ..
                    })
                ) {
                    return Ok(p(P::Float));
                }
                if let H::Unary(UnOp::Neg, inner) = &value.kind {
                    if matches!(
                        &inner.kind,
                        H::Lit(hir::Lit {
                            kind: hir::LitKind::Float(_),
                            ..
                        })
                    ) {
                        return Ok(p(P::Float));
                    }
                }
                // Observing a type does not evaluate/capture the operand in
                // surrounding runtime closures. Closures inside the operand
                // still build their own capture types during checking.
                let captures = std::mem::take(&mut self.captures);
                let locals = self.locals.clone();
                let result = self.result.take();
                let partial_return_context = self.partial_return_context.take();
                let optional_returns = std::mem::take(&mut self.pending_optional_returns);
                let error_returns = std::mem::take(&mut self.pending_error_returns);
                let return_sites = std::mem::take(&mut self.inferred_return_sites);
                let returns = self.return_observations.take();
                let observing = std::mem::replace(&mut self.observing_type, true);
                let checked = self.expr(value, None).map(|value| value.ty);
                self.observing_type = observing;
                self.captures = captures;
                self.locals = locals;
                self.result = result;
                self.partial_return_context = partial_return_context;
                self.pending_optional_returns = optional_returns;
                self.pending_error_returns = error_returns;
                self.inferred_return_sites = return_sites;
                self.return_observations = returns;
                checked
            }
            H::TyClosure(context, body) => {
                let H::TyClosureEnvironment(bindings) = &context.kind else {
                    if let H::ResolvedClauseParam(_, id) = &context.kind {
                        let declaration = DeclarationId {
                            package: semantic_package(&self.package_identities, id.pkg),
                            local: id.index,
                        };
                        if self.trait_input_domain(declaration) == Some(Term::ClosureContextType) {
                            let context = self
                                .alias_substitution
                                .get(&declaration)
                                .map(|value| (**value).clone())
                                .unwrap_or(Term::Rigid {
                                    declaration,
                                    sort: Sort::ComptimeValue,
                                });
                            let signature = self.type_expr(body)?;
                            return Term::contextual_closure(context, signature)
                                .map_err(|e| error(expr.span, e.to_string()));
                        }
                    }
                    return Err(error(
                        context.span,
                        "open closure contexts require environment-parameter elaboration not yet implemented",
                    ));
                };
                let mut names = std::collections::HashSet::new();
                let mut environment = Vec::new();
                for binding in *bindings {
                    if !names.insert(binding.name.name) {
                        return Err(error(
                            binding.name.span,
                            "duplicate closure environment binding",
                        ));
                    }
                    if binding.value.is_some() {
                        return Err(error(
                            binding.name.span,
                            "closure environment sources require dependency/provenance checking not yet implemented",
                        ));
                    }
                    environment.push(self.type_expr(binding.ty)?);
                }
                let body = self.type_expr(body)?;
                if environment.is_empty() {
                    return Ok(body);
                }
                if matches!(body, Term::Closure { .. }) {
                    return Err(error(
                        expr.span,
                        "a type cannot have repeated nonempty closure qualification",
                    ));
                }
                if !matches!(body, Term::Function { .. } | Term::ComptimeFunction(_) | Term::UnsafeFunction(_)) {
                    return Err(error(
                        expr.span,
                        "non-callable closure-qualified bodies are not yet implemented",
                    ));
                }
                Ok(Term::Closure {
                    environment,
                    signature: Box::new(body),
                })
            }
            H::TyModifiedFn(modifiers, params) => self.form_modified_callable_type(*modifiers, params, None, expr.span),
            H::TyNFFn(params) => self.form_normal_form_callable_type(params, None, expr.span),
            H::TyFn(params) | H::TyComptimeFn(params) => {
                self.form_callable_type(params, None, matches!(expr.kind, H::TyComptimeFn(_)), expr.span)
            }
            H::TyFnArrow(input, output) => {
                if let H::TyNFFn(params) = &input.kind {
                    return self.form_normal_form_callable_type(params, Some(output), expr.span);
                }
                if let H::TyModifiedFn(modifiers, params) = &input.kind {
                    return self.form_modified_callable_type(*modifiers, params, Some(output), expr.span);
                }
                if let H::TyFn(params) | H::TyComptimeFn(params) = &input.kind {
                    return self.form_callable_type(params, Some(output), matches!(input.kind, H::TyComptimeFn(_)), expr.span);
                }

                let input = self.type_expr(input)?;
                if !matches!(input.callable_signature(), Term::Function { .. }) {
                    return Err(error(expr.span, "arrow input must be a function type"));
                }
                let output = self
                    .type_expr(output)?
                    .abstract_parameters(&[])
                    .map_err(|e| error(output.span, e.to_string()))?;
                input
                    .with_callable_result(output)
                    .ok_or_else(|| error(expr.span, "arrow input must be a function type"))
            }
            H::ResolvedClauseParam(_, id) => {
                let declaration = DeclarationId {
                    package: semantic_package(&self.package_identities, id.pkg),
                    local: id.index,
                };
                if let Some(domain) = self.trait_input_domain(declaration) {
                    let domain = self.substituted_trait_domain(&domain, expr.span)?;
                    if domain != p(P::Type) {
                        return Err(error(
                            expr.span,
                            "trait value parameter cannot be used as a Type",
                        ));
                    }
                }
                if self.forbidden_default_inputs.contains(&declaration) {
                    return Err(error(
                        expr.span,
                        "normal-form default references itself or a later parameter",
                    ));
                }
                if self.alias_value_bindings.contains_key(&declaration) {
                    return Err(error(
                        expr.span,
                        "required comptime input is not yet available in this definition",
                    ));
                }
                if let Some(value) = self.alias_substitution.get(&declaration).cloned() {
                    return self.copy_alias_type(&value, expr.span);
                }
                if self
                    .type_parameters
                    .values()
                    .any(|parameters| parameters.contains(&declaration))
                {
                    Ok(Term::Rigid {
                        declaration,
                        sort: Sort::Type,
                    })
                } else {
                    Err(error(
                        expr.span,
                        "clause parameter is not an available Type parameter",
                    ))
                }
            }
            H::TyVoid => Ok(p(P::Void)),
            H::TyType => Ok(p(P::Type)),
            H::TyNoReturn => Ok(p(P::Never)),
            H::TyOptional(inner) => Ok(Term::Optional(Box::new(self.type_expr(inner)?))),
            H::TyPtr(inner) => Ok(Term::Pointer(Box::new(self.type_expr(inner)?))),
            H::Tuple(_) => Err(error(
                expr.span,
                "a parenthesized tuple is a value, not a type; use Tuple<...> to construct a tuple type",
            )),
            H::Unit => Err(error(
                expr.span,
                "() is a value, not a type; use void or Tuple<> as appropriate",
            )),
            H::Builtin(name) => {
                if name.as_str() == "BitVec" {
                    return Err(error(
                        expr.span,
                        "builtin BitVec is a type constructor and requires a width argument",
                    ));
                }
                let primitive = match name.as_str() {
                    "bool" => P::Bool,
                    "char" => P::Char,
                    "str" => P::Str,
                    "Symbol" => P::Symbol,
                    "Type" => P::Type,
                    "Object" => P::Object,
                    "Integer" => P::Integer,
                    "Float" => P::Float,
                    "void" => P::Void,
                    "NoReturn" => P::Never,
                    "i8" => P::I8,
                    "i16" => P::I16,
                    "i32" => P::I32,
                    "i64" => P::I64,
                    "i128" => P::I128,
                    "u8" => P::U8,
                    "u16" => P::U16,
                    "u32" => P::U32,
                    "u64" => P::U64,
                    "u128" => P::U128,
                    "isize" => P::Isize,
                    "usize" => P::Usize,
                    "f16" => P::F16,
                    "bf16" => P::Bf16,
                    "f32" => P::F32,
                    "f64" => P::F64,
                    _ => return Err(error(expr.span, format!("unsupported builtin type {name}"))),
                };
                Ok(p(primitive))
            }
            H::ResolvedDef { def_id, .. } => {
                let owner = self.package.definition_owner(*def_id).ok_or_else(|| {
                    error(expr.span, "resolved type declaration has no HIR owner")
                })?;
                if matches!(
                    self.package.item(owner).map(|i| &i.kind),
                    Some(ItemKind::Associated(_))
                ) {
                    let value = self.lexical_associated_value(owner, expr.span)?;
                    return match value.kind {
                        K::TypeValue(ty) => Ok(ty),
                        _ => Err(error(
                            expr.span,
                            "associated value used as a type does not contain a completed Type",
                        )),
                    };
                }
                if matches!(
                    self.package.item(owner).map(|i| &i.kind),
                    Some(ItemKind::Const(..))
                ) {
                    let value = self.constant_value(owner, expr.span)?;
                    return match value.kind {
                        K::TypeValue(ty) => Ok(ty),
                        _ => match self.required_normal_input(expr, value, Vec::new())?.0 {
                            comptime_interpretor::Value::Type(ty) => Ok(ty),
                            _ => Err(error(
                                expr.span,
                                "constant used as a type does not contain a Type value",
                            )),
                        },
                    };
                }
                if let Some(ItemKind::TypeAlias(value, ..)) =
                    self.package.item(owner).map(|i| &i.kind)
                {
                    let arguments = self
                        .type_clause_arguments(owner, &[], None, expr.span)
                        .map_err(|mut failure| {
                            if failure.message == "type argument count mismatch" {
                                failure.message =
                                    "generic type alias requires explicit type arguments".into();
                            }
                            failure
                        })?;
                    self.check_type_application(owner, &arguments, expr.span)?;
                    return self.alias_type(owner, value, &arguments, expr.span);
                }
                if !self.aliases.insert(owner) {
                    return Err(error(expr.span, "cyclic type alias"));
                }
                let result = match self.package.item(owner).map(|i| &i.kind) {
                    Some(ItemKind::Struct(_) | ItemKind::Enum(_) | ItemKind::Newtype(..)) => {
                        (|| {
                            let arguments = self
                                .type_clause_arguments(owner, &[], None, expr.span)
                                .map_err(|mut failure| {
                                    if failure.message.contains("argument count mismatch") {
                                        failure.message =
                                            "generic type requires explicit type arguments".into();
                                    }
                                    failure
                                })?;
                            self.check_type_application(owner, &arguments, expr.span)?;
                            let mut arguments = arguments;
                            arguments.extend(self.nominal_capture_arguments(owner, expr.span)?);
                            Ok(Term::Nominal {
                                declaration: declaration(
                                    self.package,
                                    &self.package_identities,
                                    owner,
                                ),
                                arguments,
                            })
                        })()
                    }
                    _ => Err(error(
                        expr.span,
                        "declaration does not produce a supported type",
                    )),
                };
                self.aliases.remove(&owner);
                result
            }
            _ => Err(error(
                expr.span,
                "type expression requires comptime elaboration not yet implemented",
            )),
        }
    }
    fn type_predicate(&mut self, expr: &Expr<'_>) -> Result<n::TypePredicate, Error> {
        enum Work<'a, 'hir> {
            Expr(&'a Expr<'hir>, usize),
            Not,
            Binary(bool),
        }
        let budget = crate::ptc::Budget::default();
        let mut fuel = budget.steps;
        let mut pending = vec![Work::Expr(expr, 0)];
        let mut values = Vec::new();
        while let Some(work) = pending.pop() {
            match work {
                Work::Expr(expr, depth) => {
                    if depth >= budget.max_depth {
                        return Err(error(
                            expr.span,
                            "type proposition elaboration depth exhausted",
                        ));
                    }
                    fuel = fuel.checked_sub(1).ok_or_else(|| {
                        error(expr.span, "type proposition elaboration budget exhausted")
                    })?;
                    match &expr.kind {
                        H::Unary(UnOp::Not, value) => {
                            pending.push(Work::Not);
                            pending.push(Work::Expr(value, depth + 1));
                        }
                        H::Binary(op @ (BinOp::And | BinOp::Or), left, right) => {
                            pending.push(Work::Binary(*op == BinOp::And));
                            pending.push(Work::Expr(right, depth + 1));
                            pending.push(Work::Expr(left, depth + 1));
                        }
                        _ => values.push(self.type_predicate_atom(expr)?),
                    }
                }
                Work::Not => {
                    let value = values.pop().expect("checked proposition operand");
                    values.push(n::TypePredicate::Not(Box::new(value)));
                }
                Work::Binary(and) => {
                    let right = values.pop().expect("checked right proposition");
                    let left = values.pop().expect("checked left proposition");
                    let parts = vec![left, right];
                    values.push(if and {
                        n::TypePredicate::And(parts)
                    } else {
                        n::TypePredicate::Or(parts)
                    });
                }
            }
        }
        Ok(values.pop().expect("checked proposition root"))
    }

    fn type_predicate_atom(&mut self, expr: &Expr<'_>) -> Result<n::TypePredicate, Error> {
        match &expr.kind {
            H::TraitBound(subject, bound) => {
                let subject = self.type_expr(subject)?;
                let atoms = self
                    .parent_interfaces_for(bound, &subject)?
                    .into_iter()
                    .map(|interface| {
                        n::TypePredicate::Trait(n::TraitRequirement {
                            interface: interface.declaration,
                            interface_arguments: interface.arguments,
                            subject: subject.clone(),
                        })
                    })
                    .collect();
                Ok(n::TypePredicate::And(atoms))
            }
            H::Lit(hir::Lit {
                kind: hir::LitKind::Bool(value),
                ..
            }) => Ok(n::TypePredicate::Bool(*value)),
            H::Binary(op @ (BinOp::Eq | BinOp::Ne), a, b) => {
                let operand_error = |mut failure: Error| {
                    // Retain the failing inner span and resumability flag. A
                    // malformed type application is not a program predicate,
                    // and must not lose its concrete formation diagnostic.
                    failure.message = format!(
                        "requires type operand could not be elaborated: {}; program specifications are not handled by PTC",
                        failure.message
                    );
                    failure
                };
                let left = self.type_expr(a).map_err(operand_error)?;
                let right = self.type_expr(b).map_err(operand_error)?;
                let equality = n::TypePredicate::Equal(left, right);
                Ok(if *op == BinOp::Ne {
                    n::TypePredicate::Not(Box::new(equality))
                } else {
                    equality
                })
            }
            _ => Err(error(
                expr.span,
                "requires is not a supported type proposition; program specifications are not handled by PTC",
            )),
        }
    }
    fn nominal_context(&self) -> crate::ptc::Context {
        let mut context = crate::ptc::Context::default();
        for declaration in &self.code_packet_nominals {
            context.admit_nominal_constructor(*declaration);
        }
        // Checked builder results have no source HIR owner. Their nominal
        // identity must survive fresh binder and required-invocation contexts.
        for declaration in self.structs.keys() {
            context.admit_nominal_constructor(*declaration);
        }
        // Nominal declaration identity does not depend on obtaining its field
        // layout. World preparation can compare impl heads before unrelated
        // layouts/signatures have been demanded; formation is checked separately.
        for (owner, info) in self.package.owners() {
            if matches!(
                info.node.expect_item().kind,
                ItemKind::Struct(_) | ItemKind::Enum(_) | ItemKind::Newtype(..)
            ) {
                context.admit_nominal_constructor(declaration(
                    self.package,
                    &self.package_identities,
                    owner,
                ));
            }
        }
        // Only checked opaque definitions with the supported immutable input
        // telescope receive forward congruence. No body equation or inverse
        // constructor rule is introduced by this admission.
        for owner in self.normal_parameters.keys() {
            if self.is_registered_normal_form(*owner) {
                context.admit_congruence(declaration(
                    self.package,
                    &self.package_identities,
                    *owner,
                ));
                let head = declaration(self.package, &self.package_identities, *owner);
                if Some(head) == self.protocols.meta_list
                    && self.normal_parameters[owner].len() == 1
                    && self.normal_parameters[owner][0].2 == p(P::Type)
                {
                    context.admit_list_inference_head(head);
                }
                if Some(head) == self.protocols.tuple
                    && self.normal_type_list_tail(*owner)
                    && self.normal_parameters[owner].len() == 1
                    && self.is_type_list_type(&self.normal_parameters[owner][0].2)
                {
                    context.admit_tuple_head(head);
                }
            }
        }
        context
    }
    fn definition_context(&self, owner: OwnerId) -> crate::ptc::Context {
        let base = self.binder_context(owner);
        let scope = self
            .signatures
            .get(&owner)
            .or_else(|| self.interface_signatures.get(&owner))
            .and_then(|signature| match signature.callable_signature() {
                Term::Function { parameters, .. } if !parameters.is_empty() => {
                    Some(vec![parameters.iter().map(|p| p.sort).collect()])
                }
                _ => None,
            })
            .or_else(|| {
                self.open_functions
                    .get(&owner)
                    .map(|definition| vec![definition.input_sorts()])
            })
            .unwrap_or_default();
        let mut context = base.theory_in_scope(scope);
        for assumption in base.assumptions() {
            context.assume(assumption.clone());
        }
        for capability in self.capabilities.get(&owner).into_iter().flatten() {
            if std::iter::once(&capability.subject)
                .chain(&capability.interface_arguments)
                .any(|ty| {
                    ty.substitution_parameters()
                        .is_ok_and(|slots| !slots.is_empty())
                })
            {
                context.assume(crate::ptc::Formula::from_predicate(
                    &n::TypePredicate::Trait(capability.clone()),
                ));
            }
        }
        if let Some(equations) = self.associated_definition_equations.get(&declaration(
            self.package,
            &self.package_identities,
            owner,
        )) {
            for (projection, value) in equations {
                context.assume(crate::ptc::Formula::equal(
                    projection.clone(),
                    value.clone(),
                ));
            }
        }
        if let Some(requirements) = self.requirements.get(&owner) {
            for requirement in requirements {
                context.assume(crate::ptc::Formula::from_predicate(requirement));
            }
        }
        context
    }
    fn binder_assumptions(&self, owner: OwnerId) -> Vec<n::TraitRequirement> {
        let mut assumptions = self.capabilities.get(&owner).cloned().unwrap_or_default();
        if let Some(parent) = self.self_containers.get(&owner) {
            for requirement in self.capabilities.get(parent).into_iter().flatten() {
                if !assumptions.contains(requirement) {
                    assumptions.push(requirement.clone());
                }
            }
        }
        if let Some(interface) = self.checked_traits.iter().find(|interface| {
            self.self_containers.get(&owner).is_some_and(|parent| {
                matches_declaration(
                    self.package,
                    &self.package_identities,
                    *parent,
                    interface.declaration,
                )
            }) || interface.methods.iter().any(|method| {
                method.declaration == declaration(self.package, &self.package_identities, owner)
            }) || interface.associated.iter().any(|member| {
                member.declaration == declaration(self.package, &self.package_identities, owner)
            })
        }) {
            assumptions.push(n::TraitRequirement {
                subject: Term::Rigid {
                    declaration: interface.self_parameter,
                    sort: Sort::Type,
                },
                interface: interface.declaration,
                interface_arguments: interface
                    .type_parameters
                    .iter()
                    .map(|id| Term::Rigid {
                        declaration: *id,
                        sort: if self
                            .trait_input_domain(*id)
                            .is_some_and(|ty| ty != p(P::Type))
                        {
                            Sort::ComptimeValue
                        } else {
                            Sort::Type
                        },
                    })
                    .collect(),
            });
        }
        assumptions
    }
    // Carry the same type premises used to check the body into owned NIR.
    // Program verification must recover scalar domains without guessing from
    // the body's chosen operations or relying on the transient checker state.
    fn definition_requirements(&self, owner: OwnerId) -> Vec<n::TypePredicate> {
        let mut requirements = self.requirements.get(&owner).cloned().unwrap_or_default();
        for requirement in crate::capabilities::subject_requirements_with_layouts(
            &self.checked_traits,
            &self.binder_assumptions(owner),
            crate::ptc::Budget::default(),
            self.implementation_index.parameter_domains(),
            Some(self.implementation_index.nominal_layouts()),
        ) {
            if !requirements.contains(&requirement) {
                requirements.push(requirement);
            }
        }
        requirements
    }
    fn binder_context(&self, owner: OwnerId) -> crate::ptc::Context {
        let mut context = self.nominal_context();
        for requirement in crate::capabilities::subject_requirements_with_layouts(
            &self.checked_traits,
            &self.binder_assumptions(owner),
            crate::ptc::Budget::default(),
            self.implementation_index.parameter_domains(),
            Some(self.implementation_index.nominal_layouts()),
        ) {
            context.assume(crate::ptc::Formula::from_predicate(&requirement));
        }
        context
    }
    fn check_type_application(
        &mut self,
        owner: OwnerId,
        arguments: &[Term],
        span: Span,
    ) -> Result<(), Error> {
        self.check_formation_application(owner, FormationInputs::ready(arguments), span)
    }
    fn check_formation_application(
        &mut self,
        owner: OwnerId,
        mut arguments: FormationInputs,
        span: Span,
    ) -> Result<(), Error> {
        arguments.environment.extend(
            self.alias_substitution
                .iter()
                .filter(|(id, _)| !self.type_parameters[&owner].contains(id))
                .map(|(id, value)| (*id, value.clone())),
        );
        if let Some(pending) = &mut self.pending_type_uses {
            let scope = self.signature_owner.and_then(|owner| {
                self.alias_scopes
                    .get(&owner)
                    .copied()
                    .unwrap_or(Some(owner))
            });
            pending.push((scope, !self.checking_contract, owner, arguments, span));
            Ok(())
        } else {
            self.check_formation_requirements(owner, &arguments, span)
        }
    }
    fn capability_argument_view(&self, term: &Term, span: Span) -> Result<Term, Error> {
        if term.sort() == Sort::Type {
            self.callable_input_view(term, span)
        } else {
            // Interface indices such as LiteralInto's Symbol are values, not
            // Type expressions. Preserve their identity and lexical binders.
            normal::clone_known_term(term, &mut comptime_runtime::Budget::default())
                .ok_or_else(|| error(span, "capability argument view budget exhausted"))
        }
    }

    fn requirement_type_view(
        &self,
        input: &n::TypePredicate,
        span: Span,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<n::TypePredicate, Error> {
        use n::TypePredicate as Q;
        if depth >= 64 {
            return Err(error(span, "associated requirement view depth exhausted"));
        }
        *fuel = fuel
            .checked_sub(1)
            .ok_or_else(|| error(span, "associated requirement view budget exhausted"))?;
        Ok(match input {
            Q::Bool(value) => Q::Bool(*value),
            Q::Trait(requirement) => Q::Trait(n::TraitRequirement {
                interface: requirement.interface,
                subject: self.callable_input_view(&requirement.subject, span)?,
                interface_arguments: requirement
                    .interface_arguments
                    .iter()
                    .map(|term| self.capability_argument_view(term, span))
                    .collect::<Result<_, _>>()?,
            }),
            Q::Equal(a, b) => Q::Equal(
                self.callable_input_view(a, span)?,
                self.callable_input_view(b, span)?,
            ),
            Q::Not(inner) => Q::Not(Box::new(self.requirement_type_view(
                inner,
                span,
                fuel,
                depth + 1,
            )?)),
            Q::And(items) | Q::Or(items) => {
                let values = items
                    .iter()
                    .map(|item| self.requirement_type_view(item, span, fuel, depth + 1))
                    .collect::<Result<_, _>>()?;
                if matches!(input, Q::And(_)) {
                    Q::And(values)
                } else {
                    Q::Or(values)
                }
            }
        })
    }
    fn check_requirements(
        &self,
        owner: OwnerId,
        arguments: &[Term],
        span: Span,
    ) -> Result<(), Error> {
        self.check_formation_requirements(owner, &FormationInputs::ready(arguments), span)
    }
    fn check_formation_requirements(
        &self,
        owner: OwnerId,
        inputs: &FormationInputs,
        span: Span,
    ) -> Result<(), Error> {
        let arguments = &inputs.terms;
        let requirements = self
            .requirements
            .get(&owner)
            .ok_or_else(|| error(span, "function requirements await definition elaboration"))?;
        // A normal head has a mixed Type/value telescope. Its capability
        // indices must use the same complete substitution as other predicates.
        let normal_capabilities: Vec<_> = if self.normal_parameters.contains_key(&owner) {
            self.capabilities
                .get(&owner)
                .into_iter()
                .flatten()
                .cloned()
                .map(n::TypePredicate::Trait)
                .collect()
        } else {
            Vec::new()
        };
        let mut work: Vec<_> = requirements
            .iter()
            .chain(&normal_capabilities)
            .rev()
            .collect();
        while let Some(requirement) = work.pop() {
            if let n::TypePredicate::And(parts) = requirement {
                work.extend(parts.iter().rev());
                continue;
            }
            let terms = required::requirement_terms(requirement, span)?;
            if terms.iter().any(|ty| {
                ty.substitution_parameters()
                    .is_ok_and(|slots| !slots.is_empty())
            }) {
                let sorts = if let Some(definition) = self.open_functions.get(&owner) {
                    definition.input_sorts()
                } else {
                    let signature = self
                        .signatures
                        .get(&owner)
                        .or_else(|| self.interface_signatures.get(&owner))
                        .ok_or_else(|| {
                            error(span, "ordinary type requirement needs a function signature")
                        })?;
                    let Term::Function { parameters, .. } = signature.callable_signature() else {
                        return Err(error(
                            span,
                            "ordinary type requirement needs a function signature",
                        ));
                    };
                    parameters.iter().map(|p| p.sort).collect()
                };
                let frames = vec![sorts];
                for ty in terms {
                    ty.check_scope(&frames)
                        .map_err(|e| error(span, e.to_string()))?;
                }
                // Retained in the declaration; discharge after ordinary input preparation.
                continue;
            }
            if inputs
                .deferred
                .iter()
                .any(|input| requirement.mentions_rigid(*input))
            {
                // The stable head contract is retained in NIR and checked after
                // the constructor evaluates these inputs, before a value escapes.
                continue;
            }
            // Interface formation has no implementation subject yet. Its
            // Self-dependent obligations are checked on each implementation.
            if matches!(
                self.package.item(owner).map(|item| &item.kind),
                Some(ItemKind::Trait(_))
            ) && requirement.mentions_rigid(declaration(
                self.package,
                &self.package_identities,
                owner,
            )) {
                continue;
            }
            let requirement =
                self.instantiate_formation_environment(requirement, &inputs.environment, span)?;
            let declarations: Vec<_> = if let Some(parameters) = self.normal_parameters.get(&owner)
            {
                parameters.iter().map(|(id, _, _)| *id).collect()
            } else {
                self.declaration_binders(owner)
                    .iter()
                    .map(|p| p.declaration)
                    .collect()
            };
            let domains: Vec<_> = declarations
                .iter()
                .map(|id| self.trait_input_domain(*id).unwrap_or_else(|| p(P::Type)))
                .collect();
            let substitution = nir::DeclarationSubstitution::new(
                &declarations,
                &domains,
                arguments,
                self.context.scope(),
            )
            .map_err(|e| error(span, e.to_string()))?;
            let instantiated = substitution
                .instantiate_predicate(&requirement)
                .map_err(|e| error(span, e.to_string()))?;
            self.establish_type_requirement(&instantiated, span)?;
        }
        if let Some(parameters) = self.normal_parameters.get(&owner) {
            if parameters.len() != arguments.len() {
                return Err(error(span, "normal-form contract argument count mismatch"));
            }
            Ok(())
        } else {
            self.check_capabilities_in_environment(
                owner,
                arguments,
                &inputs.environment,
                &inputs.deferred,
                span,
            )
        }
    }
    // Shared discharge path for completed type prerequisites. Program
    // predicates never enter this PTC/implementation-world boundary.
    fn establish_type_requirement(
        &self,
        instantiated: &n::TypePredicate,
        span: Span,
    ) -> Result<(), Error> {
        let mut outcome = self.context.prove(
            crate::ptc::Formula::from_predicate(instantiated),
            crate::ptc::Budget::default(),
        );
        if self.interfaces_ready
            && matches!(
                outcome,
                crate::ptc::Outcome::Unknown { .. } | crate::ptc::Outcome::NotEntailed(_)
            )
        {
            let viewed = self.requirement_type_view(instantiated, span, &mut 10_000, 0)?;
            let assumptions = self.capability_assumptions();
            outcome = crate::capabilities::World {
                index: &self.implementation_index,
                implementations: &self.checked_implementations,
                traits: &self.checked_traits,
                assumptions: &assumptions,
                context: &self.context,
            }
            .check_predicate(&viewed, crate::ptc::Budget::default())
            .map_err(|message| error(span, message))?;
        }
        match outcome {
            crate::ptc::Outcome::Entailed(_) => {}
            crate::ptc::Outcome::NotEntailed(_) => {
                return Err(error(
                    span,
                    format!(
                        "type requirement is not satisfied{}: {instantiated}",
                        if matches!(instantiated, n::TypePredicate::Trait(_)) {
                            " (trait bound)"
                        } else {
                            ""
                        }
                    ),
                ));
            }
            crate::ptc::Outcome::Unknown { .. } => {
                return Err(error(
                    span,
                    format!(
                        "type requirement remains unresolved{}: {instantiated}",
                        if matches!(instantiated, n::TypePredicate::Trait(_)) {
                            " (trait bound)"
                        } else {
                            ""
                        }
                    ),
                ));
            }
            crate::ptc::Outcome::IllFormed(reason) => {
                return Err(error(span, format!("invalid type requirement: {reason}")));
            }
        }
        Ok(())
    }
    fn require(&self, actual: &Term, expected: &Term, span: Span) -> Result<(), Error> {
        // Diverging control flow is admissible at a result position; this does
        // not assert the false type equation Never = expected.
        if is_never(actual) {
            return Ok(());
        }
        use crate::ptc::{Budget, Formula, Outcome};
        // PTC receives well-scoped type propositions. Preserve the enclosing
        // telescope instead of submitting its body as a free bound variable.
        let scoped = |ty: &Term| -> Result<Term, Error> {
            let parameters = self.call_inference_scope();
            if parameters.is_empty() {
                return Ok(ty.clone());
            }
            Ok(Term::Function {
                parameters,
                result: Box::new(ty.clone()),
            })
        };
        let has_inputs = !actual
            .substitution_parameters()
            .map_err(|e| error(span, e.to_string()))?
            .is_empty()
            || !expected
                .substitution_parameters()
                .map_err(|e| error(span, e.to_string()))?
                .is_empty();
        let frames = self.call_input_frames();
        let (left, right) = if has_inputs && self.context.scope() != frames.as_slice() {
            (scoped(actual)?, scoped(expected)?)
        } else {
            (actual.clone(), expected.clone())
        };
        let mut outcome = self.context.prove(
            Formula::equal(left.clone(), right.clone()),
            Budget::default(),
        );
        if self.interfaces_ready
            && matches!(outcome, Outcome::Unknown { .. } | Outcome::NotEntailed(_))
        {
            // PTC knows type equations, but does not select implementation
            // initializers. Resolve concrete associated projections in the
            // admitted world before retrying; given views remain abstract and
            // normal-form heads retain their identity.
            let left = self.callable_input_view(&left, span)?;
            let right = self.callable_input_view(&right, span)?;
            outcome = self
                .context
                .prove(Formula::equal(left, right), Budget::default());
        }
        match outcome {
            Outcome::Entailed(_) => Ok(()),
            Outcome::NotEntailed(_) => Err(error(
                span,
                format!("type mismatch: expected {expected}, found {actual}"),
            )),
            Outcome::Unknown { reason, .. } => Err(error(
                span,
                format!(
                    "unresolved type equality ({reason:?}): expected {expected}, found {actual}"
                ),
            )),
            Outcome::IllFormed(reason) => {
                Err(error(span, format!("ill-formed type equality: {reason}")))
            }
        }
    }
    fn block(&mut self, block: &hir::Block<'_>, expected: Option<&Term>) -> Result<n::Expr, Error> {
        self.block_context(block, expected, None)
    }
    fn block_context(
        &mut self,
        block: &hir::Block<'_>,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        self.block_context_with_input(block, expected, partial, None)
    }
    fn block_context_with_input(
        &mut self,
        block: &hir::Block<'_>,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
        input_context: Option<&crate::calls::InputContext>,
    ) -> Result<n::Expr, Error> {
        let saved = self.locals.clone();
        let saved_code = self.code_values.clone();
        let saved_patterns = self.pattern_codes.clone();
        let saved_context = self.context.clone();
        let saved_staged: BTreeSet<_> = self
            .specialization_inputs
            .iter()
            .map(|(binding, _, _)| *binding)
            .collect();
        let saved_types: BTreeSet<_> = self.type_input_bindings.keys().copied().collect();
        let saved_partial_values: BTreeSet<_> =
            self.partial_staged_bindings.keys().copied().collect();
        let saved_values: BTreeSet<_> = self.value_input_bindings.keys().copied().collect();
        let result = (|| {
            let mut statements = Vec::new();
            let mut staged_patterns = BTreeMap::new();
            let mut diverges = false;
            for statement in block.stmts {
                let value = self.expr(statement, None)?;
                if let K::LetPattern {
                    pattern,
                    value: input,
                    ..
                } = &value.kind
                {
                    if let Some(inputs) =
                        self.completed_pattern_inputs(pattern, input, statement.span)?
                    {
                        staged_patterns.insert(statements.len(), inputs);
                    }
                }
                diverges |= is_never(&value.ty);
                if !diverges {
                    self.assume_statement_type_facts(&value, statement.span)?;
                    if self
                        .signature_owner
                        .is_some_and(|owner| self.open_functions.contains_key(&owner))
                    {
                        self.assume_open_continuation(statement)?;
                    }
                }
                statements.push(value);
            }
            let mut tail = block
                .expr
                .map(|value| {
                    // This block cannot deliver its tail. Check an independently
                    // typed tail without imposing the block's result domain;
                    // keep contextual inference for literals/open callables.
                    // A spread producer still has a declared field layout to
                    // check, even when its runtime evaluation cannot finish.
                    if diverges
                        && self.runtime_value_context()
                        && self.preparing_spread.is_none()
                        && input_context.is_none()
                    {
                        if let Some(checked) = self.argument_attempt(value, None, None)? {
                            return Ok(checked);
                        }
                    }
                    match input_context {
                        Some(input_context) if expected.is_none() => {
                            self.input_context_expression(value, input_context)
                        }
                        _ => self.expr_context(value, expected, partial),
                    }
                })
                .transpose()?
                .map(Box::new);
            if tail.is_none() && !diverges {
                tail = self
                    .implicit_error_success(expected, block.span)?
                    .map(Box::new);
            }
            let ty = if diverges {
                p(P::Never)
            } else {
                tail.as_ref().map(|x| x.ty.clone()).unwrap_or(p(P::Void))
            };
            if let Some(expected) = expected {
                self.require(&ty, expected, block.span)?;
            }
            self.materialize_mixed_staged_patterns(
                &mut statements,
                tail.as_deref(),
                block,
                &staged_patterns,
            )?;
            self.erase_consumed_staged_aliases(&mut statements, tail.as_deref(), block.span)?;
            self.stabilize_error_let_bindings(&mut statements, block.span)?;
            Ok(n::Expr {
                ty,
                source: source(block.span),
                kind: K::Block {
                    statements,
                    result: tail,
                },
            })
        })();
        self.specialization_inputs
            .retain(|(binding, _, _)| saved_staged.contains(binding));
        self.type_input_bindings
            .retain(|binding, _| saved_types.contains(binding));
        self.partial_staged_bindings
            .retain(|binding, _| saved_partial_values.contains(binding));
        self.value_input_bindings
            .retain(|binding, _| saved_values.contains(binding));
        self.locals = saved;
        self.code_values = saved_code;
        self.pattern_codes = saved_patterns;
        self.context = saved_context;
        result
    }
    /// Lexical clause inputs are fixed by the enclosing instance; source
    /// arguments only instantiate the local function's own clause telescope.
    fn lexical_function_arguments(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<Vec<Term>, Error> {
        if !matches!(
            self.package.item(owner).map(|item| &item.kind),
            Some(ItemKind::Fn(..) | ItemKind::FnDeclaration(..))
        ) {
            return Ok(Vec::new());
        }
        let Some(parent) = self.package.lexical_parents.get(&owner).copied() else {
            return Ok(Vec::new());
        };
        self.ensure_signature(parent, span)?;
        let parameters = self.type_parameters[&parent].clone();
        let mut arguments = parameters
            .into_iter()
            .map(|id| {
                if let Some(value) = self.alias_substitution.get(&id).cloned() {
                    self.copy_alias_type(&value, span)
                } else {
                    Ok(Term::Rigid {
                        declaration: id,
                        sort: self.input_binders(&[id])[0].sort,
                    })
                }
            })
            .collect::<Result<Vec<_>, Error>>()?;
        for (binding, _, sort, _) in self.lexical_staged_inputs(parent, span)? {
            // Inside a descendant, this input belongs to the descendant's
            // fixed lexical telescope, not to the parent's parameter frame.
            // Reusing the outer Bound index would alias the recursive callee's
            // own parameters (which may have completely different sorts).
            if let Some((value, _)) = self.captured_staged_input(binding, span)? {
                arguments.push(value);
                continue;
            }
            let value = if sort == Sort::Type {
                self.type_input_bindings.get(&local(binding)).cloned()
            } else {
                self.value_input_bindings
                    .get(&local(binding))
                    .map(|(value, _)| value.clone())
            }
            .ok_or_else(|| error(span, "lexical staged input is unavailable at this use"))?;
            arguments.push(value);
        }
        Ok(arguments)
    }

    /// Receiver transport follows declaration syntax, including explicit UFCS.
    /// An ordinary first parameter remains a value even when named similarly.
    fn comptime_receiver_arguments(
        &self,
        owner: Option<OwnerId>,
        arguments: &mut [n::CallArgument],
    ) {
        let receiver = owner
            .and_then(|owner| self.package.item(owner))
            .is_some_and(|item| {
                matches!(&item.kind, ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)
                if sig.modifiers.is_comptime && !sig.modifiers.is_normal_form_head
                    && sig.params.first().is_some_and(|param|
                        matches!(param.ty.kind, TyParamKind::Itself { is_ref: false })))
            });
        if !receiver {
            return;
        }
        for argument in arguments {
            if argument.parameter == 0 && !argument.is_default {
                argument.passing = if matches!(argument.value.kind, K::Local(id) if self.immutable.contains(&id))
                {
                    n::ArgumentPassing::ComptimeReferenceNoRebind
                } else {
                    n::ArgumentPassing::ComptimeReference
                };
            }
        }
    }

    fn callable_type(&self, owner: OwnerId, signature: Term) -> Term {
        let comptime = self.package.item(owner).is_some_and(|item| {
            matches!(&item.kind, ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)
                if sig.modifiers.is_comptime && !sig.modifiers.is_normal_form_head)
        });
        let is_unsafe = self.package.item(owner).is_some_and(|item| {
            matches!(&item.kind, ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)
                if sig.modifiers.is_unsafe)
        });
        signature
            .with_comptime_phase(comptime)
            .with_unsafe_call_requirement(is_unsafe)
    }
    fn function_instance(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        arguments: Vec<Term>,
        expected: Option<&Term>,
        allow_optional: bool,
    ) -> Result<n::Expr, Error> {
        self.ensure_signature(owner, expr.span)?;
        let parameters = self.type_parameters.get(&owner).ok_or_else(|| {
            error(
                expr.span,
                "explicit instantiation requires a generic function",
            )
        })?;
        if parameters.is_empty() {
            return Err(error(
                expr.span,
                "explicit instantiation requires a generic function",
            ));
        }
        if !allow_optional {
            if let Some(ItemKind::Fn(signature, _) | ItemKind::FnDeclaration(signature)) =
                self.package.item(owner).map(|item| &item.kind)
            {
                arguments::require_function_value_parameters(signature.params, expr.span)?;
            }
        }
        if !allow_optional
            && self.open_functions.contains_key(&owner)
            && self.known_callable_signature(owner).is_none()
        {
            return self.open_function_value(expr, owner, &arguments, expected);
        }
        let scope = self.call_inference_domains();
        let frames = if scope.is_empty() {
            vec![]
        } else {
            vec![scope.iter().map(|p| p.0).collect()]
        };
        let template = self
            .known_callable_signature(owner)
            .or_else(|| self.interface_signatures.get(&owner).cloned())
            .ok_or_else(|| error(expr.span, "declaration is not a supported callable"))?;
        let parameters = self
            .declaration_binders(owner)
            .iter()
            .map(|p| p.declaration)
            .collect::<Vec<_>>();
        let ty = self.instantiate_input_types_in_scope(
            &template,
            &parameters,
            &arguments,
            &frames,
            expr.span,
        )?;
        let ty = self.callable_type(owner, ty);
        self.check_type_application(owner, &arguments, expr.span)?;
        if let Some(expected) = expected {
            self.require(&ty, expected, expr.span)?;
        }
        Ok(n::Expr {
            ty,
            source: source(expr.span),
            kind: K::FunctionInstance {
                declaration: declaration(self.package, &self.package_identities, owner),
                arguments,
            },
        })
    }
    fn explicit_function_instance(
        &mut self,
        expr: &Expr<'_>,
        callee: &Expr<'_>,
        args: &[Arg<'_>],
        expected: Option<&Term>,
        allow_optional: bool,
    ) -> Result<n::Expr, Error> {
        if let H::Projection(base, name) = &callee.kind {
            if let Some(method) =
                self.static_method(base, name.name.as_str(), callee.span, callee.hir_id)?
            {
                return self.explicit_method_instance(expr, method, args, expected, allow_optional);
            }
        }
        let H::ResolvedDef { def_id, .. } = callee.kind else {
            return Err(error(
                expr.span,
                "explicit instantiation requires a resolved generic function",
            ));
        };
        let owner = self
            .package
            .definition_owner(def_id)
            .ok_or_else(|| error(expr.span, "generic function has no HIR owner"))?;
        self.ensure_signature(owner, expr.span)?;
        let lexical = self.lexical_function_arguments(owner, expr.span)?;
        if let Some(value) =
            self.comptime_function_instance(expr, owner, args, &lexical, None, allow_optional)?
        {
            return if allow_optional {
                Ok(value)
            } else {
                self.callable_value_view(value, expected, expr.span)
            };
        }
        if allow_optional {
            if let Some(arguments) =
                self.partial_positional_clause_prefix(owner, args, &lexical, expr.span)?
            {
                return self.partial_callable_instance(expr, owner, arguments);
            }
        }
        if !lexical.is_empty() {
            let arguments =
                self.type_clause_arguments_with_prefix(owner, args, None, expr.span, lexical)?;
            let value = self.function_instance(expr, owner, arguments, None, allow_optional)?;
            return if allow_optional {
                Ok(value)
            } else {
                self.callable_value_view(value, expected, expr.span)
            };
        }
        if self.has_optional_type_clauses(owner)
            || self
                .trait_domains(owner)?
                .iter()
                .any(|domain| *domain != p(P::Type))
        {
            let arguments = self.type_clause_arguments(owner, args, None, expr.span)?;
            let value = self.function_instance(expr, owner, arguments, None, allow_optional)?;
            return if allow_optional {
                Ok(value)
            } else {
                self.callable_value_view(value, expected, expr.span)
            };
        }
        let arguments = self
            .expanded_type_inputs(args, self.type_parameters[&owner].len(), expr.span)?
            .into_iter()
            .map(|(name, ty)| {
                if name.is_some() {
                    Err(error(
                        expr.span,
                        "explicit type arguments currently require positional inputs",
                    ))
                } else {
                    Ok(ty)
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        let value = self.function_instance(expr, owner, arguments, None, allow_optional)?;
        if allow_optional {
            Ok(value)
        } else {
            self.callable_value_view(value, expected, expr.span)
        }
    }
    fn contextual_function_instance(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        let fixed = self.lexical_function_arguments(owner, expr.span)?;
        self.contextual_function_instance_with_fixed(expr, owner, fixed, expected, partial)
    }
    fn contextual_function_instance_with_fixed(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        fixed: Vec<Term>,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        self.contextual_function_instance_with_fixed_view(
            expr, owner, fixed, expected, partial, None,
        )
    }
    fn contextual_function_instance_with_fixed_view(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        fixed: Vec<Term>,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
        view: Option<&n::Expr>,
    ) -> Result<n::Expr, Error> {
        self.ensure_signature(owner, expr.span)?;
        let (generic_parameters, template, pending_defaults) =
            self.function_type_plan(owner, &fixed, expr.span)?;
        if generic_parameters.is_empty() {
            let pending_open = self.open_functions.contains_key(&owner)
                && self.known_callable_signature(owner).is_none();
            let value = self.function_instance(expr, owner, template, None, !pending_open)?;
            return self.callable_value_view(value, expected, expr.span);
        }
        let (original, known_parameters, known_result) = match self
            .known_callable_signature(owner)
            .or_else(|| self.interface_signatures.get(&owner).cloned())
        {
            Some(signature) => (signature, None, true),
            None => {
                let (signature, known, result) = self
                    .open_callable_inference_shape(owner)
                    .ok_or_else(|| error(expr.span, "declaration is not a supported callable"))?;
                (signature, Some(known), result)
            }
        };
        let expected_shape = expected
            .map(|ty| self.structural_type(ty, expr.span))
            .transpose()?;
        // A complete function type owns a parameter frame. Remove it before
        // comparing components in the caller's scope; partial callable contexts
        // already contain caller-relative components. Substitution also rejects
        // dependencies on the removed frame rather than capturing caller slots.
        let (mut actual, mut result) = match (
            expected_shape.as_ref().map(Term::callable_signature),
            partial,
        ) {
            (Some(Term::Function { parameters, result }), _) => (
                parameters
                    .iter()
                    .map(|p| p.ty.substitute(&[]).map(Some))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| error(expr.span, e.to_string()))?,
                Some(
                    result
                        .substitute(&[])
                        .map_err(|e| error(expr.span, e.to_string()))?,
                ),
            ),
            (None, Some(context)) => (context.parameters.clone(), context.result.clone()),
            _ => {
                return Err(context_error(
                    expr.span,
                    "generic function value requires instantiation through a call or function type context",
                ));
            }
        };
        if let Some(known) = known_parameters {
            for (actual, known) in actual.iter_mut().zip(known) {
                if !known {
                    *actual = None;
                }
            }
            if !known_result {
                result = None;
            }
        }
        if let Some(partial) = partial {
            let phase_type = self.callable_type(owner, original.clone());
            if matches!(phase_type, Term::ComptimeFunction(_)) != partial.comptime {
                return Err(error(
                    expr.span,
                    "function context requires a different execution phase",
                ));
            }
        }
        let all_parameters = &self.type_parameters[&owner];
        if fixed.len() > all_parameters.len() {
            return Err(error(
                expr.span,
                "too many fixed generic function arguments",
            ));
        }

        let signature =
            self.instantiate_input_types(&original, all_parameters, &template, expr.span)?;
        // Refine stable components after method-local binders enter inference.
        let scheme = crate::calls::Scheme {
            parameter_sorts: self
                .input_binders(&generic_parameters)
                .iter()
                .map(|p| p.sort)
                .collect(),
            type_parameters: generic_parameters.len(),
            signature: signature
                .abstract_parameters(&self.input_binders(&generic_parameters))
                .map_err(|e| error(expr.span, e.to_string()))?,
        };
        let scope = self.call_inference_domains();
        let mut candidate = scheme
            .infer_sparse_scoped_types(
                &actual,
                result.as_ref(),
                &self.context,
                crate::ptc::Budget::default(),
                &scope,
            )
            .map_err(|e| call_inference_error(expr.span, e))?;
        candidate
            .refine_type_views_metered(
                |ty, budget| {
                    self.callable_input_view_metered(ty, budget, expr.span)
                        .map_err(|e| e.message)
                },
                crate::ptc::Budget::default(),
            )
            .map_err(|e| call_inference_error(expr.span, e))?;
        if !self.settle_function_defaults(
            owner,
            &generic_parameters,
            &template,
            &pending_defaults,
            &mut BTreeMap::new(),
            &mut candidate,
            expr.span,
        )? {
            return Err(context_error(
                expr.span,
                "function type defaults are waiting for inferred inputs",
            ));
        }
        let instance = candidate.finish().map_err(|e| {
            let message = format!("generic function context inference failed: {e:?}");
            Error {
                message,
                ..call_inference_error(expr.span, e)
            }
        })?;
        let arguments = template
            .iter()
            .map(|ty| {
                self.instantiate_input_types(
                    ty,
                    &generic_parameters,
                    &instance.type_arguments,
                    expr.span,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let value = self.function_instance(expr, owner, arguments, None, false)?;
        if view.is_some() {
            Ok(value)
        } else {
            self.callable_value_view(value, expected, expr.span)
        }
    }
    // Staged availability is checked separately from required execution.
    // Carry only the phase context here; do not eagerly invoke deferred inputs.
    fn with_staged_input<R>(&mut self, staged: bool, check: impl FnOnce(&mut Self) -> R) -> R {
        let saved = self.in_staged_input;
        self.in_staged_input |= staged;
        let result = check(self);
        self.in_staged_input = saved;
        result
    }

    /// Check an input transactionally. Only a complete expression publishes
    /// captures, return observations and local state. Immutable declaration
    /// tables are shared; they are not copied for speculative input checking.
    fn argument_attempt(
        &mut self,
        input: &Expr<'_>,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<Option<n::Expr>, Error> {
        self.argument_attempt_with_defaults(input, expected, partial, false)
    }
    fn argument_attempt_with_defaults(
        &mut self,
        input: &Expr<'_>,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
        allow_defaults: bool,
    ) -> Result<Option<n::Expr>, Error> {
        self.argument_attempt_with_input(input, expected, partial, allow_defaults, None)
    }
    fn wrap_inferred_optional(&self, value: n::Expr, span: Span) -> n::Expr {
        n::Expr {
            source: source(span),
            ty: Term::Optional(Box::new(value.ty.clone())),
            kind: K::OptionalPresent(Box::new(value)),
        }
    }

    fn input_context_expression(
        &mut self,
        input: &Expr<'_>,
        context: &crate::calls::InputContext,
    ) -> Result<n::Expr, Error> {
        use crate::calls::InputContext as C;
        if let C::Optional(payload_context) = context {
            let directed = matches!(
                &input.kind,
                H::Null
                    | H::Block(_)
                    | H::If(..)
                    | H::Match(..)
                    | H::When(_)
                    | H::Return(_)
                    | H::EffectRequest(_)
                    | H::ErrorHandle { .. }
                    | H::EffectHandle(..)
            ) || enum_groups::symbol_path(input).is_some()
                || matches!(&input.kind, H::Application(head,_) | H::ExtendedApplication(head,_)
                    if enum_groups::symbol_path(head).is_some());
            if !directed {
                if let Some(mut value) = self.argument_attempt(input, None, None)? {
                    if is_never(&value.ty) {
                        return Ok(value);
                    }
                    // Preserve existing layers first. Only add outer layers required
                    // by the context; the enclosing candidate checks leaf constraints.
                    let mut depth = 0;
                    let mut cursor = context;
                    while let C::Optional(inner) = cursor {
                        depth += 1;
                        if depth > 128 {
                            return Err(error(input.span, "optional context depth exhausted"));
                        }
                        cursor = inner;
                    }
                    let mut ty = value.ty.clone();
                    let mut present = 0;
                    while present < depth {
                        let Ok(inner) = self.optional_payload(&ty, input.span) else {
                            break;
                        };
                        present += 1;
                        ty = inner;
                    }
                    for _ in present..depth {
                        value = self.wrap_inferred_optional(value, input.span);
                    }
                    return Ok(value);
                }
                let value = self.input_context_expression(input, payload_context)?;
                if is_never(&value.ty) {
                    return Ok(value);
                }
                return Ok(self.wrap_inferred_optional(value, input.span));
            }
        }
        let checked = match (context, &input.kind) {
            (
                C::Nominal { declaration, .. },
                H::Application(callee, args) | H::ExtendedApplication(callee, args),
            ) if enum_groups::symbol_path(callee).is_some() => {
                let name = enum_groups::symbol_path(callee).unwrap();
                self.infer_contextual_variant(input, *declaration, name.as_str(), args, context)
            }
            (
                C::Optional(_)
                | C::Tuple(_)
                | C::NormalTuple { .. }
                | C::Record(_)
                | C::Nominal { .. },
                H::Application(..) | H::ExtendedApplication(..) | H::EffectRequest(_),
            )
            | (
                C::Callable(_),
                H::Application(..) | H::ExtendedApplication(..) | H::EffectRequest(_),
            ) => {
                let saved = self
                    .structural_result_context
                    .replace((input.hir_id, context.clone()));
                let result = self.expr(input, None);
                self.structural_result_context = saved;
                result
            }
            (C::Known(ty), _) => self.expr(input, Some(ty)),
            (C::Callable(callable), _) => self.expr_context(input, None, Some(callable)),
            (
                C::Optional(_)
                | C::Tuple(_)
                | C::NormalTuple { .. }
                | C::Record(_)
                | C::Nominal { .. }
                | C::ErrorResult { .. },
                H::Match(value, arms),
            ) => self.match_expr_with_input(input, value, arms, None, None, Some(context)),
            (
                C::Optional(_)
                | C::Tuple(_)
                | C::NormalTuple { .. }
                | C::Record(_)
                | C::Nominal { .. }
                | C::ErrorResult { .. },
                H::When(arms),
            ) => self.when_expr_with_input(input, arms, None, None, Some(context)),
            (
                C::Optional(_)
                | C::Tuple(_)
                | C::NormalTuple { .. }
                | C::Record(_)
                | C::Nominal { .. }
                | C::ErrorResult { .. },
                H::Block(block),
            ) => {
                let saved = self.comptime_callee_result;
                if saved == Some(input.hir_id) {
                    self.comptime_callee_result = block.expr.map(|tail| tail.hir_id);
                }
                let result = self.block_context_with_input(block, None, None, Some(context));
                self.comptime_callee_result = saved;
                result
            }
            (
                C::Optional(_)
                | C::Tuple(_)
                | C::NormalTuple { .. }
                | C::Record(_)
                | C::Nominal { .. }
                | C::ErrorResult { .. },
                H::If(condition, yes, Some(no)),
            ) => {
                let condition = self.expr(condition, Some(&p(P::Bool)))?;
                let inferred =
                    self.optional_branch_context(input.span, context, 2, |checker, index| {
                        checker.under_type_condition(
                            &condition,
                            index == 0,
                            input.span,
                            |checker| {
                                if index == 0 {
                                    checker.block_context_with_input(yes, None, None, Some(context))
                                } else {
                                    checker.input_context_expression(no, context)
                                }
                            },
                        )
                    })?;
                let yes = self.under_type_condition(&condition, true, input.span, |checker| {
                    checker.block_context_with_input(yes, inferred.as_ref(), None, Some(context))
                })?;
                let no = self.under_type_condition(&condition, false, input.span, |checker| {
                    if is_never(&yes.ty) {
                        match &inferred {
                            Some(ty) => checker.expr(no, Some(ty)),
                            None => checker.input_context_expression(no, context),
                        }
                    } else {
                        checker.expr(no, Some(&yes.ty))
                    }
                })?;
                let ty = if is_never(&condition.ty) {
                    p(P::Never)
                } else if is_never(&yes.ty) {
                    no.ty.clone()
                } else {
                    self.require(&no.ty, &yes.ty, input.span)?;
                    yes.ty.clone()
                };
                Ok(n::Expr {
                    ty,
                    source: source(input.span),
                    kind: K::If {
                        condition: Box::new(condition),
                        then_branch: Box::new(yes),
                        else_branch: Some(Box::new(no)),
                    },
                })
            }
            (C::Record(fields), H::CollectedProperties(properties)) => {
                self.property_packet_with_context(input, properties, None, Some(fields))
            }
            (C::Tuple(fields) | C::NormalTuple { fields, .. }, H::Unit) if fields.is_empty() => {
                Ok(n::Expr {
                    ty: self.tuple_value_type(Vec::new(), input.span)?,
                    source: source(input.span),
                    kind: K::Tuple(Vec::new()),
                })
            }
            (C::Tuple(fields) | C::NormalTuple { fields, .. }, H::Tuple(items)) => {
                if fields.len() != items.len() {
                    return Err(error(input.span, "tuple argument arity mismatch"));
                }
                let values = items
                    .iter()
                    .zip(fields)
                    .map(|(item, context)| self.input_context_expression(item, context))
                    .collect::<Result<Vec<_>, _>>()?;
                let ty = if values.iter().any(|value| is_never(&value.ty)) {
                    p(P::Never)
                } else {
                    self.tuple_value_type(
                        values.iter().map(|value| value.ty.clone()).collect(),
                        input.span,
                    )?
                };
                Ok(n::Expr {
                    ty,
                    source: source(input.span),
                    kind: K::Tuple(values),
                })
            }
            (C::ErrorResult { errors, .. }, H::ErrorNew(_)) => self.expr(
                input,
                Some(&Term::error_result(errors.clone(), p(P::Never))),
            ),
            (C::ErrorResult { result, .. }, _) => self.input_context_expression(input, result),
            _ => self.expr(input, None),
        }?;
        if let C::ErrorResult { errors, .. } = context {
            self.deliver_error_context(checked, errors, input.span)
        } else {
            Ok(checked)
        }
    }

    fn argument_attempt_with_input(
        &mut self,
        input: &Expr<'_>,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
        allow_defaults: bool,
        input_context: Option<&crate::calls::InputContext>,
    ) -> Result<Option<n::Expr>, Error> {
        self.checking_attempt(allow_defaults, true, |checker| {
            if expected.is_none()
                && matches!(
                    input_context,
                    Some(
                        crate::calls::InputContext::Optional(_)
                            | crate::calls::InputContext::Tuple(_)
                            | crate::calls::InputContext::NormalTuple { .. }
                            | crate::calls::InputContext::Record(_)
                            | crate::calls::InputContext::Nominal { .. }
                            | crate::calls::InputContext::Callable(_)
                    )
                )
            {
                match input_context {
                    Some(input_context) => checker.input_context_expression(input, input_context),
                    None => checker.expr_context(input, expected, partial),
                }
            } else {
                checker.expr_context(input, expected, partial)
            }
        })
    }

    /// Speculative branches share the argument transaction but never publish
    /// their local bindings, captures or return evidence, even on success.
    fn checking_attempt(
        &mut self,
        allow_defaults: bool,
        commit: bool,
        check: impl FnOnce(&mut Self) -> Result<n::Expr, Error>,
    ) -> Result<Option<n::Expr>, Error> {
        let saved = (
            self.locals.clone(),
            self.code_values.clone(),
            self.pattern_codes.clone(),
            self.aliases.clone(),
            self.result.clone(),
            self.partial_return_context.clone(),
            self.pending_optional_returns.clone(),
            self.pending_error_returns.clone(),
            self.inferred_return_sites.clone(),
            self.captures.clone(),
            self.return_observations.clone(),
            self.loops.clone(),
            self.immutable.clone(),
            self.context.clone(),
            self.field_defaults.clone(),
            self.active_defaults.clone(),
            self.specialization_inputs.clone(),
            self.type_input_bindings.clone(),
            self.partial_staged_bindings.clone(),
            self.value_input_bindings.clone(),
            self.derived_staged_bindings.clone(),
        );
        let pending_len = self.pending_type_uses.as_ref().map(Vec::len);
        let old_defaults = std::mem::replace(&mut self.numeric_defaults, allow_defaults);
        let pattern_pending = self.pattern_quote_pending;
        let attempt = check(self);
        if attempt.is_err() {
            self.pattern_quote_pending = pattern_pending;
        }
        self.numeric_defaults = old_defaults;
        if !commit || attempt.is_err() {
            if let (Some(pending), Some(len)) = (&mut self.pending_type_uses, pending_len) {
                pending.truncate(len);
            }
            (
                self.locals,
                self.code_values,
                self.pattern_codes,
                self.aliases,
                self.result,
                self.partial_return_context,
                self.pending_optional_returns,
                self.pending_error_returns,
                self.inferred_return_sites,
                self.captures,
                self.return_observations,
                self.loops,
                self.immutable,
                self.context,
                self.field_defaults,
                self.active_defaults,
                self.specialization_inputs,
                self.type_input_bindings,
                self.partial_staged_bindings,
                self.value_input_bindings,
                self.derived_staged_bindings,
            ) = saved;
        }
        match attempt {
            Ok(value) => Ok(Some(value)),
            Err(failure) if failure.needs_context => Ok(None),
            Err(failure) => Err(failure),
        }
    }

    fn optional_branch_context(
        &mut self,
        span: Span,
        context: &crate::calls::InputContext,
        count: usize,
        mut check: impl FnMut(&mut Self, usize) -> Result<n::Expr, Error>,
    ) -> Result<Option<Term>, Error> {
        if !matches!(context, crate::calls::InputContext::Optional(_))
            && !matches!(context, crate::calls::InputContext::ErrorResult { .. })
        {
            return Ok(None);
        }
        // Prefer evidence from every branch before defaulting numeric literals.
        for defaults in [false, true] {
            if defaults && !self.numeric_defaults {
                break;
            }
            for index in 0..count {
                self.optional_branch_probe_budget = self
                    .optional_branch_probe_budget
                    .checked_sub(1)
                    .ok_or_else(|| {
                        error(
                            span,
                            "optional branch inference budget exhausted; add a type annotation",
                        )
                    })?;
                let probing = std::mem::replace(&mut self.probing_optional_branch, true);
                let attempt =
                    self.checking_attempt(defaults, false, |checker| check(checker, index));
                self.probing_optional_branch = probing;
                if let Some(value) = attempt? {
                    if !is_never(&value.ty)
                        && !matches!(&value.ty, Term::ErrorResult { result, .. } if is_never(result))
                    {
                        return Ok(Some(value.ty));
                    }
                }
            }
        }
        // The enclosing probe only needs a result type. With no evidence,
        // falling through to ordinary branch checking would probe this same
        // subtree again, multiplying work for every nested null-only branch.
        // The final, non-speculative check still validates every branch.
        if self.probing_optional_branch {
            return Err(context_error(
                span,
                "optional branches need a result type context",
            ));
        }
        Ok(None)
    }
    fn generic_call(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        args: &[Arg<'_>],
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        let known = self.lexical_function_arguments(owner, expr.span)?;
        self.generic_call_with_receiver(expr, owner, args, expected, partial, Vec::new(), known)
    }
    fn generic_call_with_receiver(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        args: &[Arg<'_>],
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
        prefix: Vec<n::Expr>,
        known: Vec<Term>,
    ) -> Result<n::Expr, Error> {
        if self.is_quote_function(owner) {
            self.record_quote_application(expr.span)?;
        }
        let structural_result = self
            .structural_result_context
            .as_ref()
            .filter(|(id, _)| *id == expr.hir_id)
            .map(|(_, context)| context.clone());
        self.ensure_signature(owner, expr.span)?;
        let all_parameters = self.type_parameters[&owner].clone();
        if known.len() > all_parameters.len() {
            return Err(error(expr.span, "too many fixed generic call arguments"));
        }
        let (parameters, template, pending_defaults) =
            self.function_type_plan(owner, &known, expr.span)?;
        let signature = self.instantiate_input_types(
            self.signatures
                .get(&owner)
                .or_else(|| self.interface_signatures.get(&owner))
                .ok_or_else(|| error(expr.span, "declaration is not a supported callable"))?,
            &all_parameters,
            &template,
            expr.span,
        )?;
        let Some(ItemKind::Fn(declaration_sig, _) | ItemKind::FnDeclaration(declaration_sig)) =
            self.package.item(owner).map(|item| &item.kind)
        else {
            unreachable!()
        };
        let declarations = declaration_sig.params;
        let offset = prefix.len();
        if offset > declarations.len() {
            return Err(error(expr.span, "method receiver count mismatch"));
        }
        let (routes, supplied) = route_arguments(
            args,
            Some(&declarations[offset..]),
            declarations.len() - offset,
            expr.span,
        )?;
        let assigned: Vec<_> = std::iter::repeat_n(true, offset).chain(supplied).collect();
        let Term::Function {
            parameters: ordinary,
            result,
        } = &signature
        else {
            unreachable!()
        };
        let (known_inputs, known_values, seed, mut collected) = self
            .infer_independent_call_inputs(
                ordinary,
                result,
                &parameters,
                &routes,
                offset,
                &prefix,
                expr,
                expected,
            )?;
        let caller_scope = self.call_inference_domains();
        let caller_frames = if caller_scope.is_empty() {
            vec![]
        } else {
            vec![caller_scope.iter().map(|p| p.0).collect()]
        };
        let prepared_signature = self.instantiate_input_types_in_scope(
            &signature,
            &known_inputs,
            &known_values,
            &caller_frames,
            expr.span,
        )?;
        let preview_arguments = template
            .iter()
            .map(|ty| {
                self.instantiate_input_types_in_scope(
                    ty,
                    &known_inputs,
                    &known_values,
                    &caller_frames,
                    expr.span,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let preview = n::Expr {
            source: source(expr.span),
            ty: self.callable_type(owner, prepared_signature.clone()),
            kind: K::FunctionInstance {
                declaration: declaration(self.package, &self.package_identities, owner),
                arguments: preview_arguments,
            },
        };
        let Term::Function {
            parameters: prepared_parameters,
            result: prepared_result,
        } = &prepared_signature
        else {
            unreachable!()
        };
        let mut prepared = self.prepare_call_types(
            prepared_parameters,
            prepared_result,
            &preview,
            Some(owner),
            &routes,
            offset,
            &assigned[offset..],
            &prefix,
            expr,
            seed,
        )?;
        let call_binding =
            self.prepare_call_binding(prepared_parameters, &prepared.symbols, expr.span)?;
        // Ordinary Type inputs are values in the function frame, whereas the
        // clause parameters below become inference variables in an outer frame.
        let inference_signature = Term::Function {
            parameters: ordinary
                .iter()
                .map(|parameter| {
                    Ok(Parameter {
                        sort: parameter.sort,
                        ty: call_binding
                            .substitute(&parameter.ty)
                            .and_then(|ty| ty.abstract_parameters(&[]))
                            .map_err(|e| error(expr.span, e.to_string()))?,
                    })
                })
                .collect::<Result<Vec<_>, Error>>()?,
            result: Box::new(
                call_binding
                    .substitute(result)
                    .and_then(|ty| ty.abstract_parameters(&[]))
                    .map_err(|e| error(expr.span, e.to_string()))?,
            ),
        };
        let input_domains = parameters
            .iter()
            .map(|id| self.trait_input_domain(*id).unwrap_or_else(|| p(P::Type)))
            .collect::<Vec<_>>();
        let scheme = crate::calls::Scheme {
            parameter_sorts: self
                .input_binders(&parameters)
                .iter()
                .map(|p| p.sort)
                .collect(),
            type_parameters: parameters.len(),
            signature: inference_signature
                .abstract_parameters(&self.input_binders(&parameters))
                .map_err(|e| error(expr.span, e.to_string()))?,
        };
        self.refresh_list_inference_metadata();
        let mut inputs = vec![None; declarations.len()];
        let mut checked = vec![None; declarations.len()];
        for (index, value) in prefix.into_iter().enumerate() {
            checked[index] = Some(prepared.values[index].take().unwrap_or(value));
        }
        let discharge_effects = self.active_effect_environment().is_some()
            && !matches!(expected, Some(Term::Computation { .. }));
        let initial_types: Vec<_> = checked
            .iter()
            .map(|value| value.as_ref().and_then(arguments::inference_type).cloned())
            .collect();
        // Propagate result information before probing children. Besides making
        // bidirectional checking effective for nested calls, this avoids
        // recursively repeating context-free attempts along an identity chain.
        let mut initial = scheme
            .infer_declared_sparse_types_mode(
                &parameters,
                &input_domains,
                &initial_types,
                expected,
                partial,
                &self.context,
                crate::ptc::Budget::default(),
                discharge_effects,
                &caller_scope,
            )
            .map_err(|e| call_inference_error(expr.span, e))?;
        if let Some(context) = &structural_result {
            initial
                .constrain_result_context(context, crate::ptc::Budget::default())
                .map_err(|e| call_inference_error(expr.span, e))?;
        }
        initial
            .refine_type_views_metered(
                |ty, budget| {
                    self.callable_input_view_metered(ty, budget, expr.span)
                        .map_err(|e| e.message)
                },
                crate::ptc::Budget::default(),
            )
            .map_err(|e| call_inference_error(expr.span, e))?;
        let mut resolved_defaults = BTreeMap::new();
        self.settle_function_defaults(
            owner,
            &parameters,
            &template,
            &pending_defaults,
            &mut resolved_defaults,
            &mut initial,
            expr.span,
        )?;
        for route in &routes {
            let parameter = &(route.parameter + offset);
            inputs[*parameter] = Some(route);
            let value = route.expression(expr);
            let context = initial
                .parameter_type(*parameter)
                .map_err(|e| call_inference_error(value.span, e))?;
            let partial = if context.is_none() {
                initial
                    .parameter_callable_context(*parameter)
                    .map_err(|e| call_inference_error(value.span, e))?
            } else {
                None
            };
            let input_context = initial
                .parameter_input_context(*parameter)
                .map_err(|e| call_inference_error(value.span, e))?;
            if let Some(ty) = &context {
                route.require_collection_type(self, ty, expr.span)?;
            }
            if context.is_none()
                && matches!(&scheme.signature,Term::Function {parameters,..} if matches!(parameters[*parameter].ty,Term::Computation {..}))
            {
                self.preserve_effect_value = true;
            }
            checked[*parameter] = if let Some(value) = prepared.values[*parameter].take() {
                if route.has_collection_fields() {
                    if let K::Tuple(fields) | K::AnonymousRecord { fields } = &value.kind {
                        collected.insert(*parameter, fields.iter().cloned().map(Some).collect());
                    }
                }
                Some(value)
            } else if route.has_collection_fields() {
                collected.entry(*parameter).or_default();
                None
            } else {
                self.with_staged_input(declarations[*parameter].ty.is_comptime(), |this| {
                    this.argument_attempt_with_input(
                        &value,
                        context.as_ref(),
                        partial.as_ref(),
                        false,
                        Some(&input_context),
                    )
                })?
            };
        }
        for (index, assigned) in assigned.iter().enumerate() {
            if !assigned {
                checked[index] = Some(if let Some(value) = prepared.values[index].take() {
                    value
                } else {
                    let mut value = prepared.defaults[index].take().unwrap();
                    value
                        .rewrite_types(&mut |ty| call_binding.substitute(ty))
                        .map_err(|e| error(expr.span, e.to_string()))?;
                    value
                });
            }
        }
        let instance = loop {
            let actual: Vec<_> = checked
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    value
                        .as_ref()
                        .and_then(arguments::inference_type)
                        // A definition-side default may still contain this
                        // declaration's rigid binders. It is not a concrete
                        // caller input that can constrain those same binders.
                        .filter(|ty| {
                            assigned[index]
                                || ty
                                    .abstract_parameters(&self.input_binders(&parameters))
                                    .is_ok_and(|t| t.check_scope(&[]).is_ok())
                        })
                        .cloned()
                })
                .collect();
            let mut candidate = scheme
                .infer_declared_sparse_types_mode(
                    &parameters,
                    &input_domains,
                    &actual,
                    expected,
                    partial,
                    &self.context,
                    crate::ptc::Budget::default(),
                    discharge_effects,
                    &caller_scope,
                )
                .map_err(|e| call_inference_error(expr.span, e))?;
            for (index, values) in &collected {
                if checked[*index]
                    .as_ref()
                    .is_some_and(|value| !is_never(&value.ty))
                {
                    continue;
                }
                if values.is_empty() {
                    continue;
                }
                let context = inputs[*index].unwrap().retained_collection_context(
                    self,
                    values,
                    &candidate
                        .parameter_input_context(*index)
                        .map_err(|e| call_inference_error(expr.span, e))?,
                );
                candidate
                    .constrain_parameter_context(*index, &context, crate::ptc::Budget::default())
                    .map_err(|e| call_inference_error(expr.span, e))?;
            }
            for (index, route) in inputs.iter().enumerate() {
                if let Some(context) = route.and_then(|route| route.local_collection_context(self))
                {
                    candidate
                        .constrain_parameter_context(index, &context, crate::ptc::Budget::default())
                        .map_err(|e| call_inference_error(expr.span, e))?;
                }
            }
            if let Some(context) = &structural_result {
                candidate
                    .constrain_result_context(context, crate::ptc::Budget::default())
                    .map_err(|e| call_inference_error(expr.span, e))?;
            }
            candidate
                .refine_type_views_metered(
                    |ty, budget| {
                        self.callable_input_view_metered(ty, budget, expr.span)
                            .map_err(|e| e.message)
                    },
                    crate::ptc::Budget::default(),
                )
                .map_err(|e| call_inference_error(expr.span, e))?;
            let defaults_ready = self.settle_function_defaults(
                owner,
                &parameters,
                &template,
                &pending_defaults,
                &mut resolved_defaults,
                &mut candidate,
                expr.span,
            )?;
            if checked.iter().all(Option::is_some) {
                if !defaults_ready {
                    return Err(context_error(
                        expr.span,
                        "function type defaults are waiting for inferred inputs",
                    ));
                }
                break candidate
                    .finish()
                    .map_err(|e| call_inference_error(expr.span, e))?;
            }
            let mut progress = false;
            for (index, input) in inputs.iter().enumerate() {
                if checked[index].is_some() {
                    continue;
                }
                let route = input.expect("an unchecked parameter has an explicit input");
                if route.has_collection_fields() {
                    let context = candidate
                        .parameter_input_context(index)
                        .map_err(|e| call_inference_error(expr.span, e))?;
                    let (value, advanced) = self.attempt_collection_fields(
                        expr,
                        route,
                        &context,
                        collected.entry(index).or_default(),
                        declarations[index].ty.is_comptime(),
                        false,
                    )?;
                    checked[index] = value;
                    progress |= advanced;
                    continue;
                }
                let input = route.expression(expr);
                let context = candidate.parameter_type(index).map_err(|e| {
                    error(input.span, format!("generic call inference failed: {e:?}"))
                })?;
                let partial = if context.is_none() {
                    candidate
                        .parameter_callable_context(index)
                        .map_err(|e| call_inference_error(input.span, e))?
                } else {
                    None
                };
                if let Some(ty) = &context {
                    route.require_collection_type(self, ty, expr.span)?;
                }
                let input_context = candidate
                    .parameter_input_context(index)
                    .map_err(|e| call_inference_error(input.span, e))?;
                if context.is_some()
                    || partial.is_some()
                    || matches!(
                        input_context,
                        crate::calls::InputContext::Optional(_)
                            | crate::calls::InputContext::Tuple(_)
                            | crate::calls::InputContext::NormalTuple { .. }
                            | crate::calls::InputContext::Record(_)
                            | crate::calls::InputContext::Nominal { .. }
                    )
                {
                    checked[index] =
                        self.with_staged_input(declarations[index].ty.is_comptime(), |this| {
                            this.argument_attempt_with_input(
                                &input,
                                context.as_ref(),
                                partial.as_ref(),
                                false,
                                Some(&input_context),
                            )
                        })?;
                    progress |= checked[index].is_some();
                }
            }
            if !progress && self.numeric_defaults {
                for (index, input) in inputs.iter().enumerate() {
                    if checked[index].is_some() {
                        continue;
                    }
                    let route = input.expect("unchecked explicit argument");
                    if route.has_collection_fields() {
                        let context = candidate
                            .parameter_input_context(index)
                            .map_err(|e| call_inference_error(expr.span, e))?;
                        let (value, advanced) = self.attempt_collection_fields(
                            expr,
                            route,
                            &context,
                            collected.entry(index).or_default(),
                            declarations[index].ty.is_comptime(),
                            true,
                        )?;
                        checked[index] = value;
                        if advanced {
                            progress = true;
                            break;
                        }
                        continue;
                    }
                    let input = route.expression(expr);
                    let context = candidate
                        .parameter_type(index)
                        .map_err(|e| call_inference_error(input.span, e))?;
                    let partial = candidate
                        .parameter_callable_context(index)
                        .map_err(|e| call_inference_error(input.span, e))?;
                    let input_context = candidate
                        .parameter_input_context(index)
                        .map_err(|e| call_inference_error(input.span, e))?;
                    if let Some(ty) = &context {
                        route.require_collection_type(self, ty, expr.span)?;
                    }
                    checked[index] =
                        self.with_staged_input(declarations[index].ty.is_comptime(), |this| {
                            this.argument_attempt_with_input(
                                &input,
                                context.as_ref(),
                                partial.as_ref(),
                                true,
                                Some(&input_context),
                            )
                        })?;
                    // Recompute constraints before defaulting a second input.
                    if checked[index].is_some() {
                        progress = true;
                        break;
                    }
                }
            }
            if !progress {
                return Err(context_error(
                    expr.span,
                    "generic argument context is unresolved; add a type annotation or explicit type arguments",
                ));
            }
        };
        let full_arguments = template
            .iter()
            .map(|ty| {
                self.instantiate_input_types_in_scope(
                    ty,
                    &parameters,
                    &instance.type_arguments,
                    &caller_frames,
                    expr.span,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.check_type_application(owner, &full_arguments, expr.span)?;
        self.check_ordinary_capabilities(owner, &full_arguments, &prepared.symbols, expr.span)?;
        self.check_ordinary_type_requirements(
            owner,
            &full_arguments,
            &prepared.symbols,
            expr.span,
        )?;
        // Elaboration may visit delayed inputs later, but runtime evaluation
        // remains in the original source order and each expression occurs once.
        let mut arguments = Vec::new();
        for (parameter, value) in checked.iter_mut().enumerate().take(offset) {
            arguments.push(n::CallArgument {
                passing: Default::default(),
                parameter,
                value: value.take().unwrap(),
                is_default: false,
            });
        }
        for route in &routes {
            let parameter = route.parameter + offset;
            arguments.push(n::CallArgument {
                passing: Default::default(),
                parameter,
                value: checked[parameter].take().unwrap(),
                is_default: false,
            });
        }
        for (parameter, assigned) in assigned.iter().enumerate() {
            if !assigned {
                let mut value = checked[parameter].take().unwrap();
                value
                    .rewrite_types(&mut |ty| {
                        ty.instantiate_parameters_in_scope(
                            &self.input_binders(&parameters),
                            &instance.type_arguments,
                            &caller_frames,
                        )
                    })
                    .map_err(|e| error(expr.span, e.to_string()))?;
                value.rewrite_types(&mut |ty| self.callable_input_view(ty, expr.span))?;
                arguments.push(n::CallArgument {
                    passing: Default::default(),
                    parameter,
                    value,
                    is_default: true,
                });
            }
        }
        if let Term::Function {
            parameters: completed,
            ..
        } = &instance.signature
        {
            for argument in &mut arguments {
                let target = completed[argument.parameter]
                    .ty
                    .substitute(&[])
                    .map_err(|e| error(expr.span, e.to_string()))?;
                self.adapt_error_argument_layout(&mut argument.value, &target, expr.span, 0)?;
            }
        }
        self.comptime_receiver_arguments(Some(owner), &mut arguments);
        let actual: Vec<_> = arguments.iter().map(|arg| arg.value.ty.clone()).collect();
        let ty = if actual.iter().any(is_never) {
            p(P::Never)
        } else {
            self.callable_input_view(&instance.result, expr.span)?
        };
        let call = n::Expr {
            ty,
            source: source(expr.span),
            kind: K::Call {
                callee: Box::new(n::Expr {
                    ty: self.callable_type(
                        owner,
                        self.instantiate_input_types_in_scope(
                            &signature,
                            &parameters,
                            &instance.type_arguments,
                            &caller_frames,
                            expr.span,
                        )?,
                    ),
                    source: source(expr.span),
                    kind: K::FunctionInstance {
                        declaration: declaration(self.package, &self.package_identities, owner),
                        arguments: full_arguments,
                    },
                }),
                arguments,
                parameter_bindings: declarations
                    .iter()
                    .map(|param| local(param.hir_id))
                    .collect(),
            },
        };
        let call = self.freeze_staged_call_inputs(call, expr)?;
        self.stage_collected_call(call, &routes, offset, expr.span)
    }
    fn call(
        &mut self,
        expr: &Expr<'_>,
        callee: &Expr<'_>,
        args: &[Arg<'_>],
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        // Direct comptime calls must be checked in their source phase before
        // runtime expected types or spread preparation can affect their inputs.
        let head = if let H::NFApplication(head, _) = callee.kind {
            head
        } else {
            callee
        };
        if let H::ResolvedDef { def_id, .. } = head.kind {
            if self.package.definition_owner(def_id).is_some_and(|owner| {
                self.is_code_generator(owner) || self.is_pattern_generator(owner)
            }) {
                return Err(error(
                    expr.span,
                    "code generator must be invoked in a comptime code position",
                ));
            }
        }
        let comptime = if let H::ResolvedDef { def_id, .. } = head.kind {
            self.package.definition_owner(def_id).and_then(|owner| self.package.item(owner))
                .is_some_and(|item| matches!(&item.kind, ItemKind::Fn(sig, _) if sig.modifiers.is_comptime && !sig.modifiers.is_normal_form_head))
        } else {
            false
        };
        if comptime && self.protocols.splice.is_some() && self.runtime_value_context() {
            let value =
                self.with_staged_input(true, |this| this.call(expr, callee, args, None, None))?;
            return self.deliver_comptime_result(expr, value, expected);
        }
        if args.iter().any(|arg| matches!(arg, Arg::Expand(_))) {
            return self.spread_call(expr, callee, args, expected, partial);
        }
        if let Some(call) = self.receiver_call(expr, callee, args, expected, partial)? {
            return Ok(call);
        }
        let explicit = if let H::NFApplication(head, types) = &callee.kind {
            Some((*head, *types))
        } else {
            None
        };
        let head = explicit.map_or(callee, |(head, _)| head);
        let direct = if let H::ResolvedDef { def_id, .. } = head.kind {
            self.package.definition_owner(def_id).filter(|owner| {
                !matches!(
                    self.package.item(*owner).map(|item| &item.kind),
                    Some(ItemKind::Const(..) | ItemKind::Associated(_))
                )
            })
        } else {
            None
        };
        if let Some(owner) = direct {
            if matches!(&self.package.item(owner).unwrap().kind, ItemKind::Fn(sig, _) if sig.modifiers.is_normal_form_head)
            {
                return Err(error(
                    callee.span,
                    "normal-form heads cannot be invoked as ordinary functions",
                ));
            }
            self.require_non_trait_body_reference(owner, callee.span)?;
            self.ensure_signature(owner, expr.span)?;
            if self.open_functions.contains_key(&owner) {
                return self.explicit_open_function_call(
                    expr,
                    owner,
                    explicit.map(|(_, args)| args),
                    args,
                    expected,
                );
            }
            if explicit.is_none()
                && self
                    .type_parameters
                    .get(&owner)
                    .is_some_and(|types| !types.is_empty())
            {
                return self.generic_call(expr, owner, args, expected, partial);
            }
        }
        let callee = if let Some((head, types)) = explicit {
            self.explicit_function_instance(callee, head, types, None, true)?
        } else if let Some(owner) = direct {
            self.ensure_signature(owner, expr.span)?;
            let ty = self
                .signatures
                .get(&owner)
                .ok_or_else(|| error(callee.span, "declaration is not a supported callable"))?
                .clone();
            let ty = self.callable_type(owner, ty);
            self.check_type_application(owner, &[], expr.span)?;
            n::Expr {
                ty,
                source: source(callee.span),
                kind: if self.requirements.get(&owner).is_some_and(Vec::is_empty) {
                    K::Function(declaration(self.package, &self.package_identities, owner))
                } else {
                    K::FunctionInstance {
                        declaration: declaration(self.package, &self.package_identities, owner),
                        arguments: vec![],
                    }
                },
            }
        } else {
            self.invocation_callee(callee)?
        };
        self.finish_call(expr, callee, args, Vec::new(), expected, partial)
    }

    /// One source-ordered plan for ordinary, static and receiver calls.
    fn finish_call(
        &mut self,
        expr: &Expr<'_>,
        callee: n::Expr,
        args: &[Arg<'_>],
        prefix: Vec<n::Expr>,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        if prefix.is_empty() || !matches!(callee.kind, K::ComptimeFunctionInstance { .. }) {
            return self.finish_prepared_call(expr, callee, args, prefix, expected, partial);
        }
        // The receiver is the leftmost source computation. Bind it outside
        // clause preparation, also when a spread later wraps the inner call.
        let saved_locals = self.locals.clone();
        let result = (|| {
            let mut statements = Vec::new();
            let mut prepared = Vec::new();
            for value in prefix {
                prepared.push(self.bind_type_input(value, &mut statements, expr.span)?);
            }
            let call =
                self.finish_prepared_call(expr, callee, args, prepared, expected, partial)?;
            Ok(n::Expr {
                source: source(expr.span),
                ty: call.ty.clone(),
                kind: K::Block {
                    statements,
                    result: Some(Box::new(call)),
                },
            })
        })();
        self.locals = saved_locals;
        result
    }

    fn finish_prepared_call(
        &mut self,
        expr: &Expr<'_>,
        mut callee: n::Expr,
        args: &[Arg<'_>],
        prefix: Vec<n::Expr>,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        if callee.ty.is_normal_form_callable() && !matches!(expr.kind, H::NFApplication(..)) {
            return Err(error(
                expr.span,
                "normal-form heads cannot be invoked as ordinary functions",
            ));
        }
        // Selected trait methods and stored comptime callables use this same
        // boundary. The synthesized splice method itself remains runtime.
        if callee.ty.is_comptime_callable()
            && self.protocols.splice.is_some()
            && self.runtime_value_context()
        {
            let value = self.with_staged_input(true, |this| {
                this.finish_call(expr, callee, args, prefix, None, None)
            })?;
            return self.deliver_comptime_result(expr, value, expected);
        }
        let direct_id = match &callee.kind {
            K::ComptimeFunctionInstance {
                declaration: id, ..
            }
            | K::Function(id)
            | K::FunctionInstance {
                declaration: id, ..
            } => Some(*id),
            // Interface entry metadata remains independent of the selected body.
            K::TraitMethod { member, .. } => Some(*member),
            _ => None,
        };
        let direct = direct_id
            .filter(|_| !callee.ty.is_normal_form_callable())
            .and_then(|id| {
                self.package.owners().find_map(|(owner, _)| {
                    matches_declaration(self.package, &self.package_identities, owner, id)
                        .then_some(owner)
                })
            });
        if direct.is_some_and(|owner| self.is_quote_function(owner)) {
            self.record_quote_application(expr.span)?;
        }
        if let (Some(owner), K::FunctionInstance { arguments, .. }) = (direct, &callee.kind) {
            if arguments.len() < self.type_parameters[&owner].len() {
                return self.generic_call_with_receiver(
                    expr,
                    owner,
                    args,
                    expected,
                    partial,
                    prefix,
                    arguments.clone(),
                );
            }
        }
        // Preserve selected clause arguments when a constant/function value
        // denotes an open declaration. Analysis-only signatures suppress this
        // route while constructing ordinary behavior call edges.
        if let Some(owner) = direct
            && self.open_functions.contains_key(&owner)
            && !self.signatures.contains_key(&owner)
            && prefix.is_empty()
        {
            let clauses = match &callee.kind {
                K::Function(_) => Some(Vec::new()),
                K::FunctionInstance { arguments, .. } => Some(arguments.clone()),
                _ => None,
            };
            if let Some(clauses) = clauses {
                self.check_type_application(owner, &clauses, expr.span)?;
                let saved = self.alias_substitution.clone();
                for (binder, argument) in self.declaration_binders(owner).into_iter().zip(clauses) {
                    self.alias_substitution
                        .insert(binder.declaration, std::sync::Arc::new(argument));
                }
                let result = self.instantiated_open_call(expr, owner, args, expected);
                self.alias_substitution = saved;
                return result;
            }
        }
        if let (
            Some(owner),
            K::TraitMethod {
                interface_arguments,
                method_arguments,
                ..
            },
        ) = (direct, &callee.kind)
        {
            if 1 + interface_arguments.len() + method_arguments.len()
                < self.type_parameters[&owner].len()
            {
                return self
                    .generic_trait_call(expr, owner, callee, args, prefix, expected, partial);
            }
        }
        let retained_callable = self.structural_type(&callee.ty, expr.span)?;
        let callable = self.callable_input_view(&retained_callable, expr.span)?;
        let signature = if let Term::Closure { signature, .. } = &callable {
            signature.as_ref()
        } else {
            &callable
        };
        let Term::Function { parameters, result } = signature.callable_signature() else {
            return Err(error(
                expr.span,
                if is_never(&callee.ty) {
                    "diverging callee has no callable signature for checking its arguments"
                } else {
                    "called expression is not a function"
                },
            ));
        };
        let declarations = direct
            .and_then(|owner| self.package.item(owner))
            .and_then(|item| match &item.kind {
                ItemKind::Fn(signature, _) | ItemKind::FnDeclaration(signature) => {
                    Some(signature.params)
                }
                _ => None,
            });
        let offset = prefix.len();
        if offset > parameters.len() {
            return Err(error(expr.span, "method receiver count mismatch"));
        }
        let (routes, assigned) = route_arguments(
            args,
            declarations.map(|p| &p[offset..]),
            parameters.len() - offset,
            expr.span,
        )
        .map_err(|mut failure| {
            if offset != 0 && failure.message.starts_with("argument count mismatch") {
                failure.message = format!("method {}", failure.message);
            }
            failure
        })?;
        let mut prepared = self.prepare_call_types(
            parameters,
            result,
            &callee,
            direct,
            &routes,
            offset,
            &assigned,
            &prefix,
            expr,
            vec![],
        )?;
        let mut arguments: Vec<_> = prefix
            .into_iter()
            .enumerate()
            .map(|(parameter, value)| n::CallArgument {
                passing: Default::default(),
                parameter,
                value,
                is_default: false,
            })
            .collect();
        let type_inputs = prepared.symbols;
        let call_binding = self.prepare_call_binding(&parameters, &type_inputs, expr.span)?;
        if let Some(owner) = direct {
            // Trait entries carry Self, interface arguments and method arguments
            // separately. Contract checking needs their complete declaration substitution.
            let type_arguments = self
                .method_instantiation_inputs(&callee)
                .map(|(_, arguments)| arguments)
                .unwrap_or_default();
            self.check_ordinary_capabilities(owner, &type_arguments, &type_inputs, expr.span)?;
            self.check_ordinary_type_requirements(owner, &type_arguments, &type_inputs, expr.span)?;
        }
        for route in &routes {
            let parameter = route.parameter + offset;
            let input_type = call_binding
                .substitute(&parameters[parameter].ty)
                .map_err(|e| {
                    error(
                        expr.span,
                        format!("dependent call input is not available: {e}"),
                    )
                })?;
            let input_type = self.callable_input_view(&input_type, expr.span)?;
            route.require_collection_type(self, &input_type, expr.span)?;
            let value = route.expression(expr);
            let checked = if let Some(value) = prepared.values[parameter].take() {
                value
            } else {
                self.with_staged_input(parameters[parameter].sort != Sort::Runtime, |this| {
                    this.expr(&value, Some(&input_type))
                })?
            };
            arguments.push(n::CallArgument {
                passing: Default::default(),
                parameter,
                value: checked,
                is_default: false,
            });
        }
        for (parameter, assigned) in assigned.iter().enumerate() {
            let parameter = parameter + offset;
            if !assigned {
                let value = if let Some(value) = prepared.values[parameter].take() {
                    value
                } else {
                    let mut value = prepared.defaults[parameter].take().unwrap();
                    // Value bindings name the callee; annotations name the caller.
                    value
                        .rewrite_types(&mut |ty| call_binding.substitute(ty))
                        .map_err(|e| error(expr.span, e.to_string()))?;
                    value.rewrite_types(&mut |ty| {
                        // The traversal includes clause value terms as well as
                        // annotations. A pending value is not a Type projection;
                        // the selected comptime instance supplies it at invocation.
                        if ty.sort() == Sort::Type {
                            self.callable_input_view(ty, expr.span)
                        } else {
                            self.copy_alias_type(ty, expr.span)
                        }
                    })?;
                    value
                };
                arguments.push(n::CallArgument {
                    passing: Default::default(),
                    parameter,
                    value,
                    is_default: true,
                });
            }
        }
        let ty = if arguments
            .iter()
            .any(|argument| is_never(&argument.value.ty))
        {
            p(P::Never)
        } else {
            let result = call_binding.substitute(&result).map_err(|e| {
                error(
                    expr.span,
                    format!("dependent call result is not available: {e}"),
                )
            })?;
            self.callable_input_view(&result, expr.span)?
        };
        if let Some(expected) = expected {
            self.require_expression(&ty, expected, expr.span)?;
        }
        // Retain the interface-origin signature for phase validation. The
        // selected projection view above checks inputs/results; IIR closure
        // resolves all retained annotations together without losing origin.
        self.comptime_receiver_arguments(direct, &mut arguments);
        callee.ty = retained_callable;
        let call = n::Expr {
            ty,
            source: source(expr.span),
            kind: K::Call {
                callee: Box::new(callee),
                arguments,
                parameter_bindings: declarations
                    .map(|params| params.iter().map(|p| local(p.hir_id)).collect())
                    .unwrap_or_default(),
            },
        };
        let call = self.freeze_staged_call_inputs(call, expr)?;
        self.stage_collected_call(call, &routes, offset, expr.span)
    }

    fn closure(
        &mut self,
        expr: &Expr<'_>,
        parameters: &[hir::expr::ClosureParam<'_>],
        annotation: Option<&Expr<'_>>,
        body_id: hir::BodyId,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        let body = self
            .package
            .body(body_id)
            .ok_or_else(|| error(expr.span, "missing lambda body"))?;
        self.closure_body(
            expr, parameters, annotation, body.value, expected, partial, false,
        )
        .map(|(value, _)| value)
    }

    fn closure_body(
        &mut self,
        expr: &Expr<'_>,
        parameters: &[hir::expr::ClosureParam<'_>],
        annotation: Option<&Expr<'_>>,
        body: &Expr<'_>,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
        tactic: bool,
    ) -> Result<(n::Expr, bool), Error> {
        let expected_shape = expected
            .map(|ty| self.structural_type(ty, expr.span))
            .transpose()?;
        let expected_signature = match expected_shape.as_ref() {
            Some(Term::Closure { signature, .. }) => Some(signature.as_ref()),
            other => other,
        };
        let comptime = self.in_required_comptime || self.in_staged_input || (!self.in_code_body && self.signature_owner
            .and_then(|owner| self.package.item(owner))
            .is_some_and(|item| matches!(&item.kind, ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig) if sig.modifiers.is_comptime)));
        let contextual = match expected_signature.map(Term::callable_signature) {
            Some(Term::Function { parameters, result }) => Some((parameters, result)),
            Some(_) => {
                return Err(incompatible_context(
                    expr.span,
                    "lambda target is not callable",
                ));
            }
            None => None,
        };
        if expected_signature.is_some_and(|ty| ty.is_comptime_callable() != comptime)
            || partial.is_some_and(|context| context.comptime != comptime)
        {
            return Err(error(
                expr.span,
                "lambda target requires a different execution phase",
            ));
        }
        if let Some((expected, _)) = contextual {
            if expected.len() != parameters.len() {
                return Err(error(
                    expr.span,
                    "lambda parameter count does not match target",
                ));
            }
        }
        if partial.is_some_and(|context| context.parameters.len() != parameters.len()) {
            return Err(error(
                expr.span,
                "lambda parameter count does not match target",
            ));
        }
        let saved_locals = self.locals.clone();
        let saved_result = self.result.clone();
        let saved_partial_return = self.partial_return_context.take();
        let saved_optional_returns = std::mem::take(&mut self.pending_optional_returns);
        let saved_error_returns = std::mem::take(&mut self.pending_error_returns);
        let saved_return_sites = std::mem::take(&mut self.inferred_return_sites);
        let saved_tactic = self.tactic_frame.take();
        // A callback infers its own result from both its tail and explicit
        // returns, even when the callback value is being probed by a parent.
        let saved_branch_probe = std::mem::replace(&mut self.probing_optional_branch, false);
        if tactic {
            self.tactic_frame = Some((
                source(body.span),
                parameters.iter().map(|p| p.hir_id).collect(),
                false,
            ));
        }
        let saved_returns = self.return_observations.take();
        let saved_loops = std::mem::take(&mut self.loops);
        self.captures.push(CaptureFrame {
            comptime,
            outer: self.locals.keys().copied().collect(),
            values: Vec::new(),
        });
        let checked = (|| {
            let mut parameter_types = Vec::new();
            for (index, parameter) in parameters.iter().enumerate() {
                if !matches!(parameter.pat.kind, hir::PatternKind::Binding(_, _, None)) {
                    return Err(error(
                        parameter.span,
                        "lambda parameters must be anchored identifiers",
                    ));
                }
                let context_type = contextual
                    .map(|(parameters, _)| parameters[index].ty.substitute_partial(&[]))
                    .transpose()
                    .map_err(|e| error(parameter.span, e.to_string()))?
                    .or_else(|| partial.and_then(|context| context.parameters[index].clone()))
                    .map(|ty| self.callable_input_view(&ty, parameter.span))
                    .transpose()?;
                let ty = if let Some(annotation) = parameter.ty {
                    let ty = self.type_expr(annotation)?;
                    if let Some(context_type) = context_type.as_ref() {
                        self.require(&ty, context_type, parameter.span)?;
                    }
                    ty
                } else {
                    context_type.ok_or_else(|| {
                        context_error(
                            parameter.span,
                            "lambda parameter needs an annotation or a callable target",
                        )
                    })?
                };
                self.locals.insert(local(parameter.hir_id), ty.clone());
                parameter_types.push(Parameter {
                    sort: Sort::Runtime,
                    ty: ty
                        .abstract_parameters(&[])
                        .map_err(|e| error(parameter.span, e.to_string()))?,
                });
            }
            let contextual_result = contextual
                .map(|(_, result)| result.substitute_partial(&[]))
                .transpose()
                .map_err(|e| error(expr.span, e.to_string()))?
                .or_else(|| partial.and_then(|context| context.result.clone()))
                .map(|ty| self.callable_input_view(&ty, expr.span))
                .transpose()?;
            let result_type = if let Some(annotation) = annotation {
                let ty = self.type_expr(annotation)?;
                if let Some(expected) = contextual_result.as_ref() {
                    self.require(&ty, expected, expr.span)?;
                }
                Some(ty)
            } else {
                contextual_result
            };
            self.result = result_type.clone();
            self.return_observations = Some(ReturnObservations::Infer(Vec::new()));
            let result_context = partial.and_then(|context| context.result_context.as_deref());
            let mut body = if result_type.is_none() {
                let mut return_hint = None;
                let attempted = self.checking_attempt(self.numeric_defaults, true, |checker| {
                    let checked = checker.computation_body_with_input(body, None, result_context);
                    if matches!(&checked, Err(failure) if failure.needs_context) {
                        return_hint = checker
                            .return_observations
                            .as_ref()
                            .and_then(|returns| returns.iter().find(|ty| !is_never(ty)).cloned());
                    }
                    checked
                })?;
                match attempted {
                    Some(body) => body,
                    None => {
                        let hint = return_hint.ok_or_else(|| {
                            context_error(
                                body.span,
                                "callback result needs a tail or explicit return type",
                            )
                        })?;
                        // Retry once with concrete return evidence. The failed
                        // attempt has rolled back captures, pending returns and
                        // local state; only the completed type crosses attempts.
                        self.computation_body_with_input(body, Some(&hint), result_context)?
                    }
                }
            } else {
                self.computation_body_with_input(body, result_type.as_ref(), result_context)?
            };
            let returns = self.return_observations.as_ref().unwrap().clone();
            let mut result = result_type.unwrap_or_else(|| {
                if !is_never(&body.ty) {
                    body.ty.clone()
                } else {
                    returns
                        .iter()
                        .find(|ty| !is_never(ty) && !matches!(ty, Term::ErrorResult { result, .. } if is_never(result)))
                        .cloned()
                        .or_else(|| returns.iter().find(|ty| !is_never(ty)).cloned())
                        .unwrap_or_else(|| p(P::Never))
                }
            });
            let value_context = match result_context {
                Some(crate::calls::InputContext::Computation { result, .. }) => {
                    Some(result.as_ref())
                }
                other => other,
            };
            let allowed_errors = match value_context {
                Some(crate::calls::InputContext::ErrorResult { errors, .. }) => {
                    Some(errors.as_slice())
                }
                _ => None,
            };
            let completed_error_returns =
                !self.pending_error_returns.is_empty() || allowed_errors.is_some();
            if completed_error_returns {
                result =
                    self.complete_error_returns(&mut body, result, allowed_errors, expr.span)?;
            }
            if !self.pending_optional_returns.is_empty() {
                let value_result = if let Term::Computation { result, .. } = &result {
                    result.as_ref()
                } else {
                    &result
                };
                self.optional_payload(value_result, expr.span)?;
                let mut remaining = self
                    .pending_optional_returns
                    .keys()
                    .copied()
                    .collect::<BTreeSet<_>>();
                nir::visit::expression_mut(&mut body, &mut 100_000, |node, _| {
                    if matches!(node.kind, K::Return(None)) {
                        if let Some(empty_source) = self
                            .pending_optional_returns
                            .get(&(node.source.start, node.source.end))
                        {
                            remaining.remove(&(node.source.start, node.source.end));
                            node.kind = K::Return(Some(Box::new(n::Expr {
                                source: *empty_source,
                                ty: value_result.clone(),
                                kind: K::OptionalAbsent,
                            })));
                        }
                    }
                    Ok(())
                })
                .map_err(|message| error(expr.span, message))?;
                if !remaining.is_empty() {
                    return Err(error(expr.span, "unresolved optional return site"));
                }
            }
            self.require(&body.ty, &result, expr.span)?;
            for actual in returns.iter() {
                let value_result = if let Term::Computation { result, .. } = &result {
                    result.as_ref()
                } else {
                    &result
                };
                if !completed_error_returns {
                    self.require(actual, value_result, expr.span)?;
                }
            }
            let captures = self.captures.last().unwrap().values.clone();
            let signature = Term::Function {
                parameters: parameter_types,
                result: Box::new(
                    result
                        .abstract_parameters(&[])
                        .map_err(|e| error(expr.span, e.to_string()))?,
                ),
            }
            .with_comptime_phase(comptime);
            let ty = if captures.is_empty() {
                signature
            } else {
                Term::Closure {
                    environment: captures.iter().map(|capture| capture.ty.clone()).collect(),
                    signature: Box::new(signature),
                }
            };
            if let Some(expected) = expected {
                self.require(&ty, expected, expr.span)?;
            }
            Ok(n::Expr {
                ty,
                source: source(expr.span),
                kind: K::Closure {
                    parameters: parameters.iter().map(|p| local(p.hir_id)).collect(),
                    captures,
                    body: std::sync::Arc::new(body),
                },
            })
        })();
        self.captures.pop();
        self.locals = saved_locals;
        self.result = saved_result;
        self.partial_return_context = saved_partial_return;
        self.pending_optional_returns = saved_optional_returns;
        self.pending_error_returns = saved_error_returns;
        self.inferred_return_sites = saved_return_sites;
        let used = self.tactic_frame.as_ref().is_some_and(|(_, _, used)| *used);
        self.tactic_frame = saved_tactic;
        self.probing_optional_branch = saved_branch_probe;
        self.return_observations = saved_returns;
        self.loops = saved_loops;
        checked.map(|value| (value, used)).map_err(|mut failure| {
            if !tactic {
                failure.needs_tactic = false;
            }
            failure
        })
    }

    /// Let either operand supply a type before committing numeric defaults.
    /// Incomplete attempts use the generic-call transaction to restore scope.
    fn peer_operands(
        &mut self,
        left: &Expr<'_>,
        right: &Expr<'_>,
        expected: Option<&Term>,
        fallback: Option<&Term>,
    ) -> Result<(n::Expr, n::Expr), Error> {
        let a = self.argument_attempt(left, expected, None)?;
        self.peer_operands_with_left(left, right, expected, fallback, a)
    }
    fn peer_operands_with_left(
        &mut self,
        left: &Expr<'_>,
        right: &Expr<'_>,
        expected: Option<&Term>,
        fallback: Option<&Term>,
        a: Option<n::Expr>,
    ) -> Result<(n::Expr, n::Expr), Error> {
        self.peer_operands_with_ready(left, right, expected, fallback, a, None)
    }
    fn peer_operands_with_ready(
        &mut self,
        left: &Expr<'_>,
        right: &Expr<'_>,
        expected: Option<&Term>,
        fallback: Option<&Term>,
        mut a: Option<n::Expr>,
        mut b: Option<n::Expr>,
    ) -> Result<(n::Expr, n::Expr), Error> {
        let defaults = self.numeric_defaults;
        let context = expected.or_else(|| a.as_ref().filter(|a| !is_never(&a.ty)).map(|a| &a.ty));
        if b.is_none() {
            b = self.argument_attempt(right, context, None)?;
        }
        if a.is_none() {
            if let Some(b) = b.as_ref().filter(|b| !is_never(&b.ty)) {
                a = self.argument_attempt(left, Some(&b.ty), None)?;
            }
        }
        if b.is_none() {
            if let Some(a) = a.as_ref().filter(|a| !is_never(&a.ty)) {
                b = self.argument_attempt(right, Some(&a.ty), None)?;
            }
        }
        if a.is_none() || b.is_none() {
            if !defaults {
                return Err(context_error(
                    left.span,
                    "numeric operands await a shared type context",
                ));
            }
            if a.is_none() {
                let context = expected
                    .or_else(|| b.as_ref().filter(|b| !is_never(&b.ty)).map(|b| &b.ty))
                    .or(fallback);
                a = Some(self.expr(left, context)?);
            }
            if b.is_none() {
                let context = expected
                    .or_else(|| a.as_ref().filter(|a| !is_never(&a.ty)).map(|a| &a.ty))
                    .or(fallback);
                b = Some(self.expr(right, context)?);
            }
        }
        Ok((a.unwrap(), b.unwrap()))
    }

    fn float_associated_constant(
        &self,
        ty: &Term,
        name: &str,
        span: Span,
    ) -> Result<Option<n::Expr>, Error> {
        if !matches!(name, "NaN" | "Infinity") {
            return Ok(None);
        }
        // Membership needs positive type-equality evidence. An unresolved
        // opaque head must remain available to ordinary member lookup, not
        // become a failed attempt to obtain a structural float view.
        let ty = if matches!(ty, Term::Primitive(P::F16 | P::Bf16 | P::F32 | P::F64)) {
            ty.clone()
        } else {
            let Some(ty) = [p(P::F16), p(P::Bf16), p(P::F32), p(P::F64)]
                .into_iter()
                .find(|candidate| {
                    matches!(
                        self.context.prove(
                            crate::ptc::Formula::equal(ty.clone(), candidate.clone()),
                            crate::ptc::Budget::default(),
                        ),
                        crate::ptc::Outcome::Entailed(_)
                    )
                })
            else {
                return Ok(None);
            };
            ty
        };
        let literal = match (&ty, name) {
            (Term::Primitive(P::F16), "NaN") => n::Literal::Float16(0x7e00),
            (Term::Primitive(P::F16), "Infinity") => n::Literal::Float16(0x7c00),
            (Term::Primitive(P::Bf16), "NaN") => n::Literal::BFloat16(0x7fc0),
            (Term::Primitive(P::Bf16), "Infinity") => n::Literal::BFloat16(0x7f80),
            (Term::Primitive(P::F32), "NaN") => n::Literal::Float32(0x7fc00000),
            (Term::Primitive(P::F32), "Infinity") => n::Literal::Float32(0x7f800000),
            (Term::Primitive(P::F64), "NaN") => n::Literal::Float64(0x7ff8000000000000),
            (Term::Primitive(P::F64), "Infinity") => n::Literal::Float64(0x7ff0000000000000),
            _ => return Ok(None),
        };
        Ok(Some(n::Expr {
            ty,
            source: source(span),
            kind: K::Literal(literal),
        }))
    }

    fn native_equality_shape(&self, shape: &Term) -> bool {
        matches!(shape, Term::Primitive(p) if matches!(p,
            P::Type | P::Integer | P::BitVec(_) | P::Bool | P::Char | P::Str | P::Symbol | P::Float | P::F16 | P::Bf16 | P::F32 | P::F64 | P::Usize | P::Isize
        ) || self.integer_layout(*p).is_some())
            || self.proved_primitive_family(
                shape,
                &[
                    P::Bool,
                    P::Usize,
                    P::Isize,
                    P::Char,
                    P::Str,
                    P::Symbol,
                    P::F16,
                    P::Bf16,
                    P::F32,
                    P::F64,
                    P::U8,
                    P::U16,
                    P::U32,
                    P::U64,
                    P::U128,
                    P::I8,
                    P::I16,
                    P::I32,
                    P::I64,
                    P::I128,
                ],
            )
    }

    fn equality_expression(
        &self,
        left: n::Expr,
        right: n::Expr,
        shape: &Term,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let ty = p(if is_never(&left.ty) || is_never(&right.ty) {
            P::Never
        } else {
            P::Bool
        });
        let left = Box::new(left);
        let right = Box::new(right);
        let kind = match shape {
            Term::Primitive(P::Type) => K::TypeEqual(left, right),
            Term::Primitive(P::Integer) => K::IntegerBinary {
                op: n::IntegerOp::Equal,
                left,
                right,
            },
            Term::Primitive(P::BitVec(_)) => K::BitEqual(left, right),
            Term::Primitive(primitive)
                if matches!(
                    primitive,
                    P::Bool
                        | P::Char
                        | P::Str
                        | P::Symbol
                        | P::Float
                        | P::F16
                        | P::Bf16
                        | P::F32
                        | P::F64
                        | P::Usize
                        | P::Isize
                ) || self.integer_layout(*primitive).is_some() =>
            {
                K::ScalarEqual(left, right)
            }
            _ if self.native_equality_shape(shape) => K::ScalarEqual(left, right),
            _ => {
                return Err(error(
                    span,
                    "equality protocol for this type is not implemented",
                ));
            }
        };
        Ok(n::Expr {
            ty,
            source: source(span),
            kind,
        })
    }

    fn known_pattern_type_value(&mut self, value: &n::Expr) -> Option<Term> {
        if value.ty != p(P::Type) {
            return None;
        }
        let mut budget = comptime_runtime::Budget {
            steps: usize::try_from(self.required_steps).unwrap_or(usize::MAX),
            max_limbs: 4096,
        };
        let before = budget.steps;
        let value = self.known_normal_input(value, 0, &mut budget);
        self.required_steps -= (before - budget.steps) as u64;
        value.filter(|value| value.sort() == Sort::Type)
    }

    fn pattern(
        &mut self,
        pattern: &hir::pattern::Pattern<'_>,
        ty: &Term,
    ) -> Result<n::Pattern, Error> {
        self.pattern_observed(pattern, ty, None)
    }
    fn pattern_observed(
        &mut self,
        pattern: &hir::pattern::Pattern<'_>,
        ty: &Term,
        observed: Option<&Term>,
    ) -> Result<n::Pattern, Error> {
        use hir::pattern::PatternKind as HP;
        match &pattern.kind {
            HP::CodeSplice(operand, outputs) => {
                self.record_quote_hole(pattern.span)?;
                self.connect_pattern_code(operand, outputs, ty, pattern.span)
            }
            HP::ErrorOk(inner) => {
                let Term::ErrorResult { result, .. } = ty else {
                    return Err(error(
                        pattern.span,
                        "success pattern requires an error-qualified input",
                    ));
                };
                Ok(n::Pattern::ErrorResult {
                    label: None,
                    inner: Box::new(self.pattern(inner, result)?),
                })
            }
            HP::ErrorErr(inner) => {
                let Term::ErrorResult { errors, .. } = ty else {
                    return Err(error(
                        pattern.span,
                        "error pattern requires an error-qualified input",
                    ));
                };
                if matches!(inner.kind, HP::Wild) {
                    self.required_steps = self
                        .required_steps
                        .checked_sub(errors.len() as u64)
                        .ok_or_else(|| error(pattern.span, "error pattern budget exhausted"))?;
                    return Ok(n::Pattern::Or(
                        errors
                            .iter()
                            .map(|label| n::Pattern::ErrorResult {
                                label: Some(label.clone()),
                                inner: Box::new(n::Pattern::Wild),
                            })
                            .collect(),
                    ));
                }
                let label = if errors.len() == 1 {
                    errors[0].clone()
                } else {
                    let mut selected = *inner;
                    loop {
                        match &selected.kind {
                            HP::Guard(first, _) | HP::Then(first, _, _) => selected = first,
                            HP::Binding(_, _, Some(first)) => selected = first,
                            _ => break,
                        }
                    }
                    let callee = match &selected.kind {
                        HP::Const(value) => *value,
                        HP::AppTuple(head, _) | HP::Struct(head, _, _) => {
                            let HP::Const(value) = &head.kind else {
                                return Err(error(
                                    pattern.span,
                                    "error payload requires a type-identifying pattern",
                                ));
                            };
                            *value
                        }
                        _ => {
                            return Err(error(
                                pattern.span,
                                "multiple error types require a type-identifying payload pattern",
                            ));
                        }
                    };
                    let label = self
                        .variant_target(callee, None)?
                        .ok_or_else(|| {
                            error(
                                pattern.span,
                                "multiple error types require a qualified variant pattern",
                            )
                        })?
                        .0;
                    errors
                        .iter()
                        .find(|allowed| self.require(&label, allowed, pattern.span).is_ok())
                        .cloned()
                        .ok_or_else(|| {
                            error(
                                pattern.span,
                                "error pattern type is absent from the input error row",
                            )
                        })?
                };
                Ok(n::Pattern::ErrorResult {
                    label: Some(label.clone()),
                    inner: Box::new(self.pattern(inner, &label)?),
                })
            }
            HP::OptionSome(inner) => {
                let payload = self.optional_payload(ty, pattern.span)?;
                Ok(n::Pattern::OptionalPresent(Box::new(
                    self.pattern(inner, &payload)?,
                )))
            }
            HP::OptionNull => {
                self.optional_payload(ty, pattern.span)?;
                Ok(n::Pattern::OptionalAbsent)
            }

            HP::Not(inner) => {
                let saved = self.locals.clone();
                let checked = self.pattern(inner, ty);
                // Bindings exist only while checking the failed candidate.
                // Success of its negation exports none of them.
                self.locals = saved;
                let checked = checked?;
                if let Some(mask) = boolean_pattern_coverage(&checked) {
                    return Ok(match mask {
                        0 => n::Pattern::Wild,
                        1 => n::Pattern::Literal(n::Literal::Bool(true)),
                        2 => n::Pattern::Literal(n::Literal::Bool(false)),
                        _ => n::Pattern::Or(Vec::new()),
                    });
                }
                if !self
                    .negated_pattern_domain(
                        ty,
                        &mut 100_000,
                        0,
                        &mut std::collections::HashMap::new(),
                    )
                    .map_err(|message| error(pattern.span, message))?
                {
                    return Err(error(
                        pattern.span,
                        "negated pattern elaboration for this domain is not implemented",
                    ));
                }
                // An executable candidate must run normally, including guard
                // effects and rollback. Only its final match result is inverted.
                let index = self.next_temporary_binding;
                self.next_temporary_binding = index.checked_add(1).ok_or_else(|| {
                    error(pattern.span, "pattern temporary binding space exhausted")
                })?;
                let binding = n::LocalId {
                    owner: u32::MAX,
                    index,
                };
                let boolean = |value| n::Expr {
                    ty: p(P::Bool),
                    // Distinct synthetic arm boundaries anchored in the `not`
                    // prefix. Nested negations have their own prefix anchors.
                    source: n::SourceRange {
                        start: pattern.span.lo().0,
                        end: pattern.span.lo().0 + u32::from(value),
                    },
                    kind: K::Literal(n::Literal::Bool(value)),
                };
                // This alias is needed only by the candidate's condition. It
                // must not become an exported binding of an `or` alternative.
                Ok(n::Pattern::Observe {
                    binding,
                    inner: Box::new(n::Pattern::Guard {
                        pattern: Box::new(n::Pattern::Wild),
                        condition: Box::new(n::Expr {
                            ty: p(P::Bool),
                            source: source(pattern.span),
                            kind: K::Match {
                                value: Box::new(n::Expr {
                                    ty: ty.clone(),
                                    source: n::SourceRange {
                                        start: pattern.span.lo().0 + 1,
                                        end: pattern.span.lo().0 + 1,
                                    },
                                    kind: K::Local(binding),
                                }),
                                arms: vec![
                                    n::MatchArm {
                                        pattern: checked,
                                        guard: None,
                                        body: boolean(false),
                                    },
                                    n::MatchArm {
                                        pattern: n::Pattern::Wild,
                                        guard: None,
                                        body: boolean(true),
                                    },
                                ],
                            },
                        }),
                    }),
                })
            }
            HP::Range(lower, upper, bound) => {
                if lower.is_none() && upper.is_none() {
                    return Ok(n::Pattern::Wild);
                }
                let shape = self.structural_type(ty, pattern.span)?;
                if !matches!(&shape, Term::Primitive(primitive) if matches!(primitive, P::Integer | P::Float | P::F32 | P::F64 | P::Char) || self.integer_layout(*primitive).is_some())
                {
                    return Err(error(
                        pattern.span,
                        "range pattern ordering protocol for this type is not implemented",
                    ));
                }
                let mut fresh = || {
                    let index = self.next_temporary_binding;
                    self.next_temporary_binding = index.checked_add(1).ok_or_else(|| {
                        error(pattern.span, "pattern temporary binding space exhausted")
                    })?;
                    Ok::<_, Error>(n::LocalId {
                        owner: u32::MAX,
                        index,
                    })
                };
                let observed = fresh()?;
                let lower_binding = fresh()?;
                let upper_binding = fresh()?;
                let local_expr = |binding| n::Expr {
                    ty: ty.clone(),
                    source: source(pattern.span),
                    kind: K::Local(binding),
                };
                let mut statements = Vec::new();
                let mut checks = Vec::new();
                let mut diverges = false;
                for (endpoint, binding, upper_end) in [
                    (*lower, lower_binding, false),
                    (*upper, upper_binding, true),
                ] {
                    let Some(endpoint) = endpoint else { continue };
                    let endpoint = self.expr(endpoint, Some(ty))?;
                    diverges |= is_never(&endpoint.ty);
                    statements.push(n::Expr {
                        ty: if is_never(&endpoint.ty) {
                            p(P::Never)
                        } else {
                            p(P::Void)
                        },
                        source: endpoint.source,
                        kind: K::Let {
                            binding,
                            value: Box::new(endpoint),
                        },
                    });
                    let exclusive =
                        upper_end && matches!(bound, hir::pattern::BoundType::Exclusive);
                    let (left, right) = if upper_end {
                        (local_expr(observed), local_expr(binding))
                    } else {
                        (local_expr(binding), local_expr(observed))
                    };
                    let kind = if shape == p(P::Integer) {
                        K::IntegerBinary {
                            op: if exclusive {
                                n::IntegerOp::Less
                            } else {
                                n::IntegerOp::LessEqual
                            },
                            left: Box::new(left),
                            right: Box::new(right),
                        }
                    } else if exclusive {
                        // NaN is unordered: `not (upper <= value)` is not
                        // equivalent to the required ordered `value < upper`.
                        K::ScalarLess(Box::new(left), Box::new(right))
                    } else {
                        K::ScalarLessEqual(Box::new(left), Box::new(right))
                    };
                    let check = n::Expr {
                        ty: p(P::Bool),
                        source: source(pattern.span),
                        kind,
                    };
                    checks.push(check);
                }
                let first = checks.remove(0);
                let result = if let Some(second) = checks.pop() {
                    n::Expr {
                        ty: p(P::Bool),
                        source: source(pattern.span),
                        kind: K::And(Box::new(first), Box::new(second)),
                    }
                } else {
                    first
                };
                Ok(n::Pattern::Observe {
                    binding: observed,
                    inner: Box::new(n::Pattern::Guard {
                        pattern: Box::new(n::Pattern::Wild),
                        condition: Box::new(n::Expr {
                            ty: p(if diverges { P::Never } else { P::Bool }),
                            source: source(pattern.span),
                            kind: K::Block {
                                statements,
                                result: Some(Box::new(result)),
                            },
                        }),
                    }),
                })
            }
            HP::BitVec { radix, segments } => {
                let shape = self.structural_type(ty, pattern.span)?;
                let Term::Primitive(P::BitVec(width)) = shape else {
                    return Err(error(
                        pattern.span,
                        "bit-vector patterns require a BitVec subject",
                    ));
                };
                let binding = local(pattern.hir_id);
                let mut remaining = width;
                let mut values = Vec::new();
                let mut patterns = Vec::new();
                for segment in segments {
                    let (segment_width, segment_ty, checked) = match segment {
                        hir::pattern::BitSegment::Literal { spelling, span } => {
                            let digits = spelling.bytes().filter(|c| *c != b'_').count();
                            let bits_per_digit = match radix {
                                2 => 1usize,
                                8 => 3,
                                16 => 4,
                                _ => return Err(error(*span, "invalid bit-vector radix")),
                            };
                            let segment_width = digits
                                .checked_mul(bits_per_digit)
                                .ok_or_else(|| error(*span, "bit-vector pattern width overflow"))?;
                            let prefix = match radix {
                                2 => "0b",
                                8 => "0o",
                                _ => "0x",
                            };
                            let mut budget = comptime_runtime::Budget::default();
                            let integer = comptime_runtime::Integer::parse_literal(
                                &format!("{prefix}{spelling}"),
                                &mut budget,
                            )
                            .map_err(|_| {
                                error(*span, "invalid digit in bit-vector literal segment")
                            })?;
                            let bits = integer
                                .to_bitvec(segment_width, &mut budget)
                                .map_err(|e| error(*span, e.to_string()))?;
                            (
                                segment_width,
                                p(P::BitVec(segment_width)),
                                n::Pattern::Literal(n::Literal::BitVec {
                                    width: segment_width,
                                    words: bits.words().to_vec(),
                                }),
                            )
                        }
                        hir::pattern::BitSegment::Typed { pattern: inner, ty } => {
                            let ty = self.type_expr(ty)?;
                            let shape = self.structural_type(&ty, inner.span)?;
                            let width=match shape {
                                Term::Primitive(P::BitVec(width))=>width,
                                Term::Primitive(primitive)=>primitive.unrestricted_scalar_width_in(self.target).map(|width|width as usize)
                                    .ok_or_else(||error(inner.span,"bit-vector segment type requires a supported unrestricted scalar or BitVec representation"))?,
                                _=>return Err(error(inner.span,"bit-vector segment type has no supported representation")),
                            };
                            let checked = self.pattern(inner, &ty)?;
                            (width, ty, checked)
                        }
                        hir::pattern::BitSegment::Computed(expr) => {
                            let value = self.required_expression(expr, None)?;
                            let comptime_interpretor::Value::BitVec(bits) = value else {
                                return Err(error(
                                    expr.span,
                                    "computed bit-vector segment requires a concrete BitVec value",
                                ));
                            };
                            let width = bits.width();
                            (
                                width,
                                p(P::BitVec(width)),
                                n::Pattern::Literal(n::Literal::BitVec {
                                    width,
                                    words: bits.words().to_vec(),
                                }),
                            )
                        }
                    };
                    let start = remaining.checked_sub(segment_width).ok_or_else(|| {
                        error(
                            pattern.span,
                            "bit-vector pattern width differs from its subject",
                        )
                    })?;
                    let slice = n::Expr {
                        ty: p(P::BitVec(segment_width)),
                        source: source(pattern.span),
                        kind: K::BitSlice {
                            value: Box::new(n::Expr {
                                ty: shape.clone(),
                                source: source(pattern.span),
                                kind: K::Local(binding),
                            }),
                            start,
                            end: remaining,
                        },
                    };
                    let value = if self.structural_type(&segment_ty, pattern.span)? == slice.ty {
                        slice
                    } else {
                        n::Expr {
                            ty: segment_ty,
                            source: source(pattern.span),
                            kind: K::BitConvert(Box::new(slice)),
                        }
                    };
                    remaining = start;
                    values.push(value);
                    patterns.push(checked);
                }
                if remaining != 0 {
                    return Err(error(
                        pattern.span,
                        "bit-vector pattern width differs from its subject",
                    ));
                }
                let mut checked = n::Pattern::Wild;
                for (value, next) in values.into_iter().zip(patterns) {
                    checked = n::Pattern::Then {
                        first: Box::new(checked),
                        value: Box::new(value),
                        next: Box::new(next),
                    };
                }
                Ok(n::Pattern::Observe {
                    binding,
                    inner: Box::new(checked),
                })
            }
            HP::Then(first, value, next) => {
                let first = self.pattern_observed(first, ty, observed)?;
                let (value, next) = self.under_observed_pattern_type_facts(
                    &first,
                    observed,
                    value.span,
                    |checker| {
                        let value = checker.expr(value, None)?;
                        let observed = checker.known_pattern_type_value(&value);
                        let next = checker.pattern_observed(next, &value.ty, observed.as_ref())?;
                        Ok((value, next))
                    },
                )?;
                self.lower_computed_pattern(first, value, next, pattern.span)
            }
            HP::Guard(pattern, condition) => {
                let pattern = self.pattern_observed(pattern, ty, observed)?;
                let condition = self.under_observed_pattern_type_facts(
                    &pattern,
                    observed,
                    condition.span,
                    |checker| checker.expr(condition, Some(&p(P::Bool))),
                )?;
                Ok(n::Pattern::Guard {
                    pattern: Box::new(pattern),
                    condition: Box::new(condition),
                })
            }
            HP::List(elements, rest) => self.list_pattern(pattern, elements, *rest, ty),
            HP::NormalForm(head, arguments) => {
                if self.structural_type(ty, pattern.span)? != p(P::Type) {
                    return Err(error(
                        pattern.span,
                        "normal-form patterns require a Type value",
                    ));
                }
                let H::ResolvedDef { def_id, .. } = head.kind else {
                    return Err(error(
                        head.span,
                        "normal-form pattern head requires a resolved declaration",
                    ));
                };
                let owner = self
                    .package
                    .definition_owner(def_id)
                    .ok_or_else(|| error(head.span, "missing normal-form head"))?;
                let Some(ItemKind::Fn(sig, _)) = self.package.item(owner).map(|item| &item.kind)
                else {
                    return Err(error(
                        head.span,
                        "pattern head is not a normal-form definition",
                    ));
                };
                if !sig.modifiers.is_normal_form_head {
                    return Err(error(
                        head.span,
                        "pattern head is not a normal-form definition",
                    ));
                }
                self.ensure_signature(owner, head.span)?;
                let parameters = self.normal_parameters[&owner].clone();
                if arguments.len() != parameters.len() {
                    return Err(error(
                        pattern.span,
                        "normal-form pattern argument count mismatch",
                    ));
                }
                // Route names first, then elaborate in declaration order: a
                // later parameter's type may depend on an earlier Type input
                // even when the named patterns were written in reverse order.
                let mut routed = vec![None; parameters.len()];
                let mut positional = 0;
                for argument in *arguments {
                    let slot = if let Some(name) = argument.name {
                        sig.clause_params
                            .iter()
                            .position(|parameter| parameter.name.name == name.name)
                            .ok_or_else(|| {
                                error(name.span, "unknown normal-form pattern argument")
                            })?
                    } else {
                        while positional < routed.len() && routed[positional].is_some() {
                            positional += 1;
                        }
                        let slot = positional;
                        positional += 1;
                        slot
                    };
                    if slot >= routed.len() || routed[slot].replace(&argument.pattern).is_some() {
                        return Err(error(
                            argument.pattern.span,
                            "duplicate normal-form pattern argument",
                        ));
                    }
                }
                let head_id = declaration(self.package, &self.package_identities, owner);
                let observed_arguments = match observed {
                    Some(Term::NormalForm {
                        head, arguments, ..
                    }) if *head == head_id && arguments.len() == parameters.len() => {
                        Some(arguments)
                    }
                    _ => None,
                };
                let mut fields = Vec::with_capacity(parameters.len());
                let mut known_parameters = Vec::new();
                let mut known_types = Vec::new();
                for (slot, (argument, (parameter, _, declared_ty))) in
                    routed.into_iter().zip(&parameters).enumerate()
                {
                    let argument = argument.expect("arity and routing cover every parameter");
                    let ty = declared_ty
                        .instantiate_parameters(&known_parameters, &known_types)
                        .map_err(|failure| error(argument.span, failure.to_string()))?;
                    let dependent = parameters
                        .iter()
                        .any(|(parameter, _, _)| ty.mentions_rigid(*parameter));
                    let observed_type = if ty == p(P::Type) {
                        observed_arguments
                            .and_then(|arguments| arguments.get(slot))
                            .filter(|value| value.sort() == Sort::Type)
                    } else {
                        None
                    };
                    let field = if dependent {
                        if matches!(argument.kind, HP::Wild) {
                            n::Pattern::Wild
                        } else {
                            return Err(error(
                                argument.span,
                                "dependent normal-form value patterns require contextual pattern elaboration not yet implemented",
                            ));
                        }
                    } else {
                        self.pattern_observed(argument, &ty, observed_type)?
                    };
                    if ty == p(P::Type) {
                        let mut evidence = matched_type_pattern_value(
                            &field,
                            &mut self.required_steps,
                            argument.span,
                            0,
                        )?
                        .cloned();
                        if evidence.is_none() {
                            evidence = self.known_type_equality_pattern(&field, argument.span)?;
                        }
                        if let Some(value) = evidence.or_else(|| observed_type.cloned()) {
                            // This evidence belongs only to the successful
                            // candidate; do not install a global solver fact.
                            known_parameters.push(nir::DeclarationParameter {
                                declaration: *parameter,
                                sort: Sort::Type,
                            });
                            known_types.push(value.clone());
                        }
                    }
                    if ty != p(P::Type) {
                        let evidence = self
                            .matched_value_pattern_input(&field, &ty, argument.span, 0)?
                            .or_else(|| {
                                observed_arguments
                                    .and_then(|args| args.get(slot))
                                    .filter(|value| value.sort() == Sort::ComptimeValue)
                                    .cloned()
                            });
                        if let Some(value) = evidence {
                            known_parameters.push(nir::DeclarationParameter {
                                declaration: *parameter,
                                sort: Sort::ComptimeValue,
                            });
                            known_types.push(value);
                        }
                    }
                    fields.push(field);
                }
                Ok(n::Pattern::NormalForm {
                    head: declaration(self.package, &self.package_identities, owner),
                    arguments: fields,
                })
            }
            HP::Tuple(fields) => {
                let shape = self.structural_type(ty, pattern.span)?;
                if shape == p(P::Type) {
                    let observed_fields = observed
                        .and_then(|value| match value {
                            Term::Tuple(elements) => Some(elements.as_slice()),
                            _ => self.bound_tuple_elements(value),
                        })
                        .filter(|elements| elements.len() == fields.len());
                    let fields = fields
                        .iter()
                        .enumerate()
                        .map(|(index, field)| {
                            self.pattern_observed(
                                field,
                                &p(P::Type),
                                observed_fields.map(|elements| &elements[index]),
                            )
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    return Ok(n::Pattern::TypeTuple(fields));
                }
                let Term::Tuple(types) = &shape else {
                    return Err(error(
                        pattern.span,
                        "tuple pattern requires a tuple type; custom pattern view elaboration is not implemented",
                    ));
                };
                if fields.len() != types.len() {
                    return Err(error(pattern.span, "tuple pattern arity mismatch"));
                }
                let fields = fields
                    .iter()
                    .zip(types)
                    .map(|(pattern, ty)| self.pattern(pattern, ty))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(n::Pattern::Tuple(fields))
            }
            HP::Struct(callee, fields, rest) if matches!(callee.kind, HP::Wild) => {
                let shape = self.structural_type(ty, pattern.span)?;
                if let Term::Record(declared) = &shape {
                    if *rest {
                        return Err(error(
                            pattern.span,
                            "record rest binding is not implemented",
                        ));
                    }
                    let mut routed = vec![None; declared.len()];
                    let mut seen = BTreeSet::new();
                    for field in *fields {
                        let slot = declared
                            .iter()
                            .position(|(name, _)| name == field.ident.name.as_str())
                            .ok_or_else(|| error(field.span, "unknown struct pattern field"))?;
                        if !seen.insert(slot) {
                            return Err(error(field.span, "duplicate struct pattern field"));
                        }
                        routed[slot] = Some(field);
                    }
                    // Runtime field traversal follows declaration order, so candidate
                    // locals must become available to guards in that same order.
                    let mut checked = Vec::with_capacity(routed.len());
                    for (slot, field) in routed.into_iter().enumerate() {
                        checked.push(match field {
                            Some(field) => self.pattern(&field.pat, &declared[slot].1)?,
                            None => n::Pattern::Wild,
                        });
                    }
                    return Ok(n::Pattern::AnonymousRecord {
                        names: declared.iter().map(|(name, _)| name.clone()).collect(),
                        fields: checked,
                    });
                }
                let Term::Nominal {
                    declaration,
                    arguments,
                } = &shape
                else {
                    return Err(error(
                        pattern.span,
                        "record pattern requires a struct type; custom pattern view elaboration is not implemented",
                    ));
                };
                let definition = self
                    .structs
                    .get(declaration)
                    .ok_or_else(|| error(pattern.span, "record pattern requires a struct type"))?;
                if *rest {
                    return Err(error(
                        pattern.span,
                        "record rest binding is not implemented",
                    ));
                }
                let declared = definition
                    .instantiate_fields_in_scope(arguments, &self.call_input_frames())
                    .map_err(|e| error(pattern.span, e.to_string()))?;
                let mut routed = vec![None; declared.len()];
                let mut seen = BTreeSet::new();
                for field in *fields {
                    let slot = declared
                        .iter()
                        .position(|(name, _)| name == field.ident.name.as_str())
                        .ok_or_else(|| error(field.span, "unknown struct pattern field"))?;
                    if !seen.insert(slot) {
                        return Err(error(field.span, "duplicate struct pattern field"));
                    }
                    routed[slot] = Some(field);
                }
                // Runtime field traversal follows declaration order, so candidate
                // locals must become available to guards in that same order.
                let mut checked = Vec::with_capacity(routed.len());
                for (slot, field) in routed.into_iter().enumerate() {
                    checked.push(match field {
                        Some(field) => self.pattern(&field.pat, &declared[slot].1)?,
                        None => n::Pattern::Wild,
                    });
                }
                Ok(n::Pattern::Record {
                    declaration: *declaration,
                    fields: checked,
                })
            }
            HP::Struct(callee, fields, rest) => {
                let HP::Const(callee) = &callee.kind else {
                    return Err(error(
                        pattern.span,
                        "named enum pattern requires a variant name",
                    ));
                };
                let Some((variant_type, index)) = self.variant_target(callee, Some(ty))? else {
                    return Err(error(
                        pattern.span,
                        "named enum pattern requires an enum context",
                    ));
                };
                self.require(&variant_type, ty, pattern.span)?;
                let Term::Nominal { declaration, .. } = variant_type else {
                    unreachable!()
                };
                let variant =
                    self.instantiated_variants(&variant_type, pattern.span)?[index].clone();
                let n::VariantShape::Named(names) = &variant.shape else {
                    return Err(error(
                        pattern.span,
                        "enum variant does not have named fields",
                    ));
                };
                if *rest {
                    return Err(error(pattern.span, "enum rest binding is not implemented"));
                }
                let mut routed = vec![None; names.len()];
                let mut seen = BTreeSet::new();
                for field in *fields {
                    let slot = names
                        .iter()
                        .position(|n| n == field.ident.name.as_str())
                        .ok_or_else(|| error(field.span, "unknown enum payload field"))?;
                    if !seen.insert(slot) {
                        return Err(error(field.span, "duplicate enum payload pattern field"));
                    }
                    routed[slot] = Some(field);
                }
                // Runtime field traversal follows declaration order, so candidate
                // locals must become available to guards in that same order.
                let mut checked = Vec::with_capacity(routed.len());
                for (slot, field) in routed.into_iter().enumerate() {
                    checked.push(match field {
                        Some(field) => self.pattern(&field.pat, &variant.fields[slot])?,
                        None => n::Pattern::Wild,
                    });
                }
                Ok(n::Pattern::Variant {
                    declaration,
                    index,
                    fields: checked,
                })
            }
            HP::AppTuple(callee, fields) => {
                let HP::Const(callee) = &callee.kind else {
                    return Err(error(
                        pattern.span,
                        "enum payload pattern requires a variant name",
                    ));
                };
                let Some((variant_type, index)) = self.variant_target(callee, Some(ty))? else {
                    return Err(error(
                        pattern.span,
                        "enum payload pattern requires an enum context",
                    ));
                };
                self.require(&variant_type, ty, pattern.span)?;
                let Term::Nominal { declaration, .. } = variant_type else {
                    unreachable!()
                };
                let variant =
                    self.instantiated_variants(&variant_type, pattern.span)?[index].clone();
                if variant.shape != n::VariantShape::Tuple || fields.len() != variant.fields.len() {
                    return Err(error(pattern.span, "enum payload pattern arity mismatch"));
                }
                let fields = fields
                    .iter()
                    .zip(&variant.fields)
                    .map(|(pattern, ty)| self.pattern(pattern, ty))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(n::Pattern::Variant {
                    declaration,
                    index,
                    fields,
                })
            }
            HP::Wild => Ok(n::Pattern::Wild),
            HP::Binding(hir::common::BindingMode::ByValue, _, inner) => {
                let inner = inner
                    .map(|p| self.pattern_observed(p, ty, observed))
                    .transpose()?
                    .unwrap_or(n::Pattern::Wild);
                let binding = local(pattern.hir_id);
                self.locals.insert(binding, ty.clone());
                Ok(n::Pattern::Binding {
                    binding,
                    inner: Box::new(inner),
                })
            }
            HP::Const(value) => {
                // The parser represents the empty tuple pattern as a unit
                // constant. In a Type subject it observes the tuple view,
                // just as HP::Tuple does; it does not construct a Type value.
                if matches!(value.kind, H::Unit)
                    && self.structural_type(ty, pattern.span)? == p(P::Type)
                {
                    return Ok(n::Pattern::TypeTuple(Vec::new()));
                }
                if let Some(group) = self.enum_group_pattern(value, ty)? {
                    return Ok(group);
                }
                let value = self.expr(value, Some(ty))?;
                if self.structural_type(ty, pattern.span)? == p(P::Type) {
                    let index = self.next_temporary_binding;
                    self.next_temporary_binding = index.checked_add(1).ok_or_else(|| {
                        error(pattern.span, "pattern temporary binding space exhausted")
                    })?;
                    let binding = n::LocalId {
                        owner: u32::MAX,
                        index,
                    };
                    let condition = n::Expr {
                        source: source(pattern.span),
                        ty: if is_never(&value.ty) {
                            p(P::Never)
                        } else {
                            p(P::Bool)
                        },
                        kind: K::TypeEqual(
                            Box::new(n::Expr {
                                source: source(pattern.span),
                                ty: p(P::Type),
                                kind: K::Local(binding),
                            }),
                            Box::new(value),
                        ),
                    };
                    // Equality matching is an observation, not inversion of a
                    // normal-form head. A guard leaves coverage conservative
                    // and uses the existing candidate-local rollback protocol.
                    return Ok(n::Pattern::Guard {
                        pattern: Box::new(n::Pattern::Binding {
                            binding,
                            inner: Box::new(n::Pattern::Wild),
                        }),
                        condition: Box::new(condition),
                    });
                }
                match value.kind {
                    K::Literal(literal) => Ok(n::Pattern::Literal(literal)),
                    K::Unit => Ok(n::Pattern::Unit),
                    K::OptionalAbsent => Ok(n::Pattern::OptionalAbsent),
                    // `()` is parsed as a constant pattern, but with a bound
                    // Tuple<> context its checked value is an empty tuple.
                    // Match its layout directly, just like a nonempty tuple
                    // pattern; it does not require a tuple equality protocol.
                    K::Tuple(fields) if fields.is_empty() => Ok(n::Pattern::Tuple(Vec::new())),
                    K::Variant {
                        declaration,
                        index,
                        fields,
                    } if fields.is_empty() => Ok(n::Pattern::Variant {
                        declaration,
                        index,
                        fields: Vec::new(),
                    }),
                    _ => {
                        let shape = self.structural_type(ty, pattern.span)?;
                        let index = self.next_temporary_binding;
                        self.next_temporary_binding = index.checked_add(1).ok_or_else(|| {
                            error(pattern.span, "pattern temporary binding space exhausted")
                        })?;
                        let binding = n::LocalId {
                            owner: u32::MAX,
                            index,
                        };
                        let observed = n::Expr {
                            ty: ty.clone(),
                            source: source(pattern.span),
                            kind: K::Local(binding),
                        };
                        let condition =
                            self.equality_expression(observed, value, &shape, pattern.span)?;
                        Ok(n::Pattern::Guard {
                            pattern: Box::new(n::Pattern::Binding {
                                binding,
                                inner: Box::new(n::Pattern::Wild),
                            }),
                            condition: Box::new(condition),
                        })
                    }
                }
            }
            HP::Or(patterns) => {
                let saved = self.locals.clone();
                let mut alternatives = Vec::new();
                let mut canonical = None;
                let mut exported = saved.clone();
                for pattern in *patterns {
                    self.locals = saved.clone();
                    let mut checked = self.pattern_observed(pattern, ty, observed)?;
                    let bindings = pattern_bindings(pattern)?;
                    if let Some(first) = &canonical {
                        let first: &BTreeMap<String, n::LocalId> = first;
                        if !first.keys().eq(bindings.keys()) {
                            self.locals = saved;
                            return Err(error(
                                pattern.span,
                                "or-pattern branches must export the same binding names",
                            ));
                        }
                        let mut remap = BTreeMap::new();
                        for (name, binding) in &bindings {
                            let target = first[name];
                            self.require(&self.locals[binding], &exported[&target], pattern.span)?;
                            remap.insert(*binding, target);
                        }
                        checked = n::Pattern::Remap {
                            pattern: Box::new(checked),
                            bindings: remap.into_iter().collect(),
                        };
                    } else {
                        canonical = Some(bindings);
                        exported = self.locals.clone();
                    }
                    alternatives.push(checked);
                }
                self.locals = exported;
                Ok(n::Pattern::Or(alternatives))
            }
            _ => Err(error(
                pattern.span,
                "pattern view/protocol elaboration is not implemented for this pattern",
            )),
        }
    }

    fn match_expr(
        &mut self,
        expr: &Expr<'_>,
        value: &Expr<'_>,
        arms: &[hir::pattern::PatternArm<'_>],
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        self.match_expr_with_input(expr, value, arms, expected, partial, None)
    }
    fn match_arm_pattern(
        &mut self,
        arm: &hir::pattern::PatternArm<'_>,
        value: &n::Expr,
    ) -> Result<n::Pattern, Error> {
        let observed = self.known_pattern_type_value(value);
        let domain = arguments::inference_type(value).unwrap_or(&value.ty);
        let pattern = self.pattern_observed(&arm.pat, domain, observed.as_ref())?;
        let guard = arm
            .guard
            .map(|guard| {
                self.under_matched_pattern_type_facts(&pattern, value, guard.span, |checker| {
                    checker.expr(guard, Some(&p(P::Bool)))
                })
            })
            .transpose()?;
        // Final guard runs after candidate selection, including or-patterns.
        Ok(match guard {
            Some(condition) => n::Pattern::Guard {
                pattern: Box::new(pattern),
                condition: Box::new(condition),
            },
            None => pattern,
        })
    }
    fn match_expr_with_input(
        &mut self,
        expr: &Expr<'_>,
        value: &Expr<'_>,
        arms: &[hir::pattern::PatternArm<'_>],
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
        input_context: Option<&crate::calls::InputContext>,
    ) -> Result<n::Expr, Error> {
        let behavior_pattern = self.checking_open_behavior
            && self
                .inline_selection
                .is_some_and(|(id, _)| id == expr.hir_id);
        let inline_input = self
            .inline_match_input
            .as_ref()
            .filter(|(id, _, _)| *id == expr.hir_id)
            .cloned();
        let staged_match = inline_input.is_some()
            || (self.in_unselected_inline
                && self
                    .inline_selection
                    .is_some_and(|(id, _)| id == expr.hir_id));
        let value = if let Some((_, value, _)) = &inline_input {
            value.clone()
        } else {
            self.expr(value, None)?
        };
        let saved = self.locals.clone();
        let saved_data = inline_input.as_ref().map(|_| self.generator_data.clone());
        let saved_selection = self.inline_selection;
        let saved_suppressed = self.in_unselected_inline;
        let result = (|| {
            let inferred = if expected.is_none() {
                match input_context {
                    Some(context) => self.optional_branch_context(
                        expr.span,
                        context,
                        arms.len(),
                        |checker, index| {
                            let arm = &arms[index];
                            let pattern = checker.match_arm_pattern(arm, &value)?;
                            checker.under_matched_pattern_type_facts(
                                &pattern,
                                &value,
                                arm.body.span,
                                |checker| checker.input_context_expression(arm.body, context),
                            )
                        },
                    )?,
                    None => None,
                }
            } else {
                None
            };
            let expected = expected.or(inferred.as_ref());
            let mut checked = Vec::new();
            let mut result_ty = p(P::Never);
            let mut selected = None;
            let mut selected_bindings = Vec::new();
            for (index, arm) in arms.iter().enumerate() {
                self.locals = saved.clone();
                if let Some(saved_data) = &saved_data {
                    self.generator_data = saved_data.clone();
                }
                let pattern = if behavior_pattern {
                    self.with_staged_input(true, |this| this.match_arm_pattern(arm, &value))?
                } else {
                    self.match_arm_pattern(arm, &value)?
                };
                let staged: BTreeSet<_> = self
                    .locals
                    .keys()
                    .filter(|id| !saved.contains_key(id))
                    .copied()
                    .collect();
                if staged_match && !self.checking_open_behavior {
                    self.check_inline_pattern_availability(&pattern, arm.span)?;
                }
                if let Some((_, _, subject)) = &inline_input {
                    if selected.is_none() {
                        if let Some(bindings) =
                            self.select_inline_pattern(&pattern, subject, arm.body)?
                        {
                            selected = Some(index);
                            selected_bindings =
                                self.materialize_inline_bindings(&bindings, arm.body)?;
                        }
                    }
                    self.inline_selection = Some((expr.hir_id, selected.unwrap_or(usize::MAX)));
                    self.in_unselected_inline = selected != Some(index);
                }
                let arm_context =
                    expected.or_else(|| (!is_never(&result_ty)).then_some(&result_ty));
                let saved_staged = self.open_staged_locals.clone();
                let saved_staged_members = self.open_member_locals.clone();
                if staged_match {
                    self.open_staged_locals.extend(staged);
                }
                let body = self.under_matched_pattern_type_facts(
                    &pattern,
                    &value,
                    arm.body.span,
                    |checker| {
                        checker.with_callee_delivery(
                            expr.hir_id,
                            Some(arm.body.hir_id),
                            |checker| match input_context {
                                Some(input_context) if arm_context.is_none() => {
                                    checker.input_context_expression(arm.body, input_context)
                                }
                                _ => checker.expr_context(arm.body, arm_context, partial),
                            },
                        )
                    },
                );
                self.open_staged_locals = saved_staged;
                self.open_member_locals = saved_staged_members;
                let body = body?;
                if !is_never(&body.ty) {
                    if !is_never(&result_ty) {
                        self.require(&body.ty, &result_ty, arm.span)?;
                    }
                    result_ty = body.ty.clone();
                }
                let mut checked_arm = n::MatchArm {
                    pattern,
                    guard: None,
                    body,
                };
                if inline_input.is_none() {
                    self.stabilize_error_success_binding(&mut checked_arm, &value.ty, arm.span)?;
                }
                checked.push(checked_arm);
            }
            // Desugarings such as `matches` give both synthetic arm bodies the
            // same source span. Cleanup uses body sites as runtime identities:
            // assign distinct generated sites while preserving diagnostics.
            self.required_steps = self
                .required_steps
                .checked_sub(checked.len() as u64)
                .ok_or_else(|| error(expr.span, "match arm source budget exhausted"))?;
            let mut body_sites = BTreeMap::new();
            for arm in &checked {
                *body_sites
                    .entry((arm.body.source.start, arm.body.source.end))
                    .or_insert(0usize) += 1;
            }
            for arm in &mut checked {
                let original = arm.body.source;
                if original == source(expr.span) || body_sites[&(original.start, original.end)] > 1
                {
                    let index = u32::try_from(self.expansion_sources.len())
                        .map_err(|_| error(expr.span, "match arm source space exhausted"))?;
                    self.expansion_sources.push(n::ExpansionSource {
                        original,
                        site: source(expr.span),
                    });
                    arm.body.source = n::SourceRange::generated(index);
                }
            }
            let patterns: Vec<_> = checked
                .iter()
                .filter(|arm| arm.guard.is_none())
                .map(|arm| &arm.pattern)
                .collect();
            let exhaustive = self.patterns_exhaustive(&patterns, &value.ty);
            if exhaustive != crate::coverage::Coverage::Exhaustive {
                return Err(error(
                    expr.span,
                    if exhaustive == crate::coverage::Coverage::Unknown {
                        "match exhaustiveness could not be proved within supported analysis and budget; add an unguarded fallback pattern"
                    } else {
                        "non-exhaustive match; add an unguarded fallback pattern"
                    },
                ));
            }
            if inline_input.is_some() {
                let index =
                    selected.ok_or_else(|| error(expr.span, "inline match has no selected arm"))?;
                let body = checked.swap_remove(index).body;
                return Ok(n::Expr {
                    ty: body.ty.clone(),
                    source: source(expr.span),
                    kind: K::Block {
                        statements: selected_bindings,
                        result: Some(Box::new(body)),
                    },
                });
            }
            if behavior_pattern {
                // Preserve the original typed patterns for branch facts and
                // coverage above; only runtime behavior evidence is abstracted.
                for arm in &mut checked {
                    self.abstract_inline_pattern(&mut arm.pattern, expr.span)?;
                }
            }
            self.stabilize_error_success_patterns(&mut checked, &value.ty, &result_ty, expr.span)?;
            Ok(n::Expr {
                ty: if is_never(&value.ty) {
                    p(P::Never)
                } else {
                    result_ty
                },
                source: source(expr.span),
                kind: K::Match {
                    value: Box::new(value),
                    arms: checked,
                },
            })
        })();
        self.locals = saved;
        if let Some(saved_data) = saved_data {
            self.generator_data = saved_data;
        }
        self.inline_selection = saved_selection;
        self.in_unselected_inline = saved_suppressed;
        result
    }

    /// Elaborate once, then choose execution phase from dependencies, never by
    /// catching an interpreter failure. A closed runtime call remains runtime.
    fn bit_read_index(&mut self, index: &Expr<'_>) -> Result<(n::Expr, Option<usize>), Error> {
        let saved_numeric = std::mem::replace(&mut self.numeric_defaults, true);
        let checked = self.with_staged_input(true, |this| this.expr(index, None));
        self.numeric_defaults = saved_numeric;
        let checked = checked?;
        let shape = self.structural_type(&checked.ty, index.span)?;
        if !matches!(shape, Term::Primitive(P::Integer | P::Never))
            && !matches!(shape, Term::Primitive(p) if self.integer_layout(p).is_some())
        {
            return Err(error(index.span, "BitVec index must produce an integer"));
        }
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let availability = (|| {
            let mut runtime = !nir::free_bindings::collect(&checked, &mut fuel)?.is_empty();
            nir::visit::expression(&checked, &mut fuel, |value, _| {
                if let K::Call { callee, .. } = &value.kind {
                    runtime |= !callee.ty.is_comptime_callable();
                }
                runtime |= matches!(
                    value.kind,
                    K::Return(_)
                        | K::Break(_)
                        | K::Continue(_)
                        | K::AddressOf(_)
                        | K::ReadPointer(_)
                        | K::WritePointer { .. }
                        | K::BitWritePointer { .. }
                        | K::PointerField { .. }
                        | K::PointerAnonymousField { .. }
                        | K::EffectResume(_)
                        | K::EffectHandle { .. }
                        | K::EffectRun { .. }
                        | K::EffectComputation { .. }
                        | K::EffectRequest { .. }
                );
                Ok(())
            })?;
            Ok::<_, String>(runtime)
        })();
        self.required_steps -= (before - fuel) as u64;
        if availability.map_err(|e| error(index.span, e))? {
            return Ok((checked, None));
        }
        let (value, _) = self.required_normal_input(index, checked.clone(), Vec::new())?;
        let value = match value {
            comptime_interpretor::Value::Integer(value) => value,
            comptime_interpretor::Value::UnsignedInteger(value) => {
                comptime_runtime::Integer::from_u128(value)
            }
            comptime_interpretor::Value::SignedInteger(value) => {
                comptime_runtime::Integer::from_i128(value)
            }
            _ => return Err(error(index.span, "BitVec index must produce an integer")),
        };
        if value.is_negative() {
            return Err(error(index.span, "BitVec index must be nonnegative"));
        }
        let position = value
            .to_u128()
            .ok()
            .and_then(|v| usize::try_from(v).ok())
            .ok_or_else(|| {
                error(
                    index.span,
                    "BitVec index exceeds the implementation size limit",
                )
            })?;
        Ok((checked, Some(position)))
    }

    fn float_proxy(
        &self,
        spelling: &str,
        negative: bool,
        expected: Option<&Term>,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let target = match expected {
            Some(ty) => self.structural_type(ty, span)?,
            None if self.numeric_defaults => p(P::F64),
            None => {
                return Err(context_error(
                    span,
                    "float runtime proxy awaits type context",
                ));
            }
        };
        if matches!(
            target,
            Term::Rigid {
                sort: Sort::Type,
                ..
            }
        ) {
            self.establish_type_requirement(
                &n::TypePredicate::Or(vec![
                    n::TypePredicate::Equal(target.clone(), p(P::Float)),
                    n::TypePredicate::Equal(target.clone(), p(P::F16)),
                    n::TypePredicate::Equal(target.clone(), p(P::Bf16)),
                    n::TypePredicate::Equal(target.clone(), p(P::F32)),
                    n::TypePredicate::Equal(target.clone(), p(P::F64)),
                ]),
                span,
            )?;
            nir::float_formation::FloatFormation::parse(
                spelling,
                negative,
                &mut comptime_runtime::Budget::default(),
            )
            .map_err(|e| error(span, e.to_string()))?;
            return Ok(n::Expr {
                ty: target,
                source: source(span),
                kind: K::Literal(n::Literal::PendingFloat {
                    spelling: spelling.into(),
                    negative,
                }),
            });
        }
        if !matches!(
            target,
            Term::Primitive(P::Float | P::F16 | P::Bf16 | P::F32 | P::F64)
        ) {
            return Err(error(
                span,
                "float literal requires a Float, f16, bf16, f32 or f64 target",
            ));
        }
        let mut budget = comptime_runtime::Budget::default();
        let formation =
            nir::float_formation::FloatFormation::parse(spelling, negative, &mut budget)
                .map_err(|e| error(span, e.to_string()))?;
        let failure = |e: comptime_runtime::Error| {
            error(
                span,
                if e == comptime_runtime::Error::OutOfRange {
                    "overflowing finite float literal conversion is not yet implemented".into()
                } else {
                    e.to_string()
                },
            )
        };
        let Term::Primitive(domain) = target else {
            unreachable!()
        };
        let literal = formation.form(domain, &mut budget).map_err(failure)?;
        Ok(n::Expr {
            ty: target,
            source: source(span),
            kind: K::Literal(literal),
        })
    }
    fn integer_proxy(
        &self,
        spelling: &str,
        negative: bool,
        expected: Option<&Term>,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let target = match expected {
            Some(ty) => self.structural_type(ty, span)?,
            None if self.numeric_defaults => p(if self.in_required_comptime || self.signature_owner
                .and_then(|owner| self.package.item(owner))
                .is_some_and(|item| matches!(&item.kind, ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig) if sig.modifiers.is_comptime)) {
                    P::Integer
                } else { P::I32 }),
            None => {
                return Err(context_error(
                    span,
                    "integer runtime proxy awaits type context",
                ));
            }
        };
        if matches!(
            target,
            Term::Rigid {
                sort: Sort::Type,
                ..
            }
        ) {
            let domains = [
                P::Integer,
                P::U8,
                P::U16,
                P::U32,
                P::U64,
                P::U128,
                P::I8,
                P::I16,
                P::I32,
                P::I64,
                P::I128,
                P::Usize,
                P::Isize,
            ];
            self.establish_type_requirement(
                &n::TypePredicate::Or(
                    domains
                        .into_iter()
                        .map(|domain| n::TypePredicate::Equal(target.clone(), p(domain)))
                        .collect(),
                ),
                span,
            )?;
            comptime_runtime::Integer::parse_literal(
                spelling,
                &mut comptime_runtime::Budget::default(),
            )
            .map_err(|e| error(span, e.to_string()))?;
            return Ok(n::Expr {
                ty: target,
                source: source(span),
                kind: K::Literal(n::Literal::PendingInteger {
                    spelling: spelling.into(),
                    negative,
                }),
            });
        }
        let Term::Primitive(primitive) = target else {
            return Err(error(
                span,
                "integer runtime proxy requires a supported fixed-width integer target",
            ));
        };
        if primitive == P::Integer {
            comptime_runtime::Integer::parse_literal(
                spelling,
                &mut comptime_runtime::Budget::default(),
            )
            .map_err(|e| error(span, e.to_string()))?;
            return Ok(n::Expr {
                ty: p(P::Integer),
                source: source(span),
                kind: K::Literal(n::Literal::Integer(if negative {
                    format!("-{spelling}")
                } else {
                    spelling.into()
                })),
            });
        }
        if let P::BitVec(width) = primitive {
            let mut budget = comptime_runtime::Budget::default();
            let mut integer = comptime_runtime::Integer::parse_literal(spelling, &mut budget)
                .map_err(|e| error(span, e.to_string()))?;
            if negative {
                integer = integer.negated();
            }
            let bits = integer.to_bitvec(width, &mut budget).map_err(|_| {
                error(
                    span,
                    "integer literal does not fit the BitVec width or conversion budget",
                )
            })?;
            return Ok(n::Expr {
                ty: target,
                source: source(span),
                kind: K::Literal(n::Literal::BitVec {
                    width,
                    words: bits.words().to_vec(),
                }),
            });
        }
        let Some((signed, width)) = self.target.map_or_else(
            || primitive.fixed_integer_layout(),
            |target| target.integer_layout(primitive),
        ) else {
            return Err(error(
                span,
                if matches!(primitive, P::Isize | P::Usize) {
                    "integer runtime proxy requires target pointer-width information"
                } else {
                    "integer runtime proxy requires a supported fixed-width integer target"
                },
            ));
        };
        let mut exact = comptime_runtime::Integer::parse_literal(
            spelling,
            &mut comptime_runtime::Budget::default(),
        )
        .map_err(|e| error(span, e.to_string()))?;
        if negative {
            exact = exact.negated();
        }
        let literal = if signed {
            n::Literal::SignedInteger(
                exact
                    .to_signed(width)
                    .map_err(|e| error(span, e.to_string()))?,
            )
        } else {
            n::Literal::UnsignedInteger(
                exact
                    .to_unsigned(width)
                    .map_err(|e| error(span, e.to_string()))?,
            )
        };
        Ok(n::Expr {
            ty: p(primitive),
            source: source(span),
            kind: K::Literal(literal),
        })
    }
    fn expr(&mut self, expr: &Expr<'_>, expected: Option<&Term>) -> Result<n::Expr, Error> {
        self.expr_context(expr, expected, None)
    }
    #[inline(never)]
    fn arithmetic_expression(
        &mut self,
        expr: &Expr<'_>,
        op: &BinOp,
        left: &Expr<'_>,
        right: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let operator = match op {
            BinOp::Add => crate::arithmetic::Operator::Add,
            BinOp::Sub => crate::arithmetic::Operator::Subtract,
            BinOp::Mul => crate::arithmetic::Operator::Multiply,
            BinOp::Div => crate::arithmetic::Operator::Divide,
            BinOp::Rem => crate::arithmetic::Operator::Remainder,
            _ => unreachable!(),
        };
        let mut checked_right = None;
        let checked_left = if let Some(protocol) = self.protocols.arithmetic[operator as usize] {
            let checked = self.argument_attempt(left, None, None)?;
            if let Some(value) = checked.as_ref().filter(|value| !is_never(&value.ty)) {
                let shape = match self
                    .context
                    .structural_view_with_heads_metered(
                        &value.ty,
                        crate::ptc::Budget::default(),
                        |head| Some(head) == self.protocols.tuple,
                    )
                    .0
                {
                    Ok(view) => view.map(|(ty, _)| ty).unwrap_or_else(|| value.ty.clone()),
                    Err(crate::ptc::UnknownReason::UnvalidatedModel) => value.ty.clone(),
                    Err(reason) => {
                        return Err(error(
                            left.span,
                            format!("arithmetic operand view remains unresolved: {reason:?}"),
                        ));
                    }
                };
                let native = self.native_arithmetic_type(op, &shape);
                // A native left operand does not imply a homogeneous RHS.
                // Probe it without a fabricated left-type requirement, and
                // retain successful work for either the protocol or core path.
                checked_right = self.argument_attempt(right, None, None)?;
                // Try the native operand context transactionally before asking
                // for a protocol context. This preserves numeric literal
                // inference (including nested literals), while an incompatible
                // enum/callable context can be retried through a unique trait
                // RHS even inside a block, branch, or generic call. Never use
                // semantic errors or solver exhaustion as candidate evidence.
                let mut native_failure = None;
                if checked_right.is_none() && native {
                    match self.argument_attempt(right, Some(&value.ty), None) {
                        Ok(value) => checked_right = value,
                        Err(failure) if failure.incompatible_context => {
                            native_failure = Some(failure)
                        }
                        Err(failure) => return Err(failure),
                    }
                }
                if checked_right.is_none() {
                    if let Some(rhs) =
                        self.protocol_argument_context(&value.ty, protocol, expr.span)?
                    {
                        checked_right = Some(self.expr(right, Some(&rhs))?);
                    } else if let Some(mut failure) = native_failure {
                        failure
                            .message
                            .push_str("; arithmetic RHS has no unique protocol context");
                        return Err(failure);
                    }
                }
                let homogeneous = match checked_right.as_ref() {
                    Some(rhs) if !is_never(&rhs.ty) && rhs.ty != value.ty => {
                        match self.context.prove(
                            crate::ptc::Formula::equal(value.ty.clone(), rhs.ty.clone()),
                            crate::ptc::Budget::default(),
                        ) {
                            crate::ptc::Outcome::Entailed(_) => true,
                            crate::ptc::Outcome::NotEntailed(_)
                            | crate::ptc::Outcome::Unknown {
                                reason: crate::ptc::UnknownReason::UnvalidatedModel,
                                ..
                            } => false,
                            outcome => {
                                return Err(error(
                                    expr.span,
                                    format!(
                                        "arithmetic operand equality remains unresolved: {outcome}"
                                    ),
                                ));
                            }
                        }
                    }
                    _ => true,
                };
                if !native || !homogeneous {
                    let right = match checked_right.take() {
                        Some(right) => right,
                        None => self.expr(right, None)?,
                    };
                    return self.finish_binary_protocol_call(
                        expr,
                        protocol,
                        operator.method(),
                        checked.unwrap(),
                        right,
                        expected,
                    );
                }
            }
            Some(checked)
        } else {
            None
        };
        let hint = expected
            .map(|ty| self.structural_type(ty, expr.span))
            .transpose()?;
        let hint = hint.as_ref().filter(|ty| {
            matches!(
                ty,
                Term::Primitive(P::Float | P::F16 | P::Bf16 | P::F32 | P::F64 | P::Integer)
            ) || matches!(ty, Term::Primitive(p) if self.integer_layout(*p).is_some())
                || self.proved_integer_family(ty, false)
                || self.proved_float_family(ty)
        });
        let fallback = (is_numeric_literal(left) && !is_float_literal(left)).then(|| p(P::Integer));
        let (left, right) = match checked_left {
            Some(checked) => self.peer_operands_with_ready(
                left,
                right,
                hint,
                fallback.as_ref(),
                checked,
                checked_right,
            )?,
            None => self.peer_operands(left, right, hint, fallback.as_ref())?,
        };
        // No operator or RHS executes after a nonreturning left operand.
        // Both source operands were still elaborated above. In particular,
        // do not invent a rounding protocol from the unreachable RHS type.
        if is_never(&left.ty) {
            if let Some(expected) = expected {
                self.require_expression(&left.ty, expected, expr.span)?;
            }
            return Ok(left);
        }
        let shape = self.structural_type(&left.ty, expr.span)?;
        // A literal/block receiver may only become typed during peer inference.
        // Reuse both checked operands when selecting its protocol: do not
        // re-elaborate producers or assume the associated Output is the input
        // type. In particular signed division/remainder have no native policy.
        if !is_never(&left.ty) && !self.native_arithmetic_type(op, &shape) {
            if let Some(protocol) = self.protocols.arithmetic[operator as usize] {
                return self.finish_binary_protocol_call(
                    expr,
                    protocol,
                    operator.method(),
                    left,
                    right,
                    expected,
                );
            }
        }
        let ty = if is_never(&left.ty) || is_never(&right.ty) {
            p(P::Never)
        } else {
            shape.clone()
        };
        if *op == BinOp::Rem
            && !matches!(shape, Term::Primitive(p) if matches!(self.integer_layout(p), Some((false, _))))
            && !self.proved_integer_family(&shape, true)
        {
            return Err(error(
                expr.span,
                "protocol or construct not yet implemented: remainder rounding for this type",
            ));
        }
        if shape == p(P::Float) && *op == BinOp::Div {
            return Err(error(expr.span, "exact Float division is not implemented"));
        }
        let kind = if matches!(
            shape,
            Term::Primitive(P::Float | P::F16 | P::Bf16 | P::F32 | P::F64)
        ) || self.proved_float_family(&shape)
        {
            K::FloatBinary {
                op: match op {
                    BinOp::Add => n::FloatOp::Add,
                    BinOp::Sub => n::FloatOp::Subtract,
                    BinOp::Mul => n::FloatOp::Multiply,
                    _ => n::FloatOp::Divide,
                },
                left: Box::new(left),
                right: Box::new(right),
            }
        } else {
            let integer_type = if matches!(shape, Term::Primitive(p) if self.integer_layout(p).is_some())
                || self.proved_integer_family(&shape, false)
            {
                shape.clone()
            } else {
                p(P::Integer)
            };
            self.require(&left.ty, &integer_type, expr.span)?;
            self.require(&right.ty, &integer_type, expr.span)?;
            if *op == BinOp::Div
                && !matches!(shape, Term::Primitive(p) if matches!(self.integer_layout(p), Some((false, _))))
                && !self.proved_integer_family(&shape, true)
            {
                return Err(error(
                    expr.span,
                    "Integer division operator rounding protocol is not implemented",
                ));
            }
            K::IntegerBinary {
                op: match op {
                    BinOp::Add => n::IntegerOp::Add,
                    BinOp::Sub => n::IntegerOp::Subtract,
                    BinOp::Div => n::IntegerOp::DivideTrunc,
                    BinOp::Rem => n::IntegerOp::RemainderTrunc,
                    _ => n::IntegerOp::Multiply,
                },
                left: Box::new(left),
                right: Box::new(right),
            }
        };
        if let Some(expected) = expected {
            self.require(&ty, expected, expr.span)?;
        }
        Ok(n::Expr {
            ty,
            source: source(expr.span),
            kind,
        })
    }

    fn expr_context(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        // Ordinary scalar recursion must not retain the larger adaptation
        // frame used by error-result and optional destinations.
        // Primitive identities cannot denote optional types.
        if matches!(expected, Some(Term::ErrorResult { .. }))
            || expected.is_some_and(|ty| !matches!(ty, Term::Primitive(_)))
        {
            self.expr_context_adapted(expr, expected, partial)
        } else {
            self.expr_context_unadapted(expr, expected, partial)
        }
    }

    #[inline(never)]
    fn expr_context_adapted(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        if let Some(target @ Term::ErrorResult { result, .. }) = expected {
            // Branching syntax distributes the destination to its yielded paths.
            let directed = matches!(
                expr.kind,
                H::Block(_)
                    | H::If(_, _, Some(_))
                    | H::Match(..)
                    | H::When(_)
                    | H::Return(_)
                    | H::ErrorNew(_)
                    | H::ErrorHandle { .. }
                    | H::EffectHandle(..)
            );
            if !directed {
                let value = if let Some(value) = self.argument_attempt(expr, None, partial)? {
                    value
                } else {
                    self.expr_context(expr, Some(result), partial)?
                };
                if is_never(&value.ty) {
                    return Ok(value);
                }
                let conversion = if matches!(value.ty, Term::ErrorResult { .. }) {
                    nir::error_results::Conversion::Embed
                } else {
                    nir::error_results::Conversion::Success
                };
                return self.convert_error_value(value, target, conversion, expr.span);
            }
        }
        if self.optional_target_probe != Some(expr.hir_id) {
            if let Some(target) = expected {
                if let Ok(payload) = self.optional_payload(target, expr.span) {
                    // Branches and null receive the whole destination, not a payload hint.
                    let directed = matches!(
                        &expr.kind,
                        H::Null
                            | H::Block(_)
                            | H::If(..)
                            | H::Match(..)
                            | H::When(_)
                            | H::Return(_)
                            | H::EffectRequest(_)
                            | H::ErrorHandle { .. }
                            | H::EffectHandle(..)
                    ) || enum_groups::symbol_path(expr).is_some()
                        || matches!(&expr.kind, H::Application(head,_) | H::ExtendedApplication(head,_)
                            if enum_groups::symbol_path(head).is_some());
                    if !directed {
                        if let Some(value) = self.argument_attempt(expr, None, partial)? {
                            return self.inject_optional_value(value, target, expr.span);
                        }
                        // A generic producer or optional elimination may itself
                        // produce the complete optional, preserving nested layers.
                        if matches!(
                            expr.kind,
                            H::Application(..) | H::ExtendedApplication(..) | H::OptionPropagate(_)
                        ) {
                            let saved = self.optional_target_probe.replace(expr.hir_id);
                            let attempt = self.argument_attempt_with_defaults(
                                expr,
                                Some(target),
                                partial,
                                false,
                            );
                            self.optional_target_probe = saved;
                            if let Ok(Some(value)) = attempt {
                                return Ok(value);
                            }
                        }
                        let value = self.expr_context(expr, Some(&payload), partial)?;
                        if is_never(&value.ty) {
                            return Ok(value);
                        }
                        return Ok(n::Expr {
                            source: source(expr.span),
                            ty: target.clone(),
                            kind: K::OptionalPresent(Box::new(value)),
                        });
                    }
                }
            }
        }
        self.expr_context_unadapted(expr, expected, partial)
    }

    // An omitted block tail or bare return still delivers a void value.
    // Represent its success injection explicitly, preserving the error ABI.
    fn implicit_error_success(
        &mut self,
        expected: Option<&Term>,
        span: Span,
    ) -> Result<Option<n::Expr>, Error> {
        let Some(target @ Term::ErrorResult { .. }) = expected else {
            return Ok(None);
        };
        let value = n::Expr {
            source: source(span),
            ty: p(P::Void),
            kind: K::Unit,
        };
        self.convert_error_value(value, target, nir::error_results::Conversion::Success, span)
            .map(Some)
    }

    fn convert_error_value(
        &mut self,
        value: n::Expr,
        target: &Term,
        conversion: nir::error_results::Conversion,
        span: Span,
    ) -> Result<n::Expr, Error> {
        nir::error_results::validate_conversion(
            conversion,
            &value.ty,
            target,
            &mut 100_000,
            &mut |a, b| self.require(a, b, span).is_ok(),
        )
        .map_err(|e| error(span, format!("error result conversion: {e}")))?;
        Ok(n::Expr {
            // Conversion and operand are distinct runtime cleanup boundaries,
            // even when an implicit conversion shares the operand's syntax.
            source: self.error_generated_source(span)?,
            ty: target.clone(),
            kind: K::ErrorConvert {
                conversion,
                value: Box::new(value),
            },
        })
    }

    fn inject_optional_value(
        &self,
        mut value: n::Expr,
        target: &Term,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let mut layers = Vec::new();
        let mut inner = target.clone();
        for _ in 0..128 {
            if self.require(&value.ty, &inner, span).is_ok() {
                for ty in layers.into_iter().rev() {
                    value = n::Expr {
                        source: source(span),
                        ty,
                        kind: K::OptionalPresent(Box::new(value)),
                    };
                }
                return Ok(value);
            }
            let payload = self.optional_payload(&inner, span)?;
            layers.push(inner);
            inner = payload;
        }
        Err(error(span, "optional injection depth exhausted"))
    }

    fn expr_context_unadapted(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        // Keep scalar operand recursion off the call/control dispatcher frame.
        let scalar = matches!(
            expr.kind,
            H::Unary(UnOp::Neg | UnOp::Not, _)
                | H::Binary(
                    BinOp::And
                        | BinOp::Or
                        | BinOp::Eq
                        | BinOp::Ne
                        | BinOp::Lt
                        | BinOp::Le
                        | BinOp::Gt
                        | BinOp::Ge
                        | BinOp::Add
                        | BinOp::Sub
                        | BinOp::Mul
                        | BinOp::Div
                        | BinOp::Rem,
                    _,
                    _
                )
        );
        if !scalar {
            return self.expr_context_general(expr, expected, partial);
        }
        let preserve = std::mem::take(&mut self.preserve_effect_value);
        let value = match &expr.kind {
            H::Unary(_, _) => self.unary_expression(expr, expected),
            H::Binary(BinOp::And | BinOp::Or, _, _) => self.logical_expression(expr, expected),
            H::Binary(
                op @ (BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem),
                a,
                b,
            ) => self.arithmetic_expression(expr, op, a, b, expected),
            _ => self.comparison_expression(expr, expected),
        }?;
        if preserve {
            Ok(value)
        } else {
            self.run_computation(value, expected, expr.span)
        }
    }

    #[inline(never)]
    fn expr_context_general(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        if self.data_consumer_call(expr).is_some() {
            return self.check_data_consumer_expression(expr, expected);
        }
        if let H::Application(callee, arguments) = &expr.kind
            && let H::ResolvedDef { def_id, .. } = &callee.kind
            && let Some(owner) = self.package.definition_owner(*def_id)
            && let Some(name) = self.compiler_operation_binding(owner, callee.span)?
        {
            let bound = Expr {
                hir_id: callee.hir_id,
                span: callee.span,
                kind: H::Builtin(name),
            };
            let call = Expr {
                hir_id: expr.hir_id,
                span: expr.span,
                kind: H::Application(&bound, arguments),
            };
            return self.expr_context_general(&call, expected, partial);
        }
        // Inline control retains the delivery position of its ordinary
        // control node. Conditions and unselected branches keep their existing
        // staged checking; only result-producing leaves inherit callee use.
        if let H::InlineControl(inner) = expr.kind {
            if self.comptime_callee_result == Some(expr.hir_id) {
                return self.with_callee_delivery(expr.hir_id, Some(inner.hir_id), |this| {
                    this.expr_context_unadapted(expr, expected, partial)
                });
            }
        }
        let preserve_effect_value = std::mem::take(&mut self.preserve_effect_value);
        // Calls recurse through arguments. Keep that path outside the large
        // general dispatcher frame, including projection and extended calls.
        let value = if intrinsics::is_application(expr) {
            self.intrinsic_expression(expr, expected)
        } else {
            match &expr.kind {
                // Recursive control-flow wrappers must not retain the large
                // general dispatcher frame for each nested source expression.
                H::Block(block) | H::UnsafeBlock(block) => {
                    // A block used as a callee delivers its tail in that position;
                    // statements and their arguments remain ordinary expressions.
                    let saved = self.comptime_callee_result;
                    if saved == Some(expr.hir_id) {
                        self.comptime_callee_result = block.expr.map(|tail| tail.hir_id);
                    }
                    let result = self.block_context(block, expected, partial);
                    self.comptime_callee_result = saved;
                    if matches!(expr.kind, H::UnsafeBlock(_)) {
                        result.map(|body| n::Expr {
                            source: source(expr.span),
                            ty: body.ty.clone(),
                            kind: K::UnsafeScope(Box::new(body)),
                        })
                    } else {
                        result
                    }
                }
                H::Match(value, arms) => self.match_expr(expr, value, arms, expected, partial),
                H::When(arms) => self.when_expr(expr, arms, expected, partial),
                H::Application(callee, args) if matches!(&callee.kind, H::Builtin(name) if matches!(name.as_str(), "enum_group_paths" | "enum_variant_paths" | "enum_variant_shapes" | "enum_variant_field_names" | "enum_variant_field_types")) => {
                    self.enum_reflection_builtin(expr, callee, args, expected)
                }
                H::Application(callee, args) if matches!(&callee.kind, H::Builtin(name) if matches!(name.as_str(), "object_concat" | "object_append" | "object_replace" | "object_remove" | "object_slice")) => {
                    self.object_edit_builtin(expr, callee, args, expected)
                }
                H::Application(callee, args) if matches!(&callee.kind, H::Builtin(name) if matches!(name.as_str(), "object_length" | "object_key" | "object_entry_type" | "object_value" | "object_get")) => {
                    self.object_query_builtin(expr, callee, args, expected)
                }
                H::Application(callee, args) if matches!(&callee.kind, H::Builtin(name) if matches!(name.as_str(), "build_enum" | "build_enum_paths" | "build_symbol_enum_paths" | "build_bits_enum_paths")) =>
                {
                    let H::Builtin(name) = &callee.kind else {
                        unreachable!()
                    };
                    self.build_enum_builtin(
                        expr,
                        args,
                        expected,
                        name.as_str() != "build_enum",
                        name.as_str() == "build_symbol_enum_paths",
                        name.as_str() == "build_bits_enum_paths",
                    )
                }
                H::Application(callee, args) if matches!(&callee.kind, H::Builtin(name) if name.as_str() == "build_struct") => {
                    self.build_struct_builtin(expr, args, expected)
                }
                H::Application(callee, args) if matches!(&callee.kind, H::Builtin(name) if matches!(name.as_str(), "struct_field_types" | "struct_field_names")) => {
                    self.struct_reflection_builtin(expr, callee, args, expected)
                }
                H::Application(callee, args) if matches!(&callee.kind, H::Builtin(name) if matches!(name.as_str(), "list_length" | "list_concat")) => {
                    self.list_builtin(expr, callee, args, expected)
                }
                H::Application(callee, args) if matches!(&callee.kind, H::Builtin(name) if matches!(name.as_str(), "comptime_integer_div_trunc" | "comptime_integer_rem_trunc")) =>
                {
                    if self.runtime_value_context() || self.in_code_body {
                        return Err(error(
                            expr.span,
                            "explicit Integer division requires comptime invocation",
                        ));
                    }
                    let [Arg::Positional(left), Arg::Positional(right)] = args else {
                        return Err(error(
                            expr.span,
                            "explicit Integer division requires two positional Integer operands",
                        ));
                    };
                    let H::Builtin(name) = &callee.kind else {
                        unreachable!()
                    };
                    let left = self.expr(left, Some(&p(P::Integer)))?;
                    let right = self.expr(right, Some(&p(P::Integer)))?;
                    let ty = if is_never(&left.ty) || is_never(&right.ty) {
                        p(P::Never)
                    } else {
                        p(P::Integer)
                    };
                    if let Some(expected) = expected {
                        self.require_expression(&ty, expected, expr.span)?;
                    }
                    Ok(n::Expr {
                        source: source(expr.span),
                        ty,
                        kind: K::IntegerBinary {
                            op: if name.as_str() == "comptime_integer_div_trunc" {
                                n::IntegerOp::DivideTrunc
                            } else {
                                n::IntegerOp::RemainderTrunc
                            },
                            left: Box::new(left),
                            right: Box::new(right),
                        },
                    })
                }
                H::Application(callee, args) if matches!(&callee.kind, H::Builtin(name) if name.as_str() == "comptime_integer_convert") =>
                {
                    if self.runtime_value_context() || self.in_code_body {
                        return Err(error(
                            expr.span,
                            "integer conversion requires comptime invocation",
                        ));
                    }
                    let [Arg::Positional(target), Arg::Positional(value)] = args else {
                        return Err(error(
                            expr.span,
                            "integer conversion requires a Type and an integer",
                        ));
                    };
                    let target = self.type_expr(target)?;
                    let value = self.expr(value, None)?;
                    let ty = if is_never(&value.ty) {
                        p(P::Never)
                    } else {
                        target
                    };
                    if !n::comptime_integer_conversion_valid(&value.ty, &ty) {
                        return Err(error(
                            expr.span,
                            "integer conversion requires concrete integer domains",
                        ));
                    }
                    if let Some(expected) = expected {
                        self.require_expression(&ty, expected, expr.span)?;
                    }
                    Ok(n::Expr {
                        source: source(expr.span),
                        ty,
                        kind: K::ComptimeIntegerConvert(Box::new(value)),
                    })
                }
                H::Application(callee, args) if matches!(&callee.kind, H::Builtin(name) if name.as_str() == "integer_representation") =>
                {
                    let [Arg::Positional(value)] = args else {
                        return Err(error(
                            expr.span,
                            "integer representation requires one positional integer",
                        ));
                    };
                    let value = self.expr(value, None)?;
                    let ty = if is_never(&value.ty) {
                        p(P::Never)
                    } else {
                        p(P::U128)
                    };
                    if !n::integer_representation_valid(&value.ty, &ty) {
                        return Err(error(
                            expr.span,
                            "integer representation requires a machine integer",
                        ));
                    }
                    if let Some(expected) = expected {
                        self.require_expression(&ty, expected, expr.span)?;
                    }
                    Ok(n::Expr {
                        source: source(expr.span),
                        ty,
                        kind: K::IntegerRepresentation(Box::new(value)),
                    })
                }
                H::Application(callee, args) if matches!(&callee.kind, H::Builtin(name) if matches!(name.as_str(), "str_byte_length" | "str_byte" | "symbol_byte_length" | "symbol_byte")) => {
                    self.text_builtin(expr, callee, args, expected)
                }
                H::Application(callee, args) => {
                    let quote_head = if let H::NFApplication(head, _) = callee.kind {
                        head
                    } else {
                        callee
                    };
                    if let H::ResolvedDef { def_id, .. } = quote_head.kind {
                        if self
                            .package
                            .definition_owner(def_id)
                            .is_some_and(|owner| self.is_quote_function(owner))
                        {
                            self.record_quote_application(expr.span)?;
                        }
                    }
                    if let Some((ty, index)) = self.variant_target(callee, expected)? {
                        self.construct_variant(expr, ty, index, args, expected)
                    } else {
                        self.call(expr, callee, args, expected, partial)
                    }
                }
                H::ExtendedApplication(callee, args) => {
                    if let Some((ty, index)) = self.variant_target(callee, expected)? {
                        self.construct_named_variant(expr, ty, index, args, expected)
                    } else if let Some(call) =
                        self.extended_call(expr, callee, args, expected, partial)?
                    {
                        Ok(call)
                    } else {
                        self.construct(expr, callee, args, expected)
                    }
                }
                H::Projection(value, field) => {
                    self.projection_expression(expr, value, field, expected, partial)
                }
                H::Binary(BinOp::Concat, _, _) => self.concat_expression(expr, expected),
                _ => self.expr_context_inner(expr, expected, partial),
            }
        }?;
        if matches!(expr.kind, H::Assign(..) | H::AssignOp(..) | H::Ref(_))
            || matches!(value.kind, K::Closure { .. })
        {
            self.invalidate_staged_storage(&value, expr.span)?;
        }
        if preserve_effect_value {
            Ok(value)
        } else {
            self.run_computation(value, expected, expr.span)
        }
    }
}

fn is_numeric_literal(expr: &Expr<'_>) -> bool {
    let expr = match &expr.kind {
        H::Unary(UnOp::Neg, inner) => *inner,
        _ => expr,
    };
    matches!(
        expr.kind,
        H::Lit(hir::Lit {
            kind: hir::LitKind::Integer(_) | hir::LitKind::Float(_),
            ..
        })
    )
}

fn is_float_literal(expr: &Expr<'_>) -> bool {
    let expr = match &expr.kind {
        H::Unary(UnOp::Neg, inner) => *inner,
        _ => expr,
    };
    matches!(
        expr.kind,
        H::Lit(hir::Lit {
            kind: hir::LitKind::Float(_),
            ..
        })
    )
}

// Evidence about an original retained input, conditional on a guard outcome.
// Unknown is a safe expression with no unique identity; None from the evaluator
// means an unsupported computation that may change the candidate binding.
#[derive(Clone, Copy)]
enum PatternGuard<T> {
    Impossible,
    Unknown,
    Exact(T),
}

impl<T: PartialEq> PatternGuard<T> {
    fn both(self, other: Self) -> Self {
        match (self, other) {
            (Self::Impossible, _) | (_, Self::Impossible) => Self::Impossible,
            (Self::Exact(value), _) | (_, Self::Exact(value)) => Self::Exact(value),
            _ => Self::Unknown,
        }
    }
    fn either(self, other: Self) -> Self {
        match (self, other) {
            (Self::Impossible, other) | (other, Self::Impossible) => other,
            (Self::Exact(left), Self::Exact(right)) if left == right => Self::Exact(left),
            _ => Self::Unknown,
        }
    }
}

fn pattern_guard_evidence<'a, T: PartialEq>(
    expression: &'a n::Expr,
    binding: n::LocalId,
    truth: bool,
    fuel: &mut u64,
    span: Span,
    depth: usize,
    atom: &mut impl FnMut(&'a n::Expr, bool, &mut u64) -> Result<Option<PatternGuard<T>>, Error>,
) -> Result<Option<PatternGuard<T>>, Error> {
    use PatternGuard::{Impossible, Unknown};
    if depth >= 128 {
        return Err(error(span, "type guard evidence depth exhausted"));
    }
    *fuel = fuel
        .checked_sub(1)
        .ok_or_else(|| error(span, "type guard evidence budget exhausted"))?;
    Ok(Some(match &expression.kind {
        K::Literal(n::Literal::Bool(value)) => {
            if *value == truth {
                Unknown
            } else {
                Impossible
            }
        }
        K::Local(_) => Unknown,
        K::Not(value) => {
            return pattern_guard_evidence(value, binding, !truth, fuel, span, depth + 1, atom);
        }
        K::And(left, right) | K::Or(left, right) => {
            // Only this finite, non-mutating subset may combine observations
            // of a binding across operands. Calls, statement blocks and writes remain
            // unsupported, even if one operand appears to identify the type.
            let Some(left) =
                pattern_guard_evidence(left, binding, truth, fuel, span, depth + 1, atom)?
            else {
                return Ok(None);
            };
            let Some(right) =
                pattern_guard_evidence(right, binding, truth, fuel, span, depth + 1, atom)?
            else {
                return Ok(None);
            };
            let both = matches!(expression.kind, K::And(..)) == truth;
            if both {
                left.both(right)
            } else {
                left.either(right)
            }
        }
        K::ScalarEqual(left, right) if left.ty == p(P::Bool) && right.ty == p(P::Bool) => {
            // A literal side fixes the other outcome directly. Do not branch
            // twice over nested `== true` wrappers: their cost must be linear.
            if let K::Literal(n::Literal::Bool(value)) = left.kind {
                return pattern_guard_evidence(
                    right,
                    binding,
                    truth == value,
                    fuel,
                    span,
                    depth + 1,
                    atom,
                );
            }
            if let K::Literal(n::Literal::Bool(value)) = right.kind {
                return pattern_guard_evidence(
                    left,
                    binding,
                    truth == value,
                    fuel,
                    span,
                    depth + 1,
                    atom,
                );
            }
            let Some(left_true) =
                pattern_guard_evidence(left, binding, true, fuel, span, depth + 1, atom)?
            else {
                return Ok(None);
            };
            let Some(left_false) =
                pattern_guard_evidence(left, binding, false, fuel, span, depth + 1, atom)?
            else {
                return Ok(None);
            };
            let Some(right_for_true) =
                pattern_guard_evidence(right, binding, truth, fuel, span, depth + 1, atom)?
            else {
                return Ok(None);
            };
            let Some(right_for_false) =
                pattern_guard_evidence(right, binding, !truth, fuel, span, depth + 1, atom)?
            else {
                return Ok(None);
            };
            left_true
                .both(right_for_true)
                .either(left_false.both(right_for_false))
        }
        K::Block {
            statements,
            result: Some(result),
        } if statements.is_empty() => {
            return pattern_guard_evidence(result, binding, truth, fuel, span, depth + 1, atom);
        }
        K::If {
            condition,
            then_branch,
            else_branch: Some(else_branch),
        } => {
            let Some(yes_condition) =
                pattern_guard_evidence(condition, binding, true, fuel, span, depth + 1, atom)?
            else {
                return Ok(None);
            };
            let Some(no_condition) =
                pattern_guard_evidence(condition, binding, false, fuel, span, depth + 1, atom)?
            else {
                return Ok(None);
            };
            let Some(yes) =
                pattern_guard_evidence(then_branch, binding, truth, fuel, span, depth + 1, atom)?
            else {
                return Ok(None);
            };
            let Some(no) =
                pattern_guard_evidence(else_branch, binding, truth, fuel, span, depth + 1, atom)?
            else {
                return Ok(None);
            };
            yes_condition.both(yes).either(no_condition.both(no))
        }
        _ => {
            if let Some(evidence) = atom(expression, truth, fuel)? {
                return Ok(Some(evidence));
            }
            if readonly_guard_comparison(expression, fuel, span)? {
                Unknown
            } else {
                return Ok(None);
            }
        }
    }))
}

// A comparison may constrain a different input/domain than the identity
// currently being recovered. Keep its outcome unknown, but only after proving
// structurally that its operands cannot modify the candidate or call code.
fn readonly_guard_comparison(expr: &n::Expr, fuel: &mut u64, span: Span) -> Result<bool, Error> {
    let (left, right) = match &expr.kind {
        K::TypeEqual(left, right)
        | K::ScalarEqual(left, right)
        | K::ScalarLess(left, right)
        | K::ScalarLessEqual(left, right)
        | K::ScalarGreater(left, right)
        | K::ScalarGreaterEqual(left, right)
        | K::BitEqual(left, right) => (left, right),
        K::IntegerBinary { op, left, right }
            if matches!(
                op,
                n::IntegerOp::Equal
                    | n::IntegerOp::NotEqual
                    | n::IntegerOp::Less
                    | n::IntegerOp::LessEqual
                    | n::IntegerOp::Greater
                    | n::IntegerOp::GreaterEqual
            ) =>
        {
            (left, right)
        }
        _ => return Ok(false),
    };
    Ok(readonly_guard_operand(left, fuel, span)? && readonly_guard_operand(right, fuel, span)?)
}

fn readonly_guard_operand(expr: &n::Expr, fuel: &mut u64, span: Span) -> Result<bool, Error> {
    let mut pending = vec![(expr, 0usize)];
    while let Some((expr, depth)) = pending.pop() {
        *fuel = fuel
            .checked_sub(1)
            .ok_or_else(|| error(span, "guard observation budget exhausted"))?;
        if depth >= 128 {
            return Err(error(span, "guard observation depth exhausted"));
        }
        match &expr.kind {
            K::Local(_) | K::Literal(_) | K::TypeValue(_) => {}
            K::IntegerNegate(value)
            | K::FloatNegate(value)
            | K::Not(value)
            | K::BitConvert(value)
            | K::TakeBits(value)
            | K::TextQuery { subject: value, .. }
            | K::IntegerRepresentation(value)
            | K::ComptimeIntegerConvert(value)
            | K::SymbolName(value)
            | K::BitSlice { value, .. } => pending.push((value, depth + 1)),
            K::IntegerBinary { left, right, .. }
            | K::FloatBinary { left, right, .. }
            | K::And(left, right)
            | K::Or(left, right)
            | K::BitConcat(left, right)
            | K::BitEqual(left, right)
            | K::TypeEqual(left, right)
            | K::ScalarEqual(left, right)
            | K::ScalarLess(left, right)
            | K::ScalarLessEqual(left, right)
            | K::ScalarGreater(left, right)
            | K::ScalarGreaterEqual(left, right) => {
                pending.push((right, depth + 1));
                pending.push((left, depth + 1));
            }
            K::Block {
                statements,
                result: Some(value),
            } if statements.is_empty() => pending.push((value, depth + 1)),
            K::If {
                condition,
                then_branch,
                else_branch: Some(no),
            } => {
                pending.push((no, depth + 1));
                pending.push((then_branch, depth + 1));
                pending.push((condition, depth + 1));
            }
            _ => return Ok(false),
        }
    }
    Ok(true)
}

type PatternTypeGuard<'a> = PatternGuard<&'a Term>;

fn pattern_type_guard<'a>(
    expression: &'a n::Expr,
    binding: n::LocalId,
    truth: bool,
    fuel: &mut u64,
    span: Span,
    depth: usize,
) -> Result<Option<PatternTypeGuard<'a>>, Error> {
    use PatternGuard::{Exact, Unknown};
    pattern_guard_evidence(
        expression,
        binding,
        truth,
        fuel,
        span,
        depth,
        &mut |expression, truth, _| {
            Ok(Some(match &expression.kind {
                K::TypeEqual(left, right)
                    if matches!(left.kind, K::Local(_) | K::TypeValue(_))
                        && matches!(right.kind, K::Local(_) | K::TypeValue(_)) =>
                {
                    if truth {
                        match (&left.kind, &right.kind) {
                            (K::Local(id), K::TypeValue(ty)) | (K::TypeValue(ty), K::Local(id))
                                if *id == binding =>
                            {
                                Exact(ty)
                            }
                            _ => Unknown,
                        }
                    } else {
                        Unknown
                    }
                }
                _ => return Ok(None),
            }))
        },
    )
}

/// A successful pattern fixes its subject to this exact Type value. Wrappers
/// preserve that fact; an alternative contributes only when every arm agrees.
fn matched_type_pattern_value<'a>(
    pattern: &'a n::Pattern,
    fuel: &mut u64,
    span: Span,
    depth: usize,
) -> Result<Option<&'a Term>, Error> {
    if depth >= 128 {
        return Err(error(span, "type pattern evidence depth exhausted"));
    }
    *fuel = fuel
        .checked_sub(1)
        .ok_or_else(|| error(span, "type pattern evidence budget exhausted"))?;
    match pattern {
        n::Pattern::Binding { inner, .. } | n::Pattern::Remap { pattern: inner, .. } => {
            matched_type_pattern_value(inner, fuel, span, depth + 1)
        }
        n::Pattern::Guard { pattern, condition } => {
            if let Some(ty) = matched_type_pattern_value(pattern, fuel, span, depth + 1)? {
                return Ok(Some(ty));
            }
            if let n::Pattern::Binding { binding, inner } = pattern.as_ref() {
                if matches!(inner.as_ref(), n::Pattern::Wild) {
                    if let Some(PatternTypeGuard::Exact(ty)) =
                        pattern_type_guard(condition, *binding, true, fuel, span, depth + 1)?
                    {
                        return Ok(Some(ty));
                    }
                }
            }
            Ok(None)
        }
        n::Pattern::Then { first, value, next } => {
            if let Some(ty) = matched_type_pattern_value(first, fuel, span, depth + 1)? {
                return Ok(Some(ty));
            }
            // A bare binding followed by observing that same binding still
            // observes the original subject. Do not transfer facts from an
            // unrelated expression or through effectful intermediate guards.
            if let (n::Pattern::Binding { binding, inner }, K::Local(id)) =
                (first.as_ref(), &value.kind)
            {
                if *binding == *id && matches!(inner.as_ref(), n::Pattern::Wild) {
                    return matched_type_pattern_value(next, fuel, span, depth + 1);
                }
            }
            Ok(None)
        }
        n::Pattern::Or(arms) => {
            let mut common = None;
            for arm in arms {
                let Some(ty) = matched_type_pattern_value(arm, fuel, span, depth + 1)? else {
                    return Ok(None);
                };
                if common.is_some_and(|value| value != ty) {
                    return Ok(None);
                }
                common = Some(ty);
            }
            Ok(common)
        }
        _ => Ok(None),
    }
}

// Pure Boolean patterns can be complemented without executing a guard or view.
// The resulting ordinary pattern shares coverage and matching with every phase.
fn boolean_pattern_coverage(pattern: &n::Pattern) -> Option<u8> {
    let mut pending = vec![pattern];
    let mut mask = 0;
    let mut fuel = 100_000usize;
    while let Some(pattern) = pending.pop() {
        fuel = fuel.checked_sub(1)?;
        match pattern {
            n::Pattern::Wild => mask |= 3,
            n::Pattern::Literal(n::Literal::Bool(value)) => mask |= if *value { 2 } else { 1 },
            n::Pattern::Binding { inner, .. } => pending.push(inner),
            n::Pattern::Remap { pattern, .. } => pending.push(pattern),
            n::Pattern::Or(patterns) => {
                if patterns.len() > fuel {
                    return None;
                }
                pending.extend(patterns);
            }
            _ => return None,
        }
    }
    Some(mask)
}

fn pattern_binding_ids(pattern: &hir::pattern::Pattern<'_>) -> Vec<n::LocalId> {
    use hir::pattern::PatternKind;
    let mut pending = vec![pattern];
    let mut bindings = Vec::new();
    while let Some(pattern) = pending.pop() {
        match &pattern.kind {
            PatternKind::Binding(_, _, inner) => {
                bindings.push(local(pattern.hir_id));
                if let Some(inner) = inner {
                    pending.push(inner);
                }
            }
            PatternKind::Then(first, _, next) => {
                pending.push(first);
                pending.push(next);
            }
            PatternKind::BitVec { segments, .. } => {
                for segment in segments {
                    if let hir::pattern::BitSegment::Typed { pattern, .. } = segment {
                        pending.push(pattern);
                    }
                }
            }
            PatternKind::ErrorOk(pattern)
            | PatternKind::ErrorErr(pattern)
            | PatternKind::Guard(pattern, _)
            | PatternKind::OptionSome(pattern) => pending.push(pattern),
            PatternKind::NormalForm(_, arguments) => {
                pending.extend(arguments.iter().map(|argument| &argument.pattern))
            }
            PatternKind::Tuple(fields)
            | PatternKind::List(fields, _)
            | PatternKind::AppTuple(_, fields)
            | PatternKind::Or(fields) => pending.extend(fields.iter()),
            PatternKind::Struct(_, fields, _) => {
                pending.extend(fields.iter().map(|field| &field.pat))
            }
            _ => {}
        }
    }
    bindings
}

fn pattern_bindings(
    pattern: &hir::pattern::Pattern<'_>,
) -> Result<BTreeMap<String, n::LocalId>, Error> {
    use hir::pattern::PatternKind;
    let mut result = BTreeMap::new();
    match &pattern.kind {
        PatternKind::CodeSplice(_, outputs) => {
            for output in outputs {
                if result
                    .insert(output.name.name.to_string(), local(output.hir_id))
                    .is_some()
                {
                    return Err(error(pattern.span, "duplicate pattern binding"));
                }
            }
        }
        PatternKind::Then(first, _, next) => {
            result = pattern_bindings(first)?;
            for (name, id) in pattern_bindings(next)? {
                if result.insert(name, id).is_some() {
                    return Err(error(pattern.span, "duplicate pattern binding"));
                }
            }
        }
        PatternKind::BitVec { segments, .. } => {
            for segment in segments {
                if let hir::pattern::BitSegment::Typed { pattern: inner, .. } = segment {
                    for (name, id) in pattern_bindings(inner)? {
                        if result.insert(name, id).is_some() {
                            return Err(error(pattern.span, "duplicate pattern binding"));
                        }
                    }
                }
            }
        }
        PatternKind::ErrorOk(pattern)
        | PatternKind::ErrorErr(pattern)
        | PatternKind::Guard(pattern, _)
        | PatternKind::OptionSome(pattern) => result = pattern_bindings(pattern)?,
        PatternKind::Binding(_, name, inner) => {
            if let Some(inner) = inner {
                result = pattern_bindings(inner)?;
            }
            if result
                .insert(name.name.to_string(), local(pattern.hir_id))
                .is_some()
            {
                return Err(error(pattern.span, "duplicate pattern binding"));
            }
        }
        PatternKind::Struct(_, fields, _) => {
            for field in *fields {
                for (name, id) in pattern_bindings(&field.pat)? {
                    if result.insert(name, id).is_some() {
                        return Err(error(field.span, "duplicate pattern binding"));
                    }
                }
            }
        }
        PatternKind::NormalForm(_, arguments) => {
            for argument in *arguments {
                for (name, id) in pattern_bindings(&argument.pattern)? {
                    if result.insert(name, id).is_some() {
                        return Err(error(argument.pattern.span, "duplicate pattern binding"));
                    }
                }
            }
        }
        PatternKind::Tuple(fields)
        | PatternKind::List(fields, _)
        | PatternKind::AppTuple(_, fields) => {
            for field in *fields {
                for (name, id) in pattern_bindings(field)? {
                    if result.insert(name, id).is_some() {
                        return Err(error(field.span, "duplicate pattern binding"));
                    }
                }
            }
        }
        PatternKind::Or(alternatives) => {
            if let Some(first) = alternatives.first() {
                result = pattern_bindings(first)?;
            }
        }
        _ => {}
    }
    Ok(result)
}

#[cfg(test)]
mod type_guard_tests;
