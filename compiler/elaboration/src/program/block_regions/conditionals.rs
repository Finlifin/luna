//! Ordinary and contextual branches retain checked children and their ambient
//! checking boundaries. Only result joining differs between the two rules.
use super::*;
use driver::{Request, Step};
use refinements::TypeConditionScope;

struct Ambient {
    comptime: bool,
    suppressed: bool,
    callee: Option<hir::HirId>,
}
impl Ambient {
    fn capture(checker: &Checker<'_, '_>) -> Self {
        Self {
            comptime: checker.in_required_comptime,
            suppressed: checker.in_unselected_inline,
            callee: checker.comptime_callee_result,
        }
    }
    fn restore(&self, checker: &mut Checker<'_, '_>) {
        checker.in_required_comptime = self.comptime;
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
    Then(n::Expr),
    Else(n::Expr, CheckedBlock, Destination),
    Complete,
}

pub(super) struct Frame<'b, 'h> {
    source: &'b Expr<'h>,
    destination: Destination,
    state: State,
    branch: Option<TypeConditionScope>,
    ambient: Option<Ambient>,
    selection: Option<usize>,
    // None means the enclosing expression adapter owns effect delivery.
    preserve_effect: Option<bool>,
}

impl<'b, 'h> Frame<'b, 'h> {
    pub(super) fn new(
        checker: &mut Checker<'_, '_>,
        source: &'b Expr<'h>,
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
        source: &'b Expr<'h>,
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

    fn create(
        checker: &mut Checker<'_, '_>,
        source: &'b Expr<'h>,
        destination: Destination,
        own_effect_delivery: bool,
    ) -> Result<Self, Error> {
        checker.required_steps = checker
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(source.span, "conditional checking budget exhausted"))?;
        let ordinary = matches!(destination, Destination::Ordinary(_));
        Ok(Self {
            source,
            destination,
            state: State::Ready,
            branch: None,
            ambient: ordinary.then(|| Ambient::capture(checker)),
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
    ) -> Result<Step<'b, 'h>, Error> {
        let H::If(condition, yes, no) = self.source.kind else {
            unreachable!("validated conditional")
        };
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
                if self.ambient.is_some() {
                    checker.in_required_comptime |= self.selection.is_some();
                }
                self.state = State::Condition;
                Ok(Step::Child(Request::Expression(condition, destination)))
            }
            (State::Condition, Some(value)) => {
                let condition = value.ordinary(self.source.span)?;
                self.leave_branch(checker);
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
                    let yes = discard_result(value.ordinary(self.source.span)?);
                    return self.finish_ordinary(
                        checker,
                        condition,
                        yes,
                        None,
                        &Destination::statement(),
                    );
                };
                let destination = match &value {
                    CheckedBlock::Ordinary(yes) => {
                        self.destination
                            .following_branch(checker, &yes.ty, self.source.span)?
                    }
                    CheckedBlock::Contextual(_) => self.destination.clone(),
                };
                self.enter_branch(checker, &condition, false, Some(no.hir_id))?;
                self.state = State::Else(condition, value, destination.clone());
                Ok(Step::Child(Request::Expression(no, destination)))
            }
            (State::Else(condition, yes, destination), Some(no)) => {
                self.leave_branch(checker);
                if matches!(self.destination, Destination::Ordinary(_)) {
                    self.finish_ordinary(
                        checker,
                        condition,
                        yes.ordinary(self.source.span)?,
                        Some(no.ordinary(self.source.span)?),
                        &destination,
                    )
                } else {
                    Ok(Step::Conditional(CheckedBlock::Contextual(
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
        self.branch = Some(checker.enter_type_condition(condition, positive, self.source.span)?);
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
    ) -> Result<Step<'b, 'h>, Error> {
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
            self.source,
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
        Ok(Step::Conditional(CheckedBlock::Ordinary(value)))
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
        self.leave_branch(checker);
    }
}

impl std::fmt::Display for Frame<'_, '_> {
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
            State::Then(condition) => write!(f, "(phase then-child) (condition {condition})"),
            State::Else(condition, then_input, destination) => write!(
                f,
                "(phase else-child) (condition {condition}) (then {then_input}) (else-destination {destination})"
            ),
            State::Complete => f.write_str("(phase complete)"),
        }?;
        f.write_str(" (selection")?;
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
            source: &expr,
            destination: Destination::Contextual(ContextualTail { depth: 7 }),
            ambient: None,
            selection: None,
            preserve_effect: None,
            state: State::Else(
                condition,
                CheckedBlock::Contextual(then_input),
                Destination::Contextual(ContextualTail { depth: 7 }),
            ),
            branch: None,
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
