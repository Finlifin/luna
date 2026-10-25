//! Item lowering — AST top-level definition nodes → HIR [`Item`].
//!
//! Each item definition in the AST creates a new **owner** in the HIR
//! [`Package`]. The owner gets a unique [`OwnerId`], and the item's
//! subtree (expressions, patterns, etc.) is allocated into the HIR arena
//! under that owner.
//!
//! **Key design point**: In Flurry, **clause declarations** play the role
//! of generic parameters. A function like
//!
//! ```text
//! fn map<T, U>(xs: [T], f: fn(T) -> U) -> [U]
//! ```
//!
//! has `T` and `U` declared in its clause list. During lowering we split
//! the clause list into `ClauseParam`s (generic type parameters) and
//! `ClauseConstraint`s (where-clause bounds).

use ast::{NodeIndex, NodeKind};
use hir::{
    body::{Body, BodyParam},
    common::{
        FnParam, Ident, Symbol, TPARAM_COMPTIME, TPARAM_ERROR, TPARAM_IMPLICIT, TPARAM_LAMBDA,
        TPARAM_QUOTE, TyParam, TyParamKind,
    },
    expr::{Expr, ExprKind},
    hir_id::{HirId, ItemLocalId, OwnerId},
    item::*,
    owner::{OwnerInfo, OwnerNode, OwnerNodes},
};
use resolve::RibKind;
use rustc_span::Span;

use crate::{LoweringContext, expr::ExprContext};

