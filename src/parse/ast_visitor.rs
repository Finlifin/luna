use rustc_span::SourceMap;
use super::ast::{Ast, NodeIndex, NodeKind, NodeType};

/// AST visitor trait，用于遍历 AST 并对每个节点执行操作
pub trait AstVisitor {
    /// 访问者的返回类型
    type Output;

    /// 访问一个节点
    fn visit_node(&mut self, ast: &Ast, node_index: NodeIndex, source_map: &SourceMap) -> Self::Output;

    /// 访问无子节点的节点（叶子节点）
    fn visit_no_child(&mut self, ast: &Ast, node_index: NodeIndex, kind: NodeKind, source_map: &SourceMap) -> Self::Output;

    /// 访问单子节点
    fn visit_single_child(&mut self, ast: &Ast, node_index: NodeIndex, kind: NodeKind, child: NodeIndex, source_map: &SourceMap) -> Self::Output;

    /// 访问双子节点
    fn visit_double_children(&mut self, ast: &Ast, node_index: NodeIndex, kind: NodeKind, children: [NodeIndex; 2], source_map: &SourceMap) -> Self::Output;

    /// 访问三子节点
    fn visit_triple_children(&mut self, ast: &Ast, node_index: NodeIndex, kind: NodeKind, children: [NodeIndex; 3], source_map: &SourceMap) -> Self::Output;

    /// 访问四子节点
    fn visit_quadruple_children(&mut self, ast: &Ast, node_index: NodeIndex, kind: NodeKind, children: [NodeIndex; 4], source_map: &SourceMap) -> Self::Output;

    /// 访问多个子节点
    fn visit_multi_children(&mut self, ast: &Ast, node_index: NodeIndex, kind: NodeKind, children: &[NodeIndex], source_map: &SourceMap) -> Self::Output;

    /// 访问单个子节点 + 多个子节点
    fn visit_single_with_multi_children(&mut self, ast: &Ast, node_index: NodeIndex, kind: NodeKind, single_child: NodeIndex, multi_children: &[NodeIndex], source_map: &SourceMap) -> Self::Output;

    /// 访问双子节点 + 多个子节点
    fn visit_double_with_multi_children(&mut self, ast: &Ast, node_index: NodeIndex, kind: NodeKind, double_children: [NodeIndex; 2], multi_children: &[NodeIndex], source_map: &SourceMap) -> Self::Output;

    /// 访问三子节点 + 多个子节点
    fn visit_triple_with_multi_children(&mut self, ast: &Ast, node_index: NodeIndex, kind: NodeKind, triple_children: [NodeIndex; 3], multi_children: &[NodeIndex], source_map: &SourceMap) -> Self::Output;

    /// 访问函数定义节点 (id, params, return_type, handles_effect, clauses, body)
    fn visit_function_def(&mut self, ast: &Ast, node_index: NodeIndex, kind: NodeKind, id: NodeIndex, params: &[NodeIndex], return_type: NodeIndex, handles_effect: NodeIndex, clauses: &[NodeIndex], body: NodeIndex, source_map: &SourceMap) -> Self::Output;

    /// 访问钻石函数定义节点 (id, type_params, return_type, clauses, body)
    fn visit_diamond_function_def(&mut self, ast: &Ast, node_index: NodeIndex, kind: NodeKind, id: NodeIndex, type_params: &[NodeIndex], return_type: NodeIndex, clauses: &[NodeIndex], body: NodeIndex, source_map: &SourceMap) -> Self::Output;

    /// 访问效果定义节点 (id, params, return_type, clauses)
    fn visit_effect_def(&mut self, ast: &Ast, node_index: NodeIndex, kind: NodeKind, id: NodeIndex, params: &[NodeIndex], return_type: NodeIndex, clauses: &[NodeIndex], source_map: &SourceMap) -> Self::Output;

    /// 访问处理器定义节点 (effect, params, return_type, clauses, body)
    fn visit_handles_def(&mut self, ast: &Ast, node_index: NodeIndex, kind: NodeKind, effect: NodeIndex, params: &[NodeIndex], return_type: NodeIndex, clauses: &[NodeIndex], body: NodeIndex, source_map: &SourceMap) -> Self::Output;

    /// 访问类型定义节点 (id, clauses, body) - 用于 struct/enum/union/impl/extend/module
    fn visit_type_def(&mut self, ast: &Ast, node_index: NodeIndex, kind: NodeKind, id: NodeIndex, clauses: &[NodeIndex], body: NodeIndex, source_map: &SourceMap) -> Self::Output;

