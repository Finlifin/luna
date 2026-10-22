//! Pattern lowering — AST pattern nodes → HIR [`Pattern`].

use ast::{NodeIndex, NodeKind};
use hir::{
    common::BindingMode,
    pattern::{FieldPat, Pattern, PatternKind},
};
use rustc_span::Span;

use crate::{LoweringContext, expr::ExprContext};

impl<'hir, 'ast, 'resolver: 'ast> LoweringContext<'hir, 'ast, 'resolver> {
    pub(crate) fn pattern_expression_outputs(
        &self,
        value: &hir::Expr<'_>,
    ) -> Option<Vec<hir::common::Ident>> {
        use hir::ExprKind as H;
        let mut pending = vec![(value, true)];
        let mut found: Option<Vec<hir::common::Ident>> = None;
        let mut fuel = 4096usize;
        while let Some((value, result)) = pending.pop() {
            fuel = fuel.checked_sub(1)?;
            match &value.kind {
                H::Return(Some(value)) => pending.push((value, true)),
                H::InlineControl(value) => pending.push((value, result)),
                H::Semi(value) => pending.push((value, false)),
                H::Block(block) | H::Loop(block) | H::For { body: block, .. } => {
                    for statement in block.stmts {
                        pending.push((statement, false));
                    }
                    if let Some(tail) = block.expr {
                        pending.push((tail, result));
                    }
                }
                H::If(_, yes, no) => {
                    for statement in yes.stmts {
                        pending.push((statement, false));
                    }
                    if let Some(tail) = yes.expr {
                        pending.push((tail, result));
                    }
                    if let Some(no) = no {
                        pending.push((no, result));
                    }
                }
                H::When(arms) => pending.extend(arms.iter().map(|arm| (arm.body, result))),
                H::Match(_, arms) => pending.extend(arms.iter().map(|arm| (arm.body, result))),
                _ if result => {
                    let Some(outputs) = self.pattern_expression_leaf_outputs(value) else {
                        continue;
                    };
                    if let Some(first) = &found {
                        let names = |items: &[hir::common::Ident]| {
                            items
                                .iter()
                                .map(|item| item.name.as_str().to_string())
                                .collect::<std::collections::BTreeSet<_>>()
                        };
                        if names(first) != names(&outputs) {
                            return None;
                        }
                    } else {
                        found = Some(outputs);
                    }
                }
                _ => {}
            }
        }
        found
    }

    fn pattern_expression_leaf_outputs(
        &self,
        value: &hir::Expr<'_>,
    ) -> Option<Vec<hir::common::Ident>> {
        match &value.kind {
            hir::ExprKind::ResolvedLocal(_, id) | hir::ExprKind::ResolvedFnParam(_, id) => {
                self.pattern_code_outputs.get(id).cloned()
            }
            hir::ExprKind::CodePatternQuote(pattern, substitutions) => {
                Some(if substitutions.is_empty() {
                    pattern_output_names(pattern)
                } else {
                    substitutions.iter().map(|entry| entry.port).collect()
                })
            }
            hir::ExprKind::Application(callee, _)
            | hir::ExprKind::ExtendedApplication(callee, _) => {
                let callee = match callee.kind {
                    hir::ExprKind::NFApplication(head, _) => head,
                    _ => callee,
                };
                let def_id = match callee.kind {
                    hir::ExprKind::ResolvedDef { def_id, .. } => def_id,
                    hir::ExprKind::Projection(base, name) => {
                        if let Some(member) = self.package.member_projections.get(&callee.hir_id) {
                            *member
                        } else {
                            // An explicit trait view names the public contract.
                            // Resolve unique inherited origins before body lowering;
                            // implementation selection and type checking occur later.
                            let hir::ExprKind::Cast(_, interface) = base.kind else {
                                return None;
                            };
                            let interface = match interface.kind {
                                hir::ExprKind::NFApplication(head, _) => head,
                                _ => interface,
                            };
                            let hir::ExprKind::ResolvedDef {
                                def_id: interface, ..
                            } = interface.kind
                            else {
                                return None;
                            };
                            let scope = self.resolver.find_scope_for_def(interface)?;
                            if self.resolver.scope_tree().get(scope)?.kind
                                != resolve::scope::ScopeKind::TraitBody
                            {
                                return None;
                            }
                            self.resolver.pattern_trait_member(
                                interface,
                                name.name,
                                self.file_scope,
                                name.span,
                            )?
                        }
                    }
                    _ => return None,
                };
                // The declaration scan precedes all body lowering, including
                // bodies in other files. Use its names even when the callee's
                // HIR signature does not exist yet; elaboration checks types.
                if let Some(scope) = self.resolver.find_scope_for_def(def_id)
                    && let Some(outputs) = self
                        .resolver
                        .scope_tree()
                        .get(scope)
                        .and_then(|scope| scope.pattern_result_outputs.as_ref())
                {
                    return Some(
                        outputs
                            .iter()
                            .map(|(name, span)| hir::common::Ident {
                                name: *name,
                                span: *span,
                            })
                            .collect(),
                    );
                }
                let owner = self.package.definition_owner(def_id)?;
                let (hir::ItemKind::Fn(signature, _) | hir::ItemKind::FnDeclaration(signature)) =
                    &self.package.item(owner)?.kind
                else {
                    return None;
                };
                let hir::ExprKind::ContextualPatternType(_, outputs) = &signature.return_ty?.kind
                else {
                    return None;
                };
                Some(outputs.iter().map(|output| output.name).collect())
            }
            _ => None,
        }
    }

    /// Lower an AST node in pattern position into an HIR [`Pattern`].
    pub fn lower_pattern(&mut self, node: NodeIndex) -> Pattern<'hir> {
        let Some((kind, span, children)) = self.ast.get_node(node) else {
            unreachable!("invalid pattern node: no such node index {:?}", node);
        };

        match kind {
            NodeKind::SplicePattern => {
                let operand = self.lower_expr(ExprContext::QuoteArgument, children[0]);
                let mut outputs: Vec<_> = self
                    .ast
                    .get_multi_child_slice(children[1])
                    .unwrap_or(&[])
                    .iter()
                    .map(|&entry| {
                        let parts = self.ast.get_children(entry);
                        hir::pattern::CodePatternOutput {
                            port: self.node_to_ident(parts[0]),
                            name: self.node_to_ident(parts[1]),
                            hir_id: self.next_hir_id(),
                        }
                    })
                    .collect();
                let declared = self
                    .pattern_expression_outputs(&operand)
                    .unwrap_or_default();
                for port in declared {
                    if !outputs.iter().any(|output| output.port.name == port.name) {
                        let name = hir::common::Ident {
                            name: port.name,
                            span,
                        };
                        outputs.push(hir::pattern::CodePatternOutput {
                            port: name,
                            name,
                            hir_id: self.next_hir_id(),
                        });
                    }
                }
                Pattern {
                    hir_id: self.next_hir_id(),
                    span,
                    kind: PatternKind::CodeSplice(self.arena.alloc_expr(operand), outputs),
                }
            }
            NodeKind::ErrorOkPattern | NodeKind::ErrorPattern => {
                let inner = self.lower_pattern(children[0]);
                let inner = self.arena.alloc_pattern(inner);
                Pattern {
                    hir_id: self.next_hir_id(),
                    span,
                    kind: if kind == NodeKind::ErrorOkPattern {
                        PatternKind::ErrorOk(inner)
                    } else {
                        PatternKind::ErrorErr(inner)
                    },
                }
            }
            NodeKind::OptionSomePattern => {
                let inner = self.lower_pattern(children[0]);
                Pattern {
                    hir_id: self.next_hir_id(),
                    span,
                    kind: PatternKind::OptionSome(self.arena.alloc_pattern(inner)),
                }
            }
            NodeKind::NotPattern => {
                let inner = self.lower_pattern(children[0]);
                Pattern {
                    hir_id: self.next_hir_id(),
                    kind: PatternKind::Not(self.arena.alloc_pattern(inner)),
                    span,
                }
            }
            NodeKind::AndIsPattern | NodeKind::IfGuardPattern => {
                // Candidate-local scope: only the left pattern is visible while
                // lowering the next expression. The caller later exports the
                // complete successful pattern, including all duplicate checks.
                let scope = self.local_enter(resolve::RibKind::MatchBody);
                let first = self.lower_pattern(children[0]);
                self.bind_arm_pattern(children[0], &first);
                let value = self.lower_expr(ExprContext::RuntimeFunctionBody, children[1]);
                let next = if kind == NodeKind::AndIsPattern {
                    Some(self.lower_pattern(children[2]))
                } else {
                    None
                };
                self.local_exit(scope);
                let first = self.arena.alloc_pattern(first);
                let value = self.arena.alloc_expr(value);
                let kind = match next {
                    Some(next) => PatternKind::Then(first, value, self.arena.alloc_pattern(next)),
                    None => PatternKind::Guard(first, value),
                };
                Pattern {
                    hir_id: self.next_hir_id(),
                    kind,
                    span,
                }
            }
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
            NodeKind::Int
            | NodeKind::Real
            | NodeKind::Str
            | NodeKind::StringConcat
            | NodeKind::Char
            | NodeKind::Bool
            | NodeKind::Symbol
            | NodeKind::LiteralExtension
            | NodeKind::Negative => {
                let expr = self.lower_expr(ExprContext::PatternConstant, node);
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
                let name_ident = self.node_to_ident(children[1]);
                let inner = self.lower_pattern(children[0]);
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
                // Later tuple fields may observe bindings established by
                // earlier fields. Keep these candidate bindings private until
                // the caller publishes the complete successful pattern.
                let scope = self.local_enter(resolve::RibKind::MatchBody);
                let mut pats = Vec::with_capacity(elem_nodes.len());
                for &node in elem_nodes {
                    let pattern = self.lower_pattern(node);
                    self.bind_arm_pattern(node, &pattern);
                    pats.push(pattern);
                }
                self.local_exit(scope);
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
                let scope = self.local_enter(resolve::RibKind::MatchBody);
                let mut pats = Vec::with_capacity(elem_nodes.len());
                for &node in elem_nodes {
                    let pattern = self.lower_pattern(node);
                    self.bind_arm_pattern(node, &pattern);
                    pats.push(pattern);
                }
                self.local_exit(scope);
                let pats_slice = self.arena.alloc_pattern_slice(pats);
                Pattern {
                    hir_id: self.next_hir_id(),
                    kind: PatternKind::List(pats_slice, None),
                    span,
                }
            }

            // Ctor(a, b) application pattern
            NodeKind::NormalFormHeadApplicationPattern => {
                let head = self.lower_expr(ExprContext::PatternConstant, children[0]);
                let scope = self.local_enter(resolve::RibKind::MatchBody);
                let mut arguments = Vec::new();
                for node in self.ast.get_multi_child_slice(children[1]).unwrap_or(&[]) {
                    let (name, pattern) =
                        if self.ast.get_node_kind(*node) == Some(NodeKind::PropertyPattern) {
                            let children = self.ast.get_children(*node);
                            (Some(self.node_to_ident(children[0])), children[1])
                        } else {
                            (None, *node)
                        };
                    let lowered = self.lower_pattern(pattern);
                    self.bind_arm_pattern(pattern, &lowered);
                    arguments.push(hir::pattern::TypePatternArg {
                        name,
                        pattern: lowered,
                    });
                }
                self.local_exit(scope);
                Pattern {
                    hir_id: self.next_hir_id(),
                    span,
                    kind: PatternKind::NormalForm(
                        self.arena.alloc_expr(head),
                        self.arena.alloc_type_pattern_arg_slice(arguments),
                    ),
                }
            }
            NodeKind::ApplicationPattern => {
                let callee = self.lower_pattern(children[0]);
                let callee_ref = self.arena.alloc_pattern(callee);

                let args_node = children[1];
                let arg_nodes = self.ast.get_multi_child_slice(args_node).unwrap_or(&[]);
                // Positional variant payloads have the same left-to-right
                // candidate scope as tuple patterns. Later observations may
                // use earlier payload bindings, but no candidate-local name
                // escapes until the caller publishes the complete pattern.
                let scope = self.local_enter(resolve::RibKind::MatchBody);
                let mut sub_pats = Vec::with_capacity(arg_nodes.len());
                for &node in arg_nodes {
                    let pattern = self.lower_pattern(node);
                    self.bind_arm_pattern(node, &pattern);
                    sub_pats.push(pattern);
                }
                self.local_exit(scope);
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
                let field_pats = self.lower_candidate_fields(field_nodes);
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
                let field_pats = self.lower_candidate_fields(field_nodes);
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

            // Range endpoints are expressions in the surrounding scope, not
            // binders. Preserve inclusive ends all the way into HIR.
            NodeKind::RangeFromToPattern
            | NodeKind::RangeFromToInclusivePattern
            | NodeKind::RangeFromPattern
            | NodeKind::RangeToPattern
            | NodeKind::RangeToInclusivePattern
            | NodeKind::RangeFull => {
                let (lo, hi) = match kind {
                    NodeKind::RangeFull => (None, None),
                    NodeKind::RangeFromPattern => (Some(children[0]), None),
                    NodeKind::RangeToPattern | NodeKind::RangeToInclusivePattern => {
                        (None, Some(children[0]))
                    }
                    _ => (Some(children[0]), Some(children[1])),
                };
                let hir_id = self.next_hir_id();
                let mut lower = |node| {
                    let node = match self.ast.get_node(node) {
                        Some((
                            NodeKind::ExprAsPattern | NodeKind::TypeBindPattern,
                            _,
                            children,
                        )) => children[0],
                        _ => node,
                    };
                    let value = self.lower_expr(ExprContext::PatternConstant, node);
                    &*self.arena.alloc_expr(value)
                };
                Pattern {
                    hir_id,
                    kind: PatternKind::Range(
                        lo.map(&mut lower),
                        hi.map(&mut lower),
                        if matches!(
                            kind,
                            NodeKind::RangeFromToInclusivePattern
                                | NodeKind::RangeToInclusivePattern
                        ) {
                            hir::pattern::BoundType::Inclusive
                        } else {
                            hir::pattern::BoundType::Exclusive
                        },
                    ),
                    span,
                }
            }

            // <expr> and quoted identifiers observe an outer expression;
            // neither introduces a new binding in the matching arm.
            NodeKind::ExprAsPattern | NodeKind::TypeBindPattern => {
                let expr = self.lower_expr(ExprContext::PatternConstant, children[0]);
                let expr_ref = self.arena.alloc_expr(expr);
                Pattern {
                    hir_id: self.next_hir_id(),
                    kind: PatternKind::Const(expr_ref),
                    span,
                }
            }

            // Unit / null / undefined
            NodeKind::Unit | NodeKind::Null | NodeKind::Undefined => {
                let expr = self.lower_expr(ExprContext::PatternConstant, node);
                let expr_ref = self.arena.alloc_expr(expr);
                Pattern {
                    hir_id: self.next_hir_id(),
                    kind: PatternKind::Const(expr_ref),
                    span,
                }
            }

            // Projection-based path as pattern (e.g. Mod.Variant)
            NodeKind::Projection => {
                // A qualified name is a resolved pattern head, not a binding
                // pattern named after the path's first segment. Resolve it in
                // the surrounding scope before publishing payload binders.
                let value = self.lower_expr(ExprContext::PatternConstant, node);
                let value = self.arena.alloc_expr(value);
                Pattern {
                    hir_id: self.next_hir_id(),
                    kind: PatternKind::Const(value),
                    span,
                }
            }

            NodeKind::BitVecBinPattern
            | NodeKind::BitVecOctPattern
            | NodeKind::BitVecHexPattern => {
                let radix = match kind {
                    NodeKind::BitVecBinPattern => 2,
                    NodeKind::BitVecOctPattern => 8,
                    _ => 16,
                };
                let nodes = self
                    .ast
                    .get_multi_child_slice(children[0])
                    .unwrap_or(&[])
                    .to_vec();
                let mut segments = Vec::new();
                for (index, node) in nodes.into_iter().enumerate() {
                    let (kind, segment_span, children) = self.ast.get_node(node).unwrap();
                    let segment = match kind {
                        NodeKind::Int | NodeKind::Id => {
                            let spelling = self
                                .ast
                                .source_content(node, self.source_map)
                                .unwrap_or_default();
                            let spelling = if index == 0
                                && (spelling.starts_with("0b")
                                    || spelling.starts_with("0o")
                                    || spelling.starts_with("0x"))
                            {
                                spelling[2..].to_string()
                            } else {
                                spelling
                            };
                            hir::pattern::BitSegment::Literal {
                                spelling,
                                span: segment_span,
                            }
                        }
                        NodeKind::TypeBoundDeclClause => {
                            let pattern = self.lower_pattern(children[0]);
                            let ty = self.lower_expr(ExprContext::PatternConstant, children[1]);
                            hir::pattern::BitSegment::Typed {
                                pattern: self.arena.alloc_pattern(pattern),
                                ty: self.arena.alloc_expr(ty),
                            }
                        }
                        NodeKind::ExprAsPattern => {
                            let expr = self.lower_expr(ExprContext::PatternConstant, children[0]);
                            hir::pattern::BitSegment::Computed(self.arena.alloc_expr(expr))
                        }
                        _ => {
                            self.emit_invalid_pattern("invalid bit-vector segment", segment_span);
                            return self.make_error_pattern(span);
                        }
                    };
                    segments.push(segment);
                }
                Pattern {
                    hir_id: self.next_hir_id(),
                    kind: PatternKind::BitVec { radix, segments },
                    span,
                }
            }

            other => {
                self.emit_invalid_pattern(&format!("{:?}", other), span);
                self.make_error_pattern(span)
            }
        }
    }

    /// Publish preceding fields only inside this candidate. Elaboration still
    /// checks that references are available in the declaration's match order.
    fn lower_candidate_fields(&mut self, nodes: &[NodeIndex]) -> Vec<FieldPat<'hir>> {
        let scope = self.local_enter(resolve::RibKind::MatchBody);
        let mut fields = Vec::with_capacity(nodes.len());
        for &node in nodes {
            let field = self.lower_field_pattern(node);
            let pattern_node = if self.ast.get_node_kind(node) == Some(NodeKind::PropertyPattern) {
                self.ast.get_children(node)[1]
            } else {
                node
            };
            self.bind_arm_pattern(pattern_node, &field.pat);
            fields.push(field);
        }
        self.local_exit(scope);
        fields
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

