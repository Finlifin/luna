//! Structural preflight before recursive type operations. No normalization or proof.
use crate::Term;

// Type keys are walked iteratively before recursive cloning/substitution. This
// is a structural work limit, not a type proposition or normalization step.
pub fn charge_structure(ty: &Term, fuel: &mut usize) -> Result<(), &'static str> {
    let mut pending = vec![(ty, 0)];
    while let Some((ty, depth)) = pending.pop() {
        if *fuel == 0 || depth >= 64 {
            return Err("type structure budget exhausted");
        }
        *fuel -= 1;
        match ty {
            Term::ErrorResult { errors, result } => {
                if errors.len().saturating_add(1) > *fuel {
                    return Err("type structure budget exhausted");
                }
                pending.push((result, depth + 1));
                pending.extend(errors.iter().map(|ty| (ty, depth + 1)));
            }
            Term::Computation { effects, result } => {
                *fuel = fuel
                    .checked_sub(effects.len())
                    .ok_or("type structure budget exhausted")?;
                let width = effects.iter().try_fold(1usize, |n, operation| {
                    n.checked_add(operation.arguments.len())
                });
                if width.is_none_or(|n| n > *fuel) {
                    return Err("type structure budget exhausted");
                }
                pending.push((result, depth + 1));
                for operation in effects {
                    pending.extend(operation.arguments.iter().map(|ty| (ty, depth + 1)));
                }
            }
            Term::AssociatedType {
                arguments, subject, ..
            } => {
                pending.push((subject, depth + 1));
                pending.extend(arguments.iter().map(|ty| (ty, depth + 1)));
            }
            Term::Optional(inner)
            | Term::Pointer(inner)
            | Term::ComptimeFunction(inner)
            | Term::NormalFormFunction(inner, _)
            | Term::UnsafeFunction(inner) => pending.push((inner, depth + 1)),
            Term::OptionalValue { ty, value } => {
                pending.push((ty, depth + 1));
                if let Some(value) = value {
                    pending.push((value, depth + 1));
                }
            }
            Term::ErrorValue { ty, label, value } => {
                pending.push((ty, depth + 1));
                if let Some(label) = label {
                    pending.push((label, depth + 1));
                }
                pending.push((value, depth + 1));
            }
            Term::ListConcat { ty, parts: fields } | Term::NominalValue { ty, fields, .. } => {
                pending.push((ty, depth + 1));
                pending.extend(fields.iter().map(|field| (field, depth + 1)));
            }
            Term::Record(fields) | Term::RecordValue(fields) => {
                pending.extend(fields.iter().map(|(_, ty)| (ty, depth + 1)))
            }
            Term::Tuple(fields)
            | Term::TypeListValue(fields)
            | Term::TupleValue(fields)
            | Term::Nominal {
                arguments: fields, ..
            }
            | Term::NormalForm {
                arguments: fields, ..
            } => pending.extend(fields.iter().map(|ty| (ty, depth + 1))),
            Term::Closure {
                environment,
                signature,
            } => {
                pending.push((signature, depth + 1));
                pending.extend(environment.iter().map(|ty| (ty, depth + 1)));
            }
            Term::Function { parameters, result } => {
                pending.push((result, depth + 1));
                pending.extend(
                    parameters
                        .iter()
                        .map(|parameter| (&parameter.ty, depth + 1)),
                );
            }
            _ => (),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boolean() -> Term {
        Term::Primitive(crate::Primitive::Bool)
    }

    #[test]
    fn depth_boundary_and_fuel_recovery() {
        let mut value = boolean();
        for _ in 0..63 {
            value = Term::Optional(Box::new(value));
        }
        let mut fuel = 64;
        assert_eq!(charge_structure(&value, &mut fuel), Ok(()));
        assert_eq!(fuel, 0);
        value = Term::Optional(Box::new(value));
        assert!(charge_structure(&value, &mut 1000).is_err());
        let shallow = Term::Record(vec![("value".into(), boolean())]);
        assert!(charge_structure(&shallow, &mut 1).is_err());
        let mut fuel = 2;
        assert_eq!(charge_structure(&shallow, &mut fuel), Ok(()));
        assert_eq!(fuel, 0);
    }

    #[test]
    fn rows_charge_labels_and_argument_structure() {
        let errors = Term::ErrorResult {
            errors: vec![boolean()],
            result: Box::new(boolean()),
        };
        assert!(charge_structure(&errors, &mut 2).is_err());
        let mut fuel = 3;
        charge_structure(&errors, &mut fuel).unwrap();
        assert_eq!(fuel, 0);
        let effects = Term::Computation {
            effects: vec![crate::effects::Operation {
                declaration: crate::DeclarationId {
                    package: 1,
                    local: 1,
                },
                arguments: vec![boolean()],
            }],
            result: Box::new(boolean()),
        };
        assert!(charge_structure(&effects, &mut 3).is_err());
        let mut fuel = 4;
        charge_structure(&effects, &mut fuel).unwrap();
        assert_eq!(fuel, 0);
    }
}
