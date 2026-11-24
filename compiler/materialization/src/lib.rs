//! Compile-time closure of concrete function/interface dependencies. Templates
//! stay in NIR; only fixed selections are exported to the execution table.
mod associated_selection;

#[cfg(test)]
mod callable_tests;

pub use associated_selection::resolve_associated_value_for_invocation;

mod captured_templates;
mod conditional_calls;
mod dead_tails;
mod input_admission;
mod mutable_callbacks;
mod publication;
mod query_budget;
mod source_queries;

pub use source_queries::{
    AssociatedAnswer, SelectedSourceRequest, SourceInvocationQueries, check_world_predicate,
    resolve_method_for_invocation,
};

mod recursive_factories;
mod staged;
mod template_factories;

use crate::ptc::{Budget, Context};
use type_resolution::{associated, capabilities, ptc};

mod dispatch_function;
mod expansion;

pub use expansion::{DefinitionPublisher, Expansion};
use nir::program::{self as n, ExprKind as K, Pattern as P};
use nir::{DeclarationId as Id, Term};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use type_resolution::dispatch::DeclarationWorld;
pub use type_resolution::dispatch::{
    BorrowedCatalog, ClosedDeclarations, DeclarationSnapshot, DeclarationSource, MemberDataSource,
};

// Interface defaults use interface parameter identities, independently of the
// selected body's parameter names. Only their free references move to the body
// namespace; closure-owned bindings remain local to that closure.
fn rebind_default_locals(
    expression: &mut n::Expr,
    aliases: &std::collections::BTreeMap<n::LocalId, n::LocalId>,
    fuel: &mut usize,
    depth: usize,
) -> Result<(), String> {
    if depth >= 128 {
        return Err("staged method default scope depth exceeded".into());
    }
    iir::visit::expression_mut_pruned(expression, fuel, |expr, fuel| {
        if let K::Closure {
            captures,
            parameters,
            body,
        } = &mut expr.kind
        {
            *fuel = fuel
                .checked_sub(aliases.len() + captures.len() + parameters.len())
                .ok_or("staged method default binding budget exhausted")?;
            let mut inner = aliases.clone();
            for parameter in parameters {
                inner.remove(parameter);
            }
            for capture in captures {
                inner.remove(&capture.binding);
                if let Some(value) = &mut capture.value {
                    rebind_default_locals(value, aliases, fuel, depth + 1)?;
                } else if let Some(binding) = aliases.get(&capture.binding) {
                    capture.value = Some(Box::new(n::Expr {
                        source: expr.source,
                        ty: capture.ty.clone(),
                        kind: K::Local(*binding),
                    }));
                }
            }
            rebind_default_locals(std::sync::Arc::make_mut(body), &inner, fuel, depth + 1)?;
            return Ok(false);
        }
        match &mut expr.kind {
            K::Local(binding) => {
                if let Some(replacement) = aliases.get(binding) {
                    *binding = *replacement;
                }
            }
            K::Assign { place, .. } | K::BitAssign { place, .. } | K::BitUpdate { place, .. } => {
                if let Some(replacement) = aliases.get(&place.root) {
                    place.root = *replacement;
                }
            }
            _ => {}
        }
        Ok(true)
    })
}

enum Task {
    Function(Id, Vec<Term>),
    Interface(n::InterfaceInstance, Term),
    Type(Term),
}

struct Builder<'a, 'e> {
    expansion: Option<&'e dyn Expansion>,
    quote_cursor: u32,
    inline_effects: BTreeSet<Id>,
    active_staged_inlines: BTreeSet<Id>,
    quote_sources: Vec<n::ExpansionSource>,
    retain_templates: bool,
    package: &'a n::Package,
    world: DeclarationWorld<'a>,
    value_scope: input_admission::ValueScope,
    queue: VecDeque<Task>,
    functions: HashSet<(Id, Vec<Term>)>,
    staged_functions: HashMap<(Id, Vec<Term>), n::Function>,
    generated_functions: HashMap<Id, n::Function>,
    recursive_factories: HashMap<iir::InstanceKey, recursive_factories::Selection>,
    invocation_snapshot: Option<staged::Snapshot>,
    types: HashSet<Term>,
    // Definition-side views may retain abstract projections. Never feed this
    // cache into strict normalization or publish it as completed IIR evidence.
    staging_views: HashMap<Term, Term>,
    interfaces: HashMap<(n::InterfaceInstance, Term), usize>,
    associated_demands: HashSet<(usize, Id)>,
    output: Vec<n::DispatchInstance>,
    fuel: usize,
    budget: Budget,
    instances: Option<Vec<iir::FunctionInstance>>,
    runtime_bindings: BTreeSet<nir::RuntimeId>,
    global_runtime_deliveries: BTreeMap<nir::RuntimeId, n::Expr>,
    nominals: Vec<iir::NominalInstance>,
    interface_contracts: Vec<iir::InterfaceContract>,
    effect_contracts: Vec<iir::effects::Contract>,
    effect_instances: HashSet<nir::effects::Operation>,
    frozen_methods: Vec<n::Expr>,
    callable_identities: Vec<iir::CallableIdentity>,
    type_cores: nir::TypeInterner,
    expression_owner: Option<u64>,
    active_caller_env: Option<(
        Vec<n::RuntimeEnvParameter>,
        BTreeMap<nir::RuntimeId, n::Expr>,
    )>,
    active_closure_scopes: Vec<BTreeSet<n::LocalId>>,
}

/// Bounded, transactional: failure returns no partially materialized world.
pub fn dispatch_instances(
    package: &n::Package,
    budget: Budget,
) -> Result<Vec<n::DispatchInstance>, String> {
    build(
        DeclarationSource::Borrowed(package),
        budget,
        Roots::All,
        BTreeSet::new(),
        &[],
        None,
        None,
    )
    .map(|result| result.dispatch)
}

/// Prepare transitive dispatch dependencies of explicit entry points and the
/// parent/capability closure of concrete implementations retained by the NIR VM.
/// Unrelated function bodies remain unvisited.
/// This preserves NIR function bodies and does not impose runtime IIR closure
/// or layout requirements on a comptime invocation.
pub fn dispatch_instances_for_roots(
    package: &n::Package,
    roots: &[iir::InstanceKey],
    budget: Budget,
) -> Result<Vec<n::DispatchInstance>, String> {
    build(
        DeclarationSource::Borrowed(package),
        budget,
        Roots::Dispatch(roots),
        BTreeSet::new(),
        &[],
        None,
        None,
    )
    .map(|result| result.dispatch)
}

/// Prepare reachable dispatch while retaining ordinary comptime input telescopes
/// for binding by the interpreter at invocation time. Type and value inputs both
/// remain invocation binders; this does not authorize their IIR materialization.
pub fn dispatch_instances_for_invocation(
    package: &n::Package,
    roots: &[iir::InstanceKey],
    budget: Budget,
) -> Result<Vec<n::DispatchInstance>, String> {
    build(
        DeclarationSource::Borrowed(package),
        budget,
        Roots::Invocation(roots),
        BTreeSet::new(),
        &[],
        None,
        None,
    )
    .map(|result| result.dispatch)
}

/// Resolve one concrete method through the same checked dispatch closure used
/// by execution. Consumers must still enforce the interface's call contract;
/// the selected body identity alone is not permission to access memory.
pub fn resolve_method_instance(
    package: &n::Package,
    callee: &n::Expr,
    fuel: &mut usize,
) -> Result<n::MethodInstance, String> {
    resolve_method_instance_in_source(DeclarationSource::Borrowed(package), callee, fuel)
}

pub fn resolve_method_instance_in_source(
    source: DeclarationSource<'_>,
    callee: &n::Expr,
    fuel: &mut usize,
) -> Result<n::MethodInstance, String> {
    let mut selected = None;
    let result = build(
        source,
        Budget {
            steps: *fuel,
            ..Budget::default()
        },
        Roots::Method(callee, &mut selected),
        BTreeSet::new(),
        &[],
        None,
        None,
    );
    match result {
        Ok(result) => {
            *fuel = result.remaining;
            selected.ok_or_else(|| "method selection produced no body identity".into())
        }
        Err(reason) => {
            // Failed speculative selections cannot replenish their work budget.
            *fuel = 0;
            Err(reason)
        }
    }
}

/// Prepare an invocation-demanded method identity in the frozen source world.
/// No implementation body is traversed or executed by this selection.
pub fn resolve_method_identity_in_context(
    package: &n::Package,
    callee: &n::Expr,
    context: &Context,
    assumptions: &[n::TraitRequirement],
    fuel: &mut usize,
) -> Result<n::MethodInstance, nir::NominalAdmissionError> {
    SourceInvocationQueries::new(package, context, assumptions).identity(callee, fuel)
}

/// Normalize annotations on a private expansion candidate without invoking
/// code, selecting bodies or publishing an executable instance. Unresolved
/// definition-side projections remain views; opaque heads retain identity.
struct StagingAnnotationRewriter<F>(F);

impl<F: FnMut(&Term) -> Result<Term, String>> n::Rewriter for StagingAnnotationRewriter<F> {
    const SPECIALIZE: bool = true;

    type Error = String;

    fn ty(&mut self, ty: &mut Term) -> Result<(), String> {
        *ty = (self.0)(ty)?;
        Ok(())
    }

    fn expression(&mut self, expression: &mut n::Expr) -> Result<bool, String> {
        // An interface reference retains the declaration-side signature. Its
        // associated result is normalized on the enclosing call, not erased
        // from the method contract before dispatch/materialization checks it.
        Ok(!matches!(expression.kind, K::TraitMethod { .. }))
    }
}

pub fn normalize_inline_annotations(
    package: &n::Package,
    function: &mut n::Function,
    fuel: &mut usize,
) -> Result<(), String> {
    let result = build(
        DeclarationSource::Borrowed(package),
        Budget {
            steps: *fuel,
            ..Budget::default()
        },
        Roots::Annotations(function),
        BTreeSet::new(),
        &[],
        None,
        None,
    );
    match result {
        Ok(result) => {
            *fuel = result.remaining;
            Ok(())
        }
        Err(reason) => {
            *fuel = 0;
            Err(reason)
        }
    }
}

/// Select a quote body without enqueueing it as an executable function.
pub fn resolve_quote_method_instance(
    package: &n::Package,
    callee: &n::Expr,
    fuel: &mut usize,
) -> Result<n::MethodInstance, String> {
    resolve_quote_method_instance_in_source(DeclarationSource::Borrowed(package), callee, fuel)
}

/// Select a quote body using a shared declaration source without enqueueing it
/// as an executable function.
pub fn resolve_quote_method_instance_in_source(
    source: DeclarationSource<'_>,
    callee: &n::Expr,
    fuel: &mut usize,
) -> Result<n::MethodInstance, String> {
    let mut selected = None;
    let result = build(
        source,
        Budget {
            steps: *fuel,
            ..Budget::default()
        },
        Roots::QuoteMethod(callee, &mut selected),
        BTreeSet::new(),
        &[],
        None,
        None,
    );
    match result {
        Ok(result) => {
            *fuel = result.remaining;
            selected.ok_or("quote selection produced no body identity".into())
        }
        Err(reason) => {
            *fuel = 0;
            Err(reason)
        }
    }
}

/// Private comptime snapshots may retain admitted rigid binder premises.
pub fn dispatch_instances_in_context(
    package: &n::Package,
    context: &Context,
    assumptions: &[n::TraitRequirement],
    value_parameters: &[(Id, Term)],
    live_methods: &[comptime_interpretor::TraitMethodIdentity],
    budget: &mut Budget,
) -> Result<Vec<n::DispatchInstance>, String> {
    let result = build(
        DeclarationSource::Borrowed(package),
        *budget,
        Roots::AllWithMethods(live_methods),
        BTreeSet::new(),
        &[],
        Some(DispatchContext {
            types: context,
            assumptions,
            value_parameters,
        }),
        None,
    );
    match result {
        Ok(result) => {
            budget.steps = result.remaining;
            Ok(result.dispatch)
        }
        Err(reason) => {
            // A failed private build publishes no graph and cannot refund
            // work performed before its diagnostic was produced.
            budget.steps = 0;
            Err(reason)
        }
    }
}

/// Materialize an explicit set of function instances. Failure publishes no
/// partial graph; recursive calls reuse the same instance identity.
pub fn instantiate(
    package: &n::Package,
    roots: &[iir::InstanceKey],
    runtime_bindings: BTreeSet<nir::RuntimeId>,
    budget: Budget,
    expansion: &dyn Expansion,
) -> Result<iir::Package, String> {
    instantiate_with_types(package, roots, &[], runtime_bindings, budget, expansion)
}

/// Retain layouts needed by external concrete operations, including cleanup.
pub fn instantiate_with_types(
    package: &n::Package,
    roots: &[iir::InstanceKey],
    types: &[nir::Term],
    runtime_bindings: BTreeSet<nir::RuntimeId>,
    budget: Budget,
    expansion: &dyn Expansion,
) -> Result<iir::Package, String> {
    instantiate_with_types_in_source(
        DeclarationSource::Borrowed(package),
        roots,
        types,
        runtime_bindings,
        budget,
        expansion,
    )
}

/// Reuse immutable declaration metadata; every materialization still owns its
/// proof context, expansion state, work queue and transactional output graph.
pub fn instantiate_in_source(
    source: DeclarationSource<'_>,
    roots: &[iir::InstanceKey],
    runtime_bindings: BTreeSet<nir::RuntimeId>,
    budget: Budget,
    expansion: &dyn Expansion,
) -> Result<iir::Package, String> {
    instantiate_with_types_in_source(source, roots, &[], runtime_bindings, budget, expansion)
}

/// Retain external layout roots in the same source-bound materialization.
pub fn instantiate_with_types_in_source(
    source: DeclarationSource<'_>,
    roots: &[iir::InstanceKey],
    types: &[nir::Term],
    runtime_bindings: BTreeSet<nir::RuntimeId>,
    mut budget: Budget,
    expansion: &dyn Expansion,
) -> Result<iir::Package, String> {
    instantiate_with_types_in_source_in_budget(
        source,
        roots,
        types,
        runtime_bindings,
        &mut budget,
        expansion,
    )
}

/// One construction allowance covers input phase checks, dependency discovery,
/// expansion, retained metadata and final graph validation. Failed construction
/// reserves the granted allowance and never returns a partial graph.
pub fn instantiate_with_types_in_source_in_budget(
    source: DeclarationSource<'_>,
    roots: &[iir::InstanceKey],
    types: &[nir::Term],
    runtime_bindings: BTreeSet<nir::RuntimeId>,
    budget: &mut Budget,
    expansion: &dyn Expansion,
) -> Result<iir::Package, String> {
    InstantiationSession::new(source).instantiate(roots, types, runtime_bindings, budget, expansion)
}

struct IncrementalIir<'a> {
    roots: Vec<iir::InstanceKey>,
    type_roots: Vec<nir::Term>,
    quote_cursor: u32,
    inline_effects: BTreeSet<Id>,
    quote_sources: Vec<n::ExpansionSource>,
    world: DeclarationWorld<'a>,
    value_scope: input_admission::ValueScope,
    functions: HashSet<(Id, Vec<Term>)>,
    staged_functions: HashMap<(Id, Vec<Term>), n::Function>,
    generated_functions: HashMap<Id, n::Function>,
    recursive_factories: HashMap<iir::InstanceKey, recursive_factories::Selection>,
    invocation_snapshot: Option<staged::Snapshot>,
    types: HashSet<Term>,
    staging_views: HashMap<Term, Term>,
    interfaces: HashMap<(n::InterfaceInstance, Term), usize>,
    associated_demands: HashSet<(usize, Id)>,
    output: Vec<n::DispatchInstance>,
    instances: Vec<iir::FunctionInstance>,
    runtime_bindings: BTreeSet<nir::RuntimeId>,
    global_runtime_deliveries: BTreeMap<nir::RuntimeId, n::Expr>,
    nominals: Vec<iir::NominalInstance>,
    interface_contracts: Vec<iir::InterfaceContract>,
    effect_contracts: Vec<iir::effects::Contract>,
    effect_instances: HashSet<nir::effects::Operation>,
    frozen_methods: Vec<n::Expr>,
    callable_identities: Vec<iir::CallableIdentity>,
    type_cores: nir::TypeInterner,
}

/// Repeated graph construction in one immutable source. Only completed source
/// phase validation survives `instantiate`; `extend` transactionally retains
/// monotonic incremental IIR state across prefix-growing requests.
pub struct InstantiationSession<'a> {
    source: DeclarationSource<'a>,
    phase_checked: bool,
    incremental: Option<IncrementalIir<'a>>,
}

impl<'a> InstantiationSession<'a> {
    pub fn new(source: DeclarationSource<'a>) -> Self {
        Self {
            source,
            phase_checked: false,
            incremental: None,
        }
    }

    pub fn instantiate(
        &mut self,
        roots: &[iir::InstanceKey],
        types: &[nir::Term],
        runtime_bindings: BTreeSet<nir::RuntimeId>,
        budget: &mut Budget,
        expansion: &dyn Expansion,
    ) -> Result<iir::Package, String> {
        self.incremental = None;
        let mut construction_budget = *budget;
        budget.steps = 0;
        let source = self.source;
        let package = source.package();
        construction_budget.steps = construction_budget
            .steps
            .checked_sub(1)
            .ok_or("IIR phase evidence access budget exhausted")?;
        if !self.phase_checked {
            source
                .phase_inspection()
                .check_package(&mut construction_budget.steps)
                .map_err(|e| format!("IIR input phase: {}", e.message))?;
            // This certifies only source phase structure, even if later graph
            // construction fails. No instance graph or caller evidence is retained.
            self.phase_checked = true;
        }
        let mut runtime_bindings = runtime_bindings;
        runtime_bindings.extend(package.runtime_domains.keys().copied());
        construction_budget.steps = construction_budget
            .steps
            .checked_sub(runtime_bindings.len())
            .ok_or("IIR runtime binding copy budget exhausted")?;
        let BuildResult {
            published_source,
            mut dispatch,
            functions,
            nominals,
            interfaces,
            expansion_sources,
            effect_contracts,
            callable_identities,
            global_runtime_deliveries,
            frozen_methods,
            mut type_cores,
            remaining: mut publication_fuel,
        } = build(
            source,
            construction_budget,
            Roots::Iir(roots),
            runtime_bindings.clone(),
            types,
            None,
            Some(expansion),
        )?;
        let templates = captured_templates::storage(
            &functions,
            &frozen_methods,
            published_source
                .as_ref()
                .map(|s| s.package().as_ref())
                .unwrap_or(package),
            &dispatch,
            &mut type_cores,
            &mut publication_fuel,
        )?;
        // A selected interface may have conditionally available or simply unused
        // methods. Publish executable links only for demanded concrete bodies.
        publication::retain_demanded_methods(
            &functions,
            &templates,
            &callable_identities,
            &mut dispatch,
            &mut publication_fuel,
        )?;
        publication_fuel = publication_fuel
            .checked_sub(roots.len())
            .ok_or("IIR root copy budget exhausted")?;
        let normal_forms =
            publication::copy_normal_forms(&package.normal_forms, &mut publication_fuel)?;
        let source_pkg = published_source
            .as_ref()
            .map_or(package, |snapshot| snapshot.package().as_ref());
        let source_volatile = &source_pkg.volatile_storage;
        publication_fuel = publication_fuel
            .checked_sub(source_volatile.entry_count())
            .ok_or("IIR volatile storage copy budget exhausted")?;
        let source_match_plans = &source_pkg.match_plans;
        publication_fuel = publication_fuel
            .checked_sub(source_match_plans.entry_count())
            .ok_or("IIR match plans copy budget exhausted")?;
        let output = iir::Package {
            target: package.target,
            expansion_sources,
            effect_contracts,
            roots: roots.to_vec(),
            functions,
            templates,
            dispatch,
            runtime_bindings,
            global_runtime_deliveries,
            volatile_storage: source_volatile.clone(),
            match_plans: source_match_plans.clone(),
            nominals,
            interfaces,
            normal_forms,
            callable_identities,
            statics: package.statics.clone(),
        };
        validate_materialized_bit_convert_safety(
            &output,
            published_source
                .as_ref()
                .map(|s| s.package().as_ref())
                .unwrap_or(package),
            &mut publication_fuel,
        )?;
        output.validate(&mut publication_fuel)?;
        budget.steps = publication_fuel;
        Ok(output)
    }

