//! HIR to typed NIR with a first-order function-phase boundary.
//! Unsupported semantic protocols fail explicitly. Bound Splice calls preserve
//! the boundary between comptime producers and runtime conversion instances.
//! Resource/effect verification remains a separate obligation;
//! this is a type-elaboration output, not permission to generate executable code.
mod aliases;
mod arguments;
pub mod attributes;
mod computed_patterns;
mod control;
mod delivered_types;
mod delivery;
mod effects;
mod error_handlers;
mod expression_values;
mod extended;
mod function_bodies;
mod function_templates;
mod function_types;
mod generated_functions;
mod inline_control;
mod inline_loops;
pub mod inlining;
mod input_observation;
mod input_routing;
mod intrinsics;
mod lexical_domains;
mod nominal_fields;
mod spreads;
mod tactics;

use arguments::route_arguments;
use control::ReturnObservations;

mod bit_indices;
mod block_regions;
mod branch_probes;
pub mod build_context;
mod builtin_bindings;
mod call_selection;
mod checking_attempts;
mod closure_bodies;
use closure_bodies::ClosureConstRegions;
mod clause_inputs;
use clause_inputs::CallableClauseView;

use checking_attempts::Disposition as CheckingDisposition;

mod cmtt;
mod constants;
mod declaration_inputs;
mod declaration_scope;
mod defaults;
mod dependent_inputs;
mod derivation;
mod dynamic_instances;
mod enum_groups;
mod input_domains;
mod input_facts;
mod interfaces;
mod iteration;
mod lexical_inputs;
mod local_bindings;

use local_bindings::{DeclarationBindings, LocalBindings, SourceBindings};

mod constructor_environment;
mod list_operations;
mod list_patterns;
mod literal_conversion;
mod nominal;
mod nominal_encoding;
mod normal;
mod open_functions;
mod operators;
mod pattern_checking;
mod returned_environment;
mod runtime_defaults;
mod runtime_environment;
mod runtime_origins;

use pattern_checking::*;

mod places;
mod projections;
mod purity;
mod refinements;
mod required;
mod source_declarations;
mod splicing;
mod text_queries;
mod trait_inputs;
mod type_acquisition;
mod type_data;
mod type_defaults;

use crate::verification::purity::{copy_scalar, stable_default};
pub(crate) use cmtt::{expand_bound_quote, expand_materialized_quotes};
use hir::common::{Arg, BinOp, TyParamKind, UnOp};
use hir::{Expr, ExprKind as H, ItemKind, OwnerId};
use nir::program::{self as n, ExprKind as K};
use nir::{DeclarationId, Parameter, Primitive as P, Sort, Term};
use rustc_span::Span;
use std::collections::{BTreeMap, BTreeSet};
use type_data::CompletedType;

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
    // Signature preparation may retry this error only after other atoms add
    // evidence or another call supplies inputs. It does not identify a waiter.
    retry_after_capability_progress: bool,
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
        retry_after_capability_progress: false,
        incompatible_context: false,
    }
}

