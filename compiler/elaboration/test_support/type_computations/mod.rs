//! Isolated dependency-scheduler harness for inference and VM protocol tests.
//! This is not the source checker's invocation owner. Production required calls
//! return ordinary values without allocating a Type inference output. Retain
//! this model to exercise dependency planning, cancellation and non-replay; it
//! does not demonstrate source-region suspension or Type-description admission.
use super::gradual;
use comptime_interpretor::{Budget, Interpreter, InvocationValue, Value};
use nir::program::{FunctionPhase, Package};
use nir::{DeclarationId, Primitive, Sort, Term};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};
use type_resolution::{InferenceContext, InferenceError, InferenceTerm, MetaId};

mod demand;
mod execution;
#[allow(unused_imports)] // Each integration target uses a different protocol subset.
pub use execution::{Invocation, Progress as InvocationProgress, Start as InvocationStart};

const DEFAULT_SCHEDULING_STEPS: usize = 100_000;

#[derive(Clone, Debug)]
pub struct Input {
    pub ty: InferenceTerm,
    pub value: InferenceTerm,
}

/// An already evaluated invocation-local value. Its graph may contain mutable
/// captures; it is never frozen into a Term or persisted in a residual call.
pub struct LiveInput {
    pub ty: InferenceTerm,
    pub value: Value,
}

/// Completed inputs with a mixture of frozen data and live value graphs.
pub struct PreparedInput {
    pub ty: InferenceTerm,
    pub value: InvocationValue,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    ForeignContext,
    ForeignSession,
    InvalidAnnotation(usize),
    SpeculativeContext,
    UnknownComputation,
    ConsumedInputs,
    Cancelled,
    InvalidTarget,
    Arity,
    Inference(InferenceError),
    Invocation(comptime_interpretor::Error),
    NotAType,
    DependencyFailed(MetaId),
    DependencyCycle(MetaId),
    SchedulingBudget,
    Signature(nir::deferred_signatures::Error),
}
impl From<InferenceError> for Error {
    fn from(value: InferenceError) -> Self {
        Self::Inference(value)
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Quoted diagnostics remain one escaped atom in the surrounding dump.
        write!(f, "(type-computation-error {:?})", format!("{self:?}"))
    }
}
impl std::error::Error for Error {}

#[derive(Debug, PartialEq, Eq)]
pub enum State {
    Waiting(BTreeSet<MetaId>),
    Complete(Term),
    Failed(Error),
}
impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Waiting(inputs) => {
                write!(f, "(waiting")?;
                for input in inputs {
                    write!(f, " {input}")?;
                }
                write!(f, ")")
            }
            Self::Complete(ty) => write!(f, "(complete {ty})"),
            Self::Failed(error) => write!(f, "(failed {error})"),
        }
    }
}

/// One declaration annotation, before staged-input substitution and runtime
/// projection. A known annotation retains the signature's original telescope;
/// a computed annotation contains the completed invocation result.
pub enum AnnotationState<'a> {
    Ready(&'a Term),
    Waiting(&'a BTreeSet<MetaId>),
    Failed(&'a Error),
}

/// A caller-specialized runtime signature. Definition descriptions and the
/// declaration's staged telescope remain unchanged in the saved task.
pub enum RuntimeSignatureState<'a> {
    Ready(Term),
    Waiting(&'a BTreeSet<MetaId>),
    Failed(&'a Error),
}
impl fmt::Display for RuntimeSignatureState<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ready(signature) => write!(f, "(runtime-signature-ready {signature})"),
            Self::Waiting(inputs) => {
                write!(f, "(runtime-signature-waiting")?;
                for input in *inputs {
                    write!(f, " {input}")?;
                }
                write!(f, ")")
            }
            Self::Failed(error) => write!(f, "(runtime-signature-failed {error})"),
        }
    }
}

/// Admission is separate from computing a Type. A rejected refinement must
/// stop the signature prefix before any later annotation invokes comptime code.
#[derive(Debug)]
pub enum AdmissionError<E> {
    Computation(Error),
    Rejected { slot: usize, reason: E },
}
impl<E: fmt::Display> fmt::Display for AdmissionError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Computation(error) => write!(f, "(signature-admission-computation {error})"),
            Self::Rejected { slot, reason } => write!(
                f,
                "(signature-admission-rejected {slot} {:?})",
                reason.to_string()
            ),
        }
    }
}
impl fmt::Display for AnnotationState<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ready(ty) => write!(f, "(annotation-ready {ty})"),
            Self::Waiting(inputs) => {
                write!(f, "(annotation-waiting")?;
                for input in *inputs {
                    write!(f, " {input}")?;
                }
                write!(f, ")")
            }
            Self::Failed(error) => write!(f, "(annotation-failed {error})"),
        }
    }
}
enum PendingInputs {
    Symbolic(Vec<Input>),
    Prepared {
        domains: Vec<InferenceTerm>,
        values: Option<Vec<InvocationValue>>,
    },
}
impl PendingInputs {
    fn dependencies(&self, context: &InferenceContext) -> Result<BTreeSet<MetaId>, InferenceError> {
        let mut fuel = DEFAULT_SCHEDULING_STEPS;
        self.dependencies_metered(context, &mut fuel)
    }
    fn dependencies_metered(
        &self,
        context: &InferenceContext,
        fuel: &mut usize,
    ) -> Result<BTreeSet<MetaId>, InferenceError> {
        match self {
            Self::Symbolic(inputs) => dependencies_metered(context, inputs, fuel),
            Self::Prepared { domains, .. } => {
                let mut result = BTreeSet::new();
                for domain in domains {
                    result.extend(context.unresolved_metered(domain, fuel)?);
                }
                Ok(result)
            }
        }
    }
}
struct Pending {
    target: DeclarationId,
    inputs: PendingInputs,
    state: State,
}

