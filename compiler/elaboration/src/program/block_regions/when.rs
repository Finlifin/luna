//! Ordered arms retain their lexical, type-fact and delivery boundaries.
//! Conditions and bodies return through the same driver as blocks and ifs.
use super::*;
use driver::{Request, Step};

enum State {
    Ready,
    Probing,
    ProbeCondition { arm: usize, condition: usize },
    ProbeBody,
    Condition,
    Body(Option<n::Expr>),
    Complete,
}

struct Ordinary {
    branch_type: Option<Term>,
    fallback: bool,
    reaches_next_arm: bool,
    selected_exit: bool,
    selection: Option<usize>,
    suppressed: bool,
    comptime: bool,
    in_code_body: bool,
    callee: Option<hir::HirId>,
    preserve_effect: Option<bool>,
}

pub(super) struct Frame<'h> {
    source: Expr<'h>,
    arms: &'h [hir::expr::CondictionArm<'h>],
    destination: Destination,
    ordinary: Option<Ordinary>,
    search: Option<branch_probes::Search>,
    probe: Option<branch_probes::Probe>,
    input_delivery: Option<Destination>,
    state: State,
    branches: Vec<(Option<n::Expr>, CheckedBlock)>,
    scope: LexicalScope,
    branch: Option<LexicalScope>,
    premise: Option<refinements::TypeConditionScope>,
}

