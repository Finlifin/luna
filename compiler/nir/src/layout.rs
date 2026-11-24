//! Target-aware physical storage layout calculation for NIR types.
//!
//! Matches host LLVM type lowering (`ptr`, `{ i1, payload }`, `{ i{8,16,32}, union }`
//! for error results and aggregate enums, sequential structs, transparent newtypes,
//! and unit/symbol/fixed-bit enums).
//! Pointers and function pointers do not recurse into pointees or parameter/result
//! bodies; direct or indirect by-value cycles and comptime-only domains are rejected.

use crate::TypeKind;
use crate::{DeclarationId, Primitive, Sort, Term};
use std::fmt;

const MAX_LAYOUT_DEPTH: usize = 128;

const MAX_BIT_WIDTH: u32 = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TypeLayoutQuery {
    SizeOf,
    AlignOf,
    StrideOf,
    BitWidthOf,
    FieldOffset,
}

impl TypeLayoutQuery {
    pub fn select(self, layout: TypeLayout) -> Result<u64, LayoutError> {
        match self {
            Self::SizeOf => Ok(layout.size),
            Self::AlignOf => Ok(layout.align),
            Self::StrideOf => align_up(layout.size, layout.align),
            Self::BitWidthOf => layout.size.checked_mul(8).ok_or(LayoutError::Overflow),
            Self::FieldOffset => Err(LayoutError::Overflow),
        }
    }
}

impl fmt::Display for TypeLayoutQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::SizeOf => "size-of",
            Self::AlignOf => "align-of",
            Self::StrideOf => "stride-of",
            Self::BitWidthOf => "bit-width-of",
            Self::FieldOffset => "field-offset",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TypeLayout {
    pub size: u64,
    pub align: u64,
}

impl fmt::Display for TypeLayout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(type-layout (size {}) (align {}))",
            self.size, self.align
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayoutError {
    TargetRequired,
    ComptimeOnly(Term),
    Unresolved(Term),
    Cycle(Term),
    Unsupported(Term),
    UnknownField {
        ty: Term,
        field: String,
    },
    UnalignedBitField {
        ty: Term,
        field: String,
        bit_offset: u32,
    },
    Overflow,
    BudgetExhausted,
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TargetRequired => {
                f.write_str("target-dependent layout query requires target pointer width")
            }
            Self::ComptimeOnly(ty) => {
                write!(f, "comptime-only type {ty} has no runtime storage layout")
            }
            Self::Unresolved(ty) => {
                write!(f, "cannot compute layout of unresolved type {ty}")
            }
            Self::Cycle(ty) => {
                write!(f, "type {ty} has infinite size due to a by-value cycle")
            }
            Self::Unsupported(ty) => {
                write!(f, "type {ty} has no concrete runtime storage layout")
            }
            Self::UnknownField { ty, field } => {
                write!(f, "type {ty} has no runtime field `{field}`")
            }
            Self::UnalignedBitField {
                ty,
                field,
                bit_offset,
            } => {
                write!(
                    f,
                    "bit-packed field `{field}` of {ty} starts at non-byte-aligned bit offset {bit_offset}"
                )
            }
            Self::Overflow => f.write_str("type layout size computation overflowed"),
            Self::BudgetExhausted => f.write_str("type layout computation budget exhausted"),
        }
    }
}

impl std::error::Error for LayoutError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NominalLayout {
    Struct(Vec<Term>),
    PackedStruct(Vec<Term>),
    BitPackedStruct(Vec<Term>),
    Newtype(Term),
    EnumUnit(usize),
    EnumSymbol,
    EnumFixedBits {
        width: u32,
        niche: Option<(u64, u64)>,
        exhaustive: bool,
    },
    EnumAggregate(Vec<Vec<Term>>),
}

/// LSB-first bit range and value restriction summary for one field inside an
/// `attrs.packed.bit` struct.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BitPackedFieldLayout {
    pub bit_offset: u32,
    pub bit_width: u32,
    pub restricted: bool,
}

/// Exact bit-level layout computed for an `attrs.packed.bit` struct.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BitPackedStructLayout {
    pub total_bits: u32,
    pub fields: Vec<BitPackedFieldLayout>,
    pub restricted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BitPackedError {
    Layout(LayoutError),
    UnsupportedField {
        field_index: usize,
        ty: Term,
        reason: &'static str,
    },
    WidthOverflow,
}

impl fmt::Display for BitPackedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Layout(error) => error.fmt(f),
            Self::UnsupportedField {
                field_index,
                ty,
                reason,
            } => write!(
                f,
                "field {field_index} of type {ty} is unsupported in attrs.packed.bit: {reason}"
            ),
            Self::WidthOverflow => {
                f.write_str("attrs.packed.bit total bit width exceeds supported maximum")
            }
        }
    }
}

impl std::error::Error for BitPackedError {}

