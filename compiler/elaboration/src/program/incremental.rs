//! Incremental upstream body and open-definition caching during elaboration.
//!
//! Pre-contract-inheritance upstream bodies and open definitions are recorded
//! only when all global side-state mutations outside deterministic per-item
//! deltas (`match_plans`, `safety_contracts.{levels,decreases,predicate_roles}`,
//! and exact-base `next_temporary_binding` allocations) are absent.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

/// Pre-elaborated NIR function body and deterministic per-body side outputs.
#[derive(Clone, Debug)]
pub struct CachedUpstreamFunction {
    pub function: n::Function,
    pub is_template_behavior: bool,
    pub is_spec: bool,
    pub match_plans: Vec<(n::MatchPlanKey, n::MatchPlan)>,
    pub safety_level: Option<n::SafetyLevel>,
    pub decreases: Option<Vec<n::Expr>>,
    pub predicate_roles: Vec<(n::SourceRange, n::PredicateRole)>,
    pub expansion_sources_base: usize,
    pub added_expansion_sources: Vec<n::ExpansionSource>,
    pub temp_binding_base: u32,
    pub temp_binding_delta: u32,
}

/// Pre-checked open definition state and its deterministic side outputs.
#[derive(Clone)]
pub(super) struct CachedOpenDefinition {
    initial_fingerprint: String,
    initial_requires_open: bool,
    checked_definition: open_functions::Definition,
    ordinary_behavior_function: Option<n::Function>,
    match_plans: Vec<(n::MatchPlanKey, n::MatchPlan)>,
    safety_level: Option<n::SafetyLevel>,
    decreases: Option<Vec<n::Expr>>,
    predicate_roles: Vec<(n::SourceRange, n::PredicateRole)>,
    expansion_sources_base: usize,
    added_expansion_sources: Vec<n::ExpansionSource>,
    temp_binding_base: u32,
    temp_binding_delta: u32,
}

impl std::fmt::Debug for CachedOpenDefinition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CachedOpenDefinition")
            .field("initial_requires_open", &self.initial_requires_open)
            .field(
                "has_ordinary_behavior",
                &self.ordinary_behavior_function.is_some(),
            )
            .field("match_plans", &self.match_plans.len())
            .field("predicate_roles", &self.predicate_roles.len())
            .field("temp_binding_base", &self.temp_binding_base)
            .field("temp_binding_delta", &self.temp_binding_delta)
            .finish()
    }
}

/// Elaborated upstream function bodies and open definitions reusable across compiles.
#[derive(Clone, Debug, Default)]
pub struct CachedUpstreamBodies {
    pub functions: BTreeMap<DeclarationId, CachedUpstreamFunction>,
    pub(super) open_definitions: BTreeMap<DeclarationId, CachedOpenDefinition>,
}

impl CachedUpstreamBodies {
    pub fn is_empty(&self) -> bool {
        self.functions.is_empty() && self.open_definitions.is_empty()
    }
}

