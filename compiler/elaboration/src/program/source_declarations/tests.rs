use super::*;
use hir::{HirArena, Ident, Item, OwnerInfo, OwnerNode, OwnerNodes, Symbol};
use rustc_span::DUMMY_SP;

fn add<'h>(
    package: &mut hir::Package<'h>,
    arena: &'h HirArena,
    name: &str,
    import: bool,
) -> OwnerId {
    let owner = package.alloc_owner_id();
    let kind = if import {
        ItemKind::Use(hir::item::UsePath {
            path: hir::Path {
                anchor: hir::common::PathAnchor::Local,
                segments: &[],
                span: DUMMY_SP,
                res: None,
            },
            kind: hir::item::UseKind::Simple,
            span: DUMMY_SP,
        })
    } else {
        ItemKind::FnDeclaration(hir::FnSig {
            params: &[],
            return_ty: None,
            return_bind: None,
            return_binding: None,
            modifiers: Default::default(),
            clause_params: &[],
            clause_constraints: &[],
            span: DUMMY_SP,
        })
    };
    let item = arena.alloc_item(Item {
        owner_id: owner,
        ident: Ident::new(Symbol::intern(name), DUMMY_SP),
        span: DUMMY_SP,
        kind,
    });
    package.insert_owner(
        owner,
        OwnerInfo {
            node: OwnerNode::Item(item),
            nodes: OwnerNodes::new(),
        },
    );
    owner
}

#[test]
fn imports_do_not_shadow_resolver_identities_and_package_ids_remain_distinct() {
    let arena = HirArena::new();
    let mut package = hir::Package::new();
    let import = add(&mut package, &arena, "import", true);
    let first = add(&mut package, &arena, "first", false);
    let second = add(&mut package, &arena, "second", false);
    package.bind_definition(
        hir::common::DefId {
            pkg: 0,
            index: import.def_id.raw(),
        },
        first,
    );
    package.bind_definition(
        hir::common::DefId {
            pkg: 1,
            index: import.def_id.raw(),
        },
        second,
    );
    let identities = BTreeMap::from([(0, 77), (1, 88)]);
    let index = SourceDeclarations::new(&package, &identities, &mut 100).unwrap();
    let a = DeclarationId {
        package: 77,
        local: 0,
    };
    let b = DeclarationId {
        package: 88,
        local: 0,
    };
    assert_eq!(index.owner(a), Some(first));
    assert_eq!(index.owner(b), Some(second));
    assert_eq!(index.item(a).unwrap().ident.name.as_str(), "first");
    assert_eq!(index.item(b).unwrap().ident.name.as_str(), "second");
    assert_eq!(
        index.owner(DeclarationId {
            package: 0,
            local: 0
        }),
        None
    );
    assert_eq!(
        index.owner(DeclarationId {
            package: 77,
            local: 1
        }),
        None
    );
    assert_eq!(index.owners.len(), 2);
    let dump = index.to_string();
    assert!(dump.starts_with("(source-declarations (declaration "));
    assert!(dump.contains("(owner 1)"));
    assert!(dump.contains("(owner 2)"));
    assert_eq!(dump, index.to_string());
}

#[test]
fn sparse_views_are_charged_before_construction_and_failed_attempts_can_retry() {
    let arena = HirArena::new();
    let mut package = hir::Package::new();
    for _ in 0..1024 {
        package.alloc_owner_id();
    }
    let owner = add(&mut package, &arena, "last", false);
    let identities = BTreeMap::new();
    let mut fuel = 1024;
    assert!(SourceDeclarations::new(&package, &identities, &mut fuel).is_err());
    let mut fuel = 1025;
    let index = SourceDeclarations::new(&package, &identities, &mut fuel).unwrap();
    assert_eq!(fuel, 0);
    let identity = DeclarationId {
        package: 0,
        local: owner.def_id.raw(),
    };
    // Queries reuse the exact borrowed view; no reconstruction or renewed
    // construction allowance is needed for misses or repeated hits.
    for _ in 0..1024 {
        assert_eq!(index.owner(identity), Some(owner));
        assert!(
            index
                .owner(DeclarationId {
                    package: 9,
                    ..identity
                })
                .is_none()
        );
    }
}

#[test]
fn duplicate_semantic_identities_are_rejected_instead_of_selecting_first_owner() {
    let arena = HirArena::new();
    let mut package = hir::Package::new();
    let first = add(&mut package, &arena, "first", false);
    let second = add(&mut package, &arena, "second", false);
    package.bind_definition(hir::common::DefId { pkg: 0, index: 9 }, first);
    package.bind_definition(hir::common::DefId { pkg: 1, index: 9 }, second);
    let aliases = BTreeMap::from([(0, 42), (1, 42)]);
    let failure = SourceDeclarations::new(&package, &aliases, &mut 100)
        .err()
        .unwrap();
    assert!(
        failure
            .message
            .contains("duplicate source declaration identity")
    );
    let identities = BTreeMap::from([(0, 42), (1, 43)]);
    let repaired = SourceDeclarations::new(&package, &identities, &mut 100).unwrap();
    assert_eq!(
        repaired.owner(DeclarationId {
            package: 42,
            local: 9
        }),
        Some(first)
    );
    assert_eq!(
        repaired.owner(DeclarationId {
            package: 43,
            local: 9
        }),
        Some(second)
    );
}

#[test]
fn equal_numeric_ids_in_successor_views_do_not_reuse_old_source_items() {
    let arena = HirArena::new();
    let mut before = hir::Package::new();
    let owner = add(&mut before, &arena, "before", false);
    let old = SourceDeclarations::new(&before, &BTreeMap::new(), &mut 100).unwrap();
    let mut after = hir::Package::new();
    assert_eq!(add(&mut after, &arena, "after", false), owner);
    let new = SourceDeclarations::new(&after, &BTreeMap::new(), &mut 100).unwrap();
    let identity = DeclarationId {
        package: 0,
        local: 0,
    };
    assert_eq!(old.item(identity).unwrap().ident.name.as_str(), "before");
    assert_eq!(new.item(identity).unwrap().ident.name.as_str(), "after");
}
