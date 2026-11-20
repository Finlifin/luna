//! Runtime default providers are checked with the payload domains of their
//! lexical quote inputs. These temporary locals belong to the provider only.
use super::*;
use runtime_origins::Place;

struct Provider {
    value: n::Expr,
    inputs: BTreeMap<n::LocalId, DeclarationId>,
}

impl Checker<'_, '_> {
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
        self.runtime_default_provider(owner, value, expected)
            .map(|_| ())
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

    pub(super) fn default_runtime_argument(
        &mut self,
        owner: OwnerId,
        param: DeclarationId,
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
        let expected_inner = self.callable_input_view(expected_inner, value.span)?;
        let Provider {
            value: mut checked,
            inputs,
        } = self.runtime_default_provider(owner, value, Some(&expected_inner))?;
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let free = nir::free_bindings::collect(&checked, &mut fuel);
        self.required_steps -= (before - fuel) as u64;
        let free = free.map_err(|reason| error(value.span, reason.to_string()))?;
        if !free.is_empty() {
            let mut fields = Vec::new();
            let mut root = &mut checked;
            let binding = loop {
                match root.kind {
                    K::PointerField {
                        ref mut pointer,
                        index,
                        ..
                    }
                    | K::PointerAnonymousField {
                        ref mut pointer,
                        index,
                        ..
                    } => {
                        fields.push(index);
                        root = pointer;
                    }
                    K::Local(binding) => break binding,
                    _ => {
                        return Err(error(
                            value.span,
                            "computed quote default requires a scoped runtime operand",
                        ));
                    }
                }
            };
            let id = inputs.get(&binding).ok_or_else(|| {
                error(value.span, "runtime default refers to an unavailable input")
            })?;
            let identity = self
                .alias_substitution
                .get(id)
                .cloned()
                .unwrap_or(Term::Rigid {
                    declaration: *id,
                    sort: Sort::Runtime,
                });
            let base = self.deliver_runtime_dependency(&identity, &root.ty, span)?;
            // A lexical temporary is a delivery location. Projection identity
            // still follows the original supplier even after acquisition.
            let computed_root = match &identity {
                Term::Runtime(id) => self.local_runtime_deliveries.get(id).is_some_and(|entry| {
                    entry.producer_site.is_some() || entry.projection.is_some()
                }),
                _ => false,
            };
            let stable_place = !computed_root && self.runtime_place(&base, span)?.is_some();
            *root = base;
            if !stable_place {
                let Term::Runtime(id) = identity else {
                    return Err(error(
                        span,
                        "computed runtime dependency has no producer identity",
                    ));
                };
                fields.reverse();
                let mut path = runtime_origins::Projection {
                    root: id,
                    fields: Vec::new(),
                };
                if let Some(parent) = self
                    .local_runtime_deliveries
                    .get(&id)
                    .and_then(|d| d.projection.as_ref())
                {
                    self.required_steps = self
                        .required_steps
                        .checked_sub(parent.fields.len() as u64)
                        .ok_or_else(|| error(span, "runtime projection path budget exhausted"))?;
                    path.root = parent.root;
                    path.fields.extend_from_slice(&parent.fields);
                }
                path.fields.extend(fields);
                return self.intern_local_runtime_operand(
                    LocalRuntimeOrigin::Projection(std::sync::Arc::new(path)),
                    checked,
                    span,
                );
            }
            return self.intern_runtime_operand(checked, value, span);
        }
        let id = if let Some(Place::Global(origin)) = self.runtime_place(&checked, span)? {
            self.intern_global_runtime_operand(origin, checked, span)?
        } else {
            let id = self.intern_runtime_id(Term::Lift(Box::new(checked.ty.clone())), span)?;
            self.global_runtime_deliveries.insert(id, checked);
            id
        };
        self.global_runtime_origin_ids.insert(key, id);
        Ok(Term::Runtime(id))
    }
}