    pub fn extend(
        &mut self,
        roots: &[iir::InstanceKey],
        types: &[nir::Term],
        runtime_bindings: BTreeSet<nir::RuntimeId>,
        budget: &mut Budget,
        expansion: &dyn Expansion,
    ) -> Result<iir::Package, String> {
        let prior = self.incremental.take();
        let mut construction_budget = *budget;
        budget.steps = 0;
        let source = self.source;
        let package = source.package();
        construction_budget.steps = construction_budget
            .steps
            .checked_sub(1)
            .ok_or("IIR phase evidence access budget exhausted")?;
        if !self.phase_checked {
            source
                .phase_inspection()
                .check_package(&mut construction_budget.steps)
                .map_err(|e| format!("IIR input phase: {}", e.message))?;
            self.phase_checked = true;
        }
        let mut runtime_bindings = runtime_bindings;
        runtime_bindings.extend(package.runtime_domains.keys().copied());
        construction_budget.steps = construction_budget
            .steps
            .checked_sub(runtime_bindings.len())
            .ok_or("IIR runtime binding copy budget exhausted")?;

        let (mut builder, new_roots, new_types, is_cold) = match prior {
            Some(prev)
                if prev.runtime_bindings == runtime_bindings
                    && roots.starts_with(&prev.roots)
                    && types.starts_with(&prev.type_roots) =>
            {
                let root_prefix = prev.roots.len();
                let type_prefix = prev.type_roots.len();
                let builder = Builder {
                    expansion: Some(expansion),
                    quote_cursor: prev.quote_cursor,
                    inline_effects: prev.inline_effects,
                    active_staged_inlines: BTreeSet::new(),
                    quote_sources: prev.quote_sources,
                    retain_templates: false,
                    package,
                    world: prev.world,
                    value_scope: prev.value_scope,
                    queue: VecDeque::new(),
                    functions: prev.functions,
                    staged_functions: prev.staged_functions,
                    generated_functions: prev.generated_functions,
                    recursive_factories: prev.recursive_factories,
                    invocation_snapshot: prev.invocation_snapshot,
                    types: prev.types,
                    staging_views: prev.staging_views,
                    interfaces: prev.interfaces,
                    associated_demands: prev.associated_demands,
                    output: prev.output,
                    fuel: construction_budget.steps,
                    budget: construction_budget,
                    instances: Some(prev.instances),
                    runtime_bindings: prev.runtime_bindings,
                    global_runtime_deliveries: prev.global_runtime_deliveries,
                    nominals: prev.nominals,
                    interface_contracts: prev.interface_contracts,
                    effect_contracts: prev.effect_contracts,
                    effect_instances: prev.effect_instances,
                    frozen_methods: prev.frozen_methods,
                    callable_identities: prev.callable_identities,
                    type_cores: prev.type_cores,
                    expression_owner: None,
                    active_caller_env: None,
                    active_closure_scopes: Vec::new(),
                };
                (builder, &roots[root_prefix..], &types[type_prefix..], false)
            }
            _ => {
                let world = source.world(&mut construction_budget, None)?;
                let mut input_fuel = construction_budget.steps;
                let value_scope = input_admission::ValueScope::new(&[], &mut input_fuel)?;
                input_fuel = input_fuel
                    .checked_sub(
                        package
                            .effects
                            .len()
                            .saturating_add(package.expansion_sources.len()),
                    )
                    .ok_or("materialized inline effect index budget exhausted")?;
                let inline_effects = package
                    .effects
                    .iter()
                    .filter(|effect| effect.inline && effect.abi == nir::effects::Abi::Direct)
                    .map(|effect| effect.declaration)
                    .collect();
                let builder = Builder {
                    expansion: Some(expansion),
                    quote_cursor: package.quote_identity_cursor,
                    inline_effects,
                    active_staged_inlines: BTreeSet::new(),
                    quote_sources: package.expansion_sources.clone(),
                    retain_templates: false,
                    package,
                    world,
                    value_scope,
                    queue: VecDeque::new(),
                    functions: HashSet::new(),
                    staged_functions: HashMap::new(),
                    generated_functions: HashMap::new(),
                    recursive_factories: HashMap::new(),
                    invocation_snapshot: None,
                    types: HashSet::new(),
                    staging_views: HashMap::new(),
                    interfaces: HashMap::new(),
                    associated_demands: HashSet::new(),
                    output: vec![],
                    fuel: input_fuel,
                    budget: construction_budget,
                    instances: Some(Vec::new()),
                    runtime_bindings,
                    global_runtime_deliveries: BTreeMap::new(),
                    nominals: vec![],
                    interface_contracts: vec![],
                    effect_contracts: vec![],
                    effect_instances: HashSet::new(),
                    frozen_methods: vec![],
                    callable_identities: vec![],
                    type_cores: nir::TypeInterner::default(),
                    expression_owner: None,
                    active_caller_env: None,
                    active_closure_scopes: Vec::new(),
                };
                (builder, roots, types, true)
            }
        };

        for ty in new_types {
            let ty = builder.normalized_type(ty)?;
            builder.queue.push_back(Task::Type(ty));
        }
        for root in new_roots {
            builder.tick(0)?;
            for argument in root.arguments.iter() {
                crate::associated::charge_projection_copy(argument, &mut builder.fuel)
                    .map_err(|_| "dispatch root argument type budget exhausted".to_owned())?;
            }
            builder
                .queue
                .push_back(Task::Function(root.declaration, root.arguments.to_vec()));
        }
        if is_cold {
            for (&id, expr) in &package.global_runtime_deliveries {
                builder.tick(0)?;
                let mut remaining = u64::try_from(builder.fuel).unwrap_or(u64::MAX);
                nir::copy_budget::charge_expression(expr, &mut remaining).map_err(|reason| {
                    format!("global runtime delivery copy budget exhausted: {reason:?}")
                })?;
                builder.fuel = usize::try_from(remaining).unwrap_or(usize::MAX);
                let mut expr = expr.clone();
                builder.normalize_expression(&mut expr)?;
                builder.expr(&expr, 0, false)?;
                builder.global_runtime_deliveries.insert(id, expr);
            }
        }
        drain_queue(&mut builder)?;

        let mut publication_fuel = builder.fuel;
        let functions = builder.instances.take().unwrap_or_default();
        let templates = captured_templates::storage(
            &functions,
            &builder.frozen_methods,
            builder.world.scope().declaration_source().package(),
            &builder.output,
            &mut builder.type_cores,
            &mut publication_fuel,
        )?;
        // Clone dispatch before pruning non-demanded methods so the retained
        // Builder dispatch table keeps all slot indices and specialized methods
        // available for subsequent incremental extension rounds.
        let mut dispatch = builder.output.clone();
        publication::retain_demanded_methods(
            &functions,
            &templates,
            &builder.callable_identities,
            &mut dispatch,
            &mut publication_fuel,
        )?;
        publication_fuel = publication_fuel
            .checked_sub(roots.len())
            .ok_or("IIR root copy budget exhausted")?;
        let normal_forms =
            publication::copy_normal_forms(&package.normal_forms, &mut publication_fuel)?;
        let source_world = builder.world.scope();
        let source_pkg = source_world
            .published_snapshot()
            .map_or(package, |snapshot| snapshot.package().as_ref());
        let source_volatile = &source_pkg.volatile_storage;
        publication_fuel = publication_fuel
            .checked_sub(source_volatile.entry_count())
            .ok_or("IIR volatile storage copy budget exhausted")?;
        let source_match_plans = &source_pkg.match_plans;
        publication_fuel = publication_fuel
            .checked_sub(source_match_plans.entry_count())
            .ok_or("IIR match plans copy budget exhausted")?;
        let output = iir::Package {
            target: package.target,
            expansion_sources: builder.quote_sources.clone(),
            effect_contracts: builder.effect_contracts.clone(),
            roots: roots.to_vec(),
            functions: functions.clone(),
            templates,
            dispatch,
            runtime_bindings: builder.runtime_bindings.clone(),
            global_runtime_deliveries: builder.global_runtime_deliveries.clone(),
            volatile_storage: source_volatile.clone(),
            match_plans: source_match_plans.clone(),
            nominals: builder.nominals.clone(),
            interfaces: builder.interface_contracts.clone(),
            normal_forms,
            callable_identities: builder.callable_identities.clone(),
            statics: package.statics.clone(),
        };
        validate_materialized_bit_convert_safety(&output, source_pkg, &mut publication_fuel)?;
        output.validate(&mut publication_fuel)?;
        self.incremental = Some(IncrementalIir {
            roots: roots.to_vec(),
            type_roots: types.to_vec(),
            quote_cursor: builder.quote_cursor,
            inline_effects: builder.inline_effects,
            quote_sources: builder.quote_sources,
            world: builder.world,
            value_scope: builder.value_scope,
            functions: builder.functions,
            staged_functions: builder.staged_functions,
            generated_functions: builder.generated_functions,
            recursive_factories: builder.recursive_factories,
            invocation_snapshot: builder.invocation_snapshot,
            types: builder.types,
            staging_views: builder.staging_views,
            interfaces: builder.interfaces,
            associated_demands: builder.associated_demands,
            output: builder.output,
            instances: functions,
            runtime_bindings: builder.runtime_bindings,
            global_runtime_deliveries: builder.global_runtime_deliveries,
            nominals: builder.nominals,
            interface_contracts: builder.interface_contracts,
            effect_contracts: builder.effect_contracts,
            effect_instances: builder.effect_instances,
            frozen_methods: builder.frozen_methods,
            callable_identities: builder.callable_identities,
            type_cores: builder.type_cores,
        });
        budget.steps = publication_fuel;
        Ok(output)
    }
}

impl std::fmt::Display for InstantiationSession<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(instantiation-session (phase {}) (source {}))",
            if self.phase_checked {
                "checked"
            } else {
                "pending"
            },
            self.source.package()
        )
    }
}

