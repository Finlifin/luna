//! Type-valued clause routing and default checking for nominal declarations.
use super::*;
use comptime_interpretor::Value;

impl<'h> Checker<'_, 'h> {
    fn type_packet_contexts(&mut self, args: &[Arg<'_>], count: usize, span: Span)
        -> Result<Vec<Option<Term>>, Error> {
        if !args.iter().any(|arg| matches!(arg, Arg::Expand(_))) { return Ok(vec![None; args.len()]); }
        let empty = args.iter().enumerate().filter_map(|(index, arg)|
            matches!(arg, Arg::Expand(value) if matches!(value.kind, H::Unit)).then_some((index, 0)))
            .collect();
        self.spread_packet_layout_with_widths(args, &vec![p(P::Type); count], span, &empty)?
            .into_iter().map(|fields| fields.map(|fields| self.tuple_value_type(fields, span)).transpose()).collect()
    }

    fn type_input_slot(
        clauses: &[hir::clause::ClauseParam<'_>], name: Option<hir::common::Ident>,
        positional: &mut usize, span: Span,
    ) -> Result<usize, Error> {
        if let Some(name) = name {
            return clauses.iter().position(|clause| clause.name.name == name.name
                && matches!(clause.kind, hir::clause::ClauseParamKind::Optional(..)))
                .ok_or_else(|| error(span, "unknown named type argument"));
        }
        while *positional < clauses.len()
            && matches!(clauses[*positional].kind, hir::clause::ClauseParamKind::Optional(..)) {
            *positional += 1;
        }
        let slot = *positional;
        *positional += 1;
        Ok(slot)
    }

    /// Concrete declaration arguments are required computations. A tuple
    /// producer executes once, including an empty packet; its fields must be
    /// Type values. Literal packets keep symbolic declaration binders intact.
    pub(super) fn expanded_type_inputs(
        &mut self, args: &[Arg<'_>], count: usize, span: Span,
    ) -> Result<Vec<(Option<hir::common::Ident>, Term)>, Error> {
        let mut inputs = Vec::new();
        let contexts = self.type_packet_contexts(args, count, span)?;
        for (index, arg) in args.iter().enumerate() {
            match arg {
                Arg::Positional(value) => inputs.push((None, self.type_expr(value)?)),
                Arg::Named(name, value) => inputs.push((Some(*name), self.type_expr(value)?)),
                Arg::Expand(value) => {
                    if matches!(value.kind, H::Unit) { continue; }
                    if let H::Tuple(fields) = value.kind {
                        for field in fields { inputs.push((None, self.type_expr(field)?)); }
                    } else {
                        let (packet, ty) = self.required_typed_expression(value, contexts[index].as_ref())?;
                        let Term::Tuple(fields) = self.structural_type(&ty, value.span)? else {
                            return Err(error(value.span, "type argument spread requires a tuple of Type values"));
                        };
                        for field in &fields { self.require(field, &p(P::Type), value.span)?; }
                        let Value::Tuple(values) = packet else {
                            return Err(error(value.span, "type argument spread did not produce a tuple"));
                        };
                        for value in values {
                            let ty = match value {
                                Value::Type(ty) => ty,
                                Value::TypeParameter(declaration) => Term::Rigid { declaration, sort: Sort::Type },
                                _ => return Err(error(span, "type argument spread element must be a Type value")),
                            };
                            inputs.push((None, ty));
                        }
                    }
                }
                _ => return Err(error(span, "unsupported type argument routing protocol")),
            }
        }
        Ok(inputs)
    }

    /// Invocation-time Type computations retain packet preparation in NIR.
    /// Unpacking once also preserves empty producers and early control exits.
    fn prepared_type_inputs(
        &mut self, args: &[Arg<'_>], count: usize, statements: &mut Vec<n::Expr>, span: Span,
    ) -> Result<Vec<(Option<hir::common::Ident>, n::Expr)>, Error> {
        let mut inputs = Vec::new();
        let contexts = self.type_packet_contexts(args, count, span)?;
        for (index, arg) in args.iter().enumerate() {
            match arg {
                Arg::Positional(value) | Arg::Named(_, value) => {
                    let name = match arg { Arg::Named(name, _) => Some(*name), _ => None };
                    let value = self.expr(value, Some(&p(P::Type)))?;
                    inputs.push((name, self.bind_type_input(value, statements, span)?));
                }
                Arg::Expand(value) => {
                    if matches!(value.kind, H::Unit) { continue; }
                    if let H::Tuple(fields) = value.kind {
                        for field in fields {
                            let value = self.expr(field, Some(&p(P::Type)))?;
                            inputs.push((None, self.bind_type_input(value, statements, span)?));
                        }
                        continue;
                    }
                    let value = self.expr(value, contexts[index].as_ref())?;
                    let Term::Tuple(fields) = self.structural_type(&value.ty, span)? else {
                        return Err(error(span, "type argument spread requires a tuple of Type values"));
                    };
                    let mut pattern = Vec::new();
                    for field in fields {
                        self.require(&field, &p(P::Type), span)?;
                        let binding = self.spread_binding(span)?;
                        self.locals.insert(binding, p(P::Type));
                        pattern.push(n::Pattern::Binding { binding, inner: Box::new(n::Pattern::Wild) });
                        inputs.push((None, n::Expr { source: value.source, ty: p(P::Type), kind: K::Local(binding) }));
                    }
                    statements.push(n::Expr {
                        source: n::SourceRange { start: value.source.end, end: value.source.end },
                        ty: p(P::Void),
                        kind: K::LetPattern { pattern: n::Pattern::Tuple(pattern), value: Box::new(value), guard: None, otherwise: None },
                    });
                }
                _ => return Err(error(span, "unsupported type argument routing protocol")),
            }
        }
        Ok(inputs)
    }

    pub(super) fn type_clause_params(&self, owner: OwnerId) -> Vec<hir::clause::ClauseParam<'h>> {
        match &self.package.item(owner).unwrap().kind {
            ItemKind::Trait(def) => def.clause_params.to_vec(),
            ItemKind::Struct(def) => def.clause_params.to_vec(),
            ItemKind::Enum(def) => def.clause_params.to_vec(),
            ItemKind::TypeAlias(_, clauses, _) => clauses.to_vec(),
            ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig) => sig.clause_params.to_vec(),
            _ => Vec::new(),
        }
    }

    pub(super) fn has_optional_type_clauses(&self, owner: OwnerId) -> bool {
        self.type_clause_params(owner)
            .iter()
            .any(|clause| matches!(clause.kind, hir::clause::ClauseParamKind::Optional(..)))
    }

    /// Build a Type-valued application while retaining computations and their
    /// source order. Fresh bindings let defaults reuse preceding inputs once.
    pub(super) fn defaulted_type_expression(
        &mut self,
        owner: OwnerId,
        args: &[Arg<'_>],
        span: Span,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let clauses = self.type_clause_params(owner);
        let parameters = self.type_parameters[&owner].clone();
        let saved_locals = self.locals.clone();
        let saved_substitution = self.alias_substitution.clone();
        let saved_bindings = self.alias_value_bindings.clone();
        let saved_owner = self.signature_owner;
        let saved_context = self.context.clone();
        let saved_mode = self.in_required_comptime;
        let result = (|| {
            let mut values: Vec<Option<n::Expr>> = vec![None; clauses.len()];
            let mut statements = Vec::new();
            let mut positional = 0;
            let count = clauses.iter().filter(|clause| !matches!(clause.kind, hir::clause::ClauseParamKind::Optional(..))).count();
            for (name, value) in self.prepared_type_inputs(args, count, &mut statements, span)? {
                let slot = Self::type_input_slot(&clauses, name, &mut positional, span)?;
                let cell = values.get_mut(slot)
                    .ok_or_else(|| error(span, "type argument count mismatch"))?;
                if cell.replace(value).is_some() {
                    return Err(error(span, "duplicate type argument"));
                }
            }
            self.signature_owner = Some(owner);
            self.context = self.binder_context(owner);
            self.in_required_comptime = true;
            let mut arguments = Vec::new();
            for (slot, value) in values.into_iter().enumerate() {
                let value = if let Some(value) = value {
                    value
                } else {
                    let hir::clause::ClauseParamKind::Optional(_, _, default) = clauses[slot].kind
                    else {
                        return Err(error(span, "type argument count mismatch"));
                    };
                    if self.active_type_defaults.len() >= 64
                        || !self.active_type_defaults.insert((owner, slot))
                    {
                        return Err(error(span, "cyclic or excessively deep type default"));
                    }
                    let value = self.expr(default, Some(&p(P::Type)));
                    self.active_type_defaults.remove(&(owner, slot));
                    self.bind_type_input(value?, &mut statements, span)?
                };
                match &value.kind {
                    K::TypeValue(ty) => {
                        self.alias_substitution
                            .insert(parameters[slot], std::sync::Arc::new(ty.clone()));
                        self.alias_value_bindings.remove(&parameters[slot]);
                    }
                    K::Local(binding) => {
                        self.alias_value_bindings.insert(parameters[slot], *binding);
                        self.alias_substitution.remove(&parameters[slot]);
                    }
                    _ => unreachable!(),
                }
                arguments.push(value);
            }
            self.signature_owner = saved_owner;
            self.context = saved_context.clone();
            self.in_required_comptime = saved_mode;
            self.alias_substitution = saved_substitution.clone();
            self.alias_value_bindings = saved_bindings.clone();
            let body =
                if let ItemKind::TypeAlias(value, ..) = &self.package.item(owner).unwrap().kind {
                    self.alias_value_expression(owner, value, arguments, span)?
                } else {
                    let mut terms = Vec::new();
                    let mut deferred = BTreeSet::new();
                    for (value, parameter) in arguments.iter().zip(&parameters) {
                        if let K::TypeValue(ty) = &value.kind {
                            terms.push(ty.clone());
                        } else {
                            deferred.insert(*parameter);
                            terms.push(Term::Rigid {
                                declaration: *parameter,
                                sort: Sort::Type,
                            });
                        }
                    }
                    self.check_formation_application(
                        owner,
                        FormationInputs {
                            terms: terms.clone(),
                            deferred: deferred.clone(),
                        },
                        span,
                    )?;
                    let normal_head = self.normal_parameters.contains_key(&owner);
                    let kind = if normal_head {
                        K::NormalFormType {
                            plan: None,
                            head: declaration(self.package, &self.package_identities, owner),
                            arguments: arguments
                                .into_iter()
                                .map(n::NormalFormArgument::Value)
                                .collect(),
                        }
                    } else if deferred.is_empty() {
                        K::TypeValue(Term::Nominal {
                            declaration: declaration(self.package, &self.package_identities, owner),
                            arguments: terms,
                        })
                    } else {
                        self.ensure_nominal(owner, span)?;
                        K::NominalType {
                            declaration: declaration(self.package, &self.package_identities, owner),
                            arguments,
                        }
                    };
                    n::Expr {
                        source: source(span),
                        ty: p(P::Type),
                        kind,
                    }
                };
            let ty = if statements.iter().any(|value| is_never(&value.ty)) {
                p(P::Never)
            } else {
                body.ty.clone()
            };
            if let Some(expected) = expected {
                self.require_expression(&ty, expected, span)?;
            }
            if statements.is_empty() {
                Ok(body)
            } else {
                Ok(n::Expr {
                    source: source(span),
                    ty,
                    kind: K::Block {
                        statements,
                        result: Some(Box::new(body)),
                    },
                })
            }
        })();
        self.locals = saved_locals;
        self.alias_substitution = saved_substitution;
        self.alias_value_bindings = saved_bindings;
        self.signature_owner = saved_owner;
        self.context = saved_context;
        self.in_required_comptime = saved_mode;
        result
    }

    fn bind_type_input(
        &mut self,
        value: n::Expr,
        statements: &mut Vec<n::Expr>,
        span: Span,
    ) -> Result<n::Expr, Error> {
        if matches!(value.kind, K::TypeValue(_)) {
            return Ok(value);
        }
        let binding = self.spread_binding(span)?;
        self.locals.insert(binding, p(P::Type));
        statements.push(n::Expr {
            source: value.source,
            ty: if is_never(&value.ty) {
                p(P::Never)
            } else {
                p(P::Void)
            },
            kind: K::Let {
                binding,
                value: Box::new(value),
            },
        });
        Ok(n::Expr {
            source: source(span),
            ty: p(P::Type),
            kind: K::Local(binding),
        })
    }

    /// Keep mandatory parameters available for inference while fixing defaults.
    pub(super) fn function_type_plan(
        &mut self,
        owner: OwnerId,
        known: &[Term],
        span: Span,
    ) -> Result<
        (
            Vec<DeclarationId>,
            Vec<Term>,
            Vec<(usize, Vec<DeclarationId>)>,
        ),
        Error,
    > {
        let parameters = self.type_parameters[&owner].clone();
        if known.len() > parameters.len() {
            return Err(error(span, "too many fixed generic call arguments"));
        }
        let clauses = self.type_clause_params(owner);
        let inherited = parameters.len() - clauses.len();
        let mut inferred = Vec::new();
        let mut pending = Vec::new();
        let mut inputs = known.to_vec();
        for index in known.len()..parameters.len() {
            if index >= inherited
                && matches!(
                    clauses[index - inherited].kind,
                    hir::clause::ClauseParamKind::Optional(..)
                )
            {
                let dependencies = self.function_default_dependencies(
                    owner,
                    index - inherited,
                    &inputs,
                    &inferred,
                    span,
                )?;
                if dependencies.is_empty() {
                    let default =
                        self.type_clause_default(owner, index - inherited, &inputs, None, span)?;
                    inputs.push(default);
                } else {
                    pending.push((index, dependencies));
                    inferred.push(parameters[index]);
                    inputs.push(Term::Rigid {
                        declaration: parameters[index],
                        sort: Sort::Type,
                    });
                }
            } else {
                inferred.push(parameters[index]);
                inputs.push(Term::Rigid {
                    declaration: parameters[index],
                    sort: Sort::Type,
                });
            }
        }
        Ok((inferred, inputs, pending))
    }

    fn function_default_dependencies(
        &mut self,
        owner: OwnerId,
        slot: usize,
        inputs: &[Term],
        inferred: &[DeclarationId],
        span: Span,
    ) -> Result<Vec<DeclarationId>, Error> {
        if inferred.is_empty() {
            return Ok(Vec::new());
        }
        let clauses = self.type_clause_params(owner);
        let hir::clause::ClauseParamKind::Optional(_, _, value) = clauses[slot].kind else {
            unreachable!()
        };
        if self.active_type_defaults.len() >= 64 || !self.active_type_defaults.insert((owner, slot))
        {
            return Err(error(span, "cyclic or excessively deep type default"));
        }
        let saved_owner = self.signature_owner.replace(owner);
        let saved_context = self.context.clone();
        let saved_mode = std::mem::replace(&mut self.in_required_comptime, true);
        let saved_substitution = self.alias_substitution.clone();
        self.context = self.binder_context(owner);
        self.alias_substitution.extend(
            self.type_parameters[&owner]
                .iter()
                .copied()
                .zip(inputs.iter().cloned().map(std::sync::Arc::new)),
        );
        let result = (|| {
            let mut checked = self.expr(value, Some(&p(P::Type)))?;
            let mut budget = comptime_runtime::Budget {
                steps: self.required_steps as usize,
                max_limbs: 4096,
            };
            let before = budget.steps;
            let structural = self.known_normal_input(&checked, 0, &mut budget).is_some();
            self.required_steps -= (before - budget.steps) as u64;
            if structural {
                return Ok(Vec::new());
            }
            let mut dependencies = BTreeSet::new();
            checked.rewrite_types(&mut |ty| {
                dependencies.extend(inferred.iter().copied().filter(|id| ty.mentions_rigid(*id)));
                self.copy_alias_type(ty, span)
            })?;
            Ok(dependencies.into_iter().collect())
        })();
        self.signature_owner = saved_owner;
        self.context = saved_context;
        self.in_required_comptime = saved_mode;
        self.alias_substitution = saved_substitution;
        self.active_type_defaults.remove(&(owner, slot));
        result
    }

    pub(super) fn settle_function_defaults(
        &mut self,
        owner: OwnerId,
        parameters: &[DeclarationId],
        template: &[Term],
        pending: &[(usize, Vec<DeclarationId>)],
        resolved: &mut BTreeMap<usize, Term>,
        candidate: &mut crate::calls::CallCandidate,
        span: Span,
    ) -> Result<bool, Error> {
        let all = self.type_parameters[&owner].clone();
        let inherited = all.len() - self.type_clause_params(owner).len();
        for (&slot, value) in resolved.iter() {
            let index = parameters.iter().position(|id| *id == all[slot]).unwrap();
            candidate
                .constrain_type_argument(index, value, crate::ptc::Budget::default())
                .map_err(|e| call_inference_error(span, e))?;
        }
        if !resolved.is_empty() {
            self.refine_default_call_views(candidate, span)?;
        }
        for (slot, dependencies) in pending {
            if resolved.contains_key(slot) {
                continue;
            }
            let mut substitutions = Vec::new();
            let mut available = Vec::new();
            let mut ready = true;
            for dependency in dependencies {
                if pending
                    .iter()
                    .any(|(other, _)| all[*other] == *dependency && !resolved.contains_key(other))
                {
                    ready = false;
                    break;
                }
                let index = parameters.iter().position(|id| id == dependency).unwrap();
                if let Some(value) = candidate
                    .type_argument(index)
                    .map_err(|e| call_inference_error(span, e))?
                {
                    substitutions.push(*dependency);
                    available.push(value);
                } else {
                    ready = false;
                    break;
                }
            }
            if !ready {
                continue;
            }
            let inputs = template[..*slot]
                .iter()
                .map(|ty| {
                    self.instantiate_input_types(ty, &substitutions, &available, span)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let value = self.type_clause_default(owner, *slot - inherited, &inputs, None, span)?;
            let index = parameters.iter().position(|id| *id == all[*slot]).unwrap();
            candidate
                .constrain_type_argument(index, &value, crate::ptc::Budget::default())
                .map_err(|e| call_inference_error(span, e))?;
            resolved.insert(*slot, value);
            // Settling a default can expose associated types in parameter and
            // result contexts. Refresh their evidence before checking delayed
            // arguments or finishing this candidate, including on retries.
            self.refine_default_call_views(candidate, span)?;
        }
        Ok(resolved.len() == pending.len())
    }

    fn refine_default_call_views(
        &mut self,
        candidate: &mut crate::calls::CallCandidate,
        span: Span,
    ) -> Result<(), Error> {
        candidate
            .refine_type_views(
                |ty, budget| {
                    self.callable_input_view_budgeted(ty, budget, span)
                        .map_err(|error| error.message)
                },
                crate::ptc::Budget::default(),
            )
            .map_err(|error| call_inference_error(span, error))
    }

    pub(super) fn type_clause_arguments(
        &mut self,
        owner: OwnerId,
        args: &[Arg<'_>],
        subject: Option<&Term>,
        span: Span,
    ) -> Result<Vec<Term>, Error> {
        self.type_clause_arguments_with_prefix(owner, args, subject, span, Vec::new())
    }

    pub(super) fn type_clause_arguments_with_prefix(
        &mut self,
        owner: OwnerId,
        args: &[Arg<'_>],
        subject: Option<&Term>,
        span: Span,
        prefix: Vec<Term>,
    ) -> Result<Vec<Term>, Error> {
        let clauses = self.type_clause_params(owner);
        let mut supplied = vec![None; clauses.len()];
        let mut positional = 0;
        let count = clauses.iter().filter(|clause| !matches!(clause.kind, hir::clause::ClauseParamKind::Optional(..))).count();
        for (name, value) in self.expanded_type_inputs(args, count, span)? {
            let slot = Self::type_input_slot(&clauses, name, &mut positional, span)?;
            let cell = supplied.get_mut(slot)
                .ok_or_else(|| error(span, "type argument count mismatch"))?;
            if cell.replace(value).is_some() {
                return Err(error(span, "duplicate type argument"));
            }
        }
        let mut arguments = prefix;
        for (slot, supplied) in supplied.into_iter().enumerate() {
            arguments.push(match supplied {
                Some(value) => value,
                None => self.type_clause_default(owner, slot, &arguments, subject, span)?,
            });
        }
        Ok(arguments)
    }
    /// Validate a default under its declaration binders without invoking it.
    /// Concrete normalization is performed separately when the slot is omitted.
    pub(super) fn check_type_defaults(&mut self, owner: OwnerId) -> Result<(), Error> {
        if self
            .normal_parameters
            .get(&owner)
            .is_some_and(|inputs| inputs.iter().any(|(_, _, ty)| *ty != p(P::Type)))
        {
            return self.check_normal_defaults(owner);
        }
        let clauses = self.type_clause_params(owner);
        let saved_owner = self.signature_owner.replace(owner);
        let saved_context = self.context.clone();
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_mode = std::mem::replace(&mut self.in_required_comptime, true);
        let saved_substitution = std::mem::take(&mut self.alias_substitution);
        let saved_value_bindings = std::mem::take(&mut self.alias_value_bindings);
        self.context = self.binder_context(owner);
        let result = (|| {
            for (slot, clause) in clauses.iter().enumerate() {
                let hir::clause::ClauseParamKind::Optional(_, _, value) = clause.kind else {
                    continue;
                };
                let mut checked = self.expr(value, Some(&p(P::Type)))?;
                let offset = self.type_parameters[&owner].len() - clauses.len();
                let forbidden = self.type_parameters[&owner][offset + slot..].to_vec();
                checked.rewrite_types(&mut |ty| {
                    if forbidden.iter().any(|id| ty.mentions_rigid(*id)) {
                        return Err(error(
                            value.span,
                            "type default references itself or a later parameter",
                        ));
                    }
                    self.copy_alias_type(ty, value.span)
                })?;
            }
            Ok(())
        })();
        self.signature_owner = saved_owner;
        self.context = saved_context;
        self.locals = saved_locals;
        self.in_required_comptime = saved_mode;
        self.alias_substitution = saved_substitution;
        self.alias_value_bindings = saved_value_bindings;
        result
    }

    pub(super) fn type_clause_default(
        &mut self,
        owner: OwnerId,
        slot: usize,
        previous: &[Term],
        subject: Option<&Term>,
        span: Span,
    ) -> Result<Term, Error> {
        let clauses = self.type_clause_params(owner);
        let slot_index = self.type_parameters[&owner].len() - clauses.len() + slot;
        let hir::clause::ClauseParamKind::Optional(_, ty, value) = clauses[slot].kind else {
            return Err(error(span, "type argument count mismatch"));
        };
        if self.active_type_defaults.len() >= 64 || !self.active_type_defaults.insert((owner, slot))
        {
            return Err(error(span, "cyclic or excessively deep type default"));
        }
        let caller=self.call_inference_scope();
        let caller_frames=if caller.is_empty() {vec![]} else {vec![caller.iter().map(|p|p.sort).collect()]};
        let saved_owner = self.signature_owner.replace(owner);
        let context = self.definition_context(owner);
        let saved_context = std::mem::replace(&mut self.context, context);
        let saved_substitution = self.alias_substitution.clone();
        let result = (|| {
            if self.type_expr(ty)? != p(P::Type) {
                return Err(error(ty.span, "type default parameter must have type Type"));
            }
            let parameters: Vec<_> = subject
                .map(|_| declaration(self.package, &self.package_identities, owner))
                .into_iter()
                .chain(self.type_parameters[&owner][..slot_index].iter().copied())
                .collect();
            let arguments: Vec<_> = subject
                .cloned()
                .into_iter()
                .chain(previous.iter().cloned())
                .collect();
            // Defaults are invocations in this concrete telescope, not symbolic
            // computations followed by substitution of only the returned Type.
            self.alias_substitution.extend(
                parameters
                    .iter()
                    .copied()
                    .zip(arguments.iter().cloned().map(std::sync::Arc::new)),
            );
            let default = self.type_expr(value)?;
            if self.type_parameters[&owner][slot_index..]
                .iter()
                .any(|id| default.mentions_rigid(*id))
            {
                return Err(error(
                    value.span,
                    "type default references itself or a later parameter",
                ));
            }
            default
                .instantiate_types_in_scope(&parameters, &arguments,&caller_frames)
                .map_err(|e| error(span, e.to_string()))
        })();
        self.alias_substitution = saved_substitution;
        self.context = saved_context;
        self.signature_owner = saved_owner;
        self.active_type_defaults.remove(&(owner, slot));
        result
    }
}
