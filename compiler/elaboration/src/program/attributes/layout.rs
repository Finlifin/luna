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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AlignTarget {
    Definition {
        owner: OwnerId,
        align: u64,
        source: n::SourceRange,
    },
    Field {
        owner: OwnerId,
        field_index: usize,
        align: u64,
        source: n::SourceRange,
    },
}

const MAX_FLURRY_ALIGN_BYTES: u64 = 1 << 29;

fn parse_align_bytes(expr: &n::Expr) -> Option<u64> {
    match &expr.kind {
        K::Literal(n::Literal::UnsignedInteger(value)) => u64::try_from(*value).ok(),
        K::Literal(n::Literal::Integer(text)) => text.parse::<u64>().ok(),
        K::ComptimeValue(Term::Integer(value)) => value
            .to_unsigned(64)
            .ok()
            .and_then(|v| u64::try_from(v).ok()),
        _ => None,
    }
}

/// Validate the selected `__flurry_align` property on structs, enums, newtypes, and struct fields.
pub fn aligned_layouts(
    package: &hir::Package<'_>,
    merged: &[MergedTarget<'_>],
    packed_owners: &BTreeSet<OwnerId>,
    budget: &mut comptime_runtime::Budget,
) -> Result<Vec<AlignTarget>, MergeFailure> {
    let mut output = Vec::new();
    for target in merged {
        let location = target.source();
        budget.consume(1).map_err(|e| MergeFailure {
            source: location,
            message: e.to_string(),
        })?;
        let Some(selected) = target.get("__flurry_align") else {
            continue;
        };
        let location = selected.origin.source;
        let failure = |message: &str| MergeFailure {
            source: location,
            message: message.into(),
        };
        if selected.entry.is_none() || selected.origin.configuration_key.is_some() {
            return Err(failure(
                "__flurry_align cannot be attached via definition configuration; use ^attrs.layout.align(bytes)",
            ));
        }
        if selected.value.ty != p(P::Usize) {
            return Err(failure(
                "__flurry_align requires a usize alignment in bytes",
            ));
        }
        let Some(align) = parse_align_bytes(&selected.value) else {
            return Err(failure(
                "__flurry_align requires a completed usize constant value",
            ));
        };
        if align == 0 || !align.is_power_of_two() || align > MAX_FLURRY_ALIGN_BYTES {
            return Err(failure(
                "__flurry_align alignment must be a positive power of two (at most 2^29 bytes)",
            ));
        }
        match target.target {
            hir::AttributeTarget::Definition(owner) => {
                if !package.item(owner).is_some_and(|item| {
                    matches!(
                        item.kind,
                        ItemKind::Struct(_) | ItemKind::Enum(_) | ItemKind::Newtype(..)
                    )
                }) {
                    return Err(failure(
                        "__flurry_align requires a struct, enum, newtype, or struct field",
                    ));
                }
                if packed_owners.contains(&owner) {
                    return Err(failure(
                        "packed structs cannot combine ^attrs.packed.* with ^attrs.layout.align",
                    ));
                }
                output.push(AlignTarget::Definition {
                    owner,
                    align,
                    source: location,
                });
            }
            hir::AttributeTarget::Member(hir_id) => {
                let owner = hir_id.owner;
                let Some(def) = package.item(owner).and_then(|item| match &item.kind {
                    ItemKind::Struct(def) => Some(def),
                    _ => None,
                }) else {
                    return Err(failure(
                        "__flurry_align requires a struct, enum, newtype, or struct field",
                    ));
                };
                let Some(field_index) = def.fields.iter().position(|f| f.hir_id == hir_id) else {
                    return Err(failure(
                        "__flurry_align requires a struct, enum, newtype, or struct field",
                    ));
                };
                if packed_owners.contains(&owner) {
                    return Err(failure(
                        "packed struct fields cannot specify ^attrs.layout.align",
                    ));
                }
                output.push(AlignTarget::Field {
                    owner,
                    field_index,
                    align,
                    source: location,
                });
            }
        }
    }
    Ok(output)
}