/// Controls incremental upstream skip gates and multi-threaded verification.
#[derive(Clone, Debug, Default)]
pub struct IncrementalOptions {
    /// Semantic package identities whose source declarations and implementations
    /// already passed contract inheritance, purity, and machine-flow checks in
    /// the upstream base.
    pub verified_upstream_packages: BTreeSet<u64>,
    /// Pre-elaborated source function bodies and open definitions for verified
    /// upstream declarations.
    pub cached_upstream_bodies: Option<std::sync::Arc<CachedUpstreamBodies>>,
    /// Semantic upstream package identities whose upstream bodies/open definitions
    /// may be recorded for reuse after all checks succeed.
    pub record_upstream_packages: BTreeSet<u64>,
    /// Worker count for parallel verification passes (`None` uses default).
    pub jobs: Option<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct BodySideState {
    next_runtime_id: u32,
    definition_identity_next: u32,
    generated_functions: (u32, usize, usize),
    structs: usize,
    enums: usize,
    newtypes: usize,
    alias_contracts: usize,
    computed_scalar_packets: usize,
    internal_return_empty_replacements: usize,
    code_packet_nominals: usize,
    runtime_domains: usize,
    global_runtime_deliveries: usize,
    pending_default_stability: usize,
    pending_predicate_stability: usize,
    refinements: usize,
    pending_refinements: usize,
    volatile_storage: usize,
    layout_alignments: usize,
    observed_associated_types: usize,
    delivered_types: usize,
    constants: usize,
    ordinary_behavior_other: usize,
    default_behavior_functions: usize,
    safety_other_levels: usize,
    safety_other_decreases: usize,
    match_plans_other: usize,
    declared_theory_nullary: usize,
    spec_functions_other: usize,
    checked_runtime_functions_other: usize,
    checked_pattern_functions: usize,
    reflection_spans: (usize, usize, usize, usize, usize, usize),
    closed_const_regions: usize,
    open_functions_len: usize,
    checked_implementations: usize,
    signature_type_uses: usize,
    pending_const_addresses: usize,
}

fn reflection_span_counts(
    spans: &n::ReflectionSpans,
) -> (usize, usize, usize, usize, usize, usize) {
    (
        spans.structs.len(),
        spans.struct_fields.len(),
        spans.record_fields.len(),
        spans.enums.len(),
        spans.enum_variants.len(),
        spans.enum_variant_fields.len(),
    )
}

fn checker_body_side_state(
    checker: &Checker<'_, '_>,
    owner: OwnerId,
    decl_id: DeclarationId,
    expected_self_ordinary_behavior: usize,
) -> BodySideState {
    BodySideState {
        next_runtime_id: checker.next_runtime_id,
        definition_identity_next: checker.definition_identities.next(),
        generated_functions: checker.generated_functions.snapshot_counts(),
        structs: checker.structs.len(),
        enums: checker.enums.len(),
        newtypes: checker.newtypes.len(),
        alias_contracts: checker.alias_contracts.len(),
        computed_scalar_packets: checker.computed_scalar_packets.len(),
        internal_return_empty_replacements: checker.internal_return_empty_replacements.len(),
        code_packet_nominals: checker.code_packet_nominals.len(),
        runtime_domains: checker.runtime_domains.len(),
        global_runtime_deliveries: checker.global_runtime_deliveries.len(),
        pending_default_stability: checker.pending_default_stability.len(),
        pending_predicate_stability: checker.pending_predicate_stability.len(),
        refinements: checker.refinements.len(),
        pending_refinements: checker.pending_refinements.len(),
        volatile_storage: checker.volatile_storage.entry_count(),
        layout_alignments: checker.layout_alignments.entry_count(),
        observed_associated_types: checker.observed_associated_types.len(),
        delivered_types: checker.delivered_types.len(),
        constants: checker.constants.len(),
        ordinary_behavior_other: checker
            .ordinary_behavior_functions
            .len()
            .saturating_sub(expected_self_ordinary_behavior),
        default_behavior_functions: checker.default_behavior_functions.len(),
        safety_other_levels: checker
            .safety_contracts
            .levels
            .len()
            .saturating_sub(usize::from(
                checker.safety_contracts.levels.contains_key(&decl_id),
            )),
        safety_other_decreases: checker.safety_contracts.decreases.len().saturating_sub(
            usize::from(checker.safety_contracts.decreases.contains_key(&decl_id)),
        ),
        match_plans_other: checker
            .match_plans
            .plans
            .keys()
            .filter(|key| key.function != Some(decl_id))
            .count(),
        declared_theory_nullary: checker.declared_theory.formed_nullary_nominals().len(),
        spec_functions_other: checker
            .spec_functions
            .len()
            .saturating_sub(usize::from(checker.spec_functions.contains_key(&owner))),
        checked_runtime_functions_other: checker.checked_runtime_functions.len().saturating_sub(
            usize::from(checker.checked_runtime_functions.contains_key(&owner)),
        ),
        checked_pattern_functions: checker.checked_pattern_functions.len(),
        reflection_spans: reflection_span_counts(&checker.reflection_spans),
        closed_const_regions: checker.closed_const_regions.len(),
        open_functions_len: checker.open_functions.len(),
        checked_implementations: checker.checked_implementations.len(),
        signature_type_uses: checker.signature_type_uses.len(),
        pending_const_addresses: checker.pending_const_addresses.len(),
    }
}

fn definition_is_checked(def: &open_functions::Definition) -> bool {
    struct Detector {
        matched: usize,
        found: bool,
    }
    impl std::fmt::Write for Detector {
        fn write_str(&mut self, s: &str) -> std::fmt::Result {
            const TARGET: &[u8] = b"(definition-checked true)";
            for &b in s.as_bytes() {
                if b == TARGET[self.matched] {
                    self.matched += 1;
                    if self.matched == TARGET.len() {
                        self.found = true;
                        return Err(std::fmt::Error);
                    }
                } else {
                    self.matched = usize::from(b == TARGET[0]);
                }
            }
            Ok(())
        }
    }
    let mut detector = Detector {
        matched: 0,
        found: false,
    };
    let _ = std::fmt::write(&mut detector, format_args!("{def}"));
    detector.found
}

fn predicate_roles_compatible(
    checker: &Checker<'_, '_>,
    roles: &[(n::SourceRange, n::PredicateRole)],
) -> bool {
    roles.iter().all(|(site, role)| {
        checker
            .safety_contracts
            .predicate_roles
            .get(site)
            .is_none_or(|existing| existing == role)
    })
}

fn replay_side_outputs(
    checker: &mut Checker<'_, '_>,
    decl_id: DeclarationId,
    match_plans: &[(n::MatchPlanKey, n::MatchPlan)],
    safety_level: Option<n::SafetyLevel>,
    decreases: Option<&Vec<n::Expr>>,
    predicate_roles: &[(n::SourceRange, n::PredicateRole)],
    added_expansion_sources: &[n::ExpansionSource],
    temp_binding_delta: u32,
) {
    for (key, plan) in match_plans {
        checker.match_plans.insert(*key, plan.clone());
    }
    if let Some(level) = safety_level {
        checker.safety_contracts.levels.insert(decl_id, level);
    }
    if let Some(decreases) = decreases {
        checker
            .safety_contracts
            .decreases
            .insert(decl_id, decreases.clone());
    }
    for (site, role) in predicate_roles {
        checker
            .safety_contracts
            .predicate_roles
            .insert(*site, *role);
    }
    checker
        .expansion_sources
        .extend_from_slice(added_expansion_sources);
    checker.next_temporary_binding += temp_binding_delta;
}

fn collect_body_match_plans(
    checker: &Checker<'_, '_>,
    decl_id: DeclarationId,
) -> Vec<(n::MatchPlanKey, n::MatchPlan)> {
    checker
        .match_plans
        .plans
        .iter()
        .filter(|(key, _)| key.function == Some(decl_id))
        .map(|(key, plan)| (*key, plan.clone()))
        .collect()
}

pub(super) fn expected_package_defaults(
    checker: &Checker<'_, '_>,
    owner: OwnerId,
    template_behavior: bool,
) -> Vec<(usize, n::Expr)> {
    if template_behavior {
        return Vec::new();
    }
    checker.defaults[&owner]
        .iter()
        .enumerate()
        .filter_map(|(index, value)| value.clone().map(|value| (index, value)))
        .collect()
}

pub(super) struct UpstreamBodyRecorder {
    pub(super) bodies: CachedUpstreamBodies,
    known_predicate_roles: BTreeSet<n::SourceRange>,
    known_unchecked_open: BTreeSet<OwnerId>,
}

pub(super) struct PendingOpenRecord {
    owner: OwnerId,
    decl_id: DeclarationId,
    initial_fingerprint: String,
    initial_requires_open: bool,
    before_state: BodySideState,
    before_expansion_len: usize,
    before_temp: u32,
    before_roles_len: usize,
    before_ordinary_len: usize,
}

pub(super) struct PendingFunctionRecord {
    owner: OwnerId,
    decl_id: DeclarationId,
    template_behavior: bool,
    is_spec: bool,
    before_state: BodySideState,
    before_expansion_len: usize,
    before_temp: u32,
    before_roles_len: usize,
}

impl UpstreamBodyRecorder {
    pub(super) fn new(checker: &Checker<'_, '_>) -> Self {
        let known_predicate_roles = checker
            .safety_contracts
            .predicate_roles
            .keys()
            .copied()
            .collect();
        let known_unchecked_open = checker
            .open_functions
            .iter()
            .filter(|(_, def)| !definition_is_checked(def))
            .map(|(&owner, _)| owner)
            .collect();
        Self {
            bodies: CachedUpstreamBodies::default(),
            known_predicate_roles,
            known_unchecked_open,
        }
    }

