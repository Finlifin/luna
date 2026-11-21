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
}

impl TypeLayoutQuery {
    pub fn select(self, layout: TypeLayout) -> u64 {
        match self {
            Self::SizeOf => layout.size,
            Self::AlignOf => layout.align,
        }
    }
}

impl fmt::Display for TypeLayoutQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::SizeOf => "size-of",
            Self::AlignOf => "align-of",
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
    Newtype(Term),
    EnumUnit,
    EnumSymbol,
    EnumFixedBits(u32),
    EnumAggregate(Vec<Vec<Term>>),
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
}

pub fn compute_layout(
    world: &impl LayoutWorld,
    ty: &Term,
    fuel: &mut usize,
) -> Result<TypeLayout, LayoutError> {
    let mut visiting = Vec::new();
    layout_at(world, ty, 0, &mut visiting, fuel)
}

pub fn evaluate_kind(world: &impl LayoutWorld, kind: TypeKind, ty: &Term) -> Option<bool> {
    kind.evaluate_with_declarations(
        ty,
        |head| world.is_tuple_head(head),
        |_| world.nominal_kind(ty),
    )
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

fn layout_at(
    world: &impl LayoutWorld,
    ty: &Term,
    depth: usize,
    visiting: &mut Vec<Term>,
    fuel: &mut usize,
) -> Result<TypeLayout, LayoutError> {
    charge(fuel, 1)?;
    if depth >= MAX_LAYOUT_DEPTH {
        return Err(LayoutError::BudgetExhausted);
    }
    if world.is_template_identity(ty) {
        return Ok(TypeLayout { size: 8, align: 8 });
    }
    match ty {
        Term::Primitive(Primitive::Void) => Ok(TypeLayout { size: 0, align: 1 }),
        Term::Primitive(Primitive::Bool) => Ok(TypeLayout { size: 1, align: 1 }),
        Term::Primitive(Primitive::Char) => Ok(TypeLayout { size: 4, align: 4 }),
        Term::Primitive(Primitive::F16 | Primitive::Bf16) => Ok(TypeLayout { size: 2, align: 2 }),
        Term::Primitive(Primitive::F32) => Ok(TypeLayout { size: 4, align: 4 }),
        Term::Primitive(Primitive::F64) => Ok(TypeLayout { size: 8, align: 8 }),
        Term::Primitive(Primitive::I8 | Primitive::U8) => Ok(TypeLayout { size: 1, align: 1 }),
        Term::Primitive(Primitive::I16 | Primitive::U16) => Ok(TypeLayout { size: 2, align: 2 }),
        Term::Primitive(Primitive::I32 | Primitive::U32) => Ok(TypeLayout { size: 4, align: 4 }),
        Term::Primitive(Primitive::I64 | Primitive::U64) => Ok(TypeLayout { size: 8, align: 8 }),
        Term::Primitive(Primitive::I128 | Primitive::U128) => Ok(TypeLayout {
            size: 16,
            align: 16,
        }),
        Term::Primitive(Primitive::Isize | Primitive::Usize) => pointer_layout(world, ty),
        Term::Primitive(Primitive::BitVec(width)) => {
            let bits = u32::try_from(*width).map_err(|_| LayoutError::Unsupported(ty.clone()))?;
            int_layout(bits, ty)
        }
        Term::Primitive(Primitive::Str | Primitive::Symbol) => {
            let ptr = pointer_layout(world, ty)?;
            struct_layout(&[ptr, TypeLayout { size: 8, align: 8 }])
        }
        Term::Primitive(
            Primitive::Type
            | Primitive::Object
            | Primitive::Integer
            | Primitive::Float
            | Primitive::Unit,
        ) => Err(LayoutError::ComptimeOnly(ty.clone())),
        Term::Primitive(Primitive::Never) => Err(LayoutError::Unsupported(ty.clone())),
        Term::Pointer(_) => pointer_layout(world, ty),
        Term::UnsafeFunction(signature) => layout_at(world, signature, depth + 1, visiting, fuel),
        Term::Function { parameters, .. } => {
            if parameters.iter().any(|p| p.sort != Sort::Runtime) {
                return Err(LayoutError::ComptimeOnly(ty.clone()));
            }
            pointer_layout(world, ty)
        }
        Term::ComptimeFunction(_) | Term::NormalFormFunction(..) | Term::Lift(_) => {
            Err(LayoutError::ComptimeOnly(ty.clone()))
        }
        Term::Optional(payload) => {
            let payload_layout = if **payload == Term::Primitive(Primitive::Never) {
                TypeLayout { size: 0, align: 1 }
            } else {
                layout_at(world, payload, depth + 1, visiting, fuel)?
            };
            struct_layout(&[TypeLayout { size: 1, align: 1 }, payload_layout])
        }
        Term::ErrorResult { errors, result } => {
            charge(fuel, errors.len())?;
            let success_layout = if **result == Term::Primitive(Primitive::Never) {
                TypeLayout { size: 0, align: 1 }
            } else {
                layout_at(world, result, depth + 1, visiting, fuel)?
            };
            let mut branches = Vec::with_capacity(errors.len() + 1);
            branches.push(success_layout);
            for err in errors {
                branches.push(layout_at(world, err, depth + 1, visiting, fuel)?);
            }
            let tag = integer_layout(sum_tag_bits(branches.len())?)?;
            let payload = union_layout(&branches)?;
            struct_layout(&[tag, payload])
        }
        Term::Tuple(fields) => tuple_fields_layout(world, fields, depth, visiting, fuel),
        Term::Record(fields) => {
            charge(fuel, fields.len())?;
            let layouts = fields
                .iter()
                .map(|(_, field_ty)| layout_at(world, field_ty, depth + 1, visiting, fuel))
                .collect::<Result<Vec<_>, _>>()?;
            struct_layout(&layouts)
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
                    return tuple_fields_layout(world, elements, depth, visiting, fuel);
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
                    charge(fuel, fields.len())?;
                    let layouts = fields
                        .iter()
                        .map(|field_ty| layout_at(world, field_ty, depth + 1, visiting, fuel))
                        .collect::<Result<Vec<_>, _>>()?;
                    struct_layout(&layouts)
                }
                NominalLayout::PackedStruct(fields) => {
                    charge(fuel, fields.len())?;
                    let layouts = fields
                        .iter()
                        .map(|field_ty| layout_at(world, field_ty, depth + 1, visiting, fuel))
                        .collect::<Result<Vec<_>, _>>()?;
                    packed_struct_layout(&layouts)
                }
                NominalLayout::Newtype(representation) => {
                    layout_at(world, &representation, depth + 1, visiting, fuel)
                }
                NominalLayout::EnumUnit => Ok(TypeLayout { size: 4, align: 4 }),
                NominalLayout::EnumSymbol => {
                    let ptr = pointer_layout(world, ty)?;
                    struct_layout(&[ptr, TypeLayout { size: 8, align: 8 }])
                }
                NominalLayout::EnumFixedBits(width) => int_layout(width, ty),
                NominalLayout::EnumAggregate(variants) => {
                    charge(fuel, variants.len())?;
                    let mut variant_layouts = Vec::with_capacity(variants.len());
                    for variant_fields in variants {
                        charge(fuel, variant_fields.len())?;
                        let v_layouts = variant_fields
                            .iter()
                            .map(|f_ty| layout_at(world, f_ty, depth + 1, visiting, fuel))
                            .collect::<Result<Vec<_>, _>>()?;
                        variant_layouts.push(struct_layout(&v_layouts)?);
                    }
                    let tag = integer_layout(sum_tag_bits(variant_layouts.len())?)?;
                    let payload = union_layout(&variant_layouts)?;
                    struct_layout(&[tag, payload])
                }
            };
            visiting.pop();
            result
        }
        Term::Closure {
            environment,
            signature,
        } => {
            if !matches!(
                signature.callable_signature(),
                Term::Function { parameters, .. } if parameters.iter().all(|p| p.sort == Sort::Runtime)
            ) {
                return Err(LayoutError::ComptimeOnly(ty.clone()));
            }
            let ptr = pointer_layout(world, ty)?;
            charge(fuel, environment.len())?;
            let env_layouts = environment
                .iter()
                .map(|field_ty| layout_at(world, field_ty, depth + 1, visiting, fuel))
                .collect::<Result<Vec<_>, _>>()?;
            let env = struct_layout(&env_layouts)?;
            let state_count = closure_env_state_count(world, environment, depth + 1, fuel)?;
            struct_layout(&[
                ptr,
                env,
                TypeLayout {
                    size: u64::try_from(state_count).map_err(|_| LayoutError::Overflow)?,
                    align: 1,
                },
            ])
        }
        Term::Rigid { .. }
        | Term::Bound { .. }
        | Term::Hole { .. }
        | Term::AssociatedType { .. }
        | Term::ContextualClosure { .. }
        | Term::ClosureContextType
        | Term::ClosureContextValue(_) => Err(LayoutError::Unresolved(ty.clone())),
        _ => Err(LayoutError::Unsupported(ty.clone())),
    }
}

