use std::{collections::HashMap, sync::Arc};
use symbol::{DefId, Symbol};

use crate::binding::{Binding, Resolution};
use crate::error::{ResolveError, ResolveResult};
use crate::ids::ScopeId;
use crate::module_builder::ModuleTree;
use crate::rib::RibStack;
use crate::scope::ScopeTree;

/// Early name resolver for AST lowering.
///
/// Constructed from a [`ModuleTree`] (the product of the build phase) and
/// provides lookup / resolution methods that AST lowering calls as it walks
/// the AST.
///
/// ## Error policy
///
/// Every lookup method that is expected to find a result returns
/// `ResolveResult<…>`.  Callers (AST lowering) should collect or emit
/// these errors through the diagnostic infrastructure.
pub struct Resolver<'a> {
    /// The pre-built module tree (scope tree + def names).
    tree: &'a ModuleTree,
    /// Rib stack for ordered (lexical) scopes.
    ribs: RibStack,
    /// Scope-sensitive results over the immutable module tree. Shared with HIR
    /// so repeated projections do not duplicate complete candidate lists.
    type_members: HashMap<(ScopeId, Symbol), Arc<[(DefId, DefId)]>>,
}

impl<'a> Resolver<'a> {
    /// Create a resolver that queries against `tree`.
    pub fn new(tree: &'a ModuleTree) -> Self {
        Self {
            tree,
            ribs: RibStack::new(),
            type_members: HashMap::new(),
        }
    }

    /// Returns `Err(ResolveError::UnresolvedIdentifier)` if the name cannot be
    /// found in any enclosing scope.
    pub fn resolve_name(
        &self,
        name: Symbol,
        scope_id: ScopeId,
        span: rustc_span::Span,
    ) -> ResolveResult<Resolution> {
        // 1. Check the rib stack first (lexical / ordered bindings).
        if let Some(binding) = self.ribs.lookup(name) {
            return Ok(Resolution::from_binding(binding));
        }

        // 2. Walk up the scope tree.
        for scope in self.scope_tree().ancestors(scope_id) {
            if scope.items.ambiguous_glob(name) {
                return Err(ResolveError::InvalidNodeType {
                    message: format!("ambiguous glob name `{name}`"),
                    span,
                });
            }
            if let Some(binding) = self.lookup_in_scope(name, scope.id) {
                self.check_access(&binding, scope_id, span)?;
                return Ok(Resolution::from_binding(&binding));
            }
        }

        Err(ResolveError::UnresolvedIdentifier {
            name: name.to_string(),
            span,
        })
    }

    /// Access the rib stack (e.g. to push / pop ribs during lowering).
    pub fn ribs(&self) -> &RibStack {
        &self.ribs
    }

    /// Mutable access to the rib stack.
    pub fn ribs_mut(&mut self) -> &mut RibStack {
        &mut self.ribs
    }

    /// Look up a name by walking up the scope tree (no rib stack).
    /// Returns `None` if not found.
    fn resolve_name_to_binding(&self, name: Symbol, scope_id: ScopeId) -> Option<Binding> {
        for scope in self.scope_tree().ancestors(scope_id) {
            if let Some(b) = self.lookup_in_scope(name, scope.id) {
                return Some(b);
            }
        }
        None
    }

    /// Look up a name in a single scope (direct declarations + resolved
    /// imports).
    fn lookup_in_scope(&self, name: Symbol, scope_id: ScopeId) -> Option<Binding> {
        let scope = self.scope_tree().get(scope_id)?;

        // Direct declarations (including clauses).
        if let Some(b) = scope.items.get_local(name) {
            return Some(b.clone());
        }

        scope.items.unique_glob(name).cloned()
    }

    /// Look up a name in direct declarations only (no imports, no ancestor walk).
    fn lookup_direct(&self, name: Symbol, scope_id: ScopeId) -> Option<Binding> {
        let scope = self.scope_tree().get(scope_id)?;
        scope.items.get_direct(name).cloned()
    }

    /// Find the scope that a DefId owns (module, ADT, etc.).
    pub fn find_scope_for_def(&self, def_id: DefId) -> Option<ScopeId> {
        // Linear scan – fine for now; could be indexed on ModuleTree later.
        for scope in self.scope_tree().iter() {
            if scope.owner_def == def_id {
                return Some(scope.id);
            }
        }
        None
    }

    /// Anonymous implementations cannot be looked up by a spelling. Their
    /// identity is the scanner's file-qualified AST location, including when
    /// several source files happen to reuse the same node index.
    pub fn implementation_scope(
        &self,
        file: vfs::FileId,
        node: ast::NodeIndex,
    ) -> Option<(DefId, ScopeId)> {
        self.tree.impls.iter().find_map(|implementation| {
            (implementation.file_id == file && implementation.ast_node == node)
                .then_some((implementation.def_id, implementation.impl_scope))
        })
    }

