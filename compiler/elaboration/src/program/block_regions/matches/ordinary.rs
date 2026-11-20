//! Ordinary match delivery retains staged selection and its exit obligations.
//! Selector and body checking are driven by the common match frame.
use super::*;

struct BodyScope {
    staged: BTreeSet<n::LocalId>,
    members: BTreeSet<n::LocalId>,
    callee: Option<hir::HirId>,
}

pub(super) struct Ordinary {
    pub(super) selector: Option<n::Expr>,
    subject: Option<Term>,
    pub(super) behavior_pattern: bool,
    staged_match: bool,
    result: Term,
    selected: Option<usize>,
    bindings: Vec<n::Expr>,
    selection: Option<(hir::HirId, usize)>,
    suppressed: bool,
    callee: Option<hir::HirId>,
    preserve_effect: Option<bool>,
    data: Option<BTreeMap<n::LocalId, Option<n::Expr>>>,
    body: Option<BodyScope>,
}

impl Ordinary {
    pub(super) fn new(
        checker: &mut Checker<'_, '_>,
        expr: &Expr<'_>,
        own_effect_delivery: bool,
    ) -> Result<Self, Error> {
        let input = checker
            .inline_match_input
            .as_ref()
            .filter(|(id, _, _)| *id == expr.hir_id);
        let (selector, subject) = if let Some((_, value, subject)) = input {
            constants::charge_copy(value, &mut checker.required_steps, expr.span)?;
            let before = usize::try_from(checker.required_steps).unwrap_or(usize::MAX);
            let mut fuel = before;
            let subject = nir::copy_budget::clone_term(subject, &mut fuel);
            checker.required_steps -= (before - fuel) as u64;
            let subject = subject.map_err(|failure| {
                error(
                    expr.span,
                    format!("inline match input copy budget exhausted: {failure:?}"),
                )
            })?;
            (Some(value.clone()), Some(subject))
        } else {
            (None, None)
        };
        let own_selection = checker
            .inline_selection
            .is_some_and(|(id, _)| id == expr.hir_id);
        Ok(Self {
            staged_match: subject.is_some() || (checker.in_unselected_inline && own_selection),
            selector,
            subject,
            behavior_pattern: checker.checking_open_behavior && own_selection,
            result: p(P::Never),
            selected: None,
            bindings: Vec::new(),
            selection: checker.inline_selection,
            suppressed: checker.in_unselected_inline,
            callee: checker.comptime_callee_result,
            preserve_effect: own_effect_delivery
                .then(|| std::mem::take(&mut checker.preserve_effect_value)),
            data: None,
            body: None,
        })
    }

    pub(super) fn enter_arm(
        &mut self,
        checker: &mut Checker<'_, '_>,
        span: Span,
    ) -> Result<(), Error> {
        if self.subject.is_some() {
            checker.required_steps = checker
                .required_steps
                .checked_sub(checker.generator_data.len() as u64)
                .ok_or_else(|| error(span, "inline match data snapshot budget exhausted"))?;
            for value in checker.generator_data.values().flatten() {
                constants::charge_copy(value, &mut checker.required_steps, span)?;
            }
            self.data = Some(checker.generator_data.clone());
        }
        Ok(())
    }

