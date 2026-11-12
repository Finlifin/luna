use nir::{DeclarationId, Primitive, Term, copy_budget::CopyBudgetExhausted};
use type_resolution::calls::{CallableContext, InputContext as C};

fn description() -> C {
    let id = DeclarationId {
        package: 8,
        local: 3,
    };
    C::Callable(CallableContext {
        comptime: true,
        open_environment: true,
        environment: Some(vec![None, Some(Term::Primitive(Primitive::Bool))]),
        parameters: vec![Some(Term::Primitive(Primitive::Type)), None],
        result: None,
        result_context: Some(Box::new(C::Record(vec![
            (
                "field\"\n".into(),
                C::NormalTuple {
                    head: id,
                    list_domain: Some(Term::Nominal {
                        declaration: id,
                        arguments: vec![],
                    }),
                    fields: vec![C::Unknown, C::UnknownValue, C::UnknownDependentValue],
                },
            ),
            (
                "effect".into(),
                C::Computation {
                    effects: vec![nir::effects::Operation {
                        declaration: id,
                        arguments: vec![Term::String("operand".into())],
                    }],
                    result: Box::new(C::ErrorResult {
                        errors: vec![Term::Primitive(Primitive::Bool)],
                        result: Box::new(C::Optional(Box::new(C::Known(Term::Primitive(
                            Primitive::U8,
                        ))))),
                    }),
                },
            ),
            (
                "nominal".into(),
                C::Nominal {
                    declaration: id,
                    arguments: vec![C::Tuple(vec![C::Known(Term::Primitive(Primitive::Char))])],
                },
            ),
        ]))),
    })
}

#[test]
fn context_copy_preserves_partial_domains_and_retries_after_exhaustion() {
    let source = description();
    let expected = source.to_string();
    let mut failed = 0;
    let mut passed = 0;
    for allowance in 0..256 {
        let mut fuel = allowance;
        match source.clone_bounded(&mut fuel) {
            Ok(retained) => {
                assert_eq!(retained.to_string(), expected);
                assert!(fuel < allowance);
                passed += 1;
            }
            Err(error) => {
                assert_eq!(error, CopyBudgetExhausted::Steps);
                assert_eq!(source.to_string(), expected);
                assert_eq!(
                    source.clone_bounded(&mut 1000).unwrap().to_string(),
                    expected
                );
                failed += 1;
            }
        }
    }
    assert!(failed > 10 && passed > 0);
    let C::Callable(callable) = source else {
        unreachable!()
    };
    assert_eq!(
        callable.clone_bounded(&mut 1000).unwrap().to_string(),
        expected
    );
}

#[test]
fn context_copy_charges_payloads_and_preserves_depth_exhaustion() {
    let wide = C::Record(vec![("x".repeat(1000), C::Unknown)]);
    assert_eq!(
        wide.clone_bounded(&mut 100).unwrap_err(),
        CopyBudgetExhausted::Steps
    );
    let effect = C::Computation {
        effects: vec![nir::effects::Operation {
            declaration: DeclarationId {
                package: 8,
                local: 3,
            },
            arguments: vec![Term::String("x".repeat(1000).into())],
        }],
        result: Box::new(C::Unknown),
    };
    assert_eq!(
        effect.clone_bounded(&mut 100).unwrap_err(),
        CopyBudgetExhausted::Steps
    );
    let mut deep = C::Unknown;
    for _ in 0..128 {
        deep = C::Optional(Box::new(deep));
    }
    let mut fuel = 10_000;
    assert_eq!(
        deep.clone_bounded(&mut fuel).unwrap_err(),
        CopyBudgetExhausted::Depth
    );
    assert!(fuel > 0);
    // A term nested inside a partial context shares the context's remaining
    // depth allowance instead of receiving a fresh full-depth budget.
    let mut ty = Term::Primitive(Primitive::Bool);
    for _ in 0..80 {
        ty = Term::Pointer(Box::new(ty));
    }
    let mut mixed = C::Known(ty);
    for _ in 0..64 {
        mixed = C::Optional(Box::new(mixed));
    }
    assert_eq!(
        mixed.clone_bounded(&mut 10_000).unwrap_err(),
        CopyBudgetExhausted::Depth
    );
    let original = C::Known(Term::Primitive(Primitive::Bool));
    let mut fuel = 100;
    let retained = original.clone_bounded(&mut fuel).unwrap();
    let used = 100 - fuel;
    let mut insufficient = used - 1;
    assert_eq!(
        original.clone_bounded(&mut insufficient).unwrap_err(),
        CopyBudgetExhausted::Steps
    );
    assert_eq!(retained.to_string(), original.to_string());
}
