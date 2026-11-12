//! Ordered contextual arms retain their own lexical and type-fact boundaries.
//! Conditions and bodies return through the same driver as blocks and ifs.
use super::*;
use driver::{Request, Step};

enum State {
    Ready,
    Condition,
    Body(Option<n::Expr>),
    Complete,
}

pub(super) struct Frame<'b, 'h> {
    source: &'b Expr<'h>,
    arms: &'h [hir::expr::CondictionArm<'h>],
    depth: usize,
    state: State,
    branches: Vec<(Option<n::Expr>, n::NormalFormArgument)>,
    scope: LexicalScope,
    branch: Option<LexicalScope>,
    premise: Option<refinements::TypeConditionScope>,
}

impl<'b, 'h> Frame<'b, 'h> {
    pub(super) fn new(
        checker: &mut Checker<'_, '_>,
        source: &'b Expr<'h>,
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
            source,
            arms,
            depth,
            state: State::Ready,
            branches: Vec::new(),
            scope: LexicalScope::enter(checker, source.span)?,
            branch: None,
            premise: None,
        })
    }

    pub(super) fn step(
        &mut self,
        checker: &mut Checker<'_, '_>,
        returned: Option<CheckedBlock>,
    ) -> Result<Step<'b, 'h>, Error> {
        let state = std::mem::replace(&mut self.state, State::Complete);
        match (state, returned) {
            (State::Ready, None) => {
                let Some(arm) = self.arms.get(self.branches.len()) else {
                    return self.finish();
                };
                checker.required_steps = checker
                    .required_steps
                    .checked_sub(1)
                    .ok_or_else(|| error(arm.span, "when checking budget exhausted"))?;
                if let Some(condition) = arm.cond {
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
                    Ok(Step::Child(Request::Expression(condition, destination)))
                } else {
                    self.body(checker, None)
                }
            }
            (State::Condition, Some(value)) => {
                let condition = value.ordinary(self.source.span)?;
                self.body(checker, Some(condition))
            }
            (State::Body(condition), Some(value)) => {
                let arm = &self.arms[self.branches.len()];
                let body = value.contextual(arm.body.span)?;
                self.leave_branch(checker);
                if let Some(condition) = &condition {
                    // Later conditions see the failure of every earlier arm,
                    // but never the positive premise used to check its body.
                    checker.assume_type_condition(condition, false, arm.body.span)?;
                }
                self.branches.push((
                    condition,
                    n::NormalFormArgument::Block {
                        source: source(arm.span),
                        statements: Vec::new(),
                        result: Some(Box::new(body)),
                    },
                ));
                self.state = State::Ready;
                Ok(Step::Continue)
            }
            _ => Err(error(
                self.source.span,
                "when child result does not match its continuation",
            )),
        }
    }

    fn body(
        &mut self,
        checker: &mut Checker<'_, '_>,
        condition: Option<n::Expr>,
    ) -> Result<Step<'b, 'h>, Error> {
        let arm = &self.arms[self.branches.len()];
        if let Some(condition) = &condition {
            self.premise = Some(checker.enter_type_condition(condition, true, arm.body.span)?);
        }
        // Install each exit before any subsequent fallible step. Error unwinding
        // leaves the lexical child first, then the positive type premise.
        self.branch = Some(LexicalScope::enter(checker, arm.span)?);
        self.state = State::Body(condition);
        Ok(Step::Child(Request::Expression(
            arm.body,
            Destination::Contextual(ContextualTail { depth: self.depth }),
        )))
    }

    fn finish(&mut self) -> Result<Step<'b, 'h>, Error> {
        let mut branches = std::mem::take(&mut self.branches).into_iter().rev();
        let (_, mut tail) = branches
            .next()
            .ok_or_else(|| error(self.source.span, "when fallback is missing"))?;
        for (condition, body) in branches {
            tail = n::NormalFormArgument::If {
                source: source(self.source.span),
                condition: condition.expect("validated ordered arms"),
                then_input: Box::new(body),
                else_input: Box::new(tail),
            };
        }
        Ok(Step::Completed(CheckedBlock::Contextual(tail)))
    }

    fn leave_branch(&mut self, checker: &mut Checker<'_, '_>) {
        if let Some(scope) = self.branch.take() {
            scope.leave(checker);
        }
        if let Some(premise) = self.premise.take() {
            premise.leave(checker);
        }
    }

    pub(super) fn unwind(mut self, checker: &mut Checker<'_, '_>) {
        self.leave_branch(checker);
        self.scope.leave(checker);
    }
}

impl std::fmt::Display for Frame<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(when-check (source {}) (depth {}) (next {}) ",
            source(self.source.span),
            self.depth,
            self.branches.len()
        )?;
        match &self.state {
            State::Ready => f.write_str("(phase ready)"),
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
        f.write_str(" (branches")?;
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
            source: &source_expr,
            arms: &[],
            depth: 3,
            state: State::Condition,
            branches: vec![(
                Some(condition),
                n::NormalFormArgument::Value(n::Expr {
                    source: source(source_expr.span),
                    ty: p(P::Str),
                    kind: K::Literal(n::Literal::String("arm\"\n".into())),
                }),
            )],
            scope: LexicalScope {
                locals: BTreeMap::new(),
                code: BTreeMap::new(),
                patterns: BTreeMap::new(),
                context: crate::ptc::Context::from_package(&n::Package::default()).unwrap(),
                inputs: input_facts::State::default().lexical_boundary(),
            },
            branch: None,
            premise: None,
        };
        let dump = frame.to_string();
        assert!(dump.starts_with("(when-check (source "), "{dump}");
        assert!(
            dump.contains("(depth 3) (next 1) (phase condition-child)"),
            "{dump}"
        );
        assert!(dump.contains("(branches (arm (condition "), "{dump}");
        assert!(dump.contains("arm\\\"\\n"), "{dump}");
        assert!(!dump.contains('\n'), "{dump}");
        assert_eq!(dump, frame.to_string());
    }
}
