//! Separate control evaluation from the contextual delivery of literal results.
//! The selector runs as one required expression, so block locals, guards, and
//! nested subjects all remain in the same source activation.
use super::*;

#[derive(Clone)]
pub(in crate::program) struct Selection<'hir> {
    source: &'hir Expr<'hir>,
    alternatives: std::sync::Arc<[&'hir Expr<'hir>]>,
    selected: usize,
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
        let comptime_interpretor::Value::Integer(index) =
            checker.required_value(selector, P::Integer)?
        else {
            return Err(error(
                source.span,
                "clause selection did not produce an index",
            ));
        };
        let selected = index
            .to_u128()
            .ok()
            .and_then(|index| usize::try_from(index).ok())
            .filter(|i| *i < alternatives.len())
            .ok_or_else(|| error(source.span, "invalid clause selection index"))?;
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
                for (index, expr) in self.alternatives.iter().enumerate() {
                    let checked = this.expr(expr, Some(domain))?;
                    if index == self.selected {
                        result = Some(checked);
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

// Only replace whole literal results. In particular, an application at a
// result site must keep its own source-order evaluation and lexical captures.
// Projecting is read-only until every arm has a representation.
fn project<'tree, 'src: 'tree>(
    checker: &mut Checker<'_, '_>,
    arena: &'tree hir::HirArena,
    source: &'src Expr<'src>,
    alternatives: &mut Vec<&'src Expr<'src>>,
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
        _ if checker.is_closed_clause_literal(source)? => {
            let spelling = alternatives.len().to_string();
            alternatives.push(source);
            H::Lit(hir::Lit {
                kind: hir::LitKind::Integer(spelling),
                span: source.span,
            })
        }
        _ => return Ok(None),
    };
    Ok(Some(arena.alloc_expr(Expr {
        hir_id: source.hir_id,
        span: source.span,
        kind,
    })))
}