impl<'h> Frame<'h> {
    pub(super) fn new(
        checker: &mut Checker<'_, '_>,
        source: &Expr<'h>,
        depth: usize,
    ) -> Result<Self, Error> {
        let H::When(arms) = source.kind else {
            return Err(error(source.span, "contextual when requires ordered arms"));
        };
        if depth >= 128 {
            return Err(error(
                source.span,
                "contextual input nesting limit exceeded",
            ));
        }
        let fallback = arms.iter().position(|arm| arm.cond.is_none());
        if fallback.is_some_and(|index| index + 1 != arms.len()) {
            return Err(error(source.span, "when fallback must be the final arm"));
        }
        if fallback.is_none() {
            return Err(error(
                source.span,
                "contextual when requires a final fallback",
            ));
        }
        checker.required_steps = checker
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(source.span, "when checking budget exhausted"))?;
        Ok(Self {
            source: source.clone(),
            arms,
            destination: Destination::Contextual(ContextualTail { depth }),
            ordinary: None,
            search: None,
            probe: None,
            input_delivery: None,
            state: State::Ready,
            branches: Vec::new(),
            scope: LexicalScope::enter(checker, source.span)?,
            branch: None,
            premise: None,
        })
    }

    pub(super) fn ordinary(
        checker: &mut Checker<'_, '_>,
        source: &Expr<'h>,
        destination: Destination,
        own_effect_delivery: bool,
    ) -> Result<Self, Error> {
        let H::When(arms) = source.kind else {
            return Err(error(source.span, "when requires ordered arms"));
        };
        let fallback = arms.iter().position(|arm| arm.cond.is_none());
        if fallback.is_some_and(|index| index + 1 != arms.len()) {
            return Err(error(source.span, "when fallback must be the final arm"));
        }
        let view = destination.ordinary_view().ok_or_else(|| {
            error(
                source.span,
                "ordinary when requires an ordinary destination",
            )
        })?;
        let search = if view.expected.is_none() && fallback.is_some() {
            view.input.map(|context| {
                branch_probes::Search::new(checker, context, arms.len(), source.span)
            })
        } else {
            None
        };
        let input_delivery = (own_effect_delivery && destination.structural_input().is_some())
            .then(|| destination.clone());
        checker.required_steps = checker
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(source.span, "when checking budget exhausted"))?;
        let scope = LexicalScope::enter(checker, source.span)?;
        let ordinary = Ordinary {
            branch_type: fallback.and_then(|_| destination.expected().cloned()),
            fallback: fallback.is_some(),
            reaches_next_arm: true,
            selected_exit: false,
            selection: checker
                .inline_selection
                .filter(|(id, _)| *id == source.hir_id)
                .map(|(_, index)| index),
            suppressed: checker.in_unselected_inline,
            comptime: checker.in_required_comptime,
            in_code_body: checker.in_code_body,
            callee: checker.comptime_callee_result,
            preserve_effect: (own_effect_delivery && input_delivery.is_none())
                .then(|| std::mem::take(&mut checker.preserve_effect_value)),
        };
        Ok(Self {
            source: source.clone(),
            arms,
            destination,
            ordinary: Some(ordinary),
            state: if search.is_some() {
                State::Probing
            } else {
                State::Ready
            },
            search,
            probe: None,
            input_delivery,
            branches: Vec::new(),
            scope,
            branch: None,
            premise: None,
        })
    }

    pub(super) fn step(
        &mut self,
        checker: &mut Checker<'_, '_>,
        returned: Option<CheckedBlock>,
    ) -> Result<Step<'h>, Error> {
        let state = std::mem::replace(&mut self.state, State::Complete);
        match (state, returned) {
            (State::Probing, None) => self.advance_search(checker),
            (State::ProbeCondition { arm, condition }, Some(value)) => {
                let value = value.ordinary(self.source.span)?;
                checker.assume_source_type_condition(
                    self.arms[condition].cond,
                    &value,
                    condition == arm,
                    self.arms[condition].body.span,
                )?;
                self.probe_conditions(checker, arm, condition + 1)
            }
            (State::ProbeBody, Some(value)) => {
                let result = value.ordinary(self.source.span);
                let value = self
                    .probe
                    .take()
                    .expect("active when probe")
                    .finish(checker, result)?;
                self.search.as_mut().expect("when search").observe(value);
                self.state = State::Probing;
                Ok(Step::Continue)
            }

            (State::Ready, None) => {
                let Some(arm) = self.arms.get(self.branches.len()) else {
                    return self.finish(checker);
                };
                checker.required_steps = checker
                    .required_steps
                    .checked_sub(1)
                    .ok_or_else(|| error(arm.span, "when checking budget exhausted"))?;
                if let Some(ordinary) = &self.ordinary {
                    checker.in_unselected_inline = ordinary.suppressed
                        || ordinary
                            .selection
                            .is_some_and(|selected| selected != self.branches.len());
                }
                if let Some(condition) = arm.cond {
                    if let Some(ordinary) = &self.ordinary
                        && ordinary.selection.is_some()
                    {
                        checker.in_required_comptime = true;
                        checker.in_code_body = false;
                    }
                    let destination = Destination::ordinary(
                        checker,
                        OrdinaryTail {
                            expected: Some(&p(P::Bool)),
                            partial: None,
                            input: None,
                        },
                        condition.span,
                    )?;
                    self.state = State::Condition;
                    Ok(Step::Child(Request::expression(condition, destination)))
                } else {
                    self.body(checker, None)
                }
            }
            (State::Condition, Some(value)) => {
                if let Some(ordinary) = &mut self.ordinary {
                    if ordinary.reaches_next_arm && ordinary.selection.is_none() {
                        ordinary.selected_exit |= value.selected_exit();
                    }
                    checker.in_required_comptime = ordinary.comptime;
                    checker.in_code_body = ordinary.in_code_body;
                }
                let condition = value.ordinary(self.source.span)?;
                self.body(checker, Some(condition))
            }
            (State::Body(condition), Some(value)) => {
                let arm = &self.arms[self.branches.len()];
                let body = if let Some(ordinary) = &mut self.ordinary {
                    ordinary.observe_arm(
                        self.branches.len(),
                        condition.as_ref(),
                        value.selected_exit(),
                    );
                    let mut body = value.ordinary(arm.body.span)?;
                    if ordinary.selection.is_none()
                        && ordinary.branch_type.is_none()
                        && !is_never(&body.ty)
                    {
                        ordinary.branch_type = Some(body.ty.clone());
                    }
                    if !ordinary.fallback {
                        body = checker.discard_result(body, arm.body.span)?;
                    }
                    CheckedBlock::Ordinary(body)
                } else {
                    CheckedBlock::Contextual(n::NormalFormArgument::Block {
                        source: source(arm.span),
                        statements: Vec::new(),
                        result: Some(Box::new(value.contextual(arm.body.span)?)),
                    })
                };
                self.leave_branch(checker);
                if let Some(condition) = &condition {
                    // Later conditions see the failure of every earlier arm,
                    // but never the positive premise used to check its body.
                    checker.assume_source_type_condition(
                        arm.cond,
                        condition,
                        false,
                        arm.body.span,
                    )?;
                }
                self.branches.push((condition, body));
                self.state = State::Ready;
                Ok(Step::Continue)
            }
            _ => Err(error(
                self.source.span,
                "when child result does not match its continuation",
            )),
        }
    }

    fn advance_search(&mut self, checker: &mut Checker<'_, '_>) -> Result<Step<'h>, Error> {
        let search = self.search.as_mut().expect("when search");
        if let Some(request) = search.next(checker)? {
            self.probe = Some(branch_probes::Probe::begin(
                checker,
                self.source.span,
                request,
            )?);
            self.probe_conditions(checker, request.index, 0)
        } else {
            let search = self.search.take().unwrap();
            if let Some(inferred) = search.result() {
                self.destination =
                    self.destination
                        .with_expected(checker, Some(&inferred), self.source.span)?;
                self.ordinary.as_mut().unwrap().branch_type = Some(inferred);
            }
            self.state = State::Ready;
            Ok(Step::Continue)
        }
    }

    fn probe_conditions(
        &mut self,
        checker: &mut Checker<'_, '_>,
        arm: usize,
        start: usize,
    ) -> Result<Step<'h>, Error> {
        // Each candidate sees preceding negative premises and its own positive
        // premise. They belong to the probe checkpoint, not the final arms.
        for index in start..=arm {
            checker.optional_branch_probe_budget = checker
                .optional_branch_probe_budget
                .checked_sub(1)
                .ok_or_else(|| {
                    error(
                        self.source.span,
                        "optional branch inference budget exhausted; add a type annotation",
                    )
                })?;
            if let Some(condition) = self.arms[index].cond {
                let destination = Destination::ordinary(
                    checker,
                    OrdinaryTail {
                        expected: Some(&p(P::Bool)),
                        partial: None,
                        input: None,
                    },
                    condition.span,
                )?;
                self.state = State::ProbeCondition {
                    arm,
                    condition: index,
                };
                return Ok(Step::Child(Request::expression(condition, destination)));
            }
        }
        self.state = State::ProbeBody;
        Ok(Step::Child(Request::expression(
            self.arms[arm].body,
            self.destination.clone(),
        )))
    }

    pub(super) fn recover_probe(
        &mut self,
        checker: &mut Checker<'_, '_>,
        failure: Error,
    ) -> Result<(), Error> {
        let Some(probe) = self.probe.take() else {
            return Err(failure);
        };
        probe.finish(checker, Err(failure))?;
        self.state = State::Probing;
        Ok(())
    }

    fn body(
        &mut self,
        checker: &mut Checker<'_, '_>,
        condition: Option<n::Expr>,
    ) -> Result<Step<'h>, Error> {
        let arm = &self.arms[self.branches.len()];
        if self
            .ordinary
            .as_ref()
            .is_some_and(|ordinary| ordinary.selection.is_some())
            && !checker.checking_open_behavior
            && let Some(condition) = &condition
        {
            checker.check_inline_input_availability(condition, &BTreeSet::new(), arm.body.span)?;
        }
        if let Some(condition) = &condition {
            self.premise = Some(checker.enter_source_type_condition(
                arm.cond,
                condition,
                true,
                arm.body.span,
            )?);
        }
        // Install each exit before any subsequent fallible step. Error unwinding
        // leaves the lexical child first, then the positive type premise.
        self.branch = Some(LexicalScope::enter(checker, arm.span)?);
        let destination = if let Some(ordinary) = &self.ordinary {
            if ordinary.callee == Some(self.source.hir_id) {
                checker.comptime_callee_result = Some(arm.body.hir_id);
            }
            let drop_unselected_expected = checker.dependent_result_instantiation
                && ordinary.selection.is_some_and(|selected| {
                    selected < self.arms.len() && selected != self.branches.len()
                });
            if ordinary.fallback && !drop_unselected_expected {
                self.destination.with_expected(
                    checker,
                    ordinary.branch_type.as_ref(),
                    arm.body.span,
                )?
            } else {
                Destination::ordinary(
                    checker,
                    OrdinaryTail {
                        expected: if drop_unselected_expected {
                            None
                        } else {
                            ordinary.branch_type.as_ref()
                        },
                        partial: None,
                        input: None,
                    },
                    arm.body.span,
                )?
            }
        } else {
            self.destination.clone()
        };
        self.state = State::Body(condition);
        Ok(Step::Child(Request::expression(arm.body, destination)))
    }

    fn finish(&mut self, checker: &mut Checker<'_, '_>) -> Result<Step<'h>, Error> {
        if self.ordinary.is_some() {
            return self.finish_ordinary(checker);
        }
        let mut branches = std::mem::take(&mut self.branches).into_iter().rev();
        let (_, tail) = branches
            .next()
            .ok_or_else(|| error(self.source.span, "when fallback is missing"))?;
        let mut tail = tail.contextual(self.source.span)?;
        for (condition, body) in branches {
            tail = n::NormalFormArgument::If {
                source: source(self.source.span),
                condition: condition.expect("validated ordered arms"),
                then_input: Box::new(body.contextual(self.source.span)?),
                else_input: Box::new(tail),
            };
        }
        Ok(Step::Completed(CheckedBlock::Contextual(tail)))
    }

    fn finish_ordinary(&mut self, checker: &mut Checker<'_, '_>) -> Result<Step<'h>, Error> {
        let ordinary = self.ordinary.as_ref().unwrap();
        // Branch-local premises justify their own deliveries, not the join.
        checker.context = self.scope.context.clone();
        let mut branches = Vec::with_capacity(self.branches.len());
        for (condition, body) in std::mem::take(&mut self.branches) {
            branches.push((condition, body.ordinary(self.source.span)?));
        }
        let unit = || n::Expr {
            ty: p(P::Void),
            source: source(self.source.span),
            kind: K::Unit,
        };
        let selected_body = if let Some(index) = ordinary.selection
            && index != usize::MAX
        {
            Some(if let Some((_, body)) = branches.get(index) {
                constants::charge_copy(body, &mut checker.required_steps, self.source.span)?;
                body.clone()
            } else {
                unit()
            })
        } else {
            None
        };
        let value = match selected_body {
            Some(value) => {
                if let Some(expected) = self.destination.expected() {
                    checker.require(&value.ty, expected, self.source.span)?;
                }
                value
            }
            None => {
                let mut tail = unit();
                for (condition, body) in branches.into_iter().rev() {
                    let Some(condition) = condition else {
                        tail = body;
                        continue;
                    };
                    let ty = if control::conditional_never(&condition, &body.ty, &tail.ty) {
                        p(P::Never)
                    } else if ordinary.fallback && ordinary.branch_type.is_some() {
                        ordinary.branch_type.clone().unwrap()
                    } else if is_never(&body.ty) {
                        tail.ty.clone()
                    } else if is_never(&tail.ty) {
                        body.ty.clone()
                    } else if ordinary.selection == Some(usize::MAX)
                        && self.destination.expected().is_none()
                    {
                        if checker
                            .require(&tail.ty, &body.ty, self.source.span)
                            .is_ok()
                        {
                            body.ty.clone()
                        } else {
                            p(P::Void)
                        }
                    } else {
                        checker.require(&tail.ty, &body.ty, self.source.span)?;
                        body.ty.clone()
                    };
                    tail = n::Expr {
                        ty,
                        source: source(self.source.span),
                        kind: K::If {
                            condition: Box::new(condition),
                            then_branch: Box::new(body),
                            else_branch: Some(Box::new(tail)),
                        },
                    };
                }
                if let Some(expected) = self.destination.expected() {
                    checker.require(&tail.ty, expected, self.source.span)?;
                }
                tail
            }
        };
        let value = if ordinary.preserve_effect == Some(false) {
            checker.run_computation(value, self.destination.expected(), self.source.span)?
        } else {
            value
        };
        let value = if let Some(destination) = &self.input_delivery {
            destination.deliver_input(checker, value, self.source.span)?
        } else {
            value
        };
        Ok(Step::Completed(
            CheckedBlock::Ordinary(value).with_selected_exit(ordinary.selected_exit),
        ))
    }

    fn leave_branch(&mut self, checker: &mut Checker<'_, '_>) {
        if let Some(ordinary) = &self.ordinary {
            checker.comptime_callee_result = ordinary.callee;
            checker.in_unselected_inline = ordinary.suppressed;
            checker.in_required_comptime = ordinary.comptime;
            checker.in_code_body = ordinary.in_code_body;
        }
        if let Some(scope) = self.branch.take() {
            scope.leave(checker);
        }
        if let Some(premise) = self.premise.take() {
            premise.leave(checker);
        }
    }

    pub(super) fn unwind(mut self, checker: &mut Checker<'_, '_>) {
        debug_assert!(
            self.probe.is_none(),
            "finish the attempt before leaving its frame"
        );
        self.leave_branch(checker);
        self.scope.leave(checker);
    }
}

