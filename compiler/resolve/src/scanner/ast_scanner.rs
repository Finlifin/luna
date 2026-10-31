//! AST scanner – walks a single file's AST to populate scopes and collect
//! unresolved import directives.
//!
//! Migrated and refactored from `luna/src/scan/ast_scanner.rs`.

use ast::{Ast, NodeIndex, NodeKind};
use rustc_span::SourceMap;
use symbol::{PathAnchor, Symbol};

use crate::binding::{Binding, BindingKind, Visibility};
use crate::error::{ResolveError, ResolveResult};
use crate::ids::{AstNodeRef, DefId, DefIdGen, ScopeId, ScopeIdGen};
use crate::impl_directive::{ImplDirective, ImplKind};
use crate::import::{ImportDirective, ImportKind};
use crate::scope::{Scope, ScopeKind, ScopeTree};

/// Context carried through an AST scan of one file.
pub struct AstScanner<'a> {
    /// The AST being scanned.
    pub ast: &'a Ast,
    /// Source map for extracting source text from spans.
    pub source_map: &'a SourceMap,
    /// The VFS file id for this file.
    pub file_id: vfs::FileId,
    /// Scope tree (mutable, shared across the entire compilation).
    pub scope_tree: &'a mut ScopeTree,
    /// DefId generator.
    pub def_gen: &'a mut DefIdGen,
    /// ScopeId generator.
    pub scope_gen: &'a mut ScopeIdGen,
    /// Collected import directives.
    pub imports: &'a mut Vec<ImportDirective>,
    /// Collected impl directives.
    pub impls: &'a mut Vec<ImplDirective>,
    /// Mapping from DefId → name (for debug / dump).
    pub def_names: &'a mut Vec<(DefId, Symbol)>,
}

impl<'a> AstScanner<'a> {
    /// Scan the top-level items of a file AST into `parent_scope`.
    pub fn scan_file_items(&mut self, parent_scope: ScopeId) -> ResolveResult<()> {
        let file_scope_items_index = self.ast.get_children(self.ast.root)[0];
        let items = self
            .ast
            .get_multi_child_slice(file_scope_items_index)
            .ok_or_else(|| ResolveError::InternalError("Invalid items slice".into()))?;

        self.scan_items(parent_scope, items)
    }

    /// Scan a list of AST item nodes.
    fn scan_items(&mut self, parent_scope: ScopeId, items: &[NodeIndex]) -> ResolveResult<()> {
        for &item in items {
            // Strip a leading `pub` wrapper if present and determine visibility.
            let (inner, vis) = self.strip_pub_wrapper(item);
            let item_kind = self
                .ast
                .get_node_kind(inner)
                .ok_or_else(|| ResolveError::InternalError("Invalid node index".into()))?;

            match item_kind {
                NodeKind::ModuleDef => {
                    self.scan_module_def(parent_scope, inner, vis)?;
                }
                NodeKind::TraitDef => {
                    self.scan_adt_def(parent_scope, inner, ScopeKind::TraitBody, vis)?;
                }
                NodeKind::StructDef => {
                    self.scan_adt_def(parent_scope, inner, ScopeKind::AdtBody, vis)?;
                }
                NodeKind::EnumDef | NodeKind::EffectEnumDef => {
                    self.scan_adt_def(parent_scope, inner, ScopeKind::AdtBody, vis)?;
                }
                NodeKind::UnionDef => {
                    self.scan_adt_def(parent_scope, inner, ScopeKind::AdtBody, vis)?;
                }
                NodeKind::TypealiasDef | NodeKind::NewtypeDef => {
                    self.scan_named_type(parent_scope, inner, vis)?
                }
                NodeKind::ConstDef | NodeKind::ConstDecl => {
                    let name = self.extract_name(self.ast.get_children(inner)[0])?;
                    let def_id = self.def_gen.next();
                    self.define_in_scope(
                        name,
                        BindingKind::Definition(def_id, parent_scope),
                        Some(inner),
                        parent_scope,
                        vis,
                    )?;
                    self.def_names.push((def_id, name));
                }
                NodeKind::TestDef
                | NodeKind::TestTemplateDef
                | NodeKind::TestTemplateInstance
                | NodeKind::FunctionTemplateInstance
                | NodeKind::FunctionTemplateInstanceWithParams
                | NodeKind::Function
                | NodeKind::DeriverDef
                | NodeKind::AssocDecl
                | NodeKind::NormalFormHeadDef
                | NodeKind::AlgebraicEffect => {
                    self.scan_function_def(parent_scope, inner, vis)?;
                }
                NodeKind::ImplDef => {
                    self.scan_impl_def(parent_scope, inner, ImplKind::Inherent)?;
                }
                NodeKind::ImplTraitDef => {
                    self.scan_impl_def(parent_scope, inner, ImplKind::TraitImpl)?;
                }
                NodeKind::DeriveDef => {
                    self.scan_impl_def(parent_scope, inner, ImplKind::Derivation)?;
                }
                NodeKind::UseStatement => {
                    self.collect_import(parent_scope, inner, vis)?;
                }
                _ => {
                    // Other node kinds are ignored during the scan pass.
                }
            }
        }
        Ok(())
    }

