//! Required Type computations share generator storage and its failure recovery.
use super::*;

impl Checker<'_, '_> {
    pub(in crate::program) fn generator_required_type(
        &mut self,
        value: &Expr<'_>,
    ) -> Result<Term, Error> {
        let mut evidence = None;
        let completed = self.generator_data_frame_with(value, |this| {
            let checked = this.expr(value, Some(&p(P::Type)))?;
            if this.checking_code_generator {
                evidence = this.generator_type_result_evidence(&checked, value.span)?;
                if evidence.is_none() {
                    let mut fuel = usize::try_from(this.required_steps).unwrap_or(usize::MAX);
                    let free = nir::free_bindings::collect(&checked, &mut fuel);
                    this.required_steps = fuel as u64;
                    let free = free.map_err(|message| error(value.span, message))?;
                    // Closed computations retain ordinary required invocation.
                    // Never supply fabricated values for a caller's live locals.
                    if free.is_empty() {
                        this.validate_generator_data(&checked, value.span)?;
                        constants::charge_copy(&checked, &mut this.required_steps, value.span)?;
                        if let (comptime_interpretor::Value::Type(ty), _) =
                            this.required_normal_input(value, checked.clone(), Vec::new())?
                        {
                            evidence = Some(ty);
                        }
                    }
                }
            }
            Ok(checked)
        })?;
        if let Some((comptime_interpretor::Value::Type(ty), _)) = completed {
            return Ok(ty);
        }
        evidence.ok_or_else(|| error(value.span,
            "contextual Type computation requires a result identity known during definition checking"))
    }

    fn generator_type_result_evidence(
        &mut self,
        value: &n::Expr,
        span: Span,
    ) -> Result<Option<Term>, Error> {
        if let Some(ty) = self.observed_expression_type(value, span)? {
            return Ok(Some(ty));
        }
        let K::Block {
            statements,
            result: Some(result),
        } = &value.kind
        else {
            return Ok(None);
        };
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let checked = (|| {
            // A closed tail identity is unaffected by earlier data writes, but
            // a transfer could bypass it and deliver a different result.
            for statement in statements {
                let mut transfer = false;
                nir::visit::expression(statement, &mut fuel, |node, _| {
                    transfer |= matches!(node.kind, K::Return(_) | K::Break(_) | K::Continue(_));
                    Ok::<_, String>(())
                })?;
                if transfer {
                    return Ok(false);
                }
            }
            Ok(nir::free_bindings::collect(result, &mut fuel)?.is_empty())
        })();
        self.required_steps = fuel as u64;
        if !checked.map_err(|message: String| error(span, message))? {
            return Ok(None);
        }
        self.observed_expression_type(result, span)
    }
}
