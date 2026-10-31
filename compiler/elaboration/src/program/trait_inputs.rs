//! Source trait input domains and declaration-ordered supplied/default indices.
use super::*;

impl Checker<'_, '_> {
    pub(super) fn trait_domains(&mut self, owner: OwnerId) -> Result<Vec<Term>, Error> {
        if let Some(domains) = self.trait_parameter_types.get(&owner) {
            return Ok(domains.clone());
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
        for clause in self.type_clause_params(owner) {
            domains.push(match clause.kind {
                hir::clause::ClauseParamKind::Positional(_, ty)
                | hir::clause::ClauseParamKind::Optional(_, ty, _) => self.type_expr(ty)?,
                _ => p(P::Type),
            });
        }
        for (id, domain) in self.type_parameters[&owner].iter().zip(&domains) {
            self.implementation_index
                .register_parameter_domain(*id, domain.clone())
                .map_err(|message| error(self.package.item(owner).unwrap().span, message))?;
            self.inherent_index
                .register_parameter_domain(*id, domain.clone())
                .map_err(|message| error(self.package.item(owner).unwrap().span, message))?;
        }
        self.trait_parameter_types.insert(owner, domains.clone());
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
                sort: if self
                    .trait_input_domain(*id)
                    .is_some_and(|ty| ty != p(P::Type))
                {
                    Sort::ComptimeValue
                } else {
                    Sort::Type
                },
            })
            .collect()
    }

    pub(super) fn trait_input_domain(&self, id: DeclarationId) -> Option<Term> {
        if let Some(domain) = self.lexical_input_domains.get(&id) {
            return Some(domain.clone());
        }
        if let Some(domain) = self.nominal_input_prefixes.get(&id) {
            return Some(domain.clone());
        }
        self.trait_parameter_types
            .iter()
            .find_map(|(owner, types)| {
                self.type_parameters[owner]
                    .iter()
                    .position(|input| *input == id)
                    .and_then(|index| types.get(index).cloned())
            })
            .or_else(|| {
                self.normal_parameters
                    .values()
                    .flatten()
                    .find(|(parameter, _, _)| *parameter == id)
                    .map(|(_, _, domain)| domain.clone())
            })
    }

    pub(super) fn substituted_trait_domain(
        &mut self,
        domain: &Term,
        span: Span,
    ) -> Result<Term, Error> {
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
            for binder in std::iter::once(self_binder).chain(self.declaration_binders(*owner)) {
                if !seen.insert(binder.declaration) || !domain.mentions_rigid(binder.declaration) {
                    continue;
                }
                if let Some(value) = self.alias_substitution.get(&binder.declaration) {
                    binders.push(binder);
                    values.push(value.clone());
                }
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
        let all_parameters = self.type_parameters[&owner].clone();
        let inherited = all_parameters.len().saturating_sub(clauses.len());
        let parameters = all_parameters[inherited..].to_vec();
        let domains = &domains[inherited..];
        let saved_owner = self.signature_owner.replace(owner);
        let saved_context = self.context.clone();
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_bindings = std::mem::take(&mut self.normal_default_bindings);
        let saved_aliases = std::mem::take(&mut self.alias_substitution);
        let saved_forbidden = self.forbidden_default_inputs.clone();
        let saved_mode = std::mem::replace(&mut self.in_required_comptime, true);
        self.context = self.definition_context(owner);
        let result = (|| {
            // Captured parent inputs are already in lexical scope before the
            // member's own telescope, including in its default expressions.
            for parameter in self.nominal_captures(owner, self.package.item(owner).unwrap().span)? {
                if let Some(domain) = self
                    .trait_input_domain(parameter)
                    .filter(|ty| *ty != p(P::Type))
                {
                    let binding = self.spread_binding(self.package.item(owner).unwrap().span)?;
                    self.locals.insert(binding, domain.clone());
                    self.normal_default_bindings
                        .insert(parameter, (binding, domain));
                }
            }
            for (slot, clause) in clauses.iter().enumerate() {
                if let hir::clause::ClauseParamKind::Optional(_, _, value) = clause.kind {
                    self.forbidden_default_inputs
                        .extend(parameters[slot..].iter().copied());
                    let contextual = if matches!(
                        self.package.item(owner).map(|item| &item.kind),
                        Some(
                            ItemKind::Struct(_)
                                | ItemKind::Enum(_)
                                | ItemKind::Newtype(..)
                                | ItemKind::TypeAlias(..)
                        )
                    ) {
                        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                        let before = fuel;
                        let result = nir::type_computations::has_rigid_parameters_bounded(
                            &domains[slot],
                            &mut fuel,
                        );
                        self.required_steps -= (before - fuel) as u64;
                        result.map_err(|error| {
                            super::error(
                                value.span,
                                format!("default input domain inspection failed: {error}"),
                            )
                        })?
                    } else {
                        false
                    };
                    if contextual {
                        self.dependent_normal_argument(value, 0)?;
                    } else {
                        self.expr(value, Some(&domains[slot]))?;
                    }
                    self.forbidden_default_inputs = saved_forbidden.clone();
                }
                if domains[slot] != p(P::Type) {
                    let binding = self.spread_binding(clause.span)?;
                    self.locals.insert(binding, domains[slot].clone());
                    self.normal_default_bindings
                        .insert(parameters[slot], (binding, domains[slot].clone()));
                }
            }
            Ok(())
        })();
        self.signature_owner = saved_owner;
        self.context = saved_context;
        self.locals = saved_locals;
        self.normal_default_bindings = saved_bindings;
        self.alias_substitution = saved_aliases;
        self.forbidden_default_inputs = saved_forbidden;
        self.in_required_comptime = saved_mode;
        result
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
                                clauses[*slot].kind,
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
                            if parameters.iter().enumerate().any(|(index, parameter)| {
                                supplied[index].is_none() && domain.mentions_rigid(*parameter)
                            }) {
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
                                clauses[positional].kind,
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
                        if supplied[slot].is_some() || pending_literals[slot].is_some() {
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
                            && matches!(clause.kind, hir::clause::ClauseParamKind::Optional(..))
                    })
                    .ok_or_else(|| error(span, "unknown named trait argument"))?
            } else {
                while positional < clauses.len()
                    && matches!(
                        clauses[positional].kind,
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
            if supplied[slot].is_some() || pending_literals[slot].is_some() {
                return Err(error(span, "duplicate trait argument"));
            }
            // A supplied value is checked in the caller environment, against
            // the domain instantiated by inputs already evaluated in source
            // order. Do not reorder effectful argument computations.
            let domain =
                self.supplied_trait_domain(owner, domain, &supplied, &prefix, subject, value.span)?;
            let unresolved_domain = parameters.iter().enumerate().any(|(index, parameter)| {
                supplied[index].is_none() && domain.mentions_rigid(*parameter)
            });
            // Bare numeric literals have intrinsic conversion only: no call,
            // capture, or user literal protocol can execute here. Keep their
            // spelling until the domain is known instead of fixing Integer or
            // Float prematurely. Suffixed literals and general expressions
            // still follow source-order required invocation below.
            if unresolved_domain && is_numeric_literal(value) {
                pending_literals[slot] = Some(value);
                continue;
            }
            supplied[slot] = Some(if unresolved_domain {
                // Preserve source-order invocation when an earlier declaration
                // slot is supplied later (or by a default). The expression's
                // own checked type is retained for the final domain check.
                let symbolic_domain = if let H::ResolvedClauseParam(_, id) = value.kind {
                    self.trait_input_domain(DeclarationId {
                        package: semantic_package(&self.package_identities, id.pkg),
                        local: id.index,
                    })
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
        let saved_owner = self.signature_owner.replace(owner);
        let saved_context = self.context.clone();
        let saved_aliases = self.alias_substitution.clone();
        if let Some(environment) = defaults {
            self.alias_substitution.extend(environment.clone());
        }
        let saved_forbidden = self.forbidden_default_inputs.clone();
        let saved_bindings = self.normal_default_bindings.clone();
        let saved_value_bindings = self.alias_value_bindings.clone();
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
                        // the type equalities available when they were checked.
                        let definition_context =
                            std::mem::replace(&mut self.context, saved_context.clone());
                        let checked = self.require(actual, &domain, *value_span);
                        self.context = definition_context;
                        checked?;
                    }
                    value
                } else if let Some(literal) = pending_literals[slot] {
                    let domain = self.substituted_trait_domain(&domains[slot], literal.span)?;
                    let definition_context =
                        std::mem::replace(&mut self.context, saved_context.clone());
                    let checked = self.normal_value_argument(literal, &domain);
                    self.context = definition_context;
                    checked?
                } else {
                    let hir::clause::ClauseParamKind::Optional(_, _, value) = clauses[slot].kind
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
                        self.normal_value_argument(value, &domain)
                    })();
                    self.forbidden_default_inputs = saved_forbidden.clone();
                    self.active_type_defaults.remove(&(owner, slot));
                    result?
                };
                self.alias_substitution
                    .insert(parameters[slot], std::sync::Arc::new(value.clone()));
                arguments.push(value);
            }
            Ok(arguments)
        })();
        self.signature_owner = saved_owner;
        self.context = saved_context;
        self.alias_substitution = saved_aliases;
        self.forbidden_default_inputs = saved_forbidden;
        self.normal_default_bindings = saved_bindings;
        self.alias_value_bindings = saved_value_bindings;
        result
    }
}
