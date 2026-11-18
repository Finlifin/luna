//! Public callable contracts and their observed value-context signatures.
use super::*;

impl Checker<'_, '_> {
    /// Rebuild the public method contract when a VM identity re-enters NIR.
    /// The receiving value domain is an observation, not the member signature.
    pub(in crate::program) fn materialized_trait_method(
        &mut self,
        identity: &nir::TraitMethodIdentity,
        expected: &Term,
        span: Span,
    ) -> Result<n::Expr, Error> {
        use nir::type_computations::Error as QueryError;
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let result = (|| {
            fuel = fuel.checked_sub(1).ok_or(QueryError::Budget)?;
            let (contract, method) = self
                .checked_traits
                .method(identity.member)
                .filter(|(interface, _)| interface.declaration == identity.interface.declaration)
                .ok_or_else(|| {
                    QueryError::Structure("unknown materialized trait method identity".into())
                })?;
            nir::member_types::method(
                contract,
                method,
                &identity.subject,
                &identity.interface.arguments,
                &identity.method_arguments,
                |id, fuel| {
                    self.observe_input_domain(id, || {
                        *fuel = fuel.checked_sub(1).ok_or(QueryError::Budget)?;
                        Ok(())
                    })
                },
                &self.call_input_frames(),
                &mut fuel,
            )
        })();
        self.required_steps = fuel as u64;
        let signature = result.map_err(|failure| error(span, failure.to_string()))?;
        let nir::code_interfaces::MethodSignature::Callable(ty) = signature else {
            return Err(error(
                span,
                "contextual method identity is not an ordinary callable",
            ));
        };
        for term in identity.terms() {
            let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
            let mut remaining = before;
            let charged = nir::type_computations::charge_term(term, &mut remaining);
            self.required_steps -= (before - remaining) as u64;
            charged.map_err(|failure| error(span, failure.to_string()))?;
        }
        let value = n::Expr {
            source: source(span),
            ty,
            kind: K::TraitMethod {
                interface: identity.interface.declaration,
                interface_arguments: identity.interface.arguments.clone(),
                subject: identity.subject.clone(),
                member: identity.member,
                method_arguments: identity.method_arguments.clone(),
            },
        };
        self.callable_value_view(value, Some(expected), span)
    }

    /// Expose the selected callable type to value contexts while retaining the
    /// member's declaration signature on the inner node for phase validation.
    pub(in crate::program) fn callable_value_view(
        &self,
        value: n::Expr,
        expected: Option<&Term>,
        span: Span,
    ) -> Result<n::Expr, Error> {
        if let Some((owner, _)) = self.method_instance_shape(&value) {
            if let Some(ItemKind::Fn(signature, _) | ItemKind::FnDeclaration(signature)) =
                self.package.item(owner).map(|item| &item.kind)
            {
                if signature
                    .params
                    .iter()
                    .any(|parameter| parameter.ty.is_implicit() || parameter.ty.is_lambda())
                {
                    arguments::require_function_value_parameters(signature.params, span)?;
                }
            }
        }
        let ty = self.callable_input_view(&value.ty, span)?;
        if let Some(expected) = expected {
            self.require(&ty, expected, span)?;
        }
        if ty == value.ty {
            return Ok(value);
        }
        Ok(n::Expr {
            ty,
            source: value.source,
            kind: K::Block {
                statements: vec![],
                result: Some(Box::new(value)),
            },
        })
    }

    /// Expression rewrites include staged values as well as Type annotations.
    /// Preserve value terms for ordinary domain admission by the invocation.
    pub(in crate::program) fn callable_argument_term_view(
        &mut self,
        term: &Term,
        span: Span,
    ) -> Result<Term, Error> {
        if term.sort() == Sort::Type {
            self.callable_input_view(term, span)
        } else {
            self.copy_alias_type(term, span)
        }
    }

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

    pub(in crate::program) fn callable_input_view_metered(
        &self,
        signature: &Term,
        budget: &mut Budget,
    ) -> Result<Term, nir::NominalAdmissionError> {
        // This view is also used for complete interface argument vectors.
        // A finished scalar index has no associated Type projection to solve.
        if signature.is_structural_index_value() {
            crate::associated::charge_projection_copy(signature, &mut budget.steps)?;
            return Ok(signature.clone());
        }
        if !self.interfaces_ready {
            crate::associated::charge_projection_copy(signature, &mut budget.steps)?;
            return Ok(signature.clone());
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
            let checked = self.callable_input_view_metered(&scoped, budget)?;
            let Term::Function { result, .. } = checked else {
                unreachable!()
            };
            return Ok(*result);
        }
        let assumptions = self.capability_assumptions();
        crate::capabilities::World {
            index: &self.implementation_index,
            implementations: &self.checked_implementations,
            traits: &self.checked_traits,
            assumptions: &assumptions,
            context: &self.context,
        }
        .associated_type_view_metered(signature, budget)
        .map(|value| value.value)
    }
}