fn tuple_fields_layout(
    world: &impl LayoutWorld,
    fields: &[Term],
    depth: usize,
    visiting: &mut Vec<Term>,
    fuel: &mut usize,
) -> Result<TypeLayout, LayoutError> {
    if fields.is_empty() {
        return Ok(TypeLayout { size: 0, align: 1 });
    }
    charge(fuel, fields.len())?;
    let layouts = fields
        .iter()
        .map(|field_ty| layout_at(world, field_ty, depth + 1, visiting, fuel))
        .collect::<Result<Vec<_>, _>>()?;
    struct_layout(&layouts)
}

pub fn packed_struct_layout(fields: &[TypeLayout]) -> Result<TypeLayout, LayoutError> {
    let mut size = 0u64;
    for field in fields {
        size = size.checked_add(field.size).ok_or(LayoutError::Overflow)?;
    }
    Ok(TypeLayout { size, align: 1 })
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
            Some(NominalLayout::Newtype(repr)) => {
                closure_field_state_count(world, &repr, depth + 1, fuel)
            }
            Some(
                NominalLayout::EnumUnit
                | NominalLayout::EnumSymbol
                | NominalLayout::EnumFixedBits(_)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::TargetData;

    struct TestWorldVec {
        target: Option<TargetData>,
        tuple_head: Option<DeclarationId>,
        list_head: Option<DeclarationId>,
        nominals: Vec<(Term, NominalLayout)>,
    }

    impl TestWorldVec {
        fn new(target: Option<TargetData>) -> Self {
            Self {
                target,
                tuple_head: None,
                list_head: None,
                nominals: Vec::new(),
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
                NominalLayout::Struct(_) | NominalLayout::PackedStruct(_) => {
                    crate::NominalKind::Struct
                }
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
            Ok(TypeLayout { size: 24, align: 8 })
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
}
