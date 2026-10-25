use nir::{Parameter, Primitive as P, Sort, Term, TypeError};

fn signature() -> Term {
    Term::Function {
        parameters: vec![
            Parameter {
                sort: Sort::Type,
                ty: Term::Primitive(P::Type),
            },
            Parameter {
                sort: Sort::Runtime,
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
fn unsafe_identity_survives_staging_binding_and_result_replacement() {
    for comptime in [false, true] {
        let ordinary = signature().with_comptime_phase(comptime);
        let unsafe_type = Term::UnsafeFunction(Box::new(ordinary.clone()));
        unsafe_type.check_scope(&[]).unwrap();
        assert_ne!(unsafe_type, ordinary);
        assert!(unsafe_type.is_unsafe_callable());
        assert_eq!(unsafe_type.is_comptime_callable(), comptime);
        assert_eq!(unsafe_type.callable_signature(), &signature());
        assert!(unsafe_type.to_string().starts_with("(unsafe-function "));
        let bound = unsafe_type
            .bind_function_inputs(&[Some(Term::Primitive(P::Bool)), None])
            .unwrap();
        let expected = Term::Function {
            parameters: vec![Parameter {
                sort: Sort::Runtime,
                ty: Term::Primitive(P::Bool),
            }],
            result: Box::new(Term::Primitive(P::Bool)),
        };
        assert_eq!(
            bound,
            Term::UnsafeFunction(Box::new(expected.with_comptime_phase(comptime)))
        );
        let replaced = bound
            .with_callable_result(Term::Primitive(P::Char))
            .unwrap();
        replaced.check_scope(&[]).unwrap();
        assert!(replaced.is_unsafe_callable());
        assert_eq!(replaced.is_comptime_callable(), comptime);
        assert!(
            matches!(replaced.callable_signature(), Term::Function {result, ..} if **result == Term::Primitive(P::Char))
        );
    }
    let staged = Term::UnsafeFunction(Box::new(signature())).with_comptime_phase(true);
    staged.check_scope(&[]).unwrap();
    assert!(staged.is_unsafe_callable() && staged.is_comptime_callable());
}

#[test]
fn unsafe_qualification_has_one_canonical_position_inside_environment() {
    let raw = Term::UnsafeFunction(Box::new(signature().with_comptime_phase(true)));
    let closure = Term::contextual_closure(
        Term::ClosureContextValue(vec![Term::Primitive(P::Bool)]),
        raw.clone(),
    )
    .unwrap();
    closure.check_scope(&[]).unwrap();
    assert!(closure.is_unsafe_callable() && closure.is_comptime_callable());
    assert_eq!(closure.callable_signature(), &signature());
    for malformed in [
        Term::UnsafeFunction(Box::new(Term::Primitive(P::Bool))),
        Term::UnsafeFunction(Box::new(raw.clone())),
        Term::UnsafeFunction(Box::new(closure)),
    ] {
        assert_eq!(
            malformed.check_scope(&[]),
            Err(TypeError::InvalidUnsafeFunctionSignature)
        );
    }
    assert_eq!(
        Term::ComptimeFunction(Box::new(raw)).check_scope(&[]),
        Err(TypeError::InvalidComptimeFunctionSignature)
    );
}

#[test]
fn unsafe_wrapper_does_not_hide_free_binders_from_scope_validation() {
    let invalid = Term::UnsafeFunction(Box::new(Term::Function {
        parameters: vec![],
        result: Box::new(Term::Bound {
            depth: 0,
            index: 0,
            sort: Sort::Type,
        }),
    }));
    assert!(invalid.check_scope(&[]).is_err());
}

#[test]
fn unsafe_wrapper_cannot_hide_comptime_only_runtime_storage() {
    let signature = |result| Term::Function {
        parameters: vec![],
        result: Box::new(result),
    };
    let package = nir::program::Package::default();
    for inner in [
        signature(Term::Primitive(P::Type)),
        signature(Term::Primitive(P::Bool)).with_comptime_phase(true),
    ] {
        let ty = Term::UnsafeFunction(Box::new(inner));
        let error = nir::phases::validate_runtime_storage(&ty, &package, &[]).unwrap_err();
        assert!(error.contains("comptime-only"), "{error}");
    }
    let ty = Term::UnsafeFunction(Box::new(signature(Term::Primitive(P::Bool))));
    nir::phases::validate_runtime_storage(&ty, &package, &[]).unwrap();
}
