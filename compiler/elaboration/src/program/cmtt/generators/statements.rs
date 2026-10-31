//! Control of the comptime construction program. Runtime control inside a
//! quote remains ordinary NIR and never targets these construction loops.
use super::*;

use comptime_interpretor::CodeValue as CodeWitness;

#[derive(Clone, Copy)]
pub(in crate::program::cmtt) enum ResultInterface<'a> {
    Data(&'a Term),
    Expression(
        Option<&'a Interface>,
        Option<&'a crate::calls::InputContext>,
    ),
    Pattern(
        Option<&'a patterns::PatternInterface>,
        Option<&'a Term>,
        Option<&'a crate::calls::InputContext>,
    ),
}

pub(in crate::program::cmtt) enum Control {
    Next,
    DataReturn(Option<comptime_interpretor::Value>),
    Break(n::LocalId),
    Continue(n::LocalId),
    Return(Arc<ExpressionCode>),
    PatternReturn(Arc<patterns::PatternCode>),
    Abstract(Flow),
}

#[derive(Default)]
pub(in crate::program::cmtt) struct Flow {
    pub(in crate::program::cmtt) next: bool,
    data_return: bool,
    breaks: BTreeSet<n::LocalId>,
    continues: BTreeSet<n::LocalId>,
    pub(in crate::program::cmtt) returns: Vec<Arc<ExpressionCode>>,
    pub(in crate::program::cmtt) pattern_returns: Vec<Arc<patterns::PatternCode>>,
}

impl Control {
    pub(in crate::program::cmtt) fn flow(self) -> Flow {
        match self {
            Self::DataReturn(_) => Flow {
                data_return: true,
                ..Default::default()
            },
            Self::Next => Flow {
                next: true,
                ..Default::default()
            },
            Self::Break(target) => Flow {
                breaks: BTreeSet::from([target]),
                ..Default::default()
            },
            Self::Continue(target) => Flow {
                continues: BTreeSet::from([target]),
                ..Default::default()
            },
            Self::Return(code) => Flow {
                returns: vec![code],
                ..Default::default()
            },
            Self::PatternReturn(code) => Flow {
                pattern_returns: vec![code],
                ..Default::default()
            },
            Self::Abstract(flow) => flow,
        }
    }
}
impl Flow {
    pub(in crate::program::cmtt) fn has_transfer(&self) -> bool {
        !self.breaks.is_empty() || !self.continues.is_empty()
    }
}
impl fmt::Display for Flow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(generator-flow (next {}) (breaks", self.next)?;
        for target in &self.breaks {
            write!(f, " {target}")?;
        }
        f.write_str(") (continues")?;
        for target in &self.continues {
            write!(f, " {target}")?;
        }
        f.write_str(") (returns")?;
        for code in &self.returns {
            write!(f, " {code}")?;
        }
        for code in &self.pattern_returns {
            write!(f, " (pattern {code})")?;
        }
        write!(f, ") (data-return {}))", self.data_return)
    }
}

fn known_boolean(value: &n::Expr, depth: usize) -> Option<bool> {
    if depth >= 64 {
        return None;
    }
    match &value.kind {
        K::Literal(n::Literal::Bool(value)) => Some(*value),
        K::Not(value) => Some(!known_boolean(value, depth + 1)?),
        K::And(left, right) => {
            Some(known_boolean(left, depth + 1)? && known_boolean(right, depth + 1)?)
        }
        K::Or(left, right) => {
            Some(known_boolean(left, depth + 1)? || known_boolean(right, depth + 1)?)
        }
        _ => None,
    }
}

// Definition-side flow evidence only: inspect literals without invoking
// endpoints, callbacks, or abstract parameter values.
fn known_integer_literal(
    value: &n::Expr,
    budget: &mut comptime_runtime::Budget,
    depth: usize,
) -> Result<Option<comptime_runtime::Integer>, comptime_runtime::Error> {
    budget.consume(1)?;
    if depth >= 64 {
        return Ok(None);
    }
    Ok(match &value.kind {
        K::Literal(n::Literal::Integer(text)) => {
            let (negative, digits) = text
                .strip_prefix('-')
                .map_or((false, text.as_str()), |digits| (true, digits));
            let value = comptime_runtime::Integer::parse_literal(digits, budget)?;
            Some(if negative {
                value.negate(budget)?
            } else {
                value
            })
        }
        K::Literal(n::Literal::SignedInteger(value)) => {
            Some(comptime_runtime::Integer::from_i128(*value))
        }
        K::Literal(n::Literal::UnsignedInteger(value)) => {
            Some(comptime_runtime::Integer::from_u128(*value))
        }
        K::IntegerNegate(inner) => known_integer_literal(inner, budget, depth + 1)?
            .map(|value| value.negate(budget))
            .transpose()?,
        _ => None,
    })
}

