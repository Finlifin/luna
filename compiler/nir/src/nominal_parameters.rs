//! Nominal declaration binders. Layout substitution preserves value-indexed
//! identities; domain admission and trait/program obligations remain separate.
use crate::{DeclarationId, DeclarationParameter, Primitive, Sort, Term, TypeError};
use std::collections::BTreeSet;

/// Resource exhaustion is distinct from a disproved input domain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdmissionError {
    Budget,
    Invalid(String),
}
impl std::fmt::Display for AdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Budget => f.write_str("nominal input validation budget exhausted"),
            Self::Invalid(message) => f.write_str(message),
        }
    }
}
impl std::error::Error for AdmissionError {}
impl From<&str> for AdmissionError {
    fn from(message: &str) -> Self {
        Self::Invalid(message.into())
    }
}
impl From<String> for AdmissionError {
    fn from(message: String) -> Self {
        Self::Invalid(message)
    }
}
impl From<crate::type_computations::Error> for AdmissionError {
    fn from(error: crate::type_computations::Error) -> Self {
        match error {
            crate::type_computations::Error::Budget => Self::Budget,
            error => Self::Invalid(error.to_string()),
        }
    }
}
impl From<AdmissionError> for String {
    fn from(error: AdmissionError) -> Self {
        error.to_string()
    }
}

pub fn binders(
    parameters: &[DeclarationId],
    domains: &[Term],
) -> Result<Vec<DeclarationParameter>, TypeError> {
    if !domains.is_empty() && domains.len() != parameters.len() {
        return Err(TypeError::ArgumentCount {
            expected: parameters.len(),
            actual: domains.len(),
        });
    }
    let mut seen = BTreeSet::new();
    parameters
        .iter()
        .enumerate()
        .map(|(index, declaration)| {
            if !seen.insert(*declaration) {
                return Err(TypeError::DuplicateDeclarationParameter(*declaration));
            }
            let domain = domains
                .get(index)
                .unwrap_or(&Term::Primitive(Primitive::Type));
            if domain.sort() != Sort::Type {
                return Err(TypeError::WrongSort {
                    expected: Sort::Type,
                    actual: domain.sort(),
                });
            }
            domain.check_scope(&[])?;
            Ok(DeclarationParameter {
                declaration: *declaration,
                sort: if *domain == Term::Primitive(Primitive::Type) {
                    Sort::Type
                } else {
                    Sort::ComptimeValue
                },
            })
        })
        .collect()
}

/// Admission of completed value indices at the concrete layout boundary.
/// Symbolic declarations need their own domain evidence and are not guessed here.
pub fn validate_completed(
    parameters: &[DeclarationId],
    domains: &[Term],
    arguments: &[Term],
    fuel: &mut usize,
) -> Result<(), AdmissionError> {
    validate_completed_in(parameters, domains, arguments, fuel, None)
}

pub fn validate_completed_in(
    parameters: &[DeclarationId],
    domains: &[Term],
    arguments: &[Term],
    fuel: &mut usize,
    target: Option<crate::target::TargetData>,
) -> Result<(), AdmissionError> {
    validate_completed_with_layouts_in(
        parameters,
        domains,
        arguments,
        fuel,
        |_, _, _| Err("nominal value admission requires declaration layout evidence".into()),
        target,
    )
}

/// Admission with authoritative declaration-order payload domains. The callback
/// must validate the nominal instance and variant, or a registered Tuple head
/// for tuple value indices. A value's own type tag is never sufficient evidence
/// of its layout. All work shares the caller's fuel.
pub fn validate_completed_with_layouts(
    parameters: &[DeclarationId],
    domains: &[Term],
    arguments: &[Term],
    fuel: &mut usize,
    layout: impl FnMut(&Term, Option<u32>, &mut usize) -> Result<Vec<Term>, AdmissionError>,
) -> Result<(), AdmissionError> {
    validate_completed_with_layouts_in(parameters, domains, arguments, fuel, layout, None)
}

