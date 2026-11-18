//! Dependency checking retains its continuation and default expression explicitly.
//! The work stack is a checking schedule, never an execution schedule or VM frame.
use super::*;

enum Task {
    Enter(usize),
    Check(usize),
    Default {
        slot: usize,
        value: n::Expr,
        references: BTreeMap<n::LocalId, usize>,
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
            } => {
                write!(f, "(complete-default {slot} (references")?;
                for (binding, dependency) in references {
                    write!(f, " (input {dependency} {binding})")?;
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
fn spend(fuel: &mut u64, steps: usize, span: Span) -> Result<(), Error> {
    *fuel = fuel
        .checked_sub(steps as u64)
        .ok_or_else(|| error(span, "dependent input preparation budget exhausted"))?;
    Ok(())
}

fn copy_evidence(value: &Term, steps: &mut u64, span: Span) -> Result<Term, Error> {
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

/// Borrow ordinary HIR directly. A collected tuple or tactic argument owns
/// only its routing wrapper; its children still borrow the call's source.
/// Key by declaration slot so a chain of demands does not rescan every route.
struct InputSource<'a> {
    route: &'a arguments::RoutedArgument<'a>,
    expression: std::borrow::Cow<'a, Expr<'a>>,
}

pub(super) struct Sources<'a>(BTreeMap<usize, InputSource<'a>>);

impl<'a> Sources<'a> {
    pub(super) fn new(
        routes: &'a [arguments::RoutedArgument<'a>],
        offset: usize,
        call: &Expr<'_>,
        fuel: &mut u64,
    ) -> Result<Self, Error> {
        spend(fuel, routes.len(), call.span)?;
        Ok(Self(
            routes
                .iter()
                .map(|route| {
                    (
                        route.parameter + offset,
                        InputSource {
                            route,
                            expression: route.expression_view(call),
                        },
                    )
                })
                .collect(),
        ))
    }
}

/// The parent owns checked inputs and default continuations across child checks.
/// Source borrows still belong to the enclosing checking region, not a VM task.
#[must_use = "drive the input frame to completion or discard its uncommitted results"]
pub(in crate::program) struct Frame<'b, 'h> {
    parameters: &'b [Parameter],
    callee: &'b n::Expr,
    owner: Option<OwnerId>,
    sources: &'b Sources<'h>,
    source: &'b Expr<'h>,
    prepared: PreparedInputs,
    progress: Progress,
    phase: Phase,
}

#[derive(Clone, Copy)]
enum Phase {
    Ready,
    Child { slot: usize, caller_staged: bool },
    Failed,
    Complete,
}

pub(in crate::program) enum Poll<'b> {
    Continue,
    Child(Request<'b>),
    Complete(PreparedInputs),
}

pub(in crate::program) struct Request<'b> {
    slot: usize,
    pub(in crate::program) input: &'b Expr<'b>,
    pub(in crate::program) expected: Term,
}

impl std::fmt::Display for Request<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(dependent-input-request (slot {}) (source {} {}) (expected {}))",
            self.slot,
            self.input.hir_id,
            source(self.input.span),
            self.expected
        )
    }
}

