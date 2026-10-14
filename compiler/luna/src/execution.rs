//! Bounded reference execution through the concrete instance graph.
use comptime_interpretor::{Budget, Interpreter, Value};
use nir::program::{FunctionPhase, Package};

/// Fuel is a per-stage bound, like the existing instantiation CLI budget.
/// Resource verification and native code generation remain separate passes.
pub fn interpret(package: &Package, entry: &str, fuel: u64) -> Result<Value, String> {
    interpret_selected(package, entry, None, fuel)
}

pub fn interpret_in_package(
    package: &Package,
    identity: u64,
    entry: &str,
    fuel: u64,
) -> Result<Value, String> {
    interpret_selected(package, entry, Some(identity), fuel)
}

/// Reference execution with source-bound Copy/Move decisions for direct-passing
/// slots. This checks representation and request multiplicity, not complete ownership or cleanup;
/// the resource-checked execution entrypoint remains `interpret_with_protocols`.
pub fn interpret_effects_with_protocols(
    package: &Package,
    entry: &str,
    copy_protocol: &str,
    drop_protocol: &str,
    fuel: u64,
) -> Result<Value, String> {
    let steps =
        usize::try_from(fuel).map_err(|_| "execution fuel does not fit the host work counter")?;
    if steps == 0 {
        return Err("execution fuel must be positive".into());
    }
    let budget = elaboration::ptc::Budget {
        steps,
        ..Default::default()
    };
    let (package, declaration) =
        crate::invocation::prepare_entry_in_package(package, entry, FunctionPhase::Runtime, None)?;
    let protocols = crate::resources::ProtocolContext::new(
        &package,
        copy_protocol,
        Some(drop_protocol),
        budget,
    )?;
    let world = protocols.world(&package);
    let key = iir::InstanceKey {
        declaration,
        arguments: vec![],
    };
    let graph = elaboration::materialization::instantiate(
        &package,
        std::slice::from_ref(&key),
        Default::default(),
        budget,
    )?;
    let mut plan_fuel = steps;
    let plan = iir::effect_values::Plan::acquire(&graph, &mut plan_fuel, |ty, fuel| {
        if *ty == nir::Term::Primitive(nir::Primitive::Void) {
            return Ok(iir::effect_values::Delivery::Copy);
        }
        let decision = crate::protocols::copy_delivery(
            &package,
            &world,
            &protocols.items,
            ty,
            elaboration::ptc::Budget {
                steps: budget.steps.min(*fuel),
                ..budget
            },
        )?;
        Ok(match decision {
            elaboration::verification::moves::Delivery::Copy => iir::effect_values::Delivery::Copy,
            elaboration::verification::moves::Delivery::Move => iir::effect_values::Delivery::Move,
        })
    })?;
    let mut usage_fuel = steps;
    elaboration::verification::effect_usage::check(&graph, &plan, &mut usage_fuel)?;
    let solver = |predicate: &nir::program::TypePredicate| runtime_type_predicate(&world, predicate);
    Interpreter::from_iir(
        &graph,
        Budget {
            steps: fuel,
            depth: 128,
        },
    )
    .map_err(|e| format!("cannot prepare effect reference execution: {e}"))?
    .with_type_solver(&solver)
    .with_effect_values(&plan)
    .map_err(|e| format!("cannot install effect value plan: {e}"))?
    .invoke_instance(&key, vec![])
    .map_err(|e| format!("effect reference execution of '{entry}' failed: {e}"))
}

fn interpret_selected(
    package: &Package,
    entry: &str,
    identity: Option<u64>,
    fuel: u64,
) -> Result<Value, String> {
    if fuel == 0 {
        return Err("execution fuel must be positive".into());
    }
    let (package, declaration) = crate::invocation::prepare_entry_in_package(
        package,
        entry,
        FunctionPhase::Runtime,
        identity,
    )?;
    let key = iir::InstanceKey {
        declaration,
        arguments: vec![],
    };
    let graph = elaboration::materialization::instantiate(
        &package,
        std::slice::from_ref(&key),
        Default::default(),
        elaboration::ptc::Budget {
            steps: usize::try_from(fuel)
                .map_err(|_| "execution fuel does not fit the host work counter")?,
            ..Default::default()
        },
    )?;
    let context = elaboration::ptc::Context::from_package(&package)
        .map_err(|error| format!("cannot prepare runtime type context: {error}"))?;
    let mut index = elaboration::implementation::Index::default();
    for implementation in package.implementations.iter().filter(|item| item.interface.is_some()) {
        index.insert(elaboration::implementation::ImplementationHead {
            declaration: implementation.declaration, interface: implementation.interface,
            interface_arguments: implementation.interface_arguments.clone(),
            subject: implementation.subject.clone(), type_parameters: implementation.type_parameters.clone(),
            requirements: implementation.requirements.clone(),
        }, elaboration::ptc::Budget::default()).map_err(|error| format!("runtime implementation index: {error:?}"))?;
    }
    let world = elaboration::capabilities::World {
        index: &index, implementations: &package.implementations, traits: &package.traits,
        assumptions: &[], context: &context,
    };
    let solver = |predicate: &nir::program::TypePredicate| runtime_type_predicate(&world, predicate);
    Interpreter::from_iir(
        &graph,
        Budget {
            steps: fuel,
            depth: 128,
        },
    )
    .map_err(|e| format!("cannot prepare runtime reference execution: {e}"))?
    .with_type_solver(&solver)
    .invoke_instance(&key, vec![])
    .map_err(|e| format!("runtime reference execution of '{entry}' failed: {e}"))
}