impl Ordinary {
    fn observe_arm(&mut self, index: usize, condition: Option<&n::Expr>, body_exit: bool) {
        if let Some(selected) = self.selection {
            self.selected_exit |= selected == index && body_exit;
        } else if self.reaches_next_arm {
            if condition.is_some_and(|expr| is_never(&expr.ty)) {
                self.reaches_next_arm = false;
                return;
            }
            let choice = condition.map_or(Some(true), control::boolean);
            self.selected_exit |= choice != Some(false) && body_exit;
            self.reaches_next_arm = choice != Some(true);
        }
    }
}

impl std::fmt::Display for Ordinary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(when-delivery (fallback {}) (reaches-next-arm {}) (selected-exit {}) (suppressed {}) (branch-type",
            self.fallback, self.reaches_next_arm, self.selected_exit, self.suppressed
        )?;
        if let Some(ty) = &self.branch_type {
            write!(f, " {ty}")?;
        }
        f.write_str(") (selection")?;
        if let Some(selection) = self.selection {
            write!(f, " {selection}")?;
        }
        f.write_str(") (callee")?;
        if let Some(callee) = self.callee {
            write!(f, " {callee}")?;
        }
        f.write_str(") (preserve-effect")?;
        if let Some(preserve) = self.preserve_effect {
            write!(f, " {preserve}")?;
        }
        f.write_str("))")
    }
}

