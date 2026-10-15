//! Driver binding for the ordinary value Splice protocol.
pub use elaboration::splice::Protocol;
use intrinsic::{LangItem, LangItemDef, LangItems};
use nir::program::Package;

pub fn protocol<'a>(package: &'a Package, items: &LangItems) -> Result<Protocol<'a>, String> {
    let Some(LangItemDef::UserDef(id)) = items.get(LangItem::Splice) else {
        return Err("Splice language item is not bound to a source trait".into());
    };
    elaboration::splice::protocol(&package.traits, id)
}

/// Resolve an explicitly requested driver selector after ordinary name
/// resolution. This is not a language-level implicit lookup for `Splice`.
pub(crate) fn resolve_source(
    hir: &hir::Package<'_>,
    packages: &[(u32, u64, &str)],
    selector: Option<&str>,
    identities: &elaboration::program::Protocols,
) -> Result<elaboration::program::Protocols, String> {
    let Some(selector) = selector else {
        return Ok(identities.clone());
    };
    let (selected_owner, name) = match selector.split_once("::") {
        Some((package, path)) => {
            let path = path.split("::").collect::<Vec<_>>();
            if package.is_empty() || path.iter().any(|part| part.is_empty()) {
                return Err("Splice protocol selector must be trait, package::trait or package::module::trait".into());
            }
            let providers = packages
                .iter()
                .filter(|(_, _, name)| *name == package)
                .collect::<Vec<_>>();
            let [provider] = providers.as_slice() else {
                return Err(format!(
                    "Splice protocol package `{package}` is missing or ambiguous"
                ));
            };
            let owner =
                super::source::resolve_module_path(hir, provider.0, &path).ok_or_else(|| {
                    format!("Splice protocol `{selector}` is not a declared trait or is ambiguous")
                })?;
            (Some(owner), *path.last().unwrap())
        }
        None if !selector.is_empty() => (None, selector),
        None => return Err("Splice protocol selector must not be empty".into()),
    };
    let mut matches = Vec::new();
    for (owner, info) in hir.owners() {
        let item = info.node.expect_item();
        if !matches!(item.kind, hir::ItemKind::Trait(_)) || item.ident.name.as_str() != name {
            continue;
        }
        let Some(definition) = hir.owner_definition(owner) else {
            continue;
        };
        if selected_owner.is_some_and(|selected| owner != selected) {
            continue;
        }
        let identity = packages
            .iter()
            .find(|(id, _, _)| *id == definition.pkg)
            .ok_or("Splice protocol declaration has no source package identity")?
            .1;
        matches.push(nir::DeclarationId {
            package: identity,
            local: definition.index,
        });
    }
    let [id] = matches.as_slice() else {
        return Err(format!(
            "Splice protocol `{selector}` is {}",
            if matches.is_empty() {
                "not a declared trait"
            } else {
                "ambiguous"
            }
        ));
    };
    if identities.splice.is_some_and(|previous| previous != *id) {
        return Err("Splice source selector conflicts with the driver's bound identity".into());
    }
    Ok(elaboration::program::Protocols {
        splice: Some(*id),
        ..identities.clone()
    })
}
