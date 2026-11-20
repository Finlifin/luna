//! Conditional rules retain checked children and their checking boundaries.
//! Partial inputs additionally own speculative evidence searches and rollback.
use super::*;
use driver::{Request, Step};
use refinements::TypeConditionScope;

struct Ambient {
    comptime: bool,
    in_code_body: bool,
    suppressed: bool,
    callee: Option<hir::HirId>,
}

impl Ambient {
    fn capture(checker: &Checker<'_, '_>) -> Self {
        Self {
            comptime: checker.in_required_comptime,
            in_code_body: checker.in_code_body,
            suppressed: checker.in_unselected_inline,
            callee: checker.comptime_callee_result,
        }
    }

    fn restore(&self, checker: &mut Checker<'_, '_>) {
        checker.in_required_comptime = self.comptime;
        checker.in_code_body = self.in_code_body;
        checker.in_unselected_inline = self.suppressed;
        checker.comptime_callee_result = self.callee;
    }
}

impl std::fmt::Display for Ambient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(conditional-parent (comptime {}) (suppressed {}) (callee",
            self.comptime, self.suppressed
        )?;
        if let Some(callee) = self.callee {
            write!(f, " {callee}")?;
        }
        f.write_str("))")
    }
}

enum State {
    Ready,
    Condition,
    Inferring(n::Expr, branch_probes::Search),
    Then(n::Expr),
    Else(n::Expr, CheckedBlock, Destination),
    Complete,
}

pub(super) struct Frame<'h> {
    source: Expr<'h>,
    destination: Destination,
    state: State,
    branch: Option<TypeConditionScope>,
    ambient: Option<Ambient>,
    selection: Option<usize>,
    selected_exit: bool,
    // None means the enclosing expression adapter owns effect delivery.
    preserve_effect: Option<bool>,
    partial_input: bool,
    own_input_delivery: bool,
    probe: Option<branch_probes::Probe>,
}

impl<'h> Frame<'h> {
    pub(super) fn new(
        checker: &mut Checker<'_, '_>,
        source: &Expr<'h>,
        depth: usize,
    ) -> Result<Self, Error> {
        if depth >= 128 {
            return Err(error(
                source.span,
                "contextual input nesting limit exceeded",
            ));
        }
        if !matches!(source.kind, H::If(_, _, Some(_))) {
            return Err(error(
                source.span,
                "contextual conditional requires two branches",
            ));
        }
        Self::create(
            checker,
            source,
            Destination::Contextual(ContextualTail { depth }),
            false,
        )
    }

    pub(super) fn ordinary(
        checker: &mut Checker<'_, '_>,
        source: &Expr<'h>,
        destination: Destination,
        own_effect_delivery: bool,
    ) -> Result<Self, Error> {
        if !matches!(source.kind, H::If(..)) || destination.ordinary_view().is_none() {
            return Err(error(
                source.span,
                "ordinary conditional requires an ordinary destination",
            ));
        }
        Self::create(checker, source, destination, own_effect_delivery)
    }

    pub(super) fn partial(
        checker: &mut Checker<'_, '_>,
        source: &Expr<'h>,
        destination: Destination,
        own_input_delivery: bool,
    ) -> Result<Self, Error> {
        if !matches!(source.kind, H::If(_, _, Some(_))) || destination.structural_input().is_none()
        {
            return Err(error(
                source.span,
                "partial conditional requires a structural input",
            ));
        }
        let mut frame = Self::create(checker, source, destination, false)?;
        frame.ambient = None;
        frame.selection = None;
        frame.partial_input = true;
        frame.own_input_delivery = own_input_delivery;
        Ok(frame)
    }