    /// Attributes/modifiers preserve declaration identity. Semantic processing
    /// remains in lowering; scanning only exposes the wrapped declaration.
    fn strip_pub_wrapper(&self, mut item: NodeIndex) -> (NodeIndex, Visibility) {
        let mut visibility = Visibility::Package;
        loop {
            match self.ast.get_node_kind(item) {
                Some(NodeKind::FunctionTemplateDef) => {
                    item = self.ast.get_children(item)[0];
                }
                Some(NodeKind::Pub) => {
                    visibility = Visibility::Public;
                    item = self.ast.get_children(item)[0];
                }
                Some(NodeKind::Private) => {
                    visibility = Visibility::Private;
                    item = self.ast.get_children(item)[0];
                }
                Some(NodeKind::Attribute | NodeKind::AttributeSetTrue) => {
                    item = self.ast.get_children(item)[1]
                }
                _ => return (item, visibility),
            }
        }
    }

    fn scan_module_def(
        &mut self,
        parent_scope: ScopeId,
        item: NodeIndex,
        vis: Visibility,
    ) -> ResolveResult<()> {
        let name = self.extract_name(self.ast.get_children(item)[0])?;
        let def_id = self.def_gen.next();
        let scope_id = self.scope_gen.next();

        // Create module scope
        let scope = Scope::new(
            scope_id,
            ScopeKind::Module,
            Some(parent_scope),
            Some(name),
            def_id,
            false, // unordered
        );
        self.scope_tree.add_scope(scope);
        self.scope_tree.add_child(parent_scope, scope_id);

        // Register the module name in the parent scope
        self.define_in_scope(
            name,
            BindingKind::Definition(def_id, parent_scope),
            Some(item),
            parent_scope,
            vis,
        )?;
        self.def_names.push((def_id, name));

        // ModuleDef has two children: name and body.
        let block_index = self.ast.get_children(item)[1];
        let block_items_index = self.ast.get_children(block_index)[0];
        if let Some(block_items) = self.ast.get_multi_child_slice(block_items_index) {
            self.scan_items(scope_id, block_items)?;
        }

        Ok(())
    }

    fn trait_parent_paths(
        &self,
        node: NodeIndex,
        depth: usize,
    ) -> Option<Vec<Vec<(Symbol, rustc_span::Span)>>> {
        if node == 0 {
            return Some(Vec::new());
        }
        if depth >= 64 {
            return None;
        }
        let children = self.ast.get_children(node);
        match self.ast.get_node_kind(node)? {
            NodeKind::Add => {
                let mut paths = self.trait_parent_paths(children[0], depth + 1)?;
                paths.extend(self.trait_parent_paths(children[1], depth + 1)?);
                Some(paths)
            }
            _ => Some(vec![self.trait_parent_path(node, depth + 1)?]),
        }
    }
    fn trait_parent_path(
        &self,
        node: NodeIndex,
        depth: usize,
    ) -> Option<Vec<(Symbol, rustc_span::Span)>> {
        if depth >= 64 {
            return None;
        }
        let (_, span, children) = self.ast.get_node(node)?;
        match self.ast.get_node_kind(node)? {
            NodeKind::Id => Some(vec![(self.extract_name(node).ok()?, span)]),
            NodeKind::NormalFormHeadApplication => self.trait_parent_path(children[0], depth + 1),
            NodeKind::Projection => {
                let mut path = self.trait_parent_path(children[0], depth + 1)?;
                if self.ast.get_node_kind(children[1]) != Some(NodeKind::Id) {
                    return None;
                }
                path.push((self.extract_name(children[1]).ok()?, span));
                Some(path)
            }
            _ => None,
        }
    }

