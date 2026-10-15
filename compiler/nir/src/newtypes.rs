//! Nominal newtype declarations. Representation queries are explicit; they
//! never rewrite a nominal term during equality, member or trait lookup.
use crate::program::{TraitRequirement, TypePredicate};
use crate::{DeclarationId, Sort, Term, TypeError};
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub struct Newtype {
    pub declaration: DeclarationId,
    pub name: String,
    pub type_parameters: Vec<DeclarationId>,
    pub requirements: Vec<TypePredicate>,
    pub capabilities: Vec<TraitRequirement>,
    pub representation: Term,
}
impl Newtype {
    pub fn instantiate_representation(&self, arguments: &[Term]) -> Result<Term, TypeError> {
        self.instantiate_representation_in_scope(arguments, &[])
    }
    pub fn instantiate_representation_in_scope(
        &self,
        arguments: &[Term],
        frames: &[Vec<Sort>],
    ) -> Result<Term, TypeError> {
        let actual = self.representation.sort();
        if actual != Sort::Type {
            return Err(TypeError::WrongSort {
                expected: Sort::Type,
                actual,
            });
        }
        self.representation
            .instantiate_types_in_scope(&self.type_parameters, arguments, frames)
    }
}
impl fmt::Display for Newtype {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(newtype (id {}) (name ", self.declaration)?;
        crate::display::string(f, &self.name)?;
        f.write_str(") (type-parameters")?;
        for parameter in &self.type_parameters {
            write!(f, " {parameter}")?;
        }
        f.write_str(") (capabilities")?;
        for capability in &self.capabilities {
            write!(f, " {capability}")?;
        }
        f.write_str(") (requires")?;
        for requirement in &self.requirements {
            write!(f, " {requirement}")?;
        }
        write!(f, ") (representation {}))", self.representation)
    }
}

/// Validate declaration metadata independently of executable casts. A newtype
/// cannot borrow the identity of a struct or enum to obtain its operations.
pub fn validate(package: &crate::program::Package) -> Result<(), &'static str> {
    let mut fuel = 100_000;
    let mut identities = std::collections::BTreeSet::new();
    for definition in &package.structs {
        identities.insert(definition.declaration);
    }
    for definition in &package.enums {
        identities.insert(definition.declaration);
    }
    for definition in &package.newtypes {
        spend(
            &mut fuel,
            definition.type_parameters.len().saturating_add(1),
        )
        .map_err(|_| "newtype metadata validation budget exhausted")?;
        if !identities.insert(definition.declaration) {
            return Err("duplicate nominal newtype identity");
        }
        let arguments = definition
            .type_parameters
            .iter()
            .map(|&declaration| Term::Rigid {
                declaration,
                sort: Sort::Type,
            })
            .collect::<Vec<_>>();
        definition
            .representation_budgeted(&arguments, &[], &mut fuel)
            .map_err(|_| "invalid or excessively large newtype representation or telescope")?;
    }
    Ok(())
}

/// The operation changes ownership of the whole input value; it neither reads
/// a field nor provides evidence for Copy/Clone/Drop on either endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Wrap,
    Unwrap,
}
impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Wrap => "wrap",
            Self::Unwrap => "unwrap",
        })
    }
}

/// Declaration-side proof of a single representation edge. Private fields
/// prevent assembling a cast from two unrelated types with compatible layouts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conversion {
    declaration: DeclarationId,
    direction: Direction,
    source: Term,
    target: Term,
}
impl Conversion {
    pub fn declaration(&self) -> DeclarationId {
        self.declaration
    }
    pub fn direction(&self) -> Direction {
        self.direction
    }
    pub fn source(&self) -> &Term {
        &self.source
    }
    pub fn target(&self) -> &Term {
        &self.target
    }
}
impl fmt::Display for Conversion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(newtype-conversion (declaration {}) (direction {}) (source {}) (target {}))",
            self.declaration, self.direction, self.source, self.target
        )
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConversionError {
    Budget,
    Type(TypeError),
    DuplicateIdentity(DeclarationId),
}
impl fmt::Display for ConversionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Budget => f.write_str("newtype representation query budget exhausted"),
            Self::Type(error) => write!(f, "invalid newtype representation query: {error}"),
            Self::DuplicateIdentity(id) => write!(f, "ambiguous newtype declaration {id}"),
        }
    }
}
impl std::error::Error for ConversionError {}

fn spend(fuel: &mut usize, amount: usize) -> Result<(), ConversionError> {
    match fuel.checked_sub(amount) {
        Some(left) => {
            *fuel = left;
            Ok(())
        }
        None => {
            *fuel = 0;
            Err(ConversionError::Budget)
        }
    }
}
fn charge(ty: &Term, fuel: &mut usize) -> Result<(), ConversionError> {
    crate::type_computations::charge_term(ty, fuel).map_err(|_| {
        *fuel = 0;
        ConversionError::Budget
    })
}

