use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
};

use internment::ArenaIntern;

use crate::hir::{Clause, HirId, Import, MClause, MImport};

pub type Symbol<'hir> = ArenaIntern<'hir, str>;
pub type ScopeId = usize;

pub struct ScopeManager<'hir> {
    pub root: ScopeId,
    next_scope_id: ScopeId,
    scopes: HashMap<ScopeId, Scope<'hir>>,
}

#[derive(Debug)]
pub struct Scope<'hir> {
    ordered: bool,
    id: ScopeId,
    name: Option<Symbol<'hir>>,
    parent: Option<ScopeId>,
    // 匿名作用域怎么办
    items: Vec<Item<'hir>>,
    imports: RefCell<Vec<Import<'hir>>>,
    clauses: RefCell<Vec<Clause<'hir>>>,
}

#[derive(Debug, Clone, Copy)]
pub struct Item<'hir> {
    pub symbol: Symbol<'hir>,
    pub hir_id: HirId,
    pub scope_id: Option<ScopeId>,
}

impl<'hir> Item<'hir> {
    pub fn new(symbol: Symbol<'hir>, hir_id: HirId, scope_id: Option<ScopeId>) -> Self {
        Item {
            symbol,
            hir_id,
            scope_id,
        }
    }

    /// update the hir_id locally
    pub unsafe fn update_hir_id_unchecked(&self, hir_id: HirId) {
        unsafe {
            let ptr = self as *const _ as *mut Item<'hir>;
            (*ptr).hir_id = hir_id;
        }
    }
}

