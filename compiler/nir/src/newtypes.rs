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
    /// Empty retains the all-Type telescope used by existing declarations.
    pub parameter_types: Vec<Term>,
    /// Lexical captures occupy the identity suffix but precede own parameters semantically.
    pub capture_count: usize,
    pub requirements: Vec<TypePredicate>,
    pub capabilities: Vec<TraitRequirement>,
    pub representation: Term,
}
impl Newtype {
    pub fn parameter_binders(&self) -> Result<Vec<crate::DeclarationParameter>, TypeError> {
        if self.capture_count > self.type_parameters.len() {
            return Err(TypeError::ArgumentCount {
                expected: self.type_parameters.len(),
                actual: self.capture_count,
            });
        }
        crate::nominal_parameters::binders(&self.type_parameters, &self.parameter_types)
    }

    pub fn validate_completed_arguments(
        &self,
        arguments: &[Term],
        fuel: &mut usize,
    ) -> Result<(), crate::nominal_parameters::AdmissionError> {
        self.validate_completed_arguments_with_layouts(arguments, fuel, |_, _, _| {
            Err("nominal value admission requires declaration layout evidence".into())
        })
    }

    /// Preserve capture-first domain admission when payload layout evidence is
    /// available. Stored argument order and type identity stay unchanged.
    pub fn validate_completed_arguments_with_layouts(
        &self,
        arguments: &[Term],
        fuel: &mut usize,
        layout: impl FnMut(
            &Term,
            Option<u32>,
            &mut usize,
        ) -> Result<Vec<Term>, crate::NominalAdmissionError>,
    ) -> Result<(), crate::NominalAdmissionError> {
        let own = self
            .type_parameters
            .len()
            .checked_sub(self.capture_count)
            .ok_or("invalid newtype capture count")?;
        if self.capture_count == 0 {
            return crate::nominal_parameters::validate_completed_with_layouts(
                &self.type_parameters,
                &self.parameter_types,
                arguments,
                fuel,
                layout,
            );
        }
        if arguments.len() != self.type_parameters.len()
            || (!self.parameter_types.is_empty() && self.parameter_types.len() != arguments.len())
        {
            return Err("newtype argument/domain count mismatch".into());
        }
        *fuel = fuel
            .checked_sub(arguments.len())
            .ok_or(crate::nominal_parameters::AdmissionError::Budget)?;
        for domain in &self.parameter_types {
            crate::type_computations::charge_term(domain, fuel)
                .map_err(crate::nominal_parameters::AdmissionError::from)?;
        }
        for argument in arguments {
            crate::type_computations::charge_term(argument, fuel)
                .map_err(crate::nominal_parameters::AdmissionError::from)?;
        }
        let order = (own..arguments.len()).chain(0..own).collect::<Vec<_>>();
        let parameters = order
            .iter()
            .map(|&i| self.type_parameters[i])
            .collect::<Vec<_>>();
        let domains = if self.parameter_types.is_empty() {
            vec![]
        } else {
            order
                .iter()
                .map(|&i| self.parameter_types[i].clone())
                .collect()
        };
        let arguments = order
            .iter()
            .map(|&i| arguments[i].clone())
            .collect::<Vec<_>>();
        crate::nominal_parameters::validate_completed_with_layouts(
            &parameters,
            &domains,
            &arguments,
            fuel,
            layout,
        )
    }

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
        self.representation.instantiate_parameters_in_scope(
            &self.parameter_binders()?,
            arguments,
            frames,
        )
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
        write!(f, ") (captures {}) (parameter-types", self.capture_count)?;
        for domain in &self.parameter_types {
            write!(f, " {domain}")?;
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
        let binders = definition
            .parameter_binders()
            .map_err(|_| "invalid newtype parameter domains")?;
        let mut parameters = std::collections::BTreeMap::new();
        let own = binders.len() - definition.capture_count;
        for index in (own..binders.len()).chain(0..own) {
            let parameter = &binders[index];
            if let Some(domain) = definition.parameter_types.get(index) {
                validate_contract_capture(domain, &parameters, &mut fuel)?;
            }
            parameters.insert(parameter.declaration, parameter.sort);
        }
        let arguments = binders
            .iter()
            .map(|parameter| Term::Rigid {
                declaration: parameter.declaration,
                sort: parameter.sort,
            })
            .collect::<Vec<_>>();
        definition
            .representation_budgeted(&arguments, &[], &mut fuel)
            .map_err(|_| "invalid or excessively large newtype representation or telescope")?;
        validate_contract_capture(&definition.representation, &parameters, &mut fuel).map_err(
            |reason| {
                if reason == "newtype contract has an undeclared captured parameter" {
                    "newtype representation has an undeclared captured parameter"
                } else {
                    reason
                }
            },
        )?;
        // Formation premises are part of the same closed declaration as its
        // representation. Even a constant representation may capture types
        // solely through a capability or requirement.
        for capability in &definition.capabilities {
            validate_contract_capture(&capability.subject, &parameters, &mut fuel)?;
            for argument in &capability.interface_arguments {
                validate_contract_capture(argument, &parameters, &mut fuel)?;
            }
        }
        spend(&mut fuel, definition.requirements.len())
            .map_err(|_| "newtype metadata validation budget exhausted")?;
        let mut predicates: Vec<_> = definition.requirements.iter().collect();
        while let Some(predicate) = predicates.pop() {
            match predicate {
                TypePredicate::Bool(_) => {}
                TypePredicate::Equal(left, right) => {
                    validate_contract_capture(left, &parameters, &mut fuel)?;
                    validate_contract_capture(right, &parameters, &mut fuel)?;
                }
                TypePredicate::Trait(capability) => {
                    validate_contract_capture(&capability.subject, &parameters, &mut fuel)?;
                    for argument in &capability.interface_arguments {
                        validate_contract_capture(argument, &parameters, &mut fuel)?;
                    }
                }
                TypePredicate::Not(inner) => {
                    spend(&mut fuel, 1)
                        .map_err(|_| "newtype metadata validation budget exhausted")?;
                    predicates.push(inner);
                }
                TypePredicate::And(parts) | TypePredicate::Or(parts) => {
                    spend(&mut fuel, parts.len())
                        .map_err(|_| "newtype metadata validation budget exhausted")?;
                    predicates.extend(parts);
                }
            }
        }
    }
    Ok(())
}

fn validate_contract_capture(
    term: &Term,
    parameters: &std::collections::BTreeMap<DeclarationId, Sort>,
    fuel: &mut usize,
) -> Result<(), &'static str> {
    charge(term, fuel).map_err(|_| "newtype metadata validation budget exhausted")?;
    let free = term
        .rigid_parameters()
        .map_err(|_| "invalid newtype contract parameters")?;
    if free
        .into_iter()
        .any(|(parameter, sort)| parameters.get(&parameter) != Some(&sort))
    {
        return Err("newtype contract has an undeclared captured parameter");
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
        for domain in &self.parameter_types {
            charge(domain, fuel)?;
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
