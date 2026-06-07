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
    body::{Body, Param},
    common::{
        Ident, Symbol, TPARAM_COMPTIME, TPARAM_ERROR, TPARAM_IMPLICIT, TPARAM_LAMBDA, TPARAM_QUOTE,
        TyParam, TyParamKind,
    },
    expr::{Expr, ExprKind},
    hir_id::{HirId, ItemLocalId, OwnerId},
    item::*,
    node,
    owner::{OwnerInfo, OwnerNode, OwnerNodes},
};
use middle::ty;
use rustc_span::Span;

use crate::LoweringContext;

impl<'hir, 'ast> LoweringContext<'hir, 'ast> {
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
        self.current_owner = root_owner_id;

        // Lower each top-level node.
        let mut item_ids = Vec::new();
        // Copy elem_nodes to a Vec so we don't borrow self immutably and mutably
        let elem_nodes_vec: Vec<NodeIndex> = elem_nodes.to_vec();
        for &elem in &elem_nodes_vec {
            if elem == 0 {
                continue;
            }

            let owner = self.lower_top_level_node(elem);
            item_ids.push(owner);
        }

        // Build the root module item.
        self.current_owner = root_owner_id;
        self.reset_hir_id_counter();

        let mod_def = ModDef { items: item_ids };
        let ident = Ident::new(Symbol::intern("<root>"), span);
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
        println!(
            "lowering top-level node: {}",
            self.ast.dump_to_s_expression(node, self.source_map)
        );
        let Some((kind, span, children)) = self.ast.get_node(node) else {
            unreachable!("invalid top-level node: no such node index {:?}", node);
        };