    /// 访问 trait 定义节点 (id, super_trait, clauses, body)
    fn visit_trait_def(&mut self, ast: &Ast, node_index: NodeIndex, kind: NodeKind, id: NodeIndex, super_trait: NodeIndex, clauses: &[NodeIndex], body: NodeIndex, source_map: &SourceMap) -> Self::Output;

    /// 访问 impl trait/extend trait 定义节点 (trait_expr, type_expr, clauses, body)
    fn visit_impl_trait_def(&mut self, ast: &Ast, node_index: NodeIndex, kind: NodeKind, trait_expr: NodeIndex, type_expr: NodeIndex, clauses: &[NodeIndex], body: NodeIndex, source_map: &SourceMap) -> Self::Output;

    /// 访问 derive 定义节点 (traits, type_expr, clauses)
    fn visit_derive_def(&mut self, ast: &Ast, node_index: NodeIndex, kind: NodeKind, traits: &[NodeIndex], type_expr: NodeIndex, clauses: &[NodeIndex], source_map: &SourceMap) -> Self::Output;

    /// 访问类型别名节点 (id, type_params, type_expr) - 用于 typealias/newtype
    fn visit_type_alias(&mut self, ast: &Ast, node_index: NodeIndex, kind: NodeKind, id: NodeIndex, type_params: &[NodeIndex], type_expr: NodeIndex, source_map: &SourceMap) -> Self::Output;