    pub(super) fn body_destination(
        &mut self,
        checker: &mut Checker<'_, '_>,
        source: &Expr<'_>,
        destination: &Destination,
        arm: &hir::pattern::PatternArm<'_>,
        index: usize,
        pattern: &n::Pattern,
        parent: &LexicalScope,
    ) -> Result<Destination, Error> {
        if self.staged_match && !checker.checking_open_behavior {
            checker.check_inline_pattern_availability(pattern, arm.span)?;
        }
        if let Some(subject) = &self.subject {
            if self.selected.is_none() {
                if let Some(bindings) = checker.select_inline_pattern(pattern, subject, arm.body)? {
                    self.selected = Some(index);
                    checker.immutable.extend(bindings.keys().copied());
                    self.bindings = checker.materialize_inline_bindings(&bindings, arm.body)?;
                }
            }
            checker.inline_selection = Some((source.hir_id, self.selected.unwrap_or(usize::MAX)));
            checker.in_unselected_inline = self.selected != Some(index);
        }
        let destination = destination.following_branch(checker, &self.result, arm.span)?;
        checker.required_steps = checker
            .required_steps
            .checked_sub(
                checker
                    .open_staged_locals
                    .len()
                    .saturating_add(checker.open_member_locals.len())
                    .saturating_add(checker.locals.len()) as u64,
            )
            .ok_or_else(|| error(arm.span, "match body scope budget exhausted"))?;
        self.body = Some(BodyScope {
            staged: checker.open_staged_locals.clone(),
            members: checker.open_member_locals.clone(),
            callee: checker.comptime_callee_result,
        });
        if self.staged_match {
            checker.open_staged_locals.extend(
                checker
                    .locals
                    .keys()
                    .filter(|id| !parent.locals.contains_key(id))
                    .copied(),
            );
        }
        if self.callee == Some(source.hir_id) {
            checker.comptime_callee_result = Some(arm.body.hir_id);
        }
        Ok(destination)
    }

    pub(super) fn accept_body(
        &mut self,
        checker: &mut Checker<'_, '_>,
        pattern: n::Pattern,
        body: n::Expr,
        input: &Term,
        span: Span,
    ) -> Result<(n::Pattern, CheckedBlock), Error> {
        if !is_never(&body.ty) {
            if !is_never(&self.result) {
                checker.require(&body.ty, &self.result, span)?;
            }
            self.result = body.ty.clone();
        }
        let mut arm = n::MatchArm {
            pattern,
            guard: None,
            body,
        };
        if self.subject.is_none() {
            checker.stabilize_error_success_binding(&mut arm, input, span)?;
        }
        debug_assert!(arm.guard.is_none());
        Ok((arm.pattern, CheckedBlock::Ordinary(arm.body)))
    }

    pub(super) fn finish(
        &mut self,
        checker: &mut Checker<'_, '_>,
        source_expr: &Expr<'_>,
        destination: &Destination,
        value: n::Expr,
        mut checked: Vec<n::MatchArm>,
    ) -> Result<n::Expr, Error> {
        let span = source_expr.span;
        // Synthetic arms may share a source span, but must not share a cleanup
        // identity. Keep diagnostics anchored to the original source.
        checker.required_steps = checker
            .required_steps
            .checked_sub(checked.len() as u64)
            .ok_or_else(|| error(span, "match arm source budget exhausted"))?;
        let mut body_sites = BTreeMap::new();
        for arm in &checked {
            *body_sites
                .entry((arm.body.source.start, arm.body.source.end))
                .or_insert(0usize) += 1;
        }
        for arm in &mut checked {
            let original = arm.body.source;
            if original == source(span) || body_sites[&(original.start, original.end)] > 1 {
                let index = u32::try_from(checker.expansion_sources.len())
                    .map_err(|_| error(span, "match arm source space exhausted"))?;
                checker.expansion_sources.push(n::ExpansionSource {
                    original,
                    site: source(span),
                });
                arm.body.source = n::SourceRange::generated(index);
            }
        }
        let patterns: Vec<_> = checked.iter().map(|arm| &arm.pattern).collect();
        checker.require_match_coverage(&patterns, &value.ty, span)?;
        let result = if self.subject.is_some() {
            let index = self
                .selected
                .ok_or_else(|| error(span, "inline match has no selected arm"))?;
            let body = checked.swap_remove(index).body;
            n::Expr {
                ty: body.ty.clone(),
                source: source(span),
                kind: K::Block {
                    statements: std::mem::take(&mut self.bindings),
                    result: Some(Box::new(body)),
                },
            }
        } else {
            if self.behavior_pattern {
                // Abstract only behavior evidence after coverage and branch
                // facts have used the original typed patterns.
                for arm in &mut checked {
                    checker.abstract_inline_pattern(&mut arm.pattern, span)?;
                }
            }
            checker.stabilize_error_success_patterns(
                &mut checked,
                &value.ty,
                &self.result,
                span,
            )?;
            n::Expr {
                ty: if is_never(&value.ty) {
                    p(P::Never)
                } else {
                    self.result.clone()
                },
                source: source(span),
                kind: K::Match {
                    value: Box::new(value),
                    arms: checked,
                },
            }
        };
        // The former source adapter restored these before ordinary effect
        // delivery. Keep that boundary even when this frame is nested directly.
        checker.inline_selection = self.selection;
        checker.in_unselected_inline = self.suppressed;
        if self.preserve_effect == Some(false) {
            checker.run_computation(result, destination.expected(), span)
        } else {
            Ok(result)
        }
    }

