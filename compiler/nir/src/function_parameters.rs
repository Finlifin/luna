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
    let substitution =
        crate::DeclarationSubstitution::bounded(parameters, domains, arguments, &[], fuel)
            .map_err(|error| error.to_string())?;
    for (index, argument) in arguments.iter().enumerate() {
        let domain = domains
            .get(index)
            .unwrap_or(&Term::Primitive(crate::Primitive::Type));
        let domain = substitution
            .instantiate_bounded(domain, fuel)
            .map_err(|error| error.to_string())?;
        if domain == Term::Primitive(crate::Primitive::Type) {
            continue;
        }
        if let Term::TypeListValue(elements) = argument {
            let Term::NormalForm {
                head: declaration, ..
            } = &domain
            else {
                return Err("callable type-list domain is not a bound meta.List<Type>".into());
            };
            let mut found = None;
            for head in &package.normal_forms {
                *fuel = fuel
                    .checked_sub(1)
                    .ok_or("callable type-list evidence budget exhausted")?;
                if head.declaration == *declaration {
                    if found.is_some() {
                        return Err("ambiguous callable type-list representation evidence".into());
                    }
                    found = Some(head);
                }
            }
            if !found.is_some_and(|head| head.is_type_list_instance(&domain))
                || elements
                    .iter()
                    .any(|element| element.sort() != crate::Sort::Type)
            {
                return Err("callable type-list lacks matching meta.List<Type> evidence".into());
            }
            continue;
        }
        crate::nominal_parameters::validate_symbolic_value_in(
            argument,
            &domain,
            fuel,
            |ty, variant, fuel| {
                crate::normal_forms::nominal_value_fields_budgeted(package, ty, variant, fuel)
                    .ok_or_else(|| "callable input lacks validated layout evidence".into())
            },
            |_, _, _| false,
            package.target,
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(())
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