impl<'hir> Scope<'hir> {
    #[inline]
    fn new(
        id: ScopeId,
        name: Option<Symbol<'hir>>,
        parent: Option<ScopeId>,
        ordered: bool,
    ) -> Self {
        Scope {
            id,
            parent,
            name,
            items: Vec::new(),
            ordered,
            imports: RefCell::new(vec![]),
            clauses: RefCell::new(vec![]),
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
        result.root = result.add_scope(None, None, false, 0).unwrap();
        result
    }

    pub fn info(&self) -> String {
        println!("[DEBUG] ScopeManager Info:");
        let mut output = String::new();
        for scope in self.scopes.values() {
            output.push_str(&format!("Scope {}: {:?}\n", scope.id, scope.name));
            for item in &scope.items {
                output.push_str(&format!(
                    "  - Item: {} (HirId: {:?})\n",
                    item.symbol, item.hir_id
                ));
            }
        }
        output
    }

    /// 从指定作用域开始解析符号，如果找不到则递归查找父作用域
    pub fn resolve(&self, name: Symbol<'hir>, scope_id: ScopeId) -> Option<Item<'hir>> {
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

    pub fn lookup_path(&self, path: &[Symbol<'hir>], scope_id: ScopeId) -> Option<Item<'hir>> {
        let mut current_scope_id = scope_id;
        let mut last_item = None;

        for (i, name) in path.iter().enumerate() {
            if let Some(item) = self.lookup(*name, current_scope_id) {
                last_item = Some(item.clone());

                // Only move to the item's scope if it's not the last element
                if i < path.len() - 1 {
                    current_scope_id = item.scope_id.expect("Item should have a scope");
                }
            } else {
                return None;
            }
        }

        last_item
    }

    /// 在指定作用域中查找符号
    pub fn lookup(&self, name: ArenaIntern<'hir, str>, scope_id: ScopeId) -> Option<Item<'hir>> {
        if let Some(scope) = self.scopes.get(&scope_id) {
            if let Some(item) = scope.items.iter().rev().find(|s| s.symbol == name) {
                return Some(item.clone());
            }
            return scope
                .imports
                .borrow()
                .iter()
                .rev()
                .find_map(|import| match import {
                    Import::All(scope_id) => self.lookup(name, *scope_id),
                    Import::Multi(scope_id, names) => names.iter().rev().find_map(|&import_name| {
                        if import_name == name {
                            self.lookup(name, *scope_id)
                        } else {
                            None
                        }
                    }),
                    Import::Single(scope_id, import_name) => {
                        if *import_name == name {
                            self.lookup(name, *scope_id)
                        } else {
                            None
                        }
                    }
                    Import::Alias {
                        scope_id,
                        alias,
                        original,
                    } => {
                        if *alias == name {
                            self.lookup(*original, *scope_id)
                        } else {
                            None
                        }
                    }
                });
        }
        None
    }

    pub fn items(&self, scope_id: ScopeId) -> Option<&[Item<'hir>]> {
        self.scopes.get(&scope_id).map(|s| &s.items[..])
    }

    pub fn add_scope(
        &mut self,
        name: Option<Symbol<'hir>>,
        parent: Option<ScopeId>,
        ordered: bool,
        hir_id: HirId,
    ) -> Result<ScopeId, ScopeError<'hir>> {
        let id = self.next_scope_id;
        self.next_scope_id += 1;

        if let (Some(parent_id), Some(name)) = (parent, name) {
            if let Some(parent_scope) = self.scopes.get_mut(&parent_id) {
                if !parent_scope.ordered {
                    // check uniqueness of the name in the parent scope
                    if parent_scope.items.iter().any(|item| item.symbol == name) {
                        return Err(ScopeError::DuplicateSymbol(name));
                    }
                }
                parent_scope.items.push(Item::new(name, hir_id, Some(id)));
            } else {
                return Err(ScopeError::InvalidParentScope(parent_id));
            }
        }
        let scope = Scope::new(id, name, parent, ordered);
        self.scopes.insert(id, scope);
        Ok(id)
    }

    pub fn add_item(
        &mut self,
        item: Item<'hir>,
        scope_id: ScopeId,
    ) -> Result<(), ScopeError<'hir>> {
        if let Some(scope) = self.scopes.get_mut(&scope_id) {
            if !scope.ordered {
                // check uniqueness of the name in the parent scope
                if scope
                    .items
                    .iter()
                    .any(|existing| existing.symbol == item.symbol)
                {
                    return Err(ScopeError::DuplicateSymbol(item.symbol));
                }
            }
            scope.items.push(item);
            Ok(())
        } else {
            Err(ScopeError::InvalidParentScope(scope_id))
        }
    }

    pub fn add_clause(
        &self,
        scope_id: ScopeId,
        clause: Clause<'hir>,
    ) -> Result<(), ScopeError<'hir>> {
        if let Some(scope) = self.scopes.get(&scope_id) {
            scope.clauses.borrow_mut().push(clause);
            Ok(())
        } else {
            Err(ScopeError::InvalidParentScope(scope_id))
        }
    }

    pub fn add_import(
        &self,
        scope_id: ScopeId,
        import: Import<'hir>,
    ) -> Result<(), ScopeError<'hir>> {
        if let Some(scope) = self.scopes.get(&scope_id) {
            scope.imports.borrow_mut().push(import);
            Ok(())
        } else {
            Err(ScopeError::InvalidParentScope(scope_id))
        }
    }

    pub fn scope_name(&self, scope_id: ScopeId) -> Option<Symbol<'hir>> {
        self.scopes.get(&scope_id).and_then(|s| s.name)
    }

    pub fn scope_hir_id(&self, scope_id: ScopeId) -> Option<HirId> {
        self.scopes.get(&scope_id).map(|s| s.id)
    }

    pub fn scope_parent(&self, scope_id: ScopeId) -> Option<ScopeId> {
        self.scopes.get(&scope_id).and_then(|s| s.parent)
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
        self.visit_scope_recursive(manager, manager.root)
    }

    fn visit_scope(&mut self, scope: &Scope<'hir>, manager: &ScopeManager<'hir>) -> String {
        let scope_name = scope
            .name
            .map_or("root".to_string(), |name| name.to_string());

        // 分离纯符号和子scope引用
        let (symbols, child_scope_refs): (Vec<&Item<'hir>>, Vec<&Item<'hir>>) =
            scope.items.iter().partition(|item| item.scope_id.is_none());

        let mut parts = Vec::new();

        // 直接添加符号（不用items包装）
        for item in &symbols {
            parts.push(format!("(symbol {})", item.symbol));
        }

        // 递归处理子scope
        for item in &child_scope_refs {
            if let Some(child_id) = item.scope_id {
                // 避免无限递归：确保child_id不等于当前scope_id
                if child_id != scope.id {
                    if let Some(child_scope) = manager.scopes.get(&child_id) {
                        let child_content = self.visit_scope(child_scope, manager);

                        // 检查子scope是否有内容
                        if child_content.trim().is_empty() {
                            parts.push(format!("(child {})", item.symbol));
                        } else {
                            // 如果子scope有内容，将其包装在child中
                            // 去掉子scope的外层括号和名称
                            let content_without_outer =
                                if child_content.starts_with('(') && child_content.ends_with(')') {
                                    &child_content[1..child_content.len() - 1]
                                } else {
                                    &child_content
                                };

                            // 找到第一个空格后的内容（去掉scope名称）
                            if let Some(space_pos) = content_without_outer.find(' ') {
                                let content = content_without_outer[space_pos..].trim();
                                if content.is_empty() {
                                    parts.push(format!("(child {})", item.symbol));
                                } else {
                                    parts.push(format!("(child {} {})", item.symbol, content));
                                }
                            } else {
                                // 如果子scope没有内容，只显示名称
                                parts.push(format!("(child {})", item.symbol));
                            }
                        }
                    }
                }
            }
        }

        // if parts.is_empty() {
        //     format!(
        //         "({} %{} :parent {})",
        //         scope_name,
        //         scope.id,
        //         scope.parent.unwrap_or(0)
        //     )
        // } else {
        //     format!(
        //         "({} {} %{} :parent {})",
        //         scope_name,
        //         parts.join(" "),
        //         scope.id,
        //         scope.parent.unwrap_or(0)
        //     )
        // }
        let imports = scope
            .imports
            .borrow()
            .iter()
            .map(|i| format!("{}", i))
            .collect::<Vec<_>>()
            .join(" ");
        if parts.is_empty() {
            format!("({} :imports {})", scope_name, imports)
        } else {
            format!("({} :imports {} {})", scope_name, imports, parts.join(" "),)
        }
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
    fn visit_scope_recursive(&mut self, manager: &ScopeManager<'_>, scope_id: ScopeId) -> String {
        if let Some(scope) = manager.scopes.get(&scope_id) {
            self.visit_scope(scope, manager)
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
