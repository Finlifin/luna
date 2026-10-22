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

pub fn validate(value: &Term, domain: &Term, fuel: &mut usize) -> Result<(), Error> {
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
        if let (Term::Integer(value), Some((signed, width))) =
            (value, expected.fixed_integer_layout())
        {
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