    fn take_new_predicate_roles(
        &mut self,
        checker: &Checker<'_, '_>,
        before_roles_len: usize,
    ) -> Option<Vec<(n::SourceRange, n::PredicateRole)>> {
        let after_len = checker.safety_contracts.predicate_roles.len();
        if after_len == before_roles_len {
            return Some(Vec::new());
        }
        let mut added = Vec::new();
        for (&site, &role) in &checker.safety_contracts.predicate_roles {
            if self.known_predicate_roles.insert(site) {
                added.push((site, role));
            }
        }
        if after_len == before_roles_len + added.len() {
            Some(added)
        } else {
            None
        }
    }

    pub(super) fn begin_open(
        &self,
        checker: &Checker<'_, '_>,
        owner: OwnerId,
        decl_id: DeclarationId,
    ) -> Option<PendingOpenRecord> {
        let def = checker.open_functions.get(&owner)?;
        if definition_is_checked(def) {
            return None;
        }
        Some(PendingOpenRecord {
            owner,
            decl_id,
            initial_fingerprint: def.to_string(),
            initial_requires_open: def.requires_open_instance(),
            before_state: checker_body_side_state(checker, owner, decl_id, 0),
            before_expansion_len: checker.expansion_sources.len(),
            before_temp: checker.next_temporary_binding,
            before_roles_len: checker.safety_contracts.predicate_roles.len(),
            before_ordinary_len: checker.ordinary_behavior_functions.len(),
        })
    }

