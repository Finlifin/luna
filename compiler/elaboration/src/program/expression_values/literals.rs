//! Literal and unit values.
use super::*;

impl Checker<'_, '_> {
    #[inline(never)]
    pub(super) fn literals_value_expression(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
        _partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        let (ty, kind) = match &expr.kind {
            H::LiteralConversion { literal, suffix } => {
                return self.literal_conversion(expr, literal, suffix.name.as_str(), expected);
            }
            H::Lit(lit) => match &lit.kind {
                hir::LitKind::Bool(x) => (p(P::Bool), K::Literal(n::Literal::Bool(*x))),
                hir::LitKind::Char(x) => (p(P::Char), K::Literal(n::Literal::Char(*x))),
                hir::LitKind::String(x) => (p(P::Str), K::Literal(n::Literal::String(x.clone()))),
                hir::LitKind::Integer(spelling) => {
                    let value = self.integer_proxy(spelling, false, expected, expr.span)?;
                    (value.ty, value.kind)
                }
                hir::LitKind::Float(spelling) => {
                    let value = self.float_proxy(spelling, false, expected, expr.span)?;
                    (value.ty, value.kind)
                }
                hir::LitKind::Symbol(name) => {
                    if expected.is_none() && !self.numeric_defaults {
                        return Err(context_error(
                            expr.span,
                            "symbol awaits a contextual target before literal defaulting",
                        ));
                    }
                    let shape = expected
                        .map(|ty| self.structural_type(ty, expr.span))
                        .transpose()?;
                    // A bare symbol can inhabit optional Symbol without being
                    // interpreted as an enum path. Preserve every optional layer.
                    if let Some(Term::Optional(_)) = &shape {
                        let mut leaf = shape.as_ref().unwrap().clone();
                        for _ in 0..128 {
                            let Term::Optional(payload) = leaf else { break };
                            leaf = self.structural_type(&payload, expr.span)?;
                        }
                        if leaf == p(P::Symbol) {
                            let value = n::Expr {
                                source: source(expr.span),
                                ty: p(P::Symbol),
                                kind: K::Literal(n::Literal::Symbol(name.as_str().to_owned())),
                            };
                            return self.inject_optional_value(value, expected.unwrap(), expr.span);
                        }
                    }
                    if expected.is_none() || shape.as_ref() == Some(&p(P::Symbol)) {
                        (
                            p(P::Symbol),
                            K::Literal(n::Literal::Symbol(name.as_str().to_owned())),
                        )
                    } else {
                        let value =
                            self.enum_variant(expected.unwrap(), name.as_str(), expr.span)?;
                        (value.ty, value.kind)
                    }
                }
                _ => return Err(error(expr.span, "literal protocol is not implemented")),
            },
            H::Null => {
                let ty = expected.ok_or_else(|| {
                    context_error(expr.span, "null requires an optional target type")
                })?;
                self.optional_payload(ty, expr.span)?;
                (ty.clone(), K::OptionalAbsent)
            }
            H::Unit => {
                let empty_tuple = expected
                    .map(|ty| self.structural_type(ty, expr.span))
                    .transpose()?
                    .is_some_and(|ty| matches!(ty, Term::Tuple(fields) if fields.is_empty()));
                if empty_tuple {
                    (
                        self.tuple_value_type(Vec::new(), expr.span)?,
                        K::Tuple(Vec::new()),
                    )
                } else {
                    (p(P::Void), K::Unit)
                }
            }
            _ => unreachable!("value expression category was checked by the dispatcher"),
        };
        self.finish_value_expression(expr, expected, ty, kind)
    }
}
