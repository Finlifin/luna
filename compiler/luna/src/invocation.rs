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
        Self::new_in_budget(package, entry, identity, &mut Budget::default().steps)
    }

    /// Select an entry using the caller's construction allowance. Later source
    /// projection and execution consume the allowance supplied to `invoke`.
    pub fn new_in_budget(
        package: std::sync::Arc<Package>,
        entry: &str,
        identity: Option<u64>,
        fuel: &mut u64,
    ) -> Result<Self, String> {
        let function = entry::select_in(&package, entry, FunctionPhase::Comptime, identity, fuel)?;
        *fuel = fuel
            .checked_sub(entry.len() as u64)
            .ok_or("entry name retention budget exhausted")?;
        let invocation = entry::Entry::new(function);
        let code = comptime_interpretor::CodeSession::new(package.clone());
        Ok(Self {
            code,
            source: DeclarationSnapshot::new(package),
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

/// Invoke through the ordinary driver preparation with a checked HIR publisher.
/// The caller retains the publisher and can recover its latest NIR afterward.
pub fn invoke_in_package_with_struct_publisher(
    package: &Package,
    identity: u64,
    entry: &str,
    fuel: u64,
    publisher: &elaboration::definitions::StructPublisher<'_>,
) -> Result<Value, String> {
    invoke_selected_with_publisher(package, entry, Some(identity), fuel, Some(publisher))
}

pub(crate) fn invoke_selected(
    package: &Package,
    entry: &str,
    identity: Option<u64>,
    fuel: u64,
) -> Result<Value, String> {
    invoke_selected_with_publisher(package, entry, identity, fuel, None)
}

pub(crate) fn invoke_selected_with_publisher(
    package: &Package,
    entry: &str,
    identity: Option<u64>,
    fuel: u64,
    publisher: Option<&dyn elaboration::definitions::StructBuildService>,
) -> Result<Value, String> {
    if fuel == 0 {
        return Err("invocation fuel must be positive".into());
    }
    let mut fuel = fuel;
    let function = entry::select_in(package, entry, FunctionPhase::Comptime, identity, &mut fuel)?;
    let mut invocation = entry::Entry::new(function);
    // Source resolvers select a declaration view of this exact package. The VM
    // excludes preselected dispatch without copying unrelated bodies or inputs.
    let source = DeclarationSource::Borrowed(package);
    execute_prepared(
        source,
        &mut invocation,
        entry,
        fuel,
        publisher,
        None,
        &mut InvocationStats::default(),
    )
}

fn execute_prepared(
    source: DeclarationSource<'_>,
    invocation: &mut entry::Entry,
    entry: &str,
    fuel: u64,
    publisher: Option<&dyn elaboration::definitions::StructBuildService>,
    mut session: Option<&mut comptime_interpretor::CodeSession>,
    stats: &mut InvocationStats,
) -> Result<Value, String> {
    let package = source.package();
    let preparation_budget = elaboration::ptc::Budget::default();
    let mut preparation = elaboration::ptc::Budget {
        steps: usize::try_from(fuel)
            .map_err(|_| "invocation fuel does not fit the host counter")?,
        ..preparation_budget
    };
    let context = elaboration::ptc::Context::from_package_metered(package, &mut preparation.steps)
        .map_err(|error| format!("cannot prepare invocation type context: {error}"))?;
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
        steps: preparation.steps as u64,
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
    let mut interpreter =
        interpreter.map_err(|error| format!("cannot prepare comptime invocation: {error}"))?;
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
        None => vm
            .start_invocation(Value::Function(invocation.declaration), vec![])?
            .run_to_completion(),
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
            interface: nir::program::FunctionDeclaration {
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