    /// 默认的 visitor 实现，根据节点类型分发到对应的方法
    fn default_visit_node(&mut self, ast: &Ast, node_index: NodeIndex, source_map: &SourceMap) -> Self::Output {
        if node_index == 0 {
            // 处理无效节点的默认行为需要由具体实现决定
            panic!("Cannot visit invalid node");
        }
        
        if let Some(kind) = ast.get_node_kind(node_index) {
            match kind.node_type() {
                NodeType::NoChild => {
                    self.visit_no_child(ast, node_index, kind, source_map)
                }
                NodeType::SingleChild => {
                    let children = ast.get_children(node_index);
                    let child_index = children[0];
                    self.visit_single_child(ast, node_index, kind, child_index, source_map)
                }
                NodeType::DoubleChildren => {
                    let children = ast.get_children(node_index);
                    self.visit_double_children(ast, node_index, kind, [children[0], children[1]], source_map)
                }
                NodeType::TripleChildren => {
                    let children = ast.get_children(node_index);
                    self.visit_triple_children(ast, node_index, kind, [children[0], children[1], children[2]], source_map)
                }
                NodeType::QuadrupleChildren => {
                    let children = ast.get_children(node_index);
                    self.visit_quadruple_children(ast, node_index, kind, [children[0], children[1], children[2], children[3]], source_map)
                }
                NodeType::MultiChildren => {
                    let elements = ast.get_children(node_index)[0];
                    let child_nodes = ast.get_multi_child_slice(elements).unwrap();
                    self.visit_multi_children(ast, node_index, kind, child_nodes, source_map)
                }
                NodeType::SingleWithMultiChildren => {
                    let children = ast.get_children(node_index);
                    let first_child = children[0];
                    let multi_children_node = children[1];
                    let multi_children = ast.get_multi_child_slice(multi_children_node).unwrap();
                    self.visit_single_with_multi_children(ast, node_index, kind, first_child, multi_children, source_map)
                }
                NodeType::DoubleWithMultiChildren => {
                    let children = ast.get_children(node_index);
                    let first_child = children[0];
                    let second_child = children[1];
                    let multi_children_node = children[2];
                    let multi_children = ast.get_multi_child_slice(multi_children_node).unwrap();
                    self.visit_double_with_multi_children(ast, node_index, kind, [first_child, second_child], multi_children, source_map)
                }
                NodeType::TripleWithMultiChildren => {
                    let children = ast.get_children(node_index);
                    let first_child = children[0];
                    let second_child = children[1];
                    let third_child = children[2];
                    let multi_children_node = children[3];
                    let multi_children = ast.get_multi_child_slice(multi_children_node).unwrap();
                    self.visit_triple_with_multi_children(ast, node_index, kind, [first_child, second_child, third_child], multi_children, source_map)
                }

                // Complex children patterns
                NodeType::FunctionDefChildren => {
                    let children = ast.get_children(node_index);
                    let id = children[0];
                    let params_node = children[1];
                    let return_type = children[2];
                    let handles_effect = children[3];
                    let clauses_node = children[4];
                    let body = children[5];

                    let params = ast.get_multi_child_slice(params_node).unwrap();
                    let clauses = ast.get_multi_child_slice(clauses_node).unwrap();

                    self.visit_function_def(ast, node_index, kind, id, params, return_type, handles_effect, clauses, body, source_map)
                }

                NodeType::DiamondFunctionDefChildren => {
                    let children = ast.get_children(node_index);
                    let id = children[0];
                    let type_params_node = children[1];
                    let return_type = children[2];
                    let clauses_node = children[3];
                    let body = children[4];

                    let type_params = ast.get_multi_child_slice(type_params_node).unwrap();
                    let clauses = ast.get_multi_child_slice(clauses_node).unwrap();

                    self.visit_diamond_function_def(ast, node_index, kind, id, type_params, return_type, clauses, body, source_map)
                }

                NodeType::EffectDefChildren => {
                    let children = ast.get_children(node_index);
                    let id = children[0];
                    let params_node = children[1];
                    let return_type = children[2];
                    let clauses_node = children[3];

                    let params = ast.get_multi_child_slice(params_node).unwrap();
                    let clauses = ast.get_multi_child_slice(clauses_node).unwrap();

                    self.visit_effect_def(ast, node_index, kind, id, params, return_type, clauses, source_map)
                }

                NodeType::HandlesDefChildren => {
                    let children = ast.get_children(node_index);
                    let effect = children[0];
                    let params_node = children[1];
                    let return_type = children[2];
                    let clauses_node = children[3];
                    let body = children[4];

                    let params = ast.get_multi_child_slice(params_node).unwrap();
                    let clauses = ast.get_multi_child_slice(clauses_node).unwrap();

                    self.visit_handles_def(ast, node_index, kind, effect, params, return_type, clauses, body, source_map)
                }

                NodeType::TypeDefChildren => {
                    let children = ast.get_children(node_index);
                    let id = children[0];
                    let clauses_node = children[1];
                    let body = children[2];

                    let clauses = ast.get_multi_child_slice(clauses_node).unwrap();

                    self.visit_type_def(ast, node_index, kind, id, clauses, body, source_map)
                }

                NodeType::TraitDefChildren => {
                    let children = ast.get_children(node_index);
                    let id = children[0];
                    let super_trait = children[1];
                    let clauses_node = children[2];
                    let body = children[3];

                    let clauses = ast.get_multi_child_slice(clauses_node).unwrap();

                    self.visit_trait_def(ast, node_index, kind, id, super_trait, clauses, body, source_map)
                }

                NodeType::ImplTraitDefChildren | NodeType::ExtendTraitDefChildren => {
                    let children = ast.get_children(node_index);
                    let trait_expr = children[0];
                    let type_expr = children[1];
                    let clauses_node = children[2];
                    let body = children[3];

                    let clauses = ast.get_multi_child_slice(clauses_node).unwrap();

                    self.visit_impl_trait_def(ast, node_index, kind, trait_expr, type_expr, clauses, body, source_map)
                }

                NodeType::DeriveDefChildren => {
                    let children = ast.get_children(node_index);
                    let traits_node = children[0];
                    let type_expr = children[1];
                    let clauses_node = children[2];

                    let traits = ast.get_multi_child_slice(traits_node).unwrap();
                    let clauses = ast.get_multi_child_slice(clauses_node).unwrap();

                    self.visit_derive_def(ast, node_index, kind, traits, type_expr, clauses, source_map)
                }

                NodeType::TypeAliasChildren => {
                    let children = ast.get_children(node_index);
                    let id = children[0];
                    let type_params_node = children[1];
                    let type_expr = children[2];

                    let type_params = ast.get_multi_child_slice(type_params_node).unwrap();

                    self.visit_type_alias(ast, node_index, kind, id, type_params, type_expr, source_map)
                }
            }
        } else {
            panic!("Invalid node index: {}", node_index)
        }
    }
}

/// 遍历 AST 的辅助函数
pub fn visit_ast<V: AstVisitor>(visitor: &mut V, ast: &Ast, node_index: NodeIndex, source_map: &SourceMap) -> V::Output {
    visitor.visit_node(ast, node_index, source_map)
}

/// S-表达式转储 visitor，模仿 dump_to_s_expression 函数
pub struct SExpressionVisitor;

impl AstVisitor for SExpressionVisitor {
    type Output = String;

    fn visit_node(&mut self, ast: &Ast, node_index: NodeIndex, source_map: &SourceMap) -> Self::Output {
        if node_index == 0 {
            return "(<invalid node>)".to_string();
        }
        self.default_visit_node(ast, node_index, source_map)
    }

