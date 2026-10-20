//! Definition-scoped, demand-driven preparation of ordinary parameter defaults.
use super::*;

impl Checker<'_, '_> {
    pub(super) fn ensure_defaults(&mut self, owner: OwnerId, span: Span) -> Result<(), Error> {
        if self.open_functions.contains_key(&owner) {
            self.defaults.entry(owner).or_default();
            return Ok(());
        }
        let item = self.package.item(owner).unwrap();
        let (ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)) = &item.kind else {
            return Err(error(
                span,
                "default preparation needs a function declaration",
            ));
        };
        for index in 0..sig.params.len() {
            self.ensure_parameter_default(owner, index, span)?;
        }
        // Zero-parameter functions still publish a complete empty vector.
        self.defaults.entry(owner).or_default();
        Ok(())
    }

    pub(super) fn ensure_parameter_default(
        &mut self,
        owner: OwnerId,
        index: usize,
        span: Span,
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
        let saved_stage = std::mem::replace(&mut self.in_staged_input, false);
        let saved_owner = self.signature_owner.replace(owner);
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_captures = std::mem::take(&mut self.captures);
        let saved_result = self.result.take();
        let saved_partial_return = self.partial_return_context.take();
        let saved_optional_returns = std::mem::take(&mut self.pending_optional_returns);
        let saved_returns = self.return_observations.take();
        let saved_loops = std::mem::take(&mut self.loops);
        let saved_immutable = std::mem::take(&mut self.immutable);
        let saved_context = self.context.clone();
        let saved_contract = std::mem::replace(&mut self.checking_contract, false);
        let saved_numeric = std::mem::replace(&mut self.numeric_defaults, true);
        let saved_pending = self.pending_type_uses.replace(Vec::new());
        let result = (|| {
            self.ensure_signature(owner, span)?;
            self.ensure_contract(owner, span)?;
            self.context = self.definition_context(owner);
            let item = self.package.item(owner).unwrap();
            let (ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)) = &item.kind else {
                return Err(error(
                    span,
                    "default preparation needs a function declaration",
                ));
            };
            let Term::Function { parameters, .. } = self
                .signatures
                .get(&owner)
                .or_else(|| self.interface_signatures.get(&owner))
                .unwrap()
                .clone()
            else {
                unreachable!()
            };
            let parameter = sig
                .params
                .get(index)
                .ok_or_else(|| error(span, "unknown default parameter"))?;
            // Earlier parameters are bindings in the declaration frame. Their
            // default expressions are independent queries, not prerequisites.
            for (parameter, ty) in sig.params.iter().zip(&parameters).take(index) {
                self.locals.insert(local(parameter.hir_id), ty.ty.clone());
            }
            let ty = &parameters[index];
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
        })();
        let uses = std::mem::replace(&mut self.pending_type_uses, saved_pending).unwrap();
        self.in_staged_input = saved_stage;
        self.signature_owner = saved_owner;
        self.locals = saved_locals;
        self.captures = saved_captures;
        self.result = saved_result;
        self.partial_return_context = saved_partial_return;
        self.pending_optional_returns = saved_optional_returns;
        self.return_observations = saved_returns;
        self.loops = saved_loops;
        self.immutable = saved_immutable;
        self.context = saved_context.clone();
        self.checking_contract = saved_contract;
        self.numeric_defaults = saved_numeric;
        self.active_function_defaults.remove(&key);
        let (count, default) = result?;
        if self.pending_type_uses.is_none() {
            for (scope, assume, target, arguments, span) in &uses {
                self.context = match scope {
                    Some(scope) if *assume => self.definition_context(*scope),
                    Some(scope) => self.binder_context(*scope),
                    None => self.nominal_context(),
                };
                let checked = self.check_formation_requirements(*target, arguments, *span);
                self.context = saved_context.clone();
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

    pub(super) fn require_stable_default(
        &mut self,
        value: &n::Expr,
        span: Span,
    ) -> Result<(), Error> {
        if stable_default(value) {
            return Ok(());
        }
        let viewed = self.default_stability_view(value, span)?;
        if !stable_default(&viewed) {
            // Ordinary and open defaults share P3 evidence. Bodies need not
            // be available while signatures/defaults are demanded, so retain
            // the scoped analysis view for the complete-package check.
            self.pending_default_stability.push((viewed, span));
        }
        Ok(())
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
