use nir::{Parameter, Primitive as P, Sort, Term, TypeError};

fn signature() -> Term {
    Term::Function {
        parameters: vec![
            Parameter {
                sort: Sort::Type,
                ty: Term::Primitive(P::Type),
            },
            Parameter {
                sort: Sort::ComptimeValue,
                ty: Term::Bound {
                    depth: 0,
                    index: 0,
                    sort: Sort::Type,
                },
            },
        ],
        result: Box::new(Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        }),
    }
}

#[test]
fn normal_form_callable_identity_keeps_calling_form_and_collector() {
    let ordinary = signature().with_comptime_phase(true);
    let fixed = Term::NormalFormFunction(Box::new(signature()), None);
    let collected = Term::NormalFormFunction(Box::new(signature()), Some(1));
    for ty in [&fixed, &collected] {
        ty.check_scope(&[]).unwrap();
        assert!(ty.is_normal_form_callable() && ty.is_comptime_callable());
        assert_eq!(ty.callable_signature(), &signature());
        assert_ne!(*ty, ordinary);
    }
    assert!(!ordinary.is_normal_form_callable());
    assert_eq!(fixed.clone().with_comptime_phase(true), fixed);
    assert_eq!(fixed.clone().with_comptime_phase(false), fixed);
    assert_ne!(fixed, collected);
    assert!(
        collected
            .to_string()
            .starts_with("(normal-form-function (collector 1)")
    );
    assert!(fixed.to_string().contains("(collector none)"));
}

#[test]
fn normal_form_specialization_renumbers_or_removes_the_collector() {
    let ty = Term::NormalFormFunction(Box::new(signature()), Some(1));
    let partial = ty
        .bind_function_inputs(&[Some(Term::Primitive(P::Bool)), None])
        .unwrap();
    assert_eq!(
        partial,
        Term::NormalFormFunction(
            Box::new(Term::Function {
                parameters: vec![Parameter {
                    sort: Sort::ComptimeValue,
                    ty: Term::Primitive(P::Bool)
                }],
                result: Box::new(Term::Primitive(P::Bool)),
            }),
            Some(0)
        )
    );
    let complete = partial
        .bind_function_inputs(&[Some(Term::Bool(true))])
        .unwrap();
    assert_eq!(
        complete,
        Term::NormalFormFunction(
            Box::new(Term::Function {
                parameters: vec![],
                result: Box::new(Term::Primitive(P::Bool)),
            }),
            None
        )
    );
    let result = partial
        .with_callable_result(Term::Primitive(P::Char))
        .unwrap();
    assert!(matches!(result, Term::NormalFormFunction(_, Some(0))));
    result.check_scope(&[]).unwrap();
}

#[test]
fn normal_form_callable_rejects_runtime_inputs_and_invalid_collectors() {
    for collector in [Some(0), Some(2), Some(usize::MAX)] {
        assert_eq!(
            Term::NormalFormFunction(Box::new(signature()), collector).check_scope(&[]),
            Err(TypeError::InvalidNormalFormFunctionSignature)
        );
    }
    let mut runtime = signature();
    let Term::Function { parameters, .. } = &mut runtime else {
        unreachable!()
    };
    parameters[1].sort = Sort::Runtime;
    assert_eq!(
        Term::NormalFormFunction(Box::new(runtime), None).check_scope(&[]),
        Err(TypeError::InvalidNormalFormFunctionSignature)
    );
    assert_eq!(
        Term::NormalFormFunction(Box::new(Term::Primitive(P::Bool)), None).check_scope(&[]),
        Err(TypeError::InvalidNormalFormFunctionSignature)
    );
}