    fn scan_adt_def(
        &mut self,
        parent_scope: ScopeId,
        item: NodeIndex,
        scope_kind: ScopeKind,
        vis: Visibility,
    ) -> ResolveResult<()> {
        let name = self.extract_name(self.ast.get_children(item)[0])?;
        let def_id = self.def_gen.next();
        let scope_id = self.scope_gen.next();

        let mut scope = Scope::new(
            scope_id,
            scope_kind,
            Some(parent_scope),
            Some(name),
            def_id,
            false,
        );
        if scope_kind == ScopeKind::TraitBody {
            scope.trait_parent_paths = self.trait_parent_paths(self.ast.get_children(item)[1], 0);
        }
        self.scope_tree.add_scope(scope);
        self.scope_tree.add_child(parent_scope, scope_id);

        self.define_in_scope(
            name,
            BindingKind::Definition(def_id, parent_scope),
            Some(item),
            parent_scope,
            vis,
        )?;
        self.def_names.push((def_id, name));

        // Scan clauses
        let clause_index = if scope_kind == ScopeKind::TraitBody {
            2
        } else {
            1
        };
        self.scan_clauses(scope_id, def_id, item, clause_index)?;

        // Scan body items (child[2] = body block, child[0] = items list)
        let body_index = self.ast.get_children(item)[clause_index + 1];
        let body_items_index = self.ast.get_children(body_index)[0];
        if let Some(body_items) = self.ast.get_multi_child_slice(body_items_index) {
            self.scan_items(scope_id, body_items)?;
        }

        Ok(())
    }

    fn scan_named_type(
        &mut self,
        parent: ScopeId,
        item: NodeIndex,
        vis: Visibility,
    ) -> ResolveResult<()> {
        let name = self.extract_name(self.ast.get_children(item)[0])?;
        let definition = self.def_gen.next();
        let scope = self.scope_gen.next();
        self.scope_tree.add_scope(Scope::new(
            scope,
            if self.ast.get_node_kind(item) == Some(NodeKind::NewtypeDef) {
                ScopeKind::Newtype
            } else {
                ScopeKind::TypeAlias
            },
            Some(parent),
            None,
            definition,
            true,
        ));
        self.scope_tree.add_child(parent, scope);
        self.define_in_scope(
            name,
            BindingKind::Definition(definition, parent),
            Some(item),
            parent,
            vis,
        )?;
        self.def_names.push((definition, name));
        self.scan_clauses(scope, definition, item, 1)
    }

