//! Exact integer spelling formed only once a concrete target domain is known.
use crate::{
    Primitive,
    program::{Expr, ExprKind, Literal},
    target::TargetData,
};
use comptime_runtime::{Budget, Integer};

/// Formation diagnostics retain both language identity and target layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormationError {
    pub source: crate::program::SourceRange,
    pub target: crate::Term,
    pub layout: Option<TargetData>,
    pub cause: comptime_runtime::Error,
}

impl std::fmt::Display for FormationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let reason = if self.layout.is_none()
            && matches!(
                self.target,
                crate::Term::Primitive(Primitive::Usize | Primitive::Isize)
            )
            && self.cause == comptime_runtime::Error::InvalidLiteral
        {
            "integer formation requires explicit target pointer-width".to_owned()
        } else {
            format!("integer formation: {}", self.cause)
        };
        write!(
            f,
            "(integer-formation-error {} (target {}) (layout ",
            self.source, self.target
        )?;
        match self.layout {
            Some(layout) => write!(f, "{layout}")?,
            None => write!(f, "unknown")?,
        }
        write!(f, ") (reason {}))", Literal::String(reason))
    }
}

impl std::error::Error for FormationError {}

/// Form an already selected scalar domain without inferring its type identity.
/// Platform integers require explicit target data; this never falls back to the
/// compiler process's pointer width. BitVec formation has a separate contract.
pub fn form(
    spelling: &str,
    negative: bool,
    domain: Primitive,
    target: Option<TargetData>,
    budget: &mut Budget,
) -> Result<Literal, comptime_runtime::Error> {
    let mut value = Integer::parse_literal(spelling, budget)?;
    if negative {
        value = value.negated();
    }
    if domain == Primitive::Integer {
        return Ok(Literal::Integer(if negative {
            format!("-{spelling}")
        } else {
            spelling.into()
        }));
    }
    let (signed, width) = target
        .map_or_else(
            || domain.fixed_integer_layout(),
            |target| target.integer_layout(domain),
        )
        .ok_or(comptime_runtime::Error::InvalidLiteral)?;
    if signed {
        Ok(Literal::SignedInteger(value.to_signed(width)?))
    } else {
        Ok(Literal::UnsignedInteger(value.to_unsigned(width)?))
    }
}

/// Complete concrete annotations in a private specialization candidate. Rigid
/// annotations remain pending; IIR admission must reject any surviving node.
/// A failed node is unchanged, but earlier siblings may already be formed.
pub fn complete(
    expression: &mut Expr,
    target: Option<TargetData>,
    fuel: &mut usize,
) -> Result<(), String> {
    crate::visit::expression_mut(expression, fuel, |expr, fuel| {
        let ExprKind::Literal(Literal::PendingInteger { spelling, negative }) = &expr.kind else {
            return Ok(());
        };
        let crate::Term::Primitive(domain) = expr.ty else {
            return Ok(());
        };
        let mut budget = Budget {
            steps: *fuel,
            max_limbs: *fuel,
        };
        let result = form(spelling, *negative, domain, target, &mut budget);
        *fuel = budget.steps;
        expr.kind = ExprKind::Literal(result.map_err(|cause| {
            FormationError {
                source: expr.source,
                target: expr.ty.clone(),
                layout: target,
                cause,
            }
            .to_string()
        })?);
        Ok(())
    })
}