    fn visit_no_child(&mut self, ast: &Ast, node_index: NodeIndex, kind: NodeKind, source_map: &SourceMap) -> Self::Output {
        let source_file = source_map.lookup_source_file(ast.get_span(node_index).unwrap().lo());

        let source_content = match &source_file.src {
            Some(content) => content.as_str(),
            None => {
                eprintln!("Error: Source file content not available");
                return "<invalid source>".to_string();
            }
        };

        let byte_start = (ast.get_span(node_index).unwrap().lo().0 - source_file.start_pos.0) as usize;
        let byte_end = (ast.get_span(node_index).unwrap().hi().0 - source_file.start_pos.0) as usize;
        format!("({} {})", kind, source_content[byte_start..byte_end].trim())
    }

    fn visit_single_child(&mut self, ast: &Ast, _node_index: NodeIndex, kind: NodeKind, child: NodeIndex, source_map: &SourceMap) -> Self::Output {
        format!(
            "({} {})",
            kind,
            self.visit_node(ast, child, source_map)
        )
    }

    fn visit_double_children(&mut self, ast: &Ast, _node_index: NodeIndex, kind: NodeKind, children: [NodeIndex; 2], source_map: &SourceMap) -> Self::Output {
        format!(
            "({} {} {})",
            kind,
            self.visit_node(ast, children[0], source_map),
            self.visit_node(ast, children[1], source_map)
        )
    }

    fn visit_triple_children(&mut self, ast: &Ast, _node_index: NodeIndex, kind: NodeKind, children: [NodeIndex; 3], source_map: &SourceMap) -> Self::Output {
        format!(
            "({} {} {} {})",
            kind,
            self.visit_node(ast, children[0], source_map),
            self.visit_node(ast, children[1], source_map),
            self.visit_node(ast, children[2], source_map)
        )
    }

    fn visit_quadruple_children(&mut self, ast: &Ast, _node_index: NodeIndex, kind: NodeKind, children: [NodeIndex; 4], source_map: &SourceMap) -> Self::Output {
        format!(
            "({} {} {} {} {})",
            kind,
            self.visit_node(ast, children[0], source_map),
            self.visit_node(ast, children[1], source_map),
            self.visit_node(ast, children[2], source_map),
            self.visit_node(ast, children[3], source_map)
        )
    }

    fn visit_multi_children(&mut self, ast: &Ast, _node_index: NodeIndex, kind: NodeKind, children: &[NodeIndex], source_map: &SourceMap) -> Self::Output {
        let children_str = children
            .iter()
            .map(|&child_index| self.visit_node(ast, child_index, source_map))
            .collect::<Vec<_>>()
            .join(" ");
        format!("({} {})", kind, children_str)
    }

    fn visit_single_with_multi_children(&mut self, ast: &Ast, _node_index: NodeIndex, kind: NodeKind, single_child: NodeIndex, multi_children: &[NodeIndex], source_map: &SourceMap) -> Self::Output {
        let multi_children_str = multi_children
            .iter()
            .map(|&child_index| self.visit_node(ast, child_index, source_map))
            .collect::<Vec<_>>()
            .join(" ");
        format!(
            "({} {} {})",
            kind,
            self.visit_node(ast, single_child, source_map),
            multi_children_str
        )
    }

    fn visit_double_with_multi_children(&mut self, ast: &Ast, _node_index: NodeIndex, kind: NodeKind, double_children: [NodeIndex; 2], multi_children: &[NodeIndex], source_map: &SourceMap) -> Self::Output {
        let multi_children_str = multi_children
            .iter()
            .map(|&child_index| self.visit_node(ast, child_index, source_map))
            .collect::<Vec<_>>()
            .join(" ");
        format!(
            "({} {} {} {})",
            kind,
            self.visit_node(ast, double_children[0], source_map),
            self.visit_node(ast, double_children[1], source_map),
            multi_children_str
        )
    }

    fn visit_triple_with_multi_children(&mut self, ast: &Ast, _node_index: NodeIndex, kind: NodeKind, triple_children: [NodeIndex; 3], multi_children: &[NodeIndex], source_map: &SourceMap) -> Self::Output {
        let multi_children_str = multi_children
            .iter()
            .map(|&child_index| self.visit_node(ast, child_index, source_map))
            .collect::<Vec<_>>()
            .join(" ");
        format!(
            "({} {} {} {} {})",
            kind,
            self.visit_node(ast, triple_children[0], source_map),
            self.visit_node(ast, triple_children[1], source_map),
            self.visit_node(ast, triple_children[2], source_map),
            multi_children_str
        )
    }

