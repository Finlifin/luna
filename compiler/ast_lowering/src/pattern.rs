//! Pattern lowering — AST pattern nodes → HIR [`Pattern`].

use ast::{NodeIndex, NodeKind};
use hir::{
    common::BindingMode,
    pattern::{FieldPat, Pattern, PatternKind},
};
use rustc_span::Span;

use crate::LoweringContext;

impl<'hir, 'ast> LoweringContext<'hir, 'ast> {
    /// Lower an AST node in pattern position into an HIR [`Pattern`].
    pub fn lower_pattern(&mut self, node: NodeIndex) -> Pattern<'hir> {
        let Some((kind, span, children)) = self.ast.get_node(node) else {
            unreachable!("invalid pattern node: no such node index {:?}", node);
        };

        match kind {
            // _ (wildcard)
            NodeKind::Wildcard => Pattern {
                hir_id: self.next_hir_id(),
                kind: PatternKind::Wild,
                span,
            },

            // identifier as binding
            NodeKind::Id => {
                let ident = self.node_to_ident(node);
                Pattern {
                    hir_id: self.next_hir_id(),
                    kind: PatternKind::Binding(BindingMode::ByValue, ident, None),
                    span,
                }
            }

            // Literals used as constant patterns
            NodeKind::Int | NodeKind::Real | NodeKind::Str | NodeKind::Char | NodeKind::Bool => {
                let expr = self.lower_expr(node);
                let expr_ref = self.arena.alloc_expr(expr);
                Pattern {
                    hir_id: self.next_hir_id(),
                    kind: PatternKind::Const(expr_ref),
                    span,
                }
            }

            // `ref pattern`
            NodeKind::RefPattern => {
                let inner = self.lower_pattern(children[0]);
                let inner_ref = self.arena.alloc_pattern(inner);
                Pattern {
                    hir_id: self.next_hir_id(),
                    kind: PatternKind::Ref(inner_ref),
                    span,
                }
            }

            // `a | b` (or-pattern)
            NodeKind::OrPattern => {
                let lhs = self.lower_pattern(children[0]);
                let rhs = self.lower_pattern(children[1]);
                let pats = self.arena.alloc_pattern_slice(vec![lhs, rhs]);
                Pattern {
                    hir_id: self.next_hir_id(),
                    kind: PatternKind::Or(pats),
                    span,
                }
            }

            // `pat as name`
            NodeKind::AsBindPattern => {
                let name_ident = self.node_to_ident(children[0]);
                let inner = self.lower_pattern(children[1]);
                let inner_ref = self.arena.alloc_pattern(inner);
                Pattern {
                    hir_id: self.next_hir_id(),
                    kind: PatternKind::Binding(BindingMode::ByValue, name_ident, Some(inner_ref)),
                    span,
                }
            }

            // (a, b, c) tuple pattern
            NodeKind::TuplePattern => {
                let elems_node = children[0];
                let elem_nodes = self.ast.get_multi_child_slice(elems_node).unwrap_or(&[]);
                let pats: Vec<_> = elem_nodes.iter().map(|&n| self.lower_pattern(n)).collect();
                let pats_slice = self.arena.alloc_pattern_slice(pats);
                Pattern {
                    hir_id: self.next_hir_id(),
                    kind: PatternKind::Tuple(pats_slice),
                    span,
                }
            }

            // [a, b, c] list pattern
            NodeKind::ListPattern => {
                let elems_node = children[0];
                let elem_nodes = self.ast.get_multi_child_slice(elems_node).unwrap_or(&[]);
                let pats: Vec<_> = elem_nodes.iter().map(|&n| self.lower_pattern(n)).collect();
                let pats_slice = self.arena.alloc_pattern_slice(pats);
                Pattern {
                    hir_id: self.next_hir_id(),
                    kind: PatternKind::Tuple(pats_slice),
                    span,
                }
            }

            // Ctor(a, b) application pattern
            NodeKind::ApplicationPattern => {
                let callee = self.lower_pattern(children[0]);
                let callee_ref = self.arena.alloc_pattern(callee);

                let args_node = children[1];
                let arg_nodes = self.ast.get_multi_child_slice(args_node).unwrap_or(&[]);
                let sub_pats: Vec<_> = arg_nodes.iter().map(|&n| self.lower_pattern(n)).collect();
                let sub_pats_slice = self.arena.alloc_pattern_slice(sub_pats);

                Pattern {
                    hir_id: self.next_hir_id(),
                    kind: PatternKind::AppTuple(callee_ref, sub_pats_slice),
                    span,
                }
            }

            // Ctor { field: pat, ... } extended application pattern
            NodeKind::ExtendedApplicationPattern => {
                let callee = self.lower_pattern(children[0]);
                let callee_ref = self.arena.alloc_pattern(callee);

                let fields_node = children[1];
                let field_nodes = self.ast.get_multi_child_slice(fields_node).unwrap_or(&[]);
                let field_pats: Vec<_> = field_nodes
                    .iter()
                    .map(|&n| self.lower_field_pattern(n))
                    .collect();
                let field_pats_slice = self.arena.alloc_field_pat_slice(field_pats);

                Pattern {
                    hir_id: self.next_hir_id(),
                    kind: PatternKind::Struct(callee_ref, field_pats_slice, false),
                    span,
                }
            }

            // { field: pat, ... } struct pattern
            NodeKind::StructPattern => {
                let fields_node = children[0];
                let field_nodes = self.ast.get_multi_child_slice(fields_node).unwrap_or(&[]);
                let field_pats: Vec<_> = field_nodes
                    .iter()
                    .map(|&n| self.lower_field_pattern(n))
                    .collect();
                let field_pats_slice = self.arena.alloc_field_pat_slice(field_pats);

                // Use an empty path for anonymous struct patterns
                let wild = self.arena.alloc_pattern(Pattern {
                    hir_id: self.next_hir_id(),
                    kind: PatternKind::Wild,
                    span: span,
                });

                Pattern {
                    hir_id: self.next_hir_id(),
                    kind: PatternKind::Struct(wild, field_pats_slice, false),
                    span,
                }
            }

            // Range patterns
            NodeKind::RangeFromToPattern => {
                let lo = self.lower_expr(children[0]);
                let hi = self.lower_expr(children[1]);
                let lo_ref = self.arena.alloc_expr(lo);
                let hi_ref = self.arena.alloc_expr(hi);
                Pattern {
                    hir_id: self.next_hir_id(),
                    kind: PatternKind::Range(
                        Some(lo_ref),
                        Some(hi_ref),
                        hir::pattern::BoundType::Exclusive,
                    ),
                    span,
                }
            }

            NodeKind::RangeFromPattern => {
                let lo = self.lower_expr(children[0]);
                let lo_ref = self.arena.alloc_expr(lo);
                Pattern {
                    hir_id: self.next_hir_id(),
                    kind: PatternKind::Range(
                        Some(lo_ref),
                        None,
                        hir::pattern::BoundType::Exclusive,
                    ),
                    span,
                }
            }

            NodeKind::RangeToPattern => {
                let hi = self.lower_expr(children[0]);
                let hi_ref = self.arena.alloc_expr(hi);
                Pattern {
                    hir_id: self.next_hir_id(),
                    kind: PatternKind::Range(
                        None,
                        Some(hi_ref),
                        hir::pattern::BoundType::Exclusive,
                    ),
                    span,
                }
            }

            // < expr > pattern (expression-as-pattern)
            NodeKind::ExprAsPattern => {
                let expr = self.lower_expr(children[0]);
                let expr_ref = self.arena.alloc_expr(expr);
                Pattern {
                    hir_id: self.next_hir_id(),
                    kind: PatternKind::Const(expr_ref),
                    span,
                }
            }

            // Unit / null / undefined
            NodeKind::Unit | NodeKind::Null | NodeKind::Undefined => {
                let expr = self.lower_expr(node);
                let expr_ref = self.arena.alloc_expr(expr);
                Pattern {
                    hir_id: self.next_hir_id(),
                    kind: PatternKind::Const(expr_ref),
                    span,
                }
            }

            // Projection-based path as pattern (e.g. Mod.Variant)
            NodeKind::Projection => {
                let base = self.lower_pattern(children[0]);
                Pattern {
                    hir_id: self.next_hir_id(),
                    kind: PatternKind::Projection(
                        self.arena.alloc_pattern(base),
                        self.node_to_ident(children[1]),
                    ),
                    span,
                }
            }

            other => {
                self.emit_invalid_pattern(&format!("{:?}", other), span);
                self.make_error_pattern(span)
            }
        }
    }

    /// Lower a field pattern node (`id: pattern`).
    fn lower_field_pattern(&mut self, node: NodeIndex) -> FieldPat<'hir> {
        let Some((kind, span, children)) = self.ast.get_node(node) else {
            unreachable!("invalid field pattern node: no such node index {:?}", node);
        };

        match kind {
            NodeKind::PropertyPattern => {
                let ident = self.node_to_ident(children[0]);
                let pat = self.lower_pattern(children[1]);
                FieldPat { ident, pat, span }
            }
            _ => {
                // Shorthand: `name` → `name: name`
                let ident = self.node_to_ident(node);
                let pat = Pattern {
                    hir_id: self.next_hir_id(),
                    kind: PatternKind::Binding(BindingMode::ByValue, ident.clone(), None),
                    span,
                };
                FieldPat { ident, pat, span }
            }
        }
    }

    /// Create an error pattern (used as a recovery node).
    pub(crate) fn make_error_pattern(&mut self, span: Span) -> Pattern<'hir> {
        Pattern {
            hir_id: self.next_hir_id(),
            kind: PatternKind::Invalid,
            span,
        }
    }
}
