//! Nominal declaration binders. Layout substitution preserves value-indexed
//! identities; domain admission and trait/program obligations remain separate.
use crate::{DeclarationId, DeclarationParameter, Primitive, Sort, Term, TypeError};
use std::collections::BTreeSet;

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
) -> Result<(), String> {
    let parameters = binders(parameters, domains).map_err(|e| e.to_string())?;
    if parameters.len() != arguments.len() {
        return Err("nominal argument count mismatch".into());
    }
    for (index, (parameter, argument)) in parameters.iter().zip(arguments).enumerate() {
        crate::type_computations::charge_term(argument, fuel).map_err(|e| e.to_string())?;
        argument.check_scope(&[]).map_err(|e| e.to_string())?;
        if parameter.sort != argument.sort() {
            return Err("nominal argument sort mismatch".into());
        }
        if parameter.sort == Sort::Type {
            continue;
        }
        let domain = crate::type_computations::instantiate_parameters_bounded(
            &domains[index],
            &parameters[..index],
            &arguments[..index],
            &[],
            fuel,
        )
        .map_err(|e| e.to_string())?;
        if matches!(argument, Term::RecordValue(_)) && argument.is_structural_index_value() {
            crate::trait_parameters::validate_record_index(argument, &domain, fuel)?;
            continue;
        }
        if let (Term::Integer(value), Term::Primitive(primitive)) = (argument, &domain) {
            if let Some((signed, width)) = primitive.fixed_integer_layout() {
                let fits = if signed {
                    value.to_signed(width).is_ok()
                } else {
                    value.to_unsigned(width).is_ok()
                };
                if !fits {
                    return Err("nominal integer index is outside its declared domain".into());
                }
                continue;
            }
        }
        if argument.scalar_index_domain().map(Term::Primitive).as_ref() != Some(&domain) {
            return Err("nominal value index lacks matching completed domain evidence".into());
        }
    }
    Ok(())
}
