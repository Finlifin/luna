//! Bounded source-range unrolling; runtime bodies retain their own scopes.
use super::*;

mod comptime_ranges;

/// Validate and split a `'tuple_indexed` pattern into `(index_pat, element_pat)`.
pub(in crate::program) fn tuple_indexed_parts<'hir>(
    pat: &'hir hir::pattern::Pattern<'hir>,
) -> Result<
    (
        Option<&'hir hir::pattern::Pattern<'hir>>,
        Option<&'hir hir::pattern::Pattern<'hir>>,
    ),
    Error,
> {
    match &pat.kind {
        hir::pattern::PatternKind::Wild => Ok((None, None)),
        hir::pattern::PatternKind::Tuple(fields) if fields.len() == 2 => {
            let index_bindings: BTreeSet<_> = pattern_binding_ids(&fields[0]).into_iter().collect();
            let elem_bindings = pattern_binding_ids(&fields[1]);
            if elem_bindings.iter().any(|id| index_bindings.contains(id)) {
                return Err(error(
                    pat.span,
                    "duplicate binding in tuple_indexed pattern",
                ));
            }
            Ok((Some(&fields[0]), Some(&fields[1])))
        }
        _ => Err(error(
            pat.span,
            "tuple_indexed pattern must be a 2-tuple (index, element) or wildcard _",
        )),
    }
}

