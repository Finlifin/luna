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

#[test]
fn opening_a_callable_frame_keeps_its_captures_distinct_from_chosen_slots() {
    use nir::DeclarationId;
    use nir::normal_forms::{callable_type_from_declarations, open_inputs};
    use nir::type_computations::Error;
    let id = |local| DeclarationId { package: 17, local };
    let rigid = |declaration| Term::Rigid {
        declaration,
        sort: Sort::Type,
    };
    let nested = |first, outer| Term::Function {
        parameters: vec![],
        result: Box::new(Term::Tuple(vec![first, outer])),
    };
    let ty = Term::Primitive(P::Type);
    let domain = nested(rigid(id(1)), rigid(id(7)));
    let head = callable_type_from_declarations(
        [
            (id(1), Sort::Type, &ty),
            (id(2), Sort::ComptimeValue, &domain),
        ],
        None,
        &mut 1000,
    )
    .unwrap();
    // Neither an earlier nor a later fresh key can reuse the captured identity.
    for ids in [[id(7), id(9)], [id(8), id(7)]] {
        assert_eq!(
            open_inputs(&head, &ids, &mut 1000),
            Err(Error::Scope(TypeError::CapturedDeclarationParameter(id(7))))
        );
    }
    let ids = [id(8), id(9)];
    let opened = open_inputs(&head, &ids, &mut 1000).unwrap();
    assert_eq!(opened[1].2, nested(rigid(id(8)), rigid(id(7))));
    let closed = callable_type_from_declarations(
        opened.iter().map(|(id, sort, domain)| (*id, *sort, domain)),
        None,
        &mut 1000,
    )
    .unwrap();
    assert_eq!(head, closed);
}

#[test]
fn forming_a_telescope_cannot_bind_a_declaration_already_read_as_free() {
    use nir::DeclarationId;
    use nir::callable_types::DeclarationTelescope;
    let id = |local| DeclarationId { package: 19, local };
    let rigid = |id| Term::Rigid {
        declaration: id,
        sort: Sort::Type,
    };
    let captured = |slot| {
        Err(nir::type_computations::Error::Scope(
            TypeError::CapturedDeclarationParameter(id(slot)),
        ))
    };
    let mut form = DeclarationTelescope::default();
    assert_eq!(
        form.push(id(1), Sort::ComptimeValue, &rigid(id(1)), &mut 100),
        captured(1)
    );
    form.push(id(1), Sort::ComptimeValue, &rigid(id(2)), &mut 100)
        .unwrap();
    assert_eq!(
        form.push(id(2), Sort::Type, &Term::Primitive(P::Type), &mut 100),
        captured(2)
    );
    form.push(id(3), Sort::Type, &Term::Primitive(P::Type), &mut 100)
        .unwrap();
    let signature = form.finish(&rigid(id(3)), &mut 100).unwrap();
    let Term::Function { parameters, result } = signature else {
        panic!()
    };
    assert_eq!(parameters.len(), 2);
    assert_eq!(parameters[0].ty, rigid(id(2)));
    assert_eq!(
        *result,
        Term::Bound {
            depth: 0,
            index: 1,
            sort: Sort::Type
        }
    );
}
