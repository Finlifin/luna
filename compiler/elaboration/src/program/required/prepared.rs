//! Checked source computations, separated from their invocation. This is an
//! owned preparation result, not a Type or an inference-variable snapshot.
use super::restore::restore_required_packet;
use super::*;
use crate::materialization::{DeclarationSnapshot, DeclarationSource};
use comptime_interpretor::{CodeSession, InvocationValue};
use std::sync::{Arc, Mutex, OnceLock};
mod inputs;

/// Own the declarations and their derived index together. Once published into
/// a preparation, this world is immutable; invocation state lives elsewhere.
pub(super) struct RequiredWorld {
    source: DeclarationSnapshot,
    phase: Option<nir::phases::PhaseCheckedPackage<'static>>,
    code: Mutex<Option<CodeSession>>,
    free_inputs: OnceLock<BTreeSet<DeclarationId>>,
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
            free_inputs: OnceLock::new(),
        })
    }

    /// Only publish a callable source after both its declarations and its
    /// separately stored programs have passed phase and arithmetic inspection.
    pub(super) fn capture(
        package: n::Package,
        steps: &mut u64,
        span: Span,
        expansions: &[n::ExpansionSource],
        owner: &::materialization::MemberDataSource,
    ) -> Result<Arc<Self>, Error> {
        use crate::verification::flow;
        let before = usize::try_from(*steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let result = (|| {
            let snapshot = DeclarationSnapshot::capture_for_data_owner(package, &mut fuel, owner)
                .map_err(|failure| error(span, failure.to_string()))?;
            let phase = DeclarationSource::Snapshot(&snapshot).phase_inspection()
                .check_package_with_evidence(&mut fuel)
                .map_err(|e| flow::Error { source: e.source, reason: flow::Reason::Phase(e.message) })
                .and_then(|phase| {
                    flow::validate_machine_operations_checked(&phase)?;
                    Ok(phase)
                })
                .map_err(|mut failure| {
                    failure.source = failure.source.original(expansions).unwrap_or(failure.source);
                    let message = match failure.reason {
                        flow::Reason::Phase("runtime function is unavailable during comptime execution") =>
                            format!("required comptime evaluation cannot execute a runtime function or normal-form head: {failure}"),
                        _ => failure.to_string(),
                    };
                    error(span, message)
                })?;
            Self::with_phase(snapshot.clone(), &phase)
                .map_err(|failure| error(span, failure.to_string()))
        })();
        *steps -= (before - fuel) as u64;
        result
    }

    fn with_phase(
        source: DeclarationSnapshot,
        phase: &nir::phases::PhaseCheckedPackage<'_>,
    ) -> Result<Arc<Self>, comptime_interpretor::Error> {
        let phase =
            phase
                .retain_in(source.package().clone())
                .ok_or(comptime_interpretor::Error {
                    kind: comptime_interpretor::ErrorKind::InvalidCallPlan,
                    source: None,
                })?;
        Ok(Arc::new(Self {
            source,
            phase: Some(phase),
            code: Mutex::new(None),
            free_inputs: OnceLock::new(),
        }))
    }

    pub(super) fn free_inputs(
        &self,
        steps: &mut u64,
        span: Span,
    ) -> Result<&BTreeSet<DeclarationId>, Error> {
        *steps = steps
            .checked_sub(1)
            .ok_or_else(|| error(span, "required source input observation budget exhausted"))?;
        if let Some(inputs) = self.free_inputs.get() {
            return Ok(inputs);
        }
        let package = self.package();
        let owned;
        let domains = match &self.phase {
            Some(checked) => checked.parameter_domains(),
            None => {
                let before = usize::try_from(*steps).unwrap_or(usize::MAX);
                let mut fuel = before;
                let result = package.declaration_parameter_domains_checked(&mut fuel);
                *steps -= (before - fuel) as u64;
                owned = result.map_err(|failure| error(span, failure.to_string()))?;
                &owned
            }
        };
        let completed = nir::copy_budget::retained_program_inputs(
            package,
            self.source.member_programs(),
            domains,
            steps,
        )
        .map_err(|limit| input_observation::structural_limit(limit, span))?;
        Ok(self.free_inputs.get_or_init(|| completed))
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

    pub(super) fn package(&self) -> &n::Package {
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
}

#[cfg_attr(test, derive(Clone))]
pub(in crate::program) struct PreparedRequired {
    pub(super) world: Arc<RequiredWorld>,
    pub entry: DeclarationId,
    pub inputs: Vec<(n::LocalId, Term, Option<Term>)>,
    pub result_type: Term,
    pub(super) scope: Arc<InvocationScope>,
}

/// Lexical evidence of a checked computation. Reusing the preparation shares
/// these immutable premises; supplying captures never edits the proof scope.
#[cfg_attr(test, derive(Clone))]
pub(super) struct InvocationScope {
    /// ABI capture position to source parameter. Explicit or live supplies
    /// outside the signature have no parameter slot in the caller.
    pub(super) parameter_slots: Box<[Option<usize>]>,
    pub(super) scoped_parameters: Vec<nir::DeclarationParameter>,
    pub(super) scoped_arguments: Vec<Term>,
    pub(super) input_frames: Vec<Vec<Sort>>,
    pub(super) invocation_assumptions: Vec<n::TraitRequirement>,
    pub(super) abstract_parameters: Vec<DeclarationId>,
    pub(super) abstract_value_parameters: Vec<(DeclarationId, Term)>,
    pub(super) context: crate::ptc::Context,
    pub(super) in_signature: bool,
}

impl PreparedRequired {
    pub(in crate::program) fn definition_identity_cursor(&self) -> u32 {
        self.world.package().definition_identity_cursor
    }

    /// Read the declared result binder and PTC guarantees of a directly
    /// called Type-producing entry function without peeking into its body.
    pub(in crate::program) fn direct_call_result_guarantees(
        &self,
        steps: &mut u64,
        span: Span,
    ) -> Result<Option<(DeclarationId, Vec<n::TypePredicate>)>, Error> {
        let package = self.world.package();
        let Some(entry) = package
            .functions
            .iter()
            .find(|f| f.declaration == self.entry)
        else {
            return Ok(None);
        };
        let mut expr = &entry.body;
        loop {
            match &expr.kind {
                K::Return(Some(inner)) => expr = inner,
                K::Block {
                    statements,
                    result: Some(inner),
                }
                | K::CallPreparation {
                    statements,
                    result: Some(inner),
                } if statements.is_empty() => {
                    expr = inner;
                }
                _ => break,
            }
        }
        let K::Call { callee, .. } = &expr.kind else {
            return Ok(None);
        };
        let (target_id, type_args): (DeclarationId, &[Term]) = match &callee.kind {
            K::Function(id) => (*id, &[]),
            K::FunctionInstance {
                declaration,
                arguments,
            } => (*declaration, arguments),
            _ => return Ok(None),
        };
        let Some(target) = package
            .functions
            .iter()
            .find(|f| f.declaration == target_id)
        else {
            return Ok(None);
        };
        let Some(binder) = target.result_type_binder else {
            return Ok(None);
        };
        if target.result_guarantees.is_empty() {
            return Ok(None);
        }
        if target.type_parameters.len() != type_args.len()
            || type_args.iter().any(|arg| arg.check_scope(&[]).is_err())
        {
            return Ok(None);
        }
        let before = usize::try_from(*steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let guarantees = (|| -> Result<Vec<n::TypePredicate>, String> {
            let mut result = if target.type_parameters.is_empty() {
                target.result_guarantees.clone()
            } else {
                let domains = vec![p(P::Type); target.type_parameters.len()];
                let substitution = nir::DeclarationSubstitution::bounded(
                    &target.type_parameters,
                    &domains,
                    type_args,
                    &[],
                    &mut fuel,
                )
                .map_err(|e| e.to_string())?;
                target
                    .result_guarantees
                    .iter()
                    .map(|g| {
                        substitution
                            .instantiate_predicate_bounded(g, &mut fuel)
                            .map_err(|e| e.to_string())
                    })
                    .collect::<Result<Vec<_>, _>>()?
            };
            if !self.scope.scoped_parameters.is_empty() {
                for g in &mut result {
                    nir::copy_budget::charge_predicate_substitution(
                        g,
                        &self.scope.scoped_parameters,
                        &self.scope.scoped_arguments,
                        &mut fuel,
                    )
                    .map_err(|e| format!("{e:?}"))?;
                    *g = g
                        .instantiate_parameters_in_scope(
                            &self.scope.scoped_parameters,
                            &self.scope.scoped_arguments,
                            &self.scope.input_frames,
                        )
                        .map_err(|e| e.to_string())?;
                }
            }
            if result.iter().any(|g| {
                super::map_requirement(
                    g,
                    &mut |ty| {
                        ty.check_scope(&[])
                            .map_err(|e| error(span, e.to_string()))?;
                        Ok(ty.clone())
                    },
                    span,
                    0,
                )
                .is_err()
            }) {
                return Ok(Vec::new());
            }
            Ok(result)
        })();
        *steps -= (before - fuel) as u64;
        let guarantees = guarantees.map_err(|reason| error(span, reason))?;
        if guarantees.is_empty() {
            return Ok(None);
        }
        Ok(Some((binder, guarantees)))
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
            .chain(&self.scope.scoped_arguments)
            .chain(
                self.scope
                    .invocation_assumptions
                    .iter()
                    .flat_map(|requirement| {
                        std::iter::once(&requirement.subject)
                            .chain(&requirement.interface_arguments)
                    }),
            );
        for term in terms {
            if !input_observation::term(term, steps, span)?.is_disjoint(&declarations) {
                return Ok(true);
            }
        }
        Ok(!self
            .world
            .free_inputs(steps, span)?
            .is_disjoint(&declarations))
    }

    pub(in crate::program) fn pending_parameters(
        &self,
    ) -> impl Iterator<Item = Option<usize>> + '_ {
        self.inputs
            .iter()
            .zip(&self.scope.parameter_slots)
            .filter_map(|((_, _, value), slot)| value.is_none().then_some(*slot))
    }

    pub(in crate::program) fn needs_call_world(
        &self,
        symbols: &[Option<Term>],
        live: &[Option<Value>],
        steps: &mut u64,
        span: Span,
    ) -> Result<bool, Error> {
        *steps = steps
            .checked_sub(self.inputs.len() as u64)
            .ok_or_else(|| error(span, "annotation capture selection budget exhausted"))?;
        if self
            .pending_parameters()
            .flatten()
            .any(|slot| live.get(slot).is_some_and(Option::is_some))
        {
            return Ok(true);
        }
        let terms = self
            .pending_parameters()
            .flatten()
            .filter_map(|slot| symbols.get(slot).and_then(Option::as_ref));
        Ok(!Discovery::Declarations
            .frozen_references(terms, steps, span)?
            .is_empty())
    }

    /// Produce the call's private argument packet directly from the captured
    /// ABI. The saved code, declaration world, and lexical premises are shared.
    pub(in crate::program) fn for_annotation_call(
        &self,
        symbols: &[Option<Term>],
        steps: &mut u64,
        span: Span,
    ) -> Result<Self, Error> {
        let inputs = self
            .inputs
            .iter()
            .zip(&self.scope.parameter_slots)
            .map(|((id, domain, frozen), &slot)| {
                let value = match frozen {
                    Some(value) => value,
                    None => {
                        let slot = slot.ok_or_else(|| {
                            error(span, "deferred annotation has an unknown source input")
                        })?;
                        symbols.get(slot).and_then(Option::as_ref).ok_or_else(|| {
                            error(
                                span,
                                "deferred annotation needs a completed comptime argument",
                            )
                        })?
                    }
                };
                let domain = nominal_fields::metered(steps, |fuel| {
                    domain.substitute_partial_bounded(symbols, fuel)
                })
                .map_err(|failure| error(span, failure.to_string()))?;
                Ok((*id, domain, Some(super::inputs::copy(value, steps, span)?)))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        Ok(Self {
            world: self.world.clone(),
            entry: self.entry,
            inputs,
            result_type: super::inputs::copy(&self.result_type, steps, span)?,
            scope: self.scope.clone(),
        })
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
            None,
        )
    }

    pub(in crate::program) fn invoke_publishing(
        self,
        values: Vec<(n::LocalId, Value)>,
        steps: &mut u64,
        span: Span,
        package_identity: u64,
        identities: crate::definitions::DefinitionIdentities,
        reader: Option<data::Reader<'_>>,
    ) -> Result<((Value, Term), crate::definitions::PublishedDefinitions), Error> {
        let world = Arc::clone(&self.world);
        // The receiver installs concrete descriptors into the package. Only the
        // invocation itself may use its lexical premises and trait assumptions.
        let context = self.scope.context.theory_context();
        let queries = crate::materialization::SourceInvocationQueries::in_source(
            DeclarationSource::Snapshot(&world.source),
            &context,
            &[],
        );
        let solver = |predicate: &n::TypePredicate, fuel: &mut u64| {
            queries.check(predicate, Default::default(), fuel)
        };
        let publisher = crate::definitions::AllocatedDefinitionPublisher::new_with_type_solver(
            world.package(),
            package_identity,
            identities,
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
            reader,
        );
        let diagnostic = publisher.last_diagnostic();
        let (definitions, _) = publisher.finish();
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
        reader: Option<data::Reader<'_>>,
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
            entry,
            inputs,
            result_type,
            scope,
            ..
        } = self;
        let InvocationScope {
            scoped_parameters,
            scoped_arguments,
            input_frames,
            invocation_assumptions,
            abstract_parameters,
            abstract_value_parameters,
            context: invocation_type_context,
            in_signature,
            ..
        } = scope.as_ref();
        let inputs = inputs::complete(inputs, values, span)?;
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
            invocation_assumptions,
        );
        let type_solver = |predicate: &n::TypePredicate, remaining: &mut u64| {
            use comptime_interpretor::ErrorKind;
            let restored;
            let (predicate, proof_context) = if scoped_parameters.is_empty() {
                (predicate, invocation_type_context)
            } else {
                let before = usize::try_from(*remaining).unwrap_or(usize::MAX);
                let mut fuel = before;
                let charged = nir::copy_budget::charge_predicate_substitution(
                    predicate,
                    scoped_parameters,
                    scoped_arguments,
                    &mut fuel,
                );
                *remaining -= (before - fuel) as u64;
                charged.map_err(|failure| match failure {
                    nir::copy_budget::CopyBudgetExhausted::Steps => ErrorKind::StepsExhausted,
                    nir::copy_budget::CopyBudgetExhausted::Depth => ErrorKind::DepthExhausted,
                })?;
                restored = predicate
                    .instantiate_parameters_in_scope(
                        scoped_parameters,
                        scoped_arguments,
                        input_frames,
                    )
                    .map_err(|error| ErrorKind::InvalidTypeContract(error.to_string()))?;
                (&restored, scoped_context.as_ref().unwrap())
            };
            source_queries.check_in_context(
                predicate,
                proof_context,
                crate::ptc::Budget::default(),
                remaining,
            )
        };
        let associated_resolver = |access: &n::Expr, remaining: &mut u64| match reader {
            Some(reader) => {
                source_queries.associated_with_data(access, remaining, reader.owner, reader.form)
            }
            None => source_queries.associated(access, remaining),
        };
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
            invocation_assumptions,
            abstract_parameters,
            abstract_value_parameters,
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
        let value = value.map_err(|failure| {
            use comptime_interpretor::ErrorKind;
            if *in_signature && (failure.kind == ErrorKind::MissingBinding
                || (failure.kind == ErrorKind::AbstractValueUnresolved
                    && scoped_parameters.iter().any(|p| p.sort == Sort::ComptimeValue)))
            {
                return error(span, format!(
                    "type computation depends on an unavailable function input; invocation-time type elaboration is not yet implemented ({failure})"
                ));
            }
            error(span, format!("required comptime evaluation failed: {failure}"))
        })?;
        let value = restore_required_packet(
            steps,
            value,
            scoped_parameters,
            scoped_arguments,
            input_frames,
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
        write!(f, ") (source-parameters")?;
        for (capture, slot) in self.scope.parameter_slots.iter().enumerate() {
            if let Some(slot) = slot {
                write!(f, " (capture {capture} (parameter {slot}))")?;
            }
        }
        write!(f, ") (scope-substitution")?;
        for (parameter, argument) in self
            .scope
            .scoped_parameters
            .iter()
            .zip(&self.scope.scoped_arguments)
        {
            write!(
                f,
                " (binding {} {} {argument})",
                parameter.declaration, parameter.sort
            )?;
        }
        write!(f, ") (input-frames")?;
        for frame in &self.scope.input_frames {
            write!(f, " (frame")?;
            for sort in frame {
                write!(f, " {sort}")?;
            }
            write!(f, ")")?;
        }
        write!(f, ") (assumptions")?;
        for assumption in &self.scope.invocation_assumptions {
            write!(f, " {assumption}")?;
        }
        write!(f, ") (abstract-parameters")?;
        for parameter in &self.scope.abstract_parameters {
            write!(f, " {parameter}")?;
        }
        write!(f, ") (abstract-value-parameters")?;
        for (id, domain) in &self.scope.abstract_value_parameters {
            write!(f, " ({id} {domain})")?;
        }
        write!(
            f,
            ") (in-signature {}) (context {}) (source {})",
            self.scope.in_signature, self.scope.context, self.world.source
        )?;
        write!(f, ")")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl PreparedRequired {
        fn edit_scope(&mut self) -> &mut InvocationScope {
            Arc::make_mut(&mut self.scope)
        }

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
                    source: nir::program::SourceRange { start: 0, end: 0 },
                    phase: n::FunctionPhase::Comptime,
                    inline: false,
                    pure: true,
                    is_unsafe: false,
                    preconditions: vec![],
                    postconditions: vec![],
                    result_binding: None,
                    result_type_binder: None,
                    result_guarantees: vec![],
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
            scope: Arc::new(InvocationScope {
                parameter_slots: Box::default(),
                scoped_parameters: vec![],
                scoped_arguments: vec![],
                input_frames: vec![],
                invocation_assumptions: vec![],
                abstract_parameters: vec![],
                abstract_value_parameters: vec![],
                context: crate::ptc::Context::default(),
                in_signature: false,
            }),
        }
    }

    #[test]
    fn prepared_annotation_selects_its_source_slots_and_reuses_no_argument_values() {
        let mut prepared = literal_preparation(true);
        let binding = |index| n::LocalId { owner: 11, index };
        let span = Span::default();
        let bound = Term::Bound {
            depth: 0,
            index: 3,
            sort: Sort::Type,
        };
        prepared.inputs = vec![
            (binding(101), p(P::Str), Some(Term::String("fixed".into()))),
            (binding(10), p(P::Type), None),
            (binding(25), bound, None),
        ];
        prepared.edit_scope().parameter_slots = Box::from([None, Some(3), Some(4)]);
        let callback = Term::FunctionValue {
            declaration: prepared.entry,
            arguments: vec![],
        };
        let mut caller = vec![
            Some(callback.clone()),
            None,
            None,
            Some(p(P::Bool)),
            Some(Term::Bool(true)),
        ];
        let mut live = vec![Some(Value::Bool(true)), None, None, None, None];
        let needs_source = |caller: &[Option<Term>], live: &[Option<Value>]| {
            prepared
                .needs_call_world(caller, live, &mut 100, span)
                .unwrap()
        };
        assert!(!needs_source(&caller, &live));
        live[4] = Some(Value::Bool(false));
        assert!(needs_source(&caller, &live));
        live[4] = None;
        caller[4] = Some(Term::TupleValue(vec![callback]));
        assert!(needs_source(&caller, &live));
        caller[4] = None;
        assert!(
            prepared
                .for_annotation_call(&caller, &mut 1000, span)
                .is_err()
        );

        for value in [false, true] {
            caller[4] = Some(Term::Bool(value));
            let call = prepared
                .for_annotation_call(&caller, &mut 1000, span)
                .unwrap();
            assert_eq!(call.inputs[0], prepared.inputs[0]);
            assert_eq!(
                call.inputs[2],
                (binding(25), p(P::Bool), Some(Term::Bool(value)))
            );
            assert!(Arc::ptr_eq(&call.world, &prepared.world));
            assert!(prepared.inputs[2].2.is_none());
        }
        assert!(prepared.to_string().contains("(capture 2 (parameter 4))"));
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
            prepared.edit_scope().abstract_parameters.push(declaration);
            prepared.edit_package(|package| {
                package.functions[0].body.kind = K::TypeValue(rigid.clone());
            });
            if scoped {
                prepared
                    .edit_scope()
                    .scoped_parameters
                    .push(nir::DeclarationParameter {
                        declaration,
                        sort: Sort::Type,
                    });
                prepared.edit_scope().scoped_arguments.push(bound.clone());
                prepared.edit_scope().input_frames.push(vec![Sort::Type]);
            }
            let mut steps = 100_000;
            assert_eq!(
                prepared
                    .clone()
                    .invoke(&mut steps, Span::default())
                    .unwrap(),
                (
                    Value::Type(if scoped { bound.clone() } else { rigid.clone() }),
                    p(P::Type)
                )
            );
            assert!(steps < 100_000);
            // The retained source/code does not authorize an unknown Type input.
            let mut denied = prepared.clone();
            denied.edit_scope().abstract_parameters.clear();
            assert!(denied.invoke(&mut 100_000, Span::default()).is_err());
            assert!(prepared.invoke(&mut 100_000, Span::default()).is_ok());
        }
    }

    #[test]
    fn concrete_publication_cannot_export_a_caller_proof() {
        let owner = DeclarationId {
            package: 7,
            local: 41,
        };
        let slot = DeclarationId {
            package: 7,
            local: 42,
        };
        for actual in [P::U8, P::Bool] {
            let mut prepared = literal_preparation(true);
            prepared.edit_package(|package| {
                let source = package.functions[0].body.source;
                let expr = |ty, kind| n::Expr { ty, kind, source };
                package.structs.push(n::Struct {
                    declaration: owner,
                    name: "Checked".into(),
                    type_parameters: vec![slot],
                    parameter_types: vec![],
                    requirements: vec![n::TypePredicate::Equal(
                        Term::Rigid {
                            declaration: slot,
                            sort: Sort::Type,
                        },
                        p(P::Bool),
                    )],
                    capabilities: vec![],
                    fields: vec![],
                    packed: false,
                });
                let representation = Term::Nominal {
                    declaration: owner,
                    arguments: vec![p(actual)],
                };
                package.functions[0].body.kind = K::BuildNewtype(Box::new(expr(
                    Term::Tuple(vec![p(P::Symbol), p(P::Type)]),
                    K::Tuple(vec![
                        expr(
                            p(P::Symbol),
                            K::Literal(n::Literal::Symbol("Wrapped".into())),
                        ),
                        expr(p(P::Type), K::TypeValue(representation)),
                    ]),
                )));
            });
            // This local premise suffices while evaluating the operand. The
            // global descriptor gets its own formation check after reservation.
            prepared
                .edit_scope()
                .context
                .assume(crate::ptc::Formula::equal(p(actual), p(P::Bool)));
            let identities = crate::definitions::DefinitionIdentities::from(60);
            let result = prepared.invoke_publishing(
                vec![],
                &mut 100_000,
                Span::default(),
                7,
                identities.clone(),
                None,
            );
            assert_eq!(identities.next(), 61, "the request reached the publisher");
            match (actual, result) {
                (P::Bool, Ok((_, additions))) => assert_eq!(additions.newtypes.len(), 1),
                (P::U8, Err(failure)) => assert!(
                    failure.to_string().contains("TypeContractRejected"),
                    "{failure}"
                ),
                _ => panic!("unexpected publication result for {actual:?}"),
            }
        }
    }

    #[test]
    fn direct_proof_retries_without_publishing_a_declaration_directory() {
        use comptime_interpretor::ContractOutcome;
        let prepared = literal_preparation(false);
        let predicate = n::TypePredicate::Equal(p(P::Bool), p(P::Bool));
        let queries = crate::materialization::SourceInvocationQueries::in_source(
            DeclarationSource::Snapshot(&prepared.world.source),
            &prepared.scope.context,
            &[],
        );
        let mut tiny = 1;
        assert_eq!(
            queries.check(&predicate, Default::default(), &mut tiny),
            Err(comptime_interpretor::ErrorKind::StepsExhausted)
        );
        assert_eq!(tiny, 0);
        let mut empty = crate::ptc::Budget {
            steps: 0,
            ..Default::default()
        };
        assert!(prepared.world.source.prepare(&mut empty).is_err());
        let mut remaining = 100_000;
        assert_eq!(
            queries.check(&predicate, Default::default(), &mut remaining),
            Ok(ContractOutcome::Satisfied)
        );
        assert!(remaining < 100_000);
        // Equality consumed only PTC work. Only a separate successful
        // declaration demand can make a zero-cost directory read succeed.
        assert!(matches!(
            prepared.world.source.prepare(&mut empty),
            Err(nir::NominalAdmissionError::Budget)
        ));
        prepared
            .world
            .source
            .prepare(&mut crate::ptc::Budget::default())
            .unwrap();
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
                .clone()
                .invoke(&mut 100_000, Span::default())
                .unwrap()
                .0;
            let queries = crate::materialization::SourceInvocationQueries::in_source(
                DeclarationSource::Snapshot(&prepared.world.source),
                &prepared.scope.context,
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
            assert!(prepared.clone().invoke(&mut 0, Span::default()).is_err());
            assert_eq!(
                prepared
                    .clone()
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
            prepared.edit_scope().context =
                crate::ptc::Context::from_package(prepared.world.package()).unwrap();
            let mut granted = prepared.clone();
            granted.edit_scope().invocation_assumptions.push(premise);
            let expected = granted
                .clone()
                .invoke(&mut 100_000, Span::default())
                .unwrap()
                .0;
            let failure = prepared
                .clone()
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
        prepared.world =
            RequiredWorld::with_phase(DeclarationSnapshot::new(package.clone()), &phase).unwrap();
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
                    .clone()
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
    fn annotation_calls_share_declarations_and_isolate_input_assignments() {
        let span = Span::default();
        let (prepared, _) = pending_boolean_preparation();
        let owner = Arc::downgrade(&prepared.world);
        let call = |flag| {
            prepared
                .for_annotation_call(&[Some(Term::Bool(flag))], &mut 100, span)
                .unwrap()
        };
        let first = call(true);
        let second = call(false);
        assert!(Arc::ptr_eq(&prepared.world, &first.world));
        assert!(Arc::ptr_eq(&first.scope, &second.scope));
        assert!(std::ptr::eq(
            first.world.capabilities(&[], &first.scope.context).index,
            second.world.capabilities(&[], &second.scope.context).index
        ));
        assert!(prepared.inputs[0].2.is_none());
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
                .clone()
                .invoke(&mut baseline, Span::default())
                .unwrap();
            let allowance = 100_000 - baseline + 1000;
            prepared.edit_package(|package| {
                package.functions[0].requirements =
                    vec![n::TypePredicate::Equal(p(P::Bool), p(P::Bool))];
            });
            let source = DeclarationSource::Snapshot(&prepared.world.source);
            let context = &prepared.scope.context;
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
                    queries.check_in_context(
                        &predicate,
                        &with_premise,
                        Default::default(),
                        &mut 100_000
                    ),
                    Ok(ContractOutcome::Satisfied)
                );
                assert!(matches!(
                    queries.check(&predicate, crate::ptc::Budget::default(), &mut 100_000),
                    Err(comptime_interpretor::ErrorKind::InvalidTypeContract(_))
                ));
            }
            drop(queries);
            drop(scope);
            assert!(prepared.clone().invoke(&mut 0, Span::default()).is_err());
            for _ in 0..2 {
                let mut remaining = allowance;
                let actual = prepared
                    .clone()
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
                first.clone().invoke(&mut 100_000, span).unwrap().0,
                Value::Bool(true)
            );
            assert_eq!(
                second.clone().invoke(&mut 100_000, span).unwrap().0,
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
            |p, term| p.edit_scope().scoped_arguments.push(term),
            |p, term| {
                let requirement = n::TraitRequirement {
                    subject: term,
                    interface: p.entry,
                    interface_arguments: vec![],
                };
                p.edit_scope().invocation_assumptions.push(requirement);
            },
            |p, term| {
                let requirement = n::TraitRequirement {
                    subject: p.result_type.clone(),
                    interface: p.entry,
                    interface_arguments: vec![term],
                };
                p.edit_scope().invocation_assumptions.push(requirement);
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
    fn projected_function_inputs_belong_to_their_source_and_lexical_telescope() {
        let mut prepared = literal_preparation(true);
        let id = |local| DeclarationId { package: 7, local };
        let ty = |local| Term::Rigid {
            declaration: id(local),
            sort: Sort::Type,
        };
        let span = Span::default();
        prepared.edit_package(|package| {
            package
                .runtime_function_identities
                .push(n::RuntimeFunctionIdentity {
                    source: nir::program::SourceRange { start: 0, end: 0 },
                    declaration: id(20),
                    type_parameters: vec![id(21), id(26)],
                    signature: Term::Function {
                        parameters: vec![nir::Parameter {
                            sort: Sort::Runtime,
                            ty: ty(21),
                        }],
                        result: Box::new(ty(22)),
                    },
                    requirements: vec![n::TypePredicate::Equal(ty(21), ty(23))],
                    capabilities: vec![n::TraitRequirement {
                        subject: ty(21),
                        interface: id(90),
                        interface_arguments: vec![ty(24)],
                    }],
                });
            package.structs.push(n::Struct {
                declaration: id(31),
                name: "Container".into(),
                type_parameters: vec![id(30)],
                parameter_types: vec![ty(28)],
                capabilities: vec![],
                requirements: vec![],
                fields: vec![],
                packed: false,
            });
            package
                .runtime_function_identities
                .push(n::RuntimeFunctionIdentity {
                    source: nir::program::SourceRange { start: 0, end: 0 },
                    declaration: id(32),
                    type_parameters: vec![id(30)],
                    signature: Term::Function {
                        parameters: vec![],
                        result: Box::new(p(P::Bool)),
                    },
                    requirements: vec![],
                    capabilities: vec![],
                });
            package.function_parameters = vec![
                nir::function_parameters::Declaration {
                    declaration: id(20),
                    parameters: vec![id(21), id(26)],
                    domains: vec![p(P::Type), ty(25)],
                },
                nir::function_parameters::Declaration {
                    declaration: id(80),
                    parameters: vec![id(81)],
                    domains: vec![ty(82)],
                },
            ];
        });

        let expected = BTreeSet::from([id(22), id(23), id(24), id(25), id(28)]);
        assert!(prepared.world.free_inputs(&mut 3, span).is_err());
        assert_eq!(
            prepared.world.free_inputs(&mut 1000, span).unwrap(),
            &expected
        );
        let fork = prepared.clone();
        assert_eq!(fork.world.free_inputs(&mut 1, span).unwrap(), &expected);
        assert!(fork.world.free_inputs(&mut 0, span).is_err());
        assert!(
            fork.uses_declaration_inputs(&[id(24)], &mut 100, span)
                .unwrap()
        );
        assert!(
            !fork
                .uses_declaration_inputs(&[id(21), id(26), id(30), id(82), id(90)], &mut 100, span)
                .unwrap()
        );
        drop(fork);
        // The same rigid is free at the call site even if it is bound in the callee.
        prepared.edit_package(|package| package.functions[0].body.kind = K::TypeValue(ty(21)));
        assert!(
            prepared
                .uses_declaration_inputs(&[id(21)], &mut 1000, span)
                .unwrap()
        );
    }

    #[test]
    fn prepared_input_observation_ignores_ambient_authority_and_declaration_names() {
        let input = DeclarationId {
            package: 7,
            local: 90,
        };
        let mut prepared = literal_preparation(true);
        prepared.edit_scope().abstract_parameters.push(input);
        prepared
            .edit_scope()
            .abstract_value_parameters
            .push((input, p(P::Bool)));
        prepared
            .edit_scope()
            .scoped_parameters
            .push(nir::DeclarationParameter {
                declaration: input,
                sort: Sort::Type,
            });
        prepared.edit_scope().scoped_arguments.push(p(P::U8));
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
            prepared.edit_scope().abstract_value_parameters = vec![(declaration, p(P::Bool))];
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
            prepared.edit_scope().abstract_parameters = vec![id];
            prepared.edit_scope().abstract_value_parameters = vec![(id, p(P::Bool))];
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
        prepared.edit_scope().parameter_slots = Box::from([Some(0)]);
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
        let (pending, binding) = pending_boolean_preparation();
        assert!(pending.to_string().contains("(pending)"));
        let foreign = n::LocalId {
            owner: binding.owner + 1,
            ..binding
        };
        for other in [binding, foreign] {
            assert!(
                pending
                    .clone()
                    .invoke_with_inputs(
                        vec![(binding, Term::Bool(true)), (other, Term::Bool(false))],
                        &mut steps,
                        span
                    )
                    .is_err()
            );
            assert_eq!(steps, 1000);
        }
        let supplied = pending
            .for_annotation_call(&[Some(Term::Bool(false))], &mut steps, span)
            .unwrap();
        assert!(
            supplied
                .clone()
                .invoke_with_inputs(vec![(binding, Term::Bool(true))], &mut steps, span)
                .is_err()
        );
        assert!(
            matches!(supplied.invoke(&mut steps, span).unwrap(), (Value::Bool(false), ty) if ty == p(P::Bool))
        );
        let wrong = pending
            .for_annotation_call(&[Some(p(P::U8))], &mut steps, span)
            .unwrap();
        assert!(
            wrong.invoke(&mut steps, span).is_err(),
            "binding a value does not bypass input domain checking"
        );
        assert!(pending.inputs[0].2.is_none());
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
                    prepared = prepared
                        .for_annotation_call(&[Some(Term::Bool(false))], &mut 100, span)
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
                .clone()
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
                .clone()
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
                    .clone()
                    .invoke_in_world(values, &mut 100_000, span, None, None)
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
                        .clone()
                        .invoke_with_live_inputs(invalid, &mut steps, span)
                        .is_err()
                );
                assert_eq!(steps, before);
            }
            for expected in [true, false, true] {
                let (value, ty) = prepared
                    .clone()
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
            scope: Arc::new(InvocationScope {
                parameter_slots: Box::default(),
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
            }),
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
