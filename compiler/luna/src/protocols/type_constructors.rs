//! Explicit driver selectors for source-defined type constructors and operator protocols.
use elaboration::program::Protocols;

#[derive(Clone, Copy, Debug, Default)]
pub struct Selectors<'a> {
    pub host_fma: Option<&'a str>,
    pub host_sqrt: Option<&'a str>,
    pub host_copysign: Option<&'a str>,
    pub host_intrinsics: &'a [(nir::intrinsics::Operation, String)],
    pub copy: Option<&'a str>,
    pub drop: Option<&'a str>,
    pub index: Option<&'a str>,
    pub iterator: Option<&'a str>,
    pub ordering: Option<&'a str>,
    pub equality: Option<&'a str>,
    pub list_pattern: Option<&'a str>,
    pub literal_into: Option<&'a str>,
    pub arithmetic: [Option<&'a str>; 7],
    pub splice: Option<&'a str>,
    pub meta_list: Option<&'a str>,
    pub tuple: Option<&'a str>,
}

impl Selectors<'_> {
    /// Binding source declarations is an elaboration action, never a syntax dump.
    pub(crate) fn validate_stage(self, emit: crate::cli::Emit) -> Result<(), String> {
        if matches!(
            emit,
            crate::cli::Emit::Nir
                | crate::cli::Emit::Attributes
                | crate::cli::Emit::MergedAttributes
        ) {
            return Ok(());
        }
        if !self.host_intrinsics.is_empty()
            || [
                self.host_fma,
                self.host_sqrt,
                self.host_copysign,
                self.copy,
                self.drop,
                self.index,
                self.iterator,
                self.ordering,
                self.equality,
                self.list_pattern,
                self.literal_into,
                self.splice,
                self.meta_list,
                self.tuple,
            ]
            .into_iter()
            .chain(self.arithmetic)
            .any(|selector| selector.is_some())
        {
            return Err("source protocol/type binding requires NIR elaboration (--emit nir or an execution/analysis command)".into());
        }
        Ok(())
    }
}

fn resolve_host_tag(
    paths: &super::source::ModulePaths<'_, '_>,
    hir: &hir::Package<'_>,
    packages: &[(u32, u64, &str)],
    label: &str,
    selector: &str,
) -> Result<(nir::DeclarationId, String), String> {
    let parts: Vec<_> = selector.split(".").collect();
    if parts.len() < 3 || parts.iter().any(|part| part.is_empty()) {
        return Err(format!(
            "host {label} selector requires package.module.Enum.variant (module optional)"
        ));
    }
    let providers: Vec<_> = packages
        .iter()
        .filter(|(_, _, name)| *name == parts[0])
        .collect();
    let [provider] = providers.as_slice() else {
        return Err(format!("host {label} package is missing or ambiguous"));
    };
    let owner = paths
        .resolve(provider.0, &parts[1..parts.len() - 1])
        .ok_or(format!(
            "host {label} enum selector is missing or ambiguous"
        ))?;
    let Some(hir::ItemKind::Enum(family)) = hir.item(owner).map(|item| &item.kind) else {
        return Err(format!("host {label} selector requires an enum"));
    };
    if !family.is_intrinsic {
        return Err(format!("host {label} enum requires intrinsic_enum"));
    }
    let variant = parts[parts.len() - 1];
    if !family
        .variants
        .iter()
        .any(|v| v.ident.name.as_str() == variant && matches!(v.kind, hir::VariantKind::Unit))
    {
        return Err(format!(
            "host {label} selector requires a declared unit variant"
        ));
    }
    let definition = hir
        .owner_definition(owner)
        .ok_or(format!("host {label} declaration identity missing"))?;
    let binding = (
        nir::DeclarationId {
            package: provider.1,
            local: definition.index,
        },
        variant.to_owned(),
    );
    Ok(binding)
}

