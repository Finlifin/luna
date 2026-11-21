//! Check the destination of a typed operand before constructing conversion IR.
use super::*;

/// Checked adaptations in the order carried by an already formed operand.
/// A type-only consumer checks the same plan without building executable IR.
#[derive(Default)]
pub(in crate::program) struct ComputedDelivery {
    optionals: Vec<Term>,
    error: Option<nir::error_results::Conversion>,
}

impl std::fmt::Display for ComputedDelivery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(computed-delivery (optional-targets")?;
        for target in &self.optionals {
            write!(f, " {target}")?;
        }
        f.write_str(") (error-conversion")?;
        if let Some(conversion) = self.error {
            write!(f, " {conversion}")?;
        }
        f.write_str("))")
    }
}

impl ComputedDelivery {
    pub(super) fn apply(
        self,
        checker: &mut Checker<'_, '_>,
        value: n::Expr,
        target: &Term,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let value = Checker::wrap_optional_layers(value, self.optionals, span);
        match self.error {
            Some(conversion) => checker.wrap_error_conversion(value, target, conversion, span),
            None => Ok(value),
        }
    }
}

impl Checker<'_, '_> {
    /// Finish delivery when an operand already has a value and its destination
    /// became known later. The source producer must not run again.
    pub(in crate::program) fn deliver_computed_value<'v>(
        &mut self,
        value: &'v n::Expr,
        target: &Term,
        span: Span,
    ) -> Result<Cow<'v, n::Expr>, Error> {
        let plan = self.computed_delivery(&value.ty, target, span)?;
        if plan.optionals.is_empty() && plan.error.is_none() {
            return Ok(Cow::Borrowed(value));
        }
        constants::charge_copy(value, &mut self.required_steps, span)?;
        plan.apply(self, value.clone(), target, span)
            .map(Cow::Owned)
    }

    pub(in crate::program) fn computed_delivery(
        &self,
        actual: &Term,
        target: &Term,
        span: Span,
    ) -> Result<ComputedDelivery, Error> {
        if actual != target {
            if matches!(target, Term::ErrorResult { .. }) {
                return self.computed_error_delivery(actual, target, span);
            }
            if !matches!(target, Term::Primitive(_))
                && matches!(self.type_identity_view(target, span)?, Term::Optional(_))
            {
                return Ok(ComputedDelivery {
                    optionals: self.optional_injection_layers(actual, target, span)?,
                    error: None,
                });
            }
        }
        self.require(actual, target, span)?;
        Ok(ComputedDelivery::default())
    }

    pub(super) fn computed_error_delivery(
        &self,
        actual: &Term,
        target: &Term,
        span: Span,
    ) -> Result<ComputedDelivery, Error> {
        if is_never(actual) {
            return Ok(ComputedDelivery::default());
        }
        let Term::ErrorResult { result, .. } = target else {
            return Err(error(
                span,
                "error delivery requires an error-result destination",
            ));
        };
        let conversion = if matches!(actual, Term::ErrorResult { .. }) {
            nir::error_results::Conversion::Embed
        } else {
            nir::error_results::Conversion::Success
        };
        let optionals = if conversion == nir::error_results::Conversion::Success
            && self.optional_payload(result, span).is_ok()
        {
            self.optional_injection_layers(actual, result, span)?
        } else {
            Vec::new()
        };
        self.check_error_conversion(
            optionals.first().unwrap_or(actual),
            target,
            conversion,
            span,
        )?;
        Ok(ComputedDelivery {
            optionals,
            error: Some(conversion),
        })
    }

    pub(in crate::program) fn convert_error_value(
        &mut self,
        value: n::Expr,
        target: &Term,
        conversion: nir::error_results::Conversion,
        span: Span,
    ) -> Result<n::Expr, Error> {
        self.check_error_conversion(&value.ty, target, conversion, span)?;
        self.wrap_error_conversion(value, target, conversion, span)
    }

    fn check_error_conversion(
        &self,
        actual: &Term,
        target: &Term,
        conversion: nir::error_results::Conversion,
        span: Span,
    ) -> Result<(), Error> {
        nir::error_results::validate_conversion_in_scope(
            conversion,
            actual,
            target,
            self.context.scope(),
            &mut 100_000,
            &mut |a, b| self.require(a, b, span).is_ok(),
        )
        .map_err(|e| {
            error(
                span,
                format!("error result conversion: {e}; input {actual}; output {target}"),
            )
        })
    }

    fn wrap_error_conversion(
        &mut self,
        value: n::Expr,
        target: &Term,
        conversion: nir::error_results::Conversion,
        span: Span,
    ) -> Result<n::Expr, Error> {
        Ok(n::Expr {
            // Conversion and operand are distinct runtime cleanup boundaries,
            // even when an implicit conversion shares the operand's syntax.
            source: self.error_generated_source(span)?,
            ty: target.clone(),
            kind: K::ErrorConvert {
                conversion,
                value: Box::new(value),
            },
        })
    }

    pub(in crate::program) fn inject_optional_value(
        &self,
        value: n::Expr,
        target: &Term,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let layers = self.optional_injection_layers(&value.ty, target, span)?;
        Ok(Self::wrap_optional_layers(value, layers, span))
    }

    fn wrap_optional_layers(mut value: n::Expr, layers: Vec<Term>, span: Span) -> n::Expr {
        for ty in layers.into_iter().rev() {
            value = n::Expr {
                source: source(span),
                ty,
                kind: K::OptionalPresent(Box::new(value)),
            };
        }
        value
    }

    fn optional_injection_layers(
        &self,
        actual: &Term,
        target: &Term,
        span: Span,
    ) -> Result<Vec<Term>, Error> {
        let mut layers = Vec::new();
        let mut inner = target.clone();
        for _ in 0..128 {
            let Err(mismatch) = self.require(actual, &inner, span) else {
                return Ok(layers);
            };
            let Term::Optional(payload) = self.type_identity_view(&inner, span)? else {
                return Err(mismatch);
            };
            layers.push(inner);
            inner = *payload;
        }
        Err(error(span, "optional injection depth exhausted"))
    }
}