    fn create(
        checker: &mut Checker<'_, '_>,
        source: &Expr<'h>,
        destination: Destination,
        own_effect_delivery: bool,
    ) -> Result<Self, Error> {
        checker.required_steps = checker
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(source.span, "conditional checking budget exhausted"))?;
        let ordinary = matches!(destination, Destination::Ordinary(_));
        Ok(Self {
            source: source.clone(),
            destination,
            state: State::Ready,
            branch: None,
            partial_input: false,
            own_input_delivery: false,
            probe: None,
            ambient: ordinary.then(|| Ambient::capture(checker)),
            selected_exit: false,
            selection: ordinary
                .then(|| {
                    checker
                        .inline_selection
                        .filter(|(id, _)| *id == source.hir_id)
                        .map(|(_, choice)| choice)
                })
                .flatten(),
            preserve_effect: own_effect_delivery
                .then(|| std::mem::take(&mut checker.preserve_effect_value)),
        })
    }

    pub(super) fn step(
        &mut self,
        checker: &mut Checker<'_, '_>,
        returned: Option<CheckedBlock>,
    ) -> Result<Step<'h>, Error> {
        let H::If(condition, yes, no) = self.source.kind else {
            unreachable!("validated conditional")
        };
        if matches!(self.state, State::Inferring(..)) {
            return self.advance_search(checker, returned);
        }
        // Boundaries remain owned by the frame even if conversion or child
        // completion fails after its state has been consumed.
        let state = std::mem::replace(&mut self.state, State::Complete);
        match (state, returned) {
            (State::Ready, None) => {
                let destination = Destination::ordinary(
                    checker,
                    OrdinaryTail {
                        expected: Some(&p(P::Bool)),
                        partial: None,
                        input: None,
                    },
                    condition.span,
                )?;
                if self.ambient.is_some() && self.selection.is_some() {
                    checker.in_required_comptime = true;
                    checker.in_code_body = false;
                }
                self.state = State::Condition;
                Ok(Step::Child(Request::expression(condition, destination)))
            }
            (State::Condition, Some(value)) => {
                self.selected_exit = value.selected_exit();
                let condition = value.ordinary(self.source.span)?;
                self.leave_branch(checker);
                if self.partial_input {
                    let search = branch_probes::Search::new(
                        checker,
                        self.destination
                            .structural_input()
                            .expect("partial destination"),
                        2,
                        self.source.span,
                    );
                    self.state = State::Inferring(condition, search);
                    return self.advance_search(checker, None);
                }
                self.enter_branch(checker, &condition, true, yes.expr.map(|tail| tail.hir_id))?;
                let destination = match &self.destination {
                    Destination::Contextual(tail) => {
                        if tail.depth + 1 >= 128 {
                            return Err(error(yes.span, "contextual input nesting limit exceeded"));
                        }
                        Destination::Contextual(ContextualTail {
                            depth: tail.depth + 1,
                        })
                    }
                    _ if no.is_none() => Destination::statement(),
                    _ => self.destination.clone(),
                };
                self.state = State::Then(condition);
                Ok(Step::Child(Request::Block(yes, destination)))
            }
            (State::Then(condition), Some(value)) => {
                self.leave_branch(checker);
                let Some(no) = no else {
                    self.join_selected_exits(&condition, value.selected_exit(), false);
                    let yes = checker
                        .discard_result(value.ordinary(self.source.span)?, self.source.span)?;
                    return self.finish_ordinary(
                        checker,
                        condition,
                        yes,
                        None,
                        &Destination::statement(),
                    );
                };
                let destination = match &value {
                    CheckedBlock::Ordinary(yes) if self.partial_input && !is_never(&yes.ty) => self
                        .destination
                        .with_expected(checker, Some(&yes.ty), self.source.span)?,
                    CheckedBlock::Ordinary(yes) | CheckedBlock::SelectedExit(yes) => self
                        .destination
                        .following_branch(checker, &yes.ty, self.source.span)?,
                    CheckedBlock::Contextual(_) => self.destination.clone(),
                };
                self.enter_branch(checker, &condition, false, Some(no.hir_id))?;
                self.state = State::Else(condition, value, destination.clone());
                Ok(Step::Child(Request::expression(no, destination)))
            }
            (State::Else(condition, yes, destination), Some(no)) => {
                self.leave_branch(checker);
                self.join_selected_exits(&condition, yes.selected_exit(), no.selected_exit());
                if self.partial_input {
                    self.finish_partial(
                        checker,
                        condition,
                        yes.ordinary(self.source.span)?,
                        no.ordinary(self.source.span)?,
                    )
                } else if matches!(self.destination, Destination::Ordinary(_)) {
                    self.finish_ordinary(
                        checker,
                        condition,
                        yes.ordinary(self.source.span)?,
                        Some(no.ordinary(self.source.span)?),
                        &destination,
                    )
                } else {
                    Ok(Step::Completed(CheckedBlock::Contextual(
                        n::NormalFormArgument::If {
                            source: source(self.source.span),
                            condition,
                            then_input: Box::new(yes.contextual(self.source.span)?),
                            else_input: Box::new(no.contextual(self.source.span)?),
                        },
                    )))
                }
            }
            _ => Err(error(
                self.source.span,
                "conditional child result does not match its continuation",
            )),
        }
    }

    fn advance_search(
        &mut self,
        checker: &mut Checker<'_, '_>,
        returned: Option<CheckedBlock>,
    ) -> Result<Step<'h>, Error> {
        if let Some(probe) = self.probe.take() {
            self.leave_branch(checker);
            let value = returned
                .ok_or_else(|| error(self.source.span, "branch probe child is missing"))
                .and_then(|value| value.ordinary(self.source.span));
            let value = probe.finish(checker, value)?;
            let State::Inferring(_, search) = &mut self.state else {
                unreachable!("probe search")
            };
            search.observe(value);
        } else if returned.is_some() {
            return Err(error(
                self.source.span,
                "branch search received an unsolicited child",
            ));
        }
        let State::Inferring(_, search) = &mut self.state else {
            unreachable!("probe search")
        };
        let request = search.next(checker)?;
        // Install the checkpoint before adding branch facts. If any later
        // operation fails, the driver can still finish this exact attempt.
        if let Some(request) = request {
            self.probe = Some(branch_probes::Probe::begin(
                checker,
                self.source.span,
                request,
            )?);
        }
        let State::Inferring(condition, search) =
            std::mem::replace(&mut self.state, State::Complete)
        else {
            unreachable!("probe search")
        };
        let H::If(_, yes, Some(no)) = self.source.kind else {
            unreachable!("partial conditional")
        };
        if let Some(request) = request {
            let positive = request.index == 0;
            let entered = self.enter_branch(checker, &condition, positive, None);
            self.state = State::Inferring(condition, search);
            entered?;
            Ok(Step::Child(if positive {
                Request::Block(yes, self.destination.clone())
            } else {
                Request::expression(no, self.destination.clone())
            }))
        } else {
            let inferred = search.result();
            if inferred.is_some() {
                self.destination =
                    self.destination
                        .with_expected(checker, inferred.as_ref(), self.source.span)?;
            }
            self.enter_branch(checker, &condition, true, None)?;
            self.state = State::Then(condition);
            Ok(Step::Child(Request::Block(yes, self.destination.clone())))
        }
    }

    pub(super) fn recover_probe(
        &mut self,
        checker: &mut Checker<'_, '_>,
        failure: Error,
    ) -> Result<(), Error> {
        let Some(probe) = self.probe.take() else {
            return Err(failure);
        };
        self.leave_branch(checker);
        probe.finish(checker, Err(failure))?;
        Ok(())
    }

    // Only a branch that the checked condition can enter can account for an
    // exit. The final delivered type must also establish noncompletion.
    fn join_selected_exits(&mut self, condition: &n::Expr, yes: bool, no: bool) {
        if (yes || no) && !is_never(&condition.ty) {
            self.selected_exit |= match self.selection {
                Some(0) => yes,
                Some(1) => no,
                Some(_) => false,
                None => match control::boolean(condition) {
                    Some(true) => yes,
                    Some(false) => no,
                    None => yes || no,
                },
            };
        }
    }

    fn finish_partial(
        &self,
        checker: &mut Checker<'_, '_>,
        condition: n::Expr,
        yes: n::Expr,
        no: n::Expr,
    ) -> Result<Step<'h>, Error> {
        let ty = if is_never(&condition.ty) {
            p(P::Never)
        } else if is_never(&yes.ty) {
            no.ty.clone()
        } else {
            checker.require(&no.ty, &yes.ty, self.source.span)?;
            yes.ty.clone()
        };
        let value = n::Expr {
            ty,
            source: source(self.source.span),
            kind: K::If {
                condition: Box::new(condition),
                then_branch: Box::new(yes),
                else_branch: Some(Box::new(no)),
            },
        };
        // The original structural context remains responsible for error
        // injection even after evidence has supplied a concrete destination.
        let value = if self.own_input_delivery {
            let Some(view) = self.destination.ordinary_view() else {
                unreachable!("partial destination")
            };
            if let Some(crate::calls::InputContext::ErrorResult { errors, .. }) = view.input {
                checker.deliver_error_context(value, errors, self.source.span)?
            } else {
                value
            }
        } else {
            value
        };
        Ok(Step::Completed(
            CheckedBlock::Ordinary(value).with_selected_exit(self.selected_exit),
        ))
    }

    fn enter_branch(
        &mut self,
        checker: &mut Checker<'_, '_>,
        condition: &n::Expr,
        positive: bool,
        delivery: Option<hir::HirId>,
    ) -> Result<(), Error> {
        if let Some(ambient) = &self.ambient {
            checker.in_unselected_inline =
                ambient.suppressed || self.selection == Some(usize::from(positive));
        }
        let H::If(source_condition, _, _) = self.source.kind else {
            unreachable!("validated conditional")
        };
        self.branch = Some(checker.enter_source_type_condition(
            Some(source_condition),
            condition,
            positive,
            self.source.span,
        )?);
        if self
            .ambient
            .as_ref()
            .is_some_and(|parent| parent.callee == Some(self.source.hir_id))
        {
            checker.comptime_callee_result = delivery;
        }
        Ok(())
    }

    fn finish_ordinary(
        &self,
        checker: &mut Checker<'_, '_>,
        condition: n::Expr,
        yes: n::Expr,
        no: Option<n::Expr>,
        branch_destination: &Destination,
    ) -> Result<Step<'h>, Error> {
        let other = no
            .as_ref()
            .map(|value| value.ty.clone())
            .unwrap_or(p(P::Void));
        let context = branch_destination.expected();
        let ty = if control::conditional_never(&condition, &yes.ty, &other) {
            p(P::Never)
        } else if no.is_some() && context.is_some() {
            // Completing branches already justified this destination under
            // their own premises; do not require that equality again outside.
            context.unwrap().clone()
        } else if is_never(&yes.ty) {
            other
        } else if is_never(&other) {
            yes.ty.clone()
        } else {
            checker.require(&other, &yes.ty, self.source.span)?;
            yes.ty.clone()
        };
        let value = checker.finish_value_expression(
            &self.source,
            self.destination.expected(),
            ty,
            K::If {
                condition: Box::new(condition),
                then_branch: Box::new(yes),
                else_branch: no.map(Box::new),
            },
        )?;
        let value = if self.preserve_effect == Some(false) {
            checker.run_computation(value, self.destination.expected(), self.source.span)?
        } else {
            value
        };
        Ok(Step::Completed(
            CheckedBlock::Ordinary(value).with_selected_exit(self.selected_exit),
        ))
    }

    fn leave_branch(&mut self, checker: &mut Checker<'_, '_>) {
        if let Some(parent) = &self.ambient {
            parent.restore(checker);
        }
        if let Some(scope) = self.branch.take() {
            scope.leave(checker);
        }
    }

    pub(super) fn unwind(mut self, checker: &mut Checker<'_, '_>) {
        debug_assert!(
            self.probe.is_none(),
            "finish the attempt before leaving its frame"
        );
        self.leave_branch(checker);
    }
}

