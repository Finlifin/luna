//! Exact source-literal evidence, kept separate from its chosen runtime domain.
//! A formation is not a completed value or proof that a generic domain is legal.
use crate::{Primitive, program::Literal};
use comptime_runtime::{Budget, Error, decimal::DecimalLiteral};
use std::fmt;

/// A failed concrete formation, with the source and selected domain retained.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormationError {
    pub source: crate::program::SourceRange,
    pub target: crate::Term,
    pub cause: Error,
}

impl fmt::Display for FormationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match self.cause {
            Error::OutOfRange => "finite float literal exceeds the selected representation".into(),
            Error::InvalidLiteral => "invalid float spelling or target domain".into(),
            Error::BudgetExhausted => "float literal formation budget exhausted".into(),
            Error::SizeLimit => "float literal formation representation limit exceeded".into(),
            _ => self.cause.to_string(),
        };
        write!(
            f,
            "(float-formation-error {} (target {}) (reason {}))",
            self.source,
            self.target,
            Literal::String(reason)
        )
    }
}

impl std::error::Error for FormationError {}

#[derive(Clone, Debug)]
pub struct FloatFormation {
    exact: DecimalLiteral,
}

impl FloatFormation {
    pub fn parse(spelling: &str, negative: bool, budget: &mut Budget) -> Result<Self, Error> {
        let exact = DecimalLiteral::parse(spelling, budget)?;
        Ok(Self {
            exact: if negative { exact.negated() } else { exact },
        })
    }

    /// Choose a concrete domain without mutating or prematurely rounding the
    /// retained decimal. Every formation attempt consumes the caller's budget.
    pub fn form(&self, domain: Primitive, budget: &mut Budget) -> Result<Literal, Error> {
        match domain {
            Primitive::Float => Ok(Literal::Float(
                self.exact.exact_value(budget)?.to_literal(budget)?,
            )),
            Primitive::F16 => Ok(Literal::Float16(self.exact.to_f16_bits(budget)?)),
            Primitive::Bf16 => Ok(Literal::BFloat16(self.exact.to_bf16_bits(budget)?)),
            Primitive::F32 => Ok(Literal::Float32(self.exact.to_f32_bits(budget)?)),
            Primitive::F64 => Ok(Literal::Float64(self.exact.to_f64_bits(budget)?)),
            _ => Err(Error::InvalidLiteral),
        }
    }
}

impl fmt::Display for FloatFormation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(float-formation {})", self.exact)
    }
}

/// Complete only concrete domains. Generic definition-side rigids remain
/// evidence; executable admission must reject anything still pending.
pub fn complete(expression: &mut crate::program::Expr, fuel: &mut usize) -> Result<(), String> {
    crate::visit::expression_mut(expression, fuel, |expr, fuel| {
        let crate::program::ExprKind::Literal(Literal::PendingFloat { spelling, negative }) =
            &expr.kind
        else {
            return Ok(());
        };
        let crate::Term::Primitive(domain) = expr.ty else {
            return Ok(());
        };
        let mut budget = Budget {
            steps: *fuel,
            max_limbs: *fuel,
        };
        let result = FloatFormation::parse(spelling, *negative, &mut budget)
            .and_then(|formation| formation.form(domain, &mut budget));
        *fuel = budget.steps;
        expr.kind = crate::program::ExprKind::Literal(result.map_err(|cause| {
            FormationError {
                source: expr.source,
                target: expr.ty.clone(),
                cause,
            }
            .to_string()
        })?);
        Ok(())
    })
}
