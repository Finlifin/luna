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
        arguments: vec![].into(),
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
    let solver = |predicate: &nir::program::TypePredicate, remaining: &mut u64| {
        crate::implementation_world::solve_metered(
            &world,
            predicate,
            elaboration::ptc::Budget::default(),
            remaining,
        )
    };
    Interpreter::from_iir(
        &graph,
        Budget {
            steps: fuel,
            depth: 128,
        },
    )
    .map_err(|e| format!("cannot prepare effect reference execution: {e}"))?
    .with_budgeted_type_solver(&solver)
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
    ReferenceSession::new(package, fuel)?.invoke(declaration, entry)
}

/// A reusable immutable type world; each entry still owns its materialization
/// and interpreter so no runtime values or spent budgets cross invocations.
pub(crate) struct ReferenceSession<'a> {
    package: std::borrow::Cow<'a, Package>,
    context: elaboration::ptc::Context,
    index: elaboration::implementation::Index,
    fuel: u64,
    budget: elaboration::ptc::Budget,
}
impl<'a> ReferenceSession<'a> {
    pub(crate) fn new(package: std::borrow::Cow<'a, Package>, fuel: u64) -> Result<Self, String> {
        if fuel == 0 {
            return Err("execution fuel must be positive".into());
        }
        let budget = elaboration::ptc::Budget {
            steps: usize::try_from(fuel)
                .map_err(|_| "execution fuel does not fit the host work counter")?,
            ..Default::default()
        };
        let context = elaboration::ptc::Context::from_package(&package)
            .map_err(|error| format!("cannot prepare runtime type context: {error}"))?;
        let index = crate::implementation_world::index(&package, budget)
            .map_err(|error| format!("runtime implementation index: {error}"))?;
        Ok(Self {
            package,
            context,
            index,
            fuel,
            budget,
        })
    }

    pub(crate) fn invoke(
        &self,
        declaration: nir::DeclarationId,
        entry: &str,
    ) -> Result<Value, String> {
        let package = self.package.as_ref();
        let key = iir::InstanceKey {
            declaration,
            arguments: vec![].into(),
        };
        let graph = elaboration::materialization::instantiate(
            package,
            std::slice::from_ref(&key),
            Default::default(),
            self.budget,
        )?;
        let world = elaboration::capabilities::World {
            index: &self.index,
            implementations: &package.implementations,
            traits: &package.traits,
            assumptions: &[],
            context: &self.context,
        };
        let solver = |predicate: &nir::program::TypePredicate, remaining: &mut u64| {
            crate::implementation_world::solve_metered(
                &world,
                predicate,
                elaboration::ptc::Budget::default(),
                remaining,
            )
        };
        Interpreter::from_iir(
            &graph,
            Budget {
                steps: self.fuel,
                depth: 128,
            },
        )
        .map_err(|e| format!("cannot prepare runtime reference execution: {e}"))?
        .with_budgeted_type_solver(&solver)
        .invoke_instance(&key, vec![])
        .map_err(|e| format!("runtime reference execution of '{entry}' failed: {e}"))
    }
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
    interpret_protocols_selected(
        package,
        entry,
        None,
        crate::resources::Selection::Name(copy_protocol),
        crate::resources::Selection::Name(drop_protocol),
        fuel,
        false,
    )
}

pub fn interpret_with_protocols_in_package(
    package: &Package,
    identity: u64,
    entry: &str,
    copy_protocol: &str,
    drop_protocol: &str,
    fuel: u64,
) -> Result<Value, String> {
    interpret_protocols_selected(
        package,
        entry,
        Some(identity),
        crate::resources::Selection::Name(copy_protocol),
        crate::resources::Selection::Name(drop_protocol),
        fuel,
        false,
    )
}

/// Explicit optional transformation before resource verification. Preparation
/// failures are reported before execution; runtime failures are never retried.
pub fn interpret_inline_with_protocols_in_package(
    package: &Package,
    identity: u64,
    entry: &str,
    copy_protocol: &str,
    drop_protocol: &str,
    fuel: u64,
) -> Result<Value, String> {
    interpret_protocols_selected(
        package,
        entry,
        Some(identity),
        crate::resources::Selection::Name(copy_protocol),
        crate::resources::Selection::Name(drop_protocol),
        fuel,
        true,
    )
}

pub(crate) fn interpret_protocols_selected(
    package: &Package,
    entry: &str,
    identity: Option<u64>,
    copy_protocol: crate::resources::Selection<'_>,
    drop_protocol: crate::resources::Selection<'_>,
    fuel: u64,
    inline_functions: bool,
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
    CheckedSession::new(
        package,
        copy_protocol,
        drop_protocol,
        fuel,
        inline_functions,
    )?
    .invoke(declaration, entry)
}