    fn scan_function_def(
        &mut self,
        parent_scope: ScopeId,
        item: NodeIndex,
        vis: Visibility,
    ) -> ResolveResult<()> {
        let name = if matches!(
            self.ast.get_node_kind(item),
            Some(NodeKind::TestDef | NodeKind::TestTemplateInstance)
        ) {
            crate::test_definition_name(
                self.file_id,
                item,
                if self.ast.get_children(item)[0] == 0 {
                    None
                } else {
                    Some(self.extract_name(self.ast.get_children(item)[0])?)
                },
            )
        } else {
            self.extract_name(self.ast.get_children(item)[0])?
        };
        let def_id = self.def_gen.next();
        let scope_id = self.scope_gen.next();

        // Function body scope (ordered – names must be declared before use)
        let mut scope = Scope::new(
            scope_id,
            ScopeKind::FnBody,
            Some(parent_scope),
            None, // anonymous body scope
            def_id,
            true, // ordered
        );
        if matches!(
            self.ast.get_node_kind(item),
            Some(NodeKind::Function | NodeKind::DeriverDef)
        ) {
            let entries = self.ast.get_children(item)[1];
            let mut parameters = Vec::new();
            let mut known = true;
            for &entry in self.ast.get_multi_child_slice(entries).unwrap_or(&[]) {
                let source_entry = entry;
                let mut staged = false;
                let mut entry = entry;
                loop {
                    match self.ast.get_node_kind(entry) {
                        Some(NodeKind::Attribute | NodeKind::AttributeSetTrue) => {
                            entry = self.ast.get_children(entry)[1]
                        }
                        Some(NodeKind::ComptimeParam) => {
                            staged = true;
                            entry = self.ast.get_children(entry)[0];
                        }
                        Some(
                            NodeKind::ImplicitParam
                            | NodeKind::LambdaParam
                            | NodeKind::ErrorParam
                            | NodeKind::CatchParam
                            | NodeKind::QuoteParam,
                        ) => entry = self.ast.get_children(entry)[0],
                        _ => break,
                    }
                }
                let name = match self.ast.get_node_kind(entry) {
                    Some(
                        NodeKind::TypeBoundParam | NodeKind::OptionalParam | NodeKind::VarargParam,
                    ) => self.ast.get_children(entry)[0],
                    Some(NodeKind::Id) => entry,
                    _ => {
                        known = false;
                        continue;
                    }
                };
                if staged {
                    scope.staged_parameters.push((
                        AstNodeRef::new(self.file_id, source_entry),
                        self.def_gen.next(),
                    ));
                }
                parameters.push((
                    self.extract_name(name)?,
                    self.ast.get_span(name).unwrap_or_default(),
                ));
            }
            scope.function_parameters = known.then_some(parameters);
            let mut result = self.ast.get_children(item)[2];
            if self.ast.get_node_kind(result) == Some(NodeKind::ResultWithId) {
                result = self.ast.get_children(result)[1];
            }
            if self.ast.get_node_kind(result) == Some(NodeKind::ContextualPatternType) {
                let entries = self.ast.get_children(result)[1];
                let mut outputs = Vec::new();
                for &entry in self.ast.get_multi_child_slice(entries).unwrap_or(&[]) {
                    let name = self.ast.get_children(entry)[0];
                    outputs.push((
                        self.extract_name(name)?,
                        self.ast.get_span(name).unwrap_or_default(),
                    ));
                }
                scope.pattern_result_outputs = Some(outputs);
            }
        }
        if self.ast.get_node_kind(item) == Some(NodeKind::TestTemplateDef) {
            let clauses = self.ast.get_children(item)[1];
            let mut anchors = Vec::new();
            for &clause in self.ast.get_multi_child_slice(clauses).unwrap_or(&[]) {
                let mut clause = clause;
                while self.ast.get_node_kind(clause) == Some(NodeKind::QuoteDeclClause) {
                    clause = self.ast.get_children(clause)[0];
                }
                if matches!(
                    self.ast.get_node_kind(clause),
                    Some(
                        NodeKind::TypeBoundDeclClause
                            | NodeKind::TypeDeclClause
                            | NodeKind::TraitBoundDeclClause
                    )
                ) {
                    anchors.push(clause);
                }
            }
            if let [anchor] = anchors.as_slice() {
                if self.ast.get_node_kind(*anchor) == Some(NodeKind::TypeBoundDeclClause) {
                    let ty = self.ast.get_children(*anchor)[1];
                    if self.ast.get_node_kind(ty) == Some(NodeKind::ContextualExprType) {
                        let entries = self.ast.get_children(ty)[1];
                        let mut names = Vec::new();
                        for &entry in self.ast.get_multi_child_slice(entries).unwrap_or(&[]) {
                            let name = self.ast.get_children(entry)[0];
                            names.push((
                                self.extract_name(name)?,
                                self.ast.get_span(name).unwrap_or_default(),
                            ));
                        }
                        scope.test_template_context = Some(names);
                    }
                }
            }
        }
        self.scope_tree.add_scope(scope);
        self.scope_tree.add_child(parent_scope, scope_id);

        // Register the function name in the parent scope
        self.define_in_scope(
            name,
            BindingKind::Definition(def_id, parent_scope),
            Some(item),
            parent_scope,
            vis,
        )?;
        self.def_names.push((def_id, name));

        if self.ast.get_node_kind(item) == Some(NodeKind::TestDef) {
            self.scan_runtime_blocks(scope_id, self.ast.get_children(item)[1])?;
            return Ok(());
        }
        if self.ast.get_node_kind(item) == Some(NodeKind::TestTemplateInstance) {
            return Ok(());
        }
        if self.ast.get_node_kind(item) == Some(NodeKind::TestTemplateDef) {
            return self.scan_clauses(scope_id, def_id, item, 1);
        }
        // Function/effect clauses use slot 3; associated declarations use slot 4.
        if matches!(
            self.ast.get_node_kind(item),
            Some(NodeKind::FunctionTemplateInstance | NodeKind::FunctionTemplateInstanceWithParams)
        ) {
            return Ok(());
        }
        if self.ast.get_node_kind(item) == Some(NodeKind::NormalFormHeadDef) {
            self.scan_clauses(scope_id, def_id, item, 1)?;
            self.scan_clauses(scope_id, def_id, item, 3)?;
        } else if matches!(
            self.ast.get_node_kind(item),
            Some(NodeKind::Function | NodeKind::DeriverDef | NodeKind::AlgebraicEffect)
        ) {
            self.scan_clauses(scope_id, def_id, item, 3)?;
        } else {
            self.scan_clauses(scope_id, def_id, item, 4)?;
        }

        if matches!(
            self.ast.get_node_kind(item),
            Some(NodeKind::Function | NodeKind::DeriverDef)
        ) {
            self.scan_runtime_blocks(scope_id, self.ast.get_children(item)[4])?;
        }
        Ok(())
    }