pub fn validate_completed_with_layouts_in(
    parameters: &[DeclarationId],
    domains: &[Term],
    arguments: &[Term],
    fuel: &mut usize,
    layout: impl FnMut(&Term, Option<u32>, &mut usize) -> Result<Vec<Term>, AdmissionError>,
    target: Option<crate::target::TargetData>,
) -> Result<(), AdmissionError> {
    validate_with_evidence_in(
        parameters,
        domains,
        arguments,
        fuel,
        layout,
        |_, _, _| false,
        target,
    )
}

/// Symbolic value leaves require independent declaration-domain evidence.
pub fn validate_with_evidence(
    parameters: &[DeclarationId],
    domains: &[Term],
    arguments: &[Term],
    fuel: &mut usize,
    layout: impl FnMut(&Term, Option<u32>, &mut usize) -> Result<Vec<Term>, AdmissionError>,
    declared: impl FnMut(&Term, &Term, &mut usize) -> bool,
) -> Result<(), AdmissionError> {
    validate_with_evidence_in(parameters, domains, arguments, fuel, layout, declared, None)
}

pub fn validate_with_evidence_in(
    parameters: &[DeclarationId],
    domains: &[Term],
    arguments: &[Term],
    fuel: &mut usize,
    mut layout: impl FnMut(&Term, Option<u32>, &mut usize) -> Result<Vec<Term>, AdmissionError>,
    mut declared: impl FnMut(&Term, &Term, &mut usize) -> bool,
    target: Option<crate::target::TargetData>,
) -> Result<(), AdmissionError> {
    admit_with_evidence_in(
        parameters,
        domains,
        arguments,
        fuel,
        &mut layout,
        &mut declared,
        target,
    )
    .map(|_| ())
}

/// Admit immutable collections using independently registered element domains.
/// The callback must charge lookup and domain copying to the shared fuel.
/// Neither a frozen value's tag nor an opaque head's shape grants this evidence.
pub fn validate_with_collections_in(
    parameters: &[DeclarationId],
    domains: &[Term],
    arguments: &[Term],
    fuel: &mut usize,
    layout: impl FnMut(&Term, Option<u32>, &mut usize) -> Result<Vec<Term>, AdmissionError>,
    declared: impl FnMut(&Term, &Term, &mut usize) -> bool,
    collection: impl FnMut(&Term, &mut usize) -> Result<Option<Term>, AdmissionError>,
    target: Option<crate::target::TargetData>,
) -> Result<(), AdmissionError> {
    admit_with_collections_in(
        parameters, domains, arguments, fuel, layout, declared, collection, target,
    )
    .map(|_| ())
}

