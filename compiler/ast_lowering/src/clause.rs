//! Clause lowering — transforms AST clause declarations into HIR
//! `ClauseParam` and `ClauseConstraint`.
//!
//! In Flurry, **clauses serve as generic parameters**. A declaration such as
//!
//! ```text
//! fn id<T, U : Show, V :- Iterator> ...
//! ```
//!
//! produces several clause nodes in the AST, which are mapped to HIR as:
//!
//! | AST `NodeKind`         | Meaning                        | HIR result                             |
//! |------------------------|--------------------------------|----------------------------------------|
//! | `TypeDeclClause`       | bare type parameter `T`        | `ClauseParam(Type(T))`                 |
//! | `TypeBoundDeclClause`  | `T : Show`                     | `ClauseParam(Positional(T, Show))`     |
//! | `TraitBoundDeclClause` | `T :- Iterator`                | `ClauseConstraint(Requires(expr))`     |
//! | `OptionalDeclClause`   | `.a : T = default`             | `ClauseParam(Optional(a, T))`          |
//! | `VarargDeclClause`     | `...a : T`                     | `ClauseParam(Varadic(a, T))`           |
//! | `QuoteDeclClause`      | quoted clause                  | `ClauseParam(Quote(name, expr))`       |
//!
//! **TODO / Ambiguities:**
//! - `TypeBoundDeclClause` (`T : Show`): currently lowered to
//!   `ClauseParamKind::Positional(T, bound_expr)`. Verify this is correct.
//! - `TraitBoundDeclClause` (`T :- Iterator`): lowered as
//!   `ClauseConstraintKind::Requires(bound_expr)`. May need a dedicated kind.

use ast::{NodeIndex, NodeKind};
use hir::{
    ClauseParam,
    clause::{ClauseConstraint, ClauseConstraintKind, ClauseParamKind},
    common::{Ident, Symbol},
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
                // bare type parameter: `T`  →  ClauseParamKind::Type(T)
                NodeKind::TypeDeclClause => {
                    let children = self.ast.get_children(clause_idx);
                    let name_node = children.first().copied().unwrap_or(clause_idx);
                    let name = self.node_to_ident(name_node);
                    result.params.push(ClauseParam {
                        hir_id: self.next_hir_id(),
                        kind: ClauseParamKind::Type(name.clone()),
                        name,
                        span,
                    });
                }

                // type with bound: `T : Show`  →  ClauseParamKind::Positional(T, bound_expr)
                // NOTE: mapping to Positional is tentative; see module-level TODO.
                NodeKind::TypeBoundDeclClause => {
                    let children = self.ast.get_children(clause_idx);
                    if children.len() < 2 {
                        self.emit_malformed("TypeBoundDeclClause: expected 2 children", span);
                        continue;
                    }
                    let name = self.node_to_ident(children[0]);
                    let bound_expr = self.lower_expr(children[1]);
                    let bound_ref = self.arena.alloc_expr(bound_expr);
                    result.params.push(ClauseParam {
                        hir_id: self.next_hir_id(),
                        kind: ClauseParamKind::Positional(name.clone(), bound_ref),
                        name,
                        span,
                    });
                }

                // trait bound constraint: `T :- Iterator`  →  ClauseConstraint::Requires
                // NOTE: using Requires as a placeholder; see module-level TODO.
                NodeKind::TraitBoundDeclClause => {
                    let children = self.ast.get_children(clause_idx);
                    if children.len() < 2 {
                        self.emit_malformed("TraitBoundDeclClause: expected 2 children", span);
                        continue;
                    }
                    let bound_expr = self.lower_expr(children[1]);
                    let bound_ref = self.arena.alloc_expr(bound_expr);
                    result.constraints.push(ClauseConstraint {
                        hir_id: self.next_hir_id(),
                        kind: ClauseConstraintKind::Requires(bound_ref),
                        span,
                    });
                }

                // optional clause: `.a : T = default`  →  ClauseParamKind::Optional(a, T)
                NodeKind::OptionalDeclClause => {
                    let children = self.ast.get_children(clause_idx);
                    if children.len() >= 2 {
                        let name = self.node_to_ident(children[0]);
                        let ty_expr = self.lower_expr(children[1]);
                        let ty_ref = self.arena.alloc_expr(ty_expr);
                        result.params.push(ClauseParam {
                            hir_id: self.next_hir_id(),
                            kind: ClauseParamKind::Optional(name.clone(), ty_ref),
                            name,
                            span,
                        });
                    }
                }

                // variadic clause: `...a : T`  →  ClauseParamKind::Varadic(a, T)
                NodeKind::VarargDeclClause => {
                    let children = self.ast.get_children(clause_idx);
                    if children.len() >= 2 {
                        let name = self.node_to_ident(children[0]);
                        let ty_expr = self.lower_expr(children[1]);
                        let ty_ref = self.arena.alloc_expr(ty_expr);
                        result.params.push(ClauseParam {
                            hir_id: self.next_hir_id(),
                            kind: ClauseParamKind::Varadic(name.clone(), ty_ref),
                            name,
                            span,
                        });
                    }
                }

                // quoted clause  →  ClauseParamKind::Quote(name, expr)
                NodeKind::QuoteDeclClause => {
                    let children = self.ast.get_children(clause_idx);
                    if !children.is_empty() {
                        let (name, expr_node) = if children.len() >= 2 {
                            (self.node_to_ident(children[0]), children[1])
                        } else {
                            (Ident::new(Symbol::intern("_"), span), children[0])
                        };
                        let inner_expr = self.lower_expr(expr_node);
                        let inner_ref = self.arena.alloc_expr(inner_expr);
                        result.params.push(ClauseParam {
                            hir_id: self.next_hir_id(),
                            kind: ClauseParamKind::Quote(name.clone(), inner_ref),
                            name,
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
}
