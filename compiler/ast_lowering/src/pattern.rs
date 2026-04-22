//! Pattern lowering — AST pattern nodes → HIR [`Pattern`].

use ast::{NodeIndex, NodeKind};
use hir::{
    common::{BindingMode, Ident, Symbol},
    pattern::{FieldPat, Pattern, PatternKind},
};
use rustc_span::Span;

use crate::LoweringContext;

impl<'hir, 'ast> LoweringContext<'hir, 'ast> {
    /// Lower an AST node in pattern position into an HIR [`Pattern`].
    pub fn lower_pattern(&mut self, node: NodeIndex) -> Pattern<'hir> {
        if node == 0 {
            return self.make_error_pattern(Span::default());
        }

        let Some(kind) = self.ast.get_node_kind(node) else {
            return self.make_error_pattern(Span::default());
        };
        let span = self.ast.get_span(node).unwrap_or(Span::default());

        match kind {
            // _ (wildcard)
            NodeKind::Wildcard => Pattern {
                hir_id: self.next_hir_id(),
                kind: PatternKind::Wild,
                span,
            },

            // identifier as binding
            NodeKind::Id => {
                let name = self.source_text(node);
                let ident = Ident::new(Symbol::intern(&name), span);
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
                let children = self.ast.get_children(node);
                if !children.is_empty() {
                    let inner = self.lower_pattern(children[0]);
                    let inner_ref = self.arena.alloc_pattern(inner);
                    Pattern {
                        hir_id: self.next_hir_id(),
                        kind: PatternKind::Ref(inner_ref),
                        span,
                    }
                } else {
                    self.make_error_pattern(span)
                }
            }

            // `a | b` (or-pattern)
            NodeKind::OrPattern => {
                let children = self.ast.get_children(node);
                if children.len() >= 2 {
                    let lhs = self.lower_pattern(children[0]);
                    let rhs = self.lower_pattern(children[1]);
                    let pats = self.arena.alloc_pattern_slice(vec![lhs, rhs]);
                    Pattern {
                        hir_id: self.next_hir_id(),
                        kind: PatternKind::Or(pats),
                        span,
                    }
                } else {
                    self.make_error_pattern(span)
                }
            }

            // `pat as name`
            NodeKind::AsBindPattern => {
                let children = self.ast.get_children(node);
                if children.len() >= 3 {
                    let name_ident = self.node_to_ident(children[0]);
                    let inner = self.lower_pattern(children[1]);
                    let inner_ref = self.arena.alloc_pattern(inner);
                    Pattern {
                        hir_id: self.next_hir_id(),
                        kind: PatternKind::Binding(
                            BindingMode::ByValue,
                            name_ident,
                            Some(inner_ref),
                        ),
                        span,
                    }
                } else {
                    self.make_error_pattern(span)
                }
            }

            // (a, b, c) tuple pattern
            NodeKind::TuplePattern => {
                let children = self.ast.get_children(node);
                if !children.is_empty() {
                    let elems_node = children[0];
                    let elem_nodes = self.ast.get_multi_child_slice(elems_node).unwrap_or(&[]);
                    let pats: Vec<_> = elem_nodes.iter().map(|&n| self.lower_pattern(n)).collect();
                    let pats_slice = self.arena.alloc_pattern_slice(pats);
                    Pattern {
                        hir_id: self.next_hir_id(),
                        kind: PatternKind::Tuple(pats_slice),
                        span,
                    }
                } else {
                    self.make_error_pattern(span)
                }
            }

            // [a, b, c] list pattern
            NodeKind::ListPattern => {
                let children = self.ast.get_children(node);
                if !children.is_empty() {
                    let elems_node = children[0];
                    let elem_nodes = self.ast.get_multi_child_slice(elems_node).unwrap_or(&[]);
                    let pats: Vec<_> = elem_nodes.iter().map(|&n| self.lower_pattern(n)).collect();
                    let pats_slice = self.arena.alloc_pattern_slice(pats);
                    Pattern {
                        hir_id: self.next_hir_id(),
                        kind: PatternKind::Tuple(pats_slice),
                        span,
                    }
                } else {
                    self.make_error_pattern(span)
                }
            }

            // Ctor(a, b) application pattern (e.g. Some(x))
            NodeKind::ApplicationPattern => {
                let children = self.ast.get_children(node);
                if children.len() >= 2 {
                    let callee = children[0];
                    let callee_expr = self.lower_expr(callee);
                    let callee_ref = self.arena.alloc_expr(callee_expr);

                    let args_node = children[1];
                    let arg_nodes = self.ast.get_multi_child_slice(args_node).unwrap_or(&[]);
                    let sub_pats: Vec<_> =
                        arg_nodes.iter().map(|&n| self.lower_pattern(n)).collect();
                    let sub_pats_slice = self.arena.alloc_pattern_slice(sub_pats);

                    Pattern {
                        hir_id: self.next_hir_id(),
                        kind: PatternKind::TupleStruct(callee_ref, sub_pats_slice),
                        span,
                    }
                } else {
                    self.make_error_pattern(span)
                }
            }

            // Ctor { field: pat, ... } extended application pattern
            NodeKind::ExtendedApplicationPattern => {
                let children = self.ast.get_children(node);
                if children.len() >= 2 {
                    let callee = children[0];
                    let path = self.lower_expr_as_path(callee);

                    let fields_node = children[1];
                    let field_nodes = self.ast.get_multi_child_slice(fields_node).unwrap_or(&[]);
                    let field_pats: Vec<_> = field_nodes
                        .iter()
                        .map(|&n| self.lower_field_pattern(n))
                        .collect();
                    let field_pats_slice = self.arena.alloc_field_pat_slice(field_pats);

                    Pattern {
                        hir_id: self.next_hir_id(),
                        kind: PatternKind::Struct(path, field_pats_slice, false),
                        span,
                    }
                } else {
                    self.make_error_pattern(span)
                }
            }

            // { field: pat, ... } struct pattern
            NodeKind::StructPattern => {
                let children = self.ast.get_children(node);
                if !children.is_empty() {
                    let fields_node = children[0];
                    let field_nodes = self.ast.get_multi_child_slice(fields_node).unwrap_or(&[]);
                    let field_pats: Vec<_> = field_nodes
                        .iter()
                        .map(|&n| self.lower_field_pattern(n))
                        .collect();
                    let field_pats_slice = self.arena.alloc_field_pat_slice(field_pats);

                    // Use an empty path for anonymous struct patterns
                    let path = hir::common::Path {
                        segments: &[],
                        span,
                    };

                    Pattern {
                        hir_id: self.next_hir_id(),
                        kind: PatternKind::Struct(path, field_pats_slice, false),
                        span,
                    }
                } else {
                    self.make_error_pattern(span)
                }
            }

            // Range patterns
            NodeKind::RangeFromToPattern => {
                let children = self.ast.get_children(node);
                if children.len() >= 2 {
                    let lo = self.lower_expr(children[0]);
                    let hi = self.lower_expr(children[1]);
                    let lo_ref = self.arena.alloc_expr(lo);
                    let hi_ref = self.arena.alloc_expr(hi);
                    Pattern {
                        hir_id: self.next_hir_id(),
                        kind: PatternKind::Range(Some(lo_ref), Some(hi_ref)),
                        span,
                    }
                } else {
                    self.make_error_pattern(span)
                }
            }

            NodeKind::RangeFromPattern => {
                let children = self.ast.get_children(node);
                if !children.is_empty() {
                    let lo = self.lower_expr(children[0]);
                    let lo_ref = self.arena.alloc_expr(lo);
                    Pattern {
                        hir_id: self.next_hir_id(),
                        kind: PatternKind::Range(Some(lo_ref), None),
                        span,
                    }
                } else {
                    self.make_error_pattern(span)
                }
            }

            NodeKind::RangeToPattern => {
                let children = self.ast.get_children(node);
                if !children.is_empty() {
                    let hi = self.lower_expr(children[0]);
                    let hi_ref = self.arena.alloc_expr(hi);
                    Pattern {
                        hir_id: self.next_hir_id(),
                        kind: PatternKind::Range(None, Some(hi_ref)),
                        span,
                    }
                } else {
                    self.make_error_pattern(span)
                }
            }

            // < expr > pattern (expression-as-pattern)
            NodeKind::ExprAsPattern => {
                let children = self.ast.get_children(node);
                if !children.is_empty() {
                    let expr = self.lower_expr(children[0]);
                    let expr_ref = self.arena.alloc_expr(expr);
                    Pattern {
                        hir_id: self.next_hir_id(),
                        kind: PatternKind::Const(expr_ref),
                        span,
                    }
                } else {
                    self.make_error_pattern(span)
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
                let path = self.lower_path_from_select(node);
                Pattern {
                    hir_id: self.next_hir_id(),
                    kind: PatternKind::Path(path),
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
        let span = self.ast.get_span(node).unwrap_or(Span::default());
        let kind = self.ast.get_node_kind(node);

        match kind {
            Some(NodeKind::PropertyPattern) => {
                let children = self.ast.get_children(node);
                if children.len() >= 2 {
                    let ident = self.node_to_ident(children[0]);
                    let pat = self.lower_pattern(children[1]);
                    FieldPat { ident, pat, span }
                } else {
                    let ident = self.node_to_ident(node);
                    let pat = Pattern {
                        hir_id: self.next_hir_id(),
                        kind: PatternKind::Binding(BindingMode::ByValue, ident.clone(), None),
                        span,
                    };
                    FieldPat { ident, pat, span }
                }
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
            kind: PatternKind::Err,
            span,
        }
    }
}
