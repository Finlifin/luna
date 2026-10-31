//! Explicit, bounded invocation of completed comptime declarations.
pub use crate::memoization::Invocation as MemoizedInvocation;
use comptime_interpretor::{Budget, Interpreter, Value};
use nir::program::{FunctionPhase, Package};
use nir::{DeclarationId, Term};

/// Reusable preparation for one entry in an immutable compilation snapshot.
/// Code is shared; values, mutable storage and evidence are fresh on each call.
pub struct PreparedInvocation {
    package: std::sync::Arc<Package>,
    graph: Option<std::sync::Arc<iir::Package>>,
    declaration: DeclarationId,
    entry: String,
    code: comptime_interpretor::CodeSession,
    last_stats: InvocationStats,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct InvocationStats {
    pub bytecode_compilations: usize,
    pub bytecode_invocations: usize,
}
impl PreparedInvocation {
    pub fn new(
        package: std::sync::Arc<Package>,
        entry: &str,
        identity: Option<u64>,
    ) -> Result<Self, String> {
        let (prepared, graph, declaration) = prepare_invocation(&package, entry, identity)?;
        let prepared = match prepared {
            std::borrow::Cow::Borrowed(_) => package.clone(),
            std::borrow::Cow::Owned(package) => std::sync::Arc::new(package),
        };
        let graph = graph.map(std::sync::Arc::new);
        let code = match &graph {
            Some(graph) => comptime_interpretor::CodeSession::for_iir(graph.clone()),
            None => comptime_interpretor::CodeSession::new(prepared.clone()),
        };
        Ok(Self {
            code,
            package: prepared,
            graph,
            declaration,
            entry: entry.into(),
            last_stats: InvocationStats::default(),
        })
    }
    pub fn invoke(&mut self, fuel: u64) -> Result<Value, String> {
        self.last_stats = InvocationStats::default();
        if fuel == 0 {
            return Err("invocation fuel must be positive".into());
        }
        execute_prepared(
            &self.package,
            self.graph.as_deref(),
            self.declaration,
            &self.entry,
            None,
            fuel,
            None,
            Some(&mut self.code),
            &mut self.last_stats,
        )
    }
    pub fn last_stats(&self) -> InvocationStats {
        self.last_stats
    }
}

pub fn invoke(package: &Package, entry: &str, fuel: u64) -> Result<Value, String> {
    invoke_selected(package, entry, None, None, fuel)
}

/// Invoke an entry belonging to a driver-selected semantic package identity.
pub fn invoke_in_package(
    package: &Package,
    identity: u64,
    entry: &str,
    fuel: u64,
) -> Result<Value, String> {
    invoke_selected(package, entry, Some(identity), None, fuel)
}

pub fn invoke_with_protocols(
    package: &Package,
    entry: &str,
    copy: &str,
    drop: &str,
    fuel: u64,
) -> Result<Value, String> {
    invoke_selected(
        package,
        entry,
        None,
        Some((
            crate::resources::Selection::Name(copy),
            crate::resources::Selection::Name(drop),
        )),
        fuel,
    )
}

/// Invoke through the ordinary driver preparation with a checked HIR publisher.
/// The caller retains the publisher and can recover its latest NIR afterward.
pub fn invoke_in_package_with_struct_publisher(
    package: &Package,
    identity: u64,
    entry: &str,
    fuel: u64,
    publisher: &elaboration::definitions::StructPublisher<'_>,
) -> Result<Value, String> {
    invoke_selected_with_publisher(package, entry, Some(identity), None, fuel, Some(publisher))
}

pub(crate) fn invoke_selected(
    package: &Package,
    entry: &str,
    identity: Option<u64>,
    protocols: Option<(
        crate::resources::Selection<'_>,
        crate::resources::Selection<'_>,
    )>,
    fuel: u64,
) -> Result<Value, String> {
    invoke_selected_with_publisher(package, entry, identity, protocols, fuel, None)
}

pub(crate) fn invoke_selected_with_publisher(
    package: &Package,
    entry: &str,
    identity: Option<u64>,
    protocols: Option<(
        crate::resources::Selection<'_>,
        crate::resources::Selection<'_>,
    )>,
    fuel: u64,
    publisher: Option<&dyn elaboration::definitions::StructBuildService>,
) -> Result<Value, String> {
    if fuel == 0 {
        return Err("invocation fuel must be positive".into());
    }
    let (prepared, graph, declaration) = prepare_invocation(package, entry, identity)?;
    execute_prepared(
        prepared.as_ref(),
        graph.as_ref(),
        declaration,
        entry,
        protocols,
        fuel,
        publisher,
        None,
        &mut InvocationStats::default(),
    )
}

fn prepare_invocation<'a>(
    package: &'a Package,
    entry: &str,
    identity: Option<u64>,
) -> Result<
    (
        std::borrow::Cow<'a, Package>,
        Option<iir::Package>,
        DeclarationId,
    ),
    String,