// Only lexical success names are supplied here. Type checking, alternative
// consistency and the actual code interface remain elaboration obligations.
fn pattern_output_names(pattern: &Pattern<'_>) -> Vec<hir::common::Ident> {
    let mut pending = vec![pattern];
    let mut names = std::collections::BTreeMap::new();
    while let Some(pattern) = pending.pop() {
        match &pattern.kind {
            PatternKind::Binding(_, name, inner) => {
                names.insert(name.name.to_string(), *name);
                pending.extend(*inner);
            }
            PatternKind::Then(first, _, next) => pending.extend([*first, *next]),
            PatternKind::Guard(inner, _)
            | PatternKind::OptionSome(inner)
            | PatternKind::ErrorOk(inner)
            | PatternKind::ErrorErr(inner) => pending.push(inner),
            PatternKind::Tuple(fields)
            | PatternKind::List(fields, _)
            | PatternKind::AppTuple(_, fields) => pending.extend(*fields),
            PatternKind::Struct(_, fields, _) => {
                pending.extend(fields.iter().map(|field| &field.pat))
            }
            PatternKind::NormalForm(_, inputs) => {
                pending.extend(inputs.iter().map(|input| &input.pattern))
            }
            PatternKind::Or(arms) => pending.extend(arms.first()),
            PatternKind::CodeSplice(_, outputs) => {
                for output in outputs {
                    names.insert(output.name.name.to_string(), output.name);
                }
            }
            PatternKind::BitVec { segments, .. } => {
                for segment in segments {
                    if let hir::pattern::BitSegment::Typed { pattern, .. } = segment {
                        pending.push(pattern);
                    }
                }
            }
            _ => {}
        }
    }
    names.into_values().collect()
}
