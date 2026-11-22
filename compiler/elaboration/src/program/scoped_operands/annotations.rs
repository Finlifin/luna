//! Local type annotations demand a concrete identity when execution reaches
//! the declaration. The checked initializer still reads its original storage.
use super::*;
use crate::program::normal::observation::{self, Failure};

// Alias roots identify an immutable value, never a mutable slot. Keep these
// separate from completed Type evidence, specialization inputs and caches.
#[derive(Clone, Default)]
pub(in crate::program) struct NamedTypes {
    roots: LocalBindings<n::LocalId>,
    witnesses: LocalBindings<nir::DeclarationParameter>,
}

impl NamedTypes {
    pub(in crate::program) fn root(&self, local: n::LocalId) -> n::LocalId {
        self.roots.get(&local).copied().unwrap_or(local)
    }
    pub(in crate::program) fn alias(&mut self, binding: n::LocalId, source: n::LocalId) {
        self.roots.insert(binding, self.root(source));
    }
    pub(in crate::program) fn get(&self, local: n::LocalId) -> Option<nir::DeclarationParameter> {
        self.witnesses.get(&self.root(local)).copied()
    }
    pub(super) fn bind(&mut self, local: n::LocalId, binder: nir::DeclarationParameter) {
        self.witnesses.insert(self.root(local), binder);
    }
    pub(in crate::program) fn is_empty(&self) -> bool {
        self.roots.is_empty() && self.witnesses.is_empty()
    }
}

impl std::fmt::Display for NamedTypes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(named-types")?;
        for (binding, root) in &self.roots {
            write!(f, " (alias {binding} {root})")?;
        }
        for (binding, binder) in &self.witnesses {
            write!(f, " (local {binding} {binder})")?;
        }
        f.write_str(")")
    }
}

#[derive(Clone)]
pub(in crate::program) struct Annotation {
    site: hir::HirId,
    forming: bool,
    pub(super) inputs: Vec<(n::LocalId, Input)>,
}

impl Annotation {
    pub(in crate::program) fn new(site: hir::HirId) -> Self {
        Self {
            site,
            forming: false,
            inputs: Vec::new(),
        }
    }
}

impl Checker<'_, '_> {
    /// Only a local actually read by this structural observation can demand a
    /// witness. Unselected branches may carry gradual Type data.
    pub(in crate::program) fn observe_annotation_head(
        &mut self,
        checked: &n::Expr,
        span: Span,
    ) -> Result<Option<Term>, Error> {
        if !self.collecting_named_types() && self.input_facts.named_types.witnesses.is_empty() {
            return self.checked_normal_input_in(checked, &BTreeMap::new(), span);
        }
        let mut bindings = BTreeMap::new();
        loop {
            self.input_facts.observing_annotation_head = true;
            let result = self.with_observation_budget(span, |checker, budget| {
                match checker.observe_normal_input_in(checked, 0, budget, &bindings) {
                    Err(Failure::LocalType(local, source)) => Ok(Err((local, source))),
                    other => observation::optional(other).map(Ok),
                }
            });
            self.input_facts.observing_annotation_head = false;
            let (local, source) = match result? {
                Ok(identity) => return Ok(identity),
                Err(read) => read,
            };
            let Some(identity) = self.named_type_identity(local, source, span)? else {
                return Ok(None);
            };
            if bindings
                .insert(local, std::sync::Arc::new(identity))
                .is_some()
            {
                return Ok(None);
            }
        }
    }

    pub(in crate::program) fn local_binding_annotation(
        &mut self,
        site: hir::HirId,
        ty: &Expr<'_>,
    ) -> Result<Term, Error> {
        let collecting = self
            .input_facts
            .annotation
            .as_ref()
            .is_some_and(|a| a.site == site);
        if !collecting {
            return self.type_expr(ty);
        }
        self.input_facts.annotation.as_mut().unwrap().forming = true;
        let result = self.type_expr(ty);
        self.input_facts.annotation.as_mut().unwrap().forming = false;
        result
    }

    pub(in crate::program) fn collecting_named_types(&self) -> bool {
        self.input_facts
            .annotation
            .as_ref()
            .is_some_and(|a| a.forming)
    }

    pub(in crate::program) fn named_type_identity(
        &mut self,
        binding: n::LocalId,
        producer: n::SourceRange,
        span: Span,
    ) -> Result<Option<Term>, Error> {
        if !self.uses_lexical_operands()
            || !self.immutable.contains(&binding)
            || self.locals.get(&binding) != Some(&p(P::Type))
        {
            return Ok(None);
        }
        let binder = if let Some(binder) = self.input_facts.named_types.get(binding) {
            binder
        } else if self
            .input_facts
            .annotation
            .as_ref()
            .is_some_and(|a| a.forming && a.site.owner.def_id.raw() == binding.owner)
            && !self.input_facts.has_identity(&binding)
        {
            let owner = self.input_facts.annotation.as_ref().unwrap().site.owner;
            let binder = self.fresh_type_binder(owner, span)?;
            self.input_facts.named_types.bind(binding, binder);
            self.input_facts.annotation.as_mut().unwrap().inputs.push((
                binding,
                Input {
                    binder,
                    producer,
                    stable_local: None,
                },
            ));
            binder
        } else {
            return Ok(None);
        };
        // The lexical reference remains a capture even if an earlier statement
        // already established the same identity for its own annotation.
        if !self.captures.is_empty() {
            self.record_runtime_expr_captures(
                &n::Expr {
                    source: producer,
                    ty: p(P::Type),
                    kind: K::Local(binding),
                },
                span,
            )?;
        }
        Ok(Some(Term::Rigid {
            declaration: binder.declaration,
            sort: binder.sort,
        }))
    }
}
