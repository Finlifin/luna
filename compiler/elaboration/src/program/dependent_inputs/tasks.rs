//! Dependency checking retains its continuation and default expression explicitly.
//! The work stack is a checking schedule, never an execution schedule or VM frame.
use super::*;
use default_writes::{Read, Writes};

enum Task {
    Enter(usize),
    Check(usize),
    Default {
        slot: usize,
        value: n::Expr,
        references: BTreeMap<n::LocalId, usize>,
        deferred: BTreeMap<n::LocalId, usize>,
        stores: BTreeSet<usize>,
    },
}
impl std::fmt::Display for Task {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Enter(slot) => write!(f, "(enter-input {slot})"),
            Self::Check(slot) => write!(f, "(check-input {slot})"),
            Self::Default {
                slot,
                value,
                references,
                deferred,
                stores,
            } => {
                write!(f, "(complete-default {slot} (references")?;
                for (binding, dependency) in references {
                    write!(f, " (input {dependency} {binding})")?;
                }
                f.write_str(") (deferred")?;
                for (binding, dependency) in deferred {
                    write!(f, " (input {dependency} {binding})")?;
                }
                f.write_str(") (stores")?;
                for id in stores {
                    write!(f, " {id}")?;
                }
                write!(f, ") (producer {value}))")
            }
        }
    }
}

struct Progress {
    tasks: Vec<Task>,
    path: Vec<usize>,
    active: BTreeSet<usize>,
}
impl Progress {
    #[cfg(test)]
    fn new(slot: usize) -> Self {
        Self {
            tasks: vec![Task::Enter(slot)],
            path: vec![],
            active: BTreeSet::new(),
        }
    }
    fn next(&mut self, fuel: &mut u64, span: Span) -> Result<Option<Task>, Error> {
        if self.tasks.is_empty() {
            return Ok(None);
        }
        spend(fuel, 1, span)?;
        Ok(self.tasks.pop())
    }
    fn enter(&mut self, slot: usize, span: Span) -> Result<(), Error> {
        if !self.active.insert(slot) {
            return Err(error(
                span,
                format!("cyclic dependent input preparation at input {slot}"),
            ));
        }
        self.path.push(slot);
        Ok(())
    }
    fn after(
        &mut self,
        dependencies: impl DoubleEndedIterator<Item = usize> + ExactSizeIterator,
        continuation: Task,
        fuel: &mut u64,
        span: Span,
    ) -> Result<(), Error> {
        spend(fuel, dependencies.len().saturating_add(1), span)?;
        self.tasks.push(continuation);
        self.tasks.extend(dependencies.rev().map(Task::Enter));
        Ok(())
    }
    fn schedule_default(
        &mut self,
        writes: &Writes,
        slot: usize,
        value: n::Expr,
        references: BTreeMap<n::LocalId, usize>,
        deferred: BTreeMap<n::LocalId, usize>,
        fuel: &mut u64,
        span: Span,
    ) -> Result<(), Error> {
        let reads = references
            .iter()
            .map(|(&id, &arg)| writes.read(id, arg, slot));
        let (dependencies, stores) = writes.dependencies(reads, fuel, span)?;
        self.after(
            dependencies.into_iter(),
            Task::Default {
                slot,
                value,
                references,
                deferred,
                stores,
            },
            fuel,
            span,
        )
    }
    fn complete(&mut self, slot: usize, span: Span) -> Result<(), Error> {
        if self.path.last() != Some(&slot) {
            return Err(error(
                span,
                "dependent input completed outside its active path",
            ));
        }
        self.path.pop();
        self.active.remove(&slot);
        Ok(())
    }
}
impl std::fmt::Display for Progress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(input-dependencies (path")?;
        for slot in &self.path {
            write!(f, " {slot}")?;
        }
        f.write_str(") (pending")?;
        for task in self.tasks.iter().rev() {
            write!(f, " {task}")?;
        }
        f.write_str("))")
    }
}
pub(in crate::program) fn spend(fuel: &mut u64, steps: usize, span: Span) -> Result<(), Error> {
    *fuel = fuel
        .checked_sub(steps as u64)
        .ok_or_else(|| error(span, "dependent input preparation budget exhausted"))?;
    Ok(())
}