impl Newtype {
    /// Bound validation, substitution and output copying before allocating the
    /// instantiated representation. The caller's frames remain explicit.
    pub fn representation_budgeted(
        &self,
        arguments: &[Term],
        frames: &[Vec<Sort>],
        fuel: &mut usize,
    ) -> Result<Term, ConversionError> {
        spend(
            fuel,
            self.type_parameters.len().saturating_add(arguments.len()),
        )?;
        spend(fuel, frames.len())?;
        for frame in frames {
            spend(fuel, frame.len())?;
        }
        let before = *fuel;
        charge(&self.representation, fuel)?;
        let representation_work = before - *fuel;
        let before = *fuel;
        for argument in arguments {
            charge(argument, fuel)?;
        }
        let argument_work = before - *fuel;
        // Each occurrence of a parameter can copy its argument. Count that
        // expansion before invoking the general structural substitution engine.
        spend(
            fuel,
            representation_work.saturating_mul(argument_work.saturating_add(1)),
        )?;
        self.instantiate_representation_in_scope(arguments, frames)
            .map_err(ConversionError::Type)
    }
}

/// Query one explicit edge after endpoint type elaboration. This is not a type
/// equality solver: callers must discharge endpoint formation obligations and
/// retain their usual type-equality evidence. It never unfolds opaque heads,
/// searches a path through multiple wrappers, or compares sizes/alignments.
/// `None` means no direct newtype conversion, while errors are never absence.
pub fn conversion(
    definitions: &[Newtype],
    source: &Term,
    target: &Term,
    frames: &[Vec<Sort>],
    fuel: &mut usize,
) -> Result<Option<Conversion>, ConversionError> {
    conversion_from(definitions.iter(), source, target, frames, fuel)
}

pub fn conversion_from<'a>(
    definitions: impl IntoIterator<Item = &'a Newtype> + Clone,
    source: &Term,
    target: &Term,
    frames: &[Vec<Sort>],
    fuel: &mut usize,
) -> Result<Option<Conversion>, ConversionError> {
    for ty in [source, target] {
        charge(ty, fuel)?;
        if ty.sort() != Sort::Type {
            return Err(ConversionError::Type(TypeError::WrongSort {
                expected: Sort::Type,
                actual: ty.sort(),
            }));
        }
    }
    spend(fuel, frames.len())?;
    for frame in frames {
        spend(fuel, frame.len())?;
    }
    source.check_scope(frames).map_err(ConversionError::Type)?;
    target.check_scope(frames).map_err(ConversionError::Type)?;
    // Identity conversions belong to the general conversion layer, not to a
    // representation edge, even if malformed metadata is self-referential.
    if source == target {
        return Ok(None);
    }
    for (nominal, representation, direction) in [
        (source, target, Direction::Unwrap),
        (target, source, Direction::Wrap),
    ] {
        let Term::Nominal {
            declaration,
            arguments,
        } = nominal
        else {
            continue;
        };
        let mut found = None;
        for definition in definitions.clone() {
            spend(fuel, 1)?;
            if definition.declaration == *declaration {
                if found.is_some() {
                    return Err(ConversionError::DuplicateIdentity(*declaration));
                }
                found = Some(definition);
            }
        }
        if let Some(definition) = found {
            let instantiated = definition.representation_budgeted(arguments, frames, fuel)?;
            charge(&instantiated, fuel)?;
            charge(representation, fuel)?;
            if instantiated == *representation {
                charge(source, fuel)?;
                charge(target, fuel)?;
                return Ok(Some(Conversion {
                    declaration: *declaration,
                    direction,
                    source: source.clone(),
                    target: target.clone(),
                }));
            }
        }
    }
    Ok(None)
}

/// Recheck a concrete IIR descriptor against its complete nominal instance key.
/// Its representation is already substituted; key arguments must not be applied
/// a second time to the now-empty declaration telescope.
pub fn instance_conversion(
    definition: &Newtype,
    key: &Term,
    source: &Term,
    target: &Term,
    fuel: &mut usize,
) -> Result<Option<Conversion>, ConversionError> {
    for ty in [key, source, target, &definition.representation] {
        charge(ty, fuel)?;
        if ty.sort() != Sort::Type {
            return Err(ConversionError::Type(TypeError::WrongSort {
                expected: Sort::Type,
                actual: ty.sort(),
            }));
        }
        ty.check_scope(&[]).map_err(ConversionError::Type)?;
    }
    if !definition.type_parameters.is_empty() {
        return Err(ConversionError::Type(TypeError::ArgumentCount {
            expected: 0,
            actual: definition.type_parameters.len(),
        }));
    }
    if !matches!(key, Term::Nominal {declaration,..} if *declaration == definition.declaration) {
        return Err(ConversionError::DuplicateIdentity(definition.declaration));
    }
    if source == target {
        return Ok(None);
    }
    let direction = if key == source && definition.representation == *target {
        Direction::Unwrap
    } else if key == target && definition.representation == *source {
        Direction::Wrap
    } else {
        return Ok(None);
    };
    charge(source, fuel)?;
    charge(target, fuel)?;
    Ok(Some(Conversion {
        declaration: definition.declaration,
        direction,
        source: source.clone(),
        target: target.clone(),
    }))
}