    pub(super) fn leave_body(&mut self, checker: &mut Checker<'_, '_>) {
        if let Some(body) = self.body.take() {
            checker.open_staged_locals = body.staged;
            checker.open_member_locals = body.members;
            checker.comptime_callee_result = body.callee;
        }
    }

    pub(super) fn leave_arm(&mut self, checker: &mut Checker<'_, '_>) {
        if let Some(data) = self.data.take() {
            checker.generator_data = data;
        }
    }

    pub(super) fn unwind(self, checker: &mut Checker<'_, '_>) {
        checker.inline_selection = self.selection;
        checker.in_unselected_inline = self.suppressed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dump_distinguishes_staged_selection_bindings_and_pending_scope() {
        let state = Ordinary {
            selector: None,
            subject: Some(Term::String("subject\"\n".into())),
            behavior_pattern: false,
            staged_match: true,
            result: p(P::Bool),
            selected: Some(1),
            bindings: Vec::new(),
            selection: None,
            suppressed: true,
            callee: None,
            preserve_effect: Some(false),
            data: Some(BTreeMap::new()),
            body: Some(BodyScope {
                staged: [n::LocalId { owner: 1, index: 2 }].into(),
                members: BTreeSet::new(),
                callee: None,
            }),
        };
        let dump = state.to_string();
        assert!(dump.contains("(selected 1)"), "{dump}");
        assert!(dump.contains("(suppressed true)"), "{dump}");
        assert!(dump.contains("(data (saved))"), "{dump}");
        assert!(dump.contains("(body-scope (staged (local 1 2))"), "{dump}");
        assert!(dump.contains("subject\\\"\\n"), "{dump}");
        assert!(!dump.contains('\n'), "{dump}");
        assert_eq!(dump, state.to_string());
    }
}

impl std::fmt::Display for Ordinary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(match-delivery (result {}) (behavior-pattern {}) (staged {}) (selected",
            self.result, self.behavior_pattern, self.staged_match
        )?;
        if let Some(index) = self.selected {
            write!(f, " {index}")?;
        }
        f.write_str(") (selector")?;
        if let Some(selector) = &self.selector {
            write!(f, " {selector}")?;
        }
        f.write_str(") (subject")?;
        if let Some(subject) = &self.subject {
            write!(f, " {subject}")?;
        }
        f.write_str(") (bindings")?;
        for binding in &self.bindings {
            write!(f, " {binding}")?;
        }
        f.write_str(") (parent-selection")?;
        if let Some((id, index)) = self.selection {
            write!(f, " {id} {index}")?;
        }
        write!(f, ") (suppressed {}) (callee", self.suppressed)?;
        if let Some(callee) = self.callee {
            write!(f, " {callee}")?;
        }
        f.write_str(") (preserve-effect")?;
        if let Some(preserve) = self.preserve_effect {
            write!(f, " {preserve}")?;
        }
        f.write_str(") (data")?;
        if let Some(data) = &self.data {
            f.write_str(" (saved")?;
            for (binding, value) in data {
                write!(f, " (binding {binding}")?;
                if let Some(value) = value {
                    write!(f, " {value}")?;
                }
                f.write_str(")")?;
            }
            f.write_str(")")?;
        }
        f.write_str(") (body-scope")?;
        if let Some(body) = &self.body {
            f.write_str(" (staged")?;
            for id in &body.staged {
                write!(f, " {id}")?;
            }
            f.write_str(") (members")?;
            for id in &body.members {
                write!(f, " {id}")?;
            }
            f.write_str(") (callee")?;
            if let Some(callee) = body.callee {
                write!(f, " {callee}")?;
            }
            f.write_str(")")?;
        }
        f.write_str("))")
    }
}
