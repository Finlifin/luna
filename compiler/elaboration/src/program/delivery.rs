//! Post-child destination rules. A continuation owns its unfinished work and
//! consumes the checked child once; it never evaluates source code itself.
//! Borrowed targets remain within the current checker call, not a frozen task.
use super::*;
use std::borrow::Cow;

enum Delivery<'t> {
    Optional { target: Cow<'t, Term>, span: Span },
    Error { target: Cow<'t, Term>, span: Span },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_delivery_dump_keeps_the_destination_and_escapes_retained_inputs() {
        let payload = Term::NormalForm {
            head: DeclarationId {
                package: 3,
                local: 7,
            },
            sort: Sort::Type,
            arguments: vec![Term::String("input\"\n".into())],
        };
        let target = Term::error_result(vec![p(P::Bool)], payload.clone());
        let state = Step::Child {
            expected: payload.clone(),
            after: Delivery::Error {
                target: Cow::Borrowed(&target),
                span: Span::default(),
            },
        };
        let dump = state.to_string();
        assert!(dump.contains(&format!("(expected {payload})")), "{dump}");
        assert!(dump.contains(&format!("(target {target})")), "{dump}");
        assert!(dump.contains("input\\\"\\n"), "{dump}");
        assert!(
            !dump.contains('\n'),
            "retained text must not break dump structure"
        );
    }
}

enum Step<'t> {
    Checked(n::Expr),
    Child { expected: Term, after: Delivery<'t> },
    Unadapted(Cow<'t, Term>),
}

impl std::fmt::Display for Delivery<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (kind, target, span) = match self {
            Self::Optional { target, span } => ("optional", target, span),
            Self::Error { target, span } => ("error", target, span),
        };
        write!(
            f,
            "(delivery {kind} (source {}) (target {target}))",
            source(*span)
        )
    }
}

impl std::fmt::Display for Step<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Checked(value) => write!(f, "(checked {value})"),
            Self::Child { expected, after } => {
                write!(f, "(check-child (expected {expected}) (after {after}))")
            }
            Self::Unadapted(target) => write!(f, "(check-unadapted (target {target}))"),
        }
    }
}

impl Delivery<'_> {
    fn finish(self, checker: &mut Checker<'_, '_>, value: n::Expr) -> Result<n::Expr, Error> {
        // A child that leaves its function/control region has no value to wrap.
        if is_never(&value.ty) {
            return Ok(value);
        }
        match self {
            Self::Optional { target, span } => Ok(n::Expr {
                source: source(span),
                ty: target.into_owned(),
                kind: K::OptionalPresent(Box::new(value)),
            }),
            Self::Error { target, span } => {
                let Term::ErrorResult { result, .. } = target.as_ref() else {
                    return Err(error(
                        span,
                        "error delivery requires an error-result destination",
                    ));
                };
                let (value, conversion) = if matches!(value.ty, Term::ErrorResult { .. }) {
                    (value, nir::error_results::Conversion::Embed)
                } else {
                    // Optional success payloads retain their own presence layers.
                    let value = if checker.optional_payload(result, span).is_ok() {
                        checker.inject_optional_value(value, result, span)?
                    } else {
                        value
                    };
                    (value, nir::error_results::Conversion::Success)
                };
                checker.convert_error_value(value, &target, conversion, span)
            }
        }
    }
}

