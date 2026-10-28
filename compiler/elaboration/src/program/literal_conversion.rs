use super::*;
use crate::ptc::{Budget, Formula, Outcome};

impl Checker<'_, '_> {
    pub(super) fn literal_conversion(
        &mut self,
        expr: &Expr<'_>,
        literal: &Expr<'_>,
        suffix: &str,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let protocol = self.protocols.literal_into.ok_or_else(|| {
            error(
                expr.span,
                "literal suffix requires a bound LiteralInto protocol",
            )
        })?;
        let source_type = match &literal.kind {
            H::Lit(value) => match value.kind {
                hir::LitKind::Integer(_) => p(P::Integer),
                hir::LitKind::Char(_) => p(P::Char),
                hir::LitKind::String(_) => p(P::Str),
                hir::LitKind::Float(_) => p(P::Float),
                _ => return Err(error(expr.span, "invalid literal conversion source")),
            },
            _ => {
                return Err(error(
                    expr.span,
                    "literal conversion requires a literal source",
                ));
            }
        };
        let inferred;
        let result = match expected {
            Some(result) => result,
            None => {
                inferred = self.literal_result_context(&source_type, protocol, suffix, expr.span)?
                    .ok_or_else(|| context_error(expr.span, "literal conversion result is ambiguous or unresolved; add a result type context"))?;
                &inferred
            }
        };
        let value = self.expr(literal, Some(&source_type))?;
        let callee = self
            .selected_method(
                source_type,
                Some(n::InterfaceInstance {
                    declaration: protocol,
                    arguments: vec![Term::Symbol(suffix.into()), result.clone()],
                }),
                "literal_into",
                expr.span,
            )?
            .ok_or_else(|| error(expr.span, "literal conversion has no selected method"))?;
        self.finish_call(expr, callee, &[], vec![value], expected, None)
    }
}

