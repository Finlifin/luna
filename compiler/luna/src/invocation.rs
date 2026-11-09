//! Explicit, bounded invocation of completed comptime declarations.
mod entry;
pub use crate::memoization::Invocation as MemoizedInvocation;
use comptime_interpretor::{Budget, Interpreter, Value};
use elaboration::materialization::{DeclarationSnapshot, DeclarationSource};
use nir::program::{FunctionPhase, Package};
use nir::{DeclarationId, Term};

/// Reusable preparation for one entry in an immutable compilation snapshot.
/// Code is shared; values, mutable storage and evidence are fresh on each call.
pub struct PreparedInvocation {
    source: DeclarationSnapshot,
    invocation: entry::Entry,
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
        let (prepared, invocation) = prepare_invocation(&package, entry, identity)?;
        let prepared = match prepared {
            std::borrow::Cow::Borrowed(_) => package.clone(),
            std::borrow::Cow::Owned(package) => std::sync::Arc::new(package),
        };
        let code = comptime_interpretor::CodeSession::new(prepared.clone());
        Ok(Self {
            code,
            source: DeclarationSnapshot::new(prepared),
            invocation,
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
            DeclarationSource::Snapshot(&self.source),
            &mut self.invocation,
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
    let (prepared, mut invocation) = prepare_invocation(package, entry, identity)?;
    // Resource binding needs a retained source. Move an already-owned
    // preparation into that owner; VM and query services borrow the same source.
    let retained;
    let source = if protocols.is_some() {
        retained = DeclarationSnapshot::new(std::sync::Arc::new(prepared.into_owned()));
        DeclarationSource::Snapshot(&retained)
    } else {
        DeclarationSource::Borrowed(prepared.as_ref())
    };
    execute_prepared(
        source,
        &mut invocation,
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
) -> Result<(std::borrow::Cow<'a, Package>, entry::Entry), String> {
    let function = entry::select(package, entry, FunctionPhase::Comptime, identity)?;
    let invocation = entry::Entry::new(function);
    let mut prepared = std::borrow::Cow::Borrowed(package);
    // A source invocation owns declarations, not a precomputed executable
    // closure. Each demanded relationship is proved by the host services.
    if !prepared.dispatch_instances.is_empty() {
        prepared.to_mut().dispatch_instances.clear();
    }
    Ok((prepared, invocation))
}

#[allow(clippy::too_many_arguments)]
fn execute_prepared(
    source: DeclarationSource<'_>,
    invocation: &mut entry::Entry,
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
    let package = source.package();
    let preparation_budget = elaboration::ptc::Budget::default();
    let context = elaboration::ptc::Context::from_package(package)
        .map_err(|error| format!("cannot prepare invocation type context: {error}"))?;
    let resources = protocols
        .map(|(copy, drop)| {
            crate::resources::ProtocolContext::selected_in_source(
                match source {
                    DeclarationSource::Snapshot(snapshot) => snapshot.clone(),
                    DeclarationSource::Borrowed(_) => {
                        return Err("resource invocation requires a retained source".into());
                    }
                },
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
            resources.package(),
            &resources.world(),
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
    let source = resources
        .as_ref()
        .map_or(source, |resources| resources.source());
    let source_queries =
        elaboration::materialization::SourceInvocationQueries::in_source(source, &context, &[]);
    let solver = |predicate: &nir::program::TypePredicate, remaining: &mut u64| {
        source_queries.check(predicate, preparation_budget, remaining)
    };
    let associated_resolver = |access: &nir::program::Expr, remaining: &mut u64| {
        source_queries.associated(access, remaining)
    };
    let method_resolver = |callee: &nir::program::Expr, remaining: &mut u64| {
        source_queries.resolve(callee, remaining)
    };
    let mut budget = Budget {
        steps: fuel,
        depth: 128,
    };
    let call = invocation.call(package, &mut budget.steps)?;
    let resolvers = comptime_interpretor::SourceResolvers {
        types: &solver,
        methods: &method_resolver,
        associated: &associated_resolver,
    };
    let interpreter = match session.as_deref_mut() {
        Some(session) => {
            Interpreter::for_comptime_resolving_in_session(package, session, resolvers, &mut budget)
        }
        None => Interpreter::for_comptime_resolving_in_budget(
            package,
            &[],
            &[],
            &[],
            resolvers,
            &mut budget,
        ),
    };
    let mut interpreter = interpreter
        .map_err(|error| format!("cannot prepare comptime invocation: {error}"))?
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
    let operation = |vm: &mut Interpreter<'_>| match call {
        Some(call) => vm.invoke_comptime_expression_owned(call),
        None => vm.invoke(invocation.declaration, vec![]),
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
    let function = entry::select(package, entry, phase, identity)?;
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
