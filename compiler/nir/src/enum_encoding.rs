//! Fixed-width and Symbol enum encoding plans shared by construction and discrimination.
//! These cover fixed masks and singleton Symbols, not a proof engine for guards.
//! No tag is added, and incomplete coverage does not make arbitrary raw bits
//! into an enum value. Source elaboration remains responsible for field types.
use comptime_runtime::{Budget, bitvec::{BitVec, Error as BitError}};
use std::{collections::{BTreeMap, BTreeSet}, fmt};

/// A Symbol domain has no implied integer width or auxiliary discriminant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SymbolVariant {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SymbolRepresentation {
    variants: Vec<SymbolVariant>,
}
impl SymbolRepresentation {
    pub fn new(variants: Vec<SymbolVariant>, budget: &mut Budget) -> Result<Self, Error> {
        let mut names = BTreeSet::new();
        let mut values = BTreeMap::new();
        for (index, variant) in variants.iter().enumerate() {
            let comparisons = variants.len().max(1).ilog2() as usize + 1;
            budget.consume(variant.name.len().saturating_add(variant.value.len()).saturating_add(1)
                .saturating_mul(comparisons))?;
            if variant.name.split('.').any(str::is_empty) || !names.insert(&variant.name) {
                return Err(Error::Invalid("invalid or duplicate Symbol enum variant name"));
            }
            if let Some(first) = values.insert(&variant.value, index) {
                budget.consume(variant.value.len())?;
                return Err(Error::SymbolOverlap { first, second:index, witness:variant.value.clone() });
            }
        }
        Ok(Self { variants })
    }
    pub fn variants(&self) -> &[SymbolVariant] { &self.variants }
    pub fn encode(&self, index: usize) -> Option<&str> {
        self.variants.get(index).map(|variant| variant.value.as_str())
    }
    pub fn discriminate(&self, symbol: &str, budget: &mut Budget) -> Result<usize, Error> {
        for (index, variant) in self.variants.iter().enumerate() {
            budget.consume(symbol.len().min(variant.value.len()).saturating_add(1))?;
            if variant.value == symbol { return Ok(index); }
        }
        Err(Error::InvalidValue)
    }
}
impl fmt::Display for SymbolRepresentation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(symbol-encoding")?;
        for variant in &self.variants {
            f.write_str(" (variant (name ")?;
            crate::display::string(f, &variant.name)?;
            f.write_str(") (symbol ")?;
            crate::display::string(f, &variant.value)?;
            f.write_str("))")?;
        }
        f.write_str(")")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    /// Least-significant-bit offset in the whole representation.
    pub start: usize,
    pub width: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Bits(BitError),
    Invalid(&'static str),
    Overlap { first: usize, second: usize, witness: BitVec },
    SymbolOverlap { first: usize, second: usize, witness: String },
    InvalidValue,
}
impl From<BitError> for Error {
    fn from(error: BitError) -> Self { Self::Bits(error) }
}
impl From<comptime_runtime::Error> for Error {
    fn from(error: comptime_runtime::Error) -> Self { Self::Bits(BitError::Resource(error)) }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bits(error) => error.fmt(f),
            Self::Invalid(message) => f.write_str(message),
            Self::Overlap { first, second, witness } => write!(f,
                "enum encodings {first} and {second} overlap at {witness}"),
            Self::SymbolOverlap { first, second, witness } => {
                write!(f, "enum encodings {first} and {second} overlap at Symbol ")?;
                crate::display::string(f, witness)
            }
            Self::InvalidValue => f.write_str("raw encoding does not belong to this enum"),
        }
    }
}
impl std::error::Error for Error {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Variant {
    name: String,
    mask: BitVec,
    value: BitVec,
    fields: Vec<Field>,
}
impl Variant {
    /// Validate an exact partition into fixed bits and named payload slots.
    /// There is no implicit value for an unbound wildcard construction slot.
    pub fn new(name: String, mask: BitVec, value: BitVec, fields: Vec<Field>,
        budget: &mut Budget) -> Result<Self, Error> {
        if mask.width() == 0 || mask.width() != value.width() {
            return Err(Error::Invalid("enum encoding requires equal nonzero widths"));
        }
        if value.and(&mask, budget)? != value {
            return Err(Error::Invalid("enum fixed value contains unconstrained bits"));
        }
        let mut names = BTreeSet::new();
        let mut occupied = mask.clone();
        for field in &fields {
            budget.consume(1)?;
            let end = field.start.checked_add(field.width)
                .filter(|end| *end <= mask.width())
                .ok_or(Error::Invalid("enum payload lies outside the whole representation"))?;
            if field.width == 0 || !names.insert(field.name.clone()) {
                return Err(Error::Invalid("enum payload has zero width or duplicate name"));
            }
            if occupied.slice(field.start, end, budget)?.words().iter().any(|word| *word != 0) {
                return Err(Error::Invalid("enum payload overlaps fixed bits or another payload"));
            }
            let zero = zero(field.width, budget)?;
            occupied = occupied.replaced(field.start, &zero.not(budget)?, budget)?;
        }
        if occupied.not(budget)?.words().iter().any(|word| *word != 0) {
            return Err(Error::Invalid("enum construction contains unbound representation bits"));
        }
        Ok(Self { name, mask, value, fields })
    }
    pub fn name(&self) -> &str { &self.name }
    pub fn fields(&self) -> &[Field] { &self.fields }
    pub fn width(&self) -> usize { self.mask.width() }
    pub fn accepts(&self, bits: &BitVec, budget: &mut Budget) -> Result<bool, Error> {
        Ok(bits.and(&self.mask, budget)? == self.value)
    }
    pub fn construct(&self, payloads: &[BitVec], budget: &mut Budget) -> Result<BitVec, Error> {
        if payloads.len() != self.fields.len() {
            return Err(Error::Invalid("enum construction has the wrong payload count"));
        }
        let mut bits = BitVec::from_words(self.width(), self.value.words(), budget)?;
        for (field, payload) in self.fields.iter().zip(payloads) {
            if payload.width() != field.width {
                return Err(Error::Invalid("enum construction has the wrong payload width"));
            }
            bits = bits.replaced(field.start, payload, budget)?;
        }
        Ok(bits)
    }
    pub fn extract(&self, bits: &BitVec, budget: &mut Budget) -> Result<Vec<BitVec>, Error> {
        if !self.accepts(bits, budget)? { return Err(Error::InvalidValue); }
        self.fields.iter().map(|field|
            Ok(bits.slice(field.start, field.start + field.width, budget)?)).collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    width: usize,
    variants: Vec<Variant>,
}
/// The source representation domain, retained separately from its bit plan.
/// A width-only declaration has no implied scalar alignment or base identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Representation {
    pub base_type: Option<crate::Primitive>,
    pub layout: Layout,
}
impl Representation {
    pub fn width(&self) -> usize { self.layout.width() }
    pub fn variants(&self) -> &[Variant] { self.layout.variants() }
    pub fn validate(&self) -> Result<(), &'static str> {
        if let Some(base) = self.base_type {
            if !base.fixed_integer_layout().is_some_and(|(signed,width)| !signed && width as usize == self.width()) {
                return Err("enum base type disagrees with its encoding width");
            }
        }
        Ok(())
    }
}
impl fmt::Display for Representation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(enum-representation (base-type ")?;
        if let Some(base) = self.base_type {write!(f, "{base}")?;} else {write!(f, "none")?;}
        write!(f, ") (layout {}))", self.layout)
    }
}
impl Layout {
    pub fn new(width: usize, variants: Vec<Variant>, budget: &mut Budget) -> Result<Self, Error> {
        if width == 0 { return Err(Error::Invalid("enum representation width must be positive")); }
        // Check size limits even for an empty enum.
        zero(width, budget)?;
        let mut names = BTreeSet::new();
        for (index, variant) in variants.iter().enumerate() {
            budget.consume(1)?;
            if variant.width() != width || !names.insert(variant.name.clone()) {
                return Err(Error::Invalid("enum variants have mismatched widths or duplicate names"));
            }
            for (previous, other) in variants[..index].iter().enumerate() {
                let shared = variant.mask.and(&other.mask, budget)?;
                if variant.value.and(&shared, budget)? == other.value.and(&shared, budget)? {
                    return Err(Error::Overlap { first: previous, second: index,
                        witness: variant.value.or(&other.value, budget)? });
                }
            }
        }
        Ok(Self { width, variants })
    }
    pub fn width(&self) -> usize { self.width }
    pub fn variants(&self) -> &[Variant] { &self.variants }
    pub fn discriminate(&self, bits: &BitVec, budget: &mut Budget) -> Result<usize, Error> {
        if bits.width() != self.width { return Err(BitError::WidthMismatch.into()); }
        for (index, variant) in self.variants.iter().enumerate() {
            if variant.accepts(bits, budget)? { return Ok(index); }
        }
        Err(Error::InvalidValue)
    }
    /// A deterministic witness outside every variant, or exact full coverage.
    /// Subtract fixed-mask cubes, with every split charged to the shared budget.
    /// Exhaustion is an error, never evidence of coverage or disjointness.
    pub fn uncovered(&self, budget: &mut Budget) -> Result<Option<BitVec>, Error> {
        let blank = zero(self.width, budget)?;
        let mut remaining = vec![(blank.clone(), blank)];
        for variant in &self.variants {
            let mut next = Vec::new();
            for (mut mask, mut value) in remaining {
                let shared = mask.and(&variant.mask, budget)?;
                if value.and(&shared, budget)? != variant.value.and(&shared, budget)? {
                    next.push((mask, value));
                    continue;
                }
                let extra = variant.mask.and(&mask.not(budget)?, budget)?;
                for (word_index, word) in extra.words().iter().enumerate() {
                    let mut word = *word;
                    while word != 0 {
                        budget.consume(1)?;
                        let bit = word_index * 32 + word.trailing_zeros() as usize;
                        word &= word - 1;
                        let expected = variant.value.bit(bit, budget)?;
                        let one = BitVec::from_words(1, &[1], budget)?;
                        mask = mask.replaced(bit, &one, budget)?;
                        let outside = value.replaced(bit, &expected.not(budget)?, budget)?;
                        next.push((mask.clone(), outside));
                        value = value.replaced(bit, &expected, budget)?;
                    }
                }
                // The last prefix is contained in this variant and is removed.
            }
            remaining = next;
            if remaining.is_empty() { return Ok(None); }
        }
        Ok(remaining.into_iter().next().map(|(_, value)| value))
    }
}

fn zero(width: usize, budget: &mut Budget) -> Result<BitVec, Error> {
    let count = width / 32 + usize::from(width % 32 != 0);
    if count > budget.max_limbs { return Err(comptime_runtime::Error::SizeLimit.into()); }
    budget.consume(count.saturating_add(1))?;
    Ok(BitVec::from_words(width, &vec![0; count], budget)?)
}
impl fmt::Display for Variant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(encoding (name {:?}) (mask {}) (value {}) (fields", self.name, self.mask, self.value)?;
        for field in &self.fields {
            write!(f, " {field}")?;
        }
        write!(f, "))")
    }
}
impl fmt::Display for Field {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(field (name {:?}) (start {}) (width {}))", self.name, self.start, self.width)
    }
}
impl fmt::Display for Layout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(enum-encoding (width {}) (variants", self.width)?;
        for variant in &self.variants { write!(f, " {variant}")?; }
        write!(f, "))")
    }
}
