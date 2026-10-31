use resolve::binding::{Binding, BindingKind, Visibility};
use resolve::ids::{DefId, ScopeId};
use resolve::scope::{Scope, ScopeKind, ScopeTree};

fn tree() -> ScopeTree {
    let mut tree = ScopeTree::new();
    for (id, kind, parent) in [
        (0, ScopeKind::Root, None),
        (1, ScopeKind::Package, Some(0)),
        (2, ScopeKind::Module, Some(1)),
        (3, ScopeKind::Module, Some(2)),
        (4, ScopeKind::Module, Some(1)),
        (5, ScopeKind::Package, Some(0)),
        (6, ScopeKind::Module, Some(5)),
    ] {
        tree.add_scope(Scope::new(
            ScopeId::new(id),
            kind,
            parent.map(ScopeId::new),
            None,
            DefId::new(0, id),
            false,
        ));
    }
    tree
}
fn declaration(vis: Visibility) -> Binding {
    Binding {
        kind: BindingKind::Definition(DefId::new(0, 99), ScopeId::new(2)),
        vis,
        entry: None,
    }
}

#[test]
fn original_package_and_private_permissions_survive_rebinding() {
    let tree = tree();
    let private = declaration(Visibility::Private).imported(ScopeId::new(4), Visibility::Public);
    assert!(private.accessible(&tree, ScopeId::new(3)));
    assert!(!private.accessible(&tree, ScopeId::new(4)));
    assert!(!private.accessible(&tree, ScopeId::new(6)));
    let package = declaration(Visibility::Package).imported(ScopeId::new(4), Visibility::Public);
    assert!(package.accessible(&tree, ScopeId::new(4)));
    assert!(!package.accessible(&tree, ScopeId::new(6)));
}

#[test]
fn entry_permissions_restrict_public_declarations_independently() {
    let tree = tree();
    let public = declaration(Visibility::Public);
    assert!(public.accessible(&tree, ScopeId::new(6)));
    let private = public.imported(ScopeId::new(2), Visibility::Private);
    assert!(private.accessible(&tree, ScopeId::new(3)));
    assert!(!private.accessible(&tree, ScopeId::new(4)));
    let package = public.imported(ScopeId::new(2), Visibility::Package);
    assert!(package.accessible(&tree, ScopeId::new(4)));
    assert!(!package.accessible(&tree, ScopeId::new(6)));
}

#[test]
fn entry_union_is_order_independent_for_the_same_identity() {
    let tree = tree();
    for (first, second) in [
        (Visibility::Private, Visibility::Public),
        (Visibility::Public, Visibility::Private),
    ] {
        let origin = declaration(Visibility::Public);
        let mut merged = origin.imported(ScopeId::new(2), first);
        merged.merge_entry(&origin.imported(ScopeId::new(2), second));
        assert!(merged.accessible(&tree, ScopeId::new(6)));
        assert_eq!(merged.kind, origin.kind);
    }
}

#[test]
fn private_adt_members_and_import_entries_use_the_lexical_module() {
    let mut tree = tree();
    let adt = ScopeId::new(7);
    tree.add_scope(Scope::new(
        adt,
        ScopeKind::AdtBody,
        Some(ScopeId::new(2)),
        None,
        DefId::new(0, 100),
        false,
    ));
    let member = Binding {
        kind: BindingKind::Definition(DefId::new(0, 101), adt),
        vis: Visibility::Private,
        entry: None,
    };
    let entry = declaration(Visibility::Public).imported(adt, Visibility::Private);
    for binding in [member, entry] {
        for scope in [2, 3, 7] {
            assert!(
                binding.accessible(&tree, ScopeId::new(scope)),
                "scope {scope}"
            );
        }
        for scope in [1, 4, 5, 6] {
            assert!(
                !binding.accessible(&tree, ScopeId::new(scope)),
                "scope {scope}"
            );
        }
    }
}