/// Execute only after publishing acquired cleanup for every reachable context.
/// Protocol names are explicit driver configuration, not language-item inference.
pub fn interpret_with_protocols(
    package: &Package,
    entry: &str,
    copy_protocol: &str,
    drop_protocol: &str,
    fuel: u64,
) -> Result<Value, String> {
    interpret_protocols_selected(package, entry, None, copy_protocol, drop_protocol, fuel, false)
}

pub fn interpret_with_protocols_in_package(
    package: &Package, identity: u64, entry: &str, copy_protocol: &str,
    drop_protocol: &str, fuel: u64,
) -> Result<Value, String> {
    interpret_protocols_selected(package, entry, Some(identity), copy_protocol, drop_protocol, fuel, false)
}

/// Explicit optional transformation before resource verification. Preparation
/// failures are reported before execution; runtime failures are never retried.
pub fn interpret_inline_with_protocols_in_package(
    package: &Package, identity: u64, entry: &str, copy_protocol: &str,
    drop_protocol: &str, fuel: u64,
) -> Result<Value, String> {
    interpret_protocols_selected(package, entry, Some(identity), copy_protocol, drop_protocol, fuel, true)
}

fn interpret_protocols_selected(
    package: &Package, entry: &str, identity: Option<u64>, copy_protocol: &str,
    drop_protocol: &str, fuel: u64, inline_functions: bool,
) -> Result<Value, String> {
    if fuel == 0 {
        return Err("execution fuel must be positive".into());
    }
    let mut steps =
        usize::try_from(fuel).map_err(|_| "execution fuel does not fit the host work counter")?;
    let budget = elaboration::ptc::Budget {
        steps,
        ..Default::default()
    };
    let (mut package, declaration) =
        crate::invocation::prepare_entry_in_package(package, entry, FunctionPhase::Runtime, identity)?;
    if inline_functions {
        let mut optimization_fuel = steps;
        elaboration::program::inlining::expand(package.to_mut(), &mut optimization_fuel)
            .map_err(|error| format!("cannot prepare optional inlining: {error}"))?;
    }
    let protocols = crate::resources::ProtocolContext::new(
        &package,
        copy_protocol,
        Some(drop_protocol),
        budget,
    )?;
    let executable = crate::cleanup::prepare_source(
        &package,
        &protocols.world(&package),
        &protocols.items,
        iir::InstanceKey {
            declaration,
            arguments: vec![],
        },
        &mut steps,
        budget,
    )
    .map_err(|e| format!("cannot prepare resource-checked execution: {e}"))?;
    let world = protocols.world(&package);
    let solver = |predicate: &nir::program::TypePredicate| runtime_type_predicate(&world, predicate);
    Interpreter::from_iir(
        &executable.graph,
        Budget {
            steps: fuel,
            depth: 128,
        },
    )
    .map_err(|e| format!("cannot prepare runtime reference execution: {e}"))?
    .with_type_solver(&solver)
    .with_cleanup_schedule(&executable.schedule)
    .map_err(|e| format!("cannot install runtime cleanup: {e}"))?
    .invoke_instance(&executable.entry, vec![])
    .map_err(|e| format!("runtime reference execution of '{entry}' failed: {e}"))
}

fn runtime_type_predicate(world: &elaboration::capabilities::World<'_>, predicate: &nir::program::TypePredicate)
    -> comptime_interpretor::ContractOutcome {
    use comptime_interpretor::ContractOutcome;
    match world.check_predicate(predicate, elaboration::ptc::Budget::default()) {
        Ok(elaboration::ptc::Outcome::Entailed(_)) => ContractOutcome::Satisfied,
        Ok(elaboration::ptc::Outcome::NotEntailed(_)) => ContractOutcome::Rejected,
        _ => ContractOutcome::Unknown,
    }
}
