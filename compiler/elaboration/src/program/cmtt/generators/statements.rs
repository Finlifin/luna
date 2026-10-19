//! Control of the comptime construction program. Runtime control inside a
//! quote remains ordinary NIR and never targets these construction loops.
use super::*;

pub(super) enum Control {
    Next,
    Break(n::LocalId),
    Continue(n::LocalId),
    Return(Arc<ExpressionCode>),
    Abstract(Flow),
}

#[derive(Default)]
pub(super) struct Flow {
    pub(super) next: bool,
    breaks: BTreeSet<n::LocalId>,
    continues: BTreeSet<n::LocalId>,
    pub(super) returns: Vec<Arc<ExpressionCode>>,
}

impl Control {
    pub(super) fn flow(self) -> Flow {
        match self {
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
            Self::Abstract(flow) => flow,
        }
    }
}
impl Flow {
    pub(super) fn has_transfer(&self) -> bool {
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
        f.write_str("))")
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
    fn join_generator_flows(
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
            .saturating_add(right.returns.len());
        self.required_steps = self
            .required_steps
            .checked_sub(work as u64)
            .ok_or_else(|| error(span, "generator control-flow budget exhausted"))?;
        left.next |= right.next;
        left.breaks.extend(right.breaks);
        left.continues.extend(right.continues);
        left.returns.extend(right.returns);
        Ok(left)
    }

    pub(super) fn sequence_generator_control(
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
            H::Semi(inner) => self.generator_statement(inner, interface, depth + 1, loops),
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
                self.generator_statement(inner, interface, depth + 1, loops)
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
                        self.generator_statement(no, interface, depth + 1, loops)?
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
                            Some(no) => self.generator_statement(no, interface, depth + 1, loops),
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
                            this.generator_statement(arm.body, interface, depth + 1, loops)
                        })?;
                        if pending && selected != Some(false) {
                            flow = self.join_generator_flows(flow, control.flow(), arm.span)?;
                        }
                        if selected == Some(true) {
                            pending = false;
                        }
                    } else if selected == Some(true) {
                        return self.generator_scope(arm.span, |this| {
                            this.generator_statement(arm.body, interface, depth + 1, loops)
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
                        this.generator_statement(body, interface, depth + 1, loops)
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
            H::Return(Some(code)) => Ok(Control::Return(self.code_value(
                code,
                interface,
                depth + 1,
            )?)),
            H::Return(None) => Err(error(
                value.span,
                "code generator return requires a code value",
            )),
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
        interface: Option<&Interface>,
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
                    let pattern = this.pattern(pat, &p(P::Integer))?;
                    if !this.pattern_exhaustive(&pattern, &p(P::Integer)) {
                        return Err(error(
                            pat.span,
                            "generator inline range requires an irrefutable pattern",
                        ));
                    }
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
                        flow.next |= nonempty != Some(true)
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
                    let existing = this.locals.keys().copied().collect::<BTreeSet<_>>();
                    let pattern = this.pattern(pat, &p(P::Integer))?;
                    if !this.pattern_exhaustive(&pattern, &p(P::Integer)) {
                        return Err(error(
                            pat.span,
                            "generator inline range requires an irrefutable pattern",
                        ));
                    }
                    for id in this.locals.keys().copied().collect::<Vec<_>>() {
                        if !existing.contains(&id) {
                            this.immutable.insert(id);
                        }
                    }
                    let bindings = this
                        .select_inline_pattern(
                            &pattern,
                            &Term::Integer(current.clone()),
                            &source_body,
                        )?
                        .ok_or_else(|| {
                            error(pat.span, "generator inline range binding did not match")
                        })?;
                    this.materialize_inline_bindings(&bindings, &source_body)?;
                    if this.generator_when_condition(*guard)? == Some(false) {
                        return Ok(Control::Next);
                    }
                    this.generator_statement_block(body, interface, depth + 1, loops)
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
        interface: Option<&Interface>,
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
                    let live = match field.as_ref() {
                        Some(field) => {
                            this.generator_value_needs_live_storage(field, input.span)?
                        }
                        None => false,
                    };
                    if live {
                        let pair = comptime_interpretor::Value::Tuple(vec![
                            comptime_interpretor::Value::Integer(
                                comptime_runtime::Integer::from_u128(index as u128),
                            ),
                            field.unwrap(),
                        ]);
                        this.bind_live_generator_pattern(&declaration, pair, pair_type)?;
                    } else {
                        let field = if let Some(field) = field {
                            this.materialize_generator_data(field, ty.clone(), input.span)?
                        } else {
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
        interface: Option<&Interface>,
        depth: usize,
        loops: &mut Vec<n::LocalId>,
    ) -> Result<Control, Error> {
        let initializer = decl
            .init
            .ok_or_else(|| error(decl.span, "generator binding needs an initializer"))?;
        if self.code_producer(initializer) {
            return Err(error(
                decl.span,
                "conditional destructuring requires generator data, not a code witness",
            ));
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
        let outputs = self.generator_match(
            site,
            initializer,
            &arms,
            expected.as_ref(),
            |this, index, body| {
                if index == 1 {
                    let control = this.generator_statement(body, interface, depth + 1, loops)?;
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

    fn generator_statement_block(
        &mut self,
        block: &hir::Block<'_>,
        interface: Option<&Interface>,
        depth: usize,
        loops: &mut Vec<n::LocalId>,
    ) -> Result<Control, Error> {
        self.generator_scope(block.span, |this| {
            let mut result = Control::Next;
            for value in block.stmts.iter().chain(block.expr.into_iter()) {
                let control = this.generator_statement(value, interface, depth + 1, loops)?;
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