/// Structural declaration substitution in an explicit caller scope. This checks
/// scopes and sorts, including a dependent domain becoming Type. It does not
/// validate scalar ranges, aggregate payloads, requirements or capabilities;
/// those remain formation/admission obligations at the owning boundary.
pub struct DeclarationSubstitution<'a> {
    parameters: Vec<DeclarationParameter>,
    arguments: &'a [Term],
    frames: &'a [Vec<Sort>],
}
impl std::fmt::Display for DeclarationSubstitution<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(declaration-substitution (inputs")?;
        for (parameter, argument) in self.parameters.iter().zip(self.arguments) {
            write!(f, " (input {parameter} {argument})")?;
        }
        f.write_str(") (frames")?;
        for frame in self.frames {
            f.write_str(" (frame")?;
            for sort in frame {
                write!(f, " {sort}")?;
            }
            f.write_str(")")?;
        }
        f.write_str("))")
    }
}
impl<'a> DeclarationSubstitution<'a> {
    pub fn new(
        ids: &[DeclarationId],
        domains: &[Term],
        arguments: &'a [Term],
        frames: &'a [Vec<Sort>],
    ) -> Result<Self, TypeError> {
        Self::from_parameters(binders(ids, domains)?, domains, arguments, frames)
    }
    fn from_parameters(
        parameters: Vec<DeclarationParameter>,
        domains: &[Term],
        arguments: &'a [Term],
        frames: &'a [Vec<Sort>],
    ) -> Result<Self, TypeError> {
        if parameters.len() != arguments.len() {
            return Err(TypeError::ArgumentCount {
                expected: parameters.len(),
                actual: arguments.len(),
            });
        }
        for argument in arguments {
            argument.check_scope(frames)?;
        }
        for (index, (parameter, argument)) in parameters.iter().zip(arguments).enumerate() {
            let expected = if parameter.sort == Sort::Type {
                Sort::Type
            } else {
                let domain = domains[index].instantiate_domain_checked_parameters_in_scope(
                    &parameters,
                    arguments,
                    frames,
                )?;
                if domain == Term::Primitive(Primitive::Type) {
                    Sort::Type
                } else {
                    Sort::ComptimeValue
                }
            };
            if expected != argument.sort() {
                return Err(TypeError::WrongSort {
                    expected,
                    actual: argument.sort(),
                });
            }
        }
        Ok(Self {
            parameters,
            arguments,
            frames,
        })
    }
    pub fn bounded(
        ids: &[DeclarationId],
        domains: &[Term],
        arguments: &'a [Term],
        frames: &'a [Vec<Sort>],
        fuel: &mut usize,
    ) -> Result<Self, crate::type_computations::Error> {
        use crate::type_computations::{Error, charge_parameter_substitution, charge_term};
        *fuel = fuel
            .checked_sub(ids.len().saturating_add(frames.len()))
            .ok_or(Error::Budget)?;
        for frame in frames {
            *fuel = fuel.checked_sub(frame.len()).ok_or(Error::Budget)?;
        }
        for domain in domains {
            charge_term(domain, fuel)?;
        }
        for argument in arguments {
            charge_term(argument, fuel)?;
        }
        let parameters = binders(ids, domains).map_err(Error::Scope)?;
        for (index, parameter) in parameters.iter().enumerate() {
            if parameter.sort == Sort::ComptimeValue {
                if let Some(domain) = domains.get(index) {
                    charge_parameter_substitution(domain, &parameters, arguments, fuel)?;
                }
            }
        }
        Self::from_parameters(parameters, domains, arguments, frames).map_err(Error::Scope)
    }
    pub fn instantiate(&self, term: &Term) -> Result<Term, TypeError> {
        term.instantiate_domain_checked_parameters_in_scope(
            &self.parameters,
            self.arguments,
            self.frames,
        )
    }
    pub fn instantiate_predicate(
        &self,
        predicate: &crate::program::TypePredicate,
    ) -> Result<crate::program::TypePredicate, TypeError> {
        use crate::program::{TraitRequirement, TypePredicate as P};
        Ok(match predicate {
            P::Bool(value) => P::Bool(*value),
            P::Trait(requirement) => P::Trait(TraitRequirement {
                interface: requirement.interface,
                subject: self.instantiate(&requirement.subject)?,
                interface_arguments: requirement
                    .interface_arguments
                    .iter()
                    .map(|ty| self.instantiate(ty))
                    .collect::<Result<_, _>>()?,
            }),
            P::Equal(a, b) => P::Equal(self.instantiate(a)?, self.instantiate(b)?),
            P::Not(inner) => P::Not(Box::new(self.instantiate_predicate(inner)?)),
            P::And(parts) | P::Or(parts) => {
                let parts = parts
                    .iter()
                    .map(|p| self.instantiate_predicate(p))
                    .collect::<Result<_, _>>()?;
                if matches!(predicate, P::And(_)) {
                    P::And(parts)
                } else {
                    P::Or(parts)
                }
            }
        })
    }
    pub fn instantiate_predicate_bounded(
        &self,
        predicate: &crate::program::TypePredicate,
        fuel: &mut usize,
    ) -> Result<crate::program::TypePredicate, crate::type_computations::Error> {
        crate::copy_budget::charge_predicate_substitution(
            predicate,
            &self.parameters,
            self.arguments,
            fuel,
        )
        .map_err(|_| crate::type_computations::Error::Budget)?;
        self.instantiate_predicate(predicate)
            .map_err(crate::type_computations::Error::Scope)
    }
    pub fn instantiate_interface_bounded(
        &self,
        interface: &crate::program::InterfaceInstance,
        fuel: &mut usize,
    ) -> Result<crate::program::InterfaceInstance, crate::type_computations::Error> {
        Ok(crate::program::InterfaceInstance {
            declaration: interface.declaration,
            arguments: interface
                .arguments
                .iter()
                .map(|ty| self.instantiate_bounded(ty, fuel))
                .collect::<Result<_, _>>()?,
        })
    }
    pub fn instantiate_requirement_bounded(
        &self,
        requirement: &crate::program::TraitRequirement,
        fuel: &mut usize,
    ) -> Result<crate::program::TraitRequirement, crate::type_computations::Error> {
        Ok(crate::program::TraitRequirement {
            interface: requirement.interface,
            subject: self.instantiate_bounded(&requirement.subject, fuel)?,
            interface_arguments: requirement
                .interface_arguments
                .iter()
                .map(|ty| self.instantiate_bounded(ty, fuel))
                .collect::<Result<_, _>>()?,
        })
    }
    pub fn instantiate_bounded(
        &self,
        term: &Term,
        fuel: &mut usize,
    ) -> Result<Term, crate::type_computations::Error> {
        crate::type_computations::charge_parameter_substitution(
            term,
            &self.parameters,
            self.arguments,
            fuel,
        )?;
        self.instantiate(term)
            .map_err(crate::type_computations::Error::Scope)
    }
}

