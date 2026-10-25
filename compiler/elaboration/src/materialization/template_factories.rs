//! Explicit factory returns use tag storage through ordinary inline return
//! lowering. The transient return slot never stores an unspecialized function.
use super::*;

// A discarded finite template choice needs its selection effects, but no
// callable address. Keep the selection as a tag computation; do not erase
// closures or unresolved producers whose environments are not represented here.
pub(super) fn discard_selections(expression: &mut n::Expr, fuel: &mut usize) -> Result<(), String> {
    iir::visit::expression_mut_pruned(expression, fuel, |expr, fuel| {
        if let K::Discard(value) = &mut expr.kind {
            if matches!(value.ty.callable_signature(), Term::Function { parameters, .. }
                if parameters.iter().any(|parameter| parameter.sort != nir::Sort::Runtime))
                && conditional_calls::scoped_targets_supported(value, fuel)?
            {
                conditional_calls::tag_targets(
                    value,
                    &mut Vec::new(),
                    &Term::Primitive(nir::Primitive::U64),
                    fuel,
                    0,
                )?;
            }
        }
        Ok(true)
    })
}

pub(super) fn charge(expr: &n::Expr, fuel: &mut usize) -> Result<(), String> {
    let mut remaining = *fuel as u64;
    nir::copy_budget::charge_expression(expr, &mut remaining)
        .map_err(|_| "template factory copy budget exhausted")?;
    *fuel = remaining as usize;
    Ok(())
}

pub(super) fn dispatch(
    mut value: n::Expr,
    ty: &Term,
    targets: &[n::Expr],
    next: &mut u32,
    sources: &mut Vec<n::ExpansionSource>,
    fuel: &mut usize,
) -> Result<n::Expr, String> {
    let mut arms = Vec::new();
    for (index, target) in targets.iter().enumerate() {
        charge(target, fuel)?;
        let mut body = target.clone();
        crate::program::inlining::freshen_copied_expression(&mut body, next, sources, fuel)?;
        arms.push(n::MatchArm {
            pattern: if index + 1 == targets.len() {
                P::Wild
            } else {
                P::Literal(n::Literal::UnsignedInteger(index as u128))
            },
            guard: None,
            body,
        });
    }
    let source = value.source;
    let index =
        u32::try_from(sources.len()).map_err(|_| "template dispatch source space exhausted")?;
    sources.push(n::ExpansionSource {
        original: source,
        site: source,
    });
    value.source = n::SourceRange::generated(index);
    Ok(n::Expr {
        source,
        ty: ty.clone(),
        kind: K::Match {
            value: Box::new(value),
            arms,
        },
    })
}

