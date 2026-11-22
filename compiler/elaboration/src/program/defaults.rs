//! Definition-scoped, demand-driven preparation of ordinary parameter defaults.
use super::*;

#[derive(Default)]
struct Prefix {
    locals: LocalBindings<Term>,
    parameters: input_facts::Parameters,
}

impl Prefix {
    fn bind(&mut self, binding: n::LocalId, index: usize, parameter: &Parameter) {
        self.locals.insert(binding, parameter.ty.clone());
        self.parameters
            .bind(binding, index, parameter.sort, &parameter.ty);
    }
}

impl Checker<'_, '_> {
    pub(super) fn visible_clause_input(
        &self,
        id: hir::common::DefId,
        span: Span,
    ) -> Result<DeclarationId, Error> {
        let input = DeclarationId {
            package: semantic_package(&self.package_identities, id.pkg),
            local: id.index,
        };
        if self.forbidden_default_inputs.contains(&input) {
            return Err(error(
                span,
                "clause default references itself or a later parameter",
            ));
        }
        Ok(input)
    }

    pub(super) fn ensure_defaults(&mut self, owner: OwnerId, span: Span) -> Result<(), Error> {
        if self.open_functions.contains_key(&owner) {
            self.defaults.entry(owner).or_default();
            return Ok(());
        }
        let item = self.package.item(owner).unwrap();
        let (ItemKind::Fn(sig, _) | ItemKind::Case(sig, _) | ItemKind::FnDeclaration(sig)) = &item.kind else {
            return Err(error(
                span,
                "default preparation needs a function declaration",
            ));
        };
        self.defaults
            .entry(owner)
            .or_default()
            .resize_with(sig.params.len(), || None);
        let Some(last) = sig
            .params
            .iter()
            .rposition(|param| matches!(param.ty.kind, TyParamKind::Optional(..)))
        else {
            return Ok(());
        };
        self.ensure_signature(owner, span)?;
        self.ensure_contract(owner, span)?;
        let signature = self
            .declared_signature(owner)
            .ok_or_else(|| {
                error(
                    span,
                    "ordinary default requires a formed declaration signature",
                )
            })?
            .into_owned();
        let Term::Function { parameters, .. } = signature else {
            unreachable!("declaration has a function telescope")
        };
        let mut prefix = Prefix::default();
        for (index, (parameter, ty)) in sig
            .params
            .iter()
            .zip(&parameters)
            .take(last + 1)
            .enumerate()
        {
            if matches!(parameter.ty.kind, TyParamKind::Optional(..)) {
                self.prepare_parameter_default(owner, index, span, Some((&prefix, ty)))?;
            }
            prefix.bind(local(parameter.hir_id), index, ty);
        }
        Ok(())
    }

    pub(super) fn ensure_parameter_default(
        &mut self,
        owner: OwnerId,
        index: usize,
        span: Span,
    ) -> Result<(), Error> {
        self.prepare_parameter_default(owner, index, span, None)
    }

    fn prepare_parameter_default(
        &mut self,
        owner: OwnerId,
        index: usize,
        span: Span,
        prepared: Option<(&Prefix, &Parameter)>,
    ) -> Result<(), Error> {
        let key = (owner, index);
        if self.default_uses.contains_key(&key) {
            if self.pending_type_uses.is_some() {
                self.signature_type_uses
                    .extend(self.default_uses.get(&key).into_iter().flatten().cloned());
            }
            return Ok(());
        }
        if self.active_function_defaults.len() >= 64 || !self.active_function_defaults.insert(key) {
            return Err(error(
                span,
                "cyclic or excessively deep function default dependency",
            ));
        }
        let saved_pending = self.pending_type_uses.replace(Vec::new());
        let result = self.in_formal_declaration(owner, |this| {
            this.check_parameter_default(owner, index, span, prepared)
        });
        let uses = std::mem::replace(&mut self.pending_type_uses, saved_pending).unwrap();
        self.active_function_defaults.remove(&key);
        let (count, default) = result?;
        if self.pending_type_uses.is_none() {
            for (scope, assume, target, arguments, span) in &uses {
                let context = match scope {
                    Some(scope) if *assume => self.definition_context(*scope),
                    Some(scope) => self.binder_context(*scope),
                    None => self.nominal_context(),
                };
                let caller = std::mem::replace(&mut self.context, context);
                let checked = self.check_formation_requirements(*target, arguments, *span);
                self.context = caller;
                checked?;
            }
        } else {
            self.signature_type_uses.extend(uses.iter().cloned());
        }
        self.default_uses.insert(key, uses);
        // Open declarations initially publish an empty lazy-default table.
        // A behavior witness may subsequently demand an ordinary default using
        // its formed signature; initialize missing slots even if the entry
        // already exists, preserving defaults checked by earlier demands.
        let defaults = self.defaults.entry(owner).or_default();
        if defaults.len() < count {
            defaults.resize_with(count, || None);
        }
        defaults[index] = default;
        Ok(())
    }

