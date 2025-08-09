use super::*;
use crate::{
    context::{
        CompilerContext,
        scope::{Item, ScopeId},
    },
    hir::{Definition, Expr, Hir, HirMapping, Module, SDefinition, Struct},
    parse::ast::{self, Ast},
    vfs::{self, NodeIdExt, Vfs},
};

impl<'hir, 'ctx, 'vfs> LoweringContext<'hir, 'ctx, 'vfs> {
    pub fn lower_expr(
        &self,
        ast: &Ast,
        node_index: ast::NodeIndex,
        owner: &Item<'hir>,
    ) -> LoweringResult<Expr<'hir>> {
        let tag = ast.get_node_kind(node_index).expect("Invalid node index");
        use ast::NodeKind::*;
        match tag {
            Int => {
                let src = ast
                    .source_content(node_index, &self.hir.source_map)
                    .expect("Failed to get integer source content");
                Ok(Expr::IntLiteral(parse_int_literal(&src, ast, node_index)?))
            }
            Id => {
                let src = ast
                    .source_content(node_index, &self.hir.source_map)
                    .expect("Failed to get identifier source content");
                let symbol = self.hir.intern_str(&src);
                let resolved = self
                    .ctx
                    .scope_manager
                    .resolve(symbol, owner.scope_id.expect("Invalid owner scope ID"))
                    .ok_or(LowerError::UnresolvedIdentifier {
                        message: format!("Unresolved identifier: `{}`", src),
                        span: ast.get_span(node_index).unwrap_or(rustc_span::DUMMY_SP),
                    })?;
                Ok(Expr::Ref(resolved.hir_id))
            }
            _ => todo!("."),
        }
    }
}

fn parse_int_literal<'hir, 'a>(
    src: &'a str,
    ast: &Ast,
    ast_node_index: ast::NodeIndex,
) -> LoweringResult<i64> {
    src.parse::<i64>().map_err(|_| LowerError::LiteralError {
        message: format!("Invalid integer literal: `{}`", src),
        span: ast.get_span(ast_node_index).unwrap_or(rustc_span::DUMMY_SP),
    })
}
