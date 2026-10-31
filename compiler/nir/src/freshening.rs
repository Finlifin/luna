//! Hygienic local identities and expansion provenance for checked NIR.
//! Callers preflight cloning/rewrite budgets before applying this transformation.
use crate::{Sort, Term, program as n, program::ExprKind as K};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// Mutate a private expansion candidate. On error, the caller discards or
/// rolls back its candidate and identity/source cursors.
pub fn expression(
    body: &mut n::Expr,
    connections: BTreeMap<n::LocalId, n::LocalId>,
    next: &mut u32,
    sources: &mut Vec<n::ExpansionSource>,
    span: n::SourceRange,
) -> Result<(), String> {
    body.rewrite_with(&mut Renamer {
        bindings: connections,
        next,
        sources,
        locations: BTreeMap::new(),
        span,
    })
}

struct Renamer<'a> {
    bindings: BTreeMap<n::LocalId, n::LocalId>,
    next: &'a mut u32,
    sources: &'a mut Vec<n::ExpansionSource>,
    locations: BTreeMap<(u32, u32), n::SourceRange>,
    span: n::SourceRange,
}
impl Renamer<'_> {
    fn fresh(&mut self) -> Result<n::LocalId, String> {
        let fresh = n::LocalId {
            owner: u32::MAX,
            index: *self.next,
        };
        *self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| "code expansion identity space exhausted".to_string())?;
        Ok(fresh)
    }
    fn rename(&mut self, id: &mut n::LocalId) -> Result<(), String> {
        let mapped = if let Some(mapped) = self.bindings.get(id) {
            *mapped
        } else {
            let fresh = self.fresh()?;
            self.bindings.insert(*id, fresh);
            fresh
        };
        *id = mapped;
        Ok(())
    }
}
impl n::Rewriter for Renamer<'_> {
    type Error = String;
    fn ty(&mut self, ty: &Term) -> Result<Term, String> {
        let runtime_slots = ty
            .substitution_parameters()
            .map_err(|failure| failure.to_string())?
            .values()
            .any(|sort| *sort == Sort::Runtime);
        if !ty.runtime_dependencies().is_empty() || runtime_slots {
            return Err("runtime-dependent quoted type substitution is not yet implemented".into());
        }
        Ok(ty.clone())
    }
    fn expression(&mut self, expression: &mut n::Expr) -> Result<bool, String> {
        let original = expression.source;
        // Staged binding can wrap a body in another block with the same
        // diagnostic range. Each lexical scope still needs a distinct runtime
        // cleanup boundary, even when both trace back to the same source.
        let independent_scope =
            matches!(expression.kind, K::Block { .. } | K::CallPreparation { .. });
        let generated = if let Some(generated) = self
            .locations
            .get(&(original.start, original.end))
            .filter(|_| !independent_scope)
        {
            *generated
        } else {
            let index = u32::try_from(self.sources.len())
                .map_err(|_| "code expansion source space exhausted".to_string())?;
            self.sources.push(n::ExpansionSource {
                original,
                site: self.span,
            });
            let generated = n::SourceRange::generated(index);
            self.locations
                .entry((original.start, original.end))
                .or_insert(generated);
            generated
        };
        expression.source = generated;
        match &mut expression.kind {
            K::Local(id)
            | K::Let { binding: id, .. }
            | K::Loop { identity: id, .. }
            | K::Break(id)
            | K::Continue(id) => self.rename(id)?,
            K::Assign { place, .. } | K::BitAssign { place, .. } | K::BitUpdate { place, .. } => {
                self.rename(&mut place.root)?
            }
            K::Closure {
                parameters,
                captures,
                body,
            } => {
                expression.ty = self.ty(&expression.ty)?;
                let mut inner = BTreeMap::new();
                for capture in captures {
                    capture.ty = self.ty(&capture.ty)?;
                    let original = capture.binding;
                    if let Some(value) = &mut capture.value {
                        value.rewrite_with(self)?;
                    } else {
                        let mut actual = original;
                        self.rename(&mut actual)?;
                        capture.value = Some(Box::new(n::Expr {
                            ty: capture.ty.clone(),
                            source: generated,
                            kind: K::Local(actual),
                        }));
                    }
                    capture.binding = self.fresh()?;
                    inner.insert(original, capture.binding);
                }
                for parameter in parameters {
                    let fresh = self.fresh()?;
                    inner.insert(*parameter, fresh);
                    *parameter = fresh;
                }
                let saved = std::mem::replace(&mut self.bindings, inner);
                let result = Arc::make_mut(body).rewrite_with(self);
                self.bindings = saved;
                result?;
                return Ok(false);
            }
            K::Call {
                callee,
                arguments,
                parameter_bindings,
            } => {
                expression.ty = self.ty(&expression.ty)?;
                callee.rewrite_with(self)?;
                for argument in arguments {
                    if argument.is_default {
                        // A selected callable owns this frame and its ABI. The
                        // caller's context substitution must never rename it.
                        let defaults = parameter_bindings.iter().map(|&id| (id, id)).collect();
                        let saved = std::mem::replace(&mut self.bindings, defaults);
                        let result = argument.value.rewrite_with(self);
                        self.bindings = saved;
                        result?;
                    } else {
                        argument.value.rewrite_with(self)?;
                    }
                }
                return Ok(false);
            }
            K::NominalType {
                bindings: Some(bindings),
                ..
            }
            | K::CheckTypeAlias {
                bindings: Some(bindings),
                ..
            } => {
                for id in bindings {
                    self.rename(id)?;
                }
            }
            K::NormalFormType {
                plan: Some(plan), ..
            } => {
                for id in &mut plan.bindings {
                    self.rename(id)?;
                }
            }
            _ => {}
        }
        Ok(true)
    }
    fn pattern(&mut self, pattern: &mut n::Pattern) -> Result<(), String> {
        match pattern {
            n::Pattern::CodeEvidence { outputs } => {
                for (id, _) in outputs {
                    self.rename(id)?;
                }
            }

            n::Pattern::Observe { binding, .. } | n::Pattern::Binding { binding, .. } => {
                self.rename(binding)?
            }
            n::Pattern::Remap { bindings, .. } => {
                for (from, to) in bindings {
                    self.rename(from)?;
                    self.rename(to)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

/// Rewrite pattern-owned bindings with the same lexical connection policy.
pub fn pattern(
    body: &mut n::Pattern,
    connections: BTreeMap<n::LocalId, n::LocalId>,
    next: &mut u32,
    sources: &mut Vec<n::ExpansionSource>,
    site: n::SourceRange,
) -> Result<(), String> {
    body.rewrite_with(&mut Renamer {
        bindings: connections,
        next,
        sources,
        locations: BTreeMap::new(),
        span: site,
    })
}
/// Copy-local identities change; lexical inputs and transfers to an enclosing
/// loop keep their identities. Used when materialization duplicates a branch.
pub fn copied_expression(
    expression: &mut n::Expr,
    next: &mut u32,
    sources: &mut Vec<n::ExpansionSource>,
    fuel: &mut usize,
) -> Result<(), String> {
    let mut connections: BTreeMap<_, _> = crate::free_bindings::collect(expression, fuel)?
        .into_iter()
        .map(|id| (id, id))
        .collect();
    let mut loops = BTreeSet::new();
    crate::visit::expression(expression, fuel, |expr, fuel| {
        crate::type_computations::charge_term(&expr.ty, fuel).map_err(|e| e.to_string())?;
        if let K::Loop { identity, .. } = expr.kind {
            loops.insert(identity);
        }
        Ok(())
    })?;
    crate::visit::expression(expression, fuel, |expr, _| {
        if let K::Break(id) | K::Continue(id) = expr.kind {
            if !loops.contains(&id) {
                connections.insert(id, id);
            }
        }
        Ok(())
    })?;
    let site = expression.source;
    self::expression(expression, connections, next, sources, site)
        .map_err(|message| format!("{}..{}: {message}", site.start, site.end))
}
