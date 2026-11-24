//! Validate merged `__flurry_purity` runtime purity-tier attributes.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PurityTier {
    P0,
    P1,
    P2,
    P3,
    P4,
    P5,
}

impl PurityTier {
    pub fn from_symbol(symbol: &str) -> Option<Self> {
        match symbol {
            "p0" => Some(Self::P0),
            "p1" => Some(Self::P1),
            "p2" => Some(Self::P2),
            "p3" => Some(Self::P3),
            "p4" => Some(Self::P4),
            "p5" => Some(Self::P5),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::P0 => "p0",
            Self::P1 => "p1",
            Self::P2 => "p2",
            Self::P3 => "p3",
            Self::P4 => "p4",
            Self::P5 => "p5",
        }
    }
}

impl std::fmt::Display for PurityTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(purity-tier {})", self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PurityEntry {
    pub owner: OwnerId,
    pub tier: PurityTier,
    pub source: n::SourceRange,
}

impl std::fmt::Display for PurityEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(purity-attribute (owner {}) {} (source {} {}))",
            self.owner, self.tier, self.source.start, self.source.end
        )
    }
}

/// Validate `__flurry_purity` entries on runtime function definitions or declarations.
/// Rejects definition configuration syntax (`.__flurry_purity`), non-runtime-function
/// targets, non-Symbol payloads, and symbols outside `.p0`..`.p5`.
pub fn purity_annotations(
    package: &hir::Package<'_>,
    merged: &[MergedTarget<'_>],
    budget: &mut comptime_runtime::Budget,
) -> Result<Vec<PurityEntry>, MergeFailure> {
    let mut output = Vec::new();
    for target in merged {
        let location = target.source();
        budget.consume(1).map_err(|e| MergeFailure {
            source: location,
            message: e.to_string(),
        })?;
        let Some(selected) = target.get("__flurry_purity") else {
            continue;
        };
        let location = selected.origin.source;
        let failure = |message: &str| MergeFailure {
            source: location,
            message: message.into(),
        };
        if selected.entry.is_none() || selected.origin.configuration_key.is_some() {
            return Err(failure(
                "__flurry_purity cannot be attached via definition configuration; use ^attrs.purity.{p0,p1,p2,p3,p4,p5}",
            ));
        }
        let hir::AttributeTarget::Definition(owner) = target.target else {
            return Err(failure("__flurry_purity requires a runtime function"));
        };
        let runtime_function = matches!(
            package.item(owner).map(|item| &item.kind),
            Some(ItemKind::Fn(sig, _) | ItemKind::Case(sig, _) | ItemKind::FnDeclaration(sig))
                if !sig.modifiers.is_comptime
                    && !sig.modifiers.is_quote
                    && !sig.modifiers.is_normal_form_head
        );
        if !runtime_function {
            return Err(failure("__flurry_purity requires a runtime function"));
        }
        if selected.value.ty != p(P::Symbol) {
            return Err(failure("__flurry_purity requires the Symbol domain"));
        }
        let symbol = match &selected.value.kind {
            K::Literal(n::Literal::Symbol(name)) => name.as_str(),
            K::ComptimeValue(Term::Symbol(name)) => name.as_ref(),
            _ => {
                return Err(failure("__flurry_purity requires a completed Symbol value"));
            }
        };
        let Some(tier) = PurityTier::from_symbol(symbol) else {
            return Err(failure(
                "unsupported __flurry_purity tier; expected .p0, .p1, .p2, .p3, .p4, or .p5",
            ));
        };
        output.push(PurityEntry {
            owner,
            tier,
            source: location,
        });
    }
    Ok(output)
}
