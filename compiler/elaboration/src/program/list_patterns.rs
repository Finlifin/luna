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
        if rest.is_some() {
            return Err(error(
                pattern.span,
                "list rest patterns require a slicing protocol",
            ));
        }
        if !self.runtime_value_context() && !self.in_code_body {
            return Err(error(
                pattern.span,
                "comptime list patterns require the comptime ListPatternView phase",
            ));
        }
        let protocol = self.protocols.list_pattern.ok_or_else(|| {
            error(
                pattern.span,
                "list patterns require a bound ListPatternView protocol",
            )
        })?;
        let size = crate::list_patterns::protocol(&self.checked_traits, protocol)
            .map_err(|message| error(pattern.span, message))?;
        let binding = n::LocalId {
            owner: u32::MAX,
            index: self.next_temporary_binding,
        };
        self.next_temporary_binding = self
            .next_temporary_binding
            .checked_add(1)
            .ok_or_else(|| error(pattern.span, "pattern temporary binding space exhausted"))?;
        self.locals.insert(binding, ty.clone());
        let receiver = n::Expr {
            source: source(pattern.span),
            ty: Term::Pointer(Box::new(ty.clone())),
            kind: K::AddressOf(Box::new(n::Expr {
                source: source(pattern.span),
                ty: ty.clone(),
                kind: K::Local(binding),
            })),
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
        let K::Literal(arity) = arity.kind else {
            unreachable!("validated unsigned size literal")
        };
        let length = call(self, "list_pattern_view_get_len", None)?;
        let mut result = n::Pattern::Then {
            first: Box::new(n::Pattern::Wild),
            value: Box::new(length),
            next: Box::new(n::Pattern::Literal(arity)),
        };
        for (index, element) in elements.iter().enumerate() {
            let value = call(self, "list_pattern_view_get_element", Some(index))?;
            let next = self.pattern(element, &value.ty)?;
            result = n::Pattern::Then {
                first: Box::new(result),
                value: Box::new(value),
                next: Box::new(next),
            };
        }
        Ok(n::Pattern::Observe {
            binding,
            inner: Box::new(result),
        })
    }
}
