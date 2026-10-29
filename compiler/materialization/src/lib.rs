//! Compile-time closure of concrete function/interface dependencies. Templates
//! stay in NIR; only fixed selections are exported to the execution table.
#[cfg(test)]
mod callable_tests;
mod captured_templates;
mod conditional_calls;
mod dead_tails;
mod mutable_callbacks;
mod recursive_factories;
mod staged;
mod template_factories;
use crate::ptc::{Budget, Context};
use type_resolution::{associated, capabilities, ptc};
mod expansion;
pub use expansion::Expansion;
use nir::program::{self as n, ExprKind as K, Pattern as P};
use nir::{DeclarationId as Id, Term};
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use type_resolution::dispatch::DeclarationWorld;

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
struct Builder<'a> {
    expansion: Option<&'a dyn Expansion>,
    quote_cursor: u32,
    inline_effects: BTreeSet<Id>,
    active_staged_inlines: BTreeSet<Id>,
    quote_sources: Vec<n::ExpansionSource>,
    retain_templates: bool,
    package: &'a n::Package,
    world: DeclarationWorld<'a>,
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
    output: Vec<n::DispatchInstance>,
    fuel: usize,
    budget: Budget,
    instances: Option<Vec<iir::FunctionInstance>>,
    runtime_bindings: BTreeSet<nir::RuntimeId>,
    nominals: Vec<iir::NominalInstance>,
    interface_contracts: Vec<iir::InterfaceContract>,
    effect_contracts: Vec<iir::effects::Contract>,
    effect_instances: HashSet<nir::effects::Operation>,
}
/// Bounded, transactional: failure returns no partially materialized world.
pub fn dispatch_instances(
    package: &n::Package,
    budget: Budget,
) -> Result<Vec<n::DispatchInstance>, String> {
    build(
        package,
        budget,
        Roots::All,
        BTreeSet::new(),
        &[],
        None,
        None,
    )
    .map(|(dispatch, _, _, _, _, _, _)| dispatch)
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
        package,
        budget,
        Roots::Dispatch(roots),
        BTreeSet::new(),
        &[],
        None,
        None,
    )
    .map(|(dispatch, _, _, _, _, _, _)| dispatch)
}

/// Prepare reachable dispatch while retaining ordinary Type telescopes for
/// binding by the comptime interpreter at invocation time.
pub fn dispatch_instances_for_invocation(
    package: &n::Package,
    roots: &[iir::InstanceKey],
    budget: Budget,
) -> Result<Vec<n::DispatchInstance>, String> {
    build(
        package,
        budget,
        Roots::Invocation(roots),
        BTreeSet::new(),
        &[],
        None,
        None,
    )
    .map(|(dispatch, _, _, _, _, _, _)| dispatch)
}

/// Resolve one concrete method through the same checked dispatch closure used
/// by execution. Consumers must still enforce the interface's call contract;
/// the selected body identity alone is not permission to access memory.
pub fn resolve_method_instance(
    package: &n::Package,
    callee: &n::Expr,
    fuel: &mut usize,
) -> Result<n::MethodInstance, String> {
    let mut selected = None;
    let result = build(
        package,
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
        Ok((_, _, _, _, remaining, _, _)) => {
            *fuel = remaining;
            selected.ok_or_else(|| "method selection produced no body identity".into())
        }
        Err(reason) => {
            // Failed speculative selections cannot replenish their work budget.
            *fuel = 0;
            Err(reason)
        }
    }
}

