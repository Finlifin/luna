//! Range iteration inside a closed comptime body stays in the invocation VM.
use super::*;

impl Checker<'_, '_> {
    pub(in crate::program) fn closed_comptime_control_body(&self) -> bool {
        !self.in_code_body
            && !self.checking_open_behavior
            && self.captures.last().is_none_or(|frame| frame.comptime)
            && self.signature_owner.is_some_and(|owner| {
                !self.open_functions.contains_key(&owner)
                    && self.package.item(owner).is_some_and(|item| {
                        matches!(&item.kind, ItemKind::Fn(sig, _)
                            if sig.modifiers.is_comptime && !sig.modifiers.is_normal_form_head)
                    })
            })
    }

    pub(super) fn comptime_range(
        &mut self,
        expr: &Expr<'_>,
        start: &Expr<'_>,
        end: &Expr<'_>,
        inclusive: bool,
        expected: Option<&Term>,
    ) -> Result<n::Expr, Error> {
        let H::For {
            pat, guard, body, ..
        } = expr.kind
        else {
            unreachable!()
        };
        // Boundaries precede the loop's lexical target and binders. They are
        // emitted once, in source order, even for an empty range.
        let start = self.expr(start, Some(&p(P::Integer)))?;
        let end = self.expr(end, Some(&p(P::Integer)))?;
        let exits = is_never(&start.ty) || is_never(&end.ty);
        let first = self.next_temporary_binding;
        self.next_temporary_binding = first
            .checked_add(3)
            .ok_or_else(|| error(expr.span, "comptime range identity space exhausted"))?;
        let cursor = n::LocalId {
            owner: u32::MAX,
            index: first,
        };
        let limit = n::LocalId {
            owner: u32::MAX,
            index: first + 1,
        };
        let current = n::LocalId {
            owner: u32::MAX,
            index: first + 2,
        };
        let at = source(expr.span);
        let node = |ty, kind| n::Expr {
            source: at,
            ty: p(ty),
            kind,
        };
        let read = |binding| node(P::Integer, K::Local(binding));
        let bind = |binding, value| {
            node(
                P::Void,
                K::Let {
                    binding,
                    value: Box::new(value),
                },
            )
        };
        let identity = local(expr.hir_id);
        let saved_locals = self.locals.clone();
        let saved_immutable = self.immutable.clone();
        let saved_context = self.context.clone();
        let saved_data = self.generator_data.clone();
        let saved_partial: BTreeSet<_> = self.partial_staged_bindings.keys().copied().collect();
        self.loops.push(identity);
        let result = (|| {
            self.immutable.extend(pattern_binding_ids(pat));
            let pattern = self.pattern(pat, &p(P::Integer))?;
            self.check_inline_pattern_availability(&pattern, pat.span)?;
            let guard = guard
                .map(|guard| self.expr(guard, Some(&p(P::Bool))))
                .transpose()?
                .map(Box::new);
            // Keep the body even for statically empty ranges: ordinary phase,
            // type and resource checks cannot be skipped by an empty input.
            let checked = discard_result(self.block(body, None)?);
            let binding = node(
                P::Void,
                K::LetPattern {
                    pattern,
                    value: Box::new(read(current)),
                    guard,
                    otherwise: Some(Box::new(node(P::Never, K::Continue(identity)))),
                },
            );
            let mut connections: BTreeMap<_, _> = saved_locals
                .keys()
                .chain(self.loops.iter())
                .map(|id| (*id, *id))
                .collect();
            connections.insert(current, current);
            let iteration = self.wrap_inline_iteration(
                checked,
                vec![binding],
                identity,
                expr.span,
                body.span,
                connections,
            )?;
            let compare = node(
                P::Bool,
                K::IntegerBinary {
                    op: if inclusive {
                        n::IntegerOp::LessEqual
                    } else {
                        n::IntegerOp::Less
                    },
                    left: Box::new(read(cursor)),
                    right: Box::new(read(limit)),
                },
            );
            let boundary = node(
                P::Void,
                K::If {
                    condition: Box::new(compare),
                    then_branch: Box::new(node(P::Void, K::Unit)),
                    else_branch: Some(Box::new(node(P::Never, K::Break(identity)))),
                },
            );
            // Advance the private cursor before entering the iteration scope.
            // A source continue then leaves that scope and reaches the next
            // cursor value; source code sees only the immutable index snapshot.
            let advance = node(
                P::Void,
                K::Assign {
                    place: n::Place {
                        root: cursor,
                        fields: vec![],
                    },
                    value: Box::new(node(
                        P::Integer,
                        K::IntegerBinary {
                            op: n::IntegerOp::Add,
                            left: Box::new(read(cursor)),
                            right: Box::new(node(
                                P::Integer,
                                K::ComptimeValue(Term::Integer(
                                    comptime_runtime::Integer::from_u128(1),
                                )),
                            )),
                        },
                    )),
                },
            );
            let looping = self.expanded_loop(
                node(
                    P::Void,
                    K::Block {
                        statements: vec![boundary, bind(current, read(cursor)), advance, iteration],
                        result: None,
                    },
                ),
                identity,
                expr.span,
            )?;
            let result = node(
                if exits { P::Never } else { P::Void },
                K::Block {
                    statements: vec![bind(cursor, start), bind(limit, end)],
                    result: Some(Box::new(looping)),
                },
            );
            if let Some(expected) = expected {
                self.require_expression(&result.ty, expected, expr.span)?;
            }
            Ok(result)
        })();
        self.loops.pop();
        self.locals = saved_locals;
        self.immutable = saved_immutable;
        self.context = saved_context;
        self.generator_data = saved_data;
        self.partial_staged_bindings
            .retain(|binding, _| saved_partial.contains(binding));
        result
    }
}
