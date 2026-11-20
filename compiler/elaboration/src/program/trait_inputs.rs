//! Source trait input domains and declaration-ordered supplied/default indices.
use super::*;

impl Checker<'_, '_> {
    pub(super) fn trait_domains(&mut self, owner: OwnerId) -> Result<Vec<Term>, Error> {
        let span = self
            .package
            .item(owner)
            .map(|item| item.span)
            .unwrap_or_default();
        if let Some(domains) = self.trait_parameter_types.get(&owner) {
            return domains
                .iter()
                .map(|domain| {
                    declaration_inputs::copy_input_domain(domain, &mut self.required_steps, span)
                })
                .collect();
        }
        if !matches!(
            self.package.item(owner).map(|i| &i.kind),
            Some(
                ItemKind::Trait(_)
                    | ItemKind::Struct(_)
                    | ItemKind::Enum(_)
                    | ItemKind::Impl(_)
                    | ItemKind::Newtype(..)
                    | ItemKind::TypeAlias(..)
            )
        ) {
            return Ok(Vec::new());
        }
        let mut domains = Vec::new();
        for clause in self.type_clause_params(owner).iter() {
            domains.push(match *clause.kind.unquoted() {
                hir::clause::ClauseParamKind::Positional(_, ty)
                | hir::clause::ClauseParamKind::Optional(_, ty, _)
                | hir::clause::ClauseParamKind::Varadic(_, ty) => self.type_expr(ty)?,
                _ => p(P::Type),
            });
        }
        let retained = domains
            .iter()
            .map(|domain| {
                declaration_inputs::copy_input_domain(domain, &mut self.required_steps, span)
            })
            .collect::<Result<_, _>>()?;
        for (id, domain) in self.type_parameters[&owner].iter().zip(&domains) {
            self.implementation_index
                .register_parameter_domain(*id, domain.clone())
                .map_err(|message| error(self.package.item(owner).unwrap().span, message))?;
            self.inherent_index
                .register_parameter_domain(*id, domain.clone())
                .map_err(|message| error(self.package.item(owner).unwrap().span, message))?;
        }
        for &id in &self.type_parameters[&owner] {
            self.parameter_domain_owners.entry(id).or_insert(owner);
        }
        self.trait_parameter_types.insert(owner, retained);
        // Self skeletons may be registered before explicit value domains are
        // known. Refresh all nominal members when their telescope is complete.
        if matches!(
            self.package.item(owner).map(|item| &item.kind),
            Some(ItemKind::Struct(_) | ItemKind::Enum(_))
        ) {
            let ty = self.nominal_self_type(owner);
            self.self_types.insert(owner, ty.clone());
            for (member, container) in &self.self_containers {
                if *container == owner {
                    self.self_types.insert(*member, ty.clone());
                }
            }
        }

        Ok(domains)
    }

    pub(super) fn declaration_binders(&self, owner: OwnerId) -> Vec<nir::DeclarationParameter> {
        self.input_binders(&self.type_parameters[&owner])
    }

    pub(super) fn input_binders(
        &self,
        parameters: &[DeclarationId],
    ) -> Vec<nir::DeclarationParameter> {
        parameters
            .iter()
            .map(|id| nir::DeclarationParameter {
                declaration: *id,
                sort: self
                    .trait_input_domain(*id)
                    .as_ref()
                    .map_or(Sort::Type, Sort::of_comptime_domain),
            })
            .collect()
    }

    pub(super) fn trait_input_domain(&self, id: DeclarationId) -> Option<Term> {
        self.observe_input_domain(id, || Ok(()))
            .expect("unmetered domain observation cannot exhaust fuel")
            .cloned()
    }

