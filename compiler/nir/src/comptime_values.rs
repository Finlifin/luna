//! Structural boundary for captured comptime values. Abstract and aggregate
//! domain admission still belongs to specialization and interpretation.
use crate::{Primitive, Sort, Term};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Budget,
    Invalid(&'static str),
}
impl Error {
    pub fn message(self) -> &'static str {
        match self {
            Self::Budget => "captured comptime value validation budget exhausted",
            Self::Invalid(message) => message,
        }
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

/// Encode an already admitted list using the common frozen-value convention.
/// The producer must establish list layout authority, element domains and
/// scope before calling this representation boundary. Shape alone grants no
/// authority to treat an arbitrary opaque normal-form head as a list.
///
/// Keep the legacy compact Type payload canonical until all readers and
/// symbolic producers migrate together; never publish two identities for one
/// admitted value. Ordinary payloads retain their exact declared domain.
pub fn freeze_admitted_list(
    domain: &Term,
    fields: Vec<Term>,
    fuel: &mut usize,
) -> Result<Term, Error> {
    crate::type_computations::charge_term(domain, fuel).map_err(|_| Error::Budget)?;
    *fuel = fuel.checked_sub(fields.len()).ok_or(Error::Budget)?;
    let Term::NormalForm {
        sort: Sort::Type,
        arguments,
        ..
    } = domain
    else {
        return Err(Error::Invalid("admitted list has an invalid domain"));
    };
    let [element] = arguments.as_slice() else {
        return Err(Error::Invalid("admitted list requires one element domain"));
    };
    if element.sort() != Sort::Type {
        return Err(Error::Invalid("admitted list element domain is not Type"));
    }
    let types = *element == Term::Primitive(Primitive::Type);
    let sort = if types {
        Sort::Type
    } else {
        Sort::ComptimeValue
    };
    if fields.iter().any(|field| field.sort() != sort) {
        return Err(Error::Invalid("admitted list element has the wrong sort"));
    }
    Ok(if types {
        Term::TypeListValue(fields)
    } else {
        Term::NominalValue {
            ty: Box::new(domain.clone()),
            variant: None,
            fields,
        }
    })
}

pub fn validate(value: &Term, domain: &Term, fuel: &mut usize) -> Result<(), Error> {
    validate_in(value, domain, None, fuel)
}

/// Validate target-dependent scalar domains only with explicit target facts.
/// Aggregate/abstract domain admission remains the invocation's responsibility.
pub fn validate_in(
    value: &Term,
    domain: &Term,
    target: Option<crate::target::TargetData>,
    fuel: &mut usize,
) -> Result<(), Error> {
    crate::type_computations::charge_term(value, fuel).map_err(|_| Error::Budget)?;
    crate::type_computations::charge_term(domain, fuel).map_err(|_| Error::Budget)?;
    if value.sort() != Sort::ComptimeValue {
        return Err(Error::Invalid(
            "captured comptime value requires a value term",
        ));
    }
    if domain.sort() != Sort::Type || *domain == Term::Primitive(Primitive::Type) {
        return Err(Error::Invalid(
            "captured comptime value requires a non-Type value domain",
        ));
    }
    // Declaration parameters may remain symbolic, but invocation-local slots
    // cannot be smuggled into this immutable value representation.
    value.check_scope(&[]).map_err(|_| {
        Error::Invalid("captured comptime value contains an unbound invocation slot")
    })?;
    if matches!(value, Term::ClosureContextValue(_)) != matches!(domain, Term::ClosureContextType) {
        return Err(Error::Invalid(
            "captured closure context disagrees with its declared domain",
        ));
    }
    if let (Some(actual), Term::Primitive(expected)) = (value.scalar_index_domain(), domain) {
        let layout = match target {
            Some(target) => target.integer_layout(*expected),
            None => expected.fixed_integer_layout(),
        };
        if let (Term::Integer(value), Some((signed, width))) = (value, layout) {
            let fits = if signed {
                value.to_signed(width).is_ok()
            } else {
                value.to_unsigned(width).is_ok()
            };
            if !fits {
                return Err(Error::Invalid(
                    "captured comptime integer is outside its declared domain",
                ));
            }
        } else if actual != *expected {
            return Err(Error::Invalid(
                "captured comptime scalar disagrees with its declared domain",
            ));
        }
    }
    Ok(())
}
