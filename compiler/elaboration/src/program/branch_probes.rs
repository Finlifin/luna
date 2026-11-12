//! Branch evidence search owns its position; a probe owns its rollback boundary.
use super::*;

#[derive(Clone, Copy)]
pub(super) struct Request {
    pub index: usize,
    defaults: bool,
}

pub(super) struct Search {
    span: Span,
    count: usize,
    index: usize,
    defaults: bool,
    allow_defaults: bool,
    nested: bool,
    found: Option<Term>,
}

impl Search {
    pub(super) fn new(
        checker: &Checker<'_, '_>,
        context: &crate::calls::InputContext,
        count: usize,
        span: Span,
    ) -> Self {
        let enabled = matches!(
            context,
            crate::calls::InputContext::Optional(_)
                | crate::calls::InputContext::ErrorResult { .. }
        );
        Self {
            span,
            count: if enabled { count } else { 0 },
            index: 0,
            defaults: false,
            allow_defaults: enabled && checker.numeric_defaults,
            nested: enabled && checker.probing_optional_branch,
            found: None,
        }
    }

    pub(super) fn next(&mut self, checker: &mut Checker<'_, '_>) -> Result<Option<Request>, Error> {
        if self.found.is_some() {
            return Ok(None);
        }
        if self.index == self.count && !self.defaults && self.allow_defaults {
            self.defaults = true;
            self.index = 0;
        }
        if self.index == self.count {
            return if self.nested {
                Err(context_error(
                    self.span,
                    "optional branches need a result type context",
                ))
            } else {
                Ok(None)
            };
        }
        checker.optional_branch_probe_budget = checker
            .optional_branch_probe_budget
            .checked_sub(1)
            .ok_or_else(|| {
                error(
                    self.span,
                    "optional branch inference budget exhausted; add a type annotation",
                )
            })?;
        let request = Request {
            index: self.index,
            defaults: self.defaults,
        };
        self.index += 1;
        Ok(Some(request))
    }

    pub(super) fn observe(&mut self, value: Option<n::Expr>) {
        if let Some(value) = value
            && !is_never(&value.ty)
            && !matches!(&value.ty, Term::ErrorResult { result, .. } if is_never(result))
        {
            self.found = Some(value.ty);
        }
    }

    pub(super) fn result(self) -> Option<Term> {
        self.found
    }
}

#[must_use = "finish the branch probe before leaving its checking owner"]
pub(super) struct Probe {
    request: Request,
    attempt: checking_attempts::Attempt,
    parent_probing: bool,
}
impl Probe {
    pub(super) fn begin(
        checker: &mut Checker<'_, '_>,
        span: Span,
        request: Request,
    ) -> Result<Self, Error> {
        let parent_probing = std::mem::replace(&mut checker.probing_optional_branch, true);
        let attempt = checking_attempts::Attempt::begin(
            checker,
            span,
            request.defaults,
            CheckingDisposition::Probe,
        );
        match attempt {
            Ok(attempt) => Ok(Self {
                request,
                attempt,
                parent_probing,
            }),
            Err(failure) => {
                checker.probing_optional_branch = parent_probing;
                Err(failure)
            }
        }
    }
    pub(super) fn finish(
        self,
        checker: &mut Checker<'_, '_>,
        value: Result<n::Expr, Error>,
    ) -> Result<Option<n::Expr>, Error> {
        let result = self.attempt.finish(checker, value);
        checker.probing_optional_branch = self.parent_probing;
        result
    }
}
impl std::fmt::Display for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(branch-probe-request (index {}) (defaults {}))",
            self.index, self.defaults
        )
    }
}
impl std::fmt::Display for Search {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(branch-search (source {}) (count {}) (next {}) (defaults {}) (allow-defaults {}) (nested {}) (found",
            source(self.span),
            self.count,
            self.index,
            self.defaults,
            self.allow_defaults,
            self.nested
        )?;
        if let Some(found) = &self.found {
            write!(f, " {found}")?;
        }
        f.write_str("))")
    }
}
impl std::fmt::Display for Probe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(branch-probe {} (parent-probing {}) {})",
            self.request, self.parent_probing, self.attempt
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_dump_preserves_position_policy_and_typed_evidence() {
        let mut search = Search {
            span: Span::default(),
            count: 2,
            index: 1,
            defaults: false,
            allow_defaults: true,
            nested: true,
            found: None,
        };
        for ty in [
            p(P::Never),
            Term::error_result(vec![p(P::Bool)], p(P::Never)),
        ] {
            search.observe(Some(n::Expr {
                source: source(search.span),
                ty,
                kind: K::Literal(n::Literal::Bool(true)),
            }));
            assert!(search.found.is_none());
        }
        search.observe(Some(n::Expr {
            source: source(search.span),
            ty: p(P::Bool),
            kind: K::Literal(n::Literal::Bool(true)),
        }));
        let dump = search.to_string();
        assert!(dump.starts_with("(branch-search "), "{dump}");
        assert!(
            dump.contains("(next 1) (defaults false) (allow-defaults true) (nested true)"),
            "{dump}"
        );
        assert!(dump.contains("(found (primitive bool))"), "{dump}");
        assert_eq!(
            Request {
                index: 1,
                defaults: true
            }
            .to_string(),
            "(branch-probe-request (index 1) (defaults true))"
        );
    }
}
