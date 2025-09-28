use std::{collections::HashMap, path, vec};

use ast::{Ast, NodeIndex, NodeKind};
use vfs::Vfs;

use crate::{
    context::{
        CompilerContext,
        scope::{self, Item, ScopeId},
    },
    hir::{Hir, Import},
};

use super::error::*;

/// Represents an unresolved import statement, including the AST node, node index, and scope ID.
#[derive(Debug, Clone, Copy)]
pub struct PendingImport<'vfs>(pub &'vfs Ast, pub ast::NodeIndex, pub ScopeId);

/// Responsible for resolving all unresolved imports in the current compilation context.
/// Maintains a queue of imports and recursively resolves `use` statements within scopes.
pub struct ImportResolver<'ctx, 'hir, 'vfs> {
    /// Compiler context for the current compilation.
    ctx: &'ctx CompilerContext<'hir>,
    /// High-level intermediate representation (HIR) mapping.
    hir: &'hir Hir,
    /// Virtual file system reference.
    vfs: &'vfs Vfs,
    /// Unresolved imports grouped by scope.
    unresolved_imports: HashMap<ScopeId, Vec<PendingImport<'vfs>>>,
    /// Scopes currently being resolved (used for cycle detection).
    resolving: HashMap<ScopeId, ()>,
}

impl<'ctx, 'hir, 'vfs> ImportResolver<'ctx, 'hir, 'vfs> {
    /// Creates a new import resolver instance.
    pub fn new(
        ctx: &'ctx CompilerContext<'hir>,
        hir: &'hir Hir,
        vfs: &'vfs Vfs,
        unresolved_imports: HashMap<ScopeId, Vec<PendingImport<'vfs>>>,
    ) -> Self {
        Self {
            ctx,
            hir,
            vfs,
            unresolved_imports,
            resolving: HashMap::new(),
        }
    }

    /// Recursively resolves all unresolved imports.
    /// Checks for cyclic dependencies and returns a `ScanError` on failure.
    pub fn resolve(&mut self) -> Result<(), ScanError> {
        // TODO: ...
        let project_scope = self
            .ctx
            .scope_manager
            .lookup_path(
                &[self.hir.intern_str("test_project")],
                self.ctx.scope_manager.root,
            )
            .unwrap();
        let builtin_scope = self
            .ctx
            .scope_manager
            .lookup_path(
                &[self.hir.intern_str("builtin")],
                self.ctx.scope_manager.root,
            )
            .unwrap();
        self.ctx.scope_manager.add_import(
            project_scope.scope_id.unwrap(),
            Import::All(builtin_scope.scope_id.unwrap()),
        );

        while let Some((k, v)) = self.next() {
            self.resolve_scope_imports(k, &v)?;
        }
        Ok(())
    }

    /// Resolves all imports for the specified scope.
    fn resolve_scope_imports(
        &mut self,
        k: usize,
        imports: &[PendingImport<'vfs>],
    ) -> Result<(), ScanError> {
        self.resolving.insert(k, ());
        for todo in imports {
            self.resolve_import(todo)?;
        }
        self.unresolved_imports.remove(&k);
        self.resolving.remove(&k);
        Ok(())
    }

    /// Resolves a single `use` statement node.
    fn resolve_import(
        &mut self,
        &PendingImport(ast, node_index, scope_id): &PendingImport<'vfs>,
    ) -> Result<(), ScanError> {
        let (Some(kind), Some(span)) = (ast.get_node_kind(node_index), ast.get_span(node_index))
        else {
            return Err(ScanError::InternalError("Invalid ast node index".into()));
        };

        if kind != NodeKind::UseStatement {
            return Err(ScanError::InvalidNodeType {
                message: "Expected use statement".into(),
                span: span,
            });
        }

        let path = ast.get_children(node_index)[0];
        let imports = self.resolve_path(ast, path, scope_id)?;
        for import in imports {
            self.ctx
                .scope_manager
                .add_import(scope_id, import)
                .map_err(|_| ScanError::InternalError("Failed to add import".into()))?;
        }

        Ok(())
    }

    /// Resolves a `use` path and returns a list of import items.
    fn resolve_path(
        &mut self,
        ast: &Ast,
        path_index: NodeIndex,
        scope_id: ScopeId,
    ) -> Result<Vec<Import<'hir>>, ScanError> {
        let mut imports = vec![];
        let (Some(kind), Some(span)) = (ast.get_node_kind(path_index), ast.get_span(path_index))
        else {
            return Err(ScanError::InternalError("Invalid path node index".into()));
        };

        match kind {
            NodeKind::Id => {
                return Err(ScanError::UnresolvedIdentifier {
                    message: format!("Shall not just use a single identifier here"),
                    span,
                });
            }
            NodeKind::PathSelectAll => {
                let (_, item) =
                    self.resolve_path_inner(ast, ast.get_children(path_index)[0], scope_id)?;
                self.ensure_scope_resolved(item.scope_id.expect("Item must have a scope"), span)?;

                imports.push(Import::All(
                    item.scope_id.ok_or(ScanError::ModuleNotFound {
                        message: "This item may not have a invalid scope".into(),
                        span: ast
                            .get_span(ast.get_children(path_index)[0])
                            .expect("Invalid path select all span"),
                    })?,
                ));
            }
            NodeKind::PathSelectMulti => {
                let (_, item) =
                    self.resolve_path_inner(ast, ast.get_children(path_index)[0], scope_id)?;
                self.ensure_scope_resolved(item.scope_id.expect("Item must have a scope"), span)?;

                let mut selected = vec![];
                for &sub_path in ast
                    .get_multi_child_slice(ast.get_children(path_index)[1])
                    .expect("Invalid ast slice index")
                {
                    // TODO: 每一项都应该能是path, 不只是一个symbol
                    let selected_name = self.hir.intern_str(
                        &ast.source_content(sub_path, &self.hir.source_map)
                            .expect("Invalid path select multi content"),
                    );
                    self.ctx
                        .scope_manager
                        .lookup(
                            selected_name,
                            item.scope_id.expect("Item must have a scope"),
                        )
                        .map(|_| selected.push(selected_name))
                        .ok_or_else(|| ScanError::UnresolvedIdentifier {
                            message: format!("Unresolved identifier: {}", selected_name),
                            span: ast
                                .get_span(sub_path)
                                .expect("Invalid path select multi span"),
                        })?;
                }
                imports.push(Import::Multi(
                    item.scope_id.expect("Item must have a scope"),
                    selected,
                ));
            }
            _ => {
                let (item_located, item) = self.resolve_path_inner(ast, path_index, scope_id)?;
                imports.push(Import::Single(item_located, item.symbol));
            }
        }
        Ok(imports)
    }

