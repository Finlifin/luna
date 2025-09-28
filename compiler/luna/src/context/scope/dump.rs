use super::*;

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
            parts.push(format!("(symbol {} :hir_id {})", item.symbol, item.hir_id));
        }

        // 递归处理子scope
        for item in &child_scope_refs {
            if let Some(child_id) = item.scope_id {
                // 避免无限递归：确保child_id不等于当前scope_id
                if child_id != scope.id {
                    if let Some(child_scope) = manager.scopes.borrow().get(&child_id) {
                        let child_content = self.visit_scope(child_scope, manager);

                        // 检查子scope是否有内容
                        if child_content.trim().is_empty() {
                            parts.push(format!("(child :hir_id {} {})", item.hir_id, item.symbol));
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
                                    parts.push(format!(
                                        "(child {} :hir_id {})",
                                        item.symbol, item.hir_id
                                    ));
                                } else {
                                    parts.push(format!(
                                        "(child {} :hir_id {} {})",
                                        item.symbol, item.hir_id, content
                                    ));
                                }
                            } else {
                                // 如果子scope没有内容，只显示名称
                                parts.push(format!(
                                    "(child :hir_id {} {})",
                                    item.hir_id, item.symbol
                                ));
                            }
                        }
                    }
                } else {
                    parts.push(format!(
                        "(ERROR child {:?} at %{} has the same id of parent scope {:?})",
                        manager.scope_name(child_id).map(|id| id.to_string()),
                        child_id,
                        scope.name
                    ))
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
        if parts.is_empty() {
            format!("({})", scope_name,)
        } else {
            format!("({} {})", scope_name, parts.join(" "),)
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
        if let Some(scope) = manager.scopes.borrow().get(&scope_id) {
            self.visit_scope(scope, manager)
        } else {
            String::new()
        }
    }
}
