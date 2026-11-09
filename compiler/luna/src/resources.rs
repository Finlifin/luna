//! Explicit ownership-obligation reporting, with a caller-selected Copy identity.
//! Successful analysis does not lower Drop or certify complete program safety.
use elaboration::{
    capabilities::World,
    materialization::DeclarationSnapshot,
    ptc::{Budget, Context},
    verification::moves::Report,
};
use intrinsic::{LangItem, LangItems};
use nir::{
    DeclarationId,
    program::{FunctionPhase, Package},
};
use std::{fmt, sync::Arc};

#[cfg(test)]
mod tests;

/// A source selector is resolved once; downstream execution preserves its identity.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Selection<'a> {
    Name(&'a str),
    Identity(DeclarationId),
}
impl fmt::Display for Selection<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Name(name) => f.write_str(name),
            Self::Identity(id) => id.fmt(f),
        }
    }
}
impl Selection<'_> {
    fn matches(self, definition: &nir::program::Trait) -> bool {
        match self {
            Self::Name(name) => definition.name == name,
            Self::Identity(id) => definition.declaration == id,
        }
    }
}

pub struct Analysis {
    pub entry: DeclarationId,
    pub copy_protocol: DeclarationId,
    pub drop_protocol: Option<DeclarationId>,
    pub report: Report,
}
impl fmt::Display for Analysis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(resource-analysis (entry {}) (copy-protocol {})",
            self.entry, self.copy_protocol
        )?;
        if let Some(protocol) = self.drop_protocol {
            write!(
                f,
                " (drop-protocol {protocol}) (stage acquired-cleanup-plans)"
            )?;
        } else {
            f.write_str(" (stage ownership-obligations)")?;
        }
        write!(f, " {})", self.report)
    }
}

/// Names are resolved uniquely in this package, then bound by declaration identity.
/// The explicit protocol argument is driver configuration, never a spelling-based
/// inference that a trait called Copy has language-item authority.
pub fn analyze(
    package: &Package,
    entry: &str,
    copy_protocol: &str,
    fuel: u64,
) -> Result<Analysis, String> {
    analyze_with_protocols(package, entry, copy_protocol, None, fuel)
}

/// An explicit Drop identity enables acquired cleanup plans in the same world.
/// This is not execution or a full resource-safety certificate.
pub fn analyze_with_protocols(
    package: &Package,
    entry: &str,
    copy_protocol: &str,
    drop_protocol: Option<&str>,
    fuel: u64,
) -> Result<Analysis, String> {
    analyze_selected(
        package,
        entry,
        None,
        Selection::Name(copy_protocol),
        drop_protocol.map(Selection::Name),
        fuel,
    )
}

pub fn analyze_with_protocols_in_package(
    package: &Package,
    identity: u64,
    entry: &str,
    copy_protocol: &str,
    drop_protocol: Option<&str>,
    fuel: u64,
) -> Result<Analysis, String> {
    analyze_selected(
        package,
        entry,
        Some(identity),
        Selection::Name(copy_protocol),
        drop_protocol.map(Selection::Name),
        fuel,
    )
}

pub(crate) fn analyze_selected(
    package: &Package,
    entry: &str,
    identity: Option<u64>,
    copy_protocol: Selection<'_>,
    drop_protocol: Option<Selection<'_>>,
    fuel: u64,
) -> Result<Analysis, String> {
    let mut entries = package
        .functions
        .iter()
        .filter(|f| f.name == entry && identity.is_none_or(|id| f.declaration.package == id));
    let function = entries
        .next()
        .ok_or_else(|| format!("resource entry `{entry}` was not found"))?;
    if entries.next().is_some() {
        return Err(format!("resource entry `{entry}` is ambiguous"));
    }
    if function.phase != FunctionPhase::Runtime {
        return Err("resource entry must be an ordinary runtime function".into());
    }
    if !function.type_parameters.is_empty()
        || !function.requirements.is_empty()
        || !function.capabilities.is_empty()
    {
        return Err("resource entry needs a nongeneric wrapper without unresolved type/capability assumptions".into());
    }
    let steps =
        usize::try_from(fuel).map_err(|_| "resource fuel does not fit the host work counter")?;
    if steps == 0 {
        return Err("resource fuel must be positive".into());
    }
    let budget = Budget {
        steps,
        ..Default::default()
    };
    let binding = ProtocolContext::selected(package, copy_protocol, drop_protocol, budget)?;
    let protocol = binding.copy;
    let drop_protocol = binding.drop;
    let package = binding.package();
    let function = package
        .functions
        .iter()
        .find(|candidate| candidate.declaration == function.declaration)
        .expect("bound source retains the selected entry");
    let world = binding.world();
    let items = &binding.items;
    let report = if drop_protocol.is_some() {
        crate::protocols::analyze_cleanup(package, &world, items, function, steps, budget)
    } else {
        crate::protocols::analyze_moves(package, &world, items, function, steps, budget)
    }
    .map_err(|e| format!("resource analysis failed: {e}"))?;
    Ok(Analysis {
        entry: function.declaration,
        copy_protocol: protocol,
        drop_protocol,
        report,
    })
}

