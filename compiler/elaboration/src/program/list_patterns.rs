//! List views elaborate to ordinary selected calls and transactional patterns.
//! No extractor is invoked to prove coverage or inspect a runtime subject.
use super::*;

impl Checker<'_, '_> {
    pub(super) fn list_pattern(
        &mut self,
        pattern: &hir::pattern::Pattern<'_>,
        elements: &[hir::pattern::Pattern<'_>],
        rest: Option<&hir::pattern::Pattern<'_>>,
        ty: &Term,
    ) -> Result<n::Pattern, Error> {
        let comptime = !self.runtime_value_context() && !self.in_code_body;
        let selected = if comptime {
            self.protocols
                .comptime_list_pattern
                .or(self.protocols.list_pattern)
        } else {
            self.protocols.list_pattern
        };
        let protocol = selected.ok_or_else(|| {
            error(
                pattern.span,
                "list patterns require a bound ListPatternView protocol",
            )
        })?;
        let contract = crate::list_patterns::for_phase(&self.checked_traits, protocol, comptime)
            .map_err(|message| error(pattern.span, message))?;
        if contract.comptime != comptime {
            return Err(error(
                pattern.span,
                if comptime {
                    "list patterns require the comptime ListPatternView phase"
                } else {
                    "list patterns require the runtime ListPatternView phase"
                },
            ));
        }
        let size = contract.size;
        let binding = n::LocalId {
            owner: u32::MAX,
            index: self.next_temporary_binding,
        };
        self.next_temporary_binding = self
            .next_temporary_binding
            .checked_add(1)
            .ok_or_else(|| error(pattern.span, "pattern temporary binding space exhausted"))?;
        self.locals.insert(binding, ty.clone());
        let subject = n::Expr {
            source: source(pattern.span),
            ty: ty.clone(),
            kind: K::Local(binding),
        };
        let receiver = if contract.by_value {
            subject
        } else {
            n::Expr {
                source: source(pattern.span),
                ty: Term::Pointer(Box::new(ty.clone())),
                kind: K::AddressOf(Box::new(subject)),
            }
        };
        let call = |this: &mut Self,
                    location: &hir::pattern::Pattern<'_>,
                    name: &str,
                    index: Option<usize>| {
            let site = Expr {
                hir_id: location.hir_id,
                span: location.span,
                kind: H::Unit,
            };
            let method = this
                .selected_method(
                    ty.clone(),
                    Some(n::InterfaceInstance {
                        declaration: protocol,
                        arguments: vec![],
                    }),
                    name,
                    pattern.span,
                )?
                .ok_or_else(|| error(pattern.span, "ListPatternView has no selected method"))?;
            let mut inputs = vec![receiver.clone()];
            if let Some(index) = index {
                inputs.push(this.integer_proxy(
                    &index.to_string(),
                    false,
                    Some(&size),
                    location.span,
                )?);
            }
            this.finish_call(&site, method, &[], inputs, None, None)
        };
        let arity = self.integer_proxy(
            &elements.len().to_string(),
            false,
            Some(&size),
            pattern.span,
        )?;
        let length = call(self, pattern, contract.length, None)?;
        let (value, next) = if rest.is_some() {
            (
                n::Expr {
                    source: source(pattern.span),
                    ty: p(P::Bool),
                    kind: if size == p(P::Integer) {
                        K::IntegerBinary {
                            op: n::IntegerOp::LessEqual,
                            left: Box::new(arity),
                            right: Box::new(length),
                        }
                    } else {
                        K::ScalarLessEqual(Box::new(arity), Box::new(length))
                    },
                },
                n::Pattern::Literal(n::Literal::Bool(true)),
            )
        } else {
            let K::Literal(arity) = arity.kind else {
                unreachable!("validated list size literal")
            };
            (length, n::Pattern::Literal(arity))
        };
        let length_check = n::Pattern::Then {
            first: Box::new(n::Pattern::Wild),
            value: Box::new(value),
            next: Box::new(next),
        };
        let mut steps = vec![length_check];
        for (index, element) in elements.iter().enumerate() {
            let value = call(self, element, contract.element, Some(index))?;
            let next = self.pattern(element, &value.ty)?;
            steps.push(n::Pattern::Then {
                first: Box::new(n::Pattern::Wild),
                value: Box::new(value),
                next: Box::new(next),
            });
        }
        if let Some(rest_pat) = rest
            && !matches!(rest_pat.kind, hir::pattern::PatternKind::Wild)
        {
            let rest_name = contract.rest.ok_or_else(|| {
                error(
                    rest_pat.span,
                    "list rest patterns require a slicing protocol",
                )
            })?;
            if let Some(default_body) = contract.rest_default {
                let resolution = self
                    .resolve_trait_capability(
                        &n::InterfaceInstance {
                            declaration: protocol,
                            arguments: vec![],
                        },
                        ty,
                        crate::ptc::Budget::default(),
                    )
                    .map_err(|e| error(rest_pat.span, format!("trait resolution failed: {e:?}")))?;
                let overridden = matches!(
                    resolution.selection,
                    crate::implementation::Selection::Selected(selected_id)
                        if self
                            .implementation_positions
                            .get(&selected_id)
                            .and_then(|pos| self.checked_implementations.get(*pos))
                            .is_some_and(|implementation| {
                                implementation
                                    .methods
                                    .iter()
                                    .any(|(method, body)| method == rest_name && *body != default_body)
                            })
                );
                if !overridden {
                    return Err(error(
                        rest_pat.span,
                        "list rest patterns require a slicing protocol",
                    ));
                }
            }
            let rest_value = call(self, rest_pat, rest_name, Some(elements.len()))?;
            let rest_next = self.pattern(rest_pat, &rest_value.ty)?;
            steps.push(n::Pattern::Then {
                first: Box::new(n::Pattern::Wild),
                value: Box::new(rest_value),
                next: Box::new(rest_next),
            });
        }
        // Each generated step observes the alias, not its incoming subject.
        // A unit bridge can therefore balance the sequence without copying the
        // subject or changing the left-to-right short-circuiting evaluation.
        // User element patterns remain intact and retain their own input.
        while steps.len() > 1 {
            let mut balanced = Vec::with_capacity(steps.len().div_ceil(2));
            let mut pending = steps.into_iter();
            while let Some(first) = pending.next() {
                balanced.push(match pending.next() {
                    Some(next) => n::Pattern::Then {
                        first: Box::new(first),
                        value: Box::new(n::Expr {
                            source: source(pattern.span),
                            ty: p(P::Void),
                            kind: K::Unit,
                        }),
                        next: Box::new(next),
                    },
                    None => first,
                });
            }
            steps = balanced;
        }
        let result = steps.pop().expect("length check is always present");
        Ok(n::Pattern::Observe {
            binding,
            inner: Box::new(result),
        })
    }
}