impl Checker<'_, '_> {
    fn literal_result_context(
        &self,
        subject: &Term,
        protocol: DeclarationId,
        suffix: &str,
        span: Span,
    ) -> Result<Option<Term>, Error> {
        use crate::implementation::Selection;
        let scope = self.call_inference_domains();
        let frames = if scope.is_empty() {
            vec![]
        } else {
            vec![scope.iter().map(|p| p.0).collect()]
        };
        let suffix = Term::Symbol(suffix.into());
        let mut choices = Vec::new();
        for capability in crate::capabilities::capability_sources_with_layouts(
            &self.checked_traits,
            &self.capability_assumptions(),
            Budget::default(),
            &frames,
            self.implementation_index.parameter_domains(),
            Some(self.implementation_index.nominal_layouts()),
        )
        .map_err(|reason| error(span, reason))?
        {
            if capability.interface != protocol {
                continue;
            }
            let [index, result] = capability.interface_arguments.as_slice() else {
                continue;
            };
            if *index != suffix {
                continue;
            }
            if capability.subject != *subject
                && !matches!(
                    self.context.prove(
                        Formula::equal(capability.subject.clone(), subject.clone()),
                        Budget::default()
                    ),
                    Outcome::Entailed(_)
                )
            {
                continue;
            }
            if !choices.contains(result) {
                choices.push(result.clone());
            }
        }
        if !choices.is_empty() {
            return Ok(if choices.len() == 1 {
                choices.pop()
            } else {
                None
            });
        }
        let contract = self
            .checked_traits
            .iter()
            .find(|t| t.declaration == protocol)
            .ok_or_else(|| error(span, "LiteralInto declaration is unavailable"))?;
        let [_, result_parameter] = contract.type_parameters.as_slice() else {
            return Err(error(
                span,
                "LiteralInto requires suffix and result parameters",
            ));
        };
        // This rigid is solely an index wildcard. Each implementation gets its
        // own inference state; no trial introduces facts into the caller.
        let wildcard = Term::Rigid {
            declaration: *result_parameter,
            sort: Sort::Type,
        };
        let candidates = self
            .implementation_index
            .candidates(
                Some(protocol),
                &[suffix.clone(), wildcard],
                subject,
                Budget::default(),
            )
            .map_err(|reason| {
                error(span, format!("literal candidate lookup failed: {reason:?}"))
            })?;
        if candidates.declarations().len() > 128 {
            return Err(error(
                span,
                "literal candidate budget exhausted; add a result type context",
            ));
        }
        for id in candidates.declarations() {
            let implementation = self
                .checked_implementations
                .iter()
                .find(|i| i.declaration == *id)
                .ok_or_else(|| error(span, "literal candidate has no checked implementation"))?;
            let [index, result] = implementation.interface_arguments.as_slice() else {
                return Ok(None);
            };
            if *index != suffix {
                // A concrete mismatch is impossible; a symbolic index has not
                // supplied enough evidence to discard the alternative.
                if matches!(index, Term::Symbol(_)) {
                    continue;
                }
                return Ok(None);
            }
            let signature = Term::Function {
                parameters: vec![Parameter {
                    sort: Sort::Runtime,
                    ty: implementation
                        .subject
                        .abstract_parameters(&[])
                        .map_err(|e| error(span, e.to_string()))?,
                }],
                result: Box::new(
                    result
                        .abstract_parameters(&[])
                        .map_err(|e| error(span, e.to_string()))?,
                ),
            }
            .abstract_types(&implementation.type_parameters)
            .map_err(|e| error(span, e.to_string()))?;
            let scheme = crate::calls::Scheme {
                parameter_sorts: vec![],
                type_parameters: implementation.type_parameters.len(),
                signature,
            };
            let mut candidate = scheme
                .infer_sparse_scoped_types(
                    &[Some(subject.clone())],
                    None,
                    &self.context,
                    Budget::default(),
                    &scope,
                )
                .map_err(|reason| {
                    error(span, format!("literal result inference failed: {reason:?}"))
                })?;
            // Only positive conjunctive equalities propose substitutions.
            // The complete implementation requirements are still checked by
            // capability selection after inference; no disjunct is assumed.
            let mut pending = implementation.requirements.iter().collect::<Vec<_>>();
            let mut equations = Vec::new();
            let mut fuel = Budget::default().steps;
            while let Some(requirement) = pending.pop() {
                fuel = fuel
                    .checked_sub(1)
                    .ok_or_else(|| error(span, "literal requirement inference budget exhausted"))?;
                match requirement {
                    n::TypePredicate::And(parts) => pending.extend(parts),
                    n::TypePredicate::Equal(left, right) => {
                        for term in [left, right] {
                            nir::type_computations::charge_term(term, &mut fuel)
                                .map_err(|e| error(span, e.to_string()))?;
                        }
                        equations.push((
                            left.abstract_types(&implementation.type_parameters)
                                .map_err(|e| error(span, e.to_string()))?,
                            right
                                .abstract_types(&implementation.type_parameters)
                                .map_err(|e| error(span, e.to_string()))?,
                        ));
                    }
                    _ => {}
                }
            }
            candidate
                .constrain_type_equations(
                    &equations,
                    Budget {
                        steps: fuel,
                        ..Budget::default()
                    },
                )
                .map_err(|reason| {
                    error(
                        span,
                        format!("literal requirement inference failed: {reason:?}"),
                    )
                })?;
            if candidate.engine().has_refuted_obligation() {
                continue;
            }
            let inferred = match candidate.finish() {
                Ok(inferred) => inferred,
                Err(crate::calls::Error::NeedsContext)
                | Err(crate::calls::Error::Obligations(crate::obligations::Error::Inference(
                    crate::InferenceError::Unresolved(_),
                ))) => return Ok(None),
                Err(reason) => {
                    return Err(error(
                        span,
                        format!("literal result inference failed: {reason:?}"),
                    ));
                }
            };
            let instance = n::InterfaceInstance {
                declaration: protocol,
                arguments: vec![suffix.clone(), inferred.result.clone()],
            };
            let resolution = self
                .resolve_trait_capability(&instance, subject, Budget::default())
                .map_err(|reason| error(span, reason))?;
            match resolution.selection {
                Selection::Selected(_) => {
                    if !choices.contains(&inferred.result) {
                        choices.push(inferred.result);
                    }
                }
                Selection::NoApplicableCandidate => {}
                _ => return Ok(None),
            }
        }
        Ok(if choices.len() == 1 {
            choices.pop()
        } else {
            None
        })
    }
}
