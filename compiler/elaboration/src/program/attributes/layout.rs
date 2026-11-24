//! Validate merged struct layout attributes.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackedStructLayout {
    pub owner: OwnerId,
    pub mode: n::PackedMode,
    pub source: n::SourceRange,
}

impl std::fmt::Display for PackedStructLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(packed-struct-layout (owner {}) (mode {}) (source {} {}))",
            self.owner, self.mode, self.source.start, self.source.end
        )
    }
}

/// Validate the selected `__flurry_packed_mode` property on struct definitions.
pub fn packed_structs(
    package: &hir::Package<'_>,
    merged: &[MergedTarget<'_>],
    budget: &mut comptime_runtime::Budget,
) -> Result<Vec<PackedStructLayout>, MergeFailure> {
    let mut output = Vec::new();
    for target in merged {
        let location = target.source();
        budget.consume(1).map_err(|e| MergeFailure {
            source: location,
            message: e.to_string(),
        })?;
        let Some(selected) = target.get("__flurry_packed_mode") else {
            continue;
        };
        let location = selected.origin.source;
        let failure = |message: &str| MergeFailure {
            source: location,
            message: message.into(),
        };
        if selected.entry.is_none() || selected.origin.configuration_key.is_some() {
            return Err(failure(
                "__flurry_packed_mode cannot be attached via definition configuration; use ^attrs.packed.byte or ^attrs.packed.bit",
            ));
        }
        let hir::AttributeTarget::Definition(owner) = target.target else {
            return Err(failure("__flurry_packed_mode requires a struct definition"));
        };
        if !package
            .item(owner)
            .is_some_and(|item| matches!(item.kind, ItemKind::Struct(_)))
        {
            return Err(failure("__flurry_packed_mode requires a struct definition"));
        }
        if selected.value.ty != p(P::Symbol) {
            return Err(failure("__flurry_packed_mode requires the Symbol domain"));
        }
        let mode = match &selected.value.kind {
            K::Literal(n::Literal::Symbol(name)) => match name.as_str() {
                "byte" => n::PackedMode::Byte,
                "bit" => n::PackedMode::Bit,
                _ => return Err(failure("__flurry_packed_mode requires .byte or .bit")),
            },
            K::ComptimeValue(Term::Symbol(name)) => match name.as_ref() {
                "byte" => n::PackedMode::Byte,
                "bit" => n::PackedMode::Bit,
                _ => return Err(failure("__flurry_packed_mode requires .byte or .bit")),
            },
            _ => {
                return Err(failure(
                    "__flurry_packed_mode requires a completed Symbol value",
                ));
            }
        };
        output.push(PackedStructLayout {
            owner,
            mode,
            source: location,
        });
    }
    Ok(output)
}
