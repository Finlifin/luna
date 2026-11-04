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
            let mut contract = None;
            for candidate in &self.checked_traits {
                fuel = fuel.checked_sub(1).ok_or(QueryError::Budget)?;
                if candidate.declaration == identity.interface.declaration {
                    contract = Some(candidate);
                    break;
                }
            }
            let contract = contract.ok_or_else(|| {
                QueryError::Structure("unknown materialized trait identity".into())
            })?;
            let mut method = None;
            for candidate in &contract.methods {
                fuel = fuel.checked_sub(1).ok_or(QueryError::Budget)?;
                if candidate.declaration == identity.member {
                    method = Some(candidate);
                    break;
                }
            }
            let method = method.ok_or_else(|| {
                QueryError::Structure("unknown materialized method identity".into())
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
        if let Some((owner, _)) = self.method_instantiation_inputs(&value) {
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
