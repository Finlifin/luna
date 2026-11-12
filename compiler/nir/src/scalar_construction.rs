//! Construct a scalar source expression from a completed value and its domain.
//!
//! This is a representation boundary, not permission to cross stages. Callers
//! still choose Splice, check runtime storage, and establish code interfaces.
//! In particular, exact Integer/Float and text expressions do not acquire a
//! runtime representation or a backing region merely by being constructed.
use crate::{
    Primitive, Term,
    program::{Expr, ExprKind, Literal, SourceRange},
    target::TargetData,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Budget,
    SizeLimit,
    Invalid(&'static str),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Budget => f.write_str("scalar construction budget exhausted"),
            Self::SizeLimit => {
                f.write_str("scalar construction representation size limit exceeded")
            }
            Self::Invalid(message) => f.write_str(message),
        }
    }
}

fn runtime_error(error: comptime_runtime::Error) -> Error {
    match error {
        comptime_runtime::Error::BudgetExhausted => Error::Budget,
        comptime_runtime::Error::SizeLimit => Error::SizeLimit,
        _ => Error::Invalid("scalar value cannot be represented in its declared domain"),
    }
}

/// Keep the supplied domain, including target-sized integer identity. Never
/// infer a new domain from the payload or unfold a nominal/normal-form head.
pub fn expression(
    value: &Term,
    domain: &Term,
    target: Option<TargetData>,
    source: SourceRange,
    fuel: &mut usize,
) -> Result<Expr, Error> {
    *fuel = fuel.checked_sub(1).ok_or(Error::Budget)?;
    let Term::Primitive(primitive) = domain else {
        return Err(Error::Invalid(
            "scalar construction requires a primitive domain",
        ));
    };
    if value.scalar_index_domain().is_none() {
        return Err(Error::Invalid(
            "scalar construction requires a completed scalar value",
        ));
    }
    crate::comptime_values::validate_in(value, domain, target, fuel).map_err(
        |error| match error {
            crate::comptime_values::Error::Budget => Error::Budget,
            crate::comptime_values::Error::Invalid(message) => Error::Invalid(message),
        },
    )?;
    let mut budget = comptime_runtime::Budget {
        steps: *fuel,
        max_limbs: 4096,
    };
    let result = (|| {
        Ok(Some(match value {
            Term::Bool(value) => Literal::Bool(*value),
            Term::Char(value) => Literal::Char(*value),
            Term::Symbol(value) => Literal::Symbol(value.to_string()),
            Term::String(value) => Literal::String(value.to_string()),
            Term::Float16(value) => Literal::Float16(*value),
            Term::BFloat16(value) => Literal::BFloat16(*value),
            Term::Float32(value) => Literal::Float32(*value),
            Term::Float64(value) => Literal::Float64(*value),
            Term::BitVec(value) => Literal::BitVec {
                width: value.width(),
                words: value.words().to_vec(),
            },
            Term::Float(value) => {
                Literal::Float(value.to_literal(&mut budget).map_err(runtime_error)?)
            }
            Term::Integer(value) if *primitive == Primitive::Integer => {
                Literal::Integer(value.to_hex_literal(&mut budget).map_err(runtime_error)?)
            }
            Term::Integer(value) => {
                let (signed, width) = target
                    .map_or_else(
                        || primitive.fixed_integer_layout(),
                        |target| target.integer_layout(*primitive),
                    )
                    .ok_or(Error::Invalid(
                        "scalar integer construction requires explicit target layout",
                    ))?;
                if signed {
                    Literal::SignedInteger(value.to_signed(width).map_err(runtime_error)?)
                } else {
                    Literal::UnsignedInteger(value.to_unsigned(width).map_err(runtime_error)?)
                }
            }
            Term::Unit => return Ok(None),
            _ => {
                return Err(Error::Invalid(
                    "scalar construction requires a completed scalar value",
                ));
            }
        }))
    })();
    *fuel = budget.steps;
    let kind = match result? {
        Some(literal) => ExprKind::Literal(literal),
        None => ExprKind::Unit,
    };
    Ok(Expr {
        source,
        ty: domain.clone(),
        kind,
    })
}

#[cfg(test)]
mod tests;