impl Checker<'_, '_> {
    /// Check availability without invoking an inactive staged computation.
    pub(super) fn check_inline_input_availability(
        &mut self,
        value: &n::Expr,
        staged: &BTreeSet<n::LocalId>,
        span: Span,
    ) -> Result<(), Error> {
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let free = nir::free_bindings::collect(value, &mut fuel);
        self.required_steps -= (before - fuel) as u64;
        // Definition checking precedes required invocation. Ordinary parameters
        // and locals of a comptime function are already in that phase even
        // while no invocation is currently running.
        let comptime_owner = self
            .signature_owner
            .and_then(|owner| self.package.item(owner))
            .is_some_and(|item| {
                matches!(&item.kind,
                ItemKind::Fn(signature, _) | ItemKind::FnDeclaration(signature)
                if signature.modifiers.is_comptime)
            });
        for binding in free.map_err(|message| error(span, message))? {
            if comptime_owner
                || self.in_required_comptime
                || staged.contains(&binding)
                || self.open_staged_locals.contains(&binding)
                || self.generator_data.contains_key(&binding)
                || self.input_facts.has_identity(&binding)
            {
                continue;
            }
            return Err(error(
                span,
                "inline computation cannot read a runtime parameter or unstaged local",
            ));
        }
        let mut fuel = usize::try_from(self.required_steps).unwrap_or(usize::MAX);
        let before = fuel;
        let checked = nir::visit::expression_in_scope(value, &mut fuel, |expr, fuel| {
            if let K::Call { callee, .. } = &expr.kind {
                if !callee.ty.is_comptime_callable() {
                    let id = match callee.kind {
                        K::Function(id)
                        | K::FunctionInstance {
                            declaration: id, ..
                        } => Some(id),
                        _ => None,
                    };
                    let mut normal_head = false;
                    if let Some(id) = id {
                        *fuel = fuel
                            .checked_sub(1)
                            .ok_or("inline phase checking budget exhausted")?;
                        if let Some(item) = self.source_declarations.item(id) {
                            normal_head = matches!(&item.kind,
                                    ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig)
                                    if sig.modifiers.is_comptime || sig.modifiers.is_normal_form_head);
                        }
                    }
                    if !normal_head {
                        return Err(
                            "runtime callable is unavailable in an inline computation".into()
                        );
                    }
                }
            }
            Ok(())
        });
        self.required_steps -= (before - fuel) as u64;
        checked.map_err(|message| error(span, message))
    }

    /// Expose only the tuple layout; the input retains its nominal type and
    /// runtime evaluation. No comptime invocation is needed to inspect shape.
    pub(super) fn tuple_enumeration_input(
        &mut self,
        input: &Expr<'_>,
    ) -> Result<(n::Expr, Vec<Term>), Error> {
        let span = input.span;
        let expected = if matches!(input.kind, H::Unit) {
            Some(self.tuple_value_type(Vec::new(), span)?)
        } else {
            None
        };
        let input = self.expr(input, expected.as_ref())?;
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
            return self.expand_inline_tuple(expr, input, false, expected);
        }
        if let H::TakeTupleIndexed(input) = iter.kind {
            return self.expand_inline_tuple(expr, input, true, expected);
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
        if self.closed_comptime_control_body() {
            let mut static_bounds = None;
            if let Ok(checked_start) = self.expr(start, Some(&p(P::Integer)))
                && let Ok(checked_end) = self.expr(end, Some(&p(P::Integer)))
            {
                let mut budget = comptime_runtime::Budget {
                    steps: usize::try_from(self.required_steps)
                        .unwrap_or(usize::MAX)
                        .min(100_000),
                    max_limbs: 4096,
                };
                if let Ok(Some(Term::Integer(s))) =
                    self.normal_input_hint(&checked_start, &mut budget)
                    && let Ok(Some(Term::Integer(e))) =
                        self.normal_input_hint(&checked_end, &mut budget)
                    && let Ok(diff) = e.subtract(&s, &mut budget)
                {
                    let count = if inclusive {
                        diff.add(&comptime_runtime::Integer::from_u128(1), &mut budget)
                            .ok()
                    } else {
                        Some(diff)
                    };
                    if let Some(ref count) = count
                        && let Ok(count_usize) = count
                            .to_u128()
                            .map_err(|_| ())
                            .and_then(|c| usize::try_from(c).map_err(|_| ()))
                        && count_usize <= 4096
                    {
                        static_bounds = Some((s, count_usize));
                    } else if count
                        .as_ref()
                        .is_some_and(|c| *c <= comptime_runtime::Integer::from_u128(0))
                    {
                        static_bounds = Some((s, 0));
                    }
                }
            }
            if static_bounds.is_none() {
                return self.comptime_range(expr, start, end, inclusive, expected);
            }
        }
        let suppressed = self.in_unselected_inline
            && self
                .inline_selection
                .is_some_and(|(id, _)| id.owner == expr.hir_id.owner);
        let (mut current, count) = if suppressed {
            // The ordinary description already checked staged endpoints. The
            // behavior witness models runtime bodies, not boundary invocation.
            if !self.checking_open_behavior {
                for boundary in [start, end] {
                    let checked = self.expr(boundary, Some(&p(P::Integer)))?;
                    self.check_inline_input_availability(
                        &checked,
                        &BTreeSet::new(),
                        boundary.span,
                    )?;
                }
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
        let input_boundary = self.input_facts.lexical_boundary();
        let saved_data = self.generator_data.clone();
        let saved_immutable = self.immutable.clone();
        let saved_context = self.context.clone();
        let saved_selection = self.inline_selection;
        let saved_suppressed = self.in_unselected_inline;
        let saved_runtime_origins = self.local_runtime_origins.clone();
        let identity = local(expr.hir_id);
        self.control.loops.push(identity);
        let result = (|| {
            let mut iterations = Vec::new();
            // An empty range still checks its ordinary body, without invoking
            // nested inline conditions that have no selected iteration.
            for index in 0..count.max(1) {
                self.locals = saved_locals.clone();
                self.input_facts.leave_lexical(&input_boundary);
                self.generator_data = saved_data.clone();
                self.immutable = saved_immutable.clone();
                self.context = saved_context.clone();
                self.local_runtime_origins = saved_runtime_origins.clone();
                let saved_delivery_ids: BTreeSet<_> =
                    self.local_runtime_deliveries.keys().copied().collect();
                self.immutable.extend(pattern_binding_ids(pat));
                let pattern = self.pattern(pat, &p(P::Integer))?;
                let staged: BTreeSet<_> = self
                    .locals
                    .keys()
                    .filter(|id| !saved_locals.contains_key(id))
                    .copied()
                    .collect();
                if !self.checking_open_behavior {
                    self.check_inline_pattern_availability(&pattern, pat.span)?;
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
                let (matched, statements) = if count == 0 {
                    (false, Vec::new())
                } else if let Some(bindings) = self.select_inline_pattern(
                    &pattern,
                    &Term::Integer(current.clone()),
                    &source_body,
                )? {
                    // Only successful matches export staged bindings. Failed
                    // iterations still receive ordinary body/phase checking.
                    let statements = self.materialize_inline_bindings(&bindings, &source_body)?;
                    (true, statements)
                } else {
                    (false, Vec::new())
                };
                if !matched {
                    self.inline_selection = Some((expr.hir_id, usize::MAX));
                    self.in_unselected_inline = true;
                }
                let selected = if let Some(guard) = guard {
                    if !matched {
                        if !self.checking_open_behavior {
                            let checked = self.with_staged_input(true, |this| {
                                this.expr(guard, Some(&p(P::Bool)))
                            })?;
                            self.check_inline_input_availability(&checked, &staged, guard.span)?;
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
                    matched
                };
                if !selected {
                    self.inline_selection = Some((expr.hir_id, usize::MAX));
                    self.in_unselected_inline = true;
                }
                // Failed/empty iterations have no concrete index value, but
                // their binders still belong to comptime. Nested suppressed
                // computations may depend on them without invoking them.
                let saved_staged = self.open_staged_locals.clone();
                let saved_members = self.open_member_locals.clone();
                self.open_staged_locals.extend(staged.iter().copied());
                let checked = self.block(body, None);
                self.open_staged_locals = saved_staged;
                self.open_member_locals = saved_members;
                let checked = self.discard_result(checked?, body.span)?;
                if count == 0 {
                    if self.checking_open_behavior {
                        // Keep the abstract iteration for definition-side P4
                        // checking, including bodies with no concrete iteration.
                        iterations.push(checked);
                    } else {
                        // Retain closed unselected loop bodies under an unreachable
                        // false guard so NIR phase and safety passes still inspect them.
                        let mut dead_statements: Vec<n::Expr> = self
                            .locals
                            .iter()
                            .filter(|(id, _)| !saved_locals.contains_key(id))
                            .map(|(id, ty)| n::Expr {
                                source: source(pat.span),
                                ty: p(P::Void),
                                kind: K::Let {
                                    binding: *id,
                                    value: Box::new(n::Expr {
                                        source: source(pat.span),
                                        ty: ty.clone(),
                                        kind: K::Literal(n::Literal::Integer("0".into())),
                                    }),
                                },
                            })
                            .collect();
                        let dead_ty = checked.ty.clone();
                        dead_statements.push(checked);
                        iterations.push(n::Expr {
                            source: source(expr.span),
                            ty: p(P::Void),
                            kind: K::If {
                                condition: Box::new(n::Expr {
                                    source: source(iter.span),
                                    ty: p(P::Bool),
                                    kind: K::Literal(n::Literal::Bool(false)),
                                }),
                                then_branch: Box::new(n::Expr {
                                    source: source(body.span),
                                    ty: dead_ty,
                                    kind: K::Block {
                                        statements: dead_statements,
                                        result: None,
                                    },
                                }),
                                else_branch: Some(Box::new(n::Expr {
                                    source: source(expr.span),
                                    ty: p(P::Void),
                                    kind: K::Unit,
                                })),
                            },
                        });
                    }
                    break;
                }
                if selected {
                    let connections = saved_locals
                        .keys()
                        .chain(self.control.loops.iter())
                        .map(|id| (*id, *id))
                        .collect();
                    iterations.push(self.wrap_inline_iteration(
                        checked,
                        statements,
                        identity,
                        expr.span,
                        body.span,
                        connections,
                        &saved_delivery_ids,
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
        self.control.loops.pop();
        self.locals = saved_locals;
        self.input_facts.leave_lexical(&input_boundary);
        self.generator_data = saved_data;
        self.immutable = saved_immutable;
        self.context = saved_context;
        self.inline_selection = saved_selection;
        self.in_unselected_inline = saved_suppressed;
        self.local_runtime_origins = saved_runtime_origins;
        result
    }

    fn expand_inline_tuple(
        &mut self,
        expr: &Expr<'_>,
        input: &Expr<'_>,
        indexed: bool,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let H::For {
            pat, guard, body, ..
        } = expr.kind
        else {
            unreachable!()
        };
        let indexed_pats = if indexed {
            Some(tuple_indexed_parts(pat)?)
        } else {
            None
        };
        let input_span = input.span;
        let (input, elements, abstract_tuple) = {
            let expected = if matches!(input.kind, H::Unit) {
                Some(self.tuple_value_type(Vec::new(), input_span)?)
            } else {
                None
            };
            let checked_input = self.expr(input, expected.as_ref())?;
            let shape = self.structural_type(&checked_input.ty, input_span)?;
            match shape {
                Term::Tuple(elements) => (checked_input, elements, false),
                _ if (self.in_unselected_inline || self.checking_open_behavior)
                    && self.is_proven_tuple_type(&checked_input.ty, input_span)? =>
                {
                    (checked_input, Vec::new(), true)
                }
                _ => {
                    return Err(error(
                        input_span,
                        if indexed {
                            "tuple_indexed requires a tuple value with established structural type evidence"
                        } else {
                            "tuple_enumerate requires a tuple value with established structural type evidence"
                        },
                    ));
                }
            }
        };
        let observed_elements = if abstract_tuple {
            None
        } else {
            self.observed_tuple_elements(&input, elements.len(), expr.span)?
        };
        let empty = elements.is_empty();
        let charge = elements.len().max(1) as u64;
        if charge > self.required_steps {
            return Err(error(
                expr.span,
                "tuple_enumerate expansion budget exhausted",
            ));
        }
        self.required_steps -= charge;
        let saved_locals = self.locals.clone();
        let input_boundary = self.input_facts.lexical_boundary();
        let saved_data = self.generator_data.clone();
        let saved_immutable = self.immutable.clone();
        let saved_context = self.context.clone();
        let saved_selection = self.inline_selection;
        let saved_suppressed = self.in_unselected_inline;
        let saved_runtime_origins = self.local_runtime_origins.clone();
        let saved_staged = self.open_staged_locals.clone();
        let saved_members = self.open_member_locals.clone();
        let suppressed = self.in_unselected_inline;
        let identity = local(expr.hir_id);
        self.control.loops.push(identity);
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
                .chain(self.control.loops.iter())
                .map(|id| (*id, *id))
                .collect();
            let mut iterations = Vec::new();
            let already_checked_empty = empty
                && self
                    .open_functions
                    .get(&expr.hir_id.owner)
                    .is_some_and(|d| d.checked_empty_iteration(expr.hir_id));
            if abstract_tuple || (empty && !already_checked_empty) {
                let binder = self.fresh_type_binder(expr.hir_id.owner, expr.span)?;
                let element_ty = Term::Rigid {
                    declaration: binder.declaration,
                    sort: Sort::Type,
                };
                if self.checking_open_behavior {
                    self.definition_refinements
                        .tuple_elements
                        .push(binder.declaration);
                }
                self.locals = saved_locals.clone();
                self.input_facts.leave_lexical(&input_boundary);
                self.generator_data = saved_data.clone();
                self.immutable = saved_immutable.clone();
                self.context = saved_context.clone();
                self.local_runtime_origins = saved_runtime_origins.clone();
                self.open_staged_locals = saved_staged.clone();
                self.open_member_locals = saved_members.clone();
                self.inline_selection = Some((expr.hir_id, usize::MAX));
                self.in_unselected_inline = true;
                let mut index_staged = BTreeSet::new();
                let elem_pat = if let Some((index_pat, elem_pat)) = indexed_pats {
                    if let Some(index_pat) = index_pat {
                        let before_index = self.locals.clone();
                        self.immutable.extend(pattern_binding_ids(index_pat));
                        let index_pattern = self.with_staged_input(true, |this| {
                            this.pattern(index_pat, &p(P::Integer))
                        })?;
                        for (id, ty) in &self.locals {
                            if !before_index.contains_key(id) {
                                index_staged.insert(*id);
                                if self.checking_open_behavior {
                                    self.ordinary_behavior_inputs.insert(*id, ty.clone());
                                }
                            }
                        }
                        if !self.checking_open_behavior {
                            self.check_inline_pattern_availability(&index_pattern, index_pat.span)?;
                        }
                        self.open_staged_locals.extend(index_staged.iter().copied());
                    }
                    elem_pat
                } else {
                    Some(pat)
                };
                if let Some(elem_pat) = elem_pat {
                    let before_elem = self.locals.clone();
                    self.immutable.extend(pattern_binding_ids(elem_pat));
                    let _element_pattern = self.pattern(elem_pat, &element_ty)?;
                    if self.checking_open_behavior {
                        for (id, ty) in &self.locals {
                            if !before_elem.contains_key(id) {
                                self.ordinary_behavior_inputs.insert(*id, ty.clone());
                            }
                        }
                    }
                }
                if let Some(guard) = guard
                    && !self.checking_open_behavior
                {
                    let checked =
                        self.with_staged_input(true, |this| this.expr(guard, Some(&p(P::Bool))))?;
                    self.check_inline_input_availability(&checked, &index_staged, guard.span)?;
                }
                let checked = self.block(body, None)?;
                let checked = self.discard_result(checked, body.span)?;
                if self.checking_open_behavior {
                    iterations.push(checked);
                }
            }
            for (index, (ty, temporary)) in elements.iter().zip(&storage).enumerate() {
                self.locals = saved_locals.clone();
                self.input_facts.leave_lexical(&input_boundary);
                self.generator_data = saved_data.clone();
                self.immutable = saved_immutable.clone();
                self.context = saved_context.clone();
                self.local_runtime_origins = saved_runtime_origins.clone();
                self.open_staged_locals = saved_staged.clone();
                self.open_member_locals = saved_members.clone();
                let saved_delivery_ids: BTreeSet<_> =
                    self.local_runtime_deliveries.keys().copied().collect();
                self.inline_selection = saved_selection;
                self.in_unselected_inline = saved_suppressed;
                let source_body = Expr {
                    hir_id: body.hir_id,
                    span: body.span,
                    kind: H::Block(body),
                };
                let mut preamble = Vec::new();
                let mut index_staged = BTreeSet::new();
                let (index_matched, elem_pat) = if let Some((index_pat, elem_pat)) = indexed_pats {
                    let index_matched = if let Some(index_pat) = index_pat {
                        let before_index = self.locals.clone();
                        self.immutable.extend(pattern_binding_ids(index_pat));
                        let index_pattern = self.with_staged_input(true, |this| {
                            this.pattern(index_pat, &p(P::Integer))
                        })?;
                        index_staged = self
                            .locals
                            .keys()
                            .filter(|id| !before_index.contains_key(id))
                            .copied()
                            .collect();
                        if self.checking_open_behavior {
                            for (id, ty) in &self.locals {
                                if !before_index.contains_key(id) {
                                    self.ordinary_behavior_inputs.insert(*id, ty.clone());
                                }
                            }
                        } else {
                            self.check_inline_pattern_availability(&index_pattern, index_pat.span)?;
                        }
                        let index_term =
                            Term::Integer(comptime_runtime::Integer::from_u128(index as u128));
                        if let Some(bindings) =
                            self.select_inline_pattern(&index_pattern, &index_term, &source_body)?
                        {
                            let stmts =
                                self.materialize_inline_bindings(&bindings, &source_body)?;
                            preamble.extend(stmts);
                            true
                        } else {
                            false
                        }
                    } else {
                        true
                    };
                    (index_matched, elem_pat)
                } else {
                    (true, Some(pat))
                };
                self.open_staged_locals.extend(index_staged.iter().copied());
                let (element_pattern, element_irrefutable) = if let Some(elem_pat) = elem_pat {
                    self.immutable.extend(pattern_binding_ids(elem_pat));
                    let element_pattern = self.pattern(elem_pat, ty)?;
                    if let Some(observed) = &observed_elements {
                        self.record_observed_pattern(
                            &element_pattern,
                            &observed[index],
                            elem_pat.span,
                        )?;
                    }
                    let irrefutable = self.pattern_exhaustive(&element_pattern, ty);
                    (element_pattern, irrefutable)
                } else {
                    (n::Pattern::Wild, true)
                };
                if !index_matched {
                    self.inline_selection = Some((expr.hir_id, usize::MAX));
                    self.in_unselected_inline = true;
                }
                let selected = if let Some(guard) = guard {
                    if !index_matched {
                        if !self.checking_open_behavior {
                            let checked = self.with_staged_input(true, |this| {
                                this.expr(guard, Some(&p(P::Bool)))
                            })?;
                            self.check_inline_input_availability(
                                &checked,
                                &index_staged,
                                guard.span,
                            )?;
                        }
                        false
                    } else if suppressed {
                        if !self.checking_open_behavior {
                            let checked = self.with_staged_input(true, |this| {
                                this.expr(guard, Some(&p(P::Bool)))
                            })?;
                            self.check_inline_input_availability(
                                &checked,
                                &index_staged,
                                guard.span,
                            )?;
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
                    index_matched
                };
                if !selected {
                    self.inline_selection = Some((expr.hir_id, usize::MAX));
                    self.in_unselected_inline = true;
                }
                let checked = self.block(body, None)?;
                let checked = self.discard_result(checked, body.span)?;
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
                        otherwise: if selected && !element_irrefutable {
                            Some(Box::new(n::Expr {
                                source: source(pat.span),
                                ty: p(P::Never),
                                kind: K::Continue(identity),
                            }))
                        } else {
                            None
                        },
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
                preamble.push(delivery);
                preamble.push(prefix);
                iterations.push(self.wrap_inline_iteration(
                    checked,
                    preamble,
                    identity,
                    expr.span,
                    body.span,
                    iteration_connections,
                    &saved_delivery_ids,
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
                    pattern: if abstract_tuple {
                        n::Pattern::Wild
                    } else {
                        n::Pattern::Tuple(
                            storage
                                .into_iter()
                                .map(|binding| n::Pattern::Binding {
                                    binding,
                                    inner: Box::new(n::Pattern::Wild),
                                })
                                .collect(),
                        )
                    },
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
        self.control.loops.pop();
        self.locals = saved_locals;
        self.input_facts.leave_lexical(&input_boundary);
        self.generator_data = saved_data;
        self.immutable = saved_immutable;
        self.context = saved_context;
        self.inline_selection = saved_selection;
        self.in_unselected_inline = saved_suppressed;
        self.local_runtime_origins = saved_runtime_origins;
        self.open_staged_locals = saved_staged;
        self.open_member_locals = saved_members;
        result
    }

    /// One expanded iteration owns its own runtime bindings and cleanup scope.
    /// The caller supplies already checked prefix bindings; their expressions
    /// remain runtime operations and are never evaluated by this wrapper.
    pub(super) fn wrap_inline_iteration(
        &mut self,
        mut checked: n::Expr,
        mut statements: Vec<n::Expr>,
        loop_identity: n::LocalId,
        loop_span: Span,
        body_span: Span,
        connections: BTreeMap<n::LocalId, n::LocalId>,
        previous_runtime_deliveries: &BTreeSet<nir::RuntimeId>,
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
        let rewritten = (|| {
            // Element-pattern failure lives in the prefix and must skip only
            // this unrolled iteration, just like a source-body continue.
            for expression in statements.iter_mut().chain(std::iter::once(&mut checked)) {
                nir::visit::expression_mut(expression, &mut fuel, |node, _| {
                    if matches!(node.kind, K::Continue(target) if target == loop_identity) {
                        node.kind = K::Break(iteration);
                    }
                    Ok(())
                })?;
            }
            Ok::<_, String>(())
        })();
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
        let renaming = cmtt::freshen_with_renaming(
            &mut expanded,
            connections,
            &mut self.next_temporary_binding,
            &mut self.expansion_sources,
            body_span,
        )?;
        let new_deliveries: Vec<_> = self
            .local_runtime_deliveries
            .keys()
            .copied()
            .filter(|id| !previous_runtime_deliveries.contains(id))
            .collect();
        for rt_id in new_deliveries {
            let Some(mut entry) = self.local_runtime_deliveries.get(&rt_id).cloned() else {
                continue;
            };
            cmtt::freshen(
                &mut entry.expr,
                renaming.clone(),
                &mut self.next_temporary_binding,
                &mut self.expansion_sources,
                body_span,
            )?;
            if let Some((ref mut binding, _)) = entry.checked_epoch {
                *binding = *renaming.get(binding).unwrap_or(binding);
            }
            self.local_runtime_deliveries.insert(rt_id, entry);
        }
        Ok(expanded)
    }

    /// Both the outer exit and each iteration use the caller's compilation
    /// budget. No generated loop receives a fresh, unaccounted traversal cap.
    pub(super) fn expanded_loop(
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
