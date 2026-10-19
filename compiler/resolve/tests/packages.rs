use resolve::{BindingKind, DependencyEdge, build_package_set};
use rustc_span::{FileName, SourceMap, source_map::FilePathMapping};
use symbol::Symbol;
fn package(map: &SourceMap, name: &str, source: &str) -> vfs::Vfs {
    let mut vfs = vfs::Vfs::new(name, format!("/{name}").into());
    let file = map.new_source_file(FileName::Custom(format!("{name}/lib.fl")), source.into());
    let root = vfs.add_file("lib.fl".into(), file);
    vfs.selected_root = Some(root);
    vfs
}
fn edge(requester: u32, alias: &str, provider: u32) -> DependencyEdge {
    DependencyEdge {
        requester,
        alias: alias.into(),
        provider,
    }
}

#[test]
fn deferred_member_cache_shares_results_without_leaking_package_access() {
    let map = SourceMap::new(FilePathMapping::empty());
    let diagnostics = diagnostic::DiagnosticContext::new(&map);
    let mut app = package(&map, "app", "fn entry() {}");
    let mut lib = package(
        &map,
        "library",
        "pub struct Holder {pub typealias Public=bool;typealias Hidden=bool;}",
    );
    let result = build_package_set(
        &map,
        &diagnostics,
        &mut [(7, &mut app), (9, &mut lib)],
        &[edge(7, "library", 9)],
    )
    .unwrap();
    assert!(result.tree.errors.is_empty());
    let mut resolver = resolve::resolver::Resolver::new(&result.tree);
    let public = Symbol::intern("Public");
    let hidden = Symbol::intern("Hidden");
    let app_scope = result.package_scopes[&7];
    let lib_scope = result.package_scopes[&9];
    let visible = resolver.accessible_type_members(public, app_scope);
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].0.pkg, 9);
    for _ in 0..128 {
        assert!(std::sync::Arc::ptr_eq(
            &visible,
            &resolver.accessible_type_members(public, app_scope)
        ));
    }
    // Both fill orders must preserve the requester's access, including a cached
    // empty result followed by a successful lookup in the defining package.
    for (first, second) in [(app_scope, lib_scope), (lib_scope, app_scope)] {
        let mut resolver = resolve::resolver::Resolver::new(&result.tree);
        let a = resolver.accessible_type_members(hidden, first);
        let b = resolver.accessible_type_members(hidden, second);
        assert_eq!(a.len(), usize::from(first == lib_scope));
        assert_eq!(b.len(), usize::from(second == lib_scope));
        assert!(std::sync::Arc::ptr_eq(
            &a,
            &resolver.accessible_type_members(hidden, first)
        ));
        assert!(std::sync::Arc::ptr_eq(
            &b,
            &resolver.accessible_type_members(hidden, second)
        ));
    }
    assert_eq!(visible.len(), 1, "later lookups changed a shared snapshot");
}

