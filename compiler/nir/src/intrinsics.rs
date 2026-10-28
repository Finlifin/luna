//! Target operation identity, independent of names and host representation.
use crate::{DeclarationId, Primitive, Term, program::Package};
use std::{collections::BTreeMap, fmt};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Tag {
    pub family: DeclarationId,
    pub variant: usize,
}

impl fmt::Display for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(intrinsic-tag (family {}) (variant {}))",
            self.family, self.variant
        )
    }
}

/// Validate identity without inferring any target behavior from the tag.
pub fn validate_tag(package: &crate::program::Package, tag: Tag) -> Result<(), &'static str> {
    let mut families = package.enums.iter().filter(|e| e.declaration == tag.family);
    let family = families.next().ok_or("unknown intrinsic tag family")?;
    if families.next().is_some() {
        return Err("ambiguous intrinsic tag family");
    }
    validate_family(family, tag)
}

pub fn validate_family(family: &crate::program::Enum, tag: Tag) -> Result<(), &'static str> {
    if family.declaration != tag.family {
        return Err("intrinsic tag family identity mismatch");
    }
    if !family.is_intrinsic {
        return Err("intrinsic tag requires an intrinsic_enum declaration");
    }
    if !family.type_parameters.is_empty()
        || !family.parameter_types.is_empty()
        || !family.requirements.is_empty()
        || !family.capabilities.is_empty()
    {
        return Err("intrinsic tag family instances are not implemented");
    }
    let variant = family
        .variants
        .get(tag.variant)
        .ok_or("unknown intrinsic tag variant")?;
    if variant.shape != crate::program::VariantShape::Unit || !variant.fields.is_empty() {
        return Err("intrinsic tag requires a payload-free unit variant");
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    FusedMultiplyAdd,
    SquareRoot,
    CopySign,
    Floor,
    Ceiling,
    Truncate,
    RoundEven,
    RoundAway,
    AbsoluteValue,
    Sine,
    Cosine,
    Exponential,
    Exponential2,
    NaturalLog,
    Log2,
    Log10,
    Power,
    IntegerPower,
    ScaleByPowerOfTwo,
    Minimum,
    Maximum,
    MinimumNumber,
    MaximumNumber,
    ByteSwap,
    BitReverse,
    BitAnd,
    BitOr,
    BitXor,
    PopulationCount,
    LeadingZeros,
    TrailingZeros,
    FunnelShiftLeft,
    FunnelShiftRight,
    IsNan,
    IsInfinite,
    IsFinite,
    IsNormal,
    IsSubnormal,
    SignBit,
    SaturatingAdd,
    SaturatingSubtract,
    IntegerMinimum,
    IntegerMaximum,
    WrappingAbsoluteValue,
}
impl Operation {
    /// Stable target-operation names used by host configuration, not source tag identities.
    pub const ALL: &'static [Self] = &[
        Self::FusedMultiplyAdd,
        Self::SquareRoot,
        Self::CopySign,
        Self::Floor,
        Self::Ceiling,
        Self::Truncate,
        Self::RoundEven,
        Self::RoundAway,
        Self::AbsoluteValue,
        Self::Sine,
        Self::Cosine,
        Self::Exponential,
        Self::Exponential2,
        Self::NaturalLog,
        Self::Log2,
        Self::Log10,
        Self::Power,
        Self::IntegerPower,
        Self::ScaleByPowerOfTwo,
        Self::Minimum,
        Self::Maximum,
        Self::MinimumNumber,
        Self::MaximumNumber,
        Self::ByteSwap,
        Self::BitReverse,
        Self::BitAnd,
        Self::BitOr,
        Self::BitXor,
        Self::PopulationCount,
        Self::LeadingZeros,
        Self::TrailingZeros,
        Self::FunnelShiftLeft,
        Self::FunnelShiftRight,
        Self::IsNan,
        Self::IsInfinite,
        Self::IsFinite,
        Self::IsNormal,
        Self::IsSubnormal,
        Self::SignBit,
        Self::SaturatingAdd,
        Self::SaturatingSubtract,
        Self::IntegerMinimum,
        Self::IntegerMaximum,
        Self::WrappingAbsoluteValue,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::FusedMultiplyAdd => "fma",
            Self::SquareRoot => "sqrt",
            Self::CopySign => "copysign",
            Self::Floor => "floor",
            Self::Ceiling => "ceil",
            Self::Truncate => "trunc",
            Self::RoundEven => "roundeven",
            Self::RoundAway => "round",
            Self::AbsoluteValue => "fabs",
            Self::Sine => "sin",
            Self::Cosine => "cos",
            Self::Exponential => "exp",
            Self::Exponential2 => "exp2",
            Self::NaturalLog => "log",
            Self::Log2 => "log2",
            Self::Log10 => "log10",
            Self::Power => "pow",
            Self::IntegerPower => "powi",
            Self::ScaleByPowerOfTwo => "ldexp",
            Self::Minimum => "minimum",
            Self::Maximum => "maximum",
            Self::MinimumNumber => "minimumnum",
            Self::MaximumNumber => "maximumnum",
            Self::ByteSwap => "bswap",
            Self::BitReverse => "bitreverse",
            Self::BitAnd => "bit_and",
            Self::BitOr => "bit_or",
            Self::BitXor => "bit_xor",
            Self::PopulationCount => "ctpop",
            Self::LeadingZeros => "ctlz",
            Self::TrailingZeros => "cttz",
            Self::FunnelShiftLeft => "fshl",
            Self::FunnelShiftRight => "fshr",
            Self::IsNan => "isnan",
            Self::IsInfinite => "isinf",
            Self::IsFinite => "isfinite",
            Self::IsNormal => "isnormal",
            Self::IsSubnormal => "issubnormal",
            Self::SignBit => "signbit",
            Self::SaturatingAdd => "add_sat",
            Self::SaturatingSubtract => "sub_sat",
            Self::IntegerMinimum => "min",
            Self::IntegerMaximum => "max",
            Self::WrappingAbsoluteValue => "abs",
        }
    }

    pub fn is_float_predicate(self) -> bool {
        matches!(
            self,
            Self::IsNan
                | Self::IsInfinite
                | Self::IsFinite
                | Self::IsNormal
                | Self::IsSubnormal
                | Self::SignBit
        )
    }
    pub fn has_integer_exponent(self) -> bool {
        matches!(self, Self::IntegerPower | Self::ScaleByPowerOfTwo)
    }

    /// Concrete operand roles are independent of the result's scalar family.
    /// Integer-exponent operations retain their floating base/result type.
    pub fn operand_type(self, scalar: &Term, index: usize) -> Term {
        if self.has_integer_exponent() && index == 1 {
            Term::Primitive(Primitive::I32)
        } else {
            scalar.clone()
        }
    }
    pub fn result_type(self, scalar: &Term) -> Term {
        if self.is_float_predicate() {
            Term::Primitive(Primitive::Bool)
        } else {
            scalar.clone()
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|op| op.name() == name)
    }

    /// Source type domain, checked before generic target admission.
    pub fn scalar_types(self) -> &'static [Primitive] {
        use Primitive::*;
        match self {
            Self::WrappingAbsoluteValue => &[I8, I16, I32, I64, I128, Isize],
            Self::BitAnd | Self::BitOr | Self::BitXor => &[
                U8, U16, U32, U64, U128, I8, I16, I32, I64, I128, Usize, Isize,
            ],
            Self::SaturatingAdd
            | Self::SaturatingSubtract
            | Self::IntegerMinimum
            | Self::IntegerMaximum => &[
                U8, U16, U32, U64, U128, I8, I16, I32, I64, I128, Usize, Isize,
            ],
            Self::ByteSwap => &[U16, U32, U64, U128, Usize],
            Self::BitReverse
            | Self::PopulationCount
            | Self::LeadingZeros
            | Self::TrailingZeros
            | Self::FunnelShiftLeft
            | Self::FunnelShiftRight => &[U8, U16, U32, U64, U128, Usize],
            Self::FusedMultiplyAdd
            | Self::SquareRoot
            | Self::CopySign
            | Self::Floor
            | Self::Ceiling
            | Self::Truncate
            | Self::RoundEven
            | Self::RoundAway
            | Self::AbsoluteValue
            | Self::Sine
            | Self::Cosine
            | Self::Exponential
            | Self::Exponential2
            | Self::NaturalLog
            | Self::Log2
            | Self::Log10
            | Self::Power
            | Self::IntegerPower
            | Self::ScaleByPowerOfTwo
            | Self::Minimum
            | Self::Maximum
            | Self::MinimumNumber
            | Self::MaximumNumber
            | Self::IsNan
            | Self::IsInfinite
            | Self::IsNormal
            | Self::IsSubnormal
            | Self::SignBit
            | Self::IsFinite => &[F32, F64],
        }
    }

    pub fn operand_count(self) -> usize {
        match self {
            Self::FusedMultiplyAdd | Self::FunnelShiftLeft | Self::FunnelShiftRight => 3,
            Self::SquareRoot
            | Self::WrappingAbsoluteValue
            | Self::Floor
            | Self::Ceiling
            | Self::Truncate
            | Self::RoundEven
            | Self::RoundAway
            | Self::AbsoluteValue
            | Self::Sine
            | Self::Cosine
            | Self::Exponential
            | Self::Exponential2
            | Self::NaturalLog
            | Self::Log2
            | Self::Log10
            | Self::ByteSwap
            | Self::BitReverse
            | Self::PopulationCount
            | Self::LeadingZeros
            | Self::TrailingZeros
            | Self::IsNan
            | Self::IsInfinite
            | Self::IsNormal
            | Self::IsSubnormal
            | Self::SignBit
            | Self::IsFinite => 1,
            Self::BitAnd
            | Self::BitOr
            | Self::BitXor
            | Self::SaturatingAdd
            | Self::SaturatingSubtract
            | Self::IntegerMinimum
            | Self::IntegerMaximum
            | Self::CopySign
            | Self::Power
            | Self::IntegerPower
            | Self::ScaleByPowerOfTwo
            | Self::Minimum
            | Self::Maximum
            | Self::MinimumNumber
            | Self::MaximumNumber => 2,
        }
    }
}