fn validate_host_bindings(protocols: &Protocols) -> Result<(), String> {
    let mut selected = std::collections::BTreeMap::new();
    for (binding, operation) in protocols.host_bindings() {
        if selected
            .insert(binding, operation)
            .is_some_and(|previous| previous != operation)
        {
            return Err("one intrinsic tag cannot bind both distinct target operations".into());
        }
    }
    Ok(())
}

pub(crate) fn resolve_source(
    hir: &hir::Package<'_>,
    packages: &[(u32, u64, &str)],
    selectors: Selectors<'_>,
    identities: &Protocols,
) -> Result<Protocols, String> {
    let paths = super::source::ModulePaths::new(hir);
    let mut result = super::splice::resolve_source(hir, packages, selectors.splice, identities)?;
    for (label, selector, target) in [
        ("FMA", selectors.host_fma, &mut result.host_fma),
        ("sqrt", selectors.host_sqrt, &mut result.host_sqrt),
        (
            "copysign",
            selectors.host_copysign,
            &mut result.host_copysign,
        ),
    ] {
        let Some(selector) = selector else {
            continue;
        };
        let binding = resolve_host_tag(&paths, hir, packages, label, selector)?;
        if target.as_ref().is_some_and(|previous| previous != &binding) {
            return Err(format!(
                "host {label} selector conflicts with driver binding"
            ));
        }
        *target = Some(binding);
    }
    for (operation, selector) in selectors.host_intrinsics {
        let binding = resolve_host_tag(&paths, hir, packages, "intrinsic", selector)?;
        if !result
            .host_intrinsics
            .iter()
            .any(|(existing, bound)| existing == &binding && bound == operation)
        {
            result.host_intrinsics.push((binding, *operation));
        }
    }
    validate_host_bindings(&result)?;
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
            ("Iterator".into(), selectors.iterator, &mut result.iterator),
            ("Ord".into(), selectors.ordering, &mut result.ordering),
            ("Eq".into(), selectors.equality, &mut result.equality),
            (
                "ListPatternView".into(),
                selectors.list_pattern,
                &mut result.list_pattern,
            ),
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
        let parts = selector.split(".").collect::<Vec<_>>();
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
                "{operator} selector requires package.trait or package.module.trait"
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
        let owner = paths
            .resolve(provider.0, &parts[1..])
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
    ] {
        let Some(selector) = selector else {
            continue;
        };
        let components = selector.split(".").collect::<Vec<_>>();
        if components.len() < 2 || components.iter().any(|part| part.is_empty()) {
            return Err(format!(
                "{label} selector must be package.definition or package.module.definition"
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
        let owner = paths
            .resolve(provider.0, &components[1..])
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
    validate_host_bindings(protocols)?;
    for ((id, name), operation) in protocols.host_bindings() {
        let mut families = package
            .enums
            .iter()
            .filter(|family| family.declaration == *id);
        let family = families.next().ok_or("host intrinsic family missing")?;
        if families.next().is_some() {
            return Err("host intrinsic family ambiguous".into());
        }
        let index = family
            .variants
            .iter()
            .position(|variant| variant.name == *name)
            .ok_or("host intrinsic variant missing")?;
        nir::intrinsics::Registry::for_family(family, index, operation).map_err(str::to_owned)?;
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
            head.role == Role::TypeList
                && head.parameters.len() == 1
                && head.parameters[0].kind == Kind::Type
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
    host_intrinsics: Vec<(nir::intrinsics::Operation, String)>,
    copy: Option<String>,
    drop: Option<String>,
    index: Option<String>,
    ordering: Option<String>,
    equality: Option<String>,
    list_pattern: Option<String>,
    iterator: Option<String>,
    literal_into: Option<String>,
    arithmetic: [Option<String>; 7],
    splice: Option<String>,
    meta_list: Option<String>,
    tuple: Option<String>,
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
            if let Some(name) = key.strip_prefix("host-intrinsic.") {
                let operation = nir::intrinsics::Operation::from_name(name)
                    .ok_or_else(|| fail("unknown host intrinsic operation"))?;
                if result
                    .host_intrinsics
                    .iter()
                    .any(|(op, _)| *op == operation)
                {
                    return Err(fail("duplicate host intrinsic operation"));
                }
                if value.split(".").count() < 3
                    || value.split(".").any(|part| {
                        part.is_empty()
                            || part
                                .chars()
                                .any(|ch| ch.is_whitespace() || matches!(ch, '=' | '\"' | '\''))
                    })
                {
                    return Err(fail("expected an unquoted package.Enum.variant selector"));
                }
                result.host_intrinsics.push((operation, value.to_owned()));
                continue;
            }
            let target = match key {
                "index-protocol" => &mut result.index,
                "ord-protocol" => &mut result.ordering,
                "eq-protocol" => &mut result.equality,
                "list-pattern-protocol" => &mut result.list_pattern,
                "iterator-protocol" => &mut result.iterator,
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
                _ => return Err(fail("unknown language-item key")),
            };
            if target.is_some() {
                return Err(fail("duplicate language-item key"));
            }
            let parts = value.split(".").collect::<Vec<_>>();
            if parts.len() < 2
                || parts.iter().any(|part| {
                    part.is_empty()
                        || part
                            .chars()
                            .any(|ch| ch.is_whitespace() || ch == '=' || ch == '"' || ch == '\'')
                })
            {
                return Err(fail(
                    "expected an unquoted package.definition or package.module.definition selector",
                ));
            }
            *target = Some(value.to_owned());
        }
        if !version {
            return Err("missing language-item configuration version".into());
        }
        Ok(result)
    }
    /// Merge defaults by operation, retaining explicit selectors verbatim so
    /// duplicate/conflicting user bindings still reach normal validation.
    pub(crate) fn merged_host_intrinsics(
        &self,
        explicit: Selectors<'_>,
        identities: &Protocols,
    ) -> Vec<(nir::intrinsics::Operation, String)> {
        use nir::intrinsics::Operation as O;
        let mut merged = explicit.host_intrinsics.to_vec();
        for (operation, selector) in &self.host_intrinsics {
            let dedicated = match operation {
                O::FusedMultiplyAdd => explicit.host_fma.is_some(),
                O::SquareRoot => explicit.host_sqrt.is_some(),
                O::CopySign => explicit.host_copysign.is_some(),
                _ => false,
            };
            if !dedicated
                && !explicit
                    .host_intrinsics
                    .iter()
                    .any(|(op, _)| op == operation)
                && !identities.host_bindings().any(|(_, op)| op == *operation)
            {
                merged.push((*operation, selector.clone()));
            }
        }
        merged
    }
    pub(crate) fn apply<'a>(
        &'a self,
        explicit: Selectors<'a>,
        identities: &Protocols,
    ) -> Selectors<'a> {
        Selectors {
            host_fma: explicit.host_fma,
            host_sqrt: explicit.host_sqrt,
            host_copysign: explicit.host_copysign,
            host_intrinsics: explicit.host_intrinsics,
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
            ordering: explicit.ordering.or_else(|| {
                identities
                    .ordering
                    .is_none()
                    .then_some(self.ordering.as_deref())
                    .flatten()
            }),
            equality: explicit.equality.or_else(|| {
                identities
                    .equality
                    .is_none()
                    .then_some(self.equality.as_deref())
                    .flatten()
            }),
            list_pattern: explicit.list_pattern.or_else(|| {
                identities
                    .list_pattern
                    .is_none()
                    .then_some(self.list_pattern.as_deref())
                    .flatten()
            }),
            iterator: explicit.iterator.or_else(|| {
                identities
                    .iterator
                    .is_none()
                    .then_some(self.iterator.as_deref())
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
        }
    }
}

#[cfg(test)]
mod literal_binding_tests {
    use super::*;
    #[test]
    fn list_pattern_binding_configuration_preserves_driver_authority() {
        let config =
            SysrootBindings::parse("version = 1\nlist-pattern-protocol = builtin.ListView\n")
                .unwrap();
        assert_eq!(
            config
                .apply(Selectors::default(), &Protocols::default())
                .list_pattern,
            Some("builtin.ListView")
        );
        let explicit = Selectors {
            list_pattern: Some("input.View"),
            ..Default::default()
        };
        assert_eq!(
            config.apply(explicit, &Protocols::default()).list_pattern,
            explicit.list_pattern
        );
        let bound = Protocols {
            list_pattern: Some(nir::DeclarationId {
                package: 1,
                local: 7,
            }),
            ..Default::default()
        };
        assert_eq!(
            config.apply(Selectors::default(), &bound).list_pattern,
            None
        );
        assert!(SysrootBindings::parse("version = 1\nlist-pattern-protocol = input.One\nlist-pattern-protocol = input.Two\n").is_err());
    }
    #[test]
    fn iterator_binding_configuration_preserves_driver_authority() {
        let config =
            SysrootBindings::parse("version = 1\niterator-protocol = builtin.Stream\n").unwrap();
        assert_eq!(
            config
                .apply(Selectors::default(), &Protocols::default())
                .iterator,
            Some("builtin.Stream")
        );
        let explicit = Selectors {
            iterator: Some("input.View"),
            ..Default::default()
        };
        assert_eq!(
            config.apply(explicit, &Protocols::default()).iterator,
            explicit.iterator
        );
        let bound = Protocols {
            iterator: Some(nir::DeclarationId {
                package: 1,
                local: 7,
            }),
            ..Default::default()
        };
        assert_eq!(config.apply(Selectors::default(), &bound).iterator, None);
        assert!(
            SysrootBindings::parse(
                "version = 1\niterator-protocol = input.One\niterator-protocol = input.Two\n"
            )
            .is_err()
        );
    }
    #[test]
    fn literal_binding_configuration_preserves_explicit_authority() {
        let config =
            SysrootBindings::parse("version = 1\nliteral-into-protocol = builtin.Convert\n")
                .unwrap();
        assert_eq!(
            config
                .apply(Selectors::default(), &Protocols::default())
                .literal_into,
            Some("builtin.Convert")
        );
        let explicit = Selectors {
            literal_into: Some("input.Custom"),
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
        assert!(SysrootBindings::parse("version = 1\nliteral-into-protocol = input.One\nliteral-into-protocol = input.Two\n").is_err());
    }
}

#[cfg(test)]
mod repository_bindings_tests {
    use super::*;

    #[test]
    fn sysroot_host_intrinsics_preserve_explicit_and_driver_authority() {
        use nir::intrinsics::Operation as O;
        let config = SysrootBindings::parse(
            "version = 1\nhost-intrinsic.sqrt = math.Ops.sqrt\nhost-intrinsic.abs = math.Ops.abs\n",
        )
        .unwrap();
        assert_eq!(
            config
                .merged_host_intrinsics(Selectors::default(), &Protocols::default())
                .len(),
            2
        );
        let explicit = [(O::WrappingAbsoluteValue, "input.Ops.abs".into())];
        let selectors = Selectors {
            host_sqrt: Some("input.Ops.root"),
            host_intrinsics: &explicit,
            ..Default::default()
        };
        assert_eq!(
            config.merged_host_intrinsics(selectors, &Protocols::default()),
            explicit
        );
        let identities = Protocols {
            host_sqrt: Some((
                nir::DeclarationId {
                    package: 1,
                    local: 2,
                },
                "sqrt".into(),
            )),
            host_intrinsics: vec![(
                (
                    nir::DeclarationId {
                        package: 1,
                        local: 3,
                    },
                    "abs".into(),
                ),
                O::WrappingAbsoluteValue,
            )],
            ..Default::default()
        };
        assert!(
            config
                .merged_host_intrinsics(Selectors::default(), &identities)
                .is_empty()
        );
        for line in [
            "host-intrinsic.unknown = math.Ops.x",
            "host-intrinsic.sqrt = math.sqrt",
            "host-intrinsic.sqrt = math.Ops.",
            "host-intrinsic.sqrt = math. Ops.sqrt",
            "host-intrinsic.sqrt = math.Ops.sqrt\nhost-intrinsic.sqrt = other.Ops.sqrt",
        ] {
            assert!(
                SysrootBindings::parse(&format!("version = 1\n{line}\n")).is_err(),
                "{line}"
            );
        }
    }

    #[test]
    fn repository_sysroot_binds_operators_without_optional_declaration() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
        let bindings = SysrootBindings::load(&root).unwrap();
        for &operation in nir::intrinsics::Operation::ALL {
            assert!(bindings.host_intrinsics.contains(&(
                operation,
                format!("builtin.math_intrinsics.target.Scalar.{}", operation.name())
            )));
        }

        assert_eq!(bindings.copy.as_deref(), Some("builtin.resources.Copy"));
        assert_eq!(bindings.drop.as_deref(), Some("builtin.resources.Drop"));
        assert!(
            SysrootBindings::parse("version = 1\noptional-type = builtin.option.Option\n").is_err()
        );
        assert_eq!(bindings.meta_list.as_deref(), Some("meta.List"));
        assert_eq!(bindings.tuple.as_deref(), Some("builtin.tuple.Tuple"));
        assert_eq!(bindings.index.as_deref(), Some("builtin.ops.Index"));
        assert_eq!(bindings.ordering.as_deref(), Some("builtin.comparison.Ord"));
        assert_eq!(bindings.equality.as_deref(), Some("builtin.comparison.Eq"));
        for (operator, declaration) in [
            (elaboration::arithmetic::Operator::Add, "Add"),
            (elaboration::arithmetic::Operator::Subtract, "Sub"),
            (elaboration::arithmetic::Operator::Multiply, "Mul"),
            (elaboration::arithmetic::Operator::Divide, "Div"),
            (elaboration::arithmetic::Operator::Remainder, "Rem"),
        ] {
            assert_eq!(
                bindings.arithmetic[operator as usize].as_deref(),
                Some(format!("builtin.ops.{declaration}").as_str())
            );
        }
    }
}

#[cfg(test)]
mod stage_tests {
    use super::*;
    use crate::cli::Emit;

    #[test]
    fn explicit_api_selectors_cannot_be_silently_ignored_by_syntax_exports() {
        let selectors = [
            Selectors {
                host_fma: Some("input.Ops.fma"),
                ..Default::default()
            },
            Selectors {
                copy: Some("Copy"),
                ..Default::default()
            },
            Selectors {
                drop: Some("Drop"),
                ..Default::default()
            },
            Selectors {
                list_pattern: Some("input.ListPatternView"),
                ..Default::default()
            },
            Selectors {
                arithmetic: [Some("input.Operator"); 7],
                ..Default::default()
            },
        ];
        let options = crate::cli::Options {
            input: "absent.fl".into(),
            emit: Emit::Hir,
            output: None,
            sysroot: None,
            index_protocol: None,
            ord_protocol: None,
            eq_protocol: None,
            list_pattern_protocol: None,
            iterator_protocol: None,
            literal_into_protocol: None,
            arithmetic_protocols: Default::default(),
            splice_protocol: None,
            meta_list_type: None,
            tuple_type: None,
        };
        for selector in selectors {
            // Validate before filesystem access, including selectors supplied
            // directly by the driver rather than through Options.
            let error = crate::frontend::compile_in_package_with_source_protocols(
                &options,
                0,
                &Default::default(),
                selector,
            )
            .err()
            .unwrap();
            assert!(error.contains("requires NIR"), "{error}");
            let error = crate::packages::compile_stage_with_selectors(
                &[],
                &[],
                0,
                None,
                Emit::Hir,
                None,
                &Default::default(),
                selector,
            )
            .err()
            .unwrap();
            assert!(error.contains("requires NIR"), "{error}");
            assert!(selector.validate_stage(Emit::Nir).is_ok());
        }
        let error = crate::packages::compile_stage_with_selectors(
            &[],
            &[],
            0,
            None,
            Emit::Ast,
            None,
            &Default::default(),
            Default::default(),
        )
        .err()
        .unwrap();
        assert!(error.contains("not AST"), "{error}");
    }
}
