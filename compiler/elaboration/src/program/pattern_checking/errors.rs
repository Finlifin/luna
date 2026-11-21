//! Identify an error label before binding its payload. The success branch and
//! each distinct applied error type keep their own NIR branch identity.
use super::*;
use hir::pattern::{Pattern, PatternKind as HP};

impl Checker<'_, '_> {
    pub(super) fn error_payload_pattern(
        &mut self,
        inner: &Pattern<'_>,
        ty: &Term,
        span: Span,
    ) -> Result<n::Pattern, Error> {
        let Term::ErrorResult { errors, .. } = ty else {
            return Err(error(
                span,
                "error pattern requires an error-qualified input",
            ));
        };
        if matches!(inner.kind, HP::Wild) {
            self.required_steps = self
                .required_steps
                .checked_sub(errors.len() as u64)
                .ok_or_else(|| error(span, "error pattern budget exhausted"))?;
            return Ok(n::Pattern::Or(
                errors
                    .iter()
                    .map(|label| n::Pattern::ErrorResult {
                        label: Some(label.clone()),
                        inner: Box::new(n::Pattern::Wild),
                    })
                    .collect(),
            ));
        }
        let (label, record) = if errors.len() == 1 {
            (errors[0].clone(), None)
        } else {
            let mut selected = inner;
            loop {
                self.required_steps = self
                    .required_steps
                    .checked_sub(1)
                    .ok_or_else(|| error(selected.span, "error pattern budget exhausted"))?;
                match &selected.kind {
                    HP::Guard(first, _) | HP::Then(first, _, _) => selected = first,
                    HP::Binding(_, _, Some(first)) => selected = first,
                    _ => break,
                }
            }
            let callee = match &selected.kind {
                HP::Const(value) => *value,
                HP::AppTuple(head, _) | HP::Struct(head, _, _) => {
                    let HP::Const(value) = &head.kind else {
                        return Err(error(
                            span,
                            "error payload requires a type-identifying pattern",
                        ));
                    };
                    *value
                }
                _ => {
                    return Err(error(
                        span,
                        "multiple error types require a type-identifying payload pattern",
                    ));
                }
            };
            let (label, record) = if matches!(selected.kind, HP::Struct(..)) {
                let record = self.named_record_head(callee, None)?;
                (record.ty.clone(), Some(record))
            } else {
                let (label, _) = self.variant_target(callee, None)?.ok_or_else(|| {
                    error(
                        selected.span,
                        "multiple error types require a qualified variant pattern",
                    )
                })?;
                (label, None)
            };
            self.required_steps = self
                .required_steps
                .checked_sub(errors.len() as u64)
                .ok_or_else(|| error(selected.span, "error pattern budget exhausted"))?;
            let label = errors
                .iter()
                .find(|allowed| {
                    self.require_type_identity(&label, allowed, selected.span)
                        .is_ok()
                })
                .cloned()
                .ok_or_else(|| {
                    error(
                        selected.span,
                        "error pattern type is absent from the input error row",
                    )
                })?;
            (label, record)
        };
        Ok(n::Pattern::ErrorResult {
            label: Some(label.clone()),
            inner: Box::new(self.pattern_with_record_head(inner, &label, None, record)?),
        })
    }
}