/// An immutable registry constructed transactionally from checked declarations.
/// Generic tag families need an instance-aware protocol and are not admitted yet.
#[derive(Debug)]
pub struct Registry {
    bindings: BTreeMap<Tag, Operation>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    UnknownFamily(Tag),
    AmbiguousFamily(Tag),
    NotIntrinsic(Tag),
    UnsupportedFamily(Tag),
    UnknownVariant(Tag),
    PayloadTag(Tag),
    DuplicateBinding(Tag),
    UnsupportedTag(Tag),
    OperandCount {
        tag: Tag,
        actual: usize,
        expected: usize,
    },
    OutputType(Tag),
    OperandType {
        tag: Tag,
        index: usize,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownFamily(tag) => write!(f, "unknown intrinsic family: {tag}"),
            Self::AmbiguousFamily(tag) => write!(f, "duplicate intrinsic family identity: {tag}"),
            Self::NotIntrinsic(tag) => write!(f, "enum lacks intrinsic_enum marker: {tag}"),
            Self::UnsupportedFamily(tag) => write!(
                f,
                "host intrinsic binding requires an unconditional, non-generic family: {tag}"
            ),
            Self::UnknownVariant(tag) => write!(f, "unknown intrinsic variant: {tag}"),
            Self::PayloadTag(tag) => {
                write!(f, "host operation requires a payload-free unit tag: {tag}")
            }
            Self::DuplicateBinding(tag) => write!(f, "duplicate host intrinsic binding: {tag}"),
            Self::UnsupportedTag(tag) => {
                write!(f, "intrinsic tag has no host operation binding: {tag}")
            }
            Self::OperandCount {
                tag,
                actual,
                expected,
            } => write!(
                f,
                "intrinsic requires {expected} operands, found {actual}: {tag}"
            ),
            Self::OutputType(tag) => {
                write!(
                    f,
                    "intrinsic requires an explicit output and operands in its operation type domain: {tag}"
                )
            }
            Self::OperandType { tag, index } => write!(
                f,
                "intrinsic operand {index} must match the requested scalar type: {tag}"
            ),
        }
    }
}
impl std::error::Error for Error {}

