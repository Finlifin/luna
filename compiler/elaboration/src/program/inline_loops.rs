//! Bounded source-range unrolling; runtime bodies retain their own scopes.
use super::*;

impl Checker<'_, '_> {
    /// Expose only the tuple layout; the input retains its nominal type and
    /// runtime evaluation. No comptime invocation is needed to inspect shape.
    pub(super) fn tuple_enumeration_input(
        &mut self,
        input: &Expr<'_>,
    ) -> Result<(n::Expr, Vec<Term>), Error> {
        let span = input.span;
        let input = self.expr(input, None)?;
        let shape = self.structural_type(&input.ty, span)?;
        let Term::Tuple(elements) = shape else {
            return Err(error(
                span,
                "tuple_enumerate requires a tuple value with established structural type evidence",
            ));
        };
        Ok((input, elements))
    }

    pub(super) fn expand_inline_range(
        &mut self,
        expr: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let H::For {
            pat,
            guard,
            iter,
            body,
            ..
        } = &expr.kind
        else {
            unreachable!()
        };
        if let H::TakeTupleEnumerate(input) = iter.kind {
            return self.expand_inline_tuple(expr, input, expected);
        }
        let H::Range {
            start: Some(start),
            end: Some(end),
            inclusive,
        } = iter.kind
        else {
            return Err(error(
                iter.span,
                "inline for currently requires a bounded comptime Integer range",
            ));
        };
        let suppressed = self.in_unselected_inline
            && self
                .inline_selection
                .is_some_and(|(id, _)| id.owner == expr.hir_id.owner);
        let (mut current, count) = if suppressed {
            // The ordinary description already checked staged endpoints. The
            // behavior witness models runtime bodies, not boundary invocation.
            if !self.checking_open_behavior {
                self.expr(start, Some(&p(P::Integer)))?;
                self.expr(end, Some(&p(P::Integer)))?;
            }
            (comptime_runtime::Integer::from_u128(0), 0)
        } else {
            let comptime_interpretor::Value::Integer(current) =
                self.required_value(start, P::Integer)?
            else {
                return Err(error(start.span, "inline range start must be Integer"));
            };
            let comptime_interpretor::Value::Integer(end) = self.required_value(end, P::Integer)?
            else {
                return Err(error(end.span, "inline range end must be Integer"));
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
                        .map_err(|_| error(iter.span, "inline range expansion budget exhausted"))?,
                )
                .map_err(|_| error(iter.span, "inline range expansion budget exhausted"))?
            };
            if count > 100_000 || count as u64 > self.required_steps {
                return Err(error(iter.span, "inline range expansion budget exhausted"));
            }
            (current, count)
        };
        let saved_locals = self.locals.clone();
        let saved_data = self.generator_data.clone();
        let saved_immutable = self.immutable.clone();
        let saved_context = self.context.clone();
        let saved_selection = self.inline_selection;
        let saved_suppressed = self.in_unselected_inline;
        let identity = local(expr.hir_id);
        self.loops.push(identity);
        let result = (|| {
            let mut iterations = Vec::new();
            // An empty range still checks its ordinary body, without invoking
            // nested inline conditions that have no selected iteration.
            for index in 0..count.max(1) {
                self.locals = saved_locals.clone();
                self.generator_data = saved_data.clone();
                self.immutable = saved_immutable.clone();
                self.context = saved_context.clone();
                let pattern = self.pattern(pat, &p(P::Integer))?;
                if !self.pattern_exhaustive(&pattern, &p(P::Integer)) {
                    return Err(error(
                        pat.span,
                        "refutable inline range patterns are not yet supported; use an irrefutable pattern with a guard",
                    ));
                }
                if self.checking_open_behavior {
                    // An abstract index is an input to the behavior witness,
                    // never a sampled iteration value or a runtime allocation.
                    for (id, ty) in &self.locals {
                        if !saved_locals.contains_key(id) {
                            self.ordinary_behavior_inputs.insert(*id, ty.clone());
                        }
                    }
                }
                let source_body = Expr {
                    hir_id: body.hir_id,
                    span: body.span,
                    kind: H::Block(body),
                };
                let statements = if count == 0 {
                    self.inline_selection = Some((expr.hir_id, usize::MAX));
                    self.in_unselected_inline = true;
                    Vec::new()
                } else {
                    let bindings = self
                        .select_inline_pattern(
                            &pattern,
                            &Term::Integer(current.clone()),
                            &source_body,
                        )?
                        .ok_or_else(|| error(pat.span, "inline range binding did not match"))?;
                    // These Integer bindings exist only during expansion. Their
                    // uses are substituted through generator_data; no runtime
                    // storage may be allocated for a comptime-only value.
                    self.materialize_inline_bindings(&bindings, &source_body)?;
                    Vec::new()
                };
                let selected = if let Some(guard) = guard {
                    if count == 0 {
                        if !self.checking_open_behavior {
                            self.with_staged_input(true, |this| {
                                this.expr(guard, Some(&p(P::Bool)))
                            })?;
                        }
                        false
                    } else {
                        let comptime_interpretor::Value::Bool(selected) =
                            self.required_value(guard, P::Bool)?
                        else {
                            return Err(error(
                                guard.span,
                                "inline range guard must produce a comptime bool",
                            ));
                        };
                        selected
                    }
                } else {
                    true
                };
                if !selected {
                    self.inline_selection = Some((expr.hir_id, usize::MAX));
                    self.in_unselected_inline = true;
                }
                let checked = discard_result(self.block(body, None)?);
                if count == 0 {
                    if self.checking_open_behavior {
                        // Keep the abstract iteration for definition-side P4
                        // checking, including bodies with no concrete iteration.
                        iterations.push(checked);
                    }
                    break;
                }
                if selected {
                    let connections = saved_locals
                        .keys()
                        .chain(self.loops.iter())
                        .map(|id| (*id, *id))
                        .collect();
                    iterations.push(self.wrap_inline_iteration(
                        checked,
                        statements,
                        identity,
                        expr.span,
                        body.span,
                        connections,
                    )?);
                }
                self.inline_selection = saved_selection;
                self.in_unselected_inline = saved_suppressed;
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
            iterations.push(n::Expr {
                source: source(expr.span),
                ty: p(P::Never),
                kind: K::Break(identity),
            });
            let body = n::Expr {
                source: source(expr.span),
                ty: p(P::Never),
                kind: K::Block {
                    statements: iterations,
                    result: None,
                },
            };
            let result = self.expanded_loop(body, identity, expr.span)?;
            if let Some(expected) = expected {
                self.require_expression(&result.ty, expected, expr.span)?;
            }
            Ok(result)
        })();
        self.loops.pop();
        self.locals = saved_locals;
        self.generator_data = saved_data;
        self.immutable = saved_immutable;
        self.context = saved_context;
        self.inline_selection = saved_selection;
        self.in_unselected_inline = saved_suppressed;
        result
    }
    pub(super) fn tuple_iteration_patterns<'h>(
        &self,
        pattern: &'h hir::pattern::Pattern<'h>,
    ) -> Result<(&'h hir::pattern::Pattern<'h>, &'h hir::pattern::Pattern<'h>), Error> {
        match pattern.kind {
            hir::pattern::PatternKind::Tuple(fields) if fields.len() == 2 => {
                Ok((&fields[0], &fields[1]))
            }
            _ => Err(error(
                pattern.span,
                "tuple_enumerate iteration requires an (index, element) pattern",
            )),
        }
    }

    fn expand_inline_tuple(
        &mut self,
        expr: &Expr<'_>,
        input: &Expr<'_>,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let H::For {
            pat, guard, body, ..
        } = expr.kind
        else {
            unreachable!()
        };
        let (index_source, element_source) = self.tuple_iteration_patterns(pat)?;
        let (input, elements) = self.tuple_enumeration_input(input)?;
        if elements.is_empty() {
            return Err(error(
                expr.span,
                "empty tuple_enumerate body checking is not yet implemented",
            ));
        }
        if elements.len() as u64 > self.required_steps {
            return Err(error(
                expr.span,
                "tuple_enumerate expansion budget exhausted",
            ));
        }
        self.required_steps -= elements.len() as u64;
        let saved_locals = self.locals.clone();
        let saved_data = self.generator_data.clone();
        let saved_immutable = self.immutable.clone();
        let saved_context = self.context.clone();
        let saved_selection = self.inline_selection;
        let saved_suppressed = self.in_unselected_inline;
        let suppressed = self.in_unselected_inline;
        let identity = local(expr.hir_id);
        self.loops.push(identity);
        let result = (|| {
            // Destructure once into private locals. Unvisited elements remain
            // owned by the outer block and are cleaned when that block exits.
            let mut storage = Vec::new();
            for _ in &elements {
                let id = n::LocalId {
                    owner: u32::MAX,
                    index: self.next_temporary_binding,
                };
                self.next_temporary_binding = self
                    .next_temporary_binding
                    .checked_add(1)
                    .ok_or_else(|| error(expr.span, "tuple iteration identity space exhausted"))?;
                storage.push(id);
            }
            let connections: BTreeMap<_, _> = saved_locals
                .keys()
                .chain(self.loops.iter())
                .map(|id| (*id, *id))
                .collect();
            let mut iterations = Vec::new();
            for (index, (ty, temporary)) in elements.iter().zip(&storage).enumerate() {
                self.locals = saved_locals.clone();
                self.generator_data = saved_data.clone();
                self.immutable = saved_immutable.clone();
                self.context = saved_context.clone();
                self.inline_selection = saved_selection;
                self.in_unselected_inline = saved_suppressed;
                let index_pattern = self.pattern(index_source, &p(P::Integer))?;
                if !self.pattern_exhaustive(&index_pattern, &p(P::Integer)) {
                    return Err(error(
                        index_source.span,
                        "tuple_enumerate index pattern must be irrefutable",
                    ));
                }
                let index_bindings: Vec<_> = self
                    .locals
                    .iter()
                    .filter(|(id, _)| !saved_locals.contains_key(id))
                    .map(|(id, ty)| (*id, ty.clone()))
                    .collect();
                let source_body = Expr {
                    hir_id: body.hir_id,
                    span: body.span,
                    kind: H::Block(body),
                };
                if suppressed {
                    if self.checking_open_behavior {
                        for (binding, ty) in &index_bindings {
                            self.ordinary_behavior_inputs.insert(*binding, ty.clone());
                        }
                    }
                } else {
                    let bindings = self
                        .select_inline_pattern(
                            &index_pattern,
                            &Term::Integer(comptime_runtime::Integer::from_u128(index as u128)),
                            &source_body,
                        )?
                        .ok_or_else(|| {
                            error(index_source.span, "tuple index pattern did not match")
                        })?;
                    self.materialize_inline_bindings(&bindings, &source_body)?;
                }
                let element_pattern = self.pattern(element_source, ty)?;
                if !self.pattern_exhaustive(&element_pattern, ty) {
                    return Err(error(
                        element_source.span,
                        "tuple_enumerate element pattern must be irrefutable",
                    ));
                }
                let selected = if let Some(guard) = guard {
                    if suppressed {
                        if !self.checking_open_behavior {
                            self.with_staged_input(true, |this| {
                                this.expr(guard, Some(&p(P::Bool)))
                            })?;
                        }
                        true
                    } else {
                        let comptime_interpretor::Value::Bool(selected) =
                            self.required_value(guard, P::Bool)?
                        else {
                            return Err(error(
                                guard.span,
                                "tuple iteration guard must produce a comptime bool",
                            ));
                        };
                        selected
                    }
                } else {
                    true
                };
                if !selected {
                    self.inline_selection = Some((expr.hir_id, usize::MAX));
                    self.in_unselected_inline = true;
                }
                let checked = discard_result(self.block(body, None)?);
                let value = n::Expr {
                    source: source(expr.span),
                    ty: ty.clone(),
                    kind: K::Local(*temporary),
                };
                let delivered = n::LocalId {
                    owner: u32::MAX,
                    index: self.next_temporary_binding,
                };
                self.next_temporary_binding = self
                    .next_temporary_binding
                    .checked_add(1)
                    .ok_or_else(|| error(expr.span, "tuple delivery identity space exhausted"))?;
                let delivery_source = u32::try_from(self.expansion_sources.len())
                    .map_err(|_| error(expr.span, "tuple delivery source space exhausted"))?;
                self.expansion_sources.push(n::ExpansionSource {
                    original: source(pat.span),
                    site: source(expr.span),
                });
                let delivery = n::Expr {
                    source: n::SourceRange::generated(delivery_source),
                    ty: p(P::Void),
                    kind: K::Let {
                        binding: delivered,
                        value: Box::new(value),
                    },
                };
                let value = n::Expr {
                    source: source(expr.span),
                    ty: ty.clone(),
                    kind: K::Local(delivered),
                };
                let prefix = n::Expr {
                    source: source(pat.span),
                    ty: p(P::Void),
                    kind: K::LetPattern {
                        pattern: if selected {
                            element_pattern
                        } else {
                            n::Pattern::Wild
                        },
                        value: Box::new(value),
                        guard: None,
                        otherwise: None,
                    },
                };
                let checked = if selected {
                    checked
                } else {
                    n::Expr {
                        source: source(body.span),
                        ty: p(P::Void),
                        kind: K::Unit,
                    }
                };
                self.required_steps = self
                    .required_steps
                    .checked_sub(connections.len() as u64)
                    .ok_or_else(|| {
                        error(expr.span, "tuple iteration connection budget exhausted")
                    })?;
                let mut iteration_connections = connections.clone();
                iteration_connections.insert(*temporary, *temporary);
                if suppressed && self.checking_open_behavior {
                    // Abstract staged inputs belong to the definition witness,
                    // not to this expansion's newly allocated runtime locals.
                    self.required_steps = self
                        .required_steps
                        .checked_sub(index_bindings.len() as u64)
                        .ok_or_else(|| {
                            error(expr.span, "tuple iteration connection budget exhausted")
                        })?;
                    iteration_connections.extend(index_bindings.iter().map(|(id, _)| (*id, *id)));
                }
                iterations.push(self.wrap_inline_iteration(
                    checked,
                    vec![delivery, prefix],
                    identity,
                    expr.span,
                    body.span,
                    iteration_connections,
                )?);
            }
            iterations.push(n::Expr {
                source: source(expr.span),
                ty: p(P::Never),
                kind: K::Break(identity),
            });
            let loop_body = n::Expr {
                source: source(expr.span),
                ty: p(P::Never),
                kind: K::Block {
                    statements: iterations,
                    result: None,
                },
            };
            let expanded = self.expanded_loop(loop_body, identity, expr.span)?;
            let fresh = u32::try_from(self.expansion_sources.len())
                .map_err(|_| error(expr.span, "tuple iteration source space exhausted"))?;
            self.expansion_sources.push(n::ExpansionSource {
                original: source(expr.span),
                site: source(expr.span),
            });
            let binding = n::Expr {
                source: source(pat.span),
                ty: p(P::Void),
                kind: K::LetPattern {
                    pattern: n::Pattern::Tuple(
                        storage
                            .into_iter()
                            .map(|binding| n::Pattern::Binding {
                                binding,
                                inner: Box::new(n::Pattern::Wild),
                            })
                            .collect(),
                    ),
                    value: Box::new(input),
                    guard: None,
                    otherwise: None,
                },
            };
            let result = n::Expr {
                source: n::SourceRange::generated(fresh),
                ty: expanded.ty.clone(),
                kind: K::Block {
                    statements: vec![binding],
                    result: Some(Box::new(expanded)),
                },
            };
            if let Some(expected) = expected {
                self.require_expression(&result.ty, expected, expr.span)?;
            }
            Ok(result)
        })();
        self.loops.pop();
        self.locals = saved_locals;
        self.generator_data = saved_data;
        self.immutable = saved_immutable;
        self.context = saved_context;
        self.inline_selection = saved_selection;
        self.in_unselected_inline = saved_suppressed;
        result
    }

    /// One expanded iteration owns its own runtime bindings and cleanup scope.
    /// The caller supplies already checked prefix bindings; their expressions
    /// remain runtime operations and are never evaluated by this wrapper.
    fn wrap_inline_iteration(
        &mut self,
        mut checked: n::Expr,
        mut statements: Vec<n::Expr>,
        loop_identity: n::LocalId,
        loop_span: Span,
        body_span: Span,
        connections: BTreeMap<n::LocalId, n::LocalId>,
    ) -> Result<n::Expr, Error> {
        let iteration = n::LocalId {
            owner: u32::MAX,
            index: self.next_temporary_binding,
        };
        self.next_temporary_binding = self
            .next_temporary_binding
            .checked_add(1)
            .ok_or_else(|| error(loop_span, "inline loop identity space exhausted"))?;
        let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let rewritten = nir::visit::expression_mut(&mut checked, &mut fuel, |node, _| {
            if matches!(node.kind, K::Continue(target) if target == loop_identity) {
                node.kind = K::Break(iteration);
            }
            Ok(())
        });
        self.required_steps -= (before - fuel) as u64;
        rewritten.map_err(|e| error(body_span, e))?;
        statements.push(checked);
        statements.push(n::Expr {
            source: source(body_span),
            ty: p(P::Never),
            kind: K::Break(iteration),
        });
        let iteration_body = n::Expr {
            // The wrapper and source body have distinct cleanup boundaries.
            source: source(loop_span),
            ty: p(P::Never),
            kind: K::Block {
                statements,
                result: None,
            },
        };
        let mut expanded = self.expanded_loop(iteration_body, iteration, body_span)?;
        constants::charge_copy(&expanded, &mut self.required_steps, loop_span)?;
        cmtt::freshen(
            &mut expanded,
            connections,
            &mut self.next_temporary_binding,
            &mut self.expansion_sources,
            body_span,
        )?;
        Ok(expanded)
    }
    /// Both the outer exit and each iteration use the caller's compilation
    /// budget. No generated loop receives a fresh, unaccounted traversal cap.
    fn expanded_loop(
        &mut self,
        body: n::Expr,
        identity: n::LocalId,
        span: Span,
    ) -> Result<n::Expr, Error> {
        let before = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let mut fuel = before;
        let completes = control::loop_may_complete(&body, identity, &mut fuel);
        self.required_steps -= (before - fuel) as u64;
        let completes = completes.map_err(|e| error(span, e))?;
        Ok(n::Expr {
            source: source(span),
            ty: p(if completes { P::Void } else { P::Never }),
            kind: K::Loop {
                identity,
                body: Box::new(body),
            },
        })
    }
}
