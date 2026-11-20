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
        self.lower_clauses_inner(nodes, result_binding, false)
    }

    fn lower_clauses_inner(
        &mut self,
        nodes: &[NodeIndex],
        result_binding: Option<(hir::Ident, NodeIndex, hir::HirId)>,
        quoted: bool,
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
                let mut inner = self.lower_clauses_inner(&[children[0]], None, true);
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
                    | NodeKind::VarargParam
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
                    NodeKind::VarargDeclClause | NodeKind::VarargParam => {
                        ClauseParamKind::Varadic(name, ty)
                    }
                    NodeKind::OptionalDeclClause | NodeKind::OptionalParam => {
                        // A quoted clause optional default is evaluated as a runtime
                        // dependency expression rather than a comptime argument.
                        let default_context = if quoted {
                            ExprContext::QuoteArgument
                        } else {
                            ExprContext::ComptimeArgument
                        };
                        let default = self.lower_expr(default_context, children[2]);
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

#[cfg(test)]
mod tests {
    use crate::LoweringContext;
    use ast::{Ast, NodeBuilder, NodeKind};
    use diagnostic::DiagnosticContext;
    use hir::{HirArena, Package, clause::ClauseParamKind, common::Symbol, expr::ExprKind};
    use resolve::{
        ModuleTree, Resolver, ScopeId,
        ids::DefId,
        scope::{Scope, ScopeKind, ScopeTree},
    };
    use rustc_span::{BytePos, SourceMap, Span, source_map::FilePathMapping};
    use vfs::FileId;

    fn add_id(ast: &mut Ast, name: &str, span: Span) -> ast::NodeIndex {
        let (hi, lo) = Symbol::intern(name).to_raw_parts();
        NodeBuilder::new(NodeKind::Id, span)
            .add_single_child(hi)
            .add_single_child(lo)
            .build(ast)
    }

    fn empty_module_tree() -> ModuleTree {
        let mut scopes = ScopeTree::new();
        scopes.add_scope(Scope::new(
            ScopeId::ROOT,
            ScopeKind::Package,
            None,
            None,
            DefId::new(0, 0),
            false,
        ));
        ModuleTree {
            scope_tree: scopes,
            def_names: Default::default(),
            def_count: 1,
            impls: vec![],
            errors: vec![],
            file_scopes: Default::default(),
        }
    }

    #[test]
    fn lowers_quote_lift_clause_and_optional_default_with_lift_expr() {
        let mut ast = Ast::new(BytePos(0));
        let span = Span::default();
        let req_name = add_id(&mut ast, "allocator", span);
        let pointee = NodeBuilder::new(NodeKind::Wildcard, span).build(&mut ast);
        let ptr_ty = NodeBuilder::new(NodeKind::PointerType, span)
            .add_single_child(pointee)
            .build(&mut ast);
        let lift_ty = NodeBuilder::new(NodeKind::LiftType, span)
            .add_single_child(ptr_ty)
            .build(&mut ast);
        let req_clause = NodeBuilder::new(NodeKind::TypeBoundDeclClause, span)
            .add_single_child(req_name)
            .add_single_child(lift_ty)
            .build(&mut ast);
        let req_quote = NodeBuilder::new(NodeKind::QuoteDeclClause, span)
            .add_single_child(req_clause)
            .build(&mut ast);

        let opt_name = add_id(&mut ast, "default_alloc", span);
        let default_expr = NodeBuilder::new(NodeKind::Null, span).build(&mut ast);
        let opt_clause = NodeBuilder::new(NodeKind::OptionalDeclClause, span)
            .add_single_child(opt_name)
            .add_single_child(lift_ty)
            .add_single_child(default_expr)
            .build(&mut ast);
        let opt_quote = NodeBuilder::new(NodeKind::QuoteDeclClause, span)
            .add_single_child(opt_clause)
            .build(&mut ast);

        let arena = HirArena::default();
        let source_map = SourceMap::new(FilePathMapping::empty());
        let diag_ctx = DiagnosticContext::new(&source_map);
        let mut package = Package::new();
        let tree = empty_module_tree();
        let mut resolver = Resolver::new(&tree);
        let mut ctx = LoweringContext::new(
            &ast,
            FileId::from_raw(0),
            &arena,
            &source_map,
            &diag_ctx,
            &mut package,
            &mut resolver,
            ScopeId::ROOT,
        );

        let lowered = ctx.lower_clauses(&[req_quote, opt_quote]);
        assert!(diag_ctx.diagnostics().is_empty());
        assert_eq!(lowered.params.len(), 2);

        let ClauseParamKind::Quote(req_inner) = &lowered.params[0].kind else {
            panic!("expected quoted positional clause");
        };
        let ClauseParamKind::Positional(_, req_ty) = req_inner.as_ref() else {
            panic!("expected positional inner clause");
        };
        assert!(matches!(
            req_ty.kind,
            ExprKind::LiftType(inner) if matches!(inner.kind, ExprKind::TyPtr(_))
        ));
        assert!(
            req_ty.kind.to_string().starts_with("(lift (type (expr ")
                && req_ty.kind.to_string().contains("(kind (ty-ptr (type "),
            "{req_ty}"
        );

        let ClauseParamKind::Quote(opt_inner) = &lowered.params[1].kind else {
            panic!("expected quoted optional clause");
        };
        let ClauseParamKind::Optional(_, opt_ty, opt_default) = opt_inner.as_ref() else {
            panic!("expected optional inner clause");
        };
        assert!(matches!(opt_ty.kind, ExprKind::LiftType(_)));
        assert!(matches!(opt_default.kind, ExprKind::Null));
    }
}