    fn visit_function_def(&mut self, ast: &Ast, _node_index: NodeIndex, kind: NodeKind, id: NodeIndex, params: &[NodeIndex], return_type: NodeIndex, handles_effect: NodeIndex, clauses: &[NodeIndex], body: NodeIndex, source_map: &SourceMap) -> Self::Output {
        let params_str = params
            .iter()
            .map(|&child_index| self.visit_node(ast, child_index, source_map))
            .collect::<Vec<_>>()
            .join(" ");
        let clauses_str = clauses
            .iter()
            .map(|&child_index| self.visit_node(ast, child_index, source_map))
            .collect::<Vec<_>>()
            .join(" ");

        format!(
            "({} {} [{}] {} {} [{}] {})",
            kind,
            self.visit_node(ast, id, source_map),
            params_str,
            self.visit_node(ast, return_type, source_map),
            self.visit_node(ast, handles_effect, source_map),
            clauses_str,
            self.visit_node(ast, body, source_map)
        )
    }

    fn visit_diamond_function_def(&mut self, ast: &Ast, _node_index: NodeIndex, kind: NodeKind, id: NodeIndex, type_params: &[NodeIndex], return_type: NodeIndex, clauses: &[NodeIndex], body: NodeIndex, source_map: &SourceMap) -> Self::Output {
        let type_params_str = type_params
            .iter()
            .map(|&child_index| self.visit_node(ast, child_index, source_map))
            .collect::<Vec<_>>()
            .join(" ");
        let clauses_str = clauses
            .iter()
            .map(|&child_index| self.visit_node(ast, child_index, source_map))
            .collect::<Vec<_>>()
            .join(" ");

        format!(
            "({} {} <{}> {} [{}] {})",
            kind,
            self.visit_node(ast, id, source_map),
            type_params_str,
            self.visit_node(ast, return_type, source_map),
            clauses_str,
            self.visit_node(ast, body, source_map)
        )
    }

    fn visit_effect_def(&mut self, ast: &Ast, _node_index: NodeIndex, kind: NodeKind, id: NodeIndex, params: &[NodeIndex], return_type: NodeIndex, clauses: &[NodeIndex], source_map: &SourceMap) -> Self::Output {
        let params_str = params
            .iter()
            .map(|&child_index| self.visit_node(ast, child_index, source_map))
            .collect::<Vec<_>>()
            .join(" ");
        let clauses_str = clauses
            .iter()
            .map(|&child_index| self.visit_node(ast, child_index, source_map))
            .collect::<Vec<_>>()
            .join(" ");

        format!(
            "({} {} [{}] {} [{}])",
            kind,
            self.visit_node(ast, id, source_map),
            params_str,
            self.visit_node(ast, return_type, source_map),
            clauses_str
        )
    }

    fn visit_handles_def(&mut self, ast: &Ast, _node_index: NodeIndex, kind: NodeKind, effect: NodeIndex, params: &[NodeIndex], return_type: NodeIndex, clauses: &[NodeIndex], body: NodeIndex, source_map: &SourceMap) -> Self::Output {
        let params_str = params
            .iter()
            .map(|&child_index| self.visit_node(ast, child_index, source_map))
            .collect::<Vec<_>>()
            .join(" ");
        let clauses_str = clauses
            .iter()
            .map(|&child_index| self.visit_node(ast, child_index, source_map))
            .collect::<Vec<_>>()
            .join(" ");

        format!(
            "({} {} [{}] {} [{}] {})",
            kind,
            self.visit_node(ast, effect, source_map),
            params_str,
            self.visit_node(ast, return_type, source_map),
            clauses_str,
            self.visit_node(ast, body, source_map)
        )
    }

    fn visit_type_def(&mut self, ast: &Ast, _node_index: NodeIndex, kind: NodeKind, id: NodeIndex, clauses: &[NodeIndex], body: NodeIndex, source_map: &SourceMap) -> Self::Output {
        let clauses_str = clauses
            .iter()
            .map(|&child_index| self.visit_node(ast, child_index, source_map))
            .collect::<Vec<_>>()
            .join(" ");

        format!(
            "({} {} [{}] {})",
            kind,
            self.visit_node(ast, id, source_map),
            clauses_str,
            self.visit_node(ast, body, source_map)
        )
    }

    fn visit_trait_def(&mut self, ast: &Ast, _node_index: NodeIndex, kind: NodeKind, id: NodeIndex, super_trait: NodeIndex, clauses: &[NodeIndex], body: NodeIndex, source_map: &SourceMap) -> Self::Output {
        let clauses_str = clauses
            .iter()
            .map(|&child_index| self.visit_node(ast, child_index, source_map))
            .collect::<Vec<_>>()
            .join(" ");

        format!(
            "({} {} {} [{}] {})",
            kind,
            self.visit_node(ast, id, source_map),
            self.visit_node(ast, super_trait, source_map),
            clauses_str,
            self.visit_node(ast, body, source_map)
        )
    }