pub(super) fn copy_evidence(value: &Term, steps: &mut u64, span: Span) -> Result<Term, Error> {
    let before = usize::try_from(*steps).unwrap_or(usize::MAX);
    let mut fuel = before;
    let result = nir::copy_budget::clone_term(value, &mut fuel);
    *steps -= (before - fuel) as u64;
    result.map_err(|failure| {
        error(
            span,
            format!("dependent input evidence copy budget exhausted: {failure:?}"),
        )
    })
}

struct Capture<'a> {
    domain: &'a Term,
    symbol: &'a Term,
    detached: bool,
    written: bool,
}

/// The parent owns checked inputs and default continuations across child checks.
/// Source borrows still belong to the enclosing checking region, not a VM task.
#[must_use = "drive the input frame to completion or discard its uncommitted results"]
pub(in crate::program) struct Frame {
    site: hir::HirId,
    span: Span,
    prepared: InputProducers,
    call: Option<nir::FunctionCallPreparation<Term>>,
    by_slot: BTreeMap<usize, usize>,
    default_slots: Option<BTreeMap<n::LocalId, usize>>,
    default_writes: Writes,
    progress: Progress,
    phase: Phase,
}

enum Phase {
    Ready,
    Child {
        slot: usize,
        caller_staged: bool,
    },
    Invoking {
        slot: usize,
        outputs: Option<DefaultOutputs>,
    },
    AwaitOperand(usize),
    Failed,
    Complete,
}

enum Resolution {
    Known(n::Expr, Term),
    Original(n::Expr, bool),
    Required(required::Invocation, Option<DefaultOutputs>),
}

/// Work leaves with the region that owns the pending slot. Only a required
/// invocation may replace its producer with an executed value.
pub(in crate::program) enum InputWork {
    Required(required::Invocation),
    Original {
        slot: usize,
        site: hir::HirId,
        call: Span,
        producer: n::Expr,
        project_return: bool,
    },
}

pub(in crate::program) enum InputReply {
    Required(required::CompletedInvocation),
    Original {
        slot: usize,
        producer: n::Expr,
        identity: Term,
    },
}

impl std::fmt::Display for InputWork {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Required(work) => work.fmt(f),
            Self::Original {
                slot,
                site,
                call,
                producer,
                ..
            } => write!(
                f,
                "(original-operand (call {site} {}) (slot {slot}) (producer {producer}))",
                source(*call)
            ),
        }
    }
}

impl InputWork {
    pub(in crate::program) fn resolve(
        self,
        checker: &mut Checker<'_, '_>,
    ) -> Result<InputReply, Error> {
        match self {
            Self::Required(work) => work.execute(checker).map(InputReply::Required),
            Self::Original {
                slot,
                call,
                producer,
                project_return,
                site,
            } => {
                if project_return
                    && let Some(identity) = checker.projected_type_return(&producer, call)?
                {
                    return Ok(InputReply::Original {
                        slot,
                        producer,
                        identity,
                    });
                }
                if project_return
                    && producer.ty == p(P::Type)
                    && let Some(identity) =
                        checker.bind_scoped_operand(site, slot, &producer, call)?
                {
                    return Ok(InputReply::Original {
                        slot,
                        producer,
                        identity,
                    });
                }
                Err(context_error(
                    call,
                    format!(
                        "dependent call input needs its original comptime invocation: slot {slot}, producer {}; the source region has no executing prefix",
                        producer.source
                    ),
                ))
            }
        }
    }
}

pub(in crate::program) enum Poll<'b> {
    Continue,
    Child(Request<'b>),
    Invoke(InputWork),
    Complete(PreparedInputs),
}

pub(in crate::program) struct Request<'b> {
    slot: usize,
    pub(in crate::program) input: arguments::RoutedSource<'b>,
    pub(in crate::program) expected: Term,
}

impl std::fmt::Display for Request<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let input = self.input.view();
        write!(
            f,
            "(dependent-input-request (slot {}) (source {} {}) (expected {}))",
            self.slot,
            input.hir_id,
            source(input.span),
            self.expected
        )
    }
}