    pub(super) fn finish_open(&mut self, checker: &Checker<'_, '_>, pending: PendingOpenRecord) {
        let added_roles = self.take_new_predicate_roles(checker, pending.before_roles_len);
        let mut touched_other_open = false;
        let mut newly_checked = Vec::new();
        for &other in &self.known_unchecked_open {
            if checker
                .open_functions
                .get(&other)
                .is_some_and(definition_is_checked)
            {
                newly_checked.push(other);
                if other != pending.owner {
                    touched_other_open = true;
                }
            }
        }
        for id in newly_checked {
            self.known_unchecked_open.remove(&id);
        }
        if touched_other_open {
            return;
        }
        let Some(predicate_roles) = added_roles else {
            return;
        };
        let ordinary_after = checker.ordinary_behavior_functions.len();
        let added_ordinary = ordinary_after.checked_sub(pending.before_ordinary_len);
        let (ordinary_behavior_function, expected_self_ordinary) = match added_ordinary {
            Some(0) => (None, 0),
            Some(1) => {
                let candidate = &checker.ordinary_behavior_functions[pending.before_ordinary_len];
                if candidate.declaration != pending.decl_id {
                    return;
                }
                (Some(candidate.clone()), 1)
            }
            _ => {
                return;
            }
        };
        let after_state = checker_body_side_state(
            checker,
            pending.owner,
            pending.decl_id,
            expected_self_ordinary,
        );
        if after_state != pending.before_state {
            return;
        }
        let Some(temp_binding_delta) = checker
            .next_temporary_binding
            .checked_sub(pending.before_temp)
        else {
            return;
        };
        if checker.expansion_sources.len() < pending.before_expansion_len {
            return;
        }
        let added_expansion_sources =
            checker.expansion_sources[pending.before_expansion_len..].to_vec();
        let Some(checked_definition) = checker.open_functions.get(&pending.owner).cloned() else {
            return;
        };
        if !definition_is_checked(&checked_definition) {
            return;
        }
        self.bodies.open_definitions.insert(
            pending.decl_id,
            CachedOpenDefinition {
                initial_fingerprint: pending.initial_fingerprint,
                initial_requires_open: pending.initial_requires_open,
                checked_definition,
                ordinary_behavior_function,
                match_plans: collect_body_match_plans(checker, pending.decl_id),
                safety_level: checker
                    .safety_contracts
                    .levels
                    .get(&pending.decl_id)
                    .copied(),
                decreases: checker
                    .safety_contracts
                    .decreases
                    .get(&pending.decl_id)
                    .cloned(),
                predicate_roles,
                expansion_sources_base: pending.before_expansion_len,
                added_expansion_sources,
                temp_binding_base: pending.before_temp,
                temp_binding_delta,
            },
        );
    }

