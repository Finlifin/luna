//! Preserve declaration parameters and bounds for staged elaboration.
//! `T` is a Type parameter, `x: Ty` a typed value parameter, and
//! `T :- Bound` a Type parameter with a trait bound. Defaults and quote
//! wrappers survive lowering; this pass does not invoke them.
use crate::{LoweringContext, expr::ExprContext};
use ast::{NodeIndex, NodeKind};
use hir::{
    ClauseParam,
    clause::{ClauseConstraint, ClauseConstraintKind, ClauseParamKind},
};

pub struct LoweredClauses<'hir> {
    pub params: Vec<ClauseParam<'hir>>,
    pub constraints: Vec<ClauseConstraint<'hir>>,
}
impl<'hir, 'ast, 'resolver: 'ast> LoweringContext<'hir, 'ast, 'resolver> {
    pub fn lower_clauses(&mut self, nodes: &[NodeIndex]) -> LoweredClauses<'hir> {
        self.lower_clauses_with_result(nodes, None)
    }
    pub(crate) fn lower_clauses_with_result(
        &mut self,
        nodes: &[NodeIndex],
        result_binding: Option<(hir::Ident, NodeIndex, hir::HirId)>,
    ) -> LoweredClauses<'hir> {
        let mut result = LoweredClauses {
            params: Vec::new(),
            constraints: Vec::new(),
        };
        for &node in nodes {
            if node == 0 {
                continue;
            }
            let Some((kind, span, children)) = self.ast.get_node(node) else {
                continue;
            };
            if matches!(
                kind,
                NodeKind::Requires | NodeKind::Ensures | NodeKind::Decreases
            ) {
                let result_scope = if kind == NodeKind::Ensures {
                    result_binding.map(|(name, node, binding)| {
                        let scope = self.local_enter(resolve::RibKind::Block);
                        self.define_local(name, node, binding);
                        scope
                    })
                } else {
                    None
                };
                let value = self.lower_expr(ExprContext::RuntimeProposition, children[1]);
                if let Some(scope) = result_scope {
                    self.local_exit(scope);
                }
                let value = self.arena.alloc_expr(value);
                let label = (children[0] != 0).then(|| self.node_to_ident(children[0]));
                let kind = match kind {
                    NodeKind::Requires => ClauseConstraintKind::Requires(value),
                    NodeKind::Ensures => ClauseConstraintKind::Ensures(value),
                    _ => ClauseConstraintKind::Decreases(value),
                };
                result.constraints.push(ClauseConstraint {
                    label,
                    hir_id: self.next_hir_id(),
                    kind,
                    span,
                });
                continue;
            }
            if kind == NodeKind::QuoteDeclClause {
                let mut inner = self.lower_clauses(&[children[0]]);
                if inner.params.len() != 1 || !inner.constraints.is_empty() {
                    self.emit_unsupported_clause("quote requires a declaration parameter", span);
                } else {
                    let mut parameter = inner.params.pop().unwrap();
                    parameter.kind = ClauseParamKind::Quote(Box::new(parameter.kind));
                    parameter.span = span;
                    result.params.push(parameter);
                }
                continue;
            }
            if !matches!(
                kind,
                NodeKind::Id
                    | NodeKind::TypeBoundParam
                    | NodeKind::TraitBoundParam
                    | NodeKind::TypeDeclClause
                    | NodeKind::TypeBoundDeclClause
                    | NodeKind::TraitBoundDeclClause
                    | NodeKind::OptionalParam
                    | NodeKind::OptionalDeclClause
                    | NodeKind::VarargDeclClause
            ) {
                self.emit_unsupported_clause(&format!("{kind:?}"), span);
                continue;
            }
            let name = self.node_to_ident(if kind == NodeKind::Id {
                node
            } else {
                children[0]
            });
            let kind = if matches!(kind, NodeKind::Id | NodeKind::TypeDeclClause) {
                ClauseParamKind::Type(name)
            } else {
                let context = if matches!(
                    kind,
                    NodeKind::TraitBoundParam | NodeKind::TraitBoundDeclClause
                ) {
                    ExprContext::Trait
                } else {
                    ExprContext::ClauseParamType
                };
                let ty = self.lower_expr(context, children[1]);
                let ty = self.arena.alloc_expr(ty);
                match kind {
                    NodeKind::TraitBoundDeclClause | NodeKind::TraitBoundParam => {
                        ClauseParamKind::TraitBound(name, ty)
                    }
                    NodeKind::TypeBoundDeclClause | NodeKind::TypeBoundParam => {
                        ClauseParamKind::Positional(name, ty)
                    }
                    NodeKind::VarargDeclClause => ClauseParamKind::Varadic(name, ty),
                    NodeKind::OptionalDeclClause | NodeKind::OptionalParam => {
                        let default = self.lower_expr(ExprContext::ComptimeArgument, children[2]);
                        ClauseParamKind::Optional(name, ty, self.arena.alloc_expr(default))
                    }
                    _ => unreachable!(),
                }
            };
            let definition = self
                .resolve(name)
                .ok()
                .and_then(|binding| match binding.kind {
                    resolve::BindingKind::ClauseParam(id, _) => Some(id),
                    _ => None,
                });
            result.params.push(ClauseParam {
                definition,
                hir_id: self.next_hir_id(),
                name,
                kind,
                span,
            });
        }
        result
    }
}