    /// Resolve one static member without searching the container's ancestors.
    pub fn resolve_member(
        &self,
        container: DefId,
        name: Symbol,
        requester: ScopeId,
        span: rustc_span::Span,
    ) -> ResolveResult<Resolution> {
        let scope =
            self.find_scope_for_def(container)
                .ok_or_else(|| ResolveError::ModuleNotFound {
                    message: "definition is not a static container".into(),
                    span,
                })?;
        if self
            .scope_tree()
            .get(scope)
            .unwrap()
            .items
            .ambiguous_glob(name)
        {
            return Err(ResolveError::InvalidNodeType {
                message: format!("ambiguous glob name `{name}`"),
                span,
            });
        }
        let binding = self.lookup_in_scope(name, scope).ok_or_else(|| {
            ResolveError::UnresolvedIdentifier {
                name: name.to_string(),
                span,
            }
        })?;
        self.check_access(&binding, requester, span)?;
        Ok(Resolution::from_binding(&binding))
    }

    /// Locate a unique trait member for early pattern-output names. This does
    /// not establish the receiver's trait capability or substitute any types.
    pub fn pattern_trait_member(
        &self,
        container: DefId,
        name: Symbol,
        requester: ScopeId,
        span: rustc_span::Span,
    ) -> Option<DefId> {
        use crate::binding::BindingKind;
        use std::collections::BTreeSet;
        let mut pending = vec![container];
        let mut seen = BTreeSet::new();
        let mut members = BTreeSet::new();
        while let Some(container) = pending.pop() {
            if !seen.insert(container) {
                continue;
            }
            if seen.len() > 4096 {
                return None;
            }
            let scope_id = self.find_scope_for_def(container)?;
            let scope = self.scope_tree().get(scope_id)?;
            if scope.kind != crate::scope::ScopeKind::TraitBody {
                return None;
            }
            if scope.items.get_direct(name).is_some() {
                let BindingKind::Definition(member, _) = self
                    .resolve_member(container, name, requester, span)
                    .ok()?
                    .kind
                else {
                    return None;
                };
                members.insert(member);
                continue;
            }
            for path in scope.trait_parent_paths.as_ref()? {
                let (head, _) = path.first()?;
                // Use the declaration telescope, never the caller's ribs.
                let mut binding = None;
                for parent in self.scope_tree().ancestors(scope_id) {
                    if parent.items.ambiguous_glob(*head) {
                        return None;
                    }
                    if let Some(found) = self.lookup_in_scope(*head, parent.id) {
                        binding = Some(found);
                        break;
                    }
                }
                let BindingKind::Definition(mut parent, _) = binding?.kind else {
                    return None;
                };
                for (segment, at) in path.iter().skip(1) {
                    let BindingKind::Definition(next, _) = self
                        .resolve_member(parent, *segment, scope_id, *at)
                        .ok()?
                        .kind
                    else {
                        return None;
                    };
                    parent = next;
                }
                pending.push(parent);
            }
        }
        if members.len() == 1 {
            members.into_iter().next()
        } else {
            None
        }
    }

    /// Resolve accessible candidates for a type container not yet known to the
    /// elaborator. Inspect each ADT scope directly instead of repeatedly scanning
    /// the scope tree through `find_scope_for_def`.
    pub fn accessible_type_members(
        &mut self,
        name: Symbol,
        requester: ScopeId,
    ) -> Arc<[(DefId, DefId)]> {
        if let Some(candidates) = self.type_members.get(&(requester, name)) {
            return Arc::clone(candidates);
        }
        let candidates: Arc<[(DefId, DefId)]> = self
            .scope_tree()
            .iter()
            .filter_map(|scope| {
                if scope.kind != crate::scope::ScopeKind::AdtBody
                    || scope.items.ambiguous_glob(name)
                {
                    return None;
                }
                let binding = self.lookup_in_scope(name, scope.id)?;
                if !binding.accessible(self.scope_tree(), requester) {
                    return None;
                }
                let crate::binding::BindingKind::Definition(member, _) = binding.kind else {
                    return None;
                };
                Some((scope.owner_def, member))
            })
            .collect();
        self.type_members
            .insert((requester, name), Arc::clone(&candidates));
        candidates
    }

    fn check_access(
        &self,
        binding: &Binding,
        requester: ScopeId,
        span: rustc_span::Span,
    ) -> ResolveResult<()> {
        if !binding.accessible(self.scope_tree(), requester) {
            return Err(ResolveError::InvalidNodeType {
                message: "name entry or original declaration is not accessible from this module"
                    .into(),
                span,
            });
        }
        Ok(())
    }

    /// The underlying scope tree.
    pub fn scope_tree(&self) -> &ScopeTree {
        &self.tree.scope_tree
    }

    /// The underlying module tree.
    pub fn module_tree(&self) -> &ModuleTree {
        self.tree
    }

    /// Look up the human-readable name for a DefId.
    pub fn def_name(&self, def_id: DefId) -> Option<&str> {
        self.tree.def_names.get(&def_id).map(|s| s.as_str())
    }
}