impl Checker<'_, '_> {
    pub(super) fn join_generator_flows(
        &mut self,
        mut left: Flow,
        right: Flow,
        span: Span,
    ) -> Result<Flow, Error> {
        let work = left
            .breaks
            .len()
            .saturating_add(left.continues.len())
            .saturating_add(left.returns.len())
            .saturating_add(right.breaks.len())
            .saturating_add(right.continues.len())
            .saturating_add(right.returns.len())
            .saturating_add(left.pattern_returns.len())
            .saturating_add(right.pattern_returns.len());
        self.required_steps = self
            .required_steps
            .checked_sub(work as u64)
            .ok_or_else(|| error(span, "generator control-flow budget exhausted"))?;
        left.next |= right.next;
        left.data_return |= right.data_return;
        left.breaks.extend(right.breaks);
        left.continues.extend(right.continues);
        left.returns.extend(right.returns);
        left.pattern_returns.extend(right.pattern_returns);
        Ok(left)
    }

    pub(in crate::program::cmtt) fn sequence_generator_control(
        &mut self,
        previous: Control,
        next: Control,
        span: Span,
    ) -> Result<Control, Error> {
        let mut previous = previous.flow();
        if previous.next {
            previous.next = false;
            previous = self.join_generator_flows(previous, next.flow(), span)?;
        }
        Ok(Control::Abstract(previous))
    }
    pub(super) fn generator_statement(
        &mut self,
        value: &Expr<'_>,
        interface: Option<&Interface>,
        result_context: Option<&crate::calls::InputContext>,
        depth: usize,
        loops: &mut Vec<n::LocalId>,
    ) -> Result<Control, Error> {
        self.generator_statement_with_result(
            value,
            ResultInterface::Expression(interface, result_context),
            depth,
            loops,
        )
    }