> {
    let (mut prepared, declaration) =
        prepare_entry_in_package(package, entry, FunctionPhase::Comptime, identity)?;
    let roots = [iir::InstanceKey {
        declaration,
        arguments: vec![].into(),
    }];
    let preparation_budget = elaboration::ptc::Budget::default();
    let mut reachability_fuel = preparation_budget.steps;
    let graph = if needs_input_staging(prepared.as_ref(), declaration, &mut reachability_fuel)?
        && needs_materialized_dispatch(prepared.as_ref(), declaration, &mut reachability_fuel)?
    {
        Some(
            elaboration::materialization::instantiate(
                prepared.as_ref(),
                &roots,
                std::collections::BTreeSet::new(),
                preparation_budget,
            )
            .map_err(|error| format!("cannot prepare staged comptime invocation: {error}"))?,
        )
    } else {
        let mut dispatch = elaboration::materialization::dispatch_instances_for_invocation(
            prepared.as_ref(),
            &roots,
            preparation_budget,
        )
        .map_err(|error| format!("cannot prepare invocation dispatch: {error}"))?;
        // The NIR world still contains its concrete implementation contracts.
        // Keep their admitted relationship witnesses (including generic parents
        // such as Once<T>), without installing unused executable method links.
        let mut selected: std::collections::HashSet<_> = dispatch
            .iter()
            .map(|instance| {
                (
                    instance.interface_instance(),
                    instance.subject.clone(),
                    instance.implementation,
                )
            })
            .collect();
        for original in &prepared.dispatch_instances {
            if selected.insert((
                original.interface_instance(),
                original.subject.clone(),
                original.implementation,
            )) {
                let mut witness = original.clone();
                witness.methods.clear();
                dispatch.push(witness);
            }
        }
        prepared.to_mut().dispatch_instances = dispatch;
        None
    };
    Ok((prepared, graph, declaration))
}