        match kind {
            NodeKind::Function => self.lower_function(node),
            NodeKind::NormalFormDef => self.lower_normal_form_def(node),
            NodeKind::StructDef => self.lower_struct_def(node),
            NodeKind::EnumDef => self.lower_enum_def(node),
            NodeKind::TraitDef => self.lower_trait_def(node),
            NodeKind::ImplDef => self.lower_impl_def(node),
            NodeKind::ImplTraitDef => self.lower_impl_trait_def(node),
            NodeKind::TypealiasDef => self.lower_type_alias(node),
            NodeKind::ModuleDef => self.lower_module_def(node),
            // NodeKind::UseStatement => self.lower_use_statement(node),

            // Attribute-wrapped definitions
            NodeKind::Attribute | NodeKind::AttributeSetTrue => {
                self.lower_top_level_node(children[1])
            }

            NodeKind::ConstDef => self.lower_const_def(node),
            // TODO: parser还不是能很好地区分ConstDef和ConstDecl，先将两者都当成ConstDef来处理，后续再完善parser以区分两者
            NodeKind::ConstDecl => self.lower_const_def(node),

            // Visibility modified definitions, TODO: handle the visibility modifier properly instead of just skipping it
            NodeKind::Pub | NodeKind::Private => self.lower_top_level_node(children[0]),

            other => unreachable!(
                "unexpected top-level node kind {} at {:?}",
                self.ast.dump_to_s_expression(node, self.source_map),
                span
            ),
        }
    }

    /// Lower an item that appears inside a block (definition inside a
    /// function body, struct body, etc.)  Returns the new [`OwnerId`].
    pub fn lower_item_in_block(&mut self, node: NodeIndex) -> OwnerId {
        self.lower_top_level_node(node)
    }

    /// Lower `Function`: a, N, b, c, N, d
    ///   (id, params, return_type, handles_effect, clauses, body)
    fn lower_function(&mut self, node: NodeIndex) -> OwnerId {
        let Some((NodeKind::Function, span, children)) = self.ast.get_node(node) else {
            unreachable!("invalid function node or no such node index {:?}", node);
        };

        let id_node = children[0];
        let params_multi = children[1];
        let return_type_node = children[2];
        let _handles_effect_node = children[3];
        let clauses_multi = children[4];
        let body_node = children[5];

        // Allocate owner
        let owner_id = self.package.alloc_owner_id();
        let prev_owner = self.current_owner;
        self.current_owner = owner_id;
        self.reset_hir_id_counter();

        // Identifier
        let ident = self.node_to_ident(id_node);

        // Modifiers — check if the first param slot encodes flags
        // (In the AST, fn modifiers are encoded via Attribute/AttributeSetTrue
        //  wrapping, but we can also check source text for keywords)
        let modifiers = FnModifiers::default();

        // Parameters
        let param_nodes = self.ast.get_multi_child_slice(params_multi).unwrap_or(&[]);
        let fn_params = self.lower_fn_params(param_nodes);

        // Return type
        let return_ty = if return_type_node != 0 {
            let ty_expr = self.lower_expr(return_type_node);
            Some(self.arena.alloc_expr(ty_expr) as &_)
        } else {
            None
        };

        // Clauses → generic params + constraints
        let clause_nodes = self.ast.get_multi_child_slice(clauses_multi).unwrap_or(&[]);
        let lowered = self.lower_clauses(clause_nodes);
        let clause_params = self.arena.alloc_clause_param_slice(lowered.params);
        let clause_constraints = self.arena.alloc_clause_slice(lowered.constraints);

        // Build FnSig
        let params_slice = self.arena.alloc_fn_param_slice(fn_params);
        let fn_sig = FnSig {
            params: params_slice,
            return_ty,
            return_bind: None,
            modifiers,
            clause_params,
            clause_constraints,
            span,
        };

        // Body
        let body_expr = self.lower_expr(body_node);
        let body_expr_ref = self.arena.alloc_expr(body_expr);

        // Build Body with params
        let body_params: Vec<Param<'hir>> = param_nodes
            .iter()
            .map(|&p| self.lower_body_param(p))
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

        self.current_owner = prev_owner;
        owner_id
    }

    /// Lower `NormalFormDef`: a, N, b, N, c
    ///   (id, type_params, return_type, clauses, body)
    fn lower_normal_form_def(&mut self, node: NodeIndex) -> OwnerId {
        let Some((NodeKind::NormalFormDef, span, children)) = self.ast.get_node(node) else {
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
        self.reset_hir_id_counter();

        let ident = self.node_to_ident(id_node);

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

        let return_ty = if return_type_node != 0 {
            let ty_expr = self.lower_expr(return_type_node);
            Some(self.arena.alloc_expr(ty_expr) as &_)
        } else {
            None
        };

        let modifiers = FnModifiers {
            is_comptime: true,
            ..Default::default()
        };

        let fn_sig = FnSig {
            params: &[],
            return_ty,
            return_bind: None,
            modifiers,
            clause_params,
            clause_constraints,
            span,
        };

        let body_expr = self.lower_expr(body_node);
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
        self.reset_hir_id_counter();

        let ident = self.node_to_ident(id_node);

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

        self.current_owner = prev_owner;
        owner_id
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
                Some(NodeKind::StructField) => {
                    if let Some(field) = self.lower_struct_field(elem) {
                        fields.push(field);
                    }
                }
                Some(
                    NodeKind::Function
                    | NodeKind::NormalFormDef
                    | NodeKind::TypealiasDef
                    | NodeKind::ConstDef
                    | NodeKind::ImplDef
                    | NodeKind::ImplTraitDef,
                ) => {
                    let owner = self.lower_top_level_node(elem);
                    nested_items.push(owner);
                }
                Some(NodeKind::Attribute | NodeKind::AttributeSetTrue) => {
                    let ch = self.ast.get_children(elem);
                    if ch.len() >= 2 {
                        let inner = ch[1];
                        if self.ast.get_node_kind(inner) == Some(NodeKind::StructField) {
                            if let Some(field) = self.lower_struct_field(inner) {
                                fields.push(field);
                            }
                        } else {
                            let owner = self.lower_top_level_node(inner);
                            nested_items.push(owner);
                        }
                    }
                }
                _ => {}
            }
        }

        (fields, nested_items)
    }

    /// Lower a single struct field: `id : type (= default)?`
    fn lower_struct_field(&mut self, node: NodeIndex) -> Option<FieldDef<'hir>> {
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
            let ty_expr = self.lower_expr(children[1]);
            self.arena.alloc_expr(ty_expr)
        } else {
            self.arena.alloc_expr(self.make_invalid_expr(span))
        };

        let default = if children[2] != 0 {
            let def_expr = self.lower_expr(children[2]);
            Some(self.arena.alloc_expr(def_expr) as &_)
        } else {
            None
        };

        Some(FieldDef {
            hir_id: self.next_hir_id(),
            ident,
            ty,
            default,
            span,
        })
    }

    /// Lower `EnumDef`: a, N, b  (id, clauses, body)
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
        self.reset_hir_id_counter();

        let ident = self.node_to_ident(id_node);

        let clause_nodes = self.ast.get_multi_child_slice(clauses_multi).unwrap_or(&[]);
        let lowered = self.lower_clauses(clause_nodes);
        let clause_params = self.arena.alloc_clause_param_slice(lowered.params);
        let clause_constraints = self.arena.alloc_clause_slice(lowered.constraints);

        let (variants, nested_items) = self.lower_enum_body(body_node);
        let variants_slice = self.arena.alloc_variant_slice(variants);

        let enum_def = EnumDef {
            variants: variants_slice,
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

        self.current_owner = prev_owner;
        owner_id
    }

    fn lower_enum_body(&mut self, body_node: NodeIndex) -> (Vec<Variant<'hir>>, Vec<OwnerId>) {
        let mut variants = Vec::new();
        let mut nested_items = Vec::new();

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
                Some(NodeKind::Function | NodeKind::TypealiasDef | NodeKind::ConstDef) => {
                    let owner = self.lower_top_level_node(elem);
                    nested_items.push(owner);
                }
                _ => {}
            }
        }

        (variants, nested_items)
    }

    fn lower_enum_variant(&mut self, node: NodeIndex) -> Option<Variant<'hir>> {
        let Some((kind, span, children)) = self.ast.get_node(node) else {
            unreachable!("invalid enum variant node: no such node index {:?}", node);
        };

        match kind {
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
                let expr = self.lower_expr(children[1]);
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
                let exprs: Vec<_> = elem_nodes.iter().map(|&n| self.lower_expr(n)).collect();
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
            _ => None,
        }
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
        let _super_trait_node = children[1];
        let clauses_multi = children[2];
        let body_node = children[3];

        let owner_id = self.package.alloc_owner_id();
        let prev_owner = self.current_owner;
        self.current_owner = owner_id;
        self.reset_hir_id_counter();

        let ident = self.node_to_ident(id_node);

        let clause_nodes = self.ast.get_multi_child_slice(clauses_multi).unwrap_or(&[]);
        let lowered = self.lower_clauses(clause_nodes);
        let clause_params = self.arena.alloc_clause_param_slice(lowered.params);
        let clause_constraints = self.arena.alloc_clause_slice(lowered.constraints);

        // Lower body items
        let body_items = self.lower_trait_body(body_node);

        let trait_def = TraitDef {
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
                Some(
                    NodeKind::Function
                    | NodeKind::NormalFormDef
                    | NodeKind::AssocDecl
                    | NodeKind::TypealiasDef
                    | NodeKind::ConstDef,
                ) => {
                    let owner = self.lower_top_level_node(elem);
                    items.push(owner);
                }
                Some(NodeKind::Attribute | NodeKind::AttributeSetTrue) => {
                    let ch = self.ast.get_children(elem);
                    if ch.len() >= 2 {
                        let owner = self.lower_top_level_node(ch[1]);
                        items.push(owner);
                    }
                }
                _ => {}
            }
        }
        items
    }

    /// Lower `ImplDef`: a, N, b  (type, clauses, body)
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
        self.reset_hir_id_counter();

        let ident = Ident::new(Symbol::intern("<impl>"), span);

        let self_ty = self.lower_expr(type_node);
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
        self.reset_hir_id_counter();

        let ident = Ident::new(Symbol::intern("<impl>"), span);

        let trait_ = self.lower_expr(trait_node);
        let trait_ref = self.arena.alloc_expr(trait_);
        let self_ty = self.lower_expr(type_node);
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

        self.current_owner = prev_owner;
        owner_id
    }

    fn lower_impl_body(&mut self, body_node: NodeIndex) -> Vec<OwnerId> {
        self.lower_trait_body(body_node) // Same structure
    }

    /// Lower `TypealiasDef`: a, N, b  (id, type_params, type_expr)
    fn lower_type_alias(&mut self, node: NodeIndex) -> OwnerId {
        let Some((NodeKind::TypealiasDef, span, children)) = self.ast.get_node(node) else {
            unreachable!("invalid type alias node or no such node index {:?}", node);
        };

        let id_node = children[0];
        let type_params_multi = children[1];
        let type_expr_node = children[2];

        let owner_id = self.package.alloc_owner_id();
        let prev_owner = self.current_owner;
        self.current_owner = owner_id;
        self.reset_hir_id_counter();

        let ident = self.node_to_ident(id_node);

        let type_param_nodes = self
            .ast
            .get_multi_child_slice(type_params_multi)
            .unwrap_or(&[]);
        let lowered = self.lower_clauses(type_param_nodes);
        let _clause_constraints = self.arena.alloc_clause_slice(lowered.constraints);

        let type_expr = self.lower_expr(type_expr_node);
        let type_expr_ref = self.arena.alloc_expr(type_expr);

        let item = Item {
            owner_id,
            ident,
            kind: ItemKind::TypeAlias(type_expr_ref),
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
        self.reset_hir_id_counter();

        let ident = self.node_to_ident(id_node);

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
            if elem != 0 {
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

        self.current_owner = prev_owner;
        owner_id
    }

    fn lower_fn_params(&mut self, param_nodes: &[NodeIndex]) -> Vec<(Ident, TyParam<'hir>)> {
        let mut params = Vec::new();
        for &p in param_nodes {
            if p == 0 {
                continue;
            }
            if let Some(param) = self.lower_fn_param(p) {
                params.push(param);
            }
        }
        params
    }

    fn lower_fn_param(&mut self, node: NodeIndex) -> Option<(Ident, TyParam<'hir>)> {
        let Some((kind, span, children)) = self.ast.get_node(node) else {
            unreachable!(
                "invalid function parameter node: no such node index {:?}",
                node
            );
        };

        match kind {
            NodeKind::TypeBoundParam => {
                // id : type
                let ident = self.node_to_ident(children[0]);
                let ty = self.lower_expr(children[1]);
                let ty_ref = self.arena.alloc_expr(ty);
                let tp = TyParam::new(self.next_hir_id(), TyParamKind::Positional(ty_ref), span);
                Some((ident, tp))
            }
            NodeKind::TraitBoundParam => {
                // id :- type
                let ident = self.node_to_ident(children[0]);
                let ty = self.lower_expr(children[1]);
                let ty_ref = self.arena.alloc_expr(ty);
                let tp = TyParam::new(self.next_hir_id(), TyParamKind::Positional(ty_ref), span);
                Some((ident, tp))
            }
            NodeKind::OptionalParam => {
                // .id : type = default
                let ident = self.node_to_ident(children[0]);
                let ty = self.lower_expr(children[1]);
                let ty_ref = self.arena.alloc_expr(ty);
                let default = self.lower_expr(children[2]);
                let default_ref = self.arena.alloc_expr(default);
                let tp = TyParam::new(
                    self.next_hir_id(),
                    TyParamKind::Optional(ident.clone(), ty_ref, default_ref),
                    span,
                );
                Some((ident, tp))
            }
            NodeKind::VarargParam => {
                // ...id : type
                let ident = self.node_to_ident(children[0]);
                let ty = self.lower_expr(children[1]);
                let ty_ref = self.arena.alloc_expr(ty);
                let tp = TyParam::new(
                    self.next_hir_id(),
                    TyParamKind::Varadic(ident.clone(), ty_ref),
                    span,
                );
                Some((ident, tp))
            }
            NodeKind::SelfParam => {
                let ident = Ident::new(Symbol::intern("self"), span);
                let tp = TyParam::new(
                    self.next_hir_id(),
                    TyParamKind::Itself { is_ref: false },
                    span,
                );
                Some((ident, tp))
            }
            NodeKind::SelfRefParam => {
                let ident = Ident::new(Symbol::intern("self"), span);
                let tp = TyParam::new(
                    self.next_hir_id(),
                    TyParamKind::Itself { is_ref: true },
                    span,
                );
                Some((ident, tp))
            }
            NodeKind::ComptimeParam => self.lower_fn_param(children[0]).map(|(i, mut tp)| {
                tp.flags |= TPARAM_COMPTIME;
                (i, tp)
            }),
            NodeKind::ImplicitParam => self.lower_fn_param(children[0]).map(|(i, mut tp)| {
                tp.flags |= TPARAM_IMPLICIT;
                (i, tp)
            }),
            NodeKind::LambdaParam => self.lower_fn_param(children[0]).map(|(i, mut tp)| {
                tp.flags |= TPARAM_LAMBDA;
                (i, tp)
            }),
            NodeKind::ErrorParam => self.lower_fn_param(children[0]).map(|(i, mut tp)| {
                tp.flags |= TPARAM_ERROR;
                (i, tp)
            }),
            NodeKind::CatchParam => {
                // No dedicated TPARAM flag yet; lower the inner param unchanged.
                self.lower_fn_param(children[0])
            }
            NodeKind::QuoteParam => self.lower_fn_param(children[0]).map(|(i, mut tp)| {
                tp.flags |= TPARAM_QUOTE;
                (i, tp)
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
                Some((ident, tp))
            }
            _ => {
                self.emit_invalid_parameter(&format!("{:?}", kind), span);
                None
            }
        }
    }

    /// Lower an AST parameter node into a Body [`Param`].
    fn lower_body_param(&mut self, node: NodeIndex) -> Param<'hir> {
        let Some((kind, span, children)) = self.ast.get_node(node) else {
            unreachable!(
                "invalid function parameter node: no such node index {:?}",
                node
            );
        };

        let (name, ty) = match kind {
            NodeKind::TypeBoundParam | NodeKind::TraitBoundParam => {
                let name = self.node_to_ident(children[0]);
                let ty = if children[1] != 0 {
                    let ty_expr = self.lower_expr(children[1]);
                    Some(self.arena.alloc_expr(ty_expr) as &_)
                } else {
                    None
                };
                (name, ty)
            }
            NodeKind::SelfParam | NodeKind::SelfRefParam => {
                (Ident::new(Symbol::intern("self"), span), None)
            }
            NodeKind::ComptimeParam
            | NodeKind::ImplicitParam
            | NodeKind::LambdaParam
            | NodeKind::ErrorParam
            | NodeKind::CatchParam
            | NodeKind::QuoteParam => (Ident::new(Symbol::intern("_"), span), None),
            NodeKind::OptionalParam => {
                let name = self.node_to_ident(children[0]);
                let ty = if children[1] != 0 {
                    let ty_expr = self.lower_expr(children[1]);
                    Some(self.arena.alloc_expr(ty_expr) as &_)
                } else {
                    None
                };
                (name, ty)
            }
            _ => {
                // Bare identifier or other — use source text as name.
                (self.node_to_ident(node), None)
            }
        };

        Param {
            hir_id: self.next_hir_id(),
            name,
            ty,
            span,
        }
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
        self.reset_hir_id_counter();

        let ident = self.node_to_ident(id_node);
        let type_expr = if type_node != 0 {
            self.arena.alloc_expr(self.lower_expr(type_node))
        } else {
            self.arena.alloc_expr(Expr {
                hir_id: self.next_hir_id(),
                kind: ExprKind::TyPlaceholder,
                span,
            })
        };
        let value_expr = self.arena.alloc_expr(self.lower_expr(value_node));

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
    fn make_error_item(&mut self, span: Span) -> OwnerId {
        let owner_id = self.package.alloc_owner_id();
        let prev_owner = self.current_owner;
        self.current_owner = owner_id;
        self.reset_hir_id_counter();

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
