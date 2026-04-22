//! AST scanner – walks a single file's AST to populate scopes and collect
//! unresolved import directives.
//!
//! Migrated and refactored from `luna/src/scan/ast_scanner.rs`.

use ast::{Ast, NodeIndex, NodeKind};
use rustc_span::SourceMap;

use crate::binding::{Binding, BindingKind, Visibility};
use crate::error::{ResolveError, ResolveResult};
use crate::ids::{AstNodeRef, DefId, DefIdGen, ScopeId, ScopeIdGen};
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
            let item_kind = self
                .ast
                .get_node_kind(item)
                .ok_or_else(|| ResolveError::InternalError("Invalid node index".into()))?;

            match item_kind {
                NodeKind::ModuleDef => {
                    self.scan_module_def(parent_scope, item)?;
                }
                NodeKind::StructDef => {
                    self.scan_adt_def(parent_scope, item, BindingKind::Struct, ScopeKind::AdtBody)?;
                }
                NodeKind::EnumDef => {
                    self.scan_adt_def(parent_scope, item, BindingKind::Enum, ScopeKind::AdtBody)?;
                }
                NodeKind::UnionDef => {
                    self.scan_adt_def(parent_scope, item, BindingKind::Union, ScopeKind::AdtBody)?;
                }
                NodeKind::Function => {
                    self.scan_function_def(parent_scope, item)?;
                }
                NodeKind::UseStatement => {
                    self.collect_import(parent_scope, item)?;
                }
                _ => {
                    // Other node kinds are ignored during the scan pass.
                }
            }
        }
        Ok(())
    }

    // ── Module definition ────────────────────────────────────────────────

    fn scan_module_def(&mut self, parent_scope: ScopeId, item: NodeIndex) -> ResolveResult<()> {
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
        self.define_in_scope(parent_scope, &name, def_id, BindingKind::Module, Some(item))?;
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

    // ── ADT definitions (struct, enum, union) ────────────────────────────

    fn scan_adt_def(
        &mut self,
        parent_scope: ScopeId,
        item: NodeIndex,
        kind: BindingKind,
        scope_kind: ScopeKind,
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

        // Register in both Type and Value namespace (e.g. struct is both a type
        // and a constructor).
        self.define_in_scope_ns(parent_scope, &name, def_id, kind, Some(item))?;
        self.define_in_scope_ns(parent_scope, &name, def_id, kind, Some(item))?;
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

    // ── Function definition ──────────────────────────────────────────────

    fn scan_function_def(&mut self, parent_scope: ScopeId, item: NodeIndex) -> ResolveResult<()> {
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

        // Register the function name in the parent scope (Value namespace)
        self.define_in_scope_ns(
            parent_scope,
            &name,
            def_id,
            BindingKind::Function,
            Some(item),
        )?;
        self.def_names.push((def_id, name));

        // Scan clauses (child[4] in the old AST layout)
        self.scan_clauses(scope_id, def_id, item, 4)?;

        Ok(())
    }

    // ── Clauses ──────────────────────────────────────────────────────────

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
                        kind: BindingKind::TypeParam,
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
                        kind: BindingKind::TypeParam,
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

    // ── Use-statement collection ─────────────────────────────────────────

    fn collect_import(&mut self, parent_scope: ScopeId, item: NodeIndex) -> ResolveResult<()> {
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
        ));

        Ok(())
    }

    /// Recursively extract path segments and the import kind from a use-path AST node.
    fn extract_import_path(
        &self,
        path_node: NodeIndex,
    ) -> ResolveResult<(Vec<String>, ImportKind)> {
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
                let mut segments = vec!["super".to_string()];
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

    /// Collect all path segments from a path node into a flat `Vec<String>`.
    fn collect_path_segments(&self, node: NodeIndex) -> ResolveResult<Vec<String>> {
        let kind = self
            .ast
            .get_node_kind(node)
            .ok_or_else(|| ResolveError::InternalError("Invalid path node".into()))?;

        match kind {
            NodeKind::Id => Ok(vec![self.source_text(node)?]),
            NodeKind::ProjectionPath => {
                let children = self.ast.get_children(node);
                let mut result = self.collect_path_segments(children[0])?;
                result.push(self.source_text(children[1])?);
                Ok(result)
            }
            NodeKind::SuperPath => {
                let children = self.ast.get_children(node);
                let mut result = vec!["super".to_string()];
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

    // ── Helpers ──────────────────────────────────────────────────────────

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
    ) -> ResolveResult<()> {
        self.define_in_scope_ns(scope_id, name, def_id, kind, ast_node)
    }

    fn define_in_scope_ns(
        &mut self,
        scope_id: ScopeId,
        name: &str,
        def_id: DefId,
        kind: BindingKind,
        ast_node: Option<NodeIndex>,
    ) -> ResolveResult<()> {
        let binding = Binding {
            kind,
            def_id,
            defined_in: scope_id,
            ast_ref: ast_node.map(|n| AstNodeRef::new(self.file_id, n)),
            vis: Visibility::Public,
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
