//! Post-child destination rules. A continuation owns its unfinished work and
//! consumes the checked child once; it never evaluates source code itself.
//! Borrowed targets remain within the current checker call, not a frozen task.
use super::*;
use std::borrow::Cow;
mod computed;

/// The same source rule applies to a partial call context and a settled type.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum TagInput {
    Whole,
    Payload,
    Probe,
    // When the payload has no such member, the receiver can use the ordinary
    // Symbol literal default as soon as the enclosing check enables defaults.
    Receiver,
}

/// A projected Symbol can denote an enum leaf or an ordinary receiver. Only
/// the destination namespace can decide which interpretation supplies a tag.
pub(super) enum PayloadTarget<'a> {
    Type(&'a Term),
    Context(&'a crate::calls::InputContext),
}

impl Checker<'_, '_> {
    // Only built-in literals and their closed structural composition have no
    // producer to run or binding to capture before the destination is known.
    pub(super) fn is_closed_input_literal(&mut self, source: &Expr<'_>) -> Result<bool, Error> {
        let mut pending = vec![source];
        while let Some(expr) = pending.pop() {
            self.required_steps = self
                .required_steps
                .checked_sub(1)
                .ok_or_else(|| error(expr.span, "input literal inspection budget exhausted"))?;
            match &expr.kind {
                H::Lit(_) | H::Unit | H::Null => {}
                H::Unary(UnOp::Neg, _) if is_numeric_literal(expr) => {}
                H::Tuple(items) | H::List(items) => pending.extend(items.iter()),
                H::Block(block) if block.stmts.is_empty() => pending.extend(block.expr),
                H::If(condition, yes, no) if yes.stmts.is_empty() => {
                    pending.push(condition);
                    pending.extend(yes.expr);
                    pending.extend(*no);
                }
                _ => return Ok(false),
            }
        }
        Ok(true)
    }

    fn enum_payload_member(
        &mut self,
        mut target: PayloadTarget<'_>,
        name: &str,
        span: Span,
    ) -> Result<bool, Error> {
        use crate::calls::InputContext as C;
        let declaration = loop {
            self.charge_expression_delivery(span)?;
            match target {
                PayloadTarget::Context(C::Known(ty)) => target = PayloadTarget::Type(ty),
                PayloadTarget::Context(
                    C::Optional(inner) | C::ErrorResult { result: inner, .. },
                ) => target = PayloadTarget::Context(inner),
                PayloadTarget::Context(C::Nominal { declaration, .. }) => break *declaration,
                PayloadTarget::Type(
                    Term::Optional(inner) | Term::ErrorResult { result: inner, .. },
                ) => target = PayloadTarget::Type(inner),
                PayloadTarget::Type(ty) => {
                    let view = self.type_identity_view(ty, span)?;
                    let mut leaf = &view;
                    while let Term::Optional(inner) | Term::ErrorResult { result: inner, .. } = leaf
                    {
                        self.charge_expression_delivery(span)?;
                        leaf = inner;
                    }
                    let Term::Nominal { declaration, .. } = leaf else {
                        return Ok(false);
                    };
                    self.ensure_nominal_type(leaf, span)?;
                    break *declaration;
                }
                _ => return Ok(false),
            }
        };
        self.enum_namespace_contains(declaration, name, span)
    }

    pub(super) fn tag_input(
        &mut self,
        expr: &Expr<'_>,
        target: PayloadTarget<'_>,
    ) -> Result<TagInput, Error> {
        Ok(match &expr.kind {
            H::Tuple(_) | H::List(_) => TagInput::Payload,
            _ if enum_groups::has_bare_symbol_head(expr) => TagInput::Payload,
            H::Null
            | H::Block(_)
            | H::If(..)
            | H::Match(..)
            | H::When(_)
            | H::Return(_)
            | H::EffectRequest(_)
            | H::ErrorHandle { .. }
            | H::EffectHandle(..) => TagInput::Whole,
            _ => {
                let head = match expr.kind {
                    H::Application(head, _) | H::ExtendedApplication(head, _) => head,
                    _ => expr,
                };
                if matches!(head.kind, H::Projection(..))
                    && let Some(name) = enum_groups::symbol_path(head)
                {
                    if self.enum_payload_member(target, &name, expr.span)? {
                        TagInput::Payload
                    } else {
                        TagInput::Receiver
                    }
                } else {
                    TagInput::Probe
                }
            }
        })
    }
}

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
                let plan = checker.computed_error_delivery(&value.ty, &target, span)?;
                plan.apply(checker, value, &target, span)
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
    /// Two-arm ifs, whens and matches deliver the complete target to their arms.
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
            H::When(_) | H::Match(..) => true,
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

    /// Closed callable shapes cannot require optional/error injection. Still
    /// run their ordinary delivery prelude before entering a retained region.
    pub(super) fn prepare_region_callable_delivery(
        &mut self,
        expr: &Expr<'_>,
        target: Option<&Term>,
        partial: Option<&crate::calls::CallableContext>,
    ) -> Result<bool, Error> {
        let Some(target) = target.filter(|target| !matches!(target, Term::Primitive(_))) else {
            return Ok(true);
        };
        if !target.is_comptime_callable() {
            return Ok(false);
        }
        self.charge_expression_delivery(expr.span)?;
        match self.prepare_delivery(expr, Cow::Borrowed(target), partial)? {
            Step::Unadapted(_) => Ok(true),
            _ => Err(error(
                expr.span,
                "callable expression lost its direct delivery rule",
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
                let input = self.tag_input(expr, PayloadTarget::Type(result))?;
                if input != TagInput::Payload
                    && let Some(value) = self.argument_attempt_with_defaults(
                        expr,
                        None,
                        partial,
                        input == TagInput::Receiver && self.numeric_defaults,
                    )?
                {
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
                let input = self.tag_input(expr, PayloadTarget::Type(&payload))?;
                if input != TagInput::Whole {
                    if matches!(input, TagInput::Probe | TagInput::Receiver)
                        && let Some(value) = self.argument_attempt_with_defaults(
                            expr,
                            None,
                            partial,
                            input == TagInput::Receiver && self.numeric_defaults,
                        )?
                    {
                        return self
                            .inject_optional_value(value, &target, expr.span)
                            .map(Step::Checked);
                    }
                    if input == TagInput::Receiver {
                        return Ok(Step::Unadapted(target));
                    }
                    if input == TagInput::Probe
                        && matches!(
                            expr.kind,
                            H::Application(..) | H::ExtendedApplication(..) | H::OptionPropagate(_)
                        )
                    {
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