impl Registry {
    pub fn for_fma_family(
        family: &crate::program::Enum,
        variant: usize,
    ) -> Result<Self, &'static str> {
        Self::for_family(family, variant, Operation::FusedMultiplyAdd)
    }
    pub fn for_family(
        family: &crate::program::Enum,
        variant: usize,
        operation: Operation,
    ) -> Result<Self, &'static str> {
        let tag = Tag {
            family: family.declaration,
            variant,
        };
        validate_family(family, tag)?;
        Ok(Self {
            bindings: BTreeMap::from([(tag, operation)]),
        })
    }
    pub fn new(package: &Package, bindings: &[(Tag, Operation)]) -> Result<Self, Error> {
        let mut result = BTreeMap::new();
        for &(tag, operation) in bindings {
            let mut families = package.enums.iter().filter(|e| e.declaration == tag.family);
            let family = families.next().ok_or(Error::UnknownFamily(tag))?;
            if families.next().is_some() {
                return Err(Error::AmbiguousFamily(tag));
            }
            if !family.is_intrinsic {
                return Err(Error::NotIntrinsic(tag));
            }
            if !family.type_parameters.is_empty()
                || !family.parameter_types.is_empty()
                || !family.requirements.is_empty()
                || !family.capabilities.is_empty()
            {
                return Err(Error::UnsupportedFamily(tag));
            }
            let variant = family
                .variants
                .get(tag.variant)
                .ok_or(Error::UnknownVariant(tag))?;
            if variant.shape != crate::program::VariantShape::Unit || !variant.fields.is_empty() {
                return Err(Error::PayloadTag(tag));
            }
            if result.insert(tag, operation).is_some() {
                return Err(Error::DuplicateBinding(tag));
            }
        }
        Ok(Self { bindings: result })
    }

    /// Pending bindings are not executable evidence. The source checker must
    /// establish the type domain, and specialization must complete admission.
    pub fn defer(&self, tag: Tag, inputs: &[Term], output: &Term) -> Result<Checked, Error> {
        let operation = *self.bindings.get(&tag).ok_or(Error::UnsupportedTag(tag))?;
        if inputs.len() != operation.operand_count() {
            return Err(Error::OperandCount {
                tag,
                actual: inputs.len(),
                expected: operation.operand_count(),
            });
        }
        let scalar = if operation.is_float_predicate() {
            &inputs[0]
        } else {
            output
        };
        if *output != operation.result_type(scalar) || !parameter(scalar) {
            return Err(Error::OutputType(tag));
        }
        for (index, input) in inputs.iter().enumerate() {
            if *input != operation.operand_type(scalar, index) {
                return Err(Error::OperandType { tag, index });
            }
        }
        Ok(Checked {
            comptime: false,
            tag,
            operation,
            signature: Signature::Pending(scalar.clone()),
        })
    }

    /// Types must be the completed source input/output types. In particular the
    /// output is void when the source omits the immediate `.as(T)` interface.
    pub fn check(&self, tag: Tag, inputs: &[Term], output: &Term) -> Result<Checked, Error> {
        let operation = *self.bindings.get(&tag).ok_or(Error::UnsupportedTag(tag))?;
        if inputs.len() != operation.operand_count() {
            return Err(Error::OperandCount {
                tag,
                actual: inputs.len(),
                expected: operation.operand_count(),
            });
        }
        let scalar = if operation.is_float_predicate() {
            &inputs[0]
        } else {
            output
        };
        if *output != operation.result_type(scalar) {
            return Err(Error::OutputType(tag));
        }
        let Term::Primitive(primitive) = scalar else {
            return Err(Error::OutputType(tag));
        };
        if !operation.scalar_types().contains(primitive) {
            return Err(Error::OutputType(tag));
        }
        for (index, input) in inputs.iter().enumerate() {
            if *input != operation.operand_type(scalar, index) {
                return Err(Error::OperandType { tag, index });
            }
        }
        Ok(Checked {
            comptime: false,
            tag,
            operation,
            signature: Signature::Concrete(*primitive),
        })
    }
}

