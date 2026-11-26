//! Callable type views retain open binders and observe selected programs at source calls.
use super::*;

impl Checker<'_, '_> {
    pub(in crate::program) fn callable_input_view(
        &self,
        signature: &Term,
        span: Span,
    ) -> Result<Term, Error> {
        self.callable_input_view_budgeted(signature, Budget::default(), span)
    }

    pub(in crate::program) fn callable_input_view_budgeted(
        &self,
        signature: &Term,
        budget: Budget,
        span: Span,
    ) -> Result<Term, Error> {
        self.callable_input_view_metered(signature, &mut { budget })
            .map_err(|failure| error(span, failure))
    }

    pub(in crate::program) fn callable_input_view_with_fuel(
        &self,
        signature: &Term,
        fuel: &mut usize,
        span: Span,
    ) -> Result<Term, Error> {
        let mut budget = Budget {
            steps: *fuel,
            ..Budget::default()
        };
        let result = self.callable_input_view_metered(signature, &mut budget);
        *fuel = budget.steps;
        result.map_err(|failure| error(span, failure))
    }

    pub(in crate::program) fn callable_input_view_metered(
        &self,
        signature: &Term,
        budget: &mut Budget,
    ) -> Result<Term, nir::NominalAdmissionError> {
        self.callable_view_input(signature, budget)?.observe(|ty| {
            let assumptions = self.capability_assumptions();
            crate::capabilities::World {
                index: &self.implementation_index,
                implementations: &self.checked_implementations,
                traits: &self.checked_traits,
                assumptions: &assumptions,
                context: &self.context,
            }
            .associated_type_view_metered(ty, budget)
            .map(|value| value.value)
        })
    }

    pub(in crate::program) fn observe_call_type(
        &mut self,
        signature: &Term,
        site: &Expr<'_>,
    ) -> Result<Term, Error> {
        let mut budget = Budget::default();
        let input = self
            .callable_view_input(signature, &mut budget)
            .map_err(|failure| error(site.span, failure))?;
        input.observe(|ty| {
            let mut environment = CallTypeEnvironment {
                observations: Vec::new(),
                assumptions: self.capability_assumptions(),
                checker: self,
                site,
                invocation_error: None,
            };
            let value = crate::associated::scoped_type_view_in(&mut environment, ty, &mut budget)
                .map(|view| view.value)
                .map_err(|failure| {
                    environment
                        .invocation_error
                        .unwrap_or_else(|| error(site.span, failure))
                })?;
            environment
                .checker
                .observed_associated_types
                .publish(environment.observations, &mut budget.steps)
                .map_err(|failure| error(site.span, failure.to_string()))?;
            Ok(value)
        })
    }

    fn callable_view_input<'t>(
        &self,
        signature: &'t Term,
        budget: &mut Budget,
    ) -> Result<CallableViewInput<'t>, nir::NominalAdmissionError> {
        // This view is also used for complete interface argument vectors.
        // A finished scalar index has no associated Type projection to solve.
        if signature.sort() != nir::Sort::Type
            || signature.is_structural_index_value()
            || !self.interfaces_ready
        {
            crate::associated::charge_projection_copy(signature, &mut budget.steps)?;
            return Ok(CallableViewInput::Unchanged(signature.clone()));
        }
        if !signature
            .substitution_parameters()
            .map_err(|e| e.to_string())?
            .is_empty()
            && self.context.scope() != self.call_input_frames().as_slice()
        {
            let parameters = self.call_inference_scope();
            if parameters.is_empty() {
                return Err("dependent callable has no enclosing function frame".into());
            }
            let scoped = Term::Function {
                parameters,
                result: Box::new(signature.clone()),
            };
            scoped.check_scope(&[]).map_err(|e| e.to_string())?;
            return Ok(CallableViewInput::Scoped(scoped));
        }
        Ok(CallableViewInput::Direct(signature))
    }
}