impl<'hir, 'ast, 'resolver: 'ast> LoweringContext<'hir, 'ast, 'resolver> {
    /// Lower a `FileScope` (the root AST node) into the HIR [`Package`].
    ///
    /// Every top-level statement/definition in the file becomes an item
    /// (or statement) inside the root module.
    pub fn lower_file_scope(&mut self, root_node: NodeIndex) {
        let Some((kind, span, children)) = self.ast.get_node(root_node) else {
            unreachable!(
                "expected FileScope as root node, but no such node index {:?}",
                root_node
            );
        };

        if kind != NodeKind::FileScope {
            self.emit_malformed("expected FileScope as root node", span);
            return;
        }

        if children.is_empty() {
            return;
        }

        let elems_node = children[0];
        let elem_nodes = self.ast.get_multi_child_slice(elems_node).unwrap_or(&[]);

        // Allocate the root module owner.
        let root_owner_id = self.package.alloc_owner_id();
        self.package.root_mod = root_owner_id;
        if let Some(scope) = self.resolver.scope_tree().get(self.file_scope) {
            self.package.bind_definition(scope.owner_def, root_owner_id);
        }
        self.current_owner = root_owner_id;

        // Lower each top-level node.
        let mut item_ids = Vec::new();
        // Copy elem_nodes to a Vec so we don't borrow self immutably and mutably
        let elem_nodes_vec: Vec<NodeIndex> = elem_nodes.to_vec();
        for &elem in &elem_nodes_vec {
            if elem == 0 {
                continue;
            }

            if self.lower_definition_configuration(elem) {
                continue;
            }
            let owner = self.lower_top_level_node(elem);
            item_ids.push(owner);
        }

        // Build the root module item.
        self.current_owner = root_owner_id;
        self.ensure_owner_counter();

        let mod_def = ModDef { items: item_ids };
        let module_name = self
            .resolver
            .scope_tree()
            .get(self.file_scope)
            .and_then(|s| s.name)
            .unwrap_or(Symbol::intern("<root>"));
        let ident = Ident::new(module_name, span);
        let item = Item {
            owner_id: root_owner_id,
            ident,
            kind: ItemKind::Mod(mod_def),
            span,
        };
        let item_ref = self.arena.alloc_item(item);
        let owner_info = OwnerInfo {
            node: OwnerNode::Item(item_ref),
            nodes: OwnerNodes::new(),
        };
        self.package.insert_owner(root_owner_id, owner_info);
    }

    /// Lower a single top-level AST node (definition or statement) into an
    /// HIR item and return its [`OwnerId`].
    fn lower_top_level_node(&mut self, node: NodeIndex) -> OwnerId {
        let Some((kind, span, children)) = self.ast.get_node(node) else {
            unreachable!("invalid top-level node: no such node index {:?}", node);
        };

        match kind {
            NodeKind::FunctionTemplateInstance | NodeKind::FunctionTemplateInstanceWithParams => {
                self.lower_function_template_instance(node, FnModifiers::default())
            }
            NodeKind::FunctionTemplateDef => {
                let owner = self.lower_top_level_node(children[0]);
                let mut item = self
                    .package
                    .item(owner)
                    .expect("lowered template item")
                    .clone();
                match &mut item.kind {
                    ItemKind::Fn(signature, _) | ItemKind::FnDeclaration(signature) => {
                        if signature.modifiers.is_comptime {
                            self.emit_malformed(
                                "function templates cannot define comptime functions",
                                span,
                            );
                        }
                        signature.modifiers.is_function_template = true;
                    }
                    _ => self
                        .emit_malformed("function template requires a function declaration", span),
                }
                let mut info = self.package.owner(owner).expect("template owner").clone();
                info.node = OwnerNode::Item(self.arena.alloc_item(item));
                self.package.insert_owner(owner, info);
                self.package
                    .pending_semantics
                    .push((span, "function template expansion".into()));
                owner
            }
            NodeKind::Function => self.lower_function(node, FnModifiers::default()),
            NodeKind::TestDef => self.lower_function(
                node,
                FnModifiers {
                    is_test: true,
                    ..Default::default()
                },
            ),
            NodeKind::TestTemplateDef => {
                self.package
                    .pending_semantics
                    .push((span, "test template CMTT expansion".into()));
                self.lower_function(
                    node,
                    FnModifiers {
                        is_test_template: true,
                        ..Default::default()
                    },
                )
            }
            NodeKind::TestTemplateInstance => self.lower_test_template_instance(node),
            NodeKind::AlgebraicEffect => self.lower_callable(
                node,
                FnModifiers::default(),
                Some(EffectModifiers::default()),
            ),
            NodeKind::EffectEnumDef => self.lower_effect_group(node),
            NodeKind::AssocDecl => self.lower_associated(node),
            NodeKind::NormalFormHeadDef => self.lower_normal_form_head_def(node),
            NodeKind::StructDef => self.lower_struct_def(node),
            NodeKind::EnumDef => self.lower_enum_def(node),
            NodeKind::TraitDef => self.lower_trait_def(node),
            NodeKind::ImplDef => self.lower_impl_def(node),
            NodeKind::ImplTraitDef => self.lower_impl_trait_def(node),
            NodeKind::TypealiasDef | NodeKind::NewtypeDef => self.lower_named_type(node),
            NodeKind::ModuleDef => self.lower_module_def(node),
            NodeKind::UseStatement => self.lower_use_statement(node),

            // Attribute-wrapped definitions
            NodeKind::Attribute | NodeKind::AttributeSetTrue => {
                if self.ast.get_node_kind(children[0]) == Some(NodeKind::Id)
                    && self.node_to_ident(children[0]).name.as_str() == "intrinsic_enum"
                {
                    let owner = self.lower_top_level_node(children[1]);
                    let mut item = self
                        .package
                        .item(owner)
                        .expect("intrinsic enum target")
                        .clone();
                    match &mut item.kind {
                        ItemKind::Enum(definition) => definition.is_intrinsic = true,
                        _ => {
                            self.emit_malformed("intrinsic_enum requires an enum declaration", span)
                        }
                    }
                    let mut info = self
                        .package
                        .owner(owner)
                        .expect("intrinsic enum owner")
                        .clone();
                    info.node = OwnerNode::Item(self.arena.alloc_item(item));
                    self.package.insert_owner(owner, info);
                    return owner;
                }
                // Recognize the supported phase modifiers and the built-in
                // direct-passing attribute. Unknown attributes stay pending.
                let mut cursor = node;
                let mut modifiers = FnModifiers::default();
                let mut effect_modifiers = EffectModifiers::default();
                while let Some((
                    kind @ (NodeKind::AttributeSetTrue | NodeKind::Attribute),
                    _,
                    parts,
                )) = self.ast.get_node(cursor)
                {
                    if kind == NodeKind::Attribute {
                        let mut path = parts[0];
                        let mut matches = true;
                        for name in ["direct_passing", "effects"] {
                            if self.ast.get_node_kind(path) != Some(NodeKind::Projection) {
                                matches = false;
                                break;
                            }
                            let fields = self.ast.get_children(path);
                            if self.ast.get_node_kind(fields[1]) != Some(NodeKind::Id)
                                || self.node_to_ident(fields[1]).name.as_str() != name
                            {
                                matches = false;
                                break;
                            }
                            path = fields[0];
                        }
                        if !matches
                            || self.ast.get_node_kind(path) != Some(NodeKind::Id)
                            || self.node_to_ident(path).name.as_str() != "attrs"
                        {
                            break;
                        }
                        effect_modifiers.is_direct_passing = true;
                        cursor = parts[1];
                        continue;
                    }
                    if self.ast.get_node_kind(parts[0]) != Some(NodeKind::Id) {
                        break;
                    }
                    let name = self.node_to_ident(parts[0]).name;
                    match name.as_str() {
                        "__flurry_kw_comptime" => modifiers.is_comptime = true,
                        "__flurry_kw_quote" => modifiers.is_quote = true,
                        "__flurry_kw_pure" => modifiers.is_pure = true,
                        "__flurry_kw_unsafe" => modifiers.is_unsafe = true,
                        "__flurry_kw_async" => effect_modifiers.is_async = true,
                        "__flurry_kw_inline" => effect_modifiers.is_inline = true,
                        _ => break,
                    }
                    cursor = parts[1];
                }
                if self.ast.get_node_kind(cursor) == Some(NodeKind::AlgebraicEffect)
                    && !modifiers.is_comptime
                    && !modifiers.is_pure
                    && !modifiers.is_quote
                    && !modifiers.is_unsafe
                {
                    return self.lower_callable(
                        cursor,
                        FnModifiers::default(),
                        Some(effect_modifiers),
                    );
                }
                if (modifiers.is_comptime
                    || modifiers.is_pure
                    || modifiers.is_quote
                    || modifiers.is_unsafe
                    || effect_modifiers.is_inline)
                    && !effect_modifiers.is_async
                    && !effect_modifiers.is_direct_passing
                    && matches!(
                        self.ast.get_node_kind(cursor),
                        Some(
                            NodeKind::Function
                                | NodeKind::FunctionTemplateInstance
                                | NodeKind::FunctionTemplateInstanceWithParams
                        )
                    )
                {
                    if modifiers.is_quote && modifiers.is_comptime {
                        self.emit_malformed("quote fn is not a comptime fn", span);
                    }
                    if modifiers.is_unsafe && modifiers.is_comptime {
                        self.package
                            .pending_semantics
                            .push((span, "unsafe staged function admission".into()));
                    }
                    modifiers.is_inline = effect_modifiers.is_inline;
                    if self.ast.get_node_kind(cursor) != Some(NodeKind::Function) {
                        return self.lower_function_template_instance(cursor, modifiers);
                    }
                    return self.lower_function(cursor, modifiers);
                }

                self.package.pending_semantics.push((
                    span,
                    if kind == NodeKind::Attribute {
                        "definition attribute"
                    } else {
                        "definition attribute or modifier"
                    }
                    .into(),
                ));
                let lexical_owner = self.current_owner;
                let target = self.lower_top_level_node(children[1]);
                // Recurse first: closer attributes apply first. Keep the
                // original expression and lexical owner for later evaluation;
                // lowering the target must not move lookup into its body.
                if kind == NodeKind::Attribute {
                    let value = self.lower_expr(ExprContext::Attribute, children[0]);
                    let value = self.arena.alloc_expr(value);
                    let expression = self.source_text(children[0]);
                    let expression_span = self.ast.get_span(children[0]).unwrap_or(span);
                    self.package
                        .definition_attributes
                        .push(hir::DeferredDefinitionAttribute {
                            target: hir::AttributeTarget::Definition(target),
                            lexical_owner,
                            expression,
                            value,
                            configuration_key: None,
                            span: expression_span,
                        });
                }
                target
            }

            NodeKind::ConstDef => self.lower_const_def(node),
            // TODO: parser还不是能很好地区分ConstDef和ConstDecl，先将两者都当成ConstDef来处理，后续再完善parser以区分两者
            NodeKind::ConstDecl => self.lower_const_def(node),

            // Visibility modified definitions, TODO: handle the visibility modifier properly instead of just skipping it
            NodeKind::Pub | NodeKind::Private => self.lower_top_level_node(children[0]),

            other => {
                self.emit_unsupported_node(&format!("top-level {other}"), span);
                self.make_error_item(span)
            }
        }
    }

    fn lower_use_statement(&mut self, node: NodeIndex) -> OwnerId {
        let span = self.ast.get_span(node).unwrap();
        let owner_id = self.package.alloc_owner_id();
        let mut cursor = self.ast.get_children(node)[0];
        let mut anchor = hir::common::PathAnchor::Local;
        loop {
            match self.ast.get_node_kind(cursor) {
                Some(NodeKind::SuperPath) => {
                    anchor = match anchor {
                        hir::common::PathAnchor::Super(n) => hir::common::PathAnchor::Super(n + 1),
                        _ => hir::common::PathAnchor::Super(1),
                    };
                    cursor = self.ast.get_children(cursor)[0];
                }
                Some(NodeKind::PackagePath) => {
                    anchor = hir::common::PathAnchor::Package;
                    cursor = self.ast.get_children(cursor)[0];
                }
                _ => break,
            }
        }
        let mut segments = Vec::new();
        fn walk<'h, 'a, 'r: 'a>(
            ctx: &mut LoweringContext<'h, 'a, 'r>,
            node: NodeIndex,
            segments: &mut Vec<hir::common::PathSegment<'h>>,
        ) -> UseKind<'h> {
            let children = ctx.ast.get_children(node);
            match ctx.ast.get_node_kind(node) {
                Some(NodeKind::Id) => {
                    segments.push(hir::common::PathSegment {
                        ident: ctx.node_to_ident(node),
                        args: &[],
                    });
                    UseKind::Simple
                }
                Some(NodeKind::ProjectionPath) => {
                    walk(ctx, children[0], segments);
                    walk(ctx, children[1], segments)
                }
                Some(NodeKind::PathAsBind) => {
                    walk(ctx, children[0], segments);
                    UseKind::Alias(ctx.node_to_ident(children[1]))
                }
                Some(NodeKind::ProjectionAllPath) => {
                    walk(ctx, children[0], segments);
                    UseKind::Glob
                }
                Some(NodeKind::ProjectionMultiPath) => {
                    walk(ctx, children[0], segments);
                    let nodes = ctx
                        .ast
                        .get_multi_child_slice(children[1])
                        .unwrap_or(&[])
                        .to_vec();
                    if nodes
                        .iter()
                        .all(|&node| ctx.ast.get_node_kind(node) == Some(NodeKind::Id))
                    {
                        let names = nodes.iter().map(|&n| ctx.node_to_ident(n)).collect();
                        UseKind::Multi(ctx.arena.alloc_ident_slice(names))
                    } else {
                        let mut paths = Vec::new();
                        for node in nodes {
                            let mut segments = Vec::new();
                            let kind = walk(ctx, node, &mut segments);
                            let span = ctx.ast.get_span(node).unwrap();
                            paths.push(UsePath {
                                path: hir::common::Path {
                                    anchor: hir::common::PathAnchor::Local,
                                    segments: ctx.arena.alloc_path_segment_slice(segments),
                                    span,
                                    res: None,
                                },
                                kind,
                                span,
                            });
                        }
                        UseKind::Group(ctx.arena.alloc_use_path_slice(paths))
                    }
                }
                _ => {
                    ctx.emit_unsupported_node("import path form", ctx.ast.get_span(node).unwrap());
                    UseKind::Simple
                }
            }
        }
        let kind = walk(self, cursor, &mut segments);
        let path = hir::common::Path {
            anchor,
            segments: self.arena.alloc_path_segment_slice(segments),
            span,
            res: None,
        };
        let item = self.arena.alloc_item(Item {
            owner_id,
            ident: Ident::new(Symbol::intern("<use>"), span),
            kind: ItemKind::Use(UsePath { path, kind, span }),
            span,
        });
        self.package.insert_owner(
            owner_id,
            OwnerInfo {
                node: OwnerNode::Item(item),
                nodes: OwnerNodes::new(),
            },
        );
        owner_id
    }

    /// Bind the declaration being lowered to its scanned identity, not to a
    /// sequential local that happens to shadow its spelling at this point.
    fn resolve_declared(&self, ident: Ident) -> resolve::ResolveResult<resolve::Resolution> {
        self.resolver
            .scope_tree()
            .get(self.file_scope)
            .and_then(|scope| scope.items.get_direct(ident.name))
            .map(resolve::Resolution::from_binding)
            .ok_or_else(|| resolve::ResolveError::UnresolvedIdentifier {
                name: ident.name.to_string(),
                span: ident.span,
            })
    }

    /// Lower an item that appears inside a block (definition inside a
    /// function body, struct body, etc.)  Returns the new [`OwnerId`].
    pub fn lower_item_in_block(&mut self, node: NodeIndex) -> OwnerId {
        let parent = self.current_owner;
        let owner = self.lower_top_level_node(node);
        self.package.lexical_parents.insert(owner, parent);
        owner
    }

    fn lower_test_template_instance(&mut self, node: NodeIndex) -> OwnerId {
        let span = self.ast.get_span(node).unwrap();
        let children = self.ast.get_children(node);
        let owner_id = self.package.alloc_owner_id();
        let previous_owner = self.current_owner;
        self.current_owner = owner_id;
        self.ensure_owner_counter();
        let template = self.lower_expr(ExprContext::Type, children[1]);
        let mut head = &template;
        while let ExprKind::NFApplication(callee, _) = &head.kind {
            head = callee;
        }
        let names = match head.kind {
            ExprKind::ResolvedDef { def_id, .. } => self
                .resolver
                .find_scope_for_def(def_id)
                .and_then(|scope| self.resolver.scope_tree().get(scope))
                .and_then(|scope| scope.test_template_context.clone()),
            _ => None,
        };
        let Some(names) = names else {
            self.emit_unsupported_node(
                "test template instance requires a declared contextual body interface",
                span,
            );
            self.current_owner = previous_owner;
            return self.make_error_item(span);
        };
        let ident = Ident::new(
            resolve::test_definition_name(
                self.file_id,
                node,
                Some(self.node_to_ident(children[0]).name),
            ),
            span,
        );
        if let Ok(binding) = self.resolve_declared(ident) {
            if let resolve::BindingKind::Definition(definition, _) = binding.kind {
                self.package.bind_definition(definition, owner_id);
            }
        } else {
            self.emit_malformed(
                "test template instance requires a scanned declaration scope",
                span,
            );
        }
        let rib = self.local_enter(RibKind::FnParam);
        let site = self.ast_node_ref(node);
        let mut ports = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (index, (name, name_span)) in names.into_iter().enumerate() {
            if !seen.insert(name) {
                self.emit_malformed("duplicate test template context binding", name_span);
                continue;
            }
            let name = Ident::new(name, name_span);
            let binding = self.next_hir_id();
            let index = index as u32;
            self.expanded_locals.insert((site, index), binding);
            self.define_symbol(name, resolve::BindingKind::ExpandedLocal(site, index));
            ports.push(TestTemplatePort {
                name,
                binding,
                formal: None,
            });
        }
        let value = self.lower_expr(ExprContext::RuntimeFunctionBody, children[2]);
        let value = self.arena.alloc_expr(value);
        let body = self.alloc_body(HirId::make_owner(owner_id), Body { params: &[], value });
        self.local_exit(rib);
        let template = self.arena.alloc_expr(template);
        let item = self.arena.alloc_item(Item {
            owner_id,
            ident,
            span,
            kind: ItemKind::TestTemplateInstance {
                interface: None,
                template,
                ports,
                body,
            },
        });
        self.package.insert_owner(
            owner_id,
            OwnerInfo {
                node: OwnerNode::Item(item),
                nodes: OwnerNodes::new(),
            },
        );
        self.package
            .pending_semantics
            .push((span, "test template CMTT expansion".into()));
        self.current_owner = previous_owner;
        owner_id
    }

    /// Lower `Function`: a, N, b, N, c
    ///   (id, params, return_type, clauses, body)
    fn lower_function(&mut self, node: NodeIndex, modifiers: FnModifiers) -> OwnerId {
        self.lower_callable(node, modifiers, None)
    }

    fn lower_callable(
        &mut self,
        node: NodeIndex,
        modifiers: FnModifiers,
        effect: Option<EffectModifiers>,
    ) -> OwnerId {
        let Some((kind, span, children)) = self.ast.get_node(node) else {
            unreachable!("invalid function node or no such node index {:?}", node);
        };

        let id_node = children[0];
        let params_multi = if modifiers.is_test || modifiers.is_test_template {
            0
        } else {
            children[1]
        };
        let return_type_node = if modifiers.is_test || modifiers.is_test_template {
            0
        } else {
            children[2]
        };
        debug_assert!(matches!(
            kind,
            NodeKind::Function
                | NodeKind::AlgebraicEffect
                | NodeKind::TestDef
                | NodeKind::TestTemplateDef
        ));
        let clauses_multi = if modifiers.is_test {
            0
        } else if modifiers.is_test_template {
            children[1]
        } else {
            children[3]
        };
        let body_node = if modifiers.is_test {
            children[1]
        } else if modifiers.is_test_template {
            children[2]
        } else {
            children[4]
        };

        // Allocate owner
        let owner_id = self.package.alloc_owner_id();
        let prev_owner = self.current_owner;
        self.current_owner = owner_id;
        self.ensure_owner_counter();

        // Identifier
        let ident = if modifiers.is_test {
            Ident {
                name: resolve::test_definition_name(
                    self.file_id,
                    node,
                    if id_node == 0 {
                        None
                    } else {
                        Some(self.node_to_ident(id_node).name)
                    },
                ),
                span,
            }
        } else {
            self.node_to_ident(id_node)
        };
        let previous_scope = self.file_scope;
        let resolved = self.resolve_declared(ident);
        if modifiers.is_test && resolved.is_err() {
            self.emit_malformed("test declaration requires a scanned declaration scope; tests inside runtime blocks are not supported", span);
        }
        if let Ok(binding) = resolved {
            if let resolve::BindingKind::Definition(definition, _) = binding.kind {
                self.package.bind_definition(definition, owner_id);
                if let Some(scope) = self.resolver.find_scope_for_def(definition) {
                    self.file_scope = scope;
                }
            }
        }

        let rib_index = self.local_enter(RibKind::FnParam);

        // Parameters
        let param_nodes = self.ast.get_multi_child_slice(params_multi).unwrap_or(&[]);
        let mut fn_params = Vec::new();
        for param_node in param_nodes.to_vec() {
            if let Some(mut param) = self.lower_fn_param(param_node) {
                if fn_params
                    .iter()
                    .any(|previous: &FnParam<'hir>| previous.name.name == param.name.name)
                {
                    self.emit_malformed("duplicate function parameter", param.span);
                }
                param.index = fn_params.len() as u32;
                self.define_fn_param(param.name, param_node, param.hir_id);
                let site = self.ast_node_ref(param_node);
                if let Some(definition) = self
                    .resolver
                    .scope_tree()
                    .get(self.file_scope)
                    .and_then(|scope| {
                        scope
                            .staged_parameters
                            .iter()
                            .find(|(source, _)| *source == site)
                    })
                    .map(|(_, definition)| *definition)
                {
                    self.package
                        .staged_parameter_definitions
                        .insert(param.hir_id, definition);
                }
                fn_params.push(param);
            }
        }

        let (return_type_node, result_binding) = match self.ast.get_node(return_type_node) {
            Some((NodeKind::ResultWithId, _, children)) => {
                let name = self.node_to_ident(children[0]);
                if fn_params.iter().any(|p| p.name.name == name.name) {
                    self.emit_malformed("result name duplicates a function parameter", name.span);
                }
                (children[1], Some((name, children[0], self.next_hir_id())))
            }
            _ => (return_type_node, None),
        };

        // Clauses → generic params + constraints
        let clause_nodes = self.ast.get_multi_child_slice(clauses_multi).unwrap_or(&[]);
        let lowered = self.lower_clauses_with_result(clause_nodes, result_binding);
        let clause_params = self.arena.alloc_clause_param_slice(lowered.params);
        let clause_constraints = self.arena.alloc_clause_slice(lowered.constraints);

        // Return type
        let return_ty = if return_type_node != 0 {
            let ty_expr = self.lower_expr(ExprContext::Type, return_type_node);
            Some(self.arena.alloc_expr(ty_expr) as &_)
        } else {
            None
        };

        // Build FnSig
        let params_slice = self.arena.alloc_fn_param_slice(fn_params);

        let fn_sig = FnSig {
            params: params_slice,
            return_ty,
            return_bind: result_binding.map(|(name, _, _)| name),
            return_binding: result_binding.map(|(_, _, binding)| binding),
            modifiers,
            clause_params,
            clause_constraints,
            span,
        };

        if let Err(message) = fn_sig.test_template_body() {
            self.emit_malformed(message, span);
        }

        if body_node == 0 {
            let item_ref = self.arena.alloc_item(Item {
                owner_id,
                ident,
                kind: match effect {
                    Some(modifiers) => ItemKind::Effect(fn_sig, modifiers),
                    None => ItemKind::FnDeclaration(fn_sig),
                },
                span,
            });
            self.package.insert_owner(
                owner_id,
                OwnerInfo {
                    node: OwnerNode::Item(item_ref),
                    nodes: OwnerNodes::new(),
                },
            );
            self.local_exit(rib_index);
            self.file_scope = previous_scope;
            self.current_owner = prev_owner;
            return owner_id;
        }

        // A code result provides virtual runtime names to quoted bodies.
        if let Some(hir::Expr {
            kind: ExprKind::ContextualExprType(bindings, _),
            ..
        }) = return_ty
        {
            let parts = self.ast.get_children(return_type_node);
            for (binding, &node) in bindings
                .iter()
                .zip(self.ast.get_multi_child_slice(parts[1]).unwrap_or(&[]))
            {
                self.define_local(binding.name, node, binding.hir_id);
            }
        }
        let saved_template_ports = self.test_template_ports.clone();
        if let Ok(Some(parameter)) = fn_sig.test_template_body() {
            let mut kind = &parameter.kind;
            while let hir::ClauseParamKind::Quote(inner) = kind {
                kind = inner;
            }
            if let hir::ClauseParamKind::Positional(_, ty) = kind {
                if let ExprKind::ContextualExprType(entries, _) = &ty.kind {
                    let mut names = std::collections::HashSet::new();
                    for entry in entries {
                        if !names.insert(entry.name.name) {
                            self.emit_malformed(
                                "duplicate test template context binding",
                                entry.name.span,
                            );
                        }
                    }
                    if let Some(definition) = parameter.definition {
                        self.test_template_ports
                            .insert(definition, entries.iter().map(|entry| entry.name).collect());
                    }
                }
            }
        }
        // Body
        let body_expr = self.lower_expr(
            if modifiers.is_comptime {
                ExprContext::ComptimeFunctionBody
            } else {
                ExprContext::RuntimeFunctionBody
            },
            body_node,
        );
        self.test_template_ports = saved_template_ports;
        let body_expr_ref = self.arena.alloc_expr(body_expr);

        // Build Body with params
        let body_params: Vec<BodyParam<'hir>> = params_slice
            .iter()
            .map(|param| BodyParam::from_fn_param(param))
            .collect();
        let body_params_slice = self.arena.alloc_param_slice(body_params);
        let body = Body {
            params: body_params_slice,
            value: body_expr_ref,
        };
        let owner_hir_id = HirId::new(owner_id, ItemLocalId::new(0));
        let body_id = self.alloc_body(owner_hir_id, body);

        // Item
        let item = Item {
            owner_id,
            ident,
            kind: ItemKind::Fn(fn_sig, body_id),
            span,
        };
        let item_ref = self.arena.alloc_item(item);
        let owner_info = OwnerInfo {
            node: OwnerNode::Item(item_ref),
            nodes: OwnerNodes::new(),
        };
        self.package.insert_owner(owner_id, owner_info);

        self.local_exit(rib_index);

        self.file_scope = previous_scope;
        self.current_owner = prev_owner;
        owner_id
    }

    /// Lower `NormalFormHeadDef`: a, N, b, N, c
    ///   (id, type_params, return_type, clauses, body)
    fn lower_normal_form_head_def(&mut self, node: NodeIndex) -> OwnerId {
        let Some((NodeKind::NormalFormHeadDef, span, children)) = self.ast.get_node(node) else {
            unreachable!(
                "not a valid normal form def node or no such node index {:?}",
                node
            );
        };

        let id_node = children[0];
        let type_params_multi = children[1];
        let return_type_node = children[2];
        let clauses_multi = children[3];
        let body_node = children[4];

        let owner_id = self.package.alloc_owner_id();
        let prev_owner = self.current_owner;
        self.current_owner = owner_id;
        self.ensure_owner_counter();

        let ident = self.node_to_ident(id_node);
        let previous_scope = self.file_scope;
        if let Ok(binding) = self.resolve_declared(ident) {
            if let resolve::BindingKind::Definition(definition, _) = binding.kind {
                self.package.bind_definition(definition, owner_id);
                if let Some(scope) = self.resolver.find_scope_for_def(definition) {
                    self.file_scope = scope;
                }
            }
        }

        let rib_index = self.local_enter(RibKind::FnParam);
        // Type params serve as the function's clause params
        let type_param_nodes = self
            .ast
            .get_multi_child_slice(type_params_multi)
            .unwrap_or(&[]);
        let clause_nodes = self.ast.get_multi_child_slice(clauses_multi).unwrap_or(&[]);

        // Merge type params and clauses
        let mut all_clause_nodes = type_param_nodes.to_vec();
        all_clause_nodes.extend_from_slice(clause_nodes);
        let lowered = self.lower_clauses(&all_clause_nodes);
        let clause_params = self.arena.alloc_clause_param_slice(lowered.params);
        let clause_constraints = self.arena.alloc_clause_slice(lowered.constraints);

        let (return_type_node, return_bind, return_binding) =
            match self.ast.get_node(return_type_node) {
                Some((NodeKind::ResultWithId, _, children)) => (
                    children[1],
                    Some(self.node_to_ident(children[0])),
                    Some(self.next_hir_id()),
                ),
                _ => (return_type_node, None, None),
            };
        let return_ty = if return_type_node != 0 {
            let ty_expr = self.lower_expr(ExprContext::Type, return_type_node);
            Some(self.arena.alloc_expr(ty_expr) as &_)
        } else {
            None
        };

        let modifiers = FnModifiers {
            is_normal_form_head: true,
            is_comptime: true,
            is_pure: true,
            ..Default::default()
        };

        let fn_sig = FnSig {
            params: &[],
            return_ty,
            return_bind,
            return_binding,
            modifiers,
            clause_params,
            clause_constraints,
            span,
        };

        let body_expr = self.lower_expr(ExprContext::ComptimeFunctionBody, body_node);
        let body_expr_ref = self.arena.alloc_expr(body_expr);
        let body = Body {
            params: &[],
            value: body_expr_ref,
        };
        let owner_hir_id = HirId::new(owner_id, ItemLocalId::new(0));
        let body_id = self.alloc_body(owner_hir_id, body);

        let item = Item {
            owner_id,
            ident,
            kind: ItemKind::Fn(fn_sig, body_id),
            span,
        };
        let item_ref = self.arena.alloc_item(item);
        self.package.insert_owner(
            owner_id,
            OwnerInfo {
                node: OwnerNode::Item(item_ref),
                nodes: OwnerNodes::new(),
            },
        );

        self.local_exit(rib_index);
        self.file_scope = previous_scope;
        self.current_owner = prev_owner;
        owner_id
    }

    /// Lower `StructDef`: a, N, b  (id, clauses, body)
    fn lower_struct_def(&mut self, node: NodeIndex) -> OwnerId {
        let Some((NodeKind::StructDef, span, children)) = self.ast.get_node(node) else {
            unreachable!(
                "not a valid struct def node or no such node index {:?}",
                node
            );
        };

        let id_node = children[0];
        let clauses_multi = children[1];
        let body_node = children[2];

        let owner_id = self.package.alloc_owner_id();
        let prev_owner = self.current_owner;
        self.current_owner = owner_id;
        self.ensure_owner_counter();

        let ident = self.node_to_ident(id_node);
        let previous_scope = self.file_scope;
        if let Ok(binding) = self.resolve_declared(ident) {
            if let resolve::BindingKind::Definition(definition, _) = binding.kind {
                self.package.bind_definition(definition, owner_id);
                if let Some(scope) = self.resolver.find_scope_for_def(definition) {
                    self.file_scope = scope;
                }
            }
        }

        // Clauses (generic params)
        let clause_nodes = self.ast.get_multi_child_slice(clauses_multi).unwrap_or(&[]);
        let lowered = self.lower_clauses(clause_nodes);
        let clause_params = self.arena.alloc_clause_param_slice(lowered.params);
        let clause_constraints = self.arena.alloc_clause_slice(lowered.constraints);

        // Body: a Block containing struct fields and nested definitions
        let (fields, nested_items) = self.lower_struct_body(body_node);
        let fields_slice = self.arena.alloc_field_def_slice(fields);

        let struct_def = StructDef {
            fields: fields_slice,
            clause_params,
            clause_constraints,
            nested_items,
        };

        let item = Item {
            owner_id,
            ident,
            kind: ItemKind::Struct(struct_def),
            span,
        };
        let item_ref = self.arena.alloc_item(item);
        self.package.insert_owner(
            owner_id,
            OwnerInfo {
                node: OwnerNode::Item(item_ref),
                nodes: OwnerNodes::new(),
            },
        );

        self.file_scope = previous_scope;
        self.current_owner = prev_owner;
        owner_id
    }

    // Keep callable modifier wrappers intact so the shared item lowering path
    // recognizes phase keywords and diagnoses unsupported attributes.
    fn is_wrapped_callable_member(&self, mut node: NodeIndex) -> bool {
        loop {
            match self.ast.get_node_kind(node) {
                Some(NodeKind::Function | NodeKind::NormalFormHeadDef) => return true,
                Some(NodeKind::Pub) => {
                    let Some(inner) = self.ast.get_children(node).first() else {
                        return false;
                    };
                    node = *inner;
                }
                Some(NodeKind::Attribute | NodeKind::AttributeSetTrue) => {
                    let Some(inner) = self.ast.get_children(node).get(1) else {
                        return false;
                    };
                    node = *inner;
                }
                _ => return false,
            }
        }
    }

    fn is_wrapped_namespace_member(&self, mut node: NodeIndex) -> bool {
        loop {
            match self.ast.get_node_kind(node) {
                Some(NodeKind::UseStatement | NodeKind::ModuleDef) => return true,
                Some(NodeKind::Pub | NodeKind::Private) => {
                    let Some(inner) = self.ast.get_children(node).first() else {
                        return false;
                    };
                    node = *inner;
                }
                Some(NodeKind::Attribute | NodeKind::AttributeSetTrue) => {
                    let Some(inner) = self.ast.get_children(node).get(1) else {
                        return false;
                    };
                    node = *inner;
                }
                _ => return false,
            }
        }
    }

    /// Preserve configuration without inventing a field or child definition.
    fn lower_definition_configuration(&mut self, node: NodeIndex) -> bool {
        if self.ast.get_node_kind(node) != Some(NodeKind::Configuration) {
            return false;
        }
        let children = self.ast.get_children(node);
        let span = self.ast.get_span(node).unwrap_or_default();
        let value_span = self.ast.get_span(children[1]).unwrap_or(span);
        let key = self.node_to_ident(children[0]).name.as_str().to_owned();
        let expression = self.source_text(children[1]);
        let value = self.lower_expr(ExprContext::Configuration, children[1]);
        let value = self.arena.alloc_expr(value);
        self.package
            .definition_attributes
            .push(hir::DeferredDefinitionAttribute {
                target: hir::AttributeTarget::Definition(self.current_owner),
                lexical_owner: self.current_owner,
                expression,
                value,
                configuration_key: Some(key),
                span: value_span,
            });
        self.package
            .pending_semantics
            .push((span, "definition configuration".into()));
        true
    }

    /// Lower the body of a struct definition.
    fn lower_struct_body(&mut self, body_node: NodeIndex) -> (Vec<FieldDef<'hir>>, Vec<OwnerId>) {
        let mut fields = Vec::new();
        let mut nested_items = Vec::new();

        let block_kind = self.ast.get_node_kind(body_node);
        let elem_nodes = match block_kind {
            Some(NodeKind::Block) => {
                let children = self.ast.get_children(body_node);
                self.ast
                    .get_multi_child_slice(children[0])
                    .unwrap_or(&[])
                    .to_vec()
            }
            _ => vec![],
        };

        for &elem in &elem_nodes {
            if elem == 0 {
                continue;
            }
            let kind = self.ast.get_node_kind(elem);
            match kind {
                _ if self.is_wrapped_callable_member(elem)
                    || self.is_wrapped_namespace_member(elem) =>
                {
                    nested_items.push(self.lower_top_level_node(elem));
                }
                Some(NodeKind::Configuration) => {
                    self.lower_definition_configuration(elem);
                }
                Some(NodeKind::StructField) => {
                    if let Some(field) = self.lower_struct_field(elem) {
                        fields.push(field);
                    }
                }
                Some(
                    NodeKind::Function
                    | NodeKind::NormalFormHeadDef
                    | NodeKind::TypealiasDef
                    | NodeKind::NewtypeDef
                    | NodeKind::StructDef
                    | NodeKind::EnumDef
                    | NodeKind::ConstDef
                    | NodeKind::ConstDecl
                    | NodeKind::ImplDef
                    | NodeKind::ImplTraitDef,
                ) => {
                    let owner = self.lower_top_level_node(elem);
                    nested_items.push(owner);
                }
                Some(NodeKind::Private)
                    if self.ast.get_children(elem).first().is_some_and(|inner| {
                        matches!(
                            self.ast.get_node_kind(*inner),
                            Some(
                                NodeKind::TypealiasDef
                                    | NodeKind::NewtypeDef
                                    | NodeKind::StructDef
                                    | NodeKind::EnumDef
                                    | NodeKind::Function
                                    | NodeKind::NormalFormHeadDef
                                    | NodeKind::ConstDef
                                    | NodeKind::ConstDecl
                            )
                        )
                    }) =>
                {
                    nested_items.push(self.lower_top_level_node(elem));
                }
                Some(NodeKind::Pub)
                    if self.ast.get_children(elem).first().is_some_and(|inner| {
                        matches!(
                            self.ast.get_node_kind(*inner),
                            Some(
                                NodeKind::Function
                                    | NodeKind::NormalFormHeadDef
                                    | NodeKind::TypealiasDef
                                    | NodeKind::NewtypeDef
                                    | NodeKind::StructDef
                                    | NodeKind::EnumDef
                                    | NodeKind::ConstDef
                                    | NodeKind::ConstDecl
                                    | NodeKind::ImplDef
                                    | NodeKind::ImplTraitDef
                            )
                        )
                    }) =>
                {
                    nested_items.push(self.lower_top_level_node(elem));
                }
                Some(NodeKind::Attribute | NodeKind::AttributeSetTrue) => {
                    self.package.pending_semantics.push((
                        self.ast.get_span(elem).unwrap_or_default(),
                        "struct member attribute".into(),
                    ));
                    let mut cursor = elem;
                    let mut attributes = Vec::new();
                    while let Some((
                        kind @ (NodeKind::Attribute | NodeKind::AttributeSetTrue),
                        _,
                        parts,
                    )) = self.ast.get_node(cursor)
                    {
                        if kind == NodeKind::Attribute {
                            attributes.push(parts[0]);
                        }
                        cursor = parts[1];
                    }
                    if self.ast.get_node_kind(cursor) == Some(NodeKind::StructField) {
                        if let Some(field) = self.lower_struct_field(cursor) {
                            for expression in attributes.into_iter().rev() {
                                let text = self.source_text(expression);
                                let value = self.lower_expr(ExprContext::Attribute, expression);
                                let value = self.arena.alloc_expr(value);
                                self.package.definition_attributes.push(
                                    hir::DeferredDefinitionAttribute {
                                        target: hir::AttributeTarget::Member(field.hir_id),
                                        lexical_owner: self.current_owner,
                                        expression: text,
                                        value,
                                        configuration_key: None,
                                        span: self.ast.get_span(expression).unwrap_or(field.span),
                                    },
                                );
                            }
                            fields.push(field);
                        }
                    } else {
                        // Preserve the complete prefix chain on nested definitions too.
                        nested_items.push(self.lower_top_level_node(elem));
                    }
                }
                _ => {
                    self.package.pending_semantics.push((
                        self.ast.get_span(elem).unwrap_or_default(),
                        "unsupported struct body entry or field visibility".into(),
                    ));
                }
            }
        }

        (fields, nested_items)
    }

    /// Lower a single struct field: `id : type (= default)?`
    fn lower_struct_field(&mut self, node: NodeIndex) -> Option<FieldDef<'hir>> {
        let (target, attributes) = self.peel_member_attributes(node);
        if target != node {
            self.package.pending_semantics.push((
                self.ast.get_span(node).unwrap_or_default(),
                "struct field attribute".into(),
            ));
        }
        let node = target;
        let Some((NodeKind::StructField, span, children)) = self.ast.get_node(node) else {
            unreachable!(
                "not a valid struct field node or no such node index {:?}",
                node
            );
        };

        // StructField: a, b, c  (id, type, default)
        if children.is_empty() {
            self.emit_invalid_struct_field("missing children", span);
            return None;
        }

        let ident = self.node_to_ident(children[0]);
        let ty = if children[1] != 0 {
            let ty_expr = self.lower_expr(ExprContext::Type, children[1]);
            self.arena.alloc_expr(ty_expr)
        } else {
            self.arena.alloc_expr(self.make_invalid_expr(span))
        };

        let default = if children[2] != 0 {
            let def_expr = self.lower_expr(ExprContext::StructFieldDefault, children[2]);
            Some(self.arena.alloc_expr(def_expr) as &_)
        } else {
            None
        };

        let field = FieldDef {
            hir_id: self.next_hir_id(),
            ident,
            ty,
            default,
            span,
        };
        for expression in attributes.into_iter().rev() {
            let value = self.lower_expr(ExprContext::Attribute, expression);
            let value = self.arena.alloc_expr(value);
            self.package
                .definition_attributes
                .push(hir::DeferredDefinitionAttribute {
                    target: hir::AttributeTarget::Member(field.hir_id),
                    lexical_owner: self.current_owner,
                    expression: self.source_text(expression),
                    value,
                    configuration_key: None,
                    span: self.ast.get_span(expression).unwrap_or(span),
                });
        }
        Some(field)
    }

    /// Lower `EnumDef`: a, N, b  (id, clauses, body)
    fn lower_effect_group(&mut self, node: NodeIndex) -> OwnerId {
        let Some((NodeKind::EffectEnumDef, span, children)) = self.ast.get_node(node) else {
            unreachable!("not a valid enum def node or no such node index {:?}", node);
        };

        let id_node = children[0];
        let clauses_multi = children[1];
        let body_node = children[2];

        let owner_id = self.package.alloc_owner_id();
        let prev_owner = self.current_owner;
        self.current_owner = owner_id;
        self.ensure_owner_counter();

        let ident = self.node_to_ident(id_node);
        let previous_scope = self.file_scope;
        if let Ok(binding) = self.resolve_declared(ident) {
            if let resolve::BindingKind::Definition(definition, _) = binding.kind {
                self.package.bind_definition(definition, owner_id);
                if let Some(scope) = self.resolver.find_scope_for_def(definition) {
                    self.file_scope = scope;
                }
            }
        }

        let clause_nodes = self.ast.get_multi_child_slice(clauses_multi).unwrap_or(&[]);
        let lowered = self.lower_clauses(clause_nodes);
        let clause_params = self.arena.alloc_clause_param_slice(lowered.params);
        let clause_constraints = self.arena.alloc_clause_slice(lowered.constraints);

        let entries = self
            .ast
            .get_multi_child_slice(self.ast.get_children(body_node)[0])
            .unwrap_or(&[])
            .to_vec();
        let operations = entries
            .into_iter()
            .map(|entry| self.lower_top_level_node(entry))
            .collect();
        let group = EffectGroupDef {
            operations,
            clause_params,
            clause_constraints,
        };

        let item = Item {
            owner_id,
            ident,
            kind: ItemKind::EffectGroup(group),
            span,
        };
        let item_ref = self.arena.alloc_item(item);
        self.package.insert_owner(
            owner_id,
            OwnerInfo {
                node: OwnerNode::Item(item_ref),
                nodes: OwnerNodes::new(),
            },
        );

        self.file_scope = previous_scope;
        self.current_owner = prev_owner;
        owner_id
    }

    fn lower_enum_def(&mut self, node: NodeIndex) -> OwnerId {
        let Some((NodeKind::EnumDef, span, children)) = self.ast.get_node(node) else {
            unreachable!("not a valid enum def node or no such node index {:?}", node);
        };

        let id_node = children[0];
        let clauses_multi = children[1];
        let body_node = children[2];

        let owner_id = self.package.alloc_owner_id();
        let prev_owner = self.current_owner;
        self.current_owner = owner_id;
        self.ensure_owner_counter();

        let ident = self.node_to_ident(id_node);
        let previous_scope = self.file_scope;
        if let Ok(binding) = self.resolve_declared(ident) {
            if let resolve::BindingKind::Definition(definition, _) = binding.kind {
                self.package.bind_definition(definition, owner_id);
                if let Some(scope) = self.resolver.find_scope_for_def(definition) {
                    self.file_scope = scope;
                }
            }
        }

        let clause_nodes = self.ast.get_multi_child_slice(clauses_multi).unwrap_or(&[]);
        let lowered = self.lower_clauses(clause_nodes);
        let clause_params = self.arena.alloc_clause_param_slice(lowered.params);
        let clause_constraints = self.arena.alloc_clause_slice(lowered.constraints);

        let (variants, nested_items, configurations) = self.lower_enum_body(body_node);
        let variants_slice = self.arena.alloc_variant_slice(variants);

        let enum_def = EnumDef {
            is_intrinsic: false,
            variants: variants_slice,
            configurations,
            clause_params,
            clause_constraints,
            nested_items,
        };

        let item = Item {
            owner_id,
            ident,
            kind: ItemKind::Enum(enum_def),
            span,
        };
        let item_ref = self.arena.alloc_item(item);
        self.package.insert_owner(
            owner_id,
            OwnerInfo {
                node: OwnerNode::Item(item_ref),
                nodes: OwnerNodes::new(),
            },
        );

        self.file_scope = previous_scope;
        self.current_owner = prev_owner;
        owner_id
    }

    fn lower_enum_body(
        &mut self,
        body_node: NodeIndex,
    ) -> (Vec<Variant<'hir>>, Vec<OwnerId>, Vec<Configuration<'hir>>) {
        let mut variants = Vec::new();
        let mut nested_items = Vec::new();
        let mut configurations = Vec::new();

        let elem_nodes = match self.ast.get_node_kind(body_node) {
            Some(NodeKind::Block) => {
                let children = self.ast.get_children(body_node);
                self.ast
                    .get_multi_child_slice(children[0])
                    .unwrap_or(&[])
                    .to_vec()
            }
            _ => vec![],
        };

        for &elem in &elem_nodes {
            if elem == 0 {
                continue;
            }
            let kind = self.ast.get_node_kind(elem);
            match kind {
                _ if self.is_wrapped_callable_member(elem)
                    || self.is_wrapped_namespace_member(elem) =>
                {
                    nested_items.push(self.lower_top_level_node(elem));
                }
                Some(NodeKind::Configuration) => {
                    let children = self.ast.get_children(elem);
                    let ident = self.node_to_ident(children[0]);
                    let value = self.lower_expr(ExprContext::Configuration, children[1]);
                    configurations.push(Configuration {
                        ident,
                        value: self.arena.alloc_expr(value),
                        span: self.ast.get_span(elem).unwrap_or_default(),
                    });
                }
                Some(
                    NodeKind::PatternEnumVariant
                    | NodeKind::ExprEnumVariant
                    | NodeKind::TupleEnumVariant
                    | NodeKind::StructEnumVariant
                    | NodeKind::SubEnumEnumVariant,
                ) => {
                    if let Some(v) = self.lower_enum_variant(elem) {
                        variants.push(v);
                    }
                }
                Some(NodeKind::Id) => {
                    // Unit variant (just an identifier)
                    let span = self.ast.get_span(elem).unwrap_or(Span::default());
                    let ident = self.node_to_ident(elem);
                    variants.push(Variant {
                        hir_id: self.next_hir_id(),
                        ident,
                        kind: VariantKind::Unit,
                        span,
                    });
                }
                Some(
                    NodeKind::Function
                    | NodeKind::TypealiasDef
                    | NodeKind::NewtypeDef
                    | NodeKind::StructDef
                    | NodeKind::EnumDef
                    | NodeKind::ConstDef
                    | NodeKind::ConstDecl,
                ) => {
                    let owner = self.lower_top_level_node(elem);
                    nested_items.push(owner);
                }
                Some(NodeKind::Private)
                    if self.ast.get_children(elem).first().is_some_and(|inner| {
                        matches!(
                            self.ast.get_node_kind(*inner),
                            Some(
                                NodeKind::TypealiasDef
                                    | NodeKind::NewtypeDef
                                    | NodeKind::StructDef
                                    | NodeKind::EnumDef
                                    | NodeKind::Function
                                    | NodeKind::NormalFormHeadDef
                                    | NodeKind::ConstDef
                                    | NodeKind::ConstDecl
                            )
                        )
                    }) =>
                {
                    nested_items.push(self.lower_top_level_node(elem));
                }
                Some(NodeKind::Pub)
                    if self.ast.get_children(elem).first().is_some_and(|inner| {
                        matches!(
                            self.ast.get_node_kind(*inner),
                            Some(
                                NodeKind::Function
                                    | NodeKind::TypealiasDef
                                    | NodeKind::NewtypeDef
                                    | NodeKind::StructDef
                                    | NodeKind::EnumDef
                                    | NodeKind::ConstDef
                                    | NodeKind::ConstDecl
                            )
                        )
                    }) =>
                {
                    nested_items.push(self.lower_top_level_node(elem));
                }
                Some(NodeKind::Attribute | NodeKind::AttributeSetTrue) => {
                    let (target, _) = self.peel_member_attributes(elem);
                    if matches!(
                        self.ast.get_node_kind(target),
                        Some(
                            NodeKind::Id
                                | NodeKind::PatternEnumVariant
                                | NodeKind::ExprEnumVariant
                                | NodeKind::TupleEnumVariant
                                | NodeKind::StructEnumVariant
                                | NodeKind::SubEnumEnumVariant
                        )
                    ) {
                        if let Some(variant) = self.lower_enum_variant(elem) {
                            variants.push(variant);
                        }
                    } else {
                        nested_items.push(self.lower_top_level_node(elem));
                    }
                }
                _ => {
                    self.package.pending_semantics.push((
                        self.ast.get_span(elem).unwrap_or_default(),
                        "enum configuration or member attribute".into(),
                    ));
                }
            }
        }

        (variants, nested_items, configurations)
    }

    fn peel_member_attributes(&self, mut node: NodeIndex) -> (NodeIndex, Vec<NodeIndex>) {
        let mut attributes = Vec::new();
        while let Some((kind @ (NodeKind::Attribute | NodeKind::AttributeSetTrue), _, parts)) =
            self.ast.get_node(node)
        {
            if kind == NodeKind::Attribute {
                attributes.push(parts[0]);
            }
            node = parts[1];
        }
        (node, attributes)
    }

    fn lower_enum_variant(&mut self, node: NodeIndex) -> Option<Variant<'hir>> {
        let (target, attributes) = self.peel_member_attributes(node);
        if target != node {
            self.package.pending_semantics.push((
                self.ast.get_span(node).unwrap_or_default(),
                "enum variant attribute".into(),
            ));
        }
        let node = target;
        let Some((kind, span, children)) = self.ast.get_node(node) else {
            unreachable!("invalid enum variant node: no such node index {:?}", node);
        };

        let variant = match kind {
            NodeKind::Id => Some(Variant {
                hir_id: self.next_hir_id(),
                ident: self.node_to_ident(node),
                kind: VariantKind::Unit,
                span,
            }),
            NodeKind::PatternEnumVariant => {
                let ident = self.node_to_ident(children[0]);
                let pat = self.lower_pattern(children[1]);
                let pat_ref = self.arena.alloc_pattern(pat);
                Some(Variant {
                    hir_id: self.next_hir_id(),
                    ident,
                    kind: VariantKind::Pattern(pat_ref),
                    span,
                })
            }
            NodeKind::ExprEnumVariant => {
                let ident = self.node_to_ident(children[0]);
                let expr = self.lower_expr(ExprContext::EnumVariantConstant, children[1]);
                let expr_ref = self.arena.alloc_expr(expr);
                Some(Variant {
                    hir_id: self.next_hir_id(),
                    ident,
                    kind: VariantKind::Const(expr_ref),
                    span,
                })
            }
            NodeKind::TupleEnumVariant => {
                let ident = self.node_to_ident(children[0]);
                let multi = children[1];
                let elem_nodes = self.ast.get_multi_child_slice(multi).unwrap_or(&[]);
                let exprs: Vec<_> = elem_nodes
                    .iter()
                    .map(|&n| self.lower_expr(ExprContext::Type, n))
                    .collect();
                let exprs_slice = self.arena.alloc_expr_slice(exprs);
                Some(Variant {
                    hir_id: self.next_hir_id(),
                    ident,
                    kind: VariantKind::Tuple(exprs_slice),
                    span,
                })
            }
            NodeKind::StructEnumVariant => {
                let ident = self.node_to_ident(children[0]);
                let multi = children[1];
                let field_nodes = self.ast.get_multi_child_slice(multi).unwrap_or(&[]);
                let fields: Vec<_> = field_nodes
                    .iter()
                    .filter_map(|&n| self.lower_struct_field(n))
                    .collect();
                let fields_slice = self.arena.alloc_field_def_slice(fields);
                Some(Variant {
                    hir_id: self.next_hir_id(),
                    ident,
                    kind: VariantKind::Struct(fields_slice),
                    span,
                })
            }
            NodeKind::SubEnumEnumVariant => {
                let ident = self.node_to_ident(children[0]);
                let multi = children[1];
                let sub_nodes = self.ast.get_multi_child_slice(multi).unwrap_or(&[]);
                let sub_variants: Vec<_> = sub_nodes
                    .iter()
                    .filter_map(|&n| self.lower_enum_variant(n))
                    .collect();
                let sub_slice = self.arena.alloc_variant_slice(sub_variants);
                Some(Variant {
                    hir_id: self.next_hir_id(),
                    ident,
                    kind: VariantKind::SubEnum(sub_slice),
                    span,
                })
            }
            _ => {
                self.package
                    .pending_semantics
                    .push((span, "enum group member attribute or declaration".into()));
                None
            }
        };
        if let Some(variant) = &variant {
            for expression in attributes.into_iter().rev() {
                let value = self.lower_expr(ExprContext::Attribute, expression);
                let value = self.arena.alloc_expr(value);
                self.package
                    .definition_attributes
                    .push(hir::DeferredDefinitionAttribute {
                        target: hir::AttributeTarget::Member(variant.hir_id),
                        lexical_owner: self.current_owner,
                        expression: self.source_text(expression),
                        value,
                        configuration_key: None,
                        span: self.ast.get_span(expression).unwrap_or(span),
                    });
            }
        }
        variant
    }

    /// Lower `TraitDef`: a, b, N, c  (id, super_trait, clauses, body)
    fn lower_trait_def(&mut self, node: NodeIndex) -> OwnerId {
        let Some((NodeKind::TraitDef, span, children)) = self.ast.get_node(node) else {
            unreachable!(
                "not a valid trait def node or no such node index {:?}",
                node
            );
        };

        let id_node = children[0];
        let super_trait_node = children[1];
        let clauses_multi = children[2];
        let body_node = children[3];

        let owner_id = self.package.alloc_owner_id();
        let prev_owner = self.current_owner;
        self.current_owner = owner_id;
        self.ensure_owner_counter();

        let ident = self.node_to_ident(id_node);
        let previous_scope = self.file_scope;
        if let Ok(binding) = self.resolve_declared(ident) {
            if let resolve::BindingKind::Definition(definition, _) = binding.kind {
                self.package.bind_definition(definition, owner_id);
                if let Some(scope) = self.resolver.find_scope_for_def(definition) {
                    self.file_scope = scope;
                }
            }
        }

        let clause_nodes = self.ast.get_multi_child_slice(clauses_multi).unwrap_or(&[]);
        let lowered = self.lower_clauses(clause_nodes);
        let clause_params = self.arena.alloc_clause_param_slice(lowered.params);
        let clause_constraints = self.arena.alloc_clause_slice(lowered.constraints);

        // Lower body items
        let body_items = self.lower_trait_body(body_node);

        let super_traits = if super_trait_node == 0 {
            None
        } else {
            let expr = self.lower_expr(ExprContext::Trait, super_trait_node);
            Some(self.arena.alloc_expr(expr) as &_)
        };
        let trait_def = TraitDef {
            super_traits,
            clause_params,
            clause_constraints,
            items: body_items,
        };

        let item = Item {
            owner_id,
            ident,
            kind: ItemKind::Trait(trait_def),
            span,
        };
        let item_ref = self.arena.alloc_item(item);
        self.package.insert_owner(
            owner_id,
            OwnerInfo {
                node: OwnerNode::Item(item_ref),
                nodes: OwnerNodes::new(),
            },
        );

        self.file_scope = previous_scope;
        self.current_owner = prev_owner;
        owner_id
    }

    fn lower_trait_body(&mut self, body_node: NodeIndex) -> Vec<OwnerId> {
        let mut items = Vec::new();
        let elem_nodes = match self.ast.get_node_kind(body_node) {
            Some(NodeKind::Block) => {
                let children = self.ast.get_children(body_node);
                self.ast
                    .get_multi_child_slice(children[0])
                    .unwrap_or(&[])
                    .to_vec()
            }
            _ => vec![],
        };

        for &elem in &elem_nodes {
            if elem == 0 {
                continue;
            }
            let kind = self.ast.get_node_kind(elem);
            match kind {
                _ if self.is_wrapped_namespace_member(elem) => {
                    items.push(self.lower_top_level_node(elem));
                }
                Some(
                    NodeKind::Function
                    | NodeKind::NormalFormHeadDef
                    | NodeKind::AssocDecl
                    | NodeKind::TypealiasDef
                    | NodeKind::NewtypeDef
                    | NodeKind::ConstDef
                    | NodeKind::ConstDecl
                    | NodeKind::StructDef
                    | NodeKind::EnumDef
                    | NodeKind::TraitDef
                    | NodeKind::ModuleDef
                    | NodeKind::ImplDef
                    | NodeKind::ImplTraitDef
                    | NodeKind::AlgebraicEffect
                    | NodeKind::EffectEnumDef
                    | NodeKind::UseStatement,
                ) => {
                    let owner = self.lower_top_level_node(elem);
                    items.push(owner);
                }
                Some(NodeKind::Pub | NodeKind::Attribute | NodeKind::AttributeSetTrue) => {
                    // Visibility wraps the declaration; do not silently drop
                    // public implementation members from owned HIR.
                    items.push(self.lower_top_level_node(elem));
                }
                Some(NodeKind::Private) => {
                    self.package.pending_semantics.push((
                        self.ast.get_span(elem).unwrap_or_default(),
                        "private trait/implementation member visibility".into(),
                    ));
                    items.push(self.lower_top_level_node(elem));
                }
                _ => {
                    self.package.pending_semantics.push((
                        self.ast.get_span(elem).unwrap_or_default(),
                        "unsupported trait/implementation body entry".into(),
                    ));
                }
            }
        }
        items
    }

    /// Lower `ImplDef`: a, N, b  (type, clauses, body)
    fn enter_implementation_scope(&mut self, node: NodeIndex, owner: OwnerId) {
        if let Some((definition, scope)) = self.resolver.implementation_scope(self.file_id, node) {
            self.package.bind_definition(definition, owner);
            self.file_scope = scope;
        } else {
            self.emit_malformed(
                "implementation has no scanned declaration scope",
                self.ast.get_span(node).unwrap_or_default(),
            );
        }
    }

    fn lower_impl_def(&mut self, node: NodeIndex) -> OwnerId {
        let Some((NodeKind::ImplDef, span, children)) = self.ast.get_node(node) else {
            unreachable!("invalid impl def node or no such node index {:?}", node);
        };

        let type_node = children[0];
        let clauses_multi = children[1];
        let body_node = children[2];

        let owner_id = self.package.alloc_owner_id();
        let prev_owner = self.current_owner;
        self.current_owner = owner_id;
        self.ensure_owner_counter();

        let ident = Ident::new(Symbol::intern("<impl>"), span);

        let previous_scope = self.file_scope;
        self.enter_implementation_scope(node, owner_id);

        let self_ty = self.lower_expr(ExprContext::Type, type_node);
        let self_ty_ref = self.arena.alloc_expr(self_ty);

        let clause_nodes = self.ast.get_multi_child_slice(clauses_multi).unwrap_or(&[]);
        let lowered = self.lower_clauses(clause_nodes);
        let clause_params = self.arena.alloc_clause_param_slice(lowered.params);
        let clause_constraints = self.arena.alloc_clause_slice(lowered.constraints);

        let body_items = self.lower_impl_body(body_node);

        let impl_def = ImplDef {
            self_ty: self_ty_ref,
            trait_ref: None,
            clause_params,
            clause_constraints,
            items: body_items,
        };

        let item = Item {
            owner_id,
            ident,
            kind: ItemKind::Impl(impl_def),
            span,
        };
        let item_ref = self.arena.alloc_item(item);
        self.package.insert_owner(
            owner_id,
            OwnerInfo {
                node: OwnerNode::Item(item_ref),
                nodes: OwnerNodes::new(),
            },
        );

        self.file_scope = previous_scope;
        self.current_owner = prev_owner;
        owner_id
    }

    /// Lower `ImplTraitDef`: a, b, N, c  (trait, type, clauses, body)
    fn lower_impl_trait_def(&mut self, node: NodeIndex) -> OwnerId {
        let Some((NodeKind::ImplTraitDef, span, children)) = self.ast.get_node(node) else {
            unreachable!(
                "invalid impl trait def node or no such node index {:?}",
                node
            );
        };

        let trait_node = children[0];
        let type_node = children[1];
        let clauses_multi = children[2];
        let body_node = children[3];

        let owner_id = self.package.alloc_owner_id();
        let prev_owner = self.current_owner;
        self.current_owner = owner_id;
        self.ensure_owner_counter();

        let ident = Ident::new(Symbol::intern("<impl>"), span);

        let previous_scope = self.file_scope;
        self.enter_implementation_scope(node, owner_id);

        let trait_ = self.lower_expr(ExprContext::Trait, trait_node);
        let trait_ref = self.arena.alloc_expr(trait_);
        let self_ty = self.lower_expr(ExprContext::Type, type_node);
        let self_ty_ref = self.arena.alloc_expr(self_ty);

        let clause_nodes = self.ast.get_multi_child_slice(clauses_multi).unwrap_or(&[]);
        let lowered = self.lower_clauses(clause_nodes);
        let clause_params = self.arena.alloc_clause_param_slice(lowered.params);
        let clause_constraints = self.arena.alloc_clause_slice(lowered.constraints);

        let body_items = self.lower_impl_body(body_node);

        let impl_def = ImplDef {
            self_ty: self_ty_ref,
            trait_ref: Some(trait_ref),
            clause_params,
            clause_constraints,
            items: body_items,
        };

        let item = Item {
            owner_id,
            ident,
            kind: ItemKind::Impl(impl_def),
            span,
        };
        let item_ref = self.arena.alloc_item(item);
        self.package.insert_owner(
            owner_id,
            OwnerInfo {
                node: OwnerNode::Item(item_ref),
                nodes: OwnerNodes::new(),
            },
        );

        self.file_scope = previous_scope;
        self.current_owner = prev_owner;
        owner_id
    }

    fn lower_impl_body(&mut self, body_node: NodeIndex) -> Vec<OwnerId> {
        self.lower_trait_body(body_node) // Same structure
    }

    fn lower_associated(&mut self, node: NodeIndex) -> OwnerId {
        let (_, span, children) = self.ast.get_node(node).unwrap();
        let owner_id = self.package.alloc_owner_id();
        let previous_owner = self.current_owner;
        let previous_scope = self.file_scope;
        self.current_owner = owner_id;
        self.ensure_owner_counter();
        let ident = self.node_to_ident(children[0]);
        if let Ok(binding) = self.resolve_declared(ident) {
            if let resolve::BindingKind::Definition(definition, _) = binding.kind {
                self.package.bind_definition(definition, owner_id);
                if let Some(scope) = self.resolver.find_scope_for_def(definition) {
                    self.file_scope = scope;
                }
            }
        }
        let rib = self.local_enter(RibKind::FnParam);
        let mut parameters: Vec<FnParam<'hir>> = Vec::new();
        for parameter in self
            .ast
            .get_multi_child_slice(children[1])
            .unwrap_or(&[])
            .to_vec()
        {
            if let Some(mut param) = self.lower_fn_param(parameter) {
                if parameters
                    .iter()
                    .any(|old| old.name.name == param.name.name)
                {
                    self.emit_malformed("duplicate associated parameter", param.span);
                }
                param.index = parameters.len() as u32;
                self.define_fn_param(param.name, parameter, param.hir_id);
                parameters.push(param);
            }
        }
        let lowered =
            self.lower_clauses(self.ast.get_multi_child_slice(children[4]).unwrap_or(&[]));
        let ty = self.lower_expr(ExprContext::Type, children[2]);
        let default = if children[3] == 0 {
            None
        } else {
            let expr = self.lower_expr(ExprContext::AssociatedDefault, children[3]);
            Some(self.arena.alloc_expr(expr) as &_)
        };
        let item = self.arena.alloc_item(Item {
            owner_id,
            ident,
            span,
            kind: ItemKind::Associated(AssociatedDecl {
                parameters: self.arena.alloc_fn_param_slice(parameters),
                ty: self.arena.alloc_expr(ty),
                default,
                clause_params: self.arena.alloc_clause_param_slice(lowered.params),
                clause_constraints: self.arena.alloc_clause_slice(lowered.constraints),
            }),
        });
        self.package.insert_owner(
            owner_id,
            OwnerInfo {
                node: OwnerNode::Item(item),
                nodes: OwnerNodes::new(),
            },
        );
        self.local_exit(rib);
        self.file_scope = previous_scope;
        self.current_owner = previous_owner;
        owner_id
    }

    /// Lower an alias or newtype telescope, preserving the declaration kind.
    fn lower_named_type(&mut self, node: NodeIndex) -> OwnerId {
        let Some((kind @ (NodeKind::TypealiasDef | NodeKind::NewtypeDef), span, children)) =
            self.ast.get_node(node)
        else {
            unreachable!("invalid type alias node or no such node index {:?}", node);
        };

        let id_node = children[0];
        let type_params_multi = children[1];
        let type_expr_node = children[2];

        let owner_id = self.package.alloc_owner_id();
        let prev_owner = self.current_owner;
        self.current_owner = owner_id;
        self.ensure_owner_counter();

        let ident = self.node_to_ident(id_node);
        let previous_scope = self.file_scope;
        if let Ok(binding) = self.resolve_declared(ident) {
            if let resolve::BindingKind::Definition(definition, _) = binding.kind {
                self.package.bind_definition(definition, owner_id);
                if let Some(scope) = self.resolver.find_scope_for_def(definition) {
                    self.file_scope = scope;
                }
            }
        }

        let type_param_nodes = self
            .ast
            .get_multi_child_slice(type_params_multi)
            .unwrap_or(&[]);
        let lowered = self.lower_clauses(type_param_nodes);
        let parameters = self.arena.alloc_clause_param_slice(lowered.params);
        let constraints = self.arena.alloc_clause_slice(lowered.constraints);

        let type_expr = self.lower_expr(ExprContext::Type, type_expr_node);
        let type_expr_ref = self.arena.alloc_expr(type_expr);

        let item = Item {
            owner_id,
            ident,
            kind: if kind == NodeKind::NewtypeDef {
                ItemKind::Newtype(type_expr_ref, parameters, constraints)
            } else {
                ItemKind::TypeAlias(type_expr_ref, parameters, constraints)
            },
            span,
        };
        let item_ref = self.arena.alloc_item(item);
        self.package.insert_owner(
            owner_id,
            OwnerInfo {
                node: OwnerNode::Item(item_ref),
                nodes: OwnerNodes::new(),
            },
        );

        self.file_scope = previous_scope;
        self.current_owner = prev_owner;
        owner_id
    }

    /// Lower `ModuleDef`: a, b  (id, body)
    fn lower_module_def(&mut self, node: NodeIndex) -> OwnerId {
        let Some((NodeKind::ModuleDef, span, children)) = self.ast.get_node(node) else {
            unreachable!("invalid module def node or no such node index {:?}", node);
        };

        let id_node = children[0];
        let body_node = children[1];

        let owner_id = self.package.alloc_owner_id();
        let prev_owner = self.current_owner;
        self.current_owner = owner_id;
        self.ensure_owner_counter();

        let ident = self.node_to_ident(id_node);
        if let Ok(binding) = self.resolve_declared(ident) {
            if let resolve::BindingKind::Definition(definition, _) = binding.kind {
                self.package.bind_definition(definition, owner_id);
            }
        }

        let previous_scope = self.file_scope;
        if let Ok(binding) = self.resolve_declared(ident) {
            if let resolve::BindingKind::Definition(definition, _) = binding.kind {
                if let Some(scope) = self.resolver.find_scope_for_def(definition) {
                    self.file_scope = scope;
                }
            }
        }

        // Lower body items
        let elem_nodes = match self.ast.get_node_kind(body_node) {
            Some(NodeKind::Block) => {
                let ch = self.ast.get_children(body_node);
                if !ch.is_empty() {
                    self.ast
                        .get_multi_child_slice(ch[0])
                        .unwrap_or(&[])
                        .to_vec()
                } else {
                    vec![]
                }
            }
            _ => vec![],
        };

        let mut item_ids = Vec::new();
        for &elem in &elem_nodes {
            if elem != 0 && !self.lower_definition_configuration(elem) {
                let owner = self.lower_top_level_node(elem);
                item_ids.push(owner);
            }
        }

        let mod_def = ModDef { items: item_ids };
        let item = Item {
            owner_id,
            ident,
            kind: ItemKind::Mod(mod_def),
            span,
        };
        let item_ref = self.arena.alloc_item(item);
        self.package.insert_owner(
            owner_id,
            OwnerInfo {
                node: OwnerNode::Item(item_ref),
                nodes: OwnerNodes::new(),
            },
        );

        self.file_scope = previous_scope;
        self.current_owner = prev_owner;
        owner_id
    }

    pub(crate) fn lower_fn_param(&mut self, node: NodeIndex) -> Option<FnParam<'hir>> {
        let (target, attributes) = self.peel_member_attributes(node);
        if target != node {
            self.package.pending_semantics.push((
                self.ast.get_span(node).unwrap_or_default(),
                "function parameter attribute".into(),
            ));
        }
        let node = target;
        let Some((kind, span, children)) = self.ast.get_node(node) else {
            unreachable!(
                "invalid function parameter node: no such node index {:?}",
                node
            );
        };

        let parameter = match kind {
            NodeKind::TypeBoundParam => {
                // id : type
                let ident = self.node_to_ident(children[0]);
                let ty = self.lower_expr(ExprContext::Type, children[1]);
                let ty_ref = self.arena.alloc_expr(ty);
                let tp = TyParam::new(self.next_hir_id(), TyParamKind::Positional(ty_ref), span);
                Some(FnParam {
                    hir_id: tp.hir_id,
                    name: ident,
                    index: 0, // You might want to set the correct index here
                    ty: tp,
                    span,
                })
            }
            NodeKind::OptionalParam => {
                // .id : type = default
                let ident = self.node_to_ident(children[0]);
                let ty = self.lower_expr(ExprContext::Type, children[1]);
                let ty_ref = self.arena.alloc_expr(ty);
                // The default code expression is checked in its declared
                // virtual context, just like a contextual const initializer.
                // These names must not become parameters or escape to later defaults.
                let code_scope = if let ExprKind::ContextualExprType(bindings, _) = &ty_ref.kind {
                    let scope = self.local_enter(resolve::RibKind::MatchBody);
                    let parts = self.ast.get_children(children[1]);
                    for (binding, &node) in bindings
                        .iter()
                        .zip(self.ast.get_multi_child_slice(parts[1]).unwrap_or(&[]))
                    {
                        self.define_local(binding.name, node, binding.hir_id);
                    }
                    Some(scope)
                } else {
                    None
                };
                let default = self.lower_expr(ExprContext::OptionalParameterDefault, children[2]);
                if let Some(scope) = code_scope {
                    self.local_exit(scope);
                }
                let default_ref = self.arena.alloc_expr(default);
                let tp = TyParam::new(
                    self.next_hir_id(),
                    TyParamKind::Optional(ident.clone(), ty_ref, default_ref),
                    span,
                );
                Some(FnParam {
                    hir_id: tp.hir_id,
                    name: ident,
                    index: 0, // You might want to set the correct index here
                    ty: tp,
                    span,
                })
            }
            NodeKind::VarargParam => {
                // ...id : type
                let ident = self.node_to_ident(children[0]);
                let ty = self.lower_expr(ExprContext::Type, children[1]);
                let ty_ref = self.arena.alloc_expr(ty);
                let tp = TyParam::new(
                    self.next_hir_id(),
                    TyParamKind::Varadic(ident.clone(), ty_ref),
                    span,
                );
                Some(FnParam {
                    hir_id: tp.hir_id,
                    name: ident,
                    index: 0, // You might want to set the correct index here
                    ty: tp,
                    span,
                })
            }
            NodeKind::SelfParam => {
                let ident = Ident::new(Symbol::intern("self"), span);
                let tp = TyParam::new(
                    self.next_hir_id(),
                    TyParamKind::Itself { is_ref: false },
                    span,
                );
                Some(FnParam {
                    hir_id: tp.hir_id,
                    name: ident,
                    index: 0, // You might want to set the correct index here
                    ty: tp,
                    span,
                })
            }
            NodeKind::SelfRefParam => {
                let ident = Ident::new(Symbol::intern("self"), span);
                let tp = TyParam::new(
                    self.next_hir_id(),
                    TyParamKind::Itself { is_ref: true },
                    span,
                );
                Some(FnParam {
                    hir_id: tp.hir_id,
                    name: ident,
                    index: 0, // You might want to set the correct index here
                    ty: tp,
                    span,
                })
            }
            NodeKind::ComptimeParam => self.lower_fn_param(children[0]).map(|mut param| {
                param.ty.flags |= TPARAM_COMPTIME;
                param
            }),
            NodeKind::ImplicitParam => self.lower_fn_param(children[0]).map(|mut param| {
                param.ty.flags |= TPARAM_IMPLICIT;
                param
            }),
            NodeKind::LambdaParam => self.lower_fn_param(children[0]).map(|mut param| {
                param.ty.flags |= TPARAM_LAMBDA;
                param
            }),
            NodeKind::ErrorParam => self.lower_fn_param(children[0]).map(|mut param| {
                param.ty.flags |= TPARAM_ERROR;
                param
            }),
            NodeKind::CatchParam => self.lower_fn_param(children[0]).map(|mut param| {
                param.ty.flags |= hir::common::TPARAM_CATCH;
                param
            }),
            NodeKind::QuoteParam => self.lower_fn_param(children[0]).map(|mut param| {
                param.ty.flags |= TPARAM_QUOTE;
                param
            }),
            NodeKind::Id => {
                // Bare identifier param (no type annotation)
                let ident = self.node_to_ident(node);
                let ty_expr = Expr {
                    hir_id: self.next_hir_id(),
                    kind: ExprKind::TyPlaceholder,
                    span,
                };
                let ty_ref = self.arena.alloc_expr(ty_expr);
                let tp = TyParam::new(self.next_hir_id(), TyParamKind::Positional(ty_ref), span);
                Some(FnParam {
                    hir_id: tp.hir_id,
                    name: ident,
                    index: 0, // You might want to set the correct index here
                    ty: tp,
                    span,
                })
            }
            _ => {
                self.emit_invalid_parameter(&format!("{:?}", kind), span);
                None
            }
        };
        if let Some(parameter) = &parameter {
            let annotation = match &parameter.ty.kind {
                TyParamKind::Positional(ty) | TyParamKind::Optional(_, ty, _) => Some(*ty),
                _ => None,
            };
            if let Some(Expr {
                kind: ExprKind::ContextualPatternType(_, outputs),
                ..
            }) = annotation
            {
                self.pattern_code_outputs.insert(
                    parameter.hir_id,
                    outputs.iter().map(|entry| entry.name).collect(),
                );
            }

            for expression in attributes.into_iter().rev() {
                let value = self.lower_expr(ExprContext::Attribute, expression);
                let value = self.arena.alloc_expr(value);
                self.package
                    .definition_attributes
                    .push(hir::DeferredDefinitionAttribute {
                        target: hir::AttributeTarget::Member(parameter.hir_id),
                        lexical_owner: self.current_owner,
                        expression: self.source_text(expression),
                        value,
                        configuration_key: None,
                        span: self.ast.get_span(expression).unwrap_or(span),
                    });
            }
        }
        parameter
    }

    fn lower_const_def(&mut self, node: NodeIndex) -> OwnerId {
        let Some((NodeKind::ConstDef | NodeKind::ConstDecl, span, children)) =
            self.ast.get_node(node)
        else {
            unreachable!("invalid const def node or no such node index {:?}", node);
        };

        let id_node = children[0];
        let type_node = children[1];
        let value_node = children[2];

        let owner_id = self.package.alloc_owner_id();
        let prev_owner = self.current_owner;
        self.current_owner = owner_id;
        self.ensure_owner_counter();

        let ident = self.node_to_ident(id_node);
        if let Ok(binding) = self.resolve_declared(ident) {
            if let resolve::BindingKind::Definition(definition, _) = binding.kind {
                self.package.bind_definition(definition, owner_id);
            }
        }
        let type_expr = if type_node != 0 {
            self.arena
                .alloc_expr(self.lower_expr(ExprContext::Type, type_node))
        } else {
            self.arena.alloc_expr(Expr {
                hir_id: self.next_hir_id(),
                kind: ExprKind::TyPlaceholder,
                span,
            })
        };
        let value_expr = self
            .arena
            .alloc_expr(self.lower_expr(ExprContext::Type, value_node));

        let item = Item {
            owner_id,
            ident,
            kind: ItemKind::Const(type_expr, value_expr),
            span,
        };
        let item_ref = self.arena.alloc_item(item);
        self.package.insert_owner(
            owner_id,
            OwnerInfo {
                node: OwnerNode::Item(item_ref),
                nodes: OwnerNodes::new(),
            },
        );

        self.current_owner = prev_owner;
        owner_id
    }

    /// Create an error item (returns an OwnerId that maps to `ItemKind::Err`).
    pub(crate) fn make_error_item(&mut self, span: Span) -> OwnerId {
        let owner_id = self.package.alloc_owner_id();
        let prev_owner = self.current_owner;
        self.current_owner = owner_id;
        self.ensure_owner_counter();

        let item = Item {
            owner_id,
            ident: Ident::new(Symbol::intern("<error>"), span),
            kind: ItemKind::Invalid,
            span,
        };
        let item_ref = self.arena.alloc_item(item);
        self.package.insert_owner(
            owner_id,
            OwnerInfo {
                node: OwnerNode::Item(item_ref),
                nodes: OwnerNodes::new(),
            },
        );

        self.current_owner = prev_owner;
        owner_id
    }
}