impl Checker<'_, '_> {
    fn charge_expression_delivery(&mut self, span: Span) -> Result<(), Error> {
        self.required_steps = self
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(span, "expression delivery budget exhausted"))?;
        Ok(())
    }

    /// The region enters a branch rule only after its value-delivery protocol.
    /// Two-arm ifs and ordered whens deliver the complete target to their arms.
    /// A one-arm expression with a structured target may instead need a whole
    /// value probe/conversion, so that entry stays with the general adapter.
    pub(super) fn prepare_region_branch_delivery(
        &mut self,
        expr: &Expr<'_>,
        target: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<bool, Error> {
        let directed = match expr.kind {
            H::If(_, _, otherwise) => otherwise.is_some(),
            H::When(_) => true,
            _ => return Ok(false),
        };
        let Some(target) = target.filter(|target| !matches!(target, Term::Primitive(_))) else {
            return Ok(true);
        };
        if !directed {
            return Ok(false);
        }
        self.charge_expression_delivery(expr.span)?;
        match self.prepare_delivery(expr, Cow::Borrowed(target), partial)? {
            Step::Unadapted(_) => Ok(true),
            _ => Err(error(
                expr.span,
                "branch expression lost its directed delivery rule",
            )),
        }
    }

    /// The synchronous adapter drives the same post-child rules. General
    /// expression checking is still an atomic call here; no suspended source
    /// region or external Type response is manufactured by this adapter.
    #[inline(never)]
    pub(super) fn check_delivery(
        &mut self,
        expr: &Expr<'_>,
        target: &Term,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<n::Expr, Error> {
        let mut expected = Cow::Borrowed(target);
        let mut pending = Vec::new();
        let mut value = loop {
            self.charge_expression_delivery(expr.span)?;
            match self.prepare_delivery(expr, expected, partial)? {
                Step::Child {
                    expected: child,
                    after,
                } => {
                    pending.push(after);
                    expected = Cow::Owned(child);
                }
                Step::Checked(value) => break value,
                Step::Unadapted(target) => {
                    break self.expr_context_unadapted(expr, Some(&target), partial)?;
                }
            }
        };
        while let Some(after) = pending.pop() {
            value = after.finish(self, value)?;
        }
        Ok(value)
    }

    fn prepare_delivery<'t>(
        &mut self,
        expr: &Expr<'_>,
        target: Cow<'t, Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<Step<'t>, Error> {
        if matches!(target.as_ref(), Term::Primitive(_)) {
            return Ok(Step::Unadapted(target));
        }
        if let Term::ErrorResult { result, .. } = target.as_ref() {
            // Branching rules pass the whole destination to yielded paths.
            let directed = matches!(
                expr.kind,
                H::Block(_)
                    | H::If(_, _, Some(_))
                    | H::Match(..)
                    | H::When(_)
                    | H::Return(_)
                    | H::ErrorNew(_)
                    | H::ErrorHandle { .. }
                    | H::EffectHandle(..)
            );
            if !directed {
                if let Some(value) = self.argument_attempt(expr, None, partial)? {
                    return Delivery::Error {
                        target,
                        span: expr.span,
                    }
                    .finish(self, value)
                    .map(Step::Checked);
                }
                // This child domain outlives this rule step. Account for its
                // owned copy before retaining it; the outer target is moved.
                let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                let mut fuel = before;
                let copied = nir::copy_budget::clone_term(result, &mut fuel);
                self.required_steps -= (before - fuel) as u64;
                let expected = copied.map_err(|failure| {
                    error(
                        expr.span,
                        format!("expression delivery context budget exhausted: {failure:?}"),
                    )
                })?;
                return Ok(Step::Child {
                    expected,
                    after: Delivery::Error {
                        target,
                        span: expr.span,
                    },
                });
            }
        }
        if self.optional_target_probe != Some(expr.hir_id) {
            if let Ok(payload) = self.optional_payload(&target, expr.span) {
                // Null, branches and contextual enum constructors take the
                // complete destination, not an inferred payload destination.
                let directed = matches!(
                    &expr.kind,
                    H::Null
                        | H::Block(_)
                        | H::If(..)
                        | H::Match(..)
                        | H::When(_)
                        | H::Return(_)
                        | H::EffectRequest(_)
                        | H::ErrorHandle { .. }
                        | H::EffectHandle(..)
                ) || enum_groups::symbol_path(expr).is_some()
                    || matches!(&expr.kind, H::Application(head,_) | H::ExtendedApplication(head,_)
                        if enum_groups::symbol_path(head).is_some());
                if !directed {
                    if let Some(value) = self.argument_attempt(expr, None, partial)? {
                        return self
                            .inject_optional_value(value, &target, expr.span)
                            .map(Step::Checked);
                    }
                    // A producer can itself deliver the whole optional. Keep
                    // this attempt separate from the subsequent payload rule.
                    if matches!(
                        expr.kind,
                        H::Application(..) | H::ExtendedApplication(..) | H::OptionPropagate(_)
                    ) {
                        let saved = self.optional_target_probe.replace(expr.hir_id);
                        let attempt = self.argument_attempt_with_defaults(
                            expr,
                            Some(&target),
                            partial,
                            false,
                        );
                        self.optional_target_probe = saved;
                        if let Ok(Some(value)) = attempt {
                            return Ok(Step::Checked(value));
                        }
                    }
                    return Ok(Step::Child {
                        expected: payload,
                        after: Delivery::Optional {
                            target,
                            span: expr.span,
                        },
                    });
                }
            }
        }
        Ok(Step::Unadapted(target))
    }
}
