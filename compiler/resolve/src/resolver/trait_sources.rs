//! Scope-sensitive trait source plans. Discovery is not method applicability.
use super::*;
use crate::{
    binding::BindingKind, deferred::NameResolutionPlan, ids::AstNodeRef, scope::ScopeKind,
};
use rustc_span::Span;
use std::{collections::HashSet, fmt};

/// Potential trait spellings retain ordinary lexical resolution and its errors.
/// Pending generated exports can add new names as well as shadow known ones:
/// the caller must wait and recollect, not treat this snapshot as complete.
#[derive(Clone, Debug)]
pub struct TraitSourcePlan {
    pub names: Vec<NameResolutionPlan>,
    pub pending_exports: Vec<AstNodeRef>,
}
impl fmt::Display for TraitSourcePlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(trait-source-plan (names")?;
        for name in &self.names {
            write!(f, " {name}")?;
        }
        f.write_str(") (pending-exports")?;
        for site in &self.pending_exports {
            write!(f, " (site (file {}) (node {}))", site.file.raw(), site.node)?;
        }
        f.write_str("))")
    }
}

/// The module tree is immutable during query resolution. Index only its static
/// spelling universe; never cache decisions involving the mutable rib stack.
pub(super) struct TraitSourceIndex {
    declarations: HashSet<DefId>,
    names: HashMap<ScopeId, Vec<Symbol>>,
}
impl TraitSourceIndex {
    pub(super) fn new(tree: &ModuleTree) -> Self {
        let declarations: HashSet<_> = tree
            .scope_tree
            .iter()
            .filter(|scope| scope.kind == ScopeKind::TraitBody)
            .map(|scope| scope.owner_def)
            .collect();
        let is_trait = |binding: &Binding| {
            matches!(binding.kind,
            BindingKind::Definition(id, _) if declarations.contains(&id))
        };
        let names = tree
            .scope_tree
            .iter()
            .map(|scope| {
                let mut names: HashSet<_> = scope
                    .items
                    .declarations()
                    .iter()
                    .filter(|(_, binding)| is_trait(binding))
                    .map(|(name, _)| *name)
                    .collect();
                for (name, candidates) in &scope.items.glob_candidates {
                    if candidates.iter().any(is_trait) {
                        names.insert(*name);
                    }
                }
                (scope.id, names.into_iter().collect())
            })
            .collect();
        Self {
            declarations,
            names,
        }
    }
}

impl Resolver<'_> {
    /// Collect potential trait source names at this lexical position. Use normal
    /// name plans for shadowing, import provenance, ambiguity and privacy. A
    /// declaration in another module is not a source until an ordinary binding
    /// makes it visible here. Aliases retain one declaration identity, but their
    /// spelling plans stay separate because generated exports can shadow one.
    pub fn visible_trait_sources(&self, requester: ScopeId, span: Span) -> TraitSourcePlan {
        let mut names = HashSet::new();
        let mut scopes = HashSet::new();
        let mut pending_exports = Vec::new();
        for rib in self.ribs.inner_to_outer() {
            scopes.insert(rib.scope_id);
            for (name, binding) in &rib.bindings {
                if matches!(binding.kind, BindingKind::Definition(id, _)
                    if self.trait_sources.declarations.contains(&id))
                {
                    names.insert(*name);
                }
            }
            if let Some(site) = rib.pending_outputs {
                if !pending_exports.contains(&site) {
                    pending_exports.push(site);
                }
            }
        }
        scopes.extend(self.scope_tree().ancestors(requester).map(|scope| scope.id));
        for scope in scopes {
            if let Some(spellings) = self.trait_sources.names.get(&scope) {
                names.extend(spellings.iter().copied());
            }
        }
        let mut names: Vec<_> = names.into_iter().collect();
        names.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        let names = names
            .into_iter()
            .filter_map(|name| {
                let plan = self.resolve_name_deferred(name, requester, span);
                // A resolved ordinary local/type parameter shadows a trait name.
                // Keep pending fallbacks and errors rather than guessing their result.
                if plan.pending_outputs.is_empty() {
                    if let Ok(resolution) = &plan.fallback {
                        if !matches!(resolution.kind, BindingKind::Definition(id, _)
                        if self.trait_sources.declarations.contains(&id))
                        {
                            return None;
                        }
                    }
                }
                Some(plan)
            })
            .collect();
        TraitSourcePlan {
            names,
            pending_exports,
        }
    }
}
