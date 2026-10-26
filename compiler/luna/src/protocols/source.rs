//! Source ownership lookup shared by explicit driver bindings.
use std::cell::OnceCell;
use std::collections::{HashMap, HashSet};

type Owner = hir::OwnerId;
struct Index<'a> {
    roots: HashMap<u32, Option<Owner>>,
    children: HashMap<(u32, Owner), HashMap<&'a str, Option<Owner>>>,
}

/// One immutable HIR snapshot owns one lazy lookup index. No names or owner IDs
/// survive into another compilation, and an unused selector set pays no scan.
pub(super) struct ModulePaths<'a, 'arena> {
    hir: &'a hir::Package<'arena>,
    index: OnceCell<Index<'a>>,
}
impl<'a, 'arena> ModulePaths<'a, 'arena> {
    pub fn new(hir: &'a hir::Package<'arena>) -> Self {
        Self {
            hir,
            index: OnceCell::new(),
        }
    }

    fn build(&self) -> Index<'a> {
        let mut nested = HashSet::new();
        let mut children = HashMap::new();
        for (owner, info) in self.hir.owners() {
            let items = match &info.node.expect_item().kind {
                hir::ItemKind::Mod(item) => &item.items,
                hir::ItemKind::Struct(item) => &item.nested_items,
                hir::ItemKind::Enum(item) => &item.nested_items,
                hir::ItemKind::Trait(item) => &item.items,
                hir::ItemKind::Impl(item) => &item.items,
                _ => continue,
            };
            nested.extend(items.iter().copied());
            // Selectors traverse modules only, even though other owners contain
            // nested declarations that must not be mistaken for package roots.
            if !matches!(info.node.expect_item().kind, hir::ItemKind::Mod(_)) {
                continue;
            }
            for child in items {
                let (Some(definition), Some(item)) =
                    (self.hir.owner_definition(*child), self.hir.item(*child))
                else {
                    continue;
                };
                let names: &mut HashMap<&str, Option<Owner>> =
                    children.entry((definition.pkg, owner)).or_default();
                names
                    .entry(item.ident.name.as_str())
                    .and_modify(|entry| *entry = None)
                    .or_insert(Some(*child));
            }
        }
        let mut roots = HashMap::new();
        for (owner, info) in self.hir.owners() {
            if matches!(info.node.expect_item().kind, hir::ItemKind::Mod(_))
                && !nested.contains(&owner)
            {
                if let Some(definition) = self.hir.owner_definition(owner) {
                    roots
                        .entry(definition.pkg)
                        .and_modify(|entry| *entry = None)
                        .or_insert(Some(owner));
                }
            }
        }
        Index { roots, children }
    }

    pub fn resolve(&self, package: u32, path: &[&str]) -> Option<Owner> {
        let index = self.index.get_or_init(|| self.build());
        let mut owner = (*index.roots.get(&package)?)?;
        for name in path {
            owner = (*index.children.get(&(package, owner))?.get(name)?)?;
        }
        Some(owner)
    }
}

/// Walk module ownership, never leaf names across a package. Directory modules
/// use the same index as source modules reconstructed by HIR lowering.
pub(super) fn resolve_module_path(
    hir: &hir::Package<'_>,
    package: u32,
    path: &[&str],
) -> Option<Owner> {
    ModulePaths::new(hir).resolve(package, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hir::common::DefId;
    use hir::*;
    use rustc_span::DUMMY_SP;

    fn module<'a>(
        arena: &'a HirArena,
        package: &mut Package<'a>,
        id: u32,
        name: &str,
        items: Vec<OwnerId>,
    ) -> OwnerId {
        let owner = package.alloc_owner_id();
        let item = arena.alloc_item(Item {
            owner_id: owner,
            ident: Ident::new(Symbol::intern(name), DUMMY_SP),
            kind: ItemKind::Mod(ModDef { items }),
            span: DUMMY_SP,
        });
        package.insert_owner(
            owner,
            OwnerInfo {
                node: OwnerNode::Item(item),
                nodes: OwnerNodes::new(),
            },
        );
        package.bind_definition(
            DefId {
                pkg: id,
                index: owner.def_id.raw(),
            },
            owner,
        );
        owner
    }

    #[test]
    fn lazy_index_reuses_snapshot_and_keeps_package_paths_separate() {
        let arena = HirArena::new();
        let mut package = Package::new();
        let leaf = module(&arena, &mut package, 1, "Scalar", vec![]);
        let nested = module(&arena, &mut package, 1, "target", vec![leaf]);
        let root = module(&arena, &mut package, 1, "root", vec![nested]);
        let other = module(&arena, &mut package, 2, "Scalar", vec![]);
        module(&arena, &mut package, 2, "root", vec![other]);
        let paths = ModulePaths::new(&package);
        assert!(paths.index.get().is_none());
        assert_eq!(paths.resolve(1, &[]), Some(root));
        let index = paths.index.get().unwrap() as *const _;
        for _ in 0..100 {
            assert_eq!(paths.resolve(1, &["target", "Scalar"]), Some(leaf));
            assert_eq!(paths.resolve(2, &["Scalar"]), Some(other));
            assert_eq!(paths.resolve(1, &["Scalar"]), None);
            assert_eq!(paths.resolve(2, &["target", "Scalar"]), None);
            assert_eq!(paths.resolve(3, &[]), None);
            assert_eq!(paths.index.get().unwrap() as *const _, index);
        }
    }

    #[test]
    fn ambiguous_children_and_new_root_in_a_later_snapshot_are_rejected() {
        let arena = HirArena::new();
        let mut package = Package::new();
        let children = (0..3)
            .map(|_| module(&arena, &mut package, 1, "same", vec![]))
            .collect();
        let root = module(&arena, &mut package, 1, "root", children);
        {
            let paths = ModulePaths::new(&package);
            assert_eq!(paths.resolve(1, &[]), Some(root));
            assert_eq!(paths.resolve(1, &["same"]), None);
        }
        module(&arena, &mut package, 1, "another", vec![]);
        assert_eq!(ModulePaths::new(&package).resolve(1, &[]), None);
    }
}