    pub(super) fn begin_function(
        &self,
        checker: &Checker<'_, '_>,
        owner: OwnerId,
        decl_id: DeclarationId,
        template_behavior: bool,
        is_spec: bool,
    ) -> PendingFunctionRecord {
        PendingFunctionRecord {
            owner,
            decl_id,
            template_behavior,
            is_spec,
            before_state: checker_body_side_state(checker, owner, decl_id, 0),
            before_expansion_len: checker.expansion_sources.len(),
            before_temp: checker.next_temporary_binding,
            before_roles_len: checker.safety_contracts.predicate_roles.len(),
        }
    }

    pub(super) fn finish_function(
        &mut self,
        checker: &Checker<'_, '_>,
        pending: PendingFunctionRecord,
        function: &n::Function,
    ) {
        let Some(predicate_roles) =
            self.take_new_predicate_roles(checker, pending.before_roles_len)
        else {
            return;
        };
        let after_state = checker_body_side_state(checker, pending.owner, pending.decl_id, 0);
        if after_state != pending.before_state {
            return;
        }
        let Some(temp_binding_delta) = checker
            .next_temporary_binding
            .checked_sub(pending.before_temp)
        else {
            return;
        };
        if checker.expansion_sources.len() < pending.before_expansion_len {
            return;
        }
        let added_expansion_sources =
            checker.expansion_sources[pending.before_expansion_len..].to_vec();
        self.bodies.functions.insert(
            pending.decl_id,
            CachedUpstreamFunction {
                function: function.clone(),
                is_template_behavior: pending.template_behavior,
                is_spec: pending.is_spec,
                match_plans: collect_body_match_plans(checker, pending.decl_id),
                safety_level: checker
                    .safety_contracts
                    .levels
                    .get(&pending.decl_id)
                    .copied(),
                decreases: checker
                    .safety_contracts
                    .decreases
                    .get(&pending.decl_id)
                    .cloned(),
                predicate_roles,
                expansion_sources_base: pending.before_expansion_len,
                added_expansion_sources,
                temp_binding_base: pending.before_temp,
                temp_binding_delta,
            },
        );
    }
}

pub(super) fn try_reuse_open_definition(
    checker: &mut Checker<'_, '_>,
    incremental: &IncrementalOptions,
    owner: OwnerId,
    decl_id: DeclarationId,
) -> bool {
    if !incremental
        .verified_upstream_packages
        .contains(&decl_id.package)
        || checker.source_declarations.owner(decl_id) != Some(owner)
    {
        return false;
    }
    let Some(cache) = incremental.cached_upstream_bodies.as_ref() else {
        return false;
    };
    let Some(cached) = cache.open_definitions.get(&decl_id) else {
        return false;
    };
    let Some(current) = checker.open_functions.get(&owner) else {
        return false;
    };
    let needs_default_behavior = checker.default_behavior_functions.contains(&owner)
        && !checker
            .ordinary_behavior_functions
            .iter()
            .any(|function| function.declaration == decl_id);
    if definition_is_checked(current) && !needs_default_behavior {
        return true;
    }
    if current.requires_open_instance() != cached.initial_requires_open
        || current.to_string() != cached.initial_fingerprint
    {
        return false;
    }
    if !cached.added_expansion_sources.is_empty()
        && checker.expansion_sources.len() != cached.expansion_sources_base
    {
        return false;
    }
    if cached.temp_binding_delta != 0 && checker.next_temporary_binding != cached.temp_binding_base
    {
        return false;
    }
    if !predicate_roles_compatible(checker, &cached.predicate_roles) {
        return false;
    }
    if let Some(behavior) = &cached.ordinary_behavior_function {
        if behavior.interface.type_parameters != checker.type_parameters[&owner]
            || behavior.interface.requirements != checker.definition_requirements(owner)
            || behavior.interface.capabilities
                != checker
                    .capabilities
                    .get(&owner)
                    .map_or(&[][..], Vec::as_slice)
        {
            return false;
        }
    }
    checker
        .open_functions
        .insert(owner, cached.checked_definition.clone());
    if let Some(behavior) = &cached.ordinary_behavior_function {
        checker.ordinary_behavior_functions.push(behavior.clone());
    }
    replay_side_outputs(
        checker,
        decl_id,
        &cached.match_plans,
        cached.safety_level,
        cached.decreases.as_ref(),
        &cached.predicate_roles,
        &cached.added_expansion_sources,
        cached.temp_binding_delta,
    );
    true
}

#[allow(clippy::too_many_arguments)]
pub(super) fn try_reuse_function(
    checker: &mut Checker<'_, '_>,
    output: &mut n::Package,
    incremental: &IncrementalOptions,
    owner: OwnerId,
    decl_id: DeclarationId,
    signature: &Term,
    template_behavior: bool,
    is_spec: bool,
    is_plain_runtime: bool,
    eligible_upstream_decl: bool,
) -> bool {
    if !eligible_upstream_decl
        || !incremental
            .verified_upstream_packages
            .contains(&decl_id.package)
    {
        return false;
    }
    let Some(cache) = incremental.cached_upstream_bodies.as_ref() else {
        return false;
    };
    let Some(cached_fn) = cache.functions.get(&decl_id) else {
        return false;
    };
    let (expected_result_binder, expected_result_guarantees) = checker
        .result_guarantees
        .get(&owner)
        .map_or((None, &[][..]), |(binder, guarantees)| {
            (Some(*binder), guarantees.as_slice())
        });
    if cached_fn.is_template_behavior != template_behavior
        || cached_fn.is_spec != is_spec
        || cached_fn.function.declaration != decl_id
        || cached_fn.function.interface.signature != *signature
        || cached_fn.function.interface.result_type_binder != expected_result_binder
        || cached_fn.function.interface.result_guarantees.as_slice() != expected_result_guarantees
        || cached_fn.function.interface.type_parameters != checker.type_parameters[&owner]
        || cached_fn.function.interface.requirements != checker.definition_requirements(owner)
        || cached_fn.function.interface.capabilities
            != checker
                .capabilities
                .get(&owner)
                .map_or(&[][..], Vec::as_slice)
        || cached_fn.function.interface.defaults
            != expected_package_defaults(checker, owner, template_behavior)
    {
        return false;
    }
    if !cached_fn.added_expansion_sources.is_empty()
        && checker.expansion_sources.len() != cached_fn.expansion_sources_base
    {
        return false;
    }
    if cached_fn.temp_binding_delta != 0
        && checker.next_temporary_binding != cached_fn.temp_binding_base
    {
        return false;
    }
    if !predicate_roles_compatible(checker, &cached_fn.predicate_roles) {
        return false;
    }
    if is_spec {
        checker
            .spec_functions
            .entry(owner)
            .or_insert_with(|| cached_fn.function.clone());
    } else if is_plain_runtime {
        checker
            .checked_runtime_functions
            .entry(owner)
            .or_insert_with(|| cached_fn.function.clone());
    }
    if template_behavior {
        checker
            .ordinary_behavior_functions
            .push(cached_fn.function.clone());
    } else {
        output.functions.push(cached_fn.function.clone());
    }
    replay_side_outputs(
        checker,
        decl_id,
        &cached_fn.match_plans,
        cached_fn.safety_level,
        cached_fn.decreases.as_ref(),
        &cached_fn.predicate_roles,
        &cached_fn.added_expansion_sources,
        cached_fn.temp_binding_delta,
    );
    true
}