#[allow(clippy::too_many_arguments)]
fn execute_prepared(
    package: &Package,
    graph: Option<&iir::Package>,
    declaration: DeclarationId,
    entry: &str,
    protocols: Option<(
        crate::resources::Selection<'_>,
        crate::resources::Selection<'_>,
    )>,
    fuel: u64,
    publisher: Option<&dyn elaboration::definitions::StructBuildService>,
    mut session: Option<&mut comptime_interpretor::CodeSession>,
    stats: &mut InvocationStats,
) -> Result<Value, String> {
    let preparation_budget = elaboration::ptc::Budget::default();
    let roots = [iir::InstanceKey {
        declaration,
        arguments: vec![].into(),
    }];
    let context = elaboration::ptc::Context::from_package(package)
        .map_err(|error| format!("cannot prepare invocation type context: {error}"))?;
    let index = crate::implementation_world::index(&package, preparation_budget)
        .map_err(|error| format!("invocation implementation index: {error}"))?;
    let world = elaboration::capabilities::World {
        index: &index,
        implementations: &package.implementations,
        traits: &package.traits,
        assumptions: &[],
        context: &context,
    };
    let resources = protocols
        .map(|(copy, drop)| {
            crate::resources::ProtocolContext::selected(
                package,
                copy,
                Some(drop),
                preparation_budget,
            )
        })
        .transpose()?;
    let copy_failure = std::cell::RefCell::new(None::<String>);
    let pointer_copy = |ty: &Term, remaining: &mut u64| {
        let Some(resources) = &resources else {
            return false;
        };
        let mut fuel = usize::try_from(*remaining).unwrap_or(usize::MAX);
        let before = fuel;
        let evidence = elaboration::verification::copy::acquire(
            package,
            &resources.world(package),
            resources.copy,
            resources.drop.expect("paired protocols"),
            ty,
            elaboration::ptc::Budget {
                steps: fuel,
                ..Default::default()
            },
            &mut fuel,
        );
        *remaining = remaining.saturating_sub((before - fuel) as u64);
        let allowed = evidence.is_ok();
        *copy_failure.borrow_mut() = evidence.err();
        allowed
    };
    let solver = |predicate: &nir::program::TypePredicate, remaining: &mut u64| {
        crate::implementation_world::solve_metered(&world, predicate, preparation_budget, remaining)
    };
    let mut budget = Budget {
        steps: fuel,
        depth: 128,
    };
    let interpreter = match graph {
        Some(graph) => match session.as_deref_mut() {
            Some(session) => {
                Interpreter::from_iir_for_comptime_in_session(graph, session, &mut budget)
            }
            None => Interpreter::from_iir_for_comptime_metered(graph, &mut budget),
        },
        None => match session.as_deref_mut() {
            Some(session) => Interpreter::for_comptime_in_session(package, session, &mut budget),
            None => Interpreter::for_comptime(package, budget),
        },
    };
    let mut interpreter = interpreter
        .map_err(|error| format!("cannot prepare comptime invocation: {error}"))?
        .with_budgeted_type_solver(&solver)
        .with_budgeted_pointer_copy_permission(&pointer_copy);
    let builder = |request: &comptime_interpretor::StructBuildRequest, remaining: &mut u64| {
        publisher
            .expect("builder installed only with publisher")
            .prepare(request, remaining)
    };
    let enum_builder = |request: &comptime_interpretor::EnumBuildRequest, remaining: &mut u64| {
        publisher
            .expect("builder installed only with publisher")
            .prepare_enum(request, remaining)
    };
    if publisher.is_some() {
        interpreter = interpreter
            .with_struct_builder(&builder)
            .with_enum_builder(&enum_builder);
    }
    let operation = |vm: &mut Interpreter<'_>| {
        if graph.is_some() {
            vm.invoke_instance(&roots[0], vec![])
        } else {
            vm.invoke(declaration, vec![])
        }
    };
    let result = match session {
        Some(session) => interpreter.with_code_session(session, operation),
        None => operation(&mut interpreter),
    };
    *stats = InvocationStats {
        bytecode_compilations: interpreter.bytecode_compilations(),
        bytecode_invocations: interpreter.bytecode_invocations(),
    };
    result.map_err(|error| {
        let mut message = format!("comptime invocation of '{entry}' failed: {error}");
        if let Some(reason) = publisher.and_then(|publisher| publisher.last_diagnostic()) {
            message.push_str(&format!("; definition publication: {reason}"));
        }
        if let Some(reason) = copy_failure.borrow().as_ref() {
            message.push_str(&format!("; pointer Copy evidence: {reason}"));
        }
        message
    })
}