    fn visit_impl_trait_def(&mut self, ast: &Ast, _node_index: NodeIndex, kind: NodeKind, trait_expr: NodeIndex, type_expr: NodeIndex, clauses: &[NodeIndex], body: NodeIndex, source_map: &SourceMap) -> Self::Output {
        let clauses_str = clauses
            .iter()
            .map(|&child_index| self.visit_node(ast, child_index, source_map))
            .collect::<Vec<_>>()
            .join(" ");

        format!(
            "({} {} {} [{}] {})",
            kind,
            self.visit_node(ast, trait_expr, source_map),
            self.visit_node(ast, type_expr, source_map),
            clauses_str,
            self.visit_node(ast, body, source_map)
        )
    }

    fn visit_derive_def(&mut self, ast: &Ast, _node_index: NodeIndex, kind: NodeKind, traits: &[NodeIndex], type_expr: NodeIndex, clauses: &[NodeIndex], source_map: &SourceMap) -> Self::Output {
        let traits_str = traits
            .iter()
            .map(|&child_index| self.visit_node(ast, child_index, source_map))
            .collect::<Vec<_>>()
            .join(" ");
        let clauses_str = clauses
            .iter()
            .map(|&child_index| self.visit_node(ast, child_index, source_map))
            .collect::<Vec<_>>()
            .join(" ");

        format!(
            "({} [{}] {} [{}])",
            kind,
            traits_str,
            self.visit_node(ast, type_expr, source_map),
            clauses_str
        )
    }

    fn visit_type_alias(&mut self, ast: &Ast, _node_index: NodeIndex, kind: NodeKind, id: NodeIndex, type_params: &[NodeIndex], type_expr: NodeIndex, source_map: &SourceMap) -> Self::Output {
        let type_params_str = type_params
            .iter()
            .map(|&child_index| self.visit_node(ast, child_index, source_map))
            .collect::<Vec<_>>()
            .join(" ");

        format!(
            "({} {} <{}> {})",
            kind,
            self.visit_node(ast, id, source_map),
            type_params_str,
            self.visit_node(ast, type_expr, source_map)
        )
    }
}

/// 便利函数：使用 SExpressionVisitor 转储 AST 为 S-表达式
pub fn dump_ast_to_s_expression(ast: &Ast, node_index: NodeIndex, source_map: &SourceMap) -> String {
    let mut visitor = SExpressionVisitor;
    visit_ast(&mut visitor, ast, node_index, source_map)
}

/// 节点计数 visitor 示例
pub struct NodeCountVisitor {
    count: u32,
}

impl NodeCountVisitor {
    pub fn new() -> Self {
        NodeCountVisitor { count: 0 }
    }

    pub fn get_count(&self) -> u32 {
        self.count
    }
}

impl AstVisitor for NodeCountVisitor {
    type Output = ();

    fn visit_node(&mut self, ast: &Ast, node_index: NodeIndex, source_map: &SourceMap) -> Self::Output {
        if node_index == 0 {
            return;
        }
        self.count += 1;
        self.default_visit_node(ast, node_index, source_map)
    }

    fn visit_no_child(&mut self, _ast: &Ast, _node_index: NodeIndex, _kind: NodeKind, _source_map: &SourceMap) -> Self::Output {
        // 叶子节点，无需进一步访问
    }

    fn visit_single_child(&mut self, ast: &Ast, _node_index: NodeIndex, _kind: NodeKind, child: NodeIndex, source_map: &SourceMap) -> Self::Output {
        self.visit_node(ast, child, source_map);
    }

    fn visit_double_children(&mut self, ast: &Ast, _node_index: NodeIndex, _kind: NodeKind, children: [NodeIndex; 2], source_map: &SourceMap) -> Self::Output {
        self.visit_node(ast, children[0], source_map);
        self.visit_node(ast, children[1], source_map);
    }

    fn visit_triple_children(&mut self, ast: &Ast, _node_index: NodeIndex, _kind: NodeKind, children: [NodeIndex; 3], source_map: &SourceMap) -> Self::Output {
        self.visit_node(ast, children[0], source_map);
        self.visit_node(ast, children[1], source_map);
        self.visit_node(ast, children[2], source_map);
    }

    fn visit_quadruple_children(&mut self, ast: &Ast, _node_index: NodeIndex, _kind: NodeKind, children: [NodeIndex; 4], source_map: &SourceMap) -> Self::Output {
        self.visit_node(ast, children[0], source_map);
        self.visit_node(ast, children[1], source_map);
        self.visit_node(ast, children[2], source_map);
        self.visit_node(ast, children[3], source_map);
    }