    /// Discover static declarations without treating ordinary local bindings as
    /// unordered items. Each source block has its own file-qualified scope.
    fn scan_runtime_blocks(&mut self, parent: ScopeId, node: NodeIndex) -> ResolveResult<()> {
        use ast::NodeType as T;
        let Some(kind) = self.ast.get_node_kind(node) else {
            return Ok(());
        };
        if matches!(
            kind,
            NodeKind::QuoteExpr
                | NodeKind::QuotePattern
                | NodeKind::FunctionTemplateDef
                | NodeKind::TestDef
                | NodeKind::TestTemplateDef
                | NodeKind::TestTemplateInstance
                | NodeKind::FunctionTemplateInstance
                | NodeKind::FunctionTemplateInstanceWithParams
        ) {
            return Ok(());
        }
        let children = self.ast.get_children(node);
        if kind == NodeKind::Block {
            let id = self.scope_gen.next();
            let owner = self.scope_tree.get(parent).expect("block parent").owner_def;
            let mut scope = Scope::new(id, ScopeKind::Block, Some(parent), None, owner, false);
            scope.block_site = Some(AstNodeRef::new(self.file_id, node));
            self.scope_tree.add_scope(scope);
            self.scope_tree.add_child(parent, id);
            let statements = self.ast.get_multi_child_slice(children[0]).unwrap_or(&[]);
            for &statement in statements {
                let (inner, _) = self.strip_pub_wrapper(statement);
                if !matches!(
                    self.ast.get_node_kind(inner),
                    Some(
                        NodeKind::ConstDecl
                            | NodeKind::TestDef
                            | NodeKind::TestTemplateDef
                            | NodeKind::TestTemplateInstance
                    )
                ) {
                    self.scan_items(id, &[statement])?;
                }
            }
            for &statement in statements {
                self.scan_runtime_blocks(id, statement)?;
            }
            return Ok(());
        }
        let (single, multi) = match kind.node_type() {
            T::NoChild => (0, None),
            T::SingleChild => (1, None),
            T::DoubleChildren => (2, None),
            T::TripleChildren => (3, None),
            T::QuadrupleChildren => (4, None),
            T::MultiChildren => (0, Some(0)),
            T::SingleWithMultiChildren => (1, Some(1)),
            T::DoubleWithMultiChildren => (2, Some(2)),
            T::TripleWithMultiChildren => (3, Some(3)),
            // Declaration bodies have their own scanner; raw FnType flags are
            // not AST indices. Neither may be traversed as runtime syntax.
            _ => return Ok(()),
        };
        for &child in &children[..single] {
            self.scan_runtime_blocks(parent, child)?;
        }
        if let Some(index) = multi {
            for &child in self
                .ast
                .get_multi_child_slice(children[index])
                .unwrap_or(&[])
            {
                self.scan_runtime_blocks(parent, child)?;
            }
        }
        Ok(())
    }

