//! Invocation-local waiters for ordinary comptime functions returning Type.
//! These are elaboration state, not normal-form heads or serializable NIR types.
//! Calls run only in the committed inference context. A failed call is terminal:
//! retrying inference must not replay its comptime effects.
use crate::{InferenceContext, InferenceError, InferenceTerm, MetaId};
use comptime_interpretor::{Budget, Interpreter, InvocationValue, Value};
use nir::program::{FunctionPhase, Package};
use nir::{DeclarationId, Primitive, Sort, Term};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

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
        self.dependencies_metered(context, &mut 100_000)
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
    ) -> Result<crate::gradual::SignatureDescription, Error> {
        crate::gradual::SignatureDescription::from_residual(&self.signature, fuel).map_err(
            |error| match error {
                crate::gradual::Error::Scope(error) => {
                    Error::Signature(nir::deferred_signatures::Error::Scope(error))
                }
                _ => Error::SchedulingBudget,
            },
        )
    }

    /// Ordinary checking observes stable declaration information, never the
    /// task's inference outputs as provisional Types. The residual computation
    /// remains in the saved signature; completing one instance does not refine
    /// the declaration's description for other instances.
    pub fn definition_description(
        &self,
        slot: usize,
        fuel: &mut usize,
    ) -> Result<crate::gradual::FrozenDescription, Error> {
        if slot > self.signature.inputs().len() {
            return Err(Error::InvalidAnnotation(slot));
        }
        let annotation = self
            .signature
            .inputs()
            .get(slot)
            .map(|input| &input.annotation)
            .unwrap_or_else(|| self.signature.result());
        crate::gradual::FrozenDescription::from_annotation(annotation, fuel).map_err(|error| {
            match error {
                crate::gradual::Error::Scope(error) => {
                    Error::Signature(nir::deferred_signatures::Error::Scope(error))
                }
                _ => Error::SchedulingBudget,
            }
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

pub(crate) enum ProofSolver<'a> {
    Unmetered(&'a dyn Fn(&nir::program::TypePredicate) -> comptime_interpretor::ContractOutcome),
    Budgeted(
        &'a dyn Fn(&nir::program::TypePredicate, &mut u64) -> comptime_interpretor::ContractOutcome,
    ),
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
    pub(crate) fn with_enum_builder(
        mut self,
        builder: &'a dyn Fn(
            &comptime_interpretor::EnumBuildRequest,
            &mut u64,
        ) -> Result<
            comptime_interpretor::PreparedEnumBuild<'a>,
            comptime_interpretor::Error,
        >,
    ) -> Self {
        self.interpreter = self.interpreter.with_enum_builder(builder);
        self
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
        Self::new_in_scope_with_pointer_copy(
            package,
            context,
            &mut budget,
            assumptions,
            parameters,
            &[],
            solver.map(ProofSolver::Unmetered),
            None,
        )
    }
    pub(crate) fn new_in_scope_with_pointer_copy(
        package: &'a Package,
        context: &InferenceContext,
        budget: &mut Budget,
        assumptions: &[nir::program::TraitRequirement],
        parameters: &[DeclarationId],
        value_parameters: &[(DeclarationId, Term)],
        solver: Option<ProofSolver<'a>>,
        pointer_copy: Option<&'a dyn Fn(&Term, &mut u64) -> bool>,
    ) -> Result<Self, Error> {
        if context.is_trial() {
            return Err(Error::SpeculativeContext);
        }
        let mut interpreter = Interpreter::for_comptime_in_budget(package, assumptions, budget)
            .map_err(Error::Invocation)?;
        let scope = interpreter
            .authorize_abstract_type_parameters(parameters)
            .and_then(|()| interpreter.authorize_abstract_value_parameters(value_parameters));
        budget.steps = interpreter.remaining_steps();
        scope.map_err(Error::Invocation)?;
        if let Some(solver) = solver {
            interpreter = match solver {
                ProofSolver::Unmetered(solver) => interpreter.with_type_solver(solver),
                ProofSolver::Budgeted(solver) => interpreter.with_budgeted_type_solver(solver),
            };
        }
        if let Some(permission) = pointer_copy {
            interpreter = interpreter.with_budgeted_pointer_copy_permission(permission);
        }
        Ok(Self {
            identity: crate::unique(),
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
                        // A declaration annotation can refer to a Type produced
                        // by another queued invocation, not just a caller-owned
                        // inference leaf. Demand those producers through the
                        // same bounded dependency scheduler as computed slots.
                        // Do not drive unrelated work or execute later slots.
                        if let Some(output) =
                            waiting.iter().find(|output| self.failed.contains(output))
                        {
                            task.state = State::Failed(Error::DependencyFailed(*output));
                            return self
                                .signature_annotation_state(task, index)
                                .map_err(computation);
                        }
                        for output in &waiting {
                            if !self.pending.contains_key(output) {
                                continue;
                            }
                            if matches!(
                                self.resume_dependencies(context, *output, fuel)
                                    .map_err(computation)?,
                                State::Failed(_)
                            ) {
                                task.state = State::Failed(Error::DependencyFailed(*output));
                                return self
                                    .signature_annotation_state(task, index)
                                    .map_err(computation);
                            }
                        }
                        let waiting = context
                            .unresolved_metered(&input, fuel)
                            .map_err(inference)?;
                        if !waiting.is_empty() {
                            let mut leaves = BTreeSet::new();
                            for output in waiting {
                                match self.state(output) {
                                    Some(State::Waiting(inputs)) => {
                                        *fuel = fuel
                                            .checked_sub(inputs.len())
                                            .ok_or_else(|| computation(Error::SchedulingBudget))?;
                                        leaves.extend(inputs.iter().copied());
                                    }
                                    _ => {
                                        *fuel = fuel
                                            .checked_sub(1)
                                            .ok_or_else(|| computation(Error::SchedulingBudget))?;
                                        leaves.insert(output);
                                    }
                                }
                            }
                            task.state = State::Waiting(leaves);
                            return self
                                .signature_annotation_state(task, index)
                                .map_err(computation);
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

    fn check_cycle(&self, context: &InferenceContext, output: MetaId) -> Result<(), Error> {
        let mut work = vec![(output, false)];
        let mut active = BTreeSet::new();
        let mut done = BTreeSet::new();
        while let Some((id, leave)) = work.pop() {
            if leave {
                active.remove(&id);
                done.insert(id);
                continue;
            }
            if done.contains(&id) {
                continue;
            }
            let Some(pending) = self.pending.get(&id) else {
                continue;
            };
            if !matches!(pending.state, State::Waiting(_)) {
                continue;
            }
            if !active.insert(id) {
                return Err(Error::DependencyCycle(id));
            }
            work.push((id, true));
            for dependency in pending.inputs.dependencies(context)? {
                work.push((dependency, false));
            }
        }
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
        let pending = self.pending.get(&output).ok_or(Error::UnknownComputation)?;
        if !matches!(pending.state, State::Waiting(_)) {
            return self.resume(context, output);
        }
        let mut work = vec![(output, false)];
        let mut active = BTreeSet::new();
        let mut done = BTreeSet::new();
        let mut order = Vec::new();
        let mut external = BTreeSet::new();
        let mut failed_dependency = false;
        while let Some((id, leave)) = work.pop() {
            *fuel = fuel.checked_sub(1).ok_or(Error::SchedulingBudget)?;
            if leave {
                active.remove(&id);
                done.insert(id);
                order.push(id);
                continue;
            }
            if done.contains(&id) {
                continue;
            }
            let Some(pending) = self.pending.get(&id) else {
                external.insert(id);
                continue;
            };
            match pending.state {
                State::Failed(_) => {
                    failed_dependency = true;
                    continue;
                }
                State::Complete(_) => continue,
                State::Waiting(_) => {}
            }
            if !active.insert(id) {
                // Preserve resume's terminal failure and propagation contract.
                return self.resume(context, output);
            }
            if context.is_assigned(id)? {
                self.resume(context, id)?;
                return self.resume(context, output);
            }
            let inputs = pending
                .inputs
                .dependencies_metered(context, fuel)
                .map_err(|error| {
                    if error == InferenceError::Budget {
                        Error::SchedulingBudget
                    } else {
                        Error::Inference(error)
                    }
                })?;
            *fuel = fuel
                .checked_sub(inputs.len())
                .ok_or(Error::SchedulingBudget)?;
            work.push((id, true));
            for input in inputs.into_iter().rev() {
                work.push((input, false));
            }
        }
        // Publication produces stable Types and cannot introduce new inference
        // edges. The preflight therefore covers this entire synchronous pass;
        // do not rescan each node's transitive graph for cycles again.
        for id in order {
            if failed_dependency {
                // Continue publishing failure along the demanded path, but do
                // not run independent siblings after the demand has failed.
                let inputs = self.pending[&id].inputs.dependencies(context)?;
                if inputs.is_disjoint(&self.failed) {
                    continue;
                }
            }
            if matches!(
                self.resume_after_cycle_check(context, id, Ok(()))?,
                State::Failed(_)
            ) {
                failed_dependency = true;
            }
        }
        let pending = self
            .pending
            .get_mut(&output)
            .ok_or(Error::UnknownComputation)?;
        if matches!(pending.state, State::Waiting(_)) {
            pending.state = State::Waiting(external);
        }
        Ok(&pending.state)
    }

    /// Recheck current alias leaves, then invoke at most once. The caller may
    /// resume dependents when this output becomes Complete; waiting is not an
    /// evaluation failure and never manufactures a provisional Type.
    pub fn resume(
        &mut self,
        context: &mut InferenceContext,
        output: MetaId,
    ) -> Result<&State, Error> {
        self.check_context(context)?;
        let acyclic = self.check_cycle(context, output);
        self.resume_after_cycle_check(context, output, acyclic)
    }
    fn resume_after_cycle_check(
        &mut self,
        context: &mut InferenceContext,
        output: MetaId,
        acyclic: Result<(), Error>,
    ) -> Result<&State, Error> {
        let pending = self
            .pending
            .get_mut(&output)
            .ok_or(Error::UnknownComputation)?;
        if !matches!(pending.state, State::Waiting(_)) {
            return Ok(&pending.state);
        }
        let result = (|| {
            acyclic?;
            // Check this before invocation so an externally assigned output
            // cannot cause effects followed by a failed publication.
            if context.is_assigned(output)? {
                return Err(Error::Inference(InferenceError::AlreadyAssigned(output)));
            }
            let waiting = pending.inputs.dependencies(context)?;
            if let Some(dependency) = waiting.intersection(&self.failed).next() {
                return Err(Error::DependencyFailed(*dependency));
            }
            if !waiting.is_empty() {
                return Ok(State::Waiting(waiting));
            }
            let mut fuel =
                usize::try_from(self.interpreter.remaining_steps()).unwrap_or(usize::MAX);
            let before = fuel;
            let value = match &mut pending.inputs {
                PendingInputs::Symbolic(inputs) => {
                    let arguments = inputs
                        .iter()
                        .map(|input| {
                            Ok((
                                context.freeze_metered(&input.ty, &mut fuel)?,
                                context.freeze_metered(&input.value, &mut fuel)?,
                            ))
                        })
                        .collect::<Result<Vec<_>, InferenceError>>();
                    self.interpreter
                        .charge_preparation(before - fuel)
                        .map_err(Error::Invocation)?;
                    self.interpreter
                        .invoke_with_normal_arguments(pending.target, arguments?)
                        .map_err(Error::Invocation)?
                }
                PendingInputs::Prepared { domains, values } => {
                    let domains = domains
                        .iter()
                        .map(|ty| context.freeze_metered(ty, &mut fuel))
                        .collect::<Result<Vec<_>, _>>();
                    self.interpreter
                        .charge_preparation(before - fuel)
                        .map_err(Error::Invocation)?;
                    let domains = domains?;
                    let values = values.take().ok_or(Error::ConsumedInputs)?;
                    self.interpreter
                        .invoke_with_prepared_arguments(
                            pending.target,
                            domains.into_iter().zip(values).collect(),
                        )
                        .map_err(Error::Invocation)?
                }
            };
            let ty = match value {
                Value::Type(ty) => ty,
                Value::TypeParameter(declaration) => Term::Rigid {
                    declaration,
                    sort: Sort::Type,
                },
                _ => return Err(Error::NotAType),
            };
            let mut fuel =
                usize::try_from(self.interpreter.remaining_steps()).unwrap_or(usize::MAX);
            let before = fuel;
            let bounded = nir::type_computations::charge_term(&ty, &mut fuel);
            self.interpreter
                .charge_preparation(before - fuel)
                .map_err(Error::Invocation)?;
            bounded.map_err(|_| Error::Inference(InferenceError::Budget))?;
            context.assign(output, InferenceTerm::Stable(ty.clone()))?;
            Ok(State::Complete(ty))
        })();
        pending.state = match result {
            Ok(state) => state,
            Err(error) => State::Failed(error),
        };
        if matches!(pending.state, State::Failed(_)) {
            self.failed.insert(output);
        }
        Ok(&pending.state)
    }
}
fn dependencies(
    context: &InferenceContext,
    inputs: &[Input],
) -> Result<BTreeSet<MetaId>, InferenceError> {
    dependencies_metered(context, inputs, &mut 100_000)
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

#[cfg(test)]
mod admission_budget_tests {
    use super::*;

    #[test]
    fn failed_session_world_creation_keeps_the_allowance_reserved() {
        let package = Package::default();
        let context = InferenceContext::default();
        let assumptions = [nir::program::TraitRequirement {
            interface_arguments: vec![],
            subject: Term::Primitive(nir::Primitive::Bool),
            interface: DeclarationId {
                package: 1,
                local: 999,
            },
        }];
        let mut budget = Budget {
            steps: 100,
            depth: 128,
        };
        let failed = Session::new_in_scope_with_pointer_copy(
            &package,
            &context,
            &mut budget,
            &assumptions,
            &[],
            &[],
            None,
            None,
        );
        assert!(matches!(failed, Err(Error::Invocation(_))));
        assert_eq!(budget.steps, 0);
        assert_eq!(budget.depth, 128);
        budget.steps = 100;
        let session = Session::new_in_scope_with_pointer_copy(
            &package,
            &context,
            &mut budget,
            &[],
            &[],
            &[],
            None,
            None,
        )
        .unwrap();
        assert_eq!(session.remaining_steps(), budget.steps);
        // Successful scope authorization can consume work even in an empty
        // declaration world. Synchronize the actual remainder, not the grant.
        assert!(budget.steps > 0 && budget.steps <= 100);
    }
}
