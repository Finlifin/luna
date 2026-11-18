//! Checked source computations, separated from their invocation. This is an
//! owned preparation result, not a Type or an inference-variable snapshot.
use super::restore::restore_required_packet;
use super::*;
use crate::materialization::{DeclarationSnapshot, DeclarationSource};
use comptime_interpretor::{CodeSession, InvocationValue};
use std::sync::{Arc, Mutex};
mod inputs;

/// Own the declarations and their derived index together. Once published into
/// a preparation, this world is immutable; invocation state lives elsewhere.
pub(super) struct RequiredWorld {
    source: DeclarationSnapshot,
    phase: Option<nir::phases::PhaseCheckedPackage<'static>>,
    code: Mutex<Option<CodeSession>>,
}

// A lease never holds a lock while invoking source code or a host callback.
// Reentrant invocations get a separate cache; only one completed cache is kept.
struct RequiredCode<'a> {
    world: &'a RequiredWorld,
    session: Option<CodeSession>,
}
impl RequiredCode<'_> {
    fn session(&mut self) -> &mut CodeSession {
        self.session.as_mut().expect("live code lease")
    }
}
impl Drop for RequiredCode<'_> {
    fn drop(&mut self) {
        let mut retained = self
            .world
            .code
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if retained.is_none() {
            *retained = self.session.take();
        }
    }
}

impl RequiredWorld {
    #[cfg(test)]
    pub(super) fn new(package: n::Package) -> Arc<Self> {
        // Retaining an owned source does not demand type/member lookup. The
        // query service prepares its directory using the first demand's budget;
        // VM declaration and phase admission remain a separate execution step.
        Arc::new(Self {
            source: DeclarationSnapshot::new(Arc::new(package)),
            phase: None,
            code: Mutex::new(None),
        })
    }

    pub(super) fn with_phase(
        package: Arc<n::Package>,
        phase: &nir::phases::PhaseCheckedPackage<'_>,
    ) -> Result<Arc<Self>, comptime_interpretor::Error> {
        let phase = phase
            .retain_in(package.clone())
            .ok_or(comptime_interpretor::Error {
                kind: comptime_interpretor::ErrorKind::InvalidCallPlan,
                source: None,
            })?;
        Ok(Arc::new(Self {
            source: DeclarationSnapshot::new(package),
            phase: Some(phase),
            code: Mutex::new(None),
        }))
    }

    fn code(&self) -> RequiredCode<'_> {
        let session = self
            .code
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
            .unwrap_or_else(|| match &self.phase {
                Some(phase) => {
                    CodeSession::with_phase_checked_source(self.source.package().clone(), phase)
                        .expect("required world retains phase evidence from its immutable source")
                }
                None => CodeSession::new(self.source.package().clone()),
            });
        RequiredCode {
            world: self,
            session: Some(session),
        }
    }

    fn package(&self) -> &n::Package {
        self.source.package()
    }

    #[cfg(test)]
    fn capabilities<'a>(
        &'a self,
        assumptions: &'a [n::TraitRequirement],
        context: &'a crate::ptc::Context,
    ) -> crate::capabilities::World<'a> {
        self.source
            .capabilities(context, assumptions, &mut crate::ptc::Budget::default())
            .unwrap()
    }

    fn solve(
        &self,
        assumptions: &[n::TraitRequirement],
        context: &crate::ptc::Context,
        predicate: &n::TypePredicate,
        remaining: &mut u64,
    ) -> comptime_interpretor::ContractOutcome {
        crate::materialization::SourceInvocationQueries::in_source(
            DeclarationSource::Snapshot(&self.source),
            context,
            assumptions,
        )
        .check(predicate, crate::ptc::Budget::default(), remaining)
    }
}

pub(in crate::program) struct PreparedRequired {
    pub(super) world: Arc<RequiredWorld>,
    pub entry: DeclarationId,
    pub inputs: Vec<(n::LocalId, Term, Option<Term>)>,
    pub result_type: Term,
    pub scoped_parameters: Vec<nir::DeclarationParameter>,
    pub scoped_arguments: Vec<Term>,
    pub input_frames: Vec<Vec<Sort>>,
    pub invocation_assumptions: Vec<n::TraitRequirement>,
    pub abstract_parameters: Vec<DeclarationId>,
    pub abstract_value_parameters: Vec<(DeclarationId, Term)>,
    pub context: crate::ptc::Context,
    pub in_signature: bool,
}

impl PreparedRequired {
    pub(in crate::program) fn definition_identity_cursor(&self) -> u32 {
        self.world.package().definition_identity_cursor
    }