/// Prepare the normal zero-supplied-argument call plan, preserving default scope.
pub(crate) fn prepare_entry_in_package<'a>(
    package: &'a Package,
    entry: &str,
    phase: FunctionPhase,
    identity: Option<u64>,
) -> Result<(std::borrow::Cow<'a, Package>, nir::DeclarationId), String> {
    let mut matches = package.functions.iter().filter(|function| {
        function.name == entry && identity.is_none_or(|id| function.declaration.package == id)
    });
    let function = matches
        .next()
        .ok_or_else(|| format!("no function named '{entry}'"))?;
    if matches.next().is_some() {
        return Err(format!(
            "function name '{entry}' is ambiguous; use a uniquely named wrapper"
        ));
    }
    if function.phase != phase {
        return Err(format!(
            "'{entry}' is not a {} function",
            if phase == FunctionPhase::Comptime {
                "comptime"
            } else {
                "runtime"
            }
        ));
    }
    if !function.type_parameters.is_empty() {
        return Err(format!(
            "'{entry}' requires generic arguments; use a concrete wrapper"
        ));
    }
    if (0..function.parameters.len())
        .any(|index| !function.defaults.iter().any(|(slot, _)| *slot == index))
    {
        return Err(format!(
            "'{entry}' requires arguments; use a zero-argument wrapper"
        ));
    }
    // Defaults belong to the ordinary call plan and its parameter scope, not
    // to the raw host invoke API. Reuse that plan for an optional-parameter entry.
    let (package, declaration) = if function.parameters.is_empty() {
        (std::borrow::Cow::Borrowed(package), function.declaration)
    } else {
        use nir::program::{CallArgument, Expr, ExprKind, Function, SourceRange};
        let nir::Term::Function { result, .. } = &function.signature else {
            return Err("invalid invocation signature".into());
        };
        let source = SourceRange { start: 0, end: 0 };
        let mut declaration = nir::DeclarationId {
            package: u64::MAX,
            local: u32::MAX,
        };
        while package
            .functions
            .iter()
            .any(|item| item.declaration == declaration)
        {
            declaration.local = declaration
                .local
                .checked_sub(1)
                .ok_or("invocation declaration space exhausted")?;
        }
        let mut arguments = Vec::new();
        for index in 0..function.parameters.len() {
            let (_, value) = function
                .defaults
                .iter()
                .find(|(slot, _)| *slot == index)
                .unwrap();
            arguments.push(CallArgument {
                passing: Default::default(),
                parameter: index,
                is_default: true,
                value: value.clone(),
            });
        }
        let wrapper = Function {
            declaration,
            name: "<command-line-invocation>".into(),
            phase,
            inline: false,
            pure: false,
            is_unsafe: false,
            preconditions: vec![],
            postconditions: vec![],
            result_binding: None,
            entry_snapshots: vec![],
            type_parameters: vec![],
            requirements: vec![],
            capabilities: vec![],
            parameters: vec![],
            defaults: vec![],
            signature: nir::Term::Function {
                parameters: vec![],
                result: result.clone(),
            },
            body: Expr {
                source,
                ty: (**result).clone(),
                kind: ExprKind::Call {
                    callee: Box::new(Expr {
                        source,
                        ty: function
                            .signature
                            .clone()
                            .with_comptime_phase(phase == FunctionPhase::Comptime),
                        kind: ExprKind::Function(function.declaration),
                    }),
                    parameter_bindings: function.parameters.clone(),
                    arguments,
                },
            },
        };
        let mut owned = package.clone();
        owned.functions.push(wrapper);
        (std::borrow::Cow::Owned(owned), declaration)
    };
    Ok((package, declaration))
}

