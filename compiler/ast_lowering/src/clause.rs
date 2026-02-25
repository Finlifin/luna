//! Clause lowering — transforms AST clause declarations into HIR
//! `ClauseParam` and `ClauseConstraint`.
//!
//! In Flurry, **clauses serve as generic parameters**. A declaration such as
//!
//! ```text
//! fn id<T, U : Show, V :- Iterator> ...
//! ```
//!
//! produces three clause nodes in the AST:
//!
//! | AST `NodeKind`         | Meaning                        | HIR result         |
//! |------------------------|--------------------------------|--------------------|
//! | `TypeDeclClause`       | bare type parameter `T`        | `ClauseParam`      |
//! | `TypeBoundDeclClause`  | type + bound `U : Show`        | `ClauseParam`      |
//! | `TraitBoundDeclClause` | trait bound `V :- Iterator`    | `ClauseConstraint` |
//! | `OptionalDeclClause`   | optional `.a : T = default`    | `ClauseConstraint` |
//! | `VarargDeclClause`     | variadic `...a : T`            | `ClauseConstraint` |
//! | `QuoteDeclClause`      | quoted clause                  | `ClauseConstraint` |

use ast::{NodeIndex, NodeKind};
use hir::{
    ClauseParam, TraitBound,
    clause::{ClauseConstraint, ClauseConstraintKind},
    common::{Ident, Path, PathSegment, Symbol},
    ty::TraitBoundKind,
};
use rustc_span::Span;

use crate::LoweringContext;

/// The result of lowering a sequence of clause nodes: separated into
/// generic parameters (type params) and constraints (where-clauses).
pub struct LoweredClauses<'hir> {
    pub params: Vec<ClauseParam<'hir>>,
    pub constraints: Vec<ClauseConstraint<'hir>>,
}

impl<'hir, 'ast> LoweringContext<'hir, 'ast> {
    /// Lower a slice of AST clause node indices into HIR clause params and
    /// constraints.
    pub fn lower_clauses(&mut self, clause_nodes: &[NodeIndex]) -> LoweredClauses<'hir> {
        let mut result = LoweredClauses {
            params: Vec::new(),
            constraints: Vec::new(),
        };

        for &clause_idx in clause_nodes {
            if clause_idx == 0 {
                continue;
            }

            let Some(kind) = self.ast.get_node_kind(clause_idx) else {
                continue;
            };
            let span = self.ast.get_span(clause_idx).unwrap_or(Span::default());

            match kind {
                // bare type parameter: `T`
                NodeKind::TypeDeclClause => {
                    let name = self.node_to_ident(clause_idx);
                    let hir_id = self.next_hir_id();
                    result.params.push(ClauseParam {
                        hir_id,
                        ident: name,
                        bounds: &[],
                        span,
                    });
                }

                // type + bound: `T : Show`  →  ClauseParam with bounds
                NodeKind::TypeBoundDeclClause => {
                    let children = self.ast.get_children(clause_idx);
                    if children.len() < 2 {
                        self.emit_malformed("TypeBoundDeclClause: expected 2 children", span);
                        continue;
                    }
                    let name = self.node_to_ident(children[0]);
                    let bound_expr = children[1];

                    let bounds = self.lower_trait_bound_expr(bound_expr);
                    let bounds_slice = self.arena.alloc_type_bound_slice(bounds);

                    let hir_id = self.next_hir_id();
                    result.params.push(ClauseParam {
                        hir_id,
                        ident: name,
                        bounds: bounds_slice,
                        span,
                    });
                }

                // trait bound constraint: `T :- Iterator`  →  ClauseConstraint::Bound
                NodeKind::TraitBoundDeclClause => {
                    let children = self.ast.get_children(clause_idx);
                    if children.len() < 2 {
                        self.emit_malformed("TraitBoundDeclClause: expected 2 children", span);
                        continue;
                    }
                    let name = self.node_to_ident(children[0]);
                    let bound_expr = children[1];

                    let bounds = self.lower_trait_bound_expr(bound_expr);
                    let bounds_slice = self.arena.alloc_type_bound_slice(bounds);

                    let hir_id = self.next_hir_id();
                    result.constraints.push(ClauseConstraint {
                        hir_id,
                        kind: ClauseConstraintKind::Bound(name, bounds_slice),
                        span,
                    });
                }

                // optional clause: `.name : type = default`
                // We treat it as a predicate constraint for now.
                NodeKind::OptionalDeclClause => {
                    let hir_id = self.next_hir_id();
                    let children = self.ast.get_children(clause_idx);
                    if children.len() >= 2 {
                        let name = self.node_to_ident(children[0]);
                        result.constraints.push(ClauseConstraint {
                            hir_id,
                            kind: ClauseConstraintKind::Param(name),
                            span,
                        });
                    }
                }

                // variadic clause: `...name : type`
                NodeKind::VarargDeclClause => {
                    let hir_id = self.next_hir_id();
                    let children = self.ast.get_children(clause_idx);
                    if children.len() >= 1 {
                        let name = self.node_to_ident(children[0]);
                        result.constraints.push(ClauseConstraint {
                            hir_id,
                            kind: ClauseConstraintKind::Param(name),
                            span,
                        });
                    }
                }

                // quoted clause
                NodeKind::QuoteDeclClause => {
                    let hir_id = self.next_hir_id();
                    let children = self.ast.get_children(clause_idx);
                    if children.len() >= 1 {
                        let inner_expr = self.lower_expr(children[0]);
                        let inner_ref = self.arena.alloc_expr(inner_expr);
                        result.constraints.push(ClauseConstraint {
                            hir_id,
                            kind: ClauseConstraintKind::Predicate(inner_ref),
                            span,
                        });
                    }
                }

                other => {
                    self.emit_unsupported_clause(&format!("{:?}", other), span);
                }
            }
        }

