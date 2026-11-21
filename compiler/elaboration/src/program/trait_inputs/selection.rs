//! Separate control and producer evaluation from contextual result delivery.
//! The selector runs as one required expression, so block locals, guards, and
//! nested subjects all remain in the same source activation.
use super::*;

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
        checked: Option<Box<n::Expr>>,
    },
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
            H::Block(_) | H::If(_, _, Some(_)) | H::When(_) | H::Match(..)
        ) {
            return Ok(None);
        }
        let arena = hir::HirArena::new();
        let mut alternatives = Vec::new();
        let Some(selector) = project(checker, &arena, source, &mut alternatives, 0)? else {
            return Ok(None);
        };
        if !alternatives
            .iter()
            .any(|site| matches!(site, Alternative::Literal(_)))
        {
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
                retain_producers(this, &root, &mut alternatives, source.span)?;
                Ok((root, ()))
            },
        )?;
        let (packet, _) = checker.invoke_required_publishing(
            prepared,
            Vec::new(),
            source.hir_id.owner,
            source.span,
        )?;
        let comptime_interpretor::Value::Object(packet) = packet else {
            return Err(error(source.span, "invalid clause selection packet"));
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
        .ok_or_else(|| error(source.span, "invalid clause selection index"))?;
        match (&mut alternatives[selected], entries.next()) {
            (Alternative::Literal(_), None) => {}
            (Alternative::Producer { site, checked }, Some(entry)) => {
                let identity = checker.required_normal_term(entry.value, site.span)?;
                *checked = Some(frozen_operand(identity, entry.ty, site.span));
            }
            _ => return Err(error(source.span, "invalid selected clause payload")),
        }
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
                        Alternative::Producer { site, checked } => {
                            let operand = checked.as_deref().ok_or_else(|| {
                                error(site.span, "missing checked clause producer")
                            })?;
                            let delivered =
                                this.deliver_computed_value(operand, domain, site.span)?;
                            if index == self.selected {
                                result = Some(delivered.into_owned());
                            }
                        }
                    }
                }
                Ok((result.expect("validated selection index"), ()))
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

// Follow only the normalized control's result edges. Inner Objects, calls and
// guards belong to user code. A retained producer is checked after inference
// even if this invocation selected a different arm; it is never run again.
fn retain_producers(
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
                if let [tag, value] = fields.as_slice() {
                    let index = match &tag.kind {
                        K::Literal(n::Literal::Integer(index)) => index.parse::<usize>().ok(),
                        _ => None,
                    };
                    let Some(Alternative::Producer { site, checked }) =
                        index.and_then(|index| alternatives.get_mut(index))
                    else {
                        return Err(error(span, "invalid staged producer site"));
                    };
                    constants::charge_copy(value, &mut checker.required_steps, site.span)?;
                    *checked = Some(Box::new(value.clone()));
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
            } else {
                fields.push(hir::expr::ObjectEntry::Value(source));
                alternatives.push(Alternative::Producer {
                    site: source,
                    checked: None,
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
