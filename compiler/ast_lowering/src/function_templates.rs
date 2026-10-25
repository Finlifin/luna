//! Instance-local inputs linked to declaration-side signatures after lowering.
use crate::{LoweringContext, expr::ExprContext};
use ast::{NodeIndex, NodeKind};
use diagnostic::{DiagnosticContext, FlurryError};
use hir::{
    ExprKind, HirArena, HirId, OwnerId, Package, body::Body, common::Ident, item::*, owner::*,
};
use resolve::{BindingKind, RibKind};

impl<'hir, 'ast, 'resolver: 'ast> LoweringContext<'hir, 'ast, 'resolver> {
    pub(crate) fn lower_function_template_instance(
        &mut self,
        node: NodeIndex,
        modifiers: FnModifiers,
    ) -> OwnerId {
        let (kind, span, children) = self.ast.get_node(node).unwrap();
        let explicit = kind == NodeKind::FunctionTemplateInstanceWithParams;
        if explicit && !modifiers.is_quote {
            self.emit_malformed(
                "explicit function template instance parameters require quote fn",
                span,
            );
            return self.make_error_item(span);
        }
        let template_node = children[if explicit { 2 } else { 1 }];
        let body_node = children[if explicit { 4 } else { 2 }];
        if modifiers.is_comptime {
            self.emit_malformed(
                "function template instances cannot be comptime functions",
                span,
            );
        }
        let owner = self.package.alloc_owner_id();
        let previous = self.current_owner;
        self.current_owner = owner;
        self.ensure_owner_counter();
        let ident = self.node_to_ident(children[0]);
        if let Ok(binding) = self.resolve(ident) {
            if let BindingKind::Definition(definition, _) = binding.kind {
                self.package.bind_definition(definition, owner);
            }
        }
        let template = self.lower_expr(ExprContext::Type, template_node);
        let mut head = &template;
        while let ExprKind::NFApplication(callee, _) = head.kind {
            head = callee;
        }
        let names = match head.kind {
            ExprKind::ResolvedDef { def_id, .. } => self
                .resolver
                .find_scope_for_def(def_id)
                .and_then(|scope| self.resolver.scope_tree().get(scope))
                .and_then(|scope| scope.function_parameters.clone()),
            _ => None,
        };
        let Some(names) = names else {
            self.emit_unsupported_node(
                "function template instance requires a declared parameter interface",
                span,
            );
            self.current_owner = previous;
            return self.make_error_item(span);
        };
        let rib = self.local_enter(RibKind::FnParam);
        let mut parameters = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let explicit_parameters = if explicit {
            let mut lowered = Vec::new();
            for node in self
                .ast
                .get_multi_child_slice(children[1])
                .unwrap_or(&[])
                .to_vec()
            {
                if let Some(mut parameter) = self.lower_fn_param(node) {
                    if !seen.insert(parameter.name.name) {
                        self.emit_malformed(
                            "duplicate function template instance parameter",
                            parameter.span,
                        );
                    }
                    parameter.index = lowered.len() as u32;
                    self.define_fn_param(parameter.name, node, parameter.hir_id);
                    parameters.push(FunctionTemplateInput {
                        name: parameter.name,
                        binding: parameter.hir_id,
                        formal: None,
                    });
                    lowered.push(parameter);
                }
            }
            Some(self.arena.alloc_fn_param_slice(lowered))
        } else {
            let site = self.ast_node_ref(node);
            for (index, (name, span)) in names.into_iter().enumerate() {
                let name = Ident::new(name, span);
                if !seen.insert(name.name) {
                    self.emit_malformed("duplicate function template parameter", span);
                }
                let binding = self.next_hir_id();
                self.expanded_locals.insert((site, index as u32), binding);
                self.define_symbol(name, BindingKind::ExpandedLocal(site, index as u32));
                parameters.push(FunctionTemplateInput {
                    name,
                    binding,
                    formal: None,
                });
            }
            None
        };
        let value = self.lower_expr(ExprContext::RuntimeFunctionBody, body_node);
        let body = self.alloc_body(
            HirId::make_owner(owner),
            Body {
                params: &[],
                value: self.arena.alloc_expr(value),
            },
        );
        self.local_exit(rib);
        let item = self.arena.alloc_item(Item {
            owner_id: owner,
            ident,
            span,
            kind: ItemKind::FunctionTemplateInstance {
                template: self.arena.alloc_expr(template),
                signature: None,
                explicit_parameters,
                parameters,
                modifiers,
                body,
            },
        });
        self.package.insert_owner(
            owner,
            OwnerInfo {
                node: OwnerNode::Item(item),
                nodes: OwnerNodes::new(),
            },
        );
        self.package
            .pending_semantics
            .push((span, "function template expansion".into()));
        self.current_owner = previous;
        owner
    }
}

pub(crate) fn link<'hir>(
    package: &mut Package<'hir>,
    arena: &'hir HirArena,
    diagnostics: &DiagnosticContext<'_>,
) {
    let instances = package
        .owners()
        .filter_map(|(owner, info)| {
            matches!(
                info.node.expect_item().kind,
                ItemKind::FunctionTemplateInstance { .. }
            )
            .then_some(owner)
        })
        .collect::<Vec<_>>();
    for owner in instances {
        let mut item = package.item(owner).unwrap().clone();
        let ItemKind::FunctionTemplateInstance {
            template,
            signature,
            explicit_parameters,
            parameters,
            ..
        } = &mut item.kind
        else {
            unreachable!()
        };
        let checked = (|| {
            let mut head = *template;
            while let ExprKind::NFApplication(callee, _) = head.kind {
                head = callee;
            }
            let ExprKind::ResolvedDef { def_id, .. } = head.kind else {
                return Err("function template head must have a declaration identity");
            };
            let declaration = package
                .definition_owner(def_id)
                .and_then(|owner| package.item(owner))
                .ok_or("function template declaration is missing from HIR")?;
            let (ItemKind::Fn(formal, _) | ItemKind::FnDeclaration(formal)) = &declaration.kind
            else {
                return Err("function instance target is not a template declaration");
            };
            if !formal.modifiers.is_function_template {
                return Err("function instance target is not a function template");
            }
            if parameters.len() != formal.params.len() {
                return Err("function template instance parameter count mismatch");
            }
            for (input, parameter) in parameters.iter_mut().zip(formal.params) {
                if explicit_parameters.is_none() && input.name.name != parameter.name.name {
                    return Err("function template inherited parameter name mismatch");
                }
                input.formal = Some(parameter.hir_id);
            }
            *signature = Some(formal.clone());
            Ok(())
        })();
        if let Err(message) = checked {
            crate::LoweringError::malformed_ast(message, item.span)
                .emit(diagnostics, rustc_span::BytePos(0));
        }
        let mut info = package.owner(owner).unwrap().clone();
        info.node = OwnerNode::Item(arena.alloc_item(item));
        package.insert_owner(owner, info);
    }
}