impl std::fmt::Display for Frame<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(when-check (source {}) {} (next {}) ",
            source(self.source.span),
            self.destination,
            self.branches.len()
        )?;
        match &self.state {
            State::Ready => f.write_str("(phase ready)"),
            State::Probing => f.write_str("(phase searching)"),
            State::ProbeCondition { arm, condition } => write!(
                f,
                "(phase probe-condition) (arm {arm}) (condition {condition})"
            ),
            State::ProbeBody => f.write_str("(phase probe-body)"),
            State::Condition => f.write_str("(phase condition-child)"),
            State::Body(condition) => {
                f.write_str("(phase body-child) (condition")?;
                if let Some(condition) = condition {
                    write!(f, " {condition}")?;
                }
                f.write_str(")")
            }
            State::Complete => f.write_str("(phase complete)"),
        }?;
        f.write_str(" (search")?;
        if let Some(search) = &self.search {
            write!(f, " {search}")?;
        }
        f.write_str(") (probe")?;
        if let Some(probe) = &self.probe {
            write!(f, " {probe}")?;
        }
        f.write_str(") (input-delivery")?;
        if let Some(destination) = &self.input_delivery {
            write!(f, " {destination}")?;
        }
        f.write_str(") (ordinary")?;
        if let Some(ordinary) = &self.ordinary {
            write!(f, " {ordinary}")?;
        }
        f.write_str(") (branches")?;
        for (condition, body) in &self.branches {
            f.write_str(" (arm (condition")?;
            if let Some(condition) = condition {
                write!(f, " {condition}")?;
            }
            write!(f, ") (body {body}))")?;
        }
        write!(f, ") {} (branch", self.scope)?;
        if let Some(branch) = &self.branch {
            write!(f, " {branch}")?;
        }
        f.write_str(") (premise")?;
        if let Some(premise) = &self.premise {
            write!(f, " {premise}")?;
        }
        f.write_str("))")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_delivery_dump_keeps_selection_and_join_requirements() {
        let delivery = Ordinary {
            branch_type: Some(p(P::Bool)),
            fallback: true,
            reaches_next_arm: false,
            selected_exit: true,
            selection: Some(2),
            suppressed: false,
            comptime: false,
            in_code_body: false,
            callee: None,
            preserve_effect: Some(true),
        };
        assert_eq!(
            delivery.to_string(),
            "(when-delivery (fallback true) (reaches-next-arm false) (selected-exit true) (suppressed false) (branch-type (primitive bool)) (selection 2) (callee) (preserve-effect true))"
        );
    }

    #[test]
    fn dump_keeps_completed_arms_and_pending_condition_separate() {
        let source_expr = Expr {
            hir_id: hir::HirId::INVALID,
            span: Span::default(),
            kind: H::When(&[]),
        };
        let condition = n::Expr {
            source: source(source_expr.span),
            ty: p(P::Bool),
            kind: K::Literal(n::Literal::Bool(false)),
        };
        let frame = Frame {
            source: source_expr.clone(),
            arms: &[],
            destination: Destination::Contextual(ContextualTail { depth: 3 }),
            ordinary: None,
            search: None,
            probe: None,
            input_delivery: None,
            state: State::Condition,
            branches: vec![(
                Some(condition),
                CheckedBlock::Contextual(n::NormalFormArgument::Value(n::Expr {
                    source: source(source_expr.span),
                    ty: p(P::Str),
                    kind: K::Literal(n::Literal::String("arm\"\n".into())),
                })),
            )],
            scope: LexicalScope {
                locals: LocalBindings::default(),
                code: LocalBindings::default(),
                patterns: LocalBindings::default(),
                context: crate::ptc::Context::from_package(&n::Package::default()).unwrap(),
                inputs: input_facts::State::default().lexical_boundary(),
            },
            branch: None,
            premise: None,
        };
        let dump = frame.to_string();
        assert!(dump.starts_with("(when-check (source "), "{dump}");
        assert!(
            dump.contains("(next 1) (phase condition-child)") && dump.contains("(depth 3)"),
            "{dump}"
        );
        assert!(dump.contains("(branches (arm (condition "), "{dump}");
        assert!(dump.contains("arm\\\"\\n"), "{dump}");
        assert!(!dump.contains('\n'), "{dump}");
        assert_eq!(dump, frame.to_string());
    }
}