/// Driver-selected language identities and the matching implementation world.
/// Both analysis and execution bind the same validated source protocols.
pub(crate) struct ProtocolContext {
    pub items: LangItems,
    pub copy: DeclarationId,
    pub drop: Option<DeclarationId>,
    context: Context,
    source: DeclarationSnapshot,
}
impl ProtocolContext {
    #[cfg(test)]
    pub fn new(
        package: &Package,
        copy_protocol: &str,
        drop_protocol: Option<&str>,
        budget: Budget,
    ) -> Result<Self, String> {
        Self::selected(
            package,
            Selection::Name(copy_protocol),
            drop_protocol.map(Selection::Name),
            budget,
        )
    }
    pub(crate) fn selected(
        package: &Package,
        copy_protocol: Selection<'_>,
        drop_protocol: Option<Selection<'_>>,
        budget: Budget,
    ) -> Result<Self, String> {
        Self::selected_in_source(
            DeclarationSnapshot::new(Arc::new(package.clone())),
            copy_protocol,
            drop_protocol,
            budget,
        )
    }

    /// Retain the exact declarations used to bind the language protocols.
    /// Other invocation services can share the directory, but not proof facts.
    pub(crate) fn selected_in_source(
        source: DeclarationSnapshot,
        copy_protocol: Selection<'_>,
        drop_protocol: Option<Selection<'_>>,
        budget: Budget,
    ) -> Result<Self, String> {
        Self::selected_in_source_metered(source, copy_protocol, drop_protocol, &mut { budget })
    }

    /// Charge selection, declaration theory, directory and language bindings to
    /// one allowance. Failure returns no protocol authority; completed immutable
    /// directory metadata may remain shared by this exact source.
    pub(crate) fn selected_in_source_metered(
        source: DeclarationSnapshot,
        copy_protocol: Selection<'_>,
        drop_protocol: Option<Selection<'_>>,
        budget: &mut Budget,
    ) -> Result<Self, String> {
        let package = source.package();
        let definition = select_protocol(package, copy_protocol, "Copy", &mut budget.steps)?;
        if !definition.type_parameters.is_empty() {
            return Err("Copy protocol must have no explicit type parameters".into());
        }
        let protocol = definition.declaration;
        let drop_protocol = drop_protocol
            .map(|name| {
                select_protocol(package, name, "Drop", &mut budget.steps)
                    .map(|definition| definition.declaration)
            })
            .transpose()?;
        let context = Context::from_package_metered(package, &mut budget.steps)
            .map_err(|e| format!("resource type context: {e}"))?;
        source
            .prepare(budget)
            .map_err(|error| format!("resource declaration directory: {error}"))?;
        let mut items = LangItems::new();
        spend_binding(&items, package, &mut budget.steps)?;
        items
            .bind_source_traits(package, &[(LangItem::Copy, protocol)])
            .map_err(|e| format!("resource Copy binding: {e}"))?;
        if let Some(protocol) = drop_protocol {
            spend_binding(&items, package, &mut budget.steps)?;
            items
                .bind_source_traits(package, &[(LangItem::Drop, protocol)])
                .map_err(|e| format!("resource Drop binding: {e}"))?;
        }
        Ok(Self {
            items,
            copy: protocol,
            drop: drop_protocol,
            context,
            source,
        })
    }

    pub(crate) fn package(&self) -> &Package {
        self.source.package()
    }

    pub(crate) fn context(&self) -> &Context {
        &self.context
    }

    pub(crate) fn source(&self) -> elaboration::materialization::DeclarationSource<'_> {
        elaboration::materialization::DeclarationSource::Snapshot(&self.source)
    }

    pub fn world(&self) -> World<'_> {
        self.source
            .capabilities(
                &self.context,
                &[],
                &mut Budget {
                    steps: 0,
                    ..Default::default()
                },
            )
            .expect("ProtocolContext construction completed its declaration directory")
    }
}

fn select_protocol<'a>(
    package: &'a Package,
    selection: Selection<'_>,
    role: &str,
    fuel: &mut usize,
) -> Result<&'a nir::program::Trait, String> {
    let mut selected = None;
    for definition in &package.traits {
        *fuel = fuel
            .checked_sub(1)
            .ok_or("resource protocol selection budget exhausted")?;
        if let Selection::Name(name) = selection {
            if name.len() != definition.name.len() {
                continue;
            }
            *fuel = fuel
                .checked_sub(name.len())
                .ok_or("resource protocol selection budget exhausted")?;
        }
        if selection.matches(definition) {
            if selected.replace(definition).is_some() {
                return Err(format!("{role} protocol `{selection}` is ambiguous"));
            }
        }
    }
    selected.ok_or_else(|| format!("{role} protocol `{selection}` was not found"))
}

// bind_source_traits checks one identity by scanning the trait table, stages a
// copy of the current bindings, then inserts one entry. Preflight that work
// before its all-or-nothing registration; no protocol proof is performed here.
fn spend_binding(items: &LangItems, package: &Package, fuel: &mut usize) -> Result<(), String> {
    let work = package
        .traits
        .len()
        .saturating_add(items.len())
        .saturating_add(1);
    *fuel = fuel
        .checked_sub(work)
        .ok_or("resource language binding budget exhausted")?;
    Ok(())
}

impl fmt::Display for ProtocolContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(resource-protocol-context (copy {}) (drop", self.copy)?;
        if let Some(drop) = self.drop {
            write!(f, " {drop}")?;
        }
        write!(
            f,
            ") (bindings {}) (context {}) (source {}))",
            self.items, self.context, self.source
        )
    }
}
