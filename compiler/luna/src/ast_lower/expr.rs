use std::option;

use super::*;
use crate::{
    context::{
        CompilerContext,
        scope::{Item, ScopeId},
    },
    hir::{
        BinaryOp, BlockKind, Definition, Expr, Hir, HirMapping, Module, Property, SDefinition,
        Struct,
    },
    hir_binary,
    hir_expr,
    hir_op,
    hir_str,
    parse::ast::{self, Ast, NodeKind},
    vfs::{self, NodeIdExt, Vfs}, // HIR macros are automatically available through #[macro_export]
};

use crate::{hir_int, hir_ref}; // Explicit macro imports

impl<'hir, 'ctx, 'vfs> LoweringContext<'hir, 'ctx, 'vfs> {
    pub fn lower_expr(
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
            SelfCap | SelfLower | Unit | Null => self.lower_special_expr(ast, node_index, tag),
            Refer | Deref | BoolNot => self.lower_unary_expr(ast, node_index, owner, tag, children),
            Int | Real | Str | Bool => self.lower_literal(ast, node_index, tag),
            Symbol => {
                let src = ast
                    .source_content(children[0], self.hir.source_map())
                    .expect("Failed to get symbol source content");
                let symbol = self.hir.intern_str(&src);
                Ok(Expr::SymbolLiteral(symbol))
            }
            Id => self.lower_identifier_expr(ast, node_index, owner),

            Add | Sub | Mul | Div | BoolEq | BoolNotEq | BoolGt | BoolGtEq | BoolLt | BoolLtEq
            | BoolAnd | BoolOr | BoolImplies | TypedWith => {
                self.lower_binary_expr(ast, node_index, owner, tag, ast.get_children(node_index))
            }

            RangeFrom | RangeTo | RangeFull | RangeFromTo | RangeFromToInclusive
            | RangeToInclusive => {
                self.lower_range_expr(ast, node_index, owner, tag, ast.get_children(node_index))
            }

            Select => self.lower_select_expr(ast, children, owner),

            ListOf => self.lower_list_of(ast, node_index, owner, children),
            Tuple => self.lower_tuple(ast, node_index, owner, children),
            Object => self.lower_object(ast, node_index, owner, children),

            Call => self.lower_call(ast, node_index, owner, children),
            DiamondCall => self.lower_diamond_call(ast, node_index, owner, children),
            ObjectCall => self.lower_object_call(ast, node_index, owner, children),

            ExprStatement => {
                let expr = self.lower_expr(ast, children[0], owner)?;
                Ok(Expr::ExprStatement(self.hir.intern_expr(expr)))
            }
            Block => self.lower_block(ast, node_index, owner, children, BlockKind::Normal),
            IfStatement => self.lower_if_statement(ast, node_index, owner, children),
            LetDecl => self.lower_let_decl(ast, node_index, owner, children),
            ConstDecl => self.lower_const_decl(ast, node_index, owner, children),
            _ => todo!("Unsupported expression kind: {:?}", tag),
        }
    }

    fn lower_special_expr(
        &self,
        ast: &Ast,
        node_index: ast::NodeIndex,
        tag: ast::NodeKind,
    ) -> LoweringResult<Expr<'hir>> {
        use ast::NodeKind::*;
        match tag {
            SelfCap => Ok(Expr::TySelf),
            SelfLower => Ok(Expr::SelfVal),
            Unit => Ok(Expr::Unit),
            Null => Ok(Expr::Null),
            _ => unreachable!(),
        }
    }

    pub fn lower_literal(
        &self,
        ast: &Ast,
        node_index: ast::NodeIndex,
        tag: ast::NodeKind,
    ) -> LoweringResult<Expr<'hir>> {
        use ast::NodeKind::*;
        match tag {
            Int => {
                let src = ast
                    .source_content(node_index, &self.hir.source_map)
                    .expect("Failed to get integer source content");
                let value = parse_int_literal(&src, ast, node_index)?;
                Ok(hir_int!(self.hir, value))
            }
            Real => {
                let src = ast
                    .source_content(node_index, &self.hir.source_map)
                    .expect("Failed to get real source content");
                let (decimal, numeric) = parse_real_literal(&src, ast, node_index)?;
                Ok(Expr::RealLiteral(decimal, numeric))
            }
            Str => {
                let src = ast
                    .source_content(node_index, &self.hir.source_map)
                    .expect("Failed to get string source content");
                Ok(hir_str!(self.hir, &src))
            }
            Bool => {
                let src = ast
                    .source_content(node_index, &self.hir.source_map)
                    .expect("Failed to get bool source content");
                let value = src == "true";
                Ok(Expr::BoolLiteral(value))
            }
            _ => unreachable!(),
        }
    }

    pub fn lower_identifier_expr(
        &self,
        ast: &Ast,
        node_index: ast::NodeIndex,
        owner: &Item<'hir>,
    ) -> LoweringResult<Expr<'hir>> {
        let src = ast
            .source_content(node_index, self.hir.source_map())
            .expect("Failed to get identifier source content");
        let symbol = self.hir.intern_str(&src);

        // check whether the identifier is preserved
        if let Some(preserved_expr) = self.hir.preserved_expr_ids.get(&symbol) {
            return Ok(**preserved_expr);
        }

        let (_, resolved) = self
            .ctx
            .scope_manager
            .resolve(symbol, owner.scope_id.expect("Invalid owner scope ID"))
            .ok_or(LowerError::UnresolvedIdentifier {
                message: format!("Unresolved identifier: `{}`", src),
                span: ast.get_span(node_index).unwrap_or(rustc_span::DUMMY_SP),
            })?;
        Ok(hir_ref!(self.hir, resolved.hir_id))
    }

    fn lower_unary_expr(
        &self,
        ast: &Ast,
        node_index: ast::NodeIndex,
        owner: &Item<'hir>,
        tag: ast::NodeKind,
        children: &[ast::NodeIndex],
    ) -> LoweringResult<Expr<'hir>> {
        let operand = self.lower_expr(ast, children[0], owner)?;
        use ast::NodeKind::*;
        match tag {
            BoolNot => Ok(hir_op!(self.hir, Not, operand)),
            Refer => Ok(hir_op!(self.hir, Refer, operand)),
            Deref => Ok(hir_op!(self.hir, Deref, operand)),
            _ => unreachable!(),
        }
    }

    fn lower_binary_expr(
        &self,
        ast: &Ast,
        node_index: ast::NodeIndex,
        owner: &Item<'hir>,
        tag: ast::NodeKind,
        children: &[ast::NodeIndex],
    ) -> LoweringResult<Expr<'hir>> {
        let left = self.lower_expr(ast, children[0], owner)?;
        let right = self.lower_expr(ast, children[1], owner)?;

        use ast::NodeKind::*;
        match tag {
            Add => Ok(hir_op!(self.hir, Add, left, right)),
            Sub => Ok(hir_op!(self.hir, Sub, left, right)),
            Mul => Ok(hir_op!(self.hir, Mul, left, right)),
            Div => Ok(hir_op!(self.hir, Div, left, right)),
            BoolEq => Ok(hir_op!(self.hir, BoolEq, left, right)),
            BoolNotEq => Ok(hir_op!(self.hir, BoolNotEq, left, right)),
            BoolLt => Ok(hir_op!(self.hir, BoolLt, left, right)),
            BoolGt => Ok(hir_op!(self.hir, BoolGt, left, right)),
            BoolLtEq => Ok(hir_op!(self.hir, BoolLtEq, left, right)),
            BoolGtEq => Ok(hir_op!(self.hir, BoolGtEq, left, right)),
            BoolAnd => Ok(hir_op!(self.hir, BoolAnd, left, right)),
            BoolOr => Ok(hir_op!(self.hir, BoolOr, left, right)),
            TypedWith => Ok(hir_op!(self.hir, BoolTypedWith, left, right)),
            _ => unreachable!(),
        }
    }

    fn lower_range_expr(
        &self,
        ast: &Ast,
        node_index: ast::NodeIndex,
        owner: &Item<'hir>,
        tag: ast::NodeKind,
        children: &[ast::NodeIndex],
    ) -> LoweringResult<Expr<'hir>> {
        use ast::NodeKind::*;
        match tag {
            RangeFrom => {
                let start = self.lower_expr(ast, children[0], owner)?;
                Ok(Expr::Range {
                    from: self.hir.intern_expr(start),
                    to: self.hir.singleton.null_,
                    inclusive: false,
                })
            }
            RangeTo => {
                let end = self.lower_expr(ast, children[0], owner)?;
                Ok(Expr::Range {
                    from: self.hir.singleton.null_,
                    to: self.hir.intern_expr(end),
                    inclusive: false,
                })
            }
            RangeFull => Ok(Expr::Range {
                from: self.hir.singleton.null_,
                to: self.hir.singleton.null_,
                inclusive: false,
            }),
            RangeFromTo => {
                let start = self.lower_expr(ast, children[0], owner)?;
                let end = self.lower_expr(ast, children[1], owner)?;
                Ok(Expr::Range {
                    from: self.hir.intern_expr(start),
                    to: self.hir.intern_expr(end),
                    inclusive: false,
                })
            }
            RangeFromToInclusive => {
                let start = self.lower_expr(ast, children[0], owner)?;
                let end = self.lower_expr(ast, children[1], owner)?;
                Ok(Expr::Range {
                    from: self.hir.intern_expr(start),
                    to: self.hir.intern_expr(end),
                    inclusive: true,
                })
            }
            RangeToInclusive => {
                let end = self.lower_expr(ast, children[0], owner)?;
                Ok(Expr::Range {
                    from: self.hir.singleton.null_,
                    to: self.hir.intern_expr(end),
                    inclusive: true,
                })
            }
            _ => unreachable!(),
        }
    }

    pub fn lower_select_expr(
        &self,
        ast: &Ast,
        children: &[ast::NodeIndex],
        owner: &Item<'hir>,
    ) -> LoweringResult<Expr<'hir>> {
        if children.len() < 2 {
            return Err(LowerError::InternalError(
                "Select node missing children".into(),
            ));
        }
        let left_expr = self.lower_expr(ast, children[0], owner)?;
        let selected_id = ast
            .source_content(children[1], self.hir.source_map())
            .ok_or(LowerError::InternalError(
                "Failed to get selected ID".into(),
            ))?;
        Ok(Expr::Select(
            self.hir.intern_expr(left_expr),
            self.hir.intern_str(&selected_id),
        ))
    }

    fn lower_list_of(
        &self,
        ast: &Ast,
        node_index: ast::NodeIndex,
        owner: &Item<'hir>,
        children: &[ast::NodeIndex],
    ) -> LoweringResult<Expr<'hir>> {
        let actual_children = ast
            .get_multi_child_slice(children[0])
            .expect("Invalid ListOf children");
        let mut elements = Vec::with_capacity(actual_children.len());
        for &child in actual_children {
            let element = self.lower_expr(ast, child, owner)?;
            elements.push(element);
        }
        Ok(Expr::List(self.hir.intern_exprs(elements)))
    }

    fn lower_tuple(
        &self,
        ast: &Ast,
        node_index: ast::NodeIndex,
        owner: &Item<'hir>,
        children: &[ast::NodeIndex],
    ) -> LoweringResult<Expr<'hir>> {
        let actual_children = ast
            .get_multi_child_slice(children[0])
            .expect("Invalid Tuple children");
        let mut elements = Vec::with_capacity(actual_children.len());
        for &child in actual_children {
            let element = self.lower_expr(ast, child, owner)?;
            elements.push(element);
        }
        Ok(Expr::Tuple(self.hir.intern_exprs(elements)))
    }

    fn lower_object(
        &self,
        ast: &Ast,
        node_index: ast::NodeIndex,
        owner: &Item<'hir>,
        children: &[ast::NodeIndex],
    ) -> LoweringResult<Expr<'hir>> {
        let actual_children = ast
            .get_multi_child_slice(children[0])
            .expect("Invalid Object children");
        let mut elements = Vec::with_capacity(actual_children.len());
        let mut properties = Vec::with_capacity(actual_children.len());
        for &child in actual_children {
            let Some((kind, span, children_of_child)) = ast.get_node(child) else {
                return Err(LowerError::InternalError(
                    "Invalid Object child node".into(),
                ));
            };
            match kind {
                NodeKind::Property => {
                    let id = ast
                        .source_content(children_of_child[0], &self.hir.source_map())
                        .expect("Invalid property ID");
                    let value = self.lower_expr(ast, children_of_child[1], owner)?;
                    properties.push(Property {
                        name: self.hir.intern_str(&id),
                        value: self.hir.intern_expr(value),
                    });
                }
                _ => {
                    elements.push(self.lower_expr(ast, child, owner)?);
                }
            }
        }
        Ok(Expr::Object(
            self.hir.intern_exprs(elements),
            self.hir.intern_properties(properties),
        ))
    }

    fn lower_call(
        &self,
        ast: &Ast,
        node_index: ast::NodeIndex,
        owner: &Item<'hir>,
        children: &[ast::NodeIndex],
    ) -> LoweringResult<Expr<'hir>> {
        let callee = self.lower_expr(ast, children[0], owner)?;
        let args_indices = ast
            .get_multi_child_slice(children[1])
            .expect("Invalid call args index");
        let mut args = Vec::new();
        let mut optional_args = Vec::new();
        for &arg in args_indices {
            let Some((kind, span, children_of_arg)) = ast.get_node(arg) else {
                return Err(LowerError::InternalError(
                    "Invalid call argument node".into(),
                ));
            };

            match kind {
                NodeKind::PropertyAssignment => {
                    let id = ast
                        .source_content(children_of_arg[0], &self.hir.source_map())
                        .expect("Invalid property ID");
                    let value = self.lower_expr(ast, children_of_arg[1], owner)?;
                    optional_args.push(Property {
                        name: self.hir.intern_str(&id),
                        value: self.hir.intern_expr(value),
                    });
                }
                _ => {
                    args.push(self.lower_expr(ast, arg, owner)?);
                }
            }
        }
        Ok(Expr::FnApply {
            callee: self.hir.intern_expr(callee),
            args: self.hir.intern_exprs(args),
            optional_args: self.hir.intern_properties(optional_args),
        })
    }

    pub fn lower_diamond_call(
        &self,
        ast: &Ast,
        node_index: ast::NodeIndex,
        owner: &Item<'hir>,
        children: &[ast::NodeIndex],
    ) -> LoweringResult<Expr<'hir>> {
        let callee = self.lower_expr(ast, children[0], owner)?;
        let args_indices = ast
            .get_multi_child_slice(children[1])
            .expect("Invalid call args index");
        let mut args = Vec::new();
        let mut optional_args = Vec::new();
        for &arg in args_indices {
            let Some((kind, span, children_of_arg)) = ast.get_node(arg) else {
                return Err(LowerError::InternalError(
                    "Invalid call argument node".into(),
                ));
            };

            match kind {
                NodeKind::PropertyAssignment => {
                    let id = ast
                        .source_content(children_of_arg[0], &self.hir.source_map())
                        .expect("Invalid property ID");
                    let value = self.lower_expr(ast, children_of_arg[1], owner)?;
                    optional_args.push(Property {
                        name: self.hir.intern_str(&id),
                        value: self.hir.intern_expr(value),
                    });
                }
                _ => {
                    args.push(self.lower_expr(ast, arg, owner)?);
                }
            }
        }
        Ok(Expr::NormalFormFnApply {
            callee: self.hir.intern_expr(callee),
            args: self.hir.intern_exprs(args),
            optional_args: self.hir.intern_properties(optional_args),
        })
    }

    fn lower_object_call(
        &self,
        ast: &Ast,
        node_index: ast::NodeIndex,
        owner: &Item<'hir>,
        children: &[ast::NodeIndex],
    ) -> LoweringResult<Expr<'hir>> {
        let callee = self.lower_expr(ast, children[0], owner)?;
        let args_indices = ast
            .get_multi_child_slice(children[1])
            .expect("Invalid call args index");
        let mut elements = Vec::new();
        let mut properties = Vec::new();
        for &arg in args_indices {
            let Some((kind, span, children_of_arg)) = ast.get_node(arg) else {
                return Err(LowerError::InternalError(
                    "Invalid call argument node".into(),
                ));
            };

            match kind {
                NodeKind::Property => {
                    let id = ast
                        .source_content(children_of_arg[0], &self.hir.source_map())
                        .expect("Invalid property ID");
                    let value = self.lower_expr(ast, children_of_arg[1], owner)?;
                    properties.push(Property {
                        name: self.hir.intern_str(&id),
                        value: self.hir.intern_expr(value),
                    });
                }
                _ => {
                    elements.push(self.lower_expr(ast, arg, owner)?);
                }
            }
        }
        Ok(Expr::FnObjectApply {
            callee: self.hir.intern_expr(callee),
            elements: self.hir.intern_exprs(elements),
            properties: self.hir.intern_properties(properties),
        })
    }

    fn lower_block(
        &self,
        ast: &Ast,
        node_index: ast::NodeIndex,
        owner: &Item<'hir>,
        children: &[ast::NodeIndex],
        kind: BlockKind,
    ) -> LoweringResult<Expr<'hir>> {
        let actual_children = ast
            .get_multi_child_slice(children[0])
            .expect("Invalid Block children");
        let mut exprs = Vec::with_capacity(actual_children.len());
        for &child in actual_children {
            let expr = self.lower_expr(ast, child, owner)?;
            exprs.push(expr);
        }
        Ok(Expr::Block(kind, self.hir.intern_exprs(exprs)))
    }

    fn lower_let_decl(
        &self,
        ast: &Ast,
        node_index: ast::NodeIndex,
        owner: &Item<'hir>,
        children: &[ast::NodeIndex],
    ) -> LoweringResult<Expr<'hir>> {
        self.assert_kind(ast, node_index, NodeKind::LetDecl);
        let (pattern, variables) = self.lower_pattern(ast, children[0], owner)?;
        let ty = if children[1] != 0 {
            self.hir
                .intern_expr(self.lower_expr(ast, children[1], owner)?)
        } else {
            self.hir.singleton.undefined_
        };
        let init = self.lower_expr(ast, children[2], owner)?;

        // 将模式中的变量添加到当前作用域
        let scope_id = owner.scope_id.expect("Invalid owner scope ID");
        for variable in variables {
            let var_hir_id = self.hir.put(crate::hir::HirMapping::Expr(
                self.hir.intern_expr(crate::hir::Expr::Ref(0)), // 临时占位符
                0,
            ));
            let item = crate::context::scope::Item::new(variable, var_hir_id, None);
            if let Err(e) = self.ctx.scope_manager.add_item(item, scope_id) {
                // 对于有序作用域，允许重复变量（如函数参数块）
                // 对于无序作用域，报告错误
                match e {
                    crate::context::scope::ScopeError::DuplicateSymbol(_) => {
                        // 在当前实现中，我们允许重复绑定（shadowing）
                        // 这是很多语言的标准行为
                    }
                    _ => {
                        return Err(LowerError::ScopeError(format!(
                            "Failed to add variable to scope: {:?}",
                            e
                        )));
                    }
                }
            }
        }

        Ok(Expr::Let {
            pattern: self.hir.intern_pattern(pattern),
            ty: ty,
            init: self.hir.intern_expr(init),
        })
    }

    fn lower_const_decl(
        &self,
        ast: &Ast,
        node_index: ast::NodeIndex,
        owner: &Item<'hir>,
        children: &[ast::NodeIndex],
    ) -> LoweringResult<Expr<'hir>> {
        self.assert_kind(ast, node_index, NodeKind::ConstDecl);
        let (pattern, variables) = self.lower_pattern(ast, children[0], owner)?;
        let ty = if children[1] != 0 {
            self.hir
                .intern_expr(self.lower_expr(ast, children[1], owner)?)
        } else {
            self.hir.singleton.undefined_
        };
        let init = self.lower_expr(ast, children[2], owner)?;

        // 将模式中的变量添加到当前作用域
        let scope_id = owner.scope_id.expect("Invalid owner scope ID");
        for variable in variables {
            let var_hir_id = self.hir.put(crate::hir::HirMapping::Expr(
                self.hir.intern_expr(crate::hir::Expr::Ref(0)), // 临时占位符
                0,
            ));
            let item = crate::context::scope::Item::new(variable, var_hir_id, None);
            if let Err(e) = self.ctx.scope_manager.add_item(item, scope_id) {
                match e {
                    crate::context::scope::ScopeError::DuplicateSymbol(_) => {
                        // 允许重复绑定（shadowing）
                    }
                    _ => {
                        return Err(LowerError::ScopeError(format!(
                            "Failed to add variable to scope: {:?}",
                            e
                        )));
                    }
                }
            }
        }

        Ok(Expr::Const {
            pattern: self.hir.intern_pattern(pattern),
            ty: ty,
            init: self.hir.intern_expr(init),
        })
    }

    fn lower_if_statement(
        &self,
        ast: &Ast,
        node_index: ast::NodeIndex,
        owner: &Item<'hir>,
        children: &[ast::NodeIndex],
    ) -> LoweringResult<Expr<'hir>> {
        self.assert_kind(ast, node_index, NodeKind::IfStatement);
        let condition = self.lower_expr(ast, children[0], owner)?;
        let then_branch = self.lower_block(ast, children[1], owner, children, BlockKind::Normal)?;
        let else_branch = if children[2] != 0 {
            let Some((kind, span, else_children)) = ast.get_node(children[2]) else {
                return Err(LowerError::InternalError("Invalid else branch".into()));
            };
            match kind {
                NodeKind::IfStatement => {
                    Some(self.lower_if_statement(ast, children[2], owner, else_children)?)
                }
                // NodeKind::IfIsMatch
                _ => Some(self.lower_expr(ast, children[2], owner)?),
            }
        } else {
            None
        };
        Ok(Expr::If {
            condition: self.hir.intern_expr(condition),
            then_branch: self.hir.intern_expr(then_branch),
            else_branch: else_branch.map(|branch| self.hir.intern_expr(branch)),
        })
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

fn parse_real_literal<'hir, 'a>(
    src: &'a str,
    ast: &Ast,
    ast_node_index: ast::NodeIndex,
) -> LoweringResult<(i32, u32)> {
    let parts: Vec<&str> = src.split('.').collect();
    if parts.len() != 2 {
        return Err(LowerError::LiteralError {
            message: format!("Invalid real literal: `{}`", src),
            span: ast.get_span(ast_node_index).unwrap_or(rustc_span::DUMMY_SP),
        });
    }
    let (decimal, numeric) = (parts[0], parts[1]);
    let decimal = decimal
        .parse::<i32>()
        .map_err(|_| LowerError::LiteralError {
            message: format!("Invalid decimal part: `{}`", decimal),
            span: ast.get_span(ast_node_index).unwrap_or(rustc_span::DUMMY_SP),
        })?;
    let numeric = numeric
        .parse::<u32>()
        .map_err(|_| LowerError::LiteralError {
            message: format!("Invalid numeric part: `{}`", numeric),
            span: ast.get_span(ast_node_index).unwrap_or(rustc_span::DUMMY_SP),
        })?;
    Ok((decimal, numeric))
}
