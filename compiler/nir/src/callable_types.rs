//! Callable types assembled from Type values in the caller's outer scope.
//! Named dependent telescopes abstract their own parameters separately.
use crate::type_computations::{Error, charge_term};
use crate::{Parameter, Primitive, Sort, Term, TypeError};

fn check_type(ty: &Term, fuel: &mut usize) -> Result<(), Error> {
    charge_term(ty, fuel)?;
    if ty.sort() != Sort::Type {
        return Err(Error::Scope(TypeError::WrongSort {
            expected: Sort::Type,
            actual: ty.sort(),
        }));
    }
    Ok(())
}

/// Each operand is evaluated outside the new callable frame. Lift free bound
/// references once; references bound by nested functions remain untouched.
pub fn from_outer(
    inputs: Vec<Term>,
    staged: &[bool],
    comptime: bool,
    fuel: &mut usize,
) -> Result<Term, Error> {
    *fuel = fuel
        .checked_sub(inputs.len().saturating_add(1))
        .ok_or(Error::Budget)?;
    if inputs.len() != staged.len() {
        return Err(Error::Scope(TypeError::ArgumentCount {
            expected: staged.len(),
            actual: inputs.len(),
        }));
    }
    for ty in &inputs {
        check_type(ty, fuel)?;
    }
    let parameters = inputs
        .iter()
        .zip(staged)
        .map(|(ty, staged)| {
            Ok(Parameter {
                sort: if !staged {
                    Sort::Runtime
                } else if *ty == Term::Primitive(Primitive::Type) {
                    Sort::Type
                } else {
                    Sort::ComptimeValue
                },
                ty: ty.abstract_parameters(&[]).map_err(Error::Scope)?,
            })
        })
        .collect::<Result<_, Error>>()?;
    Ok(Term::Function {
        parameters,
        result: Box::new(Term::Primitive(Primitive::Void)),
    }
    .with_comptime_phase(comptime))
}

/// Replace a callable result with an outer-scope Type value, preserving every
/// existing parameter binder, environment, phase and responsibility wrapper.
/// Unlike Term::with_callable_result, this introduces the result into scope.
pub fn with_result_from_outer(
    signature: Term,
    result: Term,
    fuel: &mut usize,
) -> Result<Term, Error> {
    charge_term(&signature, fuel)?;
    check_type(&result, fuel)?;
    let result = result.abstract_parameters(&[]).map_err(Error::Scope)?;
    signature
        .with_callable_result(result)
        .ok_or(Error::Scope(TypeError::InvalidFunctionSignature))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bound(depth: u32, index: usize, sort: Sort) -> Term {
        Term::Bound { depth, index, sort }
    }
    fn ty() -> Term {
        Term::Primitive(Primitive::Type)
    }

    #[test]
    fn outer_inputs_cross_exactly_one_frame_without_capturing_inner_inputs() {
        let outer = bound(0, 0, Sort::Type);
        let nested = Term::Function {
            parameters: vec![Parameter {
                sort: Sort::Type,
                ty: ty(),
            }],
            result: Box::new(Term::Record(vec![
                ("inner".into(), bound(0, 0, Sort::Type)),
                ("outer".into(), bound(1, 0, Sort::Type)),
            ])),
        };
        let signature = from_outer(
            vec![outer.clone(), nested.clone()],
            &[false, false],
            false,
            &mut 1000,
        )
        .unwrap();
        let signature = with_result_from_outer(signature, nested, &mut 1000).unwrap();
        signature.check_scope(&[vec![Sort::Type]]).unwrap();
        let Term::Function { parameters, result } = &signature else {
            panic!("{signature}")
        };
        assert_eq!(parameters[0].ty, bound(1, 0, Sort::Type));
        assert_eq!(parameters[1].ty, **result);
        let Term::Function { result, .. } = &**result else {
            panic!("{signature}")
        };
        assert_eq!(
            **result,
            Term::Record(vec![
                ("inner".into(), bound(0, 0, Sort::Type)),
                ("outer".into(), bound(2, 0, Sort::Type)),
            ])
        );
        assert!(signature.to_string().starts_with("(function"));
    }

    #[test]
    fn mixed_indices_and_callable_wrappers_keep_scope_and_identity() {
        let head = crate::DeclarationId {
            package: 1,
            local: 7,
        };
        let indexed = Term::NormalForm {
            head,
            sort: Sort::Type,
            arguments: vec![bound(0, 0, Sort::Type), bound(0, 1, Sort::ComptimeValue)],
        };
        let function =
            from_outer(vec![ty(), indexed.clone()], &[true, true], true, &mut 1000).unwrap();
        let signature = Term::Closure {
            environment: vec![bound(0, 0, Sort::Type)],
            signature: Box::new(function.with_unsafe_call_requirement(true)),
        };
        let signature = with_result_from_outer(signature, indexed, &mut 1000).unwrap();
        signature
            .check_scope(&[vec![Sort::Type, Sort::ComptimeValue]])
            .unwrap();
        assert!(signature.is_comptime_callable());
        assert!(signature.is_unsafe_callable());
        let Term::Closure { environment, .. } = &signature else {
            panic!("{signature}")
        };
        assert_eq!(environment, &[bound(0, 0, Sort::Type)]);
        let Term::Function { parameters, result } = signature.callable_signature() else {
            panic!("{signature}")
        };
        assert_eq!(parameters[0].sort, Sort::Type);
        assert_eq!(parameters[1].sort, Sort::ComptimeValue);
        assert_eq!(parameters[1].ty, **result);
        assert_eq!(
            **result,
            Term::NormalForm {
                head,
                sort: Sort::Type,
                arguments: vec![bound(1, 0, Sort::Type), bound(1, 1, Sort::ComptimeValue)],
            }
        );
    }

    #[test]
    fn construction_rejects_bad_domains_and_limits_then_recovers() {
        assert!(from_outer(vec![ty()], &[], false, &mut 100).is_err());
        assert!(from_outer(vec![Term::Bool(true)], &[false], false, &mut 100).is_err());
        assert!(with_result_from_outer(ty(), ty(), &mut 100).is_err());
        let deep = (0..129).fold(ty(), |ty, _| Term::Optional(Box::new(ty)));
        assert_eq!(
            from_outer(vec![deep], &[false], false, &mut 10_000),
            Err(Error::Budget)
        );
        assert_eq!(
            from_outer(vec![ty()], &[false], false, &mut 0),
            Err(Error::Budget)
        );
        let signature = from_outer(vec![ty()], &[false], false, &mut 100).unwrap();
        assert_eq!(
            with_result_from_outer(signature.clone(), ty(), &mut 0),
            Err(Error::Budget)
        );
        assert!(with_result_from_outer(signature.clone(), Term::Bool(true), &mut 100).is_err());
        with_result_from_outer(signature, ty(), &mut 100)
            .unwrap()
            .check_scope(&[])
            .unwrap();
    }
}
