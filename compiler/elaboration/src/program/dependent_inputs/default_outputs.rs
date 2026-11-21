//! Result and post-execution local values of one selected default.
use super::*;

pub(in crate::program) struct DefaultOutputs {
    result: Term,
    slots: Vec<(n::LocalId, Term)>,
}

pub(in crate::program) struct CompletedDefault {
    pub result: (comptime_interpretor::Value, Term),
    pub writes: Vec<(n::LocalId, Term, comptime_interpretor::Value)>,
}

impl DefaultOutputs {
    pub(in crate::program) fn prepare<'a>(
        checker: &mut Checker<'_, '_>,
        value: n::Expr,
        domain: impl Fn(n::LocalId) -> Option<&'a Term>,
        span: Span,
    ) -> Result<(n::Expr, Option<Self>), Error> {
        let writes = checker.with_observation_budget(span, |_, budget| {
            Ok(
                nir::free_bindings::access(&value, |id| domain(id).is_some(), &mut budget.steps)?
                    .writable,
            )
        })?;
        if writes.is_empty() {
            return Ok((value, None));
        }
        let (packet, outputs) = Self::pack(checker, value, writes, domain, span)?;
        Ok((packet, Some(outputs)))
    }

    pub(in crate::program) fn pack<'a>(
        checker: &mut Checker<'_, '_>,
        value: n::Expr,
        writes: BTreeSet<n::LocalId>,
        domain: impl Fn(n::LocalId) -> Option<&'a Term>,
        span: Span,
    ) -> Result<(n::Expr, Self), Error> {
        let mut slots = Vec::new();
        for id in writes {
            let ty = domain(id).ok_or_else(|| error(span, "unavailable staged default slot"))?;
            slots.push((
                id,
                tasks::copy_evidence(ty, &mut checker.required_steps, span)?,
            ));
        }
        let result = value.ty.clone();
        let mut fields = vec![value];
        fields.extend(slots.iter().map(|(id, ty)| n::Expr {
            source: source(span),
            ty: ty.clone(),
            kind: K::Local(*id),
        }));
        let ty = checker.tuple_value_type(fields.iter().map(|v| v.ty.clone()).collect(), span)?;
        Ok((
            n::Expr {
                source: source(span),
                ty,
                kind: K::Tuple(fields),
            },
            Self { result, slots },
        ))
    }

    pub(in crate::program) fn replay(
        result: n::Expr,
        writes: Vec<(n::LocalId, n::Expr)>,
    ) -> n::Expr {
        let statements = writes
            .into_iter()
            .map(|(root, value)| n::Expr {
                source: value.source,
                ty: p(P::Void),
                kind: K::Assign {
                    place: n::Place {
                        root,
                        fields: Vec::new(),
                    },
                    value: Box::new(value),
                },
            })
            .collect();
        n::Expr {
            source: result.source,
            ty: result.ty.clone(),
            kind: K::Block {
                statements,
                result: Some(Box::new(result)),
            },
        }
    }

    pub(in crate::program) fn finish(
        self,
        checker: &mut Checker<'_, '_>,
        value: comptime_interpretor::Value,
        span: Span,
        mut write: impl FnMut(n::LocalId, n::Expr, Term),
    ) -> Result<(n::Expr, Term), Error> {
        let completed = self.unpack(value, span)?;
        let (value, domain) = completed.result;
        let result = checker.freeze_completed_dependency(value, domain, span)?;
        for (id, domain, field) in completed.writes {
            let (value, symbol) = checker.freeze_completed_dependency(field, domain, span)?;
            write(id, value, symbol);
        }
        Ok(result)
    }

    pub(in crate::program) fn unpack(
        self,
        value: comptime_interpretor::Value,
        span: Span,
    ) -> Result<CompletedDefault, Error> {
        let comptime_interpretor::Value::Tuple(fields) = value else {
            return Err(error(span, "default frame did not return its slot values"));
        };
        if fields.len() != self.slots.len() + 1 {
            return Err(error(span, "invalid default frame output count"));
        }
        let mut fields = fields.into_iter();
        let result = (fields.next().unwrap(), self.result);
        let writes = self
            .slots
            .into_iter()
            .zip(fields)
            .map(|((id, domain), field)| (id, domain, field))
            .collect();
        Ok(CompletedDefault { result, writes })
    }
}

impl std::fmt::Display for DefaultOutputs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(default-outputs (result {}) (slots", self.result)?;
        for (id, ty) in &self.slots {
            write!(f, " ({id} {ty})")?;
        }
        f.write_str("))")
    }
}