    /// A definition-side call checks a supplier with the callee's staged
    /// parameters still abstract. Executable calls retain their own suppliers.
    pub(super) fn parameter_default_behavior(
        &mut self,
        owner: OwnerId,
        slot: usize,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let key = (owner, slot);
        if self.active_function_defaults.len() >= 64 || !self.active_function_defaults.insert(key) {
            return Err(error(
                span,
                "cyclic or excessively deep function default dependency",
            ));
        }
        let saved = std::mem::replace(&mut self.checking_default_behavior, true);
        let result = self.in_formal_declaration(owner, |this| {
            this.checking_open_behavior = true;
            this.describing_open_call = true;
            let (_, value) = this.check_parameter_default(owner, slot, span, None)?;
            let value = value.ok_or_else(|| error(span, "missing default supplier"))?;
            let value = this.close_default_behavior(value, owner, span)?;
            Ok((
                value,
                std::mem::take(&mut this.definition_refinements.staged_values),
            ))
        });
        self.checking_default_behavior = saved;
        self.active_function_defaults.remove(&key);
        let (value, binders) = result?;
        self.definition_refinements.staged_values.extend(binders);
        Ok(value)
    }

    fn close_default_behavior(
        &mut self,
        mut value: n::Expr,
        owner: OwnerId,
        span: Span,
    ) -> Result<n::Expr, Error> {
        if self.ordinary_behavior_inputs.is_empty() {
            return Ok(value);
        }
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let accesses = nir::free_bindings::access(
            &value,
            |binding| self.ordinary_behavior_inputs.contains_key(&binding),
            &mut fuel,
        );
        self.required_steps -= (before - fuel) as u64;
        let accesses = accesses.map_err(|message| error(span, message))?;
        let mut statements = Vec::new();
        let mut reads = BTreeMap::new();
        for (place, bindings) in [(false, accesses.readonly), (true, accesses.writable)] {
            for binding in bindings {
                let domain = self
                    .ordinary_behavior_inputs
                    .shared(&binding)
                    .expect("selected behavior input")
                    .clone();
                let ty = self.copy_alias_type(&domain, span)?;
                let binder = if ty == p(P::Type) {
                    self.fresh_type_binder(owner, span)?
                } else {
                    self.fresh_value_binder(owner, span)?
                };
                let symbol = Term::Rigid {
                    declaration: binder.declaration,
                    sort: binder.sort,
                };
                let kind = if binder.sort == Sort::Type {
                    K::TypeValue(symbol)
                } else {
                    K::ComptimeValue(symbol)
                };
                self.definition_refinements
                    .staged_values
                    .push((binder.declaration, ty));
                if place {
                    // A capture or place operation still needs a local cell.
                    statements.push(n::Expr {
                        source: value.source,
                        ty: p(P::Void),
                        kind: K::Let {
                            binding,
                            value: Box::new(n::Expr {
                                source: value.source,
                                ty: self.copy_alias_type(&domain, span)?,
                                kind,
                            }),
                        },
                    });
                } else {
                    reads.insert(binding, kind);
                }
            }
        }
        if !reads.is_empty() {
            // Substituting at each read keeps comptime-only values at their
            // consumer's stage and preserves the caller's earlier parameters.
            let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
            let before = fuel;
            let result =
                nir::visit::expression_mut_in_caller_frame(&mut value, &mut fuel, |node, _| {
                    if let K::Local(binding) = node.kind
                        && let Some(witness) = reads.get(&binding)
                    {
                        node.kind = witness.clone();
                    }
                    Ok(())
                });
            self.required_steps -= (before - fuel) as u64;
            result.map_err(|message| error(span, message))?;
        }
        if statements.is_empty() {
            return Ok(value);
        }
        Ok(n::Expr {
            source: value.source,
            ty: value.ty.clone(),
            kind: K::Block {
                statements,
                result: Some(Box::new(value)),
            },
        })
    }