/// Immutable package preparation shared across independent test executions.
/// Graph acquisition, cleanup, interpreter state and budgets remain per entry.
pub(crate) struct CheckedSession<'a> {
    package: std::borrow::Cow<'a, Package>,
    protocols: crate::resources::ProtocolContext,
    fuel: u64,
    budget: elaboration::ptc::Budget,
}

impl<'a> CheckedSession<'a> {
    pub(crate) fn new(
        mut package: std::borrow::Cow<'a, Package>,
        copy_protocol: crate::resources::Selection<'_>,
        drop_protocol: crate::resources::Selection<'_>,
        fuel: u64,
        inline_functions: bool,
    ) -> Result<Self, String> {
        if fuel == 0 {
            return Err("execution fuel must be positive".into());
        }
        let steps = usize::try_from(fuel)
            .map_err(|_| "execution fuel does not fit the host work counter")?;
        let budget = elaboration::ptc::Budget {
            steps,
            ..Default::default()
        };
        if inline_functions {
            let mut optimization_fuel = steps;
            elaboration::program::inlining::expand(package.to_mut(), &mut optimization_fuel)
                .map_err(|error| format!("cannot prepare optional inlining: {error}"))?;
        }
        let protocols = crate::resources::ProtocolContext::selected(
            &package,
            copy_protocol,
            Some(drop_protocol),
            budget,
        )?;
        Ok(Self {
            package,
            protocols,
            fuel,
            budget,
        })
    }

    pub(crate) fn invoke(
        &self,
        declaration: nir::DeclarationId,
        entry: &str,
    ) -> Result<Value, String> {
        let package = self.package.as_ref();
        let protocols = &self.protocols;
        let budget = self.budget;
        let mut steps = budget.steps;
        let executable = crate::cleanup::prepare_checked_source(
            package,
            &protocols.world(package),
            &protocols.items,
            iir::InstanceKey {
                declaration,
                arguments: vec![].into(),
            },
            &mut steps,
            budget,
        )
        .map_err(|e| format!("cannot prepare resource-checked execution: {e}"))?;
        let world = protocols.world(package);
        // Installed aggregate values need explicit Copy delivery evidence;
        // their representation alone does not authorize repeated requests.
        let effect_values =
            iir::effect_values::Plan::acquire(&executable.graph, &mut steps, |ty, fuel| {
                if *ty == nir::Term::Primitive(nir::Primitive::Void) {
                    return Ok(iir::effect_values::Delivery::Copy);
                }
                let decision = crate::protocols::copy_delivery(
                    package,
                    &world,
                    &protocols.items,
                    ty,
                    elaboration::ptc::Budget {
                        steps: budget.steps.min(*fuel),
                        ..budget
                    },
                )?;
                Ok(match decision {
                    elaboration::verification::moves::Delivery::Copy => {
                        iir::effect_values::Delivery::Copy
                    }
                    elaboration::verification::moves::Delivery::Move => {
                        iir::effect_values::Delivery::Move
                    }
                })
            })
            .map_err(|error| format!("cannot prepare effect value delivery: {error}"))?;
        let solver = |predicate: &nir::program::TypePredicate, remaining: &mut u64| {
            crate::implementation_world::solve_metered(
                &world,
                predicate,
                elaboration::ptc::Budget::default(),
                remaining,
            )
        };
        let copy_failure = std::cell::RefCell::new(None::<String>);
        let pointer_copy = |ty: &nir::Term, remaining: &mut u64| {
            let mut proof_fuel = usize::try_from(*remaining).unwrap_or(usize::MAX);
            let before = proof_fuel;
            let evidence = elaboration::verification::copy::acquire(
                package,
                &world,
                protocols.copy,
                protocols.drop.expect("checked execution requires Drop"),
                ty,
                elaboration::ptc::Budget {
                    steps: budget.steps.min(proof_fuel),
                    ..budget
                },
                &mut proof_fuel,
            );
            *remaining = remaining.saturating_sub((before - proof_fuel) as u64);
            let allowed = evidence.is_ok();
            *copy_failure.borrow_mut() = evidence.err();
            allowed
        };
        Interpreter::from_iir(
            &executable.graph,
            Budget {
                steps: self.fuel,
                depth: 128,
            },
        )
        .map_err(|e| format!("cannot prepare runtime reference execution: {e}"))?
        .with_budgeted_type_solver(&solver)
        .with_budgeted_pointer_copy_permission(&pointer_copy)
        .with_effect_values(&effect_values)
        .map_err(|error| format!("cannot install effect value delivery: {error}"))?
        .with_cleanup_schedule(&executable.schedule)
        .map_err(|e| format!("cannot install runtime cleanup: {e}"))?
        .invoke_instance(&executable.entry, vec![])
        .map_err(|e| {
            let mut message = format!("runtime reference execution of '{entry}' failed: {e}");
            if let Some(reason) = copy_failure.borrow().as_ref() {
                message.push_str(&format!("; pointer Copy evidence: {reason}"));
            }
            message
        })
    }
}
