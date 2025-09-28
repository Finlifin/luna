use crate::{
    context::scope::ScopeManager,
    hir::{Definition, Hir, HirId, HirMapping},
};

impl Hir {
    pub fn dump_to_s_expression(&self, hir_id: HirId, scope_manager: &ScopeManager<'_>) -> String {
        match self.get(hir_id) {
            None => format!("(<invalid hir id {}>)", hir_id),
            Some(mapping) => match mapping {
                HirMapping::Expr(expr, _) => format!("(Expr %{} {})", hir_id, expr),
                HirMapping::Pattern(pat, _) => format!("(Pattern %{} {})", hir_id, pat),
                HirMapping::Definition(def, _) => match *def {
                    Definition::Module(module) => {
                        assert!(scope_manager.scope_hir_id(module.scope_id).unwrap() == hir_id);
                        let mut items_str = "".to_string();
                        if let Some(item) = scope_manager.items(module.scope_id) {
                            for item in item {
                                let item_dump =
                                    self.dump_to_s_expression(item.hir_id, scope_manager);
                                items_str.push_str(&format!(" {}", item_dump));
                            }
                        }
                        format!("(MappingModule %{} {} {})", hir_id, module, items_str)
                    }
                    Definition::Package { name, scope_id } => {
                        let mut items_str = "".to_string();
                        if let Some(items) = scope_manager.items(scope_id) {
                            for item in items {
                                let item_dump =
                                    self.dump_to_s_expression(item.hir_id, scope_manager);
                                items_str.push_str(&format!(" {}", item_dump));
                            }
                        }
                        format!("(MappingPackage %{} {} {})", hir_id, name, items_str)
                    }
                    Definition::Struct(sdef) => {
                        let fields = sdef
                            .fields
                            .iter()
                            .map(|field| field.to_string())
                            .reduce(|a, b| a + " " + &b)
                            .unwrap_or_default();
                        format!("(MappingStruct %{} {} {})", hir_id, sdef.name, fields)
                    }
                    _ => format!("(MappingDefinition %{} {})", hir_id, def),
                },
                HirMapping::Param(param, _) => format!("(MappingParam %{} {})", hir_id, param),
                HirMapping::Clause(clause, _) => format!("(MappingClause %{} {})", hir_id, clause),

                HirMapping::Package => "(Package)".to_string(),
                HirMapping::BuiltinPackage => "(BuiltinPackage)".to_string(),

                HirMapping::Unresolved(node_id, node_index, _) => {
                    format!("(Unresolved {} {})", node_id, node_index)
                }
                HirMapping::UnresolvedFileScope(node_id, _) => {
                    format!("(UnresolvedFileScope {})", node_id)
                }
                HirMapping::UnresolvedPackage(node_id) => {
                    format!("(UnresolvedPackage {})", node_id)
                }
                HirMapping::UnresolvedDirectoryModule(node_id, _) => {
                    format!("(UnresolvedDirectoryModule {})", node_id)
                }

                HirMapping::Invalid => "(Invalid)".to_string(),
            },
        }
    }
}
