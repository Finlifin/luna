//! Explicit, bounded invocation of completed comptime declarations.
pub use crate::memoization::Invocation as MemoizedInvocation;
use comptime_interpretor::{Budget, Interpreter, Value};
use nir::program::{FunctionPhase, Package};
use nir::{DeclarationId, Term};

/// Reusable preparation for one entry in an immutable compilation snapshot.
/// Code is shared; values, mutable storage and evidence are fresh on each call.
pub struct PreparedInvocation {
    package: std::sync::Arc<Package>,
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
        let (prepared, declaration) = prepare_invocation(&package, entry, identity)?;
        let prepared = match prepared {
            std::borrow::Cow::Borrowed(_) => package.clone(),
            std::borrow::Cow::Owned(package) => std::sync::Arc::new(package),
        };
        let code = comptime_interpretor::CodeSession::new(prepared.clone());
        Ok(Self {
            code,
            package: prepared,
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
    let (prepared, declaration) = prepare_invocation(package, entry, identity)?;
    execute_prepared(
        prepared.as_ref(),
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
) -> Result<(std::borrow::Cow<'a, Package>, DeclarationId), String> {
    let (mut prepared, declaration) =
        prepare_entry_in_package(package, entry, FunctionPhase::Comptime, identity)?;
    let roots = [iir::InstanceKey {
        declaration,
        arguments: vec![].into(),
    }];
    let preparation_budget = elaboration::ptc::Budget::default();
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
    Ok((prepared, declaration))
}

#[allow(clippy::too_many_arguments)]
fn execute_prepared(
    package: &Package,
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
    let associated_resolver = |access: &nir::program::Expr, remaining: &mut u64| {
        elaboration::materialization::resolve_associated_value_for_invocation(
            &world, access, remaining,
        )
    };
    let method_resolver = |callee: &nir::program::Expr, remaining: &mut u64| {
        resolve_method_for_invocation(package, &context, callee, remaining)
    };
    let mut budget = Budget {
        steps: fuel,
        depth: 128,
    };
    let interpreter = match session.as_deref_mut() {
        Some(session) => Interpreter::for_comptime_in_session(package, session, &mut budget),
        None => Interpreter::for_comptime(package, budget),
    };
    let mut interpreter = interpreter
        .map_err(|error| format!("cannot prepare comptime invocation: {error}"))?
        .with_method_resolver(&method_resolver)
        .with_associated_resolver(&associated_resolver)
        .with_budgeted_type_solver(&solver)
        .with_budgeted_pointer_copy_permission(&pointer_copy);
    let builder = |request: &comptime_interpretor::StructBuildRequest, remaining: &mut u64| {
        publisher
            .expect("builder installed only with publisher")
            .prepare(request, remaining)
    };
    let newtype_builder = |request: &comptime_interpretor::NewtypeBuildRequest,
                           remaining: &mut u64| {
        publisher
            .expect("builder installed only with publisher")
            .prepare_newtype(request, remaining)
    };
    let enum_builder = |request: &comptime_interpretor::EnumBuildRequest, remaining: &mut u64| {
        publisher
            .expect("builder installed only with publisher")
            .prepare_enum(request, remaining)
    };
    if publisher.is_some() {
        interpreter = interpreter
            .with_struct_builder(&builder)
            .with_enum_builder(&enum_builder)
            .with_newtype_builder(&newtype_builder);
    }
    let operation = |vm: &mut Interpreter<'_>| vm.invoke(declaration, vec![]);
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

/// Resolve only the invocation-demanded method in the owning source world.
/// Selection consumes the caller's budget; body execution remains in the VM.
pub(crate) fn resolve_method_for_invocation(
    package: &Package,
    context: &elaboration::ptc::Context,
    callee: &nir::program::Expr,
    remaining: &mut u64,
) -> Result<nir::program::MethodInstance, comptime_interpretor::Error> {
    let mut fuel = usize::try_from(*remaining).unwrap_or(usize::MAX);
    let before = fuel;
    let result = elaboration::materialization::resolve_method_identity_in_context(
        package,
        callee,
        context,
        &[],
        &mut fuel,
    );
    *remaining -= (before - fuel) as u64;
    result.map_err(|_| comptime_interpretor::Error {
        kind: comptime_interpretor::ErrorKind::UnmaterializedDispatch,
        source: Some(callee.source),
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
