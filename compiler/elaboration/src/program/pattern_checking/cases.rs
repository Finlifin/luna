//! A user case computes an optional payload once for the current candidate.
use super::*;

impl Checker<'_, '_> {
    pub(super) fn case_pattern(
        &mut self,
        pattern: &hir::pattern::Pattern<'_>,
        head: &Expr<'_>,
        fields: Option<&[hir::pattern::Pattern<'_>]>,
        ty: &Term,
    ) -> Result<Option<n::Pattern>, Error> {
        let H::Lit(hir::Lit {
            kind: hir::LitKind::Symbol(name),
            ..
        }) = &head.kind
        else {
            return Ok(None);
        };
        if !self.source_declarations.has_case_name(*name) {
            return Ok(None);
        }
        let Some(method) = self.selected_method(ty.clone(), None, name.as_str(), head.span)? else {
            return Ok(None);
        };
        let id = match &method.kind {
            K::Function(id)
            | K::FunctionInstance {
                declaration: id, ..
            } => *id,
            _ => return Ok(None),
        };
        if !self
            .source_declarations
            .item(id)
            .is_some_and(|item| matches!(item.kind, ItemKind::Case(..)))
        {
            return Ok(None);
        }
        let Term::Function { parameters, .. } = method.ty.callable_signature() else {
            return Err(error(head.span, "case has no callable signature"));
        };
        let Some(first) = parameters.first() else {
            return Err(error(head.span, "case pattern needs a subject parameter"));
        };
        let by_address = matches!(&first.ty, Term::Pointer(_)) && &first.ty != ty;
        let binding = n::LocalId {
            owner: u32::MAX,
            index: self.next_temporary_binding,
        };
        self.next_temporary_binding = self
            .next_temporary_binding
            .checked_add(1)
            .ok_or_else(|| error(pattern.span, "pattern temporary binding space exhausted"))?;
        let subject = n::Expr {
            ty: ty.clone(),
            source: source(pattern.span),
            kind: K::Local(binding),
        };
        let argument = if by_address {
            n::Expr {
                ty: Term::Pointer(Box::new(ty.clone())),
                source: subject.source,
                kind: K::AddressOf(Box::new(subject)),
            }
        } else {
            subject
        };
        self.locals.insert(binding, ty.clone());
        let delivery = self.finish_call(head, method, &[], vec![argument], None, None);
        self.locals.remove(&binding);
        let value = delivery?;
        let payload = self.optional_payload(&value.ty, pattern.span)?;
        let inner = if let Some(fields) = fields {
            let element_types = self.structural_type(&payload, pattern.span)?;
            let Term::Tuple(elements) = element_types else {
                return Err(error(
                    pattern.span,
                    "destructuring case must return an optional tuple",
                ));
            };
            if elements.len() != fields.len() {
                return Err(error(pattern.span, "case pattern payload arity mismatch"));
            }
            let children = fields
                .iter()
                .zip(&elements)
                .map(|(pat, ty)| self.pattern(pat, ty))
                .collect::<Result<Vec<_>, _>>()?;
            n::Pattern::Tuple(children)
        } else {
            n::Pattern::Wild
        };
        Ok(Some(n::Pattern::Observe {
            binding,
            inner: Box::new(n::Pattern::Then {
                first: Box::new(n::Pattern::Wild),
                value: Box::new(value),
                next: Box::new(n::Pattern::OptionalPresent(Box::new(inner))),
            }),
        }))
    }
}
