//! Expression lowering — AST expression nodes → HIR [`Expr`].

use ast::{NodeIndex, NodeKind};
use hir::{
    common::{BinOp, Ident, Lit, LitKind, Symbol, UnOp},
    expr::{Arg, Block, ClosureParam, Expr, ExprKind, FieldExpr, LetStmt, Stmt, StmtKind},
    body::Body,
};
use rustc_span::Span;

use crate::LoweringContext;

impl<'hir, 'ast> LoweringContext<'hir, 'ast> {
    /// Lower an AST node in expression position into an HIR [`Expr`].
    pub fn lower_expr(&mut self, node: NodeIndex) -> Expr<'hir> {
        if node == 0 {
            return self.make_invalid_expr(Span::default());
        }

        let Some(kind) = self.ast.get_node_kind(node) else {
            return self.make_invalid_expr(Span::default());
        };
        let span = self.ast.get_span(node).unwrap_or(Span::default());

        match kind {
            // ── Literals ─────────────────────────────────────────────────
            NodeKind::Int => {
                let text = self.source_text(node);
                let val = text.replace("_", "").parse::<i64>().unwrap_or(0);
                self.make_lit_expr(LitKind::Integer(val), span)
            }
            NodeKind::Real => {
                let text = self.source_text(node);
                let val = text.replace("_", "").parse::<f64>().unwrap_or(0.0);
                self.make_lit_expr(LitKind::Float(val), span)
            }
            NodeKind::Str => {
                let text = self.source_text(node);
                // Strip surrounding quotes if present
                let inner = text
                    .strip_prefix('"')
                    .and_then(|s| s.strip_suffix('"'))
                    .unwrap_or(&text);
                self.make_lit_expr(LitKind::String(inner.to_string()), span)
            }
            NodeKind::Char => {
                let text = self.source_text(node);
                let ch = text
                    .strip_prefix('\'')
                    .and_then(|s| s.strip_suffix('\''))
                    .and_then(|s| s.chars().next())
                    .unwrap_or('\0');
                self.make_lit_expr(LitKind::Char(ch), span)
            }
            NodeKind::Bool => {
                let text = self.source_text(node);
                let val = text == "true";
                self.make_lit_expr(LitKind::Bool(val), span)
            }
            NodeKind::Symbol => {
                let text = self.source_text(node);
                self.make_lit_expr(LitKind::Symbol(Symbol::intern(&text)), span)
            }
            NodeKind::Unit => self.make_lit_expr(LitKind::Bool(false), span), // placeholder
            NodeKind::Null => Expr {
                hir_id: self.next_hir_id(),
                kind: ExprKind::Null,
                span,
            },
            NodeKind::Undefined => Expr {
                hir_id: self.next_hir_id(),
                kind: ExprKind::Undefined,
                span,
            },

            // ── Identifiers / paths ──────────────────────────────────────
            NodeKind::Id | NodeKind::SelfLower | NodeKind::SelfCap => {
                let path = self.lower_expr_as_path(node);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Path(path),
                    span,
                }
            }
            NodeKind::Select => {
                let path = self.lower_path_from_select(node);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Path(path),
                    span,
                }
            }

            // ── Wildcard (type placeholder) ──────────────────────────────
            NodeKind::Wildcard => Expr {
                hir_id: self.next_hir_id(),
                kind: ExprKind::TyPlaceholder,
                span,
            },

            // ── Binary operators ─────────────────────────────────────────
            NodeKind::Add | NodeKind::Sub | NodeKind::Mul | NodeKind::Div | NodeKind::Mod
            | NodeKind::BoolEq | NodeKind::BoolNotEq | NodeKind::BoolAnd | NodeKind::BoolOr
            | NodeKind::BoolGt | NodeKind::BoolGtEq | NodeKind::BoolLt | NodeKind::BoolLtEq => {
                let children = self.ast.get_children(node);
                if children.len() >= 2 {
                    let op = self.lower_binop(kind);
                    let lhs = self.lower_expr(children[0]);
                    let rhs = self.lower_expr(children[1]);
                    let lhs_ref = self.arena.alloc_expr(lhs);
                    let rhs_ref = self.arena.alloc_expr(rhs);
                    Expr {
                        hir_id: self.next_hir_id(),
                        kind: ExprKind::Binary(op, lhs_ref, rhs_ref),
                        span,
                    }
                } else {
                    self.make_invalid_expr(span)
                }
            }

            // ── Unary operators ──────────────────────────────────────────
            NodeKind::Negative => {
                let children = self.ast.get_children(node);
                if !children.is_empty() {
                    let inner = self.lower_expr(children[0]);
                    let inner_ref = self.arena.alloc_expr(inner);
                    Expr {
                        hir_id: self.next_hir_id(),
                        kind: ExprKind::Unary(UnOp::Neg, inner_ref),
                        span,
                    }
                } else {
                    self.make_invalid_expr(span)
                }
            }
            NodeKind::BoolNot => {
                let children = self.ast.get_children(node);
                if !children.is_empty() {
                    let inner = self.lower_expr(children[0]);
                    let inner_ref = self.arena.alloc_expr(inner);
                    Expr {
                        hir_id: self.next_hir_id(),
                        kind: ExprKind::Unary(UnOp::Not, inner_ref),
                        span,
                    }
                } else {
                    self.make_invalid_expr(span)
                }
            }

            // ── Refer / Deref ────────────────────────────────────────────
            NodeKind::Refer => {
                let children = self.ast.get_children(node);
                if !children.is_empty() {
                    let inner = self.lower_expr(children[0]);
                    let inner_ref = self.arena.alloc_expr(inner);
                    Expr {
                        hir_id: self.next_hir_id(),
                        kind: ExprKind::Ref(inner_ref),
                        span,
                    }
                } else {
                    self.make_invalid_expr(span)
                }
            }
            NodeKind::Deref => {
                let children = self.ast.get_children(node);
                if !children.is_empty() {
                    let inner = self.lower_expr(children[0]);
                    let inner_ref = self.arena.alloc_expr(inner);
                    Expr {
                        hir_id: self.next_hir_id(),
                        kind: ExprKind::Deref(inner_ref),
                        span,
                    }
                } else {
                    self.make_invalid_expr(span)
                }
            }

            // ── ErrorNew ─────────────────────────────────────────────────
            NodeKind::ErrorNew => {
                let children = self.ast.get_children(node);
                if !children.is_empty() {
                    let inner = self.lower_expr(children[0]);
                    let inner_ref = self.arena.alloc_expr(inner);
                    Expr {
                        hir_id: self.next_hir_id(),
                        kind: ExprKind::ErrorNew(inner_ref),
                        span,
                    }
                } else {
                    self.make_invalid_expr(span)
                }
            }

            // ── Application (function call) ──────────────────────────────
            NodeKind::Application => {
                let children = self.ast.get_children(node);
                if children.len() >= 2 {
                    let callee = self.lower_expr(children[0]);
                    let callee_ref = self.arena.alloc_expr(callee);
                    let args_node = children[1];
                    let arg_nodes = self
                        .ast
                        .get_multi_child_slice(args_node)
                        .unwrap_or(&[]);
                    let args = self.lower_call_args(arg_nodes);
                    let args_slice = self.arena.alloc_expr_slice(args);
                    Expr {
                        hir_id: self.next_hir_id(),
                        kind: ExprKind::Call(
                            callee_ref,
                            // Wrap positional args
                            self.positional_args(args_slice),
                        ),
                        span,
                    }
                } else {
                    self.make_invalid_expr(span)
                }
            }

            // ── Extended application (e.g. `Struct { ... }`) ─────────────
            NodeKind::ExtendedApplication => {
                let children = self.ast.get_children(node);
                if children.len() >= 2 {
                    let callee = children[0];
                    let path = self.lower_expr_as_path(callee);
                    let fields_node = children[1];
                    let field_nodes = self
                        .ast
                        .get_multi_child_slice(fields_node)
                        .unwrap_or(&[]);
                    let fields = self.lower_field_exprs(field_nodes);
                    let fields_slice = self.arena.alloc_field_expr_slice(fields);
                    Expr {
                        hir_id: self.next_hir_id(),
                        kind: ExprKind::StructLit(path, fields_slice),
                        span,
                    }
                } else {
                    self.make_invalid_expr(span)
                }
            }

            // ── NormalFormApplication (e.g. `f<T>(args)` → treat as path) ──
            NodeKind::NormalFormApplication => {
                let path = self.lower_expr_as_path(node);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Path(path),
                    span,
                }
            }

            // ── Index application `expr[expr]` ──────────────────────────
            NodeKind::IndexApplication => {
                let children = self.ast.get_children(node);
                if children.len() >= 2 {
                    let base = self.lower_expr(children[0]);
                    let index = self.lower_expr(children[1]);
                    let base_ref = self.arena.alloc_expr(base);
                    let index_ref = self.arena.alloc_expr(index);
                    Expr {
                        hir_id: self.next_hir_id(),
                        kind: ExprKind::Index(base_ref, index_ref),
                        span,
                    }
                } else {
                    self.make_invalid_expr(span)
                }
            }

            // ── Tuple ────────────────────────────────────────────────────
            NodeKind::Tuple => {
                let children = self.ast.get_children(node);
                if !children.is_empty() {
                    let elems_node = children[0];
                    let elem_nodes = self
                        .ast
                        .get_multi_child_slice(elems_node)
                        .unwrap_or(&[]);
                    let elems: Vec<_> = elem_nodes
                        .iter()
                        .map(|&n| self.lower_expr(n))
                        .collect();
                    let elems_slice = self.arena.alloc_expr_slice(elems);
                    Expr {
                        hir_id: self.next_hir_id(),
                        kind: ExprKind::Tuple(elems_slice),
                        span,
                    }
                } else {
                    Expr {
                        hir_id: self.next_hir_id(),
                        kind: ExprKind::Tuple(&[]),
                        span,
                    }
                }
            }

            // ── ListOf (array literal) ───────────────────────────────────
            NodeKind::ListOf => {
                let children = self.ast.get_children(node);
                if !children.is_empty() {
                    let elems_node = children[0];
                    let elem_nodes = self
                        .ast
                        .get_multi_child_slice(elems_node)
                        .unwrap_or(&[]);
                    let elems: Vec<_> = elem_nodes
                        .iter()
                        .map(|&n| self.lower_expr(n))
                        .collect();
                    let elems_slice = self.arena.alloc_expr_slice(elems);
                    Expr {
                        hir_id: self.next_hir_id(),
                        kind: ExprKind::Array(elems_slice),
                        span,
                    }
                } else {
                    Expr {
                        hir_id: self.next_hir_id(),
                        kind: ExprKind::Array(&[]),
                        span,
                    }
                }
            }

            // ── Block ────────────────────────────────────────────────────
            NodeKind::Block | NodeKind::DoBlock | NodeKind::UnsafeBlock
            | NodeKind::AsyncBlock | NodeKind::ComptimeBlock => {
                let block = self.lower_block(node);
                let block_ref = self.arena.alloc_block(block);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Block(block_ref),
                    span,
                }
            }

            // ── If statement / expression ────────────────────────────────
            NodeKind::IfStatement => {
                let children = self.ast.get_children(node);
                if children.len() >= 2 {
                    let cond = self.lower_expr(children[0]);
                    let cond_ref = self.arena.alloc_expr(cond);
                    let then_block = self.lower_block(children[1]);
                    let then_ref = self.arena.alloc_block(then_block);
                    let else_expr = if children.len() >= 3 && children[2] != 0 {
                        let e = self.lower_expr(children[2]);
                        Some(self.arena.alloc_expr(e) as &_)
                    } else {
                        None
                    };
                    Expr {
                        hir_id: self.next_hir_id(),
                        kind: ExprKind::If(cond_ref, then_ref, else_expr),
                        span,
                    }
                } else {
                    self.make_invalid_expr(span)
                }
            }

            // ── Match expression ─────────────────────────────────────────
            NodeKind::PostMatch => {
                let children = self.ast.get_children(node);
                if children.len() >= 2 {
                    let scrutinee = self.lower_expr(children[0]);
                    let scrutinee_ref = self.arena.alloc_expr(scrutinee);
                    let arms_node = children[1];
                    let arm_nodes = self
                        .ast
                        .get_multi_child_slice(arms_node)
                        .unwrap_or(&[]);
                    let arms: Vec<_> = arm_nodes
                        .iter()
                        .map(|&n| self.lower_match_arm(n))
                        .collect();
                    let arms_slice = self.arena.alloc_arm_slice(arms);
                    Expr {
                        hir_id: self.next_hir_id(),
                        kind: ExprKind::Match(scrutinee_ref, arms_slice),
                        span,
                    }
                } else {
                    self.make_invalid_expr(span)
                }
            }

            // ── Assign ──────────────────────────────────────────────────
            NodeKind::Assign => {
                let children = self.ast.get_children(node);
                if children.len() >= 2 {
                    let lhs = self.lower_expr(children[0]);
                    let rhs = self.lower_expr(children[1]);
                    let lhs_ref = self.arena.alloc_expr(lhs);
                    let rhs_ref = self.arena.alloc_expr(rhs);
                    Expr {
                        hir_id: self.next_hir_id(),
                        kind: ExprKind::Assign(lhs_ref, rhs_ref),
                        span,
                    }
                } else {
                    self.make_invalid_expr(span)
                }
            }

            // ── Compound assignment ──────────────────────────────────────
            NodeKind::AddAssign | NodeKind::SubAssign | NodeKind::MulAssign
            | NodeKind::DivAssign => {
                let children = self.ast.get_children(node);
                if children.len() >= 2 {
                    let op = match kind {
                        NodeKind::AddAssign => BinOp::Add,
                        NodeKind::SubAssign => BinOp::Sub,
                        NodeKind::MulAssign => BinOp::Mul,
                        NodeKind::DivAssign => BinOp::Div,
                        _ => unreachable!(),
                    };
                    let lhs = self.lower_expr(children[0]);
                    let rhs = self.lower_expr(children[1]);
                    let lhs_ref = self.arena.alloc_expr(lhs);
                    let rhs_ref = self.arena.alloc_expr(rhs);
                    Expr {
                        hir_id: self.next_hir_id(),
                        kind: ExprKind::AssignOp(op, lhs_ref, rhs_ref),
                        span,
                    }
                } else {
                    self.make_invalid_expr(span)
                }
            }

            // ── Return / Break / Continue ────────────────────────────────
            NodeKind::ReturnStatement => {
                let children = self.ast.get_children(node);
                let val = if !children.is_empty() && children[0] != 0 {
                    let e = self.lower_expr(children[0]);
                    Some(self.arena.alloc_expr(e) as &_)
                } else {
                    None
                };
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Return(val),
                    span,
                }
            }
            NodeKind::BreakStatement => {
                let children = self.ast.get_children(node);
                let label = if !children.is_empty() && children[0] != 0 {
                    self.node_to_ident(children[0])
                } else {
                    Ident::new(Symbol::intern(""), span)
                };
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Break(label),
                    span,
                }
            }
            NodeKind::ContinueStatement => {
                let children = self.ast.get_children(node);
                let label = if !children.is_empty() && children[0] != 0 {
                    self.node_to_ident(children[0])
                } else {
                    Ident::new(Symbol::intern(""), span)
                };
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Continue(label),
                    span,
                }
            }

            // ── TypeCast `expr as type` ──────────────────────────────────
            NodeKind::TypeCast => {
                let children = self.ast.get_children(node);
                if children.len() >= 2 {
                    let expr = self.lower_expr(children[0]);
                    let ty = self.lower_expr(children[1]);
                    let expr_ref = self.arena.alloc_expr(expr);
                    let ty_ref = self.arena.alloc_expr(ty);
                    Expr {
                        hir_id: self.next_hir_id(),
                        kind: ExprKind::Cast(expr_ref, ty_ref),
                        span,
                    }
                } else {
                    self.make_invalid_expr(span)
                }
            }

            // ── Lambda ───────────────────────────────────────────────────
            NodeKind::Lambda => {
                self.lower_lambda_expr(node, span)
            }

            // ── PostLambda (trailing closure) ────────────────────────────
            NodeKind::PostLambda => {
                let children = self.ast.get_children(node);
                if children.len() >= 2 {
                    // Lower as a call with the lambda as the last argument
                    let callee = self.lower_expr(children[0]);
                    let lambda = self.lower_expr(children[1]);
                    let callee_ref = self.arena.alloc_expr(callee);
                    let lambda_ref = self.arena.alloc_expr(lambda);
                    let arg = Arg::Positional(lambda_ref);
                    let args = vec![arg];
                    let args_slice: &[Arg<'hir>] = unsafe {
                        // We need to allocate Arg slices — for now, use a simple
                        // vec-to-slice through the expression arena pattern.
                        // TODO: add an Arg arena to HirArena
                        std::mem::transmute::<&[Arg<'_>], &'hir [Arg<'hir>]>(
                            Vec::leak(args)
                        )
                    };
                    Expr {
                        hir_id: self.next_hir_id(),
                        kind: ExprKind::Call(callee_ref, args_slice),
                        span,
                    }
                } else {
                    self.make_invalid_expr(span)
                }
            }

            // ── Type expressions (types are first-class in Flurry) ───────
            NodeKind::PointerType => {
                let children = self.ast.get_children(node);
                if !children.is_empty() {
                    let inner = self.lower_expr(children[0]);
                    let inner_ref = self.arena.alloc_expr(inner);
                    Expr {
                        hir_id: self.next_hir_id(),
                        kind: ExprKind::TyPtr(inner_ref),
                        span,
                    }
                } else {
                    self.make_invalid_expr(span)
                }
            }
            NodeKind::OptionalType => {
                let children = self.ast.get_children(node);
                if !children.is_empty() {
                    let inner = self.lower_expr(children[0]);
                    let inner_ref = self.arena.alloc_expr(inner);
                    Expr {
                        hir_id: self.next_hir_id(),
                        kind: ExprKind::TyOptional(inner_ref),
                        span,
                    }
                } else {
                    self.make_invalid_expr(span)
                }
            }
            NodeKind::FnType => {
                self.lower_fn_type_expr(node, span)
            }

            // ── Arrow type (`A -> B`, used in type signatures) ───────────
            NodeKind::Arrow => {
                let children = self.ast.get_children(node);
                if children.len() >= 2 {
                    let input = self.lower_expr(children[0]);
                    let output = self.lower_expr(children[1]);
                    let inputs = self.arena.alloc_expr_slice(vec![input]);
                    let output_ref = self.arena.alloc_expr(output);
                    Expr {
                        hir_id: self.next_hir_id(),
                        kind: ExprKind::TyFn(inputs, output_ref),
                        span,
                    }
                } else {
                    self.make_invalid_expr(span)
                }
            }

            // ── Fallback for expression-statements ───────────────────────
            NodeKind::ExprStatement | NodeKind::InlineStatement => {
                let children = self.ast.get_children(node);
                if !children.is_empty() {
                    self.lower_expr(children[0])
                } else {
                    self.make_invalid_expr(span)
                }
            }

            // ── Anything else: emit an error and produce an invalid node ─
            other => {
                self.emit_unsupported_node(&format!("{:?}", other), span);
                self.make_invalid_expr(span)
            }
        }
    }

    // ── Block lowering ───────────────────────────────────────────────────────

    /// Lower an AST block (or block-like) node into an HIR [`Block`].
    pub fn lower_block(&mut self, node: NodeIndex) -> Block<'hir> {
        let span = self.ast.get_span(node).unwrap_or(Span::default());
        let kind = self.ast.get_node_kind(node);

        // For single-child block variants (DoBlock, AsyncBlock, etc.),
        // the child is the actual Block node.
        let block_node = match kind {
            Some(
                NodeKind::DoBlock | NodeKind::AsyncBlock | NodeKind::UnsafeBlock
                | NodeKind::ComptimeBlock,
            ) => {
                let children = self.ast.get_children(node);
                if !children.is_empty() {
                    children[0]
                } else {
                    node
                }
            }
            _ => node,
        };

        let block_kind = self.ast.get_node_kind(block_node);

        match block_kind {
            Some(NodeKind::Block | NodeKind::FileScope) => {
                let children = self.ast.get_children(block_node);
                if !children.is_empty() {
                    let elems_node = children[0];
                    let elem_nodes = self
                        .ast
                        .get_multi_child_slice(elems_node)
                        .unwrap_or(&[]);
                    self.lower_stmts_to_block(elem_nodes, span)
                } else {
                    Block {
                        hir_id: self.next_hir_id(),
                        stmts: &[],
                        expr: None,
                        span,
                    }
                }
            }
            _ => {
                // Single expression treated as a block with a trailing expr
                let expr = self.lower_expr(block_node);
                let expr_ref = self.arena.alloc_expr(expr);
                Block {
                    hir_id: self.next_hir_id(),
                    stmts: &[],
                    expr: Some(expr_ref),
                    span,
                }
            }
        }
    }

    /// Lower a list of statement-level nodes into a Block.
    fn lower_stmts_to_block(
        &mut self,
        stmt_nodes: &[NodeIndex],
        span: Span,
    ) -> Block<'hir> {
        let mut stmts = Vec::new();
        let mut trailing_expr: Option<&'hir Expr<'hir>> = None;

        for (i, &stmt_node) in stmt_nodes.iter().enumerate() {
            if stmt_node == 0 {
                continue;
            }
            let is_last = i == stmt_nodes.len() - 1;
            let kind = self.ast.get_node_kind(stmt_node);

            match kind {
                // Item definitions → StmtKind::Item
                Some(
                    NodeKind::Function
                    | NodeKind::StructDef
                    | NodeKind::EnumDef
                    | NodeKind::TraitDef
                    | NodeKind::ImplDef
                    | NodeKind::ImplTraitDef
                    | NodeKind::TypealiasDef
                    | NodeKind::ModuleDef
                    | NodeKind::NormalFormDef
                    | NodeKind::AlgebraicEffect
                    | NodeKind::UnionDef
                    | NodeKind::ExtendDef
                    | NodeKind::ExtendTraitDef
                    | NodeKind::CaseDef
                    | NodeKind::NewtypeDef
                    | NodeKind::ConstDef
                    | NodeKind::TestDef,
                ) => {
                    let owner_id = self.lower_item_in_block(stmt_node);
                    let stmt_span = self.ast.get_span(stmt_node).unwrap_or(span);
                    let stmt = Stmt {
                        hir_id: self.next_hir_id(),
                        kind: StmtKind::Item(owner_id),
                        span: stmt_span,
                    };
                    stmts.push(stmt);
                }

                // Let / Const declarations → StmtKind::Let
                Some(NodeKind::LetDecl | NodeKind::ConstDecl) => {
                    let let_stmt = self.lower_let_stmt(stmt_node);
                    let let_ref = self.arena.alloc_let_stmt(let_stmt);
                    let stmt_span = self.ast.get_span(stmt_node).unwrap_or(span);
                    let stmt = Stmt {
                        hir_id: self.next_hir_id(),
                        kind: StmtKind::Let(let_ref),
                        span: stmt_span,
                    };
                    stmts.push(stmt);
                }

                // Use/mod statements → StmtKind::Item (simplified)
                Some(NodeKind::UseStatement | NodeKind::ModStatement) => {
                    let owner_id = self.lower_item_in_block(stmt_node);
                    let stmt_span = self.ast.get_span(stmt_node).unwrap_or(span);
                    let stmt = Stmt {
                        hir_id: self.next_hir_id(),
                        kind: StmtKind::Item(owner_id),
                        span: stmt_span,
                    };
                    stmts.push(stmt);
                }

                // Expression statements
                Some(NodeKind::ExprStatement) => {
                    let children = self.ast.get_children(stmt_node);
                    if !children.is_empty() {
                        let expr = self.lower_expr(children[0]);
                        let expr_ref = self.arena.alloc_expr(expr);
                        let stmt_span = self.ast.get_span(stmt_node).unwrap_or(span);
                        let stmt = Stmt {
                            hir_id: self.next_hir_id(),
                            kind: StmtKind::Semi(expr_ref),
                            span: stmt_span,
                        };
                        stmts.push(stmt);
                    }
                }

                // Attributes wrapping definitions
                Some(NodeKind::Attribute | NodeKind::AttributeSetTrue) => {
                    // Lower the inner definition
                    let children = self.ast.get_children(stmt_node);
                    if children.len() >= 2 {
                        // children[0] = attribute expr, children[1] = definition
                        let def_node = children[1];
                        let def_kind = self.ast.get_node_kind(def_node);
                        if matches!(
                            def_kind,
                            Some(
                                NodeKind::Function
                                    | NodeKind::StructDef
                                    | NodeKind::EnumDef
                                    | NodeKind::TraitDef
                                    | NodeKind::ImplDef
                                    | NodeKind::ImplTraitDef
                                    | NodeKind::TypealiasDef
                            )
                        ) {
                            let owner_id = self.lower_item_in_block(def_node);
                            let stmt_span = self.ast.get_span(stmt_node).unwrap_or(span);
                            let stmt = Stmt {
                                hir_id: self.next_hir_id(),
                                kind: StmtKind::Item(owner_id),
                                span: stmt_span,
                            };
                            stmts.push(stmt);
                        } else {
                            let expr = self.lower_expr(def_node);
                            let expr_ref = self.arena.alloc_expr(expr);
                            let stmt_span = self.ast.get_span(stmt_node).unwrap_or(span);
                            let stmt = Stmt {
                                hir_id: self.next_hir_id(),
                                kind: StmtKind::Semi(expr_ref),
                                span: stmt_span,
                            };
                            stmts.push(stmt);
                        }
                    }
                }

                // Last expression (no semicolon) becomes trailing expr
                _ if is_last => {
                    let expr = self.lower_expr(stmt_node);
                    let expr_ref = self.arena.alloc_expr(expr);
                    trailing_expr = Some(expr_ref);
                }

                // Everything else becomes a semi expression
                _ => {
                    let expr = self.lower_expr(stmt_node);
                    let expr_ref = self.arena.alloc_expr(expr);
                    let stmt_span = self.ast.get_span(stmt_node).unwrap_or(span);
                    let stmt = Stmt {
                        hir_id: self.next_hir_id(),
                        kind: StmtKind::Semi(expr_ref),
                        span: stmt_span,
                    };
                    stmts.push(stmt);
                }
            }
        }

        let stmts_slice = self.arena.alloc_stmt_slice(stmts);
        Block {
            hir_id: self.next_hir_id(),
            stmts: stmts_slice,
            expr: trailing_expr,
            span,
        }
    }

    // ── Let statement ────────────────────────────────────────────────────────

    fn lower_let_stmt(&mut self, node: NodeIndex) -> LetStmt<'hir> {
        let span = self.ast.get_span(node).unwrap_or(Span::default());
        let children = self.ast.get_children(node);

        // LetDecl / ConstDecl: a, b, c  (pattern, type, init)
        let pat = if !children.is_empty() && children[0] != 0 {
            self.lower_pattern(children[0])
        } else {
            self.make_error_pattern(span)
        };

        let ty = if children.len() > 1 && children[1] != 0 {
            let ty_expr = self.lower_expr(children[1]);
            Some(self.arena.alloc_expr(ty_expr) as &_)
        } else {
            None
        };

        let init = if children.len() > 2 && children[2] != 0 {
            let init_expr = self.lower_expr(children[2]);
            Some(self.arena.alloc_expr(init_expr) as &_)
        } else {
            None
        };

        LetStmt {
            hir_id: self.next_hir_id(),
            pat,
            ty,
            init,
            span,
        }
    }

    // ── Match arm ────────────────────────────────────────────────────────────

    fn lower_match_arm(&mut self, node: NodeIndex) -> hir::expr::Arm<'hir> {
        let span = self.ast.get_span(node).unwrap_or(Span::default());
        let children = self.ast.get_children(node);

        // CaseArm: a, b  (pattern, body)
        let (pat, guard, body) = if children.len() >= 2 {
            let pat = self.lower_pattern(children[0]);

            // Check if pattern has an IfGuard
            let (final_pat, guard) =
                if self.ast.get_node_kind(children[0]) == Some(NodeKind::IfGuardPattern) {
                    let guard_children = self.ast.get_children(children[0]);
                    if guard_children.len() >= 2 {
                        let inner_pat = self.lower_pattern(guard_children[0]);
                        let guard_expr = self.lower_expr(guard_children[1]);
                        let guard_ref = self.arena.alloc_expr(guard_expr);
                        (inner_pat, Some(guard_ref as &_))
                    } else {
                        (pat, None)
                    }
                } else {
                    (pat, None)
                };

            let body = self.lower_expr(children[1]);
            let body_ref = self.arena.alloc_expr(body);
            (final_pat, guard, body_ref)
        } else {
            let pat = self.make_error_pattern(span);
            let body = self.make_invalid_expr(span);
            let body_ref = self.arena.alloc_expr(body);
            (pat, None, body_ref as &_)
        };

        hir::expr::Arm {
            hir_id: self.next_hir_id(),
            pat,
            guard,
            body,
            span,
        }
    }

    // ── Lambda ───────────────────────────────────────────────────────────────

    fn lower_lambda_expr(&mut self, node: NodeIndex, span: Span) -> Expr<'hir> {
        // Lambda: a, b, N  (return_type, body, params)
        let children = self.ast.get_children(node);
        if children.len() < 3 {
            return self.make_invalid_expr(span);
        }

        let return_type_node = children[0];
        let body_node = children[1];
        let params_multi = children[2];

        let param_nodes = self
            .ast
            .get_multi_child_slice(params_multi)
            .unwrap_or(&[]);

        let closure_params: Vec<ClosureParam<'hir>> = param_nodes
            .iter()
            .map(|&p| self.lower_closure_param(p))
            .collect();
        let params_slice = self.arena.alloc_closure_param_slice(closure_params);

        let ret_ty = if return_type_node != 0 {
            let ty = self.lower_expr(return_type_node);
            Some(self.arena.alloc_expr(ty) as &_)
        } else {
            None
        };

        // Create a body for the closure
        let body_expr = self.lower_expr(body_node);
        let body_expr_ref = self.arena.alloc_expr(body_expr);
        let body = Body {
            params: &[],
            value: body_expr_ref,
        };
        let closure_hir_id = self.next_hir_id();
        let body_id = self.alloc_body(closure_hir_id, body);

        Expr {
            hir_id: closure_hir_id,
            kind: ExprKind::Closure(params_slice, ret_ty, body_id),
            span,
        }
    }

    fn lower_closure_param(&mut self, node: NodeIndex) -> ClosureParam<'hir> {
        let span = self.ast.get_span(node).unwrap_or(Span::default());
        let kind = self.ast.get_node_kind(node);

        match kind {
            Some(NodeKind::TypeBoundParam) => {
                let children = self.ast.get_children(node);
                let pat = if !children.is_empty() {
                    self.lower_pattern(children[0])
                } else {
                    self.make_error_pattern(span)
                };
                let ty = if children.len() > 1 && children[1] != 0 {
                    let ty_expr = self.lower_expr(children[1]);
                    Some(self.arena.alloc_expr(ty_expr) as &_)
                } else {
                    None
                };
                ClosureParam {
                    hir_id: self.next_hir_id(),
                    pat,
                    ty,
                    span,
                }
            }
            _ => {
                let pat = self.lower_pattern(node);
                ClosureParam {
                    hir_id: self.next_hir_id(),
                    pat,
                    ty: None,
                    span,
                }
            }
        }
    }

    // ── FnType expression ────────────────────────────────────────────────────

    fn lower_fn_type_expr(&mut self, node: NodeIndex, span: Span) -> Expr<'hir> {
        // FnType: flags_u32, abi_node, N  (modifier_flags, abi_str_node, parameter_types)
        let children = self.ast.get_children(node);
        if children.len() < 3 {
            return self.make_invalid_expr(span);
        }

        let _flags = children[0]; // raw u32 bitmask
        let _abi_node = children[1];
        let params_multi = children[2];

        let param_nodes = self
            .ast
            .get_multi_child_slice(params_multi)
            .unwrap_or(&[]);

        // The last parameter type is the return type (by convention in fn type)
        if param_nodes.is_empty() {
            return Expr {
                hir_id: self.next_hir_id(),
                kind: ExprKind::TyFn(&[], self.arena.alloc_expr(self.make_invalid_expr(span))),
                span,
            };
        }

        let (param_types, ret_node) = param_nodes.split_at(param_nodes.len().saturating_sub(1));
        let inputs: Vec<_> = param_types.iter().map(|&n| self.lower_expr(n)).collect();
        let inputs_slice = self.arena.alloc_expr_slice(inputs);

        let output = if !ret_node.is_empty() {
            self.lower_expr(ret_node[0])
        } else {
            self.make_invalid_expr(span)
        };
        let output_ref = self.arena.alloc_expr(output);

        Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::TyFn(inputs_slice, output_ref),
            span,
        }
    }

    // ── Helpers ──────────────────────────────────────────────────────────────

    /// Lower call arguments (positional expressions).
    fn lower_call_args(&mut self, arg_nodes: &[NodeIndex]) -> Vec<Expr<'hir>> {
        arg_nodes
            .iter()
            .map(|&n| self.lower_expr(n))
            .collect()
    }

    /// Wrap a slice of expressions as positional `Arg`s.
    ///
    /// This leaks the Vec into a `'hir` slice. TODO: add an Arg arena.
    fn positional_args(&self, exprs: &'hir [Expr<'hir>]) -> &'hir [Arg<'hir>] {
        let args: Vec<Arg<'hir>> = exprs
            .iter()
            .map(|e| Arg::Positional(e))
            .collect();
        // SAFETY: the arena outlives everything; we leak the vec for now.
        unsafe {
            std::mem::transmute::<&[Arg<'_>], &'hir [Arg<'hir>]>(Vec::leak(args))
        }
    }

    /// Lower field expressions (for struct literals).
    fn lower_field_exprs(&mut self, field_nodes: &[NodeIndex]) -> Vec<FieldExpr<'hir>> {
        field_nodes
            .iter()
            .filter_map(|&n| {
                if n == 0 {
                    return None;
                }
                let span = self.ast.get_span(n).unwrap_or(Span::default());
                let kind = self.ast.get_node_kind(n);

                match kind {
                    Some(NodeKind::Property) => {
                        let children = self.ast.get_children(n);
                        if children.len() >= 2 {
                            let ident = self.node_to_ident(children[0]);
                            let expr = self.lower_expr(children[1]);
                            let expr_ref = self.arena.alloc_expr(expr);
                            Some(FieldExpr {
                                ident,
                                expr: expr_ref,
                                span,
                            })
                        } else {
                            None
                        }
                    }
                    _ => {
                        // Shorthand `name` → `name: name`
                        let ident = self.node_to_ident(n);
                        let path = self.lower_expr_as_path(n);
                        let expr = Expr {
                            hir_id: self.next_hir_id(),
                            kind: ExprKind::Path(path),
                            span,
                        };
                        let expr_ref = self.arena.alloc_expr(expr);
                        Some(FieldExpr {
                            ident,
                            expr: expr_ref,
                            span,
                        })
                    }
                }
            })
            .collect()
    }

    /// Convert an AST `NodeKind` to an HIR `BinOp`.
    fn lower_binop(&self, kind: NodeKind) -> BinOp {
        match kind {
            NodeKind::Add => BinOp::Add,
            NodeKind::Sub => BinOp::Sub,
            NodeKind::Mul => BinOp::Mul,
            NodeKind::Div => BinOp::Div,
            NodeKind::Mod => BinOp::Rem,
            NodeKind::BoolEq => BinOp::Eq,
            NodeKind::BoolNotEq => BinOp::Ne,
            NodeKind::BoolGt => BinOp::Gt,
            NodeKind::BoolGtEq => BinOp::Ge,
            NodeKind::BoolLt => BinOp::Lt,
            NodeKind::BoolLtEq => BinOp::Le,
            NodeKind::BoolAnd => BinOp::And,
            NodeKind::BoolOr => BinOp::Or,
            _ => BinOp::Add, // fallback
        }
    }

    /// Create an `Invalid` expression node.
    pub(crate) fn make_invalid_expr(&mut self, span: Span) -> Expr<'hir> {
        Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::Invalid,
            span,
        }
    }

    /// Create a literal expression.
    fn make_lit_expr(&mut self, kind: LitKind, span: Span) -> Expr<'hir> {
        Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::Lit(Lit { kind, span }),
            span,
        }
    }

}
