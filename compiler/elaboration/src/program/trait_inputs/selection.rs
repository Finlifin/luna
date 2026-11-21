//! Separate control and producer evaluation from contextual result delivery.
//! The selector runs as one required expression, so block locals, guards, and
//! nested subjects all remain in the same source activation.
use super::*;
mod aggregates;

#[derive(Clone)]
pub(in crate::program) struct Selection<'hir> {
    source: &'hir Expr<'hir>,
    alternatives: std::sync::Arc<[Alternative<'hir>]>,
    selected: usize,
}

enum Alternative<'hir> {
    Literal(&'hir Expr<'hir>),
    Producer {
        site: &'hir Expr<'hir>,
        domain: Option<Term>,
    },
    Frozen {
        site: &'hir Expr<'hir>,
        value: Box<n::Expr>,
    },
    Aggregate {
        site: &'hir Expr<'hir>,
        fields: Vec<Position<'hir>>,
    },
}

#[derive(Default)]
struct Position<'hir> {
    sites: Vec<Alternative<'hir>>,
    selected: Option<usize>,
}

impl<'hir> Selection<'hir> {
    pub(super) fn span(&self) -> Span {
        self.source.span
    }

    pub(super) fn prepare(
        checker: &mut Checker<'_, '_>,
        source: &'hir Expr<'hir>,
    ) -> Result<Option<ExplicitClause<'hir>>, Error> {
        if !matches!(
            source.kind,
            H::Tuple(_)
                | H::List(_)
                | H::Block(_)
                | H::If(_, _, Some(_))
                | H::When(_)
                | H::Match(..)
        ) {
            return Ok(None);
        }
        let arena = hir::HirArena::new();
        let mut alternatives = Vec::new();
        let Some(selector) = project(checker, &arena, source, &mut alternatives, 0)? else {
            return Ok(None);
        };
        if !alternatives.iter().any(|site| {
            matches!(
                site,
                Alternative::Literal(_) | Alternative::Aggregate { .. }
            )
        }) {
            return Ok(None);
        }
        // Describing an open body has no invocation-time selector. Its ordinary
        // staging rules check the original control when its domain is inferred.
        if !checker.in_required_comptime
            && (checker.describing_open_call
                || checker.checking_open_behavior
                || checker.in_unselected_inline)
        {
            // The partial call plan can settle after describe_generic_call
            // has left the explicit-operand scope.
            return Ok(Some(if checker.describing_open_call {
                ExplicitClause::OpenDescription(source)
            } else {
                ExplicitClause::Contextual(source)
            }));
        }
        // The Object is local to this invocation. Each selected producer keeps
        // its checked domain, including when it reads a block or pattern local.
        let (prepared, ()) = checker.prepare_required_root_with_live_inputs(
            source,
            required::RootKind::Expression,
            Vec::new(),
            false,
            &[],
            |this| {
                let root = this.expr(selector, Some(&p(P::Object)))?;
                retain_producer_domains(this, &root, &mut alternatives, source.span)?;
                Ok((root, ()))
            },
        )?;
        let (packet, _) = checker.invoke_required_publishing(
            prepared,
            Vec::new(),
            source.hir_id.owner,
            source.span,
        )?;
        let selected = receive(checker, packet, &mut alternatives, source.span)?;
        Ok(Some(ExplicitClause::Selection(Self {
            source,
            alternatives: alternatives.into(),
            selected,
        })))
    }

    pub(super) fn admit(
        &self,
        checker: &mut Checker<'_, '_>,
        domain: &Term,
    ) -> Result<Term, Error> {
        // Every reachable yield site is checked, including sites the selector
        // did not take. Execute only the selected delivery in the caller frame.
        let (root, ()) = checker.prepare_required_root_with_live_inputs(
            self.source,
            required::RootKind::Expression,
            Vec::new(),
            false,
            &[],
            |this| {
                let mut result = None;
                for (index, alternative) in self.alternatives.iter().enumerate() {
                    match alternative {
                        Alternative::Literal(expr) => {
                            let literal = this.expr(expr, Some(domain))?;
                            if index == self.selected {
                                result = Some(literal);
                            }
                        }
                        Alternative::Producer {
                            site,
                            domain: actual,
                        } => {
                            let actual = actual.as_ref().ok_or_else(|| {
                                error(site.span, "missing checked clause producer domain")
                            })?;
                            this.computed_delivery(actual, domain, site.span)?;
                        }
                        Alternative::Aggregate { site, fields } => {
                            let value = aggregates::check(
                                this,
                                site,
                                fields,
                                domain,
                                index == self.selected,
                            )?;
                            if index == self.selected {
                                result = Some(value);
                            }
                        }
                        Alternative::Frozen { site, value } => {
                            let span = site.span;
                            let delivered = this.deliver_computed_value(value, domain, span)?;
                            if let std::borrow::Cow::Borrowed(operand) = &delivered {
                                constants::charge_copy(operand, &mut this.required_steps, span)?;
                            }
                            result = Some(delivered.into_owned());
                        }
                    }
                }
                Ok((
                    result
                        .ok_or_else(|| error(self.source.span, "missing selected clause result"))?,
                    (),
                ))
            },
        )?;
        let (value, _) = checker.invoke_required_publishing(
            root,
            Vec::new(),
            self.source.hir_id.owner,
            self.source.span,
        )?;
        checker.required_normal_term(value, self.source.span)
    }
}

