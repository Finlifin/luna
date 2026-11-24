//! Validate merged `__flurry_abi` calling-convention attributes.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AbiEntry {
    pub owner: OwnerId,
    pub convention: n::CallingConvention,
    pub source: n::SourceRange,
}

impl std::fmt::Display for AbiEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(abi-attribute (owner {}) {} (source {} {}))",
            self.owner, self.convention, self.source.start, self.source.end
        )
    }
}

/// Validate `__flurry_abi` entries on runtime function definitions or declarations.
pub fn calling_conventions(
    package: &hir::Package<'_>,
    merged: &[MergedTarget<'_>],
    budget: &mut comptime_runtime::Budget,
) -> Result<Vec<AbiEntry>, MergeFailure> {
    let mut output = Vec::new();
    for target in merged {
        let location = target.source();
        budget.consume(1).map_err(|e| MergeFailure {
            source: location,
            message: e.to_string(),
        })?;
        let Some(selected) = target.get("__flurry_abi") else {
            continue;
        };
        let location = selected.origin.source;
        let failure = |message: &str| MergeFailure {
            source: location,
            message: message.into(),
        };
        if selected.entry.is_none() || selected.origin.configuration_key.is_some() {
            return Err(failure(
                "__flurry_abi cannot be attached via definition configuration; use ^attrs.abi",
            ));
        }
        let hir::AttributeTarget::Definition(owner) = target.target else {
            return Err(failure("__flurry_abi requires a runtime function"));
        };
        let runtime_function = matches!(
            package.item(owner).map(|item| &item.kind),
            Some(ItemKind::Fn(sig, _) | ItemKind::FnDeclaration(sig))
                if !sig.modifiers.is_comptime && !sig.modifiers.is_quote
        );
        if !runtime_function {
            return Err(failure("__flurry_abi requires a runtime function"));
        }
        if selected.value.ty != p(P::Symbol) {
            return Err(failure("__flurry_abi requires the Symbol domain"));
        }
        let symbol = match &selected.value.kind {
            K::Literal(n::Literal::Symbol(name)) => name.as_str(),
            K::ComptimeValue(Term::Symbol(name)) => name.as_ref(),
            _ => {
                return Err(failure("__flurry_abi requires a completed Symbol value"));
            }
        };
        let Some(convention) = n::CallingConvention::from_symbol(symbol) else {
            return Err(failure(
                "unsupported __flurry_abi calling convention; expected .c, .system, .stdcall, .win64, or .sysv64",
            ));
        };
        output.push(AbiEntry {
            owner,
            convention,
            source: location,
        });
    }
    Ok(output)
}