// Reachability, not execution: inspect calls and closed callable bodies until an
// ordinary Type telescope needs binding. Each declaration/member is visited once.
pub(crate) fn needs_input_staging(
    package: &Package,
    root: DeclarationId,
    fuel: &mut usize,
) -> Result<bool, String> {
    let mut pending = vec![root];
    let mut seen = std::collections::HashSet::new();
    let mut seen_methods = std::collections::HashSet::new();
    while let Some(id) = pending.pop() {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("staging reachability budget exhausted")?;
        if !seen.insert(id) {
            continue;
        }
        let Some(function) = package.functions.iter().find(|f| f.declaration == id) else {
            continue;
        };
        if matches!(function.signature.callable_signature(),Term::Function {parameters,..} if parameters.iter().any(|p|p.sort==nir::Sort::Type))
        {
            return Ok(true);
        }
        let mut staged_call = false;
        iir::visit::expressions(function, fuel, |expr, fuel| {
            if let nir::program::ExprKind::Call { callee, .. } = &expr.kind {
                staged_call |= matches!(callee.ty.callable_signature(),Term::Function {parameters,..}
                    if parameters.iter().any(|p|p.sort==nir::Sort::Type));
            }
            // Closed functions can be stored, returned, or passed to another
            // callable before invocation. Their bodies still contribute staging
            // dependencies. Merely carrying an open Type telescope, however,
            // does not bind its inputs; calls to it are detected above.
            if !matches!(expr.ty.callable_signature(),Term::Function {parameters,..}
                if parameters.iter().any(|p|p.sort==nir::Sort::Type))
            {
                match &expr.kind {
                    nir::program::ExprKind::Function(id) => pending.push(*id),
                    nir::program::ExprKind::FunctionInstance { declaration, .. } => {
                        pending.push(*declaration)
                    }
                    _ => {}
                }
            }
            if let nir::program::ExprKind::TraitMethod {
                interface, member, ..
            } = &expr.kind
            {
                // A stored method can hide calls behind its selected body.
                // Inspect possible bodies; materialization still performs actual
                // world-qualified selection and publishes only that instance.
                let open = matches!(expr.ty.callable_signature(),Term::Function {parameters,..}
                    if parameters.iter().any(|p|p.sort==nir::Sort::Type));
                if !open && seen_methods.insert((*interface, *member)) {
                    let contract = package
                        .traits
                        .iter()
                        .find(|t| t.declaration == *interface)
                        .and_then(|t| t.methods.iter().find(|m| m.declaration == *member))
                        .ok_or("missing method contract during staging reachability")?;
                    for implementation in &package.implementations {
                        *fuel = fuel
                            .checked_sub(1)
                            .ok_or("method staging reachability budget exhausted")?;
                        if implementation.interface == Some(*interface) {
                            pending.extend(
                                implementation
                                    .methods
                                    .iter()
                                    .filter(|(name, _)| name == &contract.name)
                                    .map(|(_, body)| *body),
                            );
                        }
                    }
                }
            }
            Ok(())
        })?;
        if staged_call {
            return Ok(true);
        }
    }
    Ok(false)
}

// Type telescopes alone do not require IIR: the comptime interpreter binds
// those inputs on each call. Method/associated-value selection still uses
// materialized dispatch. Follow stored and higher-order function identities
// too, including declarations whose Type telescope has not yet been bound.
fn needs_materialized_dispatch(
    package: &Package,
    root: DeclarationId,
    fuel: &mut usize,
) -> Result<bool, String> {
    use nir::program::ExprKind as K;
    let mut pending = vec![root];
    let mut seen = std::collections::HashSet::new();
    while let Some(id) = pending.pop() {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("invocation reachability budget exhausted")?;
        if !seen.insert(id) {
            continue;
        }
        let Some(function) = package
            .functions
            .iter()
            .find(|function| function.declaration == id)
        else {
            continue;
        };
        let mut dispatch = false;
        iir::visit::expressions(function, fuel, |expr, _| {
            match &expr.kind {
                K::TraitMethod { .. } | K::AssociatedValue { .. } => dispatch = true,
                K::Function(id)
                | K::FunctionInstance {
                    declaration: id, ..
                } => pending.push(*id),
                _ => {}
            }
            Ok(())
        })?;
        if dispatch {
            return Ok(true);
        }
    }
    Ok(false)
}
