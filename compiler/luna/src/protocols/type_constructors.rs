//! Explicit driver selectors for source-defined type constructors and operator protocols.
use elaboration::program::Protocols;

#[derive(Clone, Copy, Debug, Default)]
pub struct Selectors<'a> {
    pub copy: Option<&'a str>,
    pub drop: Option<&'a str>,
    pub index: Option<&'a str>,
    pub literal_into: Option<&'a str>,
    pub arithmetic: [Option<&'a str>; 7],
    pub splice: Option<&'a str>,
    pub meta_list: Option<&'a str>,
    pub tuple: Option<&'a str>,
    pub optional: Option<&'a str>,
}

pub(crate) fn resolve_source(
    hir: &hir::Package<'_>,
    packages: &[(u32, u64, &str)],
    selectors: Selectors<'_>,
    identities: &Protocols,
) -> Result<Protocols, String> {
    let mut result = super::splice::resolve_source(hir, packages, selectors.splice, identities)?;
    for (label, selector, target) in elaboration::arithmetic::Operator::ALL
        .into_iter()
        .zip(result.arithmetic.iter_mut())
        .map(|(operator, target)| {
            (
                operator.to_string(),
                selectors.arithmetic[operator as usize],
                target,
            )
        })
        .chain([
            ("Index".into(), selectors.index, &mut result.index),
            (
                "LiteralInto".into(),
                selectors.literal_into,
                &mut result.literal_into,
            ),
            ("Copy".into(), selectors.copy, &mut result.copy),
            ("Drop".into(), selectors.drop, &mut result.drop),
        ])
    {
        let operator = label;
        let Some(selector) = selector else {
            continue;
        };
        let parts = selector.split("::").collect::<Vec<_>>();
        if parts.len() == 1 && matches!(operator.as_str(), "Copy" | "Drop") {
            // Preserve the resource CLI's unique-leaf-name contract, including
            // sysroot packages and nested modules. Qualified selectors below
            // remain the unambiguous build-driver form.
            let candidates = hir
                .owners()
                .filter_map(|(owner, info)| {
                    let item = info.node.expect_item();
                    (item.ident.name.as_str() == selector
                        && matches!(item.kind, hir::ItemKind::Trait(_)))
                    .then_some(owner)
                })
                .collect::<Vec<_>>();
            let owner = match candidates.as_slice() {
                [owner] => *owner,
                [] => return Err(format!("{operator} protocol `{selector}` was not found")),
                _ => return Err(format!("{operator} protocol `{selector}` is ambiguous")),
            };
            let definition = hir
                .owner_definition(owner)
                .ok_or("resource protocol has no source identity")?;
            let provider = packages
                .iter()
                .find(|(id, _, _)| *id == definition.pkg)
                .ok_or("resource protocol source package is unavailable")?;
            let id = nir::DeclarationId {
                package: provider.1,
                local: definition.index,
            };
            if target.is_some_and(|previous| previous != id) {
                return Err(format!(
                    "{operator} selector conflicts with the driver's bound identity"
                ));
            }
            *target = Some(id);
            continue;
        }
        if parts.len() < 2 || parts.iter().any(|part| part.is_empty()) {
            return Err(format!(
                "{operator} selector requires package::trait or package::module::trait"
            ));
        }
        let providers = packages
            .iter()
            .filter(|(_, _, name)| *name == parts[0])
            .collect::<Vec<_>>();
        let [provider] = providers.as_slice() else {
            return Err(format!(
                "{operator} selector package is missing or ambiguous"
            ));
        };
        let owner = super::source::resolve_module_path(hir, provider.0, &parts[1..])
            .ok_or_else(|| format!("{operator} protocol selector is missing or ambiguous"))?;
        if !hir
            .item(owner)
            .is_some_and(|item| matches!(item.kind, hir::ItemKind::Trait(_)))
        {
            return Err(format!("{operator} protocol requires a trait"));
        }
        let definition = hir
            .owner_definition(owner)
            .ok_or("operator protocol has no source identity")?;
        let id = nir::DeclarationId {
            package: provider.1,
            local: definition.index,
        };
        if target.is_some_and(|previous| previous != id) {
            return Err(format!(
                "{operator} selector conflicts with the driver's bound identity"
            ));
        }
        *target = Some(id);
    }
    for (label, selector, target) in [
        ("meta.List", selectors.meta_list, &mut result.meta_list),
        ("Tuple", selectors.tuple, &mut result.tuple),
        ("optional", selectors.optional, &mut result.optional),
    ] {
        let Some(selector) = selector else {
            continue;
        };
        let components = selector.split("::").collect::<Vec<_>>();
        if components.len() < 2 || components.iter().any(|part| part.is_empty()) {
            return Err(format!(
                "{label} selector must be package::definition or package::module::definition"
            ));
        }
        let package_name = components[0];
        let providers = packages
            .iter()
            .filter(|(_, _, name)| *name == package_name)
            .collect::<Vec<_>>();
        let [provider] = providers.as_slice() else {
            return Err(format!(
                "{label} selector package `{package_name}` is missing or ambiguous"
            ));
        };
        let owner = super::source::resolve_module_path(hir, provider.0, &components[1..])
            .ok_or_else(|| format!("{label} selector `{selector}` is missing or ambiguous"))?;
        let item = hir
            .item(owner)
            .ok_or("type constructor owner is unavailable")?;
        if label == "optional" {
            if !matches!(item.kind, hir::ItemKind::Enum(_)) {
                return Err("optional binding requires an enum".into());
            }
            let definition = hir
                .owner_definition(owner)
                .ok_or("optional has no source identity")?;
            let id = nir::DeclarationId {
                package: provider.1,
                local: definition.index,
            };
            if target.is_some_and(|old| old != id) {
                return Err("optional selector conflicts with the driver's bound identity".into());
            }
            *target = Some(id);
            continue;
        }
        let hir::ItemKind::Fn(signature, _) = &item.kind else {
            return Err(format!(
                "{label} selector requires a pure normal-form definition"
            ));
        };
        if !signature.modifiers.is_normal_form_head || !signature.modifiers.is_pure {
            return Err(format!(
                "{label} selector requires a pure normal-form definition"
            ));
        }
        let definition = hir
            .owner_definition(owner)
            .ok_or("type constructor has no source identity")?;
        let id = &nir::DeclarationId {
            package: provider.1,
            local: definition.index,
        };
        if target.is_some_and(|old| old != *id) {
            return Err(format!(
                "{label} selector conflicts with the driver's bound identity"
            ));
        }
        *target = Some(*id);
    }
    if result.tuple.is_some() && result.meta_list.is_none() {
        return Err("Tuple binding requires a meta.List binding".into());
    }
    Ok(result)
}

