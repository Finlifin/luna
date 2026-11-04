//! Preflight the declaration metadata retained by a query world's private index.
//! This bounds copies; it is not formation evidence and never visits code bodies.
use super::*;

pub(super) fn spend(fuel: &mut usize, amount: usize) -> Result<(), AdmissionError> {
    *fuel = fuel.checked_sub(amount).ok_or(AdmissionError::Budget)?;
    Ok(())
}
fn term(value: &Term, fuel: &mut usize) -> Result<(), AdmissionError> {
    nir::type_computations::charge_term(value, fuel).map_err(Into::into)
}
fn terms(values: &[Term], fuel: &mut usize) -> Result<(), AdmissionError> {
    spend(fuel, values.len())?;
    for value in values {
        term(value, fuel)?;
    }
    Ok(())
}
fn predicates(values: &[n::TypePredicate], fuel: &mut usize) -> Result<(), AdmissionError> {
    spend(fuel, values.len())?;
    for value in values {
        nir::copy_budget::charge_predicate_substitution(value, &[], &[], fuel)
            .map_err(|_| AdmissionError::Budget)?;
    }
    Ok(())
}
pub(super) fn requirements(
    values: &[n::TraitRequirement],
    fuel: &mut usize,
) -> Result<(), AdmissionError> {
    spend(fuel, values.len())?;
    for value in values {
        term(&value.subject, fuel)?;
        terms(&value.interface_arguments, fuel)?;
    }
    Ok(())
}
fn declaration(
    name: &str,
    parameters: &[Id],
    domains: &[Term],
    conditions: &[n::TypePredicate],
    capabilities: &[n::TraitRequirement],
    fuel: &mut usize,
) -> Result<(), AdmissionError> {
    spend(fuel, name.len())?;
    spend(fuel, parameters.len())?;
    terms(domains, fuel)?;
    predicates(conditions, fuel)?;
    requirements(capabilities, fuel)
}
pub(super) fn implementation(
    value: &n::Implementation,
    fuel: &mut usize,
) -> Result<(), AdmissionError> {
    spend(fuel, value.type_parameters.len())?;
    terms(&value.parameter_types, fuel)?;
    terms(&value.interface_arguments, fuel)?;
    predicates(&value.requirements, fuel)?;
    term(&value.subject, fuel)
}
pub(super) fn layouts(package: &n::Package, fuel: &mut usize) -> Result<(), AdmissionError> {
    spend(fuel, package.normal_forms.len())?;
    for head in &package.normal_forms {
        spend(fuel, head.parameters.len())?;
        for parameter in &head.parameters {
            term(&parameter.ty, fuel)?;
        }
        predicates(&head.requirements, fuel)?;
    }
    spend(fuel, package.structs.len())?;
    for value in &package.structs {
        declaration(
            &value.name,
            &value.type_parameters,
            &value.parameter_types,
            &value.requirements,
            &value.capabilities,
            fuel,
        )?;
        spend(fuel, value.fields.len())?;
        for (name, ty) in &value.fields {
            spend(fuel, name.len())?;
            term(ty, fuel)?;
        }
    }
    spend(fuel, package.enums.len())?;
    for value in &package.enums {
        declaration(
            &value.name,
            &value.type_parameters,
            &value.parameter_types,
            &value.requirements,
            &value.capabilities,
            fuel,
        )?;
        spend(fuel, value.variants.len())?;
        for variant in &value.variants {
            spend(fuel, variant.name.len())?;
            terms(&variant.fields, fuel)?;
            if let n::VariantShape::Named(names) = &variant.shape {
                spend(fuel, names.len())?;
                for name in names {
                    spend(fuel, name.len())?;
                }
            }
        }
        spend(fuel, value.groups.len())?;
        for group in &value.groups {
            spend(fuel, group.len())?;
        }
        if let Some(encoding) = &value.encoding {
            spend(fuel, encoding.variants().len())?;
            for variant in encoding.variants() {
                spend(fuel, variant.name().len())?;
                spend(fuel, variant.fixed_mask().words().len())?;
                spend(fuel, variant.fixed_value().words().len())?;
                spend(fuel, variant.fields().len())?;
                for field in variant.fields() {
                    spend(fuel, field.name.len())?;
                }
            }
        }
        if let Some(encoding) = &value.symbol_encoding {
            spend(fuel, encoding.variants().len())?;
            for variant in encoding.variants() {
                spend(fuel, variant.name.len())?;
                spend(fuel, variant.value.len())?;
            }
        }
    }
    spend(fuel, package.newtypes.len())?;
    for value in &package.newtypes {
        declaration(
            &value.name,
            &value.type_parameters,
            &value.parameter_types,
            &value.requirements,
            &value.capabilities,
            fuel,
        )?;
        term(&value.representation, fuel)?;
    }
    Ok(())
}