    /// Scan an `impl` or `impl Trait for Type` block.
    ///
    /// Layout:
    /// - `ImplDef`      (`TypeDefChildren`):      `[type_expr, clauses_N, body]`
    /// - `ImplTraitDef` (`ImplTraitDefChildren`):  `[trait_expr, type_expr, clauses_N, body]`
    ///
    /// Creates a body scope, scans the body items into it, and pushes an
    /// [`ImplDirective`] for later phases.
    fn scan_impl_def(
        &mut self,
        owner_scope: ScopeId,
        item: NodeIndex,
        kind: ImplKind,
    ) -> ResolveResult<()> {
        let span = self.ast.get_span(item).unwrap_or_default();
        let def_id = self.def_gen.next();
        let impl_scope_id = self.scope_gen.next();

        // Body index and clauses index differ by kind.
        let (clauses_child, body_child) = match kind {
            ImplKind::Inherent => (1, Some(2)),  // a, N, b
            ImplKind::TraitImpl => (2, Some(3)), // a, b, N, c
            ImplKind::Derivation => (2, None),   // trait, subject, clauses
        };

        let scope = Scope::new(
            impl_scope_id,
            ScopeKind::ImplBlock,
            Some(owner_scope),
            None,
            def_id,
            false,
        );
        self.scope_tree.add_scope(scope);
        self.scope_tree.add_child(owner_scope, impl_scope_id);

        // Scan clauses into the impl scope.
        self.scan_clauses(impl_scope_id, def_id, item, clauses_child)?;

        // Scan body items into the impl scope.
        let children = self.ast.get_children(item);
        if let Some(body_child) = body_child {
            let body_index = children[body_child];
            let body_items_index = self.ast.get_children(body_index)[0];
            if let Some(body_items) = self.ast.get_multi_child_slice(body_items_index) {
                self.scan_items(impl_scope_id, body_items)?;
            }
        }

        self.impls.push(ImplDirective {
            def_id,
            owner_scope,
            impl_scope: impl_scope_id,
            kind,
            ast_node: item,
            file_id: self.file_id,
            span,
        });

        Ok(())
    }

    fn scan_clauses(
        &mut self,
        scope_id: ScopeId,
        _owner_def: DefId,
        item: NodeIndex,
        clauses_child_index: usize,
    ) -> ResolveResult<()> {
        let children = self.ast.get_children(item);
        if clauses_child_index >= children.len() {
            return Ok(());
        }
        let clauses_index = children[clauses_child_index];
        let clauses = match self.ast.get_multi_child_slice(clauses_index) {
            Some(c) => c,
            None => return Ok(()),
        };

        let mut seen = std::collections::HashSet::new();
        for &outer in clauses {
            let mut clause = outer;
            while self.ast.get_node_kind(clause) == Some(NodeKind::QuoteDeclClause) {
                clause = self.ast.get_children(clause)[0];
            }
            let Some(kind) = self.ast.get_node_kind(clause) else {
                continue;
            };
            if matches!(
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
                let name = self.extract_name(if kind == NodeKind::Id {
                    clause
                } else {
                    self.ast.get_children(clause)[0]
                })?;
                if !seen.insert(name) {
                    return Err(ResolveError::InvalidNodeType {
                        message: format!("duplicate clause parameter {name}"),
                        span: self.ast.get_span(clause).unwrap_or_default(),
                    });
                }
                let clause_def = self.def_gen.next();
                let binding = Binding {
                    entry: None,
                    kind: BindingKind::ClauseParam(
                        clause_def,
                        AstNodeRef::new(self.file_id, clause),
                    ),
                    vis: Visibility::Private,
                };
                if let Some(scope) = self.scope_tree.get_mut(scope_id) {
                    scope.items.add_clause(name, binding);
                }
                self.def_names.push((clause_def, name));
            }
        }

        Ok(())
    }

    fn collect_import(
        &mut self,
        parent_scope: ScopeId,
        item: NodeIndex,
        visibility: Visibility,
    ) -> ResolveResult<()> {
        let path_node = self.ast.get_children(item)[0];

        self.expand_import(
            parent_scope,
            item,
            path_node,
            PathAnchor::Local,
            Vec::new(),
            visibility,
        )
    }