pub(crate) fn validate(
    package: &nir::program::Package,
    protocols: &Protocols,
) -> Result<(), String> {
    use nir::normal_forms::{Kind, Role};
    if let Some(id) = protocols.optional {
        let definitions = package
            .enums
            .iter()
            .filter(|definition| definition.declaration == id)
            .collect::<Vec<_>>();
        if !matches!(definitions.as_slice(), [definition] if definition.is_optional_representation())
        {
            return Err("optional binding requires one Type parameter, one unit variant and one single-value variant without additional requirements".into());
        }
    }
    for (label, id) in [
        ("meta.List", protocols.meta_list),
        ("Tuple", protocols.tuple),
    ] {
        let Some(id) = id else {
            continue;
        };
        let heads = package
            .normal_forms
            .iter()
            .filter(|head| head.declaration == id)
            .collect::<Vec<_>>();
        let [head] = heads.as_slice() else {
            return Err(format!("{label} binding has no unique checked head"));
        };
        let valid = if label == "meta.List" {
            head.parameters.len() == 1 && head.parameters[0].kind == Kind::Type
        } else {
            head.role == Role::Tuple
                && head.parameters.len() == 1
                && head.parameters[0].kind == Kind::TypeList
                && matches!(&head.parameters[0].ty, nir::Term::NormalForm {head, ..} if Some(*head) == protocols.meta_list)
        };
        if !valid {
            return Err(format!(
                "{label} binding has an invalid constructor signature"
            ));
        }
    }
    Ok(())
}