impl std::fmt::Display for Frame<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(if-check (source {}) (destination {}) ",
            source(self.source.span),
            self.destination
        )?;
        match &self.state {
            State::Ready => f.write_str("(phase ready)"),
            State::Condition => f.write_str("(phase condition-child)"),
            State::Inferring(condition, search) => {
                write!(f, "(phase inferring) (condition {condition}) {search}")
            }
            State::Then(condition) => write!(f, "(phase then-child) (condition {condition})"),
            State::Else(condition, then_input, destination) => write!(
                f,
                "(phase else-child) (condition {condition}) (then {then_input}) (else-destination {destination})"
            ),
            State::Complete => f.write_str("(phase complete)"),
        }?;
        write!(
            f,
            " (partial-input {}) (input-delivery {}) (selected-exit {}) (probe",
            self.partial_input, self.own_input_delivery, self.selected_exit
        )?;
        if let Some(probe) = &self.probe {
            write!(f, " {probe}")?;
        }
        f.write_str(") (selection")?;
        if let Some(selection) = self.selection {
            write!(f, " {selection}")?;
        }
        let effect = match self.preserve_effect {
            None => "caller",
            Some(true) => "preserve",
            Some(false) => "run",
        };
        write!(f, ") (effect-delivery {effect}) (ambient")?;
        if let Some(ambient) = &self.ambient {
            write!(f, " {ambient}")?;
        }
        f.write_str(") (branch")?;
        if let Some(branch) = &self.branch {
            write!(f, " {branch}")?;
        }
        f.write_str("))")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_else_dump_keeps_checked_condition_then_input_and_destination() {
        let block = hir::Block {
            hir_id: hir::HirId::INVALID,
            stmts: &[],
            expr: None,
            span: Span::default(),
        };
        let leaf = Expr {
            hir_id: hir::HirId::INVALID,
            span: block.span,
            kind: H::Unit,
        };
        let expr = Expr {
            kind: H::If(&leaf, &block, Some(&leaf)),
            ..leaf
        };
        let condition = n::Expr {
            source: source(expr.span),
            ty: p(P::Bool),
            kind: K::Literal(n::Literal::Bool(true)),
        };
        let then_input = n::NormalFormArgument::Value(n::Expr {
            source: source(expr.span),
            ty: p(P::Str),
            kind: K::Literal(n::Literal::String("branch\"\n".into())),
        });
        let frame = Frame {
            source: expr,
            destination: Destination::Contextual(ContextualTail { depth: 7 }),
            ambient: None,
            selection: None,
            selected_exit: false,
            preserve_effect: None,
            state: State::Else(
                condition,
                CheckedBlock::Contextual(then_input),
                Destination::Contextual(ContextualTail { depth: 7 }),
            ),
            branch: None,
            partial_input: false,
            own_input_delivery: false,
            probe: None,
        };
        let dump = frame.to_string();
        assert!(dump.starts_with("(if-check "), "{dump}");
        assert!(dump.contains("(phase else-child)"), "{dump}");
        assert!(
            dump.contains("(condition ") && dump.contains("(then "),
            "{dump}"
        );
        assert!(dump.contains("branch\\\"\\n"), "{dump}");
        assert!(!dump.contains('\n'), "{dump}");
        let request = Step::Child(Request::Block(
            &block,
            Destination::Contextual(ContextualTail { depth: 8 }),
        ))
        .to_string();
        assert!(
            request.starts_with("(check-step (check-block "),
            "{request}"
        );
        assert!(
            request.ends_with("(contextual-tail (depth 8))))"),
            "{request}"
        );
    }
}