/// Completed declaration inputs with checked domains. Only this evidence permits
/// a definition-side value binder to become Type when its actual domain is Type.
/// Strict mixed-sort substitution remains available for fixed-sort frames.
pub struct AdmittedArguments<'a> {
    parameters: Vec<DeclarationParameter>,
    arguments: &'a [Term],
}
impl std::fmt::Display for AdmittedArguments<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(admitted-arguments")?;
        for (parameter, argument) in self.parameters.iter().zip(self.arguments) {
            write!(f, " (input {parameter} {argument})")?;
        }
        f.write_str(")")
    }
}
impl AdmittedArguments<'_> {
    pub fn instantiate(
        &self,
        term: &Term,
        fuel: &mut usize,
    ) -> Result<Term, crate::type_computations::Error> {
        crate::type_computations::charge_parameter_substitution(
            term,
            &self.parameters,
            self.arguments,
            fuel,
        )?;
        term.instantiate_admitted_parameters(&self.parameters, self.arguments)
            .map_err(crate::type_computations::Error::Scope)
    }
    pub fn instantiate_requirement(
        &self,
        requirement: &crate::program::TraitRequirement,
        fuel: &mut usize,
    ) -> Result<crate::program::TraitRequirement, crate::type_computations::Error> {
        Ok(crate::program::TraitRequirement {
            interface: requirement.interface,
            subject: self.instantiate(&requirement.subject, fuel)?,
            interface_arguments: requirement
                .interface_arguments
                .iter()
                .map(|term| self.instantiate(term, fuel))
                .collect::<Result<_, _>>()?,
        })
    }
    pub fn instantiate_predicate(
        &self,
        predicate: &crate::program::TypePredicate,
        fuel: &mut usize,
    ) -> Result<crate::program::TypePredicate, crate::type_computations::Error> {
        self.predicate(predicate, fuel, 0)
    }
    fn predicate(
        &self,
        predicate: &crate::program::TypePredicate,
        fuel: &mut usize,
        depth: usize,
    ) -> Result<crate::program::TypePredicate, crate::type_computations::Error> {
        use crate::program::TypePredicate as P;
        use crate::type_computations::Error;
        if depth >= 128 {
            return Err(Error::Budget);
        }
        *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
        Ok(match predicate {
            P::Bool(value) => P::Bool(*value),
            P::Trait(requirement) => P::Trait(self.instantiate_requirement(requirement, fuel)?),
            P::Equal(a, b) => P::Equal(self.instantiate(a, fuel)?, self.instantiate(b, fuel)?),
            P::Not(inner) => P::Not(Box::new(self.predicate(inner, fuel, depth + 1)?)),
            P::And(parts) | P::Or(parts) => {
                let parts = parts
                    .iter()
                    .map(|part| self.predicate(part, fuel, depth + 1))
                    .collect::<Result<_, _>>()?;
                if matches!(predicate, P::And(_)) {
                    P::And(parts)
                } else {
                    P::Or(parts)
                }
            }
        })
    }
}

