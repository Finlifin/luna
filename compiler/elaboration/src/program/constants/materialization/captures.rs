//! Check closure capture scope and escape before retaining a live graph
//! or reconstructing its owned environment.
use super::*;

impl Checker<'_, '_> {
    pub(in crate::program) fn validate_generator_live_callbacks(
        &mut self,
        callbacks: &[&Value],
        span: Span,
    ) -> Result<(), Error> {
        if callbacks.is_empty() {
            return Ok(());
        }
        let scope = self.local_constant_type_scope(span)?;
        let mut active = HashSet::new();
        let mut completed = HashSet::new();
        for callback in callbacks {
            self.validate_closure_snapshot(callback, span, 0, &mut active, &mut completed, &scope)?;
        }
        Ok(())
    }

    pub(super) fn local_constant_type_scope(
        &mut self,
        span: Span,
    ) -> Result<BTreeSet<DeclarationId>, Error> {
        let mut type_scope: BTreeSet<_> = self
            .signature_owner
            .and_then(|owner| self.type_parameters.get(&owner))
            .into_iter()
            .flat_map(|parameters| self.input_binders(parameters))
            .filter(|parameter| parameter.sort == Sort::Type)
            .map(|parameter| parameter.declaration)
            .collect();
        for argument in self.alias_substitution.values() {
            let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
            let before = fuel;
            let checked = nir::type_computations::charge_term(argument, &mut fuel);
            self.required_steps -= (before - fuel) as u64;
            checked.map_err(|cause| error(span, cause.to_string()))?;
            type_scope.extend(
                argument
                    .rigid_parameters()
                    .map_err(|cause| error(span, cause.to_string()))?
                    .into_iter()
                    .filter_map(|(id, sort)| (sort == Sort::Type).then_some(id)),
            );
        }
        Ok(type_scope)
    }

    fn check_capture_term(
        &mut self,
        term: &Term,
        type_scope: &BTreeSet<DeclarationId>,
        span: Span,
    ) -> Result<(), Error> {
        let steps = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let mut budget = comptime_runtime::Budget {
            steps,
            max_limbs: 4096,
        };
        let checked = comptime_interpretor::validate_snapshot_term(term, &mut budget, type_scope);
        self.required_steps -= (steps - budget.steps) as u64;
        checked.map_err(|cause| error(span, format!("incomplete closure capture term: {cause:?}")))
    }

    fn check_capture_domain(
        &mut self,
        ty: &Term,
        type_scope: &BTreeSet<DeclarationId>,
        span: Span,
    ) -> Result<(), Error> {
        if ty.sort() != Sort::Type {
            return Err(error(span, "closure capture domain must be a Type"));
        }
        self.check_capture_term(ty, type_scope, span)
    }

