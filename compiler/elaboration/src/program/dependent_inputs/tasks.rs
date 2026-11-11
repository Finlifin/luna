//! Dependency checking retains its continuation and default expression explicitly.
//! The work stack is a checking schedule, never an execution schedule or VM frame.
use super::*;

enum Task {
    Enter(usize),
    Check(usize),
    Default {
        slot: usize,
        value: n::Expr,
        references: Vec<(n::LocalId, usize)>,
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

impl Checker<'_, '_> {
    pub(in crate::program) fn prepare_type_dependency(
        &mut self,
        slot: usize,
        parameters: &[Parameter],
        callee: &n::Expr,
        owner: Option<OwnerId>,
        routes: &[arguments::RoutedArgument<'_>],
        offset: usize,
        source: &Expr<'_>,
        prepared: &mut PreparedInputs,
    ) -> Result<(), Error> {
        let mut progress = Progress::new(slot);
        while let Some(task) = progress.next(&mut self.required_steps, source.span)? {
            match task {
                Task::Enter(slot) => {
                    if prepared.symbols.get(slot).is_some_and(Option::is_some) {
                        continue;
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
                    let dependencies = self.term_substitution_inputs(&parameter.ty, source.span)?;
                    progress.after(
                        dependencies.into_keys(),
                        Task::Check(slot),
                        &mut self.required_steps,
                        source.span,
                    )?;
                }
                Task::Check(slot) => {
                    let binding =
                        self.prepare_call_binding(parameters, &prepared.symbols, source.span)?;
                    let expected = binding
                        .substitute(&parameters[slot].ty)
                        .map_err(|e| error(source.span, e.to_string()))?;
                    let expected = self.callable_input_view(&expected, source.span)?;
                    let route = routes.iter().find(|route| route.parameter + offset == slot);
                    let value = if let Some(value) = &prepared.values[slot] {
                        nir::copy_budget::charge_expression(value, &mut self.required_steps)
                            .map_err(|failure| error(source.span, format!("dependent input producer copy budget exhausted: {failure:?}")))?;
                        value.clone()
                    } else if let Some(route) = route {
                        route.require_collection_type(self, &expected, source.span)?;
                        let value = route.expression(source);
                        self.with_staged_input(true, |this| this.expr(&value, Some(&expected)))?
                    } else {
                        let owner = owner.ok_or_else(|| {
                            error(
                                source.span,
                                "dependent default needs declaration parameter metadata",
                            )
                        })?;
                        let value = match prepared.defaults[slot].take() {
                            Some(value) => value,
                            None => self.call_default(owner, slot, callee, source.span)?,
                        };
                        let item = self.package.item(owner).unwrap();
                        let (ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)) = &item.kind
                        else {
                            return Err(error(
                                source.span,
                                "dependent default needs function parameter metadata",
                            ));
                        };
                        spend(&mut self.required_steps, sig.params.len(), source.span)?;
                        let ids: BTreeMap<_, _> = sig
                            .params
                            .iter()
                            .enumerate()
                            .map(|(slot, parameter)| (local(parameter.hir_id), slot))
                            .collect();
                        let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                        let mut fuel = before;
                        let references = nir::preparation::references(&value, &mut fuel);
                        self.required_steps -= (before - fuel) as u64;
                        let references = references.map_err(|e| error(source.span, e))?;
                        spend(
                            &mut self.required_steps,
                            references.len().saturating_mul(2),
                            source.span,
                        )?;
                        let references: Vec<_> = references
                            .into_iter()
                            .filter_map(|binding| ids.get(&binding).map(|slot| (binding, *slot)))
                            .collect();
                        // Own the checked default while its referenced inputs are
                        // prepared. Resuming does not rebuild or invoke the default.
                        let dependencies: Vec<_> =
                            references.iter().map(|(_, slot)| *slot).collect();
                        progress.after(
                            dependencies.into_iter(),
                            Task::Default {
                                slot,
                                value,
                                references,
                            },
                            &mut self.required_steps,
                            source.span,
                        )?;
                        continue;
                    };
                    self.complete_type_dependency(
                        slot,
                        owner,
                        source,
                        route.is_some(),
                        value,
                        vec![],
                        None,
                        prepared,
                    )?;
                    progress.complete(slot, source.span)?;
                }
                Task::Default {
                    slot,
                    mut value,
                    references,
                } => {
                    let mut bindings = Vec::new();
                    for (binding, dependency) in &references {
                        let value = prepared.values[*dependency].as_ref().ok_or_else(|| {
                            error(source.span, "dependent default producer is incomplete")
                        })?;
                        let symbol = prepared.symbols[*dependency].as_ref().ok_or_else(|| {
                            error(source.span, "dependent default identity is incomplete")
                        })?;
                        bindings.push((
                            *binding,
                            copy_evidence(&value.ty, &mut self.required_steps, source.span)?,
                            copy_evidence(symbol, &mut self.required_steps, source.span)?,
                        ));
                    }
                    let binding =
                        self.prepare_call_binding(parameters, &prepared.symbols, source.span)?;
                    value
                        .rewrite_types(&mut |ty| binding.substitute(ty))
                        .map_err(|e| error(source.span, e.to_string()))?;
                    value.rewrite_types(&mut |ty| self.callable_input_view(ty, source.span))?;
                    let forwarded = if let K::Local(binding) = value.kind {
                        references
                            .iter()
                            .find(|(id, _)| *id == binding)
                            .and_then(|(_, dependency)| prepared.symbols[*dependency].as_ref())
                            .map(|symbol| {
                                copy_evidence(symbol, &mut self.required_steps, source.span)
                            })
                            .transpose()?
                    } else {
                        None
                    };
                    self.complete_type_dependency(
                        slot, owner, source, false, value, bindings, forwarded, prepared,
                    )?;
                    progress.complete(slot, source.span)?;
                }
            }
        }
        Ok(())
    }

    fn complete_type_dependency(
        &mut self,
        slot: usize,
        owner: Option<OwnerId>,
        source: &Expr<'_>,
        has_source: bool,
        value: n::Expr,
        bindings: Vec<(n::LocalId, Term, Term)>,
        forwarded_default: Option<Term>,
        prepared: &mut PreparedInputs,
    ) -> Result<(), Error> {
        let observed = if matches!(value.kind, K::TypeValue(_) | K::Local(_)) {
            None
        } else {
            self.preserved_input_identity(
                &value,
                bindings
                    .iter()
                    .map(|(binding, _, identity)| (*binding, identity)),
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
        let (value, symbol) = if let Some(identity) = forwarded_default {
            (value, identity)
        } else if let Some(identity) = observed {
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
                            None => self.freeze_dependency_input(source, value, bindings)?,
                        }
                    }
                }
                _ => self.freeze_dependency_input(source, value, bindings)?,
            }
        };
        prepared.values[slot] = Some(value);
        prepared.symbols[slot] = Some(symbol);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            references: vec![(n::LocalId { owner: 4, index: 2 }, 1)],
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
