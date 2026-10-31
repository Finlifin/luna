//! Declaration domains for callable clause inputs, independent of executable
//! parameters and bodies. Runtime instantiation completes dependent input domains.
use crate::{DeclarationId, DeclarationParameter, Term, TypeError};
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub struct Declaration {
    pub declaration: DeclarationId,
    pub parameters: Vec<DeclarationId>,
    pub domains: Vec<Term>,
}

impl Declaration {
    pub fn binders(&self) -> Result<Vec<DeclarationParameter>, TypeError> {
        crate::declaration_parameters(&self.parameters, &self.domains)
    }
}

/// Admit closed callable declaration inputs using their declared domains.
/// Body preconditions and type requirements are separate invocation checks.
pub fn validate_arguments(
    package: &crate::program::Package,
    parameters: &[DeclarationId],
    domains: &[Term],
    arguments: &[Term],
    fuel: &mut usize,
) -> Result<(), String> {
    crate::validate_declaration_arguments_with_collections_in(
        parameters,
        domains,
        arguments,
        fuel,
        |ty, variant, fuel| {
            crate::normal_forms::nominal_value_fields_budgeted(package, ty, variant, fuel)
                .ok_or_else(|| {
                    if *fuel == 0 {
                        crate::NominalAdmissionError::Budget
                    } else {
                        "callable input lacks validated layout evidence".into()
                    }
                })
        },
        |_, _, _| false,
        |domain, fuel| crate::normal_forms::list_element_budgeted(package, domain, fuel),
        package.target,
    )
    .map_err(|error| error.to_string())
}

impl fmt::Display for Declaration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "(function-parameters (declaration {}) (inputs",
            self.declaration
        )?;
        for parameter in &self.parameters {
            write!(f, " {parameter}")?;
        }
        f.write_str(") (domains")?;
        for domain in &self.domains {
            write!(f, " {domain}")?;
        }
        f.write_str("))")
    }
}

/// Check alpha-equivalence of implementation input domains before transporting
/// positional arguments to a method declared in another phase contribution.
/// This validates the input telescope only, not head or contract compatibility.
/// Missing domain metadata denotes the existing compact all-Type convention.
pub fn equivalent_prefix_domains(
    expected: &[DeclarationId],
    actual: &[DeclarationId],
    domains: &std::collections::BTreeMap<DeclarationId, Term>,
    fuel: &mut usize,
) -> Result<bool, &'static str> {
    if expected.len() != actual.len() {
        return Ok(false);
    }
    *fuel = fuel
        .checked_sub(expected.len().saturating_mul(2))
        .ok_or("method prefix budget exhausted")?;
    let binders = |ids: &[DeclarationId]| {
        ids.iter()
            .map(|id| DeclarationParameter {
                declaration: *id,
                sort: if domains
                    .get(id)
                    .is_some_and(|domain| *domain != Term::Primitive(crate::Primitive::Type))
                {
                    crate::Sort::ComptimeValue
                } else {
                    crate::Sort::Type
                },
            })
            .collect::<Vec<_>>()
    };
    let expected_binders = binders(expected);
    let actual_binders = binders(actual);
    let default = Term::Primitive(crate::Primitive::Type);
    for (left, right) in expected.iter().zip(actual) {
        *fuel = fuel
            .checked_sub(expected.len().saturating_mul(2))
            .ok_or("method prefix budget exhausted")?;
        let left = domains.get(left).unwrap_or(&default);
        let right = domains.get(right).unwrap_or(&default);
        crate::type_computations::charge_term(left, fuel)
            .map_err(|_| "method prefix budget exhausted")?;
        crate::type_computations::charge_term(right, fuel)
            .map_err(|_| "method prefix budget exhausted")?;
        if left.sort() != crate::Sort::Type
            || right.sort() != crate::Sort::Type
            || left.check_scope(&[]).is_err()
            || right.check_scope(&[]).is_err()
        {
            return Ok(false);
        }
        let (Ok(left), Ok(right)) = (
            left.abstract_parameters(&expected_binders),
            right.abstract_parameters(&actual_binders),
        ) else {
            return Ok(false);
        };
        if left != right {
            return Ok(false);
        }
    }
    Ok(true)
}