impl Builder<'_> {
    pub(super) fn expand_returning_template(
        &mut self,
        function: &n::Function,
        expression: &mut n::Expr,
    ) -> Result<bool, String> {
        let Term::Function { result, .. } = function.signature.callable_signature() else {
            return Ok(false);
        };
        if !matches!(result.callable_signature(),Term::Function {parameters,..}
            if parameters.iter().any(|p|p.sort!=nir::Sort::Runtime))
            || self.active_staged_inlines.contains(&function.declaration)
        {
            return Ok(false);
        }
        let mut supported = true;
        let mut has_return = false;
        iir::visit::expression_in_closure_environment(
            &function.body,
            &mut self.fuel,
            |expr, _| {
                has_return |= matches!(expr.kind, K::Return(Some(_)));
                Ok(())
            },
        )?;
        if !has_return {
            return Ok(false);
        }
        // Captured/addressed result storage needs its own environment lowering;
        // never turn those uses into a differently typed hidden storage slot.
        if let Some(binding) = function.result_binding {
            for predicate in &function.postconditions {
                iir::visit::expression_pruned(predicate, &mut self.fuel, |expr, fuel| {
                    if matches!(expr.kind, K::Closure { .. } | K::AddressOf(_)) {
                        supported &= !nir::free_bindings::collect(expr, fuel)?.contains(&binding);
                        return Ok(false);
                    }
                    Ok(true)
                })?;
            }
        }
        if !supported {
            return Ok(false);
        }
        // All mutations are private until expansion succeeds.
        charge(&function.body, &mut self.fuel)?;
        for predicate in function
            .preconditions
            .iter()
            .chain(&function.postconditions)
            .chain(function.defaults.iter().map(|(_, value)| value))
        {
            charge(predicate, &mut self.fuel)?;
        }
        nir::type_computations::charge_term(&function.signature, &mut self.fuel)
            .map_err(|error| error.to_string())?;
        let mut transformed = function.clone();
        mutable_callbacks::lower(
            &mut transformed.body,
            &mut self.quote_cursor,
            &mut self.quote_sources,
            &mut self.fuel,
        )?;
        staged::forward_template_aliases(
            &mut transformed.body,
            &mut self.quote_cursor,
            &mut self.quote_sources,
            &mut self.fuel,
        )?;
        iir::visit::expression_in_closure_environment(
            &transformed.body,
            &mut self.fuel,
            |expr, fuel| {
                if let K::Return(Some(value)) = &expr.kind {
                    supported &= conditional_calls::scoped_targets_supported(value, fuel)?;
                }
                Ok(())
            },
        )?;
        if !supported
            || !conditional_calls::scoped_targets_supported(&transformed.body, &mut self.fuel)?
        {
            return Ok(false);
        }
        let tag_type = Term::Primitive(nir::Primitive::U64);
        let mut targets = Vec::new();
        tag_returns(
            &mut transformed.body,
            &mut targets,
            &tag_type,
            &mut self.fuel,
            0,
        )?;
        conditional_calls::tag_targets(
            &mut transformed.body,
            &mut targets,
            &tag_type,
            &mut self.fuel,
            0,
        )?;
        if targets.is_empty() {
            return Ok(false);
        }
        transformed.signature = transformed
            .signature
            .with_callable_result(tag_type.clone())
            .ok_or("template factory signature is not callable")?;
        if let Some(binding) = transformed.result_binding {
            for predicate in &mut transformed.postconditions {
                iir::visit::expression_mut_pruned(predicate, &mut self.fuel, |expr, fuel| {
                    if matches!(expr.kind,K::Local(local) if local==binding) {
                        *expr = dispatch(
                            n::Expr {
                                source: expr.source,
                                ty: tag_type.clone(),
                                kind: K::Local(binding),
                            },
                            result,
                            &targets,
                            &mut self.quote_cursor,
                            &mut self.quote_sources,
                            fuel,
                        )?;
                        return Ok(false);
                    }
                    Ok(true)
                })?;
            }
        }
        charge(expression, &mut self.fuel)?;
        let mut candidate = expression.clone();
        let K::Call { callee, .. } = &mut candidate.kind else {
            return Ok(false);
        };
        callee.ty = callee
            .ty
            .clone()
            .with_callable_result(tag_type.clone())
            .ok_or("template factory callee is not callable")?;
        candidate.ty = tag_type;
        if !self.expand_inline_instance(&transformed, &mut candidate)? {
            return Ok(false);
        }
        candidate = dispatch(
            candidate,
            result,
            &targets,
            &mut self.quote_cursor,
            &mut self.quote_sources,
            &mut self.fuel,
        )?;
        self.normalize_expression(&mut candidate)?;
        *expression = candidate;
        Ok(true)
    }
}

// Capture initializers execute in the enclosing factory. Closure bodies have
// their own returns and must keep their original callable result representation.
fn tag_returns(
    body: &mut n::Expr,
    targets: &mut Vec<n::Expr>,
    tag_type: &Term,
    fuel: &mut usize,
    depth: usize,
) -> Result<(), String> {
    if depth >= 128 {
        return Err("template factory capture depth exhausted".into());
    }
    iir::visit::expression_mut_pruned(body, fuel, |expr, fuel| {
        if let K::Closure { captures, .. } = &mut expr.kind {
            for capture in captures {
                if let Some(value) = &mut capture.value {
                    tag_returns(value, targets, tag_type, fuel, depth + 1)?;
                }
            }
            return Ok(false);
        }
        if let K::Return(Some(value)) = &mut expr.kind {
            conditional_calls::tag_targets(value, targets, tag_type, fuel, 0)?;
        }
        Ok(true)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn factory_return_tags_visit_capture_initializers_but_not_closure_bodies() {
        let source = n::SourceRange { start: 0, end: 1 };
        let ty = Term::Function {
            parameters: vec![],
            result: Box::new(Term::Primitive(nir::Primitive::Bool)),
        };
        let target = |local| n::Expr {
            source,
            ty: ty.clone(),
            kind: K::Function(Id { package: 0, local }),
        };
        let returning = |value| n::Expr {
            source,
            ty: Term::Primitive(nir::Primitive::Never),
            kind: K::Return(Some(Box::new(value))),
        };
        let inner = returning(target(2));
        let mut outer = n::Expr {
            source,
            ty: ty.clone(),
            kind: K::Closure {
                captures: vec![n::Capture {
                    binding: n::LocalId { owner: 0, index: 1 },
                    ty: ty.clone(),
                    value: Some(Box::new(returning(target(1)))),
                }],
                parameters: vec![],
                body: std::sync::Arc::new(inner.clone()),
            },
        };
        let mut targets = vec![];
        tag_returns(
            &mut outer,
            &mut targets,
            &Term::Primitive(nir::Primitive::U64),
            &mut 1000,
            0,
        )
        .unwrap();
        assert_eq!(targets, vec![target(1)]);
        let K::Closure { body, captures, .. } = outer.kind else {
            unreachable!()
        };
        assert_eq!(body.as_ref(), &inner);
        let K::Return(Some(value)) = &captures[0].value.as_ref().unwrap().kind else {
            unreachable!()
        };
        assert!(matches!(
            value.kind,
            K::Literal(n::Literal::UnsignedInteger(0))
        ));
    }
}