/// The source interface can inspect arity without copying identities or
/// interpreting a producer. Public members include Self before either tuple.
enum CallableClauseView<'v> {
    Fixed(&'v [Term]),
    Prepared(&'v n::ClauseOperands),
    Member {
        subject: &'v Term,
        interface: &'v [Term],
        method: &'v [Term],
    },
}

impl CallableClauseView<'_> {
    fn len(&self) -> usize {
        match self {
            Self::Fixed(values) => values.len(),
            Self::Prepared(inputs) => inputs.arguments.len(),
            Self::Member {
                interface, method, ..
            } => 1 + interface.len() + method.len(),
        }
    }

    fn identity(&self, slot: usize) -> Option<&Term> {
        match self {
            Self::Fixed(values) => values.get(slot),
            Self::Prepared(inputs) => inputs.observed_argument(slot),
            Self::Member { subject, .. } if slot == 0 => Some(subject),
            Self::Member {
                interface, method, ..
            } => {
                let slot = slot - 1;
                if slot < interface.len() {
                    interface.get(slot)
                } else {
                    method.get(slot - interface.len())
                }
            }
        }
    }
}

impl Checker<'_, '_> {
    fn method_input_view<'v>(
        &self,
        value: &'v n::Expr,
    ) -> Option<(OwnerId, CallableClauseView<'v>)> {
        let (id, view) = match &value.kind {
            K::ComptimeFunctionInstance {
                declaration,
                inputs,
            } => (*declaration, CallableClauseView::Prepared(inputs)),
            K::FunctionInstance {
                declaration,
                arguments,
            } => (*declaration, CallableClauseView::Fixed(arguments)),
            K::TraitMethod {
                member,
                subject,
                interface_arguments,
                method_arguments,
                ..
            } => (
                *member,
                CallableClauseView::Member {
                    subject,
                    interface: interface_arguments,
                    method: method_arguments,
                },
            ),
            _ => return None,
        };
        Some((self.source_declarations.owner(id)?, view))
    }

    pub(in crate::program) fn method_instance_shape(
        &self,
        value: &n::Expr,
    ) -> Option<(OwnerId, usize)> {
        self.method_input_view(value)
            .map(|(owner, view)| (owner, view.len()))
    }

    pub(in crate::program) fn method_instantiation_inputs(
        &mut self,
        value: &n::Expr,
        span: Span,
    ) -> Result<Option<(OwnerId, Vec<Term>)>, Error> {
        let Some((owner, view)) = self.method_input_view(value) else {
            return Ok(None);
        };
        let width = view.len();
        if matches!(view, CallableClauseView::Prepared(_))
            && self.type_parameters[&owner].len() != width
        {
            return Err(error(span, "invalid comptime clause input packet"));
        }
        self.required_steps = self
            .required_steps
            .checked_sub(width as u64)
            .ok_or_else(|| error(span, "callable clause evidence budget exhausted"))?;
        let mut values = Vec::with_capacity(width);
        for slot in 0..width {
            let value = if let Some(identity) = view.identity(slot) {
                self.copy_alias_type(identity, span)?
            } else {
                let id = self.type_parameters[&owner][slot];
                let mut steps = self.required_steps;
                let domain = self
                    .observe_input_domain(id, || {
                        steps = steps
                            .checked_sub(1)
                            .ok_or(nir::type_computations::Error::Budget)?;
                        Ok(())
                    })
                    .map(|domain| domain.map_or(Sort::Type, Sort::of_comptime_domain));
                self.required_steps = steps;
                Term::Rigid {
                    declaration: id,
                    sort: domain.map_err(|reason| error(span, reason.to_string()))?,
                }
            };
            values.push(value);
        }
        Ok(Some((owner, values)))
    }

    pub(in crate::program) fn incomplete_method_instance(&self, value: &n::Expr) -> bool {
        self.method_instance_shape(value)
            .is_some_and(|(owner, width)| {
                self.type_parameters
                    .get(&owner)
                    .is_some_and(|parameters| width < parameters.len())
            })
    }
}