    fn expand_import(
        &mut self,
        owner: ScopeId,
        item: NodeIndex,
        node: NodeIndex,
        inherited: PathAnchor,
        mut prefix: Vec<Symbol>,
        visibility: Visibility,
    ) -> ResolveResult<()> {
        let (anchor, inner) = self.extract_path_anchor(node)?;
        let anchor = if anchor == PathAnchor::Local {
            inherited
        } else {
            if !prefix.is_empty() {
                return Err(ResolveError::InvalidNodeType {
                    message: "anchored path inside a grouped prefix is not implemented".into(),
                    span: self.ast.get_span(node).unwrap_or_default(),
                });
            }
            anchor
        };
        if self.ast.get_node_kind(inner) == Some(NodeKind::ProjectionMultiPath) {
            let children = self.ast.get_children(inner);
            prefix.extend(self.collect_prefix_segments(children[0])?);
            let members = self
                .ast
                .get_multi_child_slice(children[1])
                .unwrap_or(&[])
                .to_vec();
            for child in members {
                self.expand_import(owner, item, child, anchor, prefix.clone(), visibility)?;
            }
        } else {
            let (_, suffix, kind) = self.extract_import_path(inner)?;
            prefix.extend(suffix);
            self.imports.push(ImportDirective::new(
                owner,
                anchor,
                kind,
                prefix,
                self.ast.get_span(node).unwrap_or_default(),
                item,
                self.file_id,
                visibility,
            ));
        }
        Ok(())
    }

    /// Strip leading `SuperPath` / `PackagePath` nodes and return the anchor
    /// plus the innermost path node.
    ///
    /// `..a.b` → `(Super(2), projection_path(a, b))`
    /// `@a.b`  → `(Package,  projection_path(a, b))`
    /// `a.b`   → `(Local,    projection_path(a, b))`
    fn extract_path_anchor(&self, node: NodeIndex) -> ResolveResult<(PathAnchor, NodeIndex)> {
        let Some((kind, _span, children)) = self.ast.get_node(node) else {
            return Err(ResolveError::InternalError("Invalid path node".into()));
        };

        match kind {
            NodeKind::SuperPath => {
                let (inner_anchor, inner_node) = self.extract_path_anchor(children[0])?;
                let depth = match inner_anchor {
                    PathAnchor::Local => 1,
                    PathAnchor::Super(n) => n + 1,
                    PathAnchor::Package => {
                        return Err(ResolveError::InvalidNodeType {
                            message: "Cannot nest `@` inside `.` path prefix".into(),
                            span: self.ast.get_span(node).unwrap_or_default(),
                        });
                    }
                };
                Ok((PathAnchor::Super(depth), inner_node))
            }
            NodeKind::PackagePath => Ok((PathAnchor::Package, children[0])),
            _ => Ok((PathAnchor::Local, node)),
        }
    }