    pub(in crate::program::cmtt) fn generator_statement_with_result(
        &mut self,
        value: &Expr<'_>,
        interface: ResultInterface<'_>,
        depth: usize,
        loops: &mut Vec<n::LocalId>,
    ) -> Result<Control, Error> {
        if depth >= 64 {
            return Err(error(
                value.span,
                "generator statement nesting depth exhausted",
            ));
        }
        self.required_steps = self
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(value.span, "generator construction step budget exhausted"))?;
        match &value.kind {
            H::Semi(inner) => {
                self.generator_statement_with_result(inner, interface, depth + 1, loops)
            }
            H::Let(decl) if decl.otherwise.is_some() => {
                self.generator_conditional_binding(value, decl, interface, depth + 1, loops)
            }
            H::Let(decl) => {
                self.generator_binding(decl)?;
                Ok(Control::Next)
            }
            H::Assign(target, right) => {
                self.assign_generator_binding(target, right, depth + 1)?;
                Ok(Control::Next)
            }
            H::AssignOp(op, target, right) => {
                if !matches!(target.kind, H::ResolvedLocal(..) | H::ResolvedFnParam(..)) {
                    self.assign_generator_place(target, right, Some(*op))?;
                    return Ok(Control::Next);
                }
                // Ordinary binary elaboration supplies operator protocols and
                // arithmetic checks; this layer only owns the final local write.
                let expression = Expr {
                    kind: H::Binary(*op, target, right),
                    ..value.clone()
                };
                self.assign_generator_binding(target, &expression, depth + 1)?;
                Ok(Control::Next)
            }
            H::InlineControl(inner)
                if matches!(inner.kind, H::If(..) | H::When(..) | H::Match(..)) =>
            {
                self.generator_statement_with_result(inner, interface, depth + 1, loops)
            }
            H::InlineControl(inner) if matches!(inner.kind, H::For { .. }) => {
                self.generator_inline_range(inner, interface, depth + 1, loops)
            }
            H::Block(block) => self.generator_statement_block(block, interface, depth + 1, loops),
            H::If(condition, yes, no) => {
                if self.checking_code_generator {
                    let condition_ir = self.expr(condition, Some(&p(P::Bool)))?;
                    self.validate_generator_data(&condition_ir, condition.span)?;
                    let yes = self.generator_statement_block(yes, interface, depth + 1, loops)?;
                    let no = if let Some(no) = no {
                        self.generator_statement_with_result(no, interface, depth + 1, loops)?
                    } else {
                        Control::Next
                    };
                    match known_boolean(&condition_ir, 0) {
                        Some(true) => Ok(yes),
                        Some(false) => Ok(no),
                        None => Ok(Control::Abstract(self.join_generator_flows(
                            yes.flow(),
                            no.flow(),
                            value.span,
                        )?)),
                    }
                } else {
                    let (condition_value, _) =
                        self.generator_data_expression(condition, Some(&p(P::Bool)))?;
                    match condition_value {
                        comptime_interpretor::Value::Bool(true) => {
                            self.generator_statement_block(yes, interface, depth + 1, loops)
                        }
                        comptime_interpretor::Value::Bool(false) => match no {
                            Some(no) => self.generator_statement_with_result(
                                no,
                                interface,
                                depth + 1,
                                loops,
                            ),
                            None => Ok(Control::Next),
                        },
                        _ => Err(error(
                            condition.span,
                            "generator branch requires a comptime boolean",
                        )),
                    }
                }
            }
            H::When(arms) => {
                let mut flow = Flow::default();
                let mut pending = true;
                for arm in *arms {
                    let selected = self.generator_when_condition(arm.cond)?;
                    if self.checking_code_generator {
                        let control = self.generator_scope(arm.span, |this| {
                            this.generator_statement_with_result(
                                arm.body,
                                interface,
                                depth + 1,
                                loops,
                            )
                        })?;
                        if pending && selected != Some(false) {
                            flow = self.join_generator_flows(flow, control.flow(), arm.span)?;
                        }
                        if selected == Some(true) {
                            pending = false;
                        }
                    } else if selected == Some(true) {
                        return self.generator_scope(arm.span, |this| {
                            this.generator_statement_with_result(
                                arm.body,
                                interface,
                                depth + 1,
                                loops,
                            )
                        });
                    }
                }
                if self.checking_code_generator {
                    flow.next |= pending;
                    Ok(Control::Abstract(flow))
                } else {
                    Ok(Control::Next)
                }
            }
            H::Match(subject, arms) => {
                let controls =
                    self.generator_match(value, subject, arms, None, |this, _, body| {
                        this.generator_statement_with_result(body, interface, depth + 1, loops)
                    })?;
                if self.checking_code_generator {
                    let mut flow = Flow::default();
                    for control in controls {
                        flow = self.join_generator_flows(flow, control.flow(), value.span)?;
                    }
                    Ok(Control::Abstract(flow))
                } else {
                    controls
                        .into_iter()
                        .next()
                        .ok_or_else(|| error(value.span, "generator match selected no arm"))
                }
            }
            H::Loop(body) => {
                let identity = local(value.hir_id);
                loops.push(identity);
                let result = (|| {
                    if self.checking_code_generator {
                        let mut flow = self
                            .generator_statement_block(body, interface, depth + 1, loops)?
                            .flow();
                        // Backedges do not finish the enclosing block. Only a
                        // break targeting this loop reaches its successor.
                        flow.next = flow.breaks.remove(&identity);
                        flow.continues.remove(&identity);
                        return Ok(Control::Abstract(flow));
                    }
                    loop {
                        self.required_steps =
                            self.required_steps.checked_sub(1).ok_or_else(|| {
                                error(value.span, "generator construction step budget exhausted")
                            })?;
                        match self.generator_statement_block(body, interface, depth + 1, loops)? {
                            Control::Next => {}
                            Control::Continue(target) if target == identity => {}
                            Control::Break(target) if target == identity => {
                                return Ok(Control::Next);
                            }
                            transfer => return Ok(transfer),
                        }
                    }
                })();
                loops.pop();
                result
            }
            H::BreakTo(id) | H::ContinueTo(id) => {
                let target = local(*id);
                if !loops.contains(&target) {
                    return Err(error(
                        value.span,
                        "generator loop transfer targets an unavailable loop",
                    ));
                }
                Ok(if matches!(value.kind, H::BreakTo(_)) {
                    Control::Break(target)
                } else {
                    Control::Continue(target)
                })
            }
            H::Break(label) | H::Continue(label) => {
                if label.name != hir::Symbol::invalid() {
                    return Err(error(value.span, "unresolved generator loop label"));
                }
                let target = *loops
                    .last()
                    .ok_or_else(|| error(value.span, "generator loop transfer outside a loop"))?;
                Ok(if matches!(value.kind, H::Break(_)) {
                    Control::Break(target)
                } else {
                    Control::Continue(target)
                })
            }
            H::Return(Some(code)) => match interface {
                ResultInterface::Data(ty) => self.consumer_result(code, ty, depth + 1),
                ResultInterface::Expression(interface, result_context) => Ok(Control::Return(
                    self.code_value_in_context(code, interface, result_context, depth + 1)?,
                )),
                ResultInterface::Pattern(definition, subject, input_context) => Ok(
                    Control::PatternReturn(self.pattern_generator_value_inferred(
                        code,
                        definition,
                        subject,
                        input_context,
                        depth + 1,
                    )?),
                ),
            },
            H::Return(None) if matches!(interface, ResultInterface::Data(ty) if *ty == p(P::Void)) => {
                Ok(Control::DataReturn(
                    (!self.checking_code_generator).then_some(comptime_interpretor::Value::Unit),
                ))
            }
            H::Return(None) => Err(error(
                value.span,
                "code generator return requires a code value",
            )),
            _ if self.pattern_code_producer(value, None) => {
                // Discarding code erases only the resulting evidence. The
                // construction program still executes once in source order.
                self.pattern_generator_value_inferred(value, None, None, None, depth + 1)?;
                Ok(Control::Next)
            }
            _ if self.code_producer(value) => {
                self.code_value(value, None, depth + 1)?;
                Ok(Control::Next)
            }
            _ => {
                if self.checking_code_generator {
                    let checked = self.expr(value, None)?;
                    self.validate_generator_data(&checked, value.span)?;
                } else {
                    self.generator_data_expression(value, None)?;
                }
                Ok(Control::Next)
            }
        }
    }

    fn generator_inline_range(
        &mut self,
        value: &Expr<'_>,
        interface: ResultInterface<'_>,
        depth: usize,
        loops: &mut Vec<n::LocalId>,
    ) -> Result<Control, Error> {
        let H::For {
            pat,
            guard,
            iter,
            body,
            ..
        } = &value.kind
        else {
            unreachable!()
        };
        if let H::TakeTupleEnumerate(input) = iter.kind {
            return self.generator_inline_tuple(value, input, interface, depth, loops);
        }
        let H::Range {
            start: Some(start),
            end: Some(end),
            inclusive,
        } = iter.kind
        else {
            return Err(error(
                iter.span,
                "generator inline for requires a bounded comptime Integer range",
            ));
        };
        let identity = local(value.hir_id);
        loops.push(identity);
        let result = (|| {
            if self.checking_code_generator {
                let mut checked_start = None;
                let mut endpoints = [None, None];
                for (index, endpoint) in [start, end].into_iter().enumerate() {
                    let checked = self.expr(endpoint, Some(&p(P::Integer)))?;
                    self.validate_generator_data(&checked, endpoint.span)?;
                    let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                    let mut budget = comptime_runtime::Budget {
                        steps: before,
                        max_limbs: 4096,
                    };
                    let known = known_integer_literal(&checked, &mut budget, 0);
                    self.required_steps -= (before - budget.steps) as u64;
                    endpoints[index] = known.map_err(|e| error(endpoint.span, e.to_string()))?;
                    if index == 0 {
                        checked_start = Some(checked);
                    }
                }
                let nonempty = match &endpoints {
                    [Some(start), Some(end)] => {
                        let work = start
                            .bit_len()
                            .max(end.bit_len())
                            .div_ceil(32)
                            .saturating_add(1);
                        self.required_steps = self
                            .required_steps
                            .checked_sub(work as u64)
                            .ok_or_else(|| {
                                error(iter.span, "generator range flow budget exhausted")
                            })?;
                        Some(if inclusive { start <= end } else { start < end })
                    }
                    _ => None,
                };
                return self.generator_scope(body.span, |this| {
                    let existing = this.locals.keys().copied().collect::<BTreeSet<_>>();
                    this.immutable.extend(pattern_binding_ids(pat));
                    let pattern = this.pattern(pat, &p(P::Integer))?;
                    let irrefutable = this.pattern_exhaustive(&pattern, &p(P::Integer));
                    for id in this.locals.keys().copied().collect::<Vec<_>>() {
                        if !existing.contains(&id) {
                            this.generator_data.insert(id, None);
                            this.immutable.insert(id);
                        }
                    }
                    // Phase-check computations embedded in the pattern even
                    // for empty ranges and unused definitions. This probe is
                    // never invoked: the endpoint only supplies a typed input.
                    let probe = n::Expr {
                        ty: p(P::Bool),
                        source: source(pat.span),
                        kind: K::Match {
                            value: Box::new(checked_start.take().unwrap()),
                            arms: vec![n::MatchArm {
                                pattern,
                                guard: None,
                                body: n::Expr {
                                    ty: p(P::Bool),
                                    source: source(pat.span),
                                    kind: K::Literal(n::Literal::Bool(true)),
                                },
                            }],
                        },
                    };
                    this.validate_generator_data(&probe, pat.span)?;
                    let selected = this.generator_when_condition(*guard)?;
                    let mut flow = this
                        .generator_statement_block(body, interface, depth + 1, loops)?
                        .flow();
                    let breaks_here = flow.breaks.remove(&identity);
                    let continues_here = flow.continues.remove(&identity);
                    if nonempty == Some(false) || selected == Some(false) {
                        // The body was checked above but contributes no exits.
                        flow = Flow::default();
                        flow.next = true;
                    } else {
                        // A known nonempty range with an unfiltered first
                        // iteration cannot fall through an always-returning
                        // body. A finite continue path still reaches the end.
                        flow.next |= !irrefutable
                            || nonempty != Some(true)
                            || selected != Some(true)
                            || breaks_here
                            || continues_here;
                    }
                    Ok(Control::Abstract(flow))
                });
            }
            let (start_value, _) = self.generator_data_expression(start, Some(&p(P::Integer)))?;
            let (end_value, _) = self.generator_data_expression(end, Some(&p(P::Integer)))?;
            let (
                comptime_interpretor::Value::Integer(mut current),
                comptime_interpretor::Value::Integer(end),
            ) = (start_value, end_value)
            else {
                return Err(error(
                    iter.span,
                    "generator inline range endpoints must be Integer",
                ));
            };
            let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
            let mut budget = comptime_runtime::Budget {
                steps: before,
                max_limbs: 4096,
            };
            let count = (|| {
                let difference = end.subtract(&current, &mut budget)?;
                if inclusive {
                    difference.add(&comptime_runtime::Integer::from_u128(1), &mut budget)
                } else {
                    Ok(difference)
                }
            })();
            self.required_steps -= (before - budget.steps) as u64;
            let count = count.map_err(|e| error(iter.span, e.to_string()))?;
            let count = if count <= comptime_runtime::Integer::from_u128(0) {
                0
            } else {
                usize::try_from(
                    count
                        .to_u128()
                        .map_err(|_| error(iter.span, "generator inline range budget exhausted"))?,
                )
                .map_err(|_| error(iter.span, "generator inline range budget exhausted"))?
            };
            if count > 100_000 || count as u64 > self.required_steps {
                return Err(error(iter.span, "generator inline range budget exhausted"));
            }
            let source_body = Expr {
                hir_id: body.hir_id,
                span: body.span,
                kind: H::Block(body),
            };
            for index in 0..count {
                self.required_steps = self
                    .required_steps
                    .checked_sub(current.bit_len().div_ceil(32).saturating_add(1) as u64)
                    .ok_or_else(|| error(iter.span, "generator inline range budget exhausted"))?;
                let control = self.generator_scope(body.span, |this| {
                    let input = this.materialize_generator_data(
                        comptime_interpretor::Value::Integer(current.clone()),
                        p(P::Integer),
                        pat.span,
                    )?;
                    let arms = [
                        hir::pattern::PatternArm {
                            hir_id: pat.hir_id,
                            pat: (**pat).clone(),
                            guard: None,
                            body: &source_body,
                            span: pat.span,
                        },
                        hir::pattern::PatternArm {
                            hir_id: pat.hir_id,
                            pat: hir::pattern::Pattern {
                                hir_id: pat.hir_id,
                                kind: hir::pattern::PatternKind::Wild,
                                span: pat.span,
                            },
                            guard: None,
                            body: &source_body,
                            span: pat.span,
                        },
                    ];
                    // Pattern computations use the same live frame as the
                    // body. Failure discards candidate bindings, not writes.
                    let selected = this.generator_match_prepared(
                        value,
                        &source_body,
                        &arms,
                        input,
                        false,
                        &mut |this, arm, _| {
                            if arm == 1 || this.generator_when_condition(*guard)? == Some(false) {
                                return Ok(Control::Next);
                            }
                            this.generator_statement_block(body, interface, depth + 1, loops)
                        },
                    )?;
                    selected.into_iter().next().ok_or_else(|| {
                        error(pat.span, "generator range matching produced no selection")
                    })
                })?;
                match control {
                    Control::Next => {}
                    Control::Continue(target) if target == identity => {}
                    Control::Break(target) if target == identity => return Ok(Control::Next),
                    other => return Ok(other),
                }
                if index + 1 < count {
                    let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
                    let mut budget = comptime_runtime::Budget {
                        steps: before,
                        max_limbs: 4096,
                    };
                    let next = current.add(&comptime_runtime::Integer::from_u128(1), &mut budget);
                    self.required_steps -= (before - budget.steps) as u64;
                    current = next.map_err(|e| error(iter.span, e.to_string()))?;
                }
            }
            Ok(Control::Next)
        })();
        loops.pop();
        result
    }

    fn generator_inline_tuple(
        &mut self,
        value: &Expr<'_>,
        input: &Expr<'_>,
        interface: ResultInterface<'_>,
        depth: usize,
        loops: &mut Vec<n::LocalId>,
    ) -> Result<Control, Error> {
        let H::For {
            pat, guard, body, ..
        } = &value.kind
        else {
            unreachable!()
        };
        self.tuple_iteration_patterns(pat)?;
        let (ty, values) = if self.checking_code_generator {
            let checked = self.expr(input, None)?;
            self.validate_generator_data(&checked, input.span)?;
            (checked.ty, None)
        } else {
            let (value, ty) = self.generator_data_expression(input, None)?;
            let comptime_interpretor::Value::Tuple(fields) = value else {
                return Err(error(
                    input.span,
                    "generator tuple_enumerate requires a tuple data value",
                ));
            };
            (ty, Some(fields))
        };
        let Term::Tuple(elements) = self.structural_type(&ty, input.span)? else {
            return Err(error(
                input.span,
                "generator tuple_enumerate requires an established tuple layout",
            ));
        };
        if elements.is_empty() {
            return Err(error(
                input.span,
                "empty tuple_enumerate body checking is not yet implemented",
            ));
        }
        self.required_steps = self
            .required_steps
            .checked_sub(elements.len() as u64)
            .ok_or_else(|| error(input.span, "generator tuple enumeration budget exhausted"))?;
        let values = if let Some(values) = values {
            if values.len() != elements.len() {
                return Err(error(
                    input.span,
                    "generator tuple layout does not match its value",
                ));
            }
            values.into_iter().map(Some).collect::<Vec<_>>()
        } else {
            (0..elements.len()).map(|_| None).collect::<Vec<_>>()
        };
        let declaration = hir::decl::LetDecl {
            hir_id: value.hir_id,
            mutable: false,
            comptime: true,
            pattern: (**pat).clone(),
            guard: None,
            otherwise: None,
            ty: None,
            init: Some(input),
            span: pat.span,
        };
        let identity = local(value.hir_id);
        loops.push(identity);
        let result = (|| {
            let mut control = Control::Next;
            for (index, (ty, field)) in elements.into_iter().zip(values).enumerate() {
                let next = self.generator_scope(body.span, |this| {
                    let pair_type =
                        this.tuple_value_type(vec![p(P::Integer), ty.clone()], pat.span)?;
                    let irrefutable = this.generator_scope(pat.span, |this| {
                        this.immutable.extend(pattern_binding_ids(pat));
                        let pattern = this.with_generator_pattern_inputs(pat.span, |this| {
                            this.pattern(pat, &pair_type)
                        })?;
                        Ok(this.pattern_exhaustive(&pattern, &pair_type))
                    })?;
                    if !irrefutable {
                        let id = this.spread_binding(pat.span)?;
                        this.locals.insert(id, pair_type.clone());
                        let marker = n::Expr {
                            ty: pair_type.clone(),
                            source: source(pat.span),
                            kind: K::Local(id),
                        };
                        let pair = if let Some(field) = field {
                            let data = comptime_interpretor::Value::Tuple(vec![
                                comptime_interpretor::Value::Integer(
                                    comptime_runtime::Integer::from_u128(index as u128),
                                ),
                                field,
                            ]);
                            // Keep the evaluated packet in a live cell. In
                            // particular, pointer fields must retain aliases.
                            this.generator_data.insert(id, Some(marker.clone()));
                            this.generator_cells
                                .insert(id, (marker.clone(), GeneratorCell::owned(data)));
                            marker
                        } else {
                            this.generator_data.insert(id, None);
                            marker
                        };
                        let arms = [
                            hir::pattern::PatternArm {
                                hir_id: pat.hir_id,
                                pat: (**pat).clone(),
                                guard: None,
                                body: input,
                                span: pat.span,
                            },
                            hir::pattern::PatternArm {
                                hir_id: pat.hir_id,
                                pat: hir::pattern::Pattern {
                                    hir_id: pat.hir_id,
                                    kind: hir::pattern::PatternKind::Wild,
                                    span: pat.span,
                                },
                                guard: None,
                                body: input,
                                span: pat.span,
                            },
                        ];
                        let outputs = this.generator_match_prepared(
                            value,
                            input,
                            &arms,
                            pair,
                            false,
                            &mut |this, selected_arm, _| {
                                if selected_arm == 1 {
                                    return Ok(Control::Next);
                                }
                                let selected = this.generator_when_condition(*guard)?;
                                if this.checking_code_generator {
                                    let mut flow = this
                                        .generator_statement_block(
                                            body,
                                            interface,
                                            depth + 1,
                                            loops,
                                        )?
                                        .flow();
                                    if selected == Some(false) {
                                        return Ok(Control::Next);
                                    }
                                    let continues_here = flow.continues.remove(&identity);
                                    if selected.is_none() || continues_here {
                                        flow.next = true;
                                    }
                                    Ok(Control::Abstract(flow))
                                } else if selected == Some(false) {
                                    Ok(Control::Next)
                                } else {
                                    this.generator_statement_block(
                                        body,
                                        interface,
                                        depth + 1,
                                        loops,
                                    )
                                }
                            },
                        )?;
                        if this.checking_code_generator {
                            let mut flow = Flow::default();
                            for output in outputs {
                                flow = this.join_generator_flows(flow, output.flow(), pat.span)?;
                            }
                            return Ok(Control::Abstract(flow));
                        }
                        return outputs.into_iter().next().ok_or_else(|| {
                            error(pat.span, "generator tuple matching produced no selection")
                        });
                    }
                    if let Some(field) = field {
                        let pair = comptime_interpretor::Value::Tuple(vec![
                            comptime_interpretor::Value::Integer(
                                comptime_runtime::Integer::from_u128(index as u128),
                            ),
                            field,
                        ]);
                        this.bind_live_generator_pattern(&declaration, pair, pair_type)?;
                    } else {
                        let field = {
                            // Each element has its own established type, but no
                            // invented data value during definition checking.
                            let id = n::LocalId {
                                owner: u32::MAX,
                                index: this.next_temporary_binding,
                            };
                            this.next_temporary_binding =
                                this.next_temporary_binding.checked_add(1).ok_or_else(|| {
                                    error(input.span, "generator tuple identity space exhausted")
                                })?;
                            this.locals.insert(id, ty.clone());
                            this.generator_data.insert(id, None);
                            n::Expr {
                                ty: ty.clone(),
                                source: source(input.span),
                                kind: K::Local(id),
                            }
                        };
                        let pair = n::Expr {
                            ty: pair_type.clone(),
                            source: source(pat.span),
                            kind: K::Tuple(vec![
                                n::Expr {
                                    ty: p(P::Integer),
                                    source: source(pat.span),
                                    kind: K::Literal(n::Literal::Integer(index.to_string())),
                                },
                                field,
                            ]),
                        };
                        this.bind_generator_pattern(&declaration, pair, pair_type, false)?;
                    }
                    let selected = this.generator_when_condition(*guard)?;
                    if this.checking_code_generator {
                        let body =
                            this.generator_statement_block(body, interface, depth + 1, loops)?;
                        if selected == Some(false) {
                            return Ok(Control::Next);
                        }
                        let mut flow = body.flow();
                        if selected.is_none() {
                            flow.next = true;
                        }
                        if flow.continues.remove(&identity) {
                            flow.next = true;
                        }
                        Ok(Control::Abstract(flow))
                    } else if selected == Some(false) {
                        Ok(Control::Next)
                    } else {
                        this.generator_statement_block(body, interface, depth + 1, loops)
                    }
                })?;
                if self.checking_code_generator {
                    control = self.sequence_generator_control(control, next, body.span)?;
                } else {
                    match next {
                        Control::Next => {}
                        Control::Continue(target) if target == identity => {}
                        Control::Break(target) if target == identity => return Ok(Control::Next),
                        other => return Ok(other),
                    }
                }
            }
            if self.checking_code_generator {
                let mut flow = control.flow();
                if flow.breaks.remove(&identity) {
                    flow.next = true;
                }
                Ok(Control::Abstract(flow))
            } else {
                Ok(Control::Next)
            }
        })();
        loops.pop();
        result
    }

    pub(in crate::program::cmtt) fn generator_when_condition(
        &mut self,
        condition: Option<&Expr<'_>>,
    ) -> Result<Option<bool>, Error> {
        let Some(condition) = condition else {
            return Ok(Some(true));
        };
        if self.checking_code_generator {
            let checked = self.expr(condition, Some(&p(P::Bool)))?;
            self.validate_generator_data(&checked, condition.span)?;
            Ok(known_boolean(&checked, 0))
        } else {
            let (value, _) = self.generator_data_expression(condition, Some(&p(P::Bool)))?;
            match value {
                comptime_interpretor::Value::Bool(value) => Ok(Some(value)),
                _ => Err(error(
                    condition.span,
                    "generator when condition requires a comptime boolean",
                )),
            }
        }
    }

    fn generator_conditional_binding(
        &mut self,
        site: &Expr<'_>,
        decl: &hir::decl::LetDecl<'_>,
        interface: ResultInterface<'_>,
        depth: usize,
        loops: &mut Vec<n::LocalId>,
    ) -> Result<Control, Error> {
        let initializer = decl
            .init
            .ok_or_else(|| error(decl.span, "generator binding needs an initializer"))?;
        if self.pattern_code_producer(initializer, decl.ty)
            || self.code_producer(initializer)
            || decl
                .ty
                .is_some_and(|ty| matches!(ty.kind, H::ContextualExprType(..)))
        {
            return self.generator_conditional_code_binding(decl, interface, depth, loops);
        }
        let expected = decl.ty.map(|ty| self.type_expr(ty)).transpose()?;
        let otherwise = decl.otherwise.unwrap();
        let arms = [
            hir::pattern::PatternArm {
                hir_id: decl.hir_id,
                pat: decl.pattern.clone(),
                guard: decl.guard,
                body: initializer,
                span: decl.span,
            },
            hir::pattern::PatternArm {
                hir_id: decl.hir_id,
                pat: hir::pattern::Pattern {
                    hir_id: decl.hir_id,
                    kind: hir::pattern::PatternKind::Wild,
                    span: decl.span,
                },
                guard: None,
                body: otherwise,
                span: decl.span,
            },
        ];
        let outputs = self.generator_match_with_binding_mutability(
            site,
            initializer,
            &arms,
            expected.as_ref(),
            decl.mutable,
            |this, index, body| {
                if index == 1 {
                    let control =
                        this.generator_statement_with_result(body, interface, depth + 1, loops)?;
                    let falls_through = match &control {
                        Control::Next => true,
                        Control::Abstract(flow) => flow.next,
                        _ => false,
                    };
                    if falls_through {
                        return Err(error(body.span, "generator let-else branch must diverge"));
                    }
                    return Ok((control, Vec::new()));
                }
                let mut bindings = Vec::new();
                for id in pattern_bindings(&decl.pattern)?.into_values() {
                    let ty = this
                        .locals
                        .get(&id)
                        .ok_or_else(|| error(decl.span, "generator let-else missing binding type"))?
                        .clone();
                    let data = this.generator_data.get(&id).ok_or_else(|| {
                        error(decl.span, "generator let-else missing binding value")
                    })?;
                    if let Some(data) = data {
                        constants::charge_copy(data, &mut this.required_steps, decl.span)?;
                    }
                    let cell = this.generator_cells.get(&id).cloned();
                    bindings.push((id, ty, data.clone(), cell));
                }
                Ok((Control::Next, bindings))
            },
        )?;
        let mut result = Control::Next;
        let mut flow = Flow::default();
        for (control, bindings) in outputs {
            for (id, ty, value, cell) in bindings {
                if let Some(cell) = cell {
                    self.generator_cells.insert(id, cell);
                }
                self.locals.insert(id, ty);
                self.generator_data.insert(id, value);
                if decl.mutable {
                    self.immutable.remove(&id);
                } else {
                    self.immutable.insert(id);
                }
            }
            if self.checking_code_generator {
                flow = self.join_generator_flows(flow, control.flow(), decl.span)?;
            } else {
                result = control;
            }
        }
        Ok(if self.checking_code_generator {
            Control::Abstract(flow)
        } else {
            result
        })
    }

    fn generator_conditional_code_binding(
        &mut self,
        decl: &hir::decl::LetDecl<'_>,
        interface: ResultInterface<'_>,
        depth: usize,
        loops: &mut Vec<n::LocalId>,
    ) -> Result<Control, Error> {
        // Bind the whole witness only on the success path. The initializer and
        // guard run once; the failure branch sees the original lexical scope.
        let (condition, bindings) = self.generator_scope(decl.span, |this| {
            let mut plain = decl.clone();
            plain.guard = None;
            plain.otherwise = None;
            let pattern = this.pattern_code_producer(plain.init.unwrap(), plain.ty);
            if pattern {
                this.pattern_code_binding_mode(&plain, None, true)?;
            } else {
                this.code_binding(&plain, None, true)?;
            }
            let condition = this.generator_when_condition(decl.guard)?;
            let bindings = pattern_bindings(&decl.pattern)?
                .into_values()
                .map(|id| {
                    (if pattern {
                        this.pattern_codes
                            .get(&id)
                            .cloned()
                            .map(CodeWitness::Pattern)
                    } else {
                        this.code_values
                            .get(&id)
                            .cloned()
                            .map(CodeWitness::Expression)
                    })
                    .map(|code| (id, code))
                    .ok_or_else(|| error(decl.span, "conditional code binding has no witness"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok((condition, bindings))
        })?;
        let failure = if self.checking_code_generator || condition == Some(false) {
            let otherwise = decl.otherwise.unwrap();
            let failure = self.generator_scope(otherwise.span, |this| {
                this.generator_statement_with_result(otherwise, interface, depth + 1, loops)
            })?;
            if matches!(&failure, Control::Next)
                || matches!(&failure, Control::Abstract(flow) if flow.next)
            {
                return Err(error(
                    otherwise.span,
                    "generator let-else branch must diverge",
                ));
            }
            Some(failure)
        } else {
            None
        };
        if self.checking_code_generator || condition == Some(true) {
            for (id, code) in bindings {
                match code {
                    CodeWitness::Expression(code) => {
                        self.code_values.insert(id, code);
                    }
                    CodeWitness::Pattern(code) => {
                        self.pattern_codes.insert(id, code);
                    }
                }
                if decl.mutable {
                    self.immutable.remove(&id);
                } else {
                    self.immutable.insert(id);
                }
            }
        }
        match condition {
            Some(true) => Ok(Control::Next),
            Some(false) => Ok(failure.unwrap()),
            None if self.checking_code_generator => {
                Ok(Control::Abstract(self.join_generator_flows(
                    Control::Next.flow(),
                    failure.unwrap().flow(),
                    decl.span,
                )?))
            }
            None => Err(error(
                decl.span,
                "generator binding guard requires a comptime boolean",
            )),
        }
    }

    fn generator_statement_block(
        &mut self,
        block: &hir::Block<'_>,
        interface: ResultInterface<'_>,
        depth: usize,
        loops: &mut Vec<n::LocalId>,
    ) -> Result<Control, Error> {
        self.generator_scope(block.span, |this| {
            let mut result = Control::Next;
            for value in block.stmts.iter().chain(block.expr.into_iter()) {
                let control =
                    this.generator_statement_with_result(value, interface, depth + 1, loops)?;
                if this.checking_code_generator {
                    result = this.sequence_generator_control(result, control, value.span)?;
                } else if !matches!(control, Control::Next) {
                    return Ok(control);
                }
            }
            Ok(result)
        })
    }
}
