//! Routed declaration inputs retain collection, dependency and slot checks.
//! The active slot owns its child demand and temporary observation/default
//! boundaries. Checked producers are moved into Progress, never replayed here.
use super::*;

use request::Request;

struct Pending<'b, 'h> {
    slot: usize,
    domain: DeclarationInputDomain,
    input: Option<&'b Expr<'h>>,
}

pub(super) enum Poll<'b, 'h> {
    Continue,
    Child(Request<'b, 'h>),
    Complete,
}

pub(super) struct Frame<'b, 'h> {
    span: Span,
    interface: interface::Interface<'h>,
    defaults: Option<environment::Environment>,
    checking: InputChecking,
    routed: Vec<Option<RoutedDeclarationInput<'b, 'h>>>,
    bindings: BTreeMap<n::LocalId, std::sync::Arc<Term>>,
    progress: progress::Progress,
    preparations: Vec<n::NormalFormPreparation>,
    collection: Option<collections::Frame<'b, 'h>>,
    pending: Option<Pending<'b, 'h>>,
    dependencies: Option<domain::Frame>,
    observations: Option<constants::PreparedObservations>,
    active_default: Option<(OwnerId, usize)>,
    default_scope: Option<scope::Scope>,
    complete: bool,
}

impl<'b, 'h: 'b> Frame<'b, 'h> {
    pub(super) fn new(
        span: Span,
        interface: interface::Interface<'h>,
        defaults: Option<environment::Environment>,
        checking: InputChecking,
        routed: Vec<Option<RoutedDeclarationInput<'b, 'h>>>,
        bindings: BTreeMap<n::LocalId, std::sync::Arc<Term>>,
        progress: progress::Progress,
        collection: collections::Frame<'b, 'h>,
    ) -> Self {
        Self {
            span,
            interface,
            defaults,
            checking,
            routed,
            bindings,
            progress,
            preparations: Vec::new(),
            collection: Some(collection),
            pending: None,
            dependencies: None,
            observations: None,
            active_default: None,
            default_scope: None,
            complete: false,
        }
    }

    pub(super) fn poll(&mut self, checker: &mut Checker<'_, '_>) -> Result<Poll<'b, 'h>, Error> {
        if self.complete || self.pending.is_some() {
            return Err(error(
                self.span,
                "declaration slot check is pending or complete",
            ));
        }
        if self.dependencies.is_some() {
            return self.poll_dependencies(checker);
        }
        if let Some(collection) = &self.collection {
            if let Some(slot) = collection.domain_slot() {
                self.dependencies = Some(domain::Frame::new(
                    slot,
                    self.progress.declaration_prefix(),
                    self.span,
                ));
                return self.poll_dependencies(checker);
            }
            return self.poll_collection(checker);
        }
        let Some(slot) = self.progress.current() else {
            self.complete = true;
            return Ok(Poll::Complete);
        };
        if self.routed[slot].is_some() && self.progress.may_need_preparation(slot) {
            self.dependencies = Some(domain::Frame::new(
                slot,
                self.progress.declaration_prefix(),
                self.span,
            ));
            return self.poll_dependencies(checker);
        }
        let domain = self.progress.domain(checker, slot, self.span)?;
        self.check_slot(checker, slot, domain)
    }

    fn poll_dependencies(&mut self, checker: &mut Checker<'_, '_>) -> Result<Poll<'b, 'h>, Error> {
        let dependencies = self.dependencies.as_mut().expect("active domain check");
        match dependencies.poll(checker, &self.progress, |slot| {
            self.routed[slot].is_none() && self.interface.default(slot).is_some()
        })? {
            domain::Poll::Continue => Ok(Poll::Continue),
            domain::Poll::Input { slot, domain } => self.check_slot(checker, slot, domain),
            domain::Poll::Complete(domain) => {
                let slot = self.dependencies.take().unwrap().slot();
                if let Some(collection) = &mut self.collection {
                    collection.accept_domain(domain)?;
                    Ok(Poll::Continue)
                } else {
                    self.check_slot(checker, slot, domain)
                }
            }
        }
    }

    fn poll_collection(&mut self, checker: &mut Checker<'_, '_>) -> Result<Poll<'b, 'h>, Error> {
        match self
            .collection
            .as_mut()
            .expect("active collection")
            .poll(checker)?
        {
            collections::Poll::Child(request) => Ok(Poll::Child(request)),
            collections::Poll::Complete => {
                let (slot, collected, preparations) = self.collection.take().unwrap().finish()?;
                if !collected.is_empty() {
                    self.routed[slot.expect("collected inputs have a collector")] =
                        Some(RoutedDeclarationInput::Collected(collected));
                }
                self.preparations = preparations;
                Ok(Poll::Continue)
            }
        }
    }

    fn check_slot(
        &mut self,
        checker: &mut Checker<'_, '_>,
        slot: usize,
        domain: DeclarationInputDomain,
    ) -> Result<Poll<'b, 'h>, Error> {
        if self.routed[slot].is_none() && self.interface.is_collector(slot) {
            let ty = domain.ty;
            if checker.list_element_type(&ty).is_none() {
                return Err(error(
                    self.span,
                    "normal-form variadic collector requires meta.List<T>",
                ));
            }
            let identity =
                checker.checked_collected_list_in(&[], &ty, &self.bindings, self.span)?;
            checker
                .locals
                .insert(self.progress.binding(slot), ty.clone());
            self.progress.record(
                slot,
                n::NormalFormArgument::Value(n::Expr {
                    source: source(self.span),
                    ty,
                    kind: K::TypeList(Vec::new()),
                }),
                identity,
                &mut checker.required_steps,
                self.span,
            )?;
            return Ok(Poll::Continue);
        }
        let explicit = self.routed[slot].take();
        let provider = self.interface.default(slot);
        let declaration_default = explicit.is_none()
            && self.checking == InputChecking::Contextual
            && provider
                .is_some_and(|provider| checker.normal_parameters.contains_key(&provider.owner));
        let input = match &explicit {
            Some(RoutedDeclarationInput::Source(input)) => Some(*input),
            Some(_) => None,
            None if declaration_default => None,
            None => {
                let provider = provider
                    .ok_or_else(|| error(self.span, "normal-form argument count mismatch"))?;
                self.default_scope = Some(scope::Scope::enter(checker, self.span)?);
                Some(self.enter_default(checker, slot, provider)?)
            }
        };
        if explicit.is_none() {
            let provider = provider.expect("omitted input has a default provider");
            let guard = (provider.owner, provider.slot);
            if checker.active_type_defaults.len() >= 64
                || !checker.active_type_defaults.insert(guard)
            {
                return Err(error(
                    self.span,
                    "cyclic or excessively deep normal-form default",
                ));
            }
            self.active_default = Some(guard);
        }
        self.pending = Some(Pending {
            slot,
            domain,
            input,
        });
        let domain = &self.pending.as_ref().unwrap().domain;
        let immediate = if declaration_default {
            let provider = provider.expect("declaration default provider");
            Some(
                checker
                    .connected_normal_default(
                        provider.owner,
                        provider.slot,
                        &self.progress,
                        self.span,
                    )
                    .map(n::NormalFormArgument::Value),
            )
        } else {
            match explicit {
                Some(RoutedDeclarationInput::Collected(items)) => {
                    Some(self.collection(checker, items))
                }
                Some(RoutedDeclarationInput::Prepared(value)) => Some((|| {
                    if domain.pending.is_empty() {
                        checker.require_expression(&value.ty, &domain.ty, self.span)?;
                    }
                    Ok(n::NormalFormArgument::Value(value))
                })()),
                _ => None,
            }
        };
        if let Some(value) = immediate {
            self.accept(checker, value)?;
            return Ok(Poll::Continue);
        }
        let contextual = !domain.pending.is_empty() && self.checking == InputChecking::Contextual;
        let expected = (domain.pending.is_empty())
            .then(|| checker.copy_alias_type(&domain.ty, self.span))
            .transpose()?;
        if !contextual {
            self.observations = Some(constants::PreparedObservations::enter(
                checker,
                &self.bindings,
                self.span,
            )?);
        }
        let input = input.expect("source or declaration default");
        Ok(Poll::Child(if contextual {
            Request::contextual(input)
        } else {
            Request::ordinary(input, expected)
        }))
    }

    fn collection(
        &self,
        checker: &mut Checker<'_, '_>,
        items: Vec<n::NormalFormArgument>,
    ) -> Result<n::NormalFormArgument, Error> {
        let domain = &self.pending.as_ref().unwrap().domain;
        if (domain.pending.is_empty() || self.checking == InputChecking::Typed)
            && items.iter().all(|item| {
                matches!(
                    item,
                    n::NormalFormArgument::Value(_) | n::NormalFormArgument::ListSpread(_)
                )
            })
        {
            let mut collection = clause_inputs::ClauseCollection::default();
            for item in items {
                match item {
                    n::NormalFormArgument::Value(value) => collection.push_element(value),
                    n::NormalFormArgument::ListSpread(value) => collection.push_spread(value),
                    _ => unreachable!("checked collection input"),
                }
            }
            checker
                .clause_collection_expression(collection, domain.ty.clone(), self.span)
                .map(n::NormalFormArgument::Value)
        } else {
            Ok(n::NormalFormArgument::List {
                source: source(self.span),
                items,
            })
        }
    }

    fn enter_default(
        &self,
        checker: &mut Checker<'_, '_>,
        slot: usize,
        provider: interface::DefaultSource<'h>,
    ) -> Result<&'h Expr<'h>, Error> {
        self.progress.require_default_prefix(slot, self.span)?;
        if let Some(defaults) = &self.defaults {
            defaults.install(checker);
        }
        // Only preceding declaration inputs enter a default. Their observations
        // never replace the executable producers retained in source order.
        for previous in 0..slot {
            let parameter = self.progress.telescope.parameters[previous].0;
            checker.alias_substitution.remove(&parameter);
            checker.alias_value_bindings.remove(&parameter);
            checker.normal_default_bindings.remove(&parameter);
            if checker.locals[&self.progress.binding(previous)] == p(P::Type) {
                if let Some(ty) = self.progress.identity(previous) {
                    let ty = checker.copy_alias_type(ty, self.span)?;
                    checker
                        .alias_substitution
                        .insert(parameter, std::sync::Arc::new(ty));
                } else {
                    checker
                        .alias_value_bindings
                        .insert(parameter, self.progress.binding(previous));
                }
            } else {
                if let Some(value) = self.progress.identity(previous) {
                    let value = checker.copy_alias_type(value, self.span)?;
                    checker
                        .alias_substitution
                        .insert(parameter, std::sync::Arc::new(value));
                }
                let domain = environment::copy_term(
                    &checker.locals[&self.progress.binding(previous)],
                    &mut checker.required_steps,
                    self.span,
                )?;
                checker
                    .normal_default_bindings
                    .insert(parameter, (self.progress.binding(previous), domain));
            }
        }
        checker.signature_owner = Some(provider.owner);
        checker.context = checker.binder_context(provider.owner);
        checker.in_required_comptime = true;
        Ok(provider.expression)
    }

    fn accept(
        &mut self,
        checker: &mut Checker<'_, '_>,
        result: Result<n::NormalFormArgument, Error>,
    ) -> Result<(), Error> {
        let pending = self
            .pending
            .take()
            .ok_or_else(|| error(self.span, "declaration slot has no pending child"))?;
        self.unwind(checker);
        let slot = pending.slot;
        let value = result.map_err(|mut failure| {
            if self.progress.telescope.parameters[slot].2 != p(P::Type)
                && failure.message.contains("type mismatch")
            {
                failure.message = format!(
                    "normal-form value argument type mismatch: {}",
                    failure.message
                );
            }
            failure
        })?;
        let identity = match &value {
            n::NormalFormArgument::List { items, .. } if pending.domain.pending.is_empty() => {
                checker.checked_collected_list_in(
                    items,
                    &pending.domain.ty,
                    &self.bindings,
                    self.span,
                )?
            }
            _ => match value.expression() {
                Some(value) => checker.preserved_input_identity(
                    value,
                    |id, _| Ok(self.bindings.get(&id).cloned()),
                    self.span,
                )?,
                None => None,
            },
        };
        // A known identity can specialize subsequent fields while its own
        // domain check remains pending. If using it would change the binder's
        // sort, only completion of that domain can authorize the replacement.
        let identity = identity.filter(|observed| {
            pending.domain.pending.is_empty()
                || observed.sort() == Sort::of_comptime_domain(&pending.domain.ty)
        });
        let ty = value
            .expression()
            .map(|expr| expr.ty.clone())
            .unwrap_or(pending.domain.ty);
        checker.record_declaration_input(
            &mut self.progress,
            slot,
            value,
            identity,
            ty,
            &mut self.bindings,
            self.span,
        )
    }

    pub(super) fn unwind(&mut self, checker: &mut Checker<'_, '_>) {
        if let Some(observations) = self.observations.take() {
            observations.leave(checker);
        }
        if let Some(scope) = self.default_scope.take() {
            scope.leave(checker);
        }
        if let Some(guard) = self.active_default.take() {
            checker.active_type_defaults.remove(&guard);
        }
    }

    pub(super) fn accept_child(
        &mut self,
        checker: &mut Checker<'_, '_>,
        result: Result<n::NormalFormArgument, Error>,
    ) -> Result<(), Error> {
        if self.pending.is_some() {
            self.accept(checker, result)
        } else if let Some(collection) = &mut self.collection {
            collection.accept(checker, &mut self.bindings, result)
        } else {
            self.accept(checker, result)
        }
    }

    pub(super) fn finish(
        self,
        checker: &mut Checker<'_, '_>,
    ) -> Result<(PreparedTelescope, PreparedDeclarationInputs), Error> {
        if !self.complete || self.pending.is_some() {
            return Err(error(
                self.span,
                "declaration inputs are not completely checked",
            ));
        }
        self.progress
            .finish(self.preparations, &mut checker.required_steps, self.span)
    }
}