    fn check_parameter_default(
        &mut self,
        owner: OwnerId,
        index: usize,
        span: Span,
        prepared: Option<(&Prefix, &Parameter)>,
    ) -> Result<(usize, Option<n::Expr>), Error> {
        self.ensure_signature(owner, span)?;
        self.ensure_contract(owner, span)?;
        self.context = self.definition_context(owner);
        let item = self.package.item(owner).unwrap();
        let (ItemKind::Fn(sig, _) | ItemKind::Case(sig, _) | ItemKind::FnDeclaration(sig)) = &item.kind else {
            return Err(error(
                span,
                "default preparation needs a function declaration",
            ));
        };
        let parameter = sig
            .params
            .get(index)
            .ok_or_else(|| error(span, "unknown default parameter"))?;
        // A single demand builds just its prefix. Package checking advances
        // the same persistent roots once for all defaults in source order.
        let ty = match prepared {
            Some((prefix, ty)) => {
                self.locals = prefix.locals.clone();
                self.input_facts.parameters = prefix.parameters.clone();
                ty.clone()
            }
            None => {
                let signature = self.declared_signature(owner).ok_or_else(|| {
                    error(
                        span,
                        "ordinary default requires a formed declaration signature",
                    )
                })?;
                let Term::Function { parameters, .. } = signature.as_ref() else {
                    unreachable!("declaration has a function telescope")
                };
                let mut prefix = Prefix::default();
                for (slot, (earlier, ty)) in
                    sig.params.iter().zip(parameters).take(index).enumerate()
                {
                    prefix.bind(local(earlier.hir_id), slot, ty);
                }
                let ty = parameters[index].clone();
                self.locals = prefix.locals;
                self.input_facts.parameters = prefix.parameters;
                ty
            }
        };
        let default = if let TyParamKind::Optional(_, _, value) = parameter.ty.kind {
            let value = self.with_staged_input(ty.sort != Sort::Runtime, |this| {
                this.expr(value, Some(&ty.ty))
            })?;
            if !sig.modifiers.is_comptime && !parameter.ty.is_comptime() {
                self.require_stable_default(&value, parameter.span)?;
            }
            Some(value)
        } else {
            None
        };
        Ok((sig.params.len(), default))
    }

    pub(super) fn field_default(
        &mut self,
        owner: OwnerId,
        variant: Option<usize>,
        index: usize,
    ) -> Result<std::sync::Arc<n::Expr>, Error> {
        let item = self.package.item(owner).unwrap();
        let field = match (&item.kind, variant) {
            (ItemKind::Struct(def), None) => &def.fields[index],
            (ItemKind::Enum(_), Some(v)) => {
                let id = declaration(self.package, &self.package_identities, owner);
                let hir::item::VariantKind::Struct(fields) = &self.enum_sources[&id][v].kind else {
                    return Err(error(item.span, "missing required enum payload field"));
                };
                &fields[index]
            }
            _ => unreachable!(),
        };
        if let Some(checked) = self.field_defaults.shared(&field.hir_id) {
            return Ok(checked.clone());
        }
        let value = field.default.ok_or_else(|| {
            error(
                field.span,
                format!(
                    "missing required {} field {}",
                    if variant.is_some() {
                        "enum payload"
                    } else {
                        "struct"
                    },
                    field.ident.name
                ),
            )
        })?;
        if !self.active_defaults.insert((owner, variant, index)) {
            return Err(error(field.span, "cyclic field default elaboration"));
        }
        let checked = self.in_formal_declaration(owner, |this| {
            this.context = this.definition_context(owner);
            let id = declaration(this.package, &this.package_identities, owner);
            let domain = match variant {
                None => &this.structs[&id].fields[index].1,
                Some(variant) => &this.enums[&id].variants[variant].fields[index],
            };
            let domain = declaration_inputs::copy_input_domain(
                domain,
                &mut this.required_steps,
                field.span,
            )?;
            this.expr(value, Some(&domain))
        });
        self.active_defaults.remove(&(owner, variant, index));
        let checked = std::sync::Arc::new(checked?);
        self.field_defaults.insert(field.hir_id, checked.clone());
        Ok(checked)
    }

    pub(super) fn require_stable_default(
        &mut self,
        value: &n::Expr,
        span: Span,
    ) -> Result<(), Error> {
        if let Some(obligation) = self.stable_observation_obligation(value, span)? {
            self.pending_default_stability.push(obligation);
        }
        Ok(())
    }

    pub(super) fn require_stable_predicate(
        &mut self,
        value: &n::Expr,
        span: Span,
    ) -> Result<(), Error> {
        if let Some(obligation) = self.stable_observation_obligation(value, span)? {
            self.pending_predicate_stability.push(obligation);
        }
        Ok(())
    }

