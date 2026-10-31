//! Structural preflight before recursive type operations. No normalization or proof.
use crate::Term;

// Keep the stricter depth policy used by projection and memory consumers, but
// share complete term edges and payload accounting with other type operations.
pub fn charge_structure(ty: &Term, fuel: &mut usize) -> Result<(), &'static str> {
    crate::type_computations::charge_term_with_depth(ty, fuel, 64)
        .map_err(|_| "type structure budget exhausted")
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
        let mut fuel = 7;
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
    #[test]
    fn closure_and_callable_inputs_cannot_hide_depth_or_payload_work() {
        let id = crate::DeclarationId {
            package: 1,
            local: 1,
        };
        let wrappers: [fn(Term) -> Term; 4] = [
            |ty| Term::ClosureContextValue(vec![ty]),
            |ty| Term::ContextualClosure {
                context: Box::new(Term::ClosureContextType),
                signature: Box::new(ty),
            },
            |ty| Term::FunctionValue {
                declaration: crate::DeclarationId {
                    package: 1,
                    local: 1,
                },
                arguments: vec![ty],
            },
            |ty| Term::ClosureSnapshot {
                factory: crate::DeclarationId {
                    package: 1,
                    local: 1,
                },
                arguments: vec![ty],
            },
        ];
        for wrap in wrappers {
            let deep = (0..64).fold(boolean(), |ty, _| Term::Optional(Box::new(ty)));
            assert!(charge_structure(&wrap(deep), &mut 10_000).is_err());
            let payload = Term::NormalForm {
                head: id,
                sort: crate::Sort::Type,
                arguments: vec![Term::String("x".repeat(100).into())],
            };
            let input = wrap(payload);
            assert!(charge_structure(&input, &mut 50).is_err());
            charge_structure(&input, &mut 1000).unwrap();
        }
    }
}