    // One lookup order for both existing owned consumers and bounded borrowed
    // observers. Inspecting a sort does not require copying the domain payload.
    pub(super) fn observe_input_domain(
        &self,
        id: DeclarationId,
        mut charge: impl FnMut() -> Result<(), nir::type_computations::Error>,
    ) -> Result<Option<&Term>, nir::type_computations::Error> {
        charge()?;
        if let Some(domain) = self.lexical_input_domains.get(&id) {
            return Ok(Some(domain));
        }
        charge()?;
        if let Some(domain) = self.nominal_input_prefixes.get(&id) {
            return Ok(Some(domain));
        }
        if let Some(&owner) = self.parameter_domain_owners.get(&id) {
            charge()?;
            if let Some(domains) = self.trait_parameter_types.get(&owner) {
                for (index, input) in self.type_parameters[&owner].iter().enumerate() {
                    charge()?;
                    if *input == id {
                        if let Some(domain) = domains.get(index) {
                            return Ok(Some(domain));
                        }
                        break;
                    }
                }
            }
            for prefix in self
                .normal_input_prefixes
                .get(&owner)
                .into_iter()
                .chain(self.normal_parameters.get(&owner))
            {
                charge()?;
                for (parameter, _, domain) in prefix {
                    charge()?;
                    if *parameter == id {
                        return Ok(Some(domain));
                    }
                }
            }
        }
        Ok(None)
    }