/// A target binding, either pending specialization or checked for a concrete
/// source scalar type. Platform layout is read from the consuming package's
/// explicit target, never cached here or substituted into type identity.
/// Only `validate`/`checked` grant concrete scalar admission; `validate_template`
/// checks a template's shape. Neither grants source unsafe permission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checked {
    comptime: bool,
    tag: Tag,
    operation: Operation,
    signature: Signature,
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum Signature {
    Concrete(Primitive),
    Pending(Term),
}
fn parameter(ty: &Term) -> bool {
    matches!(
        ty,
        Term::Rigid {
            sort: crate::Sort::Type,
            ..
        }
    )
}
impl Checked {
    pub fn for_comptime(mut self) -> Self {
        self.comptime = true;
        self
    }
    pub fn is_comptime(&self) -> bool {
        self.comptime
    }

    /// Revalidate after every rewrite/instantiation; stored evidence is never
    /// permission to change operand types, tag identity, or the output width.
    pub fn validate(
        &self,
        tag: Tag,
        inputs: &[crate::program::Expr],
        output: &Term,
    ) -> Result<(), &'static str> {
        self.validate_template(tag, inputs, output)?;
        if !matches!(self.signature, Signature::Concrete(_)) {
            return Err("intrinsic binding requires concrete specialization");
        }
        Ok(())
    }
    pub fn validate_template(
        &self,
        tag: Tag,
        inputs: &[crate::program::Expr],
        output: &Term,
    ) -> Result<(), &'static str> {
        if tag != self.tag {
            return Err("intrinsic binding has a different tag identity");
        }
        let expected = match &self.signature {
            Signature::Concrete(primitive) if self.operation.scalar_types().contains(primitive) => {
                Term::Primitive(*primitive)
            }
            Signature::Pending(ty) if parameter(ty) => ty.clone(),
            _ => return Err("invalid intrinsic scalar specialization"),
        };
        if inputs.len() == self.operation.operand_count()
            && *output == self.operation.result_type(&expected)
            && inputs
                .iter()
                .enumerate()
                .all(|(index, input)| input.ty == self.operation.operand_type(&expected, index))
        {
            Ok(())
        } else {
            Err("intrinsic binding disagrees with its input/output signature")
        }
    }

    pub fn tag(&self) -> Tag {
        self.tag
    }
    pub fn operation(&self) -> Operation {
        self.operation
    }
    /// Target-independent width, if one exists. A concrete platform type still
    /// needs an explicit target to obtain its layout.
    pub fn width(&self) -> Option<u32> {
        self.width_for_target(None)
    }
    pub fn width_for_target(&self, target: Option<crate::target::TargetData>) -> Option<u32> {
        match self.signature {
            Signature::Concrete(primitive) => match primitive {
                Primitive::U8 | Primitive::I8 => Some(8),
                Primitive::U16 | Primitive::I16 => Some(16),
                Primitive::U32 | Primitive::I32 | Primitive::F32 => Some(32),
                Primitive::U64 | Primitive::I64 | Primitive::F64 => Some(64),
                Primitive::U128 | Primitive::I128 => Some(128),
                Primitive::Usize | Primitive::Isize => target.map(|target| target.pointer_bits()),
                _ => None,
            },
            Signature::Pending(_) => None,
        }
    }
    /// Check layout-specific restrictions without selecting a missing target.
    /// Pending scalar types and target-independent platform bindings remain
    /// deferred. Consumers that need a width must also require `Some`.
    pub fn validate_target_layout(
        &self,
        target: Option<crate::target::TargetData>,
    ) -> Result<Option<u32>, &'static str> {
        let width = self.width_for_target(target);
        if self.operation == Operation::ByteSwap && width.is_some_and(|bits| bits % 16 != 0) {
            return Err(
                "bswap requires a positive multiple of 16 bits in the selected target layout",
            );
        }
        Ok(width)
    }

    /// Concrete evidence is immutable; only a pending type may be substituted.
    pub(crate) fn rewrite_pending<E>(
        &mut self,
        transform: &mut impl FnMut(&Term) -> Result<Term, E>,
    ) -> Result<(), E> {
        if let Signature::Pending(ty) = &self.signature {
            self.signature = match transform(ty)? {
                Term::Primitive(primitive)
                    if self.operation.scalar_types().contains(&primitive) =>
                {
                    Signature::Concrete(primitive)
                }
                ty => Signature::Pending(ty),
            };
        }
        Ok(())
    }
}