    /// Recursively resolves a path node, returning the target scope and symbol item.
    fn resolve_path_inner(
        &mut self,
        ast: &Ast,
        path_index: NodeIndex,
        scope_id: ScopeId,
    ) -> Result<(ScopeId, Item<'hir>), ScanError> {
        let (Some(kind), Some(span)) = (ast.get_node_kind(path_index), ast.get_span(path_index))
        else {
            return Err(ScanError::InternalError("Invalid path node index".into()));
        };

        let result = match kind {
            NodeKind::Id => {
                let name = self.hir.intern_str(
                    &ast.source_content(path_index, &self.hir.source_map)
                        .expect("Invalid id content"),
                );
                let Some((_, item)) = self.ctx.scope_manager.resolve(name, scope_id) else {
                    return Err(ScanError::UnresolvedIdentifier {
                        message: format!("Unresolved identifier: {}", name),
                        span: ast.get_span(path_index).expect("Invalid id span"),
                    });
                };
                self.ensure_scope_resolved(item.scope_id.expect("Invalid scope id"), span)?;
                (scope_id, item)
            }
            NodeKind::SuperPath => {
                let Some(parent_scope_id) = self.parent_scope(scope_id) else {
                    return Err(ScanError::UnresolvedIdentifier {
                        message: "No parent scope".into(),
                        span: ast.get_span(path_index).expect("Invalid super path span"),
                    });
                };
                self.resolve_path_inner(ast, ast.get_children(path_index)[0], parent_scope_id)?
            }
            NodeKind::PathSelect => {
                let (where_left_located, left) =
                    self.resolve_path_inner(ast, ast.get_children(path_index)[0], scope_id)?;
                let selected = self.hir.intern_str(
                    &ast.source_content(ast.get_children(path_index)[1], &self.hir.source_map)
                        .expect("Invalid path select content"),
                );

                let left_scope_id = left.scope_id.expect("Invalid scope ID");
                self.ensure_scope_resolved(left_scope_id, span)?;

                let left_scope_id = left.scope_id.expect("Seems you are using a path select without a valid scope initially, some type resolution should be completed later");
                let Some(item) = self.ctx.scope_manager.lookup(selected, left_scope_id) else {
                    return Err(ScanError::UnresolvedIdentifier {
                        message: format!("Unresolved identifier: {}", selected),
                        span: ast.get_span(path_index).expect("Invalid path select span"),
                    });
                };
                (left_scope_id, item)
            }
            _ => {
                return Err(ScanError::InvalidNodeType {
                    message: "Expected path node".into(),
                    span: ast.get_span(path_index).unwrap_or_default(),
                });
            }
        };
        Ok(result)
    }

    /// Ensures that all imports in the given scope are resolved, or recursively resolves them if not.
    fn ensure_scope_resolved(
        &mut self,
        scope_id: usize,
        span_of_the_path_resolving: rustc_span::Span,
    ) -> Result<(), ScanError> {
        if self.resolving.contains_key(&scope_id) {
            return Err(ScanError::CyclicImport {
                message: "Cyclic import detected".into(),
                span: span_of_the_path_resolving,
            });
        }
        Ok(
            if let Some(todo) = self.unresolved_imports.get(&scope_id).cloned() {
                self.resolve_scope_imports(scope_id, &todo)?;
            },
        )
    }

    /// Returns the parent scope ID for the given scope.
    fn parent_scope(&self, scope_id: ScopeId) -> Option<ScopeId> {
        self.ctx.scope_manager.scope_parent(scope_id)
    }

    /// Returns the next scope and its unresolved imports to be resolved.
    fn next(&self) -> Option<(ScopeId, Vec<PendingImport<'vfs>>)> {
        self.unresolved_imports
            .iter()
            .next()
            .map(|(k, v)| (*k, v.clone()))
    }
}