    pub(super) fn bounded_input_substitution<'a>(
        &self,
        parameters: &[DeclarationId],
        arguments: &'a [Term],
        frames: &'a [Vec<Sort>],
        fuel: &mut usize,
    ) -> Result<nir::DeclarationSubstitution<'a>, nir::type_computations::Error> {
        use nir::type_computations::Error::Budget;
        *fuel = fuel.checked_sub(parameters.len()).ok_or(Budget)?;
        let mut domains = Vec::with_capacity(parameters.len());
        for id in parameters {
            let domain = self.observe_input_domain(*id, || {
                *fuel = fuel.checked_sub(1).ok_or(Budget)?;
                Ok(())
            })?;
            domains.push(domain.unwrap_or(&Term::Primitive(P::Type)));
        }
        nir::DeclarationSubstitution::bounded_domains(
            parameters,
            &domains,
            arguments,
            nir::DeclarationInputScope::Caller(frames),
            fuel,
        )
    }

    pub(super) fn substituted_trait_domain(
        &mut self,
        domain: &Term,
        span: Span,
    ) -> Result<Term, Error> {
        // An empty environment still goes through domain validation and
        // associated-type observation. It needs no declaration-table scan.
        if self.alias_substitution.is_empty() {
            return self.instantiate_trait_domain(domain, &[], &[], span);
        }
        let mut binders = Vec::new();
        let mut values = Vec::new();
        // Members can inherit the same declaration parameter. A substitution
        // telescope contains that identity once, regardless of its owners.
        let mut seen = BTreeSet::new();
        for owner in self.trait_parameter_types.keys() {
            let self_binder = nir::DeclarationParameter {
                declaration: declaration(self.package, &self.package_identities, *owner),
                sort: Sort::Type,
            };
            // Select identities before obtaining their sorts. Constructing the
            // complete binder list here repeatedly searched the global domain
            // table for every unrelated parameter at every type reference.
            for id in std::iter::once(self_binder.declaration)
                .chain(self.type_parameters[owner].iter().copied())
            {
                if !seen.insert(id) {
                    continue;
                }
                let Some(value) = self.alias_substitution.shared(&id) else {
                    continue;
                };
                if !domain.mentions_rigid(id) {
                    continue;
                }
                let binder = if id == self_binder.declaration {
                    self_binder
                } else {
                    self.input_binders(&[id])[0]
                };
                binders.push(binder);
                values.push(value.clone());
            }
        }
        let values = values
            .iter()
            .map(|value| self.copy_alias_type(value, span))
            .collect::<Result<Vec<_>, _>>()?;
        self.instantiate_trait_domain(domain, &binders, &values, span)
    }

    pub(super) fn instantiate_trait_domain(
        &mut self,
        domain: &Term,
        binders: &[nir::DeclarationParameter],
        values: &[Term],
        span: Span,
    ) -> Result<Term, Error> {
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let ids = binders
            .iter()
            .map(|binder| binder.declaration)
            .collect::<Vec<_>>();
        let domains = ids
            .iter()
            .map(|id| self.trait_input_domain(*id).unwrap_or_else(|| p(P::Type)))
            .collect::<Vec<_>>();
        let frames = self.call_input_frames();
        let result =
            nir::DeclarationSubstitution::bounded(&ids, &domains, values, &frames, &mut fuel)
                .and_then(|substitution| substitution.instantiate_bounded(domain, &mut fuel));
        self.required_steps -= (before - fuel) as u64;
        let domain = result
            .map_err(|e| error(span, format!("trait input domain substitution failed: {e}")))?;
        // Substituting Self may make a previously abstract associated domain
        // concrete. Obtain that view before contextual literals are checked,
        // so widths and overflow rules come from the selected implementation.
        self.callable_input_view(&domain, span)
    }

    fn supplied_trait_domain(
        &mut self,
        owner: OwnerId,
        domain: &Term,
        supplied: &[Option<Term>],
        prefix: &[Term],
        subject: Option<&Term>,
        span: Span,
    ) -> Result<Term, Error> {
        let binders = self.declaration_binders(owner);
        let mut substitutions = Vec::new();
        let mut values = Vec::new();
        if let Some(subject) = subject {
            substitutions.push(nir::DeclarationParameter {
                declaration: declaration(self.package, &self.package_identities, owner),
                sort: Sort::Type,
            });
            values.push(self.copy_alias_type(subject, span)?);
        }
        for (binder, supplied) in binders.iter().zip(
            prefix
                .iter()
                .map(Some)
                .chain(supplied.iter().map(Option::as_ref)),
        ) {
            if let Some(value) = supplied {
                substitutions.push(*binder);
                values.push(self.copy_alias_type(value, span)?);
            }
        }
        self.instantiate_trait_domain(domain, &substitutions, &values, span)
    }

    pub(super) fn check_trait_defaults(
        &mut self,
        owner: OwnerId,
        domains: &[Term],
    ) -> Result<(), Error> {
        let clauses = self.type_clause_params(owner);
        let inherited = self.type_parameters[&owner]
            .len()
            .saturating_sub(clauses.len());
        let parameters = self.type_parameters[&owner][inherited..].to_vec();
        let domains = &domains[inherited..];
        self.in_formal_declaration(owner, |this| {
            this.in_required_comptime = true;
            this.context = this.definition_context(owner);
            this.input_facts.supplied = this.input_facts.supplied.child(
                &[],
                &[],
                &mut this.required_steps,
                this.package.item(owner).unwrap().span,
            )?;
            // Captured parent inputs are already in lexical scope before the
            // member's own telescope, including in its default expressions.
            for parameter in this.nominal_captures(owner, this.package.item(owner).unwrap().span)? {
                if let Some(domain) = this
                    .trait_input_domain(parameter)
                    .filter(|ty| Sort::of_comptime_domain(ty) == Sort::ComptimeValue)
                {
                    this.install_abstract_clause_input(
                        parameter,
                        &domain,
                        this.package.item(owner).unwrap().span,
                    )?;
                }
            }
            this.forbidden_default_inputs.extend(&parameters);
            for (slot, clause) in clauses.iter().enumerate() {
                if let hir::clause::ClauseParamKind::Optional(_, _, value) = clause.kind.unquoted()
                {
                    if let Term::Lift(inner) = &domains[slot] {
                        this.check_runtime_default(value, inner)?;
                    } else if matches!(
                        this.package.item(owner).map(|item| &item.kind),
                        Some(
                            ItemKind::Struct(_)
                                | ItemKind::Enum(_)
                                | ItemKind::Newtype(..)
                                | ItemKind::TypeAlias(..)
                        )
                    ) && this.default_domain_has_rigid(&domains[slot], value.span)?
                    {
                        this.dependent_normal_argument(value, 0)?;
                    } else {
                        this.expr(value, Some(&domains[slot]))?;
                    }
                }
                this.forbidden_default_inputs.remove(&parameters[slot]);
                if Sort::of_comptime_domain(&domains[slot]) == Sort::ComptimeValue {
                    this.install_abstract_clause_input(
                        parameters[slot],
                        &domains[slot],
                        clause.span,
                    )?;
                }
            }
            Ok(())
        })
    }

    fn install_abstract_clause_input(
        &mut self,
        parameter: DeclarationId,
        domain: &Term,
        span: Span,
    ) -> Result<(), Error> {
        let binding = self.spread_binding(span)?;
        let value = Term::Rigid {
            declaration: parameter,
            sort: Sort::ComptimeValue,
        };
        self.input_facts.supplied.supply(
            binding,
            domain,
            &value,
            &mut self.required_steps,
            span,
        )?;
        self.locals.insert(binding, domain.clone());
        self.immutable.insert(binding);
        self.normal_default_bindings
            .insert(parameter, (binding, domain.clone()));
        Ok(())
    }

    /// Fix an explicitly supplied positional prefix; remaining declaration
    /// inputs are solved by the ordinary call candidate, not guessed here.
    pub(super) fn partial_positional_clause_prefix(
        &mut self,
        owner: OwnerId,
        args: &[Arg<'_>],
        fixed: &[Term],
        span: Span,
    ) -> Result<Option<Vec<Term>>, Error> {
        let clauses = self.type_clause_params(owner);
        let count = self.type_parameters[&owner].len();
        let inherited = count.saturating_sub(clauses.len());
        if fixed.len() < inherited
            || fixed.len() > count
            || args.is_empty()
            || args.len() >= count - fixed.len()
            || self.has_optional_type_clauses(owner)
            || !args.iter().all(|arg| matches!(arg, Arg::Positional(_)))
        {
            return Ok(None);
        }
        let domains = self.trait_domains(owner)?;
        let mut supplied = vec![None; clauses.len()];
        for (slot, value) in fixed[inherited..].iter().enumerate() {
            supplied[slot] = Some(value.clone());
        }
        let mut result = fixed.to_vec();
        for arg in args {
            let Arg::Positional(value) = arg else {
                unreachable!()
            };
            let slot = result.len() - inherited;
            let domain = self.supplied_trait_domain(
                owner,
                &domains[result.len()],
                &supplied,
                &fixed[..inherited],
                None,
                span,
            )?;
            let value = self.normal_value_argument(value, &domain)?;
            supplied[slot] = Some(value.clone());
            result.push(value);
        }
        Ok(Some(result))
    }

    pub(super) fn trait_clause_arguments(
        &mut self,
        owner: OwnerId,
        args: &[Arg<'_>],
        subject: Option<&Term>,
        span: Span,
        domains: &[Term],
    ) -> Result<Vec<Term>, Error> {
        self.trait_clause_arguments_with_environment(owner, args, subject, span, domains, None)
    }

    pub(super) fn trait_clause_arguments_with_environment(
        &mut self,
        owner: OwnerId,
        args: &[Arg<'_>],
        subject: Option<&Term>,
        span: Span,
        domains: &[Term],
        defaults: Option<&BTreeMap<DeclarationId, std::sync::Arc<Term>>>,
    ) -> Result<Vec<Term>, Error> {
        self.trait_clause_arguments_with_prefix(
            owner,
            args,
            subject,
            span,
            domains,
            defaults,
            Vec::new(),
        )
    }

    pub(super) fn trait_clause_arguments_with_prefix(
        &mut self,
        owner: OwnerId,
        args: &[Arg<'_>],
        subject: Option<&Term>,
        span: Span,
        domains: &[Term],
        defaults: Option<&BTreeMap<DeclarationId, std::sync::Arc<Term>>>,
        prefix: Vec<Term>,
    ) -> Result<Vec<Term>, Error> {
        let clauses = self.type_clause_params(owner);
        let all_parameters = self.type_parameters[&owner].clone();
        if all_parameters.len() != prefix.len() + clauses.len()
            || domains.len() != all_parameters.len()
        {
            return Err(error(span, "clause input prefix arity mismatch"));
        }
        let parameters = all_parameters[prefix.len()..].to_vec();
        let domains = &domains[prefix.len()..];
        let mut supplied: Vec<Option<Term>> = vec![None; clauses.len()];
        let mut pending_domains = vec![None; clauses.len()];
        let mut pending_literals = vec![None; clauses.len()];
        let mut pending_runtime = vec![None; clauses.len()];
        let mut positional = 0;
        // Explicit inputs run in caller/source order. Defaults run afterwards
        // in the definition environment; no caller name becomes a default.
        let mut pending: std::collections::VecDeque<_> = args.iter().cloned().collect();
        while let Some(arg) = pending.pop_front() {
            let (name, value) = match arg {
                Arg::Positional(value) => (None, value),
                Arg::Named(name, value) => (Some(name), value),
                Arg::Expand(value) => {
                    if matches!(value.kind, H::Unit) {
                        continue;
                    }
                    if let H::Tuple(fields) = value.kind {
                        self.required_steps = self
                            .required_steps
                            .checked_sub(fields.len() as u64)
                            .ok_or_else(|| {
                                error(value.span, "trait input packet budget exhausted")
                            })?;
                        for field in fields.iter().rev() {
                            pending.push_front(Arg::Positional(field));
                        }
                        continue;
                    }
                    // Infer only an unambiguous positional layout. The layout
                    // probe reads signatures/shapes and never invokes a packet.
                    let remaining_slots: Vec<_> = (positional..clauses.len())
                        .filter(|slot| {
                            !matches!(
                                clauses[*slot].kind.unquoted(),
                                hir::clause::ClauseParamKind::Optional(..)
                            )
                        })
                        .collect();
                    let remaining_args: Vec<_> = std::iter::once(Arg::Expand(value))
                        .chain(pending.iter().cloned())
                        .collect();
                    let empty = remaining_args
                        .iter()
                        .enumerate()
                        .filter_map(|(index, arg)| {
                            matches!(arg, Arg::Expand(expr) if matches!(expr.kind, H::Unit))
                                .then_some((index, 0))
                        })
                        .collect();
                    let layout = self.spread_packet_layout_with_widths(
                        &remaining_args,
                        &remaining_slots,
                        value.span,
                        &empty,
                    )?;
                    let mut context = None;
                    if let Some(slots) = &layout[0] {
                        let mut types = Vec::new();
                        let mut ready = true;
                        for slot in slots {
                            let domain = self.supplied_trait_domain(
                                owner,
                                &domains[*slot],
                                &supplied,
                                &prefix,
                                subject,
                                value.span,
                            )?;
                            if !input_observation::pending_domain_inputs(
                                &domains[*slot],
                                parameters
                                    .iter()
                                    .zip(&supplied)
                                    .map(|(id, value)| (*id, value.is_some())),
                                &mut self.required_steps,
                                value.span,
                            )?
                            .is_empty()
                            {
                                ready = false;
                                break;
                            }
                            types.push(domain);
                        }
                        if ready {
                            context = Some(self.tuple_value_type(types, value.span)?);
                        }
                    }
                    let (packet, ty) = self.required_typed_expression(value, context.as_ref())?;
                    let Term::Tuple(fields) = self.structural_type(&ty, value.span)? else {
                        return Err(error(value.span, "trait argument spread requires a tuple"));
                    };
                    let comptime_interpretor::Value::Tuple(values) = packet else {
                        return Err(error(
                            value.span,
                            "trait argument spread did not produce a tuple",
                        ));
                    };
                    if fields.len() != values.len() {
                        return Err(error(value.span, "trait argument packet shape mismatch"));
                    }
                    for (actual, element) in fields.into_iter().zip(values) {
                        while positional < clauses.len()
                            && matches!(
                                clauses[positional].kind.unquoted(),
                                hir::clause::ClauseParamKind::Optional(..)
                            )
                        {
                            positional += 1;
                        }
                        let slot = positional;
                        positional += 1;
                        if slot >= clauses.len() {
                            return Err(error(value.span, "trait argument count mismatch"));
                        }
                        if supplied[slot].is_some()
                            || pending_literals[slot].is_some()
                            || pending_runtime[slot].is_some()
                        {
                            return Err(error(value.span, "duplicate trait argument"));
                        }
                        let element = match element {
                            comptime_interpretor::Value::TypeParameter(declaration)
                                if actual == p(P::Type) =>
                            {
                                Term::Rigid {
                                    declaration,
                                    sort: Sort::Type,
                                }
                            }
                            element => self.required_normal_term(element, value.span)?,
                        };
                        supplied[slot] = Some(element);
                        pending_domains[slot] = Some((actual, value.span));
                    }
                    continue;
                }
                _ => return Err(error(span, "unsupported trait argument routing protocol")),
            };
            let slot = if let Some(name) = name {
                clauses
                    .iter()
                    .position(|clause| {
                        clause.name.name == name.name
                            && matches!(
                                clause.kind.unquoted(),
                                hir::clause::ClauseParamKind::Optional(..)
                            )
                    })
                    .ok_or_else(|| error(span, "unknown named trait argument"))?
            } else {
                while positional < clauses.len()
                    && matches!(
                        clauses[positional].kind.unquoted(),
                        hir::clause::ClauseParamKind::Optional(..)
                    )
                {
                    positional += 1;
                }
                let slot = positional;
                positional += 1;
                slot
            };
            let domain = domains
                .get(slot)
                .ok_or_else(|| error(span, "trait argument count mismatch"))?;
            if supplied[slot].is_some()
                || pending_literals[slot].is_some()
                || pending_runtime[slot].is_some()
            {
                return Err(error(span, "duplicate trait argument"));
            }
            // A supplied value is checked in the caller environment, against
            // the domain instantiated by inputs already evaluated in source
            // order. Do not reorder effectful argument computations.
            let unresolved_domain = !input_observation::pending_domain_inputs(
                domain,
                parameters
                    .iter()
                    .zip(&supplied)
                    .map(|(id, value)| (*id, value.is_some())),
                &mut self.required_steps,
                value.span,
            )?
            .is_empty();
            let domain =
                self.supplied_trait_domain(owner, domain, &supplied, &prefix, subject, value.span)?;
            // Bare numeric literals have intrinsic conversion only: no call,
            // capture, or user literal protocol can execute here. Keep their
            // spelling until the domain is known instead of fixing Integer or
            // Float prematurely. Suffixed literals and general expressions
            // still follow source-order required invocation below.
            if unresolved_domain && is_numeric_literal(value) {
                pending_literals[slot] = Some(value);
                continue;
            }
            if unresolved_domain
                && (clauses[slot].kind.is_quote() || matches!(domain, Term::Lift(_)))
            {
                pending_runtime[slot] = Some(value);
                continue;
            }
            supplied[slot] = Some(if unresolved_domain {
                // Preserve source-order invocation when an earlier declaration
                // slot is supplied later (or by a default). The expression's
                // own checked type is retained for the final domain check.
                let symbolic_domain = if let H::ResolvedClauseParam(_, id) = value.kind {
                    let input = self.visible_clause_input(id, value.span)?;
                    self.trait_input_domain(input)
                } else {
                    None
                };
                if let Some(actual) = symbolic_domain {
                    // A caller clause is already a stable symbolic input.
                    // Keep it symbolic while the callee's domain awaits an
                    // earlier default; no invocation can supply its local yet.
                    let actual = self.substituted_trait_domain(&actual, value.span)?;
                    let input = self.normal_value_argument(value, &actual)?;
                    pending_domains[slot] = Some((actual, value.span));
                    input
                } else {
                    let (computed, actual) = self.required_typed_expression(value, None)?;
                    pending_domains[slot] = Some((actual, value.span));
                    self.required_normal_term(computed, value.span)?
                }
            } else {
                self.normal_value_argument(value, &domain)?
            });
        }
        struct CallerScope {
            owner: Option<OwnerId>,
            context: crate::ptc::Context,
            aliases: DeclarationBindings<Term>,
            forbidden: BTreeSet<DeclarationId>,
            bindings: DeclarationBindings<(n::LocalId, Term)>,
            value_bindings: DeclarationBindings<n::LocalId>,
        }
        impl CallerScope {
            fn swap(&mut self, checker: &mut Checker<'_, '_>) {
                std::mem::swap(&mut self.owner, &mut checker.signature_owner);
                std::mem::swap(&mut self.context, &mut checker.context);
                std::mem::swap(&mut self.aliases, &mut checker.alias_substitution);
                std::mem::swap(&mut self.forbidden, &mut checker.forbidden_default_inputs);
                std::mem::swap(&mut self.bindings, &mut checker.normal_default_bindings);
                std::mem::swap(&mut self.value_bindings, &mut checker.alias_value_bindings);
            }
            fn with_caller<R>(
                &mut self,
                checker: &mut Checker<'_, '_>,
                run: impl FnOnce(&mut Checker<'_, '_>) -> R,
            ) -> R {
                self.swap(checker);
                let result = run(checker);
                self.swap(checker);
                result
            }
        }
        let mut caller_scope = CallerScope {
            owner: self.signature_owner.replace(owner),
            context: self.context.clone(),
            aliases: self.alias_substitution.clone(),
            forbidden: self.forbidden_default_inputs.clone(),
            bindings: self.normal_default_bindings.clone(),
            value_bindings: self.alias_value_bindings.clone(),
        };
        if let Some(environment) = defaults {
            self.alias_substitution.extend(environment.clone());
        }
        if let Some(environment) = defaults {
            for parameter in environment.keys() {
                self.normal_default_bindings.remove(parameter);
                self.alias_value_bindings.remove(parameter);
            }
        }
        // Explicit inputs were evaluated in the caller. Defaults now belong to
        // this application, even when a recursive generator reuses binder IDs.
        for parameter in &parameters {
            self.normal_default_bindings.remove(parameter);
        }
        self.context = self.definition_context(owner);
        if let Some(subject) = subject {
            self.alias_substitution.insert(
                declaration(self.package, &self.package_identities, owner),
                std::sync::Arc::new(subject.clone()),
            );
        }
        let result = (|| {
            for (parameter, value) in all_parameters.iter().zip(&prefix) {
                self.alias_substitution
                    .insert(*parameter, std::sync::Arc::new(value.clone()));
                self.normal_default_bindings.remove(parameter);
                self.alias_value_bindings.remove(parameter);
            }
            let mut arguments = prefix;
            for (slot, supplied) in supplied.into_iter().enumerate() {
                let value = if let Some(value) = supplied {
                    if let Some((actual, value_span)) = &pending_domains[slot] {
                        let domain = self.substituted_trait_domain(&domains[slot], *value_span)?;
                        // Explicit expressions belong to the caller, including
                        // its type equalities, aliases, and signature owner.
                        caller_scope
                            .with_caller(self, |this| this.require(actual, &domain, *value_span))?;
                    }
                    value
                } else if let Some(literal) = pending_literals[slot] {
                    let domain = self.substituted_trait_domain(&domains[slot], literal.span)?;
                    caller_scope
                        .with_caller(self, |this| this.normal_value_argument(literal, &domain))?
                } else if let Some(runtime_expr) = pending_runtime[slot] {
                    let domain =
                        self.substituted_trait_domain(&domains[slot], runtime_expr.span)?;
                    caller_scope.with_caller(self, |this| {
                        this.normal_value_argument(runtime_expr, &domain)
                    })?
                } else {
                    let hir::clause::ClauseParamKind::Optional(_, _, value) =
                        *clauses[slot].kind.unquoted()
                    else {
                        return Err(error(span, "trait argument count mismatch"));
                    };
                    if self.active_type_defaults.len() >= 64
                        || !self.active_type_defaults.insert((owner, slot))
                    {
                        return Err(error(span, "cyclic or excessively deep trait default"));
                    }
                    self.forbidden_default_inputs
                        .extend(parameters[slot..].iter().copied());
                    let result = (|| {
                        let domain = self.substituted_trait_domain(&domains[slot], value.span)?;
                        if matches!(domain, Term::Lift(_)) {
                            let prefix_args: Vec<_> = subject
                                .cloned()
                                .into_iter()
                                .chain(arguments.iter().cloned())
                                .collect();
                            self.default_runtime_argument(
                                owner,
                                parameters[slot],
                                &prefix_args,
                                value,
                                &domain,
                                span,
                            )
                        } else {
                            self.normal_value_argument(value, &domain)
                        }
                    })();
                    self.forbidden_default_inputs = caller_scope.forbidden.clone();
                    self.active_type_defaults.remove(&(owner, slot));
                    result?
                };
                self.alias_substitution
                    .insert(parameters[slot], std::sync::Arc::new(value.clone()));
                arguments.push(value);
            }
            Ok(arguments)
        })();
        caller_scope.swap(self);
        result
    }
}
