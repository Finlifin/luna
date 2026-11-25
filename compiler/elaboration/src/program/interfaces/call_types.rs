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
        if signature.is_structural_index_value() || !self.interfaces_ready {
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
}