/// Readiness of one annotation's complete producer set. Ready means every
/// observed input has settled, not that a provisional Type has been published.
enum DemandState {
    Ready,
    Waiting(BTreeSet<MetaId>),
    Failed(MetaId),
}

/// One invocation's signature resolution. The originating session owns all
/// computation outputs and effects; a task cannot be moved to another session.
pub struct SignatureTask {
    session: u64,
    signature: nir::deferred_signatures::Signature,
    bindings: Vec<InferenceTerm>,
    instantiated_annotations: Vec<Option<Term>>,
    outputs: Vec<MetaId>,
    state: State,
}
impl SignatureTask {
    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn definition_signature(
        &self,
        fuel: &mut usize,
    ) -> Result<gradual::SignatureDescription, Error> {
        gradual::SignatureDescription::from_residual(&self.signature, fuel).map_err(|error| {
            match error {
                gradual::Error::Scope(error) => {
                    Error::Signature(nir::deferred_signatures::Error::Scope(error))
                }
                _ => Error::SchedulingBudget,
            }
        })
    }

    /// Ordinary checking observes stable declaration information, never the
    /// task's inference outputs as provisional Types. The residual computation
    /// remains in the saved signature; completing one instance does not refine
    /// the declaration's description for other instances.
    pub fn definition_description(
        &self,
        slot: usize,
        fuel: &mut usize,
    ) -> Result<gradual::FrozenDescription, Error> {
        if slot > self.signature.inputs().len() {
            return Err(Error::InvalidAnnotation(slot));
        }
        let annotation = self
            .signature
            .inputs()
            .get(slot)
            .map(|input| &input.annotation)
            .unwrap_or_else(|| self.signature.result());
        gradual::FrozenDescription::from_annotation(annotation, fuel).map_err(|error| match error {
            gradual::Error::Scope(error) => {
                Error::Signature(nir::deferred_signatures::Error::Scope(error))
            }
            _ => Error::SchedulingBudget,
        })
    }
}
impl fmt::Display for SignatureTask {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(signature-task {} (outputs", self.signature)?;
        for output in &self.outputs {
            write!(f, " {output}")?;
        }
        write!(f, ") (bindings")?;
        for binding in &self.bindings {
            write!(f, " {binding}")?;
        }
        write!(f, ") (instantiated-annotations")?;
        for annotation in &self.instantiated_annotations {
            match annotation {
                Some(ty) => write!(f, " (ready {ty})")?,
                None => write!(f, " (pending)")?,
            }
        }
        write!(f, ") {})", self.state)
    }
}

/// One package and one committed inference world. No Clone/trial operation is
/// offered: speculative inference does not roll back interpreter side effects.
pub struct Session<'a> {
    identity: u64,
    package: &'a Package,
    context: u64,
    interpreter: Interpreter<'a>,
    pending: BTreeMap<MetaId, Pending>,
    failed: BTreeSet<MetaId>,
}
impl<'a> Session<'a> {
    pub fn new(
        package: &'a Package,
        context: &InferenceContext,
        budget: Budget,
    ) -> Result<Self, Error> {
        Self::new_in_scope(package, context, budget, &[], &[], None)
    }
    /// Configure checked definition publication before any computation is
    /// queued. All waiters share this interpreter's generated catalog, so a
    /// completed Type can be consumed by subsequent calls without rebuilding
    /// the source package. The host owns publication and identity allocation;
    /// this does not publish definitions into the caller's source checker.
    pub fn new_with_struct_builder(
        package: &'a Package,
        context: &InferenceContext,
        budget: Budget,
        builder: &'a dyn Fn(
            &comptime_interpretor::StructBuildRequest,
            &mut u64,
        ) -> Result<
            comptime_interpretor::PreparedStructBuild<'a>,
            comptime_interpretor::Error,
        >,
    ) -> Result<Self, Error> {
        Ok(Self::new(package, context, budget)?.with_struct_builder(builder))
    }
    pub(crate) fn with_struct_builder(
        mut self,
        builder: &'a dyn Fn(
            &comptime_interpretor::StructBuildRequest,
            &mut u64,
        ) -> Result<
            comptime_interpretor::PreparedStructBuild<'a>,
            comptime_interpretor::Error,
        >,
    ) -> Self {
        self.interpreter = self.interpreter.with_struct_builder(builder);
        self
    }
    /// Capture one declaration's Type inputs and proof environment before any
    /// work is queued. The environment cannot change while waiters are active.
    pub fn new_in_scope(
        package: &'a Package,
        context: &InferenceContext,
        mut budget: Budget,
        assumptions: &[nir::program::TraitRequirement],
        parameters: &[DeclarationId],
        solver: Option<
            &'a dyn Fn(&nir::program::TypePredicate) -> comptime_interpretor::ContractOutcome,
        >,
    ) -> Result<Self, Error> {
        Self::new_in_scope_in_budget(
            package,
            context,
            &mut budget,
            assumptions,
            parameters,
            &[],
            solver,
        )
    }
    pub(crate) fn new_in_scope_in_budget(
        package: &'a Package,
        context: &InferenceContext,
        budget: &mut Budget,
        assumptions: &[nir::program::TraitRequirement],
        parameters: &[DeclarationId],
        value_parameters: &[(DeclarationId, Term)],
        solver: Option<
            &'a dyn Fn(&nir::program::TypePredicate) -> comptime_interpretor::ContractOutcome,
        >,
    ) -> Result<Self, Error> {
        if context.is_trial() {
            return Err(Error::SpeculativeContext);
        }
        let mut interpreter = Interpreter::for_comptime_in_scope_in_budget(
            package,
            assumptions,
            parameters,
            value_parameters,
            budget,
        )
        .map_err(Error::Invocation)?;
        if let Some(solver) = solver {
            interpreter = interpreter.with_type_solver(solver);
        }
        Ok(Self {
            identity: type_resolution::fresh_identity(),
            package,
            context: context.identity(),
            interpreter,
            pending: BTreeMap::new(),
            failed: BTreeSet::new(),
        })
    }

    /// Source invocations use compiler selection services instead of an eagerly
    /// materialized dispatch closure. With `start`, body and contract associated
    /// selections leave as owned requests; admission services remain synchronous.
    /// The scheduler owns committed computations and rejects speculative contexts.
    pub fn new_resolving_in_scope(
        package: &'a Package,
        context: &InferenceContext,
        code: &mut comptime_interpretor::CodeSession,
        budget: &mut Budget,
        assumptions: &[nir::program::TraitRequirement],
        parameters: &[DeclarationId],
        value_parameters: &[(DeclarationId, Term)],
        resolvers: comptime_interpretor::SourceResolvers<'a>,
    ) -> Result<Self, Error> {
        if context.is_trial() {
            return Err(Error::SpeculativeContext);
        }
        let interpreter = Interpreter::for_comptime_resolving_in_scope_in_session(
            package,
            code,
            assumptions,
            parameters,
            value_parameters,
            resolvers,
            budget,
        )
        .map_err(Error::Invocation)?;
        Ok(Self {
            identity: type_resolution::fresh_identity(),
            package,
            context: context.identity(),
            interpreter,
            pending: BTreeMap::new(),
            failed: BTreeSet::new(),
        })
    }

    fn check_context(&self, context: &InferenceContext) -> Result<(), Error> {
        if context.is_trial() {
            return Err(Error::SpeculativeContext);
        }
        if context.identity() != self.context {
            return Err(Error::ForeignContext);
        }
        Ok(())
    }
    /// Reserve a Type inference variable. Nothing invokes the function until
    /// both input domains and input values have settled.
    pub fn enqueue(
        &mut self,
        context: &mut InferenceContext,
        target: DeclarationId,
        inputs: Vec<Input>,
    ) -> Result<MetaId, Error> {
        self.check_context(context)?;
        let waiting = self.check_inputs(context, target, &inputs)?;
        let output = context.fresh(Sort::Type);
        self.pending.insert(
            output,
            Pending {
                target,
                inputs: PendingInputs::Symbolic(inputs),
                state: State::Waiting(waiting),
            },
        );
        Ok(output)
    }
    /// Queue completed values without copying their capture environments. Only
    /// the domains may wait on inference. The values are consumed at most once,
    /// including when invocation or result publication fails.
    pub fn enqueue_live(
        &mut self,
        context: &mut InferenceContext,
        target: DeclarationId,
        inputs: Vec<LiveInput>,
    ) -> Result<MetaId, Error> {
        self.enqueue_prepared(
            context,
            target,
            inputs
                .into_iter()
                .map(|input| PreparedInput {
                    ty: input.ty,
                    value: InvocationValue::Live(input.value),
                })
                .collect(),
        )
    }

    pub fn enqueue_prepared(
        &mut self,
        context: &mut InferenceContext,
        target: DeclarationId,
        inputs: Vec<PreparedInput>,
    ) -> Result<MetaId, Error> {
        self.check_context(context)?;
        self.check_target(target, inputs.len())?;
        let (domains, values) = inputs
            .into_iter()
            .map(|input| (input.ty, input.value))
            .unzip();
        let inputs = PendingInputs::Prepared {
            domains,
            values: Some(values),
        };
        let waiting = inputs.dependencies(context)?;
        let output = context.fresh(Sort::Type);
        self.pending.insert(
            output,
            Pending {
                target,
                inputs,
                state: State::Waiting(waiting),
            },
        );
        Ok(output)
    }
    fn check_inputs(
        &self,
        context: &InferenceContext,
        target: DeclarationId,
        inputs: &[Input],
    ) -> Result<BTreeSet<MetaId>, Error> {
        self.check_target(target, inputs.len())?;
        Ok(dependencies(context, inputs)?)
    }
    fn check_target(&self, target: DeclarationId, input_count: usize) -> Result<(), Error> {
        let function = self
            .package
            .functions
            .iter()
            .find(|f| f.declaration == target)
            .ok_or(Error::InvalidTarget)?;
        let Term::Function { parameters, result } = function.signature.callable_signature() else {
            return Err(Error::InvalidTarget);
        };
        if function.phase != FunctionPhase::Comptime
            || !function.type_parameters.is_empty()
            || **result != Term::Primitive(Primitive::Type)
        {
            return Err(Error::InvalidTarget);
        }
        if parameters.len() != input_count {
            return Err(Error::Arity);
        }
        Ok(())
    }
    pub fn state(&self, output: MetaId) -> Option<&State> {
        self.pending.get(&output).map(|p| &p.state)
    }
    pub fn remaining_steps(&self) -> u64 {
        self.interpreter.remaining_steps()
    }
    pub fn events(&self) -> &[comptime_interpretor::Event] {
        self.interpreter.events()
    }

    /// Instantiate a stable saved call into this invocation's inference world.
    /// Unused slots never become dependencies, including unrelated runtime slots.
    pub fn enqueue_residual(
        &mut self,
        context: &mut InferenceContext,
        residual: &nir::type_computations::Invocation,
        bindings: &[InferenceTerm],
    ) -> Result<MetaId, Error> {
        self.check_context(context)?;
        if residual.scope().len() != bindings.len() {
            return Err(Error::Arity);
        }
        let inputs = self.residual_inputs(context, residual, bindings)?;
        self.enqueue(context, residual.target(), inputs)
    }

    fn residual_inputs(
        &self,
        context: &InferenceContext,
        residual: &nir::type_computations::Invocation,
        bindings: &[InferenceTerm],
    ) -> Result<Vec<Input>, Error> {
        if residual.scope().len() != bindings.len() {
            return Err(Error::Arity);
        }
        for slot in residual.dependencies() {
            let actual = context.sort(&bindings[*slot])?;
            let expected = residual.scope()[*slot];
            if actual != expected {
                return Err(InferenceError::WrongSort { expected, actual }.into());
            }
        }
        let inputs = residual
            .inputs()
            .iter()
            .map(|input| Input {
                ty: InferenceTerm::Instantiate {
                    template: input.ty.clone(),
                    arguments: bindings.to_vec(),
                },
                value: InferenceTerm::Instantiate {
                    template: input.value.clone(),
                    arguments: bindings.to_vec(),
                },
            })
            .collect();
        Ok(inputs)
    }

    pub fn enqueue_signature(
        &mut self,
        context: &mut InferenceContext,
        signature: nir::deferred_signatures::Signature,
        bindings: &[InferenceTerm],
    ) -> Result<SignatureTask, Error> {
        self.check_context(context)?;
        if signature.inputs().len() != bindings.len() {
            return Err(Error::Arity);
        }
        let prepared = signature
            .computations()
            .iter()
            .map(|invocation| {
                let inputs = self.residual_inputs(context, invocation, bindings)?;
                self.check_inputs(context, invocation.target(), &inputs)?;
                Ok((invocation.target(), inputs))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let outputs = prepared
            .into_iter()
            .map(|(target, inputs)| self.enqueue(context, target, inputs))
            .collect::<Result<Vec<_>, _>>()?;
        let instantiated_annotations = vec![None; signature.inputs().len() + 1];
        Ok(SignatureTask {
            session: self.identity,
            signature,
            bindings: bindings.to_vec(),
            instantiated_annotations,
            outputs,
            state: State::Waiting(BTreeSet::new()),
        })
    }

    /// Resolve annotations in declaration order. A waiting or failed earlier
    /// annotation cannot execute a later one; a shared computation runs once.
    pub fn resume_signature<'s>(
        &mut self,
        context: &mut InferenceContext,
        task: &'s mut SignatureTask,
        fuel: &mut usize,
    ) -> Result<&'s State, Error> {
        self.check_context(context)?;
        if task.session != self.identity {
            return Err(Error::ForeignSession);
        }
        if !matches!(task.state, State::Waiting(_)) {
            return Ok(&task.state);
        }
        self.advance_signature(context, task, task.signature.inputs().len(), fuel)?;
        if !matches!(&task.state,State::Waiting(inputs) if inputs.is_empty()) {
            return Ok(&task.state);
        }
        let results = task
            .outputs
            .iter()
            .map(|output| match self.state(*output) {
                Some(State::Complete(ty)) => Ok(ty),
                _ => Err(Error::UnknownComputation),
            })
            .collect::<Result<Vec<_>, _>>()?;
        task.state = match task.signature.resolve(&results, fuel) {
            Ok(signature) => State::Complete(signature),
            Err(nir::deferred_signatures::Error::Budget) => return Err(Error::SchedulingBudget),
            Err(error) => State::Failed(Error::Signature(error)),
        };
        Ok(&task.state)
    }

    /// Demand only a prefix, allowing a caller to check and infer one argument
    /// before demanding annotations which depend on later staged inputs.
    /// Slots 0..inputs.len() select parameters; inputs.len() selects the result.
    /// This does not publish a completed function type or guess waiting types.
    pub fn resume_signature_annotation<'s>(
        &'s mut self,
        context: &mut InferenceContext,
        task: &'s mut SignatureTask,
        slot: usize,
        fuel: &mut usize,
    ) -> Result<AnnotationState<'s>, Error> {
        self.check_context(context)?;
        if task.session != self.identity {
            return Err(Error::ForeignSession);
        }
        if slot > task.signature.inputs().len() {
            return Err(Error::InvalidAnnotation(slot));
        }
        if matches!(task.state, State::Waiting(_)) {
            self.advance_signature(context, task, slot, fuel)?;
        }
        self.signature_annotation_state(task, slot)
    }

    /// Check each ready annotation before demanding the next computation.
    /// The callback owns staged substitution and refinement admission in the
    /// caller's context; it must not use definition-side hypothetical facts.
    /// On rejection, completed invocations remain cached, so retrying after
    /// new caller evidence never replays their effects. Admission itself is
    /// deliberately rechecked rather than cached across changing contexts.
    pub fn resume_signature_annotation_checked<'s, E>(
        &'s mut self,
        context: &mut InferenceContext,
        task: &'s mut SignatureTask,
        slot: usize,
        fuel: &mut usize,
        mut admit: impl FnMut(usize, &Term, &InferenceContext) -> Result<(), E>,
    ) -> Result<AnnotationState<'s>, AdmissionError<E>> {
        self.check_context(context)
            .map_err(AdmissionError::Computation)?;
        if task.session != self.identity {
            return Err(AdmissionError::Computation(Error::ForeignSession));
        }
        if slot > task.signature.inputs().len() {
            return Err(AdmissionError::Computation(Error::InvalidAnnotation(slot)));
        }
        for index in 0..=slot {
            if matches!(task.state, State::Waiting(_)) {
                self.advance_signature(context, task, index, fuel)
                    .map_err(AdmissionError::Computation)?;
            }
            match self
                .signature_annotation_state(task, index)
                .map_err(AdmissionError::Computation)?
            {
                AnnotationState::Ready(ty) => {
                    *fuel = fuel
                        .checked_sub(1)
                        .ok_or(AdmissionError::Computation(Error::SchedulingBudget))?;
                    admit(index, ty, context).map_err(|reason| AdmissionError::Rejected {
                        slot: index,
                        reason,
                    })?;
                }
                _ => {
                    return self
                        .signature_annotation_state(task, index)
                        .map_err(AdmissionError::Computation);
                }
            }
        }
        self.signature_annotation_state(task, slot)
            .map_err(AdmissionError::Computation)
    }

    /// Caller-side admission: substitute the invocation's input bindings into
    /// declaration annotations before checking each parameter/result. Computed
    /// Types are already invocation results and must not be substituted again.
    /// A known annotation with unresolved inputs stops this prefix just like a
    /// waiting computation. No later annotation invokes code before admission.
    /// Completed substitutions are retained, but admission is always rechecked
    /// against the caller's current evidence. This never publishes Any as Type.
    pub fn resume_instantiated_signature_annotation_checked<'s, E>(
        &'s mut self,
        context: &mut InferenceContext,
        task: &'s mut SignatureTask,
        slot: usize,
        fuel: &mut usize,
        mut admit: impl FnMut(usize, &Term, &InferenceContext) -> Result<(), E>,
    ) -> Result<AnnotationState<'s>, AdmissionError<E>> {
        let computation = AdmissionError::Computation;
        self.check_context(context).map_err(computation)?;
        if task.session != self.identity {
            return Err(computation(Error::ForeignSession));
        }
        if slot > task.signature.inputs().len() {
            return Err(computation(Error::InvalidAnnotation(slot)));
        }
        if matches!(task.state, State::Failed(_)) {
            return self
                .signature_annotation_state(task, slot)
                .map_err(computation);
        }
        for index in 0..=slot {
            *fuel = fuel
                .checked_sub(1)
                .ok_or_else(|| computation(Error::SchedulingBudget))?;
            if task.instantiated_annotations[index].is_none() {
                let annotation = task
                    .signature
                    .inputs()
                    .get(index)
                    .map(|input| &input.annotation)
                    .unwrap_or_else(|| task.signature.result());
                let ty = match annotation {
                    nir::deferred_signatures::Annotation::Known(template) => {
                        nir::type_computations::charge_term(template, fuel)
                            .map_err(|_| computation(Error::SchedulingBudget))?;
                        *fuel = fuel
                            .checked_sub(task.bindings.len())
                            .ok_or_else(|| computation(Error::SchedulingBudget))?;
                        let input = InferenceTerm::Instantiate {
                            template: template.clone(),
                            arguments: task.bindings.clone(),
                        };
                        let inference = |error| {
                            computation(if error == InferenceError::Budget {
                                Error::SchedulingBudget
                            } else {
                                Error::Inference(error)
                            })
                        };
                        let waiting = context
                            .unresolved_metered(&input, fuel)
                            .map_err(inference)?;
                        // One annotation is one demand: preflight all its
                        // producers before allowing any of them to execute.
                        // A later cycle/failure must not run an earlier sibling.
                        match self.resume_input_demands(context, &waiting, fuel) {
                            Ok(DemandState::Ready) => {}
                            Ok(DemandState::Waiting(leaves)) => {
                                task.state = State::Waiting(leaves);
                                return self
                                    .signature_annotation_state(task, index)
                                    .map_err(computation);
                            }
                            Ok(DemandState::Failed(output)) => {
                                task.state = State::Failed(Error::DependencyFailed(output));
                                return self
                                    .signature_annotation_state(task, index)
                                    .map_err(computation);
                            }
                            Err(error) => return Err(computation(error)),
                        }
                        context.freeze_metered(&input, fuel).map_err(inference)?
                    }
                    nir::deferred_signatures::Annotation::Computed(output) => {
                        let output = task.outputs[*output];
                        match self
                            .resume_dependencies(context, output, fuel)
                            .map_err(computation)?
                        {
                            State::Complete(ty) => {
                                nir::type_computations::charge_term(ty, fuel)
                                    .map_err(|_| computation(Error::SchedulingBudget))?;
                                ty.clone()
                            }
                            State::Waiting(inputs) => {
                                task.state = State::Waiting(inputs.clone());
                                return self
                                    .signature_annotation_state(task, index)
                                    .map_err(computation);
                            }
                            State::Failed(_) => {
                                task.state = State::Failed(Error::DependencyFailed(output));
                                return self
                                    .signature_annotation_state(task, index)
                                    .map_err(computation);
                            }
                        }
                    }
                };
                task.instantiated_annotations[index] = Some(ty);
            }
            // Clear an old prefix wait only after the same annotation settles.
            if matches!(task.state, State::Waiting(_)) {
                task.state = State::Waiting(BTreeSet::new());
            }
            let ty = task.instantiated_annotations[index]
                .as_ref()
                .expect("resolved annotation");
            admit(index, ty, context).map_err(|reason| AdmissionError::Rejected {
                slot: index,
                reason,
            })?;
        }
        Ok(AnnotationState::Ready(
            task.instantiated_annotations[slot]
                .as_ref()
                .expect("resolved prefix"),
        ))
    }

    /// Complete caller substitution and admission before dropping staged input
    /// slots. Unused runtime values need not be inferred just to form their
    /// types. Every retry rechecks admission; already executed computations are
    /// retained by the task and are never replayed by signature projection.
    pub fn resume_runtime_signature_checked<'s, E>(
        &'s mut self,
        context: &mut InferenceContext,
        task: &'s mut SignatureTask,
        fuel: &mut usize,
        admit: impl FnMut(usize, &Term, &InferenceContext) -> Result<(), E>,
    ) -> Result<RuntimeSignatureState<'s>, AdmissionError<E>> {
        let slot = task.signature.inputs().len();
        // End the prefix API's borrow before inspecting its cached annotations.
        self.resume_instantiated_signature_annotation_checked(context, task, slot, fuel, admit)?;
        match &task.state {
            State::Failed(error) => return Ok(RuntimeSignatureState::Failed(error)),
            State::Waiting(inputs) if !inputs.is_empty() => {
                return Ok(RuntimeSignatureState::Waiting(inputs));
            }
            _ => {}
        }
        let computation = AdmissionError::Computation;
        *fuel = fuel
            .checked_sub(slot.saturating_add(1))
            .ok_or_else(|| computation(Error::SchedulingBudget))?;
        let mut parameters = Vec::new();
        for (index, input) in task.signature.inputs().iter().enumerate() {
            if input.sort != Sort::Runtime {
                continue;
            }
            let ty = task.instantiated_annotations[index]
                .as_ref()
                .ok_or_else(|| computation(Error::UnknownComputation))?;
            nir::type_computations::charge_term(ty, fuel)
                .map_err(|_| computation(Error::SchedulingBudget))?;
            // The invocation has already substituted its original telescope.
            // A free declaration binder must never be captured by the new one.
            ty.check_scope(&[]).map_err(|error| {
                computation(Error::Signature(nir::deferred_signatures::Error::Scope(
                    error,
                )))
            })?;
            parameters.push(nir::Parameter {
                sort: Sort::Runtime,
                ty: ty.clone(),
            });
        }
        let result = task.instantiated_annotations[slot]
            .as_ref()
            .ok_or_else(|| computation(Error::UnknownComputation))?;
        nir::type_computations::charge_term(result, fuel)
            .map_err(|_| computation(Error::SchedulingBudget))?;
        result.check_scope(&[]).map_err(|error| {
            computation(Error::Signature(nir::deferred_signatures::Error::Scope(
                error,
            )))
        })?;
        Ok(RuntimeSignatureState::Ready(Term::Function {
            parameters,
            result: Box::new(result.clone()),
        }))
    }

    fn signature_annotation_state<'s>(
        &'s self,
        task: &'s SignatureTask,
        slot: usize,
    ) -> Result<AnnotationState<'s>, Error> {
        match &task.state {
            State::Failed(error) => return Ok(AnnotationState::Failed(error)),
            State::Waiting(inputs) if !inputs.is_empty() => {
                return Ok(AnnotationState::Waiting(inputs));
            }
            _ => {}
        }
        let annotation = task
            .signature
            .inputs()
            .get(slot)
            .map(|input| &input.annotation)
            .unwrap_or_else(|| task.signature.result());
        let ty = match annotation {
            nir::deferred_signatures::Annotation::Known(ty) => ty,
            nir::deferred_signatures::Annotation::Computed(index) => {
                match self.state(task.outputs[*index]) {
                    Some(State::Complete(ty)) => ty,
                    _ => return Err(Error::UnknownComputation),
                }
            }
        };
        Ok(AnnotationState::Ready(ty))
    }

    fn advance_signature(
        &mut self,
        context: &mut InferenceContext,
        task: &mut SignatureTask,
        through: usize,
        fuel: &mut usize,
    ) -> Result<(), Error> {
        // Resolve in first-use order, independent of computation storage order.
        let mut order = Vec::new();
        let mut seen = BTreeSet::new();
        for annotation in task
            .signature
            .inputs()
            .iter()
            .map(|i| &i.annotation)
            .chain(std::iter::once(task.signature.result()))
            .take(through + 1)
        {
            *fuel = fuel.checked_sub(1).ok_or(Error::SchedulingBudget)?;
            if let nir::deferred_signatures::Annotation::Computed(index) = annotation {
                if seen.insert(*index) {
                    order.push(*index);
                }
            }
        }
        for index in order {
            let output = task.outputs[index];
            match self.resume_dependencies(context, output, fuel)? {
                State::Waiting(inputs) => {
                    task.state = State::Waiting(inputs.clone());
                    return Ok(());
                }
                State::Failed(_) => {
                    task.state = State::Failed(Error::DependencyFailed(output));
                    return Ok(());
                }
                State::Complete(_) => {}
            }
        }
        task.state = State::Waiting(BTreeSet::new());
        Ok(())
    }

    /// Drive only the computations reachable from this demanded output.
    /// Plan before executing so a cycle or exhausted scheduling budget cannot
    /// partially invoke an otherwise ready branch. External inference leaves
    /// remain waiters; they are never guessed or assigned by this scheduler.
    /// A waiting demand reports the external leaves of the whole dependency
    /// graph, so callers can subscribe to the inputs which can actually wake it.
    pub fn resume_dependencies(
        &mut self,
        context: &mut InferenceContext,
        output: MetaId,
        fuel: &mut usize,
    ) -> Result<&State, Error> {
        self.check_context(context)?;
        if !self.is_waiting(output)? {
            return Ok(&self.pending[&output].state);
        }
        let plan = match self.plan_demand(context, output, fuel) {
            Ok(plan) => plan,
            Err(
                error @ (Error::DependencyCycle(_)
                | Error::Inference(InferenceError::AlreadyAssigned(_))),
            ) => {
                return self.fail_demand(output, error);
            }
            Err(error) => return Err(error),
        };
        let external = self.execute_demand_plan(context, plan)?;
        let pending = self.pending.get_mut(&output).expect("planned demand");
        if matches!(pending.state, State::Waiting(_)) {
            pending.state = State::Waiting(external);
        }
        Ok(&pending.state)
    }

    fn resume_input_demands(
        &mut self,
        context: &mut InferenceContext,
        outputs: &BTreeSet<MetaId>,
        fuel: &mut usize,
    ) -> Result<DemandState, Error> {
        let plan = match self.plan_demands(context, outputs.iter().copied(), fuel) {
            Ok(plan) => plan,
            Err(demand::Failure::Unresolved(error)) => return Err(error),
            Err(demand::Failure::Rejected { output, reason }) => {
                self.fail_demand(output, reason)?;
                return Ok(DemandState::Failed(output));
            }
        };
        let external = self.execute_demand_plan(context, plan)?;
        for output in outputs {
            if matches!(self.state(*output), Some(State::Failed(_))) {
                return Ok(DemandState::Failed(*output));
            }
        }
        Ok(if external.is_empty() {
            DemandState::Ready
        } else {
            DemandState::Waiting(external)
        })
    }

    fn execute_demand_plan(
        &mut self,
        context: &mut InferenceContext,
        plan: demand::Plan,
    ) -> Result<BTreeSet<MetaId>, Error> {
        // The exclusive context cannot acquire new edges during this pass:
        // invocation publishes stable Types only. Retain the observed graph
        // instead of giving every execution/failure scan a fresh allowance.
        let mut failed_dependency = plan.failed;
        for computation in plan.order {
            let mut waiting = computation.inputs;
            if failed_dependency && waiting.is_disjoint(&self.failed) {
                continue;
            }
            waiting.retain(|input| !matches!(self.state(*input), Some(State::Complete(_))));
            if matches!(
                self.resume_prepared(context, computation.output, waiting)?,
                State::Failed(_)
            ) {
                failed_dependency = true;
            }
        }
        Ok(plan.external)
    }

    /// Recheck current alias leaves, then invoke at most once. The caller may
    /// resume dependents when this output becomes Complete; waiting is not an
    /// evaluation failure and never manufactures a provisional Type.
    pub fn resume(
        &mut self,
        context: &mut InferenceContext,
        output: MetaId,
    ) -> Result<&State, Error> {
        let mut fuel = DEFAULT_SCHEDULING_STEPS;
        self.resume_bounded(context, output, &mut fuel)
    }

    /// Resume this output only, using one caller-owned scheduling allowance for
    /// its dependency preflight. Exhaustion leaves queued inputs untouched and
    /// invokes nothing. Dependency bodies are driven only by resume_dependencies.
    pub fn resume_bounded(
        &mut self,
        context: &mut InferenceContext,
        output: MetaId,
        fuel: &mut usize,
    ) -> Result<&State, Error> {
        self.resume_one(context, output, fuel)
    }

    /// Start one ready producer with an exclusively owned invocation progress.
    /// A dependency wait invokes nothing; dropping active execution cancels it.
    pub fn start<'s>(
        &'s mut self,
        context: &mut InferenceContext,
        output: MetaId,
        fuel: &mut usize,
    ) -> Result<InvocationStart<'s, 'a>, Error> {
        let Some(waiting) = self.prepare_resume(context, output, fuel)? else {
            return Ok(InvocationStart::Settled(&self.pending[&output].state));
        };
        if !waiting.is_empty() {
            self.pending.get_mut(&output).unwrap().state = State::Waiting(waiting);
            return Ok(InvocationStart::Settled(&self.pending[&output].state));
        }
        Ok(execution::begin(
            &mut self.interpreter,
            context,
            self.pending.get_mut(&output).unwrap(),
            &mut self.failed,
            output,
        ))
    }

    fn is_waiting(&self, output: MetaId) -> Result<bool, Error> {
        self.pending
            .get(&output)
            .map(|pending| matches!(pending.state, State::Waiting(_)))
            .ok_or(Error::UnknownComputation)
    }

    fn fail_demand(&mut self, output: MetaId, error: Error) -> Result<&State, Error> {
        let pending = self
            .pending
            .get_mut(&output)
            .ok_or(Error::UnknownComputation)?;
        pending.state = State::Failed(error);
        self.failed.insert(output);
        Ok(&pending.state)
    }

    fn prepare_resume(
        &mut self,
        context: &mut InferenceContext,
        output: MetaId,
        fuel: &mut usize,
    ) -> Result<Option<BTreeSet<MetaId>>, Error> {
        self.check_context(context)?;
        if !self.is_waiting(output)? {
            return Ok(None);
        }
        let mut plan = match self.plan_demand(context, output, fuel) {
            Ok(plan) => plan,
            Err(
                error @ (Error::DependencyCycle(_)
                | Error::Inference(InferenceError::AlreadyAssigned(_))),
            ) => {
                self.fail_demand(output, error)?;
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        let computation = plan.order.pop().expect("waiting root is planned last");
        debug_assert_eq!(computation.output, output);
        if let Some(dependency) = computation.inputs.intersection(&self.failed).next() {
            self.fail_demand(output, Error::DependencyFailed(*dependency))?;
            return Ok(None);
        }
        Ok(Some(computation.inputs))
    }

    fn resume_one(
        &mut self,
        context: &mut InferenceContext,
        output: MetaId,
        fuel: &mut usize,
    ) -> Result<&State, Error> {
        let Some(waiting) = self.prepare_resume(context, output, fuel)? else {
            return Ok(&self.pending[&output].state);
        };
        self.resume_prepared(context, output, waiting)
    }

    fn resume_prepared(
        &mut self,
        context: &mut InferenceContext,
        output: MetaId,
        waiting: BTreeSet<MetaId>,
    ) -> Result<&State, Error> {
        let pending = self
            .pending
            .get_mut(&output)
            .ok_or(Error::UnknownComputation)?;
        if !matches!(pending.state, State::Waiting(_)) {
            return Ok(&pending.state);
        }
        // Dependency failures and externally assigned outputs settle before
        // consuming inputs or entering an invocation. Ready work uses the same
        // owner and publication boundary as explicitly suspended execution.
        let failure = match context.is_assigned(output) {
            Err(error) => Some(Error::Inference(error)),
            Ok(true) => Some(Error::Inference(InferenceError::AlreadyAssigned(output))),
            Ok(false) => waiting
                .intersection(&self.failed)
                .next()
                .copied()
                .map(Error::DependencyFailed),
        };
        if let Some(error) = failure {
            pending.state = State::Failed(error);
            self.failed.insert(output);
        } else if !waiting.is_empty() {
            pending.state = State::Waiting(waiting);
        } else {
            execution::drive(
                &mut self.interpreter,
                context,
                pending,
                &mut self.failed,
                output,
            );
        }
        Ok(&pending.state)
    }
}
fn dependencies(
    context: &InferenceContext,
    inputs: &[Input],
) -> Result<BTreeSet<MetaId>, InferenceError> {
    let mut fuel = DEFAULT_SCHEDULING_STEPS;
    dependencies_metered(context, inputs, &mut fuel)
}
fn dependencies_metered(
    context: &InferenceContext,
    inputs: &[Input],
    fuel: &mut usize,
) -> Result<BTreeSet<MetaId>, InferenceError> {
    let mut result = BTreeSet::new();
    for input in inputs {
        result.extend(context.unresolved_metered(&input.ty, fuel)?);
        result.extend(context.unresolved_metered(&input.value, fuel)?);
    }
    Ok(result)
}
impl fmt::Display for Session<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(type-computations")?;
        for (output, pending) in &self.pending {
            write!(
                f,
                " (computation (output {output}) (target {}) (inputs",
                pending.target
            )?;
            match &pending.inputs {
                PendingInputs::Symbolic(inputs) => {
                    for input in inputs {
                        write!(f, " (input (type {}) (value {}))", input.ty, input.value)?;
                    }
                }
                PendingInputs::Prepared { domains, values } => {
                    for ty in domains {
                        write!(f, " (prepared-input (type {ty}))")?;
                    }
                    write!(f, " (consumed {})", values.is_none())?;
                }
            }
            write!(f, ") {})", pending.state)?;
        }
        write!(f, ")")
    }
}