impl<'b, 'h: 'b> Frame<'b, 'h> {
    pub(super) fn new(
        checker: &mut Checker<'_, '_>,
        demanded: BTreeSet<usize>,
        parameters: &'b [Parameter],
        callee: &'b n::Expr,
        owner: Option<OwnerId>,
        sources: &'b Sources<'h>,
        source: &'b Expr<'h>,
        prepared: PreparedInputs,
    ) -> Result<Self, Error> {
        spend(&mut checker.required_steps, demanded.len(), source.span)?;
        Ok(Self {
            parameters,
            callee,
            owner,
            sources,
            source,
            prepared,
            progress: Progress {
                tasks: demanded.into_iter().rev().map(Task::Enter).collect(),
                path: vec![],
                active: BTreeSet::new(),
            },
            phase: Phase::Ready,
        })
    }

    pub(in crate::program) fn poll(
        &mut self,
        checker: &mut Checker<'_, '_>,
    ) -> Result<Poll<'b>, Error> {
        match self.phase {
            Phase::Ready => {}
            Phase::Child { .. } => {
                return Err(error(
                    self.source.span,
                    "dependent input frame awaits its child",
                ));
            }
            Phase::Failed | Phase::Complete => {
                return Err(error(
                    self.source.span,
                    "dependent input frame is no longer active",
                ));
            }
        }
        let result = self.step(checker);
        if result.is_err() {
            self.phase = Phase::Failed;
        }
        result
    }

    fn step(&mut self, checker: &mut Checker<'_, '_>) -> Result<Poll<'b>, Error> {
        let parameters = self.parameters;
        let callee = self.callee;
        let owner = self.owner;
        let sources = self.sources;
        let source = self.source;
        let prepared = &mut self.prepared;
        let progress = &mut self.progress;
        let Some(task) = progress.next(&mut checker.required_steps, source.span)? else {
            self.phase = Phase::Complete;
            return Ok(Poll::Complete(std::mem::take(prepared)));
        };
        match task {
            Task::Enter(slot) => {
                if prepared.symbols.get(slot).is_some_and(Option::is_some) {
                    return Ok(Poll::Continue);
                }
                let parameter = parameters
                    .get(slot)
                    .ok_or_else(|| error(source.span, "invalid dependent input slot"))?;
                if parameter.sort == Sort::Runtime {
                    return Err(error(
                        source.span,
                        "dependent comptime input cannot read a runtime argument",
                    ));
                }
                progress.enter(slot, source.span)?;
                let dependencies = checker.term_substitution_inputs(&parameter.ty, source.span)?;
                progress.after(
                    dependencies.into_keys(),
                    Task::Check(slot),
                    &mut checker.required_steps,
                    source.span,
                )?;
            }
            Task::Check(slot) => {
                let binding =
                    checker.prepare_call_binding(parameters, &prepared.symbols, source.span)?;
                let expected = binding
                    .substitute(&parameters[slot].ty)
                    .map_err(|e| error(source.span, e.to_string()))?;
                let expected = checker.callable_input_view(&expected, source.span)?;
                let route = sources.0.get(&slot);
                // Completion returns the producer to its declaration slot.
                // A failed observation terminates this preparation.
                let value = if let Some(value) = prepared.values[slot].take() {
                    value
                } else if let Some(input) = route {
                    input
                        .route
                        .require_collection_type(checker, &expected, source.span)?;
                    self.phase = Phase::Child {
                        slot,
                        caller_staged: std::mem::replace(&mut checker.in_staged_input, true),
                    };
                    return Ok(Poll::Child(Request {
                        slot,
                        input: &input.expression,
                        expected,
                    }));
                } else {
                    let owner = owner.ok_or_else(|| {
                        error(
                            source.span,
                            "dependent default needs declaration parameter metadata",
                        )
                    })?;
                    let value = match prepared.defaults[slot].take() {
                        Some(value) => value,
                        None => checker.call_default(owner, slot, callee, source.span)?,
                    };
                    let item = checker.package.item(owner).unwrap();
                    let (ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)) = &item.kind else {
                        return Err(error(
                            source.span,
                            "dependent default needs function parameter metadata",
                        ));
                    };
                    spend(&mut checker.required_steps, sig.params.len(), source.span)?;
                    let ids: BTreeMap<_, _> = sig
                        .params
                        .iter()
                        .enumerate()
                        .map(|(slot, parameter)| (local(parameter.hir_id), slot))
                        .collect();
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
                    // Own the checked default while its referenced inputs are
                    // prepared. Resuming does not rebuild or invoke the default.
                    let dependencies: Vec<_> = references.values().copied().collect();
                    progress.after(
                        dependencies.into_iter(),
                        Task::Default {
                            slot,
                            value,
                            references,
                        },
                        &mut checker.required_steps,
                        source.span,
                    )?;
                    return Ok(Poll::Continue);
                };
                checker.complete_type_dependency(
                    slot,
                    owner,
                    callee,
                    source,
                    route.is_some(),
                    value,
                    BTreeMap::new(),
                    prepared,
                )?;
                progress.complete(slot, source.span)?;
            }
            Task::Default {
                slot,
                mut value,
                references,
            } => {
                let binding =
                    checker.prepare_call_binding(parameters, &prepared.symbols, source.span)?;
                value
                    .rewrite_types(&mut |ty| binding.substitute(ty))
                    .map_err(|e| error(source.span, e.to_string()))?;
                value.rewrite_types(&mut |term| {
                    checker.callable_argument_term_view(term, source.span)
                })?;
                checker.complete_type_dependency(
                    slot, owner, callee, source, false, value, references, prepared,
                )?;
                progress.complete(slot, source.span)?;
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

    pub(in crate::program) fn accept(
        &mut self,
        checker: &mut Checker<'_, '_>,
        value: n::Expr,
    ) -> Result<(), Error> {
        let (slot, caller_staged) = self.take_child()?;
        checker.in_staged_input = caller_staged;
        checker.complete_type_dependency(
            slot,
            self.owner,
            self.callee,
            self.source,
            true,
            value,
            BTreeMap::new(),
            &mut self.prepared,
        )?;
        self.progress.complete(slot, self.source.span)?;
        self.phase = Phase::Ready;
        Ok(())
    }

    /// The returned expression belongs to the active region child. Its slot
    /// and the phase to restore are both held by this continuation.
    fn take_child(&mut self) -> Result<(usize, bool), Error> {
        let Phase::Child {
            slot,
            caller_staged,
        } = self.phase
        else {
            return Err(error(
                self.source.span,
                "dependent input has no requested child",
            ));
        };
        self.phase = Phase::Failed;
        Ok((slot, caller_staged))
    }
}

