//! Expression lowering — AST expression nodes → HIR [`Expr`].

use ast::{NodeIndex, NodeKind};
use hir::{
    body::Body,
    common::{
        Arg, BinOp, Ident, Lit, LitKind, Path, PathAnchor, PathSegment, Symbol, TyParam,
        TyParamKind, UnOp,
    },
    decl::LetDecl,
    expr::{Block, ClosureParam, CondictionArm, Expr, ExprKind},
    pattern::{Pattern, PatternArm, PatternKind},
};
use rustc_span::Span;

use crate::LoweringContext;

pub(crate) enum ExprContext {
    // comptime contexts
    Type,
    Trait,
    ClauseParamType,
    FunctionParamType,
    ConstDefValue,
    Attribute,
    Configuration,
    ComptimeFunctionBody,
    ComptimeArgument,
    InlineBranchCondition,
    InlineForLoopIterator,
    ComptimeProposition,

    // runtime contexts
    StructFieldDefault,
    RuntimeFunctionBody,
    RuntimeProposition,
}

impl<'hir, 'ast> LoweringContext<'hir, 'ast> {
    /// Lower an AST node in expression position into an HIR [`Expr`].
    pub fn lower_expr(&mut self, node: NodeIndex) -> Expr<'hir> {
        let Some((kind, span, children)) = self.ast.get_node(node) else {
            unreachable!("invalid expression node: no such node index {:?}", node);
        };