impl std::fmt::Display for Frame<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(declaration-slot-frame {} (source {}) (checking {}) (complete {}) {} (pending",
            self.interface,
            source(self.span),
            match self.checking {
                InputChecking::Contextual => "contextual",
                InputChecking::Typed => "typed",
            },
            self.complete,
            self.progress
        )?;
        if let Some(pending) = &self.pending {
            write!(f, " (slot {} {})", pending.slot, pending.domain)?;
            if let Some(input) = pending.input {
                write!(
                    f,
                    " (child {} (source {}))",
                    input.hir_id,
                    source(input.span)
                )?;
            }
        }
        f.write_str(") (dependencies")?;
        if let Some(dependencies) = &self.dependencies {
            write!(f, " {dependencies}")?;
        }
        f.write_str(") (collection")?;
        if let Some(collection) = &self.collection {
            write!(f, " {collection}")?;
        }
        f.write_str(") (active-default")?;
        if let Some((owner, slot)) = self.active_default {
            write!(f, " (owner {owner}) (slot {slot})")?;
        }
        f.write_str(") (default-scope")?;
        if let Some(scope) = &self.default_scope {
            write!(f, " {scope}")?;
        }
        f.write_str(") (observations")?;
        if let Some(observations) = &self.observations {
            write!(f, " {observations}")?;
        }
        f.write_str(") (routed")?;
        for (slot, input) in self.routed.iter().enumerate() {
            if let Some(input) = input {
                write!(f, " (slot {slot} {input})")?;
            }
        }
        f.write_str(") (bindings")?;
        for (binding, value) in &self.bindings {
            write!(f, " (binding {binding} {value})")?;
        }
        f.write_str(") (preparations")?;
        for preparation in &self.preparations {
            write!(
                f,
                " (before {} {})",
                preparation.before, preparation.expression
            )?;
        }
        f.write_str(") (default-provider")?;
        if let Some(defaults) = &self.defaults {
            write!(f, " {defaults}")?;
        }
        f.write_str("))")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declaration_slot_dump_keeps_source_demand_and_prepared_code_separate() {
        let span = Span::default();
        let owner = hir::OwnerId::new(hir::LocalDefId::new(1));
        let source_expr = Expr {
            hir_id: hir::HirId::new(owner, hir::ItemLocalId::new(2)),
            span,
            kind: H::Unit,
        };
        let declaration = DeclarationId {
            package: 7,
            local: 1,
        };
        let binding = n::LocalId { owner: 1, index: 3 };
        let domain = p(P::Str);
        let parameters = [(declaration, Sort::ComptimeValue, domain.clone())];
        let progress = progress::Progress::new(
            domain::Telescope::new(parameters.into(), &mut 100, span).unwrap(),
            vec![binding],
            vec![0],
            KnownInputs::default(),
            vec![],
            &mut 100,
            span,
        )
        .unwrap();
        let mut frame = Frame::new(
            span,
            interface::Interface::positional(1, None, span).unwrap(),
            None,
            InputChecking::Typed,
            vec![None],
            BTreeMap::new(),
            progress,
            collections::Frame::new(span, InputChecking::Typed, None, 0, vec![], vec![]),
        );
        frame.collection = None;
        frame.preparations = vec![n::NormalFormPreparation {
            before: 0,
            expression: n::Expr {
                source: source(span),
                ty: domain.clone(),
                kind: K::Literal(n::Literal::String("prepared\"\n".into())),
            },
        }];
        frame.pending = Some(Pending {
            slot: 0,
            domain: DeclarationInputDomain {
                ty: domain.clone(),
                pending: BTreeSet::new(),
            },
            input: Some(&source_expr),
        });
        let request = Request::ordinary(&source_expr, Some(domain));
        let dump = frame.to_string();
        for fragment in [
            "(checking typed)",
            "(unprepared)",
            "(child ",
            "(pending)",
            "(preparations (before 0",
            "prepared\\\"\\n",
        ] {
            assert!(dump.contains(fragment), "missing {fragment}: {dump}");
        }
        assert!(!dump.contains('\n'), "{dump}");
        let request = request.to_string();
        assert!(
            request.contains("(ordinary (expected (primitive str)))"),
            "{request}"
        );
        assert!(!request.contains("prepared"), "{request}");
    }
}