impl Frame {
    pub(in crate::program) fn new(
        checker: &mut Checker<'_, '_>,
        signature: Term,
        site: &CallSite<'_>,
        assigned: &[bool],
        offset: usize,
        mut prepared: InputProducers,
    ) -> Result<Self, Error> {
        let source = &site.source;
        let routes = &site.routes;
        let view = CallSignature::of(&signature).expect("selected callable signature");
        let demanded = checker.call_input_demands(
            view,
            site,
            assigned,
            offset,
            source,
            &mut prepared.defaults,
        )?;
        spend(&mut checker.required_steps, demanded.len(), source.span)?;
        let by_slot = if demanded.is_empty() {
            BTreeMap::new()
        } else {
            spend(&mut checker.required_steps, routes.len(), source.span)?;
            routes
                .iter()
                .enumerate()
                .map(|(index, input)| (offset + input.parameter, index))
                .collect()
        };
        let default_writes = if demanded.is_empty() {
            Writes::default()
        } else {
            Writes::collect(
                checker,
                site.owner,
                view.parameters,
                &prepared.defaults,
                source.span,
            )?
        };
        let call = nir::FunctionCallPreparation::new(signature, checker.call_input_frames())
            .map_err(|failure| error(source.span, failure.to_string()))?;
        Ok(Self {
            call: Some(call),
            by_slot,
            default_slots: None,
            default_writes,
            site: source.hir_id,
            span: source.span,
            prepared,
            progress: Progress {
                tasks: demanded.into_iter().rev().map(Task::Enter).collect(),
                path: vec![],
                active: BTreeSet::new(),
            },
            phase: Phase::Ready,
        })
    }

