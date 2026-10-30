use resolve::{Binding, BindingKind, Resolver, Rib, RibKind, ScopeId, build_package_set};
use resolve::{binding::Visibility, ids::AstNodeRef};
use rustc_span::{DUMMY_SP, FileName, SourceMap, source_map::FilePathMapping};
use symbol::Symbol;

fn with_source(source: &str, test: impl FnOnce(&mut Resolver<'_>, ScopeId)) {
    let map = SourceMap::new(FilePathMapping::empty());
    let diagnostics = diagnostic::DiagnosticContext::new(&map);
    let mut package = vfs::Vfs::new("app", "/app".into());
    let file = map.new_source_file(FileName::Custom("app/lib.fl".into()), source.into());
    package.selected_root = Some(package.add_file("lib.fl".into(), file));
    let result = build_package_set(&map, &diagnostics, &mut [(1, &mut package)], &[]).unwrap();
    assert!(!diagnostics.has_errors(), "{source}");
    assert!(result.tree.errors.is_empty(), "{:?}", result.tree.errors);
    let client = result
        .tree
        .scope_tree
        .iter()
        .find(|s| s.name == Some(Symbol::intern("client")))
        .unwrap()
        .id;
    test(&mut Resolver::new(&result.tree), client);
}
fn local(node: u32) -> Binding {
    Binding {
        kind: BindingKind::Local(site(node)),
        vis: Visibility::Private,
        entry: None,
    }
}
fn site(node: u32) -> AstNodeRef {
    AstNodeRef::new(vfs::FileId::from_raw(0), node)
}

#[test]
fn traits_require_lexical_visibility_and_aliases_keep_declaration_identity() {
    with_source(
        "trait Base {} mod provider {pub trait Hidden {}} mod unrelated {pub trait Other {}} mod client {use provider.Hidden as Imported; use provider.Hidden as Again;}",
        |resolver, client| {
            let plan = resolver.visible_trait_sources(client, DUMMY_SP);
            assert!(plan.pending_exports.is_empty());
            assert_eq!(
                plan.names
                    .iter()
                    .map(|p| p.name.as_str())
                    .collect::<Vec<_>>(),
                ["Again", "Base", "Imported"]
            );
            assert_eq!(
                plan.names[0].fallback.as_ref().unwrap().kind,
                plan.names[2].fallback.as_ref().unwrap().kind
            );
            assert!(plan.to_string().starts_with("(trait-source-plan (names"));
        },
    );
}

#[test]
fn ordinary_declarations_and_live_locals_shadow_outer_trait_names() {
    with_source(
        "trait Protocol {} mod client {struct Protocol {}}",
        |resolver, client| {
            assert!(
                resolver
                    .visible_trait_sources(client, DUMMY_SP)
                    .names
                    .is_empty()
            );
        },
    );
    with_source("trait Protocol {} mod client {}", |resolver, client| {
        assert_eq!(
            resolver.visible_trait_sources(client, DUMMY_SP).names.len(),
            1
        );
        let mut rib = Rib::new(RibKind::Block, ScopeId::INVALID);
        rib.define(Symbol::intern("Protocol"), local(3));
        resolver.ribs_mut().push(rib);
        assert!(
            resolver
                .visible_trait_sources(client, DUMMY_SP)
                .names
                .is_empty()
        );
        resolver.ribs_mut().pop();
        assert_eq!(
            resolver.visible_trait_sources(client, DUMMY_SP).names.len(),
            1
        );
    });
}

#[test]
fn glob_reexports_preserve_identity_and_do_not_duplicate_names() {
    with_source(
        "mod provider {pub pure comptime trait Protocol {}} mod bridge {pub use provider.*;} mod client {use provider.*; use bridge.*;}",
        |resolver, client| {
            let plan = resolver.visible_trait_sources(client, DUMMY_SP);
            assert_eq!(plan.names.len(), 1);
            assert_eq!(plan.names[0].name.as_str(), "Protocol");
            assert!(plan.names[0].fallback.is_ok());
        },
    );
}

#[test]
fn ambiguous_globs_are_preserved_until_an_explicit_binding_shadows_them() {
    with_source(
        "mod a {pub trait Protocol {}} mod b {pub trait Protocol {}} mod client {use a.*;use b.*;}",
        |resolver, client| {
            let plan = resolver.visible_trait_sources(client, DUMMY_SP);
            assert_eq!(plan.names.len(), 1);
            assert!(
                plan.names[0]
                    .fallback
                    .as_ref()
                    .unwrap_err()
                    .message()
                    .contains("ambiguous glob")
            );
            let mut rib = Rib::new(RibKind::Block, ScopeId::INVALID);
            rib.define(Symbol::intern("Protocol"), local(4));
            resolver.ribs_mut().push(rib);
            assert!(
                resolver
                    .visible_trait_sources(client, DUMMY_SP)
                    .names
                    .is_empty()
            );
        },
    );
}

#[test]
fn pending_generated_names_prevent_treating_empty_or_fallback_sets_as_complete() {
    for prefix in ["", "trait Protocol {}"] {
        with_source(&format!("{prefix} mod client {{}}"), |resolver, client| {
            let mut rib = Rib::new(RibKind::MatchBody, ScopeId::INVALID);
            rib.pending_outputs = Some(site(5));
            resolver.ribs_mut().push(rib);
            let plan = resolver.visible_trait_sources(client, DUMMY_SP);
            assert_eq!(plan.pending_exports, [site(5)]);
            for name in &plan.names {
                assert_eq!(name.pending_outputs, [site(5)]);
            }
            assert!(
                plan.to_string()
                    .contains("(pending-exports (site (file 0) (node 5)))")
            );
            resolver.ribs_mut().pop();
            assert!(
                resolver
                    .visible_trait_sources(client, DUMMY_SP)
                    .pending_exports
                    .is_empty()
            );
        });
    }
}

#[test]
fn a_rib_can_introduce_a_new_alias_without_invalidating_the_static_name_index() {
    with_source("trait Protocol {} mod client {}", |resolver, client| {
        let original = resolver
            .resolve_name(Symbol::intern("Protocol"), client, DUMMY_SP)
            .unwrap();
        let mut rib = Rib::new(RibKind::Block, ScopeId::INVALID);
        rib.define(
            Symbol::intern("LocalAlias"),
            Binding {
                kind: original.kind,
                vis: Visibility::Package,
                entry: None,
            },
        );
        resolver.ribs_mut().push(rib);
        let plan = resolver.visible_trait_sources(client, DUMMY_SP);
        assert_eq!(
            plan.names
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            ["LocalAlias", "Protocol"]
        );
        assert_eq!(plan.names[0].fallback.as_ref().unwrap().kind, original.kind);
    });
}

#[test]
fn imported_entry_cannot_widen_the_original_trait_permission() {
    use resolve::{
        DefId, ModuleTree,
        scope::{Scope, ScopeKind, ScopeTree},
    };
    let mut scopes = ScopeTree::new();
    for (number, kind, parent) in [
        (0, ScopeKind::Package, None),
        (1, ScopeKind::Module, Some(0)),
        (2, ScopeKind::TraitBody, Some(1)),
        (3, ScopeKind::Module, Some(0)),
    ] {
        scopes.add_scope(Scope::new(
            ScopeId::new(number),
            kind,
            parent.map(ScopeId::new),
            None,
            DefId::new(0, number),
            false,
        ));
    }
    let original = Binding {
        kind: BindingKind::Definition(DefId::new(0, 2), ScopeId::new(1)),
        vis: Visibility::Private,
        entry: None,
    };
    scopes
        .get_mut(ScopeId::new(3))
        .unwrap()
        .items
        .define(
            Symbol::intern("Protocol"),
            original.imported(ScopeId::new(3), Visibility::Public),
        )
        .unwrap();
    let tree = ModuleTree {
        scope_tree: scopes,
        def_names: Default::default(),
        def_count: 4,
        impls: vec![],
        errors: vec![],
        file_scopes: Default::default(),
    };
    let resolver = Resolver::new(&tree);
    let plan = resolver.visible_trait_sources(ScopeId::new(3), DUMMY_SP);
    assert_eq!(plan.names.len(), 1);
    assert!(
        plan.names[0]
            .fallback
            .as_ref()
            .unwrap_err()
            .message()
            .contains("not accessible")
    );
}

#[test]
fn clause_parameter_shadows_a_trait_without_erasing_other_visible_sources() {
    with_source(
        "trait Protocol {} trait Other {} mod client {}",
        |resolver, client| {
            let mut rib = Rib::new(RibKind::ClauseParam, ScopeId::INVALID);
            rib.define(
                Symbol::intern("Protocol"),
                Binding {
                    kind: BindingKind::ClauseParam(resolve::DefId::new(1, 900), site(6)),
                    vis: Visibility::Private,
                    entry: None,
                },
            );
            resolver.ribs_mut().push(rib);
            let plan = resolver.visible_trait_sources(client, DUMMY_SP);
            assert_eq!(
                plan.names
                    .iter()
                    .map(|p| p.name.as_str())
                    .collect::<Vec<_>>(),
                ["Other"]
            );
        },
    );
}
