//! Clone generic method binders without sharing their resolver identities.
use super::*;
use hir::clause::{ClauseParam, ClauseParamKind as K};

impl<'h> Rebind<'_, 'h> {
    pub(in crate::program::function_templates) fn clauses(
        &mut self,
        parameters: &[ClauseParam<'h>],
    ) -> Result<&'h [ClauseParam<'h>], Error> {
        let mut result = Vec::new();
        let mut identities = std::collections::BTreeSet::new();
        for parameter in parameters {
            self.charge(parameter.span, 0)?;
            let old = parameter.definition.ok_or_else(|| {
                error(
                    parameter.span,
                    "generated clause parameter has no source identity",
                )
            })?;
            let new = self
                .definitions
                .get(&old)
                .copied()
                .filter(|new| *new != old && new.is_valid())
                .ok_or_else(|| {
                    error(
                        parameter.span,
                        "generated clause parameter requires a fresh definition identity",
                    )
                })?;
            if !identities.insert((new.pkg, new.index)) {
                return Err(error(
                    parameter.span,
                    "generated clause parameters share a definition identity",
                ));
            }
            let mut parameter = parameter.clone();
            parameter.hir_id = self.id(parameter.hir_id);
            parameter.definition = Some(new);
            parameter.kind = self.clause_kind(&parameter.kind, parameter.span, 0)?;
            result.push(parameter);
        }
        Ok(self.arena.alloc_clause_param_slice(result))
    }

    pub(in crate::program::function_templates) fn constraints(
        &mut self,
        constraints: &[hir::clause::ClauseConstraint<'h>],
    ) -> Result<&'h [hir::clause::ClauseConstraint<'h>], Error> {
        let mut result = Vec::new();
        for constraint in constraints {
            self.charge(constraint.span, 0)?;
            let mut constraint = constraint.clone();
            constraint.hir_id = self.id(constraint.hir_id);
            constraint.kind = match constraint.kind {
                C::Requires(value) => C::Requires(self.expr(value, 0)?),
                C::Ensures(value) => C::Ensures(self.expr(value, 0)?),
                C::Decreases(value) => C::Decreases(self.expr(value, 0)?),
                C::Outcome => C::Outcome,
            };
            result.push(constraint);
        }
        Ok(self.arena.alloc_clause_slice(result))
    }

    fn clause_kind(
        &mut self,
        kind: &K<'h>,
        span: rustc_span::Span,
        depth: usize,
    ) -> Result<K<'h>, Error> {
        self.charge(span, depth)?;
        let depth = depth + 1;
        Ok(match kind {
            K::Type(name) => K::Type(*name),
            K::Positional(name, ty) => K::Positional(*name, self.expr(ty, depth)?),
            K::Optional(name, ty, default) => {
                K::Optional(*name, self.expr(ty, depth)?, self.expr(default, depth)?)
            }
            K::Varadic(name, ty) => K::Varadic(*name, self.expr(ty, depth)?),
            K::TraitBound(name, bound) => K::TraitBound(*name, self.expr(bound, depth)?),
            K::Quote(inner) => K::Quote(Box::new(self.clause_kind(inner, span, depth)?)),
        })
    }
}