        result
    }

    /// Lower a trait-bound expression (the RHS of `: Show` or `:- Iterator`)
    /// into a list of `TraitBound`s.
    ///
    /// Currently handles simple identifiers and paths. Complex bounds can be
    /// extended here later.
    fn lower_trait_bound_expr(&mut self, node: NodeIndex) -> Vec<TraitBound<'hir>> {
        if node == 0 {
            return Vec::new();
        }

        let Some(kind) = self.ast.get_node_kind(node) else {
            return Vec::new();
        };
        let span = self.ast.get_span(node).unwrap_or(Span::default());

        match kind {
            NodeKind::Id => {
                let name = self.source_text(node);
                let ident = Ident::new(Symbol::intern(&name), span);
                let seg = PathSegment { ident, args: &[] };
                let segments = self.arena.alloc_path_segment_slice(vec![seg]);
                let path = Path { segments, span };
                vec![TraitBound {
                    kind: TraitBoundKind::Trait(path),
                    span,
                }]
            }

            NodeKind::Select => {
                let path = self.lower_path_from_select(node);
                vec![TraitBound {
                    kind: TraitBoundKind::Trait(path),
                    span,
                }]
            }

            NodeKind::NormalFormApplication => {
                // e.g. `Iterator<Item = T>` — for now lower as a simple path
                let children = self.ast.get_children(node);
                if !children.is_empty() {
                    let base = children[0];
                    let path = self.lower_expr_as_path(base);
                    vec![TraitBound {
                        kind: TraitBoundKind::Trait(path),
                        span,
                    }]
                } else {
                    Vec::new()
                }
            }

            // Trait intersection `A + B` lowered as multiple bounds
            NodeKind::Add => {
                let children = self.ast.get_children(node);
                if children.len() >= 2 {
                    let mut bounds = self.lower_trait_bound_expr(children[0]);
                    bounds.extend(self.lower_trait_bound_expr(children[1]));
                    bounds
                } else {
                    Vec::new()
                }
            }

            _ => {
                // Fallback: try to interpret as a single-segment path
                let name = self.source_text(node);
                if !name.is_empty() {
                    let ident = Ident::new(Symbol::intern(&name), span);
                    let seg = PathSegment { ident, args: &[] };
                    let segments = self.arena.alloc_path_segment_slice(vec![seg]);
                    let path = Path { segments, span };
                    vec![TraitBound {
                        kind: TraitBoundKind::Trait(path),
                        span,
                    }]
                } else {
                    Vec::new()
                }
            }
        }
    }
}
