//! Demand-driven nominal layouts for required computations and forward uses.
use super::*;

pub(super) fn enum_leaves<'h>(
    variants: &'h [hir::item::Variant<'h>],
) -> Result<(Vec<(&'h hir::item::Variant<'h>, String)>, Vec<String>), Error> {
    let mut pending = variants
        .iter()
        .rev()
        .map(|variant| (variant, String::new(), 0usize))
        .collect::<Vec<_>>();
    let (mut leaves, mut groups, mut seen) = (Vec::new(), Vec::new(), BTreeSet::new());
    while let Some((variant, prefix, depth)) = pending.pop() {
        if depth >= 64 {
            return Err(error(variant.span, "enum group nesting limit exceeded"));
        }
        let name = format!("{prefix}{}", variant.ident.name);
        if !seen.insert(name.clone()) {
            return Err(error(variant.span, "duplicate enum member"));
        }
        if let hir::item::VariantKind::SubEnum(children) = &variant.kind {
            groups.push(name.clone());
            for child in children.iter().rev() {
                pending.push((child, format!("{name}."), depth + 1));
            }
        } else {
            leaves.push((variant, name));
        }
    }
    Ok((leaves, groups))
}

/// Explicit member arguments resolve in the caller; defaults use the selected parent.
pub(super) struct InputEnvironment<'a> {
    pub substitution: &'a BTreeMap<DeclarationId, std::sync::Arc<Term>>,
    pub bindings: &'a BTreeMap<DeclarationId, n::LocalId>,
    pub normal_bindings: &'a BTreeMap<DeclarationId, (n::LocalId, Term)>,
    pub context: &'a crate::ptc::Context,
}

impl Checker<'_, '_> {
    // Source clause arity excludes this lexical environment. Keep the same
    // suffix in declaration telescopes and every nominal value constructor.
    pub(super) fn nominal_captures(
        &self,
        owner: OwnerId,
        _span: Span,
    ) -> Result<Vec<DeclarationId>, Error> {
        Ok(self.nominal_capture_ids(owner))
    }

    pub(super) fn nominal_capture_ids(&self, owner: OwnerId) -> Vec<DeclarationId> {
        if !matches!(
            self.package.item(owner).map(|item| &item.kind),
            Some(
                ItemKind::Newtype(..)
                    | ItemKind::TypeAlias(..)
                    | ItemKind::Struct(_)
                    | ItemKind::Enum(_)
            )
        ) {
            return Vec::new();
        }
        let mut captures = Vec::new();
        let mut cursor = owner;
        let mut seen = BTreeSet::new();
        while let Some(parent) = self.self_containers.get(&cursor).copied() {
            if !seen.insert(parent) {
                break;
            }
            if matches!(
                self.package.item(parent).map(|item| &item.kind),
                Some(ItemKind::Trait(_))
            ) {
                let id = declaration(self.package, &self.package_identities, parent);
                if !captures.contains(&id) {
                    captures.push(id);
                }
            }
            for parameter in &self.type_parameters[&parent] {
                if !captures.contains(parameter) {
                    captures.push(*parameter);
                }
            }
            cursor = parent;
        }
        captures
    }

