//! Explicit driver selectors for source-defined type constructors and operator protocols.
use elaboration::program::Protocols;

#[derive(Clone, Copy, Debug, Default)]
pub struct Selectors<'a> {
    pub dispatch_fuel: Option<usize>,
    pub host_fma: Option<&'a str>,
    pub host_sqrt: Option<&'a str>,
    pub host_copysign: Option<&'a str>,
    pub comptime_intrinsics: &'a [(nir::intrinsics::Operation, String)],
    pub host_intrinsics: &'a [(nir::intrinsics::Operation, String)],
    pub copy: Option<&'a str>,
    pub drop: Option<&'a str>,
    pub index: Option<&'a str>,
    pub iterator: Option<&'a str>,
    pub into_iterator: Option<&'a str>,
    pub variadic_collector: Option<&'a str>,
    pub extended_variadic_collector: Option<&'a str>,
    pub ordering: Option<&'a str>,
    pub equality: Option<&'a str>,
    pub list_pattern: Option<&'a str>,
    pub comptime_list_pattern: Option<&'a str>,
    pub stable_list_pattern: Option<&'a str>,
    pub list_pattern_endless: Option<&'a str>,
    pub stable_list_pattern_endless: Option<&'a str>,
    pub range_end: Option<&'a str>,
    pub range_pattern: Option<&'a str>,
    pub stable_range_pattern: Option<&'a str>,
    pub concat_pattern: Option<&'a str>,
    pub stable_concat_pattern: Option<&'a str>,
    pub map_pattern: Option<&'a str>,
    pub stable_map_pattern: Option<&'a str>,
    pub struct_pattern: Option<&'a str>,
    pub stable_struct_pattern: Option<&'a str>,
    pub type_pattern: Option<&'a str>,
    pub literal_into: Option<&'a str>,
    pub handle_error: Option<&'a str>,
    pub deref: Option<&'a str>,
    pub deref_mut: Option<&'a str>,
    pub reference: Option<&'a str>,
    pub implicit_passing: Option<&'a str>,
    pub call_context: Option<&'a str>,
    pub call_description: Option<&'a str>,
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
        if !self.comptime_intrinsics.is_empty()
            || !self.host_intrinsics.is_empty()
            || [
                self.host_fma,
                self.host_sqrt,
                self.host_copysign,
                self.copy,
                self.drop,
                self.index,
                self.iterator,
                self.into_iterator,
                self.variadic_collector,
                self.extended_variadic_collector,
                self.ordering,
                self.equality,
                self.list_pattern,
                self.comptime_list_pattern,
                self.stable_list_pattern,
                self.list_pattern_endless,
                self.stable_list_pattern_endless,
                self.range_end,
                self.range_pattern,
                self.stable_range_pattern,
                self.concat_pattern,
                self.stable_concat_pattern,
                self.map_pattern,
                self.stable_map_pattern,
                self.struct_pattern,
                self.stable_struct_pattern,
                self.type_pattern,
                self.literal_into,
                self.handle_error,
                self.deref,
                self.deref_mut,
                self.reference,
                self.implicit_passing,
                self.call_context,
                self.call_description,
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
    for (binding, operation, comptime) in protocols
        .host_bindings()
        .map(|(tag, op)| (tag, op, false))
        .chain(
            protocols
                .comptime_intrinsics
                .iter()
                .map(|(tag, op)| (tag, *op, true)),
        )
    {
        if let Some((previous, phase)) = selected.insert(binding, (operation, comptime)) {
            if phase != comptime {
                return Err("intrinsic tag cannot belong to both runtime and comptime".into());
            }
            if previous != operation {
                return Err("one intrinsic tag cannot bind both distinct target operations".into());
            }
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
    result.dispatch_fuel = selectors.dispatch_fuel.or(result.dispatch_fuel);
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
    for (operation, selector) in selectors.comptime_intrinsics {
        let binding = resolve_host_tag(&paths, hir, packages, "comptime intrinsic", selector)?;
        if result.host_bindings().any(|(tag, _)| tag == &binding) {
            return Err("intrinsic tag cannot belong to both runtime and comptime".into());
        }
        if let Some((_, previous)) = result
            .comptime_intrinsics
            .iter()
            .find(|(tag, _)| tag == &binding)
        {
            if previous != operation {
                return Err("conflicting comptime intrinsic binding".into());
            }
        } else {
            result.comptime_intrinsics.push((binding, *operation));
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
            (
                "IntoIterator".into(),
                selectors.into_iterator,
                &mut result.into_iterator,
            ),
            (
                "VariadicCollector".into(),
                selectors.variadic_collector,
                &mut result.variadic_collector,
            ),
            (
                "ExtendedVariadicCollector".into(),
                selectors.extended_variadic_collector,
                &mut result.extended_variadic_collector,
            ),
            ("Ord".into(), selectors.ordering, &mut result.ordering),
            ("Eq".into(), selectors.equality, &mut result.equality),
            (
                "ListPatternView".into(),
                selectors.list_pattern,
                &mut result.list_pattern,
            ),
            (
                "ComptimeListPatternView".into(),
                selectors.comptime_list_pattern,
                &mut result.comptime_list_pattern,
            ),
            (
                "StableListPatternView".into(),
                selectors.stable_list_pattern,
                &mut result.stable_list_pattern,
            ),
            (
                "ListPatternViewEndless".into(),
                selectors.list_pattern_endless,
                &mut result.list_pattern_endless,
            ),
            (
                "StableListPatternViewEndless".into(),
                selectors.stable_list_pattern_endless,
                &mut result.stable_list_pattern_endless,
            ),
            (
                "RangePatternView".into(),
                selectors.range_pattern,
                &mut result.range_pattern,
            ),
            (
                "StableRangePatternView".into(),
                selectors.stable_range_pattern,
                &mut result.stable_range_pattern,
            ),
            (
                "ConcatPatternView".into(),
                selectors.concat_pattern,
                &mut result.concat_pattern,
            ),
            (
                "StableConcatPatternView".into(),
                selectors.stable_concat_pattern,
                &mut result.stable_concat_pattern,
            ),
            (
                "MapPatternView".into(),
                selectors.map_pattern,
                &mut result.map_pattern,
            ),
            (
                "StableMapPatternView".into(),
                selectors.stable_map_pattern,
                &mut result.stable_map_pattern,
            ),
            (
                "StructPatternView".into(),
                selectors.struct_pattern,
                &mut result.struct_pattern,
            ),
            (
                "StableStructPatternView".into(),
                selectors.stable_struct_pattern,
                &mut result.stable_struct_pattern,
            ),
            (
                "TypePatternView".into(),
                selectors.type_pattern,
                &mut result.type_pattern,
            ),
            (
                "LiteralInto".into(),
                selectors.literal_into,
                &mut result.literal_into,
            ),
            (
                "HandleError".into(),
                selectors.handle_error,
                &mut result.handle_error,
            ),
            ("Deref".into(), selectors.deref, &mut result.deref),
            (
                "DerefMut".into(),
                selectors.deref_mut,
                &mut result.deref_mut,
            ),
            (
                "Reference".into(),
                selectors.reference,
                &mut result.reference,
            ),
            (
                "ImplicitPassing".into(),
                selectors.implicit_passing,
                &mut result.implicit_passing,
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
    use nir::normal_forms::BuiltinCallable;
    for (label, builtin, selector, target) in [
        (
            "meta.List",
            BuiltinCallable::List,
            selectors.meta_list,
            &mut result.meta_list,
        ),
        (
            "Tuple",
            BuiltinCallable::Tuple,
            selectors.tuple,
            &mut result.tuple,
        ),
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
        let valid = match &item.kind {
            hir::ItemKind::Fn(signature, _) => {
                signature.modifiers.is_normal_form_head && signature.modifiers.is_pure
            }
            hir::ItemKind::Const(_, initializer) => {
                matches!(&initializer.kind, hir::ExprKind::Builtin(symbol) if BuiltinCallable::from_symbol(symbol.as_str()) == Some(builtin))
            }
            _ => false,
        };
        if !valid {
            return Err(format!(
                "{label} selector requires a pure normal-form definition or builtin callable binding"
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
    if let Some(selector) = selectors.range_end {
        let parts = selector.split(".").collect::<Vec<_>>();
        if parts.len() < 2 || parts.iter().any(|part| part.is_empty()) {
            return Err("RangeEnd selector requires package.enum or package.module.enum".into());
        }
        let providers = packages
            .iter()
            .filter(|(_, _, name)| *name == parts[0])
            .collect::<Vec<_>>();
        let [provider] = providers.as_slice() else {
            return Err("RangeEnd selector package is missing or ambiguous".into());
        };
        let owner = paths
            .resolve(provider.0, &parts[1..])
            .ok_or_else(|| "RangeEnd selector is missing or ambiguous".to_string())?;
        if !hir
            .item(owner)
            .is_some_and(|item| matches!(item.kind, hir::ItemKind::Enum(_)))
        {
            return Err("RangeEnd selector requires an enum".into());
        }
        let definition = hir
            .owner_definition(owner)
            .ok_or("RangeEnd has no source identity")?;
        let id = nir::DeclarationId {
            package: provider.1,
            local: definition.index,
        };
        if result.range_end.is_some_and(|previous| previous != id) {
            return Err("RangeEnd selector conflicts with the driver's bound identity".into());
        }
        result.range_end = Some(id);
    }
    if result.deref_mut.is_some() && result.deref.is_none() {
        return Err("DerefMut binding requires a Deref binding".into());
    }
    for (label, selector, target) in [
        ("Context", selectors.call_context, &mut result.call_context),
        (
            "CallDescription",
            selectors.call_description,
            &mut result.call_description,
        ),
    ] {
        let Some(selector) = selector else {
            continue;
        };
        let parts = selector.split(".").collect::<Vec<_>>();
        if parts.len() < 2 || parts.iter().any(|part| part.is_empty()) {
            return Err(format!(
                "{label} selector requires package.struct or package.module.struct"
            ));
        }
        let providers = packages
            .iter()
            .filter(|(_, _, name)| *name == parts[0])
            .collect::<Vec<_>>();
        let [provider] = providers.as_slice() else {
            return Err(format!("{label} selector package is missing or ambiguous"));
        };
        let owner = paths
            .resolve(provider.0, &parts[1..])
            .ok_or_else(|| format!("{label} selector is missing or ambiguous"))?;
        if !hir
            .item(owner)
            .is_some_and(|item| matches!(item.kind, hir::ItemKind::Struct(_)))
        {
            return Err(format!("{label} selector requires a struct"));
        }
        let definition = hir
            .owner_definition(owner)
            .ok_or_else(|| format!("{label} has no source identity"))?;
        let id = nir::DeclarationId {
            package: provider.1,
            local: definition.index,
        };
        if target.is_some_and(|previous| previous != id) {
            return Err(format!(
                "{label} selector conflicts with the driver's bound identity"
            ));
        }
        *target = Some(id);
    }
    if result.range_pattern.is_some() && result.range_end.is_none() {
        return Err("RangePatternView binding requires a RangeEnd binding".into());
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
    for ((id, name), operation) in protocols.host_bindings().chain(
        protocols
            .comptime_intrinsics
            .iter()
            .map(|(tag, op)| (tag, *op)),
    ) {
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
                && matches!(
                    head.collector,
                    Some(nir::normal_forms::Collector { slot: 0, .. })
                )
                && head.parameters.len() == 1
                && head.parameters[0].kind == Kind::Value
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
    comptime_intrinsics: Vec<(nir::intrinsics::Operation, String)>,
    host_intrinsics: Vec<(nir::intrinsics::Operation, String)>,
    copy: Option<String>,
    drop: Option<String>,
    index: Option<String>,
    ordering: Option<String>,
    equality: Option<String>,
    list_pattern: Option<String>,
    comptime_list_pattern: Option<String>,
    stable_list_pattern: Option<String>,
    list_pattern_endless: Option<String>,
    stable_list_pattern_endless: Option<String>,
    range_end: Option<String>,
    range_pattern: Option<String>,
    stable_range_pattern: Option<String>,
    concat_pattern: Option<String>,
    stable_concat_pattern: Option<String>,
    map_pattern: Option<String>,
    stable_map_pattern: Option<String>,
    struct_pattern: Option<String>,
    stable_struct_pattern: Option<String>,
    type_pattern: Option<String>,
    iterator: Option<String>,
    into_iterator: Option<String>,
    variadic_collector: Option<String>,
    extended_variadic_collector: Option<String>,
    literal_into: Option<String>,
    handle_error: Option<String>,
    deref: Option<String>,
    deref_mut: Option<String>,
    reference: Option<String>,
    implicit_passing: Option<String>,
    call_context: Option<String>,
    call_description: Option<String>,
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
            if let Some(name) = key
                .strip_prefix("host-intrinsic.")
                .or_else(|| key.strip_prefix("comptime-intrinsic."))
            {
                let bindings = if key.starts_with("comptime-") {
                    &mut result.comptime_intrinsics
                } else {
                    &mut result.host_intrinsics
                };
                let operation = nir::intrinsics::Operation::from_name(name)
                    .ok_or_else(|| fail("unknown host intrinsic operation"))?;
                if bindings.iter().any(|(op, _)| *op == operation) {
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
                bindings.push((operation, value.to_owned()));
                continue;
            }
            let target = match key {
                "index-protocol" => &mut result.index,
                "ord-protocol" => &mut result.ordering,
                "eq-protocol" => &mut result.equality,
                "list-pattern-protocol" => &mut result.list_pattern,
                "comptime-list-pattern-protocol" => &mut result.comptime_list_pattern,
                "stable-list-pattern-protocol" => &mut result.stable_list_pattern,
                "list-pattern-endless-protocol" => &mut result.list_pattern_endless,
                "stable-list-pattern-endless-protocol" => &mut result.stable_list_pattern_endless,
                "range-end-type" => &mut result.range_end,
                "range-pattern-protocol" => &mut result.range_pattern,
                "stable-range-pattern-protocol" => &mut result.stable_range_pattern,
                "concat-pattern-protocol" => &mut result.concat_pattern,
                "stable-concat-pattern-protocol" => &mut result.stable_concat_pattern,
                "map-pattern-protocol" => &mut result.map_pattern,
                "stable-map-pattern-protocol" => &mut result.stable_map_pattern,
                "struct-pattern-protocol" => &mut result.struct_pattern,
                "stable-struct-pattern-protocol" => &mut result.stable_struct_pattern,
                "type-pattern-protocol" => &mut result.type_pattern,
                "iterator-protocol" => &mut result.iterator,
                "into-iterator-protocol" => &mut result.into_iterator,
                "variadic-collector-protocol" => &mut result.variadic_collector,
                "extended-variadic-collector-protocol" => &mut result.extended_variadic_collector,
                "literal-into-protocol" => &mut result.literal_into,
                "handle-error-protocol" => &mut result.handle_error,
                "deref-protocol" => &mut result.deref,
                "deref-mut-protocol" => &mut result.deref_mut,
                "reference-protocol" => &mut result.reference,
                "implicit-passing-protocol" => &mut result.implicit_passing,
                "call-context-type" => &mut result.call_context,
                "call-description-type" => &mut result.call_description,
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

    pub(crate) fn merged_comptime_intrinsics(
        &self,
        explicit: Selectors<'_>,
        identities: &Protocols,
    ) -> Vec<(nir::intrinsics::Operation, String)> {
        let mut merged = explicit.comptime_intrinsics.to_vec();
        for (operation, selector) in &self.comptime_intrinsics {
            if !explicit
                .comptime_intrinsics
                .iter()
                .any(|(op, _)| op == operation)
                && !identities
                    .comptime_intrinsics
                    .iter()
                    .any(|(_, op)| op == operation)
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
            dispatch_fuel: explicit.dispatch_fuel,
            host_fma: explicit.host_fma,
            host_sqrt: explicit.host_sqrt,
            host_copysign: explicit.host_copysign,
            comptime_intrinsics: explicit.comptime_intrinsics,
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
            comptime_list_pattern: explicit.comptime_list_pattern.or_else(|| {
                identities
                    .comptime_list_pattern
                    .is_none()
                    .then_some(self.comptime_list_pattern.as_deref())
                    .flatten()
            }),
            stable_list_pattern: explicit.stable_list_pattern.or_else(|| {
                identities
                    .stable_list_pattern
                    .is_none()
                    .then_some(self.stable_list_pattern.as_deref())
                    .flatten()
            }),
            list_pattern_endless: explicit.list_pattern_endless.or_else(|| {
                identities
                    .list_pattern_endless
                    .is_none()
                    .then_some(self.list_pattern_endless.as_deref())
                    .flatten()
            }),
            stable_list_pattern_endless: explicit.stable_list_pattern_endless.or_else(|| {
                identities
                    .stable_list_pattern_endless
                    .is_none()
                    .then_some(self.stable_list_pattern_endless.as_deref())
                    .flatten()
            }),
            range_end: explicit.range_end.or_else(|| {
                identities
                    .range_end
                    .is_none()
                    .then_some(self.range_end.as_deref())
                    .flatten()
            }),
            range_pattern: explicit.range_pattern.or_else(|| {
                identities
                    .range_pattern
                    .is_none()
                    .then_some(self.range_pattern.as_deref())
                    .flatten()
            }),
            stable_range_pattern: explicit.stable_range_pattern.or_else(|| {
                identities
                    .stable_range_pattern
                    .is_none()
                    .then_some(self.stable_range_pattern.as_deref())
                    .flatten()
            }),
            concat_pattern: explicit.concat_pattern.or_else(|| {
                identities
                    .concat_pattern
                    .is_none()
                    .then_some(self.concat_pattern.as_deref())
                    .flatten()
            }),
            stable_concat_pattern: explicit.stable_concat_pattern.or_else(|| {
                identities
                    .stable_concat_pattern
                    .is_none()
                    .then_some(self.stable_concat_pattern.as_deref())
                    .flatten()
            }),
            map_pattern: explicit.map_pattern.or_else(|| {
                identities
                    .map_pattern
                    .is_none()
                    .then_some(self.map_pattern.as_deref())
                    .flatten()
            }),
            stable_map_pattern: explicit.stable_map_pattern.or_else(|| {
                identities
                    .stable_map_pattern
                    .is_none()
                    .then_some(self.stable_map_pattern.as_deref())
                    .flatten()
            }),
            struct_pattern: explicit.struct_pattern.or_else(|| {
                identities
                    .struct_pattern
                    .is_none()
                    .then_some(self.struct_pattern.as_deref())
                    .flatten()
            }),
            stable_struct_pattern: explicit.stable_struct_pattern.or_else(|| {
                identities
                    .stable_struct_pattern
                    .is_none()
                    .then_some(self.stable_struct_pattern.as_deref())
                    .flatten()
            }),
            type_pattern: explicit.type_pattern.or_else(|| {
                identities
                    .type_pattern
                    .is_none()
                    .then_some(self.type_pattern.as_deref())
                    .flatten()
            }),
            iterator: explicit.iterator.or_else(|| {
                identities
                    .iterator
                    .is_none()
                    .then_some(self.iterator.as_deref())
                    .flatten()
            }),
            into_iterator: explicit.into_iterator.or_else(|| {
                identities
                    .into_iterator
                    .is_none()
                    .then_some(self.into_iterator.as_deref())
                    .flatten()
            }),
            variadic_collector: explicit.variadic_collector.or_else(|| {
                identities
                    .variadic_collector
                    .is_none()
                    .then_some(self.variadic_collector.as_deref())
                    .flatten()
            }),
            extended_variadic_collector: explicit.extended_variadic_collector.or_else(|| {
                identities
                    .extended_variadic_collector
                    .is_none()
                    .then_some(self.extended_variadic_collector.as_deref())
                    .flatten()
            }),
            literal_into: explicit.literal_into.or_else(|| {
                identities
                    .literal_into
                    .is_none()
                    .then_some(self.literal_into.as_deref())
                    .flatten()
            }),
            handle_error: explicit.handle_error.or_else(|| {
                identities
                    .handle_error
                    .is_none()
                    .then_some(self.handle_error.as_deref())
                    .flatten()
            }),
            deref: explicit.deref.or_else(|| {
                identities
                    .deref
                    .is_none()
                    .then_some(self.deref.as_deref())
                    .flatten()
            }),
            deref_mut: explicit.deref_mut.or_else(|| {
                identities
                    .deref_mut
                    .is_none()
                    .then_some(self.deref_mut.as_deref())
                    .flatten()
            }),
            reference: explicit.reference.or_else(|| {
                identities
                    .reference
                    .is_none()
                    .then_some(self.reference.as_deref())
                    .flatten()
            }),
            implicit_passing: explicit.implicit_passing.or_else(|| {
                identities
                    .implicit_passing
                    .is_none()
                    .then_some(self.implicit_passing.as_deref())
                    .flatten()
            }),
            call_context: explicit.call_context.or_else(|| {
                identities
                    .call_context
                    .is_none()
                    .then_some(self.call_context.as_deref())
                    .flatten()
            }),
            call_description: explicit.call_description.or_else(|| {
                identities
                    .call_description
                    .is_none()
                    .then_some(self.call_description.as_deref())
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
    fn comptime_list_pattern_binding_configuration_preserves_driver_authority() {
        let config =
            SysrootBindings::parse("version = 1\ncomptime-list-pattern-protocol = meta.ListView\n")
                .unwrap();
        assert_eq!(
            config
                .apply(Selectors::default(), &Protocols::default())
                .comptime_list_pattern,
            Some("meta.ListView")
        );
        let explicit = Selectors {
            comptime_list_pattern: Some("input.View"),
            ..Default::default()
        };
        assert_eq!(
            config
                .apply(explicit, &Protocols::default())
                .comptime_list_pattern,
            explicit.comptime_list_pattern
        );
        let bound = Protocols {
            comptime_list_pattern: Some(nir::DeclarationId {
                package: 1,
                local: 7,
            }),
            ..Default::default()
        };
        assert_eq!(
            config
                .apply(Selectors::default(), &bound)
                .comptime_list_pattern,
            None
        );
        assert!(SysrootBindings::parse("version = 1\ncomptime-list-pattern-protocol = input.One\ncomptime-list-pattern-protocol = input.Two\n").is_err());
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
            let path = if operation.is_platform() {
                format!(
                    "std.platform.linux.target.System.{}",
                    operation.name().strip_prefix("linux_").unwrap()
                )
            } else if operation.is_memory() {
                format!(
                    "builtin.mem_impl.raw.target.Memory.{}",
                    operation.name().strip_prefix("mem_").unwrap()
                )
            } else {
                format!(
                    "builtin.numeric.math_intrinsics.target.Scalar.{}",
                    operation.name()
                )
            };
            assert!(bindings.host_intrinsics.contains(&(operation, path)));
        }

        assert_eq!(
            bindings.copy.as_deref(),
            Some("builtin.protocols.resources.Copy")
        );
        assert_eq!(
            bindings.drop.as_deref(),
            Some("builtin.protocols.resources.Drop")
        );
        assert!(
            SysrootBindings::parse("version = 1\noptional-type = builtin.option.Option\n").is_err()
        );
        assert_eq!(bindings.meta_list.as_deref(), Some("meta.List"));
        assert_eq!(bindings.tuple.as_deref(), Some("builtin.types.tuple.Tuple"));
        assert_eq!(
            bindings.index.as_deref(),
            Some("builtin.protocols.ops.Index")
        );
        assert_eq!(
            bindings.ordering.as_deref(),
            Some("builtin.numeric.comparison.Ord")
        );
        assert_eq!(
            bindings.equality.as_deref(),
            Some("builtin.numeric.comparison.Eq")
        );
        assert_eq!(
            bindings.range_end.as_deref(),
            Some("builtin.protocols.patterns.RangeEnd")
        );
        assert_eq!(
            bindings.range_pattern.as_deref(),
            Some("builtin.protocols.patterns.RangePatternView")
        );
        assert_eq!(
            bindings.stable_range_pattern.as_deref(),
            Some("builtin.protocols.patterns.StableRangePatternView")
        );
        for (operator, declaration) in [
            (elaboration::arithmetic::Operator::Add, "Add"),
            (elaboration::arithmetic::Operator::Subtract, "Sub"),
            (elaboration::arithmetic::Operator::Multiply, "Mul"),
            (elaboration::arithmetic::Operator::Divide, "Div"),
            (elaboration::arithmetic::Operator::Remainder, "Rem"),
        ] {
            assert_eq!(
                bindings.arithmetic[operator as usize].as_deref(),
                Some(format!("builtin.protocols.ops.{declaration}").as_str())
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
            dispatch_fuel: None,
            comptime_intrinsics: Vec::new(),
            input: "absent.fl".into(),
            emit: Emit::Hir,
            output: None,
            build_modes: Default::default(),
            build_targets: Default::default(),
            sysroot: None,
            index_protocol: None,
            ord_protocol: None,
            eq_protocol: None,
            list_pattern_protocol: None,
            comptime_list_pattern_protocol: None,
            stable_list_pattern_protocol: None,
            list_pattern_endless_protocol: None,
            stable_list_pattern_endless_protocol: None,
            range_end_type: None,
            range_pattern_protocol: None,
            stable_range_pattern_protocol: None,
            concat_pattern_protocol: None,
            stable_concat_pattern_protocol: None,
            map_pattern_protocol: None,
            stable_map_pattern_protocol: None,
            struct_pattern_protocol: None,
            stable_struct_pattern_protocol: None,
            type_pattern_protocol: None,
            iterator_protocol: None,
            into_iterator_protocol: None,
            variadic_collector_protocol: None,
            extended_variadic_collector_protocol: None,
            literal_into_protocol: None,
            deref_protocol: None,
            deref_mut_protocol: None,
            reference_protocol: None,
            implicit_passing_protocol: None,
            call_context_type: None,
            call_description_type: None,
            arithmetic_protocols: Default::default(),
            splice_protocol: None,
            meta_list_type: None,
            tuple_type: None,
            incremental: Default::default(),
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

#[cfg(test)]
mod comptime_math_bindings_tests {
    use super::*;
    use nir::intrinsics::Operation as O;

    #[test]
    fn defaults_preserve_partial_overrides_and_driver_identity() {
        let config = SysrootBindings::parse("version = 1\ncomptime-intrinsic.sqrt = meta.math.Scalar.sqrt\ncomptime-intrinsic.fma = meta.math.Scalar.fma\n").unwrap();
        let overrides = [(O::SquareRoot, "custom.Scalar.root".into())];
        let selectors = Selectors {
            comptime_intrinsics: &overrides,
            ..Default::default()
        };
        let merged = config.merged_comptime_intrinsics(selectors, &Protocols::default());
        assert_eq!(
            merged,
            vec![
                overrides[0].clone(),
                (O::FusedMultiplyAdd, "meta.math.Scalar.fma".into())
            ]
        );
        let tag = (
            nir::DeclarationId {
                package: 7,
                local: 9,
            },
            "fma".into(),
        );
        let mut protocols = Protocols {
            comptime_intrinsics: vec![(tag.clone(), O::FusedMultiplyAdd)],
            ..Default::default()
        };
        assert_eq!(
            config.merged_comptime_intrinsics(selectors, &protocols),
            overrides
        );
        protocols.host_intrinsics.push((tag, O::FusedMultiplyAdd));
        assert!(
            validate_host_bindings(&protocols)
                .unwrap_err()
                .contains("both runtime and comptime")
        );
    }

    #[test]
    fn malformed_or_conflicting_comptime_defaults_are_rejected() {
        for entry in [
            "comptime-intrinsic.missing = meta.Scalar.x",
            "comptime-intrinsic.sqrt = meta.sqrt",
            "comptime-intrinsic.sqrt = meta.Scalar.sqrt\ncomptime-intrinsic.sqrt = other.Scalar.sqrt",
        ] {
            assert!(
                SysrootBindings::parse(&format!("version = 1\n{entry}\n")).is_err(),
                "{entry}"
            );
        }
    }
}