    fn visit_multi_children(&mut self, ast: &Ast, _node_index: NodeIndex, _kind: NodeKind, children: &[NodeIndex], source_map: &SourceMap) -> Self::Output {
        for &child in children {
            self.visit_node(ast, child, source_map);
        }
    }

    fn visit_single_with_multi_children(&mut self, ast: &Ast, _node_index: NodeIndex, _kind: NodeKind, single_child: NodeIndex, multi_children: &[NodeIndex], source_map: &SourceMap) -> Self::Output {
        self.visit_node(ast, single_child, source_map);
        for &child in multi_children {
            self.visit_node(ast, child, source_map);
        }
    }

    fn visit_double_with_multi_children(&mut self, ast: &Ast, _node_index: NodeIndex, _kind: NodeKind, double_children: [NodeIndex; 2], multi_children: &[NodeIndex], source_map: &SourceMap) -> Self::Output {
        self.visit_node(ast, double_children[0], source_map);
        self.visit_node(ast, double_children[1], source_map);
        for &child in multi_children {
            self.visit_node(ast, child, source_map);
        }
    }

    fn visit_triple_with_multi_children(&mut self, ast: &Ast, _node_index: NodeIndex, _kind: NodeKind, triple_children: [NodeIndex; 3], multi_children: &[NodeIndex], source_map: &SourceMap) -> Self::Output {
        self.visit_node(ast, triple_children[0], source_map);
        self.visit_node(ast, triple_children[1], source_map);
        self.visit_node(ast, triple_children[2], source_map);
        for &child in multi_children {
            self.visit_node(ast, child, source_map);
        }
    }

    fn visit_function_def(&mut self, ast: &Ast, _node_index: NodeIndex, _kind: NodeKind, id: NodeIndex, params: &[NodeIndex], return_type: NodeIndex, handles_effect: NodeIndex, clauses: &[NodeIndex], body: NodeIndex, source_map: &SourceMap) -> Self::Output {
        self.visit_node(ast, id, source_map);
        for &param in params {
            self.visit_node(ast, param, source_map);
        }
        self.visit_node(ast, return_type, source_map);
        self.visit_node(ast, handles_effect, source_map);
        for &clause in clauses {
            self.visit_node(ast, clause, source_map);
        }
        self.visit_node(ast, body, source_map);
    }

    fn visit_diamond_function_def(&mut self, ast: &Ast, _node_index: NodeIndex, _kind: NodeKind, id: NodeIndex, type_params: &[NodeIndex], return_type: NodeIndex, clauses: &[NodeIndex], body: NodeIndex, source_map: &SourceMap) -> Self::Output {
        self.visit_node(ast, id, source_map);
        for &type_param in type_params {
            self.visit_node(ast, type_param, source_map);
        }
        self.visit_node(ast, return_type, source_map);
        for &clause in clauses {
            self.visit_node(ast, clause, source_map);
        }
        self.visit_node(ast, body, source_map);
    }

    fn visit_effect_def(&mut self, ast: &Ast, _node_index: NodeIndex, _kind: NodeKind, id: NodeIndex, params: &[NodeIndex], return_type: NodeIndex, clauses: &[NodeIndex], source_map: &SourceMap) -> Self::Output {
        self.visit_node(ast, id, source_map);
        for &param in params {
            self.visit_node(ast, param, source_map);
        }
        self.visit_node(ast, return_type, source_map);
        for &clause in clauses {
            self.visit_node(ast, clause, source_map);
        }
    }

    fn visit_handles_def(&mut self, ast: &Ast, _node_index: NodeIndex, _kind: NodeKind, effect: NodeIndex, params: &[NodeIndex], return_type: NodeIndex, clauses: &[NodeIndex], body: NodeIndex, source_map: &SourceMap) -> Self::Output {
        self.visit_node(ast, effect, source_map);
        for &param in params {
            self.visit_node(ast, param, source_map);
        }
        self.visit_node(ast, return_type, source_map);
        for &clause in clauses {
            self.visit_node(ast, clause, source_map);
        }
        self.visit_node(ast, body, source_map);
    }

    fn visit_type_def(&mut self, ast: &Ast, _node_index: NodeIndex, _kind: NodeKind, id: NodeIndex, clauses: &[NodeIndex], body: NodeIndex, source_map: &SourceMap) -> Self::Output {
        self.visit_node(ast, id, source_map);
        for &clause in clauses {
            self.visit_node(ast, clause, source_map);
        }
        self.visit_node(ast, body, source_map);
    }