pub fn admit_with_evidence_in<'a>(
    parameters: &[DeclarationId],
    domains: &[Term],
    arguments: &'a [Term],
    fuel: &mut usize,
    layout: impl FnMut(&Term, Option<u32>, &mut usize) -> Result<Vec<Term>, AdmissionError>,
    declared: impl FnMut(&Term, &Term, &mut usize) -> bool,
    target: Option<crate::target::TargetData>,
) -> Result<AdmittedArguments<'a>, AdmissionError> {
    admit_with_collections_in(
        parameters,
        domains,
        arguments,
        fuel,
        layout,
        declared,
        |_, _| Ok(None),
        target,
    )
}

fn admit_with_collections_in<'a>(
    parameters: &[DeclarationId],
    domains: &[Term],
    arguments: &'a [Term],
    fuel: &mut usize,
    mut layout: impl FnMut(&Term, Option<u32>, &mut usize) -> Result<Vec<Term>, AdmissionError>,
    mut declared: impl FnMut(&Term, &Term, &mut usize) -> bool,
    mut collection: impl FnMut(&Term, &mut usize) -> Result<Option<Term>, AdmissionError>,
    target: Option<crate::target::TargetData>,
) -> Result<AdmittedArguments<'a>, AdmissionError> {
    *fuel = fuel
        .checked_sub(parameters.len())
        .ok_or(AdmissionError::Budget)?;
    for domain in domains {
        crate::type_computations::charge_term(domain, fuel)?;
    }
    let parameters = binders(parameters, domains).map_err(|e| e.to_string())?;
    if parameters.len() != arguments.len() {
        return Err("nominal argument count mismatch".into());
    }
    for argument in arguments {
        crate::type_computations::charge_term(argument, fuel)?;
        argument.check_scope(&[]).map_err(|e| e.to_string())?;
    }
    let mut ready = BTreeSet::new();
    let mut admitted_parameters = Vec::new();
    let mut admitted_arguments = Vec::new();
    while ready.len() != parameters.len() {
        let before = ready.len();
        for (index, parameter) in parameters.iter().enumerate() {
            *fuel = fuel.checked_sub(1).ok_or(AdmissionError::Budget)?;
            if ready.contains(&parameter.declaration) {
                continue;
            }
            let domain = domains
                .get(index)
                .unwrap_or(&Term::Primitive(Primitive::Type));
            crate::type_computations::charge_term(domain, fuel)?;
            let references = domain.rigid_parameters().map_err(|e| e.to_string())?;
            *fuel = fuel
                .checked_sub(parameters.len())
                .ok_or(AdmissionError::Budget)?;
            if parameters
                .iter()
                .any(|p| references.contains_key(&p.declaration) && !ready.contains(&p.declaration))
            {
                continue;
            }
            crate::type_computations::charge_parameter_substitution(
                domain,
                &admitted_parameters,
                &admitted_arguments,
                fuel,
            )?;
            let domain = domain
                .instantiate_admitted_parameters(&admitted_parameters, &admitted_arguments)
                .map_err(|e| e.to_string())?;
            let argument = &arguments[index];
            let expected = if domain == Term::Primitive(Primitive::Type) {
                Sort::Type
            } else {
                Sort::ComptimeValue
            };
            if argument.sort() != expected {
                return Err("nominal argument sort mismatch".into());
            }
            if expected != Sort::Type {
                validate_value(
                    argument,
                    &domain,
                    fuel,
                    &mut layout,
                    &mut declared,
                    &mut collection,
                    0,
                    target,
                )?;
            }
            crate::type_computations::charge_term(argument, fuel)?;
            admitted_parameters.push(*parameter);
            admitted_arguments.push(argument.clone());
            ready.insert(parameter.declaration);
        }
        if ready.len() == before {
            return Err("nominal input domains have a cyclic dependency".into());
        }
    }
    Ok(AdmittedArguments {
        parameters,
        arguments,
    })
}