// The tree of selected packets mirrors the source aggregate. A skipped
// alternative has domains for checking, but cannot supply a field value.
fn receive(
    checker: &mut Checker<'_, '_>,
    packet: comptime_interpretor::Value,
    alternatives: &mut [Alternative<'_>],
    span: Span,
) -> Result<usize, Error> {
    checker.required_steps = checker
        .required_steps
        .checked_sub(1)
        .ok_or_else(|| error(span, "clause result delivery budget exhausted"))?;
    let comptime_interpretor::Value::Object(packet) = packet else {
        return Err(error(span, "invalid clause selection packet"));
    };
    let mut entries = packet
        .into_entries()
        .into_iter()
        .map(|entry| entry.into_value());
    let selected = match entries.next().map(|entry| entry.value) {
        Some(comptime_interpretor::Value::Integer(index)) => index
            .to_u128()
            .ok()
            .and_then(|index| usize::try_from(index).ok()),
        _ => None,
    }
    .filter(|index| *index < alternatives.len())
    .ok_or_else(|| error(span, "invalid clause selection index"))?;
    if let Alternative::Aggregate { fields, .. } = &mut alternatives[selected] {
        if entries.len() != fields.len() {
            return Err(error(span, "staged aggregate field count mismatch"));
        }
        for (field, entry) in fields.iter_mut().zip(entries) {
            field.selected = Some(receive(checker, entry.value, &mut field.sites, span)?);
        }
        return Ok(selected);
    }
    match (&mut alternatives[selected], entries.next()) {
        (Alternative::Literal(_), None) => {}
        (Alternative::Producer { site, .. }, Some(entry)) => {
            let span = site.span;
            let site = *site;
            let identity = checker.required_normal_term(entry.value, span)?;
            alternatives[selected] = Alternative::Frozen {
                site,
                value: frozen_operand(identity, entry.ty, span),
            };
        }
        _ => return Err(error(span, "invalid selected clause payload")),
    }
    if entries.next().is_some() {
        return Err(error(span, "unexpected clause selection payload"));
    }
    Ok(selected)
}

// Follow only the normalized control's result edges. Inner Objects, calls and
// guards belong to user code. A retained producer is checked after inference
// even if this invocation selected a different arm; it is never run again.
fn retain_producer_domains(
    checker: &mut Checker<'_, '_>,
    root: &n::Expr,
    alternatives: &mut [Alternative<'_>],
    span: Span,
) -> Result<(), Error> {
    let mut pending = vec![root];
    while let Some(result) = pending.pop() {
        checker.required_steps = checker
            .required_steps
            .checked_sub(1)
            .ok_or_else(|| error(span, "clause result inspection budget exhausted"))?;
        match &result.kind {
            K::If {
                then_branch,
                else_branch,
                ..
            } => {
                pending.push(then_branch);
                pending.extend(else_branch.as_deref());
            }
            K::Match { arms, .. } => pending.extend(arms.iter().map(|arm| &arm.body)),
            K::Block { result, .. } | K::CallPreparation { result, .. } => {
                pending.extend(result.as_deref())
            }
            K::Object { fields, .. } => {
                let Some((tag, operands)) = fields.split_first() else {
                    return Err(error(span, "missing clause site index"));
                };
                let index = match &tag.kind {
                    K::Literal(n::Literal::Integer(index)) => index.parse::<usize>().ok(),
                    _ => None,
                };
                match index.and_then(|index| alternatives.get_mut(index)) {
                    Some(Alternative::Literal(_)) if operands.is_empty() => {}
                    Some(Alternative::Producer { site, domain }) if operands.len() == 1 => {
                        *domain = Some(declaration_inputs::copy_input_domain(
                            &operands[0].ty,
                            &mut checker.required_steps,
                            site.span,
                        )?);
                    }
                    Some(Alternative::Aggregate { fields, .. })
                        if fields.len() == operands.len() =>
                    {
                        for (field, operand) in fields.iter_mut().zip(operands) {
                            retain_producer_domains(checker, operand, &mut field.sites, span)?;
                        }
                    }
                    _ => return Err(error(span, "invalid staged clause result site")),
                }
            }
            _ => return Err(error(span, "missing contextual result packet")),
        }
    }
    Ok(())
}

// A contextual literal keeps its source site. Other yields run inside their
// original lexical scope and return their typed value beside the site index.
fn project<'tree, 'src: 'tree>(
    checker: &mut Checker<'_, '_>,
    arena: &'tree hir::HirArena,
    source: &'src Expr<'src>,
    alternatives: &mut Vec<Alternative<'src>>,
    depth: usize,
) -> Result<Option<&'tree Expr<'tree>>, Error> {
    if depth >= 128 {
        return Err(error(
            source.span,
            "contextual input nesting limit exceeded",
        ));
    }
    checker.required_steps = checker
        .required_steps
        .checked_sub(1)
        .ok_or_else(|| error(source.span, "clause selection budget exhausted"))?;
    let mut child = |value| project(checker, arena, value, alternatives, depth + 1);
    let kind = match source.kind {
        H::Block(block) => {
            let Some(tail) = block.expr else {
                return Ok(None);
            };
            let Some(tail) = child(tail)? else {
                return Ok(None);
            };
            H::Block(arena.alloc_block(hir::Block {
                expr: Some(tail),
                ..*block
            }))
        }
        H::If(condition, yes, Some(no)) => {
            let Some(tail) = yes.expr else {
                return Ok(None);
            };
            let Some(tail) = child(tail)? else {
                return Ok(None);
            };
            let Some(no) = child(no)? else {
                return Ok(None);
            };
            H::If(
                condition,
                arena.alloc_block(hir::Block {
                    expr: Some(tail),
                    ..*yes
                }),
                Some(no),
            )
        }
        H::Match(subject, arms) => {
            let mut projected = Vec::new();
            for arm in arms {
                let Some(body) = child(arm.body)? else {
                    return Ok(None);
                };
                projected.push(hir::PatternArm {
                    body,
                    ..arm.clone()
                });
            }
            H::Match(subject, arena.alloc_arm_slice(projected))
        }
        H::When(arms) if arms.last().is_some_and(|arm| arm.cond.is_none()) => {
            let mut projected = Vec::new();
            for arm in arms {
                let Some(body) = child(arm.body)? else {
                    return Ok(None);
                };
                projected.push(hir::CondictionArm { body, ..*arm });
            }
            H::When(arena.alloc_cond_arm_slice(projected))
        }
        _ => {
            let index = arena.alloc_expr(Expr {
                hir_id: source.hir_id,
                span: source.span,
                kind: H::Lit(hir::Lit {
                    kind: hir::LitKind::Integer(alternatives.len().to_string()),
                    span: source.span,
                }),
            });
            let mut fields = vec![hir::expr::ObjectEntry::Value(index)];
            if checker.is_closed_clause_literal(source)? {
                alternatives.push(Alternative::Literal(source));
            } else if let H::Tuple(items) | H::List(items) = source.kind {
                let mut positions = Vec::with_capacity(items.len());
                for item in items {
                    let mut field = Position::default();
                    let Some(packet) = project(checker, arena, item, &mut field.sites, depth + 1)?
                    else {
                        return Ok(None);
                    };
                    fields.push(hir::expr::ObjectEntry::Value(packet));
                    positions.push(field);
                }
                alternatives.push(Alternative::Aggregate {
                    site: source,
                    fields: positions,
                });
            } else {
                fields.push(hir::expr::ObjectEntry::Value(source));
                alternatives.push(Alternative::Producer {
                    site: source,
                    domain: None,
                });
            }
            H::Object(fields)
        }
    };
    Ok(Some(arena.alloc_expr(Expr {
        hir_id: source.hir_id,
        span: source.span,
        kind,
    })))
}