/// Normalize annotations on a private expansion candidate without invoking
/// code, selecting bodies or publishing an executable instance. Unresolved
/// definition-side projections remain views; opaque heads retain identity.
struct InlineAnnotationRewriter<F>(F);
impl<F: FnMut(&Term) -> Result<Term, String>> n::Rewriter for InlineAnnotationRewriter<F> {
    type Error = String;
    fn ty(&mut self, ty: &Term) -> Result<Term, String> {
        (self.0)(ty)
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
        package,
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
        Ok((_, _, _, _, remaining, _, _)) => {
            *fuel = remaining;
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
    let mut selected = None;
    let result = build(
        package,
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
        Ok((_, _, _, _, remaining, _, _)) => {
            *fuel = remaining;
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
    live_methods: &[comptime_interpretor::TraitMethodIdentity],
    budget: Budget,
) -> Result<Vec<n::DispatchInstance>, String> {
    build(
        package,
        budget,
        Roots::AllWithMethods(live_methods),
        BTreeSet::new(),
        &[],
        Some((context, assumptions)),
        None,
    )
    .map(|(dispatch, _, _, _, _, _, _)| dispatch)
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
    nir::phases::validate(package).map_err(|e| format!("IIR input phase: {}", e.message))?;
    let (mut dispatch, functions, nominals, interfaces, _, expansion_sources, effect_contracts) =
        build(
            package,
            budget,
            Roots::Iir(roots),
            runtime_bindings.clone(),
            types,
            None,
            Some(expansion),
        )?;
    let mut template_fuel = budget.steps;
    let templates =
        captured_templates::storage(&functions, package, &dispatch, &mut template_fuel)?;
    // A selected interface may have conditionally available or simply unused
    // methods. Publish executable links only for demanded concrete bodies.
    let demanded: HashSet<_> = functions
        .iter()
        .map(|f| (&f.key.declaration, &f.key.arguments))
        .collect();
    for interface in &mut dispatch {
        interface.methods.retain(|m| {
            demanded.contains(&(&m.body, &m.arguments))
                || templates.iter().any(|table| {
                    table
                        .targets()
                        .iter()
                        .any(|key| key.declaration == m.body && key.arguments == m.arguments)
                })
        });
    }
    let output = iir::Package {
        target: package.target,
        expansion_sources,
        effect_contracts,
        roots: roots.to_vec(),
        functions,
        templates,
        dispatch,
        runtime_bindings,
        nominals,
        interfaces,
        normal_forms: package.normal_forms.clone(),
    };
    let mut validation_fuel = budget.steps;
    output.validate(&mut validation_fuel)?;
    Ok(output)
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
fn build(
    package: &n::Package,
    budget: Budget,
    roots: Roots<'_>,
    runtime_bindings: BTreeSet<nir::RuntimeId>,
    type_roots: &[nir::Term],
    context: Option<(&Context, &[n::TraitRequirement])>,
    expansion: Option<&dyn Expansion>,
) -> Result<
    (
        Vec<n::DispatchInstance>,
        Vec<iir::FunctionInstance>,
        Vec<iir::NominalInstance>,
        Vec<iir::InterfaceContract>,
        usize,
        Vec<n::ExpansionSource>,
        Vec<iir::effects::Contract>,
    ),
    String,
> {
    let quote_selection = matches!(roots, Roots::QuoteMethod(..));
    // NIR interpretation retains all unconditional concrete implementations.
    // Their parent/capability worlds must remain closed even when only a few
    // function bodies are reachable. IIR instead owns its reachable world.
    let retain_concrete_world = matches!(
        roots,
        Roots::All | Roots::AllWithMethods(_) | Roots::Dispatch(_) | Roots::Invocation(_)
    );
    let (world, mut input_fuel) = DeclarationWorld::new(package, budget, context)?;
    let inline_effects = if matches!(roots, Roots::Iir(_)) {
        input_fuel = input_fuel
            .checked_sub(package.effects.len())
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
        queue: VecDeque::new(),
        functions: HashSet::new(),
        staged_functions: HashMap::new(),
        generated_functions: HashMap::new(),
        recursive_factories: HashMap::new(),
        invocation_snapshot: None,
        types: HashSet::new(),
        staging_views: HashMap::new(),
        interfaces: HashMap::new(),
        output: vec![],
        fuel: input_fuel,
        budget,
        instances: matches!(roots, Roots::Iir(_)).then(Vec::new),
        runtime_bindings,
        nominals: vec![],
        interface_contracts: vec![],
        effect_contracts: vec![],
        effect_instances: HashSet::new(),
    };
    for ty in type_roots {
        let ty = builder.normalized_type(ty)?;
        builder.queue.push_back(Task::Type(ty));
    }
    if let Roots::Annotations(function) = roots {
        function.signature = builder.staging_type_view(&function.signature)?;
        for expression in std::iter::once(&mut function.body)
            .chain(function.defaults.iter_mut().map(|(_, value)| value))
            .chain(&mut function.preconditions)
            .chain(&mut function.postconditions)
        {
            expression.rewrite_with(&mut InlineAnnotationRewriter(|ty: &Term| {
                builder.staging_type_view(ty)
            }))?;
        }
    } else if let Roots::Dispatch(roots) | Roots::Invocation(roots) | Roots::Iir(roots) = roots {
        for root in roots {
            builder.tick(0)?;
            for argument in &root.arguments {
                crate::associated::charge_projection_copy(argument, &mut builder.fuel)
                    .map_err(|_| "dispatch root argument type budget exhausted".to_owned())?;
            }
            builder
                .queue
                .push_back(Task::Function(root.declaration, root.arguments.clone()));
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
        if !quote_selection {
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
    while let Some(task) = builder.queue.pop_front() {
        builder.tick(0)?;
        match task {
            Task::Function(id, arguments) => builder.function(id, arguments)?,
            Task::Interface(interface, subject) => {
                // A given obligation has no executable dispatch identity.
                // Method/value demands still call interface() directly.
                let given = crate::capabilities::World {
                    index: builder.world.index(),
                    implementations: &package.implementations,
                    traits: &package.traits,
                    assumptions: builder.world.assumptions(),
                    context: builder.world.context(),
                }
                .given(
                    &n::TraitRequirement {
                        interface: interface.declaration,
                        interface_arguments: interface.arguments.clone(),
                        subject: subject.clone(),
                    },
                    budget,
                )
                .is_some();
                if !given {
                    if builder.instances.is_some() {
                        let capability = builder.normalized_capability(&n::TraitRequirement {
                            interface: interface.declaration,
                            interface_arguments: interface.arguments,
                            subject,
                        })?;
                        builder.interface(capability.interface_instance(), capability.subject)?;
                    } else {
                        builder.interface(interface, subject)?;
                    }
                }
            }
            Task::Type(ty) => builder.ty(ty)?,
        }
    }
    Ok((
        builder.output,
        builder.instances.unwrap_or_default(),
        builder.nominals,
        builder.interface_contracts,
        builder.fuel,
        builder.quote_sources,
        builder.effect_contracts,
    ))
}
impl Builder<'_> {
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
        if let Some(identity) = self
            .package
            .runtime_function_identities
            .iter()
            .find(|f| f.declaration == id)
        {
            if self.instances.is_some() {
                return Err(
                    "runtime function identity has no executable definition for IIR".into(),
                );
            }
            let signature = identity
                .signature
                .instantiate_parameters(&self.input_binders(&identity.type_parameters), &arguments)
                .map_err(|e| e.to_string())?;
            self.queue.push_back(Task::Type(signature));
            return Ok(());
        }
        if !self.functions.insert((id, arguments.clone())) {
            return Ok(());
        }
        let f = self
            .package
            .functions
            .iter()
            .find(|f| f.declaration == id)
            .or_else(|| self.generated_functions.get(&id))
            .ok_or("missing dispatch function")?;
        if self.instances.is_some() && f.phase == n::FunctionPhase::NormalForm {
            return Err(
                "normal-form heads are type identities, not executable IIR functions".into(),
            );
        }
        if self.instances.is_some() {
            for argument in &arguments {
                iir::completed_type(argument, &self.runtime_bindings, &mut self.fuel)?;
                // An instance key retains its arguments even when projection
                // resolution erases their occurrence from the final signature.
                self.queue.push_back(Task::Type(argument.clone()));
            }
        }
        // Check the complete selected template before Clone or substitution
        // enters recursive owned trees. Later dependency traversal is too late
        // to enforce the depth bound safely. Include contracts and defaults,
        // and shared closure bodies that type rewriting may copy on write.
        // Ordinary templates charge substituted type occurrences as well:
        // bounding only the original tree misses wide generic expansions.
        let staged = self.staged_functions.get(&(id, arguments.clone()));
        let template = staged.unwrap_or(f);
        let parameters = self.input_binders(&f.type_parameters);
        let checked = nir::copy_budget::charge_function_substitution(
            template,
            if staged.is_some() { &[] } else { &parameters },
            if staged.is_some() { &[] } else { &arguments },
            &mut self.fuel,
        );
        checked.map_err(|reason| format!("dispatch template copy budget exhausted: {reason:?}"))?;
        let mut function = if let Some(staged) = self.staged_functions.get(&(id, arguments.clone()))
        {
            staged.clone()
        } else {
            if self.instances.is_some()
                && f.phase == n::FunctionPhase::Runtime
                && matches!(f.signature.callable_signature(),Term::Function {parameters,..} if parameters.iter().any(|p|p.sort!=nir::Sort::Runtime))
            {
                return Err("staged runtime function requires a specialized direct call".into());
            }
            f.instantiate_parameters(&parameters, &arguments)
                .map_err(|e| e.to_string())?
        };
        if self.instances.is_some() {
            function.signature = self.normalized_type(&function.signature)?;
            iir::completed_type(&function.signature, &self.runtime_bindings, &mut self.fuel)?;
        }
        self.queue.push_back(Task::Type(function.signature.clone()));
        if self.instances.is_none()
            && matches!(function.signature.callable_signature(),Term::Function {parameters,..}
                if parameters.iter().any(|parameter|parameter.sort==nir::Sort::Type))
        {
            if !self.retain_templates {
                return Err(
                    "requested dispatch function still requires ordinary Type input staging".into(),
                );
            }
            // An ordinary Type input is an invocation binder, not a closed
            // dispatch identity. Keep the template and its contracts in NIR;
            // input staging checks/materializes them after binding arguments.
            return Ok(());
        }
        if self.instances.is_none() {
            for requirement in &function.requirements {
                self.predicate_dependencies(requirement, 0)?;
            }
        }
        if self.instances.is_some() {
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
        }
        if self.instances.is_some() {
            for capability in &mut function.capabilities {
                *capability = self.normalized_capability(capability)?;
            }
        }
        for capability in &function.capabilities {
            self.queue.push_back(Task::Interface(
                capability.interface_instance(),
                capability.subject.clone(),
            ));
        }
        if self.instances.is_some() {
            iir::staging::forward_closed_locals_with_layouts(
                self.package,
                &mut function,
                &mut self.fuel,
            )?;
            self.normalize_expression(&mut function.body)?;
            for expression in function
                .preconditions
                .iter_mut()
                .chain(&mut function.postconditions)
                .chain(
                    function
                        .defaults
                        .iter_mut()
                        .map(|(_, expression)| expression),
                )
            {
                self.normalize_expression(expression)?;
            }
        }
        function.type_parameters.clear();
        if self.instances.is_some() {
            // Interface and body predicates may substitute the same staged
            // input expression. Each predicate executes at a distinct boundary,
            // even when those copies retain the same original source range.
            for predicate in function
                .preconditions
                .iter_mut()
                .chain(&mut function.postconditions)
            {
                nir::error_results::specialize::anchor_expression_copy(
                    predicate,
                    function.body.source,
                    &mut self.quote_sources,
                    &mut self.fuel,
                )?;
            }
            for expression in std::iter::once(&mut function.body)
                .chain(&mut function.preconditions)
                .chain(&mut function.postconditions)
                .chain(
                    function
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
        }
        self.expr(&function.body, 0)?;
        for expression in function
            .preconditions
            .iter()
            .chain(&function.postconditions)
            .chain(function.defaults.iter().map(|(_, e)| e))
        {
            self.expr(expression, 0)?;
        }
        if let Some(instances) = &mut self.instances {
            instances.push(iir::FunctionInstance {
                key: iir::InstanceKey {
                    declaration: id,
                    arguments,
                },
                function,
            });
        }
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
        for argument in &mut interface.arguments {
            let selected = self.normalized_interface_input(argument)?;
            if selected != *argument {
                self.queue.push_back(Task::Type(argument.clone()));
                *argument = selected;
            }
        }
        if let Some(index) = self.interfaces.get(&(interface.clone(), subject.clone())) {
            return Ok(*index);
        }
        let (id, implementation_arguments) =
            self.world
                .select_interface(&interface, &subject, self.budget)?;
        let implementation = self
            .package
            .implementations
            .iter()
            .find(|i| i.declaration == id)
            .ok_or("missing selected implementation")?;
        for capability in &implementation.capabilities {
            let capability = capability
                .instantiate_parameters(
                    &implementation
                        .parameter_binders()
                        .map_err(|e| e.to_string())?,
                    &implementation_arguments,
                )
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
        let parameter_binders = contract.binders_with_self(&mut self.fuel)?;
        for requirement in &contract.requirements {
            let requirement = requirement
                .instantiate_parameters(
                    &parameter_binders,
                    &std::iter::once(subject.clone())
                        .chain(interface.arguments.iter().cloned())
                        .collect::<Vec<_>>(),
                )
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
            let capability = capability
                .instantiate_parameters(
                    &parameter_binders,
                    &std::iter::once(subject.clone())
                        .chain(interface.arguments.iter().cloned())
                        .collect::<Vec<_>>(),
                )
                .map_err(|e| e.to_string())?;
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
            let parameters = &parameter_binders;
            let inputs: Vec<_> = std::iter::once(subject.clone())
                .chain(interface.arguments.iter().cloned())
                .collect();
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
                    .instantiate_parameters(parameters, &inputs)
                    .map_err(|e| e.to_string())?;
                method.program = method
                    .program
                    .instantiate_parameters(parameters, &inputs)
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
                    self.expr(expression, 0)?;
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
            let parameters = &parameter_binders;
            let arguments: Vec<_> = std::iter::once(subject.clone())
                .chain(interface.arguments.iter().cloned())
                .collect();
            self.queue.push_back(Task::Interface(
                parent
                    .instantiate_parameters(parameters, &arguments)
                    .map_err(|e| e.to_string())?,
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
        self.formation_requirements_with_parameters(&specialized.requirements, &[], &[])?;
        for capability in &specialized.capabilities {
            self.queue.push_back(Task::Interface(
                capability.interface_instance(),
                capability.subject.clone(),
            ));
        }
        self.output[slot].methods.push(method.clone());
        self.queue.push_back(Task::Type(
            specialized
                .signature
                .ordinary()
                .map_err(|e| e.to_string())?
                .clone(),
        ));
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
            self.expr(expression, 0)?;
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
            n::TypePredicate::Not(inner) => self.predicate_dependencies(inner, depth + 1)?,
            n::TypePredicate::And(items) | n::TypePredicate::Or(items) => {
                for item in items {
                    self.predicate_dependencies(item, depth + 1)?;
                }
            }
            n::TypePredicate::Bool(_) => {}
        }
        Ok(())
    }
    fn prove_requirement(
        &self,
        requirement: &n::TypePredicate,
    ) -> Result<crate::ptc::Outcome, String> {
        crate::capabilities::World {
            index: self.world.index(),
            implementations: &self.package.implementations,
            traits: &self.package.traits,
            assumptions: self.world.assumptions(),
            context: self.world.context(),
        }
        .check_predicate(requirement, self.budget)
    }
    fn formation_requirements_with_parameters(
        &mut self,
        requirements: &[n::TypePredicate],
        parameters: &[nir::DeclarationParameter],
        arguments: &[Term],
    ) -> Result<(), String> {
        if self.instances.is_none() {
            for requirement in requirements {
                self.predicate_dependencies(
                    &requirement
                        .instantiate_parameters(parameters, arguments)
                        .map_err(|e| e.to_string())?,
                    0,
                )?;
            }
            return Ok(());
        }
        for requirement in requirements {
            self.tick(0)?;
            let requirement = requirement
                .instantiate_parameters(parameters, arguments)
                .map_err(|e| e.to_string())?;
            let requirement = self.normalized_predicate(&requirement, 0)?;
            if !matches!(
                self.prove_requirement(&requirement)?,
                crate::ptc::Outcome::Entailed(_)
            ) {
                return Err(format!(
                    "IIR type formation requirement is not established: {requirement}"
                ));
            }
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
            self.normalize_expression(expression)?;
        }
        Ok(())
    }
    fn normalize_expression(&mut self, expression: &mut n::Expr) -> Result<(), String> {
        // Private working copies are discarded on any failure. Selected values
        // are substituted before dependency discovery, including nested patterns.
        if self.instances.is_some() {
            dead_tails::prune(expression, &mut self.fuel)?;
            // Expose runtime template factories before collecting local target
            // choices. Their argument frames and effects remain runtime code.
            let mut fuel = self.fuel;
            let result = iir::visit::expression_mut_pruned(expression, &mut fuel, |expr, fuel| {
                self.fuel = *fuel;
                let factory = matches!(&expr.kind, K::Call { callee, .. }
                    if matches!(callee.ty.callable_signature(), Term::Function { result, .. }
                        if matches!(result.callable_signature(), Term::Function { parameters, .. }
                            if parameters.iter().any(|p| p.sort != nir::Sort::Runtime))));
                let result = if factory {
                    self.stage_direct_call(expr)
                } else {
                    Ok(true)
                };
                *fuel = self.fuel;
                result
            });
            self.fuel = fuel.min(self.fuel);
            result?;
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
                    arguments: interface
                        .arguments
                        .iter()
                        .map(|ty| self.normalized_interface_input(ty))
                        .collect::<Result<_, _>>()?,
                };
                let subject = self.normalized_type(subject)?;
                let expected = self.normalized_type(&expr.ty)?;
                let (selected, remaining) = crate::capabilities::World {
                    index: self.world.index(),
                    implementations: &self.package.implementations,
                    traits: &self.package.traits,
                    assumptions: self.world.assumptions(),
                    context: self.world.context(),
                }
                .resolve_associated_value_plan_budgeted(
                    &interface,
                    *member,
                    &subject,
                    Budget {
                        steps: self.fuel,
                        ..self.budget
                    },
                )?;
                self.fuel = remaining;
                if selected.value.ty != expected {
                    return Err("associated expression type disagrees with selected member".into());
                }
                let source = expr.source;
                *expr = if selected.requires_invocation {
                    self.invoke_staged_input(&selected.value)?
                } else {
                    selected.value
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
        expression.rewrite_types(&mut |ty| self.staging_type_view(ty))?;
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
            self.expansion
                .ok_or("IIR materialization lacks an expansion service")?
                .quotes(
                    self.package,
                    expression,
                    &mut self.quote_cursor,
                    &mut self.quote_sources,
                    &mut generated,
                    &mut self.fuel,
                )?;
            self.register_quote_layouts(generated)?;
        }
        if self.instances.is_some() {
            let mut fuel = self.fuel;
            let result = iir::visit::expression_mut_pruned(expression, &mut fuel, |expr, fuel| {
                self.fuel = *fuel;
                let result = (|| {
                    if let Some(plan) =
                        iir::staging::preparation_with_layouts(self.package, expr, &mut self.fuel)?
                    {
                        let completed = self.invoke_staged_input(&plan.expression)?;
                        plan.apply_with_layouts(self.package, expr, completed, &mut self.fuel)?;
                    }
                    let mut remaining = self.fuel;
                    let completed = conditional_calls::complete_inputs(
                        expr,
                        self,
                        &mut remaining,
                        |this, input, fuel| {
                            this.fuel = *fuel;
                            let result = this.invoke_staged_input(input);
                            *fuel = this.fuel;
                            result
                        },
                    );
                    self.fuel = remaining.min(self.fuel);
                    completed?;
                    Self::resolve_conditional_callable(
                        expr,
                        !self.active_staged_inlines.is_empty(),
                        &mut self.quote_cursor,
                        &mut self.quote_sources,
                        &mut self.fuel,
                    )?;
                    self.stage_direct_call(expr)
                })();
                *fuel = self.fuel;
                result
            });
            self.fuel = fuel.min(self.fuel);
            result?;
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
                    self.package,
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
        function.preconditions.extend(program.preconditions);
        function.postconditions.extend(program.postconditions);
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
                    let selected = resolve_method_instance(self.package, callee, &mut self.fuel)?;
                    let template = self
                        .package
                        .functions
                        .iter()
                        .find(|function| function.declaration == selected.body)
                        .ok_or("missing selected inline method body")?;
                    if !template.inline {
                        return Ok(true);
                    }
                    let mut bound = template
                        .instantiate_parameters(
                            &self.input_binders(&template.type_parameters),
                            &selected.arguments,
                        )
                        .map_err(|e| e.to_string())?;
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
                        .binders_with_self(&mut self.fuel)?
                        .into_iter()
                        .chain(self.input_binders(&contract.type_parameters))
                        .collect::<Vec<_>>();
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
                    let requirements = contract
                        .requirements
                        .iter()
                        .map(|r| r.instantiate_parameters_in_scope(&parameters, &inputs, &frames))
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|e| e.to_string())?;
                    let capabilities = contract
                        .capabilities
                        .iter()
                        .map(|r| r.instantiate_parameters_in_scope(&parameters, &inputs, &frames))
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|e| e.to_string())?;
                    let mut program = contract
                        .program
                        .instantiate_parameters(&parameters, &inputs)
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
            let mut bound = template
                .instantiate_parameters(&self.input_binders(&template.type_parameters), &arguments)
                .map_err(|e| e.to_string())?;
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
        let mut template = template
            .instantiate_parameters(&self.input_binders(&template.type_parameters), &arguments)
            .map_err(|e| e.to_string())?;
        template.type_parameters.clear();
        if let Some((requirements, capabilities, program)) = entry_contract {
            Self::merge_entry_contract(&mut template, requirements, capabilities, program);
        }
        template.signature = self.staging_type_view(&template.signature)?;
        let mut fuel = self.fuel;
        let result = iir::staging::bind_call_with_layouts_and_type_views(
            self.package,
            &template,
            &arguments,
            expression,
            &mut fuel,
            self,
            |this, input, fuel| {
                let mut invokes = false;
                iir::visit::expression(input, fuel, |expr, _| {
                    invokes |= matches!(
                        expr.kind,
                        K::Call { .. }
                            | K::NormalFormType { .. }
                            | K::Assign { .. }
                            | K::Loop { .. }
                            | K::Break(_)
                            | K::Continue(_)
                            | K::CheckPredicate(_)
                            | K::Match { .. }
                            | K::LetPattern { .. }
                            | K::Field { .. }
                            | K::AnonymousField { .. }
                            | K::CallPreparation { .. }
                            | K::Closure { .. }
                    );
                    invokes |= matches!(
                        expr.kind,
                        K::EnumName(_)
                            | K::TextQuery { subject: _, .. }
                            | K::IntegerRepresentation(_)
                            | K::ComptimeIntegerConvert(_)
                            | K::SymbolName(_)
                            | K::TakeBits(_)
                            | K::BitConvert(_)
                            | K::ErrorConvert { value: _, .. }
                            | K::NewtypeConvert(_)
                            | K::BitSlice { .. }
                            | K::BitEqual(..)
                            | K::BitWritePointer { .. }
                            | K::BitUpdate { .. }
                            | K::BitWindow { .. }
                            | K::BitIndex(..)
                            | K::BitConcat(..)
                            | K::BitAssign { .. }
                    ) || (matches!(expr.kind, K::Not(_) | K::And(..) | K::Or(..))
                        && matches!(expr.ty, Term::Primitive(nir::Primitive::BitVec(_))));
                    invokes |= matches!(expr.kind, K::IntegerNegate(_) | K::IntegerBinary { .. })
                        && expr.ty == Term::Primitive(nir::Primitive::Integer);
                    // Mathematical Integer comparisons produce Bool, so their
                    // execution domain cannot be selected from result type alone.
                    invokes |= matches!(&expr.kind, K::IntegerBinary {op,..} if op.is_comparison());
                    invokes |= matches!(expr.kind, K::FloatNegate(_) | K::FloatBinary { .. });
                    invokes |= matches!(&expr.kind,
                    K::ScalarEqual(left,_) | K::ScalarLess(left,_) | K::ScalarLessEqual(left,_)
                    | K::ScalarGreater(left,_) | K::ScalarGreaterEqual(left,_)
                        if matches!(left.ty,Term::Primitive(nir::Primitive::F32 | nir::Primitive::F64)));
                    Ok(())
                })?;
                this.fuel = *fuel;
                let result = if invokes {
                    this.invoke_staged_input(input)
                } else {
                    iir::staging::fold_value_with_layouts(this.package, input, &mut this.fuel)
                };
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
        if quote {
            self.check_expanded_requirements(&instance.function, "quote")?;
            let mut generated = Vec::new();
            self.expansion
                .ok_or("IIR materialization lacks an expansion service")?
                .quote(
                    self.package,
                    &instance.function,
                    expression,
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
                .entry((instance.key.declaration, instance.key.arguments))
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
        if !self
            .expansion
            .ok_or("IIR materialization lacks an expansion service")?
            .inline(
                self.package,
                function,
                expression,
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

    fn normalized_capability(
        &mut self,
        capability: &n::TraitRequirement,
    ) -> Result<n::TraitRequirement, String> {
        Ok(n::TraitRequirement {
            interface: capability.interface,
            subject: self.normalized_type(&capability.subject)?,
            interface_arguments: capability
                .interface_arguments
                .iter()
                .map(|argument| self.normalized_interface_input(argument))
                .collect::<Result<_, _>>()?,
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
        nir::validate_declaration_arguments_with_layouts_in(
            &[*declaration],
            std::slice::from_ref(ty.as_ref()),
            std::slice::from_ref(input),
            &mut self.fuel,
            |ty, variant, fuel| {
                nir::normal_forms::nominal_value_fields_budgeted(self.package, ty, variant, fuel)
                    .ok_or_else(|| {
                        if *fuel == 0 {
                            nir::NominalAdmissionError::Budget
                        } else {
                            nir::NominalAdmissionError::from("invalid nominal value layout")
                        }
                    })
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
        Err("interface value input normalization requires domain evidence".into())
    }
    fn normalized_type(&mut self, ty: &Term) -> Result<Term, String> {
        self.world.normalized_type(ty, self.budget, &mut self.fuel)
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
        let (view, remaining) = crate::capabilities::World {
            index: self.world.index(),
            implementations: &self.package.implementations,
            traits: &self.package.traits,
            assumptions: self.world.assumptions(),
            context: self.world.context(),
        }
        .associated_type_view_cached(
            ty,
            Budget {
                steps: self.fuel,
                ..self.budget
            },
            &mut self.staging_views,
        )?;
        self.fuel = remaining;
        Ok(view.value)
    }
    fn ty(&mut self, ty: Term) -> Result<(), String> {
        if !self.types.insert(ty.clone()) {
            return Ok(());
        }
        if self.instances.is_none()
            && matches!(&ty,Term::Function {parameters,..}
                if parameters.iter().any(|parameter|parameter.sort==nir::Sort::Type))
        {
            // Do not enqueue a dependent child outside the telescope that
            // binds it. Validate the complete retained callable view instead;
            // concrete dependency closure runs after invocation input staging.
            self.staging_type_view(&ty)?;
            return Ok(());
        }
        if self.instances.is_some() {
            iir::completed_dependencies(&ty, &self.runtime_bindings, &mut self.fuel)?;
        } else if let Term::Nominal { arguments, .. } = &ty {
            let selected = self.normalized_type(&ty)?;
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
            Term::Nominal {
                declaration,
                arguments,
            } => {
                children.extend(arguments.iter().cloned());
                if let Some(s) = self
                    .package
                    .structs
                    .iter()
                    .find(|s| s.declaration == *declaration)
                {
                    if self.instances.is_some() {
                        nir::validate_declaration_arguments_with_layouts_in(
                            &s.type_parameters,
                            &s.parameter_types,
                            arguments,
                            &mut self.fuel,
                            |ty, variant, fuel| {
                                nir::normal_forms::nominal_value_fields_budgeted(
                                    self.package,
                                    ty,
                                    variant,
                                    fuel,
                                )
                                .ok_or_else(|| {
                                    if *fuel == 0 {
                                        nir::NominalAdmissionError::Budget
                                    } else {
                                        nir::NominalAdmissionError::Invalid(
                                            "nominal value layout unavailable".into(),
                                        )
                                    }
                                })
                            },
                            self.package.target,
                        )?;
                    }
                    let parameters = s.parameter_binders().map_err(|e| e.to_string())?;
                    self.formation_requirements_with_parameters(
                        &s.requirements,
                        &parameters,
                        arguments,
                    )?;
                    let mut fields = s.instantiate_fields(arguments).map_err(|e| e.to_string())?;
                    if self.instances.is_some() {
                        for (_, field) in &mut fields {
                            *field = self.normalized_type(field)?;
                        }
                        let mut definition = s.clone();
                        definition.fields = fields.clone();
                        definition.requirements = s
                            .requirements
                            .iter()
                            .map(|r| r.instantiate_parameters(&parameters, arguments))
                            .collect::<Result<_, _>>()
                            .map_err(|e| e.to_string())?;
                        definition.capabilities = s
                            .capabilities
                            .iter()
                            .map(|c| c.instantiate_parameters(&parameters, arguments))
                            .collect::<Result<_, _>>()
                            .map_err(|e| e.to_string())?;
                        for capability in &mut definition.capabilities {
                            *capability = self.normalized_capability(capability)?;
                        }
                        for requirement in &mut definition.requirements {
                            *requirement = self.normalized_predicate(requirement, 0)?;
                        }
                        definition.type_parameters.clear();
                        definition.parameter_types.clear();
                        self.nominals.push(iir::NominalInstance {
                            ty: ty.clone(),
                            definition: iir::NominalDefinition::Struct(definition),
                        });
                    }

                    for cap in &s.capabilities {
                        let cap = cap
                            .instantiate_parameters(&parameters, arguments)
                            .map_err(|e| e.to_string())?;
                        self.queue
                            .push_back(Task::Interface(cap.interface_instance(), cap.subject));
                    }
                    children.extend(fields.into_iter().map(|(_, ty)| ty));
                } else if let Some(e) = self
                    .package
                    .enums
                    .iter()
                    .find(|e| e.declaration == *declaration)
                {
                    if self.instances.is_some() {
                        nir::validate_declaration_arguments_with_layouts_in(
                            &e.type_parameters,
                            &e.parameter_types,
                            arguments,
                            &mut self.fuel,
                            |ty, variant, fuel| {
                                nir::normal_forms::nominal_value_fields_budgeted(
                                    self.package,
                                    ty,
                                    variant,
                                    fuel,
                                )
                                .ok_or_else(|| {
                                    if *fuel == 0 {
                                        nir::NominalAdmissionError::Budget
                                    } else {
                                        nir::NominalAdmissionError::Invalid(
                                            "nominal value layout unavailable".into(),
                                        )
                                    }
                                })
                            },
                            self.package.target,
                        )?;
                    }
                    let parameters = e.parameter_binders().map_err(|e| e.to_string())?;
                    self.formation_requirements_with_parameters(
                        &e.requirements,
                        &parameters,
                        arguments,
                    )?;
                    let mut variants = e
                        .instantiate_variants(arguments)
                        .map_err(|e| e.to_string())?;
                    if self.instances.is_some() {
                        for variant in &mut variants {
                            for field in &mut variant.fields {
                                *field = self.normalized_type(field)?;
                            }
                        }
                        let mut definition = e.clone();
                        definition.variants = variants.clone();
                        definition.requirements = e
                            .requirements
                            .iter()
                            .map(|r| r.instantiate_parameters(&parameters, arguments))
                            .collect::<Result<_, _>>()
                            .map_err(|e| e.to_string())?;
                        definition.capabilities = e
                            .capabilities
                            .iter()
                            .map(|c| c.instantiate_parameters(&parameters, arguments))
                            .collect::<Result<_, _>>()
                            .map_err(|e| e.to_string())?;
                        for capability in &mut definition.capabilities {
                            *capability = self.normalized_capability(capability)?;
                        }
                        for requirement in &mut definition.requirements {
                            *requirement = self.normalized_predicate(requirement, 0)?;
                        }
                        definition.type_parameters.clear();
                        definition.parameter_types.clear();
                        self.nominals.push(iir::NominalInstance {
                            ty: ty.clone(),
                            definition: iir::NominalDefinition::Enum(definition),
                        });
                    }

                    for cap in &e.capabilities {
                        let cap = cap
                            .instantiate_parameters(&parameters, arguments)
                            .map_err(|e| e.to_string())?;
                        self.queue
                            .push_back(Task::Interface(cap.interface_instance(), cap.subject));
                    }
                    children.extend(variants.into_iter().flat_map(|v| v.fields));
                } else if let Some(n) = self
                    .package
                    .newtypes
                    .iter()
                    .find(|n| n.declaration == *declaration)
                {
                    // Dispatch preparation retains generic identities. Closed
                    // value admission belongs to concrete materialization, as
                    // for struct and enum; invocation checks scoped evidence.
                    if self.instances.is_some() {
                        n.validate_completed_arguments_with_layouts_in(
                            arguments,
                            &mut self.fuel,
                            |ty, variant, fuel| {
                                nir::normal_forms::nominal_value_fields_budgeted(
                                    self.package,
                                    ty,
                                    variant,
                                    fuel,
                                )
                                .ok_or_else(|| {
                                    if *fuel == 0 {
                                        nir::NominalAdmissionError::Budget
                                    } else {
                                        nir::NominalAdmissionError::Invalid(
                                            "nominal value layout unavailable".into(),
                                        )
                                    }
                                })
                            },
                            self.package.target,
                        )?;
                    }
                    let parameters = n.parameter_binders().map_err(|e| e.to_string())?;
                    self.formation_requirements_with_parameters(
                        &n.requirements,
                        &parameters,
                        arguments,
                    )?;
                    let representation = n
                        .representation_budgeted(arguments, &[], &mut self.fuel)
                        .map_err(|e| e.to_string())?;
                    if self.instances.is_some() {
                        let mut definition = n.clone();
                        definition.representation = self.normalized_type(&representation)?;
                        definition.requirements = n
                            .requirements
                            .iter()
                            .map(|r| r.instantiate_parameters(&parameters, arguments))
                            .collect::<Result<_, _>>()
                            .map_err(|e| e.to_string())?;
                        definition.capabilities = n
                            .capabilities
                            .iter()
                            .map(|c| c.instantiate_parameters(&parameters, arguments))
                            .collect::<Result<_, _>>()
                            .map_err(|e| e.to_string())?;
                        for capability in &mut definition.capabilities {
                            *capability = self.normalized_capability(capability)?;
                        }
                        for requirement in &mut definition.requirements {
                            *requirement = self.normalized_predicate(requirement, 0)?;
                        }
                        definition.type_parameters.clear();
                        definition.parameter_types.clear();
                        definition.capture_count = 0;
                        self.nominals.push(iir::NominalInstance {
                            ty: ty.clone(),
                            definition: iir::NominalDefinition::Newtype(definition),
                        });
                    }
                    for cap in &n.capabilities {
                        let cap = cap
                            .instantiate_parameters(&parameters, arguments)
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
                    descriptor
                        .check_arguments_in_scope_with_collections_budgeted_in(
                            arguments,
                            |head| {
                                self.package.normal_forms.iter().any(|metadata| {
                                    metadata.declaration == head
                                        && metadata.role == nir::normal_forms::Role::Tuple
                                })
                            },
                            |head| {
                                self.package.normal_forms.iter().any(|metadata| {
                                    metadata.declaration == head
                                        && metadata.role == nir::normal_forms::Role::TypeList
                                })
                            },
                            |ty, variant, fuel| {
                                nir::normal_forms::nominal_value_fields_budgeted(
                                    self.package,
                                    ty,
                                    variant,
                                    fuel,
                                )
                            },
                            |_, _| false,
                            &[],
                            &mut self.fuel,
                            self.package.target,
                        )
                        .map_err(|e| format!("IIR normal-form inputs: {e}"))?;
                    let parameters: Vec<_> = descriptor
                        .parameters
                        .iter()
                        .map(|p| nir::DeclarationParameter {
                            declaration: p.declaration,
                            sort: match p.kind {
                                nir::normal_forms::Kind::Type => nir::Sort::Type,
                                nir::normal_forms::Kind::Value => nir::Sort::ComptimeValue,
                            },
                        })
                        .collect();
                    self.formation_requirements_with_parameters(
                        &descriptor.requirements,
                        &parameters,
                        arguments,
                    )?;
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
                // Prove the complete projection (including cycle checks), then
                // retain its originating Type-valued member for reference execution.
                let normalized = self.normalized_type(&ty)?;
                let interface = n::InterfaceInstance {
                    declaration: origin.interface,
                    arguments: arguments.clone(),
                };
                let slot = self.interface(interface.clone(), (**subject).clone())?;
                let selected_interface = self.output[slot].interface_instance();
                let selected_subject = self.output[slot].subject.clone();
                if !self.output[slot]
                    .associated
                    .iter()
                    .any(|(member, _)| *member == origin.member)
                {
                    let (selected, remaining) = crate::capabilities::World {
                        index: self.world.index(),
                        implementations: &self.package.implementations,
                        traits: &self.package.traits,
                        assumptions: self.world.assumptions(),
                        context: self.world.context(),
                    }
                    .resolve_associated_value_plan_budgeted(
                        &selected_interface,
                        origin.member,
                        &selected_subject,
                        Budget {
                            steps: self.fuel,
                            ..self.budget
                        },
                    )?;
                    self.fuel = remaining;
                    self.output[slot]
                        .associated
                        .push((origin.member, selected.value.clone()));
                    self.expr(&selected.value, 0)?;
                }
                children.push(normalized);
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
                            .signature
                            .instantiate_parameters(
                                &declaration.parameter_binders().map_err(|e| e.to_string())?,
                                &operation.arguments,
                            )
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
            Term::Tuple(items)
            | Term::TypeListValue(items)
            | Term::TypeListConcat(items)
            | Term::TupleValue(items) => children.extend(items.iter().cloned()),
            Term::OptionalValue { ty, value } => {
                children.push((**ty).clone());
                if let Some(value) = value {
                    children.push((**value).clone());
                }
            }
            Term::ErrorValue { ty, label, value } => {
                children.push((**ty).clone());
                if let Some(label) = label {
                    children.push((**label).clone());
                }
                children.push((**value).clone());
            }
            Term::NominalValue { ty, fields, .. } => {
                children.push((**ty).clone());
                children.extend(fields.iter().cloned());
            }
            Term::Record(fields) | Term::RecordValue(fields) => {
                children.extend(fields.iter().map(|(_, ty)| ty.clone()))
            }
            Term::Optional(t)
            | Term::Pointer(t)
            | Term::ComptimeFunction(t)
            | Term::NormalFormFunction(t, _)
            | Term::UnsafeFunction(t) => children.push((**t).clone()),
            Term::Function { parameters, result } => {
                children.extend(parameters.iter().map(|p| p.ty.clone()));
                children.push((**result).clone());
            }
            Term::Closure {
                environment,
                signature,
            } => {
                children.extend(environment.iter().cloned());
                children.push((**signature).clone());
            }
            _ => {}
        }
        self.queue.extend(children.into_iter().map(Task::Type));
        Ok(())
    }
    fn pattern(&mut self, pattern: &P, depth: usize) -> Result<(), String> {
        self.tick(depth)?;
        match pattern {
            P::ErrorResult { label, inner } => {
                if let Some(label) = label {
                    self.queue.push_back(Task::Type(label.clone()));
                }
                self.pattern(inner, depth + 1)?;
            }
            P::CodeEvidence { .. } => return Err("unresolved pattern-code evidence".into()),
            P::Then { first, value, next } => {
                self.pattern(first, depth + 1)?;
                self.expr(value, depth + 1)?;
                self.pattern(next, depth + 1)?;
            }
            P::Guard { pattern, condition } => {
                self.pattern(pattern, depth + 1)?;
                self.expr(condition, depth + 1)?;
            }
            P::OptionalPresent(pattern)
            | P::Observe { inner: pattern, .. }
            | P::Remap { pattern, .. }
            | P::Binding { inner: pattern, .. } => self.pattern(pattern, depth + 1)?,
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
                    self.pattern(p, depth + 1)?;
                }
            }
            P::OptionalAbsent | P::Wild | P::Unit | P::Literal(_) => {}
        }
        Ok(())
    }
    fn expr(&mut self, expr: &n::Expr, depth: usize) -> Result<(), String> {
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
                | K::ReadPointer(value)
                | K::PointerAnonymousField { pointer: value, .. }
                | K::PointerField { pointer: value, .. }
                | K::OptionalPresent(value)
                | K::OptionalType(value)
                | K::PointerType(value)
                | K::ListLength(value)
                | K::StructFieldTypes(value)
                | K::BuildStruct(value)
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
                | K::ComptimeIntegerConvert(value)
                | K::SymbolName(value)
                | K::EnumName(value)
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
                    self.expr_node(expr, depth)?;
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
    fn expr_node(&mut self, expr: &n::Expr, depth: usize) -> Result<(), String> {
        self.tick(depth)?;
        if self.instances.is_some() {
            iir::completed_type(&expr.ty, &self.runtime_bindings, &mut self.fuel)?;
        }
        self.queue.push_back(Task::Type(expr.ty.clone()));
        match &expr.kind {
            K::Intrinsic { tag, inputs, .. } => {
                nir::intrinsics::checked(expr).map_err(str::to_owned)?;
                self.queue.push_back(Task::Type(Term::Nominal {
                    declaration: tag.family,
                    arguments: vec![],
                }));
                for input in inputs {
                    self.expr(input, depth + 1)?;
                }
            }
            K::EffectHandle {
                computation,
                handlers,
                outer,
            } => {
                self.expr(computation, depth + 1)?;
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
                        self.pattern(&arm.pattern, depth + 1)?;
                        if let Some(g) = &arm.guard {
                            self.expr(g, depth + 1)?;
                        }
                        self.expr(&arm.body, depth + 1)?;
                    }
                }
            }
            K::EffectResume(computation) | K::EffectRun { computation, .. } => {
                self.expr(computation, depth + 1)?;
            }
            K::EffectComputation { effects, body } => {
                for operation in effects {
                    for argument in &operation.arguments {
                        self.queue.push_back(Task::Type(argument.clone()));
                    }
                }
                self.expr(body, depth + 1)?;
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
                    self.expr(argument, depth + 1)?;
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
                let interface = self.output[slot].interface_instance();
                let subject = self.output[slot].subject.clone();
                if !self.output[slot]
                    .associated
                    .iter()
                    .any(|(id, _)| id == member)
                    && !self.output[slot]
                        .associated_initializers
                        .iter()
                        .any(|(id, _)| id == member)
                {
                    let (selected, remaining) = crate::capabilities::World {
                        index: self.world.index(),
                        implementations: &self.package.implementations,
                        traits: &self.package.traits,
                        assumptions: self.world.assumptions(),
                        context: self.world.context(),
                    }
                    .resolve_associated_value_plan_budgeted(
                        &interface,
                        *member,
                        &subject,
                        Budget {
                            steps: self.fuel,
                            ..self.budget
                        },
                    )?;
                    self.fuel = remaining;
                    // Publish the demand before traversing dependencies; recursive
                    // initialization is bounded by the comptime interpreter.
                    if selected.requires_invocation {
                        let mut copy_fuel = self.fuel as u64;
                        let copied =
                            nir::copy_budget::charge_expression(&selected.value, &mut copy_fuel);
                        self.fuel = copy_fuel as usize;
                        copied.map_err(|_| "associated initializer copy budget exhausted")?;
                        let template = nir::associated_initializers::AssociatedInitializer::new(
                            selected.value.clone(),
                            &mut self.fuel,
                        )?;
                        self.output[slot]
                            .associated_initializers
                            .push((*member, template));
                        self.expr(&selected.value, depth + 1)?;
                    } else {
                        self.expr(&selected.value, depth + 1)?;
                        self.output[slot].associated.push((*member, selected.value));
                    }
                }
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
                    .callable_type()
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
                subject: packet,
            } => {
                self.expr(packet, depth + 1)?;
                let Some((interface, member)) = self.package.traits.iter().find_map(|interface| {
                    interface
                        .methods
                        .iter()
                        .find(|member| member.declaration == *declaration)
                        .map(|member| (interface, member))
                }) else {
                    return Ok(());
                };
                if self.instances.is_some() {
                    return Err("dynamic trait clauses require comptime invocation before IIR instantiation".into());
                }
                let values = match &packet.kind {
                    K::Block {
                        result: Some(result),
                        ..
                    } => result.as_ref(),
                    _ => packet.as_ref(),
                };
                let K::Tuple(values) = &values.kind else {
                    return Err("dynamic trait clause packet has no input tuple".into());
                };
                let prefix_count = 1 + interface.type_parameters.len();
                if values.len() != prefix_count + member.type_parameters.len() {
                    return Err("dynamic trait clause packet arity mismatch".into());
                }
                let prefix = values[..prefix_count]
                    .iter()
                    .map(|value| match &value.kind {
                        K::TypeValue(value) | K::ComptimeValue(value) => Ok(value.clone()),
                        _ => Err(
                            "dynamic trait dispatch requires known Self and interface inputs"
                                .to_owned(),
                        ),
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
                        self.expr(value, depth + 1)?;
                    }
                }
                self.expr(body, depth + 1)?;
            }
            K::NormalFormType {
                arguments, plan, ..
            } => {
                for p in plan.iter().flat_map(|p| &p.preparations) {
                    self.expr(&p.expression, depth + 1)?;
                }
                for pattern in arguments.iter().flat_map(|a| a.patterns()) {
                    self.pattern(pattern, depth + 1)?;
                }
                for e in arguments.iter().flat_map(|a| a.expressions()) {
                    self.expr(e, depth + 1)?;
                }
            }
            K::NominalType {
                arguments,
                preparations,
                ..
            } => {
                for preparation in preparations {
                    self.expr(&preparation.expression, depth + 1)?;
                }
                for pattern in arguments.iter().flat_map(|a| a.patterns()) {
                    self.pattern(pattern, depth + 1)?;
                }
                for value in arguments.iter().flat_map(|a| a.expressions()) {
                    self.expr(value, depth + 1)?;
                }
            }
            K::ClosureType {
                environment,
                signature,
            } => {
                for field in environment {
                    self.expr(field, depth + 1)?;
                }
                self.expr(signature, depth + 1)?;
            }
            K::CheckTypeAlias {
                arguments,
                body,
                preparations,
                ..
            } => {
                for preparation in preparations {
                    self.expr(&preparation.expression, depth + 1)?;
                }
                for argument in arguments {
                    for pattern in argument.patterns() {
                        self.pattern(pattern, depth + 1)?;
                    }
                    for expression in argument.expressions() {
                        self.expr(expression, depth + 1)?;
                    }
                }
                if let Some(body) = body {
                    self.expr(body, depth + 1)?;
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
            | K::TupleType(fields)
            | K::Tuple(fields) => {
                for e in fields {
                    self.expr(e, depth + 1)?;
                }
            }
            K::BitWritePointer {
                pointer,
                selections,
                value,
            } => {
                self.expr(pointer, depth + 1)?;
                for input in selections
                    .iter()
                    .flat_map(nir::program::BitSelection::expressions)
                {
                    self.expr(input, depth + 1)?;
                }
                self.expr(value, depth + 1)?;
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
                    self.expr(input, depth + 1)?;
                }
                self.expr(value, depth + 1)?;
            }
            K::BitAssign {
                storage_type,
                value,
                ..
            } => {
                self.queue.push_back(Task::Type(storage_type.clone()));
                self.expr(value, depth + 1)?;
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
            | K::ReadPointer(value)
            | K::PointerAnonymousField { pointer: value, .. }
            | K::PointerField { pointer: value, .. }
            | K::OptionalPresent(value)
            | K::OptionalType(value)
            | K::PointerType(value)
            | K::ListLength(value)
            | K::StructFieldTypes(value)
            | K::BuildStruct(value)
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
            | K::ComptimeIntegerConvert(value)
            | K::SymbolName(value)
            | K::EnumName(value)
            | K::Not(value)
            | K::Loop { body: value, .. } => self.expr(value, depth + 1)?,
            K::BitWindow { value, start, end } => {
                self.expr(value, depth + 1)?;
                self.expr(start, depth + 1)?;
                self.expr(end, depth + 1)?;
            }
            K::Match { value, arms } => {
                self.expr(value, depth + 1)?;
                for arm in arms {
                    self.pattern(&arm.pattern, depth + 1)?;
                    if let Some(g) = &arm.guard {
                        self.expr(g, depth + 1)?;
                    }
                    self.expr(&arm.body, depth + 1)?;
                }
            }
            K::Block { statements, result } | K::CallPreparation { statements, result } => {
                for e in statements {
                    self.expr(e, depth + 1)?;
                }
                if let Some(e) = result {
                    self.expr(e, depth + 1)?;
                }
            }
            K::LetPattern {
                pattern,
                value,
                guard,
                otherwise,
            } => {
                self.pattern(pattern, depth + 1)?;
                self.expr(value, depth + 1)?;
                if let Some(e) = guard {
                    self.expr(e, depth + 1)?;
                }
                if let Some(e) = otherwise {
                    self.expr(e, depth + 1)?;
                }
            }
            K::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.expr(condition, depth + 1)?;
                self.expr(then_branch, depth + 1)?;
                if let Some(e) = else_branch {
                    self.expr(e, depth + 1)?;
                }
            }
            K::Return(value) => {
                if let Some(e) = value {
                    self.expr(e, depth + 1)?;
                }
            }
            K::Call {
                callee, arguments, ..
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
                self.expr(callee, depth + 1)?;
                for a in arguments {
                    self.expr(&a.value, depth + 1)?;
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
                self.expr(a, depth + 1)?;
                self.expr(b, depth + 1)?;
            }
            K::BuildMetadata {
                selector, entries, ..
            } => {
                if self.instances.is_some() {
                    return Err("IIR requires package metadata materialization".into());
                }
                self.expr(selector, depth + 1)?;
                for (_, _, value) in entries {
                    if let Some(value) = value {
                        self.queue.push_back(Task::Type(value.clone()));
                    }
                }
            }
            K::ComptimeValue(ty) | K::TypeValue(ty) => {
                if self.instances.is_some() {
                    iir::completed_type(ty, &self.runtime_bindings, &mut self.fuel)?;
                }
                self.queue.push_back(Task::Type(ty.clone()));
            }
            K::TypeListValue(types) => {
                for ty in types {
                    self.tick(depth + 1)?;
                    if self.instances.is_some() {
                        iir::completed_type(ty, &self.runtime_bindings, &mut self.fuel)?;
                    }
                    self.queue.push_back(Task::Type(ty.clone()));
                }
            }
            K::Literal(_)
            | K::Local(_)
            | K::OptionalAbsent
            | K::Unit
            | K::Break(_)
            | K::Continue(_) => {}
        }
        Ok(())
    }
}
