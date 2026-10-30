//! Layout repair after generic inference has proved semantic input equality.
//! Transport preserves the whole owner, including its outer destructor.
use super::*;

impl Checker<'_, '_> {
    fn error_argument_shape(&self, ty: &Term) -> Term {
        // Only the registered Tuple head supplies a structural product view.
        // Do not force unrelated associated/opaque type queries during layout
        // inspection: their ordinary call obligations may still be pending.
        self.bound_tuple_elements(ty)
            .map(|fields| Term::Tuple(fields.to_vec()))
            .unwrap_or_else(|| ty.clone())
    }

    fn error_argument_layout_needed(
        &mut self,
        source: &Term,
        target: &Term,
        span: Span,
        depth: usize,
    ) -> Result<bool, Error> {
        if depth >= 128 {
            return Err(error(span, "error argument layout nesting limit exceeded"));
        }
        self.required_steps = self
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(span, "error argument layout budget exhausted"))?;
        if source == target {
            return Ok(false);
        }
        match (
            self.error_argument_shape(source),
            self.error_argument_shape(target),
        ) {
            (Term::ErrorResult { .. }, Term::ErrorResult { .. }) => Ok(true),
            (Term::Tuple(a), Term::Tuple(b)) if a.len() == b.len() => {
                for (a, b) in a.iter().zip(&b) {
                    if self.error_argument_layout_needed(a, b, span, depth + 1)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            (Term::Record(a), Term::Record(b))
                if a.len() == b.len() && a.iter().zip(&b).all(|((a, _), (b, _))| a == b) =>
            {
                for ((_, a), (_, b)) in a.iter().zip(&b) {
                    if self.error_argument_layout_needed(a, b, span, depth + 1)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    pub(in crate::program) fn adapt_error_argument_layout(
        &mut self,
        value: &mut n::Expr,
        target: &Term,
        span: Span,
        depth: usize,
    ) -> Result<(), Error> {
        if !self.error_argument_layout_needed(&value.ty, target, span, depth)? {
            return Ok(());
        }
        self.require(&value.ty, target, span)?;
        if matches!(
            (&value.ty, target),
            (Term::ErrorResult { .. }, Term::ErrorResult { .. })
        ) {
            let mut converted = self.convert_error_value(
                value.clone(),
                target,
                nir::error_results::Conversion::Embed,
                span,
            )?;
            converted.source = self.error_generated_source(span)?;
            *value = converted;
            return Ok(());
        }
        let layouts = self.normal_form_metadata();
        nir::error_results::validate_conversion_with_layouts(
            nir::error_results::Conversion::Product,
            &value.ty,
            target,
            &mut 100_000,
            &mut |a, b| self.require(a, b, span).is_ok(),
            &|ty| layouts.iter().find_map(|head| head.tuple_elements(ty)),
        )
        .map_err(|failure| error(span, failure.to_string()))?;
        let original = std::mem::replace(
            value,
            n::Expr {
                source: source(span),
                ty: p(P::Void),
                kind: K::Unit,
            },
        );
        *value = n::Expr {
            source: self.error_generated_source(span)?,
            ty: target.clone(),
            kind: K::ErrorConvert {
                conversion: nir::error_results::Conversion::Product,
                value: Box::new(original),
            },
        };
        Ok(())
    }
}
