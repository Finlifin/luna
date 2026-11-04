//! Shared, domain-directed admission of immutable comptime descriptions.
//! This checks scoped representation membership. Declaration formation, runtime
//! resource rights, transfer permission and cacheability remain separate.
use crate::{Primitive, Sort, Term};
use std::{borrow::Cow, collections::BTreeSet};

/// Resource exhaustion is distinct from a disproved input domain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Budget,
    Invalid(String),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Budget => f.write_str("value admission resource budget exhausted"),
            Self::Invalid(message) => f.write_str(message),
        }
    }
}
impl std::error::Error for Error {}
impl From<&str> for Error {
    fn from(message: &str) -> Self {
        Self::Invalid(message.into())
    }
}
impl From<String> for Error {
    fn from(message: String) -> Self {
        Self::Invalid(message)
    }
}
impl From<crate::type_computations::Error> for Error {
    fn from(error: crate::type_computations::Error) -> Self {
        match error {
            crate::type_computations::Error::Budget => Self::Budget,
            error => Self::Invalid(error.to_string()),
        }
    }
}
impl From<Error> for String {
    fn from(error: Error) -> Self {
        error.to_string()
    }
}

/// Receiving-world queries supply representation evidence without executing
/// source functions. Collection identities are registered by the caller; a
/// frozen type tag or a particular opaque-head shape is not authorization.
/// Query work and owned descriptions must consume the shared fuel.
pub struct DomainQueries<'a, 'signature> {
    pub layout: &'a mut dyn FnMut(&Term, Option<u32>, &mut usize) -> Result<Vec<Term>, Error>,
    pub declared: &'a mut dyn FnMut(&Term, &Term, &mut usize) -> bool,
    pub collection: &'a mut dyn FnMut(&Term, &mut usize) -> Result<Option<Term>, Error>,
    pub callable: &'a mut dyn FnMut(
        crate::CallableValueRef<'_>,
        &[Vec<Sort>],
        &mut usize,
    ) -> Result<Option<Cow<'signature, Term>>, Error>,
    pub target: Option<crate::target::TargetData>,
}

pub fn validate(
    argument: &Term,
    domain: &Term,
    frames: &[Vec<Sort>],
    fuel: &mut usize,
    queries: &mut DomainQueries<'_, '_>,
) -> Result<(), Error> {
    crate::type_computations::charge_term(argument, fuel)?;
    crate::type_computations::charge_term(domain, fuel)?;
    argument.check_scope(frames).map_err(|e| e.to_string())?;
    domain.check_scope(frames).map_err(|e| e.to_string())?;
    if domain.sort() != Sort::Type {
        return Err("value domain is not a Type".into());
    }
    validate_value(argument, domain, fuel, queries, frames, 0)
}