fn capability_query_error(span: Span, cause: nir::NominalAdmissionError) -> Error {
    Error {
        retry_after_capability_progress: matches!(cause, nir::NominalAdmissionError::Unresolved(_)),
        ..error(span, cause.to_string())
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
    pub comptime_list_pattern: Option<DeclarationId>,
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

/// Default allowance for checking a source world, scaled by its package count.
pub(crate) fn default_elaboration_steps(package_count: usize) -> u64 {
    const STEPS_PER_PACKAGE: u64 = 32_000_000;
    STEPS_PER_PACKAGE.saturating_mul((package_count as u64).max(1))
}

pub fn elaborate_packages_for_target(
    package: &hir::Package<'_>,
    package_identities: &BTreeMap<u32, u64>,
    protocols: &Protocols,
    target: Option<nir::target::TargetData>,
) -> Result<n::Package, Error> {
    let mut invocation_steps = default_elaboration_steps(package_identities.len());
    elaborate_packages_with_invocation_budget(
        package,
        package_identities,
        protocols,
        target,
        &mut invocation_steps,
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

enum ElaborationMode<'r> {
    Full,
    InspectAttributes,
    ResolveDerivationDefaults {
        resolved_defaults: &'r mut BTreeMap<OwnerId, OwnerId>,
        validated_deriver_attrs: &'r mut BTreeSet<OwnerId>,
    },
}

fn elaborate_with_attributes(
    package: &hir::Package<'_>,
    package_identities: &BTreeMap<u32, u64>,
    protocols: &Protocols,
    target: Option<nir::target::TargetData>,
    inspect_attributes: bool,
    invocation_steps: &mut u64,
) -> Result<(n::Package, Vec<attributes::EvaluatedAttribute>), Error> {
    let mode = if inspect_attributes {
        ElaborationMode::InspectAttributes
    } else {
        ElaborationMode::Full
    };
    elaborate_in_mode(
        package,
        package_identities,
        protocols,
        target,
        mode,
        invocation_steps,
    )
}

fn elaborate_in_mode(
    package: &hir::Package<'_>,
    package_identities: &BTreeMap<u32, u64>,
    protocols: &Protocols,
    target: Option<nir::target::TargetData>,
    mode: ElaborationMode<'_>,
    invocation_steps: &mut u64,
) -> Result<(n::Package, Vec<attributes::EvaluatedAttribute>), Error> {
    if package
        .pending_semantics
        .iter()
        .any(|(_, feature)| feature == "function template expansion")
    {
        let arena = hir::HirArena::new();
        let expanded = function_templates::expand(package, &arena)?;
        return elaborate_in_mode(
            &expanded,
            package_identities,
            protocols,
            target,
            mode,
            invocation_steps,
        );
    }
    if matches!(mode, ElaborationMode::Full)
        && package
            .pending_semantics
            .iter()
            .any(|(_, feature)| feature == "derivation expansion")
    {
        let mut resolved_defaults = BTreeMap::new();
        let mut validated_deriver_attrs = BTreeSet::new();
        let has_unselected_derive = package.owners().any(|(_, info)| {
            matches!(
                &info.node.expect_item().kind,
                ItemKind::Derive(request) if request.strategy.is_none()
            )
        });
        let has_derivation_attrs = package.definition_attributes.iter().any(|attribute| {
            let target_owner = match attribute.target {
                hir::AttributeTarget::Definition(owner) => owner,
                hir::AttributeTarget::Member(id) => id.owner,
            };
            matches!(
                package.item(target_owner).map(|item| &item.kind),
                Some(ItemKind::Deriver(_) | ItemKind::Derive(_))
            )
        });
        if has_unselected_derive || has_derivation_attrs {
            elaborate_in_mode(
                package,
                package_identities,
                protocols,
                target.clone(),
                ElaborationMode::ResolveDerivationDefaults {
                    resolved_defaults: &mut resolved_defaults,
                    validated_deriver_attrs: &mut validated_deriver_attrs,
                },
                invocation_steps,
            )?;
        }
        let arena = hir::HirArena::new();
        let expanded = derivation::expand(
            package,
            &arena,
            &resolved_defaults,
            &validated_deriver_attrs,
            &mut 100_000,
        )
        .map_err(|failure| derivation::with_context(package, failure))?;
        return elaborate_in_mode(
            &expanded,
            package_identities,
            protocols,
            target,
            ElaborationMode::Full,
            invocation_steps,
        )
        .map_err(|failure| derivation::with_context(package, failure));
    }
    let allow_derivation_pending = !matches!(mode, ElaborationMode::Full);
    if let Some((span, feature)) = package.pending_semantics.iter().find(|(_, feature)| {
        feature != "test template CMTT expansion"
            && !(allow_derivation_pending && feature == "derivation expansion")
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
    if matches!(mode, ElaborationMode::Full) && package.definition_attributes.is_empty() {
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
    let source_declarations = source_declarations::SourceDeclarations::new(
        package,
        package_identities,
        invocation_steps,
    )?;
    let mut checker = Checker {
        declared_theory: source_declarations.nominal_context.clone(),
        source_declarations,
        definition_identities: package
            .next_definition_index()
            .max(u32::try_from(package.num_defs()).unwrap_or(u32::MAX))
            .into(),
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
        closed_const_regions: BTreeMap::new(),
        open_instantiation_depth: 0,
        open_instance: None,
        open_instance_capabilities: None,
        generated_functions: Default::default(),
        ordinary_behavior_functions: Vec::new(),
        checking_open_behavior: false,
        checking_default_behavior: false,
        default_behavior_functions: BTreeSet::new(),
        describing_open_spread: false,
        describing_open_call: false,
        ordinary_behavior_inputs: LocalBindings::default(),
        open_loop_transfers: BTreeMap::new(),
        pending_default_stability: Vec::new(),
        pending_predicate_stability: Vec::new(),
        active_signatures: BTreeSet::new(),
        refinements: Default::default(),
        definition_refinements: Default::default(),
        refinement_subjects: Vec::new(),
        pending_refinements: Vec::new(),
        executable_refinement_owner: None,
        interface_signatures: BTreeMap::new(),
        self_types: BTreeMap::new(),
        self_containers: BTreeMap::new(),
        interfaces_ready: false,
        member_phases: BTreeMap::new(),
        packed_structs: BTreeSet::new(),
        direct_passing_effects: BTreeMap::new(),
        definition_attributes: BTreeMap::new(),
        evaluating_attributes: false,
        attributes_prepared: false,
        allow_packed_pointer_projection: false,
        active_early_inherent: BTreeSet::new(),
        checking_method_contract: false,
        pending_contract_projections: Vec::new(),
        checked_traits: Default::default(),
        checked_implementations: Vec::new(),
        implementation_positions: BTreeMap::new(),
        associated_initializers: BTreeMap::new(),
        implementation_relationships: BTreeMap::new(),
        complementary_associated: Vec::new(),
        active_associated_values: BTreeSet::new(),
        selected_associated_initializer: None,
        active_associated_signatures: BTreeSet::new(),
        declaration_projection_depth: 0,
        associated_definition_equations: BTreeMap::new(),
        associated_signature_values: BTreeMap::new(),
        implementation_index: crate::implementation::Index::for_builtin_packages(
            target,
            protocols.builtin_implementation_packages.iter().copied(),
        ),
        trait_parameter_types: BTreeMap::new(),
        parameter_domain_owners: std::collections::HashMap::new(),
        nominal_input_prefixes: BTreeMap::new(),
        inherent_index: crate::implementation::InherentIndex::for_builtin_packages(
            target,
            protocols.builtin_implementation_packages.iter().copied(),
        ),
        type_parameters: BTreeMap::new(),
        normal_parameters: BTreeMap::new(),
        normal_input_prefixes: BTreeMap::new(),
        lexical_input_domains: BTreeMap::new(),
        lexical_input_plans: BTreeMap::new(),
        lexical_source_protocols: lexical_inputs::SourceProtocolIndex::new(package),
        lexical_dependency_steps: usize::try_from(default_elaboration_steps(
            package_identities.len(),
        ))
        .unwrap_or(usize::MAX),
        callable_type_inputs: BTreeMap::new(),
        open_staged_locals: BTreeSet::new(),
        open_member_locals: BTreeSet::new(),
        signature_domains: BTreeMap::new(),
        default_input_scope: None,
        default_input_owner: None,
        requirements: BTreeMap::new(),
        result_guarantees: BTreeMap::new(),
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
        locals: LocalBindings::default(),
        code_values: LocalBindings::default(),
        template_code_values: BTreeMap::new(),
        pattern_codes: LocalBindings::default(),
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
        alias_substitution: DeclarationBindings::default(),
        alias_value_bindings: DeclarationBindings::default(),
        normal_default_bindings: DeclarationBindings::default(),
        definition_inputs: BTreeMap::new(),
        prepared_template_applications: BTreeSet::new(),
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
        input_facts: Default::default(),
        required_steps: *invocation_steps,
        member_sources: Default::default(),
        in_required_comptime: false,
        in_staged_input: false,
        preparing_inputs: None,
        pending_spread_inputs: BTreeMap::new(),
        observing_type: false,
        control: control::Context::default(),
        optional_branch_probe_budget: 100_000,
        probing_optional_branch: false,
        optional_target_probe: None,
        effect_environment: None,
        effect_resume_result: None,
        preserve_effect_value: false,
        captures: Vec::new(),
        structs: BTreeMap::new(),
        newtypes: BTreeMap::new(),
        alias_contracts: BTreeMap::new(),
        enums: BTreeMap::new(),
        enum_sources: BTreeMap::new(),
        field_defaults: SourceBindings::default(),
        active_defaults: BTreeSet::new(),
        immutable: BTreeSet::new(),
        next_runtime_id: 1,
        runtime_domains: BTreeMap::new(),
        global_runtime_origin_ids: std::collections::HashMap::new(),
        global_runtime_deliveries: BTreeMap::new(),
        local_runtime_origins: Default::default(),
        local_runtime_deliveries: BTreeMap::new(),
        local_write_versions: BTreeMap::new(),
        active_env_parameters: BTreeMap::new(),
    };
    // Account even on unwinding, then preserve the original panic. Candidate
    // state remains private; this does not recover or reuse a panicking checker.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        elaborate_candidate(&mut checker, package, package_identities, target, mode)
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
    mode: ElaborationMode<'_>,
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
            ItemKind::Deriver(def) => def.clause_params,
            ItemKind::Derive(req) => req.clause_params,
            ItemKind::Trait(def) => def.clause_params,
            ItemKind::Associated(def) => def.clause_params,
            ItemKind::Effect(sig, _) => sig.clause_params,
            ItemKind::EffectGroup(group) => group.clause_params,
            ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig) => sig.clause_params,
            _ => continue,
        };
        let parameters = clauses
            .iter()
            .filter_map(|p| match *p.kind.unquoted() {
                hir::clause::ClauseParamKind::Type(_)
                | hir::clause::ClauseParamKind::TraitBound(..)
                | hir::clause::ClauseParamKind::Optional(..) => {
                    p.definition.map(|id| DeclarationId {
                        package: semantic_package(package_identities, id.pkg),
                        local: id.index,
                    })
                }
                hir::clause::ClauseParamKind::Positional(..)
                | hir::clause::ClauseParamKind::Varadic(..)
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
            ItemKind::Deriver(def) => Some(def.clause_params),
            ItemKind::Derive(req) => Some(req.clause_params),
            ItemKind::Trait(def) => Some(def.clause_params),
            ItemKind::EffectGroup(group) => Some(group.clause_params),
            _ => None,
        };
        if let Some(clauses) = clauses {
            let mut parameters = Vec::new();
            let mut trait_domains = Vec::new();
            for parameter in clauses {
                let is_quote = parameter.kind.is_quote();
                let unquoted = parameter.kind.unquoted();
                let trait_domain = if matches!(
                    info.node.expect_item().kind,
                    ItemKind::Trait(_)
                        | ItemKind::Struct(_)
                        | ItemKind::Enum(_)
                        | ItemKind::Newtype(..)
                        | ItemKind::TypeAlias(..)
                        | ItemKind::Impl(_)
                        | ItemKind::Deriver(_)
                        | ItemKind::Derive(_)
                        | ItemKind::EffectGroup(_)
                ) {
                    match *unquoted {
                        hir::clause::ClauseParamKind::Positional(_, ty)
                        | hir::clause::ClauseParamKind::Optional(_, ty, _)
                        | hir::clause::ClauseParamKind::Varadic(_, ty) => {
                            Some(checker.type_expr(ty)?)
                        }
                        _ => Some(p(P::Type)),
                    }
                } else {
                    None
                };
                if let Some(domain) = &trait_domain {
                    if is_quote {
                        if !matches!(
                            info.node.expect_item().kind,
                            ItemKind::Struct(_)
                                | ItemKind::Enum(_)
                                | ItemKind::Newtype(..)
                                | ItemKind::TypeAlias(..)
                                | ItemKind::Trait(_)
                                | ItemKind::Impl(_)
                        ) || !matches!(
                            unquoted,
                            hir::clause::ClauseParamKind::Positional(..)
                                | hir::clause::ClauseParamKind::Optional(..)
                        ) {
                            return Err(error(
                                parameter.span,
                                "quote clause parameter is only supported on runtime nominal, alias, trait, or impl positional/optional clauses",
                            ));
                        }
                        if !matches!(domain, Term::Lift(_)) {
                            return Err(error(
                                parameter.span,
                                "quote clause parameter requires a lift type domain",
                            ));
                        }
                    } else if matches!(domain, Term::Lift(_)) {
                        return Err(error(
                            parameter.span,
                            "lift type domain requires a quote clause parameter",
                        ));
                    }
                    trait_domains.push(domain.clone());
                }
                let optional_type_parameter = !is_quote
                    && if let hir::clause::ClauseParamKind::Optional(_, ty, _) = *unquoted {
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
                let explicit_type_parameter = !is_quote
                    && if let hir::clause::ClauseParamKind::Positional(_, ty) = *unquoted {
                        checker.type_expr(ty)? == p(P::Type)
                    } else {
                        false
                    };
                if !explicit_type_parameter
                    && !optional_type_parameter
                    && !(trait_domain.is_some()
                        && matches!(
                            *unquoted,
                            hir::clause::ClauseParamKind::Positional(..)
                                | hir::clause::ClauseParamKind::Optional(..)
                                | hir::clause::ClauseParamKind::Varadic(..)
                        ))
                    && !matches!(
                        *unquoted,
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
                for &id in &checker.type_parameters[&owner] {
                    checker.parameter_domain_owners.entry(id).or_insert(owner);
                }
                checker.trait_parameter_types.insert(owner, trait_domains);
            }
            if matches!(
                info.node.expect_item().kind,
                ItemKind::Struct(_) | ItemKind::Enum(_)
            ) {
                let self_ty = checker.nominal_self_type(owner);
                checker.self_types.insert(owner, self_ty.clone());
                let nested = match &info.node.expect_item().kind {
                    ItemKind::Struct(def) => &def.nested_items,
                    ItemKind::Enum(def) => &def.nested_items,
                    _ => unreachable!(),
                };
                for member in nested {
                    if checker.self_types.contains_key(member) {
                        checker.self_types.insert(*member, self_ty.clone());
                    }
                }
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
    let inspect_attributes = match mode {
        ElaborationMode::InspectAttributes => true,
        ElaborationMode::Full => false,
        ElaborationMode::ResolveDerivationDefaults {
            resolved_defaults,
            validated_deriver_attrs,
        } => {
            checker.resolve_derivation_defaults(resolved_defaults, validated_deriver_attrs)?;
            return Ok((
                n::Package {
                    target,
                    ..Default::default()
                },
                Vec::new(),
            ));
        }
    };
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
    checker.checked_implementations = checker.check_interfaces()?;
    checker.implementation_positions = checker
        .checked_implementations
        .iter()
        .enumerate()
        .map(|(index, implementation)| (implementation.declaration, index))
        .collect();
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
    for interface in checker.checked_traits.iter() {
        for method in &interface.methods {
            if let Some(member) = checker.source_declarations.owner(method.declaration) {
                checker
                    .requirements
                    .entry(member)
                    .or_default()
                    .extend(interface.requirements.clone());
            }
        }
    }
    if let Some(id) = checker.protocols.splice {
        let span = checker
            .source_declarations
            .item(id)
            .map(|item| item.span)
            .unwrap_or(rustc_span::DUMMY_SP);
        crate::splice::protocol(&checker.checked_traits, id)
            .map_err(|reason| error(span, reason))?;
    }
    checker.member_sources = required::MemberSources::capture(checker)?;
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
    if let Some(id) = checker.protocols.comptime_list_pattern {
        let contract = crate::list_patterns::protocol(&checker.checked_traits, id)
            .map_err(|reason| error(rustc_span::DUMMY_SP, reason))?;
        if !contract.comptime {
            return Err(error(
                rustc_span::DUMMY_SP,
                "comptime list pattern protocol requires comptime methods",
            ));
        }
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
                if let ItemKind::Fn(..) = item.kind {
                    functions.push((owner, item, sig));
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
                    checker.completed_alias(owner, value, &[], item.span)?;
                } else {
                    checker.check_alias_definition(owner, value, item.span)?;
                }
            }
            ItemKind::Const(..) => {
                if checker
                    .compiler_operation_binding(owner, item.span)?
                    .is_some()
                {
                    // Validate binding annotations now. Fixed callable bodies
                    // are materialized on demand by ordinary value lookup.
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
    checker.prepare_staged_forwarders(functions.iter().map(|(owner, _, _)| *owner))?;
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
                abi: if checker
                    .direct_passing_effects
                    .get(&owner)
                    .copied()
                    .unwrap_or(modifiers.is_direct_passing)
                {
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
    output.traits = checker.checked_traits.to_vec();
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
    for (owner, item, sig) in functions {
        if checker.open_functions.contains_key(&owner) {
            continue;
        }
        let signature = checker.signatures[&owner].clone();
        let function = checker.check_source_function(
            owner,
            signature,
            function_bodies::SourceBody::Package,
            item.span,
        )?;
        if sig.modifiers.is_function_template && !sig.modifiers.is_pure {
            continue;
        }
        if sig.modifiers.is_function_template {
            checker.ordinary_behavior_functions.push(function);
        } else {
            output.functions.push(function);
        }
    }
    let attributes = prepared_attributes;
    checker.expand_test_templates(&mut output)?;
    // Required invocations in bodies, defaults and templates can publish types
    // after the initial layout snapshot. Consumers below need their complete
    // descriptions for contract, purity and quote analysis, not just emission.
    fn append_descriptions<'a, T: Clone + 'a>(
        output: &mut Vec<T>,
        descriptions: impl IntoIterator<Item = &'a T>,
        identity: impl Fn(&T) -> DeclarationId,
    ) {
        let mut emitted: BTreeSet<_> = output.iter().map(&identity).collect();
        for description in descriptions {
            if emitted.insert(identity(description)) {
                output.push(description.clone());
            }
        }
    }
    append_descriptions(&mut output.structs, checker.structs.values(), |item| {
        item.declaration
    });
    append_descriptions(&mut output.enums, checker.enums.values(), |item| {
        item.declaration
    });
    append_descriptions(&mut output.newtypes, checker.newtypes.values(), |item| {
        item.declaration
    });
    // Internal scalar packets use the same description publication boundary;
    // their return-transport role is metadata, not a separate enum emission path.
    output
        .internal_return_packets
        .extend(checker.computed_scalar_packets.iter().copied());
    // Staged declaration values can remain in checked default descriptions and
    // invocation inputs even though only concrete residual bodies are emitted.
    // Identity metadata admits their phase; it never authorizes execution.
    for owner in checker.open_functions.keys().copied() {
        if !matches!(&package.item(owner).unwrap().kind, ItemKind::Fn(sig, _)
            if !sig.modifiers.is_comptime && !sig.modifiers.is_normal_form_head)
        {
            continue;
        }
        if let Some(signature) = checker
            .declared_signature(owner)
            .map(|sig| sig.into_owned())
        {
            output
                .runtime_function_identities
                .push(checker.runtime_function_identity(owner, signature));
        }
    }
    output.function_parameters = checker.function_parameter_metadata();
    output
        .functions
        .extend(std::mem::take(&mut checker.generated_functions).into_bodies());
    output.alias_contracts = checker.alias_contracts.values().cloned().collect();
    output.refinements = std::mem::take(&mut checker.refinements);
    output.expansion_sources = std::mem::take(&mut checker.expansion_sources);
    output.definition_attributes = std::mem::take(&mut checker.definition_attributes);
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
                retry_after_capability_progress: false,
                incompatible_context: false,
            }
        })?;
    // Definition-side witnesses are analysis-only, including suppliers whose
    // inline alternatives must be checked before their inputs are supplied.
    output.runtime_domains = checker.runtime_domains.clone();
    output.global_runtime_deliveries = checker.global_runtime_deliveries.clone();
    purity::with_definition_bodies(
        &mut output,
        &mut checker.ordinary_behavior_functions,
        |output| {
            purity::validate_defaults(
                output,
                &checker.pending_default_stability,
                &checker.protocols,
            )?;
            purity::validate_predicates(
                output,
                &checker.pending_predicate_stability,
                &checker.protocols,
            )
        },
    )?;
    cmtt::expand_quote_functions(
        &mut output,
        package,
        package_identities,
        &checker.protocols,
        &mut checker.next_temporary_binding,
        &mut checker.required_steps,
    )?;
    output.runtime_domains = checker.runtime_domains.clone();
    output.global_runtime_deliveries = checker.global_runtime_deliveries.clone();
    output.quote_identity_cursor = checker.next_temporary_binding;
    output.definition_identity_cursor = checker.definition_identities.next();
    if output
        .implementations
        .iter()
        .any(|i| !i.type_parameters.is_empty() || !i.requirements.is_empty())
    {
        let default_dispatch_fuel = crate::ptc::Budget::default()
            .steps
            .saturating_add(output.functions.len().saturating_mul(1_000))
            .saturating_add(output.implementations.len().saturating_mul(1_000))
            .min(16_000_000);
        output.dispatch_instances = crate::materialization::dispatch_instances(
            &output,
            crate::ptc::Budget {
                steps: checker
                    .protocols
                    .dispatch_fuel
                    .unwrap_or(default_dispatch_fuel),
                ..crate::ptc::Budget::default()
            },
        )
        .map_err(|message| error(Span::default(), message))?;
    }
    // Ordinary behavior witnesses cover all inline branches but are not
    // executable artifacts: their comptime inputs remain abstract.
    purity::with_definition_bodies(
        &mut output,
        &mut checker.ordinary_behavior_functions,
        |output| purity::validate_runtime_promises(output, &checker.protocols),
    )?;
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
            retry_after_capability_progress: false,
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

impl Checker<'_, '_> {
    fn resolve_derivation_defaults(
        &mut self,
        resolved_defaults: &mut BTreeMap<OwnerId, OwnerId>,
        validated_deriver_attrs: &mut BTreeSet<OwnerId>,
    ) -> Result<(), Error> {
        let evaluated = self.evaluate_attributes_filtered(|attribute| {
            let hir::AttributeTarget::Definition(owner) = attribute.target else {
                return false;
            };
            matches!(
                self.package.item(owner).map(|item| &item.kind),
                Some(ItemKind::Deriver(_) | ItemKind::Derive(_))
            )
        })?;
        let markers = if evaluated.is_empty() {
            Vec::new()
        } else {
            let mut budget = comptime_runtime::Budget {
                steps: usize::try_from(self.required_steps).unwrap_or(usize::MAX),
                ..Default::default()
            };
            let before = budget.steps;
            let result = (|| {
                let selected = attributes::merge(&evaluated, 1024, &mut budget)?;
                let markers =
                    attributes::default_strategy_markers(self.package, &selected, &mut budget)?;
                for target in &selected {
                    let hir::AttributeTarget::Definition(owner) = target.target else {
                        continue;
                    };
                    if self
                        .package
                        .item(owner)
                        .is_some_and(|item| matches!(item.kind, ItemKind::Deriver(_)))
                        && target.iter().all(|(key, _)| key == "derivation_default")
                    {
                        validated_deriver_attrs.insert(owner);
                    }
                }
                Ok(markers)
            })();
            self.required_steps = self
                .required_steps
                .saturating_sub((before - budget.steps) as u64);
            result.map_err(|failure: attributes::MergeFailure| {
                let mut diagnostic = error(Span::default(), failure.message);
                diagnostic.source = failure.source;
                diagnostic
            })?
        };

        let mut registry = crate::derivation::Registry::default();
        let mut strategy_owners = BTreeMap::new();
        let mut strategy_params: BTreeMap<DeclarationId, Vec<DeclarationId>> = BTreeMap::new();
        for marker in markers {
            if !marker.enabled {
                continue;
            }
            let Some(item) = self.package.item(marker.owner) else {
                continue;
            };
            let ItemKind::Deriver(generator) = &item.kind else {
                continue;
            };
            self.ensure_contract(marker.owner, item.span)?;
            self.signature_owner = Some(marker.owner);
            self.context = self.definition_context(marker.owner);
            let instance = self.interface_expr_optional_subject(generator.trait_ref, None)?;
            let decl = declaration(self.package, &self.package_identities, marker.owner);
            registry
                .register(
                    crate::derivation::DefaultStrategy {
                        declaration: decl,
                        interface: instance.declaration,
                        interface_arguments: instance.arguments,
                    },
                    crate::ptc::Budget::default(),
                )
                .map_err(|failure| {
                    let message = match failure {
                        crate::implementation::Error::Ownership(
                            crate::implementation::OwnershipError::NoLocalIdentity,
                        ) => "default derivation strategy requires a local trait or trait argument",
                        crate::implementation::Error::Ownership(
                            crate::implementation::OwnershipError::OpenBeforeLocalIdentity,
                        ) => "default derivation strategy trait arguments must establish a local identity before open parameters",
                        crate::implementation::Error::Ownership(
                            crate::implementation::OwnershipError::InherentRequiresLocalNominal,
                        ) => "default derivation strategy ownership requires a trait or nominal identity",
                        crate::implementation::Error::DuplicateDeclaration(_) => {
                            "conflicting default derivation strategy registration"
                        }
                        crate::implementation::Error::BudgetExhausted => {
                            "default derivation strategy registration budget exhausted"
                        }
                        crate::implementation::Error::InvalidPhaseHead(_)
                        | crate::implementation::Error::AmbiguousPhaseRelationship
                        | crate::implementation::Error::UndeterminedPhaseRelationship(_) => {
                            "invalid default derivation strategy ownership"
                        }
                    };
                    let mut diagnostic = error(Span::default(), message);
                    diagnostic.source = marker.source;
                    diagnostic
                })?;
            strategy_owners.insert(decl, marker.owner);
            strategy_params.insert(
                decl,
                self.type_parameters
                    .get(&marker.owner)
                    .cloned()
                    .unwrap_or_default(),
            );
        }

        let mut select_fuel = 100_000usize;
        for (request_owner, info) in self.package.owners() {
            let item = info.node.expect_item();
            let ItemKind::Derive(request) = &item.kind else {
                continue;
            };
            if request.strategy.is_some() {
                continue;
            }
            self.ensure_contract(request_owner, item.span)?;
            self.signature_owner = Some(request_owner);
            self.context = self.definition_context(request_owner);
            let subject = self.type_expr(request.self_ty)?;
            let requested = self.interface_expr_for(request.trait_ref, &subject)?;
            let visible: BTreeSet<DeclarationId> = self
                .package
                .derive_visible_strategies
                .get(&request_owner)
                .into_iter()
                .flat_map(|defs| defs.iter())
                .filter_map(|def_id| self.package.definition_owner(*def_id))
                .map(|owner| declaration(self.package, &self.package_identities, owner))
                .collect();
            let selection = registry
                .select(
                    requested.declaration,
                    &visible,
                    &mut select_fuel,
                    |candidate, fuel| {
                        if candidate.interface_arguments.len() != requested.arguments.len() {
                            return Ok(crate::derivation::Applicability::Rejected);
                        }
                        if candidate.interface_arguments == requested.arguments {
                            return Ok(crate::derivation::Applicability::Applicable);
                        }
                        let params = strategy_params
                            .get(&candidate.declaration)
                            .map(Vec::as_slice)
                            .unwrap_or(&[]);
                        if params.is_empty() {
                            return Ok(crate::derivation::Applicability::Rejected);
                        }
                        let mut bindings = BTreeMap::new();
                        if match_default_strategy_pairs(
                            &candidate.interface_arguments,
                            &requested.arguments,
                            params,
                            &mut bindings,
                            fuel,
                        )? {
                            Ok(crate::derivation::Applicability::Applicable)
                        } else {
                            Ok(crate::derivation::Applicability::Rejected)
                        }
                    },
                )
                .map_err(|failure| {
                    let message = match failure {
                        crate::implementation::Error::BudgetExhausted => {
                            "default derivation strategy selection budget exhausted"
                        }
                        _ => "default derivation strategy selection failed",
                    };
                    error(item.span, message)
                })?;
            match selection {
                crate::derivation::Selection::Unique(decl) => {
                    resolved_defaults.insert(request_owner, strategy_owners[&decl]);
                }
                crate::derivation::Selection::Missing => {
                    return Err(error(
                        item.span,
                        "no applicable default derivation strategy found",
                    ));
                }
                crate::derivation::Selection::Ambiguous(_) => {
                    return Err(error(
                        item.span,
                        "ambiguous default derivation strategies for trait",
                    ));
                }
                crate::derivation::Selection::Pending { .. } => {
                    return Err(error(
                        item.span,
                        "default derivation strategy applicability could not be resolved",
                    ));
                }
            }
        }
        Ok(())
    }
}

fn match_default_strategy_pairs(
    patterns: &[Term],
    actuals: &[Term],
    params: &[DeclarationId],
    bindings: &mut BTreeMap<DeclarationId, Term>,
    fuel: &mut usize,
) -> Result<bool, crate::implementation::Error> {
    if patterns.len() != actuals.len() {
        return Ok(false);
    }
    for (pattern, actual) in patterns.iter().zip(actuals) {
        if !match_default_strategy_term(pattern, actual, params, bindings, fuel)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn match_default_strategy_term(
    pattern: &Term,
    actual: &Term,
    params: &[DeclarationId],
    bindings: &mut BTreeMap<DeclarationId, Term>,
    fuel: &mut usize,
) -> Result<bool, crate::implementation::Error> {
    *fuel = fuel
        .checked_sub(1)
        .ok_or(crate::implementation::Error::BudgetExhausted)?;
    if let Term::Rigid { declaration, sort } = pattern
        && params.contains(declaration)
    {
        if actual.sort() != *sort {
            return Ok(false);
        }
        return match bindings.get(declaration) {
            Some(previous) => Ok(previous == actual),
            None => {
                bindings.insert(*declaration, actual.clone());
                Ok(true)
            }
        };
    }
    match (pattern, actual) {
        (
            Term::Nominal {
                declaration: a_decl,
                arguments: a_args,
            },
            Term::Nominal {
                declaration: b_decl,
                arguments: b_args,
            },
        ) => Ok(a_decl == b_decl
            && match_default_strategy_pairs(a_args, b_args, params, bindings, fuel)?),
        (
            Term::NormalForm {
                head: a_head,
                sort: a_sort,
                arguments: a_args,
            },
            Term::NormalForm {
                head: b_head,
                sort: b_sort,
                arguments: b_args,
            },
        ) => Ok(a_head == b_head
            && a_sort == b_sort
            && match_default_strategy_pairs(a_args, b_args, params, bindings, fuel)?),
        (Term::Tuple(a), Term::Tuple(b))
        | (Term::TupleValue(a), Term::TupleValue(b))
        | (Term::ClosureContextValue(a), Term::ClosureContextValue(b)) => {
            match_default_strategy_pairs(a, b, params, bindings, fuel)
        }
        (
            Term::ListConcat {
                ty: a_ty,
                parts: a_parts,
            },
            Term::ListConcat {
                ty: b_ty,
                parts: b_parts,
            },
        ) => Ok(
            match_default_strategy_term(a_ty, b_ty, params, bindings, fuel)?
                && match_default_strategy_pairs(a_parts, b_parts, params, bindings, fuel)?,
        ),
        (Term::Record(a), Term::Record(b)) | (Term::RecordValue(a), Term::RecordValue(b)) => {
            if a.len() != b.len() {
                return Ok(false);
            }
            for ((a_name, a_val), (b_name, b_val)) in a.iter().zip(b) {
                if a_name != b_name
                    || !match_default_strategy_term(a_val, b_val, params, bindings, fuel)?
                {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (
            Term::NominalValue {
                ty: a_ty,
                variant: a_var,
                fields: a_fields,
            },
            Term::NominalValue {
                ty: b_ty,
                variant: b_var,
                fields: b_fields,
            },
        ) => Ok(a_var == b_var
            && match_default_strategy_term(a_ty, b_ty, params, bindings, fuel)?
            && match_default_strategy_pairs(a_fields, b_fields, params, bindings, fuel)?),
        (
            Term::OptionalValue {
                ty: a_ty,
                value: a_val,
            },
            Term::OptionalValue {
                ty: b_ty,
                value: b_val,
            },
        ) => {
            if !match_default_strategy_term(a_ty, b_ty, params, bindings, fuel)? {
                return Ok(false);
            }
            match (a_val, b_val) {
                (None, None) => Ok(true),
                (Some(a), Some(b)) => match_default_strategy_term(a, b, params, bindings, fuel),
                _ => Ok(false),
            }
        }
        (
            Term::ErrorValue {
                ty: a_ty,
                label: a_lbl,
                value: a_val,
            },
            Term::ErrorValue {
                ty: b_ty,
                label: b_lbl,
                value: b_val,
            },
        ) => {
            if !match_default_strategy_term(a_ty, b_ty, params, bindings, fuel)?
                || !match_default_strategy_term(a_val, b_val, params, bindings, fuel)?
            {
                return Ok(false);
            }
            match (a_lbl, b_lbl) {
                (None, None) => Ok(true),
                (Some(a), Some(b)) => match_default_strategy_term(a, b, params, bindings, fuel),
                _ => Ok(false),
            }
        }
        (Term::RuntimeField { base: a, index: i }, Term::RuntimeField { base: b, index: j }) => {
            Ok(i == j && match_default_strategy_term(a, b, params, bindings, fuel)?)
        }
        (Term::Optional(a), Term::Optional(b))
        | (Term::Pointer(a), Term::Pointer(b))
        | (Term::Lift(a), Term::Lift(b))
        | (Term::ComptimeFunction(a), Term::ComptimeFunction(b))
        | (Term::UnsafeFunction(a), Term::UnsafeFunction(b)) => {
            match_default_strategy_term(a, b, params, bindings, fuel)
        }
        (Term::NormalFormFunction(a_sig, a_idx), Term::NormalFormFunction(b_sig, b_idx)) => Ok(
            a_idx == b_idx && match_default_strategy_term(a_sig, b_sig, params, bindings, fuel)?,
        ),
        (
            Term::ErrorResult {
                errors: a_err,
                result: a_res,
            },
            Term::ErrorResult {
                errors: b_err,
                result: b_res,
            },
        ) => Ok(
            match_default_strategy_pairs(a_err, b_err, params, bindings, fuel)?
                && match_default_strategy_term(a_res, b_res, params, bindings, fuel)?,
        ),
        (
            Term::Computation {
                effects: a_eff,
                result: a_res,
            },
            Term::Computation {
                effects: b_eff,
                result: b_res,
            },
        ) => Ok(
            a_eff == b_eff && match_default_strategy_term(a_res, b_res, params, bindings, fuel)?
        ),
        (
            Term::AssociatedType {
                origin: a_orig,
                arguments: a_args,
                subject: a_subj,
            },
            Term::AssociatedType {
                origin: b_orig,
                arguments: b_args,
                subject: b_subj,
            },
        ) => Ok(a_orig == b_orig
            && match_default_strategy_pairs(a_args, b_args, params, bindings, fuel)?
            && match_default_strategy_term(a_subj, b_subj, params, bindings, fuel)?),
        (
            Term::ContextualClosure {
                context: a_ctx,
                signature: a_sig,
            },
            Term::ContextualClosure {
                context: b_ctx,
                signature: b_sig,
            },
        ) => Ok(
            match_default_strategy_term(a_ctx, b_ctx, params, bindings, fuel)?
                && match_default_strategy_term(a_sig, b_sig, params, bindings, fuel)?,
        ),
        (
            Term::Closure {
                environment: a_env,
                signature: a_sig,
            },
            Term::Closure {
                environment: b_env,
                signature: b_sig,
            },
        ) => Ok(
            match_default_strategy_pairs(a_env, b_env, params, bindings, fuel)?
                && match_default_strategy_term(a_sig, b_sig, params, bindings, fuel)?,
        ),
        (
            Term::Function {
                parameters: a_params,
                result: a_res,
            },
            Term::Function {
                parameters: b_params,
                result: b_res,
            },
        ) => {
            if a_params.len() != b_params.len() {
                return Ok(false);
            }
            for (ap, bp) in a_params.iter().zip(b_params) {
                if ap.sort != bp.sort
                    || !match_default_strategy_term(&ap.ty, &bp.ty, params, bindings, fuel)?
                {
                    return Ok(false);
                }
            }
            match_default_strategy_term(a_res, b_res, params, bindings, fuel)
        }
        (
            Term::FunctionValue {
                declaration: a_decl,
                arguments: a_args,
            },
            Term::FunctionValue {
                declaration: b_decl,
                arguments: b_args,
            },
        ) => Ok(a_decl == b_decl
            && match_default_strategy_pairs(a_args, b_args, params, bindings, fuel)?),
        (
            Term::ClosureSnapshot {
                factory: a_fac,
                arguments: a_args,
            },
            Term::ClosureSnapshot {
                factory: b_fac,
                arguments: b_args,
            },
        ) => {
            Ok(a_fac == b_fac
                && match_default_strategy_pairs(a_args, b_args, params, bindings, fuel)?)
        }
        (Term::MethodValue(a), Term::MethodValue(b)) => Ok(a.member == b.member
            && a.interface.declaration == b.interface.declaration
            && match_default_strategy_pairs(
                &a.interface.arguments,
                &b.interface.arguments,
                params,
                bindings,
                fuel,
            )?
            && match_default_strategy_term(&a.subject, &b.subject, params, bindings, fuel)?
            && match_default_strategy_pairs(
                &a.method_arguments,
                &b.method_arguments,
                params,
                bindings,
                fuel,
            )?),
        (
            Term::Primitive(_)
            | Term::ClosureContextType
            | Term::Hole { .. }
            | Term::Bound { .. }
            | Term::Rigid { .. }
            | Term::Runtime(_)
            | Term::Unit
            | Term::Bool(_)
            | Term::Char(_)
            | Term::BitVec(_)
            | Term::Float16(_)
            | Term::BFloat16(_)
            | Term::Float32(_)
            | Term::Float64(_)
            | Term::String(_)
            | Term::Symbol(_)
            | Term::Integer(_)
            | Term::Float(_)
            | Term::Value { .. },
            _,
        ) => Ok(pattern == actual),
        _ => Ok(false),
    }
}

#[derive(Clone)]
struct CaptureFrame {
    const_regions: ClosureConstRegions,
    site: n::LocalId,
    addresses: BTreeMap<nir::RuntimeId, n::LocalId>,
    comptime: bool,
    outer: BTreeSet<n::LocalId>,
    values: Vec<n::Capture>,
}

#[derive(Clone, Debug)]
struct FormationInputs {
    terms: Vec<Term>,
    // Executable type constructors retain deferred contracts for invocation.
    deferred: BTreeSet<DeclarationId>,
    environment: DeclarationBindings<Term>,
}

impl FormationInputs {
    fn ready(terms: &[Term]) -> Self {
        Self {
            terms: terms.to_vec(),
            deferred: BTreeSet::new(),
            environment: DeclarationBindings::default(),
        }
    }
}

struct OldScope {
    owner: OwnerId,
    active: bool,
    bindings: BTreeMap<n::LocalId, n::LocalId>,
}

#[derive(Clone)]
enum GeneratorCell {
    Owned(comptime_interpretor::Storage),
    Reference(comptime_interpretor::ValueReference),
}

impl GeneratorCell {
    fn owned(value: comptime_interpretor::Value) -> Self {
        Self::Owned(comptime_interpretor::Storage::new(value))
    }

    fn live(value: comptime_interpretor::Value) -> Self {
        match value {
            comptime_interpretor::Value::ComptimeReference(reference) => Self::Reference(reference),
            value => Self::owned(value),
        }
    }

    fn value(&self) -> comptime_interpretor::Value {
        match self {
            Self::Owned(cell) => comptime_interpretor::Value::Storage(cell.clone()),
            Self::Reference(reference) => {
                comptime_interpretor::Value::ComptimeReference(reference.clone())
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CallUse {
    Invocation,
    DefinitionBehavior,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum GlobalRuntimeOrigin {
    DefaultParam {
        owner: OwnerId,
        param: DeclarationId,
        prefix_args: Vec<Term>,
    },
    ConstAddress {
        declaration: DeclarationId,
        ty: Term,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum LocalRuntimeOrigin {
    AddressOf {
        base: n::LocalId,
        closure: Option<n::LocalId>,
    },
    LocalValue {
        binding: n::LocalId,
        epoch: u64,
    },
    Ephemeral {
        occurrence: u32,
        site: n::LocalId,
    },
}

#[derive(Clone, Debug)]
struct LocalRuntimeDelivery {
    address_owner: Option<n::LocalId>,
    producer_site: Option<n::LocalId>,
    expr: n::Expr,
    checked_epoch: Option<(n::LocalId, u64)>,
}

struct Checker<'a, 'h> {
    // Declaration authority only. Lexical premises never enter this shared root.
    declared_theory: crate::ptc::Context,
    source_declarations: source_declarations::SourceDeclarations<'a, 'h>,
    definition_identities: crate::definitions::DefinitionIdentities,
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
    closed_const_regions: BTreeMap<OwnerId, open_functions::CheckedRegions>,
    open_instantiation_depth: usize,
    open_instance: Option<std::sync::Arc<generated_functions::InstanceKey>>,
    open_instance_capabilities: Option<Vec<n::TraitRequirement>>,
    generated_functions: generated_functions::GeneratedFunctions,
    ordinary_behavior_functions: Vec<n::Function>,
    checking_open_behavior: bool,
    checking_default_behavior: bool,
    default_behavior_functions: BTreeSet<OwnerId>,
    describing_open_spread: bool,
    describing_open_call: bool,
    ordinary_behavior_inputs: LocalBindings<Term>,
    open_loop_transfers: BTreeMap<n::LocalId, open_functions::LoopTransfers>,
    pending_default_stability: Vec<purity::StableObservation>,
    pending_predicate_stability: Vec<purity::StableObservation>,
    active_signatures: BTreeSet<OwnerId>,
    refinements: nir::refinements::Refinements,
    definition_refinements: refinements::DefinitionRefinements,
    refinement_subjects: Vec<Term>,
    pending_refinements: Vec<refinements::Pending>,
    executable_refinement_owner: Option<OwnerId>,
    interface_signatures: BTreeMap<OwnerId, Term>,
    self_types: BTreeMap<OwnerId, Term>,
    self_containers: BTreeMap<OwnerId, OwnerId>,
    interfaces_ready: bool,
    member_phases: BTreeMap<OwnerId, n::TraitPhase>,
    packed_structs: BTreeSet<OwnerId>,
    direct_passing_effects: BTreeMap<OwnerId, bool>,
    definition_attributes: BTreeMap<DeclarationId, Term>,
    evaluating_attributes: bool,
    attributes_prepared: bool,
    allow_packed_pointer_projection: bool,
    active_early_inherent: BTreeSet<String>,
    checking_method_contract: bool,
    pending_contract_projections: Vec<interfaces::PendingProjection>,
    checked_traits: std::sync::Arc<interfaces::CheckedTraits>,
    checked_implementations: Vec<n::Implementation>,
    implementation_positions: BTreeMap<DeclarationId, usize>,
    associated_initializers: BTreeMap<(DeclarationId, DeclarationId), interfaces::AssociatedRecipe>,
    implementation_relationships: BTreeMap<DeclarationId, DeclarationId>,
    complementary_associated: Vec<((DeclarationId, DeclarationId), interfaces::AssociatedRecipe)>,
    active_associated_values: BTreeSet<(DeclarationId, DeclarationId)>,
    selected_associated_initializer: Option<(DeclarationId, DeclarationId)>,
    active_associated_signatures: BTreeSet<OwnerId>,
    declaration_projection_depth: usize,
    associated_definition_equations: BTreeMap<DeclarationId, Vec<(Term, Term)>>,
    associated_signature_values: BTreeMap<OwnerId, n::Expr>,
    implementation_index: crate::implementation::Index,
    trait_parameter_types: BTreeMap<OwnerId, Vec<Term>>,
    parameter_domain_owners: std::collections::HashMap<DeclarationId, OwnerId>,
    nominal_input_prefixes: BTreeMap<DeclarationId, Term>,
    inherent_index: crate::implementation::InherentIndex,
    type_parameters: BTreeMap<OwnerId, Vec<DeclarationId>>,
    normal_parameters: BTreeMap<OwnerId, Vec<(DeclarationId, n::LocalId, Term)>>,
    // Active signature prefixes are not completed head metadata and must never
    // be copied into required-invocation packages.
    normal_input_prefixes: BTreeMap<OwnerId, Vec<(DeclarationId, n::LocalId, Term)>>,
    lexical_input_domains: BTreeMap<DeclarationId, Term>,
    lexical_input_plans: BTreeMap<OwnerId, lexical_inputs::LexicalInputPlan>,
    lexical_source_protocols: lexical_inputs::SourceProtocolIndex<'a, 'h>,
    lexical_dependency_steps: usize,
    callable_type_inputs: BTreeMap<n::LocalId, Option<function_types::StagedParameter>>,
    // Description-only provenance; never authorizes executable code capture.
    open_staged_locals: BTreeSet<n::LocalId>,
    // Pending member results, not Type identities or permission evidence.
    open_member_locals: BTreeSet<n::LocalId>,
    signature_domains: BTreeMap<OwnerId, Vec<(Sort, Option<Term>)>>,
    default_input_scope: Option<lexical_domains::RetainedDomains>,
    default_input_owner: Option<OwnerId>,
    requirements: BTreeMap<OwnerId, Vec<n::TypePredicate>>,
    result_guarantees: BTreeMap<OwnerId, (DeclarationId, Vec<n::TypePredicate>)>,
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
    locals: LocalBindings<Term>,
    code_values: LocalBindings<cmtt::ExpressionCode>,
    template_code_values: BTreeMap<(u32, u32), std::sync::Arc<cmtt::ExpressionCode>>,
    pattern_codes: LocalBindings<cmtt::patterns::PatternCode>,
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
    generator_cells: BTreeMap<n::LocalId, (n::Expr, GeneratorCell)>,
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
    completed_aliases:
        std::collections::HashMap<aliases::Application, std::sync::Arc<CompletedType>>,
    checked_alias_definitions: BTreeSet<OwnerId>,
    alias_scopes: BTreeMap<OwnerId, Option<OwnerId>>,
    alias_substitution: DeclarationBindings<Term>,
    alias_value_bindings: DeclarationBindings<n::LocalId>,
    normal_default_bindings: DeclarationBindings<(n::LocalId, Term)>,
    definition_inputs: BTreeMap<DeclarationId, n::Expr>,
    prepared_template_applications: BTreeSet<OwnerId>,
    forbidden_default_inputs: BTreeSet<DeclarationId>,
    next_temporary_binding: u32,
    alias_expression_budget: usize,
    constants: BTreeMap<OwnerId, n::Expr>,
    active_constants: BTreeSet<OwnerId>,
    active_nominals: BTreeSet<OwnerId>,
    active_required: BTreeSet<(n::LocalId, required::RootKind)>,
    active_contracts: BTreeSet<OwnerId>,
    active_function_defaults: BTreeSet<(OwnerId, usize)>,
    active_type_defaults: BTreeSet<(OwnerId, usize)>,
    default_uses:
        BTreeMap<(OwnerId, usize), Vec<(Option<OwnerId>, bool, OwnerId, FormationInputs, Span)>>,
    contract_uses: BTreeMap<OwnerId, Vec<(Option<OwnerId>, bool, OwnerId, FormationInputs, Span)>>,
    required_steps: u64,
    member_sources: required::MemberSources,
    input_facts: input_facts::State,
    in_required_comptime: bool,
    in_staged_input: bool,
    preparing_inputs: Option<n::SourceRange>,
    pending_spread_inputs: BTreeMap<n::LocalId, spreads::PendingPacket>,
    observing_type: bool,
    control: control::Context,
    optional_branch_probe_budget: usize,
    probing_optional_branch: bool,
    optional_target_probe: Option<hir::HirId>,
    effect_environment: Option<Vec<nir::effects::Operation>>,
    effect_resume_result: Option<Term>,
    preserve_effect_value: bool,
    captures: Vec<CaptureFrame>,
    structs: BTreeMap<DeclarationId, n::Struct>,
    newtypes: BTreeMap<DeclarationId, nir::newtypes::Newtype>,
    alias_contracts: BTreeMap<DeclarationId, nir::type_aliases::Contract>,
    enums: BTreeMap<DeclarationId, n::Enum>,
    enum_sources: BTreeMap<DeclarationId, Vec<&'h hir::item::Variant<'h>>>,
    field_defaults: SourceBindings<n::Expr>,
    active_defaults: BTreeSet<(OwnerId, Option<usize>, usize)>,
    immutable: BTreeSet<n::LocalId>,
    next_runtime_id: u32,
    runtime_domains: BTreeMap<nir::RuntimeId, Term>,
    global_runtime_origin_ids: std::collections::HashMap<GlobalRuntimeOrigin, nir::RuntimeId>,
    global_runtime_deliveries: BTreeMap<nir::RuntimeId, n::Expr>,
    local_runtime_origins: runtime_origins::LocalOrigins,
    local_runtime_deliveries: BTreeMap<nir::RuntimeId, LocalRuntimeDelivery>,
    local_write_versions: BTreeMap<n::LocalId, u64>,
    active_env_parameters: BTreeMap<DeclarationId, (n::LocalId, Term)>,
}

impl Checker<'_, '_> {
    fn intern_runtime_id(&mut self, domain: Term, span: Span) -> Result<nir::RuntimeId, Error> {
        let local_id = self.next_runtime_id;
        self.next_runtime_id = local_id
            .checked_add(1)
            .ok_or_else(|| error(span, "runtime dependency identity space exhausted"))?;
        let id = nir::RuntimeId {
            instance: 0,
            local: local_id,
        };
        self.implementation_index
            .register_runtime_domain(id, domain.clone())
            .map_err(|message| error(span, message))?;
        self.inherent_index
            .register_runtime_domain(id, domain.clone())
            .map_err(|message| error(span, message))?;
        self.runtime_domains.insert(id, domain);
        Ok(id)
    }

    fn record_runtime_expr_captures(&mut self, expr: &n::Expr, span: Span) -> Result<(), Error> {
        let mut work = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = work;
        let free = nir::free_bindings::collect(expr, &mut work);
        self.required_steps = self.required_steps.saturating_sub((before - work) as u64);
        let free = free.map_err(|reason| error(span, reason.to_string()))?;
        for binding in free {
            let Some(ty) = self.locals.get(&binding).cloned() else {
                return Err(error(
                    span,
                    "runtime dependency refers to a local binding that is no longer in scope",
                ));
            };
            for frame in &mut self.captures {
                if frame.outer.contains(&binding)
                    && !frame
                        .values
                        .iter()
                        .any(|capture| capture.binding == binding)
                {
                    frame.values.push(n::Capture {
                        value: None,
                        binding,
                        ty: ty.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    fn deliver_runtime_dependency(
        &mut self,
        term: &Term,
        expected_inner: &Term,
        span: Span,
    ) -> Result<n::Expr, Error> {
        match term {
            Term::Rigid {
                declaration,
                sort: Sort::Runtime,
            } => {
                let Some((binding, inner_ty)) =
                    self.active_env_parameters.get(declaration).cloned()
                else {
                    return Err(error(
                        span,
                        "runtime quote dependency is not available in this function scope",
                    ));
                };
                self.require_expression(&inner_ty, expected_inner, span)?;
                let expr = n::Expr {
                    source: source(span),
                    ty: inner_ty,
                    kind: K::Local(binding),
                };
                self.record_runtime_expr_captures(&expr, span)?;
                Ok(expr)
            }
            Term::RuntimeField { .. } => {
                let value = self.deliver_runtime_field(term, span)?;
                self.require_expression(&value.ty, expected_inner, span)?;
                Ok(value)
            }
            Term::Runtime(id) => {
                let delivered = if let Some(entry) = self.local_runtime_deliveries.get(id) {
                    if let Some((binding, epoch)) = entry.checked_epoch {
                        let current = self
                            .local_write_versions
                            .get(&binding)
                            .copied()
                            .unwrap_or(0);
                        if current != epoch {
                            return Err(error(
                                span,
                                "runtime dependency binding was modified after capture",
                            ));
                        }
                    }
                    constants::charge_copy(&entry.expr, &mut self.required_steps, span)?;
                    let owner = entry.address_owner;
                    let operand = entry.expr.clone();
                    self.capture_runtime_operand(*id, owner, operand, span)?
                } else if let Some(global_expr) = self.global_runtime_deliveries.get(id) {
                    constants::charge_copy(global_expr, &mut self.required_steps, span)?;
                    global_expr.clone()
                } else {
                    return Err(error(
                        span,
                        "runtime dependency is not deliverable in this scope",
                    ));
                };
                self.require_expression(&delivered.ty, expected_inner, span)?;
                Ok(delivered)
            }
            _ => Err(error(span, "invalid runtime dependency argument")),
        }
    }

    fn integer_layout(&self, primitive: P) -> Option<(bool, u32)> {
        self.target.map_or_else(
            || primitive.fixed_integer_layout(),
            |target| target.integer_layout(primitive),
        )
    }

    /// Resolve signature dependencies on demand, retaining only completed
    /// signatures. Recursive value bodies do not require recursive signatures;
    /// cycles through type observations are diagnosed explicitly.
    fn validate_comptime_receiver(&self, owner: OwnerId) -> Result<(), Error> {
        let Some(ItemKind::Fn(signature, _) | ItemKind::FnDeclaration(signature)) =
            self.package.item(owner).map(|item| &item.kind)
        else {
            return Ok(());
        };
        if signature.modifiers.is_comptime {
            if let Some(receiver) = signature
                .params
                .iter()
                .find(|parameter| matches!(parameter.ty.kind, TyParamKind::Itself { is_ref: true }))
            {
                return Err(error(
                    receiver.span,
                    "comptime methods use value-reference self; replace *self with self",
                ));
            }
        }
        Ok(())
    }

    fn ensure_signature(&mut self, owner: OwnerId, span: Span) -> Result<(), Error> {
        self.validate_comptime_receiver(owner)?;
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
        let checked = self.in_declaration_scope(owner, |this| this.form_signature(owner, span));
        self.active_signatures.remove(&owner);
        if checked.is_ok() {
            self.publish_normal_head(owner);
        } else {
            // A recursive signature check can see the staged telescope. Once
            // formation fails its provisional constructor evidence expires.
            self.normal_parameters.remove(&owner);
        }
        checked
    }

    fn form_signature(&mut self, owner: OwnerId, span: Span) -> Result<(), Error> {
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
            let lexical_inputs = self.lexical_input_plan(owner, span)?;
            let mut type_parameters = if let Some(parent) = self
                .effect_parents
                .get(&owner)
                .or_else(|| self.self_containers.get(&owner))
            {
                self.type_parameters[parent]
                    .iter()
                    .copied()
                    .chain(self.nominal_capture_ids(*parent))
                    .collect()
            } else if package.lexical_parents.contains_key(&owner) {
                lexical_inputs.declarations().collect()
            } else {
                match self.self_types.get(&owner) {
                    Some(Term::Rigid {
                        declaration: interface,
                        ..
                    }) => {
                        let parent = self.source_declarations.owner(*interface).unwrap();
                        std::iter::once(*interface)
                            .chain(self.type_parameters[&parent].iter().copied())
                            .collect()
                    }
                    _ => Vec::new(),
                }
            };
            self.prepare_template_application(owner)?;
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
                    self.source_declarations.owner(*interface)
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
            let inherited = type_parameters.len();
            if (sig.modifiers.is_comptime
                || sig.modifiers.is_normal_form_head
                || sig.modifiers.is_quote
                || sig.modifiers.is_function_template
                || matches!(item.kind, ItemKind::Effect(..)))
                && type_parameters[..inherited].iter().any(|id| {
                    self.trait_input_domain(*id)
                        .is_some_and(|domain| matches!(domain, Term::Lift(_)))
                })
            {
                return Err(error(
                    item.span,
                    "comptime, normal-form, quote, template, or effect functions cannot inherit quote runtime dependencies",
                ));
            }
            let mut local_sorts = BTreeMap::new();
            self.normal_input_prefixes.entry(owner).or_default();
            for (slot, parameter) in sig.clause_params.iter().enumerate() {
                use hir::clause::ClauseParamKind as Clause;
                self.ready_signature_capabilities(owner, &mut pending_capabilities, false)?;
                let is_quote = parameter.kind.is_quote();
                let unquoted = parameter.kind.unquoted();
                let domain = if is_quote {
                    if matches!(item.kind, ItemKind::Effect(..))
                        || sig.modifiers.is_comptime
                        || sig.modifiers.is_normal_form_head
                        || sig.modifiers.is_quote
                        || sig.modifiers.is_function_template
                    {
                        return Err(error(
                            parameter.span,
                            "quote clause parameter is only supported on ordinary runtime functions",
                        ));
                    }
                    let (Clause::Positional(_, declared) | Clause::Optional(_, declared, _)) =
                        *unquoted
                    else {
                        return Err(error(
                            parameter.span,
                            "quote clause parameter must be positional or optional",
                        ));
                    };
                    let domain = self.type_expr(declared)?;
                    if !matches!(domain, Term::Lift(_)) {
                        return Err(error(
                            parameter.span,
                            "quote clause parameter requires a lift type domain",
                        ));
                    }
                    if !self.normal_input_domain_supported(
                        &domain,
                        &type_parameters[..inherited],
                        &local_sorts,
                        parameter.span,
                    )? {
                        return Err(error(
                            parameter.span,
                            "quote clause parameter type is not yet supported",
                        ));
                    }
                    domain
                } else {
                    let domain = match *unquoted {
                        Clause::Type(_) | Clause::TraitBound(..) => p(P::Type),
                        Clause::Varadic(_, declared) => {
                            let domain = self.type_expr(declared)?;
                            if self.list_element_type(&domain).is_none() {
                                return Err(error(
                                    parameter.span,
                                    "variadic clause input requires a bound meta.List<T>",
                                ));
                            }
                            if slot + 1 != sig.clause_params.len() {
                                return Err(error(
                                    parameter.span,
                                    "variadic clause input must be the final parameter",
                                ));
                            }
                            domain
                        }
                        Clause::Positional(_, declared) | Clause::Optional(_, declared, _) => {
                            let domain = self.type_expr(declared)?;
                            if !self.normal_input_domain_supported(
                                &domain,
                                &type_parameters[..inherited],
                                &local_sorts,
                                parameter.span,
                            )? {
                                return Err(error(
                                    parameter.span,
                                    "normal-form parameter type is not yet supported",
                                ));
                            }
                            domain
                        }
                        Clause::Quote(_) => unreachable!(),
                    };
                    if matches!(domain, Term::Lift(_)) {
                        return Err(error(
                            parameter.span,
                            "lift type domain requires a quote clause parameter",
                        ));
                    }
                    domain
                };
                let definition = parameter
                    .definition
                    .ok_or_else(|| error(parameter.span, "missing clause parameter identity"))?;
                let id = DeclarationId {
                    package: semantic_package(&self.package_identities, definition.pkg),
                    local: definition.index,
                };
                let sort = Sort::of_comptime_domain(&domain);
                local_sorts.insert(id, sort);
                self.record_clause_input(owner, id, local(parameter.hir_id), domain);
                if !sig.modifiers.is_normal_form_head || sort == Sort::Type {
                    if sig.modifiers.is_normal_form_head {
                        // Publish each Type binder for the next domain check.
                        if type_parameters.len() == inherited {
                            self.type_parameters.insert(owner, type_parameters.clone());
                        }
                        self.type_parameters.get_mut(&owner).unwrap().push(id);
                    } else if sort == Sort::Type {
                        let entry = self.type_parameters.entry(owner).or_default();
                        if !entry.contains(&id) {
                            entry.push(id);
                        }
                    }
                    type_parameters.push(id);
                }
            }
            let normal_parameters = &self.normal_input_prefixes[&owner];
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
                let domains: Vec<_> = type_parameters[..inherited]
                    .iter()
                    .map(|id| self.trait_input_domain(*id).unwrap_or_else(|| p(P::Type)))
                    .chain(normal_parameters.iter().map(|(_, _, ty)| ty.clone()))
                    .collect();
                for (id, domain) in type_parameters.iter().zip(&domains) {
                    self.implementation_index
                        .register_parameter_domain(*id, domain.clone())
                        .map_err(|message| error(span, message))?;
                    self.inherent_index
                        .register_parameter_domain(*id, domain.clone())
                        .map_err(|message| error(span, message))?;
                }
                for &id in &type_parameters {
                    self.parameter_domain_owners.entry(id).or_insert(owner);
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
                self.normal_parameters
                    .insert(owner, normal_parameters.clone());
            }
            for clause in sig.clause_constraints {
                if let hir::clause::ClauseConstraintKind::Requires(value) = clause.kind {
                    required::signature_capability_atoms(value, &mut pending_capabilities, 0)?;
                }
            }
            self.ready_signature_capabilities(owner, &mut pending_capabilities, false)?;
            if let ItemKind::Effect(_, modifiers) = &item.kind {
                let is_direct_passing = self
                    .direct_passing_effects
                    .get(&owner)
                    .copied()
                    .unwrap_or(modifiers.is_direct_passing);
                if is_direct_passing && (modifiers.is_async || !sig.params.is_empty()) {
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
                self.check_template_parameter_annotation(parameter, &annotation)?;
                if annotation.needs_instance() {
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
                    self.signature_parameter_domain(
                        owner,
                        ty,
                        sig.clause_constraints,
                        parameter.span,
                    )?
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
                self.input_facts.parameters.bind(
                    local(parameter.hir_id),
                    parameter_index,
                    parameter_type.sort,
                    &parameter_type.ty,
                );
                if parameters.len() == parameter_index {
                    parameters.push(parameter_type);
                }
            }
            self.ready_signature_capabilities(owner, &mut pending_capabilities, true)?;
            let result_annotation = if let Some(ty) = sig.return_ty {
                self.signature_annotation(ty)?
            } else {
                open_functions::Annotation::Known(p(P::Void))
            };
            // A closed signature does not complete staged body inputs. Runtime
            // free functions also receive inputs through declaration clauses.
            // Comptime bodies already execute local constants per invocation;
            // receiver definitions retain their separate checking path.
            if result_annotation.needs_instance()
                || annotations
                    .iter()
                    .any(|(_, annotation)| annotation.needs_instance())
                || (!sig.modifiers.is_normal_form_head
                    && (parameters
                        .iter()
                        .any(|parameter| parameter.sort != Sort::Runtime)
                        || (!sig.modifiers.is_comptime
                            && !self.self_containers.contains_key(&owner)
                            && !self.self_types.contains_key(&owner)
                            && !self.type_parameters[&owner].is_empty()))
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
        self.normal_input_prefixes.remove(&owner);
        self.signature_domains.remove(&owner);
        if checked.is_err() {
            self.lexical_input_plans.remove(&owner);
            self.pending_refinements
                .retain(|pending| pending.owner != owner);
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

    fn variant_target(
        &mut self,
        callee: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<Option<(Term, usize)>, Error> {
        let Some((ty, name)) = self.observed_enum_member_reference(callee, expected)? else {
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
        let variant = self
            .instantiated_variants(&ty, expr.span)?
            .into_iter()
            .nth(index)
            .ok_or_else(|| error(expr.span, "enum variant index out of bounds"))?;
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
        let owner = self.source_declarations.owner(id);
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
                let default = self.field_default(owner, Some(index), slot)?;
                let frames = self.call_input_frames();
                nominal_fields::metered(&mut self.required_steps, |fuel| {
                    default.instantiate_declaration_parameters_bounded(
                        &self.enums[&id].type_parameters,
                        &self.enums[&id].parameter_types,
                        arguments,
                        &frames,
                        fuel,
                    )
                })
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
            .source_declarations
            .owner(id)
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
            let sort = Sort::of_comptime_domain(&domain);
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
                                Some(head) => crate::calls::InputContext::NormalTuple {
                                    head,
                                    fields,
                                    list_domain: self.bound_tuple_list_domain(),
                                },
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
                    |ty, budget| self.callable_input_view_metered(ty, budget),
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
                None => {
                    let frames = self.call_input_frames();
                    nominal_fields::metered(&mut self.required_steps, |fuel| {
                        default
                            .expect("omitted payload has a checked default")
                            .instantiate_declaration_parameters_bounded(
                                &definition.type_parameters,
                                &definition.parameter_types,
                                &complete_arguments,
                                &frames,
                                fuel,
                            )
                    })
                    .map_err(|e| error(expr.span, e.to_string()))?
                }
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
        let variant = self
            .instantiated_variants(&ty, expr.span)?
            .into_iter()
            .nth(index)
            .ok_or_else(|| error(expr.span, "enum variant index out of bounds"))?;
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

    fn enum_variant(&mut self, ty: &Term, name: &str, span: Span) -> Result<n::Expr, Error> {
        let shape = self.structural_type(ty, span)?;
        let ty = &shape;
        let Term::Nominal { declaration, .. } = ty else {
            return Err(incompatible_context(
                span,
                "variant requires a nominal enum context",
            ));
        };
        self.validate_enum_payloads(ty, span)?;
        let definition = self
            .enums
            .get(declaration)
            .ok_or_else(|| error(span, "variant requires an enum type"))?;
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

    fn bound_tuple_metadata(&self) -> Option<nir::normal_forms::Head> {
        let identity = self.protocols.tuple?;
        let owner = self.source_declarations.owner(identity)?;
        self.normal_parameters.get(&owner)?;
        self.normal_form_head_metadata(owner)
    }

    fn bound_tuple_list_domain(&self) -> Option<Term> {
        let head = self.bound_tuple_metadata()?;
        let [parameter] = head.parameters.as_slice() else {
            return None;
        };
        self.is_type_list_type(&parameter.ty)
            .then(|| parameter.ty.clone())
    }

    fn bound_tuple_elements<'a>(&self, ty: &'a Term) -> Option<&'a [Term]> {
        if !matches!(ty, Term::NormalForm { head, sort: Sort::Type, .. } if Some(*head) == self.protocols.tuple)
        {
            return None;
        }
        self.bound_tuple_metadata()?.tuple_elements(ty)
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
                    let fields = nir::normal_forms::nominal_value_fields_with_collections_in(
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
                        &|domain, fuel| self.collection_domain_budgeted(domain, fuel),
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
        crate::coverage::check_metered_with_collections_in(
            &self.structs,
            &self.enums,
            ty,
            patterns,
            100_000,
            &self.context,
            self.bound_tuple_metadata().as_ref(),
            &|domain, fuel| self.collection_domain_budgeted(domain, fuel),
            self.target,
        )
        .0
    }

    fn pattern_exhaustive(&self, pattern: &n::Pattern, ty: &Term) -> bool {
        self.patterns_exhaustive(&[pattern], ty) == crate::coverage::Coverage::Exhaustive
    }

    fn construct(
        &mut self,
        expr: &Expr<'_>,
        callee: &Expr<'_>,
        args: &[Arg<'_>],
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let ty = self.type_expr(callee)?;
        self.with_constructor_inputs(callee, ty, expr.span, |this, ty| {
            this.construct_typed(expr, ty, args, expected)
        })
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
        let declaration = *id;
        let frames = self.call_input_frames();
        let def = self
            .structs
            .get(id)
            .ok_or_else(|| error(expr.span, "missing struct descriptor"))?;
        let (instantiated_fields, type_parameters, parameter_types) =
            nominal_fields::constructor_fields(def, arguments, &frames, &mut self.required_steps)
                .map_err(|e| error(expr.span, e.to_string()))?;
        let owner = self.source_declarations.owner(*id);
        let mut supplied = BTreeMap::new();
        for arg in args {
            let Arg::Named(name, value) = arg else {
                return Err(error(expr.span, "struct fields must be fully named"));
            };
            let index = instantiated_fields
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
                let default = self.field_default(
                    owner.ok_or_else(|| {
                        error(
                            expr.span,
                            format!("missing struct field {}", instantiated_fields[index].0),
                        )
                    })?,
                    None,
                    index,
                )?;
                let frames = self.call_input_frames();
                nominal_fields::metered(&mut self.required_steps, |fuel| {
                    default.instantiate_declaration_parameters_bounded(
                        &type_parameters,
                        &parameter_types,
                        arguments,
                        &frames,
                        fuel,
                    )
                })
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
                declaration,
                fields,
            },
        })
    }

    fn check_struct_cycles(&self) -> Result<(), Error> {
        let tuple = self.bound_tuple_metadata();
        let mut representations = crate::representations::Checker::new(
            tuple.as_ref(),
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
            | H::LiftType(_)
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
                    .or_else(|| {
                        self.definition_inputs
                            .get(&declaration)
                            .map(|value| value.ty.clone())
                    })
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
                    || match head.kind {
                        H::ResolvedDef { def_id, .. } => {
                            self.package.definition_owner(def_id).is_some_and(|owner| {
                                self.constants
                                    .get(&owner)
                                    .is_some_and(|value| value.ty.is_normal_form_callable())
                                    || self.package.item(owner).is_some_and(|item| {
                                        match &item.kind {
                                            ItemKind::Fn(sig, _) => {
                                                sig.modifiers.is_normal_form_head
                                            }
                                            ItemKind::Const(annotation, _) => {
                                                function_types::normal_form_inputs(annotation)
                                                    .is_some()
                                            }
                                            _ => false,
                                        }
                                    })
                            })
                        }
                        H::ResolvedFnParam(_, id) | H::ResolvedLocal(_, id) => self
                            .locals
                            .get(&local(id))
                            .is_some_and(Term::is_normal_form_callable),
                        _ => false,
                    }
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

    fn form_type_identity(&mut self, expr: &Expr<'_>) -> Result<Term, Error> {
        match &expr.kind {
            H::ResolvedLocal(_, id) if self.callable_type_inputs.contains_key(&local(*id)) => {
                let value = self
                    .callable_type_input_expression(expr, local(*id), Some(&p(P::Type)))?
                    .expect("active callable input");
                let K::TypeValue(ty) = value.kind else {
                    return Err(error(
                        expr.span,
                        "function type dependency requires an earlier explicit comptime Type parameter",
                    ));
                };
                Ok(ty)
            }
            H::ResolvedLocal(_, id)
                if self.input_facts.derived.contains(&local(*id))
                    && self.locals.get(&local(*id)) == Some(&p(P::Type))
                    && self.input_facts.types.contains_key(&local(*id)) =>
            {
                // A checked const field can have a complete Type identity even
                // when another field still depends on invocation-time inputs.
                let ty = &self.input_facts.types[&local(*id)];
                let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let before = fuel;
                let charged = nir::type_computations::charge_term(ty, &mut fuel);
                self.required_steps -= (before - fuel) as u64;
                charged.map_err(|e| error(expr.span, e.to_string()))?;
                Ok(ty.clone())
            }
            H::ResolvedLocal(_, id)
                if !self.callable_type_inputs.contains_key(&local(*id))
                    && matches!(
                        self.generator_data.get(&local(*id)),
                        Some(Some(n::Expr {
                            kind: K::TypeValue(_),
                            ..
                        }))
                    ) =>
            {
                let Some(Some(n::Expr {
                    kind: K::TypeValue(ty),
                    ..
                })) = self.generator_data.get(&local(*id))
                else {
                    unreachable!()
                };
                let ty = ty.clone();
                let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let before = fuel;
                let charged = nir::type_computations::charge_term(&ty, &mut fuel);
                self.required_steps -= (before - fuel) as u64;
                charged.map_err(|e| error(expr.span, e.to_string()))?;
                Ok(ty)
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
                self.input_facts
                    .type_identity(&binding)
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
                let labels = self.error_type_labels(errors)?;
                Ok(Term::error_result(labels, self.type_expr(result)?))
            }
            H::EffectQualifiedType(effects, result) => {
                let operations = self.effect_row(effects, 0)?;
                Ok(Term::computation(operations, self.type_expr(result)?))
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
                    if let Some(value) = self.alias_substitution.shared(declaration).cloned() {
                        return self.copy_alias_type(&value, expr.span);
                    }
                }
                if !self.alias_substitution.is_empty() {
                    let parameters = self.input_binders(
                        &self.alias_substitution.keys().copied().collect::<Vec<_>>(),
                    );
                    let arguments: Vec<_> = self.alias_substitution.values().cloned().collect();
                    return ty
                        .instantiate_parameters(&parameters, &arguments)
                        .map_err(|e| error(expr.span, e.to_string()));
                }
                Ok(ty)
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
                if let H::ResolvedFnParam(_, id) = &value.kind
                    && (self.checking_open_behavior || self.signature_owner == Some(id.owner))
                    && self.locals.get(&local(*id)).is_none()
                    && self
                        .package
                        .item(id.owner)
                        .is_some_and(|item| match &item.kind {
                            ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig) => {
                                sig.params.iter().any(|p| p.hir_id == *id)
                            }
                            _ => false,
                        })
                    && (self.open_functions.contains_key(&id.owner)
                        || self.signature_domains.contains_key(&id.owner))
                {
                    let key = local(*id);
                    let binder =
                        if let Some(binder) = self.definition_refinements.subjects.get(&key) {
                            *binder
                        } else {
                            let binder = self.fresh_type_binder(id.owner, expr.span)?;
                            self.definition_refinements.subjects.insert(key, binder);
                            binder
                        };
                    return Ok(Term::Rigid {
                        declaration: binder.declaration,
                        sort: binder.sort,
                    });
                }
                // Observing a type does not evaluate/capture the operand in
                // surrounding runtime closures. Closures inside the operand
                // still build their own capture types during checking.
                let captures = std::mem::take(&mut self.captures);
                let locals = self.locals.clone();
                let saved_control = std::mem::take(&mut self.control);
                // Preserve lexical loop visibility for the unevaluated operand.
                self.control.loops = saved_control.loops.clone();
                let observing = std::mem::replace(&mut self.observing_type, true);
                let checked = self.expr(value, None).map(|value| value.ty);
                self.observing_type = observing;
                self.captures = captures;
                self.locals = locals;
                self.control = saved_control;
                checked
            }
            H::TyClosure(context, body) => {
                let H::TyClosureEnvironment(bindings) = &context.kind else {
                    if let H::ResolvedClauseParam(_, id) = &context.kind {
                        let declaration = self.visible_clause_input(*id, context.span)?;
                        if self.trait_input_domain(declaration) == Some(Term::ClosureContextType) {
                            let context = self
                                .alias_substitution
                                .get(&declaration)
                                .cloned()
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
                if !matches!(
                    body,
                    Term::Function { .. } | Term::ComptimeFunction(_) | Term::UnsafeFunction(_)
                ) {
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
            H::TyModifiedFn(modifiers, params) => {
                self.form_modified_callable_type(*modifiers, params, None, expr.span)
            }
            H::TyNFFn(params) | H::TyFn(params) | H::TyComptimeFn(params) => {
                self.form_callable_type(params, None, function_types::phase(&expr.kind), expr.span)
            }
            H::TyFnArrow(input, output) => {
                if let H::TyModifiedFn(modifiers, params) = &input.kind {
                    return self.form_modified_callable_type(
                        *modifiers,
                        params,
                        Some(output),
                        expr.span,
                    );
                }
                if let H::TyNFFn(params) | H::TyFn(params) | H::TyComptimeFn(params) = &input.kind {
                    return self.form_callable_type(
                        params,
                        Some(output),
                        function_types::phase(&input.kind),
                        expr.span,
                    );
                }

                let input = self.type_expr(input)?;
                if !matches!(input.callable_signature(), Term::Function { .. }) {
                    return Err(error(expr.span, "arrow input must be a function type"));
                }
                let output = self.type_expr(output)?;
                self.computed_callable_result(input, output, expr.span)
            }
            H::ResolvedClauseParam(_, id) => {
                let declaration = self.visible_clause_input(*id, expr.span)?;
                if let Some(value) = self.definition_input(declaration, expr.span)? {
                    let K::TypeValue(ty) = value.kind else {
                        return Err(error(
                            expr.span,
                            "definition value input cannot be used as a Type",
                        ));
                    };
                    return Ok(ty);
                }
                if let Some(domain) = self.trait_input_domain(declaration) {
                    let domain = self.substituted_trait_domain(&domain, expr.span)?;
                    if domain != p(P::Type) {
                        return Err(error(
                            expr.span,
                            "trait value parameter cannot be used as a Type",
                        ));
                    }
                }
                if self.alias_value_bindings.contains_key(&declaration) {
                    return Err(error(
                        expr.span,
                        "required comptime input is not yet available in this definition",
                    ));
                }
                if let Some(value) = self.alias_substitution.shared(&declaration).cloned() {
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
            H::LiftType(inner) => Ok(Term::Lift(Box::new(self.type_expr(inner)?))),
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
        if let Some((kind, arg)) = self.type_predicate_kind_call(expr)? {
            let operand = self.type_expr(arg)?;
            return Ok(n::TypePredicate::Kind(operand, kind));
        }
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
        let mut context = self.declared_theory.clone();
        for declaration in &self.code_packet_nominals {
            context.admit_nominal_constructor(*declaration);
        }
        // A recursive check may refer to an enclosing head whose telescope
        // has been formed but whose signature has not yet been published.
        for &owner in &self.active_signatures {
            if let Some((head, role)) = self.normal_head_contract(owner) {
                context.admit_normal_form_head(head, role);
            }
        }
        context
    }

    fn definition_context(&self, owner: OwnerId) -> crate::ptc::Context {
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
        self.definition_context_in_scope(owner, scope)
    }

    fn definition_context_in_scope(
        &self,
        owner: OwnerId,
        scope: Vec<Vec<Sort>>,
    ) -> crate::ptc::Context {
        let base = self.binder_context(owner);
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
        let interface = self
            .self_containers
            .get(&owner)
            .and_then(|&parent| {
                self.checked_traits.by_id(declaration(
                    self.package,
                    &self.package_identities,
                    parent,
                ))
            })
            .or_else(|| {
                self.checked_traits.member_interface(declaration(
                    self.package,
                    &self.package_identities,
                    owner,
                ))
            });
        if let Some(interface) = interface {
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
                        sort: self
                            .trait_input_domain(*id)
                            .as_ref()
                            .map_or(Sort::Type, Sort::of_comptime_domain),
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
        if !self.alias_substitution.is_empty() {
            arguments.environment = self.alias_substitution.clone();
            for parameter in &self.type_parameters[&owner] {
                arguments.environment.remove(parameter);
            }
        }
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
            Q::Kind(ty, kind) => Q::Kind(self.callable_input_view(ty, span)?, *kind),
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

    /// Equality observations keep caller telescope slots in their original scope.
    fn scoped_equality_operands(
        &self,
        actual: &Term,
        expected: &Term,
        span: Span,
    ) -> Result<(Term, Term), Error> {
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
        Ok(if has_inputs && self.context.scope() != frames.as_slice() {
            (scoped(actual)?, scoped(expected)?)
        } else {
            (actual.clone(), expected.clone())
        })
    }

    fn require(&self, actual: &Term, expected: &Term, span: Span) -> Result<(), Error> {
        // Diverging control flow is admissible at a result position; this does
        // not assert the false type equation Never = expected.
        if is_never(actual) {
            return Ok(());
        }
        use crate::ptc::{Budget, Formula, Outcome};
        let (left, right) = self.scoped_equality_operands(actual, expected, span)?;
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
        self.check_ordinary_block_region(
            block,
            block_regions::OrdinaryTail {
                expected,
                partial,
                input: input_context,
            },
        )
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
        callable_target: Option<&Term>,
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
            if self
                .trait_domains(owner)?
                .iter()
                .any(|domain| matches!(domain, Term::Lift(_)))
            {
                return Err(error(
                    expr.span,
                    "functions with quote runtime dependencies cannot be taken as bare function values",
                ));
            }
            if let Some(ItemKind::Fn(signature, _) | ItemKind::FnDeclaration(signature)) =
                self.package.item(owner).map(|item| &item.kind)
            {
                arguments::require_function_value_parameters(signature.params, expr.span)?;
            }
        }
        if !allow_optional && self.open_value_needs_instance(owner) {
            return self.open_function_value(expr, owner, &arguments, callable_target);
        }
        let scope = self.call_inference_domains();
        let frames = if scope.is_empty() {
            vec![]
        } else {
            vec![scope.iter().map(|p| p.0).collect()]
        };
        let template = self
            .declared_signature(owner)
            .map(|sig| sig.into_owned())
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
        Ok(n::Expr {
            ty,
            source: source(expr.span),
            kind: K::FunctionInstance {
                declaration: declaration(self.package, &self.package_identities, owner),
                arguments,
            },
        })
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
        if self
            .trait_domains(owner)?
            .iter()
            .any(|domain| matches!(domain, Term::Lift(_)))
        {
            return Err(error(
                expr.span,
                "functions with quote runtime dependencies cannot be taken as bare function values",
            ));
        }
        let (generic_parameters, template, pending_defaults) =
            self.function_type_plan(owner, &fixed, expr.span)?;
        if generic_parameters.is_empty() {
            let pending_open = self.open_value_needs_instance(owner);
            let value = self.function_instance(expr, owner, template, expected, !pending_open)?;
            return self.callable_value_view(value, expected, expr.span);
        }
        let (original, known_parameters, known_result) =
            match self.declared_signature(owner).map(|sig| sig.into_owned()) {
                Some(signature) => (signature, None, true),
                None => {
                    let (signature, known, result) =
                        self.open_callable_inference_shape(owner).ok_or_else(|| {
                            error(expr.span, "declaration is not a supported callable")
                        })?;
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
                |ty, budget| self.callable_input_view_metered(ty, budget),
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
        let value = self.function_instance(expr, owner, arguments, expected, false)?;
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
                H::Match(..),
            ) => self.check_ordinary_match_region(input, None, None, Some(context)),
            (
                C::Optional(_)
                | C::Tuple(_)
                | C::NormalTuple { .. }
                | C::Record(_)
                | C::Nominal { .. }
                | C::ErrorResult { .. },
                H::When(_),
            ) => self.check_ordinary_when_region(input, None, None, Some(context)),
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
                H::If(_, _, Some(_)),
            ) => self.check_partial_if_region(input, context),
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
            (C::Tuple(_) | C::NormalTuple { .. }, H::Tuple(items)) => {
                self.check_partial_tuple(input, items, context)
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
        self.checking_attempt(
            input.span,
            allow_defaults,
            CheckingDisposition::Commit,
            |checker| {
                if expected.is_none()
                    && matches!(
                        input_context,
                        Some(
                            crate::calls::InputContext::Optional(_)
                                | crate::calls::InputContext::Tuple(_)
                                | crate::calls::InputContext::NormalTuple { .. }
                                | crate::calls::InputContext::Record(_)
                                | crate::calls::InputContext::Nominal { .. }
                                | crate::calls::InputContext::ErrorResult { .. }
                                | crate::calls::InputContext::Callable(_)
                        )
                    )
                {
                    match input_context {
                        Some(input_context) => {
                            checker.input_context_expression(input, input_context)
                        }
                        None => checker.expr_context(input, expected, partial),
                    }
                } else {
                    checker.expr_context(input, expected, partial)
                }
            },
        )
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
        self.generic_call_with_receiver(
            expr,
            owner,
            args,
            expected,
            partial,
            Vec::new(),
            known,
            None,
        )
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
        selected_method: Option<interfaces::SelectedMethod>,
    ) -> Result<n::Expr, Error> {
        self.ensure_signature(owner, expr.span)?;
        let plan = self.function_type_plan(owner, &known, expr.span)?;
        self.generic_call_with_plan(
            expr,
            owner,
            args,
            expected,
            partial,
            prefix,
            plan,
            selected_method,
        )
    }

    fn generic_call_with_plan(
        &mut self,
        expr: &Expr<'_>,
        owner: OwnerId,
        args: &[Arg<'_>],
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
        prefix: Vec<n::Expr>,
        plan: type_defaults::FunctionTypePlan,
        selected_method: Option<interfaces::SelectedMethod>,
    ) -> Result<n::Expr, Error> {
        if self.is_quote_function(owner) {
            self.record_quote_application(expr.span)?;
        }
        let written_types = self.acquire_written_call_types(expr)?;
        let structural_result = self
            .structural_result_context
            .as_ref()
            .filter(|(id, _)| *id == expr.hir_id)
            .map(|(_, context)| context.clone());
        let all_parameters = self.type_parameters[&owner].clone();
        let (parameters, template, pending_defaults) = plan;
        let signature = self.instantiate_input_types(
            self.declared_signature(owner)
                .ok_or_else(|| error(expr.span, "declaration is not a supported callable"))?
                .as_ref(),
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
        let site = dependent_inputs::CallSite {
            callee: preview,
            owner: Some(owner),
            routes,
            source: expr.clone(),
        };
        let dependent_inputs::PreparedInputs {
            producers: mut prepared,
            binding: call_binding,
            ..
        } =
            self.prepare_call_types(prepared_signature, &site, offset, &assigned[offset..], seed)?;
        let routes = site.routes;
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
                |ty, budget| self.callable_input_view_metered(ty, budget),
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
                    |ty, budget| self.callable_input_view_metered(ty, budget),
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
                            | crate::calls::InputContext::ErrorResult { .. }
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
        self.check_ordinary_capabilities(
            owner,
            &full_arguments,
            call_binding.observations(),
            expr.span,
        )?;
        self.check_ordinary_type_requirements(
            owner,
            &full_arguments,
            call_binding.observations(),
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
                value.rewrite_types(&mut |ty| self.callable_argument_term_view(ty, expr.span))?;
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
        let callee_expr = if let Some(method) = selected_method {
            method.callee(self, owner, &full_arguments, expr.span)?
        } else {
            n::Expr {
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
            }
        };
        let inputs = self.call_environment_inputs(&callee_expr, expr.span)?;
        let mut call = n::Expr {
            ty,
            source: source(expr.span),
            kind: K::Call {
                callee: Box::new(callee_expr),
                arguments,
                env_arguments: Vec::new(),
                parameter_bindings: declarations
                    .iter()
                    .map(|param| local(param.hir_id))
                    .collect(),
            },
        };
        let environment =
            self.prepare_call_environment(&mut call, inputs, written_types, offset, &routes, expr)?;
        let call = self.freeze_staged_call_inputs(call, expr)?;
        environment.finish(self, call, &routes, offset, expr)
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

    fn known_pattern_type_value(
        &mut self,
        value: &n::Expr,
        span: Span,
    ) -> Result<Option<Term>, Error> {
        if value.ty != p(P::Type) {
            return Ok(None);
        }
        let observed = self
            .with_observation_budget(span, |this, budget| this.normal_input_hint(value, budget))?;
        Ok(observed.filter(|value| value.sort() == Sort::Type))
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
                        | K::ConstAddress { .. }
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
        // A checked scalar literal is already a value, with no callable,
        // capture, cleanup or user literal protocol left to execute. Decode it
        // under the shared budget instead of constructing a complete VM world
        // for every static slice endpoint. All other expressions retain the
        // ordinary invocation path, including calls that happen to be constant.
        let immediate = if matches!(
            checked.kind,
            K::Literal(
                n::Literal::Integer(_)
                    | n::Literal::SignedInteger(_)
                    | n::Literal::UnsignedInteger(_)
            )
        ) {
            match self.checked_normal_input_in(&checked, &BTreeMap::new(), index.span)? {
                Some(Term::Integer(value)) => Some(value),
                _ => None,
            }
        } else {
            None
        };
        let value = if let Some(value) = immediate {
            value
        } else {
            let (value, _) = self.required_normal_input(index, checked.clone(), Vec::new())?;
            match value {
                comptime_interpretor::Value::Integer(value) => value,
                comptime_interpretor::Value::UnsignedInteger(value) => {
                    comptime_runtime::Integer::from_u128(value)
                }
                comptime_interpretor::Value::SignedInteger(value) => {
                    comptime_runtime::Integer::from_i128(value)
                }
                _ => return Err(error(index.span, "BitVec index must produce an integer")),
            }
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

    fn finish_arithmetic_expression(
        &mut self,
        expr: &Expr<'_>,
        left: n::Expr,
        right: n::Expr,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let H::Binary(op, _, _) = &expr.kind else {
            return Err(error(expr.span, "arithmetic requires two source operands"));
        };
        let operator = block_regions::arithmetic::operator(expr)
            .ok_or_else(|| error(expr.span, "expected an arithmetic operator"))?;
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
        match expected {
            // Blocks always deliver the whole destination to their tail. They
            // never use the value-level optional/error adaptation probe.
            _ if matches!(expr.kind, H::Block(_)) => {
                self.expr_context_unadapted(expr, expected, partial)
            }
            Some(target) if !matches!(target, Term::Primitive(_)) => {
                self.check_delivery(expr, target, partial)
            }
            _ => self.expr_context_unadapted(expr, expected, partial),
        }
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
        let frames = self.context.scope().to_vec();
        nir::error_results::validate_conversion_in_scope(
            conversion,
            &value.ty,
            target,
            &frames,
            &mut 100_000,
            &mut |a, b| self.require(a, b, span).is_ok(),
        )
        .map_err(|e| {
            error(
                span,
                format!(
                    "error result conversion: {e}; input {}; output {target}",
                    value.ty
                ),
            )
        })?;
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
            H::Binary(BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem, _, _) => {
                self.arithmetic_expression(expr, expected)
            }
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
        let allow_packed_projection =
            std::mem::replace(&mut self.allow_packed_pointer_projection, false);
        if self.data_consumer_call(expr)?.is_some() {
            return self.check_data_consumer_expression(expr, expected);
        }
        if let H::Application(callee, arguments) = &expr.kind
            && let H::ResolvedDef { def_id, .. } = &callee.kind
            && let Some(owner) = self.package.definition_owner(*def_id)
            && let Some(name) = self.direct_compiler_operation(owner, callee.span)?
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
                H::If(..) => self.check_ordinary_if_region(expr, expected, partial),
                H::Match(..) => self.check_ordinary_match_region(expr, expected, partial, None),
                H::When(_) => self.check_ordinary_when_region(expr, expected, partial, None),
                H::Application(callee, args) if matches!(&callee.kind, H::Builtin(name) if matches!(name.as_str(), "enum_group_paths" | "enum_variant_paths" | "enum_variant_shapes" | "enum_variant_field_names" | "enum_variant_field_types" | "enum_symbol_values" | "enum_bit_width" | "enum_bit_base_type" | "enum_bit_masks" | "enum_bit_values" | "enum_bit_offsets")) => {
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
                H::Application(callee, args) if matches!(&callee.kind, H::Builtin(name) if name.as_str() == "build_newtype") => {
                    self.build_newtype_builtin(expr, args, expected)
                }
                H::Application(callee, args) if matches!(&callee.kind, H::Builtin(name) if name.as_str() == "build_struct") => {
                    self.build_struct_builtin(expr, args, expected)
                }
                H::Application(callee, args) if matches!(&callee.kind, H::Builtin(name) if name.as_str() == "newtype_representation") => {
                    self.newtype_reflection_builtin(expr, args, expected)
                }
                H::Application(callee, args) if matches!(&callee.kind, H::Builtin(name) if matches!(name.as_str(), "struct_field_types" | "struct_field_names")) => {
                    self.struct_reflection_builtin(expr, callee, args, expected)
                }
                H::Application(callee, args) if matches!(&callee.kind, H::Builtin(name) if name.as_str() == "attrs_of") => {
                    self.attrs_of_reflection_builtin(expr, args, expected)
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
                H::Application(callee, args) if matches!(&callee.kind, H::Builtin(name) if matches!(name.as_str(), "str_byte_length" | "str_byte" | "str_slice" | "str_concat" | "symbol_byte_length" | "symbol_byte")) => {
                    self.text_builtin(expr, callee, args, expected)
                }
                H::Application(callee, args) => {
                    if let Some((kind, arg)) = self.type_predicate_kind_call(expr)? {
                        let operand = self.type_observation_operand(arg)?;
                        let ty = p(if is_never(&operand.ty) {
                            P::Never
                        } else {
                            P::Bool
                        });
                        if let Some(expected) = expected {
                            self.require_expression(&ty, expected, expr.span)?;
                        }
                        Ok(n::Expr {
                            source: source(expr.span),
                            ty,
                            kind: K::TypeKind {
                                kind,
                                operand: Box::new(operand),
                            },
                        })
                    } else if let Some((query, arg)) = self.type_layout_query_call(expr)? {
                        let operand = self.type_observation_operand(arg)?;
                        let ty = p(if is_never(&operand.ty) {
                            P::Never
                        } else {
                            P::Usize
                        });
                        if let Some(expected) = expected {
                            self.require_expression(&ty, expected, expr.span)?;
                        }
                        Ok(n::Expr {
                            source: source(expr.span),
                            ty,
                            kind: K::TypeLayout {
                                query,
                                operand: Box::new(operand),
                            },
                        })
                    } else {
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
                            self.with_constructor_inputs(callee, ty, expr.span, |this, ty| {
                                this.construct_variant(expr, ty, index, args, expected)
                            })
                        } else {
                            self.call(expr, callee, args, expected, partial)
                        }
                    }
                }
                H::ExtendedApplication(callee, args) => {
                    if let Some((ty, index)) = self.variant_target(callee, expected)? {
                        self.with_constructor_inputs(callee, ty, expr.span, |this, ty| {
                            this.construct_named_variant(expr, ty, index, args, expected)
                        })
                    } else if let Some(call) =
                        self.extended_call(expr, callee, args, expected, partial)?
                    {
                        Ok(call)
                    } else {
                        self.construct(expr, callee, args, expected)
                    }
                }
                H::Projection(value, field) => {
                    self.allow_packed_pointer_projection = allow_packed_projection;
                    let result = self.projection_expression(expr, value, field, expected, partial);
                    self.allow_packed_pointer_projection = false;
                    result
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

    fn type_observation_operand(&mut self, arg: &Expr<'_>) -> Result<n::Expr, Error> {
        if self.runtime_value_context() {
            let ty = self.type_expr(arg)?;
            return Ok(n::Expr {
                source: source(arg.span),
                ty: p(P::Type),
                kind: K::TypeValue(ty),
            });
        }
        let operand = self.expr(arg, Some(&p(P::Type)))?;
        if !matches!(operand.kind, K::TypeValue(_))
            && operand.ty == p(P::Type)
            && let Some(ty) = self.checked_normal_input_in(&operand, &BTreeMap::new(), arg.span)?
            && ty.sort() == Sort::Type
        {
            return Ok(n::Expr {
                source: operand.source,
                ty: p(P::Type),
                kind: K::TypeValue(ty),
            });
        }
        Ok(operand)
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
