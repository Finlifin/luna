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
        if rest.is_some_and(|rest| !matches!(rest.kind, hir::pattern::PatternKind::Wild)) {
            return Err(error(
                pattern.span,
                "list rest patterns require a slicing protocol",
            ));
        }
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
        let call = |this: &mut Self, name: &str, index: Option<usize>| {
            let location = index.map(|index| &elements[index]).unwrap_or(pattern);
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
                    pattern.span,
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
        let length = call(self, contract.length, None)?;
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
            let value = call(self, contract.element, Some(index))?;
            let next = self.pattern(element, &value.ty)?;
            steps.push(n::Pattern::Then {
                first: Box::new(n::Pattern::Wild),
                value: Box::new(value),
                next: Box::new(next),
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