    pub(in crate::program) fn poll<'h>(
        &mut self,
        checker: &mut Checker<'_, '_>,
        site: &CallSite<'h>,
    ) -> Result<Poll<'h>, Error> {
        match self.phase {
            Phase::Ready => {}
            Phase::Child { .. } => {
                return Err(error(self.span, "dependent input frame awaits its child"));
            }
            Phase::AwaitOperand(_) | Phase::Invoking { .. } | Phase::Failed | Phase::Complete => {
                return Err(error(
                    self.span,
                    "dependent input frame is no longer active",
                ));
            }
        }
        let result = self.step(checker, site);
        if result.is_err() {
            self.phase = Phase::Failed;
        }
        result
    }

    fn step<'h>(
        &mut self,
        checker: &mut Checker<'_, '_>,
        site: &CallSite<'h>,
    ) -> Result<Poll<'h>, Error> {
        let callee = &site.callee;
        let owner = site.owner;
        let routes = &site.routes;
        let source = &site.source;
        let prepared = &mut self.prepared;
        let progress = &mut self.progress;
        let Some(task) = progress.next(&mut checker.required_steps, source.span)? else {
            self.phase = Phase::Complete;
            let (signature, binding) = self
                .call
                .take()
                .expect("active input frame")
                .finish_with_signature()
                .map_err(|e| error(source.span, e.to_string()))?;
            return Ok(Poll::Complete(PreparedInputs {
                producers: std::mem::take(prepared),
                signature,
                binding,
            }));
        };
        let call = self.call.as_mut().expect("active input frame");
        match task {
            Task::Enter(slot) => {
                if call
                    .binding()
                    .observations()
                    .get(slot)
                    .is_some_and(Option::is_some)
                {
                    return Ok(Poll::Continue);
                }
                let parameter = call
                    .parameters()
                    .get(slot)
                    .ok_or_else(|| error(source.span, "invalid dependent input slot"))?;
                if parameter.sort == Sort::Runtime {
                    return Err(error(
                        source.span,
                        "dependent comptime input cannot read a runtime argument",
                    ));
                }
                progress.enter(slot, source.span)?;
                let mut dependencies =
                    checker.term_substitution_inputs(&parameter.ty, source.span)?;
                if let Some(default) = &prepared.defaults[slot] {
                    // Specialize the producer's annotations even if its value
                    // stops depending on an entry argument after a store.
                    dependencies.extend(checker.checked_substitution_inputs(default, source.span)?);
                }
                progress.after(
                    dependencies.into_keys(),
                    Task::Check(slot),
                    &mut checker.required_steps,
                    source.span,
                )?;
            }
            Task::Check(slot) => {
                let expected = checker
                    .with_call_type_fuel(|checker, fuel| {
                        call.prepare_metered(slot, fuel, |ty, fuel| {
                            checker.callable_input_view_with_fuel(ty, fuel, source.span)
                        })
                    })
                    .map_err(|failure| match failure {
                        nir::CallError::Contract { error, .. } => error,
                        other => error(source.span, other.to_string()),
                    })?;
                let route = self.by_slot.get(&slot).map(|&index| &routes[index]);
                // Completion returns the producer to its declaration slot.
                // A failed observation terminates this preparation.
                let value = if let Some(value) = prepared.values[slot].take() {
                    value
                } else if let Some(input) = route {
                    input.require_collection_type(checker, &expected, source.span)?;
                    self.phase = Phase::Child {
                        slot,
                        caller_staged: std::mem::replace(&mut checker.in_staged_input, true),
                    };
                    return Ok(Poll::Child(Request {
                        slot,
                        input: input.requested_source(source),
                        expected,
                    }));
                } else {
                    let owner = owner.ok_or_else(|| {
                        error(
                            source.span,
                            "dependent default needs declaration parameter metadata",
                        )
                    })?;
                    let mut value = match prepared.defaults[slot].take() {
                        Some(value) => value,
                        None => checker.call_default(site, owner, slot, source.span)?,
                    };
                    checker.specialize_default_annotations(
                        &mut value,
                        call.binding(),
                        source.span,
                    )?;
                    let ids = match &mut self.default_slots {
                        Some(slots) => slots,
                        vacant @ None => {
                            let item = checker.package.item(owner).unwrap();
                            let (ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)) = &item.kind
                            else {
                                return Err(error(
                                    source.span,
                                    "dependent default needs function parameter metadata",
                                ));
                            };
                            spend(&mut checker.required_steps, sig.params.len(), source.span)?;
                            vacant.insert(
                                sig.params
                                    .iter()
                                    .enumerate()
                                    .map(|(slot, parameter)| (local(parameter.hir_id), slot))
                                    .collect(),
                            )
                        }
                    };
                    let before = usize::try_from(checker.required_steps).unwrap_or(usize::MAX);
                    let mut fuel = before;
                    let references = nir::free_bindings::collect(&value, &mut fuel);
                    checker.required_steps -= (before - fuel) as u64;
                    let references = references.map_err(|e| error(source.span, e.to_string()))?;
                    spend(
                        &mut checker.required_steps,
                        references.len().saturating_mul(2),
                        source.span,
                    )?;
                    let references: BTreeMap<_, _> = references
                        .into_iter()
                        .filter_map(|binding| ids.get(&binding).map(|slot| (binding, *slot)))
                        .collect();
                    if references
                        .values()
                        .any(|&i| call.parameters()[i].sort == Sort::Runtime)
                    {
                        return Err(error(
                            source.span,
                            "dependent comptime input cannot read a runtime argument",
                        ));
                    }
                    let (references, deferred) = if !references.is_empty()
                        && matches!(value.kind, K::Block { .. } | K::CallPreparation { .. })
                    {
                        let live = checker.with_observation_budget(source.span, |_, budget| {
                            input_observation::normal_exit_inputs(&value, budget)
                        })?;
                        references
                            .into_iter()
                            .partition(|(id, _)| live.contains(id))
                    } else {
                        (references, BTreeMap::new())
                    };
                    progress.schedule_default(
                        &self.default_writes,
                        slot,
                        value,
                        references,
                        deferred,
                        &mut checker.required_steps,
                        source.span,
                    )?;
                    return Ok(Poll::Continue);
                };
                let resolution = checker.resolve_type_dependency(
                    owner,
                    callee,
                    source,
                    route.is_some(),
                    value,
                    BTreeMap::new(),
                )?;
                return self.resolved(slot, resolution);
            }
            Task::Default {
                slot,
                value,
                mut references,
                deferred,
                stores,
            } => {
                let symbols = call.binding().observations();
                self.default_writes
                    .observe(checker, stores, call.binding(), source.span)?;
                let receiver = checker.has_comptime_value_receiver(owner);
                let mut captures = BTreeMap::new();
                for (&id, &arg) in &references {
                    let read = self.default_writes.read(id, arg, slot);
                    let symbol = self.default_writes.value(read, symbols, source.span)?;
                    let domain = prepared.values[arg]
                        .as_ref()
                        .map_or(&call.parameters()[arg].ty, |v| &v.ty);
                    captures.insert(
                        id,
                        Capture {
                            domain,
                            symbol,
                            detached: !(receiver && arg == 0)
                                && matches!(domain, Term::Primitive(p) if *p != P::Object),
                            written: !matches!(read, Read::Argument(i) if i == arg),
                        },
                    );
                }
                if !deferred.is_empty() {
                    let mut facts = BTreeMap::new();
                    let mut snapshots = BTreeSet::new();
                    for (&id, capture) in &captures {
                        facts.insert(
                            id,
                            std::sync::Arc::new(copy_evidence(
                                capture.symbol,
                                &mut checker.required_steps,
                                source.span,
                            )?),
                        );
                        if capture.detached {
                            snapshots.insert(id);
                        }
                    }
                    for (&id, &arg) in &deferred {
                        if !(receiver && arg == 0)
                            && matches!(call.parameters()[arg].ty, Term::Primitive(p) if p != P::Object)
                        {
                            snapshots.insert(id);
                        }
                    }
                    let identity =
                        checker.with_observation_budget(source.span, |checker, budget| {
                            normal::observation::hint(
                                checker.observe_input_tail_in(&value, facts, snapshots, budget),
                            )
                        })?;
                    if let Some(identity) = identity {
                        self.deliver(slot, value, identity)?;
                    } else {
                        // The slice is only a hint. An opaque producer may
                        // still need every capture in its executable frame.
                        references.extend(deferred);
                        progress.schedule_default(
                            &self.default_writes,
                            slot,
                            value,
                            references,
                            BTreeMap::new(),
                            &mut checker.required_steps,
                            source.span,
                        )?;
                    }
                    return Ok(Poll::Continue);
                }
                let resolution = checker
                    .resolve_type_dependency(owner, callee, source, false, value, captures)?;
                return self.resolved(slot, resolution);
            }
        }

        Ok(Poll::Continue)
    }

    pub(in crate::program) fn unwind(&mut self, checker: &mut Checker<'_, '_>) {
        if let Phase::Child { caller_staged, .. } = self.phase {
            checker.in_staged_input = caller_staged;
            self.phase = Phase::Failed;
        }
    }

    pub(in crate::program) fn accept<'h>(
        &mut self,
        checker: &mut Checker<'_, '_>,
        value: n::Expr,
        site: &CallSite<'h>,
    ) -> Result<Poll<'h>, Error> {
        let (slot, caller_staged) = self.take_child()?;
        checker.in_staged_input = caller_staged;
        let resolution = checker.resolve_type_dependency(
            site.owner,
            &site.callee,
            &site.source,
            true,
            value,
            BTreeMap::new(),
        )?;
        self.resolved(slot, resolution)
    }

    fn resolved<'h>(&mut self, slot: usize, resolution: Resolution) -> Result<Poll<'h>, Error> {
        match resolution {
            Resolution::Known(value, identity) => {
                self.deliver(slot, value, identity)?;
                Ok(Poll::Continue)
            }
            Resolution::Original(producer, project_return) => {
                self.phase = Phase::AwaitOperand(slot);
                Ok(Poll::Invoke(InputWork::Original {
                    slot,
                    site: self.site,
                    call: self.span,
                    producer,
                    project_return,
                }))
            }
            Resolution::Required(invocation, outputs) => {
                self.phase = Phase::Invoking { slot, outputs };
                Ok(Poll::Invoke(InputWork::Required(invocation)))
            }
        }
    }

    fn deliver(&mut self, slot: usize, value: n::Expr, identity: Term) -> Result<(), Error> {
        let span = self.span;
        self.call
            .as_mut()
            .expect("active input frame")
            .supply(slot, identity)
            .map_err(|failure| error(span, failure.to_string()))?;
        self.progress.complete(slot, span)?;
        self.prepared.values[slot] = Some(value);
        self.phase = Phase::Ready;
        Ok(())
    }

    pub(in crate::program) fn accept_reply(
        &mut self,
        checker: &mut Checker<'_, '_>,
        reply: InputReply,
    ) -> Result<(), Error> {
        match reply {
            InputReply::Required(reply) => self.accept_required(checker, reply),
            InputReply::Original {
                slot,
                producer,
                identity,
            } => {
                if !matches!(self.phase, Phase::AwaitOperand(expected) if expected == slot) {
                    return Err(error(
                        self.span,
                        "original operand has no matching pending slot",
                    ));
                }
                self.deliver(slot, producer, identity)
            }
        }
    }

    fn accept_required(
        &mut self,
        checker: &mut Checker<'_, '_>,
        reply: required::CompletedInvocation,
    ) -> Result<(), Error> {
        let Phase::Invoking { slot, outputs } = std::mem::replace(&mut self.phase, Phase::Failed)
        else {
            return Err(error(
                self.span,
                "dependent input has no pending invocation",
            ));
        };
        let (value, ty) = reply.publish(checker)?;
        let (value, identity) = if let Some(outputs) = outputs {
            // The required producer already ran. Replay its completed stores
            // into the default frame without evaluating user code a second time.
            let mut writes = Vec::new();
            let (result, identity) =
                outputs.finish(checker, value, self.span, |id, value, _| {
                    writes.push((id, value));
                })?;
            (DefaultOutputs::replay(result, writes), identity)
        } else {
            checker.freeze_completed_dependency(value, ty, self.span)?
        };
        self.deliver(slot, value, identity)
    }

    /// The returned expression belongs to the active region child. Its slot
    /// and the phase to restore are both held by this continuation.
    fn take_child(&mut self) -> Result<(usize, bool), Error> {
        let Phase::Child {
            slot,
            caller_staged,
        } = self.phase
        else {
            return Err(error(self.span, "dependent input has no requested child"));
        };
        self.phase = Phase::Failed;
        Ok((slot, caller_staged))
    }
}