    /// Recursively extract path anchor, prefix segments, and import kind from
    /// a use-path AST node.
    fn extract_import_path(
        &self,
        path_node: NodeIndex,
    ) -> ResolveResult<(PathAnchor, Vec<Symbol>, ImportKind)> {
        // Step 1: strip Super / Package anchor from the outermost node.
        let (anchor, inner_node) = self.extract_path_anchor(path_node)?;

        let kind = self
            .ast
            .get_node_kind(inner_node)
            .ok_or_else(|| ResolveError::InternalError("Invalid path node".into()))?;

        // Step 2: extract prefix segments and import kind from the inner path.
        match kind {
            NodeKind::Id => {
                let name = self.extract_name(inner_node)?;
                Ok((
                    anchor,
                    vec![],
                    ImportKind::Single {
                        source_scope: None,
                        name,
                    },
                ))
            }
            NodeKind::ProjectionPath => {
                let children = self.ast.get_children(inner_node);
                let right_name = self.extract_name(children[1])?;
                let left_segs = self.collect_prefix_segments(children[0])?;
                Ok((
                    anchor,
                    left_segs,
                    ImportKind::Single {
                        source_scope: None,
                        name: right_name,
                    },
                ))
            }
            NodeKind::ProjectionAllPath => {
                let children = self.ast.get_children(inner_node);
                let segs = self.collect_prefix_segments(children[0])?;
                Ok((anchor, segs, ImportKind::Glob { source_scope: None }))
            }
            NodeKind::ProjectionMultiPath => {
                let children = self.ast.get_children(inner_node);
                let segs = self.collect_prefix_segments(children[0])?;
                let names_slice = self.ast.get_multi_child_slice(children[1]).ok_or_else(|| {
                    ResolveError::InternalError("Invalid multi-import names".into())
                })?;
                let mut names = Vec::new();
                for &name_node in names_slice {
                    names.push(self.extract_name(name_node)?);
                }
                Ok((
                    anchor,
                    segs,
                    ImportKind::Multi {
                        source_scope: None,
                        names,
                    },
                ))
            }
            NodeKind::PathAsBind => {
                let children = self.ast.get_children(inner_node);
                let alias = self.extract_name(children[1])?;
                // Inner path must resolve to a single name.
                let (original, left_segs) = match self.ast.get_node_kind(children[0]) {
                    Some(NodeKind::Id) => (self.extract_name(children[0])?, vec![]),
                    Some(NodeKind::ProjectionPath) => {
                        let ic = self.ast.get_children(children[0]);
                        (
                            self.extract_name(ic[1])?,
                            self.collect_prefix_segments(ic[0])?,
                        )
                    }
                    _ => {
                        return Err(ResolveError::InvalidNodeType {
                            message: "Expected simple path inside `as` bind".into(),
                            span: self.ast.get_span(inner_node).unwrap_or_default(),
                        });
                    }
                };
                Ok((
                    anchor,
                    left_segs,
                    ImportKind::Alias {
                        source_scope: None,
                        original,
                        alias,
                    },
                ))
            }
            _ => Err(ResolveError::InvalidNodeType {
                message: format!("Unexpected node kind in use path: {:?}", kind),
                span: self.ast.get_span(inner_node).unwrap_or_default(),
            }),
        }
    }

    /// Collect flat name segments from a prefix path node (`Id` or chained
    /// `ProjectionPath`).  No anchors allowed here — they are stripped by
    /// `extract_path_anchor` before this is called.
    fn collect_prefix_segments(&self, node: NodeIndex) -> ResolveResult<Vec<Symbol>> {
        let Some((kind, _span, children)) = self.ast.get_node(node) else {
            return Err(ResolveError::InternalError("Invalid path node".into()));
        };

        match kind {
            NodeKind::Id => Ok(vec![self.extract_name(node)?]),
            NodeKind::ProjectionPath => {
                let mut result = self.collect_prefix_segments(children[0])?;
                result.push(self.extract_name(children[1])?);
                Ok(result)
            }
            _ => Err(ResolveError::InvalidNodeType {
                message: format!("Unexpected node kind in path segment: {:?}", kind),
                span: self.ast.get_span(node).unwrap_or_default(),
            }),
        }
    }

    fn extract_name(&self, id_node: NodeIndex) -> ResolveResult<Symbol> {
        let children = self.ast.get_children(id_node);
        // SAFETY: hi/lo produced by Symbol::to_raw_parts() in the parser.
        Ok(unsafe { Symbol::from_raw_parts(children[0], children[1]) })
    }

    fn source_text(&self, node: NodeIndex) -> ResolveResult<String> {
        self.source_text_raw(node)
    }

    fn source_text_raw(&self, node: NodeIndex) -> ResolveResult<String> {
        self.ast
            .source_content(node, self.source_map)
            .ok_or_else(|| {
                ResolveError::InternalError(format!(
                    "Failed to get source content for node {}",
                    node
                ))
            })
    }

    fn define_in_scope(
        &mut self,
        name: Symbol,
        kind: BindingKind,
        ast_node: Option<NodeIndex>,
        scope_id: ScopeId,
        vis: Visibility,
    ) -> ResolveResult<()> {
        let binding = Binding {
            kind,
            vis,
            entry: None,
        };

        if let Some(scope) = self.scope_tree.get_mut(scope_id) {
            if let Err(_old) = scope.items.define(name, binding) {
                // In an unordered scope, duplicate is an error
                if !scope.ordered {
                    return Err(ResolveError::DuplicateDefinition {
                        name: name.as_str().to_string(),
                        first_span: rustc_span::DUMMY_SP,
                        second_span: ast_node
                            .and_then(|n| self.ast.get_span(n))
                            .unwrap_or_default(),
                    });
                }
            }
        }
        Ok(())
    }
}