/// Defaults belong to the explicitly selected sysroot, never to names found in
/// an input program. Package-set clients may supply their own driver identities.
#[derive(Default)]
pub(crate) struct SysrootBindings {
    copy: Option<String>,
    drop: Option<String>,
    index: Option<String>,
    literal_into: Option<String>,
    arithmetic: [Option<String>; 7],
    splice: Option<String>,
    meta_list: Option<String>,
    tuple: Option<String>,
    optional: Option<String>,
}
impl SysrootBindings {
    pub(crate) fn load(root: &std::path::Path) -> Result<Self, String> {
        use std::io::Read;
        let path = root.join("language-items.conf");
        let file = match std::fs::File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(format!("cannot read '{}': {error}", path.display())),
        };
        let mut text = String::new();
        file.take(65537)
            .read_to_string(&mut text)
            .map_err(|error| format!("cannot read '{}': {error}", path.display()))?;
        if text.len() > 65536 {
            return Err(format!(
                "'{}' exceeds the language-item configuration size limit",
                path.display()
            ));
        }
        Self::parse(&text).map_err(|error| format!("{}: {error}", path.display()))
    }
    fn parse(text: &str) -> Result<Self, String> {
        let mut result = Self::default();
        let mut version = false;
        for (index, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap().trim();
            if line.is_empty() {
                continue;
            }
            let fail = |reason: &str| format!("line {}: {reason}", index + 1);
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| fail("expected key = value"))?;
            let key = key.trim();
            let value = value.trim();
            if key == "version" {
                if version {
                    return Err(fail("duplicate version"));
                }
                if value != "1" {
                    return Err(fail("unsupported language-item configuration version"));
                }
                version = true;
                continue;
            }
            let target = match key {
                "index-protocol" => &mut result.index,
                "literal-into-protocol" => &mut result.literal_into,
                "copy-protocol" => &mut result.copy,
                "drop-protocol" => &mut result.drop,
                "add-protocol" => &mut result.arithmetic[0],
                "sub-protocol" => &mut result.arithmetic[1],
                "mul-protocol" => &mut result.arithmetic[2],
                "div-protocol" => &mut result.arithmetic[3],
                "rem-protocol" => &mut result.arithmetic[4],
                "neg-protocol" => &mut result.arithmetic[5],
                "not-protocol" => &mut result.arithmetic[6],
                "splice-protocol" => &mut result.splice,
                "meta-list-type" => &mut result.meta_list,
                "tuple-type" => &mut result.tuple,
                "optional-type" => &mut result.optional,
                _ => return Err(fail("unknown language-item key")),
            };
            if target.is_some() {
                return Err(fail("duplicate language-item key"));
            }
            let parts = value.split("::").collect::<Vec<_>>();
            if parts.len() < 2
                || parts.iter().any(|part| {
                    part.is_empty()
                        || part
                            .chars()
                            .any(|ch| ch.is_whitespace() || ch == '=' || ch == '"' || ch == '\'')
                })
            {
                return Err(fail(
                    "expected an unquoted package::definition or package::module::definition selector",
                ));
            }
            *target = Some(value.to_owned());
        }
        if !version {
            return Err("missing language-item configuration version".into());
        }
        Ok(result)
    }
    pub(crate) fn apply<'a>(
        &'a self,
        explicit: Selectors<'a>,
        identities: &Protocols,
    ) -> Selectors<'a> {
        Selectors {
            copy: explicit.copy.or_else(|| {
                identities
                    .copy
                    .is_none()
                    .then_some(self.copy.as_deref())
                    .flatten()
            }),
            drop: explicit.drop.or_else(|| {
                identities
                    .drop
                    .is_none()
                    .then_some(self.drop.as_deref())
                    .flatten()
            }),
            index: explicit.index.or_else(|| {
                identities
                    .index
                    .is_none()
                    .then_some(self.index.as_deref())
                    .flatten()
            }),
            literal_into: explicit.literal_into.or_else(|| {
                identities
                    .literal_into
                    .is_none()
                    .then_some(self.literal_into.as_deref())
                    .flatten()
            }),
            arithmetic: std::array::from_fn(|index| {
                explicit.arithmetic[index].or_else(|| {
                    identities.arithmetic[index]
                        .is_none()
                        .then_some(self.arithmetic[index].as_deref())
                        .flatten()
                })
            }),
            splice: explicit.splice.or_else(|| {
                identities
                    .splice
                    .is_none()
                    .then_some(self.splice.as_deref())
                    .flatten()
            }),
            meta_list: explicit.meta_list.or_else(|| {
                identities
                    .meta_list
                    .is_none()
                    .then_some(self.meta_list.as_deref())
                    .flatten()
            }),
            tuple: explicit.tuple.or_else(|| {
                identities
                    .tuple
                    .is_none()
                    .then_some(self.tuple.as_deref())
                    .flatten()
            }),
            optional: explicit.optional.or_else(|| {
                identities
                    .optional
                    .is_none()
                    .then_some(self.optional.as_deref())
                    .flatten()
            }),
        }
    }
}

#[cfg(test)]
mod literal_binding_tests {
    use super::*;
    #[test]
    fn literal_binding_configuration_preserves_explicit_authority() {
        let config =
            SysrootBindings::parse("version = 1\nliteral-into-protocol = builtin::Convert\n")
                .unwrap();
        assert_eq!(
            config
                .apply(Selectors::default(), &Protocols::default())
                .literal_into,
            Some("builtin::Convert")
        );
        let explicit = Selectors {
            literal_into: Some("input::Custom"),
            ..Default::default()
        };
        assert_eq!(
            config.apply(explicit, &Protocols::default()).literal_into,
            explicit.literal_into
        );
        let bound = Protocols {
            literal_into: Some(nir::DeclarationId {
                package: 1,
                local: 7,
            }),
            ..Default::default()
        };
        assert_eq!(
            config.apply(Selectors::default(), &bound).literal_into,
            None
        );
        assert!(SysrootBindings::parse("version = 1\nliteral-into-protocol = input::One\nliteral-into-protocol = input::Two\n").is_err());
    }
}
