//! Path lowering — transforms AST path-like nodes into HIR [`Path`].

use ast::{NodeIndex, NodeKind};
use hir::common::{GenericArg, Ident, Path, PathSegment, Symbol};
use rustc_span::Span;

use crate::LoweringContext;

impl<'hir, 'ast> LoweringContext<'hir, 'ast> {
    /// Lower an AST `Select` chain (a.b.c) into an HIR `Path`.
    pub fn lower_path_from_select(&mut self, node: NodeIndex) -> Path<'hir> {
        let mut segments = Vec::new();
        self.collect_path_segments(node, &mut segments);
        let span = self.ast.get_span(node).unwrap_or(Span::default());
        let segments_slice = self.arena.alloc_path_segment_slice(segments);
        Path {
            segments: segments_slice,
            span,
        }
    }

    /// Recursively collect path segments from nested `Select` nodes.
    fn collect_path_segments(
        &mut self,
        node: NodeIndex,
        segments: &mut Vec<PathSegment<'hir>>,
    ) {
        if node == 0 {
            return;
        }
        let Some(kind) = self.ast.get_node_kind(node) else {
            return;
        };

        match kind {
            NodeKind::Select => {
                let children = self.ast.get_children(node);
                if children.len() >= 2 {
                    // Left side: deeper path or base identifier
                    self.collect_path_segments(children[0], segments);
                    // Right side: the next segment
                    let rhs = children[1];
                    let ident = self.node_to_ident(rhs);
                    segments.push(PathSegment {
                        ident,
                        args: &[],
                    });
                }
            }

            NodeKind::NormalFormApplication => {
                // path<generic_args>
                let children = self.ast.get_children(node);
                if !children.is_empty() {
                    let base = children[0];
                    let multi_args_node = if children.len() > 1 {
                        children[1]
                    } else {
                        0
                    };

                    // Collect the base path segments
                    self.collect_path_segments(base, segments);

                    // Attach generic args to the last segment
                    if multi_args_node != 0 {
                        let arg_nodes = self
                            .ast
                            .get_multi_child_slice(multi_args_node)
                            .unwrap_or(&[]);
                        let args = self.lower_generic_args(arg_nodes);
                        if let Some(last) = segments.last_mut() {
                            last.args = self.arena.alloc_generic_arg_slice(args);
                        }
                    }
                }
            }

            NodeKind::Id | NodeKind::SelfLower | NodeKind::SelfCap => {
                let ident = self.node_to_ident(node);
                segments.push(PathSegment {
                    ident,
                    args: &[],
                });
            }

            NodeKind::SuperPath => {
                // . path  (super path)
                let children = self.ast.get_children(node);
                if !children.is_empty() {
                    let ident = Ident::new(
                        Symbol::intern("super"),
                        self.ast.get_span(node).unwrap_or(Span::default()),
                    );
                    segments.push(PathSegment {
                        ident,
                        args: &[],
                    });
                    self.collect_path_segments(children[0], segments);
                }
            }

            NodeKind::PackagePath => {
                // @ path
                let children = self.ast.get_children(node);
                if !children.is_empty() {
                    let ident = Ident::new(
                        Symbol::intern("@"),
                        self.ast.get_span(node).unwrap_or(Span::default()),
                    );
                    segments.push(PathSegment {
                        ident,
                        args: &[],
                    });
                    self.collect_path_segments(children[0], segments);
                }
            }

            _ => {
                // Fallback: try to use source text as a single segment
                let name = self.source_text(node);
                let span = self.ast.get_span(node).unwrap_or(Span::default());
                let ident = Ident::new(Symbol::intern(&name), span);
                segments.push(PathSegment {
                    ident,
                    args: &[],
                });
            }
        }
    }

    /// Lower an expression node into a path (best-effort).
    ///
    /// Used when an expression is in type position and should be
    /// interpreted as a path (e.g. `Iterator` or `std.io.Read`).
    pub fn lower_expr_as_path(&mut self, node: NodeIndex) -> Path<'hir> {
        let mut segments = Vec::new();
        self.collect_path_segments(node, &mut segments);
        let span = self.ast.get_span(node).unwrap_or(Span::default());

        if segments.is_empty() {
            // Produce a single-segment path from source text
            let name = self.source_text(node);
            let ident = Ident::new(Symbol::intern(&name), span);
            segments.push(PathSegment {
                ident,
                args: &[],
            });
        }

        let segments_slice = self.arena.alloc_path_segment_slice(segments);
        Path {
            segments: segments_slice,
            span,
        }
    }

    /// Lower generic argument nodes (inside `<...>`) to HIR `GenericArg`s.
    fn lower_generic_args(&mut self, arg_nodes: &[NodeIndex]) -> Vec<GenericArg<'hir>> {
        let mut args = Vec::with_capacity(arg_nodes.len());
        for &arg_node in arg_nodes {
            if arg_node == 0 {
                continue;
            }
            let kind = self.ast.get_node_kind(arg_node);

            match kind {
                Some(NodeKind::OptionalArg) => {
                    // .name = expr
                    let children = self.ast.get_children(arg_node);
                    if children.len() >= 2 {
                        let name_ident = self.node_to_ident(children[0]);
                        let val_expr = self.lower_expr(children[1]);
                        let val_ref = self.arena.alloc_expr(val_expr);
                        args.push(GenericArg::Optional(name_ident, val_ref));
                    }
                }
                _ => {
                    let expr = self.lower_expr(arg_node);
                    let expr_ref = self.arena.alloc_expr(expr);
                    args.push(GenericArg::Expr(expr_ref));
                }
            }
        }
        args
    }
}