/// Admitted scalar operations have no pointer, allocation or owning payload
/// effects; their input expressions still carry all ordinary evaluation effects.
pub fn checked(expr: &crate::program::Expr) -> Result<&Checked, &'static str> {
    let crate::program::ExprKind::Intrinsic {
        tag,
        inputs,
        binding,
    } = &expr.kind
    else {
        return Err("expected intrinsic node");
    };
    let binding = binding
        .as_ref()
        .ok_or("intrinsic target behavior admission is not implemented")?;
    binding.validate(*tag, inputs, &expr.ty)?;
    Ok(binding)
}
impl fmt::Display for Checked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self.operation {
            Operation::FusedMultiplyAdd => "fused-multiply-add",
            Operation::SquareRoot => "square-root",
            Operation::CopySign => "copy-sign",
            operation => operation.name(),
        };
        write!(
            f,
            "({}-intrinsic {} (operation {name}) ",
            if self.comptime { "comptime" } else { "host" },
            self.tag
        )?;
        match &self.signature {
            Signature::Concrete(Primitive::F32) => write!(f, "(float-width 32))"),
            Signature::Concrete(Primitive::F64) => write!(f, "(float-width 64))"),
            Signature::Concrete(primitive) => {
                write!(f, "(scalar-type {}))", Term::Primitive(*primitive))
            }
            Signature::Pending(ty) => write!(f, "(pending-type {ty}))"),
        }
    }
}

#[cfg(test)]
mod operation_names_tests {
    use super::Operation;

    #[test]
    fn operation_names_are_unique_and_round_trip() {
        let mut names = std::collections::BTreeSet::new();
        for &operation in Operation::ALL {
            assert!(names.insert(operation.name()));
            assert_eq!(Operation::from_name(operation.name()), Some(operation));
        }
        for invalid in ["", "FMA", "fma ", "llvm.sqrt.f32", "minnum", "unknown"] {
            assert_eq!(Operation::from_name(invalid), None);
        }
        assert_eq!(Operation::RoundAway.name(), "round");
        assert_eq!(Operation::RoundEven.name(), "roundeven");
    }
}
