//! AST scanner – walks a single file's AST to populate scopes and collect
//! unresolved import directives.
//!
//! Migrated and refactored from `luna/src/scan/ast_scanner.rs`.

use ast::{Ast, NodeIndex, NodeKind};
use rustc_span::SourceMap;

use crate::binding::{Binding, BindingKind, Visibility};
use crate::error::{ResolveError, ResolveResult};
use crate::ids::{AstNodeRef, DefId, DefIdGen, ScopeId, ScopeIdGen};
use crate::impl_directive::{ImplDirective, ImplKind};
use crate::import::{ImportDirective, ImportKind, PathSegment};
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
    pub def_names: &'a mut Vec<(DefId, String)>,
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
                NodeKind::StructDef => {
                    self.scan_adt_def(
                        parent_scope,
                        inner,
                        BindingKind::Struct,
                        ScopeKind::AdtBody,
                        vis,
                    )?;
                }
                NodeKind::EnumDef => {
                    self.scan_adt_def(
                        parent_scope,
                        inner,
                        BindingKind::Enum,
                        ScopeKind::AdtBody,
                        vis,
                    )?;
                }
                NodeKind::UnionDef => {
                    self.scan_adt_def(
                        parent_scope,
                        inner,
                        BindingKind::Union,
                        ScopeKind::AdtBody,
                        vis,
                    )?;
                }
                NodeKind::Function => {
                    self.scan_function_def(parent_scope, inner, vis)?;
                }
                NodeKind::ImplDef => {
                    self.scan_impl_def(parent_scope, inner, ImplKind::Inherent)?;
                }
                NodeKind::ImplTraitDef => {
                    self.scan_impl_def(parent_scope, inner, ImplKind::TraitImpl)?;
                }
                NodeKind::UseStatement => {
                    self.collect_import(parent_scope, inner, vis == Visibility::Public)?;
                }
                _ => {
                    // Other node kinds are ignored during the scan pass.
                }
            }
        }
        Ok(())
    }

    /// Returns `(inner_node, visibility)`, stripping a `Pub` wrapper if present.
    /// Unwrapped items default to [`Visibility::Package`].
    fn strip_pub_wrapper(&self, item: NodeIndex) -> (NodeIndex, Visibility) {
        if self.ast.get_node_kind(item) == Some(NodeKind::Pub) {
            let children = self.ast.get_children(item);
            if !children.is_empty() {
                return (children[0], Visibility::Public);
            }
        }
        (item, Visibility::Package)
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
            Some(name.clone()),
            def_id,
            false, // unordered
        );
        self.scope_tree.add_scope(scope);
        self.scope_tree.add_child(parent_scope, scope_id);

        // Register the module name in the parent scope
        self.define_in_scope(
            parent_scope,
            &name,
            def_id,
            BindingKind::Module,
            Some(item),
            vis,
        )?;
        self.def_names.push((def_id, name));

        // Scan clauses
        self.scan_clauses(scope_id, def_id, item, 1)?;

        // Scan the module body (child[2] = block, block child[0] = items)
        let block_index = self.ast.get_children(item)[2];
        let block_items_index = self.ast.get_children(block_index)[0];
        if let Some(block_items) = self.ast.get_multi_child_slice(block_items_index) {
            self.scan_items(scope_id, block_items)?;
        }

        Ok(())
    }

    fn scan_adt_def(
        &mut self,
        parent_scope: ScopeId,
        item: NodeIndex,
        kind: BindingKind,
        scope_kind: ScopeKind,
        vis: Visibility,
    ) -> ResolveResult<()> {
        let name = self.extract_name(self.ast.get_children(item)[0])?;
        let def_id = self.def_gen.next();
        let scope_id = self.scope_gen.next();

        let scope = Scope::new(
            scope_id,
            scope_kind,
            Some(parent_scope),
            Some(name.clone()),
            def_id,
            false,
        );
        self.scope_tree.add_scope(scope);
        self.scope_tree.add_child(parent_scope, scope_id);

        self.define_in_scope(parent_scope, &name, def_id, kind, Some(item), vis)?;
        self.def_names.push((def_id, name));

        // Scan clauses
        self.scan_clauses(scope_id, def_id, item, 1)?;

        // Scan body items (child[2] = body block, child[0] = items list)
        let body_index = self.ast.get_children(item)[2];
        let body_items_index = self.ast.get_children(body_index)[0];
        if let Some(body_items) = self.ast.get_multi_child_slice(body_items_index) {
            self.scan_items(scope_id, body_items)?;
        }

        Ok(())
    }

    fn scan_function_def(
        &mut self,
        parent_scope: ScopeId,
        item: NodeIndex,
        vis: Visibility,
    ) -> ResolveResult<()> {
        let name = self.extract_name(self.ast.get_children(item)[0])?;
        let def_id = self.def_gen.next();
        let scope_id = self.scope_gen.next();

        // Function body scope (ordered – names must be declared before use)
        let scope = Scope::new(
            scope_id,
            ScopeKind::FnBody,
            Some(parent_scope),
            None, // anonymous body scope
            def_id,
            true, // ordered
        );
        self.scope_tree.add_scope(scope);
        self.scope_tree.add_child(parent_scope, scope_id);

        // Register the function name in the parent scope
        self.define_in_scope(
            parent_scope,
            &name,
            def_id,
            BindingKind::Function,
            Some(item),
            vis,
        )?;
        self.def_names.push((def_id, name));

        // Scan clauses (child[4] in the old AST layout)
        self.scan_clauses(scope_id, def_id, item, 4)?;

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
            ImplKind::Inherent => (1, 2),  // a, N, b
            ImplKind::TraitImpl => (2, 3), // a, b, N, c
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
        if body_child < children.len() {
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

        for &clause in clauses {
            let Some((kind, _span, _)) = self.ast.get_node(clause) else {
                continue;
            };

            match kind {
                NodeKind::TypeDeclClause => {
                    let name = self.extract_name(self.ast.get_children(clause)[0])?;
                    let clause_def = self.def_gen.next();
                    let binding = Binding {
                        kind: BindingKind::ClauseParam,
                        def_id: clause_def,
                        defined_in: scope_id,
                        ast_ref: Some(AstNodeRef::new(self.file_id, clause)),
                        vis: Visibility::Private,
                    };
                    if let Some(scope) = self.scope_tree.get_mut(scope_id) {
                        scope.items.add_clause(name.clone(), binding);
                    }
                    self.def_names.push((clause_def, name));
                }
                NodeKind::TypeBoundDeclClause => {
                    let name = self.extract_name(self.ast.get_children(clause)[0])?;
                    let clause_def = self.def_gen.next();
                    let binding = Binding {
                        kind: BindingKind::ClauseParam,
                        def_id: clause_def,
                        defined_in: scope_id,
                        ast_ref: Some(AstNodeRef::new(self.file_id, clause)),
                        vis: Visibility::Private,
                    };
                    if let Some(scope) = self.scope_tree.get_mut(scope_id) {
                        scope.items.add_clause(name.clone(), binding);
                    }
                    self.def_names.push((clause_def, name));
                }
                NodeKind::OptionalDeclClause => {
                    let name = self.extract_name(self.ast.get_children(clause)[0])?;
                    let clause_def = self.def_gen.next();
                    let binding = Binding {
                        kind: BindingKind::Param,
                        def_id: clause_def,
                        defined_in: scope_id,
                        ast_ref: Some(AstNodeRef::new(self.file_id, clause)),
                        vis: Visibility::Private,
                    };
                    if let Some(scope) = self.scope_tree.get_mut(scope_id) {
                        scope.items.add_clause(name.clone(), binding);
                    }
                    self.def_names.push((clause_def, name));
                }
                NodeKind::Requires | NodeKind::Ensures | NodeKind::Decreases => {
                    // Contract clauses – nothing to define
                }
                _ => {
                    // Unknown clause kind – skip
                }
            }
        }

        Ok(())
    }

    fn collect_import(
        &mut self,
        parent_scope: ScopeId,
        item: NodeIndex,
        is_reexport: bool,
    ) -> ResolveResult<()> {
        let span = self.ast.get_span(item).unwrap_or_default();
        let path_node = self.ast.get_children(item)[0];

        let (path_segments, kind) = self.extract_import_path(path_node)?;

        self.imports.push(ImportDirective::new(
            parent_scope,
            kind,
            path_segments,
            span,
            item,
            self.file_id,
            is_reexport,
        ));

        Ok(())
    }

    /// Recursively extract path segments and the import kind from a use-path AST node.
    fn extract_import_path(
        &self,
        path_node: NodeIndex,
    ) -> ResolveResult<(Vec<PathSegment>, ImportKind)> {
        let kind = self
            .ast
            .get_node_kind(path_node)
            .ok_or_else(|| ResolveError::InternalError("Invalid path node".into()))?;

        match kind {
            NodeKind::Id => {
                let name = self.source_text(path_node)?;
                Ok((
                    vec![],
                    ImportKind::Single {
                        source_scope: None,
                        name,
                    },
                ))
            }
            NodeKind::ProjectionPath => {
                let children = self.ast.get_children(path_node);
                let right_name = self.source_text(children[1])?;
                let left_segments = self.collect_path_segments(children[0])?;
                Ok((
                    left_segments,
                    ImportKind::Single {
                        source_scope: None,
                        name: right_name,
                    },
                ))
            }
            NodeKind::ProjectionAllPath => {
                let children = self.ast.get_children(path_node);
                let segments = self.collect_path_segments(children[0])?;
                Ok((segments, ImportKind::Glob { source_scope: None }))
            }
            NodeKind::ProjectionMultiPath => {
                let children = self.ast.get_children(path_node);
                let segments = self.collect_path_segments(children[0])?;
                let names_slice = self.ast.get_multi_child_slice(children[1]).ok_or_else(|| {
                    ResolveError::InternalError("Invalid multi-import names".into())
                })?;
                let mut names = Vec::new();
                for &name_node in names_slice {
                    names.push(self.source_text(name_node)?);
                }
                Ok((
                    segments,
                    ImportKind::Multi {
                        source_scope: None,
                        names,
                    },
                ))
            }
            NodeKind::SuperPath => {
                let children = self.ast.get_children(path_node);
                let mut segments = vec![PathSegment::Super];
                let inner_segments = self.collect_path_segments(children[0])?;
                segments.extend(inner_segments);
                Ok((segments, ImportKind::Glob { source_scope: None }))
            }
            _ => Err(ResolveError::InvalidNodeType {
                message: format!("Unexpected node kind in use path: {:?}", kind),
                span: self.ast.get_span(path_node).unwrap_or_default(),
            }),
        }
    }

    /// Collect all path segments from a path node into a flat `Vec<PathSegment>`.
    fn collect_path_segments(&self, node: NodeIndex) -> ResolveResult<Vec<PathSegment>> {
        let kind = self
            .ast
            .get_node_kind(node)
            .ok_or_else(|| ResolveError::InternalError("Invalid path node".into()))?;

        match kind {
            NodeKind::Id => Ok(vec![PathSegment::Name(self.source_text(node)?)]),
            NodeKind::ProjectionPath => {
                let children = self.ast.get_children(node);
                let mut result = self.collect_path_segments(children[0])?;
                result.push(PathSegment::Name(self.source_text(children[1])?));
                Ok(result)
            }
            NodeKind::SuperPath => {
                let children = self.ast.get_children(node);
                let mut result = vec![PathSegment::Super];
                let rest = self.collect_path_segments(children[0])?;
                result.extend(rest);
                Ok(result)
            }
            _ => Err(ResolveError::InvalidNodeType {
                message: format!("Unexpected node kind in path segment: {:?}", kind),
                span: self.ast.get_span(node).unwrap_or_default(),
            }),
        }
    }

    fn extract_name(&self, id_node: NodeIndex) -> ResolveResult<String> {
        self.source_text(id_node)
    }

    fn source_text(&self, node: NodeIndex) -> ResolveResult<String> {
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
        scope_id: ScopeId,
        name: &str,
        def_id: DefId,
        kind: BindingKind,
        ast_node: Option<NodeIndex>,
        vis: Visibility,
    ) -> ResolveResult<()> {
        let binding = Binding {
            kind,
            def_id,
            defined_in: scope_id,
            ast_ref: ast_node.map(|n| AstNodeRef::new(self.file_id, n)),
            vis,
        };

        if let Some(scope) = self.scope_tree.get_mut(scope_id) {
            if let Err(_old) = scope.items.define(name.to_string(), binding) {
                // In an unordered scope, duplicate is an error
                if !scope.ordered {
                    return Err(ResolveError::DuplicateDefinition {
                        name: name.to_string(),
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