        match kind {
            NodeKind::Id => Expr {
                hir_id: self.next_hir_id(),
                kind: ExprKind::Ident(self.node_to_symbol(node)),
                span,
            },
            NodeKind::SelfLower => Expr {
                hir_id: self.next_hir_id(),
                kind: ExprKind::SelfValue,
                span,
            },
            NodeKind::SelfCap => Expr {
                hir_id: self.next_hir_id(),
                kind: ExprKind::TySelf,
                span,
            },
            NodeKind::Projection => {
                let base_expr = self.lower_expr(children[0]);
                let base_ref = self.arena.alloc_expr(base_expr);
                let field_ident = self.node_to_ident(children[1]);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Projection(base_ref, field_ident),
                    span,
                }
            }
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
                self.make_lit_expr(LitKind::Symbol(self.node_to_symbol(children[0])), span)
            }
            NodeKind::Unit => Expr {
                hir_id: self.next_hir_id(),
                kind: ExprKind::Unit,
                span,
            },
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

            NodeKind::Wildcard => Expr {
                hir_id: self.next_hir_id(),
                kind: ExprKind::TyPlaceholder,
                span,
            },
            NodeKind::Add
            | NodeKind::Sub
            | NodeKind::Mul
            | NodeKind::Div
            | NodeKind::Mod
            | NodeKind::BoolEq
            | NodeKind::BoolNotEq
            | NodeKind::BoolAnd
            | NodeKind::BoolOr
            | NodeKind::BoolGt
            | NodeKind::BoolGtEq
            | NodeKind::BoolLt
            | NodeKind::BoolLtEq => {
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
            }
            NodeKind::Negative => {
                let inner = self.lower_expr(children[0]);
                let inner_ref = self.arena.alloc_expr(inner);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Unary(UnOp::Neg, inner_ref),
                    span,
                }
            }
            NodeKind::BoolNot => {
                let inner = self.lower_expr(children[0]);
                let inner_ref = self.arena.alloc_expr(inner);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Unary(UnOp::Not, inner_ref),
                    span,
                }
            }
            NodeKind::Refer => {
                let inner = self.lower_expr(children[0]);
                let inner_ref = self.arena.alloc_expr(inner);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Ref(inner_ref),
                    span,
                }
            }
            NodeKind::Deref => {
                let inner = self.lower_expr(children[0]);
                let inner_ref = self.arena.alloc_expr(inner);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Deref(inner_ref),
                    span,
                }
            }
            NodeKind::ErrorNew => {
                let inner = self.lower_expr(children[0]);
                let inner_ref = self.arena.alloc_expr(inner);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::ErrorNew(inner_ref),
                    span,
                }
            }
            NodeKind::Application => {
                let callee = self.lower_expr(children[0]);
                let callee_ref = self.arena.alloc_expr(callee);
                let args_node = children[1];
                let arg_nodes = self.ast.get_multi_child_slice(args_node).unwrap_or(&[]);
                let args: Vec<Arg<'hir>> = arg_nodes
                    .iter()
                    .map(|&n| {
                        let e = self.lower_expr(n);
                        Arg::Positional(self.arena.alloc_expr(e))
                    })
                    .collect();
                let args_slice = self.arena.alloc_arg_slice(args);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Application(callee_ref, args_slice),
                    span,
                }
            }
            NodeKind::ExtendedApplication => {
                let callee = self.lower_expr(children[0]);
                let callee_ref = self.arena.alloc_expr(callee);
                let args = self.lower_extend_args(children[1]);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::NFApplication(callee_ref, args),
                    span,
                }
            }
            NodeKind::NormalFormApplication => {
                let base = self.lower_expr(children[0]);
                let base_ref = self.arena.alloc_expr(base);
                let args: &'hir [Arg<'hir>] = self.lower_args(children[1]);

                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::NFApplication(base_ref, args),
                    span,
                }
            }
            NodeKind::IndexApplication => {
                let base = self.lower_expr(children[0]);
                let index = self.lower_expr(children[1]);
                let base_ref = self.arena.alloc_expr(base);
                let index_ref = self.arena.alloc_expr(index);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Index(base_ref, index_ref),
                    span,
                }
            }
            NodeKind::Tuple => {
                let elems_node = children[0];
                let elem_nodes = self.ast.get_multi_child_slice(elems_node).unwrap_or(&[]);
                let elems: Vec<_> = elem_nodes.iter().map(|&n| self.lower_expr(n)).collect();
                let elems_slice = self.arena.alloc_expr_slice(elems);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Tuple(elems_slice),
                    span,
                }
            }
            NodeKind::ListOf => {
                let elems_node = children[0];
                let elem_nodes = self.ast.get_multi_child_slice(elems_node).unwrap_or(&[]);
                let elems: Vec<_> = elem_nodes.iter().map(|&n| self.lower_expr(n)).collect();
                let elems_slice = self.arena.alloc_expr_slice(elems);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::List(elems_slice),
                    span,
                }
            }
            NodeKind::Block
            | NodeKind::DoBlock
            | NodeKind::UnsafeBlock
            | NodeKind::AsyncBlock
            | NodeKind::ComptimeBlock => {
                let block = self.lower_block(node);
                let block_ref = self.arena.alloc_block(block);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Block(block_ref),
                    span,
                }
            }
            NodeKind::IfStatement => {
                let cond = self.lower_expr(children[0]);
                let cond_ref = self.arena.alloc_expr(cond);
                let then_block = self.lower_block(children[1]);
                let then_ref = self.arena.alloc_block(then_block);
                let else_expr = if children[2] != 0 {
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
            }
            NodeKind::PostMatch => {
                let scrutinee = self.lower_expr(children[0]);
                let scrutinee_ref = self.arena.alloc_expr(scrutinee);
                let arms_node = children[1];
                let arm_nodes = self.ast.get_multi_child_slice(arms_node).unwrap_or(&[]);
                let arms: Vec<_> = arm_nodes.iter().map(|&n| self.lower_match_arm(n)).collect();
                let arms_slice = self.arena.alloc_arm_slice(arms);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Match(scrutinee_ref, arms_slice),
                    span,
                }
            }
            NodeKind::Assign => {
                let lhs = self.lower_expr(children[0]);
                let rhs = self.lower_expr(children[1]);
                let lhs_ref = self.arena.alloc_expr(lhs);
                let rhs_ref = self.arena.alloc_expr(rhs);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Assign(lhs_ref, rhs_ref),
                    span,
                }
            }
            NodeKind::AddAssign
            | NodeKind::SubAssign
            | NodeKind::MulAssign
            | NodeKind::DivAssign => {
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
            }
            NodeKind::ReturnStatement => {
                let val = if children[0] != 0 {
                    let e = self.lower_expr(children[0]);
                    Some(self.arena.alloc_expr(e) as &_)
                } else {
                    None
                };
                let return_expr = Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Return(val),
                    span,
                };
                // while guard: `return val while guard`  →  `if guard { return val }`
                if children[1] != 0 {
                    self.wrap_with_guard(children[1], return_expr, span)
                } else {
                    return_expr
                }
            }
            NodeKind::ResumeStatement => {
                let val = if children[0] != 0 {
                    let e = self.lower_expr(children[0]);
                    Some(self.arena.alloc_expr(e) as &_)
                } else {
                    None
                };
                let resume_expr = Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Resume(val),
                    span,
                };
                if children[1] != 0 {
                    self.wrap_with_guard(children[1], resume_expr, span)
                } else {
                    resume_expr
                }
            }
            NodeKind::BreakStatement => {
                let label = if children[0] != 0 {
                    self.node_to_ident(children[0])
                } else {
                    Ident::new(Symbol::invalid(), span)
                };
                let break_expr = Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Break(label),
                    span,
                };
                // while guard: `break :l while guard`  →  `if guard { break :l }`
                if children[1] != 0 {
                    self.wrap_with_guard(children[1], break_expr, span)
                } else {
                    break_expr
                }
            }
            NodeKind::ContinueStatement => {
                let label = if children[0] != 0 {
                    self.node_to_ident(children[0])
                } else {
                    Ident::new(Symbol::invalid(), span)
                };
                let cont_expr = Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Continue(label),
                    span,
                };
                // while guard: `continue :l while guard`  →  `if guard { continue :l }`
                if children[1] != 0 {
                    self.wrap_with_guard(children[1], cont_expr, span)
                } else {
                    cont_expr
                }
            }
            NodeKind::TypeCast => {
                let expr = self.lower_expr(children[0]);
                let ty = self.lower_expr(children[1]);
                let expr_ref = self.arena.alloc_expr(expr);
                let ty_ref = self.arena.alloc_expr(ty);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Cast(expr_ref, ty_ref),
                    span,
                }
            }
            NodeKind::Lambda => self.lower_lambda_expr(node, span),
            NodeKind::PostLambda => {
                // Lower as a call with the lambda as the last argument
                let callee = self.lower_expr(children[0]);
                let lambda = self.lower_expr(children[1]);
                let callee_ref = self.arena.alloc_expr(callee);
                let lambda_ref = self.arena.alloc_expr(lambda);
                let arg = Arg::Positional(lambda_ref);
                let args_slice = self.arena.alloc_arg_slice(vec![arg]);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Application(callee_ref, args_slice),
                    span,
                }
            }
            NodeKind::PointerType => {
                let inner = self.lower_expr(children[0]);
                let inner_ref = self.arena.alloc_expr(inner);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::TyPtr(inner_ref),
                    span,
                }
            }
            NodeKind::OptionalType => {
                let inner = self.lower_expr(children[0]);
                let inner_ref = self.arena.alloc_expr(inner);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::TyOptional(inner_ref),
                    span,
                }
            }
            NodeKind::FnType => self.lower_fn_type_expr(node, span),
            NodeKind::Arrow => {
                let input = self.lower_expr(children[0]);
                let output = self.lower_expr(children[1]);
                let input_ref = self.arena.alloc_expr(input);
                let output_ref = self.arena.alloc_expr(output);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::TyFnArrow(input_ref, output_ref),
                    span,
                }
            }
            NodeKind::ExprStatement | NodeKind::InlineStatement => self.lower_expr(children[0]),

            // ── Desugaring: bool matches ─────────────────────────────────
            // `a matches b`  →  `match a { b => true, _ => false }`
            NodeKind::BoolMatches => {
                let scrutinee = self.lower_expr(children[0]);
                let scrutinee_ref = self.arena.alloc_expr(scrutinee);
                let pat = self.lower_pattern(children[1]);

                let true_expr = self.make_lit_expr(LitKind::Bool(true), span);
                let true_ref = self.arena.alloc_expr(true_expr);
                let arm_true = PatternArm {
                    hir_id: self.next_hir_id(),
                    pat,
                    body: true_ref,
                    span,
                };

                let false_expr = self.make_lit_expr(LitKind::Bool(false), span);
                let false_ref = self.arena.alloc_expr(false_expr);
                let wild_pat = self.make_wild_pattern(span);
                let arm_false = PatternArm {
                    hir_id: self.next_hir_id(),
                    pat: wild_pat,
                    body: false_ref,
                    span,
                };

                let arms_slice = self.arena.alloc_arm_slice([arm_true, arm_false]);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Match(scrutinee_ref, arms_slice),
                    span,
                }
            }

            // ── Desugaring: if-is-match ──────────────────────────────────
            // `if a is b { c } else { d }`  →  `match a { b => c, _ => d }`
            NodeKind::IfIsMatch => {
                let scrutinee = self.lower_expr(children[0]);
                let scrutinee_ref = self.arena.alloc_expr(scrutinee);
                let pat = self.lower_pattern(children[1]);

                let then_expr = self.lower_expr(children[2]);
                let then_ref = self.arena.alloc_expr(then_expr);
                let arm_then = PatternArm {
                    hir_id: self.next_hir_id(),
                    pat,
                    body: then_ref,
                    span,
                };

                let else_expr = if children[3] != 0 {
                    self.lower_expr(children[3])
                } else {
                    Expr {
                        hir_id: self.next_hir_id(),
                        kind: ExprKind::Tuple(&[]),
                        span,
                    }
                };
                let else_ref = self.arena.alloc_expr(else_expr);
                let wild_pat = self.make_wild_pattern(span);
                let arm_else = PatternArm {
                    hir_id: self.next_hir_id(),
                    pat: wild_pat,
                    body: else_ref,
                    span,
                };

                let arms_slice = self.arena.alloc_arm_slice([arm_then, arm_else]);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Match(scrutinee_ref, arms_slice),
                    span,
                }
            }

            // ── Desugaring: if-match (same as post-match) ────────────────
            // `if a match { b => c, … }`  →  `match a { b => c, … }`
            NodeKind::IfMatch => {
                let scrutinee = self.lower_expr(children[0]);
                let scrutinee_ref = self.arena.alloc_expr(scrutinee);
                let arm_nodes = self.ast.get_multi_child_slice(children[1]).unwrap_or(&[]);
                let arms: Vec<_> = arm_nodes.iter().map(|&n| self.lower_match_arm(n)).collect();
                let arms_slice = self.arena.alloc_arm_slice(arms);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Match(scrutinee_ref, arms_slice),
                    span,
                }
            }

            // ── WhenStatement ────────────────────────────────────────────
            // `when { cond1 => body1, cond2 => body2, … }`
            NodeKind::WhenStatement => {
                let arm_nodes: &[NodeIndex] = if children.is_empty() {
                    &[]
                } else {
                    self.ast.get_multi_child_slice(children[0]).unwrap_or(&[])
                };
                let cond_arms: Vec<CondictionArm<'hir>> = arm_nodes
                    .iter()
                    .map(|&n| {
                        let arm_span = self.ast.get_span(n).unwrap_or(span);
                        let ac = self.ast.get_children(n);
                        if ac.len() >= 2 {
                            let cond = self.lower_expr(ac[0]);
                            let body = self.lower_expr(ac[1]);
                            let cond_ref = self.arena.alloc_expr(cond);
                            let body_ref = self.arena.alloc_expr(body);
                            CondictionArm {
                                hir_id: self.next_hir_id(),
                                cond: cond_ref,
                                body: body_ref,
                                span: arm_span,
                            }
                        } else {
                            let inv1 = self.make_invalid_expr(arm_span);
                            let inv2 = self.make_invalid_expr(arm_span);
                            let r1 = self.arena.alloc_expr(inv1);
                            let r2 = self.arena.alloc_expr(inv2);
                            CondictionArm {
                                hir_id: self.next_hir_id(),
                                cond: r1,
                                body: r2,
                                span: arm_span,
                            }
                        }
                    })
                    .collect();
                let arms_slice = self.arena.alloc_cond_arm_slice(cond_arms);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::When(arms_slice),
                    span,
                }
            }

            // ── Desugaring: while ────────────────────────────────────────
            // `while :label? cond { body }`
            //   →  `loop { if !cond { break :label }; body }`
            NodeKind::WhileStatement => {
                let label = if children[0] != 0 {
                    self.node_to_ident(children[0])
                } else {
                    Ident::new(Symbol::invalid(), span)
                };

                // if !cond { break :label }
                let cond = self.lower_expr(children[1]);
                let cond_ref = self.arena.alloc_expr(cond);
                let not_cond = Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Unary(UnOp::Not, cond_ref),
                    span,
                };
                let not_cond_ref = self.arena.alloc_expr(not_cond);
                let break_expr = Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Break(label),
                    span,
                };
                let break_ref = self.arena.alloc_expr(break_expr);
                let guard_then = Block {
                    hir_id: self.next_hir_id(),
                    stmts: &[],
                    expr: Some(break_ref),
                    span,
                };
                let guard_then_ref = self.arena.alloc_block(guard_then);
                let guard_if = Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::If(not_cond_ref, guard_then_ref, None),
                    span,
                };
                let guard_if_ref = self.arena.alloc_expr(guard_if);
                let guard_semi = Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Semi(guard_if_ref),
                    span,
                };

                // body
                let body_block = self.lower_block(children[2]);
                let body_block_ref = self.arena.alloc_block(body_block);
                let body_expr = Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Block(body_block_ref),
                    span,
                };
                let body_expr_ref = self.arena.alloc_expr(body_expr);

                let loop_stmts = self.arena.alloc_expr_slice([guard_semi]);
                let loop_block = Block {
                    hir_id: self.next_hir_id(),
                    stmts: loop_stmts,
                    expr: Some(body_expr_ref),
                    span,
                };
                let loop_block_ref = self.arena.alloc_block(loop_block);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Loop(loop_block_ref),
                    span,
                }
            }

            // ── Desugaring: while-is-match ───────────────────────────────
            // `while :label? scrutinee is pat { body }`
            //   →  `loop { match scrutinee { pat => body, _ => break :label } }`
            NodeKind::WhileIsMatch => {
                let label = if children[0] != 0 {
                    self.node_to_ident(children[0])
                } else {
                    Ident::new(Symbol::invalid(), span)
                };

                let scrutinee = self.lower_expr(children[1]);
                let scrutinee_ref = self.arena.alloc_expr(scrutinee);
                let pat = self.lower_pattern(children[2]);

                let body_expr = self.lower_expr(children[3]);
                let body_ref = self.arena.alloc_expr(body_expr);
                let arm_body = PatternArm {
                    hir_id: self.next_hir_id(),
                    pat,
                    body: body_ref,
                    span,
                };

                let break_expr = Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Break(label),
                    span,
                };
                let break_ref = self.arena.alloc_expr(break_expr);
                let wild_pat = self.make_wild_pattern(span);
                let arm_break = PatternArm {
                    hir_id: self.next_hir_id(),
                    pat: wild_pat,
                    body: break_ref,
                    span,
                };

                let arms_slice = self.arena.alloc_arm_slice([arm_body, arm_break]);
                let match_expr = Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Match(scrutinee_ref, arms_slice),
                    span,
                };
                let match_ref = self.arena.alloc_expr(match_expr);
                let loop_block = Block {
                    hir_id: self.next_hir_id(),
                    stmts: &[],
                    expr: Some(match_ref),
                    span,
                };
                let loop_block_ref = self.arena.alloc_block(loop_block);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Loop(loop_block_ref),
                    span,
                }
            }

            // ── Desugaring: while-match ──────────────────────────────────
            // `while :label? scrutinee match { arms… }`
            //   →  `loop { match scrutinee { arms… } }`
            NodeKind::WhileMatch => {
                let scrutinee = self.lower_expr(children[1]);
                let scrutinee_ref = self.arena.alloc_expr(scrutinee);
                let arm_nodes = self.ast.get_multi_child_slice(children[2]).unwrap_or(&[]);
                let arms: Vec<_> = arm_nodes.iter().map(|&n| self.lower_match_arm(n)).collect();
                let arms_slice = self.arena.alloc_arm_slice(arms);
                let match_expr = Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Match(scrutinee_ref, arms_slice),
                    span,
                };
                let match_ref = self.arena.alloc_expr(match_expr);
                let loop_block = Block {
                    hir_id: self.next_hir_id(),
                    stmts: &[],
                    expr: Some(match_ref),
                    span,
                };
                let loop_block_ref = self.arena.alloc_block(loop_block);
                Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::Loop(loop_block_ref),
                    span,
                }
            }

            // ── Desugaring: for ──────────────────────────────────────────
            // `for :label? pat in iterable { body }`
            //   →  `{ let __iter__ = iterable.__into_iter__(); loop { match __iter__.__next__() { Some(pat) => body, None => break :label } } }`
            // Note: `__into_iter__` and `__next__` are placeholder method names.
            NodeKind::ForStatement => self.lower_for_stmt(node),

            other => {
                self.emit_unsupported_node(&format!("{:?}", other), span);
                self.make_invalid_expr(span)
            }
        }
    }

    /// Lower an AST block (or block-like) node into an HIR [`Block`].
    pub fn lower_block(&mut self, node: NodeIndex) -> Block<'hir> {
        let Some((kind, span, children)) = self.ast.get_node(node) else {
            unreachable!("invalid block node: no such node index {:?}", node);
        };

        // For single-child block variants (DoBlock, AsyncBlock, etc.),
        // the child is the actual Block node.
        let block_node = match kind {
            NodeKind::DoBlock
            | NodeKind::AsyncBlock
            | NodeKind::UnsafeBlock
            | NodeKind::ComptimeBlock => children[0],
            _ => node,
        };

        let block_kind = self.ast.get_node_kind(block_node);

        match block_kind {
            Some(NodeKind::Block | NodeKind::FileScope) => {
                let children = self.ast.get_children(block_node);
                let elems_node = children[0];
                let elem_nodes = self.ast.get_multi_child_slice(elems_node).unwrap_or(&[]);
                self.lower_stmts_to_block(elem_nodes, span)
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
    fn lower_stmts_to_block(&mut self, stmt_nodes: &[NodeIndex], span: Span) -> Block<'hir> {
        let mut stmts: Vec<Expr<'hir>> = Vec::new();
        let mut trailing_expr: Option<&'hir Expr<'hir>> = None;

        for (i, &stmt_node) in stmt_nodes.iter().enumerate() {
            if stmt_node == 0 {
                continue;
            }
            let is_last = i == stmt_nodes.len() - 1;
            let kind = self.ast.get_node_kind(stmt_node);

            match kind {
                // Item definitions → ExprKind::Item
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
                    stmts.push(Expr {
                        hir_id: self.next_hir_id(),
                        kind: ExprKind::Item(owner_id),
                        span: stmt_span,
                    });
                }

                // Let / Const declarations → ExprKind::Let
                Some(NodeKind::LetDecl | NodeKind::ConstDecl) => {
                    let let_decl = self.lower_let_decl(stmt_node);
                    let let_ref = self.arena.alloc_let_decl(let_decl);
                    let stmt_span = self.ast.get_span(stmt_node).unwrap_or(span);
                    stmts.push(Expr {
                        hir_id: self.next_hir_id(),
                        kind: ExprKind::Let(let_ref),
                        span: stmt_span,
                    });
                }

                // Use/mod statements → ExprKind::Item
                Some(NodeKind::UseStatement | NodeKind::ModStatement) => {
                    let owner_id = self.lower_item_in_block(stmt_node);
                    let stmt_span = self.ast.get_span(stmt_node).unwrap_or(span);
                    stmts.push(Expr {
                        hir_id: self.next_hir_id(),
                        kind: ExprKind::Item(owner_id),
                        span: stmt_span,
                    });
                }

                // Expression statements → ExprKind::Semi
                Some(NodeKind::ExprStatement) => {
                    let children = self.ast.get_children(stmt_node);
                    let expr = self.lower_expr(children[0]);
                    let expr_ref = self.arena.alloc_expr(expr);
                    let stmt_span = self.ast.get_span(stmt_node).unwrap_or(span);
                    stmts.push(Expr {
                        hir_id: self.next_hir_id(),
                        kind: ExprKind::Semi(expr_ref),
                        span: stmt_span,
                    });
                }

                // Attributes wrapping definitions
                Some(NodeKind::Attribute | NodeKind::AttributeSetTrue) => {
                    let children = self.ast.get_children(stmt_node);
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
                        stmts.push(Expr {
                            hir_id: self.next_hir_id(),
                            kind: ExprKind::Item(owner_id),
                            span: stmt_span,
                        });
                    } else {
                        let expr = self.lower_expr(def_node);
                        let expr_ref = self.arena.alloc_expr(expr);
                        let stmt_span = self.ast.get_span(stmt_node).unwrap_or(span);
                        stmts.push(Expr {
                            hir_id: self.next_hir_id(),
                            kind: ExprKind::Semi(expr_ref),
                            span: stmt_span,
                        });
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
                    stmts.push(Expr {
                        hir_id: self.next_hir_id(),
                        kind: ExprKind::Semi(expr_ref),
                        span: stmt_span,
                    });
                }
            }
        }

        let stmts_slice = self.arena.alloc_expr_slice(stmts);
        Block {
            hir_id: self.next_hir_id(),
            stmts: stmts_slice,
            expr: trailing_expr,
            span,
        }
    }

    fn lower_for_stmt(&mut self, node: NodeIndex) -> Expr<'hir> {
        let Some((NodeKind::ForStatement, span, children)) = self.ast.get_node(node) else {
            unreachable!(
                "not a for stmt node or invalid for statement node: no such node index {:?}",
                node
            );
        };

        let label = if children[0] != 0 {
            self.node_to_ident(children[0])
        } else {
            Ident::new(Symbol::invalid(), span)
        };
        let pat_node = children[1];
        let iter_node = children[2];
        let body_node = children[3];

        // iterable.__into_iter__()
        let iterable = self.lower_expr(iter_node);
        let iterable_ref = self.arena.alloc_expr(iterable);
        let into_iter_ident = Ident::new(Symbol::intern("__into_iter__"), span);
        let proj = Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::Projection(iterable_ref, into_iter_ident),
            span,
        };
        let proj_ref = self.arena.alloc_expr(proj);
        let no_args = self.arena.alloc_arg_slice([]);
        let into_iter_call = Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::Application(proj_ref, no_args),
            span,
        };
        let into_iter_ref = self.arena.alloc_expr(into_iter_call);

        // let __iter__ = iterable.__into_iter__()
        let iter_ident = Ident::new(Symbol::intern("__iter__"), span);
        let iter_let = LetDecl {
            hir_id: self.next_hir_id(),
            name: iter_ident.clone(),
            ty: None,
            init: Some(into_iter_ref),
            span,
        };
        let iter_let_ref = self.arena.alloc_let_decl(iter_let);
        let let_expr = Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::Let(iter_let_ref),
            span,
        };

        // __iter__.__next__()
        let iter_seg = PathSegment {
            ident: iter_ident,
            args: &[],
        };
        let iter_segs = self.arena.alloc_path_segment_slice([iter_seg]);
        let iter_path = Path {
            anchor: PathAnchor::Local,
            segments: iter_segs,
            span,
            res: None,
        };
        let iter_path_expr = Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::Path(iter_path),
            span,
        };
        let iter_path_ref = self.arena.alloc_expr(iter_path_expr);
        let next_ident = Ident::new(Symbol::intern("__next__"), span);
        let proj_next = Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::Projection(iter_path_ref, next_ident),
            span,
        };
        let proj_next_ref = self.arena.alloc_expr(proj_next);
        let no_args2 = self.arena.alloc_arg_slice([]);
        let next_call = Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::Application(proj_next_ref, no_args2),
            span,
        };
        let next_call_ref = self.arena.alloc_expr(next_call);

        // some? => body
        let loop_pat = self.lower_pattern(pat_node);
        let some_pat = Pattern {
            hir_id: self.next_hir_id(),
            kind: PatternKind::OptionSome(self.arena.alloc_pattern(loop_pat)),
            span,
        };
        let body_block = self.lower_block(body_node);
        let body_block_ref = self.arena.alloc_block(body_block);
        let body_block_expr = Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::Block(body_block_ref),
            span,
        };
        let body_expr_ref = self.arena.alloc_expr(body_block_expr);
        let arm_some = PatternArm {
            hir_id: self.next_hir_id(),
            pat: some_pat,
            body: body_expr_ref,
            span,
        };

        // None => break :label
        let none_pat = Pattern {
            hir_id: self.next_hir_id(),
            kind: PatternKind::OptionNull,
            span,
        };
        let break_expr = Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::Break(label),
            span,
        };
        let break_ref = self.arena.alloc_expr(break_expr);
        let arm_none = PatternArm {
            hir_id: self.next_hir_id(),
            pat: none_pat,
            body: break_ref,
            span,
        };

        let arms_slice = self.arena.alloc_arm_slice([arm_some, arm_none]);
        let match_expr = Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::Match(next_call_ref, arms_slice),
            span,
        };
        let match_ref = self.arena.alloc_expr(match_expr);
        let loop_block = Block {
            hir_id: self.next_hir_id(),
            stmts: &[],
            expr: Some(match_ref),
            span,
        };
        let loop_block_ref = self.arena.alloc_block(loop_block);
        let loop_expr = Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::Loop(loop_block_ref),
            span,
        };
        let loop_ref = self.arena.alloc_expr(loop_expr);

        // Outer block: { let __iter__ = …; loop { … } }
        let let_stmts = self.arena.alloc_expr_slice([let_expr]);
        let outer_block = Block {
            hir_id: self.next_hir_id(),
            stmts: let_stmts,
            expr: Some(loop_ref),
            span,
        };
        let outer_block_ref = self.arena.alloc_block(outer_block);
        Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::Block(outer_block_ref),
            span,
        }
    }

    fn lower_let_decl(&mut self, node: NodeIndex) -> LetDecl<'hir> {
        let Some((NodeKind::LetDecl, span, children)) = self.ast.get_node(node) else {
            unreachable!(
                "not a let declaration node or invalid let/const declaration node: no such node index {:?}",
                node
            );
        };

        // LetDecl / ConstDecl: a, b, c  (pattern/name, type, init)
        // LetDecl.name: Ident — for simple name bindings; complex patterns
        // are not yet representable in LetDecl (see ambiguities table).
        let name = if children[0] != 0 {
            self.node_to_ident(children[0])
        } else {
            Ident::new(Symbol::intern("_"), span)
        };

        let ty = if children[1] != 0 {
            let ty_expr = self.lower_expr(children[1]);
            Some(self.arena.alloc_expr(ty_expr) as &_)
        } else {
            None
        };

        let init = if children[2] != 0 {
            let init_expr = self.lower_expr(children[2]);
            Some(self.arena.alloc_expr(init_expr) as &_)
        } else {
            None
        };

        LetDecl {
            hir_id: self.next_hir_id(),
            name,
            ty,
            init,
            span,
        }
    }

    fn lower_args(&mut self, args_multi: NodeIndex) -> &'hir [Arg<'hir>] {
        let arg_nodes = self.ast.get_multi_child_slice(args_multi).unwrap_or(&[]);
        let args: Vec<Arg<'hir>> = arg_nodes.iter().map(|&n| self.lower_arg(n)).collect();
        self.arena.alloc_arg_slice(args)
    }

    fn lower_arg(&mut self, node: NodeIndex) -> Arg<'hir> {
        let Some((kind, _, children)) = self.ast.get_node(node) else {
            unreachable!("invalid argument node: no such node index {:?}", node);
        };

        match kind {
            NodeKind::ExpandArg => {
                let expr = self.lower_expr(children[0]);
                let expr_ref = self.arena.alloc_expr(expr);
                Arg::Expand(expr_ref)
            }
            NodeKind::OptionalArg => {
                let ident = self.node_to_ident(children[0]);
                let expr = self.lower_expr(children[1]);
                let expr_ref = self.arena.alloc_expr(expr);
                Arg::Named(ident, expr_ref)
            }
            // otherwise we just expect it to be a normal expression
            _ => {
                let expr = self.lower_expr(node);
                let expr_ref = self.arena.alloc_expr(expr);
                Arg::Positional(expr_ref)
            }
        }
    }

    fn lower_match_arm(&mut self, node: NodeIndex) -> PatternArm<'hir> {
        let Some((NodeKind::CaseArm, span, children)) = self.ast.get_node(node) else {
            unreachable!(
                "not a pattern case arm node or invalid pattern arm node: no such node index {:?}",
                node
            );
        };

        // CaseArm: a, b  (pattern, body)
        let (pat, body) = {
            // Strip IfGuard wrapper — guards not yet representable in PatternArm
            let pat_node = children[0];
            let pat = if self.ast.get_node_kind(pat_node) == Some(NodeKind::IfGuardPattern) {
                let guard_children = self.ast.get_children(pat_node);
                if !guard_children.is_empty() {
                    self.lower_pattern(guard_children[0])
                } else {
                    self.lower_pattern(pat_node)
                }
            } else {
                self.lower_pattern(pat_node)
            };
            let body = self.lower_expr(children[1]);
            let body_ref = self.arena.alloc_expr(body);
            (pat, body_ref)
        };

        PatternArm {
            hir_id: self.next_hir_id(),
            pat,
            body,
            span,
        }
    }

    fn lower_lambda_expr(&mut self, node: NodeIndex, span: Span) -> Expr<'hir> {
        // Lambda: a, b, N  (return_type, body, params)
        let Some((NodeKind::Lambda, _, children)) = self.ast.get_node(node) else {
            unreachable!(
                "not a lambda node or invalid lambda node: no such node index {:?}",
                node
            );
        };

        let return_type_node = children[0];
        let body_node = children[1];
        let params_multi = children[2];

        let param_nodes = self.ast.get_multi_child_slice(params_multi).unwrap_or(&[]);

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
        let Some((kind, span, children)) = self.ast.get_node(node) else {
            unreachable!(
                "invalid closure parameter node: no such node index {:?}",
                node
            );
        };

        match kind {
            NodeKind::TypeBoundParam => {
                let pat = self.lower_pattern(children[0]);

                let ty = if children[1] != 0 {
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

    fn lower_fn_type_expr(&mut self, node: NodeIndex, span: Span) -> Expr<'hir> {
        // FnType: flags_u32, abi_node, N  (modifier_flags, abi_str_node, parameter_types)
        let Some((NodeKind::FnType, _, children)) = self.ast.get_node(node) else {
            unreachable!(
                "not a function type node or invalid function type node: no such node index {:?}",
                node
            );
        };

        let _flags = children[0]; // raw u32 bitmask
        let _abi_node = children[1];
        let params_multi = children[2];

        let param_nodes = self.ast.get_multi_child_slice(params_multi).unwrap_or(&[]);

        // Lower each parameter type (including return type as the last entry)
        // as a `TyParamKind::Positional`. By convention, the last TyParam is the return type.
        let ty_params: Vec<TyParam<'hir>> = param_nodes
            .iter()
            .map(|&n| {
                let ty_expr = self.lower_expr(n);
                let ty_ref = self.arena.alloc_expr(ty_expr);
                TyParam::new(self.next_hir_id(), TyParamKind::Positional(ty_ref), span)
            })
            .collect();

        let params_slice = self.arena.alloc_ty_param_slice(ty_params);
        Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::TyFn(params_slice),
            span,
        }
    }

    fn lower_extend_args(&mut self, args_multi: NodeIndex) -> &'hir [Arg<'hir>] {
        let arg_nodes = self.ast.get_multi_child_slice(args_multi).unwrap_or(&[]);
        let args: Vec<Arg<'hir>> = arg_nodes
            .iter()
            .map(|&n| self.lower_extend_arg(n))
            .collect();
        self.arena.alloc_arg_slice(args)
    }

    // 类似div { element1, element2, style: "color: red" }，
    // 这样的extend application，property会采用Arg::Named来表达，而element则采用Arg::Positional
    fn lower_extend_arg(&mut self, node: NodeIndex) -> Arg<'hir> {
        let Some((kind, _, children)) = self.ast.get_node(node) else {
            unreachable!(
                "invalid extend argument node: no such node index {:?}",
                node
            );
        };

        match kind {
            NodeKind::Property => {
                let ident = self.node_to_ident(children[0]);
                let expr = self.lower_expr(children[1]);
                let expr_ref = self.arena.alloc_expr(expr);
                Arg::Named(ident, expr_ref)
            }
            _ => {
                // If it's not a Property, just lower it as a normal expression.
                Arg::Positional(self.arena.alloc_expr(self.lower_expr(node)))
            }
        }
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
            _ => unreachable!("lower_binop called with non-binop NodeKind: {:?}", kind),
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

    /// Create a wildcard pattern `_`.
    fn make_wild_pattern(&mut self, span: Span) -> Pattern<'hir> {
        Pattern {
            hir_id: self.next_hir_id(),
            kind: PatternKind::Wild,
            span,
        }
    }

    /// Wrap `inner` in `if guard { inner }` (no else branch).
    fn wrap_with_guard(
        &mut self,
        guard_node: NodeIndex,
        inner: Expr<'hir>,
        span: Span,
    ) -> Expr<'hir> {
        let guard = self.lower_expr(guard_node);
        let guard_ref = self.arena.alloc_expr(guard);
        let inner_ref = self.arena.alloc_expr(inner);
        let then_block = Block {
            hir_id: self.next_hir_id(),
            stmts: &[],
            expr: Some(inner_ref),
            span,
        };
        let then_ref = self.arena.alloc_block(then_block);
        Expr {
            hir_id: self.next_hir_id(),
            kind: ExprKind::If(guard_ref, then_ref, None),
            span,
        }
    }
}
