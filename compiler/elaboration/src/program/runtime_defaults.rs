//! Runtime default providers are checked with the payload domains of their
//! lexical quote inputs. These temporary locals belong to the provider only.
use super::*;

struct Provider {
    value: n::Expr,
    inputs: BTreeMap<n::LocalId, DeclarationId>,
}

/// A checked provider awaiting evaluation at a concrete runtime use. Its local
/// parameters are private to the provider; the result binding belongs to the
/// consuming call or constructor. Only that binding is published for delivery.
pub(super) struct ScopedDefault {
    binding: n::LocalId,
    value: n::Expr,
    inputs: Vec<(n::LocalId, Term)>,
}

impl Checker<'_, '_> {
    /// Rebuild a checked address in the active lexical environment. A
    /// projected identity can occur in a signature without such an environment.
    pub(super) fn deliver_runtime_field(
        &mut self,
        term: &Term,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let (root, fields) = self.runtime_field_path(term, span)?;
        let domain = self.runtime_root_payload(root, span)?;
        let base = self.deliver_runtime_dependency(root, &domain, span)?;
        self.apply_runtime_fields(base, fields, span)
    }

    pub(super) fn runtime_field_path<'a>(
        &mut self,
        term: &'a Term,
        span: Span,
    ) -> Result<(&'a Term, Vec<usize>), Error> {
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let result = nir::runtime_fields::path(term, &mut fuel);
        self.required_steps -= (before - fuel) as u64;
        result.map_err(|e| error(span, e.to_string()))
    }

    pub(super) fn runtime_root_payload(&mut self, root: &Term, span: Span) -> Result<Term, Error> {
        let domain = match root {
            Term::Runtime(id) => self.runtime_domains.get(id).and_then(|ty| match ty {
                Term::Lift(inner) => Some(inner.as_ref()),
                _ => None,
            }),
            Term::Rigid {
                declaration,
                sort: Sort::Runtime,
            } => self
                .active_env_parameters
                .get(declaration)
                .map(|(_, ty)| ty),
            _ => None,
        }
        .ok_or_else(|| error(span, "runtime projection root is unavailable"))?;
        declaration_inputs::copy_input_domain(domain, &mut self.required_steps, span)
    }

    pub(super) fn apply_runtime_fields(
        &mut self,
        base: n::Expr,
        fields: Vec<usize>,
        span: Span,
    ) -> Result<n::Expr, Error> {
        if fields.is_empty() {
            return Ok(base);
        }
        let frames = self.call_input_frames();
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let result = nir::runtime_fields::address(
            base,
            &fields,
            source(span),
            &mut fuel,
            |ty, index, fuel| {
                let Term::Nominal {
                    declaration,
                    arguments,
                } = ty
                else {
                    return Err("field requires a struct".into());
                };
                let def = self
                    .structs
                    .get(declaration)
                    .ok_or("missing struct descriptor")?;
                def.addressed_field(index, arguments, &frames, fuel)
            },
        );
        self.required_steps -= (before - fuel) as u64;
        result.map_err(|e| error(span, e.to_string()))
    }

    /// Interpret only field-address selections on a provider's quote inputs.
    /// The provider locals are discarded after checking and never escape as
    /// runtime identities.
    fn provider_identity(
        &mut self,
        provider: &Provider,
        span: Span,
    ) -> Result<Option<Term>, Error> {
        let mut fields = Vec::new();
        let mut root = &provider.value;
        loop {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(span, "runtime projection budget exhausted"))?;
            match &root.kind {
                K::PointerField { pointer, index, .. }
                | K::PointerAnonymousField { pointer, index, .. } => {
                    fields.push(*index);
                    root = pointer;
                }
                K::Local(binding) => {
                    let Some(id) = provider.inputs.get(binding) else {
                        return Ok(None);
                    };
                    let identity = match self.alias_substitution.get(id) {
                        Some(term) if term.sort() == Sort::Runtime => {
                            declaration_inputs::copy_input_domain(
                                term,
                                &mut self.required_steps,
                                span,
                            )?
                        }
                        Some(_) => {
                            return Err(error(span, "quote input requires a runtime identity"));
                        }
                        None => Term::Rigid {
                            declaration: *id,
                            sort: Sort::Runtime,
                        },
                    };
                    return Ok(Some(fields.into_iter().rev().fold(
                        identity,
                        |base, index| Term::RuntimeField {
                            base: Box::new(base),
                            index,
                        },
                    )));
                }
                _ => return Ok(None),
            }
        }
    }

    pub(super) fn signature_runtime_projection(
        &mut self,
        value: &Expr<'_>,
        expected: &Term,
    ) -> Result<Option<Term>, Error> {
        let mut root = value;
        while let H::Projection(base, _) = root.kind {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(value.span, "runtime projection budget exhausted"))?;
            root = base;
        }
        if !matches!(root.kind, H::ResolvedClauseParam(..)) {
            return Ok(None);
        }
        let provider = self.runtime_default_provider(value.hir_id.owner, value, Some(expected))?;
        self.provider_identity(&provider, value.span)
    }

    pub(super) fn check_runtime_default(
        &mut self,
        owner: OwnerId,
        value: &Expr<'_>,
        inner: &Term,
    ) -> Result<(), Error> {
        let expected = (!self.default_domain_has_rigid(inner, value.span)?).then_some(inner);
        if self.forward_runtime_dependency(value, expected)?.is_some() {
            return Ok(());
        }
        let provider = self.runtime_default_provider(owner, value, expected)?;
        self.require_stable_default(&provider.value, value.span)
    }

    fn runtime_default_provider(
        &mut self,
        owner: OwnerId,
        value: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<Provider, Error> {
        let reads = self.expression_declaration_dependencies(owner, value)?;
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_env = std::mem::take(&mut self.active_env_parameters);
        let saved_defaults = self.normal_default_bindings.clone();
        let saved_immutable = std::mem::take(&mut self.immutable);
        let saved_required = std::mem::replace(&mut self.in_required_comptime, false);
        let saved_staged = std::mem::replace(&mut self.in_staged_input, false);
        let result = (|| {
            let mut inputs = BTreeMap::new();
            for id in reads {
                let Some(Term::Lift(inner)) = self.trait_input_domain(id) else {
                    continue;
                };
                let inner = self.substituted_trait_domain(&inner, value.span)?;
                let binding = self.spread_binding(value.span)?;
                self.normal_default_bindings.remove(&id);
                self.locals.insert(binding, inner.clone());
                self.immutable.insert(binding);
                self.active_env_parameters.insert(id, (binding, inner));
                inputs.insert(binding, id);
            }
            let checked = self.expr(value, expected)?;
            if is_never(&checked.ty) {
                return Err(error(
                    value.span,
                    "diverging default cannot satisfy a runtime quote dependency",
                ));
            }
            Ok(Provider {
                value: checked,
                inputs,
            })
        })();
        self.active_env_parameters = saved_env;
        self.locals = saved_locals;
        self.immutable = saved_immutable;
        self.normal_default_bindings = saved_defaults;
        self.in_staged_input = saved_staged;
        self.in_required_comptime = saved_required;
        result
    }

    pub(super) fn default_result_local(&self, id: nir::RuntimeId, span: Span) -> Option<n::Expr> {
        let provider = self.local_runtime_origins.defaults.get(&id)?;
        Some(n::Expr {
            ty: provider.value.ty.clone(),
            kind: K::Local(provider.binding),
            source: source(span),
        })
    }

    pub(super) fn acquire_runtime_default<'a>(
        &mut self,
        id: nir::RuntimeId,
        prepared: impl Fn(&Term) -> Option<&'a n::Expr>,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let provider = self
            .local_runtime_origins
            .defaults
            .shared(&id)
            .cloned()
            .ok_or_else(|| error(span, "missing scoped runtime default"))?;
        let mut inputs = Vec::new();
        let mut connections = BTreeMap::new();
        for (binding, identity) in &provider.inputs {
            let (root, fields) = self.runtime_field_path(identity, span)?;
            if let Term::Runtime(id) = root
                && self.local_runtime_origins.defaults.contains_key(id)
            {
                return Err(error(
                    span,
                    "runtime default depends on a later acquisition",
                ));
            }
            let base = if let Some(base) = prepared(root) {
                constants::charge_copy(base, &mut self.required_steps, span)?;
                base.clone()
            } else {
                let ty = self.runtime_root_payload(root, span)?;
                self.deliver_runtime_dependency(root, &ty, span)?
            };
            let mut input = self.apply_runtime_fields(base, fields, span)?;
            if !matches!(input.kind, K::Local(_)) {
                inputs.push(self.prepare_call_operand(&mut input, span)?);
            }
            let K::Local(actual) = input.kind else {
                unreachable!("prepared provider input")
            };
            connections.insert(*binding, actual);
        }
        constants::charge_copy(&provider.value, &mut self.required_steps, span)?;
        let mut value = provider.value.clone();
        cmtt::freshen(
            &mut value,
            connections,
            &mut self.next_temporary_binding,
            &mut self.expansion_sources,
            span,
        )?;
        let ty = value.ty.clone();
        let body = super::runtime_environment::Acquisition(inputs).finish(self, value, span)?;
        let binding = provider.binding;
        self.locals.insert(binding, ty.clone());
        self.immutable.insert(binding);
        self.local_runtime_origins
            .acquired_bindings
            .insert(binding, id);
        self.local_runtime_deliveries.insert(
            id,
            LocalRuntimeDelivery {
                producer_site: None,
                checked_epoch: None,
                expr: n::Expr {
                    ty,
                    source: source(span),
                    kind: K::Local(binding),
                },
            },
        );
        self.local_runtime_origins.defaults.remove(&id);
        Ok(n::Expr {
            ty: p(P::Void),
            source: source(span),
            kind: K::Let {
                binding,
                value: Box::new(body),
            },
        })
    }

    pub(super) fn default_runtime_argument(
        &mut self,
        owner: OwnerId,
        param: DeclarationId,
        caller: Option<OwnerId>,
        prefix_args: &[Term],
        value: &Expr<'_>,
        domain: &Term,
        span: Span,
    ) -> Result<Term, Error> {
        let Term::Lift(expected_inner) = domain else {
            return Err(error(
                span,
                "runtime default requires a lift dependency domain",
            ));
        };
        if let Some(identity) = self.forward_runtime_dependency(value, Some(expected_inner))? {
            return Ok(identity);
        }
        let expected_inner = self.callable_input_view(expected_inner, value.span)?;
        let provider = self.runtime_default_provider(owner, value, Some(&expected_inner))?;
        if let Some(identity) = self.provider_identity(&provider, value.span)? {
            return Ok(identity);
        }
        let checked = provider.value;
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let free = nir::free_bindings::collect(&checked, &mut fuel);
        self.required_steps -= (before - fuel) as u64;
        let free = free.map_err(|reason| error(value.span, reason.to_string()))?;
        if free.is_empty() && self.runtime_place(&checked, span)?.is_some() {
            return self.intern_runtime_operand(checked, value, span);
        }
        // A closed call still evaluates runtime code. Only values which are
        // already immutable leaves can be delivered directly from the package.
        if !matches!(
            &checked.kind,
            K::Literal(_)
                | K::Unit
                | K::Function(_)
                | K::FunctionInstance { .. }
                | K::ComptimeValue(_)
                | K::TypeValue(_)
        ) || !free.is_empty()
        {
            if !self.runtime_dependency_body(caller) {
                return Err(error(
                    span,
                    "computed quote default requires a runtime acquisition site",
                ));
            }
            let mut inputs = Vec::with_capacity(free.len());
            for binding in free {
                let param = *provider.inputs.get(&binding).ok_or_else(|| {
                    error(value.span, "runtime default captures an unavailable local")
                })?;
                let identity = match self.alias_substitution.get(&param) {
                    Some(term) => {
                        declaration_inputs::copy_input_domain(term, &mut self.required_steps, span)?
                    }
                    None => Term::Rigid {
                        declaration: param,
                        sort: Sort::Runtime,
                    },
                };
                if identity.sort() != Sort::Runtime {
                    return Err(error(span, "runtime default needs a quote input"));
                }
                inputs.push((binding, identity));
            }
            let binding = self.spread_binding(span)?;
            let id = self.intern_runtime_id(Term::Lift(Box::new(checked.ty.clone())), span)?;
            self.local_runtime_origins.defaults.insert(
                id,
                ScopedDefault {
                    binding,
                    inputs,
                    value: checked,
                },
            );
            return Ok(Term::Runtime(id));
        }
        let key = GlobalRuntimeOrigin::DefaultParam {
            owner,
            param,
            prefix_args: prefix_args
                .iter()
                .map(|arg| {
                    declaration_inputs::copy_input_domain(arg, &mut self.required_steps, span)
                })
                .collect::<Result<_, _>>()?,
        };
        if let Some(&existing) = self.global_runtime_origin_ids.get(&key) {
            return Ok(Term::Runtime(existing));
        }
        let id = {
            let id = self.intern_runtime_id(Term::Lift(Box::new(checked.ty.clone())), span)?;
            self.global_runtime_deliveries.insert(id, checked);
            id
        };
        self.global_runtime_origin_ids.insert(key, id);
        Ok(Term::Runtime(id))
    }
}
