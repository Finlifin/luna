use super::*;

pub struct ScopeManager<'hir> {
    pub root: ScopeId,
    next_scope_id: Cell<ScopeId>,
    pub(crate) scopes: RefCell<HashMap<ScopeId, Scope<'hir>>>,
}

impl<'hir> ScopeManager<'hir> {
    pub fn new() -> Self {
        let mut result = Self {
            root: 0,
            next_scope_id: Cell::new(0),
            scopes: RefCell::new(HashMap::new()),
        };
        result.root = result.add_scope(None, None, false, 0).unwrap();
        result
    }

    pub fn info(&self) -> String {
        println!("[DEBUG] ScopeManager Info:");
        let mut output = String::new();
        for scope in self.scopes.borrow().values() {
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

    /// 从指定作用域开始解析符号，如果找不到则递归查找父作用域, 返回符号所在作用域和符号
    pub fn resolve(&self, name: Symbol<'hir>, scope_id: ScopeId) -> Option<(ScopeId, Item<'hir>)> {
        if let Some(scope) = self.scopes.borrow().get(&scope_id) {
            // 先在当前作用域查找
            if let Some(result) = self.lookup(name, scope_id) {
                return Some((scope_id, result.clone()));
            }

            if let Some(clause) = scope
                .clauses
                .borrow()
                .iter()
                .rev()
                .find_map(|clause| resolve_symbol_in_clause(name, scope_id, clause))
            {
                return Some((scope_id, clause));
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
        if let Some(scope) = self.scopes.borrow().get(&scope_id) {
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

    pub fn items(&self, scope_id: ScopeId) -> Option<Vec<Item<'hir>>> {
        self.scopes.borrow().get(&scope_id).map(|s| s.items.clone())
    }

    pub fn add_scope(
        &self,
        name: Option<Symbol<'hir>>,
        parent: Option<ScopeId>,
        ordered: bool,
        hir_id: HirId,
    ) -> Result<ScopeId, ScopeError<'hir>> {
        let id = self.next_scope_id.get();
        self.next_scope_id.set(id + 1);

        if let (Some(parent_id), Some(name)) = (parent, name) {
            if let Some(parent_scope) = self.scopes.borrow_mut().get_mut(&parent_id) {
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
        let scope = Scope::new(id, hir_id, name, parent, ordered);
        self.scopes.borrow_mut().insert(id, scope);
        Ok(id)
    }

    pub fn add_item(&self, item: Item<'hir>, scope_id: ScopeId) -> Result<(), ScopeError<'hir>> {
        if let Some(scope) = self.scopes.borrow_mut().get_mut(&scope_id) {
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
        if let Some(scope) = self.scopes.borrow().get(&scope_id) {
            scope.clauses.borrow_mut().push(clause);
            Ok(())
        } else {
            Err(ScopeError::InvalidParentScope(scope_id))
        }
    }

    pub fn scope_clauses(&self, scope_id: ScopeId) -> Option<Vec<Clause<'hir>>> {
        self.scopes
            .borrow()
            .get(&scope_id)
            .map(|s| s.clauses.borrow().clone())
    }

    pub fn add_import(
        &self,
        scope_id: ScopeId,
        import: Import<'hir>,
    ) -> Result<(), ScopeError<'hir>> {
        if let Some(scope) = self.scopes.borrow().get(&scope_id) {
            scope.imports.borrow_mut().push(import);
            Ok(())
        } else {
            Err(ScopeError::InvalidParentScope(scope_id))
        }
    }

    pub fn scope_name(&self, scope_id: ScopeId) -> Option<Symbol<'hir>> {
        self.scopes.borrow().get(&scope_id).and_then(|s| s.name)
    }

    pub fn scope_hir_id(&self, scope_id: ScopeId) -> Option<HirId> {
        self.scopes.borrow().get(&scope_id).map(|s| s.hir_id)
    }

    pub fn scope_parent(&self, scope_id: ScopeId) -> Option<ScopeId> {
        self.scopes.borrow().get(&scope_id).and_then(|s| s.parent)
    }
}

fn resolve_symbol_in_clause<'hir>(
    name: ArenaIntern<'hir, str>,
    scope_id: usize,
    clause: &Clause<'hir>,
) -> Option<Item<'hir>> {
    match clause {
        Clause::Decl {
            symbol, self_id, ..
        } if *symbol == name => Some(Item::new(*symbol, *self_id, None)),
        Clause::TypeDecl {
            symbol, self_id, ..
        } if *symbol == name => Some(Item::new(*symbol, *self_id, None)),
        Clause::TypeTraitBounded {
            symbol, self_id, ..
        } if *symbol == name => Some(Item::new(*symbol, *self_id, None)),

        _ => None,
    }
}
