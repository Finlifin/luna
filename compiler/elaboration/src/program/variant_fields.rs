//! Keep enum payload source evaluation separate from the variant layout.
use super::*;

pub(super) fn named_inputs<'h>(
    names: &[String],
    args: &[Arg<'h>],
    span: Span,
) -> Result<Vec<(usize, &'h Expr<'h>)>, Error> {
    let mut remaining: BTreeMap<_, _> = names
        .iter()
        .enumerate()
        .map(|(slot, name)| (name.as_str(), Some(slot)))
        .collect();
    args.iter()
        .map(|arg| {
            let Arg::Named(name, value) = arg else {
                return Err(error(span, "enum payload fields must be fully named"));
            };
            let slot = remaining
                .get_mut(name.name.as_str())
                .ok_or_else(|| error(name.span, "unknown enum payload field"))?;
            let slot = slot
                .take()
                .ok_or_else(|| error(name.span, "enum payload field supplied more than once"))?;
            Ok((slot, *value))
        })
        .collect()
}

impl Checker<'_, '_> {
    pub(super) fn finish_variant_fields(
        &mut self,
        span: Span,
        ty: Term,
        declaration: DeclarationId,
        index: usize,
        mut fields: Vec<n::Expr>,
        explicit_order: &[usize],
    ) -> Result<n::Expr, Error> {
        let ty = if fields.iter().any(|f| is_never(&f.ty)) {
            p(P::Never)
        } else {
            ty
        };
        let mut statements = Vec::new();
        // When the explicit prefix already follows storage order, the variant
        // expression evaluates the same sequence without any temporaries.
        if explicit_order.iter().copied().ne(0..explicit_order.len()) {
            for &slot in explicit_order {
                statements.push(self.prepare_call_operand(&mut fields[slot], span)?);
            }
        }
        let value = n::Expr {
            source: source(span),
            ty,
            kind: K::Variant {
                declaration,
                index,
                fields,
            },
        };
        if statements.is_empty() {
            return Ok(value);
        }
        Ok(n::Expr {
            source: source(span),
            ty: value.ty.clone(),
            kind: K::Block {
                statements,
                result: Some(Box::new(value)),
            },
        })
    }
}
