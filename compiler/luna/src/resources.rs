//! Explicit ownership-obligation reporting, with a caller-selected Copy identity.
//! Successful analysis does not lower Drop or certify complete program safety.
use elaboration::{
    capabilities::World,
    implementation::Index,
    ptc::{Budget, Context},
    verification::moves::Report,
};
use intrinsic::{LangItem, LangItems};
use nir::{
    DeclarationId,
    program::{FunctionPhase, Package},
};
use std::fmt;

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
    let world = binding.world(package);
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
    index: Index,
}
impl ProtocolContext {
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
        let mut protocols = package.traits.iter().filter(|t| copy_protocol.matches(t));
        let definition = protocols
            .next()
            .ok_or_else(|| format!("Copy protocol `{copy_protocol}` was not found"))?;
        if protocols.next().is_some() {
            return Err(format!("Copy protocol `{copy_protocol}` is ambiguous"));
        }
        if !definition.type_parameters.is_empty() {
            return Err("Copy protocol must have no explicit type parameters".into());
        }
        let protocol = definition.declaration;
        let drop_protocol = drop_protocol
            .map(|name| {
                let mut definitions = package.traits.iter().filter(|t| name.matches(t));
                let definition = definitions
                    .next()
                    .ok_or_else(|| format!("Drop protocol `{name}` was not found"))?;
                if definitions.next().is_some() {
                    return Err(format!("Drop protocol `{name}` is ambiguous"));
                }
                Ok(definition.declaration)
            })
            .transpose()?;
        let context =
            Context::from_package(package).map_err(|e| format!("resource type context: {e}"))?;
        let index = crate::implementation_world::index(&package, budget)
            .map_err(|error| format!("resource implementation index: {error}"))?;
        let mut items = LangItems::new();
        items
            .bind_source_traits(package, &[(LangItem::Copy, protocol)])
            .map_err(|e| format!("resource Copy binding: {e}"))?;
        if let Some(protocol) = drop_protocol {
            items
                .bind_source_traits(package, &[(LangItem::Drop, protocol)])
                .map_err(|e| format!("resource Drop binding: {e}"))?;
        }
        Ok(Self {
            items,
            copy: protocol,
            drop: drop_protocol,
            context,
            index,
        })
    }

    pub fn world<'a>(&'a self, package: &'a Package) -> World<'a> {
        World {
            index: &self.index,
            implementations: &package.implementations,
            traits: &package.traits,
            assumptions: &[],
            context: &self.context,
        }
    }
}