fn validate_value(
    argument: &Term,
    domain: &Term,
    fuel: &mut usize,
    queries: &mut DomainQueries<'_, '_>,
    frames: &[Vec<Sort>],
    depth: usize,
) -> Result<(), Error> {
    if depth >= 128 {
        *fuel = 0;
        return Err(Error::Budget);
    }
    *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
    if matches!(
        argument,
        Term::Rigid {
            sort: Sort::ComptimeValue,
            ..
        } | Term::Bound {
            sort: Sort::ComptimeValue,
            ..
        }
    ) {
        return if *domain != Term::Primitive(Primitive::Type)
            && (queries.declared)(argument, domain, fuel)
        {
            Ok(())
        } else {
            Err("nominal value index lacks matching completed domain evidence".into())
        };
    }
    if let Some(callable) = crate::CallableValueRef::from_term(argument) {
        if !matches!(
            domain,
            Term::Function { .. }
                | Term::ComptimeFunction(_)
                | Term::NormalFormFunction(..)
                | Term::UnsafeFunction(_)
        ) {
            return Err("callable value requires a callable domain".into());
        }
        let signature = (queries.callable)(callable, frames, fuel)?.ok_or_else(|| {
            Error::from("callable value lacks receiving-world signature evidence")
        })?;
        crate::type_computations::charge_term(&signature, fuel)?;
        signature.check_scope(frames).map_err(|e| e.to_string())?;
        return if signature.as_ref() == domain {
            Ok(())
        } else {
            Err("callable value signature differs from its declared domain".into())
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
            Some(value) => validate_value(value, payload, fuel, queries, frames, depth + 1),
            None => Ok(()),
        };
    }
    if let Term::ErrorValue { value, .. } = argument {
        let payload = crate::error_results::frozen_payload_domain(argument, domain, fuel).map_err(
            |error| {
                if error == crate::error_results::Error::Budget {
                    Error::Budget
                } else {
                    Error::from("error value index domain or branch mismatch")
                }
            },
        )?;
        return validate_value(value, payload, fuel, queries, frames, depth + 1);
    }
    if matches!(domain, Term::NormalForm { .. }) && matches!(argument, Term::NominalValue { .. }) {
        if let Some(element) = (queries.collection)(domain, fuel)? {
            crate::type_computations::charge_term(&element, fuel)?;
            element.check_scope(frames).map_err(|e| e.to_string())?;
            if element.sort() != Sort::Type {
                return Err("collection element domain is not a Type".into());
            }
            let fields = crate::normal_forms::list_value_payload(domain, argument)
                .ok_or_else(|| Error::from("collection value domain or payload mismatch"))?;
            *fuel = fuel.checked_sub(fields.len()).ok_or(Error::Budget)?;
            for value in fields {
                validate_value(value, &element, fuel, queries, frames, depth + 1)?;
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
                    .ok_or_else(|| Error::from("invalid Object entry encoding"))?;
                crate::type_computations::charge_term(domain, fuel)?;
                validate_value(value, domain, fuel, queries, frames, depth + 1)?;
            }
            return Ok(());
        }
        if !matches!(domain, Term::Nominal { .. }) || ty.as_ref() != domain {
            return Err("nominal value index has a different nominal identity".into());
        }
        let expected = (queries.layout)(domain, *variant, fuel)?;
        if expected.len() != fields.len() {
            return Err("nominal value index payload arity mismatch".into());
        }
        for (value, domain) in fields.iter().zip(&expected) {
            crate::type_computations::charge_term(domain, fuel).map_err(Error::from)?;
            validate_value(value, domain, fuel, queries, frames, depth + 1)?;
        }
        return Ok(());
    }
    if let (Term::TupleValue(values), Term::NormalForm { .. }) = (argument, domain) {
        // Only the caller's registered layout evidence may expose a tuple
        // normal form. Its body and its spelling do not establish this view.
        let domains = (queries.layout)(domain, None, fuel)?;
        if values.len() != domains.len() {
            return Err("nominal tuple index arity mismatch".into());
        }
        for (value, domain) in values.iter().zip(&domains) {
            crate::type_computations::charge_term(domain, fuel).map_err(Error::from)?;
            validate_value(value, domain, fuel, queries, frames, depth + 1)?;
        }
        return Ok(());
    }
    if let (Term::TupleValue(values), Term::Tuple(domains)) = (argument, domain) {
        if values.len() != domains.len() {
            return Err("nominal tuple index arity mismatch".into());
        }
        for (value, domain) in values.iter().zip(domains) {
            validate_value(value, domain, fuel, queries, frames, depth + 1)?;
        }
        return Ok(());
    }
    if let (Term::RecordValue(values), Term::Record(domains)) = (argument, domain) {
        if values.len() != domains.len() {
            return Err("nominal record index arity mismatch".into());
        }
        let mut names = BTreeSet::new();
        for (name, _) in domains {
            *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
            if !names.insert(name) {
                return Err("duplicate nominal record domain field".into());
            }
        }
        let mut seen = BTreeSet::new();
        for (name, value) in values {
            *fuel = fuel.checked_sub(domains.len()).ok_or(Error::Budget)?;
            if !seen.insert(name) {
                return Err("duplicate nominal record index field".into());
            }
            let domain = domains
                .iter()
                .find_map(|(field, domain)| (field == name).then_some(domain))
                .ok_or_else(|| Error::from("unknown nominal record index field"))?;
            validate_value(value, domain, fuel, queries, frames, depth + 1)?;
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
                queries,
                frames,
                depth + 1,
            )?;
        }
        return Ok(());
    }
    if let (Term::Integer(value), Term::Primitive(primitive)) = (argument, domain) {
        if let Some((signed, width)) = queries.target.map_or_else(
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