impl std::fmt::Display for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(dependent-input-frame (source {}) (phase ",
            source(self.span)
        )?;
        match &self.phase {
            Phase::Ready => f.write_str("ready")?,
            Phase::Child {
                slot,
                caller_staged,
            } => write!(f, "(child {slot} (caller-staged {caller_staged}))")?,
            Phase::Invoking { slot, outputs } => {
                write!(f, "(invoking {slot}")?;
                if let Some(outputs) = outputs {
                    write!(f, " {outputs}")?;
                }
                f.write_str(")")?;
            }
            Phase::AwaitOperand(slot) => write!(f, "(await-original-operand {slot})")?,
            Phase::Failed => f.write_str("failed")?,
            Phase::Complete => f.write_str("complete")?,
        }
        write!(
            f,
            ") {} {} {}",
            self.progress, self.prepared, self.default_writes
        )?;
        if let Some(call) = &self.call {
            write!(f, " {call}")?;
        }
        f.write_str(")")
    }
}

impl std::fmt::Display for InputProducers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(inputs")?;
        for (slot, value) in self.values.iter().enumerate() {
            write!(f, " (slot {slot} (producer")?;
            if let Some(value) = value {
                write!(f, " {value}")?;
            }
            f.write_str(") (default")?;
            if let Some(value) = &self.defaults[slot] {
                write!(f, " {value}")?;
            }
            f.write_str("))")?;
        }
        f.write_str(")")
    }
}