/// The caller has already checked scope and charged the input tree. Symbolic
/// leaves require independent declaration-domain evidence supplied by `declared`.
pub(crate) fn validate_symbolic_value(
    argument: &Term,
    domain: &Term,
    fuel: &mut usize,
    layout: impl FnMut(&Term, Option<u32>, &mut usize) -> Result<Vec<Term>, AdmissionError>,
    declared: impl FnMut(&Term, &Term, &mut usize) -> bool,
) -> Result<(), AdmissionError> {
    validate_symbolic_value_in(argument, domain, fuel, layout, declared, None)
}

pub(crate) fn validate_symbolic_value_in(
    argument: &Term,
    domain: &Term,
    fuel: &mut usize,
    mut layout: impl FnMut(&Term, Option<u32>, &mut usize) -> Result<Vec<Term>, AdmissionError>,
    mut declared: impl FnMut(&Term, &Term, &mut usize) -> bool,
    target: Option<crate::target::TargetData>,
) -> Result<(), AdmissionError> {
    validate_value(
        argument,
        domain,
        fuel,
        &mut layout,
        &mut declared,
        &mut |_, _| Ok(None),
        0,
        target,
    )
}

fn validate_value(
    argument: &Term,
    domain: &Term,
    fuel: &mut usize,
    layout: &mut impl FnMut(&Term, Option<u32>, &mut usize) -> Result<Vec<Term>, AdmissionError>,
    declared: &mut impl FnMut(&Term, &Term, &mut usize) -> bool,
    collection: &mut impl FnMut(&Term, &mut usize) -> Result<Option<Term>, AdmissionError>,
    depth: usize,
    target: Option<crate::target::TargetData>,
) -> Result<(), AdmissionError> {
    if depth >= 128 {
        return Err("nominal value nesting exceeds admission depth".into());
    }
    *fuel = fuel.checked_sub(1).ok_or(AdmissionError::Budget)?;
    if matches!(
        argument,
        Term::Rigid {
            sort: Sort::ComptimeValue,
            ..
        }
    ) {
        return if declared(argument, domain, fuel) {
            Ok(())
        } else {
            Err("nominal value index lacks matching completed domain evidence".into())
        };
    }
    if let Term::OptionalValue { ty, value } = argument {
        let Term::Optional(payload) = domain else {
            return Err("optional value index requires an optional domain".into());
        };
        if ty.as_ref() != domain {
            return Err("optional value index domain mismatch".into());
        }
        return match value {
            Some(value) => validate_value(
                value,
                payload,
                fuel,
                layout,
                declared,
                collection,
                depth + 1,
                target,
            ),
            None => Ok(()),
        };
    }
    if let Term::ErrorValue { value, .. } = argument {
        let payload = crate::error_results::frozen_payload_domain(argument, domain, fuel).map_err(
            |error| {
                if error == crate::error_results::Error::Budget {
                    AdmissionError::Budget
                } else {
                    AdmissionError::from("error value index domain or branch mismatch")
                }
            },
        )?;
        return validate_value(
            value,
            payload,
            fuel,
            layout,
            declared,
            collection,
            depth + 1,
            target,
        );
    }
    if matches!(domain, Term::NormalForm { .. })
        && matches!(argument, Term::TypeListValue(_) | Term::NominalValue { .. })
    {
        if let Some(element) = collection(domain, fuel)? {
            let fields =
                crate::normal_forms::list_value_payload(domain, argument).ok_or_else(|| {
                    AdmissionError::from("collection value domain or payload mismatch")
                })?;
            *fuel = fuel
                .checked_sub(fields.len())
                .ok_or(AdmissionError::Budget)?;
            for value in fields {
                validate_value(
                    value,
                    &element,
                    fuel,
                    layout,
                    declared,
                    collection,
                    depth + 1,
                    target,
                )?;
            }
            return Ok(());
        }
    }
    if let Term::NominalValue {
        ty,
        variant,
        fields,
    } = argument
    {
        if *domain == Term::Primitive(Primitive::Object)
            && ty.as_ref() == domain
            && variant.is_none()
        {
            for entry in fields {
                let (_, domain, value) = crate::objects::entry_parts(entry)
                    .ok_or_else(|| AdmissionError::from("invalid Object entry encoding"))?;
                crate::type_computations::charge_term(domain, fuel)?;
                validate_value(
                    value,
                    domain,
                    fuel,
                    layout,
                    declared,
                    collection,
                    depth + 1,
                    target,
                )?;
            }
            return Ok(());
        }
        if !matches!(domain, Term::Nominal { .. }) || ty.as_ref() != domain {
            return Err("nominal value index has a different nominal identity".into());
        }
        let expected = layout(domain, *variant, fuel)?;
        if expected.len() != fields.len() {
            return Err("nominal value index payload arity mismatch".into());
        }
        for (value, domain) in fields.iter().zip(&expected) {
            crate::type_computations::charge_term(domain, fuel).map_err(AdmissionError::from)?;
            validate_value(
                value,
                domain,
                fuel,
                layout,
                declared,
                collection,
                depth + 1,
                target,
            )?;
        }
        return Ok(());
    }
    if let (Term::TupleValue(values), Term::NormalForm { .. }) = (argument, domain) {
        // Only the caller's registered layout evidence may expose a tuple
        // normal form. Its body and its spelling do not establish this view.
        let domains = layout(domain, None, fuel)?;
        if values.len() != domains.len() {
            return Err("nominal tuple index arity mismatch".into());
        }
        for (value, domain) in values.iter().zip(&domains) {
            crate::type_computations::charge_term(domain, fuel).map_err(AdmissionError::from)?;
            validate_value(
                value,
                domain,
                fuel,
                layout,
                declared,
                collection,
                depth + 1,
                target,
            )?;
        }
        return Ok(());
    }
    if let (Term::TupleValue(values), Term::Tuple(domains)) = (argument, domain) {
        if values.len() != domains.len() {
            return Err("nominal tuple index arity mismatch".into());
        }
        for (value, domain) in values.iter().zip(domains) {
            validate_value(
                value,
                domain,
                fuel,
                layout,
                declared,
                collection,
                depth + 1,
                target,
            )?;
        }
        return Ok(());
    }
    if let (Term::RecordValue(values), Term::Record(domains)) = (argument, domain) {
        if values.len() != domains.len() {
            return Err("nominal record index arity mismatch".into());
        }
        let mut names = BTreeSet::new();
        for (name, _) in domains {
            *fuel = fuel.checked_sub(1).ok_or(AdmissionError::Budget)?;
            if !names.insert(name) {
                return Err("duplicate nominal record domain field".into());
            }
        }
        let mut seen = BTreeSet::new();
        for (name, value) in values {
            *fuel = fuel
                .checked_sub(domains.len())
                .ok_or(AdmissionError::Budget)?;
            if !seen.insert(name) {
                return Err("duplicate nominal record index field".into());
            }
            let domain = domains
                .iter()
                .find_map(|(field, domain)| (field == name).then_some(domain))
                .ok_or_else(|| AdmissionError::from("unknown nominal record index field"))?;
            validate_value(
                value,
                domain,
                fuel,
                layout,
                declared,
                collection,
                depth + 1,
                target,
            )?;
        }
        return Ok(());
    }
    if *domain == Term::Primitive(Primitive::Type) && argument.sort() == Sort::Type {
        return Ok(());
    }
    if let (Term::ClosureContextValue(fields), Term::ClosureContextType) = (argument, domain) {
        for field in fields {
            validate_value(
                field,
                &Term::Primitive(Primitive::Type),
                fuel,
                layout,
                declared,
                collection,
                depth + 1,
                target,
            )?;
        }
        return Ok(());
    }
    if let (Term::Integer(value), Term::Primitive(primitive)) = (argument, domain) {
        if let Some((signed, width)) = target.map_or_else(
            || primitive.fixed_integer_layout(),
            |target| target.integer_layout(*primitive),
        ) {
            let fits = if signed {
                value.to_signed(width).is_ok()
            } else {
                value.to_unsigned(width).is_ok()
            };
            if !fits {
                return Err("nominal integer index is outside its declared domain".into());
            }
            return Ok(());
        }
    }
    if argument.scalar_index_domain().map(Term::Primitive).as_ref() != Some(domain) {
        return Err("nominal value index lacks matching completed domain evidence".into());
    }
    Ok(())
}