impl std::fmt::Display for Frame<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(dependent-input-frame (source {}) (phase ",
            source(self.source.span)
        )?;
        match self.phase {
            Phase::Ready => f.write_str("ready")?,
            Phase::Child {
                slot,
                caller_staged,
            } => write!(f, "(child {slot} (caller-staged {caller_staged}))")?,
            Phase::Failed => f.write_str("failed")?,
            Phase::Complete => f.write_str("complete")?,
        }
        write!(f, ") {} {})", self.progress, self.prepared)
    }
}

impl std::fmt::Display for PreparedInputs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(inputs")?;
        for (slot, value) in self.values.iter().enumerate() {
            write!(f, " (slot {slot} (producer")?;
            if let Some(value) = value {
                write!(f, " {value}")?;
            }
            f.write_str(") (identity")?;
            if let Some(value) = &self.symbols[slot] {
                write!(f, " {value}")?;
            }
            write!(f, ") (live {}) (default", self.live[slot].is_some())?;
            if let Some(value) = &self.defaults[slot] {
                write!(f, " {value}")?;
            }
            f.write_str("))")?;
        }
        f.write_str(")")
    }
}

impl Checker<'_, '_> {
    fn complete_type_dependency(
        &mut self,
        slot: usize,
        owner: Option<OwnerId>,
        callee: &n::Expr,
        source: &Expr<'_>,
        has_source: bool,
        value: n::Expr,
        references: BTreeMap<n::LocalId, usize>,
        prepared: &mut PreparedInputs,
    ) -> Result<(), Error> {
        let observed = if matches!(value.kind, K::TypeValue(_)) {
            None
        } else {
            self.preserved_input_identity(
                &value,
                |id, fuel| {
                    let Some(slot) = references.get(&id) else {
                        return Ok(None);
                    };
                    let symbol = prepared.symbols[*slot].as_ref().ok_or_else(|| {
                        error(source.span, "dependent default identity is incomplete")
                    })?;
                    Ok(Some(std::sync::Arc::new(copy_evidence(
                        symbol,
                        fuel,
                        source.span,
                    )?)))
                },
                source.span,
            )?
        };
        if !matches!(value.kind, K::TypeValue(_) | K::Local(_)) {
            let unresolved = owner
                .and_then(|owner| self.type_parameters.get(&owner))
                .cloned()
                .unwrap_or_default();
            let inputs = self.checked_declaration_inputs(&value, source.span)?;
            if unresolved.iter().any(|id| inputs.contains(id)) {
                // Opaque heads and other structural Type values can retain
                // rigids without invoking a computation over unknown inputs.
                if observed.is_none() {
                    return Err(context_error(
                        source.span,
                        "dependent Type computation awaits inferred type arguments",
                    ));
                }
            }
        }
        let (value, symbol) = if let Some(identity) = observed {
            // Identity evidence does not complete the producer. Its writes,
            // calls and control transfers remain at the source input slot.
            (value, identity)
        } else {
            match &value.kind {
                K::TypeValue(ty) => {
                    let symbol = copy_evidence(ty, &mut self.required_steps, source.span)?;
                    (value, symbol)
                }
                K::Local(binding) if has_source && self.input_facts.types.contains_key(binding) => {
                    let symbol = copy_evidence(
                        &self.input_facts.types[binding],
                        &mut self.required_steps,
                        source.span,
                    )?;
                    (value, symbol)
                }
                K::Local(binding)
                    if has_source && self.input_facts.values.contains_key(binding) =>
                {
                    let symbol = copy_evidence(
                        &self.input_facts.values[binding].0,
                        &mut self.required_steps,
                        source.span,
                    )?;
                    (value, symbol)
                }
                K::Local(binding) => {
                    // A prepared container input is supplied by the enclosing
                    // invocation. Its declaration identity is enough to check
                    // the callee telescope and container premises now; retain
                    // the Local expression so execution receives the actual
                    // value rather than freezing this symbolic identity.
                    let captured = self
                        .alias_value_bindings
                        .iter()
                        .find_map(|(parameter, id)| (id == binding).then_some(*parameter));
                    if let Some(declaration) = captured {
                        (
                            value,
                            Term::Rigid {
                                declaration,
                                sort: Sort::Type,
                            },
                        )
                    } else {
                        match self.spread_input_symbol(*binding, &value.ty, source)? {
                            Some(symbol) => (value, symbol),
                            None => self.complete_unobserved_call_input(
                                callee,
                                source,
                                value,
                                &references,
                                prepared,
                            )?,
                        }
                    }
                }
                _ => self.complete_unobserved_call_input(
                    callee,
                    source,
                    value,
                    &references,
                    prepared,
                )?,
            }
        };
        prepared.values[slot] = Some(value);
        prepared.symbols[slot] = Some(symbol);
        Ok(())
    }