impl Checker<'_, '_> {
    fn resolve_type_dependency(
        &mut self,
        owner: Option<OwnerId>,
        callee: &n::Expr,
        source: &Expr<'_>,
        has_source: bool,
        value: n::Expr,
        captures: BTreeMap<n::LocalId, Capture<'_>>,
    ) -> Result<Resolution, Error> {
        spend(&mut self.required_steps, captures.len(), source.span)?;
        let snapshots = captures
            .iter()
            .filter_map(|(&id, capture)| capture.detached.then_some(id))
            .collect();
        let identity = match &value.kind {
            K::TypeValue(ty) => Some(copy_evidence(ty, &mut self.required_steps, source.span)?),
            _ => self.preserved_input_with_snapshots(
                &value,
                |id, fuel| {
                    let Some(capture) = captures.get(&id) else {
                        return Ok(None);
                    };
                    Ok(Some(std::sync::Arc::new(copy_evidence(
                        capture.symbol,
                        fuel,
                        source.span,
                    )?)))
                },
                snapshots,
                source.span,
            )?,
        };
        if let Some(identity) = identity {
            // Observation leaves the producer and all its effects at its source position.
            return Ok(Resolution::Known(value, identity));
        }
        if let K::Local(binding) = &value.kind {
            if has_source {
                if let Some(binder) = self.input_facts.named_types.get(*binding) {
                    return Ok(Resolution::Known(
                        value,
                        Term::Rigid {
                            declaration: binder.declaration,
                            sort: binder.sort,
                        },
                    ));
                }
                let fact = self
                    .input_facts
                    .type_identity(binding)
                    .or_else(|| self.input_facts.value_identity(binding).map(|(ty, _)| ty));
                if let Some(fact) = fact {
                    let identity = copy_evidence(fact, &mut self.required_steps, source.span)?;
                    return Ok(Resolution::Known(value, identity));
                }
            }
            // The enclosing invocation supplies the actual captured Type.
            if let Some(declaration) = self
                .alias_value_bindings
                .iter()
                .find_map(|(parameter, id)| (id == binding).then_some(*parameter))
            {
                return Ok(Resolution::Known(
                    value,
                    Term::Rigid {
                        declaration,
                        sort: Sort::Type,
                    },
                ));
            }
            if let Some(identity) = self.spread_input_symbol(*binding, &value.ty, source)? {
                return Ok(Resolution::Known(value, identity));
            }
        } else if let Some(owner) = owner
            && self
                .type_parameters
                .get(&owner)
                .is_some_and(|ids| !ids.is_empty())
        {
            let inputs = self.checked_declaration_inputs(&value, source.span)?;
            if self.type_parameters[&owner]
                .iter()
                .any(|id| inputs.contains(id))
            {
                return Err(context_error(
                    source.span,
                    "dependent Type computation awaits inferred type arguments",
                ));
            }
        }
        // This frame belongs to the suspended source definition. Keep the
        // producer for phase and effect checking, with a fresh identity used
        // only for typing the remaining slots and result of this call.
        let lexical_owner = source.hir_id.owner;
        if has_source
            && captures.is_empty()
            && self.describing_input_of(lexical_owner)
            && let Some(identity) =
                self.deferred_definition_input(lexical_owner, &value, source.span)?
        {
            return Ok(Resolution::Known(value, identity));
        }
        // At a concrete comptime call without entry constraints this fact can
        // specialize the remaining domains and result without proving an entry
        // obligation or changing the execution order of the explicit operand.
        let project_return = has_source
            && captures.is_empty()
            && callee.ty.is_comptime_callable()
            && owner.is_none_or(|id| {
                self.type_parameters.get(&id).is_none_or(Vec::is_empty)
                    && self.requirements.get(&id).is_none_or(Vec::is_empty)
                    && self.capabilities.get(&id).is_none_or(Vec::is_empty)
            });
        if project_return {
            return Ok(Resolution::Original(value, true));
        }
        let callable = self.structural_type(&callee.ty, source.span)?;
        let callable = self.callable_input_view(&callable, source.span)?;
        if callable.is_comptime_callable() || captures.values().any(|c| c.written) {
            return Ok(Resolution::Original(value, false));
        }
        let mut bindings = Vec::new();
        for (&binding, capture) in &captures {
            bindings.push((
                binding,
                copy_evidence(capture.domain, &mut self.required_steps, source.span)?,
                copy_evidence(capture.symbol, &mut self.required_steps, source.span)?,
            ));
        }
        let (value, outputs) = if captures.is_empty() {
            (value, None)
        } else {
            DefaultOutputs::prepare(
                self,
                value,
                |id| captures.get(&id).map(|c| c.domain),
                source.span,
            )?
        };
        let prepared =
            self.prepare_required_expression(source, None, Some(value), bindings, false)?;
        Ok(Resolution::Required(
            self.required_invocation(prepared, Vec::new(), source.hir_id.owner, source.span),
            outputs,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_frame_keeps_pending_results_private_and_checks_child_ownership() {
        let span = Span::default();
        let source_expr = Expr {
            hir_id: hir::HirId::new(
                hir::OwnerId::new(hir::LocalDefId::new(1)),
                hir::ItemLocalId::new(2),
            ),
            span,
            kind: H::Unit,
        };
        let value = n::Expr {
            source: source(span),
            ty: p(P::Str),
            kind: K::Literal(n::Literal::String("retained\"\n".into())),
        };
        let mut prepared = InputProducers::new(2);
        prepared.values[0] = Some(value.clone());
        prepared.defaults[1] = Some(value.clone());
        let signature = Term::Function {
            parameters: vec![
                Parameter {
                    sort: Sort::ComptimeValue,
                    ty: p(P::Str)
                };
                2
            ],
            result: Box::new(p(P::Void)),
        };
        let mut call = nir::FunctionCallPreparation::new(signature, vec![]).unwrap();
        call.prepare(0, |domain| Ok::<_, nir::TypeError>(domain.clone()))
            .unwrap();
        call.supply(0, Term::String("retained\"\n".into())).unwrap();
        call.prepare(1, |domain| Ok::<_, nir::TypeError>(domain.clone()))
            .unwrap();
        let mut frame = Frame {
            site: source_expr.hir_id,
            call: Some(call),
            span,
            by_slot: BTreeMap::new(),
            default_slots: None,
            default_writes: Writes::default(),
            prepared,
            progress: Progress::new(1),
            phase: Phase::Child {
                slot: 1,
                caller_staged: false,
            },
        };
        let dump = frame.to_string();
        assert!(dump.starts_with("(dependent-input-frame "), "{dump}");
        for text in [
            "(phase (child 1 (caller-staged false)))",
            "(default-writes)",
            "(slot 0 (producer",
            "(slot 1 (producer)",
            "retained\\\"\\n",
        ] {
            assert!(dump.contains(text), "missing {text}: {dump}");
        }
        assert!(!dump.contains('\n'), "{dump}");
        assert_eq!(frame.take_child().unwrap(), (1, false));
        let consumed = frame.to_string();
        assert!(
            frame.take_child().is_err(),
            "a child result was accepted twice"
        );
        assert_eq!(frame.to_string(), consumed);
        assert!(matches!(frame.phase, Phase::Failed));
        let request = Request {
            slot: 1,
            input: arguments::RoutedSource::Expression(std::borrow::Cow::Borrowed(&source_expr)),
            expected: p(P::Str),
        }
        .to_string();
        assert!(request.contains("(slot 1)"), "{request}");
        assert!(request.contains("(expected (primitive str))"), "{request}");
        assert!(!request.contains("retained"), "{request}");
    }

    #[test]
    fn dependency_cycles_are_distinct_from_budget_exhaustion() {
        let span = Span::default();
        let mut progress = Progress::new(0);
        let mut fuel = 100;
        assert!(matches!(
            progress.next(&mut fuel, span).unwrap(),
            Some(Task::Enter(0))
        ));
        progress.enter(0, span).unwrap();
        progress
            .after([1].into_iter(), Task::Check(0), &mut fuel, span)
            .unwrap();
        assert!(matches!(
            progress.next(&mut fuel, span).unwrap(),
            Some(Task::Enter(1))
        ));
        progress.enter(1, span).unwrap();
        progress
            .after([0].into_iter(), Task::Check(1), &mut fuel, span)
            .unwrap();
        let before = progress.to_string();
        let exhausted = progress.next(&mut 0, span).err().unwrap();
        assert!(exhausted.message.contains("budget exhausted"));
        assert_eq!(
            progress.to_string(),
            before,
            "failed polling cannot consume a task"
        );
        assert!(matches!(
            progress.next(&mut fuel, span).unwrap(),
            Some(Task::Enter(0))
        ));
        let cycle = progress.enter(0, span).err().unwrap();
        assert!(
            cycle
                .message
                .contains("cyclic dependent input preparation at input 0")
        );
        assert!(!cycle.message.contains("budget"));
        assert_eq!(progress.path, [0, 1]);
        assert!(progress.complete(0, span).is_err());
        progress.complete(1, span).unwrap();
        progress.complete(0, span).unwrap();
        progress.enter(0, span).unwrap();
    }

    #[test]
    fn retained_defaults_keep_their_bindings_and_escaped_producer() {
        let span = Span::default();
        let value = n::Expr {
            source: source(span),
            ty: p(P::Str),
            kind: K::Literal(n::Literal::String("default\"\n".into())),
        };
        let task = Task::Default {
            slot: 3,
            value,
            references: BTreeMap::from([(n::LocalId { owner: 4, index: 2 }, 1)]),
            deferred: BTreeMap::from([(n::LocalId { owner: 4, index: 0 }, 0)]),
            stores: BTreeSet::new(),
        };
        let mut progress = Progress::new(3);
        progress.tasks.clear();
        progress
            .after([1].into_iter(), task, &mut 100, span)
            .unwrap();
        let dump = progress.to_string();
        assert!(
            dump.contains("(enter-input 1) (complete-default 3"),
            "{dump}"
        );
        assert!(
            dump.contains("(references (input 1 (local 4 2)))"),
            "{dump}"
        );
        assert!(dump.contains("(deferred (input 0 (local 4 0)))"), "{dump}");
        assert!(dump.contains("default\\\"\\n"), "{dump}");
        let term = Term::String("x".repeat(1024).into());
        assert!(copy_evidence(&term, &mut 1, span).is_err());
        assert_eq!(copy_evidence(&term, &mut 10_000, span).unwrap(), term);
    }
}
