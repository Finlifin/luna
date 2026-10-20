//! Source ownership lookup shared by explicit driver bindings.
/// Walk module ownership rather than matching leaf names across a package.
/// This also handles directory modules reconstructed by HIR lowering.
pub(super) fn resolve_module_path(
    hir: &hir::Package<'_>,
    package: u32,
    path: &[&str],
) -> Option<hir::OwnerId> {
    let mut nested = std::collections::HashSet::new();
    for (_, info) in hir.owners() {
        let children = match &info.node.expect_item().kind {
            hir::ItemKind::Mod(item) => &item.items,
            hir::ItemKind::Struct(item) => &item.nested_items,
            hir::ItemKind::Enum(item) => &item.nested_items,
            hir::ItemKind::Trait(item) => &item.items,
            hir::ItemKind::Impl(item) => &item.items,
            _ => continue,
        };
        nested.extend(children.iter().copied());
    }
    let roots = hir
        .owners()
        .filter_map(|(owner, info)| {
            (matches!(info.node.expect_item().kind, hir::ItemKind::Mod(_))
                && !nested.contains(&owner)
                && hir
                    .owner_definition(owner)
                    .is_some_and(|id| id.pkg == package))
            .then_some(owner)
        })
        .collect::<Vec<_>>();
    let &[mut owner] = roots.as_slice() else {
        return None;
    };
    for name in path {
        let hir::ItemKind::Mod(module) = &hir.item(owner)?.kind else {
            return None;
        };
        let mut matches = module.items.iter().copied().filter(|child| {
            hir.owner_definition(*child)
                .is_some_and(|id| id.pkg == package)
                && hir
                    .item(*child)
                    .is_some_and(|item| item.ident.name.as_str() == *name)
        });
        owner = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
    }
    Some(owner)
}