impl From<LayoutError> for BitPackedError {
    fn from(error: LayoutError) -> Self {
        Self::Layout(error)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NicheScalar {
    Pointer,
    Int { bits: u32 },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Niche {
    pub path: Vec<usize>,
    pub scalar: NicheScalar,
    pub start: u64,
    pub count: u64,
    pub needs_bool_widen: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LayoutWithNiche {
    pub layout: TypeLayout,
    pub niche: Option<Niche>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldLookup {
    Index(usize),
    NotFound,
    Unavailable,
}

pub trait LayoutWorld {
    fn target(&self) -> Option<crate::target::TargetData>;

    fn is_tuple_head(&self, head: DeclarationId) -> bool;

    fn is_list_head(&self, head: DeclarationId) -> bool;

    fn nominal_kind(&self, ty: &Term) -> Option<crate::NominalKind>;

    fn is_template_identity(&self, _ty: &Term) -> bool {
        false
    }

    fn nominal_layout(
        &self,
        ty: &Term,
        fuel: &mut usize,
    ) -> Result<Option<NominalLayout>, LayoutError>;

    fn nominal_field_index(
        &self,
        _ty: &Term,
        _field: &str,
        _fuel: &mut usize,
    ) -> Result<FieldLookup, LayoutError> {
        Ok(FieldLookup::Unavailable)
    }
}

pub fn compute_layout(
    world: &impl LayoutWorld,
    ty: &Term,
    fuel: &mut usize,
) -> Result<TypeLayout, LayoutError> {
    compute_layout_and_niche(world, ty, fuel).map(|res| res.layout)
}

pub fn compute_stride(
    world: &impl LayoutWorld,
    ty: &Term,
    fuel: &mut usize,
) -> Result<u64, LayoutError> {
    let layout = compute_layout(world, ty, fuel)?;
    align_up(layout.size, layout.align)
}

pub fn compute_bit_width(
    world: &impl LayoutWorld,
    ty: &Term,
    fuel: &mut usize,
) -> Result<u64, LayoutError> {
    let mut visiting = Vec::new();
    bit_width_at(world, ty, 0, &mut visiting, fuel)
}

pub fn compute_field_offset(
    world: &impl LayoutWorld,
    ty: &Term,
    field: &str,
    fuel: &mut usize,
) -> Result<u64, LayoutError> {
    // Validate the enclosing type's runtime layout, target requirements, and acyclicity first.
    compute_layout(world, ty, fuel)?;
    match ty {
        Term::Record(fields) => {
            let Some(index) = fields.iter().position(|(name, _)| name == field) else {
                return Err(LayoutError::UnknownField {
                    ty: ty.clone(),
                    field: field.to_string(),
                });
            };
            let mut offset = 0u64;
            for (i, (_, field_ty)) in fields.iter().enumerate() {
                let fl = compute_layout(world, field_ty, fuel)?;
                offset = align_up(offset, fl.align)?;
                if i == index {
                    return Ok(offset);
                }
                offset = offset.checked_add(fl.size).ok_or(LayoutError::Overflow)?;
            }
            Err(LayoutError::UnknownField {
                ty: ty.clone(),
                field: field.to_string(),
            })
        }
        Term::Nominal { .. } => {
            let Some(definition) = world.nominal_layout(ty, fuel)? else {
                return Err(LayoutError::Unresolved(ty.clone()));
            };
            let index = match world.nominal_field_index(ty, field, fuel)? {
                FieldLookup::Index(index) => index,
                FieldLookup::NotFound => {
                    return Err(LayoutError::UnknownField {
                        ty: ty.clone(),
                        field: field.to_string(),
                    });
                }
                FieldLookup::Unavailable => return Err(LayoutError::Unresolved(ty.clone())),
            };
            match definition {
                NominalLayout::Struct(fields) => {
                    let mut offset = 0u64;
                    for (i, field_ty) in fields.iter().enumerate() {
                        let fl = compute_layout(world, field_ty, fuel)?;
                        offset = align_up(offset, fl.align)?;
                        if i == index {
                            return Ok(offset);
                        }
                        offset = offset.checked_add(fl.size).ok_or(LayoutError::Overflow)?;
                    }
                    Err(LayoutError::UnknownField {
                        ty: ty.clone(),
                        field: field.to_string(),
                    })
                }
                NominalLayout::PackedStruct(fields) => {
                    let mut offset = 0u64;
                    for (i, field_ty) in fields.iter().enumerate() {
                        let fl = compute_layout(world, field_ty, fuel)?;
                        if i == index {
                            return Ok(offset);
                        }
                        offset = offset.checked_add(fl.size).ok_or(LayoutError::Overflow)?;
                    }
                    Err(LayoutError::UnknownField {
                        ty: ty.clone(),
                        field: field.to_string(),
                    })
                }
                NominalLayout::BitPackedStruct(fields) => {
                    let bp = bit_packed_struct_layout(world, &fields, fuel).map_err(
                        |err| match err {
                            BitPackedError::Layout(layout_err) => layout_err,
                            BitPackedError::UnsupportedField { .. } => {
                                LayoutError::Unsupported(ty.clone())
                            }
                            BitPackedError::WidthOverflow => LayoutError::Overflow,
                        },
                    )?;
                    let Some(field_layout) = bp.fields.get(index) else {
                        return Err(LayoutError::UnknownField {
                            ty: ty.clone(),
                            field: field.to_string(),
                        });
                    };
                    if field_layout.bit_offset % 8 != 0 {
                        return Err(LayoutError::UnalignedBitField {
                            ty: ty.clone(),
                            field: field.to_string(),
                            bit_offset: field_layout.bit_offset,
                        });
                    }
                    Ok(u64::from(field_layout.bit_offset / 8))
                }
                _ => Err(LayoutError::UnknownField {
                    ty: ty.clone(),
                    field: field.to_string(),
                }),
            }
        }
        _ => Err(LayoutError::UnknownField {
            ty: ty.clone(),
            field: field.to_string(),
        }),
    }
}

pub fn compute_type_layout_query(
    world: &impl LayoutWorld,
    query: TypeLayoutQuery,
    ty: &Term,
    field: Option<&str>,
    fuel: &mut usize,
) -> Result<u64, LayoutError> {
    match query {
        TypeLayoutQuery::SizeOf => compute_layout(world, ty, fuel).map(|l| l.size),
        TypeLayoutQuery::AlignOf => compute_layout(world, ty, fuel).map(|l| l.align),
        TypeLayoutQuery::StrideOf => compute_stride(world, ty, fuel),
        TypeLayoutQuery::BitWidthOf => compute_bit_width(world, ty, fuel),
        TypeLayoutQuery::FieldOffset => {
            let Some(field) = field else {
                return Err(LayoutError::Unsupported(ty.clone()));
            };
            compute_field_offset(world, ty, field, fuel)
        }
    }
}

fn bit_width_at(
    world: &impl LayoutWorld,
    ty: &Term,
    depth: usize,
    visiting: &mut Vec<Term>,
    fuel: &mut usize,
) -> Result<u64, LayoutError> {
    charge(fuel, 1)?;
    if depth >= MAX_LAYOUT_DEPTH {
        return Err(LayoutError::BudgetExhausted);
    }
    if world.is_template_identity(ty) {
        return Ok(64);
    }
    match ty {
        Term::Primitive(Primitive::Void) => Ok(0),
        Term::Primitive(Primitive::Bool) => Ok(1),
        Term::Primitive(Primitive::Char) => Ok(32),
        Term::Primitive(Primitive::I8 | Primitive::U8) => Ok(8),
        Term::Primitive(Primitive::I16 | Primitive::U16 | Primitive::F16 | Primitive::Bf16) => {
            Ok(16)
        }
        Term::Primitive(Primitive::I32 | Primitive::U32 | Primitive::F32) => Ok(32),
        Term::Primitive(Primitive::I64 | Primitive::U64 | Primitive::F64) => Ok(64),
        Term::Primitive(Primitive::I128 | Primitive::U128) => Ok(128),
        Term::Primitive(Primitive::Isize | Primitive::Usize) => Ok(u64::from(
            world
                .target()
                .ok_or(LayoutError::TargetRequired)?
                .pointer_bits(),
        )),
        Term::Primitive(Primitive::BitVec(width)) => {
            let bits = u32::try_from(*width).map_err(|_| LayoutError::Unsupported(ty.clone()))?;
            if bits > MAX_BIT_WIDTH {
                return Err(LayoutError::Unsupported(ty.clone()));
            }
            Ok(u64::from(bits))
        }
        Term::Primitive(
            Primitive::Type
            | Primitive::Object
            | Primitive::Integer
            | Primitive::Float
            | Primitive::Unit,
        )
        | Term::EffectRowType
        | Term::ErrorRowType
        | Term::ReachabilityType
        | Term::AnyType => Err(LayoutError::ComptimeOnly(ty.clone())),
        Term::Primitive(Primitive::Never) => Err(LayoutError::Unsupported(ty.clone())),
        Term::Pointer(_) => Ok(u64::from(
            world
                .target()
                .ok_or(LayoutError::TargetRequired)?
                .pointer_bits(),
        )),
        Term::UnsafeFunction(signature) => {
            bit_width_at(world, signature, depth + 1, visiting, fuel)
        }
        Term::Function { parameters, .. } => {
            if parameters.iter().any(|p| p.sort != Sort::Runtime) {
                return Err(LayoutError::ComptimeOnly(ty.clone()));
            }
            Ok(u64::from(
                world
                    .target()
                    .ok_or(LayoutError::TargetRequired)?
                    .pointer_bits(),
            ))
        }
        Term::ComptimeFunction(_) | Term::NormalFormFunction(..) | Term::Lift(_) => {
            Err(LayoutError::ComptimeOnly(ty.clone()))
        }
        Term::Nominal { .. } => {
            if visiting.contains(ty) {
                return Err(LayoutError::Cycle(ty.clone()));
            }
            let Some(definition) = world.nominal_layout(ty, fuel)? else {
                return Err(LayoutError::Unresolved(ty.clone()));
            };
            visiting.push(ty.clone());
            let result: Result<Option<u64>, LayoutError> = match definition {
                NominalLayout::BitPackedStruct(fields) => {
                    bit_packed_struct_layout_at(world, &fields, depth + 1, visiting, fuel)
                        .map(|bp| Some(u64::from(bp.total_bits)))
                        .map_err(|err| match err {
                            BitPackedError::Layout(layout_err) => layout_err,
                            BitPackedError::UnsupportedField { .. } => {
                                LayoutError::Unsupported(ty.clone())
                            }
                            BitPackedError::WidthOverflow => LayoutError::Overflow,
                        })
                }
                NominalLayout::EnumFixedBits { width, .. } => {
                    if width > MAX_BIT_WIDTH {
                        Err(LayoutError::Unsupported(ty.clone()))
                    } else {
                        Ok(Some(u64::from(width)))
                    }
                }
                NominalLayout::EnumUnit(variant_count) => {
                    Ok(Some(u64::from(unit_enum_tag_bits(variant_count)?)))
                }
                NominalLayout::Newtype(representation) => {
                    bit_width_at(world, &representation, depth + 1, visiting, fuel).map(Some)
                }
                NominalLayout::Struct(_)
                | NominalLayout::PackedStruct(_)
                | NominalLayout::EnumSymbol
                | NominalLayout::EnumAggregate(_) => Ok(None),
            };
            visiting.pop();
            match result? {
                Some(bits) => Ok(bits),
                None => {
                    let layout = layout_with_niche_at(world, ty, depth, visiting, fuel)?.layout;
                    layout.size.checked_mul(8).ok_or(LayoutError::Overflow)
                }
            }
        }
        _ => {
            let layout = layout_with_niche_at(world, ty, depth, visiting, fuel)?.layout;
            layout.size.checked_mul(8).ok_or(LayoutError::Overflow)
        }
    }
}

pub fn compute_layout_and_niche(
    world: &impl LayoutWorld,
    ty: &Term,
    fuel: &mut usize,
) -> Result<LayoutWithNiche, LayoutError> {
    let mut visiting = Vec::new();
    layout_with_niche_at(world, ty, 0, &mut visiting, fuel)
}

pub fn unit_enum_tag_bits(variant_count: usize) -> Result<u32, LayoutError> {
    if variant_count <= 1 {
        Ok(0)
    } else if variant_count <= 256 {
        Ok(8)
    } else if variant_count <= 65_536 {
        Ok(16)
    } else if u32::try_from(variant_count - 1).is_ok() {
        Ok(32)
    } else {
        Err(LayoutError::Overflow)
    }
}

pub fn evaluate_kind(world: &impl LayoutWorld, kind: TypeKind, ty: &Term) -> Option<bool> {
    kind.evaluate_with_declarations(
        ty,
        |head| world.is_tuple_head(head),
        |_| world.nominal_kind(ty),
    )
}

/// Evaluate `Type.predicates.is_not_sized(ty)` against a concrete `LayoutWorld`,
/// returning `Err(LayoutError::ComptimeOnly(_))` if any component is comptime-only.
pub fn evaluate_not_sized(
    world: &impl LayoutWorld,
    ty: &Term,
    fuel: &mut usize,
) -> Result<Option<bool>, LayoutError> {
    let mut visiting = Vec::new();
    evaluate_not_sized_at(world, ty, 0, &mut visiting, fuel)
}

fn evaluate_not_sized_aggregate<'a>(
    world: &impl LayoutWorld,
    fields: impl Iterator<Item = &'a Term>,
    depth: usize,
    visiting: &mut Vec<Term>,
    fuel: &mut usize,
) -> Result<Option<bool>, LayoutError> {
    let mut unknown = false;
    let mut saw_unsized = false;
    for field in fields {
        match evaluate_not_sized_at(world, field, depth, visiting, fuel)? {
            Some(true) => saw_unsized = true,
            Some(false) => {}
            None => unknown = true,
        }
    }
    if saw_unsized {
        Ok(Some(true))
    } else if unknown {
        Ok(None)
    } else {
        Ok(Some(false))
    }
}

fn evaluate_not_sized_at(
    world: &impl LayoutWorld,
    ty: &Term,
    depth: usize,
    visiting: &mut Vec<Term>,
    fuel: &mut usize,
) -> Result<Option<bool>, LayoutError> {
    charge(fuel, 1)?;
    if depth >= MAX_LAYOUT_DEPTH {
        return Err(LayoutError::BudgetExhausted);
    }
    match ty {
        Term::Primitive(
            Primitive::Type
            | Primitive::Object
            | Primitive::Integer
            | Primitive::Float
            | Primitive::Unit,
        )
        | Term::ComptimeFunction(_)
        | Term::NormalFormFunction(..)
        | Term::Lift(_)
        | Term::ClosureContextType
        | Term::EffectRowType
        | Term::ErrorRowType
        | Term::ReachabilityType
        | Term::AnyType => Err(LayoutError::ComptimeOnly(ty.clone())),
        Term::Function { parameters, .. } => {
            if parameters.iter().any(|p| p.sort != Sort::Runtime) {
                Err(LayoutError::ComptimeOnly(ty.clone()))
            } else {
                Ok(Some(false))
            }
        }
        Term::UnsafeFunction(inner) => {
            evaluate_not_sized_at(world, inner, depth + 1, visiting, fuel)
        }
        Term::Closure {
            environment,
            signature,
        } => {
            if !matches!(signature.as_ref(), Term::TraitObject { .. })
                && !matches!(
                    signature.callable_signature(),
                    Term::Function { parameters, .. } if parameters.iter().all(|p| p.sort == Sort::Runtime)
                )
            {
                return Err(LayoutError::ComptimeOnly(ty.clone()));
            }
            evaluate_not_sized_aggregate(world, environment.iter(), depth + 1, visiting, fuel)
        }
        Term::Primitive(
            Primitive::Bool
            | Primitive::Char
            | Primitive::Str
            | Primitive::Symbol
            | Primitive::Void
            | Primitive::Never
            | Primitive::I8
            | Primitive::I16
            | Primitive::I32
            | Primitive::I64
            | Primitive::I128
            | Primitive::Isize
            | Primitive::U8
            | Primitive::U16
            | Primitive::U32
            | Primitive::U64
            | Primitive::U128
            | Primitive::Usize
            | Primitive::F16
            | Primitive::Bf16
            | Primitive::F32
            | Primitive::F64
            | Primitive::BitVec(_),
        )
        | Term::Pointer(_) => Ok(Some(false)),
        Term::TraitObject { .. } => Ok(Some(false)),
        Term::Optional(inner) => evaluate_not_sized_at(world, inner, depth + 1, visiting, fuel),
        Term::Tuple(elements) => {
            evaluate_not_sized_aggregate(world, elements.iter(), depth + 1, visiting, fuel)
        }
        Term::Record(fields) => evaluate_not_sized_aggregate(
            world,
            fields.iter().map(|(_, t)| t),
            depth + 1,
            visiting,
            fuel,
        ),
        Term::ErrorResult { errors, result } => evaluate_not_sized_aggregate(
            world,
            errors.iter().chain(std::iter::once(result.as_ref())),
            depth + 1,
            visiting,
            fuel,
        ),
        Term::NormalForm {
            head,
            sort: Sort::Type,
            arguments,
        } => {
            if world.is_list_head(*head) {
                return Err(LayoutError::ComptimeOnly(ty.clone()));
            }
            if world.is_tuple_head(*head) {
                if let Some(elements) =
                    crate::normal_forms::type_list_fields_with_layouts(arguments, &|id| {
                        world.is_list_head(id)
                    })
                {
                    return evaluate_not_sized_aggregate(
                        world,
                        elements.iter(),
                        depth + 1,
                        visiting,
                        fuel,
                    );
                }
            }
            Ok(None)
        }
        Term::Nominal { .. } => {
            if visiting.contains(ty) {
                return Err(LayoutError::Cycle(ty.clone()));
            }
            let Some(definition) = world.nominal_layout(ty, fuel)? else {
                return Ok(None);
            };
            visiting.push(ty.clone());
            let result = match definition {
                NominalLayout::Struct(fields)
                | NominalLayout::PackedStruct(fields)
                | NominalLayout::BitPackedStruct(fields) => {
                    evaluate_not_sized_aggregate(world, fields.iter(), depth + 1, visiting, fuel)
                }
                NominalLayout::Newtype(repr) => {
                    evaluate_not_sized_at(world, &repr, depth + 1, visiting, fuel)
                }
                NominalLayout::EnumUnit(_)
                | NominalLayout::EnumSymbol
                | NominalLayout::EnumFixedBits { .. } => Ok(Some(false)),
                NominalLayout::EnumAggregate(variants) => evaluate_not_sized_aggregate(
                    world,
                    variants.iter().flat_map(|fields| fields.iter()),
                    depth + 1,
                    visiting,
                    fuel,
                ),
            };
            visiting.pop();
            result
        }
        _ => Ok(None),
    }
}

fn charge(fuel: &mut usize, amount: usize) -> Result<(), LayoutError> {
    *fuel = fuel
        .checked_sub(amount)
        .ok_or(LayoutError::BudgetExhausted)?;
    Ok(())
}

pub fn align_up(value: u64, align: u64) -> Result<u64, LayoutError> {
    debug_assert!(align >= 1 && align.is_power_of_two());
    let remainder = value % align;
    if remainder == 0 {
        Ok(value)
    } else {
        value
            .checked_add(align - remainder)
            .ok_or(LayoutError::Overflow)
    }
}

pub fn integer_layout(bits: u32) -> Result<TypeLayout, LayoutError> {
    if bits == 0 {
        return Ok(TypeLayout { size: 0, align: 1 });
    }
    let raw_bytes = u64::from(bits).div_ceil(8);
    let align = raw_bytes
        .checked_next_power_of_two()
        .unwrap_or(16)
        .min(16)
        .max(1);
    let size = align_up(raw_bytes, align)?;
    Ok(TypeLayout { size, align })
}

fn int_layout(bits: u32, ty: &Term) -> Result<TypeLayout, LayoutError> {
    if bits > MAX_BIT_WIDTH {
        return Err(LayoutError::Unsupported(ty.clone()));
    }
    integer_layout(bits)
}

fn pointer_layout(world: &impl LayoutWorld, ty: &Term) -> Result<TypeLayout, LayoutError> {
    let target = world.target().ok_or(LayoutError::TargetRequired)?;
    int_layout(target.pointer_bits(), ty)
}

pub fn struct_layout(fields: &[TypeLayout]) -> Result<TypeLayout, LayoutError> {
    let mut offset = 0u64;
    let mut max_align = 1u64;
    for field in fields {
        offset = align_up(offset, field.align)?;
        offset = offset
            .checked_add(field.size)
            .ok_or(LayoutError::Overflow)?;
        max_align = max_align.max(field.align);
    }
    let size = align_up(offset, max_align)?;
    Ok(TypeLayout {
        size,
        align: max_align,
    })
}

pub fn union_layout(branches: &[TypeLayout]) -> Result<TypeLayout, LayoutError> {
    let mut max_size = 0u64;
    let mut max_align = 1u64;
    for branch in branches {
        max_size = max_size.max(branch.size);
        max_align = max_align.max(branch.align);
    }
    let size = align_up(max_size, max_align)?;
    Ok(TypeLayout {
        size,
        align: max_align,
    })
}

pub fn sum_tag_bits(branch_count: usize) -> Result<u32, LayoutError> {
    if branch_count <= 256 {
        Ok(8)
    } else if branch_count <= 65_536 {
        Ok(16)
    } else if u32::try_from(branch_count.saturating_sub(1)).is_ok() {
        Ok(32)
    } else {
        Err(LayoutError::Overflow)
    }
}

fn no_niche(layout: TypeLayout) -> LayoutWithNiche {
    LayoutWithNiche {
        layout,
        niche: None,
    }
}

fn tag_niche(branch_count: usize, tag_bits: u32) -> Option<Niche> {
    if tag_bits == 0 || tag_bits >= 64 {
        return None;
    }
    let space = 1u64 << tag_bits;
    let used = branch_count as u64;
    (used < space).then(|| Niche {
        path: vec![0],
        scalar: NicheScalar::Int { bits: tag_bits },
        start: used,
        count: space - used,
        needs_bool_widen: false,
    })
}

fn fields_layout_with_niche(
    world: &impl LayoutWorld,
    fields: &[Term],
    depth: usize,
    visiting: &mut Vec<Term>,
    fuel: &mut usize,
) -> Result<LayoutWithNiche, LayoutError> {
    charge(fuel, fields.len())?;
    let mut field_layouts = Vec::with_capacity(fields.len());
    let mut best: Option<(usize, Niche)> = None;
    for (index, field_ty) in fields.iter().enumerate() {
        let fw = layout_with_niche_at(world, field_ty, depth, visiting, fuel)?;
        field_layouts.push(fw.layout);
        if let Some(niche) = fw.niche
            && best
                .as_ref()
                .is_none_or(|(_, prev)| niche.count > prev.count)
        {
            best = Some((index, niche));
        }
    }
    let layout = struct_layout(&field_layouts)?;
    let niche = best.map(|(index, mut niche)| {
        niche.path.insert(0, index);
        niche
    });
    Ok(LayoutWithNiche { layout, niche })
}

fn variant_payload_layout_with_niche(
    world: &impl LayoutWorld,
    fields: &[Term],
    depth: usize,
    visiting: &mut Vec<Term>,
    fuel: &mut usize,
) -> Result<LayoutWithNiche, LayoutError> {
    if fields.len() == 1 {
        charge(fuel, 1)?;
        layout_with_niche_at(world, &fields[0], depth, visiting, fuel)
    } else {
        fields_layout_with_niche(world, fields, depth, visiting, fuel)
    }
}

fn layout_with_niche_at(
    world: &impl LayoutWorld,
    ty: &Term,
    depth: usize,
    visiting: &mut Vec<Term>,
    fuel: &mut usize,
) -> Result<LayoutWithNiche, LayoutError> {
    charge(fuel, 1)?;
    if depth >= MAX_LAYOUT_DEPTH {
        return Err(LayoutError::BudgetExhausted);
    }
    if world.is_template_identity(ty) {
        return Ok(no_niche(TypeLayout { size: 8, align: 8 }));
    }
    match ty {
        Term::Primitive(Primitive::Void) => Ok(no_niche(TypeLayout { size: 0, align: 1 })),
        Term::Primitive(Primitive::Bool) => Ok(LayoutWithNiche {
            layout: TypeLayout { size: 1, align: 1 },
            niche: Some(Niche {
                path: Vec::new(),
                scalar: NicheScalar::Int { bits: 8 },
                start: 2,
                count: 254,
                needs_bool_widen: true,
            }),
        }),
        Term::Primitive(Primitive::Char) => Ok(LayoutWithNiche {
            layout: TypeLayout { size: 4, align: 4 },
            niche: Some(Niche {
                path: Vec::new(),
                scalar: NicheScalar::Int { bits: 32 },
                start: 0x11_0000,
                count: u64::from(u32::MAX) - 0x11_0000 + 1,
                needs_bool_widen: false,
            }),
        }),
        Term::Primitive(Primitive::F16 | Primitive::Bf16) => {
            Ok(no_niche(TypeLayout { size: 2, align: 2 }))
        }
        Term::Primitive(Primitive::F32) => Ok(no_niche(TypeLayout { size: 4, align: 4 })),
        Term::Primitive(Primitive::F64) => Ok(no_niche(TypeLayout { size: 8, align: 8 })),
        Term::Primitive(Primitive::I8 | Primitive::U8) => {
            Ok(no_niche(TypeLayout { size: 1, align: 1 }))
        }
        Term::Primitive(Primitive::I16 | Primitive::U16) => {
            Ok(no_niche(TypeLayout { size: 2, align: 2 }))
        }
        Term::Primitive(Primitive::I32 | Primitive::U32) => {
            Ok(no_niche(TypeLayout { size: 4, align: 4 }))
        }
        Term::Primitive(Primitive::I64 | Primitive::U64) => {
            Ok(no_niche(TypeLayout { size: 8, align: 8 }))
        }
        Term::Primitive(Primitive::I128 | Primitive::U128) => Ok(no_niche(TypeLayout {
            size: 16,
            align: 16,
        })),
        Term::Primitive(Primitive::Isize | Primitive::Usize) => {
            Ok(no_niche(pointer_layout(world, ty)?))
        }
        Term::Primitive(Primitive::BitVec(width)) => {
            let bits = u32::try_from(*width).map_err(|_| LayoutError::Unsupported(ty.clone()))?;
            Ok(no_niche(int_layout(bits, ty)?))
        }
        Term::Primitive(Primitive::Str | Primitive::Symbol) => {
            let ptr = pointer_layout(world, ty)?;
            Ok(no_niche(struct_layout(&[
                ptr,
                TypeLayout { size: 8, align: 8 },
            ])?))
        }
        Term::Primitive(
            Primitive::Type
            | Primitive::Object
            | Primitive::Integer
            | Primitive::Float
            | Primitive::Unit,
        )
        | Term::EffectRowType
        | Term::ErrorRowType
        | Term::ReachabilityType
        | Term::AnyType => Err(LayoutError::ComptimeOnly(ty.clone())),
        Term::Primitive(Primitive::Never) => Err(LayoutError::Unsupported(ty.clone())),
        Term::Pointer(_) => Ok(LayoutWithNiche {
            layout: pointer_layout(world, ty)?,
            niche: Some(Niche {
                path: Vec::new(),
                scalar: NicheScalar::Pointer,
                start: 0,
                count: 1,
                needs_bool_widen: false,
            }),
        }),
        Term::TraitObject { .. } => {
            let ptr = pointer_layout(world, ty)?;
            Ok(no_niche(struct_layout(&[ptr, ptr])?))
        }
        Term::UnsafeFunction(signature) => {
            layout_with_niche_at(world, signature, depth + 1, visiting, fuel)
        }
        Term::Function { parameters, .. } => {
            if parameters.iter().any(|p| p.sort != Sort::Runtime) {
                return Err(LayoutError::ComptimeOnly(ty.clone()));
            }
            Ok(LayoutWithNiche {
                layout: pointer_layout(world, ty)?,
                niche: Some(Niche {
                    path: Vec::new(),
                    scalar: NicheScalar::Pointer,
                    start: 0,
                    count: 1,
                    needs_bool_widen: false,
                }),
            })
        }
        Term::ComptimeFunction(_) | Term::NormalFormFunction(..) | Term::Lift(_) => {
            Err(LayoutError::ComptimeOnly(ty.clone()))
        }
        Term::Optional(payload) => {
            if **payload == Term::Primitive(Primitive::Never) {
                return Ok(LayoutWithNiche {
                    layout: TypeLayout { size: 1, align: 1 },
                    niche: Some(Niche {
                        path: vec![0],
                        scalar: NicheScalar::Int { bits: 8 },
                        start: 2,
                        count: 254,
                        needs_bool_widen: true,
                    }),
                });
            }
            let inner = layout_with_niche_at(world, payload, depth + 1, visiting, fuel)?;
            if let Some(mut niche) = inner.niche
                && niche.count >= 1
            {
                let remaining = if niche.count > 1 {
                    niche.start = niche.start.checked_add(1).ok_or(LayoutError::Overflow)?;
                    niche.count -= 1;
                    niche.needs_bool_widen = false;
                    Some(niche)
                } else {
                    None
                };
                Ok(LayoutWithNiche {
                    layout: inner.layout,
                    niche: remaining,
                })
            } else {
                let layout = struct_layout(&[TypeLayout { size: 1, align: 1 }, inner.layout])?;
                Ok(LayoutWithNiche {
                    layout,
                    niche: Some(Niche {
                        path: vec![0],
                        scalar: NicheScalar::Int { bits: 8 },
                        start: 2,
                        count: 254,
                        needs_bool_widen: true,
                    }),
                })
            }
        }
        Term::ErrorResult { errors, result } => {
            charge(fuel, errors.len())?;
            let success_layout = if **result == Term::Primitive(Primitive::Never) {
                TypeLayout { size: 0, align: 1 }
            } else {
                layout_with_niche_at(world, result, depth + 1, visiting, fuel)?.layout
            };
            let mut branches = Vec::with_capacity(errors.len() + 1);
            branches.push(success_layout);
            for err in errors {
                branches.push(layout_with_niche_at(world, err, depth + 1, visiting, fuel)?.layout);
            }
            let tag = integer_layout(sum_tag_bits(branches.len())?)?;
            let payload = union_layout(&branches)?;
            Ok(no_niche(struct_layout(&[tag, payload])?))
        }
        Term::Tuple(fields) => tuple_fields_layout_with_niche(world, fields, depth, visiting, fuel),
        Term::Record(fields) => {
            let field_types: Vec<Term> = fields.iter().map(|(_, t)| t.clone()).collect();
            fields_layout_with_niche(world, &field_types, depth + 1, visiting, fuel)
        }
        Term::NormalForm {
            head,
            sort: Sort::Type,
            arguments,
        } => {
            if world.is_tuple_head(*head) {
                if let Some(elements) =
                    crate::normal_forms::type_list_fields_with_layouts(arguments, &|id| {
                        world.is_list_head(id)
                    })
                {
                    return tuple_fields_layout_with_niche(world, elements, depth, visiting, fuel);
                }
                return Err(LayoutError::Unresolved(ty.clone()));
            }
            if world.is_list_head(*head) {
                return Err(LayoutError::ComptimeOnly(ty.clone()));
            }
            Err(LayoutError::Unsupported(ty.clone()))
        }
        Term::Nominal { .. } => {
            if visiting.contains(ty) {
                return Err(LayoutError::Cycle(ty.clone()));
            }
            let Some(definition) = world.nominal_layout(ty, fuel)? else {
                return Err(LayoutError::Unresolved(ty.clone()));
            };
            visiting.push(ty.clone());
            let result = match definition {
                NominalLayout::Struct(fields) => {
                    fields_layout_with_niche(world, &fields, depth + 1, visiting, fuel)
                }
                NominalLayout::PackedStruct(fields) => {
                    charge(fuel, fields.len())?;
                    let layouts = fields
                        .iter()
                        .map(|field_ty| {
                            layout_with_niche_at(world, field_ty, depth + 1, visiting, fuel)
                                .map(|r| r.layout)
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(no_niche(packed_struct_layout(&layouts)?))
                }
                NominalLayout::BitPackedStruct(fields) => {
                    let bp = bit_packed_struct_layout_at(world, &fields, depth + 1, visiting, fuel)
                        .map_err(|err| match err {
                            BitPackedError::Layout(layout_err) => layout_err,
                            BitPackedError::UnsupportedField { .. } => {
                                LayoutError::Unsupported(ty.clone())
                            }
                            BitPackedError::WidthOverflow => LayoutError::Overflow,
                        })?;
                    Ok(no_niche(int_layout(bp.total_bits, ty)?))
                }
                NominalLayout::Newtype(representation) => {
                    layout_with_niche_at(world, &representation, depth + 1, visiting, fuel)
                }
                NominalLayout::EnumUnit(variant_count) => {
                    let bits = unit_enum_tag_bits(variant_count)?;
                    let layout = integer_layout(bits)?;
                    let niche = if bits > 0 && (variant_count as u64) < (1u64 << bits) {
                        Some(Niche {
                            path: Vec::new(),
                            scalar: NicheScalar::Int { bits },
                            start: variant_count as u64,
                            count: (1u64 << bits) - (variant_count as u64),
                            needs_bool_widen: false,
                        })
                    } else {
                        None
                    };
                    Ok(LayoutWithNiche { layout, niche })
                }
                NominalLayout::EnumSymbol => {
                    let ptr = pointer_layout(world, ty)?;
                    Ok(no_niche(struct_layout(&[
                        ptr,
                        TypeLayout { size: 8, align: 8 },
                    ])?))
                }
                NominalLayout::EnumFixedBits {
                    width,
                    niche,
                    exhaustive: _,
                } => {
                    let layout = int_layout(width, ty)?;
                    let niche = niche.filter(|_| matches!(width, 8 | 16 | 32 | 64)).map(
                        |(start, count)| Niche {
                            path: Vec::new(),
                            scalar: NicheScalar::Int { bits: width },
                            start,
                            count,
                            needs_bool_widen: false,
                        },
                    );
                    Ok(LayoutWithNiche { layout, niche })
                }
                NominalLayout::EnumAggregate(variants) => {
                    if variants.is_empty() {
                        Ok(no_niche(TypeLayout { size: 0, align: 1 }))
                    } else if variants.len() == 1 {
                        variant_payload_layout_with_niche(
                            world,
                            &variants[0],
                            depth + 1,
                            visiting,
                            fuel,
                        )
                    } else {
                        charge(fuel, variants.len())?;
                        let mut non_empty = variants
                            .iter()
                            .enumerate()
                            .filter(|(_, fields)| !fields.is_empty());
                        let single_data = match (non_empty.next(), non_empty.next()) {
                            (Some((idx, _)), None) => Some(idx),
                            _ => None,
                        };
                        if let Some(data_idx) = single_data {
                            let empty_count = (variants.len() - 1) as u64;
                            let data = variant_payload_layout_with_niche(
                                world,
                                &variants[data_idx],
                                depth + 1,
                                visiting,
                                fuel,
                            )?;
                            if let Some(mut niche) = data.niche
                                && niche.count >= empty_count
                            {
                                let remaining = if niche.count > empty_count {
                                    niche.start = niche
                                        .start
                                        .checked_add(empty_count)
                                        .ok_or(LayoutError::Overflow)?;
                                    niche.count -= empty_count;
                                    niche.needs_bool_widen = false;
                                    Some(niche)
                                } else {
                                    None
                                };
                                return {
                                    visiting.pop();
                                    Ok(LayoutWithNiche {
                                        layout: data.layout,
                                        niche: remaining,
                                    })
                                };
                            }
                            let tag_bits = sum_tag_bits(variants.len())?;
                            let tag = integer_layout(tag_bits)?;
                            let payload = union_layout(&[data.layout])?;
                            let layout = struct_layout(&[tag, payload])?;
                            let niche = tag_niche(variants.len(), tag_bits);
                            Ok(LayoutWithNiche { layout, niche })
                        } else {
                            let mut variant_layouts = Vec::with_capacity(variants.len());
                            for variant_fields in variants {
                                charge(fuel, variant_fields.len())?;
                                let v_layouts = variant_fields
                                    .iter()
                                    .map(|f_ty| {
                                        layout_with_niche_at(world, f_ty, depth + 1, visiting, fuel)
                                            .map(|r| r.layout)
                                    })
                                    .collect::<Result<Vec<_>, _>>()?;
                                variant_layouts.push(struct_layout(&v_layouts)?);
                            }
                            let tag_bits = sum_tag_bits(variant_layouts.len())?;
                            let tag = integer_layout(tag_bits)?;
                            let payload = union_layout(&variant_layouts)?;
                            let layout = struct_layout(&[tag, payload])?;
                            let niche = tag_niche(variant_layouts.len(), tag_bits);
                            Ok(LayoutWithNiche { layout, niche })
                        }
                    }
                }
            };
            visiting.pop();
            result
        }
        Term::Closure {
            environment,
            signature,
        } => {
            let body = if matches!(signature.as_ref(), Term::TraitObject { .. }) {
                let ptr = pointer_layout(world, ty)?;
                struct_layout(&[ptr, ptr])?
            } else {
                if !matches!(
                    signature.callable_signature(),
                    Term::Function { parameters, .. } if parameters.iter().all(|p| p.sort == Sort::Runtime)
                ) {
                    return Err(LayoutError::ComptimeOnly(ty.clone()));
                }
                pointer_layout(world, ty)?
            };
            charge(fuel, environment.len())?;
            let env_layouts = environment
                .iter()
                .map(|field_ty| {
                    layout_with_niche_at(world, field_ty, depth + 1, visiting, fuel)
                        .map(|r| r.layout)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let env = struct_layout(&env_layouts)?;
            let state_count = closure_env_state_count(world, environment, depth + 1, fuel)?;
            Ok(no_niche(struct_layout(&[
                body,
                env,
                TypeLayout {
                    size: u64::try_from(state_count).map_err(|_| LayoutError::Overflow)?,
                    align: 1,
                },
            ])?))
        }
        Term::Rigid { .. }
        | Term::Bound { .. }
        | Term::Hole { .. }
        | Term::AssociatedType { .. }
        | Term::ContextualClosure { .. }
        | Term::ClosureContextValue(_) => Err(LayoutError::Unresolved(ty.clone())),
        Term::ClosureContextType => Err(LayoutError::ComptimeOnly(ty.clone())),
        _ => Err(LayoutError::Unsupported(ty.clone())),
    }
}

fn tuple_fields_layout_with_niche(
    world: &impl LayoutWorld,
    fields: &[Term],
    depth: usize,
    visiting: &mut Vec<Term>,
    fuel: &mut usize,
) -> Result<LayoutWithNiche, LayoutError> {
    if fields.is_empty() {
        return Ok(no_niche(TypeLayout { size: 0, align: 1 }));
    }
    fields_layout_with_niche(world, fields, depth + 1, visiting, fuel)
}

pub fn packed_struct_layout(fields: &[TypeLayout]) -> Result<TypeLayout, LayoutError> {
    let mut size = 0u64;
    for field in fields {
        size = size.checked_add(field.size).ok_or(LayoutError::Overflow)?;
    }
    Ok(TypeLayout { size, align: 1 })
}

/// Compute `(bit_width, restricted)` for a field allowed inside an
/// `attrs.packed.bit` struct, recursing through transparent `newtype`s and
/// nested `attrs.packed.bit` structs without byte padding.
pub fn bit_packed_field_info(
    world: &impl LayoutWorld,
    ty: &Term,
    fuel: &mut usize,
) -> Result<(u32, bool), BitPackedError> {
    let mut visiting = Vec::new();
    bit_packed_field_info_at(world, ty, 0, &mut visiting, fuel)
}

/// Compute the LSB-first bit layout of an `attrs.packed.bit` struct's fields.
pub fn bit_packed_struct_layout(
    world: &impl LayoutWorld,
    fields: &[Term],
    fuel: &mut usize,
) -> Result<BitPackedStructLayout, BitPackedError> {
    let mut visiting = Vec::new();
    bit_packed_struct_layout_at(world, fields, 0, &mut visiting, fuel)
}

fn bit_packed_field_info_at(
    world: &impl LayoutWorld,
    ty: &Term,
    depth: usize,
    visiting: &mut Vec<Term>,
    fuel: &mut usize,
) -> Result<(u32, bool), BitPackedError> {
    charge(fuel, 1)?;
    if depth >= MAX_LAYOUT_DEPTH {
        return Err(BitPackedError::Layout(LayoutError::BudgetExhausted));
    }
    let unsupported = |reason: &'static str| BitPackedError::UnsupportedField {
        field_index: 0,
        ty: ty.clone(),
        reason,
    };
    match ty {
        Term::Primitive(Primitive::Void) => Ok((0, false)),
        Term::Primitive(Primitive::Bool) => Ok((1, false)),
        Term::Primitive(Primitive::BitVec(width)) => {
            let bits = u32::try_from(*width).map_err(|_| BitPackedError::WidthOverflow)?;
            if bits > MAX_BIT_WIDTH {
                return Err(BitPackedError::WidthOverflow);
            }
            Ok((bits, false))
        }
        Term::Primitive(Primitive::F16 | Primitive::Bf16) => Ok((16, false)),
        Term::Primitive(Primitive::F32) => Ok((32, false)),
        Term::Primitive(Primitive::F64) => Ok((64, false)),
        Term::Primitive(Primitive::Char) => Ok((32, true)),
        Term::Primitive(Primitive::I8 | Primitive::U8) => Ok((8, false)),
        Term::Primitive(Primitive::I16 | Primitive::U16) => Ok((16, false)),
        Term::Primitive(Primitive::I32 | Primitive::U32) => Ok((32, false)),
        Term::Primitive(Primitive::I64 | Primitive::U64) => Ok((64, false)),
        Term::Primitive(Primitive::I128 | Primitive::U128) => Ok((128, false)),
        Term::Primitive(Primitive::Isize | Primitive::Usize) => {
            let bits = world
                .target()
                .ok_or(BitPackedError::Layout(LayoutError::TargetRequired))?
                .pointer_bits();
            if bits > MAX_BIT_WIDTH {
                return Err(BitPackedError::WidthOverflow);
            }
            Ok((bits, false))
        }
        Term::Primitive(Primitive::Str | Primitive::Symbol) => Err(unsupported(
            "str and Symbol are not allowed in an attrs.packed.bit struct",
        )),
        Term::Primitive(Primitive::Never) => Err(unsupported(
            "Never is not allowed in an attrs.packed.bit struct",
        )),
        Term::Primitive(
            Primitive::Type
            | Primitive::Object
            | Primitive::Integer
            | Primitive::Float
            | Primitive::Unit,
        ) => Err(BitPackedError::Layout(LayoutError::ComptimeOnly(
            ty.clone(),
        ))),
        Term::Pointer(_)
        | Term::TraitObject { .. }
        | Term::Function { .. }
        | Term::UnsafeFunction(_)
        | Term::Closure { .. } => Err(unsupported(
            "pointer, trait-object, and callable types are not allowed in an attrs.packed.bit struct",
        )),
        Term::Optional(_) | Term::ErrorResult { .. } | Term::Computation { .. } => {
            Err(unsupported(
                "optional, error-result, and effect types are not allowed in an attrs.packed.bit struct",
            ))
        }
        Term::Tuple(_) | Term::Record(_) => Err(unsupported(
            "tuple and structural record types are not allowed in an attrs.packed.bit struct",
        )),
        Term::NormalForm {
            head,
            sort: Sort::Type,
            ..
        } => {
            if world.is_list_head(*head) {
                Err(BitPackedError::Layout(LayoutError::ComptimeOnly(
                    ty.clone(),
                )))
            } else if world.is_tuple_head(*head) {
                Err(unsupported(
                    "tuple and structural record types are not allowed in an attrs.packed.bit struct",
                ))
            } else {
                Err(unsupported(
                    "opaque normal-form types are not allowed in an attrs.packed.bit struct",
                ))
            }
        }
        Term::NormalForm { .. }
        | Term::ComptimeFunction(_)
        | Term::NormalFormFunction(..)
        | Term::Lift(_) => Err(BitPackedError::Layout(LayoutError::ComptimeOnly(
            ty.clone(),
        ))),
        Term::Nominal { .. } => {
            if visiting.contains(ty) {
                return Err(BitPackedError::Layout(LayoutError::Cycle(ty.clone())));
            }
            let Some(definition) = world.nominal_layout(ty, fuel)? else {
                return Err(BitPackedError::Layout(LayoutError::Unresolved(ty.clone())));
            };
            visiting.push(ty.clone());
            let result = match definition {
                NominalLayout::BitPackedStruct(inner_fields) => {
                    let inner = bit_packed_struct_layout_at(
                        world,
                        &inner_fields,
                        depth + 1,
                        visiting,
                        fuel,
                    )?;
                    Ok((inner.total_bits, inner.restricted))
                }
                NominalLayout::Newtype(repr) => {
                    bit_packed_field_info_at(world, &repr, depth + 1, visiting, fuel)
                }
                NominalLayout::EnumFixedBits {
                    width, exhaustive, ..
                } => {
                    if width > MAX_BIT_WIDTH {
                        Err(BitPackedError::WidthOverflow)
                    } else {
                        Ok((width, !exhaustive))
                    }
                }
                NominalLayout::Struct(_) => Err(unsupported(
                    "ordinary struct is not allowed in an attrs.packed.bit struct",
                )),
                NominalLayout::PackedStruct(_) => Err(unsupported(
                    "attrs.packed.byte struct is not allowed in an attrs.packed.bit struct",
                )),
                NominalLayout::EnumUnit(_)
                | NominalLayout::EnumSymbol
                | NominalLayout::EnumAggregate(_) => Err(unsupported(
                    "only fixed-width bit-encoded enums are allowed in an attrs.packed.bit struct",
                )),
            };
            visiting.pop();
            result
        }
        Term::Rigid { .. }
        | Term::Bound { .. }
        | Term::Hole { .. }
        | Term::AssociatedType { .. }
        | Term::ContextualClosure { .. }
        | Term::ClosureContextValue(_) => {
            Err(BitPackedError::Layout(LayoutError::Unresolved(ty.clone())))
        }
        Term::ClosureContextType
        | Term::EffectRowType
        | Term::ErrorRowType
        | Term::ReachabilityType
        | Term::AnyType => Err(BitPackedError::Layout(LayoutError::ComptimeOnly(
            ty.clone(),
        ))),
        Term::Runtime(_) | Term::RuntimeField { .. } => Err(unsupported(
            "runtime-dependent types are not allowed in an attrs.packed.bit struct",
        )),
        _ => Err(BitPackedError::Layout(LayoutError::Unsupported(ty.clone()))),
    }
}

fn bit_packed_struct_layout_at(
    world: &impl LayoutWorld,
    fields: &[Term],
    depth: usize,
    visiting: &mut Vec<Term>,
    fuel: &mut usize,
) -> Result<BitPackedStructLayout, BitPackedError> {
    charge(fuel, fields.len())?;
    let mut bit_fields = Vec::with_capacity(fields.len());
    let mut total_bits = 0u32;
    let mut restricted = false;
    for (field_index, field_ty) in fields.iter().enumerate() {
        let (bit_width, field_restricted) =
            bit_packed_field_info_at(world, field_ty, depth, visiting, fuel).map_err(|err| {
                match err {
                    BitPackedError::UnsupportedField { ty, reason, .. } => {
                        BitPackedError::UnsupportedField {
                            field_index,
                            ty,
                            reason,
                        }
                    }
                    other => other,
                }
            })?;
        let next_total = total_bits
            .checked_add(bit_width)
            .filter(|&w| w <= MAX_BIT_WIDTH)
            .ok_or(BitPackedError::WidthOverflow)?;
        bit_fields.push(BitPackedFieldLayout {
            bit_offset: total_bits,
            bit_width,
            restricted: field_restricted,
        });
        total_bits = next_total;
        restricted |= field_restricted;
    }
    Ok(BitPackedStructLayout {
        total_bits,
        fields: bit_fields,
        restricted,
    })
}

fn closure_env_state_count(
    world: &impl LayoutWorld,
    fields: &[Term],
    depth: usize,
    fuel: &mut usize,
) -> Result<usize, LayoutError> {
    let mut total = 1usize;
    for field in fields {
        let contribution = closure_field_state_count(world, field, depth + 1, fuel)?;
        total = total
            .checked_add(contribution)
            .ok_or(LayoutError::Overflow)?;
    }
    Ok(total)
}

fn closure_field_state_count(
    world: &impl LayoutWorld,
    ty: &Term,
    depth: usize,
    fuel: &mut usize,
) -> Result<usize, LayoutError> {
    charge(fuel, 1)?;
    if depth >= MAX_LAYOUT_DEPTH {
        return Err(LayoutError::BudgetExhausted);
    }
    if world.is_template_identity(ty) {
        return Ok(1);
    }
    match ty {
        Term::Record(fields) => {
            let inner: Vec<Term> = fields.iter().map(|(_, t)| t.clone()).collect();
            closure_env_state_count(world, &inner, depth + 1, fuel)
        }
        Term::Tuple(fields) if !fields.is_empty() => {
            closure_env_state_count(world, fields, depth + 1, fuel)
        }
        Term::NormalForm {
            head,
            sort: Sort::Type,
            arguments,
        } if world.is_tuple_head(*head) => {
            if let Some(elements) =
                crate::normal_forms::type_list_fields_with_layouts(arguments, &|id| {
                    world.is_list_head(id)
                })
            {
                if elements.is_empty() {
                    Ok(1)
                } else {
                    closure_env_state_count(world, elements, depth + 1, fuel)
                }
            } else {
                Err(LayoutError::Unresolved(ty.clone()))
            }
        }
        Term::Nominal { .. } => match world.nominal_layout(ty, fuel)? {
            Some(NominalLayout::Struct(fields)) => {
                closure_env_state_count(world, &fields, depth + 1, fuel)
            }
            Some(NominalLayout::PackedStruct(fields)) => {
                closure_env_state_count(world, &fields, depth + 1, fuel)
            }
            Some(NominalLayout::BitPackedStruct(_)) => Ok(1),
            Some(NominalLayout::Newtype(repr)) => {
                closure_field_state_count(world, &repr, depth + 1, fuel)
            }
            Some(
                NominalLayout::EnumUnit(_)
                | NominalLayout::EnumSymbol
                | NominalLayout::EnumFixedBits { .. }
                | NominalLayout::EnumAggregate(_),
            ) => Ok(1),
            None => Err(LayoutError::Unresolved(ty.clone())),
        },
        Term::Closure { environment, .. } => {
            closure_env_state_count(world, environment, depth + 1, fuel)
        }
        Term::UnsafeFunction(inner) => closure_field_state_count(world, inner, depth + 1, fuel),
        _ => Ok(1),
    }
}

/// [`LayoutWorld`] backed by an elaborated NIR [`Package`](crate::program::Package).
pub struct PackageLayoutWorld<'a>(pub &'a crate::program::Package);

impl LayoutWorld for PackageLayoutWorld<'_> {
    fn target(&self) -> Option<crate::target::TargetData> {
        self.0.target
    }

    fn is_tuple_head(&self, head: DeclarationId) -> bool {
        self.0
            .normal_forms
            .iter()
            .any(|nf| nf.declaration == head && nf.role == crate::normal_forms::Role::Tuple)
    }

    fn is_list_head(&self, head: DeclarationId) -> bool {
        self.0
            .normal_forms
            .iter()
            .any(|nf| nf.declaration == head && nf.role == crate::normal_forms::Role::TypeList)
    }

    fn nominal_kind(&self, ty: &Term) -> Option<crate::NominalKind> {
        let Term::Nominal { declaration, .. } = ty else {
            return None;
        };
        if self.0.structs.iter().any(|s| s.declaration == *declaration) {
            Some(crate::NominalKind::Struct)
        } else if self.0.enums.iter().any(|e| e.declaration == *declaration) {
            Some(crate::NominalKind::Enum)
        } else if self
            .0
            .newtypes
            .iter()
            .any(|n| n.declaration == *declaration)
        {
            Some(crate::NominalKind::Newtype)
        } else {
            None
        }
    }

    fn nominal_layout(
        &self,
        ty: &Term,
        fuel: &mut usize,
    ) -> Result<Option<NominalLayout>, LayoutError> {
        let Term::Nominal {
            declaration,
            arguments,
        } = ty
        else {
            return Ok(None);
        };
        let admission_err = |err: crate::NominalAdmissionError| match err {
            crate::NominalAdmissionError::Budget => LayoutError::BudgetExhausted,
            crate::NominalAdmissionError::Unresolved(_) => LayoutError::Unresolved(ty.clone()),
            crate::NominalAdmissionError::Invalid(_) => LayoutError::Unsupported(ty.clone()),
        };
        if let Some(def) = self
            .0
            .structs
            .iter()
            .find(|s| s.declaration == *declaration)
        {
            let fields = crate::normal_forms::instantiate_value_fields_checked(
                &def.type_parameters,
                &def.parameter_types,
                arguments,
                def.fields.iter().map(|(_, t)| t),
                fuel,
            )
            .map_err(admission_err)?;
            return Ok(Some(match def.packed {
                crate::program::PackedMode::None => NominalLayout::Struct(fields),
                crate::program::PackedMode::Byte => NominalLayout::PackedStruct(fields),
                crate::program::PackedMode::Bit => NominalLayout::BitPackedStruct(fields),
            }));
        }
        if let Some(def) = self
            .0
            .newtypes
            .iter()
            .find(|n| n.declaration == *declaration)
        {
            let repr =
                def.representation_budgeted(arguments, &[], fuel)
                    .map_err(|err| match err {
                        crate::newtypes::ConversionError::Budget => LayoutError::BudgetExhausted,
                        _ => LayoutError::Unsupported(ty.clone()),
                    })?;
            return Ok(Some(NominalLayout::Newtype(repr)));
        }
        if let Some(def) = self.0.enums.iter().find(|e| e.declaration == *declaration) {
            if def.symbol_encoding.is_some() {
                return Ok(Some(NominalLayout::EnumSymbol));
            }
            if let Some(encoding) = &def.encoding {
                let bits = u32::try_from(encoding.width())
                    .map_err(|_| LayoutError::Unsupported(ty.clone()))?;
                return Ok(Some(NominalLayout::EnumFixedBits {
                    width: bits,
                    niche: encoding.layout.contiguous_niche(),
                    exhaustive: encoding.layout.is_exhaustive(),
                }));
            }
            if def.variants.iter().all(|v| v.fields.is_empty()) {
                return Ok(Some(NominalLayout::EnumUnit(def.variants.len())));
            }
            let mut variants = Vec::with_capacity(def.variants.len());
            for variant in &def.variants {
                let fields = crate::normal_forms::instantiate_value_fields_checked(
                    &def.type_parameters,
                    &def.parameter_types,
                    arguments,
                    variant.fields.iter(),
                    fuel,
                )
                .map_err(admission_err)?;
                variants.push(fields);
            }
            return Ok(Some(NominalLayout::EnumAggregate(variants)));
        }
        Ok(None)
    }

    fn nominal_field_index(
        &self,
        ty: &Term,
        field: &str,
        fuel: &mut usize,
    ) -> Result<FieldLookup, LayoutError> {
        charge(fuel, 1)?;
        let Term::Nominal { declaration, .. } = ty else {
            return Ok(FieldLookup::NotFound);
        };
        if let Some(def) = self
            .0
            .structs
            .iter()
            .find(|s| s.declaration == *declaration)
        {
            return Ok(def
                .fields
                .iter()
                .position(|(name, _)| name == field)
                .map_or(FieldLookup::NotFound, FieldLookup::Index));
        }
        Ok(FieldLookup::NotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::TargetData;

    struct TestWorldVec {
        target: Option<TargetData>,
        tuple_head: Option<DeclarationId>,
        list_head: Option<DeclarationId>,
        nominals: Vec<(Term, NominalLayout)>,
        field_names: Vec<(Term, Vec<String>)>,
    }

    impl TestWorldVec {
        fn new(target: Option<TargetData>) -> Self {
            Self {
                target,
                tuple_head: None,
                list_head: None,
                nominals: Vec::new(),
                field_names: Vec::new(),
            }
        }
    }

    impl LayoutWorld for TestWorldVec {
        fn target(&self) -> Option<TargetData> {
            self.target
        }

        fn is_tuple_head(&self, head: DeclarationId) -> bool {
            self.tuple_head == Some(head)
        }

        fn is_list_head(&self, head: DeclarationId) -> bool {
            self.list_head == Some(head)
        }

        fn nominal_kind(&self, ty: &Term) -> Option<crate::NominalKind> {
            let (_, descriptor) = self.nominals.iter().find(|(key, _)| key == ty)?;
            Some(match descriptor {
                NominalLayout::Struct(_)
                | NominalLayout::PackedStruct(_)
                | NominalLayout::BitPackedStruct(_) => crate::NominalKind::Struct,
                NominalLayout::Newtype(_) => crate::NominalKind::Newtype,
                _ => crate::NominalKind::Enum,
            })
        }

        fn nominal_layout(
            &self,
            ty: &Term,
            _fuel: &mut usize,
        ) -> Result<Option<NominalLayout>, LayoutError> {
            Ok(self
                .nominals
                .iter()
                .find(|(key, _)| key == ty)
                .map(|(_, def)| def.clone()))
        }

        fn nominal_field_index(
            &self,
            ty: &Term,
            field: &str,
            _fuel: &mut usize,
        ) -> Result<FieldLookup, LayoutError> {
            Ok(match self.field_names.iter().find(|(key, _)| key == ty) {
                Some((_, names)) => names
                    .iter()
                    .position(|name| name == field)
                    .map_or(FieldLookup::NotFound, FieldLookup::Index),
                None => FieldLookup::NotFound,
            })
        }
    }

    fn id(local: u32) -> DeclarationId {
        DeclarationId { package: 1, local }
    }

    #[test]
    fn primitives_bitvecs_and_comptime_domains_match_llvm() {
        let world64 = TestWorldVec::new(Some(TargetData::new(64).unwrap()));
        let world32 = TestWorldVec::new(Some(TargetData::new(32).unwrap()));
        let mut fuel = 10_000;

        let cases = [
            (Primitive::Void, (0, 1)),
            (Primitive::Bool, (1, 1)),
            (Primitive::I8, (1, 1)),
            (Primitive::U8, (1, 1)),
            (Primitive::I16, (2, 2)),
            (Primitive::U16, (2, 2)),
            (Primitive::F16, (2, 2)),
            (Primitive::Bf16, (2, 2)),
            (Primitive::I32, (4, 4)),
            (Primitive::U32, (4, 4)),
            (Primitive::F32, (4, 4)),
            (Primitive::Char, (4, 4)),
            (Primitive::I64, (8, 8)),
            (Primitive::U64, (8, 8)),
            (Primitive::F64, (8, 8)),
            (Primitive::I128, (16, 16)),
            (Primitive::U128, (16, 16)),
            (Primitive::BitVec(0), (0, 1)),
            (Primitive::BitVec(1), (1, 1)),
            (Primitive::BitVec(9), (2, 2)),
            (Primitive::BitVec(24), (4, 4)),
            (Primitive::BitVec(48), (8, 8)),
            (Primitive::BitVec(96), (16, 16)),
            (Primitive::BitVec(129), (32, 16)),
            (Primitive::BitVec(256), (32, 16)),
            (Primitive::Str, (16, 8)),
            (Primitive::Symbol, (16, 8)),
        ];
        for (prim, (size, align)) in cases {
            assert_eq!(
                compute_layout(&world64, &Term::Primitive(prim), &mut fuel),
                Ok(TypeLayout { size, align }),
                "failed for {prim}"
            );
        }

        assert_eq!(
            compute_layout(&world64, &Term::Primitive(Primitive::Usize), &mut fuel),
            Ok(TypeLayout { size: 8, align: 8 })
        );
        assert_eq!(
            compute_layout(&world32, &Term::Primitive(Primitive::Usize), &mut fuel),
            Ok(TypeLayout { size: 4, align: 4 })
        );

        let no_target = TestWorldVec::new(None);
        assert_eq!(
            compute_layout(&no_target, &Term::Primitive(Primitive::Usize), &mut fuel),
            Err(LayoutError::TargetRequired)
        );
        assert_eq!(
            compute_layout(&no_target, &Term::Primitive(Primitive::U32), &mut fuel),
            Ok(TypeLayout { size: 4, align: 4 })
        );

        for ct in [
            Primitive::Type,
            Primitive::Object,
            Primitive::Integer,
            Primitive::Float,
            Primitive::Unit,
        ] {
            assert!(matches!(
                compute_layout(&world64, &Term::Primitive(ct), &mut fuel),
                Err(LayoutError::ComptimeOnly(_))
            ));
        }
    }

    #[test]
    fn aggregates_recursive_pointers_and_cycles_behave_accurately() {
        let mut world = TestWorldVec::new(Some(TargetData::new(64).unwrap()));
        let mut fuel = 10_000;

        let opt_u64 = Term::Optional(Box::new(Term::Primitive(Primitive::U64)));
        assert_eq!(
            compute_layout(&world, &opt_u64, &mut fuel),
            Ok(TypeLayout { size: 16, align: 8 })
        );
        let opt_never = Term::Optional(Box::new(Term::Primitive(Primitive::Never)));
        assert_eq!(
            compute_layout(&world, &opt_never, &mut fuel),
            Ok(TypeLayout { size: 1, align: 1 })
        );

        let err_ty = Term::ErrorResult {
            errors: vec![Term::Primitive(Primitive::U8)],
            result: Box::new(Term::Primitive(Primitive::U64)),
        };
        assert_eq!(
            compute_layout(&world, &err_ty, &mut fuel),
            Ok(TypeLayout { size: 16, align: 8 })
        );

        // struct Link { value: u32, next: ?*Link }
        let link = Term::Nominal {
            declaration: id(1),
            arguments: vec![],
        };
        world.nominals.push((
            link.clone(),
            NominalLayout::Struct(vec![
                Term::Primitive(Primitive::U32),
                Term::Optional(Box::new(Term::Pointer(Box::new(link.clone())))),
            ]),
        ));
        assert_eq!(
            compute_layout(&world, &Term::Pointer(Box::new(link.clone())), &mut fuel),
            Ok(TypeLayout { size: 8, align: 8 })
        );
        assert_eq!(
            compute_layout(&world, &link, &mut fuel),
            Ok(TypeLayout { size: 16, align: 8 })
        );
        assert!(matches!(
            compute_layout(
                &world,
                &Term::Lift(Box::new(Term::Pointer(Box::new(Term::Primitive(
                    Primitive::U32
                ))))),
                &mut fuel,
            ),
            Err(LayoutError::ComptimeOnly(_))
        ));

        // Mutual by-value cycle A -> B -> A must fail cleanly.
        let a = Term::Nominal {
            declaration: id(2),
            arguments: vec![],
        };
        let b = Term::Nominal {
            declaration: id(3),
            arguments: vec![],
        };
        world
            .nominals
            .push((a.clone(), NominalLayout::Struct(vec![b.clone()])));
        world
            .nominals
            .push((b.clone(), NominalLayout::Struct(vec![a.clone()])));
        assert!(matches!(
            compute_layout(&world, &a, &mut fuel),
            Err(LayoutError::Cycle(_))
        ));

        // Direct self cycle must fail cleanly.
        let invalid = Term::Nominal {
            declaration: id(4),
            arguments: vec![],
        };
        world.nominals.push((
            invalid.clone(),
            NominalLayout::Struct(vec![invalid.clone()]),
        ));
        assert!(matches!(
            compute_layout(&world, &invalid, &mut fuel),
            Err(LayoutError::Cycle(_))
        ));

        // Exhausted budget fails without computing a bogus size.
        let mut tiny = 0usize;
        assert_eq!(
            compute_layout(&world, &Term::Primitive(Primitive::U32), &mut tiny),
            Err(LayoutError::BudgetExhausted)
        );
    }

    #[test]
    fn error_result_uses_tagged_union_sum_layout() {
        let mut world = TestWorldVec::new(Some(TargetData::new(64).unwrap()));
        let mut fuel = 10_000;

        let packed_9b = Term::Nominal {
            declaration: id(10),
            arguments: vec![],
        };
        world.nominals.push((
            packed_9b.clone(),
            NominalLayout::PackedStruct(vec![
                Term::Primitive(Primitive::U64),
                Term::Primitive(Primitive::U8),
            ]),
        ));

        let empty_struct = Term::Nominal {
            declaration: id(11),
            arguments: vec![],
        };
        world
            .nominals
            .push((empty_struct.clone(), NominalLayout::Struct(vec![])));

        assert_eq!(
            compute_layout(
                &world,
                &Term::ErrorResult {
                    errors: vec![Term::Primitive(Primitive::U8)],
                    result: Box::new(Term::Primitive(Primitive::U8)),
                },
                &mut fuel,
            ),
            Ok(TypeLayout { size: 2, align: 1 })
        );

        assert_eq!(
            compute_layout(
                &world,
                &Term::ErrorResult {
                    errors: vec![Term::Primitive(Primitive::U64); 4],
                    result: Box::new(Term::Primitive(Primitive::U64)),
                },
                &mut fuel,
            ),
            Ok(TypeLayout { size: 16, align: 8 })
        );

        assert_eq!(
            compute_layout(
                &world,
                &Term::ErrorResult {
                    errors: vec![Term::Primitive(Primitive::U128)],
                    result: Box::new(Term::Primitive(Primitive::U8)),
                },
                &mut fuel,
            ),
            Ok(TypeLayout {
                size: 32,
                align: 16,
            })
        );

        assert_eq!(
            compute_layout(
                &world,
                &Term::ErrorResult {
                    errors: vec![packed_9b],
                    result: Box::new(Term::Primitive(Primitive::U64)),
                },
                &mut fuel,
            ),
            Ok(TypeLayout { size: 24, align: 8 })
        );

        assert_eq!(
            compute_layout(
                &world,
                &Term::ErrorResult {
                    errors: vec![Term::Primitive(Primitive::Void), empty_struct],
                    result: Box::new(Term::Primitive(Primitive::Void)),
                },
                &mut fuel,
            ),
            Ok(TypeLayout { size: 1, align: 1 })
        );

        assert_eq!(
            compute_layout(
                &world,
                &Term::ErrorResult {
                    errors: vec![Term::Primitive(Primitive::U8)],
                    result: Box::new(Term::Primitive(Primitive::Never)),
                },
                &mut fuel,
            ),
            Ok(TypeLayout { size: 2, align: 1 })
        );

        assert_eq!(sum_tag_bits(256), Ok(8));
        assert_eq!(sum_tag_bits(257), Ok(16));
        assert_eq!(sum_tag_bits(65_536), Ok(16));
        assert_eq!(sum_tag_bits(65_537), Ok(32));
    }

    #[test]
    fn aggregate_enum_uses_tagged_union_sum_layout() {
        let mut world = TestWorldVec::new(Some(TargetData::new(64).unwrap()));
        let mut fuel = 10_000;

        let packed_9b = Term::Nominal {
            declaration: id(20),
            arguments: vec![],
        };
        world.nominals.push((
            packed_9b.clone(),
            NominalLayout::PackedStruct(vec![
                Term::Primitive(Primitive::U64),
                Term::Primitive(Primitive::U8),
            ]),
        ));

        let empty_struct = Term::Nominal {
            declaration: id(21),
            arguments: vec![],
        };
        world
            .nominals
            .push((empty_struct.clone(), NominalLayout::Struct(vec![])));

        let small = Term::Nominal {
            declaration: id(22),
            arguments: vec![],
        };
        world.nominals.push((
            small.clone(),
            NominalLayout::EnumAggregate(vec![
                vec![],
                vec![Term::Primitive(Primitive::U8)],
                vec![
                    Term::Primitive(Primitive::U8),
                    Term::Primitive(Primitive::U8),
                ],
            ]),
        ));
        assert_eq!(
            compute_layout(&world, &small, &mut fuel),
            Ok(TypeLayout { size: 3, align: 1 })
        );

        let multi = Term::Nominal {
            declaration: id(23),
            arguments: vec![],
        };
        world.nominals.push((
            multi.clone(),
            NominalLayout::EnumAggregate(vec![
                vec![],
                vec![
                    Term::Primitive(Primitive::U64),
                    Term::Primitive(Primitive::U64),
                ],
                vec![
                    Term::Primitive(Primitive::U64),
                    Term::Primitive(Primitive::U64),
                ],
                vec![
                    Term::Primitive(Primitive::U64),
                    Term::Primitive(Primitive::U64),
                ],
            ]),
        ));
        assert_eq!(
            compute_layout(&world, &multi, &mut fuel),
            Ok(TypeLayout { size: 24, align: 8 })
        );

        let wide = Term::Nominal {
            declaration: id(24),
            arguments: vec![],
        };
        world.nominals.push((
            wide.clone(),
            NominalLayout::EnumAggregate(vec![
                vec![Term::Primitive(Primitive::U8)],
                vec![Term::Primitive(Primitive::U128)],
            ]),
        ));
        assert_eq!(
            compute_layout(&world, &wide, &mut fuel),
            Ok(TypeLayout {
                size: 32,
                align: 16,
            })
        );

        let mixed_packed = Term::Nominal {
            declaration: id(25),
            arguments: vec![],
        };
        world.nominals.push((
            mixed_packed.clone(),
            NominalLayout::EnumAggregate(vec![
                vec![packed_9b],
                vec![Term::Primitive(Primitive::U64)],
            ]),
        ));
        assert_eq!(
            compute_layout(&world, &mixed_packed, &mut fuel),
            Ok(TypeLayout { size: 24, align: 8 })
        );

        let zero_payload = Term::Nominal {
            declaration: id(26),
            arguments: vec![],
        };
        world.nominals.push((
            zero_payload.clone(),
            NominalLayout::EnumAggregate(vec![
                vec![Term::Primitive(Primitive::Void)],
                vec![empty_struct],
            ]),
        ));
        assert_eq!(
            compute_layout(&world, &zero_payload, &mut fuel),
            Ok(TypeLayout { size: 1, align: 1 })
        );

        let many_257 = Term::Nominal {
            declaration: id(27),
            arguments: vec![],
        };
        let mut variants = vec![vec![Term::Primitive(Primitive::U8)]];
        variants.extend((1..257).map(|_| Vec::new()));
        world
            .nominals
            .push((many_257.clone(), NominalLayout::EnumAggregate(variants)));
        assert_eq!(
            compute_layout(&world, &many_257, &mut fuel),
            Ok(TypeLayout { size: 4, align: 2 })
        );
    }

    #[test]
    fn unit_enum_widths_and_compositional_niches_minimize_layouts() {
        let mut world = TestWorldVec::new(Some(TargetData::new(64).unwrap()));
        let mut fuel = 10_000;

        for (local, count, expected, exp_niche) in [
            (30, 0usize, TypeLayout { size: 0, align: 1 }, None),
            (31, 1, TypeLayout { size: 0, align: 1 }, None),
            (32, 3, TypeLayout { size: 1, align: 1 }, Some((3, 253))),
            (33, 256, TypeLayout { size: 1, align: 1 }, None),
            (
                34,
                257,
                TypeLayout { size: 2, align: 2 },
                Some((257, 65_279)),
            ),
        ] {
            let ty = Term::Nominal {
                declaration: id(local),
                arguments: vec![],
            };
            world
                .nominals
                .push((ty.clone(), NominalLayout::EnumUnit(count)));
            let res = compute_layout_and_niche(&world, &ty, &mut fuel).unwrap();
            assert_eq!(res.layout, expected, "unit enum count {count}");
            assert_eq!(
                res.niche.as_ref().map(|n| (n.start, n.count)),
                exp_niche,
                "unit enum niche count {count}"
            );
        }

        // Bool and nested optionals pack into 1 byte (up to 255 layers!).
        let opt_bool = Term::Optional(Box::new(Term::Primitive(Primitive::Bool)));
        let opt2_bool = Term::Optional(Box::new(opt_bool.clone()));
        let opt3_bool = Term::Optional(Box::new(opt2_bool.clone()));
        assert_eq!(
            compute_layout(&world, &opt_bool, &mut fuel),
            Ok(TypeLayout { size: 1, align: 1 })
        );
        assert_eq!(
            compute_layout(&world, &opt2_bool, &mut fuel),
            Ok(TypeLayout { size: 1, align: 1 })
        );
        assert_eq!(
            compute_layout(&world, &opt3_bool, &mut fuel),
            Ok(TypeLayout { size: 1, align: 1 })
        );

        // Pointer null niche eliminates tag in ?*u64, then falls back in ??*u64.
        let opt_ptr = Term::Optional(Box::new(Term::Pointer(Box::new(Term::Primitive(
            Primitive::U64,
        )))));
        let opt2_ptr = Term::Optional(Box::new(opt_ptr.clone()));
        assert_eq!(
            compute_layout(&world, &opt_ptr, &mut fuel),
            Ok(TypeLayout { size: 8, align: 8 })
        );
        assert_eq!(
            compute_layout(&world, &opt2_ptr, &mut fuel),
            Ok(TypeLayout { size: 16, align: 8 })
        );

        // Char has non-Unicode codepoint niches (> 0x10_FFFF).
        let opt_char = Term::Optional(Box::new(Term::Primitive(Primitive::Char)));
        assert_eq!(
            compute_layout(&world, &opt_char, &mut fuel),
            Ok(TypeLayout { size: 4, align: 4 })
        );

        // ?u64 has no inner niche (size 16), then ??u64 reuses ?u64's i1 tag niche (still size 16).
        let opt_u64 = Term::Optional(Box::new(Term::Primitive(Primitive::U64)));
        let opt2_u64 = Term::Optional(Box::new(opt_u64.clone()));
        assert_eq!(
            compute_layout(&world, &opt_u64, &mut fuel),
            Ok(TypeLayout { size: 16, align: 8 })
        );
        assert_eq!(
            compute_layout(&world, &opt2_u64, &mut fuel),
            Ok(TypeLayout { size: 16, align: 8 })
        );

        // Struct with pointer niche packs into ?S and { a: u8, b: Color } picks largest niche.
        let s_with_ptr = Term::Nominal {
            declaration: id(35),
            arguments: vec![],
        };
        world.nominals.push((
            s_with_ptr.clone(),
            NominalLayout::Struct(vec![
                Term::Primitive(Primitive::U64),
                Term::Pointer(Box::new(Term::Primitive(Primitive::U32))),
            ]),
        ));
        assert_eq!(
            compute_layout(&world, &Term::Optional(Box::new(s_with_ptr)), &mut fuel),
            Ok(TypeLayout { size: 16, align: 8 })
        );

        // Single-variant payload enum unwraps transparently and niche-encoded payload enums omit tag.
        let single_payload = Term::Nominal {
            declaration: id(36),
            arguments: vec![],
        };
        world.nominals.push((
            single_payload.clone(),
            NominalLayout::EnumAggregate(vec![vec![Term::Primitive(Primitive::U64)]]),
        ));
        assert_eq!(
            compute_layout(&world, &single_payload, &mut fuel),
            Ok(TypeLayout { size: 8, align: 8 })
        );

        let maybe_ptr = Term::Nominal {
            declaration: id(37),
            arguments: vec![],
        };
        world.nominals.push((
            maybe_ptr.clone(),
            NominalLayout::EnumAggregate(vec![
                vec![],
                vec![Term::Pointer(Box::new(Term::Primitive(Primitive::U32)))],
            ]),
        ));
        assert_eq!(
            compute_layout(&world, &maybe_ptr, &mut fuel),
            Ok(TypeLayout { size: 8, align: 8 })
        );

        let three_with_bool = Term::Nominal {
            declaration: id(38),
            arguments: vec![],
        };
        world.nominals.push((
            three_with_bool.clone(),
            NominalLayout::EnumAggregate(vec![
                vec![],
                vec![],
                vec![Term::Primitive(Primitive::Bool)],
            ]),
        ));
        assert_eq!(
            compute_layout(&world, &three_with_bool, &mut fuel),
            Ok(TypeLayout { size: 1, align: 1 })
        );

        let pair_opt = Term::Nominal {
            declaration: id(39),
            arguments: vec![],
        };
        world.nominals.push((
            pair_opt.clone(),
            NominalLayout::EnumAggregate(vec![
                vec![],
                vec![
                    Term::Primitive(Primitive::Bool),
                    Term::Primitive(Primitive::U64),
                ],
            ]),
        ));
        assert_eq!(
            compute_layout(&world, &pair_opt, &mut fuel),
            Ok(TypeLayout { size: 16, align: 8 })
        );

        // Fixed-bit enum niche eliminates tag in ?Packet.
        let packet_ty = Term::Nominal {
            declaration: id(40),
            arguments: vec![],
        };
        world.nominals.push((
            packet_ty.clone(),
            NominalLayout::EnumFixedBits {
                width: 8,
                niche: Some((1, 159)),
                exhaustive: false,
            },
        ));
        assert_eq!(
            compute_layout(&world, &Term::Optional(Box::new(packet_ty)), &mut fuel),
            Ok(TypeLayout { size: 1, align: 1 })
        );
    }

    #[test]
    fn bit_packed_structs_compute_lsb_offsets_nested_bits_and_reject_non_pod_fields() {
        let mut world = TestWorldVec::new(Some(TargetData::new(64).unwrap()));
        let mut fuel = 10_000;

        // Inner bit-packed struct: bool (1) + BitVec<4> (4) = 5 bits (no byte padding when nested!).
        let inner = Term::Nominal {
            declaration: id(101),
            arguments: vec![],
        };
        world.nominals.push((
            inner.clone(),
            NominalLayout::BitPackedStruct(vec![
                Term::Primitive(Primitive::Bool),
                Term::Primitive(Primitive::BitVec(4)),
            ]),
        ));
        assert_eq!(
            compute_layout(&world, &inner, &mut fuel),
            Ok(TypeLayout { size: 1, align: 1 })
        );

        // Outer bit-packed struct: inner (5) + Void (0) + inner (5) = 10 bits -> size 2, align 2.
        let outer = Term::Nominal {
            declaration: id(102),
            arguments: vec![],
        };
        let outer_fields = vec![
            inner.clone(),
            Term::Primitive(Primitive::Void),
            inner.clone(),
        ];
        world.nominals.push((
            outer.clone(),
            NominalLayout::BitPackedStruct(outer_fields.clone()),
        ));
        let layout = bit_packed_struct_layout(&world, &outer_fields, &mut fuel).unwrap();
        assert_eq!(layout.total_bits, 10);
        assert!(!layout.restricted);
        assert_eq!(
            layout.fields,
            vec![
                BitPackedFieldLayout {
                    bit_offset: 0,
                    bit_width: 5,
                    restricted: false,
                },
                BitPackedFieldLayout {
                    bit_offset: 5,
                    bit_width: 0,
                    restricted: false,
                },
                BitPackedFieldLayout {
                    bit_offset: 5,
                    bit_width: 5,
                    restricted: false,
                },
            ]
        );
        assert_eq!(
            compute_layout(&world, &outer, &mut fuel),
            Ok(TypeLayout { size: 2, align: 2 })
        );

        // Byte-packed struct holding inner bit-packed struct sees standalone BitVec<5> byte layout (size 1).
        let byte_holder = Term::Nominal {
            declaration: id(103),
            arguments: vec![],
        };
        world.nominals.push((
            byte_holder.clone(),
            NominalLayout::PackedStruct(vec![Term::Primitive(Primitive::U8), inner.clone()]),
        ));
        assert_eq!(
            compute_layout(&world, &byte_holder, &mut fuel),
            Ok(TypeLayout { size: 2, align: 1 })
        );

        // Newtype, exhaustive vs sparse EnumFixedBits, and char restrictedness.
        let nt_bits = Term::Nominal {
            declaration: id(104),
            arguments: vec![],
        };
        world.nominals.push((
            nt_bits.clone(),
            NominalLayout::Newtype(Term::Primitive(Primitive::BitVec(5))),
        ));
        let full_enum = Term::Nominal {
            declaration: id(105),
            arguments: vec![],
        };
        world.nominals.push((
            full_enum.clone(),
            NominalLayout::EnumFixedBits {
                width: 3,
                niche: None,
                exhaustive: true,
            },
        ));
        let sparse_enum = Term::Nominal {
            declaration: id(106),
            arguments: vec![],
        };
        world.nominals.push((
            sparse_enum.clone(),
            NominalLayout::EnumFixedBits {
                width: 3,
                niche: Some((5, 3)),
                exhaustive: false,
            },
        ));
        let unrestricted = bit_packed_struct_layout(
            &world,
            &[
                nt_bits,
                full_enum,
                Term::Primitive(Primitive::F16),
                Term::Primitive(Primitive::Bf16),
                Term::Primitive(Primitive::F32),
                Term::Primitive(Primitive::F64),
                Term::Primitive(Primitive::Usize),
            ],
            &mut fuel,
        )
        .unwrap();
        assert_eq!(unrestricted.total_bits, 5 + 3 + 16 + 16 + 32 + 64 + 64);
        assert!(!unrestricted.restricted);
        let with_char =
            bit_packed_struct_layout(&world, &[Term::Primitive(Primitive::Char)], &mut fuel)
                .unwrap();
        assert_eq!(with_char.total_bits, 32);
        assert!(with_char.restricted);
        let with_sparse = bit_packed_struct_layout(&world, &[sparse_enum], &mut fuel).unwrap();
        assert_eq!(with_sparse.total_bits, 3);
        assert!(with_sparse.restricted);

        // Disallowed fields fail cleanly.
        for bad in [
            Term::Pointer(Box::new(Term::Primitive(Primitive::U8))),
            Term::Primitive(Primitive::Str),
            Term::Primitive(Primitive::Symbol),
            Term::Optional(Box::new(Term::Primitive(Primitive::U8))),
            Term::Tuple(vec![Term::Primitive(Primitive::U8)]),
            byte_holder,
        ] {
            assert!(
                matches!(
                    bit_packed_struct_layout(&world, &[bad.clone()], &mut fuel),
                    Err(BitPackedError::UnsupportedField { field_index: 0, .. })
                ),
                "expected unsupported field error for {bad}"
            );
        }

        // Cycle fails cleanly.
        let cycle = Term::Nominal {
            declaration: id(107),
            arguments: vec![],
        };
        world.nominals.push((
            cycle.clone(),
            NominalLayout::BitPackedStruct(vec![cycle.clone()]),
        ));
        assert!(matches!(
            bit_packed_struct_layout(&world, &[cycle], &mut fuel),
            Err(BitPackedError::Layout(LayoutError::Cycle(_)))
        ));
    }

    #[test]
    fn stride_bit_width_and_field_offset_queries_are_exact() {
        let mut world = TestWorldVec::new(Some(TargetData::new(64).unwrap()));
        let mut fuel = 10_000;
        assert_eq!(TypeLayoutQuery::StrideOf.to_string(), "stride-of");
        assert_eq!(TypeLayoutQuery::BitWidthOf.to_string(), "bit-width-of");
        assert_eq!(TypeLayoutQuery::FieldOffset.to_string(), "field-offset");

        assert_eq!(
            compute_bit_width(&world, &Term::Primitive(Primitive::Void), &mut fuel),
            Ok(0)
        );
        assert_eq!(
            compute_bit_width(&world, &Term::Primitive(Primitive::Bool), &mut fuel),
            Ok(1)
        );
        assert_eq!(
            compute_bit_width(&world, &Term::Primitive(Primitive::BitVec(5)), &mut fuel),
            Ok(5)
        );
        assert_eq!(
            compute_bit_width(&world, &Term::Primitive(Primitive::Usize), &mut fuel),
            Ok(64)
        );
        assert_eq!(
            compute_stride(&world, &Term::Primitive(Primitive::U32), &mut fuel),
            Ok(4)
        );

        let st = Term::Nominal {
            declaration: id(200),
            arguments: vec![],
        };
        world.nominals.push((
            st.clone(),
            NominalLayout::Struct(vec![
                Term::Primitive(Primitive::U8),
                Term::Primitive(Primitive::U64),
                Term::Primitive(Primitive::U8),
            ]),
        ));
        world
            .field_names
            .push((st.clone(), vec!["a".into(), "b".into(), "c".into()]));
        assert_eq!(compute_stride(&world, &st, &mut fuel), Ok(24));
        assert_eq!(compute_bit_width(&world, &st, &mut fuel), Ok(192));
        assert_eq!(compute_field_offset(&world, &st, "a", &mut fuel), Ok(0));
        assert_eq!(compute_field_offset(&world, &st, "b", &mut fuel), Ok(8));
        assert_eq!(compute_field_offset(&world, &st, "c", &mut fuel), Ok(16));
        assert!(matches!(
            compute_field_offset(&world, &st, "missing", &mut fuel),
            Err(LayoutError::UnknownField { .. })
        ));

        let packed_st = Term::Nominal {
            declaration: id(201),
            arguments: vec![],
        };
        world.nominals.push((
            packed_st.clone(),
            NominalLayout::PackedStruct(vec![
                Term::Primitive(Primitive::U8),
                Term::Primitive(Primitive::U64),
            ]),
        ));
        world
            .field_names
            .push((packed_st.clone(), vec!["a".into(), "b".into()]));
        assert_eq!(compute_stride(&world, &packed_st, &mut fuel), Ok(9));
        assert_eq!(compute_bit_width(&world, &packed_st, &mut fuel), Ok(72));
        assert_eq!(
            compute_field_offset(&world, &packed_st, "b", &mut fuel),
            Ok(1)
        );

        let bp_st = Term::Nominal {
            declaration: id(202),
            arguments: vec![],
        };
        world.nominals.push((
            bp_st.clone(),
            NominalLayout::BitPackedStruct(vec![
                Term::Primitive(Primitive::BitVec(5)),
                Term::Primitive(Primitive::BitVec(3)),
                Term::Primitive(Primitive::U8),
            ]),
        ));
        world
            .field_names
            .push((bp_st.clone(), vec!["a".into(), "b".into(), "c".into()]));
        assert_eq!(compute_bit_width(&world, &bp_st, &mut fuel), Ok(16));
        assert_eq!(compute_field_offset(&world, &bp_st, "a", &mut fuel), Ok(0));
        assert!(matches!(
            compute_field_offset(&world, &bp_st, "b", &mut fuel),
            Err(LayoutError::UnalignedBitField { bit_offset: 5, .. })
        ));
        assert_eq!(compute_field_offset(&world, &bp_st, "c", &mut fuel), Ok(1));

        let rec = Term::Record(vec![
            ("x".into(), Term::Primitive(Primitive::U16)),
            ("y".into(), Term::Primitive(Primitive::U32)),
        ]);
        assert_eq!(compute_field_offset(&world, &rec, "x", &mut fuel), Ok(0));
        assert_eq!(compute_field_offset(&world, &rec, "y", &mut fuel), Ok(4));
    }
}
