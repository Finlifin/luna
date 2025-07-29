use std::collections::HashMap;

use internment::ArenaIntern;

use crate::hir::HirId;

pub type Symbol<'hir> = ArenaIntern<'hir, str>;
pub type ScopeId = usize;

pub struct ScopeManager<'hir> {
    pub root: ScopeId,
    next_scope_id: ScopeId,
    scopes: HashMap<ScopeId, Scope<'hir>>,
}

pub struct Scope<'hir> {
    ordered: bool,
    id: ScopeId,
    name: Option<Symbol<'hir>>,
    parent: Option<ScopeId>,
    // 匿名作用域怎么办
    items: Vec<Item<'hir>>,
    imports: (),
    params: (),
}

#[derive(Debug, Clone)]
pub struct Item<'hir> {
    symbol: Symbol<'hir>,
    hir_id: HirId,
    scope_id: Option<ScopeId>,
}

impl<'hir> Item<'hir> {
    pub fn new(symbol: Symbol<'hir>, hir_id: HirId, scope_id: Option<ScopeId>) -> Self {
        Item { symbol, hir_id, scope_id }
    }
}

impl<'hir> Scope<'hir> {
    #[inline]
    fn new(id: ScopeId, name: Option<Symbol<'hir>>, parent: Option<ScopeId>, ordered: bool) -> Self {
        Scope {
            id,
            parent,
            name,
            items: Vec::new(),
            ordered,
            imports: (),
            params: (),
        }
    }
}

impl<'hir> ScopeManager<'hir> {
    pub fn new() -> Self {
        let mut result = Self {
            root: 0,
            next_scope_id: 0,
            scopes: HashMap::new(),
        };
        result.root = result.add_scope(None, None, false).unwrap();
        result
    }

    /// 从指定作用域开始解析符号，如果找不到则递归查找父作用域
    pub fn resolve(
        &self,
        name: Symbol<'hir>,
        scope_id: ScopeId,
    ) -> Option<Item<'hir>> {
        if let Some(scope) = self.scopes.get(&scope_id) {
            // 先在当前作用域查找
            if let Some(result) = self.lookup(name, scope_id) {
                return Some(result.clone());
            }

            // 如果没有找到，递归查找父作用域
            if let Some(parent_id) = scope.parent {
                return self.resolve(name, parent_id);
            }
        }
        None
    }

    /// 在指定作用域中查找符号
    pub fn lookup(
        &self,
        name: ArenaIntern<'hir, str>,
        scope_id: ScopeId,
    ) -> Option<Item<'hir>> {
        if let Some(scope) = self.scopes.get(&scope_id) {
            if let Some(item) = scope.items.iter().rev().find(|s| s.symbol == name) {
                return Some(item.clone());
            }
        }
        None
    }

    pub fn add_scope(&mut self, name: Option<Symbol<'hir>>, parent: Option<ScopeId>, ordered: bool) -> Result<ScopeId, ScopeError<'hir>> {
        let id = self.next_scope_id;
        self.next_scope_id += 1;

        if let (Some(parent_id), Some(name)) = (parent, name) {
            if let Some(parent_scope) = self.scopes.get_mut(&parent_id) {
                if !parent_scope.ordered  {
                    // check uniqueness of the name in the parent scope
                    if parent_scope.items.iter().any(|item| item.symbol == name) {
                        return Err(ScopeError::DuplicateSymbol(name));
                    }
                }
                parent_scope.items.push(Item::new(name, 0, Some(id)));
            } else {
                return Err(ScopeError::InvalidParentScope(parent_id));
            }
        }
        
        let scope = Scope::new(id, name, parent, ordered);
        self.scopes.insert(id, scope);
        Ok(id)
    }

    pub fn add_item(&mut self, item: Item<'hir>, scope_id: ScopeId) -> Result<(), ScopeError<'hir>> {
        if let Some(scope) = self.scopes.get_mut(&scope_id) {
            scope.items.push(item);
            Ok(())
        } else {
            Err(ScopeError::InvalidParentScope(scope_id))
        }
    }
}

#[derive(Debug, Clone)]
pub enum ScopeError<'hir> {
    DuplicateSymbol(Symbol<'hir>),
    InvalidParentScope(ScopeId),
}

// Visitor pattern for ScopeManager
pub trait ScopeVisitor<'hir> {
    type Output;
    
    fn visit_scope_manager(&mut self, manager: &ScopeManager<'hir>) -> Self::Output;
    fn visit_scope(&mut self, scope: &Scope<'hir>, manager: &ScopeManager<'hir>) -> String;
    fn visit_symbol(&mut self, symbol: Symbol<'hir>, scope_id: Option<ScopeId>) -> String;
}

// S-expression visitor for scopes
pub struct ScopeSExpressionVisitor;

impl ScopeSExpressionVisitor {
    pub fn new() -> Self {
        Self
    }
}

impl<'hir> ScopeVisitor<'hir> for ScopeSExpressionVisitor {
    type Output = String;
    
    fn visit_scope_manager(&mut self, manager: &ScopeManager<'hir>) -> Self::Output {
        format!(
            "(ScopeManager (root {}) (next_scope_id {}) (scopes {}))",
            manager.root,
            manager.next_scope_id,
            self.visit_scope_recursive(manager, manager.root)
        )
    }
    
    fn visit_scope(&mut self, scope: &Scope<'hir>, _manager: &ScopeManager<'hir>) -> String {
        let symbols = scope.items
            .iter()
            .map(|item| self.visit_symbol(item.symbol, item.scope_id))
            .collect::<Vec<_>>()
            .join(" ");
            
        format!(
            "(Scope (id {}) (ordered {}) (name {}) (parent {}) (symbols {}))",
            scope.id,
            scope.ordered,
            scope.name.map_or("nil".to_string(), |name| format!("\"{}\"", name)),
            scope.parent.map_or("nil".to_string(), |parent| parent.to_string()),
            symbols
        )
    }
    
    fn visit_symbol(&mut self, symbol: Symbol<'hir>, scope_id: Option<ScopeId>) -> String {
        if let Some(id) = scope_id {
            format!("(Symbol \"{}\" scope:{})", symbol, id)
        } else {
            format!("(Symbol \"{}\" scope:nil)", symbol)
        }
    }
}

impl ScopeSExpressionVisitor {
    /// 递归访问作用域树
    fn visit_scope_recursive(&mut self, manager: &ScopeManager<'_>, scope_id: ScopeId) -> String {
        if let Some(scope) = manager.scopes.get(&scope_id) {
            let scope_str = self.visit_scope(scope, manager);
            
            // 查找所有子作用域
            let mut children: Vec<ScopeId> = manager.scopes
                .values()
                .filter(|s| s.parent == Some(scope_id))
                .map(|s| s.id)
                .collect();
            children.sort(); // 按ID排序以保证输出一致性
            
            // 递归访问子作用域
            if children.is_empty() {
                scope_str
            } else {
                let children_str = children
                    .iter()
                    .map(|&child_id| self.visit_scope_recursive(manager, child_id))
                    .collect::<Vec<_>>()
                    .join(" ");
                
                format!("{} (children {})", scope_str, children_str)
            }
        } else {
            String::new()
        }
    }
}

impl<'hir> ScopeManager<'hir> {
    pub fn accept<V: ScopeVisitor<'hir>>(&self, visitor: &mut V) -> V::Output {
        visitor.visit_scope_manager(self)
    }
}