    pub(super) fn nominal_parameter_types(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<Vec<Term>, Error> {
        if let Some(parent) = self.self_containers.get(&owner).copied() {
            self.trait_domains(parent)?;
        }
        let mut domains = self.trait_domains(owner)?;
        if domains.is_empty() {
            domains.resize(self.type_parameters[&owner].len(), p(P::Type));
        }
        domains.extend(
            self.nominal_captures(owner, span)?
                .into_iter()
                .map(|id| self.trait_input_domain(id).unwrap_or_else(|| p(P::Type))),
        );
        Ok(domains)
    }

    pub(super) fn nominal_capture_expressions(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<Vec<n::Expr>, Error> {
        self.nominal_captures(owner, span)?
            .into_iter()
            .map(|parameter| self.captured_input_expression(parameter, span))
            .collect()
    }

    pub(super) fn captured_input_expression(
        &mut self,
        parameter: DeclarationId,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let domain = self
            .trait_input_domain(parameter)
            .unwrap_or_else(|| p(P::Type));
        if domain != p(P::Type) {
            let ty = self.substituted_trait_domain(&domain, span)?;
            if let Some(binding) =
                self.alias_value_bindings
                    .get(&parameter)
                    .copied()
                    .or_else(|| {
                        self.normal_default_bindings
                            .get(&parameter)
                            .map(|(id, _)| *id)
                    })
            {
                return Ok(n::Expr {
                    source: source(span),
                    ty,
                    kind: K::Local(binding),
                });
            }
            let value = self
                .alias_substitution
                .get(&parameter)
                .cloned()
                .ok_or_else(|| {
                    error(
                        span,
                        "captured value input is not yet available in this definition",
                    )
                })?;
            let value = self.copy_alias_type(&value, span)?;
            // Captures are already checked inputs, not computations to invoke
            // while the enclosing generic definition is still symbolic.
            return Ok(n::Expr {
                source: source(span),
                ty,
                kind: K::ComptimeValue(value),
            });
        }
        let kind = if let Some(binding) = self.alias_value_bindings.get(&parameter) {
            K::Local(*binding)
        } else {
            let ty = if let Some(value) = self.alias_substitution.get(&parameter).cloned() {
                self.copy_alias_type(&value, span)?
            } else {
                Term::Rigid {
                    declaration: parameter,
                    sort: Sort::Type,
                }
            };
            K::TypeValue(ty)
        };
        Ok(n::Expr {
            source: source(span),
            ty: p(P::Type),
            kind,
        })
    }

    pub(super) fn nominal_capture_arguments(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<Vec<Term>, Error> {
        self.nominal_captures(owner, span)?
            .into_iter()
            .map(|parameter| {
                if self.alias_value_bindings.contains_key(&parameter) {
                    return Err(error(
                        span,
                        "captured input is not yet available in this definition",
                    ));
                }
                if let Some(value) = self.alias_substitution.get(&parameter).cloned() {
                    self.copy_alias_type(&value, span)
                } else {
                    Ok(Term::Rigid {
                        declaration: parameter,
                        sort: self.input_binders(&[parameter])[0].sort,
                    })
                }
            })
            .collect()
    }

    /// Nominal telescopes reuse contextual normal inputs in source order; the
    /// nominal head stays distinct from an opaque normal-form computation.
    pub(super) fn contextual_nominal_expression(
        &mut self,
        owner: OwnerId,
        args: &[Arg<'_>],
        span: Span,
        expected: Option<&Term>,
    ) -> Result<Option<n::Expr>, Error> {
        self.contextual_nominal_expression_in(owner, args, span, expected, None)
    }

    pub(super) fn contextual_nominal_expression_in(
        &mut self,
        owner: OwnerId,
        args: &[Arg<'_>],
        span: Span,
        expected: Option<&Term>,
        caller: Option<&InputEnvironment<'_>>,
    ) -> Result<Option<n::Expr>, Error> {
        if !matches!(
            self.package.item(owner).map(|item| &item.kind),
            Some(
                ItemKind::Struct(_)
                    | ItemKind::Enum(_)
                    | ItemKind::Newtype(..)
                    | ItemKind::TypeAlias(..)
            )
        ) {
            return Ok(None);
        }
        let alias = match &self.package.item(owner).unwrap().kind {
            ItemKind::TypeAlias(value, ..) => Some(*value),
            _ => None,
        };
        let domains = self.trait_domains(owner)?;
        let captures = self.nominal_captures(owner, span)?;
        let parameters = self.type_parameters[&owner].clone();
        if !domains.iter().any(|domain| {
            parameters
                .iter()
                .chain(&captures)
                .any(|id| domain.mentions_rigid(*id))
                || (alias.is_some() && domain != &p(P::Type))
        }) {
            return Ok(None);
        }
        if let Some(expanded) = normal::literal_spreads(args) {
            return self.contextual_nominal_expression_in(owner, &expanded, span, expected, caller);
        }
        let mut capture_binders = Vec::new();
        let mut capture_values = Vec::new();
        for binder in self.input_binders(&captures) {
            if let Some(value) = self.alias_substitution.get(&binder.declaration).cloned() {
                capture_values.push(self.copy_alias_type(&value, span)?);
                capture_binders.push(binder);
            }
        }
        let domains = domains
            .iter()
            .map(|domain| {
                self.instantiate_trait_domain(domain, &capture_binders, &capture_values, span)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let saved_locals = self.locals.clone();
        let saved_normal = self.normal_default_bindings.clone();
        let saved_aliases = self.alias_substitution.clone();
        let saved_values = self.alias_value_bindings.clone();
        let saved_owner = self.signature_owner;
        let saved_context = self.context.clone();
        let saved_mode = self.in_required_comptime;
        if let Some(caller) = caller {
            self.alias_substitution = caller.substitution.clone();
            self.alias_value_bindings = caller.bindings.clone();
            self.normal_default_bindings = caller.normal_bindings.clone();
            self.context = caller.context.clone();
        }
        let result = (|| {
            let clauses = self.type_clause_params(owner);
            let mut positional = 0;
            let mut slots = Vec::new();
            let mut supplied = BTreeSet::new();
            let mut inputs = vec![None; parameters.len()];
            let mut prepared_inputs = vec![None; parameters.len()];
            let mut preparations = Vec::new();
            let positional_domains = clauses
                .iter()
                .zip(&domains)
                .filter_map(|(clause, domain)| {
                    (!matches!(clause.kind, hir::clause::ClauseParamKind::Optional(..)))
                        .then_some(domain.clone())
                })
                .collect::<Vec<_>>();
            for (index, arg) in args.iter().enumerate() {
                if let Arg::Expand(input) = arg {
                    let packet = self.expr(input, None)?;
                    let fields = self.prepared_packet_fields(
                        &packet,
                        args,
                        index,
                        &positional_domains,
                        input.span,
                    )?;
                    let before = slots.len();
                    let mut pattern = Vec::new();
                    for field in fields {
                        let slot = Self::type_input_slot(&clauses, None, &mut positional, span)?;
                        if slot >= parameters.len() || !supplied.insert(slot) {
                            return Err(error(span, "duplicate or excess type argument"));
                        }
                        let binding = self.spread_binding(input.span)?;
                        self.locals.insert(binding, field.clone());
                        pattern.push(n::Pattern::Binding {
                            binding,
                            inner: Box::new(n::Pattern::Wild),
                        });
                        prepared_inputs[slot] = Some(n::Expr {
                            source: packet.source,
                            ty: field,
                            kind: K::Local(binding),
                        });
                        slots.push(slot);
                    }
                    preparations.push(n::NormalFormPreparation {
                        before,
                        expression: n::Expr {
                            source: packet.source,
                            ty: if is_never(&packet.ty) {
                                p(P::Never)
                            } else {
                                p(P::Void)
                            },
                            kind: K::LetPattern {
                                pattern: n::Pattern::Tuple(pattern),
                                value: Box::new(packet),
                                guard: None,
                                otherwise: None,
                            },
                        },
                    });
                    continue;
                }
                let (name, input) = match arg {
                    Arg::Named(name, input) => (Some(*name), *input),
                    Arg::Positional(input) => (None, *input),
                    _ => unreachable!(),
                };
                let slot = Self::type_input_slot(&clauses, name, &mut positional, span)?;
                if slot >= parameters.len() || !supplied.insert(slot) {
                    return Err(error(span, "duplicate or excess type argument"));
                }
                slots.push(slot);
                inputs[slot] = Some(input);
            }
            let has_defaults = slots.len() != parameters.len();
            for slot in 0..parameters.len() {
                if !supplied.contains(&slot) {
                    slots.push(slot);
                }
            }
            let mut bindings = if has_defaults || alias.is_some() {
                Some(
                    (0..parameters.len())
                        .map(|_| self.spread_binding(span))
                        .collect::<Result<Vec<_>, _>>()?,
                )
            } else {
                None
            };
            let binders = self.declaration_binders(owner);
            let mut known_parameters = Vec::new();
            let mut known_values = Vec::new();
            let mut arguments = (0..parameters.len()).map(|_| None).collect::<Vec<_>>();
            let mut terms = binders
                .iter()
                .map(|binder| Term::Rigid {
                    declaration: binder.declaration,
                    sort: binder.sort,
                })
                .collect::<Vec<_>>();
            let mut unresolved = parameters.iter().copied().collect::<BTreeSet<_>>();
            let mut deferred = BTreeSet::new();
            for &slot in &slots {
                let explicit = supplied.contains(&slot);
                let input = if prepared_inputs[slot].is_some() {
                    None
                } else if let Some(input) = inputs[slot] {
                    Some(input)
                } else {
                    let hir::clause::ClauseParamKind::Optional(_, _, default) = clauses[slot].kind
                    else {
                        return Err(error(span, "type argument count mismatch"));
                    };
                    if caller.is_some() {
                        self.alias_substitution = saved_aliases.clone();
                        self.alias_value_bindings = saved_values.clone();
                        self.normal_default_bindings = saved_normal.clone();
                        self.context = saved_context.clone();
                    }
                    self.signature_owner = Some(owner);
                    self.context = self.binder_context(owner);
                    self.in_required_comptime = true;
                    // Only preceding declaration inputs enter a default's lexical scope.
                    for previous in 0..slot {
                        let id = parameters[previous];
                        self.alias_substitution.remove(&id);
                        self.alias_value_bindings.remove(&id);
                        self.normal_default_bindings.remove(&id);
                        if domains[previous] == p(P::Type) {
                            if !unresolved.contains(&id) {
                                let known = self.copy_alias_type(&terms[previous], span)?;
                                self.alias_substitution
                                    .insert(id, std::sync::Arc::new(known));
                            } else {
                                self.alias_value_bindings
                                    .insert(id, bindings.as_ref().unwrap()[previous]);
                            }
                        } else {
                            // Preserve known value indices while elaborating later
                            // default types. The local still owns evaluation of
                            // the input; this substitution only records identity.
                            if !unresolved.contains(&id) {
                                let known = self.copy_alias_type(&terms[previous], span)?;
                                self.alias_substitution
                                    .insert(id, std::sync::Arc::new(known));
                            }
                            let domain = self.instantiate_trait_domain(
                                &domains[previous],
                                &known_parameters,
                                &known_values,
                                span,
                            )?;
                            self.normal_default_bindings
                                .insert(id, (bindings.as_ref().unwrap()[previous], domain));
                        }
                    }
                    Some(default)
                };
                let domain = self.instantiate_trait_domain(
                    &domains[slot],
                    &known_parameters,
                    &known_values,
                    span,
                )?;
                let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let before = fuel;
                let dependent =
                    nir::type_computations::has_rigid_parameters_bounded(&domain, &mut fuel);
                self.required_steps -= (before - fuel) as u64;
                let dependent = dependent.map_err(|e| {
                    error(span, format!("nominal input domain inspection failed: {e}"))
                })?;
                if !explicit
                    && (self.active_type_defaults.len() >= 64
                        || !self.active_type_defaults.insert((owner, slot)))
                {
                    return Err(error(span, "cyclic or excessively deep type default"));
                }
                let checked = if let Some(value) = prepared_inputs[slot].take() {
                    if !dependent {
                        self.require_expression(&value.ty, &domain, span)?;
                    }
                    Ok(n::NormalFormArgument::Value(value))
                } else if dependent {
                    self.dependent_normal_argument(input.unwrap(), 0)
                } else {
                    self.expr(input.unwrap(), Some(&domain))
                        .map(n::NormalFormArgument::Value)
                };
                if !explicit {
                    self.active_type_defaults.remove(&(owner, slot));
                }
                let argument = checked?;
                let mut budget = comptime_runtime::Budget {
                    steps: self.required_steps as usize,
                    max_limbs: 4096,
                };
                let before = budget.steps;
                let known = argument
                    .expression()
                    .and_then(|value| self.known_normal_input(value, 0, &mut budget));
                self.required_steps -= (before - budget.steps) as u64;
                if let Some(bindings) = &bindings {
                    self.locals.insert(
                        bindings[slot],
                        argument
                            .expression()
                            .map_or_else(|| domain.clone(), |value| value.ty.clone()),
                    );
                }
                let term = if let Some(known) = known {
                    unresolved.remove(&parameters[slot]);
                    known_parameters.push(binders[slot]);
                    known_values.push(self.copy_alias_type(&known, span)?);
                    known
                } else {
                    deferred.insert(parameters[slot]);
                    Term::Rigid {
                        declaration: parameters[slot],
                        sort: binders[slot].sort,
                    }
                };
                terms[slot] = term;
                arguments[slot] = Some(argument);
            }
            self.locals = saved_locals.clone();
            self.normal_default_bindings = saved_normal.clone();
            self.alias_substitution = saved_aliases.clone();
            self.alias_value_bindings = saved_values.clone();
            self.signature_owner = saved_owner;
            self.context = saved_context.clone();
            self.in_required_comptime = saved_mode;
            if alias.is_some() {
                self.ensure_alias_formation_contract(owner, span)?;
            }
            self.check_formation_application(
                owner,
                FormationInputs {
                    terms,
                    deferred,
                    environment: BTreeMap::new(),
                },
                span,
            )?;
            let mut arguments = arguments
                .into_iter()
                .map(Option::unwrap)
                .collect::<Vec<_>>();
            arguments.extend(
                self.nominal_capture_expressions(owner, span)?
                    .into_iter()
                    .map(n::NormalFormArgument::Value),
            );
            if let Some(bindings) = &mut bindings {
                for _ in parameters.len()..arguments.len() {
                    bindings.push(self.spread_binding(span)?);
                }
            }
            // Captures have already been evaluated by the enclosing instance.
            // Bind them before checking defaults whose domains mention them;
            // stored argument order still keeps own inputs before captures.
            let capture_count = arguments.len() - parameters.len();
            for preparation in &mut preparations {
                preparation.before += capture_count;
            }
            slots = (parameters.len()..arguments.len()).chain(slots).collect();
            let mut alias_body = None;
            if let Some(alias) = alias {
                let bindings = bindings.as_ref().unwrap();
                let own_inputs = parameters
                    .iter()
                    .enumerate()
                    .map(|(index, _)| {
                        Ok(n::Expr {
                            source: source(span),
                            ty: self.substituted_trait_domain(&domains[index], span)?,
                            kind: K::Local(bindings[index]),
                        })
                    })
                    .collect::<Result<Vec<_>, Error>>()?;
                alias_body = Some(Box::new(self.alias_value_expression_with_guard(
                    owner, alias, own_inputs, span, false,
                )?));
            } else {
                self.ensure_nominal_expression(owner, span)?;
            }
            let ty = if arguments.iter().any(n::NormalFormArgument::is_diverging)
                || preparations.iter().any(|p| is_never(&p.expression.ty))
                || alias_body.as_ref().is_some_and(|body| is_never(&body.ty))
            {
                p(P::Never)
            } else {
                p(P::Type)
            };
            if let Some(expected) = expected {
                self.require_expression(&ty, expected, span)?;
            }
            let kind = if let Some(body) = alias_body {
                let contract_parameters = self.ensure_alias_formation_contract(owner, span)?;
                let mut all_parameters = parameters.clone();
                all_parameters.extend(self.nominal_captures(owner, span)?);
                let source_indices = contract_parameters
                    .iter()
                    .map(|id| {
                        all_parameters
                            .iter()
                            .position(|source| source == id)
                            .ok_or_else(|| error(span, "alias input capture is unavailable"))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let order = slots
                    .into_iter()
                    .filter_map(|source| source_indices.iter().position(|index| *index == source))
                    .collect();
                let original_bindings = bindings.unwrap();
                let bindings = source_indices
                    .iter()
                    .map(|index| original_bindings[*index])
                    .collect();
                let mut original = arguments.into_iter().map(Some).collect::<Vec<_>>();
                let arguments = source_indices
                    .into_iter()
                    .map(|index| original[index].take().unwrap())
                    .collect();
                K::CheckTypeAlias {
                    declaration: declaration(self.package, &self.package_identities, owner),
                    preparations,
                    bindings: Some(bindings),
                    order: Some(order),
                    arguments,
                    body: Some(body),
                }
            } else {
                K::NominalType {
                    preparations,
                    bindings,
                    order: Some(slots),
                    declaration: declaration(self.package, &self.package_identities, owner),
                    arguments,
                }
            };
            Ok(Some(n::Expr {
                source: source(span),
                ty,
                kind,
            }))
        })();
        self.locals = saved_locals;
        self.normal_default_bindings = saved_normal;
        self.alias_substitution = saved_aliases;
        self.alias_value_bindings = saved_values;
        self.signature_owner = saved_owner;
        self.context = saved_context;
        self.in_required_comptime = saved_mode;
        result
    }

    /// Dynamic nominal construction still needs its declaration's reachable
    /// descriptors. Actual arguments remain evaluated by the generated NIR.
    pub(super) fn ensure_nominal_expression(
        &mut self,
        owner: OwnerId,
        span: Span,
    ) -> Result<(), Error> {
        if self.in_required_comptime {
            self.ensure_invocation_type(
                &Term::Nominal {
                    declaration: declaration(self.package, &self.package_identities, owner),
                    arguments: vec![],
                },
                span,
            )
        } else {
            self.ensure_nominal(owner, span)
        }
    }

    /// Execution can inspect nested types before the global declaration pass.
    /// Close only the reachable descriptor graph, retaining nominal identities
    /// and stopping at already visited declarations, including pointer cycles.
    pub(super) fn ensure_invocation_type(&mut self, ty: &Term, span: Span) -> Result<(), Error> {
        let mut pending = BTreeSet::new();
        let mut seen = BTreeSet::new();
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let result =
            nir::type_computations::nominal_declarations_bounded(ty, &mut fuel, &mut pending);
        self.required_steps -= (before - fuel) as u64;
        result.map_err(|failure| error(span, failure.to_string()))?;
        while let Some(id) = pending.pop_first() {
            if !seen.insert(id) {
                continue;
            }
            self.ensure_nominal_type(
                &Term::Nominal {
                    declaration: id,
                    arguments: vec![],
                },
                span,
            )?;
            let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
            let before = fuel;
            let result = (|| {
                let mut collect = |ty: &Term| {
                    nir::type_computations::nominal_declarations_bounded(
                        ty,
                        &mut fuel,
                        &mut pending,
                    )
                };
                if let Some(definition) = self.structs.get(&id) {
                    for ty in &definition.parameter_types {
                        collect(ty)?;
                    }
                    for (_, ty) in &definition.fields {
                        collect(ty)?;
                    }
                } else if let Some(definition) = self.enums.get(&id) {
                    for ty in &definition.parameter_types {
                        collect(ty)?;
                    }
                    for variant in &definition.variants {
                        for ty in &variant.fields {
                            collect(ty)?;
                        }
                    }
                } else if let Some(definition) = self.newtypes.get(&id) {
                    for ty in &definition.parameter_types {
                        collect(ty)?;
                    }
                    collect(&definition.representation)?;
                }
                Ok::<_, nir::type_computations::Error>(())
            })();
            self.required_steps -= (before - fuel) as u64;
            result.map_err(|failure| error(span, failure.to_string()))?;
        }
        Ok(())
    }

    pub(super) fn ensure_nominal_type(&mut self, ty: &Term, span: Span) -> Result<(), Error> {
        let Term::Nominal {
            declaration: id, ..
        } = ty
        else {
            return Ok(());
        };
        if self.structs.contains_key(id)
            || self.enums.contains_key(id)
            || self.newtypes.contains_key(id)
        {
            return Ok(());
        }
        let owner = self
            .package
            .owners()
            .find_map(|(owner, _)| {
                (matches_declaration(self.package, &self.package_identities, owner, *id))
                    .then_some(owner)
            })
            .ok_or_else(|| error(span, "nominal declaration is missing"))?;
        self.ensure_nominal(owner, span)
    }
    pub(super) fn ensure_nominal(&mut self, owner: OwnerId, span: Span) -> Result<(), Error> {
        let id = declaration(self.package, &self.package_identities, owner);
        if self.structs.contains_key(&id)
            || self.enums.contains_key(&id)
            || self.newtypes.contains_key(&id)
        {
            return Ok(());
        }
        if self.active_nominals.len() >= 64 || !self.active_nominals.insert(owner) {
            return Err(error(
                span,
                "cyclic or excessively deep nominal representation dependency",
            ));
        }
        let saved_owner = self.signature_owner.replace(owner);
        let saved_context = self.context.clone();
        let saved_locals = std::mem::take(&mut self.locals);
        // Layouts describe the declaration telescope, never the first caller's
        // concrete environment. Instantiation happens at each nominal use.
        let saved_substitution = std::mem::take(&mut self.alias_substitution);
        let saved_bindings = std::mem::take(&mut self.alias_value_bindings);
        self.context = self.binder_context(owner);
        let result = (|| {
            let item = self.package.item(owner).unwrap();
            match &item.kind {
                ItemKind::Newtype(value, ..) => {
                    let representation = self.type_expr(value)?;
                    // Direct representation cycles have no finite underlying value.
                    // Keep nominal identities intact while checking this edge.
                    if let Term::Nominal {
                        declaration: target,
                        ..
                    } = &representation
                    {
                        if let Some(target_owner) =
                            self.package.owners().find_map(|(owner, info)| {
                                (matches_declaration(
                                    self.package,
                                    &self.package_identities,
                                    owner,
                                    *target,
                                ) && matches!(info.node.expect_item().kind, ItemKind::Newtype(..)))
                                .then_some(owner)
                            })
                        {
                            self.ensure_nominal(target_owner, value.span)?;
                        }
                    }
                    let mut type_parameters = self.type_parameters[&owner].clone();
                    let captures = self.nominal_captures(owner, span)?;
                    let capture_count = captures.len();
                    type_parameters.extend(captures);
                    let parameter_types = self.nominal_parameter_types(owner, span)?;
                    self.newtypes.insert(
                        id,
                        nir::newtypes::Newtype {
                            capture_count,
                            parameter_types,
                            declaration: id,
                            name: item.ident.name.to_string(),
                            type_parameters,
                            requirements: self
                                .requirements
                                .get(&owner)
                                .cloned()
                                .unwrap_or_default(),
                            capabilities: self
                                .capabilities
                                .get(&owner)
                                .cloned()
                                .unwrap_or_default(),
                            representation,
                        },
                    );
                }
                ItemKind::Enum(def) => {
                    let (leaves, groups) = enum_leaves(def.variants)?;
                    let encoded = self.enum_encoding(def, &leaves, item.span)?;
                    let mut names = BTreeSet::new();
                    for nested in &def.nested_items {
                        if let Some(item) = self.package.item(*nested) {
                            names.insert(item.ident.name.to_string());
                        }
                    }
                    for group in &groups {
                        if !names.insert(group.clone()) {
                            return Err(error(item.span, "duplicate enum member"));
                        }
                    }
                    let mut variants = Vec::new();
                    for (variant_index, (variant, full_name)) in leaves.iter().enumerate() {
                        let (shape, fields) = match &variant.kind {
                            hir::item::VariantKind::Pattern(_)
                            | hir::item::VariantKind::Const(_)
                                if encoded.is_some() =>
                            {
                                let encoding = encoded.as_ref().unwrap();
                                let fields = encoding.payloads[variant_index].clone();
                                let names = encoding
                                    .bits
                                    .as_ref()
                                    .map(|bits| {
                                        bits.variants()[variant_index]
                                            .fields()
                                            .iter()
                                            .map(|field| field.name.clone())
                                            .collect::<Vec<_>>()
                                    })
                                    .unwrap_or_default();
                                (
                                    if names.is_empty() {
                                        n::VariantShape::Unit
                                    } else {
                                        n::VariantShape::Named(names)
                                    },
                                    fields,
                                )
                            }
                            hir::item::VariantKind::Unit => (n::VariantShape::Unit, Vec::new()),
                            hir::item::VariantKind::Tuple(types) => (
                                n::VariantShape::Tuple,
                                types
                                    .iter()
                                    .map(|ty| self.type_expr(ty))
                                    .collect::<Result<Vec<_>, _>>()?,
                            ),
                            hir::item::VariantKind::Struct(fields) => {
                                let mut names = BTreeSet::new();
                                let mut types = Vec::new();
                                let mut ordered = Vec::new();
                                for field in *fields {
                                    let name = field.ident.name.to_string();
                                    if !names.insert(name.clone()) {
                                        return Err(error(
                                            field.span,
                                            "duplicate enum payload field",
                                        ));
                                    }
                                    ordered.push(name);
                                    types.push(self.type_expr(field.ty)?);
                                }
                                (n::VariantShape::Named(ordered), types)
                            }
                            _ => {
                                return Err(error(
                                    variant.span,
                                    "enum representation elaboration is not implemented",
                                ));
                            }
                        };
                        let name = full_name.clone();
                        if !names.insert(name.clone()) {
                            return Err(error(variant.span, "duplicate enum member"));
                        }
                        variants.push(n::Variant {
                            name,
                            shape,
                            fields,
                        });
                    }
                    let (encoding, symbol_encoding) = encoded
                        .map(|encoding| (encoding.bits, encoding.symbols))
                        .unwrap_or_default();
                    let parameter_types = self.nominal_parameter_types(owner, span)?;
                    self.enums.insert(
                        declaration(self.package, &self.package_identities, owner),
                        n::Enum {
                            is_intrinsic: def.is_intrinsic,
                            parameter_types,
                            capabilities: self
                                .capabilities
                                .get(&owner)
                                .cloned()
                                .unwrap_or_default(),
                            requirements: self
                                .requirements
                                .get(&owner)
                                .cloned()
                                .unwrap_or_default(),
                            type_parameters: self.type_parameters[&owner]
                                .iter()
                                .copied()
                                .chain(self.nominal_captures(owner, span)?)
                                .collect(),
                            declaration: declaration(self.package, &self.package_identities, owner),
                            name: item.ident.name.to_string(),
                            variants,
                            groups,
                            encoding,
                            symbol_encoding,
                        },
                    );
                    self.enum_sources
                        .insert(id, leaves.into_iter().map(|(variant, _)| variant).collect());
                }
                ItemKind::Struct(def) => {
                    let mut names = BTreeSet::new();
                    let mut fields = Vec::new();
                    for field in def.fields {
                        let name = field.ident.name.as_str().to_string();
                        if !names.insert(name.clone()) {
                            return Err(error(field.span, "duplicate struct field"));
                        }
                        fields.push((name, self.type_expr(field.ty)?));
                    }
                    let parameter_types = self.nominal_parameter_types(owner, span)?;
                    self.structs.insert(
                        declaration(self.package, &self.package_identities, owner),
                        n::Struct {
                            parameter_types,
                            capabilities: self
                                .capabilities
                                .get(&owner)
                                .cloned()
                                .unwrap_or_default(),
                            requirements: self
                                .requirements
                                .get(&owner)
                                .cloned()
                                .unwrap_or_default(),
                            type_parameters: self.type_parameters[&owner]
                                .iter()
                                .copied()
                                .chain(self.nominal_captures(owner, span)?)
                                .collect(),
                            declaration: declaration(self.package, &self.package_identities, owner),
                            name: item.ident.name.as_str().into(),
                            fields,
                        },
                    );
                }
                _ => return Err(error(span, "declaration is not a struct or enum")),
            }
            Ok(())
        })();
        self.signature_owner = saved_owner;
        self.context = saved_context;
        self.locals = saved_locals;
        self.alias_substitution = saved_substitution;
        self.alias_value_bindings = saved_bindings;
        self.active_nominals.remove(&owner);
        if result.is_ok() {
            self.implementation_index.sync_nominal_layouts(
                self.structs.values(),
                self.enums.values(),
                self.newtypes.values(),
            );
        }
        result
    }
}