    fn complete_unobserved_call_input(
        &mut self,
        callee: &n::Expr,
        source: &Expr<'_>,
        value: n::Expr,
        references: &BTreeMap<n::LocalId, usize>,
        prepared: &PreparedInputs,
    ) -> Result<(n::Expr, Term), Error> {
        let callable = self.structural_type(&callee.ty, source.span)?;
        let callable = self.callable_input_view(&callable, source.span)?;
        if callable.is_comptime_callable() {
            // The dependency walk is not the invocation's execution order.
            // In particular, bindings above are identity evidence, not values
            // delivered by an executed prefix. A separate required invocation
            // would consume effects here and replace the original producer.
            // Until source checking can wait on that invocation, report the
            // missing continuation rather than silently changing its behavior.
            return Err(context_error(
                source.span,
                "dependent call input needs its original comptime invocation; source execution continuation is not yet implemented",
            ));
        }
        let mut bindings = Vec::new();
        for (&binding, &slot) in references {
            let producer = prepared.values[slot]
                .as_ref()
                .ok_or_else(|| error(source.span, "dependent default producer is incomplete"))?;
            let symbol = prepared.symbols[slot]
                .as_ref()
                .ok_or_else(|| error(source.span, "dependent default identity is incomplete"))?;
            bindings.push((
                binding,
                copy_evidence(&producer.ty, &mut self.required_steps, source.span)?,
                copy_evidence(symbol, &mut self.required_steps, source.span)?,
            ));
        }
        self.freeze_dependency_input(source, value, bindings)
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
        let mut prepared = PreparedInputs::new(2);
        prepared.values[0] = Some(value.clone());
        prepared.symbols[0] = Some(Term::String("retained\"\n".into()));
        prepared.defaults[1] = Some(value.clone());
        let mut frame = Frame {
            parameters: &[],
            callee: &value,
            owner: None,
            sources: &Sources(BTreeMap::new()),
            source: &source_expr,
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
            input: &source_expr,
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
        assert!(dump.contains("default\\\"\\n"), "{dump}");
        let term = Term::String("x".repeat(1024).into());
        assert!(copy_evidence(&term, &mut 1, span).is_err());
        assert_eq!(copy_evidence(&term, &mut 10_000, span).unwrap(), term);
    }
}