    fn stable_observation_obligation(
        &mut self,
        value: &n::Expr,
        span: Span,
    ) -> Result<Option<purity::StableObservation>, Error> {
        if stable_default(value) {
            return Ok(None);
        }
        let viewed = self.default_stability_view(value, span)?;
        if !stable_default(&viewed) {
            // Ordinary and open defaults share P3 evidence. Bodies need not
            // be available while signatures/defaults are demanded, so retain
            // the scoped analysis view for the complete-package check.
            // Open definitions may have no executable instances at all. Carry
            // the lexical stage evidence with the obligation, rather than
            // recovering it accidentally from some caller's formed instance.
            let mut staged = BTreeSet::new();
            let mut scope = purity::ObservationScope::default();
            if let Some(owner) = self.signature_owner {
                if let Some(parameters) = self.type_parameters.get(&owner) {
                    scope.parameters = self.input_binders(parameters);
                }
                staged.extend(self.ordinary_behavior_inputs.keys().copied());
                scope
                    .parameters
                    .extend(self.definition_refinements.staged_values.iter().map(
                        |(declaration, domain)| nir::DeclarationParameter {
                            declaration: *declaration,
                            sort: Sort::of_comptime_domain(domain),
                        },
                    ));
                scope.requirements = self.definition_requirements(owner);
                scope.input_scope = self.lexical_input_domains().map(|(sort, _)| sort).collect();
                if let Some(ItemKind::Fn(sig, _) | ItemKind::Case(sig, _) | ItemKind::FnDeclaration(sig)) =
                    self.package.item(owner).map(|item| &item.kind)
                {
                    // signature_domains is a temporary signature-building
                    // telescope and has already been released here. The
                    // checked declaration retains the parameter's phase.
                    for parameter in sig.params {
                        let binding = local(parameter.hir_id);
                        if parameter.ty.is_comptime() && self.locals.contains_key(&binding) {
                            staged.insert(binding);
                        }
                    }
                }
            }
            return Ok(Some(purity::StableObservation {
                value: viewed,
                source: source(span),
                staged_parameters: staged,
                scope,
            }));
        }
        Ok(None)
    }

    fn default_stability_view(&mut self, value: &n::Expr, span: Span) -> Result<n::Expr, Error> {
        // This private analysis view is discarded: source annotations, normal
        // form identities and runtime binding frames are never rewritten.
        super::constants::charge_copy(value, &mut self.required_steps, span)?;
        let mut viewed = value.clone();
        let mut cache = std::collections::HashMap::<Term, Term>::new();
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let checked = iir::visit::expression_mut(&mut viewed, &mut fuel, |expr, fuel| {
            expr.ty = self.default_observation_view(
                &expr.ty,
                &mut cache,
                &mut std::collections::HashSet::new(),
                fuel,
                0,
            )?;
            Ok(())
        });
        self.required_steps -= (before - fuel) as u64;
        checked.map_err(|message| error(span, message))?;
        Ok(viewed)
    }

    fn default_observation_view(
        &self,
        ty: &Term,
        cache: &mut std::collections::HashMap<Term, Term>,
        active: &mut std::collections::HashSet<Term>,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<Term, String> {
        if depth >= 128 {
            return Err("default observation type depth exceeded".into());
        }
        crate::associated::charge_projection_copy(ty, fuel)?;
        if copy_scalar(ty) || active.contains(ty) {
            return Ok(ty.clone());
        }
        if let Some(view) = cache.get(ty) {
            crate::associated::charge_projection_copy(view, fuel)?;
            return Ok(view.clone());
        }
        active.insert(ty.clone());
        let result = (|| {
            let view = if matches!(ty, Term::Tuple(_) | Term::Record(_)) {
                ty.clone()
            } else {
                let (result, used) = self.context.structural_view_with_heads_metered(
                    ty,
                    crate::ptc::Budget {
                        steps: (*fuel).min(100_000),
                        max_depth: 128,
                    },
                    |head| Some(head) == self.protocols.tuple,
                );
                *fuel = fuel.saturating_sub(used);
                result
                    .ok()
                    .flatten()
                    .map(|(ty, _)| ty)
                    .unwrap_or_else(|| ty.clone())
            };
            let view = if let Some(elements) = self.bound_tuple_elements(&view) {
                crate::associated::charge_projection_copy(&view, fuel)?;
                Term::Tuple(elements.to_vec())
            } else {
                view
            };
            // Descend only through data aggregates. Pointer pointees and callable
            // binders are not observations made by evaluating this default.
            let view = match view {
                Term::Tuple(fields) => Term::Tuple(
                    fields
                        .iter()
                        .map(|field| {
                            self.default_observation_view(field, cache, active, fuel, depth + 1)
                        })
                        .collect::<Result<_, _>>()?,
                ),
                Term::Record(fields) => Term::Record(
                    fields
                        .into_iter()
                        .map(|(name, field)| {
                            self.default_observation_view(&field, cache, active, fuel, depth + 1)
                                .map(|field| (name, field))
                        })
                        .collect::<Result<_, _>>()?,
                ),
                other => other,
            };
            crate::associated::charge_projection_copy(&view, fuel)?;
            cache.insert(ty.clone(), view.clone());
            Ok(view)
        })();
        active.remove(ty);
        result
    }
}
