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

/// Validate the selected `__flurry_packed_mode` or transitional `packed`
/// property on struct definitions.
pub fn packed_structs(
    package: &hir::Package<'_>,
    merged: &[MergedTarget<'_>],
    budget: &mut comptime_runtime::Budget,
) -> Result<Vec<PackedStructLayout>, MergeFailure> {
    let mut output = Vec::new();
    for target in merged {
        let location = target
            .iter()
            .next()
            .map(|(_, value)| value.origin.source)
            .unwrap_or(n::SourceRange { start: 0, end: 0 });
        budget.consume(1).map_err(|e| MergeFailure {
            source: location,
            message: e.to_string(),
        })?;
        let mode_entry = target.get("__flurry_packed_mode");
        let legacy_entry = target.get("packed");
        if let (Some(mode_selected), Some(_)) = (mode_entry, legacy_entry) {
            return Err(MergeFailure {
                source: mode_selected.origin.source,
                message: "cannot combine legacy packed attribute with __flurry_packed_mode; use ^attrs.packed.byte or ^attrs.packed.bit".into(),
            });
        }
        if let Some(selected) = mode_entry {
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
            continue;
        }
        let Some(selected) = legacy_entry else {
            continue;
        };
        let location = selected.origin.source;
        let failure = |message: &str| MergeFailure {
            source: location,
            message: message.into(),
        };
        let hir::AttributeTarget::Definition(owner) = target.target else {
            return Err(failure("packed requires a struct definition"));
        };
        if !package
            .item(owner)
            .is_some_and(|item| matches!(item.kind, ItemKind::Struct(_)))
        {
            return Err(failure("packed requires a struct definition"));
        }
        if selected.value.ty != p(P::Bool) {
            return Err(failure("packed requires the Bool domain"));
        }
        let K::Literal(n::Literal::Bool(enabled)) = selected.value.kind else {
            return Err(failure("packed requires a completed Bool value"));
        };
        output.push(PackedStructLayout {
            owner,
            mode: if enabled {
                n::PackedMode::Byte
            } else {
                n::PackedMode::None
            },
            source: location,
        });
    }
    Ok(output)
}
