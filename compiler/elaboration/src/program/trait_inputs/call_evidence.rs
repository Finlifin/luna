//! Observe already typed ordinary inputs before invoking dependent clauses.
//! This candidate has no values, defaults, staged arguments or executable body.
use super::*;

impl Checker<'_, '_> {
    pub(super) fn observed_call_clause_types_with_prefix(
        &mut self,
        owner: OwnerId,
        site: &Expr<'_>,
        args: &[Arg<'_>],
        fixed_clauses: &[Term],
        receiver_types: &[Option<Term>],
        expected: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
        omitted: &[DeclarationId],
    ) -> Result<Vec<(DeclarationId, Term)>, Error> {
        if omitted.is_empty() {
            return Ok(vec![]);
        }
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let Some(signature) = self.declared_signature(owner) else {
            return Ok(vec![]);
        };
        let charged = nir::type_computations::charge_term(&signature, &mut fuel);
        let signature = charged.map(|()| signature.into_owned());
        self.required_steps = fuel as u64;
        let mut signature = signature.map_err(|failure| error(site.span, failure.to_string()))?;
        let ids = self.type_parameters[&owner].clone();
        if !fixed_clauses.is_empty() {
            if fixed_clauses.len() > ids.len() {
                return Ok(vec![]);
            }
            signature = self.instantiate_input_types(
                &signature,
                &ids[..fixed_clauses.len()],
                fixed_clauses,
                site.span,
            )?;
        }
        let Term::Function { parameters, result } = signature.callable_signature() else {
            return Ok(vec![]);
        };
        let Some(ItemKind::Fn(decl, _) | ItemKind::Case(decl, _) | ItemKind::FnDeclaration(decl)) =
            self.package.item(owner).map(|item| &item.kind)
        else {
            return Ok(vec![]);
        };
        let offset = receiver_types.len();
        if offset > parameters.len() || offset > decl.params.len() {
            return Ok(vec![]);
        }
        let (routes, _) = route_arguments(
            args,
            Some(&decl.params[offset..]),
            parameters.len() - offset,
            site.span,
        )?;
        let other: Vec<_> = ids[fixed_clauses.len()..]
            .iter()
            .filter(|id| !omitted.contains(id))
            .copied()
            .collect();
        // Only independent equations over omitted Type slots are available.
        // An explicit value, a selected default or an ordinary input index
        // can change its domain. Leave those equations for the real call.
        let independent = |this: &mut Self, ty: &Term| -> Result<bool, Error> {
            Ok(ty.check_scope(&[]).is_ok()
                && !input_observation::uses_any(
                    ty,
                    other.iter().copied(),
                    &mut this.required_steps,
                    site.span,
                )?)
        };
        let mut formal = Vec::new();
        let mut source_slots = Vec::new();
        let mut independent_inputs = true;
        for (slot, parameter) in parameters.iter().enumerate() {
            if !input_observation::uses_any(
                &parameter.ty,
                omitted.iter().copied(),
                &mut self.required_steps,
                site.span,
            )? {
                continue;
            }
            if !independent(self, &parameter.ty)? {
                independent_inputs = false;
                continue;
            }
            source_slots.push(slot);
            formal.push(parameter.clone());
        }
        let has_structural_result = self
            .structural_result_context
            .as_ref()
            .is_some_and(|(id, _)| *id == site.hir_id);
        let use_result = expected.is_some() || partial.is_some() || has_structural_result;
        if use_result && !independent(self, result)? {
            return Ok(vec![]);
        }
        let signature = Term::Function {
            parameters: formal,
            result: if use_result {
                result.clone()
            } else {
                Box::new(p(P::Void))
            },
        };
        let binders = self.input_binders(omitted);
        let scheme = crate::calls::Scheme {
            parameter_sorts: binders.iter().map(|p| p.sort).collect(),
            type_parameters: omitted.len(),
            signature: signature
                .abstract_parameters(&binders)
                .map_err(|e| error(site.span, e.to_string()))?,
        };
        let scope = self.call_inference_domains();
        let budget = crate::ptc::Budget::default();
        let initial_types: Vec<_> = source_slots
            .iter()
            .map(|&slot| {
                if slot < offset {
                    receiver_types[slot].clone()
                } else {
                    None
                }
            })
            .collect();
        let Ok(mut candidate) = scheme.infer_sparse_scoped_types_mode(
            &initial_types,
            expected,
            partial,
            &self.context,
            budget,
            self.active_effect_environment().is_some()
                && !matches!(expected, Some(Term::Computation { .. })),
            &scope,
        ) else {
            return Ok(vec![]);
        };
        if let Some((_, shape)) = self
            .structural_result_context
            .as_ref()
            .filter(|(id, _)| *id == site.hir_id)
            && candidate.constrain_result_context(shape, budget).is_err()
        {
            return Ok(vec![]);
        }
        let settled = |candidate: &crate::calls::CallCandidate| {
            let mut evidence = Vec::new();
            for (index, id) in omitted.iter().enumerate() {
                if let Some(ty) = candidate
                    .type_argument(index)
                    .map_err(|e| call_inference_error(site.span, e))?
                {
                    evidence.push((*id, ty));
                }
            }
            Ok(evidence)
        };
        // A result-directed parameter supplies context to its argument. For
        // example, a Packet can be delivered to ?Packet once T is fixed by
        // the result. Equating their unadapted types would discard that fact.
        let from_result = settled(&candidate)?;
        if !independent_inputs || from_result.len() == omitted.len() {
            return Ok(from_result);
        }
        let routes: BTreeMap<_, _> = routes
            .iter()
            .map(|route| (route.parameter, route))
            .collect();
        let mut observations = Vec::new();
        for (index, slot) in source_slots.into_iter().enumerate() {
            if slot < offset {
                continue;
            }
            if candidate
                .parameter_type(index)
                .map_err(|e| call_inference_error(site.span, e))?
                .is_some()
            {
                continue;
            }
            let Some(route) = routes.get(&(slot - offset)) else {
                return Ok(from_result);
            };
            let source = route.expression(site);
            let Some(ty) = self.generator_expression_type_hint(&source, 0)? else {
                return Ok(from_result);
            };
            if !is_never(&ty) {
                observations.push((index, crate::calls::InputContext::Known(ty)));
            }
        }
        for (index, observed) in observations {
            if candidate
                .constrain_parameter_context(index, &observed, budget)
                .is_err()
            {
                return Ok(from_result);
            }
        }
        settled(&candidate)
    }
}