    fn visit_trait_def(&mut self, ast: &Ast, _node_index: NodeIndex, _kind: NodeKind, id: NodeIndex, super_trait: NodeIndex, clauses: &[NodeIndex], body: NodeIndex, source_map: &SourceMap) -> Self::Output {
        self.visit_node(ast, id, source_map);
        self.visit_node(ast, super_trait, source_map);
        for &clause in clauses {
            self.visit_node(ast, clause, source_map);
        }
        self.visit_node(ast, body, source_map);
    }

    fn visit_impl_trait_def(&mut self, ast: &Ast, _node_index: NodeIndex, _kind: NodeKind, trait_expr: NodeIndex, type_expr: NodeIndex, clauses: &[NodeIndex], body: NodeIndex, source_map: &SourceMap) -> Self::Output {
        self.visit_node(ast, trait_expr, source_map);
        self.visit_node(ast, type_expr, source_map);
        for &clause in clauses {
            self.visit_node(ast, clause, source_map);
        }
        self.visit_node(ast, body, source_map);
    }

    fn visit_derive_def(&mut self, ast: &Ast, _node_index: NodeIndex, _kind: NodeKind, traits: &[NodeIndex], type_expr: NodeIndex, clauses: &[NodeIndex], source_map: &SourceMap) -> Self::Output {
        for &trait_ref in traits {
            self.visit_node(ast, trait_ref, source_map);
        }
        self.visit_node(ast, type_expr, source_map);
        for &clause in clauses {
            self.visit_node(ast, clause, source_map);
        }
    }

    fn visit_type_alias(&mut self, ast: &Ast, _node_index: NodeIndex, _kind: NodeKind, id: NodeIndex, type_params: &[NodeIndex], type_expr: NodeIndex, source_map: &SourceMap) -> Self::Output {
        self.visit_node(ast, id, source_map);
        for &type_param in type_params {
            self.visit_node(ast, type_param, source_map);
        }
        self.visit_node(ast, type_expr, source_map);
    }
}

/// 便利函数：统计 AST 中的节点数量
pub fn count_ast_nodes(ast: &Ast, node_index: NodeIndex, source_map: &SourceMap) -> u32 {
    let mut visitor = NodeCountVisitor::new();
    visit_ast(&mut visitor, ast, node_index, source_map);
    visitor.get_count()
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_s_expression_visitor() {
        // 这里应该添加一些测试用例
        // 由于需要创建完整的 AST 和 SourceMap，这里只是一个框架
        assert!(true);
    }

    #[test]
    fn test_node_count_visitor() {
        // 这里应该添加一些测试用例
        // 由于需要创建完整的 AST 和 SourceMap，这里只是一个框架
        assert!(true);
    }
}

/// 使用示例和文档
/// 
/// 这个 visitor pattern 的设计允许你：
/// 
/// 1. 实现自定义的 AST 遍历逻辑
/// 2. 在遍历过程中收集信息或进行转换
/// 3. 以类型安全的方式处理不同类型的 AST 节点
/// 
/// ## 示例用法：
/// 
/// ```ignore
/// use crate::parse::ast_visitor::{SExpressionVisitor, visit_ast, count_ast_nodes};
/// use rustc_span::SourceMap;
/// 
/// // 使用 S-表达式 visitor
/// let mut visitor = SExpressionVisitor;
/// let s_expr = visit_ast(&mut visitor, &ast, root_index, &source_map);
/// println!("AST as S-expression: {}", s_expr);
/// 
/// // 统计节点数量
/// let node_count = count_ast_nodes(&ast, root_index, &source_map);
/// println!("Total nodes: {}", node_count);
/// ```
/// 
/// ## 自定义 Visitor 示例：
/// 
/// ```ignore
/// struct MyCustomVisitor {
///     depth: usize,
/// }
/// 
/// impl AstVisitor for MyCustomVisitor {
///     type Output = ();
/// 
///     fn visit_node(&mut self, ast: &Ast, node_index: NodeIndex, source_map: &SourceMap) -> Self::Output {
///         if node_index == 0 {
///             return;
///         }
///         
///         // 自定义逻辑：打印节点信息
///         let kind = ast.get_node_kind(node_index).unwrap();
///         println!("{}{:?}", "  ".repeat(self.depth), kind);
///         
///         // 增加深度并继续遍历
///         self.depth += 1;
///         self.default_visit_node(ast, node_index, source_map);
///         self.depth -= 1;
///     }
///     
///     // 实现其他必需的方法...
///     fn visit_no_child(&mut self, _ast: &Ast, _node_index: NodeIndex, _kind: NodeKind, _source_map: &SourceMap) -> Self::Output {
///         // 叶子节点处理
///     }
///     
///     // ... 其他方法的实现
/// }
/// ```
pub struct _DocumentationPlaceholder;