    /// Observe the carried invocation inputs and code without consulting a
    /// current Checker, copying the snapshot, or running its computations.
    /// Ambient abstract binders authorize transport; they are not input uses.
    pub(in crate::program) fn uses_declaration_inputs(
        &self,
        declarations: &[DeclarationId],
        steps: &mut u64,
        span: Span,
    ) -> Result<bool, Error> {
        if declarations.is_empty() {
            return Ok(false);
        }
        *steps = steps
            .checked_sub(declarations.len() as u64)
            .ok_or_else(|| error(span, "prepared input selection budget exhausted"))?;
        let declarations = declarations.iter().copied().collect::<BTreeSet<_>>();
        let terms = self
            .inputs
            .iter()
            .flat_map(|(_, domain, value)| std::iter::once(domain).chain(value))
            .chain(std::iter::once(&self.result_type))
            .chain(&self.scoped_arguments)
            .chain(self.invocation_assumptions.iter().flat_map(|requirement| {
                std::iter::once(&requirement.subject).chain(&requirement.interface_arguments)
            }));
        for term in terms {
            if !input_observation::term(term, steps, span)?.is_disjoint(&declarations) {
                return Ok(true);
            }
        }
        // The preparation has already selected its callable closure. Inspect
        // the contracts/defaults carried with those functions as well as their
        // signatures and bodies; do not scan the surrounding declaration world.
        for function in &self.world.package().functions {
            if !input_observation::function(function, steps, span)?.is_disjoint(&declarations) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(in crate::program) fn fork(&self) -> Self {
        Self {
            world: Arc::clone(&self.world),
            entry: self.entry,
            inputs: self.inputs.clone(),
            result_type: self.result_type.clone(),
            scoped_parameters: self.scoped_parameters.clone(),
            scoped_arguments: self.scoped_arguments.clone(),
            input_frames: self.input_frames.clone(),
            invocation_assumptions: self.invocation_assumptions.clone(),
            abstract_parameters: self.abstract_parameters.clone(),
            abstract_value_parameters: self.abstract_value_parameters.clone(),
            context: self.context.clone(),
            in_signature: self.in_signature,
        }
    }

    /// Supply only captured, pending declaration inputs. Check the complete
    /// assignment before mutating the snapshot; rejected input routing leaves
    /// the preparation available for a corrected invocation.
    pub(in crate::program) fn bind_inputs(
        &mut self,
        values: Vec<(n::LocalId, Term)>,
        span: Span,
    ) -> Result<(), Error> {
        let routes = inputs::routes(&self.inputs, &values, span)?;
        for (position, (_, value)) in routes.into_iter().zip(values) {
            self.inputs[position].2 = Some(value);
        }
        Ok(())
    }

    // Invocation uses only the checked snapshot and the caller's shared budget.
    // No current source-checker world participates in trait selection.
    pub(in crate::program) fn invoke(
        self,
        steps: &mut u64,
        span: Span,
    ) -> Result<(Value, Term), Error> {
        self.invoke_with_inputs(Vec::new(), steps, span)
    }

    pub(in crate::program) fn invoke_with_inputs(
        self,
        values: Vec<(n::LocalId, Term)>,
        steps: &mut u64,
        span: Span,
    ) -> Result<(Value, Term), Error> {
        self.invoke_in_world(
            values
                .into_iter()
                .map(|(binding, value)| (binding, InvocationValue::Normal(value)))
                .collect(),
            steps,
            span,
            None,
        )
    }

    /// Supply invocation-local values without exporting or rebuilding their
    /// graphs. Frozen captures remain normal inputs; only pending captures may
    /// be supplied. The preparation itself never retains a live graph.
    pub(in crate::program) fn invoke_with_live_inputs(
        self,
        values: Vec<(n::LocalId, Value)>,
        steps: &mut u64,
        span: Span,
    ) -> Result<(Value, Term), Error> {
        self.invoke_in_world(
            values
                .into_iter()
                .map(|(binding, value)| (binding, InvocationValue::Live(value)))
                .collect(),
            steps,
            span,
            None,
        )
    }

    pub(in crate::program) fn invoke_publishing(
        self,
        values: Vec<(n::LocalId, Value)>,
        steps: &mut u64,
        span: Span,
        package_identity: u64,
        next_identity: &mut u32,
    ) -> Result<((Value, Term), crate::definitions::PublishedDefinitions), Error> {
        let world = Arc::clone(&self.world);
        let context = self.context.clone();
        let assumptions = self.invocation_assumptions.clone();
        let solver = |predicate: &n::TypePredicate, fuel: &mut u64| {
            world.solve(&assumptions, &context, predicate, fuel)
        };
        let publisher = crate::definitions::AllocatedDefinitionPublisher::new_with_type_solver(
            world.package(),
            package_identity,
            *next_identity,
            usize::MAX,
            &solver,
        );
        let result = self.invoke_in_world(
            values
                .into_iter()
                .map(|(binding, value)| (binding, InvocationValue::Live(value)))
                .collect(),
            steps,
            span,
            Some(&publisher),
        );
        let diagnostic = publisher.last_diagnostic();
        let (definitions, next) = publisher.finish();
        *next_identity = next;
        let result = result.map_err(|failure| match diagnostic {
            Some(message) => error(span, format!("{failure}; {message}")),
            None => failure,
        })?;
        Ok((result, definitions))
    }

    fn invoke_in_world(
        self,
        values: Vec<(n::LocalId, InvocationValue)>,
        steps: &mut u64,
        span: Span,
        builder: Option<&dyn crate::definitions::StructBuildService>,
    ) -> Result<(Value, Term), Error> {
        let world = Arc::clone(&self.world);
        let package = world.package();
        let mut code = world.code();
        let build = |request: &comptime_interpretor::StructBuildRequest, fuel: &mut u64| {
            builder
                .expect("configured builder service")
                .prepare(request, fuel)
        };
        let build_newtype = |request: &comptime_interpretor::NewtypeBuildRequest,
                             fuel: &mut u64| {
            builder
                .expect("configured builder service")
                .prepare_newtype(request, fuel)
        };
        let build_enum = |request: &comptime_interpretor::EnumBuildRequest, fuel: &mut u64| {
            builder
                .expect("configured builder service")
                .prepare_enum(request, fuel)
        };
        let PreparedRequired {
            world: _,
            entry,
            inputs,
            result_type,
            scoped_parameters,
            scoped_arguments,
            input_frames,
            invocation_assumptions,
            abstract_parameters,
            abstract_value_parameters,
            context,
            in_signature,
        } = self;
        let inputs = inputs::complete(inputs, values, span)?;
        let invocation_type_context = &context;
        let scoped_context = if scoped_parameters.is_empty() {
            None
        } else {
            let mut context = invocation_type_context.theory_in_scope(input_frames.clone());
            for assumption in invocation_type_context.assumptions() {
                context.assume(assumption.clone());
            }
            Some(context)
        };
        let source_queries = crate::materialization::SourceInvocationQueries::in_source(
            DeclarationSource::Snapshot(&world.source),
            invocation_type_context,
            &invocation_assumptions,
        );
        let type_solver = |predicate: &n::TypePredicate, remaining: &mut u64| {
            use comptime_interpretor::ContractOutcome;
            let restored;
            let (predicate, proof_context) = if scoped_parameters.is_empty() {
                (predicate, invocation_type_context)
            } else {
                let Ok(value) = predicate.instantiate_parameters_in_scope(
                    &scoped_parameters,
                    &scoped_arguments,
                    &input_frames,
                ) else {
                    return ContractOutcome::Unknown;
                };
                restored = value;
                (&restored, scoped_context.as_ref().unwrap())
            };
            source_queries.check_in_context(
                predicate,
                proof_context,
                crate::ptc::Budget::default(),
                remaining,
            )
        };
        let associated_resolver =
            |access: &n::Expr, remaining: &mut u64| source_queries.associated(access, remaining);
        let method_resolver =
            |callee: &n::Expr, remaining: &mut u64| source_queries.resolve(callee, remaining);
        // Inputs are already complete. Type results use the ordinary invocation
        // and packet restoration path; no inference output is needed merely to
        // return comptime data. Identity consumers perform their own checks.
        let mut preparation_budget = Budget {
            steps: std::mem::take(steps),
            depth: 128,
        };
        let prepared = Interpreter::for_comptime_resolving_in_scope_in_session(
            &package,
            code.session(),
            &invocation_assumptions,
            &abstract_parameters,
            &abstract_value_parameters,
            comptime_interpretor::SourceResolvers {
                types: &type_solver,
                methods: &method_resolver,
                associated: &associated_resolver,
            },
            &mut preparation_budget,
        );
        *steps = preparation_budget.steps;
        let mut interpreter = prepared
            .map_err(|e| error(span, format!("required comptime preparation failed: {e}")))?;
        if builder.is_some() {
            interpreter = interpreter
                .with_struct_builder(&build)
                .with_enum_builder(&build_enum)
                .with_newtype_builder(&build_newtype);
        }
        let value = interpreter.with_code_session(code.session(), |vm| {
            vm.start_prepared_invocation(entry, inputs)?
                .run_to_completion()
        });
        *steps = interpreter.remaining_steps();
        let value=value.map_err(|e| {
            if e.kind == comptime_interpretor::ErrorKind::MissingBinding
                && in_signature {
                return error(span,"type computation depends on an unavailable function input; invocation-time type elaboration is not yet implemented");
            }
            error(span,format!("required comptime evaluation failed: {e}"))
        })?;
        let value = restore_required_packet(
            steps,
            value,
            &scoped_parameters,
            &scoped_arguments,
            &input_frames,
            span,
            0,
        )?;
        Ok((value, result_type))
    }
}

impl std::fmt::Display for PreparedRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(prepared-required (entry {}) (result-type {}) (inputs",
            self.entry, self.result_type
        )?;
        for (binding, ty, value) in &self.inputs {
            write!(f, " (input {binding} {ty} ")?;
            match value {
                Some(value) => write!(f, "(available {value})")?,
                None => write!(f, "(pending)")?,
            }
            write!(f, ")")?;
        }
        write!(f, ") (scope-substitution")?;
        for (parameter, argument) in self.scoped_parameters.iter().zip(&self.scoped_arguments) {
            write!(
                f,
                " (binding {} {} {argument})",
                parameter.declaration, parameter.sort
            )?;
        }
        write!(f, ") (input-frames")?;
        for frame in &self.input_frames {
            write!(f, " (frame")?;
            for sort in frame {
                write!(f, " {sort}")?;
            }
            write!(f, ")")?;
        }
        write!(f, ") (assumptions")?;
        for assumption in &self.invocation_assumptions {
            write!(f, " {assumption}")?;
        }
        write!(f, ") (abstract-parameters")?;
        for parameter in &self.abstract_parameters {
            write!(f, " {parameter}")?;
        }
        write!(f, ") (abstract-value-parameters")?;
        for (id, domain) in &self.abstract_value_parameters {
            write!(f, " ({id} {domain})")?;
        }
        write!(
            f,
            ") (in-signature {}) (context {}) (source {})",
            self.in_signature, self.context, self.world.source
        )?;
        write!(f, ")")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl PreparedRequired {
        fn edit_package(&mut self, edit: impl FnOnce(&mut n::Package)) {
            assert_eq!(Arc::strong_count(&self.world), 1, "unique test fixture");
            let mut package = self.world.package().clone();
            edit(&mut package);
            self.world = RequiredWorld::new(package);
        }
    }

    fn literal_preparation(type_result: bool) -> PreparedRequired {
        let entry = DeclarationId {
            package: 7,
            local: 11,
        };
        let result_type = p(if type_result { P::Type } else { P::Bool });
        let body = n::Expr {
            ty: result_type.clone(),
            source: n::SourceRange { start: 0, end: 1 },
            kind: if type_result {
                K::TypeValue(p(P::U8))
            } else {
                K::Literal(n::Literal::Bool(true))
            },
        };
        let package = n::Package {
            functions: vec![n::Function {
                interface: nir::program::FunctionDeclaration {
                    phase: n::FunctionPhase::Comptime,
                    inline: false,
                    pure: true,
                    is_unsafe: false,
                    preconditions: vec![],
                    postconditions: vec![],
                    result_binding: None,
                    entry_snapshots: vec![],
                    capabilities: vec![],
                    requirements: vec![],
                    type_parameters: vec![],
                    declaration: entry,
                    name: "prepared".into(),
                    signature: Term::Function {
                        parameters: vec![],
                        result: Box::new(result_type.clone()),
                    },
                    parameters: vec![],
                    defaults: vec![],
                },
                body,
            }],
            ..Default::default()
        };
        PreparedRequired {
            world: RequiredWorld::new(package),
            entry,
            inputs: vec![],
            result_type,
            scoped_parameters: vec![],
            scoped_arguments: vec![],
            input_frames: vec![],
            invocation_assumptions: vec![],
            abstract_parameters: vec![],
            abstract_value_parameters: vec![],
            context: crate::ptc::Context::default(),
            in_signature: false,
        }
    }

    #[test]
    fn unused_query_services_do_not_prepare_a_declaration_directory() {
        for type_result in [false, true] {
            let prepared = literal_preparation(type_result);
            let world = prepared.world.clone();
            let result = prepared.invoke(&mut 100_000, Span::default()).unwrap();
            assert_eq!(
                result.0,
                if type_result {
                    Value::Type(p(P::U8))
                } else {
                    Value::Bool(true)
                }
            );
            assert!(world.code.lock().unwrap().is_some());
            // Execution validated the source, but no lookup asked for an index.
            let mut empty = crate::ptc::Budget {
                steps: 0,
                ..Default::default()
            };
            assert!(world.source.prepare(&mut empty).is_err());
        }
    }

    #[test]
    fn required_type_results_preserve_opaque_identity_and_restore_the_caller_scope() {
        let declaration = DeclarationId {
            package: 7,
            local: 90,
        };
        let rigid = Term::Rigid {
            declaration,
            sort: Sort::Type,
        };
        let bound = Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        };
        for scoped in [false, true] {
            let mut prepared = literal_preparation(true);
            prepared.abstract_parameters.push(declaration);
            prepared.edit_package(|package| {
                package.functions[0].body.kind = K::TypeValue(rigid.clone());
            });
            if scoped {
                prepared.scoped_parameters.push(nir::DeclarationParameter {
                    declaration,
                    sort: Sort::Type,
                });
                prepared.scoped_arguments.push(bound.clone());
                prepared.input_frames.push(vec![Sort::Type]);
            }
            let mut steps = 100_000;
            assert_eq!(
                prepared.fork().invoke(&mut steps, Span::default()).unwrap(),
                (
                    Value::Type(if scoped { bound.clone() } else { rigid.clone() }),
                    p(P::Type)
                )
            );
            assert!(steps < 100_000);
            // The retained source/code does not authorize an unknown Type input.
            let mut denied = prepared.fork();
            denied.abstract_parameters.clear();
            assert!(denied.invoke(&mut 100_000, Span::default()).is_err());
            assert!(prepared.invoke(&mut 100_000, Span::default()).is_ok());
        }
    }

    #[test]
    fn first_query_spends_its_budget_and_retries_without_partial_publication() {
        use comptime_interpretor::ContractOutcome;
        let prepared = literal_preparation(false);
        let predicate = n::TypePredicate::Equal(p(P::Bool), p(P::Bool));
        let mut tiny = 1;
        assert_eq!(
            prepared
                .world
                .solve(&[], &prepared.context, &predicate, &mut tiny),
            ContractOutcome::Unknown
        );
        assert_eq!(tiny, 0);
        let mut empty = crate::ptc::Budget {
            steps: 0,
            ..Default::default()
        };
        assert!(prepared.world.source.prepare(&mut empty).is_err());
        let mut remaining = 100_000;
        assert_eq!(
            prepared
                .world
                .solve(&[], &prepared.context, &predicate, &mut remaining),
            ContractOutcome::Satisfied
        );
        assert!(remaining < 100_000);
        prepared.world.source.prepare(&mut empty).unwrap();
        assert_eq!(empty.steps, 0);
        assert!(prepared.world.code.lock().unwrap().is_none());
        assert_eq!(
            prepared.invoke(&mut remaining, Span::default()).unwrap().0,
            Value::Bool(true)
        );
    }

    #[test]
    fn invalid_source_metadata_is_rejected_by_queries_and_execution() {
        let mut prepared = literal_preparation(false);
        prepared.edit_package(|package| package.functions.push(package.functions[0].clone()));
        let mut budget = crate::ptc::Budget::default();
        let before = budget.steps;
        let failure = prepared.world.source.prepare(&mut budget).err().unwrap();
        assert!(
            failure
                .to_string()
                .contains("duplicate callable declaration"),
            "{failure}"
        );
        assert!(budget.steps < before);
        let failure = prepared
            .invoke(&mut 100_000, Span::default())
            .err()
            .unwrap();
        assert!(
            failure.message.contains("duplicate function declaration"),
            "{failure}"
        );
        // Rejection of this owner grants no membership in another source world.
        assert_eq!(
            literal_preparation(false)
                .invoke(&mut 100_000, Span::default())
                .unwrap()
                .0,
            Value::Bool(true)
        );
    }

    #[test]
    fn required_execution_populates_reusable_code_for_both_result_paths() {
        for type_result in [false, true] {
            let prepared = literal_preparation(type_result);
            let expected = prepared
                .fork()
                .invoke(&mut 100_000, Span::default())
                .unwrap()
                .0;
            let queries = crate::materialization::SourceInvocationQueries::in_source(
                DeclarationSource::Snapshot(&prepared.world.source),
                &prepared.context,
                &[],
            );
            let solver = |predicate: &n::TypePredicate, fuel: &mut u64| {
                queries.check(predicate, Default::default(), fuel)
            };
            let methods = |callee: &n::Expr, fuel: &mut u64| queries.resolve(callee, fuel);
            let associated = |access: &n::Expr, fuel: &mut u64| queries.associated(access, fuel);
            let mut code = prepared.world.code();
            let mut vm = Interpreter::for_comptime_resolving_in_scope_in_session(
                prepared.world.package(),
                code.session(),
                &[],
                &[],
                &[],
                comptime_interpretor::SourceResolvers {
                    types: &solver,
                    methods: &methods,
                    associated: &associated,
                },
                &mut Budget::default(),
            )
            .unwrap();
            assert_eq!(
                vm.with_code_session(code.session(), |vm| { vm.invoke(prepared.entry, vec![]) })
                    .unwrap(),
                expected
            );
            assert!(vm.bytecode_invocations() > 0);
            assert_eq!(
                vm.bytecode_compilations(),
                0,
                "required path did not retain code"
            );
            drop(vm);
            drop(code);
            // A warm cache is never an execution allowance or a result cache.
            assert!(prepared.fork().invoke(&mut 0, Span::default()).is_err());
            assert_eq!(
                prepared
                    .fork()
                    .invoke(&mut 100_000, Span::default())
                    .unwrap()
                    .0,
                expected
            );
        }
    }

    #[test]
    fn warmed_required_code_cannot_retain_invocation_trait_authority() {
        for type_result in [false, true] {
            let mut prepared = literal_preparation(type_result);
            let interface = DeclarationId {
                package: 7,
                local: 90,
            };
            let premise = n::TraitRequirement {
                interface,
                interface_arguments: vec![],
                subject: p(P::Bool),
            };
            prepared.edit_package(|package| {
                package.traits.push(n::Trait {
                    declaration: interface,
                    self_parameter: DeclarationId {
                        package: 7,
                        local: 91,
                    },
                    name: "Granted".into(),
                    comptime_only: false,
                    type_parameters: vec![],
                    parameter_types: vec![],
                    requirements: vec![],
                    capabilities: vec![],
                    parents: vec![],
                    associated: vec![],
                    methods: vec![],
                });
                package.functions[0].capabilities.push(premise.clone());
            });
            prepared.context = crate::ptc::Context::from_package(prepared.world.package()).unwrap();
            let mut granted = prepared.fork();
            granted.invocation_assumptions.push(premise);
            let expected = granted
                .fork()
                .invoke(&mut 100_000, Span::default())
                .unwrap()
                .0;
            let failure = prepared
                .fork()
                .invoke(&mut 100_000, Span::default())
                .unwrap_err();
            assert!(failure.message.contains("TypeContract"), "{failure}");
            assert_eq!(
                granted.invoke(&mut 100_000, Span::default()).unwrap().0,
                expected
            );
            assert!(prepared.invoke(&mut 100_000, Span::default()).is_err());
        }
    }

    #[test]
    fn phase_evidence_survives_reentrant_code_leases() {
        let mut prepared = literal_preparation(false);
        let mut package = prepared.world.package().clone();
        let body = package.functions[0].body.clone();
        package.functions[0].body.kind = K::Block {
            statements: vec![body.clone(); 1000],
            result: Some(Box::new(body)),
        };
        let package = Arc::new(package);
        let mut fuel = 100_000;
        let phase = nir::phases::Inspection::new(&package)
            .check_package_with_evidence(&mut fuel)
            .unwrap();
        let phase_cost = 100_000 - fuel;
        prepared.world = RequiredWorld::with_phase(package.clone(), &phase).unwrap();
        let outer = prepared.world.code();
        for _ in 0..2 {
            let mut nested = prepared.world.code();
            let mut budget = comptime_interpretor::Budget {
                steps: phase_cost as u64 - 1,
                ..Default::default()
            };
            comptime_interpretor::Interpreter::for_comptime_in_session(
                &package,
                nested.session(),
                &mut budget,
            )
            .unwrap();
            assert!(budget.steps > 0);
        }
        drop(outer);
        assert_eq!(
            prepared.invoke(&mut 100_000, Span::default()).unwrap().0,
            Value::Bool(true)
        );
    }

    #[test]
    fn required_code_leases_allow_reentry_and_recover_after_unwind() {
        let prepared = literal_preparation(true);
        let world = prepared.world.clone();
        let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _outer = world.code();
            assert!(world.code.lock().unwrap().is_none());
            assert_eq!(
                prepared
                    .fork()
                    .invoke(&mut 100_000, Span::default())
                    .unwrap()
                    .0,
                Value::Type(p(P::U8))
            );
            assert!(world.code.lock().unwrap().is_some());
            panic!("host failure while the outer cache is leased");
        }));
        assert!(failure.is_err());
        assert!(!world.code.is_poisoned());
        assert_eq!(
            prepared.invoke(&mut 100_000, Span::default()).unwrap().0,
            Value::Type(p(P::U8))
        );
        assert!(world.code.lock().unwrap().is_some());
    }

    #[test]
    fn prepared_forks_share_declarations_and_isolate_input_assignments() {
        let span = Span::default();
        let mut prepared = literal_preparation(false);
        let binding = n::LocalId {
            owner: 11,
            index: 0,
        };
        prepared.edit_package(|package| {
            let function = &mut package.functions[0];
            function.parameters = vec![binding];
            function.signature = Term::Function {
                parameters: vec![nir::Parameter {
                    sort: Sort::ComptimeValue,
                    ty: p(P::Bool),
                }],
                result: Box::new(p(P::Bool)),
            };
            function.body.kind = K::Local(binding);
        });
        prepared.inputs = vec![(binding, p(P::Bool), None)];
        let owner = Arc::downgrade(&prepared.world);
        let mut first = prepared.fork();
        let mut second = prepared.fork();
        assert!(Arc::ptr_eq(&prepared.world, &first.world));
        assert!(std::ptr::eq(
            first.world.capabilities(&[], &first.context).index,
            second.world.capabilities(&[], &second.context).index
        ));
        assert!(
            first
                .bind_inputs(
                    vec![(binding, Term::Bool(true)), (binding, Term::Bool(false))],
                    span
                )
                .is_err()
        );
        assert_eq!(first.inputs, prepared.inputs);
        first
            .bind_inputs(vec![(binding, Term::Bool(true))], span)
            .unwrap();
        assert!(prepared.inputs[0].2.is_none());
        assert!(second.inputs[0].2.is_none());
        second
            .bind_inputs(vec![(binding, Term::Bool(false))], span)
            .unwrap();
        drop(prepared);
        assert!(owner.upgrade().is_some());
        assert_eq!(
            first.invoke(&mut 100_000, span).unwrap().0,
            Value::Bool(true)
        );
        assert!(owner.upgrade().is_some());
        assert_eq!(
            second.invoke(&mut 100_000, span).unwrap().0,
            Value::Bool(false)
        );
        assert!(owner.upgrade().is_none());
    }

    #[test]
    fn prepared_invocations_reuse_the_directory_without_reusing_proof_authority() {
        use comptime_interpretor::ContractOutcome;
        for type_result in [false, true] {
            let mut prepared = literal_preparation(type_result);
            prepared.edit_package(|package| {
                let prototype = package.functions[0].clone();
                for local in 100..2148 {
                    let mut function = prototype.clone();
                    function.declaration.local = local;
                    package.functions.push(function);
                }
            });
            // Measure a cold invocation before adding a declaration proof
            // demand. Replacing the immutable world starts a fresh code cache.
            let mut baseline = 100_000;
            prepared
                .fork()
                .invoke(&mut baseline, Span::default())
                .unwrap();
            let allowance = 100_000 - baseline + 1000;
            prepared.edit_package(|package| {
                package.functions[0].requirements =
                    vec![n::TypePredicate::Equal(p(P::Bool), p(P::Bool))];
            });
            let source = DeclarationSource::Snapshot(&prepared.world.source);
            let context = &prepared.context;
            prepared
                .world
                .source
                .prepare(&mut crate::ptc::Budget::default())
                .unwrap();
            // A fresh scope only retains the completed directory. Rebuilding
            // this declaration set cannot fit within the same allowance.
            let scope = source
                .world(
                    &mut crate::ptc::Budget {
                        steps: 1,
                        ..Default::default()
                    },
                    Some((context, &[])),
                )
                .unwrap();
            assert!(std::ptr::eq(
                scope.index(),
                prepared.world.capabilities(&[], context).index
            ));
            assert!(
                DeclarationSource::Borrowed(prepared.world.package())
                    .world(
                        &mut crate::ptc::Budget {
                            steps: 1000,
                            ..Default::default()
                        },
                        Some((context, &[])),
                    )
                    .is_err()
            );

            let bound = Term::Bound {
                depth: 0,
                index: 0,
                sort: Sort::Type,
            };
            let predicate = n::TypePredicate::Equal(bound.clone(), p(P::Bool));
            let mut with_premise = context.theory_in_scope(vec![vec![Sort::Type]]);
            with_premise.assume(crate::ptc::Formula::equal(bound, p(P::Bool)));
            let queries =
                crate::materialization::SourceInvocationQueries::in_source(source, context, &[]);
            for _ in 0..2 {
                assert_eq!(
                    prepared
                        .world
                        .solve(&[], &with_premise, &predicate, &mut 100_000),
                    ContractOutcome::Satisfied
                );
                assert_eq!(
                    queries.check(&predicate, crate::ptc::Budget::default(), &mut 100_000),
                    ContractOutcome::Unknown
                );
            }
            drop(queries);
            drop(scope);
            assert!(prepared.fork().invoke(&mut 0, Span::default()).is_err());
            for _ in 0..2 {
                let mut remaining = allowance;
                let actual = prepared
                    .fork()
                    .invoke(&mut remaining, Span::default())
                    .unwrap();
                let expected = if type_result {
                    Value::Type(p(P::U8))
                } else {
                    Value::Bool(true)
                };
                assert_eq!(actual.0, expected);
                assert!(remaining < allowance);
            }
        }
    }

    #[test]
    fn prepared_worlds_keep_same_numbered_declarations_separate() {
        let span = Span::default();
        let first = literal_preparation(false);
        let mut second = literal_preparation(false);
        second.edit_package(|package| {
            package.functions[0].body.kind = K::Literal(n::Literal::Bool(false));
        });
        assert_eq!(first.entry, second.entry);
        assert!(!Arc::ptr_eq(&first.world, &second.world));
        for _ in 0..3 {
            assert_eq!(
                first.fork().invoke(&mut 100_000, span).unwrap().0,
                Value::Bool(true)
            );
            assert_eq!(
                second.fork().invoke(&mut 100_000, span).unwrap().0,
                Value::Bool(false)
            );
        }
    }

    fn frozen_types(fields: Vec<nir::Term>) -> nir::Term {
        nir::Term::NominalValue {
            ty: Box::new(nir::Term::NormalForm {
                head: nir::DeclarationId {
                    package: 7,
                    local: 12,
                },
                sort: nir::Sort::Type,
                arguments: vec![nir::Term::Primitive(nir::Primitive::Type)],
            }),
            variant: None,
            fields,
        }
    }

    #[test]
    fn prepared_input_observation_keeps_carried_contract_and_operand_dependencies() {
        let input = DeclarationId {
            package: 7,
            local: 90,
        };
        let rigid = Term::Rigid {
            declaration: input,
            sort: Sort::Type,
        };
        let placements: &[fn(&mut PreparedRequired, Term)] = &[
            |p, term| p.result_type = term,
            |p, term| {
                p.inputs.push((
                    n::LocalId {
                        owner: 11,
                        index: 0,
                    },
                    term,
                    None,
                ))
            },
            |p, term| {
                p.inputs.push((
                    n::LocalId {
                        owner: 11,
                        index: 0,
                    },
                    p.result_type.clone(),
                    Some(term),
                ))
            },
            |p, term| p.scoped_arguments.push(term),
            |p, term| {
                p.invocation_assumptions.push(n::TraitRequirement {
                    subject: term,
                    interface: p.entry,
                    interface_arguments: vec![],
                })
            },
            |p, term| {
                p.invocation_assumptions.push(n::TraitRequirement {
                    subject: p.result_type.clone(),
                    interface: p.entry,
                    interface_arguments: vec![term],
                })
            },
            |p, term| p.edit_package(|package| package.functions[0].signature = term),
            |p, term| {
                p.edit_package(|package| package.functions[0].body.kind = K::StagedValue(term))
            },
            |p, term| {
                p.edit_package(|package| {
                    package.functions[0]
                        .requirements
                        .push(n::TypePredicate::Not(Box::new(n::TypePredicate::Equal(
                            term,
                            Term::Primitive(P::Bool),
                        ))))
                })
            },
            |p, term| {
                let entry = p.entry;
                p.edit_package(|package| {
                    package.functions[0].capabilities.push(n::TraitRequirement {
                        subject: term,
                        interface: entry,
                        interface_arguments: vec![],
                    })
                });
            },
            |p, term| {
                p.edit_package(|package| {
                    let mut predicate = package.functions[0].body.clone();
                    predicate.kind = K::TypeValue(term);
                    package.functions[0].preconditions.push(predicate);
                })
            },
            |p, term| {
                p.edit_package(|package| {
                    let mut predicate = package.functions[0].body.clone();
                    predicate.kind = K::TypeValue(term);
                    package.functions[0].postconditions.push(predicate);
                })
            },
            |p, term| {
                p.edit_package(|package| {
                    let mut default = package.functions[0].body.clone();
                    default.kind = K::TypeValue(term);
                    package.functions[0].defaults.push((0, default));
                })
            },
        ];
        for (index, place) in placements.iter().enumerate() {
            let mut prepared = literal_preparation(true);
            assert!(
                !prepared
                    .uses_declaration_inputs(&[input], &mut 1000, Span::default())
                    .unwrap()
            );
            place(&mut prepared, rigid.clone());
            let before = prepared.to_string();
            assert!(
                prepared
                    .uses_declaration_inputs(&[input], &mut 1000, Span::default())
                    .unwrap(),
                "placement {index}"
            );
            assert_eq!(prepared.to_string(), before);
        }
    }

    #[test]
    fn prepared_input_observation_ignores_ambient_authority_and_declaration_names() {
        let input = DeclarationId {
            package: 7,
            local: 90,
        };
        let mut prepared = literal_preparation(true);
        prepared.abstract_parameters.push(input);
        prepared.abstract_value_parameters.push((input, p(P::Bool)));
        prepared.scoped_parameters.push(nir::DeclarationParameter {
            declaration: input,
            sort: Sort::Type,
        });
        prepared.scoped_arguments.push(p(P::U8));
        prepared.edit_package(|package| {
            let function = &mut package.functions[0];
            function.type_parameters.push(input);
            function.capabilities.push(n::TraitRequirement {
                subject: p(P::Bool),
                interface: input,
                interface_arguments: vec![],
            });
        });
        assert!(
            !prepared
                .uses_declaration_inputs(&[input], &mut 1000, Span::default())
                .unwrap()
        );
        assert!(
            !prepared
                .uses_declaration_inputs(&[], &mut 0, Span::default())
                .unwrap()
        );
        let mut exhausted = 3;
        assert!(
            prepared
                .uses_declaration_inputs(&[input], &mut exhausted, Span::default())
                .is_err()
        );
        assert!(exhausted < 3);
        assert!(
            !prepared
                .uses_declaration_inputs(&[input], &mut 1000, Span::default())
                .unwrap()
        );
    }

    #[test]
    fn prepared_input_observation_preserves_shared_code_and_recovers_from_exhaustion() {
        let input = DeclarationId {
            package: 7,
            local: 90,
        };
        let mut prepared = literal_preparation(true);
        let mut body = prepared.world.package().functions[0].body.clone();
        body.kind = K::StagedValue(Term::Rigid {
            declaration: input,
            sort: Sort::Type,
        });
        let body = std::sync::Arc::new(body);
        prepared.edit_package(|package| {
            package.functions[0].body.kind = K::Closure {
                parameters: vec![],
                captures: vec![],
                body: body.clone(),
            };
        });
        let before = prepared.to_string();
        let mut exhausted = 6;
        assert!(
            prepared
                .uses_declaration_inputs(&[input], &mut exhausted, Span::default())
                .is_err()
        );
        assert!(exhausted < 6);
        assert_eq!(prepared.to_string(), before);
        assert!(
            prepared
                .uses_declaration_inputs(&[input], &mut 1000, Span::default())
                .unwrap()
        );
        let K::Closure { body: observed, .. } = &prepared.world.package().functions[0].body.kind
        else {
            panic!()
        };
        assert!(std::sync::Arc::ptr_eq(&body, observed));
        assert_eq!(std::sync::Arc::strong_count(&body), 2);
        assert!(matches!(observed.kind, K::StagedValue(_)));
        assert_eq!(prepared.to_string(), before);
    }

    #[test]
    fn required_value_and_type_proofs_consume_the_invocation_budget() {
        for type_result in [false, true] {
            let make = || {
                let mut prepared = literal_preparation(type_result);
                prepared.edit_package(|package| {
                    package.functions[0].requirements = vec![n::TypePredicate::And(
                        (0..64)
                            .map(|_| {
                                n::TypePredicate::Not(Box::new(n::TypePredicate::Equal(
                                    p(P::Bool),
                                    p(P::U8),
                                )))
                            })
                            .collect(),
                    )];
                });
                prepared
            };
            let mut baseline = 100_000;
            literal_preparation(type_result)
                .invoke(&mut baseline, Span::default())
                .unwrap();
            let mut enough = 100_000;
            make().invoke(&mut enough, Span::default()).unwrap();
            assert!(
                enough + 64 < baseline,
                "baseline={baseline}, with proof={enough}"
            );
            let mut limited = 32;
            assert!(make().invoke(&mut limited, Span::default()).is_err());
            assert!(limited < 32);
            let mut recovery = 100_000;
            make().invoke(&mut recovery, Span::default()).unwrap();
        }
    }

    #[test]
    fn type_invocations_transport_abstract_inputs_but_do_not_observe_them() {
        let declaration = DeclarationId {
            package: 7,
            local: 90,
        };
        let binding = n::LocalId {
            owner: 11,
            index: 0,
        };
        for observe in [false, true] {
            let mut prepared = literal_preparation(true);
            prepared.abstract_value_parameters = vec![(declaration, p(P::Bool))];
            prepared.inputs = vec![(
                binding,
                p(P::Bool),
                Some(Term::Rigid {
                    declaration,
                    sort: Sort::ComptimeValue,
                }),
            )];
            prepared.edit_package(|package| {
                let function = &mut package.functions[0];
                function.parameters = vec![binding];
                function.signature = Term::Function {
                    parameters: vec![Parameter {
                        sort: Sort::Runtime,
                        ty: p(P::Bool),
                    }],
                    result: Box::new(p(P::Type)),
                };
                if observe {
                    let result = function.body.clone();
                    function.body.kind = K::If {
                        condition: Box::new(n::Expr {
                            source: result.source,
                            ty: p(P::Bool),
                            kind: K::Local(binding),
                        }),
                        then_branch: Box::new(result.clone()),
                        else_branch: Some(Box::new(result)),
                    };
                }
            });
            let mut steps = 10_000;
            let result = prepared.invoke(&mut steps, Span::default());
            assert!(steps < 10_000);
            if observe {
                assert!(format!("{:?}", result.unwrap_err()).contains("AbstractValueUnresolved"));
            } else {
                assert_eq!(result.unwrap(), (Value::Type(p(P::U8)), p(P::Type)));
            }
        }
    }

    #[test]
    fn abstract_type_invocation_failure_is_terminal_and_does_not_poison_other_calls() {
        use crate::type_computations::{PreparedInput, Session, State};
        let declaration = DeclarationId {
            package: 7,
            local: 90,
        };
        let binding = n::LocalId {
            owner: 11,
            index: 0,
        };
        let mut prepared = literal_preparation(true);
        prepared.edit_package(|package| {
            let function = &mut package.functions[0];
            function.parameters = vec![binding];
            function.signature = Term::Function {
                parameters: vec![Parameter {
                    sort: Sort::Runtime,
                    ty: p(P::Bool),
                }],
                result: Box::new(p(P::Type)),
            };
            let output = function.body.clone();
            function.body.kind = K::If {
                condition: Box::new(n::Expr {
                    source: output.source,
                    ty: p(P::Bool),
                    kind: K::Local(binding),
                }),
                then_branch: Box::new(output.clone()),
                else_branch: Some(Box::new(output)),
            };
        });
        let mut context = crate::InferenceContext::default();
        let mut session = Session::new_in_scope_in_budget(
            prepared.world.package(),
            &context,
            &mut Budget {
                steps: 10_000,
                depth: 128,
            },
            &[],
            &[],
            &[(declaration, p(P::Bool))],
            None,
        )
        .unwrap();
        let mut failed = Vec::new();
        for (id, domain) in [
            (declaration, p(P::Bool)),
            (
                DeclarationId {
                    package: 7,
                    local: 91,
                },
                p(P::Bool),
            ),
            (declaration, p(P::U8)),
        ] {
            let output = session
                .enqueue_prepared(
                    &mut context,
                    prepared.entry,
                    vec![PreparedInput {
                        ty: crate::InferenceTerm::Stable(p(P::Bool)),
                        value: InvocationValue::Live(Value::AbstractValue {
                            declaration: id,
                            ty: domain,
                        }),
                    }],
                )
                .unwrap();
            assert!(matches!(
                session.resume(&mut context, output).unwrap(),
                State::Failed(_)
            ));
            assert!(context.freeze(&crate::InferenceTerm::Meta(output)).is_err());
            let remaining = session.remaining_steps();
            let diagnostic = format!("{}", session.state(output).unwrap());
            assert_eq!(
                format!("{}", session.resume(&mut context, output).unwrap()),
                diagnostic
            );
            assert_eq!(session.remaining_steps(), remaining);
            failed.push(output);
        }
        let output = session
            .enqueue_prepared(
                &mut context,
                prepared.entry,
                vec![PreparedInput {
                    ty: crate::InferenceTerm::Stable(p(P::Bool)),
                    value: InvocationValue::Live(Value::Bool(true)),
                }],
            )
            .unwrap();
        assert_eq!(
            session.resume(&mut context, output).unwrap(),
            &State::Complete(p(P::U8))
        );
        assert_eq!(
            context.freeze(&crate::InferenceTerm::Meta(output)).unwrap(),
            p(P::U8)
        );
        for output in failed {
            assert!(matches!(session.state(output), Some(State::Failed(_))));
        }
    }

    #[test]
    fn rejected_scope_preparation_debits_the_shared_budget_on_both_paths() {
        for type_result in [false, true] {
            let mut prepared = literal_preparation(type_result);
            let id = DeclarationId {
                package: 7,
                local: 90,
            };
            prepared.abstract_parameters = vec![id];
            prepared.abstract_value_parameters = vec![(id, p(P::Bool))];
            let mut steps = 1000;
            assert!(prepared.invoke(&mut steps, Span::default()).is_err());
            assert!(steps < 1000, "failed scope setup refunded its work");
        }
    }

    #[test]
    fn prepared_owned_inputs_keep_writeback_in_type_and_data_calls() {
        for type_result in [false, true] {
            let mut prepared = literal_preparation(type_result);
            let binding = n::LocalId {
                owner: 11,
                index: 0,
            };
            let result_type = prepared.result_type.clone();
            prepared.edit_package(|package| {
                let function = &mut package.functions[0];
                function.parameters = vec![binding];
                function.signature = Term::Function {
                    parameters: vec![Parameter {
                        sort: Sort::Runtime,
                        ty: p(P::Bool),
                    }],
                    result: Box::new(result_type),
                };
                let result = function.body.clone();
                function.body.kind = K::Block {
                    statements: vec![n::Expr {
                        source: result.source,
                        ty: p(P::Void),
                        kind: K::Assign {
                            place: n::Place {
                                root: binding,
                                fields: vec![],
                            },
                            value: Box::new(n::Expr {
                                source: result.source,
                                ty: p(P::Bool),
                                kind: K::Literal(n::Literal::Bool(false)),
                            }),
                        },
                    }],
                    result: Some(Box::new(result)),
                };
            });
            prepared.inputs = vec![(binding, p(P::Bool), None)];
            let owner = comptime_interpretor::Storage::new(Value::Bool(true));
            let mut steps = 10_000;
            prepared
                .invoke_with_live_inputs(
                    vec![(binding, Value::Storage(owner.clone()))],
                    &mut steps,
                    Span::default(),
                )
                .unwrap();
            assert_eq!(owner.read().unwrap(), Value::Bool(false));
            assert!(steps < 10_000);
        }
    }

    #[test]
    fn prepared_invocation_needs_no_source_checker_and_charges_both_execution_paths() {
        for type_result in [false, true] {
            let prepared = literal_preparation(type_result);
            let mut steps = 1000;
            let (value, ty) = prepared.invoke(&mut steps, Span::default()).unwrap();
            assert!(steps < 1000);
            if type_result {
                assert!(matches!(value, Value::Type(ref value) if *value == p(P::U8)));
                assert_eq!(ty, p(P::Type));
            } else {
                assert!(matches!(value, Value::Bool(true)));
                assert_eq!(ty, p(P::Bool));
            }
            let mut exhausted = 0;
            assert!(
                literal_preparation(type_result)
                    .invoke(&mut exhausted, Span::default())
                    .is_err()
            );
            assert_eq!(exhausted, 0);
        }
    }

    fn pending_boolean_preparation() -> (PreparedRequired, n::LocalId) {
        let mut prepared = literal_preparation(false);
        let binding = n::LocalId {
            owner: 11,
            index: 0,
        };
        prepared.edit_package(|package| {
            let function = &mut package.functions[0];
            function.parameters = vec![binding];
            function.signature = Term::Function {
                parameters: vec![Parameter {
                    sort: Sort::ComptimeValue,
                    ty: p(P::Bool),
                }],
                result: Box::new(p(P::Bool)),
            };
            function.body.kind = K::Local(binding);
        });
        prepared.inputs = vec![(binding, p(P::Bool), None)];
        (prepared, binding)
    }

    #[test]
    fn pending_source_inputs_bind_atomically_before_invocation() {
        let span = Span::default();
        let (pending, _) = pending_boolean_preparation();
        let mut steps = 1000;
        assert!(
            pending
                .invoke(&mut steps, span)
                .unwrap_err()
                .message
                .contains("unavailable function input")
        );
        assert_eq!(steps, 1000, "an unavailable input must not start execution");
        let (mut pending, binding) = pending_boolean_preparation();
        assert!(pending.to_string().contains("(pending)"));
        assert!(
            pending
                .bind_inputs(
                    vec![(binding, Term::Bool(true)), (binding, Term::Bool(false))],
                    span
                )
                .is_err()
        );
        assert!(pending.inputs[0].2.is_none());
        let foreign = n::LocalId {
            owner: binding.owner + 1,
            ..binding
        };
        assert!(
            pending
                .bind_inputs(
                    vec![(binding, Term::Bool(true)), (foreign, Term::Bool(false))],
                    span
                )
                .is_err()
        );
        assert!(pending.inputs[0].2.is_none());
        pending
            .bind_inputs(vec![(binding, Term::Bool(false))], span)
            .unwrap();
        assert!(
            pending
                .bind_inputs(vec![(binding, Term::Bool(true))], span)
                .is_err()
        );
        assert!(
            matches!(pending.invoke(&mut steps, span).unwrap(), (Value::Bool(false), ty) if ty == p(P::Bool))
        );
        assert!(steps < 1000);
        let (mut pending, binding) = pending_boolean_preparation();
        pending
            .bind_inputs(vec![(binding, p(P::U8))], span)
            .unwrap();
        assert!(
            pending.invoke(&mut 1000, span).is_err(),
            "binding a value does not bypass input domain checking"
        );
    }

    #[test]
    fn frozen_and_live_routing_reject_the_same_invalid_packets_before_execution() {
        let span = Span::default();
        for case in 0..5 {
            let (mut prepared, binding) = pending_boolean_preparation();
            let foreign = n::LocalId {
                owner: binding.owner + 1,
                ..binding
            };
            let supplied = match case {
                0 => vec![],
                1 => vec![(binding, true), (binding, false)],
                2 => vec![(binding, true), (foreign, false)],
                3 => {
                    prepared
                        .bind_inputs(vec![(binding, Term::Bool(false))], span)
                        .unwrap();
                    vec![(binding, true)]
                }
                _ => {
                    prepared.inputs.push(prepared.inputs[0].clone());
                    vec![(binding, true)]
                }
            };
            let before = prepared.to_string();
            let mut frozen_steps = 1000;
            let frozen = prepared
                .fork()
                .invoke_with_inputs(
                    supplied
                        .iter()
                        .map(|(id, value)| (*id, Term::Bool(*value)))
                        .collect(),
                    &mut frozen_steps,
                    span,
                )
                .unwrap_err();
            let mut live_steps = 1000;
            let live = prepared
                .fork()
                .invoke_with_live_inputs(
                    supplied
                        .into_iter()
                        .map(|(id, value)| (id, Value::Bool(value)))
                        .collect(),
                    &mut live_steps,
                    span,
                )
                .unwrap_err();
            assert_eq!(frozen.message, live.message, "case {case}");
            assert_eq!((frozen_steps, live_steps), (1000, 1000));
            assert_eq!(prepared.to_string(), before);
        }
    }

    #[test]
    fn mixed_supplies_follow_capture_order_in_a_type_invocation() {
        let span = Span::default();
        let mut prepared = literal_preparation(true);
        let choice = n::LocalId {
            owner: 11,
            index: 4,
        };
        let ty = n::LocalId {
            owner: 11,
            index: 1,
        };
        let expr = |ty, kind| n::Expr {
            ty,
            kind,
            source: n::SourceRange { start: 0, end: 1 },
        };
        prepared.edit_package(|package| {
            let function = &mut package.functions[0];
            function.parameters = vec![choice, ty];
            function.signature = Term::Function {
                parameters: vec![
                    Parameter {
                        sort: Sort::ComptimeValue,
                        ty: p(P::Bool),
                    },
                    Parameter {
                        sort: Sort::Type,
                        ty: p(P::Type),
                    },
                ],
                result: Box::new(p(P::Type)),
            };
            function.body = expr(
                p(P::Type),
                K::If {
                    condition: Box::new(expr(p(P::Bool), K::Local(choice))),
                    then_branch: Box::new(expr(p(P::Type), K::Local(ty))),
                    else_branch: Some(Box::new(expr(p(P::Type), K::TypeValue(p(P::Char))))),
                },
            );
        });
        prepared.inputs = vec![(choice, p(P::Bool), None), (ty, p(P::Type), None)];
        for (flag, expected) in [(true, P::U8), (false, P::Char)] {
            let values = vec![
                (ty, InvocationValue::Normal(p(P::U8))),
                (choice, InvocationValue::Live(Value::Bool(flag))),
            ];
            assert_eq!(
                prepared
                    .fork()
                    .invoke_in_world(values, &mut 100_000, span, None)
                    .unwrap(),
                (Value::Type(p(expected)), p(P::Type))
            );
        }
    }

    #[test]
    fn pending_type_capture_is_specialized_per_invocation() {
        let span = Span::default();
        for ty in [p(P::U8), p(P::Bool)] {
            let mut prepared = literal_preparation(true);
            let binding = n::LocalId {
                owner: 11,
                index: 0,
            };
            prepared.edit_package(|package| {
                let function = &mut package.functions[0];
                function.parameters = vec![binding];
                function.signature = Term::Function {
                    parameters: vec![Parameter {
                        sort: Sort::Type,
                        ty: p(P::Type),
                    }],
                    result: Box::new(p(P::Type)),
                };
                function.body.kind = K::Local(binding);
            });
            prepared.inputs = vec![(binding, p(P::Type), None)];
            let mut steps = 1000;
            let (value, result) = prepared
                .invoke_with_inputs(vec![(binding, ty.clone())], &mut steps, span)
                .unwrap();
            assert!(matches!(value, Value::Type(value) if value == ty));
            assert_eq!(result, p(P::Type));
            assert!(steps < 1000);
        }
    }

    #[test]
    fn prepared_live_inputs_keep_shared_closures_across_type_and_value_invocations() {
        use std::sync::Arc;
        let span = Span::default();
        for type_result in [false, true] {
            let mut prepared = literal_preparation(type_result);
            let result_type = prepared.result_type.clone();
            let selector = n::LocalId {
                owner: 11,
                index: 0,
            };
            let frozen = n::LocalId {
                owner: 11,
                index: 1,
            };
            let flag = n::LocalId {
                owner: 11,
                index: 2,
            };
            let expr = |ty, kind| n::Expr {
                ty,
                kind,
                source: n::SourceRange { start: 0, end: 1 },
            };
            let boolean = |value| expr(p(P::Bool), K::Literal(n::Literal::Bool(value)));
            let read = || expr(p(P::Bool), K::Local(flag));
            let callback_type = Term::Closure {
                environment: vec![p(P::Bool)],
                signature: Box::new(Term::ComptimeFunction(Box::new(Term::Function {
                    parameters: vec![],
                    result: Box::new(result_type.clone()),
                }))),
            };
            let toggle = expr(
                p(P::Void),
                K::Assign {
                    place: n::Place {
                        root: flag,
                        fields: vec![],
                    },
                    value: Box::new(expr(
                        p(P::Bool),
                        K::If {
                            condition: Box::new(read()),
                            then_branch: Box::new(boolean(false)),
                            else_branch: Some(Box::new(boolean(true))),
                        },
                    )),
                },
            );
            let selected = if type_result {
                expr(
                    p(P::Type),
                    K::If {
                        condition: Box::new(read()),
                        then_branch: Box::new(expr(p(P::Type), K::TypeValue(p(P::Bool)))),
                        else_branch: Some(Box::new(expr(p(P::Type), K::TypeValue(p(P::U8))))),
                    },
                )
            } else {
                read()
            };
            let mut factory = prepared.world.package().clone();
            factory.functions[0].signature = Term::Function {
                parameters: vec![],
                result: Box::new(callback_type.clone()),
            };
            factory.functions[0].body = expr(
                callback_type.clone(),
                K::Closure {
                    parameters: vec![],
                    captures: vec![n::Capture {
                        binding: flag,
                        ty: p(P::Bool),
                        value: Some(Box::new(boolean(false))),
                    }],
                    body: Arc::new(expr(
                        result_type.clone(),
                        K::Block {
                            statements: vec![toggle],
                            result: Some(Box::new(selected)),
                        },
                    )),
                },
            );
            let mut producer = Interpreter::for_comptime(&factory, Budget::default()).unwrap();
            let callback = producer.invoke(prepared.entry, vec![]).unwrap();
            prepared.edit_package(|package| {
                let function = &mut package.functions[0];
                function.parameters = vec![selector, frozen];
                function.signature = Term::Function {
                    parameters: vec![
                        Parameter {
                            sort: Sort::ComptimeValue,
                            ty: callback_type.clone(),
                        },
                        Parameter {
                            sort: Sort::ComptimeValue,
                            ty: p(P::Bool),
                        },
                    ],
                    result: Box::new(result_type.clone()),
                };
                function.body = expr(
                    result_type.clone(),
                    K::Call {
                        callee: Box::new(expr(callback_type.clone(), K::Local(selector))),
                        arguments: vec![],
                        parameter_bindings: vec![],
                    },
                );
            });
            prepared.inputs = vec![
                (selector, callback_type, None),
                (frozen, p(P::Bool), Some(Term::Bool(true))),
            ];
            let mut steps = 10_000;
            // Routing errors must leave the shared callback untouched.
            for invalid in [
                vec![(frozen, Value::Bool(false))],
                vec![(selector, callback.clone()), (selector, callback.clone())],
                vec![(
                    n::LocalId {
                        owner: 99,
                        index: 0,
                    },
                    callback.clone(),
                )],
            ] {
                let before = steps;
                assert!(
                    prepared
                        .fork()
                        .invoke_with_live_inputs(invalid, &mut steps, span)
                        .is_err()
                );
                assert_eq!(steps, before);
            }
            for expected in [true, false, true] {
                let (value, ty) = prepared
                    .fork()
                    .invoke_with_live_inputs(vec![(selector, callback.clone())], &mut steps, span)
                    .unwrap();
                assert_eq!(ty, result_type);
                if type_result {
                    assert_eq!(
                        value,
                        Value::Type(p(if expected { P::Bool } else { P::U8 }))
                    );
                } else {
                    assert_eq!(value, Value::Bool(expected));
                }
            }
            assert!(steps < 10_000);
            assert!(
                prepared.inputs[0].2.is_none(),
                "invocation must not modify the reusable preparation"
            );
        }
    }

    #[test]
    fn prepared_required_dump_preserves_scope_and_escaped_inputs() {
        let declaration = DeclarationId {
            package: 7,
            local: 11,
        };
        let tuple = Term::NormalForm {
            head: declaration,
            sort: Sort::Type,
            arguments: vec![frozen_types(vec![p(P::Bool), p(P::U8)])],
        };
        let prepared = PreparedRequired {
            world: RequiredWorld::new(n::Package::default()),
            entry: declaration,
            inputs: vec![(
                n::LocalId {
                    owner: 11,
                    index: 0,
                },
                p(P::Str),
                Some(Term::String("quoted \"value\"\\\n(parentheses)".into())),
            )],
            result_type: p(P::Type),
            scoped_parameters: vec![nir::DeclarationParameter {
                declaration,
                sort: Sort::Type,
            }],
            scoped_arguments: vec![tuple.clone()],
            input_frames: vec![vec![Sort::Type, Sort::ComptimeValue]],
            invocation_assumptions: vec![],
            abstract_parameters: vec![declaration],
            abstract_value_parameters: vec![],
            context: crate::ptc::Context::default(),
            in_signature: true,
        };
        let dump = prepared.to_string();
        assert_eq!(dump, prepared.to_string());
        assert!(dump.contains(&format!("(binding {declaration} {} {tuple})", Sort::Type)));
        assert!(dump.contains("(in-signature true)"));
        assert!(dump.contains("\\\"value\\\""));
        assert!(dump.contains("\\n(parentheses)"));
        // Count structural parentheses only outside strings, respecting escapes.
        let (mut depth, mut quoted, mut escaped) = (0usize, false, false);
        for ch in dump.chars() {
            if quoted {
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    quoted = false;
                }
            } else {
                match ch {
                    '"' => quoted = true,
                    '(' => depth += 1,
                    ')' => depth = depth.checked_sub(1).expect("unmatched closing parenthesis"),
                    _ => {}
                }
            }
        }
        assert_eq!(depth, 0);
        assert!(!quoted && !escaped);
    }
}