enum Roots<'a> {
    All,
    AllWithMethods(&'a [comptime_interpretor::TraitMethodIdentity]),
    Annotations(&'a mut n::Function),
    Dispatch(&'a [iir::InstanceKey]),
    Invocation(&'a [iir::InstanceKey]),
    Iir(&'a [iir::InstanceKey]),
    Method(&'a n::Expr, &'a mut Option<n::MethodInstance>),
    QuoteMethod(&'a n::Expr, &'a mut Option<n::MethodInstance>),
}

// The immutable type arena belongs to the entire materialization, including
// the storage pass that runs after dependency discovery. Analysis-only callers
// discard this private result; no active inference state is retained here.
struct BuildResult {
    published_source: Option<DeclarationSnapshot>,
    dispatch: Vec<n::DispatchInstance>,
    functions: Vec<iir::FunctionInstance>,
    nominals: Vec<iir::NominalInstance>,
    interfaces: Vec<iir::InterfaceContract>,
    remaining: usize,
    expansion_sources: Vec<n::ExpansionSource>,
    effect_contracts: Vec<iir::effects::Contract>,
    frozen_methods: Vec<n::Expr>,
    callable_identities: Vec<iir::CallableIdentity>,
    global_runtime_deliveries: BTreeMap<nir::RuntimeId, n::Expr>,
    type_cores: nir::TypeInterner,
}

struct DispatchContext<'a> {
    types: &'a Context,
    assumptions: &'a [n::TraitRequirement],
    value_parameters: &'a [(Id, Term)],
}

fn build(
    source: DeclarationSource<'_>,
    budget: Budget,
    roots: Roots<'_>,
    runtime_bindings: BTreeSet<nir::RuntimeId>,
    type_roots: &[nir::Term],
    context: Option<DispatchContext<'_>>,
    expansion: Option<&dyn Expansion>,
) -> Result<BuildResult, String> {
    let package = source.package();
    let body_free_selection = matches!(roots, Roots::QuoteMethod(..));
    // NIR interpretation retains all unconditional concrete implementations.
    // Their parent/capability worlds must remain closed even when only a few
    // function bodies are reachable. IIR instead owns its reachable world.
    let retain_concrete_world = matches!(
        roots,
        Roots::All | Roots::AllWithMethods(_) | Roots::Dispatch(_) | Roots::Invocation(_)
    );
    let mut construction_budget = budget;
    let world = source.world(
        &mut construction_budget,
        context.as_ref().map(|c| (c.types, c.assumptions)),
    )?;
    let mut input_fuel = construction_budget.steps;
    let mut runtime_bindings = runtime_bindings;
    let before_runtime_bindings = runtime_bindings.len();
    runtime_bindings.extend(package.runtime_domains.keys().copied());
    let added_runtime_bindings = runtime_bindings
        .len()
        .saturating_sub(before_runtime_bindings);
    if added_runtime_bindings > 0 {
        input_fuel = input_fuel
            .checked_sub(added_runtime_bindings)
            .ok_or("materialization runtime binding budget exhausted")?;
    }
    let value_scope = input_admission::ValueScope::new(
        context.as_ref().map_or(&[], |c| c.value_parameters),
        &mut input_fuel,
    )?;
    let inline_effects = if matches!(roots, Roots::Iir(_)) {
        input_fuel = input_fuel
            .checked_sub(
                package
                    .effects
                    .len()
                    .saturating_add(package.expansion_sources.len()),
            )
            .ok_or("materialized inline effect index budget exhausted")?;
        package
            .effects
            .iter()
            .filter(|effect| effect.inline && effect.abi == nir::effects::Abi::Direct)
            .map(|effect| effect.declaration)
            .collect()
    } else {
        BTreeSet::new()
    };
    let mut builder = Builder {
        expansion,
        inline_effects,
        active_staged_inlines: BTreeSet::new(),
        quote_cursor: package.quote_identity_cursor,
        quote_sources: if matches!(roots, Roots::Iir(_)) {
            package.expansion_sources.clone()
        } else {
            Vec::new()
        },
        retain_templates: matches!(
            roots,
            Roots::All | Roots::AllWithMethods(_) | Roots::Invocation(_)
        ),
        package,
        world,
        value_scope,
        queue: VecDeque::new(),
        functions: HashSet::new(),
        staged_functions: HashMap::new(),
        generated_functions: HashMap::new(),
        recursive_factories: HashMap::new(),
        invocation_snapshot: None,
        types: HashSet::new(),
        staging_views: HashMap::new(),
        interfaces: HashMap::new(),
        associated_demands: HashSet::new(),
        output: vec![],
        fuel: input_fuel,
        budget,
        instances: matches!(roots, Roots::Iir(_)).then(Vec::new),
        runtime_bindings,
        global_runtime_deliveries: BTreeMap::new(),
        nominals: vec![],
        interface_contracts: vec![],
        effect_contracts: vec![],
        effect_instances: HashSet::new(),
        frozen_methods: vec![],
        callable_identities: vec![],
        type_cores: nir::TypeInterner::default(),
        expression_owner: None,
        active_caller_env: None,
        active_closure_scopes: Vec::new(),
    };
    for ty in type_roots {
        let ty = builder.normalized_type(ty)?;
        builder.queue.push_back(Task::Type(ty));
    }
    if let Roots::Annotations(function) = roots {
        function.signature = builder.staging_type_view(&function.signature)?;
        for env in &mut function.interface.env_parameters {
            env.ty = builder.staging_type_view(&env.ty)?;
        }
        for expression in std::iter::once(&mut function.body)
            .chain(
                function
                    .interface
                    .defaults
                    .iter_mut()
                    .map(|(_, value)| value),
            )
            .chain(&mut function.interface.preconditions)
            .chain(&mut function.interface.postconditions)
        {
            expression.rewrite_with(&mut StagingAnnotationRewriter(|ty: &Term| {
                builder.staging_type_view(ty)
            }))?;
        }
    } else if let Roots::Dispatch(roots) | Roots::Invocation(roots) | Roots::Iir(roots) = roots {
        for root in roots {
            builder.tick(0)?;
            for argument in root.arguments.iter() {
                crate::associated::charge_projection_copy(argument, &mut builder.fuel)
                    .map_err(|_| "dispatch root argument type budget exhausted".to_owned())?;
            }
            builder
                .queue
                .push_back(Task::Function(root.declaration, root.arguments.to_vec()));
        }
    } else if let Roots::Method(callee, selected) | Roots::QuoteMethod(callee, selected) = roots {
        let K::TraitMethod {
            interface,
            interface_arguments,
            subject,
            member,
            method_arguments,
        } = &callee.kind
        else {
            return Err("method selection requires a trait method expression".into());
        };
        let slot = builder.interface(
            n::InterfaceInstance {
                declaration: *interface,
                arguments: interface_arguments.clone(),
            },
            subject.clone(),
        )?;
        let instance = builder.output[slot].interface_instance();
        let subject = builder.output[slot].subject.clone();
        let method =
            builder.specialize_method(slot, &instance, &subject, *member, method_arguments)?;
        if !body_free_selection {
            builder
                .queue
                .push_back(Task::Function(method.body, method.arguments.clone()));
        }
        *selected = Some(method);
    } else {
        if let Roots::AllWithMethods(methods) = roots {
            for method in methods {
                builder.tick(0)?;
                for term in std::iter::once(&method.subject)
                    .chain(&method.interface.arguments)
                    .chain(&method.method_arguments)
                {
                    crate::associated::charge_projection_copy(term, &mut builder.fuel)?;
                }
                let slot = builder.interface(method.interface.clone(), method.subject.clone())?;
                let interface = builder.output[slot].interface_instance();
                let subject = builder.output[slot].subject.clone();
                let selected = builder.specialize_method(
                    slot,
                    &interface,
                    &subject,
                    method.member,
                    &method.method_arguments,
                )?;
                builder
                    .queue
                    .push_back(Task::Function(selected.body, selected.arguments));
            }
        }
        for function in &package.functions {
            if function.type_parameters.is_empty() && function.requirements.is_empty() {
                builder
                    .queue
                    .push_back(Task::Function(function.declaration, vec![]));
            }
        }
    }
    if retain_concrete_world {
        for implementation in &package.implementations {
            builder.tick(0)?;
            if let Some(interface) = implementation.interface {
                if implementation.type_parameters.is_empty()
                    && implementation.requirements.is_empty()
                {
                    for term in std::iter::once(&implementation.subject)
                        .chain(&implementation.interface_arguments)
                    {
                        crate::associated::charge_projection_copy(term, &mut builder.fuel)?;
                    }
                    builder.queue.push_back(Task::Interface(
                        n::InterfaceInstance {
                            declaration: interface,
                            arguments: implementation.interface_arguments.clone(),
                        },
                        implementation.subject.clone(),
                    ));
                }
            }
        }
    }
    if builder.instances.is_some() {
        for (&id, expr) in &package.global_runtime_deliveries {
            builder.tick(0)?;
            let mut remaining = u64::try_from(builder.fuel).unwrap_or(u64::MAX);
            nir::copy_budget::charge_expression(expr, &mut remaining).map_err(|reason| {
                format!("global runtime delivery copy budget exhausted: {reason:?}")
            })?;
            builder.fuel = usize::try_from(remaining).unwrap_or(usize::MAX);
            let mut expr = expr.clone();
            builder.normalize_expression(&mut expr)?;
            builder.expr(&expr, 0, false)?;
            builder.global_runtime_deliveries.insert(id, expr);
        }
    }
    drain_queue(&mut builder)?;
    Ok(BuildResult {
        published_source: builder.world.scope().published_snapshot().cloned(),
        dispatch: builder.output,
        functions: builder.instances.unwrap_or_default(),
        nominals: builder.nominals,
        interfaces: builder.interface_contracts,
        remaining: builder.fuel,
        expansion_sources: builder.quote_sources,
        effect_contracts: builder.effect_contracts,
        frozen_methods: builder.frozen_methods,
        callable_identities: builder.callable_identities,
        global_runtime_deliveries: builder.global_runtime_deliveries,
        type_cores: builder.type_cores,
    })
}

fn drain_queue(builder: &mut Builder<'_, '_>) -> Result<(), String> {
    while let Some(task) = builder.queue.pop_front() {
        builder.tick(0)?;
        match task {
            Task::Function(id, arguments) => builder.function(id, arguments)?,
            Task::Interface(interface, subject) => {
                // A premise can discharge the type obligation, but it cannot
                // grant scope or representation evidence to its value inputs.
                let capability = builder.normalized_capability(&n::TraitRequirement {
                    interface: interface.declaration,
                    interface_arguments: interface.arguments,
                    subject,
                })?;
                // A given obligation has no executable dispatch identity.
                // Method/value demands still call interface() directly.
                let world = builder.world.capabilities();
                let given = query_budget::run(&mut builder.fuel, builder.budget, |budget| {
                    world.given_metered(&capability, budget)
                })?
                .is_some();
                if !given {
                    builder.interface(capability.interface_instance(), capability.subject)?;
                }
            }
            Task::Type(ty) => builder.ty(ty)?,
        }
    }
    Ok(())
}

impl Builder<'_, '_> {
    fn tick(&mut self, depth: usize) -> Result<(), String> {
        if depth >= self.budget.max_depth.min(128) || self.fuel == 0 {
            return Err("dispatch materialization budget exhausted".into());
        }
        self.fuel -= 1;
        Ok(())
    }

    fn input_binders(&self, parameters: &[Id]) -> Vec<nir::DeclarationParameter> {
        self.world.input_binders(parameters)
    }

    fn function(&mut self, id: Id, arguments: Vec<Term>) -> Result<(), String> {
        // Dispatch-only traversal can encounter ever-growing generic recursion
        // even when argument types do not occur in the function signature.
        // Bound terms before hashing/cloning instance keys or substituting them.
        for argument in &arguments {
            crate::associated::charge_projection_copy(argument, &mut self.fuel)
                .map_err(|_| "dispatch instance argument type budget exhausted".to_owned())?;
        }
        if let Some(identity) = self.world.runtime_function_identity(id) {
            if self.instances.is_some() {
                return Err(
                    "runtime function identity has no executable definition for IIR".into(),
                );
            }
            let domains = self
                .world
                .input_domains(&identity.type_parameters, &mut self.fuel)?;
            let substitution = nir::DeclarationSubstitution::bounded(
                &identity.type_parameters,
                &domains,
                &arguments,
                &[],
                &mut self.fuel,
            )
            .map_err(|e| e.to_string())?;
            self.callable_formation(identity.into(), &substitution)?;
            let signature = substitution
                .instantiate_bounded(&identity.signature, &mut self.fuel)
                .map_err(|e| e.to_string())?;
            self.queue.push_back(Task::Type(signature));
            return Ok(());
        }
        if !self.functions.insert((id, arguments.clone())) {
            return Ok(());
        }
        if self.instances.is_none() {
            let function = self
                .world
                .function_declaration(id)
                .ok_or("missing dispatch function")?;
            return self.dispatch_function(function, &arguments);
        }
        let f = self
            .world
            .function_declaration(id)
            .or_else(|| self.generated_functions.get(&id))
            .ok_or("missing dispatch function")?;
        if f.phase == n::FunctionPhase::NormalForm {
            return Err(
                "normal-form heads are type identities, not executable IIR functions".into(),
            );
        }
        for argument in &arguments {
            iir::completed_type(argument, &self.runtime_bindings, &mut self.fuel)?;
            self.queue.push_back(Task::Type(argument.clone()));
        }
        // Already-staged functions only need a bounded copy; ordinary templates
        // own their complete expansion preflight inside declaration substitution.
        let mut function = if let Some(staged) = self.staged_functions.get(&(id, arguments.clone()))
        {
            nir::copy_budget::charge_function_substitution(staged, &[], &[], &mut self.fuel)
                .map_err(|reason| format!("dispatch template copy budget exhausted: {reason:?}"))?;
            staged.clone()
        } else {
            if f.phase == n::FunctionPhase::Runtime
                && matches!(f.signature.callable_signature(),Term::Function {parameters,..} if parameters.iter().any(|p|p.sort!=nir::Sort::Runtime))
            {
                return Err("staged runtime function requires a specialized direct call".into());
            }
            self.world
                .instantiate_function(f, &arguments, &mut self.fuel)?
        };
        if function.phase == n::FunctionPhase::Runtime {
            let implicit = nir::program::implicit_runtime_env_parameters(
                &self.package.runtime_domains,
                &self.package.global_runtime_deliveries,
                &arguments,
                &function.interface.env_parameters,
            )?;
            function.interface.env_parameters.extend(implicit);
        }
        if function.phase == n::FunctionPhase::Comptime
            && matches!(function.signature.callable_signature(), Term::Function { parameters, .. }
                if parameters.iter().any(|parameter| parameter.sort != nir::Sort::Runtime))
        {
            return Err(format!(
                "comptime function `{}` requires invocation input staging before IIR materialization",
                function.name
            ));
        }
        function.signature = self.normalized_type(&function.signature)?;
        iir::completed_type(&function.signature, &self.runtime_bindings, &mut self.fuel)?;
        self.queue.push_back(Task::Type(function.signature.clone()));
        for env in &mut function.env_parameters {
            env.ty = self.normalized_type(&env.ty)?;
            iir::completed_type(&env.ty, &self.runtime_bindings, &mut self.fuel)?;
            self.queue.push_back(Task::Type(env.ty.clone()));
        }
        for requirement in &mut function.requirements {
            *requirement = self.normalized_predicate(requirement, 0)?;
            self.tick(0)?;
            if !matches!(
                self.prove_requirement(requirement)?,
                crate::ptc::Outcome::Entailed(_)
            ) {
                return Err(format!(
                    "IIR function type requirement is not established: {requirement}"
                ));
            }
        }
        for capability in &mut function.capabilities {
            *capability = self.normalized_capability(capability)?;
        }
        for capability in &function.capabilities {
            self.queue.push_back(Task::Interface(
                capability.interface_instance(),
                capability.subject.clone(),
            ));
        }
        iir::staging::forward_closed_locals_with_layouts(
            self.world.scope().declaration_source().package(),
            &mut function,
            &mut self.fuel,
        )?;
        let saved_caller_env = self.active_caller_env.take();
        let saved_closure_scopes = std::mem::take(&mut self.active_closure_scopes);
        self.active_caller_env = Some((function.interface.env_parameters.clone(), BTreeMap::new()));
        for expression in function.runtime_deliveries.values_mut() {
            self.normalize_owned_expression(id.package, expression)?;
        }
        self.active_caller_env = Some((
            function.interface.env_parameters.clone(),
            function.runtime_deliveries.clone(),
        ));
        self.normalize_owned_expression(id.package, &mut function.body)?;
        for expression in function
            .interface
            .preconditions
            .iter_mut()
            .chain(&mut function.interface.postconditions)
            .chain(
                function
                    .interface
                    .defaults
                    .iter_mut()
                    .map(|(_, expression)| expression),
            )
        {
            self.normalize_owned_expression(id.package, expression)?;
        }
        if function.phase == n::FunctionPhase::Runtime {
            let caller_envs = &function.interface.env_parameters;
            let caller_deliveries = &function.runtime_deliveries;
            self.deliver_missing_call_env_arguments(
                &mut function.body,
                caller_envs,
                caller_deliveries,
                None,
            )?;
            for expression in function
                .interface
                .preconditions
                .iter_mut()
                .chain(&mut function.interface.postconditions)
                .chain(
                    function
                        .interface
                        .defaults
                        .iter_mut()
                        .map(|(_, expression)| expression),
                )
            {
                self.deliver_missing_call_env_arguments(
                    expression,
                    caller_envs,
                    caller_deliveries,
                    None,
                )?;
            }
            let deliveries_snapshot = caller_deliveries.clone();
            for expression in function.runtime_deliveries.values_mut() {
                self.deliver_missing_call_env_arguments(
                    expression,
                    caller_envs,
                    &deliveries_snapshot,
                    None,
                )?;
            }
        }
        self.active_closure_scopes = saved_closure_scopes;
        self.active_caller_env = saved_caller_env;
        function.type_parameters.clear();
        // Interface and body predicates may substitute the same staged
        // input expression. Each predicate executes at a distinct boundary,
        // even when those copies retain the same original source range.
        for predicate in function
            .interface
            .preconditions
            .iter_mut()
            .chain(&mut function.interface.postconditions)
        {
            nir::error_results::specialize::anchor_expression_copy(
                predicate,
                function.body.source,
                &mut self.quote_sources,
                &mut self.fuel,
            )?;
        }
        for expression in std::iter::once(&mut function.body)
            .chain(&mut function.interface.preconditions)
            .chain(&mut function.interface.postconditions)
            .chain(
                function
                    .interface
                    .defaults
                    .iter_mut()
                    .map(|(_, expression)| expression),
            )
        {
            nir::error_results::specialize::anchor_completed_projections(
                expression,
                &mut self.quote_sources,
                &mut self.fuel,
            )?;
        }
        let comptime = function.phase == n::FunctionPhase::Comptime;
        self.expr(&function.body, 0, comptime)?;
        for expression in function
            .interface
            .preconditions
            .iter()
            .chain(&function.interface.postconditions)
            .chain(function.interface.defaults.iter().map(|(_, e)| e))
        {
            self.expr(expression, 0, comptime)?;
        }
        for expression in function.runtime_deliveries.values() {
            self.expr(expression, 0, false)?;
        }
        self.instances
            .as_mut()
            .expect("IIR function collection")
            .push(iir::FunctionInstance::new(
                iir::InstanceKey {
                    declaration: id,
                    arguments: arguments.into(),
                },
                function,
            ));
        Ok(())
    }

    fn interface(
        &mut self,
        mut interface: n::InterfaceInstance,
        mut subject: Term,
    ) -> Result<usize, String> {
        let selected_subject = self.normalized_type(&subject)?;
        if selected_subject != subject {
            self.queue.push_back(Task::Type(subject));
            subject = selected_subject;
        }
        let arguments = self.normalized_interface_arguments(
            interface.declaration,
            &subject,
            &interface.arguments,
        )?;
        for (argument, selected) in interface.arguments.iter_mut().zip(arguments) {
            if selected != *argument {
                self.queue.push_back(Task::Type(argument.clone()));
                *argument = selected;
            }
        }
        if let Some(index) = self.interfaces.get(&(interface.clone(), subject.clone())) {
            return Ok(*index);
        }
        let (id, implementation_arguments) =
            query_budget::run(&mut self.fuel, self.budget, |budget| {
                self.world.select_interface(&interface, &subject, budget)
            })?;
        let implementation = self
            .package
            .implementations
            .iter()
            .find(|i| i.declaration == id)
            .ok_or("missing selected implementation")?;
        let implementation_substitution = nir::DeclarationSubstitution::bounded(
            &implementation.type_parameters,
            &implementation.parameter_types,
            &implementation_arguments,
            &[],
            &mut self.fuel,
        )
        .map_err(|e| e.to_string())?;
        for capability in &implementation.capabilities {
            let capability = implementation_substitution
                .instantiate_requirement_bounded(capability, &mut self.fuel)
                .map_err(|e| e.to_string())?;
            self.queue.push_back(Task::Interface(
                capability.interface_instance(),
                capability.subject,
            ));
        }
        let contract = self
            .package
            .traits
            .iter()
            .find(|t| t.declaration == interface.declaration)
            .ok_or("missing dispatch interface")?;
        let parameters = contract.parameters_with_self();
        let parameter_domains = self.world.input_domains(&parameters, &mut self.fuel)?;
        let inputs: Vec<_> = std::iter::once(subject.clone())
            .chain(interface.arguments.iter().cloned())
            .collect();
        let substitution = nir::DeclarationSubstitution::bounded(
            &parameters,
            &parameter_domains,
            &inputs,
            &[],
            &mut self.fuel,
        )
        .map_err(|e| e.to_string())?;
        for requirement in &contract.requirements {
            let requirement = substitution
                .instantiate_predicate_bounded(requirement, &mut self.fuel)
                .map_err(|e| e.to_string())?;
            if !matches!(
                self.prove_requirement(&requirement)?,
                crate::ptc::Outcome::Entailed(_)
            ) {
                return Err(
                    "interface parameter requirement is not established during materialization"
                        .into(),
                );
            }
        }
        for capability in &contract.capabilities {
            let n::TypePredicate::Trait(capability) = substitution
                .instantiate_predicate_bounded(
                    &n::TypePredicate::Trait(capability.clone()),
                    &mut self.fuel,
                )
                .map_err(|e| e.to_string())?
            else {
                unreachable!()
            };
            self.queue.push_back(Task::Interface(
                capability.interface_instance(),
                capability.subject,
            ));
        }
        // Selecting an interface establishes its own contract. Method-local
        // signatures and conditions are dependencies only when that member is
        // demanded; another member may be usable without those conditions.
        if self.instances.is_some() {
            // Unconditional interface signatures remain part of the exported
            // contract even when their bodies are unused. Conditional members
            // are added by specialize_method only when actually demanded.
            let mut methods = Vec::new();
            for template in &contract.methods {
                // Runtime interface retention must not instantiate a different
                // trait phase's signatures or its associated values. A type
                // can implement this phase without a comptime contribution.
                if template.phase != n::TraitPhase::Runtime
                    || template.signature.is_comptime_callable()
                    || !template.type_parameters.is_empty()
                    || !template.requirements.is_empty()
                    || !template.capabilities.is_empty()
                {
                    continue;
                }
                let mut method = template.clone();
                method.default = None;
                method.signature = method
                    .signature
                    .try_map_types(|ty| substitution.instantiate_bounded(ty, &mut self.fuel))
                    .map_err(|e| e.to_string())?;
                let frames = match template.signature.callable_signature() {
                    Ok(Term::Function { parameters, .. }) => {
                        vec![parameters.iter().map(|p| p.sort).collect()]
                    }
                    _ => vec![],
                };
                method.program = method
                    .program
                    .instantiate_declaration_parameters(
                        &parameters,
                        &parameter_domains,
                        &inputs,
                        &frames,
                        &mut self.fuel,
                    )
                    .map_err(|e| e.to_string())?;
                self.normalize_method_contract(&mut method)?;
                self.queue.push_back(Task::Type(
                    method
                        .signature
                        .ordinary()
                        .map_err(|e| e.to_string())?
                        .clone(),
                ));
                for expression in method
                    .program
                    .preconditions
                    .iter()
                    .chain(&method.program.postconditions)
                {
                    self.expr(expression, 0, method.signature.is_comptime_callable())?;
                }
                methods.push(method);
            }
            self.interface_contracts.push(iir::InterfaceContract {
                interface: interface.clone(),
                subject: subject.clone(),
                methods,
            });
            for argument in &interface.arguments {
                self.queue.push_back(Task::Type(argument.clone()));
            }
            self.queue.push_back(Task::Type(subject.clone()));
        }
        let slot = self.output.len();
        self.output.push(n::DispatchInstance {
            associated_initializers: vec![],
            interface: interface.declaration,
            interface_arguments: interface.arguments.clone(),
            subject: subject.clone(),
            implementation: id,
            arguments: implementation_arguments,
            associated: vec![],
            methods: vec![],
        });
        self.interfaces
            .insert((interface.clone(), subject.clone()), slot);
        for parent in &contract.parents {
            let arguments = parent
                .arguments
                .iter()
                .map(|ty| substitution.instantiate_bounded(ty, &mut self.fuel))
                .collect::<Result<_, _>>()
                .map_err(|e| e.to_string())?;
            self.queue.push_back(Task::Interface(
                n::InterfaceInstance {
                    declaration: parent.declaration,
                    arguments,
                },
                subject.clone(),
            ));
        }
        Ok(slot)
    }

    fn specialize_method(
        &mut self,
        slot: usize,
        interface: &n::InterfaceInstance,
        subject: &Term,
        member: Id,
        method_arguments: &[Term],
    ) -> Result<n::MethodInstance, String> {
        if let Some(method) = self.output[slot]
            .methods
            .iter()
            .find(|m| m.member == member && m.method_arguments == method_arguments)
        {
            return Ok(method.clone());
        }
        let (method, mut specialized) = self.world.prepare_method(
            &self.output[slot],
            member,
            method_arguments,
            self.instances.is_some(),
            &mut self.fuel,
        )?;
        if self.instances.is_some() {
            self.normalize_method_contract(&mut specialized)?;
        }
        // prepare_method has already instantiated these predicates in the
        // member signature's frame. Do not recheck them in an empty frame.
        for requirement in &specialized.requirements {
            self.formation_requirement(requirement)?;
        }
        for capability in &specialized.capabilities {
            self.queue.push_back(Task::Interface(
                capability.interface_instance(),
                capability.subject.clone(),
            ));
        }
        self.output[slot].methods.push(method.clone());
        let contract_signature = specialized
            .signature
            .ordinary()
            .map_err(|e| e.to_string())?
            .clone();
        self.queue.push_back(Task::Type(contract_signature.clone()));
        if self.instances.is_none() {
            if let Some(body_signature) = self
                .world
                .callable_signature(method.body, &method.arguments, &[], &mut self.fuel)
                .map_err(|e| e.to_string())?
            {
                if body_signature != contract_signature {
                    self.queue.push_back(Task::Type(body_signature));
                }
            }
        }
        if self.instances.is_some() {
            for argument in method_arguments {
                self.queue.push_back(Task::Type(argument.clone()));
            }
            let retained = self
                .interface_contracts
                .iter_mut()
                .find(|c| c.interface == *interface && c.subject == *subject)
                .ok_or("missing retained interface contract")?;
            if !retained
                .methods
                .iter()
                .any(|m| m.declaration == member && m.type_arguments == method_arguments)
            {
                retained.methods.push(specialized.clone());
            }
        }
        for expression in specialized
            .program
            .preconditions
            .iter()
            .chain(&specialized.program.postconditions)
        {
            self.expr(expression, 0, specialized.signature.is_comptime_callable())?;
        }
        Ok(method)
    }

    fn predicate_dependencies(
        &mut self,
        predicate: &n::TypePredicate,
        depth: usize,
    ) -> Result<(), String> {
        self.tick(depth)?;
        match predicate {
            n::TypePredicate::Trait(requirement) => {
                // A logical atom is not an unconditional executable capability.
                for ty in
                    std::iter::once(&requirement.subject).chain(&requirement.interface_arguments)
                {
                    crate::associated::charge_projection_copy(ty, &mut self.fuel)?;
                    self.queue.push_back(Task::Type(ty.clone()));
                }
            }
            n::TypePredicate::Equal(a, b) => {
                for ty in [a, b] {
                    crate::associated::charge_projection_copy(ty, &mut self.fuel)?;
                    self.queue.push_back(Task::Type(ty.clone()));
                }
            }
            n::TypePredicate::Kind(ty, _) | n::TypePredicate::NotSized(ty) => {
                crate::associated::charge_projection_copy(ty, &mut self.fuel)?;
                self.queue.push_back(Task::Type(ty.clone()));
            }
            n::TypePredicate::Not(inner) => self.predicate_dependencies(inner, depth + 1)?,
            n::TypePredicate::And(items) | n::TypePredicate::Or(items) => {
                for item in items {
                    self.predicate_dependencies(item, depth + 1)?;
                }
            }
            n::TypePredicate::Bool(_) | n::TypePredicate::IntLe(..) => {}
        }
        Ok(())
    }

    fn prove_requirement(
        &mut self,
        requirement: &n::TypePredicate,
    ) -> Result<crate::ptc::Outcome, nir::NominalAdmissionError> {
        let world = self.world.capabilities();
        query_budget::run(&mut self.fuel, self.budget, |budget| {
            world.check_predicate_metered(requirement, budget)
        })
    }

    fn callable_formation(
        &mut self,
        formation: nir::callable_source::Formation<'_>,
        substitution: &nir::DeclarationSubstitution<'_>,
    ) -> Result<(), String> {
        self.formation_requirements(formation.requirements, substitution)?;
        for capability in formation.capabilities {
            let bound = substitution
                .instantiate_requirement_bounded(capability, &mut self.fuel)
                .map_err(|error| error.to_string())?;
            self.queue
                .push_back(Task::Interface(bound.interface_instance(), bound.subject));
        }
        Ok(())
    }

    fn formation_requirements(
        &mut self,
        requirements: &[n::TypePredicate],
        substitution: &nir::DeclarationSubstitution<'_>,
    ) -> Result<(), String> {
        for requirement in requirements {
            let requirement = substitution
                .instantiate_predicate_bounded(requirement, &mut self.fuel)
                .map_err(|e| e.to_string())?;
            self.formation_requirement(&requirement)?;
        }
        Ok(())
    }

    fn formation_requirement(&mut self, requirement: &n::TypePredicate) -> Result<(), String> {
        self.tick(0)?;
        if self.instances.is_none() {
            return self.predicate_dependencies(requirement, 0);
        }
        let requirement = self.normalized_predicate(requirement, 0)?;
        if !matches!(
            self.prove_requirement(&requirement)?,
            crate::ptc::Outcome::Entailed(_)
        ) {
            return Err(format!(
                "IIR type formation requirement is not established: {requirement}"
            ));
        }
        Ok(())
    }

    fn normalize_method_contract(&mut self, method: &mut n::MethodContract) -> Result<(), String> {
        method.signature = self
            .normalized_type(method.signature.ordinary().map_err(|e| e.to_string())?)?
            .into();
        for argument in &mut method.type_arguments {
            *argument = self.normalized_interface_input(argument)?;
        }
        for requirement in &mut method.requirements {
            *requirement = self.normalized_predicate(requirement, 0)?;
        }
        for capability in &mut method.capabilities {
            *capability = self.normalized_capability(capability)?;
        }
        for expression in method
            .program
            .preconditions
            .iter_mut()
            .chain(&mut method.program.postconditions)
        {
            self.normalize_owned_expression(method.declaration.package, expression)?;
        }
        Ok(())
    }

    fn normalize_owned_expression(
        &mut self,
        owner: u64,
        expression: &mut n::Expr,
    ) -> Result<(), String> {
        let previous = self.expression_owner.replace(owner);
        let result = self.normalize_expression(expression);
        self.expression_owner = previous;
        result
    }

    fn normalize_expression(&mut self, expression: &mut n::Expr) -> Result<(), String> {
        // Private working copies are discarded on any failure. Selected values
        // are substituted before dependency discovery, including nested patterns.
        if self.instances.is_some() {
            dead_tails::prune(expression, &mut self.fuel)?;
            // Expose runtime template factories before collecting local target
            // choices. Their argument frames and effects remain runtime code.
            self.walk_pruned_tracking_closures(expression, &mut |this, expr| {
                let factory = matches!(&expr.kind, K::Call { callee, .. }
                    if matches!(callee.ty.callable_signature(), Term::Function { result, .. }
                        if matches!(result.callable_signature(), Term::Function { parameters, .. }
                            if parameters.iter().any(|p| p.sort != nir::Sort::Runtime))));
                if factory {
                    this.stage_direct_call(expr)
                } else {
                    Ok(true)
                }
            })?;
        }
        let mut fuel = self.fuel;
        let result = iir::visit::expression_mut(expression, &mut fuel, |expr, fuel| {
            self.fuel = *fuel;
            Self::resolve_immediate_callable(
                expr,
                !self.active_staged_inlines.is_empty(),
                &mut self.fuel,
            )?;
            Self::resolve_conditional_callable(
                expr,
                !self.active_staged_inlines.is_empty(),
                &mut self.quote_cursor,
                &mut self.quote_sources,
                &mut self.fuel,
            )?;
            if matches!(expr.kind, K::ComptimeValue(_)) {
                *expr = self.invoke_staged_input(expr)?;
            }
            if let K::Call {
                callee, arguments, ..
            } = &mut expr.kind
            {
                if let Term::Function { parameters, .. } = callee.ty.callable_signature() {
                    for argument in arguments {
                        let parameter = parameters
                            .get(argument.parameter)
                            .ok_or("staged argument parameter index is invalid")?;
                        if parameter.sort == nir::Sort::Runtime || argument.is_default {
                            continue;
                        }
                        let mut captures = false;
                        iir::visit::expression(&argument.value, &mut self.fuel, |node, _| {
                            captures |= matches!(node.kind, K::AssociatedValue { .. })
                                && node.ty.is_comptime_callable()
                                && matches!(&node.ty, Term::Closure { environment, .. } if !environment.is_empty());
                            Ok(())
                        })?;
                        if captures
                            && nir::free_bindings::collect(&argument.value, &mut self.fuel)?
                                .is_empty()
                        {
                            // The required input is the freezing boundary, not an
                            // intermediate callback. Its local bindings and mutable
                            // capture graph must share one interpreter invocation.
                            argument.value = self.invoke_staged_input(&argument.value)?;
                        }
                    }
                }
            }
            let captured_associated_call = if let K::Call { callee, .. } = &expr.kind {
                let mut target = callee.as_ref();
                while let K::Block {
                    statements,
                    result: Some(result),
                } = &target.kind
                {
                    if !statements.is_empty() {
                        break;
                    }
                    self.tick(0)?;
                    target = result;
                }
                callee.ty.is_comptime_callable()
                    && matches!(&callee.ty, Term::Closure { environment, .. } if !environment.is_empty())
                    && matches!(target.kind, K::AssociatedValue { .. })
            } else {
                false
            };
            if captured_associated_call
                && nir::free_bindings::collect(expr, &mut self.fuel)?.is_empty()
            {
                // Keep the initialized callable and its capture graph inside
                // one invocation. Only its result crosses the freezing boundary.
                *expr = self.invoke_staged_input(expr)?;
            }
            if let K::AssociatedValue {
                interface,
                member,
                subject,
            } = &expr.kind
            {
                let interface = n::InterfaceInstance {
                    declaration: interface.declaration,
                    arguments: self.normalized_interface_arguments(
                        interface.declaration,
                        subject,
                        &interface.arguments,
                    )?,
                };
                let subject = self.normalized_type(subject)?;
                let expected = self.normalized_type(&expr.ty)?;
                let selected = self.associated_value_plan(&interface, *member, &subject)?;
                if selected.supply.expression().ty != expected {
                    return Err("associated expression type disagrees with selected member".into());
                }
                let source = expr.source;
                *expr = match selected.supply {
                    nir::associated_initializers::AssociatedSelection::Initializer(template) => {
                        self.invoke_staged_input(template.expression())?
                    }
                    nir::associated_initializers::AssociatedSelection::Completed(value) => {
                        value.into_expression()
                    }
                };
                expr.source = source;
                // Selection can expose a completed captured input after the
                // visitor has already handled the original node's kind.
                if matches!(expr.kind, K::ComptimeValue(_)) {
                    *expr = self.invoke_staged_input(expr)?;
                }
            }
            *fuel = self.fuel;
            Ok(())
        });
        self.fuel = fuel.min(self.fuel);
        result?;
        expression.rewrite_with(&mut StagingAnnotationRewriter(|ty: &Term| {
            self.staging_type_view(ty)
        }))?;
        nir::float_formation::complete(expression, &mut self.fuel)?;
        nir::integer_formation::complete(expression, self.package.target, &mut self.fuel)?;
        if self.instances.is_some() {
            // Associated initializers can expose runtime template identities.
            // Collect capture domains only after those selections are visible.
            captured_templates::forward(expression, &mut self.quote_cursor, &mut self.fuel)?;
            mutable_callbacks::lower(
                expression,
                &mut self.quote_cursor,
                &mut self.quote_sources,
                &mut self.fuel,
            )?;
            staged::forward_template_aliases(
                expression,
                &mut self.quote_cursor,
                &mut self.quote_sources,
                &mut self.fuel,
            )?;
            template_factories::discard_selections(expression, &mut self.fuel)?;
            let mut generated = Vec::new();
            let empty_deliveries = BTreeMap::new();
            let (caller_envs, caller_deliveries) = match &self.active_caller_env {
                Some((envs, deliveries)) => (envs.as_slice(), deliveries),
                None => (&[][..], &empty_deliveries),
            };
            self.expansion
                .ok_or("IIR materialization lacks an expansion service")?
                .quotes(
                    self.world.scope().declaration_source().package(),
                    expression,
                    caller_envs,
                    caller_deliveries,
                    &mut self.quote_cursor,
                    &mut self.quote_sources,
                    &mut generated,
                    &mut self.fuel,
                )?;
            self.register_quote_layouts(generated)?;
        }
        if self.instances.is_some() {
            self.walk_pruned_tracking_closures(expression, &mut |this, expr| {
                if let Some(plan) = iir::staging::preparation_with_layouts(
                    this.world.scope().declaration_source().package(),
                    expr,
                    &mut this.fuel,
                )? {
                    let completed = this.invoke_staged_input(&plan.expression)?;
                    plan.apply_with_layouts(
                        this.world.scope().declaration_source().package(),
                        expr,
                        completed,
                        &mut this.fuel,
                    )?;
                }
                let mut remaining = this.fuel;
                let completed = conditional_calls::complete_inputs(
                    expr,
                    this,
                    &mut remaining,
                    |this, input, fuel| {
                        this.fuel = *fuel;
                        let result = this.invoke_staged_input(input);
                        *fuel = this.fuel;
                        result
                    },
                );
                this.fuel = remaining.min(this.fuel);
                completed?;
                Self::resolve_conditional_callable(
                    expr,
                    !this.active_staged_inlines.is_empty(),
                    &mut this.quote_cursor,
                    &mut this.quote_sources,
                    &mut this.fuel,
                )?;
                this.stage_direct_call(expr)
            })?;
        }
        expression.rewrite_types(&mut |ty| {
            if ty.sort() == nir::Sort::Type {
                self.normalized_type(ty)
            } else {
                // Staging adds canonical value inputs to instance identities.
                // They are keys, not type annotations to normalize again.
                crate::associated::charge_projection_copy(ty, &mut self.fuel)?;
                Ok(ty.clone())
            }
        })?;
        if self.instances.is_some() {
            // Staged quote calls only became lexical bodies above. Their
            // installations could not be eliminated by the earlier NIR pass.
            let mut generated = Vec::new();
            self.expansion
                .ok_or("IIR materialization lacks an expansion service")?
                .effects(
                    self.world.scope().declaration_source().package(),
                    &self.inline_effects,
                    expression,
                    &mut self.quote_cursor,
                    &mut self.quote_sources,
                    &mut generated,
                    &mut self.fuel,
                )?;
            self.register_quote_layouts(generated)?;
            dead_tails::prune_type_preparations(expression, &mut self.fuel)?;
        }
        Ok(())
    }

    // Move formation of an immediately invoked function value into the callee
    // position. No intervening expression, capture, mutation or escaping use
    // may observe the binding. Formation still precedes all argument evaluation.
    fn resolve_immediate_callable(
        expression: &mut n::Expr,
        include_runtime: bool,
        fuel: &mut usize,
    ) -> Result<(), String> {
        let K::Block {
            statements,
            result: Some(result),
        } = &mut expression.kind
        else {
            return Ok(());
        };
        loop {
            *fuel = fuel
                .checked_sub(1)
                .ok_or("local callable resolution budget exhausted")?;
            let Some(statement) = statements.last() else {
                return Ok(());
            };
            let K::Let { binding, value } = &statement.kind else {
                return Ok(());
            };
            if !matches!(
                value.kind,
                K::Function(_)
                    | K::FunctionInstance { .. }
                    | K::TraitMethod { .. }
                    | K::Local(_)
                    | K::If { .. }
                    | K::Match { .. }
                    | K::Block { .. }
            ) {
                return Ok(());
            }
            if !matches!(value.ty.callable_signature(), Term::Function { parameters, .. }
            if include_runtime || parameters.iter().any(|p| p.sort != nir::Sort::Runtime))
            {
                return Ok(());
            }
            // Running an effect computation does not evaluate any intervening
            // operand before its callee. Preserve the row mapping while looking
            // through this immediate execution wrapper.
            let mut call = result.as_ref();
            while let K::EffectRun { computation, .. } = &call.kind {
                call = computation;
            }
            let K::Call {
                callee, arguments, ..
            } = &call.kind
            else {
                return Ok(());
            };
            if !matches!(callee.kind, K::Local(local) if local == *binding) || callee.ty != value.ty
            {
                return Ok(());
            }
            let mut escapes = false;
            for argument in arguments {
                // Include implicit captures and writes, but not unrelated
                // closures or bindings declared inside the argument itself.
                escapes |= nir::free_bindings::collect(&argument.value, fuel)?.contains(binding);
            }
            if escapes {
                return Ok(());
            }
            let statement = statements.pop().unwrap();
            let K::Let { value, .. } = statement.kind else {
                unreachable!()
            };
            let mut call = result.as_mut();
            let callee = loop {
                match &mut call.kind {
                    K::EffectRun { computation, .. } => call = computation,
                    K::Call { callee, .. } => break callee,
                    _ => unreachable!(),
                }
            };
            *callee = value;
        }
    }

    fn register_quote_layouts(&mut self, generated: Vec<n::Enum>) -> Result<(), String> {
        for definition in generated {
            if !definition.type_parameters.is_empty() {
                return Err(
                    "materialized quote return storage retained abstract Type parameters".into(),
                );
            }
            let ty = Term::Nominal {
                declaration: definition.declaration,
                arguments: vec![],
            };
            if !self.types.insert(ty.clone()) {
                return Err("materialized quote storage identity collision".into());
            }
            for variant in &definition.variants {
                for field in &variant.fields {
                    self.queue.push_back(Task::Type(field.clone()));
                }
            }
            self.nominals.push(iir::NominalInstance {
                ty,
                definition: iir::NominalDefinition::Enum(definition),
            });
        }
        Ok(())
    }

    fn returns_staged_callable(ty: &Term) -> bool {
        matches!(ty.callable_signature(), Term::Function { result, .. }
            if matches!(result.callable_signature(), Term::Function { parameters, .. }
                if parameters.iter().any(|p| p.sort != nir::Sort::Runtime)))
    }

    fn merge_entry_contract(
        function: &mut n::Function,
        requirements: Vec<n::TypePredicate>,
        capabilities: Vec<n::TraitRequirement>,
        program: nir::contracts::ProgramContract,
    ) {
        function.requirements.extend(requirements);
        function.capabilities.extend(capabilities);
        function
            .interface
            .preconditions
            .extend(program.preconditions);
        function
            .interface
            .postconditions
            .extend(program.postconditions);
        function.result_binding = function.result_binding.or(program.result_binding);
        for snapshot in program.entry_snapshots {
            if !function
                .entry_snapshots
                .iter()
                .any(|s| s.parameter == snapshot.parameter)
            {
                function.entry_snapshots.push(snapshot);
            }
        }
    }

    fn stage_direct_call(&mut self, expression: &mut n::Expr) -> Result<bool, String> {
        if self.lower_recursive_factory_call(expression)? {
            return Ok(true);
        }
        if !self.active_staged_inlines.is_empty() {
            if let K::Call { callee, .. } = &expression.kind {
                if matches!(callee.kind, K::TraitMethod { .. })
                    && !Self::returns_staged_callable(&callee.ty)
                    && matches!(callee.ty.callable_signature(), Term::Function { parameters, .. }
                        if parameters.iter().all(|p| p.sort == nir::Sort::Runtime))
                {
                    let selected = resolve_method_instance_in_source(
                        self.world.scope().declaration_source(),
                        callee,
                        &mut self.fuel,
                    )?;
                    let template = self
                        .package
                        .functions
                        .iter()
                        .find(|function| function.declaration == selected.body)
                        .ok_or("missing selected inline method body")?;
                    if !template.inline {
                        return Ok(true);
                    }
                    let mut bound = self.world.instantiate_function(
                        template,
                        &selected.arguments,
                        &mut self.fuel,
                    )?;
                    if bound.phase == n::FunctionPhase::Runtime {
                        let implicit = nir::program::implicit_runtime_env_parameters(
                            &self.package.runtime_domains,
                            &self.package.global_runtime_deliveries,
                            &selected.arguments,
                            &bound.interface.env_parameters,
                        )?;
                        bound.interface.env_parameters.extend(implicit);
                        for env in &mut bound.interface.env_parameters {
                            env.ty = self.normalized_type(&env.ty)?;
                        }
                    }
                    bound.type_parameters.clear();
                    // Keep the TraitMethod callee until expansion has merged its
                    // interface contract and rebound the argument/default frame.
                    return self
                        .expand_inline_instance(&bound, expression)
                        .map(|expanded| !expanded);
                }
            }
        }
        let mut entry_contract = None;
        // A concrete trait selection determines the body before ordinary
        // comptime inputs can be bound. Keep the interface contract, but route
        // this call through the same specialization path as a direct function.
        if let K::Call {
            callee,
            arguments,
            parameter_bindings,
            ..
        } = &mut expression.kind
        {
            if let K::TraitMethod {
                interface,
                interface_arguments,
                subject,
                member,
                method_arguments,
            } = &callee.kind
            {
                if Self::returns_staged_callable(&callee.ty)
                    || matches!(callee.ty.callable_signature(),Term::Function {parameters,..}
                        if parameters.iter().any(|p|p.sort!=nir::Sort::Runtime))
                {
                    let instance = n::InterfaceInstance {
                        declaration: *interface,
                        arguments: interface_arguments.clone(),
                    };
                    let slot = self.interface(instance, subject.clone())?;
                    let instance = self.output[slot].interface_instance();
                    let subject = self.output[slot].subject.clone();
                    let interface_contract = self
                        .package
                        .traits
                        .iter()
                        .find(|t| t.declaration == instance.declaration)
                        .ok_or("missing selected staged interface")?;
                    let contract = interface_contract
                        .methods
                        .iter()
                        .find(|m| m.declaration == *member)
                        .ok_or("missing selected staged method contract")?;
                    if contract.type_parameters.len() != method_arguments.len() {
                        return Err("dispatch method type argument count mismatch".into());
                    }
                    let implementation = self
                        .package
                        .implementations
                        .iter()
                        .find(|i| i.declaration == self.output[slot].implementation)
                        .ok_or("missing selected staged implementation")?;
                    let body = implementation
                        .methods
                        .iter()
                        .find(|(name, _)| name == &contract.name)
                        .ok_or("missing selected staged method body")?
                        .1;
                    let mut body_arguments = if contract.default == Some(body) {
                        std::iter::once(subject.clone())
                            .chain(instance.arguments.iter().cloned())
                            .collect::<Vec<_>>()
                    } else {
                        self.output[slot].arguments.clone()
                    };
                    body_arguments.extend(method_arguments.iter().cloned());
                    let method = n::MethodInstance {
                        member: *member,
                        body,
                        arguments: body_arguments,
                        method_arguments: method_arguments.clone(),
                    };
                    let template = self
                        .package
                        .functions
                        .iter()
                        .chain(&self.package.quote_templates)
                        .find(|f| f.declaration == body)
                        .ok_or("missing selected staged method body")?;
                    let parameters = interface_contract
                        .parameters_with_self()
                        .into_iter()
                        .chain(contract.type_parameters.iter().copied())
                        .collect::<Vec<_>>();
                    let domains = self.world.input_domains(&parameters, &mut self.fuel)?;
                    let inputs = std::iter::once(subject.clone())
                        .chain(instance.arguments.iter().cloned())
                        .chain(method_arguments.iter().cloned())
                        .collect::<Vec<_>>();
                    let frames = match contract.signature.callable_signature() {
                        Ok(Term::Function { parameters, .. }) => {
                            vec![parameters.iter().map(|p| p.sort).collect()]
                        }
                        _ => return Err("staged method needs a function signature".into()),
                    };
                    let substitution = nir::DeclarationSubstitution::bounded(
                        &parameters,
                        &domains,
                        &inputs,
                        &frames,
                        &mut self.fuel,
                    )
                    .map_err(|e| e.to_string())?;
                    let requirements = contract
                        .requirements
                        .iter()
                        .map(|r| substitution.instantiate_predicate_bounded(r, &mut self.fuel))
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|e| e.to_string())?;
                    let capabilities = contract
                        .capabilities
                        .iter()
                        .map(|r| substitution.instantiate_requirement_bounded(r, &mut self.fuel))
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|e| e.to_string())?;
                    let mut program = contract
                        .program
                        .instantiate_declaration_parameters(
                            &parameters,
                            &domains,
                            &inputs,
                            &frames,
                            &mut self.fuel,
                        )
                        .map_err(|e| e.to_string())?;
                    let alignment = program
                        .binding_alignment(
                            &contract.parameter_bindings,
                            &template.parameters,
                            template.result_binding,
                            &template.entry_snapshots,
                            &mut self.fuel,
                        )
                        .map_err(str::to_owned)?;
                    let aliases = alignment.aliases;
                    program.result_binding = alignment.result_binding;
                    program.entry_snapshots = alignment.entry_snapshots;
                    for predicate in program
                        .preconditions
                        .iter_mut()
                        .chain(&mut program.postconditions)
                    {
                        rebind_default_locals(predicate, &aliases, &mut self.fuel, 0)?;
                    }
                    entry_contract = Some((requirements, capabilities, program));
                    if !parameter_bindings.is_empty() {
                        if *parameter_bindings != contract.parameter_bindings
                            || parameter_bindings.len() != template.parameters.len()
                        {
                            return Err(
                                "staged method preparation bindings differ from interface".into()
                            );
                        }
                        let aliases = parameter_bindings
                            .iter()
                            .copied()
                            .zip(template.parameters.iter().copied())
                            .collect();
                        for argument in arguments.iter_mut().filter(|argument| argument.is_default)
                        {
                            rebind_default_locals(
                                &mut argument.value,
                                &aliases,
                                &mut self.fuel,
                                0,
                            )?;
                        }
                        *parameter_bindings = template.parameters.clone();
                    }
                    // The interface entry has been selected. Its callable view
                    // can now use the associated types of this implementation.
                    callee.ty = self.staging_type_view(&callee.ty)?;
                    callee.kind = K::FunctionInstance {
                        declaration: method.body,
                        arguments: method.arguments,
                    };
                }
            }
        }
        let K::Call { callee, .. } = &expression.kind else {
            return Ok(true);
        };
        let (declaration, arguments) = match &callee.kind {
            K::Function(id) => (*id, vec![]),
            K::FunctionInstance {
                declaration,
                arguments,
            } => (*declaration, arguments.clone()),
            _ => return Ok(true),
        };
        if self
            .staged_functions
            .contains_key(&(declaration, arguments.clone()))
        {
            return Ok(true);
        }
        let Some(template) = self
            .package
            .functions
            .iter()
            .chain(&self.package.quote_templates)
            .find(|f| f.declaration == declaration)
        else {
            return Ok(true);
        };
        let Term::Function { parameters, .. } = template.signature.callable_signature() else {
            return Ok(true);
        };
        if Self::returns_staged_callable(&callee.ty)
            && self.active_staged_inlines.contains(&declaration)
        {
            return Err(format!(
                "recursive staged-callback factory `{}` is not yet supported; finite target analysis is required",
                template.name
            ));
        }
        if !parameters.iter().any(|p| p.sort != nir::Sort::Runtime) {
            // Continue inline calls exposed inside a staged expansion, and
            // specialize higher-order uses of staged template arguments.
            // Ordinary runtime-only callbacks keep the existing inline policy.
            let staged_callback = parameters.iter().any(|parameter| {
                matches!(parameter.ty.callable_signature(), Term::Function { parameters, .. }
                    if parameters.iter().any(|input| input.sort != nir::Sort::Runtime))
            }) || matches!(callee.ty.callable_signature(), Term::Function { result, .. }
                if matches!(result.callable_signature(), Term::Function { parameters, .. }
                    if parameters.iter().any(|input| input.sort != nir::Sort::Runtime)));
            if !staged_callback && (self.active_staged_inlines.is_empty() || !template.inline) {
                return Ok(true);
            }
            let mut bound =
                self.world
                    .instantiate_function(template, &arguments, &mut self.fuel)?;
            if bound.phase == n::FunctionPhase::Runtime {
                let implicit = nir::program::implicit_runtime_env_parameters(
                    &self.package.runtime_domains,
                    &self.package.global_runtime_deliveries,
                    &arguments,
                    &bound.interface.env_parameters,
                )?;
                bound.interface.env_parameters.extend(implicit);
                for env in &mut bound.interface.env_parameters {
                    env.ty = self.normalized_type(&env.ty)?;
                }
            }
            bound.type_parameters.clear();
            if let Some((requirements, capabilities, program)) = entry_contract {
                Self::merge_entry_contract(&mut bound, requirements, capabilities, program);
            }
            // A staged callable is a template identity, not a runtime entry
            // address. Expose its uses before producing concrete IIR entries.
            // The normal inline argument frame preserves evaluation and contracts.
            bound.inline |= staged_callback;
            if self.expand_returning_template(&bound, expression)? {
                return Ok(false);
            }
            return self
                .expand_inline_instance(&bound, expression)
                .map(|expanded| !expanded);
        }
        // Declaration substitution closes the type inputs before staged evaluation.
        let mut template = self
            .world
            .instantiate_function(template, &arguments, &mut self.fuel)?;
        template.type_parameters.clear();
        if let Some((requirements, capabilities, program)) = entry_contract {
            Self::merge_entry_contract(&mut template, requirements, capabilities, program);
        }
        template.signature = self.staging_type_view(&template.signature)?;
        if template.phase == n::FunctionPhase::Runtime {
            let implicit = nir::program::implicit_runtime_env_parameters(
                &self.package.runtime_domains,
                &self.package.global_runtime_deliveries,
                &arguments,
                &template.interface.env_parameters,
            )?;
            template.interface.env_parameters.extend(implicit);
            for env in &mut template.interface.env_parameters {
                env.ty = self.staging_type_view(&env.ty)?;
            }
            self.deliver_staged_call_envs(expression, &template.interface.env_parameters)?;
        }
        let mut fuel = self.fuel;
        let result = iir::staging::bind_call_in_world(
            &template,
            &arguments,
            expression,
            &mut fuel,
            self,
            |this| this.world.scope().declaration_source().package().into(),
            |this, input, fuel| {
                this.fuel = *fuel;
                let result = this.complete_staged_input(input);
                *fuel = this.fuel;
                result
            },
            |this, ty, fuel| {
                this.fuel = *fuel;
                let result = this.staging_type_view(ty);
                *fuel = this.fuel;
                result
            },
        );
        self.fuel = fuel.min(self.fuel);
        let (mut instance, call) = result?;
        *expression = call;
        if instance.function.phase == n::FunctionPhase::Runtime {
            let extra = nir::program::implicit_runtime_env_parameters(
                &self.package.runtime_domains,
                &self.package.global_runtime_deliveries,
                &instance.key.arguments,
                &instance.function.interface.env_parameters,
            )?;
            if !extra.is_empty() {
                let mut normalized_extra = Vec::with_capacity(extra.len());
                for mut env in extra {
                    env.ty = self.normalized_type(&env.ty)?;
                    normalized_extra.push(env);
                }
                self.deliver_call_env_slice(expression, &normalized_extra)?;
                instance
                    .function
                    .interface
                    .env_parameters
                    .extend(normalized_extra);
            }
        }
        if matches!(instance.function.signature.callable_signature(), Term::Function { result, .. }
            if matches!(result.callable_signature(), Term::Function { parameters, .. }
                if parameters.iter().any(|p| p.sort != nir::Sort::Runtime)))
        {
            instance.function.inline = true;
        }
        let quote = self
            .package
            .quote_templates
            .iter()
            .any(|template| template.declaration == declaration);
        if !instance.function.env_parameters.is_empty() && quote {
            return Err(
                "staged quote functions with runtime environment parameters are not yet supported"
                    .into(),
            );
        }
        if quote {
            self.check_expanded_requirements(&instance.function, "quote")?;
            let mut generated = Vec::new();
            let empty_deliveries = BTreeMap::new();
            let (caller_envs, caller_deliveries) = match &self.active_caller_env {
                Some((envs, deliveries)) => (envs.as_slice(), deliveries),
                None => (&[][..], &empty_deliveries),
            };
            let in_closure = !self.active_closure_scopes.is_empty();
            self.expansion
                .ok_or("IIR materialization lacks an expansion service")?
                .quote(
                    self.world.scope().declaration_source().package(),
                    &instance.function,
                    expression,
                    caller_envs,
                    caller_deliveries,
                    in_closure,
                    &mut self.quote_cursor,
                    &mut self.quote_sources,
                    &mut generated,
                    &mut self.fuel,
                )?;
            self.register_quote_layouts(generated)?;
        } else if self.expand_returning_template(&instance.function, expression)?
            || self.expand_inline_instance(&instance.function, expression)?
        {
            // The guarded expansion already normalized its descendants. Visiting
            // again after releasing the guard would re-expand recursive calls.
            return Ok(false);
        } else {
            self.staged_functions
                .entry((instance.key.declaration, instance.key.arguments.to_vec()))
                .or_insert(instance.function);
        }
        Ok(true)
    }

    fn check_expanded_requirements(
        &mut self,
        function: &n::Function,
        category: &str,
    ) -> Result<(), String> {
        for requirement in &function.requirements {
            let requirement = self.normalized_predicate(requirement, 0)?;
            self.tick(0)?;
            if !matches!(
                self.prove_requirement(&requirement)?,
                crate::ptc::Outcome::Entailed(_)
            ) {
                return Err(format!(
                    "{category} function type requirement is not established: {requirement}"
                ));
            }
        }
        for capability in &function.capabilities {
            let capability = self.normalized_capability(capability)?;
            self.queue.push_back(Task::Interface(
                capability.interface_instance(),
                capability.subject,
            ));
        }
        Ok(())
    }

    fn expand_inline_instance(
        &mut self,
        function: &n::Function,
        expression: &mut n::Expr,
    ) -> Result<bool, String> {
        let declaration = function.declaration;
        if !function.inline
            || function.phase != n::FunctionPhase::Runtime
            || self.active_staged_inlines.contains(&declaration)
            || self.active_staged_inlines.len() >= 64
        {
            return Ok(false);
        }
        let mut generated = Vec::new();
        let empty_deliveries = BTreeMap::new();
        let (caller_envs, caller_deliveries) = match &self.active_caller_env {
            Some((envs, deliveries)) => (envs.as_slice(), deliveries),
            None => (&[][..], &empty_deliveries),
        };
        let in_closure = !self.active_closure_scopes.is_empty();
        if !self
            .expansion
            .ok_or("IIR materialization lacks an expansion service")?
            .inline(
                self.world.scope().declaration_source().package(),
                function,
                expression,
                caller_envs,
                caller_deliveries,
                in_closure,
                &mut self.quote_cursor,
                &mut self.quote_sources,
                &mut generated,
                &mut self.fuel,
            )?
        {
            return Ok(false);
        }
        self.check_expanded_requirements(function, "inline")?;
        self.register_quote_layouts(generated)?;
        self.active_staged_inlines.insert(declaration);
        let result = self.normalize_expression(expression);
        self.active_staged_inlines.remove(&declaration);
        result?;
        Ok(true)
    }

    fn collect_pattern_binders(
        pattern: &P,
        target: &mut BTreeSet<n::LocalId>,
        fuel: &mut usize,
    ) -> Result<(), String> {
        let mut work = vec![pattern];
        while let Some(current) = work.pop() {
            *fuel = fuel
                .checked_sub(1)
                .ok_or("closure scope pattern budget exhausted")?;
            match current {
                P::Binding { binding, inner } | P::Observe { binding, inner } => {
                    target.insert(*binding);
                    work.push(inner);
                }
                P::Remap { pattern, bindings } => {
                    for (_, to) in bindings {
                        target.insert(*to);
                    }
                    work.push(pattern);
                }
                P::CodeEvidence { outputs } => {
                    for (id, _) in outputs {
                        target.insert(*id);
                    }
                }
                P::OptionalPresent(inner)
                | P::ErrorResult { inner, .. }
                | P::Guard { pattern: inner, .. } => work.push(inner),
                P::Then { first, next, .. } => {
                    work.push(first);
                    work.push(next);
                }
                P::NominalType { fields, .. }
                | P::AnonymousRecord { fields, .. }
                | P::Record { fields, .. }
                | P::Tuple(fields)
                | P::TypeTuple(fields)
                | P::NormalForm {
                    arguments: fields, ..
                }
                | P::Variant { fields, .. }
                | P::Or(fields) => work.extend(fields.iter()),
                P::OptionalAbsent | P::Wild | P::Unit | P::Literal(_) => {}
            }
        }
        Ok(())
    }

    fn collect_closure_allowed_bindings(
        parameters: &[n::LocalId],
        captures: &[n::Capture],
        body: &n::Expr,
        fuel: &mut usize,
    ) -> Result<BTreeSet<n::LocalId>, String> {
        let mut allowed: BTreeSet<n::LocalId> = parameters
            .iter()
            .copied()
            .chain(captures.iter().map(|c| c.binding))
            .collect();
        nir::visit::expression_pruned(body, fuel, |expr, fuel| {
            match &expr.kind {
                K::Let { binding, .. } => {
                    allowed.insert(*binding);
                }
                K::LetPattern { pattern, .. } => {
                    Self::collect_pattern_binders(pattern, &mut allowed, fuel)?;
                }
                K::Match { arms, .. } => {
                    for arm in arms {
                        Self::collect_pattern_binders(&arm.pattern, &mut allowed, fuel)?;
                    }
                }
                K::EffectHandle { handlers, .. } => {
                    for handler in handlers {
                        for arm in &handler.arms {
                            Self::collect_pattern_binders(&arm.pattern, &mut allowed, fuel)?;
                        }
                    }
                }
                K::Closure { .. } => return Ok(false),
                _ => {}
            }
            Ok(true)
        })?;
        Ok(allowed)
    }

    fn walk_pruned_tracking_closures<F>(
        &mut self,
        expression: &mut n::Expr,
        action: &mut F,
    ) -> Result<(), String>
    where
        F: FnMut(&mut Self, &mut n::Expr) -> Result<bool, String>,
    {
        let mut fuel = self.fuel;
        let result = iir::visit::expression_mut_pruned(expression, &mut fuel, |expr, fuel| {
            self.fuel = *fuel;
            if let K::Closure {
                parameters,
                captures,
                body,
            } = &mut expr.kind
            {
                for capture in captures.iter_mut() {
                    if let Some(value) = &mut capture.value {
                        self.walk_pruned_tracking_closures(value, action)?;
                    }
                }
                let allowed = Self::collect_closure_allowed_bindings(
                    parameters,
                    captures,
                    body,
                    &mut self.fuel,
                )?;
                self.active_closure_scopes.push(allowed);
                let res =
                    self.walk_pruned_tracking_closures(std::sync::Arc::make_mut(body), action);
                self.active_closure_scopes.pop();
                res?;
                *fuel = self.fuel;
                return Ok(false);
            }
            let cont = action(self, expr)?;
            *fuel = self.fuel;
            Ok(cont)
        });
        self.fuel = fuel.min(self.fuel);
        result
    }

    fn inner_call_mut(expr: &mut n::Expr) -> Option<&mut n::Expr> {
        if matches!(expr.kind, K::Call { .. }) {
            return Some(expr);
        }
        match &mut expr.kind {
            K::Block {
                result: Some(inner),
                ..
            }
            | K::CallPreparation {
                result: Some(inner),
                ..
            } => Self::inner_call_mut(inner),
            _ => None,
        }
    }

    fn target_env_parameters(
        &mut self,
        callee: &n::Expr,
    ) -> Result<Option<Vec<n::RuntimeEnvParameter>>, String> {
        let (target_decl, target_args) = match &callee.kind {
            K::Function(id) => (*id, vec![]),
            K::FunctionInstance {
                declaration,
                arguments,
            } => (*declaration, arguments.clone()),
            K::TraitMethod {
                interface,
                interface_arguments,
                subject,
                member,
                method_arguments,
            } => {
                let instance = n::InterfaceInstance {
                    declaration: *interface,
                    arguments: interface_arguments.clone(),
                };
                let slot = self.interface(instance, subject.clone())?;
                let instance = self.output[slot].interface_instance();
                let subject = self.output[slot].subject.clone();
                let method =
                    self.specialize_method(slot, &instance, &subject, *member, method_arguments)?;
                (method.body, method.arguments)
            }
            _ => return Ok(None),
        };
        if let Some(staged) = self
            .staged_functions
            .get(&(target_decl, target_args.clone()))
        {
            if staged.phase != n::FunctionPhase::Runtime {
                return Ok(None);
            }
            return Ok(Some(staged.interface.env_parameters.clone()));
        }
        let target_fn = self
            .world
            .function_declaration(target_decl)
            .or_else(|| self.generated_functions.get(&target_decl));
        let Some(target_fn) = target_fn else {
            return Ok(None);
        };
        if target_fn.phase != n::FunctionPhase::Runtime {
            return Ok(None);
        }
        let domains = self
            .world
            .input_domains(&target_fn.type_parameters, &mut self.fuel)?;
        let frame =
            [
                nir::function_parameters::ordinary_frame(&target_fn.signature, &mut self.fuel)
                    .map_err(|e| e.to_string())?,
            ];
        let substitution = nir::DeclarationSubstitution::bounded_domains(
            &target_fn.type_parameters,
            &domains,
            &target_args,
            nir::DeclarationInputScope::ClosedInBody(&frame),
            &mut self.fuel,
        )
        .map_err(|e| e.to_string())?;
        let mut envs = Vec::with_capacity(target_fn.interface.env_parameters.len());
        for env in &target_fn.interface.env_parameters {
            let substituted = substitution
                .instantiate_bounded(
                    &Term::Rigid {
                        declaration: env.declaration,
                        sort: nir::Sort::Runtime,
                    },
                    &mut self.fuel,
                )
                .map_err(|e| e.to_string())?;
            let (declaration, runtime_id) = match substituted {
                Term::Runtime(id) => (env.declaration, Some(id)),
                Term::Rigid {
                    declaration: next,
                    sort: nir::Sort::Runtime,
                } => (next, env.runtime_id),
                _ => (env.declaration, env.runtime_id),
            };
            let ty = substitution
                .instantiate_bounded(&env.ty, &mut self.fuel)
                .map_err(|e| e.to_string())?;
            envs.push(n::RuntimeEnvParameter {
                declaration,
                runtime_id,
                binding: env.binding,
                ty,
            });
        }
        let implicit = nir::program::implicit_runtime_env_parameters(
            &self.package.runtime_domains,
            &self.package.global_runtime_deliveries,
            &target_args,
            &envs,
        )?;
        envs.extend(implicit);
        for env in &mut envs {
            env.ty = self.normalized_type(&env.ty)?;
        }
        Ok(Some(envs))
    }

    fn deliver_env_requirement(
        &mut self,
        req: &n::RuntimeEnvParameter,
        source: n::SourceRange,
        caller_env_parameters: &[n::RuntimeEnvParameter],
        caller_runtime_deliveries: &BTreeMap<nir::RuntimeId, n::Expr>,
        closure_allowed: Option<&BTreeSet<n::LocalId>>,
    ) -> Result<n::Expr, String> {
        let mut delivered = if let Some(ep) = caller_env_parameters.iter().find(|ep| {
            let matches_id = match (req.runtime_id, ep.runtime_id) {
                (Some(req_id), Some(ep_id)) => req_id == ep_id,
                (None, _) => ep.declaration == req.declaration,
                (Some(_), None) => false,
            };
            matches_id && closure_allowed.is_none_or(|allowed| allowed.contains(&ep.binding))
        }) {
            n::Expr {
                source,
                ty: ep.ty.clone(),
                kind: K::Local(ep.binding),
            }
        } else if let Some(rt_id) = req.runtime_id {
            if let Some(expr) = caller_runtime_deliveries.get(&rt_id)
                && {
                    let free = nir::free_bindings::collect(expr, &mut self.fuel)
                        .map_err(|e| e.to_string())?;
                    closure_allowed.is_none_or(|allowed| free.iter().all(|b| allowed.contains(b)))
                }
            {
                let mut expr = expr.clone();
                expr.source = source;
                expr
            } else if let Some(expr) = self
                .global_runtime_deliveries
                .get(&rt_id)
                .or_else(|| self.package.global_runtime_deliveries.get(&rt_id))
            {
                let mut expr = expr.clone();
                expr.source = source;
                expr
            } else {
                return Err(
                    "materialized call cannot deliver runtime environment parameter".into(),
                );
            }
        } else {
            return Err("materialized call cannot deliver runtime environment parameter".into());
        };
        delivered.ty = self.normalized_type(&delivered.ty)?;
        let expected_ty = self.normalized_type(&req.ty)?;
        if delivered.ty != expected_ty {
            return Err("materialized call runtime environment type mismatch".into());
        }
        Ok(delivered)
    }

    fn deliver_missing_call_env_arguments(
        &mut self,
        expr: &mut n::Expr,
        caller_env_parameters: &[n::RuntimeEnvParameter],
        caller_runtime_deliveries: &BTreeMap<nir::RuntimeId, n::Expr>,
        closure_allowed: Option<&BTreeSet<n::LocalId>>,
    ) -> Result<(), String> {
        let mut fuel = self.fuel;
        let result = iir::visit::expression_mut_pruned(expr, &mut fuel, |node, fuel| {
            self.fuel = *fuel;
            if let K::Closure {
                parameters,
                captures,
                body,
            } = &mut node.kind
            {
                for capture in captures.iter_mut() {
                    if let Some(value) = &mut capture.value {
                        self.deliver_missing_call_env_arguments(
                            value,
                            caller_env_parameters,
                            caller_runtime_deliveries,
                            closure_allowed,
                        )?;
                    }
                }
                let allowed = Self::collect_closure_allowed_bindings(
                    parameters,
                    captures,
                    body,
                    &mut self.fuel,
                )?;
                self.deliver_missing_call_env_arguments(
                    std::sync::Arc::make_mut(body),
                    caller_env_parameters,
                    caller_runtime_deliveries,
                    Some(&allowed),
                )?;
                *fuel = self.fuel;
                return Ok(false);
            }
            let call_info = if let K::Call {
                callee,
                env_arguments,
                ..
            } = &node.kind
            {
                self.target_env_parameters(callee)?
                    .map(|envs| (envs, env_arguments.len(), callee.source))
            } else {
                None
            };
            if let Some((target_envs, existing_len, call_source)) = call_info {
                if existing_len > target_envs.len() {
                    return Err("materialized call has excess runtime environment arguments".into());
                }
                if existing_len < target_envs.len() {
                    let mut extra = Vec::with_capacity(target_envs.len() - existing_len);
                    for req in &target_envs[existing_len..] {
                        extra.push(self.deliver_env_requirement(
                            req,
                            call_source,
                            caller_env_parameters,
                            caller_runtime_deliveries,
                            closure_allowed,
                        )?);
                    }
                    if let K::Call { env_arguments, .. } = &mut node.kind {
                        env_arguments.extend(extra);
                    }
                }
            }
            *fuel = self.fuel;
            Ok(true)
        });
        self.fuel = fuel.min(self.fuel);
        result
    }

    fn deliver_call_env_slice(
        &mut self,
        call: &mut n::Expr,
        reqs: &[n::RuntimeEnvParameter],
    ) -> Result<(), String> {
        if reqs.is_empty() {
            return Ok(());
        }
        let call_source = match Self::inner_call_mut(call) {
            Some(inner) => match &inner.kind {
                K::Call { callee, .. } => callee.source,
                _ => unreachable!(),
            },
            None => {
                return Err("staged call result lacks a call node for runtime environments".into());
            }
        };
        let active_env = self.active_caller_env.clone();
        let (caller_envs, caller_deliveries) = match &active_env {
            Some((envs, deliveries)) => (envs.as_slice(), deliveries),
            None => (&[][..], &BTreeMap::new()),
        };
        let closure_allowed = self.active_closure_scopes.last().cloned();
        let mut delivered = Vec::with_capacity(reqs.len());
        for req in reqs {
            delivered.push(self.deliver_env_requirement(
                req,
                call_source,
                caller_envs,
                caller_deliveries,
                closure_allowed.as_ref(),
            )?);
        }
        let inner = Self::inner_call_mut(call)
            .ok_or("staged call result lacks a call node for runtime environments")?;
        if let K::Call { env_arguments, .. } = &mut inner.kind {
            env_arguments.extend(delivered);
        }
        Ok(())
    }

    fn deliver_staged_call_envs(
        &mut self,
        call: &mut n::Expr,
        env_params: &[n::RuntimeEnvParameter],
    ) -> Result<(), String> {
        let existing_len = match Self::inner_call_mut(call) {
            Some(inner) => match &inner.kind {
                K::Call { env_arguments, .. } => env_arguments.len(),
                _ => unreachable!(),
            },
            None => return Ok(()),
        };
        if existing_len < env_params.len() {
            self.deliver_call_env_slice(call, &env_params[existing_len..])?;
        }
        Ok(())
    }

    fn normalized_capability(
        &mut self,
        capability: &n::TraitRequirement,
    ) -> Result<n::TraitRequirement, String> {
        Ok(n::TraitRequirement {
            interface: capability.interface,
            subject: self.normalized_type(&capability.subject)?,
            interface_arguments: self.normalized_interface_arguments(
                capability.interface,
                &capability.subject,
                &capability.interface_arguments,
            )?,
        })
    }

    fn normalized_predicate(
        &mut self,
        predicate: &n::TypePredicate,
        depth: usize,
    ) -> Result<n::TypePredicate, String> {
        self.tick(depth)?;
        use n::TypePredicate as P;
        Ok(match predicate {
            P::Bool(value) => P::Bool(*value),
            P::Trait(requirement) => P::Trait(self.normalized_capability(requirement)?),
            P::Equal(a, b) => P::Equal(self.normalized_type(a)?, self.normalized_type(b)?),
            P::IntLe(a, b) => P::IntLe(a.clone(), b.clone()),
            P::Kind(ty, kind) => P::Kind(self.normalized_type(ty)?, *kind),
            P::NotSized(ty) => P::NotSized(self.normalized_type(ty)?),
            P::Not(inner) => P::Not(Box::new(self.normalized_predicate(inner, depth + 1)?)),
            P::And(items) | P::Or(items) => {
                if items.len() > self.fuel {
                    return Err("IIR predicate normalization budget exhausted".into());
                }
                let items = items
                    .iter()
                    .map(|p| self.normalized_predicate(p, depth + 1))
                    .collect::<Result<_, _>>()?;
                if matches!(predicate, P::And(_)) {
                    P::And(items)
                } else {
                    P::Or(items)
                }
            }
        })
    }

    fn admit_nominal_index(&mut self, input: &Term) -> Result<bool, String> {
        let Term::NominalValue { ty, .. } = input else {
            return Ok(false);
        };
        let Term::Nominal { declaration, .. } = ty.as_ref() else {
            return Err("nominal value index has no nominal domain".into());
        };
        nir::validate_declaration_arguments_with_collections_in(
            &[*declaration],
            std::slice::from_ref(ty.as_ref()),
            std::slice::from_ref(input),
            &mut self.fuel,
            |ty, variant, fuel| {
                nir::normal_forms::query_value_fields(
                    self.world.scope().declaration_source().package(),
                    ty,
                    variant,
                    fuel,
                )
            },
            |_, _, _| false,
            |domain, fuel| {
                nir::normal_forms::list_element_budgeted(
                    self.world.scope().declaration_source().package(),
                    domain,
                    fuel,
                )
            },
            self.package.target,
        )
        .map_err(|e| e.to_string())?;
        Ok(true)
    }

    fn normalized_interface_input(&mut self, input: &Term) -> Result<Term, String> {
        if input.sort() == nir::Sort::Type {
            return self.normalized_type(input);
        }
        crate::associated::charge_projection_copy(input, &mut self.fuel)?;
        input.check_scope(&[]).map_err(|e| e.to_string())?;
        if self.admit_nominal_index(input)? || input.is_structural_index_value() {
            return Ok(input.clone());
        }
        Err(format!(
            "interface value input normalization requires domain evidence: {input}"
        ))
    }

    fn retain_associated_supply(
        &mut self,
        slot: usize,
        member: Id,
        depth: usize,
        comptime: bool,
    ) -> Result<(), String> {
        use nir::associated_initializers::AssociatedSelection;
        // Mark the demand before following code that can read the same member.
        // The builder owns this set and drops it with all output on failure.
        if !self.associated_demands.insert((slot, member)) {
            return Ok(());
        }
        let row = &self.output[slot];
        let interface = row.interface_instance();
        let subject = row.subject.clone();
        let supply = self
            .associated_value_plan(&interface, member, &subject)?
            .supply;
        self.expr(supply.expression(), depth, comptime)?;
        let output = &mut self.output[slot];
        match supply {
            AssociatedSelection::Completed(value) => {
                output.associated.push((member, value.into_expression()))
            }
            AssociatedSelection::Initializer(template) => {
                output.associated_initializers.push((member, template))
            }
        }
        Ok(())
    }

    fn associated_value_plan(
        &mut self,
        interface: &n::InterfaceInstance,
        member: Id,
        subject: &Term,
    ) -> Result<crate::associated::AssociatedValueResolution, String> {
        let mut budget = Budget {
            steps: self.fuel,
            ..self.budget
        };
        let selected = self
            .world
            .scope()
            .clone()
            .prepare_associated_member(interface, member, subject, &mut budget)
            .and_then(|selected| selected.read_in_source(&mut budget))
            .and_then(|read| match read {
                associated::AssociatedRead::Ready(value) => Ok(value),
                associated::AssociatedRead::NeedsSource(_) => {
                    Err("selected implementation has no associated value".into())
                }
            });
        self.fuel = budget.steps;
        selected.map_err(String::from)
    }

    fn normalized_type(&mut self, ty: &Term) -> Result<Term, String> {
        if self.instances.is_none() {
            return self
                .world
                .normalized_type(ty, self.budget, &mut self.fuel)
                .map_err(Into::into);
        }
        let mut host = staged::type_initializer_host(self.expansion, &mut self.invocation_snapshot);
        self.world
            .normalized_type_with_host(ty, self.budget, &mut self.fuel, Some(&mut host))
            .map_err(Into::into)
    }

    fn dispatch_type_view(&mut self, term: &Term) -> Result<Term, String> {
        let mut budget = Budget {
            steps: self.fuel,
            ..self.budget
        };
        let view = self
            .world
            .capabilities()
            .dispatch_type_view_metered(term, &mut budget);
        self.fuel = budget.steps;
        Ok(self.retain_type_dependencies(view?))
    }

    fn staging_term_view(&mut self, term: &Term) -> Result<Term, String> {
        let mut budget = Budget {
            steps: self.fuel,
            ..self.budget
        };
        let world = self.world.capabilities();
        let view = if self.instances.is_some() {
            let mut host =
                staged::type_initializer_host(self.expansion, &mut self.invocation_snapshot);
            self.world
                .scoped_type_with_host(term, &mut budget, &mut self.staging_views, &mut host)
        } else {
            world.staged_dispatch_type_view_cached_metered(
                term,
                &mut budget,
                &mut self.staging_views,
            )
        };
        self.fuel = budget.steps;
        Ok(self.retain_type_dependencies(view?))
    }

    fn retain_type_dependencies(
        &mut self,
        view: crate::associated::AssociatedTypeNormalization,
    ) -> Term {
        self.queue.extend(
            view.initializer_dependencies
                .into_iter()
                .filter(|ty| !self.types.contains(ty))
                .map(Task::Type),
        );
        view.value
    }

    fn staging_type_view(&mut self, ty: &Term) -> Result<Term, String> {
        // The annotation walker also visits complete interface identity inputs.
        if ty.sort() != nir::Sort::Type {
            // This walker also visits invocation identity inputs. Preserve
            // values for their domain-aware staging checks; an annotation view
            // must not submit them to the associated-Type normalizer or impose
            // the narrower trait-index representation rules on them.
            crate::associated::charge_projection_copy(ty, &mut self.fuel)?;
            return Ok(ty.clone());
        }
        self.staging_term_view(ty)
    }

    fn retain_staged_associated_types(&mut self, root: &Term) -> Result<(), String> {
        let mut pending = vec![(root, 0usize)];
        while let Some((node, depth)) = pending.pop() {
            self.tick(depth)?;
            if let Term::AssociatedType {
                origin,
                arguments,
                subject,
            } = node
                && node.check_scope(&[]).is_ok()
            {
                if self.world.assumptions().is_empty() {
                    crate::associated::charge_projection_copy(node, &mut self.fuel)?;
                    self.queue.push_back(Task::Type(node.clone()));
                    continue;
                }
                let normalized_subject = self.staging_type_view(subject)?;
                let mut normalized_arguments = Vec::with_capacity(arguments.len());
                for argument in arguments {
                    normalized_arguments.push(self.staging_type_view(argument)?);
                }
                let key = Term::AssociatedType {
                    origin: origin.clone(),
                    arguments: normalized_arguments,
                    subject: Box::new(normalized_subject),
                };
                if self
                    .staging_views
                    .get(&key)
                    .is_some_and(|resolved| resolved != &key)
                {
                    crate::associated::charge_projection_copy(&key, &mut self.fuel)?;
                    self.queue.push_back(Task::Type(key));
                }
            }
            node.visit_children(|child| {
                pending.push((child, depth + 1));
                Ok::<_, String>(())
            })?;
        }
        Ok(())
    }

    fn ty(&mut self, ty: Term) -> Result<(), String> {
        if !self.types.insert(ty.clone()) {
            return Ok(());
        }
        if self.instances.is_none()
            && matches!(&ty,Term::Function {parameters,..}
                if parameters.iter().any(|parameter|parameter.sort!=nir::Sort::Runtime))
        {
            // Do not enqueue a dependent child outside the telescope that
            // binds it. Validate the complete retained callable view instead;
            // concrete dependency closure runs after invocation input staging.
            self.staging_type_view(&ty)?;
            self.retain_staged_associated_types(&ty)?;
            return Ok(());
        }
        if self.instances.is_some() {
            iir::completed_dependencies(&ty, &self.runtime_bindings, &mut self.fuel)?;
        } else if let Term::Nominal { arguments, .. } = &ty {
            let selected = self.dispatch_type_view(&ty)?;
            if selected != ty {
                // Keep projection evidence as a dependency, but instantiate
                // nominal field/formation contracts with the selected inputs.
                self.queue.extend(arguments.iter().cloned().map(Task::Type));
                self.queue.push_back(Task::Type(selected));
                return Ok(());
            }
        }
        let mut children = Vec::new();
        match &ty {
            Term::MethodValue(method) => {
                let slot = self.interface(method.interface.clone(), method.subject.clone())?;
                let interface = self.output[slot].interface_instance();
                let subject = self.output[slot].subject.clone();
                let selected = self.specialize_method(
                    slot,
                    &interface,
                    &subject,
                    method.member,
                    &method.method_arguments,
                )?;
                if self.instances.is_some() {
                    let contract = self
                        .interface_contracts
                        .iter()
                        .find(|c| c.interface == interface && c.subject == subject)
                        .and_then(|c| {
                            c.methods.iter().find(|m| {
                                m.declaration == method.member
                                    && m.type_arguments == method.method_arguments
                            })
                        })
                        .ok_or("frozen method has no retained contract")?;
                    let signature = contract.signature.ordinary().map_err(|e| e.to_string())?;
                    nir::type_computations::charge_term(signature, &mut self.fuel)
                        .map_err(|e| e.to_string())?;
                    let signature = signature.clone();
                    if signature.is_comptime_callable() {
                        return Err("frozen runtime method has a comptime signature".into());
                    }
                    if matches!(signature.callable_signature(), Term::Function { parameters, .. }
                        if parameters.iter().any(|p| p.sort != nir::Sort::Runtime))
                    {
                        self.frozen_methods.push(n::Expr {
                            source: n::SourceRange { start: 0, end: 0 },
                            ty: signature,
                            kind: K::TraitMethod {
                                interface: interface.declaration,
                                interface_arguments: interface.arguments,
                                subject,
                                member: method.member,
                                method_arguments: method.method_arguments.clone(),
                            },
                        });
                    }
                    self.queue.push_back(Task::Type(Term::FunctionValue {
                        declaration: selected.body,
                        arguments: selected.arguments,
                    }));
                }
                children.extend(method.terms().cloned());
            }
            Term::FunctionValue {
                declaration,
                arguments,
            } => {
                // A callable inside a type identity is data. Validate its
                // declaration and obligations without materializing its body.
                let signature = self
                    .world
                    .callable_signature(*declaration, arguments, &[], &mut self.fuel)
                    .map_err(|error| format!("IIR callable identity: {error}"))?
                    .ok_or("IIR callable identity has no declaration evidence")?;
                // An unapplied NF head carries a public telescope. Its clauses
                // are supplied and checked only when the head is applied.
                if !signature.is_normal_form_callable()
                    && let Some(function) = self.world.function_formation(*declaration)
                {
                    let domains = self
                        .world
                        .input_domains(function.parameters, &mut self.fuel)?;
                    let substitution = nir::DeclarationSubstitution::bounded(
                        function.parameters,
                        &domains,
                        arguments,
                        &[],
                        &mut self.fuel,
                    )
                    .map_err(|e| e.to_string())?;
                    self.callable_formation(function, &substitution)?;
                }
                // Only concrete IIR publishes immutable callable evidence.
                // Annotation and dispatch exploration may still contain binders.
                if self.instances.is_some() {
                    let signature = self.normalized_type(&signature)?;
                    nir::type_computations::charge_term(&signature, &mut self.fuel)
                        .map_err(|e| e.to_string())?;
                    children.push(signature.clone());
                    self.callable_identities.push(iir::CallableIdentity::intern(
                        iir::InstanceKey {
                            declaration: *declaration,
                            arguments: arguments.clone().into(),
                        },
                        signature,
                        &mut self.type_cores,
                        &mut self.fuel,
                    )?);
                } else {
                    children.push(signature);
                }
                children.extend(arguments.iter().cloned());
            }
            Term::Nominal {
                declaration,
                arguments,
            } => {
                children.extend(arguments.iter().cloned());
                let scope = self.world.scope().clone();
                let package = scope.declaration_source().package();
                if let Some(s) = package
                    .structs
                    .iter()
                    .find(|s| s.declaration == *declaration)
                {
                    if self.instances.is_some() {
                        self.world.with_admission_queries(|queries| {
                            nir::admit_declaration_inputs(
                                &s.type_parameters,
                                &s.parameter_types,
                                arguments,
                                nir::DeclarationInputScope::Caller(&[]),
                                0,
                                &mut self.fuel,
                                queries,
                            )
                            .map_err(|failure| failure.reason)
                        })?;
                    }
                    let substitution = nir::DeclarationSubstitution::bounded(
                        &s.type_parameters,
                        &s.parameter_types,
                        arguments,
                        &[],
                        &mut self.fuel,
                    )
                    .map_err(|e| e.to_string())?;
                    self.formation_requirements(&s.requirements, &substitution)?;
                    let fields = if self.instances.is_some() {
                        let mut definition = s
                            .instantiate_definition_bounded(arguments, &[], &mut self.fuel)
                            .map_err(|e| e.to_string())?;
                        for (_, field) in &mut definition.fields {
                            *field = self.normalized_type(field)?;
                        }
                        for capability in &mut definition.capabilities {
                            *capability = self.normalized_capability(capability)?;
                        }
                        for requirement in &mut definition.requirements {
                            *requirement = self.normalized_predicate(requirement, 0)?;
                        }
                        let fields = definition
                            .fields
                            .iter()
                            .map(|(_, field)| {
                                nir::copy_budget::clone_term(field, &mut self.fuel)
                                    .map_err(|e| format!("nominal dependency copy budget: {e:?}"))
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        if definition.packed == nir::program::PackedMode::Bit {
                            validate_materialized_bit_packed_struct(
                                &definition,
                                &fields,
                                package,
                                &self.nominals,
                                &mut self.fuel,
                            )?;
                        }
                        self.nominals.push(iir::NominalInstance {
                            ty: ty.clone(),
                            definition: iir::NominalDefinition::Struct(definition),
                        });
                        fields
                    } else {
                        s.instantiate_stored_fields(arguments, &[], &mut self.fuel)
                            .map_err(|e| e.to_string())?
                    };

                    for cap in &s.capabilities {
                        let cap = substitution
                            .instantiate_requirement_bounded(cap, &mut self.fuel)
                            .map_err(|e| e.to_string())?;
                        self.queue
                            .push_back(Task::Interface(cap.interface_instance(), cap.subject));
                    }
                    children.extend(fields);
                } else if let Some(e) = package.enums.iter().find(|e| e.declaration == *declaration)
                {
                    if self.instances.is_some() {
                        self.world.with_admission_queries(|queries| {
                            nir::admit_declaration_inputs(
                                &e.type_parameters,
                                &e.parameter_types,
                                arguments,
                                nir::DeclarationInputScope::Caller(&[]),
                                0,
                                &mut self.fuel,
                                queries,
                            )
                            .map_err(|failure| failure.reason)
                        })?;
                    }
                    let substitution = nir::DeclarationSubstitution::bounded(
                        &e.type_parameters,
                        &e.parameter_types,
                        arguments,
                        &[],
                        &mut self.fuel,
                    )
                    .map_err(|e| e.to_string())?;
                    self.formation_requirements(&e.requirements, &substitution)?;
                    let fields = if self.instances.is_some() {
                        let mut definition = e
                            .instantiate_definition_bounded(arguments, &[], &mut self.fuel)
                            .map_err(|e| e.to_string())?;
                        for variant in &mut definition.variants {
                            for field in &mut variant.fields {
                                *field = self.normalized_type(field)?;
                            }
                        }
                        for capability in &mut definition.capabilities {
                            *capability = self.normalized_capability(capability)?;
                        }
                        for requirement in &mut definition.requirements {
                            *requirement = self.normalized_predicate(requirement, 0)?;
                        }
                        let fields = definition
                            .variants
                            .iter()
                            .flat_map(|variant| &variant.fields)
                            .map(|field| {
                                nir::copy_budget::clone_term(field, &mut self.fuel)
                                    .map_err(|e| format!("nominal dependency copy budget: {e:?}"))
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        self.nominals.push(iir::NominalInstance {
                            ty: ty.clone(),
                            definition: iir::NominalDefinition::Enum(definition),
                        });
                        fields
                    } else {
                        e.instantiate_stored_fields(arguments, &[], &mut self.fuel)
                            .map_err(|e| e.to_string())?
                    };

                    for cap in &e.capabilities {
                        let cap = substitution
                            .instantiate_requirement_bounded(cap, &mut self.fuel)
                            .map_err(|e| e.to_string())?;
                        self.queue
                            .push_back(Task::Interface(cap.interface_instance(), cap.subject));
                    }
                    children.extend(fields);
                } else if let Some(n) = package
                    .newtypes
                    .iter()
                    .find(|n| n.declaration == *declaration)
                {
                    // Dispatch preparation retains generic identities. Closed
                    // value admission belongs to concrete materialization, as
                    // for struct and enum; invocation checks scoped evidence.
                    if self.instances.is_some() {
                        self.world.with_admission_queries(|queries| {
                            n.admit_arguments_in_scope(arguments, &[], &mut self.fuel, queries)
                        })?;
                    }
                    let substitution = nir::DeclarationSubstitution::bounded(
                        &n.type_parameters,
                        &n.parameter_types,
                        arguments,
                        &[],
                        &mut self.fuel,
                    )
                    .map_err(|e| e.to_string())?;
                    self.formation_requirements(&n.requirements, &substitution)?;
                    let representation = if self.instances.is_some() {
                        let mut definition = n
                            .instantiate_definition_bounded(arguments, &[], &mut self.fuel)
                            .map_err(|e| e.to_string())?;
                        definition.representation =
                            self.normalized_type(&definition.representation)?;
                        for capability in &mut definition.capabilities {
                            *capability = self.normalized_capability(capability)?;
                        }
                        for requirement in &mut definition.requirements {
                            *requirement = self.normalized_predicate(requirement, 0)?;
                        }
                        let representation = nir::copy_budget::clone_term(
                            &definition.representation,
                            &mut self.fuel,
                        )
                        .map_err(|e| format!("nominal dependency copy budget: {e:?}"))?;
                        self.nominals.push(iir::NominalInstance {
                            ty: ty.clone(),
                            definition: iir::NominalDefinition::Newtype(definition),
                        });
                        representation
                    } else {
                        n.representation_budgeted(arguments, &[], &mut self.fuel)
                            .map_err(|e| e.to_string())?
                    };
                    for cap in &n.capabilities {
                        let cap = substitution
                            .instantiate_requirement_bounded(cap, &mut self.fuel)
                            .map_err(|e| e.to_string())?;
                        self.queue
                            .push_back(Task::Interface(cap.interface_instance(), cap.subject));
                    }
                    children.push(representation);
                } else if self.instances.is_some() {
                    return Err("IIR nominal type has no declaration".into());
                }
            }
            Term::NormalForm {
                head, arguments, ..
            } => {
                if self.instances.is_some() {
                    let descriptor = self
                        .package
                        .normal_forms
                        .iter()
                        .find(|h| h.declaration == *head)
                        .ok_or("IIR normal-form type has no head declaration")?;
                    self.world
                        .with_admission_queries(|queries| {
                            descriptor.check_arguments(arguments, &[], &mut self.fuel, queries)
                        })
                        .map_err(|e| format!("IIR normal-form inputs: {e}"))?;

                    self.fuel = self
                        .fuel
                        .checked_sub(descriptor.parameters.len())
                        .ok_or("normal-form parameter budget exhausted")?;
                    let parameters: Vec<_> = descriptor
                        .parameters
                        .iter()
                        .map(|p| p.declaration)
                        .collect();
                    let domains = descriptor
                        .parameters
                        .iter()
                        .map(|p| {
                            nir::copy_budget::clone_term(&p.ty, &mut self.fuel)
                                .map_err(|e| format!("normal-form domain copy budget: {e:?}"))
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    let substitution = nir::DeclarationSubstitution::bounded(
                        &parameters,
                        &domains,
                        arguments,
                        &[],
                        &mut self.fuel,
                    )
                    .map_err(|e| e.to_string())?;
                    self.formation_requirements(&descriptor.requirements, &substitution)?;
                }
                children.extend(arguments.iter().cloned());
            }
            Term::AssociatedType {
                origin,
                arguments,
                subject,
            } => {
                if self.instances.is_some() {
                    return Err("IIR requires associated type projection resolution".into());
                }
                let normalized = self.dispatch_type_view(&ty)?;
                let interface = n::InterfaceInstance {
                    declaration: origin.interface,
                    arguments: arguments.clone(),
                };
                let slot = self.interface(interface, (**subject).clone())?;
                self.retain_associated_supply(slot, origin.member, 0, true)?;
                if normalized != ty {
                    children.push(normalized);
                }
                children.extend(arguments.iter().cloned());
                children.push((**subject).clone());
            }
            Term::ErrorResult { errors, result } => {
                children.extend(errors.iter().cloned());
                children.push((**result).clone());
            }
            Term::Computation { effects, result } => {
                for operation in effects {
                    children.extend(operation.arguments.iter().cloned());
                    if self.instances.is_some() && self.effect_instances.insert(operation.clone()) {
                        self.fuel = self
                            .fuel
                            .checked_sub(self.package.effects.len())
                            .ok_or("materialized effect declaration lookup budget exhausted")?;
                        let declaration = self
                            .package
                            .effects
                            .iter()
                            .find(|d| d.declaration == operation.declaration)
                            .ok_or("materialized effect operation declaration missing")?;
                        crate::associated::charge_projection_copy(
                            &declaration.signature,
                            &mut self.fuel,
                        )
                        .map_err(|e| e.to_string())?;
                        let signature = declaration
                            .instantiate_signature(&operation.arguments, &mut self.fuel)
                            .map_err(|e| e.to_string())?;
                        let abi = declaration.abi;
                        let signature = self.normalized_type(&signature)?;
                        children.push(signature.clone());
                        self.effect_contracts.push(iir::effects::Contract {
                            operation: operation.clone(),
                            abi,
                            signature,
                        });
                    }
                }
                children.push((**result).clone());
            }
            _ => ty.visit_children(|child| {
                children.push(child.clone());
                Ok::<_, String>(())
            })?,
        }
        self.queue.extend(children.into_iter().map(Task::Type));
        Ok(())
    }

    fn pattern(&mut self, pattern: &P, depth: usize, comptime: bool) -> Result<(), String> {
        self.tick(depth)?;
        match pattern {
            P::ErrorResult { label, inner } => {
                if let Some(label) = label {
                    self.queue.push_back(Task::Type(label.clone()));
                }
                self.pattern(inner, depth + 1, comptime)?;
            }
            P::CodeEvidence { .. } => return Err("unresolved pattern-code evidence".into()),
            P::Then { first, value, next } => {
                self.pattern(first, depth + 1, comptime)?;
                self.expr(value, depth + 1, comptime)?;
                self.pattern(next, depth + 1, comptime)?;
            }
            P::Guard { pattern, condition } => {
                self.pattern(pattern, depth + 1, comptime)?;
                self.expr(condition, depth + 1, comptime)?;
            }
            P::OptionalPresent(pattern)
            | P::Observe { inner: pattern, .. }
            | P::Remap { pattern, .. }
            | P::Binding { inner: pattern, .. } => self.pattern(pattern, depth + 1, comptime)?,
            P::NominalType { fields, .. }
            | P::Record { fields, .. }
            | P::AnonymousRecord { fields, .. }
            | P::Variant { fields, .. }
            | P::NormalForm {
                arguments: fields, ..
            }
            | P::TypeTuple(fields)
            | P::Tuple(fields)
            | P::Or(fields) => {
                for p in fields {
                    self.pattern(p, depth + 1, comptime)?;
                }
            }
            P::OptionalAbsent | P::Wild | P::Unit | P::Literal(_) => {}
        }
        Ok(())
    }

    fn expr(&mut self, expr: &n::Expr, depth: usize, comptime: bool) -> Result<(), String> {
        // Ordinary operation children use a heap frontier. Preserve preorder,
        // type dependencies and semantic depth, without retaining the large
        // expr_node frame once per unary/binary operand. Lexical preparation
        // wrappers still do not consume recursive semantic depth.
        let mut pending = vec![(expr, depth)];
        while let Some((expr, depth)) = pending.pop() {
            match &expr.kind {
                K::Block { statements, result } | K::CallPreparation { statements, result } => {
                    if let Some(result) = result {
                        pending.push((result, depth));
                    }
                    pending.extend(statements.iter().rev().map(|expr| (expr, depth)));
                }
                K::AnonymousField { value, .. }
                | K::Field { value, .. }
                | K::Let { value, .. }
                | K::Assign { value, .. }
                | K::UnsafeScope(value)
                | K::Discard(value)
                | K::RequirePredicate(value)
                | K::EnsurePredicate(value)
                | K::CheckPredicate(value)
                | K::BitSlice { value, .. }
                | K::AddressOf(value)
                | K::ConstAddress { value, .. }
                | K::ReadPointer(value)
                | K::PointerAnonymousField { pointer: value, .. }
                | K::PointerField { pointer: value, .. }
                | K::OptionalPresent(value)
                | K::OptionalType(value)
                | K::PointerType(value)
                | K::ListLength(value)
                | K::NewtypeRepresentation(value)
                | K::DefinitionAttributes(value)
                | K::StructFieldTypes(value)
                | K::BuildStruct(value)
                | K::BuildNewtype(value)
                | K::BuildEnum(value)
                | K::EnumStructure { subject: value, .. }
                | K::ObjectQuery { subject: value, .. }
                | K::ObjectEdit { subject: value, .. }
                | K::StructFieldNames(value)
                | K::BitVecType(value)
                | K::IntegerNegate(value)
                | K::FloatNegate(value)
                | K::BitConvert(value)
                | K::ErrorConvert { value: value, .. }
                | K::NewtypeConvert(value)
                | K::Snapshot(value)
                | K::TakeBits(value)
                | K::TextQuery { subject: value, .. }
                | K::IntegerRepresentation(value)
                | K::TryIntegerConvert(value)
                | K::ComptimeIntegerConvert(value)
                | K::SymbolName(value)
                | K::EnumName(value)
                | K::TypeKind { operand: value, .. }
                | K::TypeLayout { operand: value, .. }
                | K::Not(value)
                | K::Loop { body: value, .. } => pending.push((value, depth + 1)),
                K::WritePointer {
                    pointer: a,
                    value: b,
                }
                | K::FunctionResultType {
                    signature: a,
                    result: b,
                }
                | K::IntegerBinary {
                    left: a, right: b, ..
                }
                | K::FloatBinary {
                    left: a, right: b, ..
                }
                | K::ScalarLess(a, b)
                | K::ScalarGreater(a, b)
                | K::ScalarGreaterEqual(a, b)
                | K::ScalarLessEqual(a, b)
                | K::TemplateEqual(a, b)
                | K::ScalarEqual(a, b)
                | K::TypeEqual(a, b)
                | K::ListConcat(a, b)
                | K::TypeListIndex(a, b)
                | K::BitEqual(a, b)
                | K::BitConcat(a, b)
                | K::BitIndex(a, b)
                | K::And(a, b)
                | K::Or(a, b) => {
                    pending.push((b, depth + 1));
                    pending.push((a, depth + 1));
                }
                _ => {
                    self.expr_node(expr, depth, comptime)?;
                    continue;
                }
            }
            self.tick(depth)?;
            if self.instances.is_some() {
                iir::completed_type(&expr.ty, &self.runtime_bindings, &mut self.fuel)?;
            }
            self.queue.push_back(Task::Type(expr.ty.clone()));
        }
        Ok(())
    }

    #[inline(never)]
    fn expr_node(&mut self, expr: &n::Expr, depth: usize, comptime: bool) -> Result<(), String> {
        self.tick(depth)?;
        if self.instances.is_some() {
            iir::completed_type(&expr.ty, &self.runtime_bindings, &mut self.fuel)?;
        }
        self.queue.push_back(Task::Type(expr.ty.clone()));
        if self.instances.is_some()
            && comptime
            && !expr.ty.is_comptime_callable()
            && let Some((declaration, arguments)) = expr.named_function()
        {
            for argument in arguments {
                nir::type_computations::charge_term(argument, &mut self.fuel)
                    .map_err(|e| e.to_string())?;
            }
            self.queue.push_back(Task::Type(Term::FunctionValue {
                declaration,
                arguments: arguments.to_vec(),
            }));
            return Ok(());
        }
        match &expr.kind {
            K::Intrinsic { tag, inputs, .. } => {
                nir::intrinsics::checked(expr).map_err(str::to_owned)?;
                self.queue.push_back(Task::Type(Term::Nominal {
                    declaration: tag.family,
                    arguments: vec![],
                }));
                for input in inputs {
                    self.expr(input, depth + 1, comptime)?;
                }
            }
            K::EffectHandle {
                computation,
                handlers,
                outer,
            } => {
                self.expr(computation, depth + 1, comptime)?;
                for operation in outer {
                    for argument in &operation.arguments {
                        self.queue.push_back(Task::Type(argument.clone()));
                    }
                }
                for handler in handlers {
                    self.queue.push_back(Task::Type(handler.signature.clone()));
                    for argument in &handler.operation.arguments {
                        self.queue.push_back(Task::Type(argument.clone()));
                    }
                    for arm in &handler.arms {
                        self.pattern(&arm.pattern, depth + 1, comptime)?;
                        if let Some(g) = &arm.guard {
                            self.expr(g, depth + 1, comptime)?;
                        }
                        self.expr(&arm.body, depth + 1, comptime)?;
                    }
                }
            }
            K::EffectResume(computation) | K::EffectRun { computation, .. } => {
                self.expr(computation, depth + 1, comptime)?;
            }
            K::EffectComputation { effects, body } => {
                for operation in effects {
                    for argument in &operation.arguments {
                        self.queue.push_back(Task::Type(argument.clone()));
                    }
                }
                self.expr(body, depth + 1, comptime)?;
            }
            K::EffectRequest {
                operation,
                signature,
                arguments,
                ..
            } => {
                self.queue.push_back(Task::Type((**signature).clone()));
                for argument in &operation.arguments {
                    self.queue.push_back(Task::Type(argument.clone()));
                }
                for argument in arguments {
                    self.expr(argument, depth + 1, comptime)?;
                }
            }
            K::AssociatedValue {
                interface,
                subject,
                member,
            } => {
                if self.instances.is_some() {
                    return Err("unresolved associated value in IIR".into());
                }
                let slot = self.interface(interface.clone(), subject.clone())?;
                self.retain_associated_supply(slot, *member, depth + 1, comptime)?;
            }
            K::Function(id) if expr.ty.is_normal_form_callable() => {
                if self.instances.is_some() {
                    return Err("normal-form callable cannot enter executable IIR".into());
                }
                let descriptor = self
                    .package
                    .normal_forms
                    .iter()
                    .find(|head| head.declaration == *id)
                    .ok_or("normal-form callable declaration is missing")?;
                if descriptor
                    .callable_type_in(&mut self.fuel)
                    .map_err(|error| error.to_string())?
                    != expr.ty
                {
                    return Err("normal-form callable type disagrees with its declaration".into());
                }
            }
            // Stored runtime template identities have no executable body in
            // IIR. Calls must specialize independently; the final graph checks
            // that no runtime call retains staged inputs and validates every
            // stored identity against its source declaration.
            K::Function(_) | K::FunctionInstance { .. }
                if matches!(expr.ty.callable_signature(), Term::Function { parameters, .. }
                    if (self.instances.is_none() && parameters.iter().any(|p| p.sort == nir::Sort::Type))
                        || (!expr.ty.is_comptime_callable() && parameters.iter().any(|p| p.sort != nir::Sort::Runtime))) =>
                {}
            K::Function(id) => self.queue.push_back(Task::Function(*id, vec![])),
            K::FunctionInstance {
                declaration,
                arguments,
            } => self
                .queue
                .push_back(Task::Function(*declaration, arguments.clone())),
            K::ComptimeFunctionInstance {
                declaration,
                inputs,
            } => {
                for operand in inputs.expressions() {
                    self.expr(operand, depth + 1, comptime)?;
                }
                let Some((interface, member)) = self.package.traits.iter().find_map(|interface| {
                    interface
                        .methods
                        .iter()
                        .find(|member| member.declaration == *declaration)
                        .map(|member| (interface, member))
                }) else {
                    if self.instances.is_some() {
                        let arguments =
                            iir::observed_clause_arguments(*declaration, inputs, &mut self.fuel)?;
                        self.queue
                            .push_back(Task::Function(*declaration, arguments));
                    }
                    return Ok(());
                };
                if self.instances.is_some() {
                    return Err("dynamic trait clauses require comptime invocation before IIR instantiation".into());
                }
                let values = &inputs.arguments;
                let prefix_count = 1 + interface.type_parameters.len();
                if values.len() != prefix_count + member.type_parameters.len() {
                    return Err("dynamic trait clause packet arity mismatch".into());
                }
                let prefix = (0..prefix_count)
                    .map(|slot| {
                        let identity = inputs.observed_argument(slot).ok_or(
                            "dynamic trait dispatch requires known Self and interface inputs",
                        )?;
                        nir::copy_budget::clone_term(identity, &mut self.fuel)
                            .map_err(|reason| format!("method input copy budget: {reason:?}"))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let interface_id = interface.declaration;
                let member_id = member.declaration;
                let member_phase = member.phase;
                let member_name = member.name.clone();
                let default = member.default;
                let method_arguments = self
                    .input_binders(&member.type_parameters)
                    .iter()
                    .map(|parameter| Term::Rigid {
                        declaration: parameter.declaration,
                        sort: parameter.sort,
                    })
                    .collect::<Vec<_>>();
                let slot = self.interface(
                    n::InterfaceInstance {
                        declaration: interface_id,
                        arguments: prefix[1..].to_vec(),
                    },
                    prefix[0].clone(),
                )?;
                let implementation = self
                    .package
                    .implementations
                    .iter()
                    .find(|item| item.declaration == self.output[slot].implementation)
                    .ok_or("missing dynamic method implementation")?;
                if !implementation.phases.contains(member_phase) {
                    return Err("selected implementation lacks required trait phase".into());
                }
                let body = implementation
                    .methods
                    .iter()
                    .find(|(name, _)| *name == member_name)
                    .ok_or("missing dynamic method body")?
                    .1;
                let mut arguments = if default == Some(body) {
                    prefix
                } else {
                    self.output[slot].arguments.clone()
                };
                arguments.extend(method_arguments.iter().cloned());
                if !self.output[slot].methods.iter().any(|method| {
                    method.member == member_id && method.method_arguments == method_arguments
                }) {
                    self.output[slot].methods.push(n::MethodInstance {
                        member: member_id,
                        body,
                        arguments: arguments.clone(),
                        method_arguments,
                    });
                    self.queue.push_back(Task::Function(body, arguments));
                }
            }
            K::TraitMethod {
                method_arguments,
                interface,
                interface_arguments,
                member,
                subject,
            } => {
                let instance = n::InterfaceInstance {
                    declaration: *interface,
                    arguments: interface_arguments.clone(),
                };
                let slot = self.interface(instance.clone(), subject.clone())?;
                let instance = self.output[slot].interface_instance();
                let subject = self.output[slot].subject.clone();
                let method =
                    self.specialize_method(slot, &instance, &subject, *member, method_arguments)?;
                if self.instances.is_some() && comptime && !expr.ty.is_comptime_callable() {
                    self.queue.push_back(Task::Type(Term::FunctionValue {
                        declaration: method.body,
                        arguments: method.arguments,
                    }));
                    return Ok(());
                }
                if !expr.ty.is_comptime_callable()
                    && matches!(expr.ty.callable_signature(), Term::Function { parameters, .. }
                        if parameters.iter().any(|p| p.sort != nir::Sort::Runtime))
                {
                    return Ok(());
                }
                if self.instances.is_some()
                    || !self
                        .package
                        .quote_templates
                        .iter()
                        .any(|template| template.declaration == method.body)
                {
                    self.queue
                        .push_back(Task::Function(method.body, method.arguments));
                }
            }
            K::Closure { captures, body, .. } => {
                for c in captures {
                    self.queue.push_back(Task::Type(c.ty.clone()));
                    if let Some(value) = &c.value {
                        self.expr(value, depth + 1, comptime)?;
                    }
                }
                self.expr(body, depth + 1, comptime)?;
            }
            K::PlannedFunctionInstance {
                arguments, plan, ..
            }
            | K::NormalFormType {
                arguments, plan, ..
            }
            | K::ApplyNormalForm {
                arguments, plan, ..
            } => {
                if matches!(expr.kind, K::PlannedFunctionInstance { .. })
                    && self.instances.is_some()
                {
                    return Err("contextual callable inputs require comptime execution before IIR instantiation".into());
                }
                if let K::ApplyNormalForm { callee, .. } = &expr.kind {
                    self.expr(callee, depth + 1, comptime)?;
                }
                for p in plan.iter().flat_map(|p| &p.preparations) {
                    self.expr(&p.expression, depth + 1, comptime)?;
                }
                for pattern in arguments.iter().flat_map(|a| a.patterns()) {
                    self.pattern(pattern, depth + 1, comptime)?;
                }
                for e in arguments.iter().flat_map(|a| a.expressions()) {
                    self.expr(e, depth + 1, comptime)?;
                }
            }
            K::NominalType {
                arguments,
                preparations,
                ..
            } => {
                for preparation in preparations {
                    self.expr(&preparation.expression, depth + 1, comptime)?;
                }
                for pattern in arguments.iter().flat_map(|a| a.patterns()) {
                    self.pattern(pattern, depth + 1, comptime)?;
                }
                for value in arguments.iter().flat_map(|a| a.expressions()) {
                    self.expr(value, depth + 1, comptime)?;
                }
            }
            K::ClosureType {
                environment,
                signature,
            } => {
                for field in environment {
                    self.expr(field, depth + 1, comptime)?;
                }
                self.expr(signature, depth + 1, comptime)?;
            }
            K::WithTypeInputs {
                arguments,
                body,
                preparations,
                ..
            } => {
                for preparation in preparations {
                    self.expr(&preparation.expression, depth + 1, comptime)?;
                }
                for argument in arguments {
                    for pattern in argument.patterns() {
                        self.pattern(pattern, depth + 1, comptime)?;
                    }
                    for expression in argument.expressions() {
                        self.expr(expression, depth + 1, comptime)?;
                    }
                }
                self.expr(body, depth + 1, comptime)?;
            }
            K::CheckTypeAlias {
                arguments,
                body,
                preparations,
                ..
            } => {
                for preparation in preparations {
                    self.expr(&preparation.expression, depth + 1, comptime)?;
                }
                for argument in arguments {
                    for pattern in argument.patterns() {
                        self.pattern(pattern, depth + 1, comptime)?;
                    }
                    for expression in argument.expressions() {
                        self.expr(expression, depth + 1, comptime)?;
                    }
                }
                if let Some(body) = body {
                    self.expr(body, depth + 1, comptime)?;
                }
            }
            K::Variant { fields, .. }
            | K::Object { fields, .. }
            | K::AnonymousRecord { fields }
            | K::Record { fields, .. }
            | K::FunctionType {
                parameters: fields, ..
            }
            | K::TypeList(fields)
            | K::Tuple(fields) => {
                for e in fields {
                    self.expr(e, depth + 1, comptime)?;
                }
            }
            K::BitWritePointer {
                pointer,
                selections,
                value,
            } => {
                self.expr(pointer, depth + 1, comptime)?;
                for input in selections
                    .iter()
                    .flat_map(nir::program::BitSelection::expressions)
                {
                    self.expr(input, depth + 1, comptime)?;
                }
                self.expr(value, depth + 1, comptime)?;
            }
            K::BitUpdate {
                storage_type,
                selections,
                value,
                ..
            } => {
                self.queue.push_back(Task::Type(storage_type.clone()));
                for input in selections
                    .iter()
                    .flat_map(nir::program::BitSelection::expressions)
                {
                    self.expr(input, depth + 1, comptime)?;
                }
                self.expr(value, depth + 1, comptime)?;
            }
            K::BitAssign {
                storage_type,
                value,
                ..
            } => {
                self.queue.push_back(Task::Type(storage_type.clone()));
                self.expr(value, depth + 1, comptime)?;
            }
            K::AnonymousField { value, .. }
            | K::Field { value, .. }
            | K::Let { value, .. }
            | K::Assign { value, .. }
            | K::UnsafeScope(value)
            | K::Discard(value)
            | K::RequirePredicate(value)
            | K::EnsurePredicate(value)
            | K::CheckPredicate(value)
            | K::BitSlice { value, .. }
            | K::AddressOf(value)
            | K::ConstAddress { value, .. }
            | K::ReadPointer(value)
            | K::PointerAnonymousField { pointer: value, .. }
            | K::PointerField { pointer: value, .. }
            | K::OptionalPresent(value)
            | K::OptionalType(value)
            | K::PointerType(value)
            | K::ListLength(value)
            | K::NewtypeRepresentation(value)
            | K::DefinitionAttributes(value)
            | K::StructFieldTypes(value)
            | K::BuildStruct(value)
            | K::BuildNewtype(value)
            | K::BuildEnum(value)
            | K::EnumStructure { subject: value, .. }
            | K::ObjectQuery { subject: value, .. }
            | K::ObjectEdit { subject: value, .. }
            | K::StructFieldNames(value)
            | K::BitVecType(value)
            | K::IntegerNegate(value)
            | K::FloatNegate(value)
            | K::BitConvert(value)
            | K::ErrorConvert { value: value, .. }
            | K::NewtypeConvert(value)
            | K::Snapshot(value)
            | K::TakeBits(value)
            | K::TextQuery { subject: value, .. }
            | K::IntegerRepresentation(value)
            | K::TryIntegerConvert(value)
            | K::ComptimeIntegerConvert(value)
            | K::SymbolName(value)
            | K::EnumName(value)
            | K::TypeKind { operand: value, .. }
            | K::TypeLayout { operand: value, .. }
            | K::Not(value)
            | K::Loop { body: value, .. } => self.expr(value, depth + 1, comptime)?,
            K::BitWindow { value, start, end } => {
                self.expr(value, depth + 1, comptime)?;
                self.expr(start, depth + 1, comptime)?;
                self.expr(end, depth + 1, comptime)?;
            }
            K::Match { value, arms } => {
                self.expr(value, depth + 1, comptime)?;
                for arm in arms {
                    self.pattern(&arm.pattern, depth + 1, comptime)?;
                    if let Some(g) = &arm.guard {
                        self.expr(g, depth + 1, comptime)?;
                    }
                    self.expr(&arm.body, depth + 1, comptime)?;
                }
            }
            K::Block { statements, result } | K::CallPreparation { statements, result } => {
                for e in statements {
                    self.expr(e, depth + 1, comptime)?;
                }
                if let Some(e) = result {
                    self.expr(e, depth + 1, comptime)?;
                }
            }
            K::LetPattern {
                pattern,
                value,
                guard,
                otherwise,
            } => {
                self.pattern(pattern, depth + 1, comptime)?;
                self.expr(value, depth + 1, comptime)?;
                if let Some(e) = guard {
                    self.expr(e, depth + 1, comptime)?;
                }
                if let Some(e) = otherwise {
                    self.expr(e, depth + 1, comptime)?;
                }
            }
            K::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.expr(condition, depth + 1, comptime)?;
                self.expr(then_branch, depth + 1, comptime)?;
                if let Some(e) = else_branch {
                    self.expr(e, depth + 1, comptime)?;
                }
            }
            K::Return(value) => {
                if let Some(e) = value {
                    self.expr(e, depth + 1, comptime)?;
                }
            }
            K::Call {
                callee,
                arguments,
                env_arguments,
                ..
            } => {
                if self.instances.is_none()
                    && matches!(callee.ty.callable_signature(), Term::Function { parameters, .. }
                        if parameters.iter().any(|p| p.sort == nir::Sort::Type))
                {
                    match &callee.kind {
                        K::Function(id) => self.queue.push_back(Task::Function(*id, vec![])),
                        K::FunctionInstance {
                            declaration,
                            arguments,
                        } => self
                            .queue
                            .push_back(Task::Function(*declaration, arguments.clone())),
                        _ => {}
                    }
                }
                self.expr(callee, depth + 1, comptime)?;
                for a in arguments {
                    self.expr(&a.value, depth + 1, comptime)?;
                }
                for arg in env_arguments {
                    self.expr(arg, depth + 1, comptime)?;
                }
            }
            K::WritePointer {
                pointer: a,
                value: b,
            }
            | K::FunctionResultType {
                signature: a,
                result: b,
            }
            | K::IntegerBinary {
                left: a, right: b, ..
            }
            | K::FloatBinary {
                left: a, right: b, ..
            }
            | K::ScalarLess(a, b)
            | K::ScalarGreater(a, b)
            | K::ScalarGreaterEqual(a, b)
            | K::ScalarLessEqual(a, b)
            | K::TemplateEqual(a, b)
            | K::ScalarEqual(a, b)
            | K::TypeEqual(a, b)
            | K::ListConcat(a, b)
            | K::TypeListIndex(a, b)
            | K::BitEqual(a, b)
            | K::BitConcat(a, b)
            | K::BitIndex(a, b)
            | K::And(a, b)
            | K::Or(a, b) => {
                self.expr(a, depth + 1, comptime)?;
                self.expr(b, depth + 1, comptime)?;
            }
            K::BuildMetadata {
                selector, entries, ..
            } => {
                self.expr(selector, depth + 1, comptime)?;
                for (_, _, value) in entries {
                    if let Some(value) = value {
                        // Retain completed immutable snapshots for comptime
                        // invocation. Runtime residual queries are rejected by
                        // the concrete artifact's phase validation.
                        if self.instances.is_some() {
                            iir::completed_type(value, &self.runtime_bindings, &mut self.fuel)?;
                        }
                        self.queue.push_back(Task::Type(value.clone()));
                    }
                }
            }
            K::StagedValue(_) => {
                return Err("dispatch retains an unbound staged value".into());
            }
            K::ComptimeValue(ty) | K::TypeValue(ty) => {
                if self.instances.is_some() {
                    iir::completed_type(ty, &self.runtime_bindings, &mut self.fuel)?;
                }
                self.queue.push_back(Task::Type(ty.clone()));
            }
            K::Literal(_)
            | K::Local(_)
            | K::OptionalAbsent
            | K::Unit
            | K::Break(_)
            | K::Continue(_)
            | K::StaticAddress(_) => {}
        }
        Ok(())
    }
}

struct MaterializationLayoutWorld<'a> {
    package: &'a n::Package,
    nominals: &'a [iir::NominalInstance],
}

impl nir::LayoutWorld for MaterializationLayoutWorld<'_> {
    fn target(&self) -> Option<nir::target::TargetData> {
        self.package.target
    }

    fn is_tuple_head(&self, head: Id) -> bool {
        self.package
            .normal_forms
            .iter()
            .any(|h| h.declaration == head && h.role == nir::normal_forms::Role::Tuple)
    }

    fn is_list_head(&self, head: Id) -> bool {
        self.package
            .normal_forms
            .iter()
            .any(|h| h.declaration == head && h.role == nir::normal_forms::Role::TypeList)
    }

    fn nominal_kind(&self, ty: &Term) -> Option<nir::NominalKind> {
        let Term::Nominal { declaration, .. } = ty else {
            return None;
        };
        if self
            .package
            .structs
            .iter()
            .any(|s| s.declaration == *declaration)
        {
            Some(nir::NominalKind::Struct)
        } else if self
            .package
            .enums
            .iter()
            .any(|e| e.declaration == *declaration)
        {
            Some(nir::NominalKind::Enum)
        } else if self
            .package
            .newtypes
            .iter()
            .any(|n| n.declaration == *declaration)
        {
            Some(nir::NominalKind::Newtype)
        } else {
            None
        }
    }

    fn nominal_layout(
        &self,
        ty: &Term,
        fuel: &mut usize,
    ) -> Result<Option<nir::NominalLayout>, nir::LayoutError> {
        let Term::Nominal {
            declaration,
            arguments,
        } = ty
        else {
            return Ok(None);
        };
        let admission_err = |err: nir::NominalAdmissionError| match err {
            nir::NominalAdmissionError::Budget => nir::LayoutError::BudgetExhausted,
            nir::NominalAdmissionError::Unresolved(_) => nir::LayoutError::Unresolved(ty.clone()),
            nir::NominalAdmissionError::Invalid(_) => nir::LayoutError::Unsupported(ty.clone()),
        };
        if let Some(instance) = self.nominals.iter().find(|item| item.ty == *ty) {
            return Ok(Some(match &instance.definition {
                iir::NominalDefinition::Struct(def) => {
                    let fields = def.fields.iter().map(|(_, t)| t.clone()).collect();
                    match def.packed {
                        n::PackedMode::None => nir::NominalLayout::Struct(fields),
                        n::PackedMode::Byte => nir::NominalLayout::PackedStruct(fields),
                        n::PackedMode::Bit => nir::NominalLayout::BitPackedStruct(fields),
                    }
                }
                iir::NominalDefinition::Newtype(def) => {
                    nir::NominalLayout::Newtype(def.representation.clone())
                }
                iir::NominalDefinition::Enum(def) => {
                    if def.symbol_encoding.is_some() {
                        nir::NominalLayout::EnumSymbol
                    } else if let Some(encoding) = &def.encoding {
                        let bits = u32::try_from(encoding.width())
                            .map_err(|_| nir::LayoutError::Unsupported(ty.clone()))?;
                        nir::NominalLayout::EnumFixedBits {
                            width: bits,
                            niche: encoding.layout.contiguous_niche(),
                            exhaustive: encoding.layout.is_exhaustive(),
                        }
                    } else if def.variants.iter().all(|v| v.fields.is_empty()) {
                        nir::NominalLayout::EnumUnit(def.variants.len())
                    } else {
                        nir::NominalLayout::EnumAggregate(
                            def.variants.iter().map(|v| v.fields.clone()).collect(),
                        )
                    }
                }
            }));
        }
        if let Some(def) = self
            .package
            .structs
            .iter()
            .find(|s| s.declaration == *declaration)
        {
            let fields = def
                .instantiate_stored_fields(arguments, &[], fuel)
                .map_err(|err| admission_err(err.into()))?;
            return Ok(Some(match def.packed {
                n::PackedMode::None => nir::NominalLayout::Struct(fields),
                n::PackedMode::Byte => nir::NominalLayout::PackedStruct(fields),
                n::PackedMode::Bit => nir::NominalLayout::BitPackedStruct(fields),
            }));
        }
        if let Some(def) = self
            .package
            .newtypes
            .iter()
            .find(|n| n.declaration == *declaration)
        {
            let repr =
                def.representation_budgeted(arguments, &[], fuel)
                    .map_err(|err| match err {
                        nir::newtypes::ConversionError::Budget => nir::LayoutError::BudgetExhausted,
                        _ => nir::LayoutError::Unsupported(ty.clone()),
                    })?;
            return Ok(Some(nir::NominalLayout::Newtype(repr)));
        }
        if let Some(def) = self
            .package
            .enums
            .iter()
            .find(|e| e.declaration == *declaration)
        {
            if def.symbol_encoding.is_some() {
                return Ok(Some(nir::NominalLayout::EnumSymbol));
            }
            if let Some(encoding) = &def.encoding {
                let bits = u32::try_from(encoding.width())
                    .map_err(|_| nir::LayoutError::Unsupported(ty.clone()))?;
                return Ok(Some(nir::NominalLayout::EnumFixedBits {
                    width: bits,
                    niche: encoding.layout.contiguous_niche(),
                    exhaustive: encoding.layout.is_exhaustive(),
                }));
            }
            if def.variants.iter().all(|v| v.fields.is_empty()) {
                return Ok(Some(nir::NominalLayout::EnumUnit(def.variants.len())));
            }
            let variants = def
                .instantiate_variant_fields(arguments, &[], fuel)
                .map_err(|err| admission_err(err.into()))?;
            return Ok(Some(nir::NominalLayout::EnumAggregate(variants)));
        }
        Ok(None)
    }
}

fn validate_materialized_bit_packed_struct(
    definition: &n::Struct,
    fields: &[Term],
    package: &n::Package,
    nominals: &[iir::NominalInstance],
    fuel: &mut usize,
) -> Result<(), String> {
    let world = MaterializationLayoutWorld { package, nominals };
    match nir::bit_packed_struct_layout(&world, fields, fuel) {
        Ok(_) | Err(nir::BitPackedError::Layout(nir::LayoutError::Unresolved(_))) => Ok(()),
        Err(nir::BitPackedError::Layout(nir::LayoutError::TargetRequired)) => {
            for (field_index, field_ty) in fields.iter().enumerate() {
                match nir::bit_packed_field_info(&world, field_ty, fuel) {
                    Ok(_)
                    | Err(nir::BitPackedError::Layout(
                        nir::LayoutError::TargetRequired | nir::LayoutError::Unresolved(_),
                    )) => {}
                    Err(nir::BitPackedError::UnsupportedField { ty, reason, .. }) => {
                        let name = definition
                            .fields
                            .get(field_index)
                            .map(|(n, _)| n.as_str())
                            .unwrap_or("<field>");
                        return Err(format!(
                            "attrs.packed.bit struct `{}` field `{name}` of type `{ty}` is not allowed: {reason}",
                            definition.name
                        ));
                    }
                    Err(err) => {
                        return Err(format!(
                            "attrs.packed.bit struct `{}`: {err}",
                            definition.name
                        ));
                    }
                }
            }
            Ok(())
        }
        Err(nir::BitPackedError::UnsupportedField {
            field_index,
            ty,
            reason,
        }) => {
            let name = definition
                .fields
                .get(field_index)
                .map(|(n, _)| n.as_str())
                .unwrap_or("<field>");
            Err(format!(
                "attrs.packed.bit struct `{}` field `{name}` of type `{ty}` is not allowed: {reason}",
                definition.name
            ))
        }
        Err(err) => Err(format!(
            "attrs.packed.bit struct `{}`: {err}",
            definition.name
        )),
    }
}

fn validate_materialized_bit_convert_safety(
    output: &iir::Package,
    package: &n::Package,
    fuel: &mut usize,
) -> Result<(), String> {
    let world = MaterializationLayoutWorld {
        package,
        nominals: &output.nominals,
    };
    let mut pending: Vec<(&n::Expr, bool)> = Vec::new();
    for instance in &output.functions {
        let is_unsafe = instance.function.interface.is_unsafe;
        pending.push((&instance.function.body, is_unsafe));
        for pre in &instance.function.interface.preconditions {
            pending.push((pre, is_unsafe));
        }
        for post in &instance.function.interface.postconditions {
            pending.push((post, is_unsafe));
        }
        for (_, default) in &instance.function.interface.defaults {
            pending.push((default, is_unsafe));
        }
        for expr in instance.function.runtime_deliveries.values() {
            pending.push((expr, is_unsafe));
        }
    }
    for expr in output.global_runtime_deliveries.values() {
        pending.push((expr, false));
    }
    for interface in &output.interfaces {
        for method in &interface.methods {
            let is_unsafe = method.is_unsafe;
            for pre in &method.program.preconditions {
                pending.push((pre, is_unsafe));
            }
            for post in &method.program.postconditions {
                pending.push((post, is_unsafe));
            }
        }
    }
    while let Some((root, unsafe_permitted)) = pending.pop() {
        nir::visit::expression_pruned_checked(root, fuel, |expr, fuel| -> Result<bool, String> {
            match &expr.kind {
                n::ExprKind::UnsafeScope(body) => {
                    pending.push((body, true));
                    Ok(false)
                }
                n::ExprKind::Closure { body, .. } => {
                    pending.push((body, false));
                    Ok(false)
                }
                n::ExprKind::BitConvert(inner) if matches!(expr.ty, Term::Nominal { .. }) => {
                    if let Some(instance) = output.nominals.iter().find(|n| n.ty == expr.ty)
                        && let iir::NominalDefinition::Struct(def) = &instance.definition
                        && def.packed == n::PackedMode::Bit
                    {
                        let (total_bits, restricted) =
                            nir::bit_packed_field_info(&world, &expr.ty, fuel).map_err(|err| {
                                format!("attrs.packed.bit BitConvert check failed: {err}")
                            })?;
                        if inner.ty != Term::Primitive(nir::Primitive::BitVec(total_bits as usize))
                        {
                            return Err(format!(
                                "BitVec conversion to attrs.packed.bit struct `{}` expects BitVec<{total_bits}>, found {}",
                                def.name, inner.ty
                            ));
                        }
                        if restricted && !unsafe_permitted {
                            return Err(
                                "BitVec conversion to attrs.packed.bit struct with restricted field representations requires an unsafe boundary".into(),
                            );
                        }
                    }
                    Ok(true)
                }
                _ => Ok(true),
            }
        })?;
    }
    Ok(())
}