enum CallableViewInput<'t> {
    Unchanged(Term),
    Direct(&'t Term),
    Scoped(Term),
}

impl CallableViewInput<'_> {
    fn observe<E>(self, view: impl FnOnce(&Term) -> Result<Term, E>) -> Result<Term, E> {
        match self {
            Self::Unchanged(ty) => Ok(ty),
            Self::Direct(ty) => view(ty),
            Self::Scoped(scoped) => {
                let Term::Function { result, .. } = view(&scoped)? else {
                    unreachable!()
                };
                Ok(*result)
            }
        }
    }
}

struct CallTypeEnvironment<'c, 'p, 'a, 'h> {
    observations: Vec<(Term, n::ObservedAssociatedType)>,
    checker: &'c mut Checker<'p, 'a>,
    site: &'c Expr<'h>,
    assumptions: Vec<n::TraitRequirement>,
    invocation_error: Option<Error>,
}

impl crate::associated::TypeProjectionEnvironment for CallTypeEnvironment<'_, '_, '_, '_> {
    fn world(&self) -> crate::capabilities::World<'_> {
        crate::capabilities::World {
            index: &self.checker.implementation_index,
            implementations: &self.checker.checked_implementations,
            traits: &self.checker.checked_traits,
            context: &self.checker.context,
            assumptions: &self.assumptions,
        }
    }

    fn resolve_type(
        &mut self,
        key: &Term,
        budget: &mut Budget,
    ) -> Result<crate::associated::TypeProjection, nir::NominalAdmissionError> {
        use crate::associated::TypeProjection;
        let world = self.world();
        let selection = world.prepare_associated_type_metered(key, budget)?;
        let pending = match selection {
            ready @ TypeProjection::Ready(_) => return Ok(ready),
            TypeProjection::Initializer(pending) => pending,
        };
        let root = match key {
            Term::AssociatedType {
                origin,
                subject,
                arguments,
            } => n::Expr {
                source: source(self.site.span),
                ty: p(P::Type),
                kind: K::AssociatedValue {
                    interface: n::InterfaceInstance {
                        declaration: origin.interface,
                        arguments: arguments.clone(),
                    },
                    subject: subject.as_ref().clone(),
                    member: origin.member,
                },
            },
            Term::AssociatedTypeCall(call) => crate::associated::associated_call_expression(
                world.traits,
                call,
                source(self.site.span),
                world.context.scope(),
                &mut budget.steps,
            )?,
            _ => unreachable!(),
        };
        if pending.needs_inputs() {
            return Ok(TypeProjection::Initializer(pending));
        }
        crate::associated::charge_projection_copy(key, &mut budget.steps)?;
        let read = (|| {
            let (prepared, ()) = self.checker.prepare_required_root_with_live_inputs(
                self.site,
                required::RootKind::AssociatedType,
                Vec::new(),
                false,
                &[],
                |_| Ok((root, ())),
            )?;
            let (value, _) = self.checker.invoke_required_publishing(
                prepared,
                Vec::new(),
                self.site.hir_id.owner,
                self.site.span,
            )?;
            type_data::CompletedType::from_value(value, self.site.span)?.identity(self.site.span)
        })();
        let value = read.map_err(|failure| {
            self.invocation_error = Some(failure);
            nir::NominalAdmissionError::Invalid("associated Type invocation failed".into())
        })?;
        let (result, fact) = pending.observed(value, source(self.site.span), &mut budget.steps)?;
        crate::associated::charge_projection_copy(key, &mut budget.steps)?;
        self.observations.push((key.clone(), fact));
        Ok(TypeProjection::Ready(result))
    }

    fn resolve_comptime_call(
        &mut self,
        call: &nir::ComptimeCall,
        budget: &mut Budget,
    ) -> Result<Option<Term>, nir::NominalAdmissionError> {
        let Some(owner) = self.checker.source_declarations.owner(call.declaration) else {
            return Ok(None);
        };
        let Some(signature) = self
            .checker
            .declared_signature(owner)
            .map(|sig| sig.into_owned())
        else {
            return Ok(None);
        };
        let type_parameters = self
            .checker
            .type_parameters
            .get(&owner)
            .cloned()
            .unwrap_or_default();
        if type_parameters.len() != call.type_arguments.len() {
            return Err(nir::NominalAdmissionError::Invalid(
                "comptime call type argument count mismatch".into(),
            ));
        }
        let callable_ty = self.checker.callable_type(owner, signature);
        crate::associated::charge_projection_copy(&callable_ty, &mut budget.steps)?;
        let instantiated_callable = if type_parameters.is_empty() {
            callable_ty
        } else {
            let param_types = vec![p(P::Type); type_parameters.len()];
            let substitution = nir::DeclarationSubstitution::bounded(
                &type_parameters,
                &param_types,
                &call.type_arguments,
                &[],
                &mut budget.steps,
            )?;
            substitution.instantiate_bounded(&callable_ty, &mut budget.steps)?
        };
        let mut expected_types = Vec::with_capacity(call.arguments.len());
        let _result = instantiated_callable
            .instantiate_function(&call.arguments, |_index, _arg, expected| {
                crate::associated::charge_projection_copy(expected, &mut budget.steps)?;
                expected_types.push(expected.clone());
                Ok::<_, nir::NominalAdmissionError>(())
            })
            .map_err(|error| {
                nir::NominalAdmissionError::Invalid(format!(
                    "comptime call contract mismatch: {error:?}"
                ))
            })?;
        let expr_source = source(self.site.span);
        let callee_kind = if call.type_arguments.is_empty() {
            K::Function(call.declaration)
        } else {
            K::FunctionInstance {
                declaration: call.declaration,
                arguments: call.type_arguments.clone(),
            }
        };
        let callee = n::Expr {
            source: expr_source,
            ty: instantiated_callable,
            kind: callee_kind,
        };
        let arguments = call
            .arguments
            .iter()
            .zip(expected_types)
            .enumerate()
            .map(|(parameter, (arg, param_ty))| {
                let kind = if param_ty == p(P::Type) {
                    K::TypeValue(arg.clone())
                } else {
                    K::ComptimeValue(arg.clone())
                };
                n::CallArgument {
                    passing: n::ArgumentPassing::Value,
                    parameter,
                    is_default: false,
                    value: n::Expr {
                        source: expr_source,
                        ty: param_ty,
                        kind,
                    },
                }
            })
            .collect();
        let root = n::Expr {
            source: expr_source,
            ty: call.ty.clone(),
            kind: K::Call {
                callee: Box::new(callee),
                arguments,
                env_arguments: Vec::new(),
                parameter_bindings: Vec::new(),
            },
        };
        let read = (|| {
            let (prepared, ()) = self.checker.prepare_required_root_with_live_inputs(
                self.site,
                required::RootKind::AssociatedType,
                Vec::new(),
                false,
                &[],
                |_| Ok((root, ())),
            )?;
            let (value, _) = self.checker.invoke_required_publishing(
                prepared,
                Vec::new(),
                self.site.hir_id.owner,
                self.site.span,
            )?;
            match value {
                comptime_interpretor::Value::Type(ty) => Ok(ty),
                other => {
                    let mut rt_budget = comptime_runtime::Budget {
                        steps: budget.steps,
                        max_limbs: 4096,
                    };
                    let term = other.to_normal_term(&mut rt_budget).map_err(|failure| {
                        error(
                            self.site.span,
                            format!("comptime call result export failed: {failure:?}"),
                        )
                    })?;
                    budget.steps = rt_budget.steps;
                    Ok(term)
                }
            }
        })();
        let value = read.map_err(|failure| {
            self.invocation_error = Some(failure);
            nir::NominalAdmissionError::Invalid("comptime call invocation failed".into())
        })?;
        Ok(Some(value))
    }
}
