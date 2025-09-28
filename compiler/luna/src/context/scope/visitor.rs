use super::*;

// Visitor pattern for ScopeManager
pub trait ScopeVisitor<'hir> {
    type Output;

    fn visit_scope_manager(&mut self, manager: &ScopeManager<'hir>) -> Self::Output;
    fn visit_scope(&mut self, scope: &Scope<'hir>, manager: &ScopeManager<'hir>) -> String;
    fn visit_symbol(&mut self, symbol: Symbol<'hir>, scope_id: Option<ScopeId>) -> String;
}

impl<'hir> ScopeManager<'hir> {
    pub fn accept<V: ScopeVisitor<'hir>>(&self, visitor: &mut V) -> V::Output {
        visitor.visit_scope_manager(self)
    }
}

// S-expression visitor for scopes
pub struct ScopeSExpressionVisitor;
