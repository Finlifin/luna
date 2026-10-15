//! Distinguish an unknown pointer origin from a field absent in this layout.
use super::{EnumDomains, Error, SourceRange, VARIANT_PATH, error};
use nir::{Term, program::Package};

pub(super) fn pointer_possible(
    package: Option<&Package>,
    ty: &Term,
    fields: &[usize],
    domains: &EnumDomains,
    qualified: &mut Vec<usize>,
    fuel: &mut usize,
    source: SourceRange,
    depth: usize,
) -> Result<bool, Error> {
    if depth > 128 {
        return Err(error(source, "pointer layout depth exhausted"));
    }
    *fuel = fuel
        .checked_sub(1)
        .ok_or_else(|| error(source, "pointer layout budget exhausted"))?;
    if let Some(types) = nir::normal_forms::tuple_fields(package, ty) {
        let Some((&field, rest)) = fields.split_first() else {
            return Ok(false);
        };
        let Some(ty) = types.get(field) else {
            return Ok(false);
        };
        qualified.push(field);
        let result = pointer_possible(
            package,
            ty,
            rest,
            domains,
            qualified,
            fuel,
            source,
            depth + 1,
        );
        qualified.pop();
        return result;
    }
    match ty {
        Term::Pointer(_) => Ok(fields.is_empty()),
        Term::Primitive(_) => Ok(false),
        Term::Record(types) => {
            let Some((&field, rest)) = fields.split_first() else {
                return Ok(false);
            };
            let Some((_, ty)) = types.get(field) else {
                return Ok(false);
            };
            qualified.push(field);
            let result = pointer_possible(
                package,
                ty,
                rest,
                domains,
                qualified,
                fuel,
                source,
                depth + 1,
            );
            qualified.pop();
            result
        }
        Term::Nominal {
            declaration,
            arguments,
        } => {
            let Some((&field, rest)) = fields.split_first() else {
                return Ok(false);
            };
            let Some(package) = package else {
                return Ok(true);
            };
            if let Some(definition) = package
                .structs
                .iter()
                .find(|item| item.declaration == *declaration)
            {
                for (_, ty) in &definition.fields {
                    crate::verification::memory::bound_type(ty, fuel)
                        .map_err(|e| error(source, e))?;
                }
                let types = definition
                    .instantiate_fields(arguments)
                    .map_err(|e| error(source, e.to_string()))?;
                let Some((_, ty)) = types.get(field) else {
                    return Ok(false);
                };
                qualified.push(field);
                let result = pointer_possible(
                    Some(package),
                    ty,
                    rest,
                    domains,
                    qualified,
                    fuel,
                    source,
                    depth + 1,
                );
                qualified.pop();
                return result;
            }
            if let Some(definition) = package
                .enums
                .iter()
                .find(|item| item.declaration == *declaration)
            {
                for variant in &definition.variants {
                    for ty in &variant.fields {
                        crate::verification::memory::bound_type(ty, fuel)
                            .map_err(|e| error(source, e))?;
                    }
                }
                let variants = definition
                    .instantiate_variants(arguments)
                    .map_err(|e| error(source, e.to_string()))?;
                let allowed = domains.get(qualified);
                for (index, variant) in variants.iter().enumerate() {
                    *fuel = fuel
                        .checked_sub(1)
                        .ok_or_else(|| error(source, "pointer variant budget exhausted"))?;
                    if allowed.is_some_and(|tags| !tags.contains(&index)) {
                        continue;
                    }
                    let Some(ty) = variant.fields.get(field) else {
                        continue;
                    };
                    qualified.extend([VARIANT_PATH, index, field]);
                    let possible = pointer_possible(
                        Some(package),
                        ty,
                        rest,
                        domains,
                        qualified,
                        fuel,
                        source,
                        depth + 1,
                    );
                    qualified.truncate(qualified.len() - 3);
                    if possible? {
                        return Ok(true);
                    }
                }
                return Ok(false);
            }
            Ok(true)
        }
        // Opaque normal forms, dynamic environments and unresolved descriptions
        // do not prove absence. Never unfold a type-producing function here.
        _ => Ok(true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opaque_types_and_exhausted_analysis_are_not_absence_evidence() {
        let source = SourceRange { start: 1, end: 2 };
        let opaque = Term::NormalForm {
            head: nir::DeclarationId {
                package: 1,
                local: 1,
            },
            sort: nir::Sort::Type,
            arguments: vec![],
        };
        let domains = EnumDomains::new();
        assert!(
            pointer_possible(
                None,
                &opaque,
                &[0],
                &domains,
                &mut vec![],
                &mut 100,
                source,
                0
            )
            .unwrap()
        );
        let scalar = Term::Primitive(nir::Primitive::Bool);
        assert!(
            !pointer_possible(
                None,
                &scalar,
                &[],
                &domains,
                &mut vec![],
                &mut 100,
                source,
                0
            )
            .unwrap()
        );
        assert!(
            pointer_possible(None, &scalar, &[], &domains, &mut vec![], &mut 0, source, 0).is_err()
        );
        assert!(
            pointer_possible(
                None,
                &scalar,
                &[],
                &domains,
                &mut vec![],
                &mut 100,
                source,
                129
            )
            .is_err()
        );
    }
}
