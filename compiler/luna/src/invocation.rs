//! Explicit, bounded invocation of completed comptime declarations.
pub use crate::memoization::Invocation as MemoizedInvocation;
use comptime_interpretor::{Budget, ContractOutcome, Interpreter, Value};
use nir::program::{FunctionPhase, Package};
use nir::{DeclarationId, Term};

pub fn invoke(package: &Package, entry: &str, fuel: u64) -> Result<Value, String> {
    invoke_selected(package, entry, None, fuel)
}

/// Invoke an entry belonging to a driver-selected semantic package identity.
pub fn invoke_in_package(
    package: &Package,
    identity: u64,
    entry: &str,
    fuel: u64,
) -> Result<Value, String> {
    invoke_selected(package, entry, Some(identity), fuel)
}

fn invoke_selected(
    package: &Package,
    entry: &str,
    identity: Option<u64>,
    fuel: u64,
) -> Result<Value, String> {
    if fuel == 0 {
        return Err("invocation fuel must be positive".into());
    }
    let (mut prepared, declaration) =
        prepare_entry_in_package(package, entry, FunctionPhase::Comptime, identity)?;
    let roots = [iir::InstanceKey {
        declaration,
        arguments: vec![],
    }];
    let preparation_budget = elaboration::ptc::Budget::default();
    let mut reachability_fuel = preparation_budget.steps;
    let graph = if needs_input_staging(prepared.as_ref(), declaration, &mut reachability_fuel)? {
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
        let dispatch = elaboration::materialization::dispatch_instances_for_roots(
            prepared.as_ref(),
            &roots,
            preparation_budget,
        )
        .map_err(|error| format!("cannot prepare invocation dispatch: {error}"))?;
        prepared.to_mut().dispatch_instances = dispatch;
        None
    };
    let package = prepared.as_ref();
    let context = elaboration::ptc::Context::from_package(package)
        .map_err(|error| format!("cannot prepare invocation type context: {error}"))?;
    let mut index = elaboration::implementation::Index::default();
    for implementation in package
        .implementations
        .iter()
        .filter(|i| i.interface.is_some())
    {
        index
            .insert(
                elaboration::implementation::ImplementationHead {
                    declaration: implementation.declaration,
                    interface: implementation.interface,
                    interface_arguments: implementation.interface_arguments.clone(),
                    subject: implementation.subject.clone(),
                    type_parameters: implementation.type_parameters.clone(),
                    requirements: implementation.requirements.clone(),
                },
                elaboration::ptc::Budget::default(),
            )
            .map_err(|error| format!("invocation implementation index: {error:?}"))?;
    }
    let world = elaboration::capabilities::World {
        index: &index,
        implementations: &package.implementations,
        traits: &package.traits,
        assumptions: &[],
        context: &context,
    };
    let solver = |predicate: &nir::program::TypePredicate| {
        use elaboration::ptc::{Budget, Outcome};
        match world.check_predicate(predicate, Budget::default()) {
            Ok(Outcome::Entailed(_)) => ContractOutcome::Satisfied,
            Ok(Outcome::NotEntailed(_)) => ContractOutcome::Rejected,
            _ => ContractOutcome::Unknown,
        }
    };
    let budget = Budget {
        steps: fuel,
        depth: 128,
    };
    let interpreter = match &graph {
        Some(graph) => Interpreter::from_iir_for_comptime(graph, budget),
        None => Interpreter::for_comptime(package, budget),
    };
    let mut interpreter = interpreter
        .map_err(|error| format!("cannot prepare comptime invocation: {error}"))?
        .with_type_solver(&solver);
    let result = if graph.is_some() {
        interpreter.invoke_instance(&roots[0], vec![])
    } else {
        interpreter.invoke(declaration, vec![])
    };
    result.map_err(|error| format!("comptime invocation of '{entry}' failed: {error}"))
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
