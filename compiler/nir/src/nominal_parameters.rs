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
    let parameters = binders(parameters, domains).map_err(|e| e.to_string())?;
    if parameters.len() != arguments.len() {
        return Err("nominal argument count mismatch".into());
    }
    // This boundary validates already supplied values, not source evaluation
    // order. Captured parent inputs may occupy a suffix after own inputs whose
    // domains use them. Check the whole input vector before substituting it.
    for (parameter, argument) in parameters.iter().zip(arguments) {
        crate::type_computations::charge_term(argument, fuel).map_err(AdmissionError::from)?;
        argument.check_scope(&[]).map_err(|e| e.to_string())?;
        if parameter.sort != argument.sort() {
            return Err("nominal argument sort mismatch".into());
        }
    }
    for (index, (parameter, argument)) in parameters.iter().zip(arguments).enumerate() {
        if parameter.sort == Sort::Type {
            continue;
        }
        let domain = crate::type_computations::instantiate_parameters_bounded(
            &domains[index],
            &parameters,
            arguments,
            &[],
            fuel,
        )
        .map_err(AdmissionError::from)?;
        validate_value(
            argument,
            &domain,
            fuel,
            &mut layout,
            &mut declared,
            0,
            target,
        )?;
    }
    Ok(())
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
        return validate_value(value, payload, fuel, layout, declared, depth + 1, target);
    }
    if let Term::NominalValue {
        ty,
        variant,
        fields,
    } = argument
    {
        if !matches!(domain, Term::Nominal { .. }) || ty.as_ref() != domain {
            return Err("nominal value index has a different nominal identity".into());
        }
        let expected = layout(domain, *variant, fuel)?;
        if expected.len() != fields.len() {
            return Err("nominal value index payload arity mismatch".into());
        }
        for (value, domain) in fields.iter().zip(&expected) {
            crate::type_computations::charge_term(domain, fuel).map_err(AdmissionError::from)?;
            validate_value(value, domain, fuel, layout, declared, depth + 1, target)?;
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
            validate_value(value, domain, fuel, layout, declared, depth + 1, target)?;
        }
        return Ok(());
    }
    if let (Term::TupleValue(values), Term::Tuple(domains)) = (argument, domain) {
        if values.len() != domains.len() {
            return Err("nominal tuple index arity mismatch".into());
        }
        for (value, domain) in values.iter().zip(domains) {
            validate_value(value, domain, fuel, layout, declared, depth + 1, target)?;
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
            validate_value(value, domain, fuel, layout, declared, depth + 1, target)?;
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