#[test]
fn deferred_member_cache_keeps_private_module_permissions_separate() {
    let map = SourceMap::new(FilePathMapping::empty());
    let diagnostics = diagnostic::DiagnosticContext::new(&map);
    let mut lib = package(
        &map,
        "library",
        "pub mod inside {pub struct Holder {private typealias Hidden=bool;}}pub mod outside {}",
    );
    let result = build_package_set(&map, &diagnostics, &mut [(9, &mut lib)], &[]).unwrap();
    assert!(result.tree.errors.is_empty(), "{:?}", result.tree.errors);
    let module = |name| {
        result
            .tree
            .scope_tree
            .iter()
            .find(|scope| scope.name == Some(Symbol::intern(name)))
            .unwrap()
            .id
    };
    let inside = module("inside");
    let outside = module("outside");
    let name = Symbol::intern("Hidden");
    for scopes in [[inside, outside], [outside, inside]] {
        let mut resolver = resolve::resolver::Resolver::new(&result.tree);
        for scope in scopes.into_iter().chain(scopes) {
            let candidates = resolver.accessible_type_members(name, scope);
            assert_eq!(candidates.len(), usize::from(scope == inside));
        }
    }
}
#[test]
fn explicit_dependency_aliases_preserve_provider_identity_and_file_namespaces() {
    let map = SourceMap::new(FilePathMapping::empty());
    let diagnostics = diagnostic::DiagnosticContext::new(&map);
    let mut app = package(&map, "app", "use dependency.value; fn entry() {value()} ");
    let mut lib = package(&map, "library", "pub fn value() {} ");
    let result = build_package_set(
        &map,
        &diagnostics,
        &mut [(7, &mut app), (9, &mut lib)],
        &[edge(7, "dependency", 9)],
    )
    .unwrap();
    assert!(result.tree.errors.is_empty(), "{:?}", result.tree.errors);
    assert!(!diagnostics.has_errors());
    let app_scope = result
        .tree
        .scope_tree
        .get(result.package_scopes[&7])
        .unwrap();
    let lib_scope = result
        .tree
        .scope_tree
        .get(result.package_scopes[&9])
        .unwrap();
    let imported = app_scope.items.get_local(Symbol::intern("value")).unwrap();
    let original = lib_scope.items.get_local(Symbol::intern("value")).unwrap();
    assert_eq!(imported.kind, original.kind);
    assert!(matches!(imported.kind,BindingKind::Definition(id,_) if id.pkg == 9));
    assert_eq!(result.file_offsets[&7], 0);
    assert_eq!(result.file_offsets[&9], 1);
    assert_eq!(result.tree.file_scopes.len(), 2);
    let dump = result.tree.dump_scope_tree();
    assert!(dump.starts_with("(scope-forest"));
    assert!(dump.contains("\"app\"") && dump.contains("\"library\""));
    assert_eq!(dump, result.tree.dump_scope_tree());
    std::fs::write("/tmp/luna-package-scopes.lisp", &dump).unwrap();
}
#[test]
fn transitive_dependencies_are_not_implicitly_visible() {
    let map = SourceMap::new(FilePathMapping::empty());
    let diagnostics = diagnostic::DiagnosticContext::new(&map);
    let mut app = package(&map, "app", "use hidden.value;");
    let mut mid = package(&map, "middle", "use hidden.value;");
    let mut lib = package(&map, "library", "pub fn value() {} ");
    let result = build_package_set(
        &map,
        &diagnostics,
        &mut [(1, &mut app), (2, &mut mid), (3, &mut lib)],
        &[edge(1, "middle", 2), edge(2, "hidden", 3)],
    )
    .unwrap();
    assert!(!result.tree.errors.is_empty());
    let middle = result
        .tree
        .scope_tree
        .get(result.package_scopes[&2])
        .unwrap();
    assert!(middle.items.get_local(Symbol::intern("value")).is_some());
}
#[test]
fn package_private_declarations_do_not_cross_dependency_boundaries() {
    for visibility in ["", "private "] {
        let map = SourceMap::new(FilePathMapping::empty());
        let diagnostics = diagnostic::DiagnosticContext::new(&map);
        let mut app = package(&map, "app", "use dep.hidden;");
        let mut lib = package(&map, "library", &format!("{visibility}fn hidden() {{}}"));
        let result = build_package_set(
            &map,
            &diagnostics,
            &mut [(1, &mut app), (2, &mut lib)],
            &[edge(1, "dep", 2)],
        )
        .unwrap();
        assert!(!result.tree.errors.is_empty(), "{visibility}");
    }
}
#[test]
fn dependency_reexports_follow_original_public_declaration_identity() {
    let map = SourceMap::new(FilePathMapping::empty());
    let diagnostics = diagnostic::DiagnosticContext::new(&map);
    let mut app = package(&map, "app", "use facade.value;");
    let mut facade = package(&map, "facade", "pub use backend.value;");
    let mut lib = package(&map, "library", "pub fn value() {} ");
    let result = build_package_set(
        &map,
        &diagnostics,
        &mut [(1, &mut app), (2, &mut facade), (3, &mut lib)],
        &[edge(1, "facade", 2), edge(2, "backend", 3)],
    )
    .unwrap();
    assert!(result.tree.errors.is_empty(), "{:?}", result.tree.errors);
    let root = result
        .tree
        .scope_tree
        .get(result.package_scopes[&1])
        .unwrap();
    assert!(
        matches!(root.items.get_local(Symbol::intern("value")).unwrap().kind,BindingKind::Definition(id,_) if id.pkg==3)
    );
}
#[test]
fn malformed_dependency_inputs_are_rejected() {
    let map = SourceMap::new(FilePathMapping::empty());
    let diagnostics = diagnostic::DiagnosticContext::new(&map);
    let mut app = package(&map, "app", "");
    let mut lib = package(&map, "library", "");
    assert!(
        build_package_set(&map, &diagnostics, &mut [(1, &mut app), (1, &mut lib)], &[]).is_err()
    );
    for edges in [
        vec![edge(1, "dep", 99)],
        vec![edge(1, "dep", 1)],
        vec![edge(1, "dep", 2), edge(1, "dep", 2)],
        vec![edge(1, "", 2)],
    ] {
        assert!(
            build_package_set(
                &map,
                &diagnostics,
                &mut [(1, &mut app), (2, &mut lib)],
                &edges
            )
            .is_err()
        );
    }
}

#[test]
fn identical_generic_sources_keep_ast_bindings_in_distinct_files() {
    let map = SourceMap::new(FilePathMapping::empty());
    let diagnostics = diagnostic::DiagnosticContext::new(&map);
    let mut first = package(&map, "first", "fn identity(value:T)->T where T {value}");
    let mut second = package(&map, "second", "fn identity(value:T)->T where T {value}");
    let result = build_package_set(
        &map,
        &diagnostics,
        &mut [(10, &mut first), (20, &mut second)],
        &[],
    )
    .unwrap();
    assert!(result.tree.errors.is_empty());
    let mut references: Vec<_> = result
        .tree
        .scope_tree
        .iter()
        .flat_map(|s| s.items.clauses())
        .filter_map(|c| match c.binding.kind {
            BindingKind::ClauseParam(id, node) => Some((id.pkg, node.file.raw(), node.node)),
            _ => None,
        })
        .collect();
    references.sort();
    assert_eq!(references.len(), 2);
    assert_eq!(references[0].0, 10);
    assert_eq!(references[1].0, 20);
    assert_eq!(references[0].1, 0);
    assert_eq!(references[1].1, 1);
    assert_eq!(references[0].2, references[1].2);
}

#[test]
fn scope_dump_escapes_names_and_contains_clause_bindings() {
    let map = SourceMap::new(FilePathMapping::empty());
    let diagnostics = diagnostic::DiagnosticContext::new(&map);
    let mut input = package(
        &map,
        "quoted\"\\\n\t\0λ",
        "fn identity(value:T)->T where T {value}",
    );
    let result = build_package_set(&map, &diagnostics, &mut [(1, &mut input)], &[]).unwrap();
    assert!(result.tree.errors.is_empty());
    let dump = result.tree.dump_scope_tree();
    assert!(dump.contains("quoted\\\"\\\\\\n\\t\\u0000λ"));
    assert!(dump.contains("(clause (name \"T\") (parameter (definition (package 1)"));
    std::fs::write("/tmp/luna-package-scopes-escaped.lisp", dump).unwrap();
}
