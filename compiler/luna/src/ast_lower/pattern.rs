use super::*;
use crate::{
    context::{
        CompilerContext,
        scope::{Item, ScopeId, Symbol},
    },
    hir::{Definition, Expr, Hir, HirMapping, Module, Pattern, SDefinition, Struct},
    parse::ast::{self, Ast},
    vfs::{self, NodeIdExt, Vfs},
};

impl<'hir, 'ctx, 'vfs> LoweringContext<'hir, 'ctx, 'vfs> {
    pub fn lower_pattern(
        &self,
        ast: &Ast,
        node_index: ast::NodeIndex,
        owner: &Item<'hir>,
    ) -> LoweringResult<(Pattern<'hir>, Vec<Symbol<'hir>>)> {
        let Some((tag, span, children)) = ast.get_node(node_index) else {
            return Err(LowerError::InternalError("Invalid node index".into()));
        };
        use ast::NodeKind::*;
        match tag {
            Id => {
                let src = ast
                    .source_content(node_index, self.hir.source_map())
                    .expect("Failed to get identifier source content");
                let symbol = self.hir.intern_str(&src);
                // 返回模式和绑定的变量列表
                Ok((Pattern::Variable(symbol), vec![symbol]))
            }
            // Null => Ok((Pattern::Null, vec![])),
            // Int | Real | Str | Bool => {
            //     let literal_expr = self.lower_literal(ast, node_index, tag)?;
            //     let literal_pattern = Pattern::Literal(self.hir.intern_expr(literal_expr));
            //     Ok((literal_pattern, vec![]))
            // }
            // Symbol => {
            //     let src = ast
            //         .source_content(children[0], self.hir.source_map())
            //         .expect("Failed to get symbol source content");
            //     let symbol = self.hir.intern_str(&src);
            //     Ok((Pattern::Symbol(symbol), vec![]))
            // }
            // Tuple => {
            //     let actual_children = ast
            //         .get_multi_child_slice(children[0])
            //         .expect("Invalid Tuple children");
            //     let mut patterns = Vec::with_capacity(actual_children.len());
            //     let mut all_variables = Vec::new();
            //     for &child in actual_children {
            //         let (pattern, variables) = self.lower_pattern(ast, child, owner)?;
            //         patterns.push(pattern);
            //         all_variables.extend(variables);
            //     }
            //     Ok((Pattern::TupleDestructure(self.hir.intern_patterns(patterns)), all_variables))
            // }
            _ => todo!("Unsupported pattern kind: {:?}", tag),
        }
    }

    pub fn lower_expr_in_pattern(
        &self,
        ast: &Ast,
        node_index: ast::NodeIndex,
        owner: &Item<'hir>,
    ) -> LoweringResult<Expr<'hir>> {
        let Some((tag, span, children)) = ast.get_node(node_index) else {
            return Err(LowerError::InternalError("Invalid node index".into()));
        };
        use ast::NodeKind::*;
        match tag {
            Id => self.lower_identifier_expr(ast, node_index, owner),
            Symbol => {
                let src = ast
                    .source_content(children[0], self.hir.source_map())
                    .expect("Failed to get symbol source content");
                let symbol = self.hir.intern_str(&src);
                Ok(Expr::SymbolLiteral(symbol))
            }

            Select => self.lower_select_expr(ast, children, owner),
            DiamondCall => self.lower_diamond_call(ast, node_index, owner, children),

            _ => todo!("Unsupported pattern kind: {:?}", tag),
        }
    }
}
