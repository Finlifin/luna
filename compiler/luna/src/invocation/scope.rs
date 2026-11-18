//! Execute an entry and admit its concrete publications in one declaration scope.
use super::{InvocationStats, entry};
use comptime_interpretor::{Budget, Interpreter, SourceResolvers, Value};
use elaboration::materialization::{DeclarationSource, SourceInvocationQueries};
use nir::program::Package;

pub(super) struct InvocationScope<'a> {
    package: &'a Package,
    pub(super) resolvers: SourceResolvers<'a>,
    budget: Budget,
}

impl InvocationScope<'_> {
    pub(super) fn with<T>(
        source: DeclarationSource<'_>,
        fuel: u64,
        run: impl FnOnce(InvocationScope<'_>) -> Result<T, String>,
    ) -> Result<T, String> {
        let package = source.package();
        let mut steps =
            usize::try_from(fuel).map_err(|_| "invocation fuel does not fit the host counter")?;
        let context = source
            .package_context(&mut steps)
            .map_err(|error| format!("cannot prepare invocation type context: {error}"))?;
        let queries = SourceInvocationQueries::in_source(source, &context, &[]);
        let types = |predicate: &nir::program::TypePredicate, fuel: &mut u64| {
            queries.check(predicate, Default::default(), fuel)
        };
        let methods = |callee: &nir::program::Expr, fuel: &mut u64| queries.resolve(callee, fuel);
        let associated =
            |access: &nir::program::Expr, fuel: &mut u64| queries.associated(access, fuel);
        run(InvocationScope {
            package,
            resolvers: SourceResolvers {
                types: &types,
                methods: &methods,
                associated: &associated,
            },
            budget: Budget {
                steps: steps as u64,
                depth: 128,
            },
        })
    }

    pub(super) fn execute(
        self,
        invocation: &mut entry::Entry,
        entry: &str,
        publisher: Option<&dyn elaboration::definitions::StructBuildService>,
        mut session: Option<&mut comptime_interpretor::CodeSession>,
        stats: &mut InvocationStats,
    ) -> Result<Value, String> {
        let mut budget = self.budget;
        let package = self.package;
        let call = invocation.call(package, &mut budget.steps)?;
        let interpreter = match session.as_deref_mut() {
            Some(session) => Interpreter::for_comptime_resolving_in_session(
                package,
                session,
                self.resolvers,
                &mut budget,
            ),
            None => Interpreter::for_comptime_resolving_in_budget(
                package,
                &[],
                &[],
                &[],
                self.resolvers,
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
        let enum_builder = |request: &comptime_interpretor::EnumBuildRequest,
                            remaining: &mut u64| {
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
}