    /// Validate owned capture data before reconstructing a snapshot graph.
    /// Materialization preserves shared closure nodes within that graph; mutable
    /// owned cells are copied at their current values, never as active references.
    pub(super) fn validate_closure_snapshot(
        &mut self,
        value: &Value,
        span: Span,
        depth: usize,
        active: &mut HashSet<ClosureIdentity>,
        completed: &mut HashSet<ClosureIdentity>,
        type_scope: &BTreeSet<DeclarationId>,
    ) -> Result<(), Error> {
        if depth >= 128 || self.required_steps == 0 {
            return Err(error(span, "closure snapshot graph budget exhausted"));
        }
        self.required_steps -= 1;
        match value {
            Value::ComptimeReference(_) => {
                return Err(error(
                    span,
                    "active comptime value reference cannot escape in a closure snapshot",
                ));
            }
            Value::Storage(cell) => {
                let mut steps = self.required_steps;
                let checked = cell.inspect_owned(&mut steps, 128, |value, steps| {
                    self.required_steps = *steps;
                    let result = self.validate_closure_snapshot(
                        value,
                        span,
                        depth + 1,
                        active,
                        completed,
                        type_scope,
                    );
                    *steps = self.required_steps;
                    result
                });
                self.required_steps = steps;
                return checked
                    .map_err(|cause| error(span, format!("capture read failed: {cause:?}")))?;
            }
            Value::Record { arguments, .. } | Value::Variant { arguments, .. } => {
                for argument in arguments {
                    self.check_capture_term(argument, type_scope, span)?;
                }
            }
            Value::List { ty, .. } => {
                self.check_capture_domain(ty, type_scope, span)?;
                if self.list_element_type(ty).is_none() {
                    return Err(error(
                        span,
                        "captured list requires its registered collection domain",
                    ));
                }
            }
            Value::Optional { ty, .. }
            | Value::Newtype { ty, .. }
            | Value::ErrorResult { ty, .. } => {
                self.check_capture_domain(ty, type_scope, span)?;
            }
            Value::Tuple(_) | Value::AnonymousRecord(_) | Value::Object(_) => {}
            Value::Closure(closure) => {
                if !closure.ty().is_comptime_callable() {
                    return Err(error(span, "closure snapshot requires comptime captures"));
                }
                let identity = ClosureIdentity::new(closure);
                if completed.contains(&identity) {
                    return Ok(());
                }
                if !active.insert(identity.clone()) {
                    return Err(error(
                        span,
                        "cyclic closure environment cannot be snapshotted",
                    ));
                }
                let result = (|| {
                    let width = closure.captured_values().len();
                    self.required_steps = self
                        .required_steps
                        .checked_sub(width as u64)
                        .ok_or_else(|| error(span, "closure snapshot graph budget exhausted"))?;
                    let captured: BTreeSet<_> =
                        closure.captured_values().map(|(id, _)| id).collect();
                    for (_, capture) in closure.captured_values() {
                        self.validate_closure_snapshot(
                            capture,
                            span,
                            depth + 1,
                            active,
                            completed,
                            type_scope,
                        )?;
                    }
                    let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                    let before = fuel;
                    let checked = nir::visit::expression_in_closure_environment(
                        closure.body(),
                        &mut fuel,
                        |expr, _| {
                            match &expr.kind {
                                K::ConstAddress { .. } => {}
                                K::AddressOf(place) => {
                                    let mut root = place.as_ref();
                                    while let K::Field { value, .. }
                                    | K::AnonymousField { value, .. } = &root.kind
                                    {
                                        root = value;
                                    }
                                    if matches!(&root.kind, K::Local(binding) if captured.contains(binding))
                                    {
                                        return Err("dependent closure input exposes captured storage; shared environment transport is not yet implemented".into());
                                    }
                                }
                                _ => {}
                            }
                            Ok(())
                        },
                    );
                    self.required_steps -= (before - fuel) as u64;
                    checked.map_err(|message| error(span, message))
                })();
                active.remove(&identity);
                if result.is_ok() {
                    completed.insert(identity);
                }
                return result;
            }
            Value::Description(data) => {
                return self
                    .refinements
                    .contains(data)
                    .then_some(())
                    .ok_or_else(|| {
                        error(
                            span,
                            "captured refinement belongs to a different declaration world",
                        )
                    });
            }
            Value::Type(ty) => return self.check_capture_domain(ty, type_scope, span),
            Value::TypeParameter(id) => {
                return type_scope.contains(id).then_some(()).ok_or_else(|| {
                    error(
                        span,
                        "captured Type parameter is outside its declaration scope",
                    )
                });
            }
            Value::Unit
            | Value::Bool(_)
            | Value::Char(_)
            | Value::String(_)
            | Value::Symbol(_)
            | Value::Integer(_)
            | Value::UnsignedInteger(_)
            | Value::SignedInteger(_)
            | Value::Float(_)
            | Value::Float16(_)
            | Value::BFloat16(_)
            | Value::Float32(_)
            | Value::Float64(_)
            | Value::BitVec(_) => {
                let mut budget = comptime_runtime::Budget {
                    steps: usize::try_from(self.required_steps).unwrap_or(usize::MAX),
                    max_limbs: 4096,
                };
                let before = budget.steps;
                let checked = value.check_scalar_snapshot(&mut budget);
                self.required_steps -= (before - budget.steps) as u64;
                return checked.map_err(|cause| {
                    error(
                        span,
                        format!("closure snapshot requires completed capture data: {cause:?}"),
                    )
                });
            }
            Value::IirCallable(_)
            | Value::FunctionInstance { .. }
            | Value::RuntimeTraitMethod(_) => {
                let mut budget = comptime_runtime::Budget {
                    steps: usize::try_from(self.required_steps).unwrap_or(usize::MAX),
                    max_limbs: 4096,
                };
                let before = budget.steps;
                let checked = value.validate_callable_identity_snapshot(&mut budget, type_scope);
                self.required_steps -= (before - budget.steps) as u64;
                return checked.map_err(|cause| {
                    error(
                        span,
                        format!(
                            "closure snapshot requires completed callable identity inputs: {cause:?}"
                        ),
                    )
                });
            }
            Value::Function(_) => return Ok(()),
            Value::AbstractValue { .. } | Value::Pointer(_) | Value::Code(_) => {
                return Err(error(
                    span,
                    "dependent closure input needs an immutable capture graph; shared resources and pointer transport are not yet implemented",
                ));
            }
        }
        value.try_for_each_data_field(|domain, child| {
            if let Some(domain) = domain {
                self.check_capture_domain(domain, type_scope, span)?;
            }
            self.validate_closure_snapshot(child, span, depth + 1, active, completed, type_scope)
        })
    }
}
